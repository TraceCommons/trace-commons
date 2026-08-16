#!/usr/bin/env bash
# Assemble TraceCommons.app around the SwiftPM executable.
#
# SwiftPM produces a bare Mach-O; a menu-bar app needs a bundle so that
# LSUIElement (no Dock icon) and a bundle identifier (UNUserNotificationCenter)
# exist at all. Signing, notarization and a DMG are out of scope -- this is an
# ad-hoc-signed development bundle.
set -euo pipefail

cd "$(dirname "$0")/.."
PACKAGE_DIR="$PWD"
REPO_ROOT="$(cd .. && pwd)"
CONFIG="${1:-debug}"
# A dev bundle gets an obviously-not-a-release version. The release path
# passes the tag's version explicitly; see release-apps.yml.
SHORT_VERSION="${2:-0.0.0-dev}"
BUILD_VERSION="${3:-1}"
BIN_DIR="$PACKAGE_DIR/.build/$CONFIG"
APP="$PACKAGE_DIR/.build/TraceCommons.app"
DYLIB_NAME="libtrace_commons_contributor_ffi.dylib"
DYLIB="$REPO_ROOT/target/$CONFIG/$DYLIB_NAME"

if [ ! -f "$DYLIB" ]; then
  echo "missing $DYLIB -- run: cargo build -p trace-commons-contributor-ffi" >&2
  exit 1
fi

swift build --configuration "$CONFIG"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Frameworks" "$APP/Contents/Resources"

./scripts/info-plist.sh "$SHORT_VERSION" "$BUILD_VERSION" \
  > "$APP/Contents/Info.plist"

cp "$BIN_DIR/TraceCommonsApp" "$APP/Contents/MacOS/TraceCommonsApp"
cp "$DYLIB" "$APP/Contents/Frameworks/$DYLIB_NAME"

# cargo stamps the dylib with an absolute install name under target/. Repoint
# it inside the bundle so the app does not depend on this checkout's path.
install_name_tool -id "@rpath/$DYLIB_NAME" "$APP/Contents/Frameworks/$DYLIB_NAME" 2>/dev/null
OLD_ID="$(otool -D "$DYLIB" | tail -1)"
install_name_tool -change "$OLD_ID" "@rpath/$DYLIB_NAME" "$APP/Contents/MacOS/TraceCommonsApp" 2>/dev/null || true
install_name_tool -add_rpath "@executable_path/../Frameworks" "$APP/Contents/MacOS/TraceCommonsApp" 2>/dev/null || true

codesign --force --sign - --timestamp=none "$APP/Contents/Frameworks/$DYLIB_NAME" >/dev/null 2>&1 || true
codesign --force --sign - --timestamp=none "$APP" >/dev/null 2>&1 || true

echo "built $APP"
