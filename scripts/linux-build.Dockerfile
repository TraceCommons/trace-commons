# Build and run environment for the Linux contributor shell.
#
# The shell is a GTK 4 / libadwaita application, so it cannot be built on a
# macOS or Windows host at all. This image is the reproducible Linux
# toolchain: everything the crate links against, plus Xvfb and a session bus
# so the application can actually be started headlessly rather than only
# compiled.
#
# Versions here are what Debian bookworm ships: GTK 4.8.3 and libadwaita
# 1.2.2. That is deliberately an old-but-widely-deployed pair -- if the shell
# builds and runs against it, it runs on the distributions contributors
# actually have. Do not raise the base image to get newer GTK without
# checking what that costs in reach.
FROM rust:1-bookworm

RUN apt-get update && apt-get install -y --no-install-recommends \
      pkg-config \
      libgtk-4-dev \
      libadwaita-1-dev \
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
