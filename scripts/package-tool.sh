#!/usr/bin/env bash
# Package an already-built organization tool: reins-audit (docs/audit.md) or reins-executor (docs/executor.md), with
# its licences and its guide. Writes dist/<tool>-<version>-<target>.tar.gz and dist/<tool>-<target>.tar.gz.
#
#   scripts/package-tool.sh <tool> <target> <version> [profile]
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
tool="${1:?tool (reins-audit or reins-executor)}"
target="${2:?target triple}"
version="${3:?release version}"
profile="${4:-release}"
case "$tool" in
reins-audit) guide=docs/audit.md ;;
reins-executor) guide=docs/executor.md ;;
*)
    echo "unknown tool: $tool (reins-audit or reins-executor)" >&2
    exit 2
    ;;
esac
[[ "$target" =~ ^[a-zA-Z0-9_-]+$ && "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ && "$profile" =~ ^[a-zA-Z0-9_-]+$ ]]
[[ -s "$guide" ]] || {
    echo "$guide is missing: the $tool archive carries it" >&2
    exit 1
}
name="$tool-$version-$target"
stage="$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/reins-tool.XXXXXX")"
trap 'rm -rf -- "$stage"' EXIT
mkdir -p "$stage/$name" "${REINS_DIST_DIR:-dist}"
dist="$(cd "${REINS_DIST_DIR:-dist}" && pwd)"
cp "${CARGO_TARGET_DIR:-target}/$target/$profile/$tool" LICENSE LICENSE-AGPL LICENSING.md NOTICE "$guide" \
    "$stage/$name/"
tar -C "$stage" -czf "$dist/$name.tar.gz" "$name"
cp "$dist/$name.tar.gz" "$dist/$tool-$target.tar.gz"
