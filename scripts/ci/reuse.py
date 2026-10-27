"""Skip CI suites that already passed on exactly the same inputs.

A suite's result depends on the files it builds and reads, not on which commit they came from. plan.py fingerprints
every suite's inputs in the tree being tested (for a pull request, its merge with the current main) and asks the
Actions cache whether each of the suite's jobs already passed with that fingerprint. Jobs record a pass as their last
step: a tiny cache entry keyed `<suite key>-<job>` (the "Record the pass" steps in ci.yml).

A suite's inputs (the same list decides whether a change selects the suite at all, plan.py):

  - COMMON: ci.yml, the admission gate and scripts/ci (this file); for Rust suites also RUST_COMMON, the workspace
    manifest and lockfile, the toolchain and cargo's config;
  - SUITE_INPUTS: the non-Rust files the suite's jobs use;
  - for Rust suites, the directories of the workspace packages it builds and of every workspace package those depend
    on (path dependencies of every kind, from the Cargo.toml files), plus every file outside those directories that
    their Rust sources name with a relative path (include_str!("../../tests/fixtures/x"),
    CARGO_MANIFEST_DIR joined with "../.." and a path constant), found by scanning the sources on every run.

Git object ids identify the contents, so the fingerprint is the hash of the sorted (path, object id) list from
`git ls-tree` on the checked-out commit: no file is read except the Rust sources that are scanned.

Which passes count: a pull request trusts its own earlier runs and main's; main trusts only main's; manual runs, the
schedule and the `ci:full` label reuse nothing. If a pull request changes a Rust build file that no selected suite's
fingerprint covers, nothing is reused either (fail closed). A pull request could forge a pass in its own runs, as it
could edit its own workflows, but no other ref sees its cache entries.
"""

from __future__ import annotations

import fnmatch
import hashlib
import os
import posixpath
import re
import subprocess
import tomllib

VERSION = "v1"
PASS_PREFIX = "ci-pass"

COMMON = [".github/workflows/ci.yml", ".github/workflows/macos-gate.yml", "scripts/ci/**"]
# Every Rust suite's inputs, too (a suite without packages builds no Rust).
RUST_COMMON = ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".cargo/**"]

# The root package (the server, `vaultwarden`) lives at the repository root next to everything else: these are its
# own files. Its other members (macros, crates/*) have directories of their own.
ROOT_PACKAGE = "vaultwarden"
ROOT_INPUTS = ["src/**", "build.rs", "migrations/**", "tests/**", "diesel.toml", "rustfmt.toml"]
ROOT_SCAN = ["src", "tests", "build.rs"]

# The workspace packages each Rust suite builds (their dependencies are added).
EVERY_PACKAGE_BUT_THE_APP = [ROOT_PACKAGE, "macros", "reins-proto", "reins-policy", "reins-core", "reins-e2e",
                             "reins-desktop", "reins-laya", "reins-replay"]
SUITE_PACKAGES: dict[str, list[str]] = {
    "rust": EVERY_PACKAGE_BUT_THE_APP,
    "clippy": EVERY_PACKAGE_BUT_THE_APP,
    "replay": ["reins-replay"],
    "desktop-linux": ["reins-desktop-app"],
    "desktop-macos": ["reins-desktop", "reins-desktop-app"],
    "desktop-windows": ["reins-desktop", "reins-desktop-app"],
    "android": ["reins-core"],
    "ios": ["reins-core"],
}

# The non-Rust files each suite's jobs use.
SUITE_INPUTS: dict[str, list[str]] = {
    "rust": [],
    "clippy": [],
    "replay": [],
    # scripts/test-linux-packages.py builds a fake release from these (and pins nfpm against release.yml's).
    "packages": ["scripts/package/**", "scripts/test-linux-packages.py", "scripts/release-feed.sh",
                 "scripts/install.sh", "scripts/install.ps1", ".github/workflows/release.yml",
                 "crates/reins-desktop-app/packaging/**", "crates/reins-desktop-app/assets/icons/app-256.png",
                 "ios/Shared/Fonts/FONTS-NOTICE.txt", "LICENSE", "LICENSE-AGPL", "LICENSING.md", "NOTICE",
                 "README.md"],
    "desktop-linux": [],
    "desktop-macos": [],
    # The job runs scripts/install.ps1 against the test build.
    "desktop-windows": ["scripts/install.ps1"],
    "android": ["android/**", "scripts/build-android-native.sh", "scripts/check-android-apk.py"],
    "ios": ["ios/**", "scripts/ios-ui-plan.py"],
}

SUITES = list(SUITE_INPUTS)

# A ../ path that names something (a bare "../.." names no file); "/../x" as in
# concat!(env!("CARGO_MANIFEST_DIR"), "/../x") too.
RELATIVE_PATH = re.compile(r'"/?((?:\.\./)+[^"\\\n{}./][^"\\\n{}]*)"')
# The package directory's parent reached in code. The scan follows one form, a join of a "../" literal and then of a
# string constant of the same file (`.join("../..").join(PALETTE_FIXTURE)`); a suite whose sources do anything else
# reads the whole repository (fail closed).
UNFOLLOWABLE = re.compile(r'CARGO_MANIFEST_DIR[^;]{0,200}?(?:\.parent\(\)|\.ancestors\(\)|join\(\s*"(?:\.\./?)+"\s*\))')
JOINED_CONSTANT = re.compile(r'CARGO_MANIFEST_DIR"\)\s*\)\s*\.join\(\s*"((?:\.\./?)+)"\s*\)\s*\.join\(\s*([A-Z][A-Z0-9_]*)\s*\)')
CONSTANT = re.compile(r'\bconst\s+([A-Z][A-Z0-9_]*)\s*:\s*&(?:\'static\s+)?str\s*=\s*"([^"\\\n]+)"\s*;')
EVERYTHING = ["**"]
# Parent reaches the scan cannot follow but that are known: file -> (how many reaches it has, the workspace packages
# they read). reins-e2e builds the server binary from the repository root. scripts/test-ci-reuse.py checks that each
# file still has exactly that many reaches; one more makes its suites read the whole repository again.
KNOWN_PARENT_READS: dict[str, tuple[int, list[str]]] = {
    "crates/reins-e2e/src/server.rs": (1, [ROOT_PACKAGE]),
}
PACKAGE_MARK = "package:"
DEPENDENCY_TABLES = ("dependencies", "dev-dependencies", "build-dependencies")

# Changed files here that no selected suite's fingerprint covers mean the inputs are not understood: nothing is reused.
BUILD_PATHS = ["crates/**", "src/**", "macros/**", "migrations/**", "tests/**", "build.rs", "Cargo.toml",
               "Cargo.lock", ".cargo/**", "rust-toolchain.toml"]


def matches(path: str, patterns: list[str]) -> bool:
    for pattern in patterns:
        if pattern == "**":
            return True
        if pattern.endswith("/**"):
            if path.startswith(pattern[:-2]):
                return True
        elif fnmatch.fnmatchcase(path, pattern):
            return True
    return False


def tree_index(root: str = ".", treeish: str = "HEAD") -> dict[str, str]:
    """path -> git object id for every file of `treeish` (submodules: their commit)."""
    out = subprocess.run(["git", "-C", root, "ls-tree", "-r", "-z", "--full-tree", treeish],
                         check=True, capture_output=True).stdout.decode()
    index = {}
    for entry in out.split("\0"):
        if entry:
            meta, path = entry.split("\t", 1)
            index[path] = meta.split()[2]
    return index


class Workspace:
    """Workspace packages and their path dependencies, from the Cargo.toml files.

    `read(path)` returns a file's text; by default from the working tree under `root`. The dry run (plan.py
    --history) reads them from a commit instead.
    """

    def __init__(self, root: str = ".", read=None, files: list[str] | None = None):
        self.root = root
        self.read = read or (lambda path: read_file(os.path.join(root, path)))
        self._files = files
        manifest = tomllib.loads(self.read("Cargo.toml"))
        workspace = manifest["workspace"]
        shared = workspace.get("dependencies", {})
        self.dirs: dict[str, str] = {}  # package -> directory ("." for the root package)
        manifests: dict[str, dict] = {}
        members = list(workspace["members"])
        if "package" in manifest:
            self.dirs[manifest["package"]["name"]] = "."
            manifests["."] = manifest
        for member in members:
            data = tomllib.loads(self.read(posixpath.join(member, "Cargo.toml")))
            self.dirs[data["package"]["name"]] = member
            manifests[member] = data
        by_dir = {d: name for name, d in self.dirs.items()}
        self.deps: dict[str, set[str]] = {}
        for member, data in manifests.items():
            tables = [data.get(t, {}) for t in DEPENDENCY_TABLES]
            for target in data.get("target", {}).values():
                tables += [target.get(t, {}) for t in DEPENDENCY_TABLES]
            found = set()
            for table in tables:
                for key, spec in table.items():
                    spec = spec if isinstance(spec, dict) else {}
                    path = spec.get("path")
                    base = member
                    if spec.get("workspace"):
                        shared_spec = shared.get(key, {})
                        path = shared_spec.get("path") if isinstance(shared_spec, dict) else None
                        base = "."
                    if path is not None:
                        target_dir = posixpath.normpath(posixpath.join(base, path))
                        if target_dir in by_dir:
                            found.add(by_dir[target_dir])
            self.deps[by_dir[member]] = found

    def closure(self, packages: list[str]) -> set[str]:
        """The packages in `packages` and everything they depend on."""
        seen: set[str] = set()
        stack = [p for p in packages if p in self.dirs]
        missing = [p for p in packages if p not in self.dirs]
        if missing:
            raise KeyError(f"not workspace packages: {', '.join(missing)}")
        while stack:
            package = stack.pop()
            if package in seen:
                continue
            seen.add(package)
            stack.extend(self.deps.get(package, ()))
        return seen

    def inputs(self, packages: set[str]) -> tuple[list[str], set[str]]:
        """(patterns, directories) of these packages' own files."""
        patterns: list[str] = []
        dirs: set[str] = set()
        for package in packages:
            directory = self.dirs[package]
            if directory == ".":
                patterns += ROOT_INPUTS
            else:
                dirs.add(directory)
        return patterns, dirs

    def references(self, packages: set[str], index: dict[str, str]) -> set[str]:
        """Paths outside these packages that their Rust sources name relatively ("*": cannot tell)."""
        patterns, dirs = self.inputs(packages)
        found: set[str] = set()
        for package in packages:
            for target in self._named_paths(self.dirs[package], index):
                if target == "*" or target.startswith(PACKAGE_MARK) or not (target in dirs or any(target.startswith(d + "/") for d in dirs)
                                         or matches(target, patterns)):
                    found.add(target)
        return found

    def _rust_files(self, top: str, index: dict[str, str]) -> list[str]:
        if top == ".":
            return [p for p in index if p.endswith(".rs") and (p in ROOT_SCAN or matches(p, [d + "/**" for d in ROOT_SCAN]))]
        return [p for p in index if p.endswith(".rs") and p.startswith(top + "/")]

    def _named_paths(self, top: str, index: dict[str, str]) -> set[str]:
        """Every existing path a Rust file of the package at `top` names with a ../ path."""
        cache = self.__dict__.setdefault("_named", {})
        if top in cache:
            return cache[top]
        named: set[str] = set()
        for path in self._rust_files(top, index):
            text = self.read(path)
            rel_dir = posixpath.dirname(path) or "."
            if UNFOLLOWABLE.search(text):
                followed = followed_joins(text, top, index)
                known = KNOWN_PARENT_READS.get(path)
                if followed is not None:
                    named |= followed
                elif known and len(UNFOLLOWABLE.findall(text)) == known[0]:
                    named |= {PACKAGE_MARK + package for package in known[1]}
                else:
                    named.add("*")
            for literal in RELATIVE_PATH.findall(text):
                # include_str!/include_bytes! resolve from the file, CARGO_MANIFEST_DIR from the package.
                for base in (rel_dir, top):
                    target = posixpath.normpath(posixpath.join(base, literal))
                    if not target.startswith("..") and exists(target, index):
                        named.add(target)
                        break
        cache[top] = named
        return named


def read_file(path: str) -> str:
    with open(path, encoding="utf-8", errors="replace") as f:
        return f.read()


def followed_joins(text: str, top: str, index: dict[str, str]) -> set[str] | None:
    """The paths of `.join("../..").join(CONSTANT)` uses, if every parent reach in `text` is one; else None."""
    constants = dict(CONSTANT.findall(text))
    reaches = len(UNFOLLOWABLE.findall(text))
    joins = JOINED_CONSTANT.findall(text)
    if len(joins) != reaches:
        return None
    found = set()
    for up, name in joins:
        if name not in constants:
            return None
        target = posixpath.normpath(posixpath.join(top, up, constants[name]))
        if target.startswith("..") or not exists(target, index):
            return None
        found.add(target)
    return found


def exists(path: str, index: dict[str, str]) -> bool:
    if path in index:
        return True
    prefix = path.rstrip("/") + "/"
    return any(p.startswith(prefix) for p in index)


def fingerprint(suite: str, index: dict[str, str], patterns: list[str], dirs: set[str], variant: str = "") -> str:
    """Hash of every (path, object id) a suite depends on. `dirs` are file or directory paths."""
    prefixes = tuple(d.rstrip("/") + "/" for d in dirs)
    lines = []
    for path, oid in index.items():
        if path in dirs or path.startswith(prefixes) or matches(path, patterns):
            lines.append(f"{path} {oid}")
    lines.sort()
    digest = hashlib.sha256(f"{VERSION}\n{suite}\n{variant.strip()}\n".encode())
    digest.update("\n".join(lines).encode())
    return digest.hexdigest()[:40]


def suite_inputs(suites: list[str], index: dict[str, str], workspace: Workspace) -> tuple[
        dict[str, tuple[list[str], set[str]]], dict[str, list[str]]]:
    """Per suite: (patterns, directories) of its inputs, and the paths outside its packages its sources read."""
    inputs: dict[str, tuple[list[str], set[str]]] = {}
    extra: dict[str, list[str]] = {}
    for suite in suites:
        patterns = COMMON + SUITE_INPUTS[suite]
        dirs: set[str] = set()
        if suite in SUITE_PACKAGES:
            patterns = patterns + RUST_COMMON
            packages = workspace.closure(SUITE_PACKAGES[suite])
            refs = workspace.references(packages, index)
            # Packages a known parent reach reads (and their dependencies) are inputs as a whole.
            read = {r[len(PACKAGE_MARK):] for r in refs if r.startswith(PACKAGE_MARK)}
            refs -= {PACKAGE_MARK + r for r in read}
            if read - packages:
                packages = packages | workspace.closure(sorted(read))
                refs |= workspace.references(packages, index)
                refs = {r for r in refs if not r.startswith(PACKAGE_MARK)}
            own, dirs = workspace.inputs(packages)
            patterns = patterns + own
            extra[suite] = sorted(r if r != "*" else "the whole repository (a path built from "
                                  "CARGO_MANIFEST_DIR's parent)" for r in refs)
            if "*" in refs:
                patterns = patterns + EVERYTHING
                refs.discard("*")
            dirs |= refs
        inputs[suite] = (patterns, dirs)
    return inputs, extra


def suite_keys(suites: list[str], index: dict[str, str], workspace: Workspace,
               variants: dict[str, str] | None = None) -> tuple[dict[str, str], dict[str, list[str]],
                                                                dict[str, tuple[list[str], set[str]]]]:
    """Per suite: the cache key prefix of its passes, the paths outside its packages it reads, and its inputs."""
    inputs, extra = suite_inputs(suites, index, workspace)
    keys = {}
    for suite in suites:
        patterns, dirs = inputs[suite]
        variant = (variants or {}).get(suite, "")
        keys[suite] = f"{PASS_PREFIX}-{VERSION}-{suite}-{fingerprint(suite, index, patterns, dirs, variant)}"
    return keys, extra, inputs


def covered(path: str, inputs: dict[str, tuple[list[str], set[str]]]) -> bool:
    """Whether some suite's inputs include `path`."""
    for patterns, dirs in inputs.values():
        if path in dirs or any(path.startswith(d.rstrip("/") + "/") for d in dirs) or matches(path, patterns):
            return True
    return False


def passed_jobs(api, repo: str, key: str, trusted_refs: set[str]) -> set[str]:
    """Jobs recorded as passed under `key` on a trusted ref."""
    jobs = set()
    page = 1
    while True:
        data = api(f"/repos/{repo}/actions/caches?key={key}-&per_page=100&page={page}")
        caches = data.get("actions_caches", [])
        for cache in caches:
            if cache.get("ref") in trusted_refs and cache["key"].startswith(key + "-"):
                jobs.add(cache["key"][len(key) + 1:])
        if len(caches) < 100:
            return jobs
        page += 1
