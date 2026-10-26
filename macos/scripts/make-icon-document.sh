#!/usr/bin/env bash
# Compile the app icon, an Icon Composer .icon, with actool.
#
# Called by make-app-bundle.sh with the channel's TC_ICON_SOURCE: one of the
# documents under macos/icons/. Produces two files into the bundle's
# Resources: Assets.car, which is what macOS draws (macOS 26 and later
# composite its layers live as Liquid Glass; macOS 14 and 15 draw the images
# actool pre-renders into it), and AppIcon.icns, the flat fallback for callers
# that ask for a file icon.
#
# # The documents are checked in, not generated
#
# The icon is the NEAR AI tile built the way Apple and the brand book both
# say to build it: Icon Composer's fill for the background (brand-tile
# #0B8FF6, and a deeper NEAR blue for dark), and the kit's N in brand-paper
# as one SVG foreground layer. A .icon is a directory -- icon.json plus an
# Assets directory -- so it lives in the repository as source and opens in
# Icon Composer by double-click. Change the icon there, not in this script.
# macos/icons/README.md has the provenance and how to preview it.
#
# # actool does not validate icon.json
#
# Verified: actool accepts an icon.json naming an asset file that does not
# exist, and accepts a key whose value is the wrong JSON type, both without a
# warning. So a typo in the document does not fail the build -- it silently
# produces a different icon. That is why the document is checked before it is
# compiled and the result is decoded after, and why nothing here should be
# "simplified" into trusting actool's exit status.
#
# # Assets.car is not reproducible
#
# Two compilations of byte-identical input produce different Assets.car
# bytes -- the rendition names embed fresh UUIDs each run. So this output
# cannot be drift-checked by hash. It is checked semantically instead, by
# decoding what came out.
set -euo pipefail

cd "$(dirname "$0")/.."
PACKAGE_DIR="$PWD"

USAGE="usage: make-icon-document.sh <AppIcon.icon> <bundle-resources-dir> <production|staging>"
SOURCE="${1:?$USAGE}"
RESOURCES="${2:?$USAGE}"
# The channel decides what the checks expect: the staging icon must carry its
# ribbon and the production icon must not.
CHANNEL="${3:?$USAGE}"
PRODUCTION="$PACKAGE_DIR/icons/production/AppIcon.icon"

if [ ! -f "$SOURCE/icon.json" ]; then
  echo "FATAL: $SOURCE is not an Icon Composer document (no icon.json)." >&2
  exit 1
fi

python3 "$PACKAGE_DIR/scripts/verify-icon-document.py" "$SOURCE" "$CHANNEL"

# One N for every channel. The staging document carries its own copy, because
# a .icon has to be self-contained to open in Icon Composer; this keeps the
# copy from drifting.
if ! cmp -s "$SOURCE/Assets/near-n.svg" "$PRODUCTION/Assets/near-n.svg"; then
  echo "FATAL: $SOURCE/Assets/near-n.svg differs from the production N." >&2
  exit 1
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
# actool names the icon after the document, and Info.plist resolves it as
# AppIcon, so it is compiled under that name whatever the source is called.
DOC="$WORK/AppIcon.icon"
mkdir -p "$WORK/out"
cp -R "$SOURCE" "$DOC"

# The deployment target is the app's own floor, LSMinimumSystemVersion in
# info-plist.sh. It makes no difference to what actool emits today -- it
# pre-renders the legacy sizes either way -- but it is the true value.
xcrun actool \
  --compile "$WORK/out" \
  --app-icon AppIcon \
  --platform macosx \
  --minimum-deployment-target 14.0 \
  --output-partial-info-plist "$WORK/partial.plist" \
  "$DOC" > "$WORK/actool.plist"

# actool reports failures in its plist rather than by exiting non-zero for
# every class of problem, so the output files are checked directly.
for produced in AppIcon.icns Assets.car; do
  if [ ! -s "$WORK/out/$produced" ]; then
    echo "FATAL: actool produced no $produced" >&2
    sed -n '1,60p' "$WORK/actool.plist" >&2
    exit 1
  fi
done

mkdir -p "$RESOURCES"
cp "$WORK/out/AppIcon.icns" "$RESOURCES/AppIcon.icns"
cp "$WORK/out/Assets.car" "$RESOURCES/Assets.car"

# The fallback icns carries four representations (256, 128, 32, 16); the
# larger sizes live in Assets.car.
swift "$PACKAGE_DIR/scripts/verify-icon.swift" "$RESOURCES/AppIcon.icns" 4 "$CHANNEL"

# Assets.car is what macOS actually draws, and the icns above says nothing
# about it: not the dark ground, not the layers, not the pre-rendered sizes
# macOS 14 and 15 use. assetutil reads the compiled catalogue.
xcrun assetutil --info "$RESOURCES/Assets.car" > "$WORK/catalogue.json"
python3 "$PACKAGE_DIR/scripts/verify-icon-appearances.py" "$SOURCE/icon.json" "$WORK/catalogue.json"

echo "built $RESOURCES/AppIcon.icns and $RESOURCES/Assets.car ($CHANNEL icon)"
