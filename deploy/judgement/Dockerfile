# The Insights judgement service, built for a dstack TDX guest.
#
# One Dockerfile, two images. PROFILE picks the cargo features, and so what
# is compiled into the binary:
#
#   production   --features near-attestation-collateral
#                The production adapters only, plus the Intel DCAP collateral
#                client that the boot-time gateway attestation needs. No
#                development model is in this binary, so none can be
#                switched on by configuration.
#   development  --features judgement-development-models
#                Adds the development models (the mock now, Jev in slice 6).
#                Synthetic or developer-owned data only.
#
# The two are pushed under different names and deployed as different dstack
# apps, so they never share an app id or a derived key. See README.md and
# spec section 2 (docs/superpowers/specs/2026-10-20-insights-judgement-service-design.md).
#
# THIS BUILD IS NOT REPRODUCIBLE, for the reasons deploy/witness/README.md
# gives under "Reproducibility": a measurement pins one artifact. The pins
# below reduce drift; they do not remove it.

ARG RUST_IMAGE=rust:1.96.1-bookworm
ARG RUNTIME_IMAGE=debian:bookworm-slim

FROM ${RUST_IMAGE} AS builder

# The server crate's library pulls openssl-sys (through webauthn-rs) and
# builds zstd from source; the judgement binary links that library whole.
RUN apt-get update && apt-get install -y --no-install-recommends \
      pkg-config \
      libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Required, with no default: an image built without saying which profile it
# is must not exist.
ARG PROFILE
ARG TRACE_COMMONS_BUILD_COMMIT=unknown
ARG SOURCE_DATE_EPOCH=0
ENV TRACE_COMMONS_BUILD_COMMIT=${TRACE_COMMONS_BUILD_COMMIT}
ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}
ENV RUSTFLAGS="--remap-path-prefix=/src=/build"

WORKDIR /src
COPY . .

RUN set -eu; \
    case "${PROFILE:-}" in \
      production) features="near-attestation-collateral" ;; \
      development) features="judgement-development-models" ;; \
      *) echo "PROFILE must be production or development" >&2; exit 1 ;; \
    esac; \
    cargo build --locked --release --bin trace-commons-judgement --features "$features" \
    && strip /src/target/release/trace-commons-judgement

FROM ${RUNTIME_IMAGE}

# GHCR links a package to its repository through this label; without it the
# first push lands private and an anonymous CVM pull gets 403.
LABEL org.opencontainers.image.source="https://github.com/TraceCommons/trace-commons"

# ca-certificates: the NEAR AI catalog, gateway report and Intel PCS are
# reached over TLS. libssl3 is the runtime half of libssl-dev above.
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates \
      libssl3 \
    && rm -rf /var/lib/apt/lists/*

# The limiter volume's mount point, owned by the service user. Docker copies
# an image directory's ownership into an empty named volume on its first
# mount, so the volume starts writable by the service. The directory alone
# is not the volume: the service refuses to boot unless a filesystem is
# mounted on it (`judgement_limiter_volume_absent`).
RUN useradd --system --no-create-home --uid 10001 judgement \
    && mkdir /data \
    && chown 10001:10001 /data
COPY --from=builder /src/target/release/trace-commons-judgement /usr/local/bin/trace-commons-judgement
USER judgement

# No arguments. Every setting is an environment variable in the measured
# compose file.
ENTRYPOINT ["/usr/local/bin/trace-commons-judgement"]
