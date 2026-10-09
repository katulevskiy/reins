#!/usr/bin/env python3
"""Check the actual packaged Android identity, label and release version.

    check-android-apk.py reins.apk --aapt2 AAPT2 --version 0.3.0 --version-code N
    check-android-apk.py reins-play.aab --aapt2 AAPT2 --version 0.3.0 --version-code N \
        --bundletool bundletool.jar --cert-sha256 "$(cat android/release-signing.sha256)"

An app bundle (the Google Play build) is also checked for its upload signature (one signer, the pinned
certificate), validated by bundletool, turned into a universal APK, and held to Google Play's rules: none of the
permissions Play restricts, only the declared foreground-service type, no cleartext traffic, a current targetSdk and
16 KB-aligned native code.
"""

import argparse
import re
import subprocess
import tempfile
import zipfile
from pathlib import Path

# Google Play: new apps and updates must target Android 16 (API 36) from 2026-08-31.
PLAY_MIN_TARGET_SDK = 36
# Permissions Google Play does not allow this app (SMS/Call Log, self-updating, and the ones that need a declaration
# this app could not make). The `full` APK from reins2fa.com has the first three; the `play` build must not.
PLAY_RESTRICTED_PERMISSIONS = {
    "android.permission.READ_SMS",
    "android.permission.SEND_SMS",
    "android.permission.RECEIVE_SMS",
    "android.permission.RECEIVE_MMS",
    "android.permission.RECEIVE_WAP_PUSH",
    "android.permission.READ_CALL_LOG",
    "android.permission.WRITE_CALL_LOG",
    "android.permission.PROCESS_OUTGOING_CALLS",
    "android.permission.REQUEST_INSTALL_PACKAGES",
    "android.permission.QUERY_ALL_PACKAGES",
    "android.permission.MANAGE_EXTERNAL_STORAGE",
    "android.permission.ACCESS_BACKGROUND_LOCATION",
    "android.permission.SCHEDULE_EXACT_ALARM",
    "android.permission.USE_EXACT_ALARM",
    "android.permission.USE_FULL_SCREEN_INTENT",
    "com.google.android.gms.permission.AD_ID",
}
# The foreground-service types declared in the Play Console (android/PLAY_STORE.md): the model download only.
PLAY_FOREGROUND_SERVICE_PERMISSIONS = {
    "android.permission.FOREGROUND_SERVICE",
    "android.permission.FOREGROUND_SERVICE_DATA_SYNC",
}
DATA_SYNC = 0x1  # ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC


def validate_badging(badging, version, version_code):
    lines = badging.splitlines()
    package = next((line for line in lines if line.startswith("package:")), "")
    attributes = dict(re.findall(r"(\w+)='([^']*)'", package))
    expected = {
        "name": "com.reins2fa.app",
        "versionName": version,
        "versionCode": str(version_code),
    }
    for name, value in expected.items():
        if attributes.get(name) != value:
            raise ValueError(
                f"APK {name} must be {value!r}, got {attributes.get(name)!r}"
            )
    labels = [
        line.split(":", 1)[1]
        for line in lines
        if re.match(r"application-label(?:-[^:]+)?:", line)
    ]
    if not labels or any(label != "'Reins'" for label in labels):
        raise ValueError("Every installed application label must be Reins")
    if "application-debuggable" in lines:
        raise ValueError("Release APK must not be debuggable")
    if re.search(r"re[w]arden", badging, re.IGNORECASE):
        raise ValueError("APK metadata contains obsolete branding")


def validate_play_badging(badging, min_target_sdk=PLAY_MIN_TARGET_SDK):
    permissions = set(re.findall(r"^uses-permission(?:-sdk-23)?: name='([^']+)'", badging, re.M))
    restricted = sorted(permissions & PLAY_RESTRICTED_PERMISSIONS)
    if restricted:
        raise ValueError(f"The Play build requests permissions Google Play restricts: {', '.join(restricted)}")
    services = sorted(
        p for p in permissions if p.startswith("android.permission.FOREGROUND_SERVICE")
    )
    undeclared = [p for p in services if p not in PLAY_FOREGROUND_SERVICE_PERMISSIONS]
    if undeclared:
        raise ValueError(
            f"Foreground-service types not declared in the Play Console: {', '.join(undeclared)} "
            "(declare them there and in android/PLAY_STORE.md, then allow them here)"
        )
    target = re.search(r"^targetSdkVersion:'(\d+)'", badging, re.M)
    if not target or int(target.group(1)) < min_target_sdk:
        found = target.group(1) if target else "none"
        raise ValueError(f"Google Play requires targetSdk {min_target_sdk} or later, the bundle has {found}")


def validate_play_manifest(xmltree):
    """`aapt2 dump xmltree --file AndroidManifest.xml` of the Play build."""
    cleartext = re.search(r":usesCleartextTraffic\([^)]*\)=(\S+)", xmltree)
    if cleartext and cleartext.group(1) not in ("false", "0x0", "0x00000000"):
        raise ValueError("The Play build allows cleartext (http) traffic")
    for value in re.findall(r":foregroundServiceType\([^)]*\)=0x([0-9a-fA-F]+)", xmltree):
        if int(value, 16) & ~DATA_SYNC:
            raise ValueError(f"A service has foreground-service types other than dataSync (0x{value})")


def validate_native_libraries(apk):
    # Check the packaged libraries, including their actual ELF architecture. Parallel build
    # artifacts must not accidentally produce a one-ABI APK or put the wrong library in an ABI folder.
    with zipfile.ZipFile(apk) as archive:
        for abi, machine in (("arm64-v8a", 183), ("x86_64", 62)):
            name = f"lib/{abi}/libreins_core.so"
            try:
                with archive.open(name) as library:
                    header = library.read(20)
            except KeyError as error:
                raise ValueError(f"APK is missing {name}") from error
            if (
                len(header) != 20
                or header[:6] != b"\x7fELF\x02\x01"
                or int.from_bytes(header[18:20], "little") != machine
            ):
                raise ValueError(f"APK has an invalid native library for {abi}")


def validate_page_alignment(apk, page=16384):
    """Every 64-bit native library's loadable segments are aligned for 16 KB pages (Google Play requires it)."""
    with zipfile.ZipFile(apk) as archive:
        for name in archive.namelist():
            if not re.match(r"lib/(arm64-v8a|x86_64)/[^/]+\.so$", name):
                continue
            with archive.open(name) as library:
                header = library.read(64)
                if header[:6] != b"\x7fELF\x02\x01":
                    raise ValueError(f"{name} is not a 64-bit little-endian ELF library")
                offset = int.from_bytes(header[0x20:0x28], "little")
                size = int.from_bytes(header[0x36:0x38], "little")
                count = int.from_bytes(header[0x38:0x3A], "little")
                if offset < 64 or size < 0x38:
                    raise ValueError(f"{name} has an invalid ELF program header table")
                library.read(offset - 64)
                table = library.read(size * count)
            for index in range(count):
                entry = table[index * size:(index + 1) * size]
                if int.from_bytes(entry[0:4], "little") == 1:  # PT_LOAD
                    align = int.from_bytes(entry[0x30:0x38], "little")
                    if align < page:
                        raise ValueError(f"{name} is aligned for {align}-byte pages, not 16 KB")


def bundle_signer_sha256(printcert):
    """The SHA-256 of the one signer's certificate, from `keytool -printcert -jarfile`."""
    signers = re.findall(r"^Signer #\d+:", printcert, re.M)
    if len(signers) != 1:
        raise ValueError(f"The bundle must have exactly one signer, it has {len(signers)}")
    digests = re.findall(r"SHA256: ([0-9A-Fa-f:]{95})", printcert)
    if not digests:
        raise ValueError("The bundle is not signed")
    return digests[0].replace(":", "").lower()


def validate_jarsigner(output):
    """`jarsigner -verify`: signed, and every entry covered (self-signed certificates are normal for Android)."""
    if "jar verified." not in output or "unsigned entries" in output or "jar is unsigned" in output:
        raise ValueError("The bundle's signature does not verify, or it has unsigned entries")


def check_bundle(aab, args):
    printcert = subprocess.run(
        ["keytool", "-printcert", "-jarfile", aab], capture_output=True, text=True, check=False
    ).stdout
    actual = bundle_signer_sha256(printcert)
    if actual != args.cert_sha256.strip().lower():
        raise ValueError(f"The bundle is signed by {actual}, not the upload certificate {args.cert_sha256}")
    verify = subprocess.run(["jarsigner", "-verify", aab], capture_output=True, text=True, check=False)
    validate_jarsigner(verify.stdout + verify.stderr)
    bundletool = ["java", "-jar", args.bundletool]
    subprocess.run(bundletool + ["validate", "--bundle", aab], check=True, capture_output=True)
    with tempfile.TemporaryDirectory(prefix="reins-aab-") as directory:
        # A throwaway key signs the universal APK, so the result does not depend on ~/.android/debug.keystore.
        keystore = Path(directory) / "check.keystore"
        subprocess.run(
            ["keytool", "-genkeypair", "-keystore", str(keystore), "-storepass", "checkonly", "-keypass",
             "checkonly", "-alias", "check", "-keyalg", "RSA", "-keysize", "2048", "-validity", "1",
             "-dname", "CN=bundle check"],
            check=True,
            capture_output=True,
        )
        apks = Path(directory) / "universal.apks"
        subprocess.run(
            bundletool + ["build-apks", "--bundle", aab, "--output", str(apks), "--mode", "universal",
                          "--ks", str(keystore), "--ks-key-alias", "check", "--ks-pass", "pass:checkonly"],
            check=True,
            capture_output=True,
        )
        with zipfile.ZipFile(apks) as archive:
            archive.extract("universal.apk", directory)
        apk = str(Path(directory) / "universal.apk")
        check_apk(apk, args)
        validate_play_badging(badging(apk, args.aapt2))
        validate_play_manifest(
            subprocess.check_output(
                [args.aapt2, "dump", "xmltree", "--file", "AndroidManifest.xml", apk], text=True
            )
        )
        validate_page_alignment(apk)
    return actual


def badging(apk, aapt2):
    return subprocess.check_output([aapt2, "dump", "badging", apk], text=True)


def check_apk(apk, args):
    validate_badging(badging(apk, args.aapt2), args.version, args.version_code)
    validate_native_libraries(apk)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("apk", help="the release APK, or the Play app bundle (.aab)")
    parser.add_argument("--aapt2", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--version-code", required=True, type=int)
    parser.add_argument("--bundletool", help="bundletool-all.jar (app bundles)")
    parser.add_argument("--cert-sha256", help="the upload certificate's SHA-256 (app bundles)")
    args = parser.parse_args()
    if args.apk.endswith(".aab"):
        if not (args.bundletool and args.cert_sha256):
            parser.error("an app bundle needs --bundletool and --cert-sha256")
        signer = check_bundle(args.apk, args)
        print(
            f"Verified Reins ({args.version}, {args.version_code}) bundle, com.reins2fa.app, upload certificate "
            f"{signer}, Google Play policy checks passed"
        )
        return
    check_apk(args.apk, args)
    print(
        f"Verified Reins ({args.version}, {args.version_code}), com.reins2fa.app, non-debuggable"
    )


if __name__ == "__main__":
    main()
