#!/bin/sh
# Installs the Rewarden desktop app:  curl -fsSL <site>/install.sh | sh
#
# Downloads the latest release for this computer, checks it against the published SHA-256, installs it to
# ~/.local/bin (REWARDEN_INSTALL_DIR to change), and restarts the background service if it is installed.
# Later updates: `rewarden update` (which also checks the release signature). Linux and macOS (Apple silicon and Intel);
# on macOS a download made with curl carries no quarantine flag, so Gatekeeper lets the (unsigned) program run.
#
# On a computer with a screen it then offers the Reins app (the menu bar / tray app that does the rest with a window;
# REINS_APP=yes or no answers for you; on a Mac it goes to /Applications, or REINS_APP_DIR). Without the app it finishes the setup here: pairs with your phone (when not
# paired yet), adds Reins to every AI harness it finds (Claude Code, Codex, Gemini CLI, Cursor) and starts the
# background service with git going through it. REWARDEN_NO_SETUP=1 skips that; REWARDEN_SERVER pairs with another
# server than the default one.
set -eu

# The site to install from (REWARDEN_RELEASES overrides the releases URL). scripts/release-desktop.sh rewrites this line
# to the site it publishes to, so the installer it serves always points back at that site.
DEFAULT_SITE="https://rewarden.arc-chat.com"
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

# The Reins app, and the first-time setup.

# The server to pair with, and where the app comes from (the release's assets, under the names "latest" links use).
PAIR_SERVER="${REWARDEN_SERVER:-https://app.reins2fa.com}"
APP_RELEASES="${REINS_APP_RELEASES:-https://github.com/katulevskiy/reins/releases/latest/download}"
rewarden="$INSTALL_DIR/rewarden"

# A terminal to ask in (the script itself arrives on stdin with `curl | sh`).
interactive=false
if [ -r /dev/tty ] && [ -w /dev/tty ] && { : </dev/tty; } 2>/dev/null; then
    interactive=true
fi
ask() { # question; true for yes
    printf '%s [Y/n] ' "$1" >/dev/tty
    answer=""
    read -r answer </dev/tty || answer=n
    case "$answer" in "" | y | Y | yes | Yes) return 0 ;; *) return 1 ;; esac
}

has_screen=false
if [ "$os" = macos ]; then
    [ -z "${SSH_CONNECTION:-}" ] && has_screen=true
elif [ -n "${DISPLAY:-}" ] || [ -n "${WAYLAND_DISPLAY:-}" ]; then
    has_screen=true
fi

# check_sum FILE NAME: FILE matches NAME's line in the release's SHA256SUMS.
check_sum() {
    fetch "$APP_RELEASES/SHA256SUMS" "$tmp/SHA256SUMS" || return 1
    want="$(awk -v n="$2" '$2 == n || $2 == "*" n { print $1; exit }' "$tmp/SHA256SUMS")"
    [ -n "$want" ] || return 1
    if command -v sha256sum >/dev/null 2>&1; then
        have="$(sha256sum "$1" | cut -d' ' -f1)"
    else
        have="$(shasum -a 256 "$1" | cut -d' ' -f1)"
    fi
    [ "$have" = "$want" ]
}

app_installed=false
install_app_macos() {
    say "Downloading the Reins app..."
    fetch "$APP_RELEASES/Reins-macOS.dmg" "$tmp/Reins.dmg" || { say "The Reins app is not published yet."; return 1; }
    check_sum "$tmp/Reins.dmg" Reins-macOS.dmg || { say "The app download does not match the published checksum."; return 1; }
    mnt="$tmp/mnt"
    mkdir -p "$mnt"
    hdiutil attach -quiet -nobrowse -readonly -mountpoint "$mnt" "$tmp/Reins.dmg" || return 1
    apps="${REINS_APP_DIR:-/Applications}"
    [ -w "$apps" ] || [ -n "${REINS_APP_DIR:-}" ] || apps="$HOME/Applications"
    mkdir -p "$apps"
    rm -rf "$apps/Reins.app"
    if ! ditto "$mnt/Reins.app" "$apps/Reins.app"; then
        hdiutil detach -quiet "$mnt" || true
        return 1
    fi
    hdiutil detach -quiet "$mnt" || true
    say "Installed the Reins app at $apps/Reins.app"
    open "$apps/Reins.app" || true
}
install_app_linux() {
    case "$arch" in x86_64 | aarch64) ;; *) return 1 ;; esac
    name="Reins-Linux-$arch.AppImage"
    say "Downloading the Reins app..."
    fetch "$APP_RELEASES/$name" "$tmp/Reins.AppImage" || { say "The Reins app is not published for $arch yet."; return 1; }
    check_sum "$tmp/Reins.AppImage" "$name" || { say "The app download does not match the published checksum."; return 1; }
    chmod 755 "$tmp/Reins.AppImage"
    cp "$tmp/Reins.AppImage" "$INSTALL_DIR/.Reins.AppImage.new"
    mv -f "$INSTALL_DIR/.Reins.AppImage.new" "$INSTALL_DIR/Reins.AppImage"
    data="${XDG_DATA_HOME:-$HOME/.local/share}"
    mkdir -p "$data/applications" "$data/icons/hicolor/256x256/apps"
    (cd "$tmp" && "$INSTALL_DIR/Reins.AppImage" --appimage-extract usr/share/icons/hicolor/256x256/apps/reins.png >/dev/null 2>&1) &&
        cp "$tmp/squashfs-root/usr/share/icons/hicolor/256x256/apps/reins.png" "$data/icons/hicolor/256x256/apps/reins.png" || true
    cat >"$data/applications/reins.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Reins
Comment=Keeps your AI agents on a leash
Exec="$INSTALL_DIR/Reins.AppImage"
Icon=reins
Terminal=false
Categories=Development;Utility;
DESKTOP
    say "Installed the Reins app at $INSTALL_DIR/Reins.AppImage (in your applications menu as Reins)"
    (nohup "$INSTALL_DIR/Reins.AppImage" >/dev/null 2>&1 &) || true
}

want_app="${REINS_APP:-}"
if [ -z "$want_app" ] && [ "$has_screen" = true ] && [ "$interactive" = true ]; then
    say ""
    say "The Reins app shows a QR code to pair with your phone, adds Reins to your AI tools, and lives in your"
    say "$([ "$os" = macos ] && echo "menu bar" || echo "tray") afterwards."
    if ask "Install the Reins app too?"; then want_app=yes; else want_app=no; fi
fi
case "$want_app" in
yes | 1 | true)
    if [ "$os" = macos ]; then install_app_macos && app_installed=true; else install_app_linux && app_installed=true; fi
    ;;
esac

if [ "$app_installed" = true ]; then
    say ""
    say "Reins is open: finish in its window (pair with your phone, then pick your AI tools)."
elif [ "${REWARDEN_NO_SETUP:-}" = 1 ] || [ "$interactive" = false ]; then
    if [ "$was_installed" = false ]; then
        say ""
        say "Next:"
        say "  rewarden login $PAIR_SERVER"
        say "      pair with your phone; it shows a key: approve only if it matches the one printed here"
        say "  rewarden harness add --all"
        say "      add Reins to every AI harness on this computer"
        say "  rewarden resume"
        say "      start the background service and send GitHub git through it (rewarden pause undoes it)"
    fi
else
    say ""
    if "$rewarden" status 2>/dev/null | grep -q '^Server: *not logged in'; then
        say "Pair this computer with your phone (open Reins on the phone and scan the code, or follow the link):"
        "$rewarden" login "$PAIR_SERVER" </dev/tty || fail "not paired; run \`rewarden login $PAIR_SERVER\` to try again"
    fi
    say ""
    "$rewarden" harness add --all || say "Could not add Reins to every harness; see \`rewarden harness list\`."
    say ""
    "$rewarden" resume || fail "the background service did not start; see \`rewarden status\`"
    say ""
    say "Done. Restart your AI tools so they pick up Reins."
fi
