#!/usr/bin/env python3
"""Type-checks the iOS targets on Linux, so Swift changes can be checked without a Mac.

Run through scripts/ios-typecheck.sh. Each target's Swift is type-checked with `swiftc` (Swift 6.4, swiftly) against the
iPhoneOS SDK of the Darwin Swift SDK that xtool installs (`xtool setup`, from an Xcode .xip), for arm64-apple-ios with
the project's deployment target and the compilation conditions Xcode would use. Nothing is linked or signed.

The targets, their folders, product types and build settings are read from ios/scripts/gen-project.py and
ios/Config/Reins.xcconfig, so a new target setting or condition reaches this check by itself. Besides the SDK, it needs:
- the Rust core's C header and module map (`import reins_coreFFI`): generated with uniffi-bindgen from a host build
  of crates/reins-core, the way build-core.sh does for iOS, and cached per Rust source;
- ONNX Runtime's headers (`import OnnxRuntimeC`): from the package's release archive pinned by Package.resolved, checked
  against the package's checksum, cached;
- the asset symbols Xcode generates from the asset catalogs (`ImageResource.foo`, `ColorResource.foo`).
"""

import argparse
import fnmatch
import hashlib
import json
import os
import re
import runpy
import shlex
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
IOS = ROOT / "ios"
HERE = Path(__file__).resolve().parent
CACHE = Path(
    os.environ.get("REINS_IOS_TYPECHECK_CACHE")
    or Path(os.environ.get("XDG_CACHE_HOME") or Path.home() / ".cache") / "reins-ios-typecheck"
)
BUNDLE = Path(
    os.environ.get("REINS_DARWIN_SDK") or Path.home() / ".config/swiftpm/swift-sdks/darwin.artifactbundle"
)
TRIPLE = "arm64-apple-ios"
PACKAGE_RESOLVED = IOS / "Reins.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved"
GENERATED_BINDINGS = IOS / "Core/Generated/reins_core.swift"


def die(message: str) -> None:
    print(f"ios-typecheck: {message}", file=sys.stderr)
    sys.exit(2)


def note(message: str) -> None:
    print(f"ios-typecheck: {message}", file=sys.stderr, flush=True)


# --- Toolchain ---------------------------------------------------------------------------------------------------


def find_swiftc() -> str:
    for candidate in (
        os.environ.get("SWIFTC"),
        Path.home() / ".local/share/swiftly/bin/swiftc",
        shutil.which("swiftc"),
    ):
        if candidate and Path(candidate).is_file():
            return str(candidate)
    die("swiftc not found: install Swift with swiftly (https://www.swift.org/install/linux/), or set SWIFTC")


def darwin_sdk() -> dict:
    """The iPhoneOS SDK, the Swift resource dir and the platform's developer dir, from the bundle's swift-sdk.json."""
    manifest = BUNDLE / "swift-sdk.json"
    if not manifest.is_file():
        die(f"no Darwin Swift SDK at {BUNDLE}: run `xtool setup` (it needs an Xcode .xip), or set REINS_DARWIN_SDK")
    spec = json.loads(manifest.read_text())["targetTriples"].get(TRIPLE)
    if not spec:
        die(f"{manifest} has no {TRIPLE} target")
    sdk = BUNDLE / spec["sdkRootPath"]
    platform = sdk.parent.parent  # .../iPhoneOS.platform/Developer
    extra = []
    for toolset in spec.get("toolsetPaths", []):
        extra += json.loads((BUNDLE / toolset).read_text()).get("swiftCompiler", {}).get("extraCLIOptions", [])
    # Linker options mean nothing to a type-check.
    extra = [o for o in extra if not o.startswith("-use-ld")]
    settings = json.loads((sdk / "SDKSettings.json").read_text())
    return {
        "sdk": sdk,
        "name": settings["CanonicalName"],  # iphoneos27.0
        "resources": BUNDLE / spec["swiftResourcesPath"],
        "platform": platform,
        "extra": extra,
    }


def macro_flags(sdk: dict) -> list[str]:
    """SwiftUI's and the SDK's macros (#Preview, @Entry, ...). The platform's plugins folder has empty stand-ins named
    after each macro module; the bundle's OpenAppleMacrosServer, an external plugin server, implements them. The
    toolchain's own macros (@Observable, #warning ...) come with swiftly's Swift."""
    server = BUNDLE / "OpenAppleMacrosServer"
    plugins = sdk["platform"] / "usr/lib/swift/host/plugins"
    if not server.is_file() or not plugins.is_dir():
        note("warning: the Darwin SDK has no OpenAppleMacrosServer; SwiftUI's macros (#Preview) will not expand")
        return []
    return ["-Xfrontend", "-external-plugin-path", "-Xfrontend", f"{plugins}#{server}"]


# --- Build settings, as Xcode resolves them ----------------------------------------------------------------------


def parse_xcconfig(path: Path, layer: dict) -> None:
    for raw in path.read_text().splitlines():
        line = raw.split("//", 1)[0].strip()
        include = re.match(r'#include(\?)?\s+"([^"]+)"', raw.strip())
        if include:
            target = path.parent / include.group(2)
            if target.is_file():
                parse_xcconfig(target, layer)
            elif not include.group(1):
                die(f"{path}: missing include {target}")
            continue
        assignment = re.match(r"([A-Za-z0-9_]+(?:\[[^\]]*\])*)\s*=\s*(.*)$", line)
        if assignment:
            layer[assignment.group(1)] = assignment.group(2).strip()


class Settings:
    """Xcode's lookup of one target's build settings: target settings over the target's xcconfig over the project's
    settings over the project's xcconfig, `$(inherited)` reaching one level down, `[sdk=...]`/`[config=...]` variants
    winning over the plain key."""

    def __init__(self, layers: list[dict], sdk_name: str, config: str, srcroot: Path):
        self.layers = layers  # lowest first
        self.sdk_name = sdk_name
        self.config = config
        self.builtins = {"SRCROOT": str(srcroot), "PROJECT_DIR": str(srcroot), "CONFIGURATION": config}

    def _from_layer(self, layer: dict, name: str):
        best, best_conditions = None, -1
        for key, value in layer.items():
            m = re.fullmatch(rf"{re.escape(name)}((?:\[[a-z]+=[^\]]*\])*)", key)
            if not m:
                continue
            conditions = re.findall(r"\[([a-z]+)=([^\]]*)\]", m.group(1))
            ok = all(
                fnmatch.fnmatch({"sdk": self.sdk_name, "config": self.config, "arch": "arm64"}.get(k, ""), pattern)
                for k, pattern in conditions
            )
            if ok and len(conditions) > best_conditions:
                best, best_conditions = value, len(conditions)
        if isinstance(best, list):
            best = " ".join(best)
        return best

    def get(self, name: str, depth: int = 0) -> str:
        if name in self.builtins:
            return self.builtins[name]
        if depth > 20:
            die(f"build setting {name} refers to itself")
        value = ""
        for layer in self.layers:
            v = self._from_layer(layer, name)
            if v is not None:
                value = str(v).replace("$(inherited)", value)
        return re.sub(r"\$\(([A-Za-z0-9_]+)\)", lambda m: self.get(m.group(1), depth + 1), value)

    def words(self, name: str) -> list[str]:
        out = []
        for word in self.get(name).split():
            if word not in out:
                out.append(word)
        return out


def project_description() -> dict:
    # The generator's module-level tables; main() only runs as a script.
    return runpy.run_path(str(IOS / "scripts/gen-project.py"), run_name="gen_project")


def target_settings(project: dict, target: dict, sdk_name: str, config: str) -> Settings:
    xcconfig: dict = {}
    parse_xcconfig(IOS / "Config/Reins.xcconfig", xcconfig)
    project_layer = dict(project["COMMON"])
    project_layer.update(project["DEBUG"] if config == "Debug" else project["RELEASE"])
    target_layer = dict(target["settings"])
    if target.get("core"):
        target_layer.update(project["CORE_LINK"])
    for key in [k for k in target_layer if k.startswith("SWIFT_INCLUDE_PATHS")]:
        target_layer[key] = " ".join([target_layer[key], *target.get("swift_includes", [])])
    if config == "Debug" and "SWIFT_ACTIVE_COMPILATION_CONDITIONS" in target_layer:
        # As gen-project.py writes the target's Debug configuration.
        target_layer["SWIFT_ACTIVE_COMPILATION_CONDITIONS"] = target_layer["SWIFT_ACTIVE_COMPILATION_CONDITIONS"].replace(
            "$(inherited)", "DEBUG $(inherited)"
        )
    return Settings([xcconfig, project_layer, xcconfig, target_layer], sdk_name, config, IOS)


# --- The Rust core's FFI header ----------------------------------------------------------------------------------


def path_crates(crate: Path, seen: set) -> None:
    if crate in seen:
        return
    seen.add(crate)
    for dep in re.findall(r'path\s*=\s*"([^"]+)"', (crate / "Cargo.toml").read_text()):
        dep_dir = (crate / dep).resolve()
        if (dep_dir / "Cargo.toml").is_file():
            path_crates(dep_dir, seen)


def core_key() -> str:
    crates: set = set()
    path_crates(ROOT / "crates/reins-core", crates)
    digest = hashlib.sha256()
    files = [ROOT / "Cargo.toml", ROOT / "Cargo.lock", ROOT / "rust-toolchain.toml"]
    for crate in sorted(crates):
        for dirpath, dirnames, filenames in os.walk(crate):
            dirnames[:] = sorted(d for d in dirnames if d not in ("target", "tests", "benches"))
            files += [Path(dirpath) / f for f in sorted(filenames)]
    for f in files:
        if f.is_file():
            digest.update(str(f.relative_to(ROOT)).encode() + b"\0" + f.read_bytes() + b"\0")
    return digest.hexdigest()[:20]


def core_bindings() -> Path:
    """include/{module.modulemap,reins_coreFFI.h} and reins_core.swift for the current Rust source. uniffi's output
    does not depend on the target, so a host (Linux) build of the core gives the header the iOS build would."""
    out = CACHE / "core" / core_key()
    if (out / "include/module.modulemap").is_file():
        return out
    cargo = Path.home() / ".cargo/bin/cargo"  # rustup's, honouring rust-toolchain.toml
    if not cargo.is_file():
        cargo = Path(shutil.which("cargo") or "")
    if not cargo.is_file():
        die("cargo not found: the Rust core's bindings are generated from a host build of crates/reins-core")
    note("generating the Rust core's Swift bindings (a host build of crates/reins-core; cached per Rust source)")
    env = {
        "HOME": str(Path.home()),
        "USER": os.environ.get("USER", ""),
        "TERM": "dumb",
        "PATH": f"{Path.home()}/.cargo/bin:/usr/local/bin:/usr/bin:/bin",
        "CARGO_TARGET_DIR": str(CACHE / "cargo-target"),
        # uniffi reads its metadata from the library's symbols; debug info only costs time and disk.
        "CARGO_PROFILE_DEV_DEBUG": "0",
    }
    common = ["--locked", "-q", "-p", "reins-core", "--features", "bindgen"]
    started = time.monotonic()
    for args in (["build", *common, "--lib"], ["build", *common, "--bin", "uniffi-bindgen"]):
        r = subprocess.run([str(cargo), *args], cwd=ROOT, env=env)
        if r.returncode != 0:
            die(f"cargo {' '.join(args)} failed")
    lib = CACHE / "cargo-target/debug/libreins_core.so"
    bindgen = CACHE / "cargo-target/debug/uniffi-bindgen"
    with tempfile.TemporaryDirectory(dir=CACHE) as tmp:
        gen = Path(tmp) / "gen"
        r = subprocess.run(
            [str(bindgen), "generate", "--library", str(lib), "--language", "swift", "--out-dir", str(gen)],
            cwd=ROOT,
            env=env,
            stdout=subprocess.DEVNULL,
        )
        if r.returncode != 0:
            die("uniffi-bindgen failed")
        staged = Path(tmp) / "out"
        (staged / "include").mkdir(parents=True)
        shutil.copy(gen / "reins_coreFFI.h", staged / "include/reins_coreFFI.h")
        shutil.copy(gen / "reins_coreFFI.modulemap", staged / "include/module.modulemap")
        shutil.copy(gen / "reins_core.swift", staged / "reins_core.swift")
        out.parent.mkdir(parents=True, exist_ok=True)
        if not out.exists():
            staged.rename(out)
    note(f"bindings ready in {time.monotonic() - started:.0f}s")
    return out


# --- Swift packages ----------------------------------------------------------------------------------------------


def fetch(url: str) -> bytes:
    with urllib.request.urlopen(url, timeout=120) as r:
        return r.read()


def onnxruntime_framework_dir(pin: dict) -> Path:
    """A folder with onnxruntime.framework/Headers for -F: the package's iOS framework without its binary.
    The archive URL and checksum are the binary target's in the package manifest at the pinned revision."""
    version, revision = pin["state"].get("version", "?"), pin["state"]["revision"]
    dest = CACHE / f"onnxruntime-{version}-{revision[:12]}"
    framework = dest / "onnxruntime.framework"
    if (framework / "Headers/onnxruntime_c_api.h").is_file():
        return dest
    repo = pin["location"].removesuffix(".git").replace("https://github.com/", "")
    note(f"fetching ONNX Runtime {version}'s headers (once)")
    manifest = fetch(f"https://raw.githubusercontent.com/{repo}/{revision}/Package.swift").decode()
    m = re.search(
        r'binaryTarget\(\s*name:\s*"onnxruntime",\s*url:\s*"([^"]+)",\s*(?://[^\n]*\s*)*checksum:\s*"([0-9a-f]{64})"',
        manifest,
    )
    if not m:
        die(f"no onnxruntime binary target in {repo}@{revision}'s Package.swift")
    url, checksum = m.groups()
    with tempfile.TemporaryDirectory(dir=CACHE) as tmp:
        archive = Path(tmp) / "archive.zip"
        archive.write_bytes(fetch(url))
        actual = hashlib.sha256(archive.read_bytes()).hexdigest()
        if actual != checksum:
            die(f"{url}: checksum {actual}, the package pins {checksum}")
        prefix = "onnxruntime.xcframework/ios-arm64/onnxruntime.framework/"
        staged = Path(tmp) / "out/onnxruntime.framework"
        with zipfile.ZipFile(archive) as z:
            for name in z.namelist():
                rel = name[len(prefix):] if name.startswith(prefix) else None
                if not rel or name.endswith("/") or ".." in rel.split("/"):
                    continue
                if not (rel.startswith("Headers/") or rel.startswith("Modules/") or rel == "Info.plist"):
                    continue  # the 40 MB library itself is not needed
                target = staged / rel
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(z.read(name))
        if not (staged / "Headers/onnxruntime_c_api.h").is_file():
            die(f"{url}: no iOS framework headers in the archive")
        dest.parent.mkdir(parents=True, exist_ok=True)
        if not dest.exists():
            (Path(tmp) / "out").rename(dest)
    return dest


def package_framework_dirs() -> list[Path]:
    """-F folders standing in for the products folder, where Xcode puts the packages' frameworks."""
    if not PACKAGE_RESOLVED.is_file():
        return []
    dirs = []
    for pin in json.loads(PACKAGE_RESOLVED.read_text())["pins"]:
        if pin["identity"] == "onnxruntime-swift-package-manager":
            dirs.append(onnxruntime_framework_dir(pin))
        else:
            note(f"warning: no headers for package {pin['identity']}; teach scripts/ios-typecheck/typecheck.py")
    return dirs


# --- Asset symbols -----------------------------------------------------------------------------------------------

SWIFT_KEYWORDS = set(
    "associatedtype class deinit enum extension fileprivate func import init inout internal let open operator "
    "private precedencegroup protocol public rethrows static struct subscript typealias var break case catch continue "
    "default defer do else fallthrough for guard if in repeat return throw switch where while Any as await false is "
    "nil self Self super throws true try".split()
)


def symbol_name(asset: str, suffix: str) -> str:
    """Xcode's naming: lower camel case from the asset's words, without a trailing "Color"/"Image"."""
    words = re.findall(r"[A-Za-z0-9]+", asset)
    if not words:
        return ""
    first = words[0]
    lead = re.match(r"[A-Z]+(?=[A-Z][a-z]|$|[0-9])", first)
    first = first[: lead.end()].lower() + first[lead.end():] if lead else first[0].lower() + first[1:]
    name = first + "".join(w[0].upper() + w[1:] for w in words[1:])
    if name.lower().endswith(suffix.lower()) and len(name) > len(suffix):
        name = name[: -len(suffix)]
    if name[0].isdigit():
        name = "_" + name
    return f"`{name}`" if name in SWIFT_KEYWORDS else name


def asset_symbols(folders: list[Path]) -> str:
    """GeneratedAssetSymbols.swift as Xcode writes it (ASSETCATALOG_COMPILER_GENERATE_ASSET_SYMBOLS, on by default;
    the Color/Image extensions of ..._SWIFT_ASSET_SYMBOL_EXTENSIONS are off in this project)."""
    colors, images = {}, {}
    for folder in folders:
        for catalog in sorted(folder.rglob("*.xcassets")):
            for dirpath, dirnames, _ in os.walk(catalog):
                dirnames.sort()
                here = Path(dirpath)
                for d in dirnames:
                    kind = d.rsplit(".", 1)[-1]
                    if kind not in ("colorset", "imageset"):
                        continue
                    # Folders that "provide namespace" prefix the asset's name.
                    namespace = []
                    p = here
                    while p != catalog:
                        contents = p / "Contents.json"
                        if contents.is_file() and json.loads(contents.read_text()).get("properties", {}).get(
                            "provides-namespace"
                        ):
                            namespace.insert(0, p.name)
                        p = p.parent
                    asset = "/".join([*namespace, d.rsplit(".", 1)[0]])
                    table = colors if kind == "colorset" else images
                    table.setdefault(symbol_name(asset, "Color" if kind == "colorset" else "Image"), asset)
    lines = [
        "// Generated by scripts/ios-typecheck: the asset symbols Xcode generates from this target's asset catalogs.",
        "import Foundation",
        "#if canImport(DeveloperToolsSupport)",
        "import DeveloperToolsSupport",
        "#endif",
        "private class ResourceBundleClass {}",
        "private let resourceBundle = Foundation.Bundle(for: ResourceBundleClass.self)",
    ]
    for kind, table in (("Color", colors), ("Image", images)):
        lines += [
            "@available(iOS 17.0, macOS 14.0, tvOS 17.0, watchOS 10.0, *)",
            f"extension DeveloperToolsSupport.{kind}Resource {{",
        ]
        for name, asset in table.items():
            if name:
                lines.append(
                    f'    static let {name} = DeveloperToolsSupport.{kind}Resource(name: "{asset}", bundle: resourceBundle)'
                )
        lines.append("}")
    return "\n".join(lines) + "\n"


# --- Type-checking -----------------------------------------------------------------------------------------------


def sources(target: dict) -> list[Path]:
    files = []
    for folder in target["folders"]:
        excluded = set(target.get("plist_exceptions", {}).get(folder, []))
        for f in sorted((IOS / folder).rglob("*.swift")):
            if str(f.relative_to(IOS / folder)) not in excluded:
                files.append(f)
    return files


DIAGNOSTIC = re.compile(r"^\S.*?:\d+:\d+: (?:error|warning): ")


def dedupe(output: str) -> str:
    """Each batch of files reports a diagnostic about an import (or a file outside the batch) again; keep the first."""
    seen, kept, keep = set(), [], True
    for line in output.splitlines(keepends=True):
        if DIAGNOSTIC.match(line):
            keep = line not in seen
            seen.add(line)
        if keep:
            kept.append(line)
    return "".join(kept)


def main() -> int:
    parser = argparse.ArgumentParser(
        prog="scripts/ios-typecheck.sh",
        description="Type-check the iOS targets against the iPhoneOS SDK, on Linux (see ios/README.md).",
    )
    parser.add_argument("targets", nargs="*", help="Reins ReinsWidgets ReinsNotifications ReinsTests ReinsUITests")
    parser.add_argument("--release", action="store_true", help="Release's conditions (no DEBUG) instead of Debug's")
    parser.add_argument("--warnings", action="store_true", help="show the compiler's warnings too")
    parser.add_argument("-v", "--verbose", action="store_true", help="print the swiftc command lines")
    args = parser.parse_args()

    project = project_description()
    by_name = {t["name"]: t for t in project["TARGETS"]}
    aliases = {"app": "Reins", "widgets": "ReinsWidgets", "notifications": "ReinsNotifications",
               "tests": "ReinsTests", "uitests": "ReinsUITests"}
    wanted = [aliases.get(t.lower(), t) for t in args.targets] or list(by_name)
    for name in wanted:
        if name not in by_name:
            die(f"unknown target {name}; targets: {' '.join(by_name)}")
    if args.release:
        # The scheme builds tests in Debug only (`@testable import` needs a testable app module).
        tests = [n for n in wanted if by_name[n]["type"].startswith("com.apple.product-type.bundle.")]
        if args.targets and tests:
            die(f"{' '.join(tests)}: test bundles build in Debug only; drop --release")
        wanted = [n for n in wanted if n not in tests]
    # Hosted unit tests import the app's module, so the app goes first and emits it.
    hosts = {by_name[n]["host"] for n in wanted if by_name[n].get("host") and not by_name[n].get("ui_tests")}
    order = [n for n in by_name if n in wanted or n in hosts]

    swiftc = find_swiftc()
    sdk = darwin_sdk()
    config = "Release" if args.release else "Debug"
    CACHE.mkdir(parents=True, exist_ok=True)
    build = CACHE / "build" / hashlib.sha256(str(ROOT).encode()).hexdigest()[:12] / config
    build.mkdir(parents=True, exist_ok=True)

    needs_core = any(
        by_name[n].get("core") or "ios-core" in str(by_name[n]["settings"].get("SWIFT_INCLUDE_PATHS[sdk=iphoneos*]", ""))
        for n in order
    )
    core = core_bindings() if needs_core else None
    frameworks = package_framework_dirs()
    if core and GENERATED_BINDINGS.is_file() and GENERATED_BINDINGS.read_bytes() != (core / "reins_core.swift").read_bytes():
        note(
            "note: ios/Core/Generated/reins_core.swift is not what the Rust core generates now; checking with the "
            "fresh bindings, as Xcode's build phase would (ios/scripts/build-core.sh refreshes the committed file)"
        )

    common = [
        swiftc,
        "-target", f"{TRIPLE}{project['DEPLOYMENT']}",
        "-sdk", str(sdk["sdk"]),
        "-resource-dir", str(sdk["resources"]),
        "-module-cache-path", str(CACHE / "module-cache"),
        "-enforce-exclusivity=checked",
        # Batches of files over every core, as Xcode does; swiftc on Linux otherwise runs one file at a time.
        f"-j{os.cpu_count() or 4}",
        "-enable-batch-mode",
        *sdk["extra"],
        *macro_flags(sdk),
    ]
    for d in frameworks:
        common += ["-F", str(d)]
    if not args.warnings:
        common.append("-suppress-warnings")

    failed = False
    results = {}
    for name in order:
        target = by_name[name]
        settings = target_settings(project, target, sdk["name"], config)
        files = sources(target)
        if core:
            files = [core / "reins_core.swift" if f == GENERATED_BINDINGS else f for f in files]
        symbols = build / f"{name}-GeneratedAssetSymbols.swift"
        symbols.write_text(asset_symbols([IOS / f for f in target["folders"]]))
        files.append(symbols)

        cmd = [*common, "-module-name", settings.get("PRODUCT_MODULE_NAME") or name,
               "-swift-version", settings.get("SWIFT_VERSION").split(".")[0] or "5",
               settings.get("SWIFT_OPTIMIZATION_LEVEL") or "-Onone"]
        cmd += [f"-D{c}" for c in settings.words("SWIFT_ACTIVE_COMPILATION_CONDITIONS")]
        cmd += [x for d in settings.words("GCC_PREPROCESSOR_DEFINITIONS") for x in ("-Xcc", f"-D{d}")]
        for inc in settings.words("SWIFT_INCLUDE_PATHS"):
            if "/target/ios-core/" in inc:
                inc = str(core / "include")
            cmd += ["-I", inc]
        if target["type"] == "com.apple.product-type.app-extension":
            cmd.append("-application-extension")  # APPLICATION_EXTENSION_API_ONLY
        if target["type"].startswith("com.apple.product-type.bundle."):
            # XCTest and Swift Testing, as Xcode adds for test bundles.
            cmd += ["-F", str(sdk["platform"] / "Library/Frameworks"), "-I", str(sdk["platform"] / "usr/lib")]
        host = target.get("host") if not target.get("ui_tests") else None
        if host:
            if results.get(host) != "ok":
                print(f"skip  {name}: {host} did not type-check")
                failed = True
                continue
            cmd += ["-I", str(build)]
        if settings.get("ENABLE_TESTABILITY") == "YES":
            cmd.append("-enable-testing")
        emits = name in hosts
        if emits:
            # Emitting the module type-checks every function body too; tests then `@testable import` it.
            cmd += ["-emit-module", "-emit-module-path", str(build / f"{name}.swiftmodule")]
        else:
            cmd.append("-typecheck")
        cmd += shlex.split(os.environ.get("IOS_TYPECHECK_SWIFTFLAGS", ""))
        cmd += [str(f) for f in files]

        if args.verbose:
            print(" ".join(cmd), file=sys.stderr)
        # Batched -emit-module leaves partial modules in the working directory.
        work = build / f"{name}.work"
        shutil.rmtree(work, ignore_errors=True)
        work.mkdir()
        started = time.monotonic()
        r = subprocess.run(cmd, cwd=work, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        elapsed = time.monotonic() - started
        output = dedupe(r.stdout)
        errors = len(re.findall(r"^\S.*?: error: ", output, re.M))
        warnings = len(re.findall(r"^\S.*?: warning: ", output, re.M))
        if r.returncode == 0:
            results[name] = "ok"
            if args.warnings and output.strip():
                print(output, end="" if output.endswith("\n") else "\n")
            extra = f", {warnings} warnings" if args.warnings else ""
            print(f"ok    {name} ({len(files)} files{extra}) {elapsed:.1f}s", flush=True)
        else:
            results[name] = "failed"
            failed = True
            print(output, end="" if output.endswith("\n") else "\n")
            print(f"FAIL  {name} ({errors or 'compiler'} errors) {elapsed:.1f}s", flush=True)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
