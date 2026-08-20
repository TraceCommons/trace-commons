#!/usr/bin/env bash
# Build AppIcon.icns from the generated mark SVG.
#
# Called by make-app-bundle.sh. Separate so it can be run and inspected on its
# own -- an icon that silently comes out blank is the failure this whole slice
# exists to fix, and it is not one you notice from a build log.
#
# # Why CoreGraphics and not an SVG rasterizer
#
# The natural input is assets/mark/mark-light.svg, and sips does rasterize it
# correctly -- that was checked, at every rung, and it is not the reason for
# this route. The reasons are that sips' SVG support is not a documented
# interface (it goes through whatever Quick Look generator the OS ships, which
# is free to change under us), and that every alternative rasterizer --
# rsvg-convert, cairosvg, ImageMagick, Inkscape -- is a new build dependency
# that is not present on a hosted runner.
#
# render-mark.swift draws the geometry with CoreGraphics instead, reading the
# JSON that crates/trace-commons-mark generates. Nothing to install, exact at
# every size, and still one description of the mark.
#
# # A trap for whoever verifies this next
#
# Do NOT verify an .icns by round-tripping it through
# `iconutil --convert iconset` and inspecting the PNGs that come out.
#
# iconutil stores the 16x16 and 32x32 rungs as ic04/ic05, which are raw ARGB
# RLE rather than PNG, and its own decoder reads them back wrong: it zeroes the
# blue channel in the bottom-right corner, so #D9DFDC comes out (217,223,0) and
# white comes out pure yellow. The bytes in the .icns are fine. Every other
# rung, being PNG, round-trips clean, which makes the artifact look exactly
# like a real corruption confined to the two smallest sizes -- and those are
# the Finder list view and the menu bar, so it reads as plausible and
# important.
#
# Verify by loading the .icns through CGImageSource, which is what macOS itself
# uses to draw it. All ten representations decode clean that way.
#
# # SUPERSEDED: this script is no longer wired into the build
#
# make-app-bundle.sh calls make-icon-document.sh instead, which builds an
# Icon Composer .icon and compiles it with actool.
#
# The claim that used to stand here -- that an .icon "is authored in Icon
# Composer's GUI and there is no documented way to produce one from a build
# script" -- was wrong, and it was wrong in the direction that costs the most:
# it read as settled, so nobody checked. A .icon is a directory (its UTI
# conforms to com.apple.package), it holds icon.json plus an Assets directory
# of SVGs, and actool compiles one with no xcodeproj involved. All three were
# verified before this comment was rewritten.
#
# This script is kept, unwired, for one reason: it produces the full
# ten-representation icns ladder, where actool's fallback icns carries four.
# If the .icon route ever has to be backed out, this is what to go back to.
# Nothing else references it -- if it is still unwired when someone next reads
# this, deleting it is reasonable.
set -euo pipefail

cd "$(dirname "$0")/.."
REPO_ROOT="$(cd .. && pwd)"

GEOMETRY="${1:-$REPO_ROOT/assets/mark/geometry.json}"
OUT="${2:-$PWD/.build/AppIcon.icns}"

if [ ! -f "$GEOMETRY" ]; then
  echo "FATAL: $GEOMETRY does not exist." >&2
  echo "Generate it: cargo run -p trace-commons-mark --bin mark-export" >&2
  exit 1
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
ICONSET="$WORK/AppIcon.iconset"
mkdir -p "$ICONSET"

# The ladder iconutil expects, by name. Every rung is drawn at its own
# resolution in one pass of the renderer.
swift "$PWD/scripts/render-mark.swift" "$GEOMETRY" light framed \
  "16:$ICONSET/icon_16x16.png" \
  "32:$ICONSET/icon_16x16@2x.png" \
  "32:$ICONSET/icon_32x32.png" \
  "64:$ICONSET/icon_32x32@2x.png" \
  "128:$ICONSET/icon_128x128.png" \
  "256:$ICONSET/icon_128x128@2x.png" \
  "256:$ICONSET/icon_256x256.png" \
  "512:$ICONSET/icon_256x256@2x.png" \
  "512:$ICONSET/icon_512x512.png" \
  "1024:$ICONSET/icon_512x512@2x.png"

for rung in icon_16x16.png icon_16x16@2x.png icon_32x32.png icon_32x32@2x.png \
            icon_128x128.png icon_128x128@2x.png icon_256x256.png \
            icon_256x256@2x.png icon_512x512.png icon_512x512@2x.png; do
  if [ ! -s "$ICONSET/$rung" ]; then
    echo "FATAL: the renderer produced no output for $rung" >&2
    exit 1
  fi
done

mkdir -p "$(dirname "$OUT")"
iconutil --convert icns --output "$OUT" "$ICONSET"

if [ ! -s "$OUT" ]; then
  echo "FATAL: iconutil produced no icns at $OUT" >&2
  exit 1
fi

# Not optional, and not a formality: the whole reason this slice exists is that
# artwork which is present, correctly named and correctly sized can still be
# blank, and nothing downstream would say so.
swift "$PWD/scripts/verify-icns.swift" "$OUT" 10

echo "built $OUT ($(wc -c < "$OUT" | tr -d ' ') bytes)"
