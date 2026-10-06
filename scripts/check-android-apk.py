#!/usr/bin/env python3
"""Check the actual packaged Android identity, label and release version."""

import argparse
import re
import subprocess


def validate_badging(badging, version, version_code):
    lines = badging.splitlines()
    package = next((line for line in lines if line.startswith("package:")), "")
    attributes = dict(re.findall(r"(\w+)='([^']*)'", package))
    expected = {"name": "com.reins2fa.app", "versionName": version, "versionCode": str(version_code)}
    for name, value in expected.items():
        if attributes.get(name) != value:
            raise ValueError(f"APK {name} must be {value!r}, got {attributes.get(name)!r}")
    labels = [line.split(":", 1)[1] for line in lines if re.match(r"application-label(?:-[^:]+)?:", line)]
    if not labels or any(label != "'Reins'" for label in labels):
        raise ValueError("Every installed application label must be Reins")
    if "application-debuggable" in lines:
        raise ValueError("Release APK must not be debuggable")
    if re.search(r"re[w]arden", badging, re.IGNORECASE):
        raise ValueError("APK metadata contains obsolete branding")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("apk")
    parser.add_argument("--aapt2", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--version-code", required=True, type=int)
    args = parser.parse_args()
    badging = subprocess.check_output([args.aapt2, "dump", "badging", args.apk], text=True)
    validate_badging(badging, args.version, args.version_code)
    print(f"Verified Reins ({args.version}, {args.version_code}), com.reins2fa.app, non-debuggable")


if __name__ == "__main__":
    main()
