# Build and run environment for the Linux contributor shell.
#
# The shell is a GTK 4 / libadwaita application, so it cannot be built on a
# macOS or Windows host at all. This image is the reproducible Linux
# toolchain: everything the crate links against, plus Xvfb and a session bus
# so the application can actually be started headlessly rather than only
# compiled.
#
# The base is Debian trixie (13). It was bookworm, for GTK 4.8.3 and
# libadwaita 1.2.2, an old-but-widely-deployed pair. The owner raised the
# shell's libadwaita floor to 1.4 on 2026-11-16 (the crate builds with
# feature `v1_4`), and bookworm's 1.2 can no longer build it. Do not raise
# the base again to get newer GTK without checking what that costs in reach.
#
# Not rebuilt since the move: confirm the image builds, and which GTK and
# libadwaita it installs, before relying on it.
FROM rust:1-trixie

RUN apt-get update && apt-get install -y --no-install-recommends \
      pkg-config \
      libgtk-4-dev \
      libadwaita-1-dev \
      libgtk4-layer-shell-dev \
      libnotify-dev \
      xvfb \
      xauth \
      x11-apps \
      imagemagick \
      xdotool \
      dbus \
      dbus-x11 \
      jq \
    && rm -rf /var/lib/apt/lists/*

# rustfmt and clippy so the crate is held to the same bar as the rest of
# the repository, even though CI cannot build it.
RUN rustup component add rustfmt clippy

# Cargo's registry and the target directory are mounted as named volumes by
# scripts/linux-build.sh, so a rebuild does not re-download or re-compile the
# dependency tree.
ENV CARGO_HOME=/cargo
ENV CARGO_TARGET_DIR=/target

WORKDIR /work
