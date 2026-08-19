#!/usr/bin/env bash
# Report whether the app's menu-bar item is actually VISIBLE, not merely present.
#
# Why this exists: the accessibility API reports a position and size for a
# status item that macOS never draws. On a notched Mac whose status area has
# grown left past the notch, new items are placed under the notch and are
# invisible, while `System Events` still answers with a plausible frame. An
# investigation that trusts that frame concludes the app fails to render its
# mark. That conclusion was reached once already; this script exists so the
# next person measures the notch before believing it.
#
# Usage:  macos/scripts/menu-bar-check.sh [pid]
# With no pid, picks the first running TraceCommonsApp.
set -euo pipefail

PID="${1:-}"
if [ -z "$PID" ]; then
  PID="$(pgrep -f 'TraceCommonsApp' | head -1 || true)"
fi
if [ -z "$PID" ]; then
  echo "no TraceCommonsApp process found; launch one first" >&2
  exit 1
fi

EXE="$(ps -o command= -p "$PID" | head -1)"
echo "process: pid=$PID"
echo "         $EXE"

# Which bundle this really is matters: several bundles are typically registered
# under ai.tracecommons.shell (dev builds under macos/.build in worktrees), and
# a bundle-id launch can start one you did not build.

FRAME="$(osascript -e "tell application \"System Events\" to tell (first process whose unix id is $PID) to get {position, size} of every menu bar item of menu bar 2" 2>&1)"
echo "status item frame (AX): $FRAME"

X="$(echo "$FRAME" | cut -d, -f1 | tr -d ' ')"
Y="$(echo "$FRAME" | cut -d, -f2 | tr -d ' ')"
W="$(echo "$FRAME" | cut -d, -f3 | tr -d ' ')"
H="$(echo "$FRAME" | cut -d, -f4 | tr -d ' ')"

# The notch, from the screen itself rather than from an assumed width. The gap
# between the two auxiliary top areas IS the notch.
GEOM_SWIFT="$(mktemp -t menubarcheck).swift"
cat > "$GEOM_SWIFT" <<'SW'
import AppKit
let s = NSScreen.main!
let l = s.auxiliaryTopLeftArea
let r = s.auxiliaryTopRightArea
if let l = l, let r = r {
    print("\(l.maxX) \(r.minX)")
} else {
    print("none none")
}
SW
read -r NOTCH_L NOTCH_R < <(swift "$GEOM_SWIFT")
rm -f "$GEOM_SWIFT"

if [ "$NOTCH_L" = "none" ]; then
  echo "notch: this display has none"
else
  echo "notch: x $NOTCH_L .. $NOTCH_R"
  ITEM_R=$((X + W))
  if awk "BEGIN{exit !($X < $NOTCH_R && $ITEM_R > $NOTCH_L)}"; then
    echo
    echo "OCCLUDED: the item's frame overlaps the notch, so macOS will not draw"
    echo "it however the label is built. This is a full menu bar on this"
    echo "machine, NOT a rendering defect in the app. Free space in the menu"
    echo "bar and re-run before concluding anything about the mark."
    exit 2
  fi
fi

SHOT="${TMPDIR:-/tmp}/menu-bar-item-$PID.png"
screencapture -x -R "$X,$Y,$W,$H" "$SHOT"
echo "captured: $SHOT"
echo "Look at it. An item that is present but blank is the failure mode this"
echo "script cannot judge for you; ImageRenderer-based harnesses render the"
echo "view happily while the real status item stays empty."
