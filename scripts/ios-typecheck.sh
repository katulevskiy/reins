#!/usr/bin/env bash
# Type-checks the iOS app's Swift on Linux against the iPhoneOS SDK that xtool installs (no Mac, no linking).
#
#   scripts/ios-typecheck.sh [--release] [--warnings] [target...]
#
# Targets: Reins ReinsWidgets ReinsNotifications ReinsTests ReinsUITests (default: all). See ios/README.md,
# "Compile check on Linux", for what it covers and what it does not. -v prints the swiftc command lines;
# IOS_TYPECHECK_SWIFTFLAGS adds flags to them.
#
# The Rust core and its UniFFI bindings are cached by a content hash of the core's sources, the crates it
# depends on, Cargo.lock, the UniFFI config and the toolchain. The cache is $REINS_IOS_TYPECHECK_CACHE, or
# ~/.cache/reins-ios-typecheck. Each hash has its own directory. A lock covers the build and publish; the
# result is written aside and renamed into place. The last three builds are kept.
exec python3 -I "$(dirname "$(readlink -f "$0")")/ios-typecheck/typecheck.py" "$@"
