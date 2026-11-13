#!/bin/bash
set -euo pipefail
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
if ! command -v rustup >/dev/null 2>&1; then
  case "$(uname -m)" in
    arm64) host=aarch64-apple-darwin; expected=63752a1919a3a4793173ec70f8f212621a7ce38ba6a246095984482f4de20f9f ;;
    x86_64) host=x86_64-apple-darwin; expected=bc107f1fb0228de7fa32413c89c1b305cea9cd256e94b01150cd054128145429 ;;
    *) echo 'refusing: unsupported Rust bootstrap host' >&2; exit 1 ;;
  esac
  bootstrap="$(mktemp -d)"
  trap 'rm -rf "$bootstrap"' EXIT
  curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 \
    "https://static.rust-lang.org/rustup/archive/1.29.3/$host/rustup-init" -o "$bootstrap/rustup-init"
  actual="$(shasum -a 256 "$bootstrap/rustup-init" | awk '{print $1}')"
  [[ "$actual" == "$expected" ]] || { echo 'refusing: Rust bootstrap checksum mismatch' >&2; exit 1; }
  chmod +x "$bootstrap/rustup-init"
  "$bootstrap/rustup-init" -y --no-modify-path --default-toolchain none
fi
rustup toolchain install 1.96.0 --profile minimal --target aarch64-apple-darwin --target x86_64-apple-darwin
rustup run 1.96.0 rustc --version
