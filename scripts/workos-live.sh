#!/bin/bash
# Live check of WorkOS AuthKit sign-in, the keyless vault and the WorkOS lifecycle sync against a real WorkOS
# environment (use a staging one: it makes and deletes a test user). Headless: Google Chrome plays the browser.
#
#   infisical run --env=dev -- scripts/workos-live.sh      (maintainers: the keys come from Infisical)
#   scripts/workos-live.sh [path to workos.env]            (default: workos.env at the repository root)
#
# With WORKOS_CLIENT_ID and WORKOS_API_KEY already in the environment the script uses them and reads no file.
# Otherwise workos.env holds WORKOS_CLIENT_ID=client_... and WORKOS_API_KEY=sk_test_... (one per line; never commit it).
# The script adds http://localhost:8765/identity/connect/oidc-signin to the environment's redirect URIs, installs
# playwright-core into a cache directory, builds the server and runs crates/reins-e2e/examples/workos_live.rs.
# Set CARGO_TARGET_DIR to reuse a server build. Port 8765 must be free.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
if [[ -n "${WORKOS_CLIENT_ID:-}" && -n "${WORKOS_API_KEY:-}" ]]; then
  ENV_FILE="the environment"
else
  ENV_FILE="${1:-$ROOT/workos.env}"
  [[ -f "$ENV_FILE" ]] || { echo "no $ENV_FILE" >&2; exit 2; }
  # `KEY=value`, tolerating `KEY==value`.
  read_var() { sed -n "s/^$1=*//p" "$ENV_FILE" | head -1 | tr -d '\r'; }
  WORKOS_CLIENT_ID="$(read_var WORKOS_CLIENT_ID)"
  WORKOS_API_KEY="$(read_var WORKOS_API_KEY)"
fi
[[ "$WORKOS_CLIENT_ID" == client_* && "$WORKOS_API_KEY" == sk_* ]] || { echo "$ENV_FILE lacks the WorkOS ids" >&2; exit 2; }
[[ "$WORKOS_API_KEY" == sk_test_* ]] || { echo "refusing a production key: this makes and deletes users" >&2; exit 2; }
export WORKOS_CLIENT_ID WORKOS_API_KEY

REDIRECT=http://localhost:8765/identity/connect/oidc-signin
if ! curl -fsS https://api.workos.com/user_management/redirect_uris -H "Authorization: Bearer $WORKOS_API_KEY" |
  grep -q "\"$REDIRECT\""; then
  curl -fsS -X POST https://api.workos.com/user_management/redirect_uris -H "Authorization: Bearer $WORKOS_API_KEY" \
    -H 'Content-Type: application/json' -d "{\"uri\":\"$REDIRECT\"}" >/dev/null
  echo "added the redirect URI $REDIRECT"
fi

DRIVER_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/reins-authkit-driver"
mkdir -p "$DRIVER_DIR"
cp "$ROOT/crates/reins-e2e/authkit/sign-in.mjs" "$DRIVER_DIR/"
if [[ ! -d "$DRIVER_DIR/node_modules/playwright-core" ]]; then
  (cd "$DRIVER_DIR" && npm init -y >/dev/null && npm install --silent playwright-core)
fi
export AUTHKIT_DRIVER="$DRIVER_DIR/sign-in.mjs"

echo "building the server and the live check"
(cd "$ROOT" && cargo build -q --features sqlite --bin vaultwarden && cargo build -q -p reins-e2e --example workos_live)
# The example binary itself: under `cargo run`, the server build it starts sees cargo's variables and rebuilds.
cd "$ROOT" && exec "${CARGO_TARGET_DIR:-$ROOT/target}/debug/examples/workos_live"
