#!/usr/bin/env bash
# Build the app icon as an Icon Composer .icon, compiled with actool.
#
# Called by make-app-bundle.sh. Produces two files into the bundle's
# Resources: Assets.car, which is what macOS 26 actually draws and what
# carries the Liquid Glass treatment, and AppIcon.icns, which actool emits
# alongside it as the legacy fallback for older systems and for callers that
# ask for a file icon.
#
# # A .icon is a directory, not a GUI-only artifact
#
# This was believed impossible when the .icns path was written -- the comment
# in make-icons.sh says an .icon "is authored in Icon Composer's GUI and there
# is no documented way to produce one from a build script". That is wrong, and
# it was worth checking rather than inheriting:
#
#   - The UTI com.apple.iconcomposer.icon conforms to com.apple.package (see
#     Icon Composer.app's Info.plist), so a .icon is a directory.
#   - It holds icon.json plus an Assets directory. IconComposerFoundation
#     carries the strings "Assets should be a directory" and rejects SVG
#     assets containing text elements, which is only meaningful if SVG is the
#     asset format.
#   - actool compiles one directly. No xcodeproj is involved, which matters
#     because this package builds with `swift build`.
#
# # Why the glyph variant and not the framed mark
#
# The system draws the tile: its shape, its ground, its shadow and its
# specular highlight, and it composites light, dark and tinted appearances
# from what we supply. A layer carrying its own opaque ground would sit inside
# that tile as a light square that never changes -- the light/dark collapse
# the .icon route exists to remove. So the layer is mark-glyph-*.svg, which is
# the two brackets on nothing, and the ground comes from the fill.
#
# # actool does not validate icon.json
#
# Verified: actool accepts an icon.json naming an asset file that does not
# exist, and accepts a key whose value is the wrong JSON type, both without a
# warning. So a typo in this document does not fail the build -- it silently
# produces a different icon. That is why this script ends by rendering the
# result and asserting the mark is in it, and why nothing here should be
# "simplified" into trusting actool's exit status.
#
# # Assets.car is not reproducible
#
# Two compilations of byte-identical input produce different Assets.car
# bytes -- the rendition names embed fresh UUIDs each run. So this output
# cannot be drift-checked by hash the way the PNG and SVG artwork is. It is
# checked semantically instead, by decoding what came out.
set -euo pipefail

cd "$(dirname "$0")/.."
PACKAGE_DIR="$PWD"
REPO_ROOT="$(cd .. && pwd)"

ASSET_DIR="${1:-$REPO_ROOT/assets/mark}"
RESOURCES="${2:?usage: make-icon-document.sh <asset-dir> <bundle-resources-dir>}"

LIGHT="$ASSET_DIR/mark-glyph-light.svg"
DARK="$ASSET_DIR/mark-glyph-dark.svg"

for svg in "$LIGHT" "$DARK"; do
  if [ ! -s "$svg" ]; then
    echo "FATAL: $svg does not exist." >&2
    echo "Generate it: cargo run -p trace-commons-mark --bin mark-export" >&2
    exit 1
  fi
  # Both brackets, by their path data. A document that compiles to a blank
  # or half-drawn icon is the failure this whole slice exists to catch, and
  # it is invisible downstream: actool will happily compile an empty SVG.
  for bracket in 'M11 28V11h17' 'M53 36v17H36'; do
    if ! grep -q "$bracket" "$svg"; then
      echo "FATAL: $svg does not carry the bracket $bracket." >&2
      exit 1
    fi
  done
done

# The dark drawing has to actually be a different drawing. If the two files
# ever come out identical -- a generator regression, a bad copy -- the icon
# would still build, still declare a dark specialization, and still show the
# light inks in dark mode, which is precisely the silent rot this check is
# here to stop.
if cmp -s "$LIGHT" "$DARK"; then
  echo "FATAL: the dark glyph is byte-identical to the light one." >&2
  echo "Dark mode would show the light palette while appearing to be set." >&2
  exit 1
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
DOC="$WORK/AppIcon.icon"
mkdir -p "$DOC/Assets" "$WORK/out"

cp "$LIGHT" "$DOC/Assets/mark-glyph-light.svg"
cp "$DARK" "$DOC/Assets/mark-glyph-dark.svg"

# The surface token from the shared palette, as the extended-sRGB components
# actool wants. Kept in step with Scheme::Light.surface() by
# icon_document_fill_matches_the_light_surface in the mark crate.
# The specialization array is base-first: the entry with NO "appearance" key
# is what every appearance starts from, and entries carrying an "appearance"
# override it. That ordering is the whole trick. An array holding only the
# dark entry -- an override with nothing to override -- is structurally
# invalid, and since actool validates nothing it is discarded in silence,
# which reads exactly like the feature not existing. The shape is taken from
# an icon.json written by Icon Composer 1.6, not inferred.
cat > "$DOC/icon.json" <<'JSON'
{
  "fill" : {
    "automatic-gradient" : "extended-srgb:1.00000,1.00000,1.00000,1.00000"
  },
  "groups" : [
    {
      "layers" : [
        {
          "image-name-specializations" : [
            {
              "value" : "mark-glyph-light.svg"
            },
            {
              "appearance" : "dark",
              "value" : "mark-glyph-dark.svg"
            }
          ]
        }
      ]
    }
  ],
  "supported-platforms" : {
    "circles" : [
      "watchOS"
    ],
    "squares" : [
      "macOS"
    ]
  }
}
JSON

xcrun actool \
  --compile "$WORK/out" \
  --app-icon AppIcon \
  --platform macosx \
  --minimum-deployment-target 26.0 \
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

# Not a formality. actool validates nothing about icon.json, so this is the
# only thing standing between a typo and a bundle that ships the wrong icon
# or a blank one. actool's icns carries four representations, not the ten an
# iconutil ladder produces.
swift "$PACKAGE_DIR/scripts/verify-icon.swift" "$RESOURCES/AppIcon.icns" 4

# Assets.car is what macOS 26 actually draws, and the icns above says nothing
# about it. What matters there is that the catalogue carries a composition per
# appearance -- the dark one is how the app looks in dark mode, and it is
# generated by actool rather than by us, so a toolchain change could drop it
# without anything else noticing. assetutil reads the compiled catalogue; the
# icns cannot answer this question.
for appearance in NSAppearanceNameAqua NSAppearanceNameDarkAqua ISAppearanceTintable; do
  if ! xcrun assetutil --info "$RESOURCES/Assets.car" \
    | grep -q "\"Appearance\" : \"$appearance\""; then
    echo "FATAL: Assets.car carries no $appearance composition." >&2
    echo "The app would have no distinct appearance there." >&2
    exit 1
  fi
done

# And that the dark composition resolves to the DARK artwork. The check above
# would pass with all three appearances sharing one layer, which is exactly
# what this build did before the specialization was understood: three
# compositions, one vector, light inks in dark mode. Nothing downstream would
# have said so -- the icon renders, it just renders wrong.
xcrun assetutil --info "$RESOURCES/Assets.car" > "$WORK/catalogue.json"
python3 "$PACKAGE_DIR/scripts/verify-icon-appearances.py" "$WORK/catalogue.json"

echo "built $RESOURCES/AppIcon.icns and $RESOURCES/Assets.car"
