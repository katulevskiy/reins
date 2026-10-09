#!/usr/bin/env bash
# Builds the update feed of one release as a single archive, reins-feed-<version>.tar.gz, from that release's assets.
# The release workflow runs it (it is published with the release and listed in SHA256SUMS); the server's
# reins-releases-sync timer then mirrors the newest release's feed into /srv/reins-releases, which reins2fa.com serves
# as /releases/, /install.sh and /install.ps1. The archive holds, under feed/:
#
#   latest.json, latest-<platform>.txt   the signed command-line manifest (`reins update`, install.sh, install.ps1)
#   files/reins-<build>-<platform>        the command-line programs it lists, taken out of the release archives
#   app.json                              the signed desktop-app manifest (the app's one-click update)
#   android/latest.json                   the Android updater's feed (the APK itself is verified by its signature)
#   install.sh, install.ps1               the installers, pointing at --site
#   fetch.txt                             "<feed path> <release asset> <sha256> <size>" for the large files that are
#                                         release assets already (installers, APK); the server downloads those itself
#   feed.json                             the signed index: every file above (fetched ones included) with its SHA-256;
#                                         the server publishes nothing it does not list
#
# The manifests are signed with the release key (Ed25519, PKCS#8 DER): latest.json over the context of
# `reins-release manifest` (reins-release/1), app.json over reins-app/1 and the index over reins-feed/1, so none can
# pass for another; and all are checked against the key the
# apps pin (crates/reins-desktop/src/update.rs) before anything is written.
#
#   scripts/release-feed.sh --version 0.2.5 --build 0.2.5-202610090300-fad997a6 --time 1791515000 \
#       --android-version-code 29858583 --dist dist --key release-signing.pk8 [--site https://reins2fa.com] [--out dist]
set -euo pipefail

die() {
    echo "release-feed: $*" >&2
    exit 1
}

version="" build="" build_time="" version_code="" dist="" key="" site="https://reins2fa.com" out=""
while (($#)); do
    case "$1" in
    --version) version="$2" ;;
    --build) build="$2" ;;
    --time) build_time="$2" ;;
    --android-version-code) version_code="$2" ;;
    --dist) dist="$2" ;;
    --key) key="$2" ;;
    --site) site="${2%/}" ;;
    --out) out="$2" ;;
    *) die "unknown argument $1" ;;
    esac
    shift 2
done
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "--version X.Y.Z is required"
[[ "$build" == "$version"-* && "$build" =~ ^[0-9A-Za-z._-]+$ ]] || die "--build must start with $version-"
[[ "$build_time" =~ ^[0-9]+$ ]] || die "--time (unix seconds) is required"
[[ "$version_code" =~ ^[0-9]+$ ]] || die "--android-version-code is required"
[[ -d "$dist" ]] || die "--dist must be the directory with the release assets"
[[ -f "$key" ]] || die "--key must be the release signing key"
out="${out:-$dist}"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
feed="$stage/feed"
mkdir -p "$feed/files" "$feed/android"

# `reins-release keygen` (ring) writes PKCS#8 v2 (with the public key), which OpenSSL 3.0 cannot read; sign with the
# same key as PKCS#8 v1, made from its 32-byte seed.
key_hex="$(od -An -tx1 -v "$key" | tr -d ' \n')"
case "$key_hex" in
3051020101300506032b657004220420*) printf '302e020100300506032b657004220420%s' "${key_hex:32:64}" | xxd -r -p >"$stage/key.der" ;;
302e020100300506032b657004220420*) cp "$key" "$stage/key.der" ;;
*) die "$key is not an Ed25519 PKCS#8 key" ;;
esac
key="$stage/key.der"

# The key must be the one the apps pin, or every client would refuse the feed.
pinned="$(sed -n 's/^pub const RELEASE_KEY: &str = "\([0-9a-f]*\)";/\1/p' "$root/crates/reins-desktop/src/update.rs")"
public="$(openssl pkey -inform DER -in "$key" -pubout -outform DER | tail -c 32 | od -An -tx1 | tr -d ' \n')"
[[ -n "$pinned" && "$public" == "$pinned" ]] || die "$key is not the release key the apps pin (update::RELEASE_KEY)"
# SubjectPublicKeyInfo of an Ed25519 key: a fixed 12-byte prefix, then the key.
printf '302a300506032b6570032100%s' "$pinned" | xxd -r -p >"$stage/public.der"

sha() { sha256sum "$1" | cut -d' ' -f1; }
size() { stat -c %s "$1"; }

# Writes feed/<name>: {<field>: <compact JSON>, "signature": <base64url Ed25519 over context + that JSON>}.
sign_json() {
    local json="$1" name="$2" field="$3" context="$4"
    { printf '%s\n' "$context"; printf '%s' "$json"; } >"$stage/message"
    openssl pkeyutl -sign -inkey "$key" -keyform DER -rawin -in "$stage/message" -out "$stage/signature"
    openssl pkeyutl -verify -pubin -inkey "$stage/public.der" -keyform DER -rawin -in "$stage/message" \
        -sigfile "$stage/signature" >/dev/null || die "$name: the signature does not verify"
    signature="$(basenc --base64url -w0 "$stage/signature" | tr -d '=')"
    jq -n --arg field "$field" --arg json "$json" --arg signature "$signature" \
        '{($field): $json, signature: $signature}' >"$feed/$name"
}
sign_manifest() { sign_json "$1" "$2" manifest reins-release/1; }

# ── the command-line programs ────────────────────────────────────────────────────────────────────────────────────
# platform:target:kind (optional ones are skipped when the release has no such archive)
cli_targets=(
    linux-x86_64:x86_64-unknown-linux-musl:required
    linux-aarch64:aarch64-unknown-linux-musl:required
    macos-aarch64:aarch64-apple-darwin:required
    macos-x86_64:x86_64-apple-darwin:required
    windows-x86_64:x86_64-pc-windows-msvc:required
    windows-aarch64:aarch64-pc-windows-msvc:optional
)
cli_assets='{}'
for t in "${cli_targets[@]}"; do
    IFS=: read -r platform triple need <<<"$t"
    dir="reins-desktop-$version-$triple"
    case "$triple" in
    *windows*) archive="$dir.zip" program="reins.exe" ;;
    *) archive="$dir.tar.gz" program="reins" ;;
    esac
    if [[ ! -f "$dist/$archive" ]]; then
        [[ "$need" == optional ]] && { echo "Skipping $platform: no $archive"; continue; }
        die "$archive is missing"
    fi
    mkdir -p "$stage/x"
    case "$archive" in
    *.zip) unzip -q -o "$dist/$archive" "$dir/$program" -d "$stage/x" ;;
    *) tar -xzf "$dist/$archive" -C "$stage/x" "$dir/$program" ;;
    esac || die "$archive has no $dir/$program"
    grep -qaF "$version ($build)" "$stage/x/$dir/$program" || die "$archive: $program does not carry the build id $build"
    # No .exe in the feed's file name: `reins update` writes it over the program it replaces.
    file="reins-$build-$platform"
    mv "$stage/x/$dir/$program" "$feed/files/$file"
    chmod 0644 "$feed/files/$file"
    (($(size "$feed/files/$file") <= 64 << 20)) || die "$file is over the 64 MiB the command-line updater accepts"
    cli_assets="$(jq -c --arg p "$platform" --arg f "$file" --arg s "$(sha "$feed/files/$file")" \
        --argjson n "$(size "$feed/files/$file")" '. + {($p): {file: $f, sha256: $s, size: $n}}' <<<"$cli_assets")"
    printf '%s %s %s %s\n' "$file" "$(sha "$feed/files/$file")" "$version" "$build" >"$feed/latest-$platform.txt"
done
manifest="$(jq -c -n --arg v "$version" --arg b "$build" --argjson t "$build_time" --argjson a "$cli_assets" \
    '{version: $v, build: $b, build_time: $t, assets: $a}')"
sign_manifest "$manifest" latest.json

# ── the desktop app's installers and the APK: release assets the server fetches ──────────────────────────────────────
: >"$feed/fetch.txt"
fetch() { # <feed path> <release asset>
    [[ -s "$dist/$2" ]] || die "$2 is missing"
    printf '%s %s %s %s\n' "$1" "$2" "$(sha "$dist/$2")" "$(size "$dist/$2")" >>"$feed/fetch.txt"
}
app_assets='{}'
# platform:release asset:feed file
for t in "macos-universal:Reins-$version-macOS.dmg:Reins-$build-macOS.dmg" \
    "windows-x86_64:Reins-$version-Windows-x64.msi:Reins-$build-Windows-x64.msi" \
    "linux-x86_64:Reins-$version-Linux-x86_64.AppImage:Reins-$build-Linux-x86_64.AppImage"; do
    IFS=: read -r platform asset file <<<"$t"
    fetch "files/$file" "$asset"
    app_assets="$(jq -c --arg p "$platform" --arg f "$file" --arg s "$(sha "$dist/$asset")" \
        --argjson n "$(size "$dist/$asset")" '. + {($p): {file: $f, sha256: $s, size: $n}}' <<<"$app_assets")"
done
manifest="$(jq -c -n --arg v "$version" --arg b "$build" --argjson t "$build_time" --argjson a "$app_assets" \
    '{version: $v, build: $b, build_time: $t, assets: $a}')"
sign_json "$manifest" app.json manifest reins-app/1

apk="reins-$version-android.apk"
fetch "android/files/reins-$build.apk" "$apk"
jq -n --argjson code "$version_code" --arg v "$version" --arg b "$build" --arg f "reins-$build.apk" \
    --arg s "$(sha "$dist/$apk")" --argjson n "$(size "$dist/$apk")" --argjson t "$build_time" \
    '{versionCode: $code, versionName: $v, build: $b, file: $f, sha256: $s, size: $n, published_at: $t}' \
    >"$feed/android/latest.json"

# ── the installers, pointing at this site ────────────────────────────────────────────────────────────────────────────
sed "s|^DEFAULT_SITE=.*|DEFAULT_SITE=\"$site\"|" "$root/scripts/install.sh" >"$feed/install.sh"
grep -qxF "DEFAULT_SITE=\"$site\"" "$feed/install.sh" || die "could not set DEFAULT_SITE in install.sh"
sed 's|^\( *\)\$DefaultSite = .*|\1$DefaultSite = "'"$site"'"|' "$root/scripts/install.ps1" >"$feed/install.ps1"
grep -qF "\$DefaultSite = \"$site\"" "$feed/install.ps1" || die "could not set \$DefaultSite in install.ps1"

# ── the signed index of everything above ─────────────────────────────────────────────────────────────────────────
files='{}'
while IFS= read -r path; do
    files="$(jq -c --arg p "$path" --arg s "$(sha "$feed/$path")" '. + {($p): $s}' <<<"$files")"
done < <(cd "$feed" && find . -type f -printf '%P\n' | sort)
while read -r path _ sha _; do
    files="$(jq -c --arg p "$path" --arg s "$sha" '. + {($p): $s}' <<<"$files")"
done <"$feed/fetch.txt"
index="$(jq -c -n --arg v "$version" --arg b "$build" --argjson f "$files" '{version: $v, build: $b, files: $f}')"
sign_json "$index" feed.json index reins-feed/1

mkdir -p "$out"
tar --sort=name --owner=0 --group=0 --numeric-owner -czf "$out/reins-feed-$version.tar.gz" -C "$stage" feed
echo "Wrote $out/reins-feed-$version.tar.gz:"
tar -tzvf "$out/reins-feed-$version.tar.gz" | awk '{print "  " $3 "  " $6}'
sed 's/^/  fetch: /' "$feed/fetch.txt"
