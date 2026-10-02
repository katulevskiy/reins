#!/usr/bin/env bash
# Writes the Markdown release notes for a release: the commits since the previous release, grouped (Breaking changes,
# Features, Fixes, Other) with links to the commits, then how to install it. Used by .github/workflows/release.yml.
#
#   scripts/release-notes.sh --version 0.2.0 --previous v0.1.0 --repo https://github.com/OWNER/REPO \
#       [--rev <commit>] [--image ghcr.io/OWNER/reins-server] [--note "text"]...
#
# --previous may be empty (the first release: every commit). --note adds a paragraph under the title (repeatable).
# The grouping follows scripts/next-version.sh: `type!:` or a `BREAKING CHANGE:` footer is breaking, `feat` a feature,
# `fix` a fix, anything else (including the `area: message` style) other. Merge commits are left out.
set -euo pipefail

version="" previous="" repo="" rev="HEAD" image="" notes=()
while (($#)); do
    case "$1" in
    --version) version="$2" ;;
    --previous) previous="$2" ;;
    --repo) repo="${2%/}" ;;
    --rev) rev="$2" ;;
    --image) image="$2" ;;
    --note) notes+=("$2") ;;
    *)
        echo "unknown argument: $1" >&2
        exit 2
        ;;
    esac
    shift 2
done
[[ -n "$version" && -n "$repo" ]] || {
    echo "usage: $0 --version X.Y.Z --previous vA.B.C|'' --repo https://github.com/OWNER/REPO [--rev REV]" >&2
    exit 2
}
tag="v$version"
image="${image:-ghcr.io/$(basename "$(dirname "$repo")" | tr '[:upper:]' '[:lower:]')/reins-server}"
range="$rev"
[[ -n "$previous" ]] && range="$previous..$rev"

# Markdown-escapes text outside `code spans` (backslashes, emphasis and HTML).
escape() {
    local text="$1" out="" i=0 part
    while [[ -n "$text" ]]; do
        if [[ "$text" == *'`'* ]]; then
            part="${text%%\`*}"
            text="${text#*\`}"
        else
            part="$text"
            text=""
        fi
        if ((i % 2 == 0)); then
            part="${part//\\/\\\\}" part="${part//\*/\\*}" part="${part//</\\<}" part="${part//>/\\>}"
            out+="$part"
        else
            out+="\`$part\`"
        fi
        i=$((i + 1))
    done
    printf '%s' "$out"
}

breaking=() features=() fixes=() other=()
re='^([A-Za-z0-9_.-]+)(\(([^)]*)\))?(!)?: (.*)$'
while IFS= read -r -d '' record || [[ -n "$record" ]]; do
    sha="${record%%$'\n'*}"
    message="${record#*$'\n'}"
    subject="${message%%$'\n'*}"
    link="([\`${sha:0:7}\`]($repo/commit/$sha))"
    type="" scope="" bang="" description="$subject"
    if [[ "$subject" =~ $re ]]; then
        type="${BASH_REMATCH[1]}" scope="${BASH_REMATCH[3]}" bang="${BASH_REMATCH[4]}" description="${BASH_REMATCH[5]}"
    fi
    # Breaking changes, features and fixes drop the type; a scope, or the area of other types, leads in bold.
    label="$scope"
    if [[ -n "$type" && "$type" != feat && "$type" != fix ]]; then
        label="$type${scope:+($scope)}"
    fi
    entry="- ${label:+**$(escape "$label"):** }$(escape "$description") $link"
    if [[ -n "$bang" ]] || grep -qE '^BREAKING[ -]CHANGE:' <<<"$message"; then
        breaking+=("$entry")
    elif [[ "$type" == feat ]]; then
        features+=("$entry")
    elif [[ "$type" == fix ]]; then
        fixes+=("$entry")
    else
        other+=("- $(escape "$subject") $link")
    fi
done < <(git log -z --reverse --no-merges --format='%H%n%B' "$range")

section() { # section <title> <entries...>
    local title="$1"
    shift
    (($#)) || return 0
    printf '## %s\n\n' "$title"
    printf '%s\n' "$@"
    printf '\n'
}

base="$repo/releases/download/$tag"
desktop="reins-desktop-$version-x86_64-unknown-linux-musl"
server="reins-server-$version-x86_64-unknown-linux-gnu"
apk="reins-$version-android.apk"

for note in ${notes[@]+"${notes[@]}"}; do
    printf '> %s\n\n' "$note"
done
section "Breaking changes" ${breaking[@]+"${breaking[@]}"}
section "Features" ${features[@]+"${features[@]}"}
section "Fixes" ${fixes[@]+"${fixes[@]}"}
section "Other" ${other[@]+"${other[@]}"}
if [[ -n "$previous" ]]; then
    printf '**Full changelog:** %s/compare/%s...%s\n\n' "$repo" "$previous" "$tag"
fi
cat <<EOF
## Install

New to Reins? Start with the [quick start]($repo/blob/$tag/docs/quick-start.md).

**Desktop app** (\`rewarden\`): pick the archive for your computer: \`x86_64-unknown-linux-musl\` or
\`aarch64-unknown-linux-musl\` (Linux, static), \`aarch64-apple-darwin\` (Apple silicon Mac) or
\`x86_64-apple-darwin\` (Intel Mac).

\`\`\`sh
curl -fsSLO $base/$desktop.tar.gz
tar xzf $desktop.tar.gz
install -m 755 $desktop/rewarden ~/.local/bin/rewarden
rewarden --version
\`\`\`

On macOS, a binary downloaded with a browser is quarantined; \`curl\` downloads are not (or run
\`xattr -d com.apple.quarantine rewarden\`).

**Server** ([self-hosting guide]($repo/blob/$tag/docs/self-hosting.md)): the Docker image (linux/amd64 and
linux/arm64, with the web vault)

\`\`\`sh
docker pull $image:$version
\`\`\`

or the Linux binary with SQLite, \`$server.tar.gz\` (no web vault; see the guide).

**Android**: download \`$apk\` on the phone and open it (Android asks you to allow installs from your browser once).

**Verify** a download against \`SHA256SUMS\`:

\`\`\`sh
curl -fsSLO $base/SHA256SUMS
sha256sum --ignore-missing -c SHA256SUMS
\`\`\`
EOF
