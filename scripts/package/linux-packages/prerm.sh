#!/bin/sh
# reins about to be removed (.deb prerm "remove", .rpm %preun with none left; not on upgrades). What `reins resume` set
# up is each user's own (their git settings and systemd user unit), so it is left alone; say what remains.
set -e

case "$1" in
remove | 0)
    echo "reins: users who ran \`reins resume\` still have git set to go through the Reins background service, which"
    echo "reins: stops with this removal. \`reins pause\` and \`reins service uninstall\` (as each user, before removing)"
    echo "reins: undo that; afterwards, remove the url.\"http://127.0.0.1:...\".insteadOf entries from their git config."
    ;;
esac

exit 0
