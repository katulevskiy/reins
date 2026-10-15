#!/usr/bin/env bash
# Type-checks the iOS app's Swift on Linux against the iPhoneOS SDK that xtool installs (no Mac, no linking).
#
#   scripts/ios-typecheck.sh [--release] [--warnings] [target...]
#
# Targets: Reins ReinsWidgets ReinsNotifications ReinsTests ReinsUITests (default: all). See ios/README.md,
# "Compile check on Linux", for what it covers and what it does not. -v prints the swiftc command lines;
# IOS_TYPECHECK_SWIFTFLAGS adds flags to them.
exec python3 -I "$(dirname "$(readlink -f "$0")")/ios-typecheck/typecheck.py" "$@"
