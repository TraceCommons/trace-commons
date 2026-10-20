#!/usr/bin/env bash
# Exercise scripts/install.sh against a fake release served from disk.
#
# An installer is the first code a user runs and the only code that runs before
# they have anything to debug with, so "it looked right" is not a standard it
# gets to be held to. This builds a release-shaped tree, points the installer at
# it, and checks what actually lands on disk — including the failure paths,
# which are the ones nobody exercises by hand.
#
# Run: scripts/test-install.sh

set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
installer="${here}/install.sh"
work=$(mktemp -d "${TMPDIR:-/tmp}/ironwire-install-test.XXXXXX")
trap 'rm -rf "$work"' EXIT

pass=0
fail=0

ok()   { printf '  \033[32mok\033[0m   %s\n' "$1"; pass=$((pass + 1)); }
bad()  { printf '  \033[31mFAIL\033[0m %s\n' "$1"; printf '       %s\n' "${2:-}"; fail=$((fail + 1)); }

# --------------------------------------------------------------- fake release

# A stand-in binary that behaves enough like the real one for the installer:
# it is executable and answers --version.
make_release() {
    local platform="$1" dir="$2" with_checksum="${3:-yes}"
    mkdir -p "${dir}/latest/download" "${work}/stage"
    cat >"${work}/stage/ironwire" <<'EOF'
#!/bin/sh
[ "$1" = "--version" ] && echo "ironwire 0.1.0-test"
EOF
    chmod 755 "${work}/stage/ironwire"
    tar -czf "${dir}/latest/download/ironwire-${platform}.tar.gz" -C "${work}/stage" ironwire
    if [ "$with_checksum" = "yes" ]; then
        (cd "${dir}/latest/download" &&
            sha256sum "ironwire-${platform}.tar.gz" >"ironwire-${platform}.tar.gz.sha256")
    fi
}

# The installer fetches with curl/wget, which handle file:// URLs.
run_installer() {
    env \
        IRONWIRE_INSTALL_BASE_URL="file://${work}/release" \
        IRONWIRE_INSTALL_DIR="${work}/bin" \
        IRONWIRE_INSTALL_PLATFORM="x86_64-unknown-linux-gnu" \
        NO_COLOR=1 \
        "$@" \
        sh "$installer"
}

echo "install.sh"

# ------------------------------------------------------------------ syntax

if sh -n "$installer" 2>/dev/null; then
    ok "is valid POSIX sh"
else
    bad "is valid POSIX sh" "sh -n rejected it"
fi

# The whole point of writing it in sh is that it runs without bash.
if command -v dash >/dev/null 2>&1 && dash -n "$installer" 2>/dev/null; then
    ok "parses under dash (no bashisms)"
elif ! command -v dash >/dev/null 2>&1; then
    printf '  \033[2mskip\033[0m dash not available\n'
fi

# ------------------------------------------------------------------ dry run

out=$(run_installer IRONWIRE_INSTALL_DRY_RUN=1 2>&1) || true
if grep -q "nothing downloaded" <<<"$out" && [ ! -e "${work}/bin/ironwire" ]; then
    ok "dry run downloads nothing"
else
    bad "dry run downloads nothing" "$out"
fi

# ------------------------------------------------------------ happy path

make_release "x86_64-unknown-linux-gnu" "${work}/release"
out=$(run_installer 2>&1) || bad "install succeeds" "$out"

if [ -x "${work}/bin/ironwire" ]; then
    ok "installs an executable binary"
else
    bad "installs an executable binary" "$out"
fi

if grep -q "checksum ok" <<<"$out"; then
    ok "verifies the published checksum"
else
    bad "verifies the published checksum" "$out"
fi

if grep -q "0.1.0-test" <<<"$out"; then
    ok "reports the installed version"
else
    bad "reports the installed version" "$out"
fi

# PATH guidance is the difference between a working install and a user who
# thinks the install failed.
if grep -q "PATH" <<<"$out"; then
    ok "tells the user how to fix their PATH"
else
    bad "tells the user how to fix their PATH" "$out"
fi

# ...and does not nag when the directory is already on PATH.
out=$(PATH="${work}/bin:$PATH" run_installer 2>&1) || true
if grep -q "already on your PATH" <<<"$out"; then
    ok "says nothing to add when PATH is already right"
else
    bad "says nothing to add when PATH is already right" "$out"
fi

# ---------------------------------------------------------- reinstall is safe

before=$(sha256sum "${work}/bin/ironwire" | cut -d' ' -f1)
run_installer >/dev/null 2>&1 || true
after=$(sha256sum "${work}/bin/ironwire" | cut -d' ' -f1)
if [ "$before" = "$after" ] && [ ! -e "${work}/bin/.ironwire.new" ]; then
    ok "reinstalling is idempotent and leaves no temp file"
else
    bad "reinstalling is idempotent and leaves no temp file" "temp file left behind"
fi

# ------------------------------------------------------- corrupted download

rm -rf "${work:?}/release" "${work:?}/bin"
make_release "x86_64-unknown-linux-gnu" "${work}/release"
# Publish a checksum that does not match what is actually there.
echo "0000000000000000000000000000000000000000000000000000000000000000  ironwire-x86_64-unknown-linux-gnu.tar.gz" \
    >"${work}/release/latest/download/ironwire-x86_64-unknown-linux-gnu.tar.gz.sha256"

if out=$(run_installer 2>&1); then
    bad "refuses a checksum mismatch" "installer exited 0 on a bad checksum"
elif grep -q "checksum mismatch" <<<"$out" && [ ! -e "${work}/bin/ironwire" ]; then
    ok "refuses a checksum mismatch and installs nothing"
else
    bad "refuses a checksum mismatch and installs nothing" "$out"
fi

# ------------------------------------------------------------ missing release

rm -rf "${work:?}/release" "${work:?}/bin"
mkdir -p "${work}/release/latest/download"
if out=$(run_installer 2>&1); then
    bad "fails when no artifact exists" "installer exited 0"
elif grep -qi "could not download" <<<"$out" && grep -q "releases" <<<"$out"; then
    ok "fails on a missing artifact and links the releases page"
else
    bad "fails on a missing artifact and links the releases page" "$out"
fi

# ------------------------------------------------------- unsupported platform

# Faking `uname` for a POSIX-sh subprocess needs `export -f`, which is itself a
# bashism — so this asserts against the source rather than pretending to
# execute a path it cannot reach. Stated, not hidden: the branch is checked for
# content, not behaviour.
if grep -q "unsupported architecture" "$installer" &&
   grep -q "cargo install --git" "$installer"; then
    ok "an unsupported platform is told how to build from source"
else
    bad "an unsupported platform is told how to build from source" "no fallback guidance"
fi

# ------------------------------------------------------------------- summary

echo
if [ "$fail" -eq 0 ]; then
    printf '\033[32m%d passed\033[0m\n' "$pass"
else
    printf '\033[31m%d failed\033[0m, %d passed\n' "$fail" "$pass"
    exit 1
fi
