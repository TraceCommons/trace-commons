#!/usr/bin/env bash
#
# build-cuda-bakeoff.sh — reproducible CUDA build of trace-commons-gate-calibrate
# on a fresh Lambda H100 SXM5 (Ubuntu 22.04). Captures the toolchain quirks
# we hit on 2026-05-14 / 2026-05-15.
#
# Usage:
#   bash scripts/operator/build-env/build-cuda-bakeoff.sh
#
# Pre-requisites on the host:
#   - CUDA 12.x toolkit installed (Lambda H100 images ship with this).
#   - Internet access (apt + cargo registry + GitHub for mistralrs).
#
# What this script handles (lessons from 2026-05-14 attempts 1 + 2):
#   1. Installs gcc-12: numkong's build script uses
#      __attribute__((target("avx512fp16"))) / -mamx-fp16 which require
#      GCC 12+. Ubuntu 22.04's default GCC 11.4 produces "unrecognized
#      command-line option" errors.
#   2. Installs Rust via rustup if not already present.
#   3. Compiles isoc23_shim.o: the ort precompiled binary (ONNX Runtime)
#      references glibc 2.38 symbols (__isoc23_strtol, __isoc23_strtoll,
#      __isoc23_strtoul, __isoc23_strtoull, and the _l locale variants).
#      Ubuntu 22.04 ships glibc 2.35 which lacks these. The shim
#      defines them as thin wrappers over the non-C23 equivalents; the
#      semantic difference (stricter overflow handling on edge inputs)
#      is benign for ORT inference paths.
#   4. Builds trace-commons-gate-calibrate with --features local-gpu-models-cuda
#      and CC=gcc-12 so the right compiler picks up the numkong build.
#
# Build time on H100 SXM5 80GB: ~6 minutes after dependency installs.

set -euo pipefail

REPO_ROOT="${REPO_ROOT:-$HOME/work/trace-commons-server}"
SHIM_C="$(dirname "$0")/isoc23_shim.c"
SHIM_O="/tmp/isoc23_shim.o"

echo "=== installing system deps ==="
sudo apt-get update -qq
sudo apt-get install -y -qq git build-essential pkg-config libssl-dev cmake zstd python3-pip gcc-12 g++-12 >/dev/null

echo "=== installing rust (if missing) ==="
if ! command -v cargo >/dev/null; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --quiet
fi
source "$HOME/.cargo/env"
rustc --version

echo "=== compiling isoc23 shim ==="
gcc-12 -O2 -fPIC -D_GNU_SOURCE -c "$SHIM_C" -o "$SHIM_O"

echo "=== building trace-commons-gate-calibrate (CUDA) ==="
cd "$REPO_ROOT"
RUSTFLAGS="-C link-arg=${SHIM_O}" \
  CC=gcc-12 \
  CXX=g++-12 \
  cargo build --release \
    -p trace-commons-server \
    --bin trace-commons-gate-calibrate \
    --features local-gpu-models-cuda

echo ""
echo "OK: binary at $REPO_ROOT/target/release/trace-commons-gate-calibrate"
ls -la "$REPO_ROOT/target/release/trace-commons-gate-calibrate"
