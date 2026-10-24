#!/usr/bin/env python3
"""Exercise the CI script's actual simulator selector without needing Xcode."""

import contextlib
import copy
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "ios/scripts/ci-xcode-test.sh"
SELECTOR = SCRIPT.read_text().split("python3 - <<'PY'\n", 1)[1].split("\nPY\n", 1)[0]
PREFIX = "com.apple.CoreSimulator.SimDeviceType."
MAX = PREFIX + "iPhone-18-Pro-Max"
OLDER = PREFIX + "iPhone-17-Pro-Max"
DUO = PREFIX + "iPhone-Duo"
IPAD = PREFIX + "iPad-Pro"


def runtime(version="27.0", supported=(MAX, OLDER), **extra):
    return {
        "name": "iOS " + version,
        "version": version,
        "identifier": "com.apple.CoreSimulator.SimRuntime.iOS-" + version.replace(".", "-"),
        "buildversion": "test-build",
        "isAvailable": True,
        "supportedDeviceTypes": [{"identifier": item} for item in supported],
    } | extra


def device(identifier):
    return {"identifier": identifier, "name": identifier.removeprefix(PREFIX).replace("-", " ")}


class SimulatorSelectionTests(unittest.TestCase):
    def select(self, runtimes, types=(MAX, OLDER, DUO, IPAD), sdk="27.0", error=None):
        def output(command, **kwargs):
            if command == ["xcrun", "--sdk", "iphonesimulator", "--show-sdk-version"]:
                return sdk + "\n"
            self.assertEqual(command[:3], ["xcrun", "simctl", "list"])
            self.assertEqual(command[4:], ["-j"])
            kind = command[3]
            return json.dumps({kind: runtimes if kind == "runtimes" else [device(item) for item in types]}).encode()

        with tempfile.TemporaryDirectory(prefix="reins-simulator-selection-") as directory:
            path = Path(directory) / "sim-choice.env"
            log = io.StringIO()
            with patch.dict(os.environ, {"RUNNER_TEMP": directory}), patch("subprocess.check_output", side_effect=output), contextlib.redirect_stdout(log):
                if error is not None:
                    with self.assertRaisesRegex(SystemExit, error):
                        exec(compile(SELECTOR, str(SCRIPT), "exec"), {})
                    self.assertFalse(path.exists(), "a failed selection must not leave a pair for simctl create")
                    return None
                exec(compile(SELECTOR, str(SCRIPT), "exec"), {})
            self.last_log = log.getvalue()
            return dict(line.split("=", 1) for line in path.read_text().splitlines())

    def test_newer_device_specific_runtime_does_not_replace_selected_sdk(self):
        normal, newer = runtime(), runtime("27.1", (DUO,))
        for runtimes in ([normal, newer], [newer, normal]):
            with self.subTest(runtimes=runtimes):
                self.assertEqual(self.select(runtimes), {"RUNTIME": normal["identifier"], "DEVTYPE": MAX})

    def test_logs_selected_sdk_runtime_build_and_device(self):
        self.select([runtime()])
        self.assertIn("SDK 27.0", self.last_log)
        self.assertIn("build test-build", self.last_log)
        self.assertIn("iPhone 18 Pro Max", self.last_log)

    def test_preferred_phone_must_be_supported_by_runtime(self):
        self.assertEqual(self.select([runtime(supported=(OLDER,))])["DEVTYPE"], OLDER)

    def test_phone_must_also_be_installed(self):
        self.assertEqual(self.select([runtime()], types=(OLDER,))["DEVTYPE"], OLDER)

    def test_same_version_variants_choose_best_compatible_phone_in_any_order(self):
        generic = runtime(identifier="com.apple.CoreSimulator.SimRuntime.iOS-27-0-generic")
        specific = runtime(supported=(DUO,), identifier="com.apple.CoreSimulator.SimRuntime.iOS-27-0-specific")
        for runtimes in ([generic, specific], [specific, generic]):
            with self.subTest(runtimes=runtimes):
                self.assertEqual(self.select(runtimes), {"RUNTIME": generic["identifier"], "DEVTYPE": MAX})

    def test_absent_or_unavailable_matching_runtime_fails(self):
        for runtimes in ([runtime("27.1", (DUO,))], [runtime(isAvailable=False)], []):
            with self.subTest(runtimes=runtimes):
                self.select(runtimes, error="no available iOS simulator runtime matching SDK 27.0")

    def test_missing_or_empty_compatibility_metadata_fails_closed(self):
        missing = runtime()
        del missing["supportedDeviceTypes"]
        for candidate in (missing, runtime(supported=())):
            with self.subTest(candidate=candidate):
                self.select([candidate], error="no compatible iPhone device type.*supportedDeviceTypes")

    def test_no_compatible_installed_phone_fails(self):
        self.select([runtime(supported=(MAX,))], types=(DUO, IPAD), error="no compatible iPhone device type")
        self.select([runtime(supported=(IPAD,))], error="no compatible iPhone device type")

    def test_non_ios_runtimes_are_excluded(self):
        for other in (runtime(name="tvOS 27.0"), runtime(platform="watchOS")):
            with self.subTest(other=other):
                self.select([other], error="no available iOS simulator runtime matching SDK")

    def test_equivalent_numeric_versions_match(self):
        for sdk, os_version in (("27.0", "27.0.0"), ("27.0.0", "27.0")):
            with self.subTest(sdk=sdk, os_version=os_version):
                self.assertEqual(self.select([runtime(os_version)], sdk=sdk)["DEVTYPE"], MAX)

    def test_supported_unlisted_iphone_is_a_fallback(self):
        self.assertEqual(self.select([runtime(supported=(DUO, IPAD))])["DEVTYPE"], DUO)

    def test_matching_runtime_without_metadata_does_not_hide_valid_pair(self):
        missing = copy.deepcopy(runtime())
        del missing["supportedDeviceTypes"]
        self.assertEqual(self.select([missing, runtime()])["DEVTYPE"], MAX)

    def test_build_and_empty_shard_never_inspect_or_create_simulators(self):
        with tempfile.TemporaryDirectory(prefix="reins-simulator-bypass-") as directory:
            root = Path(directory)
            bin_dir = root / "bin"
            bin_dir.mkdir()
            for name in ("xcrun", "python3"):
                stub = bin_dir / name
                stub.write_text("#!/bin/sh\necho unexpected simulator tool >&2\nexit 99\n")
                stub.chmod(0o755)
            build = bin_dir / "xcodebuild"
            build.write_text('#!/bin/sh\nprintf "%s\\n" "$@" > "$RUNNER_TEMP/build-args"\n')
            build.chmod(0o755)
            env = os.environ | {
                "PATH": str(bin_dir) + os.pathsep + os.environ["PATH"],
                "RUNNER_TEMP": directory,
                "GITHUB_WORKSPACE": str(root / "workspace"),
                "ONLY": "",
            }
            for mode in ("build", "test-without-building"):
                with self.subTest(mode=mode):
                    result = subprocess.run(["bash", str(SCRIPT)], env=env | {"MODE": mode}, text=True, capture_output=True)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertFalse((root / "sim-choice.env").exists())
            self.assertIn("build-for-testing", (root / "build-args").read_text())
            self.assertIn("generic/platform=iOS Simulator", (root / "build-args").read_text())


if __name__ == "__main__":
    unittest.main()
