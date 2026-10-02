#!/usr/bin/env bash
# Publishes the Rewarden Android app from the current commit: builds the release APK (the `full` flavor: text messages
# and the in-app updater) with an automatic version,
# signs it with the app's key, uploads it, then switches the permanent links to it:
#
#   $REWARDEN_SITE/app           download page
#   $REWARDEN_SITE/rewarden.apk  the latest APK
#
# The app's updater reads releases/android/latest.json. Run from anywhere in the repository:
#
#   scripts/release-android.sh            # refuses uncommitted changes
#   scripts/release-android.sh --dirty    # publish anyway (the build id says so)
#
# Settings, from the environment or scripts/release.env (see scripts/release.env.example):
#   REWARDEN_SITE                  the site that serves the releases (required)
#   REWARDEN_RELEASE_SSH           SSH destination to upload to (required)
#   REWARDEN_RELEASE_DIR           directory on it that the site serves as /releases (required)
#   REWARDEN_ANDROID_CERT_SHA1     SHA-1 of the certificate every release must be signed with, so a wrong key never
#                                  ships (required)
#   REWARDEN_ANDROID_KEYSTORE      keystore to sign with (~/.android/debug.keystore)
#   REWARDEN_ANDROID_KEYSTORE_PASS its password (android)
#   REWARDEN_RELEASE_KEEP          APKs kept on the server (5)
#
# The Google Play build is the `play` flavor, an app bundle this script does not make or upload (android/PLAY_STORE.md):
#
#   (cd android && ./gradlew bundlePlayRelease -Prewarden.versionCode=N -Prewarden.versionName=0.1.0 \
#       -Prewarden.defaultServer=$REWARDEN_SITE)   # app/build/outputs/bundle/playRelease/app-play-release.aab
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/release-env.sh"
require_settings REWARDEN_SITE REWARDEN_RELEASE_SSH REWARDEN_RELEASE_DIR REWARDEN_ANDROID_CERT_SHA1

SSH_TARGET="$REWARDEN_RELEASE_SSH"
REMOTE_DIR="$REWARDEN_RELEASE_DIR"
SITE="${REWARDEN_SITE%/}"
KEYSTORE="${REWARDEN_ANDROID_KEYSTORE:-$HOME/.android/debug.keystore}"
KEYSTORE_PASS="${REWARDEN_ANDROID_KEYSTORE_PASS:-android}"
CERT_SHA1="$REWARDEN_ANDROID_CERT_SHA1"
KEEP="${REWARDEN_RELEASE_KEEP:-5}"

dirty_ok=false
[[ "${1:-}" == "--dirty" ]] && dirty_ok=true

cd "$(git rev-parse --show-toplevel)"
# The SDK the Gradle build uses (android/local.properties), else the usual places that exist.
sdk="$(sed -n 's/^sdk\.dir=//p' android/local.properties 2>/dev/null | head -1)"
for candidate in "$sdk" "${ANDROID_HOME:-}" "${ANDROID_SDK_ROOT:-}" "$HOME/Android/Sdk"; do
    if [[ -n "$candidate" && -d "$candidate/build-tools" ]]; then
        sdk="$candidate"
        break
    fi
done
build_tools="$(ls -d "$sdk"/build-tools/* 2>/dev/null | sort -V | tail -1)"
[[ -x "$build_tools/apksigner" ]] || {
    echo "Android build-tools not found under $sdk" >&2
    exit 1
}

sha="$(git rev-parse --short=8 HEAD)"
if [[ -n "$(git status --porcelain --untracked-files=no)" ]]; then
    $dirty_ok || {
        echo "uncommitted changes; commit them or pass --dirty" >&2
        exit 1
    }
    sha="$sha.dirty"
fi
version="$(sed -n 's/.*"rewarden.versionName").getOrElse("\([^"]*\)").*/\1/p' android/app/build.gradle.kts | head -1)"
version="${version:-0.1.0}"
build_time="$(date -u +%s)"
version_code="$((build_time / 60))"
build="$version-$(date -u -d "@$build_time" +%Y%m%d%H%M)-$sha"
file="rewarden-$build.apk"
echo "Android release $build (versionCode $version_code)"

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
# Screens with secrets are never capturable in a published APK, whatever ~/.gradle/gradle.properties says.
(cd android && ./gradlew assembleFullRelease -q \
    "-Prewarden.secureScreens=true" \
    "-Prewarden.site=$SITE" \
    "-Prewarden.defaultServer=$SITE" \
    "-Prewarden.versionCode=$version_code" \
    "-Prewarden.versionName=$version" \
    "-Prewarden.build=$build")
"$build_tools/zipalign" -f -P 16 4 android/app/build/outputs/apk/full/release/app-full-release.apk "$stage/aligned.apk"
"$build_tools/apksigner" sign --ks "$KEYSTORE" --ks-pass "pass:$KEYSTORE_PASS" --key-pass "pass:$KEYSTORE_PASS" \
    --out "$stage/$file" "$stage/aligned.apk"
cert="$("$build_tools/apksigner" verify --print-certs "$stage/$file" | sed -n 's/.*certificate SHA-1 digest: //p' | head -1)"
[[ "$cert" == "$CERT_SHA1" ]] || {
    echo "the APK is signed with $cert, not $CERT_SHA1; phones would refuse it as an update" >&2
    exit 1
}
badging="$("$build_tools/aapt2" dump badging "$stage/$file" 2>/dev/null | head -1 || true)"
[[ "$badging" == *"versionCode='$version_code'"* ]] || {
    echo "the APK does not carry versionCode $version_code: $badging" >&2
    exit 1
}

apk_sha="$(sha256sum "$stage/$file" | cut -d' ' -f1)"
size="$(stat -c %s "$stage/$file")"
cat >"$stage/latest.json" <<EOF
{"versionCode": $version_code, "versionName": "$version", "build": "$build",
 "file": "$file", "sha256": "$apk_sha", "size": $size,
 "published_at": $build_time}
EOF
mb="$(awk -v s="$size" 'BEGIN { printf "%.1f", s / 1048576 }')"
date_text="$(date -u -d "@$build_time" '+%Y-%m-%d %H:%M UTC')"
cat >"$stage/app.html" <<EOF
<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Rewarden for Android</title>
<style>
body{font:16px/1.5 system-ui,sans-serif;margin:0;background:#0b0b0f;color:#eee;display:flex;min-height:100vh;align-items:center;justify-content:center}
main{max-width:420px;padding:32px 24px}h1{font-size:28px;margin:0 0 8px}p{color:#aaa}
a.button{display:block;text-align:center;background:#6c5ce7;color:#fff;text-decoration:none;font-weight:600;padding:16px;border-radius:14px;margin:24px 0}
code{font-size:12px;word-break:break-all;color:#888}
</style></head><body><main>
<h1>Rewarden for Android</h1>
<p>Approve what your AI agents may do, from your phone.</p>
<a class="button" href="/rewarden.apk">Download version $version</a>
<p>Build $build &middot; $mb MB &middot; $date_text</p>
<p>Open the downloaded file to install. Android may ask you to allow installs from your browser once. The app then
updates itself.</p>
<p><code>SHA-256 $apk_sha</code></p>
</main></body></html>
EOF

echo "Uploading to $SSH_TARGET:$REMOTE_DIR/android..."
ssh "$SSH_TARGET" "mkdir -p '$REMOTE_DIR/android/files' '$REMOTE_DIR/android/.incoming'"
scp -q "$stage/$file" "$SSH_TARGET:$REMOTE_DIR/android/files/"
scp -q "$stage/latest.json" "$stage/app.html" "$SSH_TARGET:$REMOTE_DIR/android/.incoming/"
# The APK is in place first; the link, the page and latest.json then switch with atomic renames.
ssh "$SSH_TARGET" "set -e; cd '$REMOTE_DIR/android'; chmod 644 'files/$file' .incoming/*;
    ln -sfn 'files/$file' rewarden.apk.new && mv -Tf rewarden.apk.new rewarden.apk;
    mv -f .incoming/app.html app.html; mv -f .incoming/latest.json latest.json;
    ls -1t files/ | tail -n +$((KEEP + 1)) | sed 's|^|files/|' | xargs -r rm -f"

cp "$stage/$file" "$HOME/Downloads/rewarden.apk" 2>/dev/null || true
published="$(curl -fsS "$SITE/releases/android/latest.json")"
[[ "$published" == *"$build"* ]] || {
    echo "the server does not show the new release: $published" >&2
    exit 1
}
served="$(curl -fsSL "$SITE/rewarden.apk" | sha256sum | cut -d' ' -f1)"
[[ "$served" == "$apk_sha" ]] || {
    echo "$SITE/rewarden.apk is not the new APK" >&2
    exit 1
}
echo "Published $build"
echo "  page: $SITE/app"
echo "  apk:  $SITE/rewarden.apk"
