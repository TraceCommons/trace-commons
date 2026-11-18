#!/usr/bin/env bash
# Pair macOS and Linux window captures into one self-contained HTML page.
#
# Usage: scripts/parity/gallery.sh <mac-dir> <linux-dir> [out.html]
#
# Captures are named <set>-<tab>-<panes>-<appearance>.png (see gallery.py).
# macOS captures come from the debug app's ImageRenderer hook:
#   TRACE_COMMONS_SCREENSHOT_DIR=<mac-dir> TRACE_COMMONS_SAMPLE=normalDay \
#   TRACE_COMMONS_APPEARANCE=light <debug app>
# The page is written locally and never uploaded.
set -euo pipefail
if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
  echo "usage: $0 <mac-dir> <linux-dir> [out.html]" >&2
  exit 2
fi
here="$(cd "$(dirname "$0")" && pwd)"
out="${3:-$PWD/parity-gallery.html}"
exec python3 -I "$here/gallery.py" "$1" "$2" "$out"
