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

cat > "$APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Trace Commons</string>
    <key>CFBundleDisplayName</key><string>Trace Commons</string>
    <key>CFBundleIdentifier</key><string>ai.tracecommons.shell</string>
    <key>CFBundleExecutable</key><string>TraceCommonsApp</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>0.1.0</string>
    <key>CFBundleVersion</key><string>1</string>
    <key>LSMinimumSystemVersion</key><string>14.0</string>
    <key>NSHumanReadableCopyright</key><string>Trace Commons</string>
    <!-- Menu-bar item, no Dock icon: the shape macOS users expect from a
         background utility. -->
    <key>LSUIElement</key><true/>
</dict>
</plist>
PLIST

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
