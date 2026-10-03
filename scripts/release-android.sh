#!/usr/bin/env bash
# Publishes the Reins Android app from the current commit: builds the release APK (the `full` flavor: text messages
# and the in-app updater) with an automatic version,
# signs it with the app's key, uploads it, then switches the permanent link to it:
#
#   $REINS_SITE/reins.apk  the latest APK
#
# The app's updater reads releases/android/latest.json; the download page, $REINS_SITE/app, is the website's. Run
# from anywhere in the repository:
#
#   infisical run --env=prod --path=/signing/android -- scripts/release-android.sh
#   scripts/release-android.sh            # refuses uncommitted changes
#   scripts/release-android.sh --dirty    # publish anyway (the build id says so)
#
# Settings, from the environment or scripts/release.env (see scripts/release.env.example):
#   REINS_SITE                  the site that serves the releases (required)
#   REINS_SERVER                the sign-in screen's server (reins.defaultServer in android/gradle.properties)
#   REINS_RELEASE_SSH           SSH destination to upload to (required)
#   REINS_RELEASE_DIR           directory on it that the site serves as /releases (required)
#   REINS_ANDROID_CERT_SHA1     optional additional SHA-1 check (SHA-256 is pinned in android/release-signing.sha256)
#   REINS_ANDROID_KEYSTORE      keystore to sign with (~/.android/debug.keystore)
#   REINS_ANDROID_KEYSTORE_PASS its password (android)
#   ANDROID_KEYSTORE_BASE64, ANDROID_KEYSTORE_PASSWORD, ANDROID_KEY_ALIAS, ANDROID_KEY_PASSWORD
#                              injected by Infisical; override the local keystore settings
#   REINS_RELEASE_KEEP          APKs kept on the server (5)
#
# The Google Play build is the `play` flavor, an app bundle this script does not make or upload (android/PLAY_STORE.md):
#
#   (cd android && ./gradlew bundlePlayRelease -Preins.versionCode=N -Preins.versionName=0.1.0 \
#       -Preins.build=0.1.0-N)   # app/build/outputs/bundle/playRelease/app-play-release.aab
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/release-env.sh"
require_settings REINS_SITE REINS_RELEASE_SSH REINS_RELEASE_DIR

SSH_TARGET="$REINS_RELEASE_SSH"
REMOTE_DIR="$REINS_RELEASE_DIR"
SITE="${REINS_SITE%/}"
KEYSTORE="${REINS_ANDROID_KEYSTORE:-${REINS_RELEASE_KEYSTORE:-$HOME/.android/debug.keystore}}"
KEYSTORE_PASS="${REINS_ANDROID_KEYSTORE_PASS:-${ANDROID_KEYSTORE_PASSWORD:-${REINS_RELEASE_KEYSTORE_PASSWORD:-android}}}"
KEY_ALIAS="${ANDROID_KEY_ALIAS:-${REINS_RELEASE_KEY_ALIAS:-androiddebugkey}}"
KEY_PASS="${ANDROID_KEY_PASSWORD:-${REINS_RELEASE_KEY_PASSWORD:-$KEYSTORE_PASS}}"
CERT_SHA1="${REINS_ANDROID_CERT_SHA1:-}"
KEEP="${REINS_RELEASE_KEEP:-5}"
server_arg=()
if [[ -n "${REINS_SERVER:-}" ]]; then server_arg=("-Preins.defaultServer=${REINS_SERVER%/}"); fi

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
version="$(sed -n 's/.*"reins.versionName").getOrElse("\([^"]*\)").*/\1/p' android/app/build.gradle.kts | head -1)"
version="${version:-0.1.0}"
build_time="$(date -u +%s)"
version_code="$((build_time / 60))"
build="$version-$(date -u -d "@$build_time" +%Y%m%d%H%M)-$sha"
file="reins-$build.apk"
echo "Android release $build (versionCode $version_code)"

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
# Infisical injects the same four settings used by GitHub release CI. Decode only into the private staging dir.
if [[ -n "${ANDROID_KEYSTORE_BASE64:-}" ]]; then
    umask 077
    printf '%s' "$ANDROID_KEYSTORE_BASE64" | base64 -d > "$stage/signing.keystore"
    KEYSTORE="$stage/signing.keystore"
    KEYSTORE_PASS="${ANDROID_KEYSTORE_PASSWORD:-}"
    KEY_ALIAS="${ANDROID_KEY_ALIAS:-}"
    KEY_PASS="${ANDROID_KEY_PASSWORD:-}"
    [[ -n "${ANDROID_KEYSTORE_PASSWORD:-}" && -n "${ANDROID_KEY_ALIAS:-}" && -n "${ANDROID_KEY_PASSWORD:-}" ]] || {
        echo "Infisical Android signing settings are incomplete" >&2
        exit 1
    }
fi
# Screens with secrets are never capturable in a published APK, whatever ~/.gradle/gradle.properties says.
(cd android && ./gradlew assembleFullRelease -q \
    "-Preins.secureScreens=true" \
    "-Preins.site=$SITE" \
    ${server_arg[@]+"${server_arg[@]}"} \
    "-Preins.versionCode=$version_code" \
    "-Preins.versionName=$version" \
    "-Preins.build=$build")
"$build_tools/zipalign" -f -P 16 4 android/app/build/outputs/apk/full/release/app-full-release.apk "$stage/aligned.apk"
export REINS_APKSIGNER_STORE_PASSWORD="$KEYSTORE_PASS" REINS_APKSIGNER_KEY_PASSWORD="$KEY_PASS"
"$build_tools/apksigner" sign --ks "$KEYSTORE" --ks-key-alias "$KEY_ALIAS" \
    --ks-pass env:REINS_APKSIGNER_STORE_PASSWORD --key-pass env:REINS_APKSIGNER_KEY_PASSWORD \
    --out "$stage/$file" "$stage/aligned.apk"
cert="$("$build_tools/apksigner" verify --print-certs "$stage/$file" | sed -n 's/.*certificate SHA-1 digest: //p' | head -1)"
[[ -z "$CERT_SHA1" || "$cert" == "$CERT_SHA1" ]] || {
    echo "the APK is signed with $cert, not $CERT_SHA1; phones would refuse it as an update" >&2
    exit 1
}
cert_sha256="$("$build_tools/apksigner" verify --print-certs "$stage/$file" | sed -n 's/.*certificate SHA-256 digest: //p' | head -1)"
expected_sha256="$(cat android/release-signing.sha256)"
[[ "$cert_sha256" == "$expected_sha256" ]] || {
    echo "the APK signing certificate changed; existing phones would refuse the update" >&2
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

echo "Uploading to $SSH_TARGET:$REMOTE_DIR/android..."
ssh "$SSH_TARGET" "mkdir -p '$REMOTE_DIR/android/files' '$REMOTE_DIR/android/.incoming'"
scp -q "$stage/$file" "$SSH_TARGET:$REMOTE_DIR/android/files/"
scp -q "$stage/latest.json" "$SSH_TARGET:$REMOTE_DIR/android/.incoming/"
# The APK is in place first; the link and latest.json then switch with atomic renames. The download page ($SITE/app)
# belongs to the website, not to this script.
ssh "$SSH_TARGET" "set -e; cd '$REMOTE_DIR/android'; chmod 644 'files/$file' .incoming/*;
    ln -sfn 'files/$file' reins.apk.new && mv -Tf reins.apk.new reins.apk;
    mv -f .incoming/latest.json latest.json;
    ls -1t files/ | tail -n +$((KEEP + 1)) | sed 's|^|files/|' | xargs -r rm -f"

cp "$stage/$file" "$HOME/Downloads/reins.apk" 2>/dev/null || true
published="$(curl -fsS "$SITE/releases/android/latest.json")"
[[ "$published" == *"$build"* ]] || {
    echo "the server does not show the new release: $published" >&2
    exit 1
}
served="$(curl -fsSL "$SITE/reins.apk" | sha256sum | cut -d' ' -f1)"
[[ "$served" == "$apk_sha" ]] || {
    echo "$SITE/reins.apk is not the new APK" >&2
    exit 1
}
echo "Published $build"
echo "  page: $SITE/app"
echo "  apk:  $SITE/reins.apk"
