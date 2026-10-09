#!/usr/bin/env bash
# Builds the signed APT and RPM repositories of one release from its Linux packages (scripts/package/linux-packages.sh),
# into --out/packages/, the tree scripts/release-feed.sh --packages puts in the update feed (served as
# <site>/releases/packages/):
#
#   reins.gpg, reins.asc                    the repositories' public key (binary, for /etc/apt/keyrings; armored)
#   apt/dists/stable/InRelease, Release, Release.gpg
#   apt/dists/stable/main/binary-{amd64,arm64}/Packages(.gz)
#   apt/pool/main/r/<package>/<package>_<version>_<arch>.deb
#   rpm/repodata/repomd.xml, repomd.xml.asc and the metadata it names
#   rpm/<arch>/<package>-<version>-1.<arch>.rpm   signed with rpmsign
#   rpm/reins.repo                          the dnf/zypper repository definition (gpgcheck and repo_gpgcheck on)
#
# Every repository holds the one release it is built from: each feed replaces the previous one. The .deb and .rpm
# files are copies of the packages in --packages, the .rpm files signed (the release publishes those signed copies).
#
#   GNUPGHOME=<keyring with the secret key> scripts/package/linux-repos.sh --packages dist/packages --out repo \
#       --key <fingerprint> --public-key scripts/package/linux-packages/packages-key.asc \
#       [--passphrase-file <file>] [--site https://reins2fa.com]
#
# --public-key is the key users trust: --key must be that key (or a subkey of it), or nothing is built. Needs gpg,
# gpgv, apt-ftparchive (apt-utils), createrepo_c, rpmsign and rpmkeys (rpm).
set -euo pipefail

die() {
    echo "linux-repos: $*" >&2
    exit 1
}

packages="" out="" key="" public="" passphrase_file="" site="https://reins2fa.com"
while (($#)); do
    case "$1" in
    --packages) packages="$2" ;;
    --out) out="$2" ;;
    --key) key="$2" ;;
    --public-key) public="$2" ;;
    --passphrase-file) passphrase_file="$2" ;;
    --site) site="${2%/}" ;;
    *) die "unknown argument $1" ;;
    esac
    shift 2
done
[[ -d "$packages" ]] || die "--packages must be the directory with the .deb and .rpm files"
[[ -n "$out" ]] || die "--out is required"
[[ "$key" =~ ^[0-9A-Fa-f]{40}$ ]] || die "--key must be the signing key's fingerprint (40 hex digits)"
[[ -f "$public" ]] || die "--public-key must be the repositories' public key (armored)"
[[ -z "$passphrase_file" || -f "$passphrase_file" ]] || die "--passphrase-file $passphrase_file does not exist"
[[ -n "${GNUPGHOME:-}" && -d "$GNUPGHOME" ]] || die "GNUPGHOME must be the keyring with the secret key"
for tool in gpg gpgv apt-ftparchive createrepo_c rpmsign rpmkeys; do
    command -v "$tool" >/dev/null || die "$tool is needed"
done
packages="$(cd "$packages" && pwd)"
mkdir -p "$out"
out="$(cd "$out" && pwd)"
repo="$out/packages"
rm -rf "$repo"
mkdir -p "$repo"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# ── the key ───────────────────────────────────────────────────────────────────────────────────────────────────────────
sign_args=(--batch --yes --local-user "$key" --digest-algo SHA512)
if [[ -n "$passphrase_file" ]]; then
    sign_args+=(--pinentry-mode loopback --passphrase-file "$passphrase_file")
fi
# The published key is --public-key, dearmored for apt (/etc/apt/keyrings/*.gpg) and as is for rpm and dnf.
gpg --batch --dearmor <"$public" >"$repo/reins.gpg" 2>/dev/null || die "$public is not an OpenPGP key"
cp "$public" "$repo/reins.asc"
fingerprints() { # the primary key and subkey fingerprints in a keyring file
    gpg --batch --with-colons --show-keys "$1" 2>/dev/null | awk -F: '$1 == "fpr" { print $10 }'
}
grep -qix "$key" <<<"$(fingerprints "$repo/reins.gpg")" || die "--key $key is not $public (or one of its subkeys)"
# A signature made now must verify against the published key: the secret key is there and is that key.
echo probe >"$work/probe"
gpg "${sign_args[@]}" --detach-sign --output "$work/probe.sig" "$work/probe" 2>"$work/gpg.log" ||
    die "cannot sign with $key: $(cat "$work/gpg.log")"
gpgv --keyring "$repo/reins.gpg" "$work/probe.sig" "$work/probe" 2>/dev/null ||
    die "a signature by $key does not verify with $public"

# ── RPM: sign the packages, then the repository ───────────────────────────────────────────────────────────────────────
shopt -s nullglob
rpms=("$packages"/*.rpm)
debs=("$packages"/*.deb)
shopt -u nullglob
((${#rpms[@]})) || die "no .rpm in $packages"
((${#debs[@]})) || die "no .deb in $packages"

# rpm signs with gpg; the passphrase (if any) goes the same way as above.
rpm_sign_args=(--define "_gpg_name $key" --define "_gpg_path $GNUPGHOME" --define "_gpg_digest_algo sha512")
if [[ -n "$passphrase_file" ]]; then
    rpm_sign_args+=(--define "_gpg_sign_cmd_extra_args --pinentry-mode loopback --passphrase-file $passphrase_file")
fi
mkdir -p "$work/rpmdb" "$work/rpmkeys"
rpmkeys_args=(--dbpath "$work/rpmdb" --define "_keyringpath $work/rpmkeys")
rpmkeys "${rpmkeys_args[@]}" --import "$repo/reins.asc" || die "rpmkeys cannot import $public"
for rpm in "${rpms[@]}"; do
    name="$(basename "$rpm")"
    [[ "$name" =~ ^[a-z0-9-]+-[0-9.]+-[0-9]+\.(x86_64|aarch64)\.rpm$ ]] || die "unexpected package name $name"
    arch="${BASH_REMATCH[1]}"
    mkdir -p "$repo/rpm/$arch"
    cp "$rpm" "$repo/rpm/$arch/$name"
    # A package signed before (a retried run) is signed again: --addsign replaces nothing, --resign does.
    rpmsign --resign "${rpm_sign_args[@]}" "$repo/rpm/$arch/$name" >"$work/rpmsign.log" 2>&1 ||
        die "rpmsign failed for $name: $(cat "$work/rpmsign.log")"
    check="$(rpmkeys "${rpmkeys_args[@]}" --checksig "$repo/rpm/$arch/$name" 2>&1)" || die "$name: $check"
    [[ "$check" == *signatures* && "$check" != *NOT* ]] || die "$name is not signed with $key: $check"
done
# No sqlite databases (dnf reads the XML) and gzip, which every dnf and zypper reads.
createrepo_c --quiet --no-database --general-compress-type gz "$repo/rpm" >/dev/null || die "createrepo_c failed"
gpg "${sign_args[@]}" --armor --detach-sign --output "$repo/rpm/repodata/repomd.xml.asc" "$repo/rpm/repodata/repomd.xml"
gpgv --keyring "$repo/reins.gpg" "$repo/rpm/repodata/repomd.xml.asc" "$repo/rpm/repodata/repomd.xml" 2>/dev/null ||
    die "repomd.xml.asc does not verify"
cat >"$repo/rpm/reins.repo" <<REPO
[reins]
name=Reins
baseurl=$site/releases/packages/rpm
enabled=1
gpgcheck=1
repo_gpgcheck=1
gpgkey=$site/releases/packages/reins.asc
REPO

# ── APT: one suite, stable, one component, main ───────────────────────────────────────────────────────────────────────
apt="$repo/apt"
architectures=()
for deb in "${debs[@]}"; do
    name="$(basename "$deb")"
    [[ "$name" =~ ^([a-z0-9-]+)_[0-9.]+_(amd64|arm64)\.deb$ ]] || die "unexpected package name $name"
    package="${BASH_REMATCH[1]}" arch="${BASH_REMATCH[2]}"
    mkdir -p "$apt/pool/main/${package:0:1}/$package"
    cp "$deb" "$apt/pool/main/${package:0:1}/$package/$name"
    [[ " ${architectures[*]} " == *" $arch "* ]] || architectures+=("$arch")
done
mapfile -t architectures < <(printf '%s\n' "${architectures[@]}" | sort)
suite="$apt/dists/stable"
for arch in "${architectures[@]}"; do
    mkdir -p "$suite/main/binary-$arch"
    # Filename: is relative to the repository's root (the URI of the sources entry).
    (cd "$apt" && apt-ftparchive --arch "$arch" packages pool/main) >"$suite/main/binary-$arch/Packages" ||
        die "apt-ftparchive packages failed"
    [[ -s "$suite/main/binary-$arch/Packages" ]] || die "no packages for $arch"
    gzip -9n <"$suite/main/binary-$arch/Packages" >"$suite/main/binary-$arch/Packages.gz"
done
# No Valid-Until: a repository that has not changed for a while (no release) is still valid.
apt-ftparchive \
    -o APT::FTPArchive::Release::Origin=Reins \
    -o APT::FTPArchive::Release::Label=Reins \
    -o APT::FTPArchive::Release::Suite=stable \
    -o APT::FTPArchive::Release::Codename=stable \
    -o APT::FTPArchive::Release::Architectures="${architectures[*]}" \
    -o APT::FTPArchive::Release::Components=main \
    -o APT::FTPArchive::Release::Description="Reins: phone approval for AI agents ($site)" \
    release "$suite" >"$work/Release" || die "apt-ftparchive release failed"
mv "$work/Release" "$suite/Release"
gpg "${sign_args[@]}" --clearsign --output "$suite/InRelease" "$suite/Release"
gpg "${sign_args[@]}" --armor --detach-sign --output "$suite/Release.gpg" "$suite/Release"
gpgv --keyring "$repo/reins.gpg" "$suite/InRelease" 2>/dev/null || die "InRelease does not verify"
gpgv --keyring "$repo/reins.gpg" "$suite/Release.gpg" "$suite/Release" 2>/dev/null || die "Release.gpg does not verify"

echo "Signed repositories in $repo (key $key):"
(cd "$out" && find packages -type f | sort | sed 's/^/  /')
