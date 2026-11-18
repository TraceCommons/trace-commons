#!/usr/bin/env bash
#
# Start every offered Linux terminal that is installed here, the way a
# managed session opens one, and check that the session folder and every
# helper argument arrive intact. The test it runs,
# `managed::terminal::tests::real_linux_terminals_receive_the_folder_and_every_argument`,
# opens each terminal with a stub helper that writes its working directory and
# its arguments to a file and exits. The folder holds spaces, both quotes,
# `$`, a backtick and a leading dash.
#
#   scripts/linux/managed-terminal-launch-smoke.sh
#
# Run it on a Linux desktop, from a terminal inside the graphical session
# (it needs DISPLAY or WAYLAND_DISPLAY), from the checkout to test. Run it
# twice: once with no terminal running, and once with each terminal already
# open, since GNOME Terminal and Xfce Terminal hand a new window to a
# running instance. Terminals that hold their window after the command
# exits (Konsole, Xfce Terminal, kitty, WezTerm, Alacritty, Ghostty, foot)
# leave one window each: close them by hand. foot runs only under Wayland.
#
# It fails when no offered terminal is installed, so a pass always means at
# least one terminal was started. It prints the terminals it started.
set -euo pipefail

if [ "$(uname -s)" != "Linux" ]; then
  echo "managed-terminal-launch-smoke: Linux only" >&2
  exit 2
fi
if [ -z "${DISPLAY:-}" ] && [ -z "${WAYLAND_DISPLAY:-}" ]; then
  echo "managed-terminal-launch-smoke: run inside a graphical session" >&2
  exit 2
fi

cd "$(git rev-parse --show-toplevel)"
exec cargo test -p trace-commons-contributor --lib -- \
  --ignored --exact --nocapture --test-threads=1 \
  'managed::terminal::tests::real_linux_terminals_receive_the_folder_and_every_argument'
