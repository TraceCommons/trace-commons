#!/usr/bin/env bash
# Assemble TraceCommons.app around the SwiftPM executable.
#
# SwiftPM produces a bare Mach-O; a menu-bar app needs a bundle so that
# LSUIElement (no Dock icon) and a bundle identifier (UNUserNotificationCenter)
# exist at all. Signing, notarization and a DMG are out of scope -- this is an
# ad-hoc-signed development bundle.
#
# The app ships as a universal (arm64 + x86_64) binary so it runs on both
# Apple silicon and Intel Macs. That means both the FFI dylib and the Swift
# executable have to be built for both architectures and lipo'd together --
# doing that only for one architecture and shipping it as if it covered both
# would sail through signing, notarization and Gatekeeper, then simply fail
# to launch on whichever architecture got left out.
set -euo pipefail

cd "$(dirname "$0")/.."
PACKAGE_DIR="$PWD"
REPO_ROOT="$(cd .. && pwd)"
CONFIG="${1:-debug}"
# A dev bundle gets an obviously-not-a-release version. The release path
# passes the tag's version explicitly; see release-apps.yml.
SHORT_VERSION="${2:-0.0.0-dev}"
BUILD_VERSION="${3:-1}"
APP="$PACKAGE_DIR/.build/TraceCommons.app"
DYLIB_NAME="libtrace_commons_contributor_ffi.dylib"

RUST_TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)

# cargo's own directory name for the profile, not SwiftPM's: `cargo build`
# (no flag) writes to target/<triple>/debug even though nothing here calls
# that config "debug" explicitly.
CARGO_PROFILE_DIR="debug"
CARGO_BUILD_ARGS=()
if [ "$CONFIG" = "release" ]; then
  CARGO_PROFILE_DIR="release"
  CARGO_BUILD_ARGS+=(--release)
fi

# A staging directory for the lipo'd universal dylib. TC_FFI_LIB_DIR already
# exists so Package.swift's library search path is never hardcoded; point it
# here rather than adding new plumbing.
STAGING_DIR="$PACKAGE_DIR/.build/ffi-universal"
rm -rf "$STAGING_DIR"
mkdir -p "$STAGING_DIR"

verify_universal() {
  local label="$1" bin="$2"
  local archs
  archs="$(lipo -archs "$bin")"
  for want in arm64 x86_64; do
    if [[ " $archs " != *" $want "* ]]; then
      echo "FATAL: $label ($bin) is missing $want -- got: $archs" >&2
      echo "A thin binary here would still sign, notarize and pass" >&2
      echo "Gatekeeper, then fail to launch on the missing architecture." >&2
      exit 1
    fi
  done
  echo "verified universal ($archs): $label"
}

echo "--- building the FFI dylib for ${RUST_TARGETS[*]}"
DYLIB_PATHS=()
for target in "${RUST_TARGETS[@]}"; do
  (cd "$REPO_ROOT" && cargo build ${CARGO_BUILD_ARGS[@]+"${CARGO_BUILD_ARGS[@]}"} \
    --target "$target" -p trace-commons-contributor-ffi)
  DYLIB_PATHS+=("$REPO_ROOT/target/$target/$CARGO_PROFILE_DIR/$DYLIB_NAME")
done

lipo -create "${DYLIB_PATHS[@]}" -output "$STAGING_DIR/$DYLIB_NAME"
verify_universal "FFI dylib" "$STAGING_DIR/$DYLIB_NAME"

# Package.swift reads this; without it a release build links target/debug.
export TC_FFI_LIB_DIR="$STAGING_DIR"

swift build --configuration "$CONFIG" --arch arm64 --arch x86_64

# `--arch` (needed to get a universal executable out of SwiftPM) changes
# where the product lands: a single-arch `swift build -c release` writes to
# .build/release, but passing --arch writes to
# .build/apple/Products/<Config, capitalized>.
CONFIG_CAP="$(tr '[:lower:]' '[:upper:]' <<< "${CONFIG:0:1}")${CONFIG:1}"
BIN_DIR="$PACKAGE_DIR/.build/apple/Products/$CONFIG_CAP"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Frameworks" "$APP/Contents/Resources"

./scripts/info-plist.sh "$SHORT_VERSION" "$BUILD_VERSION" \
  > "$APP/Contents/Info.plist"

cp "$BIN_DIR/TraceCommonsApp" "$APP/Contents/MacOS/TraceCommonsApp"
cp "$STAGING_DIR/$DYLIB_NAME" "$APP/Contents/Frameworks/$DYLIB_NAME"

# cargo stamps the dylib with an absolute install name under target/. Repoint
# it inside the bundle so the app does not depend on this checkout's path.
install_name_tool -id "@rpath/$DYLIB_NAME" "$APP/Contents/Frameworks/$DYLIB_NAME" 2>/dev/null
OLD_ID="$(otool -D "${DYLIB_PATHS[0]}" | tail -1)"
install_name_tool -change "$OLD_ID" "@rpath/$DYLIB_NAME" "$APP/Contents/MacOS/TraceCommonsApp" 2>/dev/null || true
install_name_tool -add_rpath "@executable_path/../Frameworks" "$APP/Contents/MacOS/TraceCommonsApp" 2>/dev/null || true

# An ad-hoc signature is what makes a DEVELOPMENT bundle launchable. The
# release path signs with a Developer ID immediately afterwards, so doing it
# here first is wasted work that also makes the release path read as if it
# might ship an ad-hoc signature.
if [ "${TC_SKIP_ADHOC_SIGN:-0}" != "1" ]; then
  codesign --force --sign - --timestamp=none "$APP/Contents/Frameworks/$DYLIB_NAME" >/dev/null 2>&1 || true
  codesign --force --sign - --timestamp=none "$APP" >/dev/null 2>&1 || true
fi

echo "--- verifying the final bundle is universal"
verify_universal "app executable" "$APP/Contents/MacOS/TraceCommonsApp"
verify_universal "embedded dylib" "$APP/Contents/Frameworks/$DYLIB_NAME"

echo "built $APP"
