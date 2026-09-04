# The redaction witness, built for a dstack TDX guest.
#
# THIS BUILD IS NOT REPRODUCIBLE. Two runs of this file from the same commit
# produce different image digests, and therefore different measurements. The
# reasons are enumerated in README.md under "Reproducibility"; three of the
# four are fixable and none of them are fixed here. Read that section before
# you pin anything this image produces.
#
# What the pins below buy is *reduction of drift*, not reproducibility: an
# exact toolchain version, `--locked` so the dependency set is the lockfile's,
# and build-time identity supplied from outside so the binary is not stamped
# with a wall clock. Set RUST_IMAGE and RUNTIME_IMAGE to `name@sha256:...`
# digests to close the base-image half.

ARG RUST_IMAGE=rust:1.96.1-bookworm
ARG RUNTIME_IMAGE=debian:bookworm-slim

FROM ${RUST_IMAGE} AS builder

# openssl and zstd are not the witness's own dependencies. They arrive through
# `trace-commons-server`'s library, which the witness binary links whole --
# webauthn-rs pulls `openssl-sys`, and `zstd` builds from source. See the
# README's note on what is inside this image: the witness's blast radius is
# the whole server crate's dependency tree, not the witness's own code.
RUN apt-get update && apt-get install -y --no-install-recommends \
      pkg-config \
      libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Supplied by the caller so the binary's own identity is not a wall clock.
# `trace-commons-build-info`'s build script reads both: TRACE_COMMONS_BUILD_COMMIT
# because .gcloudignore strips .git from deployed builds, and SOURCE_DATE_EPOCH
# because that is the standard reproducible-builds lever and the script honours
# it in preference to SystemTime::now(). Left unset, every build of the same
# commit stamps a different timestamp into the binary.
ARG TRACE_COMMONS_BUILD_COMMIT=unknown
ARG SOURCE_DATE_EPOCH=0
ENV TRACE_COMMONS_BUILD_COMMIT=${TRACE_COMMONS_BUILD_COMMIT}
ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}

# Absolute paths from the build host end up in panic messages and debug info.
# Remapping them keeps the host's directory layout out of the artifact and
# removes one source of digest drift between two machines building the same
# commit.
ENV RUSTFLAGS="--remap-path-prefix=/src=/build"

WORKDIR /src
COPY . .

# `--locked` refuses to update Cargo.lock. A build that silently resolved a new
# patch release would move the measurement with no commit behind it, which is
# the one thing a measurement is supposed to make impossible.
RUN cargo build --locked --release --bin trace-commons-witness \
    && strip /src/target/release/trace-commons-witness

FROM ${RUNTIME_IMAGE}

# Names the repository this image was built from, in the image itself.
#
# Not decoration. GHCR links a package to a repository through this label and
# nothing else, and an unlinked package cannot inherit that repository's
# visibility -- so the first push of this image landed PRIVATE even though the
# repository is public and the organisation is set to inherit access. A CVM
# pulling it anonymously then gets 403, which surfaces as an opaque deploy
# failure rather than a permissions error.
#
# It also puts provenance where an auditor can read it. This image is not
# reproducibly buildable, so a measurement pins an artifact rather than a
# source tree; the least the artifact can do is say where it came from.
LABEL org.opencontainers.image.source="https://github.com/TraceCommons/trace-commons"

# ca-certificates is needed only by the `full-pipeline` redaction mode when its
# classifier backend is reached over TLS. `deterministic-only` makes no network
# call at all. libssl3 is the runtime half of the build dependency above.
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates \
      libssl3 \
    && rm -rf /var/lib/apt/lists/*

# The witness holds raw transcripts in memory and writes nothing. It runs as a
# non-root user for the ordinary reason, and the one thing that can break is
# named in the README: it must be able to open the dstack guest-agent socket.
# If your dstack version creates that socket root-only, the container fails to
# start with a named error at boot rather than running without an identity --
# see "First boot".
RUN useradd --system --no-create-home --uid 10001 witness
COPY --from=builder /src/target/release/trace-commons-witness /usr/local/bin/trace-commons-witness
USER witness

# No CMD arguments. Every knob is an environment variable set in the compose
# file, which is what puts the configuration inside MRCONFIGID -- see the
# README's "The configuration is measured".
ENTRYPOINT ["/usr/local/bin/trace-commons-witness"]
