#!/usr/bin/env bash
# Pack the GNOME Shell extension
# (../gnome-shell-extension/edge-surfaces@tracecommons.ai) into the zip
# extensions.gnome.org takes and `gnome-extensions install` reads.
#
#   pack-gnome-shell-extension.sh [OUT_DIR]   (default: the crate's target/)
#
# Prints the zip's path. The zip holds the files the shell loads and
# nothing else: not the tests, not package.json (which only tells Node the
# files are ES modules), not the README. Files are added in a fixed order with no extra
# attributes, so the same sources give the same entries.
set -euo pipefail

crate="$(cd "$(dirname "$0")/.." && pwd)"
src="$crate/gnome-shell-extension/edge-surfaces@tracecommons.ai"
out="${1:-$crate/target}"
uuid="$(sed -n 's/^ *"uuid": *"\([^"]*\)".*/\1/p' "$src/metadata.json")"
[ -n "$uuid" ] || { echo "metadata.json has no uuid" >&2; exit 1; }

# The files the shell loads. A file extension.js imports must be here;
# edge::gnome's tests hold this list to that.
files=(
  metadata.json
  extension.js
  placement.js
)

mkdir -p "$out"
zip_path="$out/$uuid.shell-extension.zip"
rm -f "$zip_path"
(cd "$src" && zip -q -X -D "$zip_path" "${files[@]}")
echo "$zip_path"
