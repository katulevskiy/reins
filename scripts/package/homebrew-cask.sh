#!/usr/bin/env bash
# Writes the Homebrew cask for the Reins desktop app (katulevskiy/homebrew-tap, Casks/reins.rb) for one release, from
# that release's SHA256SUMS: `brew install --cask katulevskiy/tap/reins` installs Reins.app and links the `reins` command
# that comes inside it. The release workflow's `homebrew` job runs it after publishing and pushes the result to the tap.
#
#   scripts/package/homebrew-cask.sh --version 0.2.6 --sums SHA256SUMS --out tap/Casks/reins.rb
#
# The app updates itself (its one-click update), so the cask says `auto_updates true`: Homebrew then leaves an
# installed app alone instead of reinstalling over it, and `brew upgrade --greedy` still moves it to the newest release.
set -euo pipefail

die() {
    echo "homebrew-cask: $*" >&2
    exit 1
}

version="" sums="" out=""
while (($#)); do
    case "$1" in
    --version) version="$2" ;;
    --sums) sums="$2" ;;
    --out) out="$2" ;;
    *) die "unknown argument $1" ;;
    esac
    shift 2
done
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "--version X.Y.Z is required"
[[ -f "$sums" ]] || die "--sums must be the release's SHA256SUMS"
[[ -n "$out" ]] || die "--out is required"

dmg="Reins-$version-macOS.dmg"
sha="$(awk -v f="$dmg" '$2 == f || $2 == "*" f { print $1 }' "$sums")"
[[ "$sha" =~ ^[0-9a-f]{64}$ ]] || die "$sums does not list $dmg"

mkdir -p "$(dirname "$out")"
cat >"$out" <<EOF
# Written by scripts/package/homebrew-cask.sh in katulevskiy/reins for each release; do not edit by hand.
cask "reins" do
  version "$version"
  sha256 "$sha"

  url "https://github.com/katulevskiy/reins/releases/download/v#{version}/Reins-#{version}-macOS.dmg",
      verified: "github.com/katulevskiy/reins/"
  name "Reins"
  desc "Approve on your phone what your AI agents do with your accounts"
  homepage "https://reins2fa.com/"

  livecheck do
    url :url
    strategy :github_latest
  end

  auto_updates true
  depends_on macos: ">= :monterey"

  app "Reins.app"
  binary "#{appdir}/Reins.app/Contents/MacOS/reins"

  uninstall launchctl: "dev.reins.daemon",
            quit:      "com.reins2fa.desktop"

  zap trash: [
    "~/.config/reins",
    "~/.local/state/reins",
    "~/Library/LaunchAgents/dev.reins.daemon.plist",
    "~/Library/Logs/reins.log",
  ]
end
EOF
echo "Wrote $out for Reins $version ($dmg, $sha)"
