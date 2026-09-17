#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
DESKTOP_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
FRONTEND_DIR="$DESKTOP_DIR/frontend"
OUT_DIR="$DESKTOP_DIR/dist/macos-release/$(date -u +%Y%m%dT%H%M%SZ)"
TAURI_TARGET="universal-apple-darwin"
NOTARY_UPLOAD=""
NOTARY_KEY=""

cleanup_notary_material() {
  [[ -z "$NOTARY_UPLOAD" ]] || rm -f "$NOTARY_UPLOAD"
  [[ -z "$NOTARY_KEY" ]] || rm -f "$NOTARY_KEY"
}

trap cleanup_notary_material EXIT

: "${TC_RELEASE_VERSION:?Set TC_RELEASE_VERSION to the tagged semantic version}"
: "${MACOS_SIGNING_IDENTITY:?Set MACOS_SIGNING_IDENTITY to a Developer ID Application identity}"
: "${MACOS_NOTARY_ASC_KEY_P8_BASE64:?Set MACOS_NOTARY_ASC_KEY_P8_BASE64 for notarization}"
: "${MACOS_NOTARY_ASC_KEY_ID:?Set MACOS_NOTARY_ASC_KEY_ID for notarization}"
: "${MACOS_NOTARY_ASC_ISSUER_ID:?Set MACOS_NOTARY_ASC_ISSUER_ID for notarization}"

if [[ ! "$TC_RELEASE_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]]; then
  echo "TC_RELEASE_VERSION must be semantic version text" >&2
  exit 1
fi

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "macOS packaging requires Darwin" >&2
  exit 1
fi

mkdir -p "$OUT_DIR"
pnpm --dir "$FRONTEND_DIR" install --frozen-lockfile
pnpm --dir "$FRONTEND_DIR" build

run_tauri() {
  if command -v cargo-tauri >/dev/null 2>&1; then
    cargo-tauri "$@"
  elif command -v tauri >/dev/null 2>&1; then
    tauri "$@"
  else
    pnpm dlx --yes @tauri-apps/cli@2 "$@"
  fi
}

run_tauri build \
  --target "$TAURI_TARGET" \
  --config "$DESKTOP_DIR/src-tauri/tauri.conf.json" \
  --config "$DESKTOP_DIR/src-tauri/tauri.release.conf.json" \
  --config "{\"version\":\"$TC_RELEASE_VERSION\"}"

APP_PATH="$(find "$DESKTOP_DIR/src-tauri/target/$TAURI_TARGET/release/bundle/macos" -maxdepth 1 -type d -name '*.app' -print -quit)"
if [[ -z "$APP_PATH" ]]; then
  echo "Tauri did not produce a macOS app bundle" >&2
  exit 1
fi

codesign --force --deep --timestamp --options runtime \
  --sign "$MACOS_SIGNING_IDENTITY" "$APP_PATH"
codesign --verify --deep --strict --verbose=2 "$APP_PATH"

APP_NAME="$(basename "$APP_PATH" .app)"
NOTARY_UPLOAD="$OUT_DIR/$APP_NAME.notary-upload.zip"
ZIP_PATH="$OUT_DIR/$APP_NAME.app.zip"
DMG_PATH="$OUT_DIR/$APP_NAME.dmg"
ditto -c -k --keepParent "$APP_PATH" "$NOTARY_UPLOAD"

NOTARY_KEY="$OUT_DIR/notary.p8"
( umask 077; printf '%s' "$MACOS_NOTARY_ASC_KEY_P8_BASE64" | base64 -D > "$NOTARY_KEY" )
chmod 600 "$NOTARY_KEY"
xcrun notarytool submit "$NOTARY_UPLOAD" \
  --key "$NOTARY_KEY" \
  --key-id "$MACOS_NOTARY_ASC_KEY_ID" \
  --issuer "$MACOS_NOTARY_ASC_ISSUER_ID" \
  --wait
xcrun stapler staple "$APP_PATH"
xcrun stapler validate "$APP_PATH"

# The final archive contains the stapled app. The upload archive above is
# disposable and exists only because notarization runs before stapling.
ditto -c -k --keepParent "$APP_PATH" "$ZIP_PATH"
rm -f "$NOTARY_UPLOAD" "$NOTARY_KEY"

hdiutil create -volname "$APP_NAME" -srcfolder "$APP_PATH" -ov -format UDZO "$DMG_PATH"
codesign --force --timestamp --sign "$MACOS_SIGNING_IDENTITY" "$DMG_PATH"
NOTARY_KEY="$OUT_DIR/notary.p8"
( umask 077; printf '%s' "$MACOS_NOTARY_ASC_KEY_P8_BASE64" | base64 -D > "$NOTARY_KEY" )
chmod 600 "$NOTARY_KEY"
xcrun notarytool submit "$DMG_PATH" \
  --key "$NOTARY_KEY" \
  --key-id "$MACOS_NOTARY_ASC_KEY_ID" \
  --issuer "$MACOS_NOTARY_ASC_ISSUER_ID" \
  --wait
xcrun stapler staple "$DMG_PATH"
xcrun stapler validate "$DMG_PATH"
rm -f "$NOTARY_KEY"
spctl --assess --type open --context context:primary-signature "$APP_PATH"
codesign --verify --deep --strict --verbose=2 "$APP_PATH"

shasum -a 256 "$ZIP_PATH" "$DMG_PATH" | tee "$OUT_DIR/SHA256SUMS"
echo "macOS release artifacts: $OUT_DIR"
