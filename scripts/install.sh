#!/bin/sh
# Install the Trace Commons contributor CLI.
#
#   curl -fsSL https://raw.githubusercontent.com/TraceCommons/trace-commons/main/scripts/install.sh -o install.sh
#   sh install.sh
#
# Reading it before running it is encouraged and is why the two-step form is
# documented first. The piped form works too.
#
# The internal staging CLI, for Linux servers on x86_64 or arm64:
#
#   sh install.sh --staging
#
# installs the newest staging pre-release as near-ai-staging (and
# trace-commons-contributor-staging), beside any released CLI. It talks only
# to the staging servers and NEAR AI staging and keeps its own state. Pass
# --version with a staging tag (staging-<date>-<commit>-<run>) to pin one.
#
# There is deliberately no prettier URL yet: a redirect from tracecommons.ai
# would be nicer to type and would add a second thing that can be wrong, so
# this points at the one copy that exists.
#
# WHAT THIS REFUSES TO DO
#
# It will not install a binary it cannot verify. On macOS that means a valid
# Developer ID signature naming the expected team; everywhere it means the
# published SHA-256 matching. There is no --force and no --skip-verify: this
# tool reads your coding transcripts, so an installer that can be talked into
# skipping the checks is worse than no installer. If verification fails, you get
# the reason and a non-zero exit, and nothing is placed on your PATH.
#
# The checksum alone is weak -- it travels from the same origin over the same
# connection as the binary, so it mostly catches corruption. The macOS signature
# is the stronger claim: it verifies against Apple's roots rather than against
# us, so it still means something if this file or the binary were served by
# somebody else.
set -eu

REPO="TraceCommons/trace-commons"
TEAM_ID="KXSWJN7WY8"
EXPECTED_AUTHORITY="Developer ID Application: Iqlusion Inc ($TEAM_ID)"
INSTALL_DIR="${TC_INSTALL_DIR:-$HOME/.local/bin}"
VERSION="${TC_VERSION:-latest}"
CHANNEL=production

say() { printf '%s\n' "$*"; }
die() { printf 'install failed: %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "$1 is required but not installed"; }

while [ $# -gt 0 ]; do
  case "$1" in
    --dir) INSTALL_DIR="${2:?--dir needs a path}"; shift 2 ;;
    --version) VERSION="${2:?--version needs a version, e.g. 0.1.0}"; shift 2 ;;
    --staging) CHANNEL=staging; shift ;;
    -h|--help) sed -n '2,36p' "$0"; exit 0 ;;
    *) die "unknown option: $1 (try --help)" ;;
  esac
done

need curl
need uname
need mkdir

# ---- which build do you need -------------------------------------------
os="$(uname -s)"
arch="$(uname -m)"
if [ "$CHANNEL" = staging ]; then
  # The staging macOS app ships as a DMG on the same pre-release; this
  # installer carries only the Linux CLI.
  case "$os/$arch" in
    Linux/x86_64)              target=x86_64-unknown-linux-gnu ;;
    Linux/aarch64|Linux/arm64) target=aarch64-unknown-linux-gnu ;;
    Darwin/*)
      die "the staging build for macOS is the Near.ai Staging app. Download the
.dmg from the newest staging pre-release:
  https://github.com/$REPO/releases" ;;
    *)
      die "no staging build for $os $arch. Staging builds are for Linux on x86_64
or arm64, and the macOS app." ;;
  esac
fi

case "$CHANNEL/$os/$arch" in
  staging/*) ;;
  */Darwin/arm64)  target=aarch64-apple-darwin ;;
  */Darwin/x86_64) target=x86_64-apple-darwin ;;
  */Linux/x86_64)  target=x86_64-unknown-linux-gnu ;;
  */Linux/aarch64|*/Linux/arm64)
    die "no published binary for Linux on arm64 yet. Build from source:
  git clone https://github.com/$REPO
  cd trace-commons-server
  cargo build --release --bin trace-commons-contributor" ;;
  *)
    die "unsupported platform: $os $arch. See https://docs.tracecommons.ai/cli/quickstart/" ;;
esac

if [ "$CHANNEL" = staging ]; then
  asset="trace-commons-contributor-staging-$target"
  command_name=near-ai-staging
  compat_name=trace-commons-contributor-staging
else
  asset="trace-commons-contributor-$target"
  command_name=near-ai
  compat_name=trace-commons-contributor
fi

# NOT /releases/latest/download. This repository publishes two independent tag
# streams -- contributor-v* for the CLI and app-v* for the desktop app -- so
# GitHub's "latest release" is whichever was cut most recently and may hold no
# CLI assets at all. Resolve the newest contributor-v* tag explicitly.
if [ "$CHANNEL" = staging ]; then
  # Staging builds are pre-releases tagged staging-<date>-<commit>-<run>.
  # The newest one that carries this platform's CLI is the first asset URL
  # naming it: the API lists releases newest first, and an older staging
  # pre-release may hold only the macOS app.
  if [ "$VERSION" = latest ]; then
    tag="$(curl -fsSL --proto '=https' --tlsv1.2 \
        "https://api.github.com/repos/$REPO/releases?per_page=50" 2>/dev/null \
      | grep -o "\"browser_download_url\":[[:space:]]*\"[^\"]*/download/staging-[^/\"]*/$asset\"" \
      | head -1 \
      | sed -e 's#.*/download/\(staging-[^/]*\)/.*#\1#')" || tag=""
    [ -n "$tag" ] || die "could not find a staging build for $target. GitHub's API
may be rate-limiting you, or no staging build has been published for this
platform yet. Pass a staging tag explicitly instead:
  sh install.sh --staging --version staging-<date>-<commit>-<run>"
  else
    case "$VERSION" in
      staging-*) tag="$VERSION" ;;
      *) die "--version with --staging takes a staging tag, e.g. staging-20261029-abc1234-12" ;;
    esac
  fi
elif [ "$VERSION" = latest ]; then
  tag="$(curl -fsSL --proto '=https' --tlsv1.2 \
      "https://api.github.com/repos/$REPO/releases?per_page=50" 2>/dev/null \
    | grep -o '"tag_name":[[:space:]]*"contributor-v[^"]*"' \
    | head -1 \
    | sed -e 's/.*"\(contributor-v[^"]*\)"/\1/')" || tag=""
  [ -n "$tag" ] || die "could not work out the latest CLI release. GitHub's API
may be rate-limiting you. Pass a version explicitly instead:
  sh install.sh --version 0.1.0"
else
  tag="contributor-v$VERSION"
fi
base="https://github.com/$REPO/releases/download/$tag"

say "channel  : $CHANNEL"
say "release  : $tag"
say "platform : $os $arch -> $target"
say "source   : $base/$asset"
say "target   : $INSTALL_DIR"

# ---- fetch into a scratch dir, never straight onto PATH ----------------
tmp="$(mktemp -d)"
# shellcheck disable=SC2064
trap "rm -rf '$tmp'" EXIT INT TERM

curl -fsSL --proto '=https' --tlsv1.2 "$base/$asset" -o "$tmp/$asset" \
  || die "could not download $base/$asset"
curl -fsSL --proto '=https' --tlsv1.2 "$base/$asset.sha256" -o "$tmp/$asset.sha256" \
  || die "could not download the checksum for $asset"

# ---- verify BEFORE anything is installed ------------------------------
say ""
say "verifying..."

if command -v shasum >/dev/null 2>&1; then
  actual="$(shasum -a 256 "$tmp/$asset" | awk '{print $1}')"
elif command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "$tmp/$asset" | awk '{print $1}')"
else
  die "neither shasum nor sha256sum is available, so the download cannot be
verified. Refusing to install unverified."
fi
expected="$(awk '{print $1}' "$tmp/$asset.sha256")"
[ -n "$expected" ] || die "the published checksum file was empty"
if [ "$actual" != "$expected" ]; then
  die "checksum mismatch for $asset
  published: $expected
  actual:    $actual
Nothing was installed. This means the file you received is not the file that
was published -- do not use it."
fi
say "  checksum ok ($expected)"

if [ "$os" = Darwin ]; then
  need codesign
  codesign --verify --strict "$tmp/$asset" 2>/dev/null \
    || die "the code signature on $asset is not valid. Nothing was installed."
  # Pin WHO signed it, not merely that something did. A valid signature from
  # an unexpected identity is exactly the case a naive check would pass.
  authority="$(codesign -dvvv "$tmp/$asset" 2>&1 \
    | awk -F'=' '/^Authority=/ {print $2; exit}')"
  if [ "$authority" != "$EXPECTED_AUTHORITY" ]; then
    die "unexpected signing identity on $asset
  expected: $EXPECTED_AUTHORITY
  found:    ${authority:-<none>}
Nothing was installed."
  fi
  say "  signed by $authority"
  say "  (notarized: macOS checks this itself on first run)"
else
  say "  note: the Linux build is not code-signed, so the checksum above is the"
  say "        only check available."
fi

# ---- install ----------------------------------------------------------
mkdir -p "$INSTALL_DIR" || die "could not create $INSTALL_DIR"
dest="$INSTALL_DIR/$compat_name"
# Replace rather than write in place: overwriting a running binary is how you
# get a half-written executable, and a stale file's mode would survive.
rm -f "$dest"
cp "$tmp/$asset" "$dest"
chmod 755 "$dest"
# Both names dispatch through the same implementation.
near_ai_dest="$INSTALL_DIR/$command_name"
cp "$dest" "$near_ai_dest.tmp"
chmod 755 "$near_ai_dest.tmp"
mv -f "$near_ai_dest.tmp" "$near_ai_dest"

say ""
say "installed: $near_ai_dest (compatibility command: $dest)"
"$dest" --version 2>/dev/null || true

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    say ""
    say "$INSTALL_DIR is not on your PATH. Add it:"
    say "  export PATH=\"\$PATH:$INSTALL_DIR\""
    ;;
esac

say ""
if [ "$CHANNEL" = staging ]; then
  say "next, on a server with no browser, one of:"
  say "  $command_name login --code <code>      (code from \`devices add\` on your signed-in computer)"
  say "  $command_name login --invite <url>     (a staging invite link)"
  say "  $command_name login --no-browser       (NEAR AI staging sign-in, Google)"
  say "then: $command_name daemon install && systemctl --user enable --now trace-commons-contributor-staging.service"
  say "docs: https://github.com/$REPO/blob/main/docs/operator/staging-tester-guide.md"
else
  say "next: near-ai login --invite <url>"
  say "docs: https://docs.tracecommons.ai/cli/quickstart/"
fi
