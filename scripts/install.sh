#!/bin/sh
# Installs the Rewarden desktop app:  curl -fsSL <site>/install.sh | sh
#
# Downloads the latest release for this computer, checks it against the published SHA-256, installs it to
# ~/.local/bin (REWARDEN_INSTALL_DIR to change), and restarts the background service if it is installed.
# Later updates: `rewarden update` (which also checks the release signature). Linux and macOS (Apple silicon and Intel);
# on macOS a download made with curl carries no quarantine flag, so Gatekeeper lets the (unsigned) program run.
set -eu

# The site to install from (REWARDEN_RELEASES overrides the releases URL). scripts/release-desktop.sh rewrites this line
# to the site it publishes to, so the installer it serves always points back at that site.
DEFAULT_SITE="https://reins2fa.com"
RELEASES="${REWARDEN_RELEASES:-$DEFAULT_SITE/releases}"
INSTALL_DIR="${REWARDEN_INSTALL_DIR:-$HOME/.local/bin}"

say() { printf '%s\n' "$*"; }
fail() {
    printf 'rewarden install: %s\n' "$*" >&2
    exit 1
}

case "$(uname -s)" in
Linux) os=linux ;;
Darwin) os=macos ;;
*) fail "this system ($(uname -s)) is not supported yet" ;;
esac
case "$(uname -m)" in
x86_64 | amd64) arch=x86_64 ;;
aarch64 | arm64) arch=aarch64 ;;
*) fail "this processor ($(uname -m)) is not supported yet" ;;
esac
platform="$os-$arch"

if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -qO "$2" "$1"; }
else
    fail "curl or wget is needed"
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

fetch "$RELEASES/latest-$platform.txt" "$tmp/latest" || fail "there is no release for $platform yet"
file="" sha="" version="" build=""
read -r file sha version build <"$tmp/latest" || true
case "$file" in "" | .* | *[!A-Za-z0-9._-]*) fail "the release list is malformed" ;; esac
case "$sha" in *[!0-9a-f]*) fail "the release list is malformed" ;; esac
[ "${#sha}" -eq 64 ] || fail "the release list is malformed"

say "Downloading rewarden $version ($build) for $platform..."
fetch "$RELEASES/files/$file" "$tmp/rewarden" || fail "the download failed"
if command -v sha256sum >/dev/null 2>&1; then
    got="$(sha256sum "$tmp/rewarden" | cut -d' ' -f1)"
elif command -v shasum >/dev/null 2>&1; then
    got="$(shasum -a 256 "$tmp/rewarden" | cut -d' ' -f1)"
else
    fail "sha256sum or shasum is needed to check the download"
fi
[ "$got" = "$sha" ] || fail "the download does not match the published checksum; nothing was installed"
chmod 755 "$tmp/rewarden"
"$tmp/rewarden" --version >/dev/null 2>&1 || fail "the downloaded program does not run on this computer"

was_installed=false
[ -x "$INSTALL_DIR/rewarden" ] && was_installed=true
mkdir -p "$INSTALL_DIR"
cp "$tmp/rewarden" "$INSTALL_DIR/.rewarden.new"
mv -f "$INSTALL_DIR/.rewarden.new" "$INSTALL_DIR/rewarden"
say "Installed $("$INSTALL_DIR/rewarden" --version) at $INSTALL_DIR/rewarden"

if [ "$os" = linux ] && [ -f "$HOME/.config/systemd/user/rewarden.service" ]; then
    systemctl --user try-restart rewarden.service 2>/dev/null && say "Restarted the background service."
elif [ "$os" = macos ] && [ -f "$HOME/Library/LaunchAgents/dev.rewarden.daemon.plist" ]; then
    launchctl kickstart -k "gui/$(id -u)/dev.rewarden.daemon" 2>/dev/null && say "Restarted the background service."
fi

case ":$PATH:" in
*":$INSTALL_DIR:"*) ;;
*)
    say ""
    say "$INSTALL_DIR is not on your PATH. Add it, for example:"
    if [ "$os" = macos ]; then
        # zsh is the shell macOS sets up for new accounts.
        say "  echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> ~/.zshrc   # then open a new terminal"
    else
        say "  echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> ~/.bashrc   # or ~/.zshrc"
    fi
    ;;
esac

if [ "$was_installed" = false ]; then
    say ""
    say "Next:"
    say "  rewarden login"
    say "      sign in (a self-hosted server: rewarden login https://your.server); your phone shows a key: approve only"
    say "      if it matches the one printed here"
    say "  rewarden resume"
    say "      start the background service and send GitHub git through it (rewarden pause undoes it)"
fi
