#!/bin/sh
# reins installed or upgraded (.deb postinst, .rpm %post): restart each logged-in user's background service if it is
# running (their own systemd user unit, reins.service, written by `reins resume`), so that it runs the new program, as
# `reins update` does. Best effort; nothing to do without systemd (a container) or logged-in users.
set -e

if [ -d /run/systemd/system ] && command -v loginctl >/dev/null 2>&1 && command -v systemctl >/dev/null 2>&1; then
    for user in $(loginctl list-users --no-legend 2>/dev/null | awk '{ print $2 }'); do
        case "$user" in "" | *[!A-Za-z0-9._-]*) continue ;; esac
        timeout 15 systemctl --user --machine="$user@" try-restart reins.service >/dev/null 2>&1 || true
    done
fi

exit 0
