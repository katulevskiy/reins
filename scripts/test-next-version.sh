#!/usr/bin/env bash
# Tests scripts/next-version.sh and scripts/release-notes.sh against throwaway git repositories. Run from anywhere:
#
#   scripts/test-next-version.sh
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
next="$here/next-version.sh"
notes="$here/release-notes.sh"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

failures=0 count=0
check() { # check <name> <expected> <actual>
    count=$((count + 1))
    if [[ "$2" == "$3" ]]; then
        echo "ok   $1"
    else
        echo "FAIL $1: expected '$2', got '$3'"
        failures=$((failures + 1))
    fi
}
contains() { # contains <name> <needle> <haystack>
    count=$((count + 1))
    if [[ "$3" == *"$2"* ]]; then
        echo "ok   $1"
    else
        printf 'FAIL %s: %q not in:\n%s\n' "$1" "$2" "$3"
        failures=$((failures + 1))
    fi
}

repo=0
new_repo() {
    repo=$((repo + 1))
    cd "$work" && mkdir "r$repo" && cd "r$repo"
    git init -q -b main
    git config user.name test
    git config user.email test@example.com
    git config commit.gpgsign false
    git config tag.gpgsign false
}
commit() { git commit -q --allow-empty -m "$1"; }
next_version() { "$next" "$@"; }

# No tag yet: the first release is 0.1.0, whatever the commits say.
new_repo
check "empty history has no release" "" "$(next_version 2>/dev/null || true)"
commit "desktop: first"
check "first release" "0.1.0" "$(next_version)"
commit "feat!: breaking from the start"
check "first release ignores breaking" "0.1.0" "$(next_version)"
check "describe without a tag" $'version=0.1.0\ntag=v0.1.0\nprevious=\nlevel=minor' "$(next_version --describe)"
check "forced major without a tag" "1.0.0" "$(next_version --bump major)"
check "forced patch without a tag" "0.0.1" "$(next_version --bump patch)"

# Levels after a tag.
new_repo
commit "init"
git tag v0.1.0
check "nothing since the tag" "" "$(next_version)"
check "describe with nothing due" $'version=\ntag=\nprevious=v0.1.0\nlevel=none' "$(next_version --describe)"
check "forcing needs a new commit" "" "$(next_version --bump minor)"
commit "android: fix the settings screen"
check "area style is a patch" "0.1.1" "$(next_version)"
commit "fix(server): handle empty bodies"
check "fix is a patch" "0.1.1" "$(next_version)"
commit "feat(desktop): macOS builds"
check "feat with scope is a minor" "0.2.0" "$(next_version)"
commit "docs: typo"
check "lower levels do not lower it" "0.2.0" "$(next_version)"
commit "refactor!: new wire format"
check "breaking while major is 0 is a minor" "0.2.0" "$(next_version)"
check "forced major" "1.0.0" "$(next_version --bump major)"
check "forced patch" "0.1.1" "$(next_version --bump=patch)"
git tag v1.0.0
commit "perf: faster"
check "patch after 1.0.0" "1.0.1" "$(next_version)"
commit "feat: something"
check "minor after 1.0.0" "1.1.0" "$(next_version)"
commit "desktop(git)!: drop the old proxy"
check "breaking area with scope is a major" "2.0.0" "$(next_version)"
check "describe" $'version=2.0.0\ntag=v2.0.0\nprevious=v1.0.0\nlevel=major' "$(next_version --describe)"
check "rev computes for an older commit" "1.0.1" "$(next_version --rev HEAD~2)"

# The BREAKING CHANGE footer.
new_repo
commit "init"
git tag v1.2.3
git commit -q --allow-empty -m "fix: a fix" -m "BREAKING CHANGE: the config moved"
check "breaking footer" "2.0.0" "$(next_version)"
new_repo
commit "init"
git tag v1.2.3
git commit -q --allow-empty -m "fix: mentions a BREAKING CHANGE: inline only"
check "breaking only counts as a footer line" "1.2.4" "$(next_version)"
git commit -q --allow-empty -m "chore: tidy" -m "BREAKING-CHANGE: also accepted"
check "breaking footer with a hyphen" "2.0.0" "$(next_version)"

# What does not count.
new_repo
commit "init"
git tag v0.3.0
commit "docs: wording [skip release]"
check "skip release only" "" "$(next_version)"
check "skip release is forced with --bump" "0.3.1" "$(next_version --bump patch)"
git commit -q --allow-empty -m "feat: big" -m "Not yet. [skip release]"
check "skip release in the body" "" "$(next_version)"
commit "ci: real change"
check "a counting commit after skipped ones" "0.3.1" "$(next_version)"
commit "featured: not a feat"
check "a type that only starts with feat" "0.3.1" "$(next_version)"
commit "feat : not conventional"
check "a space before the colon is not a feat" "0.3.1" "$(next_version)"

# Merges, tag choice.
new_repo
commit "init"
git tag v0.9.0
git tag v0.10.0
git tag not-a-version
git tag v2.0.0-rc.1
commit "fix: x"
check "highest tag by semver, not by text" "0.10.1" "$(next_version)"
git checkout -q -b side
commit "feat: on a branch"
git checkout -q main
git merge -q --no-ff side -m "Merge branch 'side'"
check "merged commits count, merges themselves not" "0.11.0" "$(next_version)"
git checkout -q -b other HEAD~1
commit "elsewhere"
git tag v5.0.0
git checkout -q main
check "tags that are not ancestors are ignored" "0.11.0" "$(next_version)"
check "bad bump" "2" "$(next_version --bump huge >/dev/null 2>&1 || echo $?)"
check "bad rev" "2" "$(next_version --rev nope >/dev/null 2>&1 || echo $?)"

# Release notes.
new_repo
commit "init"
git tag v0.1.0
commit "feat(desktop): macOS builds"
commit "fix: no crash on <empty> input"
commit "android: new icon"
git commit -q --allow-empty -m "feat!: new protocol" -m "BREAKING CHANGE: old phones must update"
commit "docs: wording [skip release]"
out="$("$notes" --version 0.2.0 --previous v0.1.0 --repo https://github.com/o/r)"
sha="$(git rev-parse HEAD~4)"
contains "notes: breaking section" $'## Breaking changes\n\n- new protocol ([`' "$out"
contains "notes: feature with scope" "- **desktop:** macOS builds ([\`${sha:0:7}\`](https://github.com/o/r/commit/$sha))" "$out"
contains "notes: fixes" $'## Fixes\n\n- no crash on \\<empty\\> input' "$out"
contains "notes: other keeps the area" $'## Other\n\n- android: new icon' "$out"
contains "notes: compare link" "https://github.com/o/r/compare/v0.1.0...v0.2.0" "$out"
contains "notes: skipped commits are listed" "docs: wording [skip release]" "$out"
first="$("$notes" --version 0.1.0 --previous "" --repo https://github.com/o/r --rev HEAD~5)"
contains "notes: first release" "- init" "$first"
check "notes: no compare link for the first release" "" "$(grep -F compare <<<"$first" || true)"

echo
if ((failures)); then
    echo "$failures of $count checks failed"
    exit 1
fi
echo "all $count checks passed"
