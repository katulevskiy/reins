#!/usr/bin/env bash
# The minified Trust smoke test (ci.yml, "Android minified Trust smoke test"), on the emulator that job started.
#
# Quarantine: TrustReleaseTest.trustSurvivesAsyncRefreshesAndItsSheetsStillWork fails now and then on the CI emulator
# with the app not in front ("No visible control matching Trust", only the System UI on screen): 9 pull request
# failures in 11 days (2026-10-16 to 10-27), while the same code passed on main. A failed run is retried once, with a
# warning, after waking and unlocking the screen. A real regression fails both times, so the job still gates pull
# requests and main.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../android"
smoke() {
    ./gradlew --no-daemon --console=plain --no-configuration-cache connectedFullReleaseAndroidTest \
        -Preins.prebuiltNativeDir="$GITHUB_WORKSPACE/native" \
        -Preins.testBuildType=release -Preins.secureScreens=false \
        -Preins.testRunner=dev.reins.android.TrustTestRunner \
        -Preins.releaseKeystore="$RUNNER_TEMP/trust-test.keystore" \
        -Preins.releaseKeystorePassword=android -Preins.releaseKeyAlias=androiddebugkey \
        -Pandroid.testInstrumentationRunnerArguments.class=dev.reins.android.TrustReleaseTest
}
smoke && exit 0
echo "::warning title=Android Trust smoke test::Failed once; retrying (a known emulator flake, scripts/android-trust-smoke.sh)"
adb shell input keyevent KEYCODE_WAKEUP || true
adb shell wm dismiss-keyguard || true
smoke
