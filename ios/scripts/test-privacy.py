#!/usr/bin/env python3
"""The privacy manifests the generator wires into each iOS target, and the one copied into ONNX Runtime's framework."""

import importlib.util
import plistlib
import unittest
from pathlib import Path

IOS = Path(__file__).resolve().parent.parent

# What each target's own code (and, for the app and the notification extension, the Rust core they link) actually uses.
# Reason codes are Apple's required-reason API codes. Collected-data types are only on the app: extensions and tests
# do not transmit account data themselves.
EXPECTED = {
    "Reins": {
        "apis": {
            "NSPrivacyAccessedAPICategoryFileTimestamp": {"C617.1"},
            "NSPrivacyAccessedAPICategoryUserDefaults": {"CA92.1", "1C8F.1"},
        },
        "data_types": {
            "NSPrivacyCollectedDataTypeName",
            "NSPrivacyCollectedDataTypeEmailAddress",
            "NSPrivacyCollectedDataTypePhoneNumber",
            "NSPrivacyCollectedDataTypeContacts",
            "NSPrivacyCollectedDataTypeEmailsOrTextMessages",
            "NSPrivacyCollectedDataTypeOtherUserContent",
            "NSPrivacyCollectedDataTypeUserID",
            "NSPrivacyCollectedDataTypeDeviceID",
        },
    },
    "ReinsWidgets": {
        "apis": {
            "NSPrivacyAccessedAPICategoryUserDefaults": {"CA92.1", "1C8F.1"},
        },
        "data_types": set(),
    },
    "ReinsNotifications": {
        "apis": {
            "NSPrivacyAccessedAPICategoryFileTimestamp": {"C617.1"},
            "NSPrivacyAccessedAPICategoryUserDefaults": {"CA92.1", "1C8F.1"},
        },
        "data_types": set(),
    },
    "ReinsTests": {
        "apis": {
            "NSPrivacyAccessedAPICategoryFileTimestamp": {"C617.1"},
            "NSPrivacyAccessedAPICategoryUserDefaults": {"CA92.1"},
        },
        "data_types": set(),
    },
    "ReinsUITests": {
        "apis": {},
        "data_types": set(),
    },
}


def load_gen():
    path = IOS / "scripts" / "gen-project.py"
    spec = importlib.util.spec_from_file_location("gen_project", path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def object_body(text: str, oid: str) -> str:
    token = f"\t\t{oid} = {{"
    start = text.find(token)
    if start < 0:
        raise AssertionError(f"missing object {oid}")
    depth = 0
    for i in range(start + len(token) - 1, len(text)):
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
            if depth == 0:
                return text[start : i + 1]
    raise AssertionError(f"unclosed object {oid}")


class PrivacyManifestTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.gen = load_gen()

    def test_each_target_manifest_matches_the_apis_it_uses(self):
        for name, spec in EXPECTED.items():
            path = IOS / "Privacy" / name / "PrivacyInfo.xcprivacy"
            self.assertTrue(path.is_file(), path)
            data = plistlib.loads(path.read_bytes())
            self.assertIs(data["NSPrivacyTracking"], False, name)
            self.assertEqual(data["NSPrivacyTrackingDomains"], [], name)
            found = {}
            for entry in data["NSPrivacyAccessedAPITypes"]:
                category = entry["NSPrivacyAccessedAPIType"]
                self.assertNotIn(category, found, name)
                found[category] = set(entry["NSPrivacyAccessedAPITypeReasons"])
            self.assertEqual(found, spec["apis"], name)
            types = {entry["NSPrivacyCollectedDataType"] for entry in data["NSPrivacyCollectedDataTypes"]}
            self.assertEqual(types, spec["data_types"], name)
            for entry in data["NSPrivacyCollectedDataTypes"]:
                self.assertIs(entry["NSPrivacyCollectedDataTypeLinked"], True, entry["NSPrivacyCollectedDataType"])
                self.assertIs(entry["NSPrivacyCollectedDataTypeTracking"], False, entry["NSPrivacyCollectedDataType"])
                self.assertEqual(
                    entry["NSPrivacyCollectedDataTypePurposes"],
                    ["NSPrivacyCollectedDataTypePurposeAppFunctionality"],
                    entry["NSPrivacyCollectedDataType"],
                )

    def test_onnx_manifest_declares_file_metadata_only(self):
        path = IOS / "Privacy" / "OnnxRuntime" / "PrivacyInfo.xcprivacy"
        self.assertTrue(path.is_file(), path)
        data = plistlib.loads(path.read_bytes())
        self.assertIs(data["NSPrivacyTracking"], False)
        self.assertEqual(data["NSPrivacyTrackingDomains"], [])
        self.assertEqual(data["NSPrivacyCollectedDataTypes"], [])
        self.assertEqual(
            [
                (entry["NSPrivacyAccessedAPIType"], entry["NSPrivacyAccessedAPITypeReasons"])
                for entry in data["NSPrivacyAccessedAPITypes"]
            ],
            [("NSPrivacyAccessedAPICategoryFileTimestamp", ["C617.1"])],
        )
        script = (IOS / "scripts" / "fix-embedded-frameworks.sh").read_text()
        self.assertIn("Privacy/OnnxRuntime/PrivacyInfo.xcprivacy", script)
        self.assertIn("onnxruntime.framework", script)
        self.assertIn('! -f "$fw/PrivacyInfo.xcprivacy"', script)

    def test_generator_puts_each_manifest_in_that_targets_resources(self):
        self.gen.main()
        pbx = (IOS / "Reins.xcodeproj" / "project.pbxproj").read_text()
        for name in EXPECTED:
            phase = object_body(pbx, self.gen.oid("phase", name, "Resources"))
            self.assertIn("isa = PBXResourcesBuildPhase", phase, name)
            build_file = self.gen.oid("buildfile", name, "privacy")
            self.assertIn(build_file, phase, name)
            body = object_body(pbx, build_file)
            file_ref = self.gen.oid("file", "privacy", name)
            # Ids that start with a digit are quoted by the project writer.
            self.assertTrue(
                f"fileRef = {file_ref}" in body or f'fileRef = "{file_ref}"' in body,
                body,
            )
            ref = object_body(pbx, file_ref)
            self.assertIn("path = PrivacyInfo.xcprivacy", ref, name)
            subgroup = object_body(pbx, self.gen.oid("group", "privacy", name))
            self.assertIn(f"path = {name}", subgroup, name)
            self.assertIn(file_ref, subgroup, name)
        onnx = self.gen.oid("file", "privacy", "OnnxRuntime")
        self.assertIn(onnx, pbx)
        self.assertNotIn(f"fileRef = {onnx}", pbx)
        self.assertNotIn(f'fileRef = "{onnx}"', pbx)
        privacy_group = object_body(pbx, self.gen.oid("group", "privacy"))
        self.assertIn("path = Privacy", privacy_group)
        self.assertIn(self.gen.oid("group", "privacy", "OnnxRuntime"), privacy_group)


if __name__ == "__main__":
    unittest.main()
