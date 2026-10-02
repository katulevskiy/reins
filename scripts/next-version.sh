#!/usr/bin/env bash
# Computes the next release version from the commits since the latest release tag (vX.Y.Z reachable from the commit).
# Tags are the source of truth: nothing in the tree carries the version.
#
#   scripts/next-version.sh                    # prints the next version (e.g. 0.2.0), or nothing when no release is due
#   scripts/next-version.sh --bump minor       # forces the bump level (major, minor or patch)
#   scripts/next-version.sh --rev <commit>     # computes for <commit> instead of HEAD
#   scripts/next-version.sh --describe         # key=value lines: version, tag, previous, level (for $GITHUB_OUTPUT)
#
# How the level is computed (Conventional Commits, https://www.conventionalcommits.org):
#   * a `!` after the type or scope (`feat!: ...`, `desktop(git)!: ...`) or a `BREAKING CHANGE:` footer -> major;
#     while the major version is 0, a breaking change bumps the minor version instead
#   * `feat: ...` or `feat(scope): ...` -> minor
#   * anything else (fix, perf, docs, ci, chore, refactor, test, and this repository's `area: message` style) -> patch
#   * a commit whose message contains `[skip release]` does not count; when nothing counts, no release is due
# Merge commits are ignored (the commits they bring in count on their own). The first release, with no tag yet, is
# 0.1.0; with --bump, the level applies to 0.0.0.
set -euo pipefail

usage() {
    sed -n '2,8p' "$0" | sed 's/^# \{0,1\}//' >&2
    exit 2
}

bump="" rev="HEAD" describe=false
while (($#)); do
    case "$1" in
    --bump)
        bump="${2:-}"
        shift 2 || usage
        ;;
    --bump=*)
        bump="${1#*=}"
        shift
        ;;
    --rev)
        rev="${2:-}"
        shift 2 || usage
        ;;
    --rev=*)
        rev="${1#*=}"
        shift
        ;;
    --describe)
        describe=true
        shift
        ;;
    -h | --help) usage ;;
    *) usage ;;
    esac
done
case "$bump" in
"" | major | minor | patch) ;;
*)
    echo "--bump must be major, minor or patch, not '$bump'" >&2
    exit 2
    ;;
esac

commit="$(git rev-parse --verify --quiet "$rev^{commit}")" || {
    echo "not a commit: $rev" >&2
    exit 2
}

# The highest vX.Y.Z tag reachable from the commit.
previous="$(git tag --merged "$commit" --list 'v*' | grep -E '^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$' |
    sort -V | tail -n 1 || true)"

if [[ -n "$previous" ]]; then
    range="$previous..$commit"
    IFS=. read -r major minor patch <<<"${previous#v}"
else
    range="$commit"
    major=0 minor=0 patch=0
fi

# The level each counting commit asks for: 3 major, 2 minor, 1 patch.
level_of() {
    local message="$1" subject
    subject="${message%%$'\n'*}"
    if [[ "$message" == *"[skip release]"* ]]; then
        echo 0
    elif [[ "$subject" =~ ^[A-Za-z0-9_.-]+(\([^\)]*\))?!: ]] ||
        grep -qE '^BREAKING[ -]CHANGE:' <<<"$message"; then
        echo 3
    elif [[ "$subject" =~ ^feat(\([^\)]*\))?: ]]; then
        echo 2
    else
        echo 1
    fi
}

level=0 commits=0
while IFS= read -r -d '' message || [[ -n "$message" ]]; do
    commits=$((commits + 1))
    l="$(level_of "$message")"
    ((l > level)) && level=$l
done < <(git log -z --no-merges --format=%B "$range")

if [[ -n "$bump" ]]; then
    # Forcing releases even commits marked [skip release], but still needs something new since the last release.
    level=0
    if ((commits > 0)); then
        case "$bump" in
        major) level=3 ;;
        minor) level=2 ;;
        patch) level=1 ;;
        esac
    fi
elif [[ -z "$previous" ]] && ((level > 0)); then
    level=2 # The first release is 0.1.0.
elif ((level == 3 && major == 0)); then
    level=2
fi

version=""
level_name="none"
case "$level" in
3) version="$((major + 1)).0.0" level_name=major ;;
2) version="$major.$((minor + 1)).0" level_name=minor ;;
1) version="$major.$minor.$((patch + 1))" level_name=patch ;;
esac

if $describe; then
    echo "version=$version"
    echo "tag=${version:+v$version}"
    echo "previous=$previous"
    echo "level=$level_name"
elif [[ -n "$version" ]]; then
    echo "$version"
fi
