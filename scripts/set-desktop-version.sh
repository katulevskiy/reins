#!/usr/bin/env bash
# Sets the version of the desktop app (crates/rewarden-desktop and its Cargo.lock entry) for a release build, so
# `rewarden --version` reports the release. The release workflow runs it on its checkout; nothing is committed (tags
# are the source of truth for versions). Afterwards `cargo build --locked` still works.
#
#   scripts/set-desktop-version.sh 0.2.0
set -euo pipefail

version="${1:-}"
[[ "$version" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] || {
    echo "usage: $0 X.Y.Z" >&2
    exit 2
}
cd "$(dirname "${BASH_SOURCE[0]}")/.."

manifest=crates/rewarden-desktop/Cargo.toml
export VERSION="$version"
# The package's own version is the first `version = ` at the start of a line ([package] comes first); in Cargo.lock
# the version line follows the package's name line. perl, not sed: the same on Linux and macOS.
perl -0pi -e 's/^version = "[^"]*"/version = "$ENV{VERSION}"/m' "$manifest"
perl -0pi -e 's/^(name = "rewarden-desktop"\nversion = )"[^"]*"/$1"$ENV{VERSION}"/m' Cargo.lock

grep -qx "version = \"$version\"" "$manifest" || {
    echo "could not set the version in $manifest" >&2
    exit 1
}
grep -A1 -x 'name = "rewarden-desktop"' Cargo.lock | grep -qx "version = \"$version\"" || {
    echo "could not set the version in Cargo.lock" >&2
    exit 1
}
echo "rewarden-desktop $version"
