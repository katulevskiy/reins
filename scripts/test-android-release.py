#!/usr/bin/env python3
"""Exercise missing-signing failure and the packaged Android identity checks."""

import importlib.util
import os
import subprocess
import sys
import tempfile
import textwrap
import zipfile
from pathlib import Path

sys.dont_write_bytecode = True

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location(
    "apk_check", ROOT / "scripts/check-android-apk.py"
)
apk_check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(apk_check)

badging = """package: name='com.reins2fa.app' versionCode='100' versionName='0.2.0'
application-label:'Reins'
application-label-en:'Reins'
"""
apk_check.validate_badging(badging, "0.2.0", 100)
invalid = [
    badging.replace("com.reins2fa.app", "com.example.app"),
    badging.replace("'Reins'", "'Other app'"),
    badging.replace("application-label-en:'Reins'", "application-label-en:'Other app'"),
    badging.replace("versionCode='100'", "versionCode='99'"),
    badging.replace("versionName='0.2.0'", "versionName='0.1.0'"),
    badging + "application-debuggable\n",
    badging.replace("application-label", "missing-label"),
]
for value in invalid:
    try:
        apk_check.validate_badging(value, "0.2.0", 100)
    except ValueError:
        pass
    else:
        raise AssertionError(f"Accepted invalid APK metadata: {value}")

workflow = (ROOT / ".github/workflows/release.yml").read_text()
step = workflow.split("      - name: Require Android signing secrets\n", 1)[1]
lines = step.split("        run: |\n", 1)[1].splitlines()
body = []
for line in lines:
    if line.strip() and not line.startswith("          "):
        break
    body.append(line)
shell = textwrap.dedent("\n".join(body))
keys = ["KEYSTORE", "KEYSTORE_PASSWORD", "KEY_ALIAS", "KEY_PASSWORD"]
fallbacks = [
    "ANDROID_KEYSTORE_BASE64",
    "ANDROID_KEYSTORE_PASSWORD",
    "ANDROID_KEY_ALIAS",
    "ANDROID_KEY_PASSWORD",
]
with tempfile.TemporaryDirectory(prefix="reins-signing-test-") as directory:
    environment = os.environ.copy()
    for key in keys + fallbacks:
        environment.pop(key, None)
    environment["GITHUB_OUTPUT"] = str(Path(directory) / "outputs")
    # Check both supported credential sources, and each incomplete combination.
    for source in (keys, fallbacks):
        for missing in (None, *source):
            values = {key: "test-value" for key in source if key != missing}
            result = subprocess.run(
                ["bash", "-e", "-o", "pipefail", "-c", shell],
                env=environment | values,
                capture_output=True,
                text=True,
                check=False,
            )
            assert (result.returncode == 0) == (missing is None), (
                source,
                missing,
                result.stdout,
            )
    result = subprocess.run(
        ["bash", "-e", "-o", "pipefail", "-c", shell],
        env=environment,
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode != 0

assert "needs.plan.outputs.apk" not in workflow
assert "needs.android.result == 'success'" in workflow
assert "needs.android.result == 'skipped'" not in workflow
assert '[[ -s "reins-$VERSION-android.apk" ]]' in workflow

with tempfile.TemporaryDirectory(prefix="reins-apk-native-") as directory:
    apk = Path(directory) / "test.apk"
    libraries = {}
    for abi, machine in (("arm64-v8a", 183), ("x86_64", 62)):
        header = bytearray(20)
        header[:6] = b"\x7fELF\x02\x01"
        header[18:20] = machine.to_bytes(2, "little")
        libraries[f"lib/{abi}/libreins_core.so"] = header

    def write_apk(files):
        with zipfile.ZipFile(apk, "w") as archive:
            for name, contents in files.items():
                archive.writestr(name, contents)

    write_apk(libraries)
    apk_check.validate_native_libraries(apk)
    invalid_native = [
        {},
        {name: header for name, header in libraries.items() if "arm64" in name},
        {name: header for name, header in libraries.items() if "x86_64" in name},
        dict.fromkeys(libraries, b"not an ELF library"),
        dict.fromkeys(libraries, libraries["lib/arm64-v8a/libreins_core.so"]),
    ]
    for files in invalid_native:
        write_apk(files)
        try:
            apk_check.validate_native_libraries(apk)
        except ValueError:
            pass
        else:
            raise AssertionError("Accepted incomplete or mismatched native libraries")

    # 16 KB pages: every loadable segment of every 64-bit library.
    def elf(align, phoff=64):
        header = bytearray(phoff)
        header[:6] = b"\x7fELF\x02\x01"
        header[0x20:0x28] = phoff.to_bytes(8, "little")
        header[0x36:0x38] = (56).to_bytes(2, "little")
        header[0x38:0x3A] = (2).to_bytes(2, "little")
        segments = b""
        for kind, value in ((6, 8), (1, align)):  # PT_PHDR (any alignment), PT_LOAD
            entry = bytearray(56)
            entry[0:4] = kind.to_bytes(4, "little")
            entry[0x30:0x38] = value.to_bytes(8, "little")
            segments += entry
        return bytes(header) + segments

    write_apk({"lib/arm64-v8a/libreins_core.so": elf(16384), "lib/x86_64/libonnxruntime.so": elf(65536)})
    apk_check.validate_page_alignment(apk)
    for files in (
        {"lib/arm64-v8a/libreins_core.so": elf(4096)},
        {"lib/x86_64/libonnxruntime.so": elf(16384)[:40]},
        {"lib/arm64-v8a/libjnidispatch.so": b"not an ELF library"},
    ):
        write_apk(files)
        try:
            apk_check.validate_page_alignment(apk)
        except ValueError:
            pass
        else:
            raise AssertionError(f"Accepted a library without 16 KB alignment: {list(files)}")

# The Play build: no restricted permission, only the declared foreground-service type, a current targetSdk.
play_badging = badging + """targetSdkVersion:'36'
uses-permission: name='android.permission.INTERNET'
uses-permission: name='android.permission.FOREGROUND_SERVICE'
uses-permission: name='android.permission.FOREGROUND_SERVICE_DATA_SYNC'
uses-permission: name='android.permission.READ_CONTACTS'
"""
apk_check.validate_play_badging(play_badging)
invalid_play = [
    play_badging + "uses-permission: name='android.permission.READ_SMS'\n",
    play_badging + "uses-permission: name='android.permission.SEND_SMS'\n",
    play_badging + "uses-permission: name='android.permission.REQUEST_INSTALL_PACKAGES'\n",
    play_badging + "uses-permission: name='com.google.android.gms.permission.AD_ID'\n",
    play_badging + "uses-permission-sdk-23: name='android.permission.QUERY_ALL_PACKAGES'\n",
    play_badging + "uses-permission: name='android.permission.FOREGROUND_SERVICE_SPECIAL_USE'\n",
    play_badging.replace("targetSdkVersion:'36'", "targetSdkVersion:'35'"),
    play_badging.replace("targetSdkVersion:'36'\n", ""),
]
for value in invalid_play:
    try:
        apk_check.validate_play_badging(value)
    except ValueError:
        pass
    else:
        raise AssertionError(f"Accepted a Play build Google Play would refuse: {value.splitlines()[-1]}")

manifest = """E: application (line=44)
  A: http://schemas.android.com/apk/res/android:usesCleartextTraffic(0x010104ec)=false
  E: service (line=161)
    A: http://schemas.android.com/apk/res/android:foregroundServiceType(0x01010599)=0x00000001
"""
apk_check.validate_play_manifest(manifest)
apk_check.validate_play_manifest(manifest.replace("=false", "=0x0"))
for value in (
    manifest.replace("=false", "=true"),
    manifest.replace("=false", "=0xffffffff"),
    manifest.replace("=0x00000001", "=0x00000009"),
    manifest.replace("=0x00000001", "=0x40000000"),
):
    try:
        apk_check.validate_play_manifest(value)
    except ValueError:
        pass
    else:
        raise AssertionError(f"Accepted a Play manifest Google Play would refuse: {value}")

# The upload signature: one signer, its first certificate's SHA-256; jarsigner covers every entry.
pinned = (ROOT / "android/release-signing.sha256").read_text().strip()
digest = ":".join(pinned[i:i + 2] for i in range(0, 64, 2)).upper()  # keytool's format
printcert = f"Signer #1:\n\nCertificate #1:\nOwner: CN=Reins\n\t SHA1: 00:11\n\t SHA256: {digest}\n"
assert apk_check.bundle_signer_sha256(printcert) == pinned
for value in ("", printcert + printcert.replace("#1", "#2"), "Signer #1:\n"):
    try:
        apk_check.bundle_signer_sha256(value)
    except ValueError:
        pass
    else:
        raise AssertionError(f"Accepted a bundle signature: {value!r}")
apk_check.validate_jarsigner("jar verified.\n\nWarning:\nThis jar contains entries whose signer certificate is self-signed.")
for value in (
    "jar is unsigned.",
    "jar verified.\n\nWarning:\nThis jar contains unsigned entries which have not been integrity-checked.",
    "jarsigner: java.lang.SecurityException: SHA-256 digest error for classes.dex",
):
    try:
        apk_check.validate_jarsigner(value)
    except ValueError:
        pass
    else:
        raise AssertionError(f"Accepted a broken bundle signature: {value}")

# The release builds, checks and publishes the Play bundle next to the APK.
android_job = workflow.split("  android:\n", 1)[1].split("\n  play:\n", 1)[0]
assert ":app:bundlePlayRelease" in android_job
assert 'REINS_UPLOAD_KEYSTORE="$RUNNER_TEMP/release.keystore"' in android_job
assert '--cert-sha256 "$(cat android/release-signing.sha256)"' in android_job
assert '[[ -s "reins-$VERSION-play.aab" ]]' in workflow

print("56 Android release checks passed")
