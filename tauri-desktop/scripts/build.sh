#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
DESKTOP_DIR="$(cd -- "$SCRIPT_DIR/.." && pwd)"
FRONTEND_DIR="$DESKTOP_DIR/frontend"
TAURI_MANIFEST="$DESKTOP_DIR/src-tauri/Cargo.toml"

PROFILE_DIR="debug"
case "${1:-}" in
  "") ;;
  --release)
    PROFILE_DIR="release"
    ;;
  *)
    printf 'Usage: %s [--release]\n' "$0" >&2
    exit 2
    ;;
esac

pnpm --dir "$FRONTEND_DIR" install --frozen-lockfile
pnpm --dir "$FRONTEND_DIR" build
if [[ "$PROFILE_DIR" == release ]]; then
  cargo build --locked --manifest-path "$TAURI_MANIFEST" --release
else
  cargo build --locked --manifest-path "$TAURI_MANIFEST"
fi

printf 'Built %s/target/%s/trace-commons-tauri-desktop\n' "$DESKTOP_DIR" "$PROFILE_DIR"
