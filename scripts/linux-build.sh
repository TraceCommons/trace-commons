#!/usr/bin/env bash
#
# Build and run the Linux contributor shell inside a container.
#
# The shell links GTK 4 and libadwaita, so it does not build on the macOS
# host this repository is usually developed on. This script is the whole
# incantation: it builds the toolchain image once, keeps Cargo's registry and
# target directory in named volumes so rebuilds are incremental, and runs
# whatever command you give it inside the crate directory.
#
#   scripts/linux-build.sh                    # cargo build
#   scripts/linux-build.sh cargo clippy       # anything else
#   scripts/linux-build.sh --shell            # interactive shell
#   scripts/linux-build.sh --run-headless     # start the app under Xvfb
#   scripts/linux-build.sh --roots-shot       # photograph the roots window
#   scripts/linux-build.sh --roots-answer     # click through it, check output
#   scripts/linux-build.sh --probe            # talk to a throwaway daemon
#
# Nothing here touches the host toolchain, and the host workspace still
# builds on macOS because the GTK crate is excluded from it.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE=trace-commons-linux-build
CARGO_VOLUME=trace-commons-linux-cargo
TARGET_VOLUME=trace-commons-linux-target
CRATE_DIR=crates/trace-commons-contributor-gtk

build_image() {
  docker build -q -t "$IMAGE" -f "$REPO_ROOT/scripts/linux-build.Dockerfile" "$REPO_ROOT/scripts" >/dev/null
}

run() {
  docker run --rm -i \
    -v "$REPO_ROOT:/work" \
    -v "$CARGO_VOLUME:/cargo" \
    -v "$TARGET_VOLUME:/target" \
    -w "/work/$CRATE_DIR" \
    "$IMAGE" \
    bash -c "$1"
}

build_image

case "${1:---build}" in
  --build)
    run "cargo build"
    ;;
  --shell)
    docker run --rm -it \
      -v "$REPO_ROOT:/work" \
      -v "$CARGO_VOLUME:/cargo" \
      -v "$TARGET_VOLUME:/target" \
      -w "/work/$CRATE_DIR" \
      "$IMAGE" bash
    ;;
  --probe)
    # Milestone check: start a real daemon on a throwaway 0700 state
    # directory, then have the shell's client layer connect over the socket
    # and print what it got back. Proves the crate links the contributor core
    # and speaks the v1_1 contract, without needing a display.
    run "bash /work/$CRATE_DIR/scripts/probe.sh"
    ;;
  --roots-shot)
    # Photographs the roots-declaration window: no daemon, no settings file,
    # discovery pointed at fixture stores. The one check that answers "what
    # does this window look like", which no unit test can.
    run "bash /work/$CRATE_DIR/scripts/roots-shot.sh"
    ;;
  --onboarding-shots)
    # Photographs every onboarding page that styles itself. Onboarding was
    # merged carrying four class names no stylesheet defined; a camera is the
    # only thing that shows a widget rendering in GTK's defaults, because
    # `add_css_class` accepts any string without complaint.
    run "bash /work/$CRATE_DIR/scripts/onboarding-shots.sh"
    ;;
  --removed-dialog-shot)
    # Photographs the "What gets removed?" dialog, whose list is generated
    # from the protocol's detector table. Needs a click, so onboarding-shots
    # cannot reach it.
    run "bash /work/$CRATE_DIR/scripts/removed-dialog-shot.sh"
    ;;
  --roots-answer)
    # Drives the roots window with xdotool and asserts that the two answers
    # reach daemon-settings.json as a Watch and an Off. The screenshot proves
    # the window reads correctly; this proves the controls do what they say.
    run "bash /work/$CRATE_DIR/scripts/roots-answer.sh"
    ;;
  --run-headless)
    # Starts the real application under Xvfb with a private session bus.
    # This proves the process starts, realizes its widgets and reaches the
    # daemon. It does not prove the layout looks right -- nobody sees it.
    run "bash /work/$CRATE_DIR/scripts/headless-run.sh"
    ;;
  *)
    run "$*"
    ;;
esac
