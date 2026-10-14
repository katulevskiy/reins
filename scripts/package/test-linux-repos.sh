#!/usr/bin/env bash
# Installs Reins from the signed repositories scripts/package/linux-repos.sh built, the way the documentation tells
# users to, in containers of the supported distributions: the key and the sources entry or .repo file over HTTP,
# `apt-get update` (InRelease checked against the key) or dnf with gpgcheck and repo_gpgcheck, then the packages, then
# `reins --version` (and `reins-app --version` on x86_64) must name this build, then removal. The repository is
# served from this machine at http://127.0.0.1:<port>/releases/packages/ (production: <site>/releases/packages/).
#
#   scripts/package/test-linux-repos.sh --repo repo --version 0.2.5 --build 0.2.5-202610090300-fad997a6 \
#       [--images "ubuntu:24.04 fedora:44"]
#
# --repo is linux-repos.sh's --out (the directory holding packages/). Tests this machine's architecture. Needs docker
# (DOCKER=podman for podman) and python3.
set -euo pipefail

die() {
    echo "test-linux-repos: $*" >&2
    exit 1
}

repo="" version="" build="" images="ubuntu:22.04 ubuntu:24.04 ubuntu:26.04 debian:12 debian:13 fedora:44"
while (($#)); do
    case "$1" in
    --repo) repo="$2" ;;
    --version) version="$2" ;;
    --build) build="$2" ;;
    --images) images="$2" ;;
    *) die "unknown argument $1" ;;
    esac
    shift 2
done
[[ -d "$repo/packages" ]] || die "--repo must hold packages/ (linux-repos.sh --out)"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "--version X.Y.Z is required"
[[ "$build" == "$version"-* ]] || die "--build must start with $version-"
docker="${DOCKER:-docker}"
case "$(uname -m)" in
x86_64) app=true ;;
aarch64) app=false ;;
*) die "unsupported architecture $(uname -m)" ;;
esac

serve="$(mktemp -d)"
server=""
cleanup() {
    [[ -n "$server" ]] && kill "$server" 2>/dev/null
    rm -rf "$serve"
}
trap cleanup EXIT
mkdir -p "$serve/releases"
ln -s "$(cd "$repo" && pwd)/packages" "$serve/releases/packages"
port="$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')"
python3 -m http.server --bind 127.0.0.1 --directory "$serve" "$port" >/dev/null 2>&1 &
server=$!
site="http://127.0.0.1:$port"
for _ in $(seq 50); do
    curl -fsS -o /dev/null "$site/releases/packages/reins.asc" 2>/dev/null && break
    sleep 0.1
done
curl -fsS -o /dev/null "$site/releases/packages/reins.asc" || die "the test server did not start"

# The shell run in each container; $1: site, $2: "version (build)", $3: whether the app is tested, $4: sources style.
read -r -d '' debian_test <<'SH' || true
set -eu
site="$1" want="$2" app="$3" style="$4"
export DEBIAN_FRONTEND=noninteractive
# The distribution's mirrors drop connections now and then; apt-get tries each download again.
echo 'Acquire::Retries "5";' >/etc/apt/apt.conf.d/80-retries
apt-get update -qq
apt-get install -y -qq --no-install-recommends ca-certificates curl >/dev/null
# As in docs/linux-packages.md:
install -d -m 0755 /etc/apt/keyrings
curl -fsSL "$site/releases/packages/reins.gpg" -o /etc/apt/keyrings/reins.gpg
if [ "$style" = list ]; then
    echo "deb [signed-by=/etc/apt/keyrings/reins.gpg] $site/releases/packages/apt stable main" \
        >/etc/apt/sources.list.d/reins.list
else
    printf 'Types: deb\nURIs: %s\nSuites: stable\nComponents: main\nSigned-By: /etc/apt/keyrings/reins.gpg\n' \
        "$site/releases/packages/apt" >/etc/apt/sources.list.d/reins.sources
fi
apt-get update 2>&1 | tee /tmp/update.log
if grep -qiE '^(W|E):|NO_PUBKEY|not signed' /tmp/update.log; then echo "apt-get update complained"; exit 1; fi
packages=reins
[ "$app" = true ] && packages="reins reins-app"
# Without the recommended Vulkan driver (large; nothing is drawn here).
apt-get install -y --no-install-recommends $packages
reins --version
reins --version | grep -qF "$want"
if [ "$app" = true ]; then
    reins-app --version | grep -qF "$want"
    test -f /usr/share/applications/reins.desktop
    test -f /usr/share/icons/hicolor/256x256/apps/reins.png
fi
apt-get remove -y $packages
hash -r # the shell remembers where reins was when it ran it above
! command -v reins
SH

read -r -d '' fedora_test <<'SH' || true
set -eu
site="$1" want="$2" app="$3"
# Fedora's Cisco OpenH264 repository is not needed here, and its metalink answering 503 failed the whole install.
rm -f /etc/yum.repos.d/fedora-cisco-openh264.repo
# As in docs/linux-packages.md (the published .repo file, here pointing at the test server):
curl -fsSL "$site/releases/packages/rpm/reins.repo" | sed "s|https://reins2fa.com|$site|g" >/etc/yum.repos.d/reins.repo
cat /etc/yum.repos.d/reins.repo
grep -qx 'gpgcheck=1' /etc/yum.repos.d/reins.repo
grep -qx 'repo_gpgcheck=1' /etc/yum.repos.d/reins.repo
packages=reins
[ "$app" = true ] && packages="reins reins-app"
dnf install -y --setopt=install_weak_deps=False $packages
# gpgcheck=1: dnf installs only packages signed with the key it imported from gpgkey=.
rpm -q --qf '%{NAME}-%{VERSION}-%{RELEASE}.%{ARCH}: %{SIGPGP:pgpsig}\n' $packages
reins --version
reins --version | grep -qF "$want"
if [ "$app" = true ]; then
    reins-app --version | grep -qF "$want"
    test -f /usr/share/applications/reins.desktop
fi
dnf remove -y $packages
hash -r # the shell remembers where reins was when it ran it above
! command -v reins
SH

want="$version ($build)"
for image in $images; do
    case "$image" in
    ubuntu:* | debian:* | fedora:*) ;;
    *) die "no test for $image" ;;
    esac
done
# One image's install, in its own container. A failure is tried once more from scratch: what fails in these
# containers by itself is a distribution mirror; a broken package fails both times.
install_in() {
    local image="$1" script style=sources attempt
    case "$image" in
    # The one-line .list format on the oldest one, deb822 .sources elsewhere.
    ubuntu:22.04) script="$debian_test" style=list ;;
    ubuntu:* | debian:*) script="$debian_test" ;;
    fedora:*) script="$fedora_test" ;;
    esac
    for attempt in 1 2; do
        "$docker" run --rm --network host "$image" sh -c "$script" sh "$site" "$want" "$app" "$style" && return 0
        ((attempt == 2)) || { echo "::warning::$image: installing from the repositories failed; trying again"; sleep 10; }
    done
    return 1
}
# The images run side by side (each mostly waits on its mirror); their logs are shown one after another.
logs="$(mktemp -d)"
trap 'cleanup; rm -rf "$logs"' EXIT
pids=()
for image in $images; do
    install_in "$image" >"$logs/${image//[:\/]/_}.log" 2>&1 &
    pids+=("$!")
done
failed=()
index=0
for image in $images; do
    status=0
    wait "${pids[$index]}" || status=$?
    index=$((index + 1))
    echo "::group::$image ($(uname -m))"
    cat "$logs/${image//[:\/]/_}.log"
    echo "::endgroup::"
    if ((status == 0)); then
        echo "PASS $image"
    else
        echo "FAIL $image"
        failed+=("$image")
    fi
done
((${#failed[@]} == 0)) || die "installing from the repositories failed on: ${failed[*]}"
echo "Installed and ran Reins $want from the repositories on: $images"
