#!/usr/bin/env python3
"""Exercise missing-signing failure and the packaged Android identity checks."""

import importlib.util
import sys
import os
from pathlib import Path
import subprocess
import tempfile
import textwrap


sys.dont_write_bytecode = True

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("apk_check", ROOT / "scripts/check-android-apk.py")
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
fallbacks = ["ANDROID_KEYSTORE_BASE64", "ANDROID_KEYSTORE_PASSWORD", "ANDROID_KEY_ALIAS", "ANDROID_KEY_PASSWORD"]
with tempfile.TemporaryDirectory(prefix="reins-signing-test-") as directory:
    environment = os.environ.copy()
    for key in keys + fallbacks:
        environment.pop(key, None)
    environment["GITHUB_OUTPUT"] = str(Path(directory) / "outputs")
    # Check both supported credential sources, and each incomplete combination.
    for source in (keys, fallbacks):
        for missing in (None, *source):
            values = {key: "test-value" for key in source if key != missing}
            result = subprocess.run(["bash", "-e", "-o", "pipefail", "-c", shell],
                                    env=environment | values, capture_output=True, text=True)
            assert (result.returncode == 0) == (missing is None), (source, missing, result.stdout)
    result = subprocess.run(["bash", "-e", "-o", "pipefail", "-c", shell],
                            env=environment, capture_output=True, text=True)
    assert result.returncode != 0

assert "needs.plan.outputs.apk" not in workflow
assert "needs.android.result == 'success'" in workflow
assert "needs.android.result == 'skipped'" not in workflow
assert '[[ -s "reins-$VERSION-android.apk" ]]' in workflow
print("19 Android release checks passed")
