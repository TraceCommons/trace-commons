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

# The app icon. Contents/Resources was created empty by every build before
# the icon slice -- an LSUIElement app never shows an icon, so nobody noticed
# there was none to show.
#
# This is the Icon Composer .icon route: it writes both Assets.car, which is
# what macOS 26 draws and what carries the Liquid Glass treatment, and
# AppIcon.icns as the legacy fallback. scripts/info-plist.sh declares
# CFBundleIconName for the former and CFBundleIconFile for the latter, which
# is the pair actool's own partial Info.plist emits.
#
# scripts/make-icons.sh is the earlier, flat .icns-only route. It is kept
# because it produces the full ten-representation ladder that iconutil wants
# and this one produces four, so it is the thing to fall back to if the .icon
# route ever has to be backed out. It is not wired into this build.
./scripts/make-icon-document.sh "$REPO_ROOT/assets/mark" \
  "$APP/Contents/Resources"

# cargo stamps the dylib with an absolute install name under target/. Repoint
# it inside the bundle so the app does not depend on this checkout's path.
install_name_tool -id "@rpath/$DYLIB_NAME" "$APP/Contents/Frameworks/$DYLIB_NAME" 2>/dev/null
OLD_ID="$(otool -D "${DYLIB_PATHS[0]}" | tail -1)"
install_name_tool -change "$OLD_ID" "@rpath/$DYLIB_NAME" "$APP/Contents/MacOS/TraceCommonsApp" 2>/dev/null || true
install_name_tool -add_rpath "@executable_path/../Frameworks" "$APP/Contents/MacOS/TraceCommonsApp" 2>/dev/null || true

# --- Sparkle ---------------------------------------------------------------
#
# SwiftPM LINKS Sparkle but does not EMBED it. In an Xcode project that is
# the "Embed & Sign" build phase; this bundle is assembled by hand, so the
# copy has to happen here. Skipping it produces an app that builds, signs and
# notarizes cleanly and then dies on launch with
#   Library not loaded: @rpath/Sparkle.framework/Versions/B/Sparkle
# The rpath added just above is what makes @rpath resolve to Frameworks/.
SPARKLE_MATCHES=()
while IFS= read -r match; do
  SPARKLE_MATCHES+=("$match")
done < <(find "$PACKAGE_DIR/.build/artifacts" -maxdepth 4 -type d -name 'Sparkle.xcframework' 2>/dev/null)

if [ "${#SPARKLE_MATCHES[@]}" -ne 1 ]; then
  echo "FATAL: expected exactly one Sparkle.xcframework under .build/artifacts," >&2
  echo "found ${#SPARKLE_MATCHES[@]}. Run 'swift package resolve' first." >&2
  printf '  %s\n' ${SPARKLE_MATCHES[@]+"${SPARKLE_MATCHES[@]}"} >&2
  exit 1
fi
SPARKLE_XCFRAMEWORK="${SPARKLE_MATCHES[0]}"

# The XCFramework carries one directory per platform slice. Naming the slice
# explicitly means a Sparkle release that renames or splits it fails here,
# loudly, instead of shipping a thin framework that passes signing and
# notarization and then fails to launch on whichever architecture was left
# out -- the same hazard verify_universal already guards for our own code.
SPARKLE_SLICE="$SPARKLE_XCFRAMEWORK/macos-arm64_x86_64/Sparkle.framework"
if [ ! -d "$SPARKLE_SLICE" ]; then
  echo "FATAL: no macos-arm64_x86_64 slice in $SPARKLE_XCFRAMEWORK" >&2
  echo "Slices present:" >&2
  ls -1 "$SPARKLE_XCFRAMEWORK" >&2
  exit 1
fi

# ditto, not cp: Sparkle.framework/Sparkle, Versions/Current, Resources and
# Headers are symlinks into Versions/B. A copy that dereferences them yields
# a bundle codesign rejects as malformed.
rm -rf "$APP/Contents/Frameworks/Sparkle.framework"
ditto "$SPARKLE_SLICE" "$APP/Contents/Frameworks/Sparkle.framework"

SPARKLE_FRAMEWORK="$APP/Contents/Frameworks/Sparkle.framework"
verify_universal "Sparkle framework" "$SPARKLE_FRAMEWORK/Versions/B/Sparkle"

# An ad-hoc signature is what makes a DEVELOPMENT bundle launchable. The
# release path signs with a Developer ID immediately afterwards, so doing it
# here first is wasted work that also makes the release path read as if it
# might ship an ad-hoc signature.
#
# The ORDER below is the same order make-release-dmg.sh uses, and it is not
# arbitrary: codesign seals nested code, so anything inside must be signed
# before the thing that contains it. `--deep` is never used -- it re-signs
# Sparkle's Downloader XPC service without its entitlements, which is the
# single most common way a Sparkle integration breaks.
if [ "${TC_SKIP_ADHOC_SIGN:-0}" != "1" ]; then
  codesign --force --sign - --timestamp=none \
    "$SPARKLE_FRAMEWORK/Versions/B/XPCServices/Installer.xpc" >/dev/null
  codesign --force --sign - --timestamp=none --preserve-metadata=entitlements \
    "$SPARKLE_FRAMEWORK/Versions/B/XPCServices/Downloader.xpc" >/dev/null
  codesign --force --sign - --timestamp=none \
    "$SPARKLE_FRAMEWORK/Versions/B/Autoupdate" >/dev/null
  codesign --force --sign - --timestamp=none \
    "$SPARKLE_FRAMEWORK/Versions/B/Updater.app" >/dev/null
  codesign --force --sign - --timestamp=none \
    "$SPARKLE_FRAMEWORK" >/dev/null
  codesign --force --sign - --timestamp=none "$APP/Contents/Frameworks/$DYLIB_NAME" >/dev/null 2>&1 || true
  codesign --force --sign - --timestamp=none "$APP" >/dev/null 2>&1 || true
fi

echo "--- verifying the final bundle is universal"
verify_universal "app executable" "$APP/Contents/MacOS/TraceCommonsApp"
verify_universal "embedded dylib" "$APP/Contents/Frameworks/$DYLIB_NAME"

echo "built $APP"
