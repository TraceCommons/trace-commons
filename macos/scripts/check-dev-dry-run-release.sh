#!/usr/bin/env bash
# K2 (#1173): prove the dev-only dry-run switch is not in a Release build.
#
# `TC_DEV_DRY_RUN` is a developer convenience gated `#if DEBUG` in
# `DaemonDataWiring.swift` (the only place in TraceCommonsApp that names it).
# A `-c release` build defines no `DEBUG`, so the Swift compiler should drop
# that whole block -- including the string literal "TC_DEV_DRY_RUN" -- before
# it ever reaches the linker. This script builds the app `-c release` the
# same way `make-app-bundle.sh` does and then checks the actual product for
# that string, the same method C1 used to confirm `SampleDaemonClient` never
# ships: a compile-time claim is only as good as what is in the binary.
#
# Usage, from the repo root:
#   cargo build -p trace-commons-contributor-ffi --release
#   macos/scripts/check-dev-dry-run-release.sh
#
# Exits non-zero, naming the offending symbol, if the string is found.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

REPO_ROOT="$(cd .. && pwd)"
export TC_FFI_LIB_DIR="${TC_FFI_LIB_DIR:-$REPO_ROOT/target/release}"

if [ ! -e "$TC_FFI_LIB_DIR/libtrace_commons_contributor_ffi.dylib" ]; then
  echo "missing $TC_FFI_LIB_DIR/libtrace_commons_contributor_ffi.dylib" >&2
  echo "run: cargo build -p trace-commons-contributor-ffi --release" >&2
  exit 1
fi

echo "building TraceCommonsApp -c release (TC_FFI_LIB_DIR=$TC_FFI_LIB_DIR)..."
swift build -c release --product TraceCommonsApp

BINARY=".build/release/TraceCommonsApp"
if [ ! -e "$BINARY" ]; then
  echo "expected $BINARY after swift build -c release; not found" >&2
  exit 1
fi

if strings -a "$BINARY" | grep -q "TC_DEV_DRY_RUN"; then
  echo "FAIL: TC_DEV_DRY_RUN appears in the release product ($BINARY)" >&2
  echo "the #if DEBUG guard in DaemonDataWiring.swift is not excluding it" >&2
  exit 1
fi

echo "PASS: TC_DEV_DRY_RUN does not appear in $BINARY"
