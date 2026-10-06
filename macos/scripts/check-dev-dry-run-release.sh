#!/usr/bin/env bash
# K2 (#1173): prove the dev-only dry-run switch is not in a Release build.
#
# The switch is read by the Rust contributor library, and only in a debug
# build: `daemon::dev_dry_run_enabled` is `cfg(debug_assertions)`, so a
# release build has neither the code that reads `TC_DEV_DRY_RUN` nor the
# string. The app never reads the variable itself; a debug build asks the
# daemon (`status.dev_dry_run`), inside `#if DEBUG`.
#
# This script checks both release products for the string -- the Rust
# dylib, where the switch lives, and the Swift app binary -- the same method
# C1 used to confirm `SampleDaemonClient` never ships: a compile-time claim
# is only as good as what is in the binary.
#
# Usage, from the repo root:
#   cargo build -p trace-commons-contributor-ffi --release
#   macos/scripts/check-dev-dry-run-release.sh
#
# Exits non-zero, naming the offending symbol, if the string is found.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

REPO_ROOT="$(cd .. && pwd)"
export TC_FFI_LIB_DIR="${TC_FFI_LIB_DIR:-${CARGO_TARGET_DIR:-$REPO_ROOT/target}/release}"
DYLIB="$TC_FFI_LIB_DIR/libtrace_commons_contributor_ffi.dylib"

if [ ! -e "$DYLIB" ]; then
  echo "missing $TC_FFI_LIB_DIR/libtrace_commons_contributor_ffi.dylib" >&2
  echo "run: cargo build -p trace-commons-contributor-ffi --release" >&2
  exit 1
fi

if strings -a "$DYLIB" | grep "TC_DEV_DRY_RUN" >/dev/null; then
  echo "FAIL: TC_DEV_DRY_RUN appears in the release dylib ($DYLIB)" >&2
  echo "the cfg(debug_assertions) guard in daemon/mod.rs is not excluding it" >&2
  exit 1
fi
echo "PASS: TC_DEV_DRY_RUN does not appear in $DYLIB"

echo "building TraceCommonsApp -c release (TC_FFI_LIB_DIR=$TC_FFI_LIB_DIR)..."
swift build -c release --product TraceCommonsApp

BINARY=".build/release/TraceCommonsApp"
if [ ! -e "$BINARY" ]; then
  echo "expected $BINARY after swift build -c release; not found" >&2
  exit 1
fi

if strings -a "$BINARY" | grep "TC_DEV_DRY_RUN" >/dev/null; then
  echo "FAIL: TC_DEV_DRY_RUN appears in the release product ($BINARY)" >&2
  echo "a #if DEBUG guard in TraceCommonsApp is not excluding it" >&2
  exit 1
fi

echo "PASS: TC_DEV_DRY_RUN does not appear in $BINARY"
