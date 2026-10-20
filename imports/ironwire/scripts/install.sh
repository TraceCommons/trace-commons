#!/bin/sh
# IronWire installer.
#
#   curl -fsSL https://ironwire.dev/install.sh | sh
#
# Installs a single static binary to ~/.ironwire/bin and tells you what to add
# to your PATH. Never needs sudo, never writes outside $IRONWIRE_HOME and the
# install prefix, and never starts anything.
#
# POSIX sh on purpose: this is the fallback path for platforms the package
# managers miss, which includes machines without bash.
#
# Testing hooks, used by scripts/test-install.sh:
#   IRONWIRE_INSTALL_BASE_URL   where to fetch releases from
#   IRONWIRE_INSTALL_DIR        where to put the binary
#   IRONWIRE_INSTALL_PLATFORM   override platform detection
#   IRONWIRE_INSTALL_DRY_RUN=1  resolve and report, download nothing

set -eu

REPO="nearai/ironwire"
BASE_URL="${IRONWIRE_INSTALL_BASE_URL:-https://github.com/${REPO}/releases}"
INSTALL_DIR="${IRONWIRE_INSTALL_DIR:-${HOME}/.ironwire/bin}"
VERSION="${IRONWIRE_INSTALL_VERSION:-latest}"
DRY_RUN="${IRONWIRE_INSTALL_DRY_RUN:-0}"

# ---------------------------------------------------------------- output

# Colour only when stdout is a terminal. `curl | sh` is not, and escape codes
# in a CI log are noise.
if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
    BOLD=$(printf '\033[1m'); DIM=$(printf '\033[2m')
    RED=$(printf '\033[31m'); GREEN=$(printf '\033[32m'); RESET=$(printf '\033[0m')
else
    BOLD=''; DIM=''; RED=''; GREEN=''; RESET=''
fi

say()  { printf '%s\n' "$*"; }
step() { printf '%s==>%s %s\n' "$BOLD" "$RESET" "$*"; }
warn() { printf '%swarning:%s %s\n' "$RED" "$RESET" "$*" >&2; }

die() {
    printf '%serror:%s %s\n' "$RED" "$RESET" "$1" >&2
    shift
    # Every failure says what to do next. An installer that only says "failed"
    # sends people to an issue tracker for something they could have fixed.
    for line in "$@"; do printf '  %s\n' "$line" >&2; done
    exit 1
}

# ---------------------------------------------------------------- platform

detect_platform() {
    if [ -n "${IRONWIRE_INSTALL_PLATFORM:-}" ]; then
        printf '%s' "$IRONWIRE_INSTALL_PLATFORM"
        return
    fi

    os=$(uname -s)
    arch=$(uname -m)

    case "$arch" in
        x86_64 | amd64) arch=x86_64 ;;
        arm64 | aarch64) arch=aarch64 ;;
        *) die "unsupported architecture: $arch" \
               "IronWire ships x86_64 and aarch64." \
               "Build from source: cargo install --git https://github.com/${REPO}" ;;
    esac

    case "$os" in
        Darwin) printf '%s-apple-darwin' "$arch" ;;
        Linux)
            # Prefer musl on anything that is not demonstrably glibc. A static
            # binary that runs everywhere beats a dynamic one that fails on the
            # user's distro with a linker error they cannot act on.
            if is_glibc; then
                printf '%s-unknown-linux-gnu' "$arch"
            else
                printf '%s-unknown-linux-musl' "$arch"
            fi
            ;;
        MINGW* | MSYS* | CYGWIN*)
            die "this installer does not support Windows shells" \
                "Use:  winget install ironwire" \
                "or download the MSI from https://github.com/${REPO}/releases" ;;
        *) die "unsupported OS: $os" \
               "Build from source: cargo install --git https://github.com/${REPO}" ;;
    esac
}

is_glibc() {
    # `ldd --version` prints to stdout on glibc and stderr on musl, and musl's
    # ldd exits non-zero. Checking both is more reliable than parsing either.
    if command -v ldd >/dev/null 2>&1; then
        ldd --version 2>&1 | head -n1 | grep -qi 'gnu\|glibc' && return 0
    fi
    # aarch64 gnu-only builds are the common case on Linux ARM servers; only
    # x86_64 has a musl artifact, so anything else has to try gnu.
    case "$(uname -m)" in
        x86_64 | amd64) return 1 ;;
        *) return 0 ;;
    esac
}

# ---------------------------------------------------------------- fetching

have() { command -v "$1" >/dev/null 2>&1; }

fetch() {
    # $1 url, $2 destination ('-' for stdout)
    if have curl; then
        if [ "$2" = "-" ]; then
            curl -fsSL --retry 3 --retry-delay 1 "$1"
        else
            curl -fsSL --retry 3 --retry-delay 1 -o "$2" "$1"
        fi
    elif have wget; then
        if [ "$2" = "-" ]; then
            wget -qO- "$1"
        else
            wget -qO "$2" "$1"
        fi
    else
        die "neither curl nor wget is available" \
            "Install one of them, or download manually from:" \
            "  https://github.com/${REPO}/releases"
    fi
}

checksum() {
    # Print the sha256 of $1, or nothing if we have no way to compute it.
    if have sha256sum; then
        sha256sum "$1" | cut -d' ' -f1
    elif have shasum; then
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

# ---------------------------------------------------------------- install

main() {
    platform=$(detect_platform)

    if [ "$VERSION" = "latest" ]; then
        url_base="${BASE_URL}/latest/download"
    else
        url_base="${BASE_URL}/download/v${VERSION#v}"
    fi

    archive="ironwire-${platform}.tar.gz"
    step "Installing ironwire for ${BOLD}${platform}${RESET}"

    if [ "$DRY_RUN" = "1" ]; then
        say "  archive:  ${url_base}/${archive}"
        say "  into:     ${INSTALL_DIR}"
        say "${DIM}(dry run — nothing downloaded)${RESET}"
        return 0
    fi

    tmp=$(mktemp -d "${TMPDIR:-/tmp}/ironwire.XXXXXX") ||
        die "could not create a temporary directory"
    # shellcheck disable=SC2064  # expand $tmp now, deliberately
    trap "rm -rf '$tmp'" EXIT INT TERM

    say "  downloading ${archive}"
    fetch "${url_base}/${archive}" "${tmp}/${archive}" ||
        die "could not download ${url_base}/${archive}" \
            "Check that a release exists for your platform:" \
            "  https://github.com/${REPO}/releases"

    # Verify if we can, and say plainly when we cannot rather than implying we
    # did. A silent skip is worse than a stated one.
    if fetch "${url_base}/${archive}.sha256" "${tmp}/${archive}.sha256" 2>/dev/null; then
        expected=$(cut -d' ' -f1 <"${tmp}/${archive}.sha256")
        actual=$(checksum "${tmp}/${archive}")
        if [ -z "$actual" ]; then
            warn "no sha256 tool found; skipping checksum verification"
        elif [ "$expected" != "$actual" ]; then
            die "checksum mismatch for ${archive}" \
                "expected ${expected}" \
                "got      ${actual}" \
                "Do not use this download."
        else
            say "  checksum ok"
        fi
    else
        warn "no published checksum for ${archive}; skipping verification"
    fi

    tar -xzf "${tmp}/${archive}" -C "$tmp" ||
        die "could not unpack ${archive}"

    binary=$(find "$tmp" -type f -name ironwire -perm -u+x 2>/dev/null | head -n1)
    [ -n "$binary" ] || die "the archive did not contain an ironwire binary"

    mkdir -p "$INSTALL_DIR" || die "could not create ${INSTALL_DIR}"
    # Install via a temporary name and rename, so an interrupted install never
    # leaves a half-written binary where a working one used to be.
    install -m 755 "$binary" "${INSTALL_DIR}/.ironwire.new" 2>/dev/null ||
        { cp "$binary" "${INSTALL_DIR}/.ironwire.new" && chmod 755 "${INSTALL_DIR}/.ironwire.new"; } ||
        die "could not write to ${INSTALL_DIR}"
    mv -f "${INSTALL_DIR}/.ironwire.new" "${INSTALL_DIR}/ironwire" ||
        die "could not install to ${INSTALL_DIR}/ironwire"

    installed=$("${INSTALL_DIR}/ironwire" --version 2>/dev/null || echo "ironwire")
    say ""
    say "${GREEN}Installed${RESET} ${installed} to ${INSTALL_DIR}/ironwire"
    say ""

    report_path
    say "Then:"
    say "  ${BOLD}ironwire connect claude${RESET}    point Claude Code at it"
    say "  ${BOLD}ironwire serve${RESET}             start the daemon"
    say ""
    say "${DIM}IronWire binds 127.0.0.1 only and stores everything under ~/.ironwire${RESET}"
}

# Tell the user what to add to *their* shell's config, not a generic line they
# have to translate. The wrong rc file is the single most common reason a
# freshly installed CLI "does not exist".
report_path() {
    case ":${PATH}:" in
        *":${INSTALL_DIR}:"*)
            say "${INSTALL_DIR} is already on your PATH."
            say ""
            return ;;
    esac

    shell_name=$(basename "${SHELL:-sh}")
    case "$shell_name" in
        zsh)  rc="${ZDOTDIR:-$HOME}/.zshrc"; line="export PATH=\"${INSTALL_DIR}:\$PATH\"" ;;
        bash)
            if [ -f "${HOME}/.bash_profile" ]; then rc="${HOME}/.bash_profile"; else rc="${HOME}/.bashrc"; fi
            line="export PATH=\"${INSTALL_DIR}:\$PATH\"" ;;
        fish) rc="${HOME}/.config/fish/config.fish"; line="fish_add_path ${INSTALL_DIR}" ;;
        *)    rc="your shell's startup file"; line="export PATH=\"${INSTALL_DIR}:\$PATH\"" ;;
    esac

    say "Add ${INSTALL_DIR} to your PATH by adding this to ${BOLD}${rc}${RESET}:"
    say ""
    say "    ${line}"
    say ""
    say "Then restart your shell, or run it now to use ironwire immediately."
    say ""
}

main "$@"
