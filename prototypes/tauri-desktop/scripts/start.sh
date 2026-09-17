#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
DESKTOP_DIR="$(cd -- "$SCRIPT_DIR/.." && pwd)"
BINARY_NAME="trace-commons-tauri-prototype"

PROFILE_DIR="release"
NO_BUILD=0
for arg in "$@"; do
  case "$arg" in
    --release)
      PROFILE_DIR="release"
      ;;
    --no-build)
      NO_BUILD=1
      ;;
    *)
      printf 'Usage: %s [--release] [--no-build]\n' "$0" >&2
      exit 2
      ;;
  esac
done

if (( NO_BUILD == 0 )); then
  "$SCRIPT_DIR/build.sh" --release
fi

BINARY="$DESKTOP_DIR/target/$PROFILE_DIR/$BINARY_NAME"
if [[ ! -x "$BINARY" ]]; then
  printf 'Build not found: %s\n' "$BINARY" >&2
  printf 'Run %s first or omit --no-build.\n' "$SCRIPT_DIR/build.sh" >&2
  exit 1
fi

exec "$BINARY"
