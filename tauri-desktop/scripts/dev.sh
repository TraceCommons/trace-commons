#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
DESKTOP_DIR="$(cd -- "$SCRIPT_DIR/.." && pwd)"
FRONTEND_DIR="$DESKTOP_DIR/frontend"
TAURI_MANIFEST="$DESKTOP_DIR/src-tauri/Cargo.toml"
DEV_URL="http://127.0.0.1:1420"

if ! command -v curl >/dev/null 2>&1; then
  printf 'dev.sh requires curl to check Vite startup.\n' >&2
  exit 1
fi

pnpm --dir "$FRONTEND_DIR" install --frozen-lockfile
pnpm --dir "$FRONTEND_DIR" exec vite --host 127.0.0.1 --port 1420 --strictPort &
VITE_PID=$!

cleanup() {
  kill "$VITE_PID" 2>/dev/null || true
  wait "$VITE_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

for _ in {1..60}; do
  if curl --silent --fail --output /dev/null "$DEV_URL"; then
    cargo run --manifest-path "$TAURI_MANIFEST"
    exit $?
  fi
  if ! kill -0 "$VITE_PID" 2>/dev/null; then
    printf 'Vite exited before becoming ready.\n' >&2
    exit 1
  fi
  sleep 0.25
done

printf 'Vite did not become ready at %s.\n' "$DEV_URL" >&2
exit 1
