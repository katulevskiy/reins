#!/bin/bash
# In a git worktree of this repository: reuse the main checkout's Rust core build (target/ios-core) instead of
# building the core again. Run once from anywhere in the worktree.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
MAIN="$(git -C "$ROOT" worktree list --porcelain | awk '/^worktree /{print $2; exit}')"
[[ "$ROOT" == "$MAIN" ]] && { echo "this is the main checkout"; exit 0; }
mkdir -p "$ROOT/target"
[[ -e "$ROOT/target/ios-core" ]] || ln -s "$MAIN/target/ios-core" "$ROOT/target/ios-core"
echo "target/ios-core -> $MAIN/target/ios-core"
