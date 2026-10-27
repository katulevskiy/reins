#!/usr/bin/env python3
"""Tests scripts/ci/reuse.py: suite inputs from the Cargo graph, the source scan, fingerprints and pass lookups."""

import json
import os
import shutil
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts/ci"))
import reuse  # noqa: E402


def write(root: Path, files: dict[str, str]) -> None:
    for path, text in files.items():
        (root / path).parent.mkdir(parents=True, exist_ok=True)
        (root / path).write_text(text)


WORKSPACE = {
    "Cargo.toml": '[workspace]\nmembers = ["crates/a", "crates/b", "crates/c"]\n\n'
                  '[package]\nname = "vaultwarden"\n\n[dependencies]\nb = { path = "crates/b" }\n',
    "Cargo.lock": "",
    "src/main.rs": "fn main() {}\n",
    "crates/a/Cargo.toml": '[package]\nname = "a"\n\n[dependencies]\nb = { path = "../b" }\n',
    "crates/a/src/lib.rs": 'const X: &str = include_str!("../../../design/x.json");\n',
    "crates/b/Cargo.toml": '[package]\nname = "b"\n\n[dev-dependencies]\nc = { path = "../c" }\n',
    "crates/b/src/lib.rs": "",
    "crates/c/Cargo.toml": '[package]\nname = "c"\n',
    "crates/c/src/lib.rs": "",
    "design/x.json": "{}",
    "docs/readme.md": "",
}


class Repo:
    def __init__(self, files):
        self.dir = tempfile.TemporaryDirectory(prefix="reins-ci-reuse-", dir=os.environ.get("TMPDIR"))
        self.root = Path(self.dir.name)
        write(self.root, files)
        self.git("init", "-q")
        self.commit()

    def __del__(self):
        self.dir.cleanup()

    def git(self, *args):
        env = os.environ | {"GIT_CONFIG_GLOBAL": "/dev/null", "GIT_CONFIG_NOSYSTEM": "1"}
        return subprocess.run(["git", "-C", str(self.root), *args], check=True, capture_output=True, text=True,
                              env=env).stdout

    def commit(self):
        self.git("add", "-A")
        self.git("-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "-q", "--allow-empty", "-m", "x")

    def keys(self, suites):
        index = reuse.tree_index(str(self.root))
        return reuse.suite_keys(suites, index, reuse.Workspace(str(self.root)))


class ReuseTest(unittest.TestCase):
    def setUp(self):
        self.saved = (dict(reuse.SUITE_PACKAGES), dict(reuse.SUITE_INPUTS))
        reuse.SUITE_PACKAGES.clear()
        reuse.SUITE_PACKAGES.update({"sa": ["a"], "sc": ["c"], "server": [reuse.ROOT_PACKAGE]})
        reuse.SUITE_INPUTS.clear()
        reuse.SUITE_INPUTS.update({"sa": [], "sc": ["tools/c/**"], "server": [], "docs": ["docs/**"]})


    def tearDown(self):
        reuse.SUITE_PACKAGES.clear()
        reuse.SUITE_PACKAGES.update(self.saved[0])
        reuse.SUITE_INPUTS.clear()
        reuse.SUITE_INPUTS.update(self.saved[1])

    def test_closure_follows_every_dependency_kind(self):
        repo = Repo(WORKSPACE)
        ws = reuse.Workspace(str(repo.root))
        self.assertEqual(ws.closure(["a"]), {"a", "b", "c"})
        self.assertEqual(ws.closure(["vaultwarden"]), {"vaultwarden", "b", "c"})
        with self.assertRaises(KeyError):
            ws.closure(["gone"])

    def test_inputs_and_fingerprints(self):
        repo = Repo(WORKSPACE)
        keys, extra, inputs = repo.keys(["sa", "sc", "server", "docs"])
        self.assertEqual(extra["sa"], ["design/x.json"])
        self.assertTrue(reuse.covered("crates/c/src/lib.rs", {"sa": inputs["sa"]}))
        self.assertTrue(reuse.covered("design/x.json", {"sa": inputs["sa"]}))
        self.assertFalse(reuse.covered("crates/a/src/lib.rs", {"sc": inputs["sc"]}))
        self.assertTrue(reuse.covered("src/main.rs", {"server": inputs["server"]}))
        self.assertFalse(reuse.covered("crates/a/src/lib.rs", {"server": inputs["server"]}))
        self.assertFalse(reuse.covered("docs/readme.md", {k: v for k, v in inputs.items() if k != "docs"}))
        for key in keys.values():
            self.assertTrue(key.startswith(f"{reuse.PASS_PREFIX}-{reuse.VERSION}-"))

        # A change to a dependency changes its dependents' keys only.
        write(repo.root, {"crates/c/src/lib.rs": "pub fn f() {}\n"})
        repo.commit()
        after, _, _ = repo.keys(["sa", "sc", "server", "docs"])
        self.assertNotEqual(keys["sa"], after["sa"])
        self.assertNotEqual(keys["sc"], after["sc"])
        self.assertNotEqual(keys["server"], after["server"])
        self.assertEqual(keys["docs"], after["docs"])

        # A file a source names with ../ is an input.
        write(repo.root, {"design/x.json": '{"a": 1}'})
        repo.commit()
        again, _, _ = repo.keys(["sa", "sc"])
        self.assertNotEqual(after["sa"], again["sa"])
        self.assertEqual(after["sc"], again["sc"])

        # The lockfile changes every Rust suite, not the others; CI's own files change everything.
        write(repo.root, {"Cargo.lock": "# changed\n"})
        repo.commit()
        last, _, _ = repo.keys(["sa", "sc", "server", "docs"])
        self.assertTrue(all(last[s] != again.get(s, after[s]) for s in ("sa", "sc")))
        self.assertEqual(last["docs"], after["docs"])
        write(repo.root, {"scripts/ci/plan.py": "# changed\n"})
        repo.commit()
        final, _, _ = repo.keys(["sa", "docs"])
        self.assertNotEqual(final["docs"], last["docs"])
        self.assertNotEqual(final["sa"], last["sa"])

    def test_parent_reaches(self):
        files = dict(WORKSPACE)
        files["crates/c/src/lib.rs"] = (
            'pub const FIXTURE: &str = "design/x.json";\n'
            'fn t() { std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(FIXTURE); }\n')
        repo = Repo(files)
        _, extra, inputs = repo.keys(["sc"])
        self.assertEqual(extra["sc"], ["design/x.json"])
        self.assertNotIn("**", inputs["sc"][0])

        # A reach the scan cannot follow: the whole repository.
        write(repo.root, {"crates/c/src/lib.rs": 'fn t() { Path::new(env!("CARGO_MANIFEST_DIR")).parent(); }\n'})
        repo.commit()
        _, extra, inputs = repo.keys(["sc"])
        self.assertIn("**", inputs["sc"][0])
        self.assertTrue(reuse.covered("docs/readme.md", inputs))

        # A full ../ literal joined is followed like include_str!.
        write(repo.root, {"crates/c/src/lib.rs":
                          'fn t() { Path::new(env!("CARGO_MANIFEST_DIR")).join("../../design/x.json"); }\n'})
        repo.commit()
        _, extra, inputs = repo.keys(["sc"])
        self.assertEqual(extra["sc"], ["design/x.json"])
        self.assertNotIn("**", inputs["sc"][0])

    def test_known_parent_reach_reads_its_packages(self):
        files = dict(WORKSPACE)
        files["crates/c/src/lib.rs"] = 'fn root() { Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."); }\n'
        repo = Repo(files)
        saved = dict(reuse.KNOWN_PARENT_READS)
        try:
            reuse.KNOWN_PARENT_READS.clear()
            reuse.KNOWN_PARENT_READS["crates/c/src/lib.rs"] = (1, [reuse.ROOT_PACKAGE])
            _, _, inputs = repo.keys(["sc"])
            self.assertNotIn("**", inputs["sc"][0])
            self.assertTrue(reuse.covered("src/main.rs", inputs))
            # One more reach than declared: the whole repository again.
            write(repo.root, {"crates/c/src/lib.rs": files["crates/c/src/lib.rs"] * 2})
            repo.commit()
            _, _, inputs = repo.keys(["sc"])
            self.assertIn("**", inputs["sc"][0])
        finally:
            reuse.KNOWN_PARENT_READS.clear()
            reuse.KNOWN_PARENT_READS.update(saved)

    def test_manifest_dir_uses(self):
        uses = reuse.manifest_dir_uses
        self.assertEqual(uses('Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/x.json")', "crates/c"),
                         ({"crates/c/testdata/x.json"}, 0))
        self.assertEqual(uses('concat!(env!("CARGO_MANIFEST_DIR"), "/../../design/x.json")', "crates/c"),
                         ({"design/x.json"}, 0))
        self.assertEqual(uses('const F: &str = "design/x.json";\n'
                              'PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").join(F)', "crates/c"),
                         ({"design/x.json"}, 0))
        # Anything else fails closed: a variable, a runtime lookup, a bare parent, a path outside the repository.
        for text in ('Path::new(env!("CARGO_MANIFEST_DIR")).join(name)',
                     'std::env::var("CARGO_MANIFEST_DIR").unwrap()',
                     'Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")',
                     'Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(UNKNOWN)',
                     'Path::new(env!("CARGO_MANIFEST_DIR")).parent()',
                     'Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../x")'):
            self.assertEqual(uses(text, "crates/c")[1], 1, text)

    def test_a_deleted_reference_still_selects(self):
        repo = Repo(WORKSPACE)
        (repo.root / "design/x.json").unlink()
        repo.commit()
        _, extra, inputs = repo.keys(["sa"])
        self.assertIn("design/x.json", extra["sa"])
        self.assertTrue(reuse.covered("design/x.json", inputs))

    def test_mode_and_variant_change_the_key(self):
        repo = Repo(WORKSPACE)
        before, _, _ = repo.keys(["sa"])
        os.chmod(repo.root / "crates/b/src/lib.rs", 0o755)
        repo.commit()
        after, _, _ = repo.keys(["sa"])
        self.assertNotEqual(before["sa"], after["sa"])
        index = reuse.tree_index(str(repo.root))
        ws = reuse.Workspace(str(repo.root))
        one, _, _ = reuse.suite_keys(["sa"], index, ws, {"sa": "xcode=27.0"})
        two, _, _ = reuse.suite_keys(["sa"], index, ws, {"sa": "xcode=27.1"})
        self.assertNotEqual(one["sa"], two["sa"])

    def test_all_but_takes_new_members(self):
        files = dict(WORKSPACE)
        files["Cargo.toml"] = files["Cargo.toml"].replace('"crates/c"]', '"crates/c", "crates/d"]')
        files["crates/d/Cargo.toml"] = '[package]\nname = "d"\n'
        files["crates/d/src/lib.rs"] = ""
        repo = Repo(files)
        ws = reuse.Workspace(str(repo.root))
        self.assertEqual(ws.packages(reuse.AllBut(["a"])), ["b", "c", "d", "vaultwarden"])
        with self.assertRaises(KeyError):
            ws.packages(reuse.AllBut(["gone"]))

    def test_passed_jobs_only_from_trusted_refs(self):
        caches = {"actions_caches": [
            {"key": "k-rust", "ref": "refs/heads/main"},
            {"key": "k-clippy", "ref": "refs/pull/9/merge"},
            {"key": "k-desktop", "ref": "refs/pull/7/merge"},
            {"key": "k2-rust", "ref": "refs/heads/main"},
        ]}
        api = lambda path: caches  # noqa: E731
        self.assertEqual(reuse.passed_jobs(api, "o/r", "k", {"refs/heads/main", "refs/pull/9/merge"}),
                         {"rust", "clippy"})
        self.assertEqual(reuse.passed_jobs(api, "o/r", "k", {"refs/heads/main"}), {"rust"})
        self.assertEqual(reuse.passed_jobs(api, "o/r", "k", set()), set())


class RepositoryTest(unittest.TestCase):
    """The real workspace: every suite's packages exist, and the scan follows every parent reach."""

    def test_this_repository(self):
        index = reuse.tree_index(str(ROOT))
        workspace = reuse.Workspace(str(ROOT))
        keys, extra, inputs = reuse.suite_keys(reuse.SUITES, index, workspace)
        self.assertEqual(set(keys), set(reuse.SUITES))
        for suite, (patterns, _) in inputs.items():
            self.assertNotIn("**", patterns, f"{suite} reads the whole repository: {extra.get(suite)}")
        # The desktop crates do not depend on reins-core; the mobile and server suites do.
        core = "crates/reins-core/src/lib.rs"
        for suite in ("rust", "clippy", "android", "ios"):
            self.assertTrue(reuse.covered(core, {suite: inputs[suite]}), suite)
        for suite in ("desktop-linux", "desktop-macos", "desktop-windows", "replay", "packages"):
            self.assertFalse(reuse.covered(core, {suite: inputs[suite]}), suite)
        self.assertTrue(reuse.covered("crates/reins-policy/src/lib.rs", {"desktop-macos": inputs["desktop-macos"]}))
        self.assertTrue(reuse.covered("src/main.rs", {"rust": inputs["rust"]}))
        self.assertFalse(reuse.covered("docs/README.md", inputs))
        # Every workspace member cargo knows is built by some suite (cargo metadata, when cargo is here: the checks
        # job installs the toolchain first).
        members = {p for p in workspace.dirs}
        if shutil.which("cargo"):
            metadata = json.loads(subprocess.run(
                ["cargo", "metadata", "--no-deps", "--format-version", "1", "--offline"], cwd=ROOT, check=True,
                capture_output=True, text=True).stdout)
            members = {p["name"] for p in metadata["packages"]}
            self.assertEqual(members, set(workspace.dirs))
        built = set()
        for suite, spec in reuse.SUITE_PACKAGES.items():
            built |= workspace.closure(workspace.packages(spec))
        self.assertEqual(members - built, set())
        for path, (count, packages) in reuse.KNOWN_PARENT_READS.items():
            text = (ROOT / path).read_text()
            top = next(d for d in workspace.dirs.values() if d != "." and path.startswith(d + "/"))
            self.assertEqual(reuse.manifest_dir_uses(text, top)[1], count, path)
            for package in packages:
                self.assertIn(package, workspace.dirs)


if __name__ == "__main__":
    unittest.main()
