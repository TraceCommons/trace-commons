#!/usr/bin/env bash
# Run the GNOME Shell extension against a real, headless GNOME Shell and the
# app's own client (`edge::gnome::live_tests::a_running_shell_holds_the_rail_and_the_panel`).
#
#   gnome-shell-extension-verify.sh
#
# Needs gnome-shell (49 or later for --headless/--virtual-monitor as used
# here), dbus-run-session, gsettings, unzip and zip, and a cargo that can
# build this crate (without the layer-shell feature, which GNOME has no use
# for, so the box needs no gtk4-layer-shell library). Everything it writes is under a temporary directory it
# removes: the shell gets its own session bus, runtime directory, Wayland
# display name, data directory (where the extension is installed) and
# settings database, so the desktop it runs beside is not touched.
#
# The shell is told to load one more extension, written here and nowhere
# else: it sets the shell's unsafe mode so the test can read Mutter's
# answer back over org.gnome.Shell.Eval. It is never packed or shipped.
#
# Not run in CI: no runner has GNOME Shell. Weston has no GNOME Shell API
# either, so the weston job does not run the test.
set -euo pipefail

crate="$(cd "$(dirname "$0")/.." && pwd)"

# Set before the private bus starts, because the services it activates
# (dconf among them) take the bus's environment, not this script's later
# one: set afterwards, the settings below would land in the real desktop's
# database.
if [ "${1:-}" != "--inner" ]; then
  work="$(mktemp -d)"
  chmod 700 "$work"
  export XDG_RUNTIME_DIR="$work/run" XDG_DATA_HOME="$work/data" XDG_CONFIG_HOME="$work/config"
  export XDG_CACHE_HOME="$work/cache"
  mkdir -p "$XDG_RUNTIME_DIR" "$XDG_DATA_HOME/gnome-shell/extensions" "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME"
  chmod 700 "$XDG_RUNTIME_DIR"
  unset WAYLAND_DISPLAY DISPLAY DBUS_SESSION_BUS_ADDRESS
  status=0
  dbus-run-session -- "$0" --inner "$work" || status=$?
  # A GTK app on the private bus starts the document portal, which mounts
  # a FUSE filesystem in the runtime directory; it is gone once the bus is,
  # but unmount it if not, so the directory can be removed.
  if mountpoint -q "$work/run/doc" 2>/dev/null; then
    fusermount3 -u "$work/run/doc" 2>/dev/null || fusermount -u "$work/run/doc" 2>/dev/null || true
  fi
  rm -rf "$work" || echo "left behind: $work" >&2
  exit "$status"
fi
work="$2"
trap 'kill "${shell_pid:-0}" 2>/dev/null || true' EXIT
case "$XDG_CONFIG_HOME" in
  "$work"/*) ;;
  *) echo "refusing to write settings outside $work" >&2; exit 1 ;;
esac

zip_path="$("$crate/scripts/pack-gnome-shell-extension.sh" "$work")"
uuid="$(basename "$zip_path" .shell-extension.zip)"
mkdir -p "$XDG_DATA_HOME/gnome-shell/extensions/$uuid"
unzip -q "$zip_path" -d "$XDG_DATA_HOME/gnome-shell/extensions/$uuid"

unsafe=unsafe-mode@test.invalid
mkdir -p "$XDG_DATA_HOME/gnome-shell/extensions/$unsafe"
cat > "$XDG_DATA_HOME/gnome-shell/extensions/$unsafe/metadata.json" <<JSON
{"uuid": "$unsafe", "name": "test", "description": "test", "shell-version": ["$(gnome-shell --version | sed 's/[^0-9]*\([0-9]*\).*/\1/')"]}
JSON
cat > "$XDG_DATA_HOME/gnome-shell/extensions/$unsafe/extension.js" <<'JS'
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
export default class extends Extension {
    enable() { global.context.unsafe_mode = true; }
    disable() {}
}
JS

gsettings set org.gnome.shell disable-user-extensions false
gsettings set org.gnome.shell enabled-extensions "['$uuid', '$unsafe']"

display=gext-wayland
gnome-shell --headless --no-x11 --virtual-monitor 1920x1080 --wayland-display "$display" \
  > "$work/shell.log" 2>&1 &
shell_pid=$!

# The extension owns its name once enabled; wait for that, not a sleep.
for _ in $(seq 1 60); do
  if gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus \
      --method org.freedesktop.DBus.NameHasOwner ai.tracecommons.Contributor.EdgeSurfaces \
      2>/dev/null | grep -q true; then
    ready=1
    break
  fi
  sleep 1
done
if [ -z "${ready:-}" ]; then
  echo "the extension never appeared on the bus" >&2
  gnome-extensions info "$uuid" >&2 || true
  grep -iE "extension|error|exception" "$work/shell.log" >&2 || true
  exit 1
fi

status=0
WAYLAND_DISPLAY="$display" GDK_BACKEND=wayland \
  cargo test --manifest-path "$crate/Cargo.toml" --lib --no-default-features \
  edge::gnome::live_tests::a_running_shell_holds_the_rail_and_the_panel \
  -- --exact --ignored --test-threads=1 || status=$?

if [ "$status" -ne 0 ]; then
  echo "shell log:" >&2
  grep -iE "error|warn|exception|contributor" "$work/shell.log" >&2 || true
fi
exit "$status"
