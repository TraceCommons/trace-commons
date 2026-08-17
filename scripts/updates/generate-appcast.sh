#!/usr/bin/env bash
# Generate the Sparkle appcast for the macOS app.
#
# Sparkle verifies this feed with its own EdDSA key AND the app's Developer ID
# code signature. Both must hold, so a compromised bucket alone cannot push an
# update.
set -euo pipefail

die() { echo "generate-appcast: $*" >&2; exit 1; }

SHORT_VERSION=""   # e.g. 0.2.0, shown to users
BUILD_VERSION=""   # CFBundleVersion, monotonic, what Sparkle compares
DMG_URL=""
DMG_PATH=""
SIGN_UPDATE=""     # path to Sparkle's sign_update binary
OUT="dist/updates/appcast.xml"

while [ $# -gt 0 ]; do
  case "$1" in
    --short-version) SHORT_VERSION="${2:?}"; shift 2 ;;
    --build-version) BUILD_VERSION="${2:?}"; shift 2 ;;
    --dmg-url)       DMG_URL="${2:?}"; shift 2 ;;
    --dmg-path)      DMG_PATH="${2:?}"; shift 2 ;;
    --sign-update)   SIGN_UPDATE="${2:?}"; shift 2 ;;
    --out)           OUT="${2:?}"; shift 2 ;;
    *) die "unknown argument: $1" ;;
  esac
done

[ -n "$SHORT_VERSION" ] || die "--short-version is required"
[ -n "$BUILD_VERSION" ] || die "--build-version is required"
[ -n "$DMG_URL" ] || die "--dmg-url is required"
[ -f "$DMG_PATH" ] || die "dmg not found: $DMG_PATH"
[ -x "$SIGN_UPDATE" ] || die "sign_update not found or not executable: $SIGN_UPDATE"

printf '%s' "$SHORT_VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' \
  || die "short version must be three-part numeric, got '$SHORT_VERSION'"

LENGTH="$(wc -c < "$DMG_PATH" | tr -d ' ')"

# sign_update prints an attribute fragment:
#   sparkle:edSignature="..." length="..."
# Take only the signature; the length is recomputed above from the file we
# are actually publishing.
SIGNATURE="$("$SIGN_UPDATE" "$DMG_PATH" | sed -E 's/.*sparkle:edSignature="([^"]+)".*/\1/')"
[ -n "$SIGNATURE" ] || die "sign_update produced no signature"

PUBDATE="$(date -u '+%a, %d %b %Y %H:%M:%S +0000')"

mkdir -p "$(dirname "$OUT")"
cat > "$OUT" <<EOF
<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>Trace Commons</title>
    <item>
      <title>$SHORT_VERSION</title>
      <pubDate>$PUBDATE</pubDate>
      <sparkle:minimumSystemVersion>14.0</sparkle:minimumSystemVersion>
      <enclosure
        url="$DMG_URL"
        sparkle:version="$BUILD_VERSION"
        sparkle:shortVersionString="$SHORT_VERSION"
        length="$LENGTH"
        type="application/octet-stream"
        sparkle:edSignature="$SIGNATURE" />
    </item>
  </channel>
</rss>
EOF

echo "wrote $OUT"
