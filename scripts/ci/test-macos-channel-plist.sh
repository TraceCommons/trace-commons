#!/usr/bin/env bash
# The macOS bundle's Info.plist under each channel (staging design 4.1).
#
# macos/scripts/info-plist.sh reads its identity from macos/channels/
# $TC_CHANNEL.env. This holds three things about it:
#
# - a production plist, with no channel chosen or with `production`, is
#   byte-identical to the one rendered before the channel files existed
#   (the fixtures under scripts/ci/tests/fixtures were taken from that
#   script, unchanged);
# - a staging plist carries the staging bundle id, name, URL scheme, feed
#   and channel, and nothing of production's identity or feed, and it never
#   takes the production Sparkle key;
# - an unknown channel, or a channel file missing, repeating or emptying a
#   key, or naming an unsafe bundle name, renders nothing;
# - the pre-signing bundle check refuses a fake bundle whose plist, CLI and
#   FFI library do not all report the chosen channel and scheme.
#
# Seconds, no cargo build: bash, python3 and a C compiler for one stub
# library, plus plutil where it exists.
set -euo pipefail

repo="$(cd "$(dirname "$0")/../.." && pwd)"
script="$repo/macos/scripts/info-plist.sh"
fixtures="$repo/scripts/ci/tests/fixtures"
failures=0
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

fail() {
  echo "FAIL: $*" >&2
  failures=$((failures + 1))
}

pass() {
  echo "ok: $*"
}

# Every key a test might leak in from the caller's shell, cleared.
render() {
  env -u TC_CHANNEL -u TC_SPARKLE_PUBLIC_ED_KEY -u TC_STAGING_SPARKLE_PUBLIC_ED_KEY \
    "$@" bash "$script" 1.2.3 45
}

lint() {
  local label="$1" file="$2"
  if command -v plutil >/dev/null 2>&1; then
    if plutil -lint "$file" >/dev/null; then
      pass "$label: plutil accepts it"
    else
      fail "$label: plutil rejects it"
    fi
  fi
  # plutil tolerates a `--` inside an XML comment; a strict parser does not.
  # The release scripts read TCChannel with Python's plistlib, so a plist it
  # cannot parse fails the channel check at signing time on every channel.
  if python3 -I -c '
import plistlib, sys
with open(sys.argv[1], "rb") as f:
    plistlib.load(f)["TCChannel"]
' "$file" 2>/dev/null; then
    pass "$label: a strict XML parser reads its TCChannel"
  else
    fail "$label: a strict XML parser cannot read its TCChannel"
  fi
}

# --- production, byte for byte ------------------------------------------------

if render >"$scratch/default.plist" 2>"$scratch/err" \
  && cmp -s "$scratch/default.plist" "$fixtures/info-plist-production-no-key.plist"; then
  pass "no channel chosen renders today's production plist"
else
  fail "no channel chosen does not render today's production plist"
  diff "$fixtures/info-plist-production-no-key.plist" "$scratch/default.plist" >&2 || true
  cat "$scratch/err" >&2
fi

if render TC_CHANNEL=production >"$scratch/prod.plist" 2>"$scratch/err" \
  && cmp -s "$scratch/prod.plist" "$fixtures/info-plist-production-no-key.plist"; then
  pass "TC_CHANNEL=production renders today's production plist"
else
  fail "TC_CHANNEL=production does not render today's production plist"
  diff "$fixtures/info-plist-production-no-key.plist" "$scratch/prod.plist" >&2 || true
  cat "$scratch/err" >&2
fi

if render TC_CHANNEL=production TC_SPARKLE_PUBLIC_ED_KEY=dGVzdC1rZXktbm90LXJlYWw= \
  >"$scratch/prod-key.plist" 2>"$scratch/err" \
  && cmp -s "$scratch/prod-key.plist" "$fixtures/info-plist-production-with-key.plist"; then
  pass "production with a Sparkle key renders today's released plist"
else
  fail "production with a Sparkle key does not render today's released plist"
  diff "$fixtures/info-plist-production-with-key.plist" "$scratch/prod-key.plist" >&2 || true
  cat "$scratch/err" >&2
fi
lint "production" "$scratch/prod-key.plist"

# --- staging ------------------------------------------------------------------

has() {
  local label="$1" file="$2" needle="$3"
  if grep -q -F -- "$needle" "$file"; then
    pass "$label: has $needle"
  else
    fail "$label: lacks $needle"
  fi
}

lacks() {
  local label="$1" file="$2" needle="$3"
  if grep -q -F -- "$needle" "$file"; then
    fail "$label: carries $needle"
  else
    pass "$label: no $needle"
  fi
}

if render TC_CHANNEL=staging TC_STAGING_SPARKLE_PUBLIC_ED_KEY=c3RhZ2luZy1rZXktbm90LXJlYWw= \
  >"$scratch/staging.plist" 2>"$scratch/err"; then
  s="$scratch/staging.plist"
  has staging "$s" '<key>CFBundleIdentifier</key><string>ai.tracecommons.shell.staging</string>'
  has staging "$s" '<key>CFBundleName</key><string>Near.ai Staging</string>'
  has staging "$s" '<key>CFBundleDisplayName</key><string>Near.ai Staging</string>'
  has staging "$s" '<key>TCChannel</key><string>staging</string>'
  has staging "$s" '<key>TCPrivateAICampaignURL</key><string>https://storage.googleapis.com/tc-staging-updates-20261022/staging/private-ai-campaign.json</string>'
  has staging "$s" '<key>CFBundleURLSchemes</key><array><string>tracecommons-staging</string></array>'
  has staging "$s" '<key>CFBundleURLName</key><string>ai.tracecommons.shell.staging.invite</string>'
  has staging "$s" '<key>SUFeedURL</key><string>https://storage.googleapis.com/tc-staging-updates-20261022/staging/appcast.xml</string>'
  has staging "$s" '<key>SUPublicEDKey</key><string>c3RhZ2luZy1rZXktbm90LXJlYWw=</string>'
  lacks staging "$s" '<string>ai.tracecommons.shell</string>'
  lacks staging "$s" '<string>tracecommons</string>'
  lacks staging "$s" '<string>Near.ai</string>'
  lacks staging "$s" '<string>production</string>'
  lacks staging "$s" 'tracecommons-flatpak'
  lint "staging" "$s"
else
  fail "TC_CHANNEL=staging did not render"
  cat "$scratch/err" >&2
fi

# The production key is not the staging key: a staging bundle given only the
# production variable gets no feed and no key, never the production pair.
if render TC_CHANNEL=staging TC_SPARKLE_PUBLIC_ED_KEY=dGVzdC1rZXktbm90LXJlYWw= \
  >"$scratch/staging-prodkey.plist" 2>"$scratch/err"; then
  s="$scratch/staging-prodkey.plist"
  lacks "staging given the production key" "$s" 'SUFeedURL'
  lacks "staging given the production key" "$s" 'SUPublicEDKey'
  lacks "staging given the production key" "$s" 'dGVzdC1rZXktbm90LXJlYWw='
  has "staging given the production key" "$s" '<key>SUEnableAutomaticChecks</key><false/>'
else
  fail "TC_CHANNEL=staging with only the production key did not render"
  cat "$scratch/err" >&2
fi

# --- refusals -----------------------------------------------------------------

for channel in beta Production ../channels/production staging/ ""; do
  if [[ -z "$channel" ]]; then
    # Empty means unset-like for ${TC_CHANNEL:-production}; covered above.
    continue
  fi
  if render TC_CHANNEL="$channel" >"$scratch/bad.plist" 2>/dev/null; then
    fail "TC_CHANNEL='$channel' rendered a plist"
  elif [[ -s "$scratch/bad.plist" ]]; then
    fail "TC_CHANNEL='$channel' failed but still printed a partial plist"
  else
    pass "TC_CHANNEL='$channel' is refused"
  fi
done

# A channel file missing a key renders nothing: a copy of the scripts and
# channel files with one key dropped.
mkdir -p "$scratch/tree/macos/scripts" "$scratch/tree/macos/channels"
cp "$repo/macos/scripts/info-plist.sh" "$repo/macos/scripts/channel.sh" "$scratch/tree/macos/scripts/"
cp "$repo/macos/channels/"*.env "$scratch/tree/macos/channels/"
for key in TC_BUNDLE_ID TC_URL_SCHEME TC_SPARKLE_FEED_URL TC_SPARKLE_KEY_VAR; do
  grep -v "^$key=" "$repo/macos/channels/staging.env" >"$scratch/tree/macos/channels/staging.env"
  if env -u TC_SPARKLE_PUBLIC_ED_KEY TC_CHANNEL=staging \
    bash "$scratch/tree/macos/scripts/info-plist.sh" 1.2.3 45 >"$scratch/bad.plist" 2>/dev/null; then
    fail "a staging file without $key rendered a plist"
  elif [[ -s "$scratch/bad.plist" ]]; then
    fail "a staging file without $key failed but still printed a partial plist"
  else
    pass "a staging file without $key is refused"
  fi
done

# An unknown key is refused too, so a misspelt key cannot silently leave the
# real one at another file's value.
cp "$repo/macos/channels/staging.env" "$scratch/tree/macos/channels/staging.env"
echo "TC_BUNDLE_IDENTIFIER=ai.example" >>"$scratch/tree/macos/channels/staging.env"
if env -u TC_SPARKLE_PUBLIC_ED_KEY TC_CHANNEL=staging \
  bash "$scratch/tree/macos/scripts/info-plist.sh" 1.2.3 45 >/dev/null 2>&1; then
  fail "a staging file with an unknown key rendered a plist"
else
  pass "a staging file with an unknown key is refused"
fi

# A key set twice, or left empty, is refused: the second would otherwise
# silently win, and an empty bundle id or scheme is no identity at all.
cp "$repo/macos/channels/staging.env" "$scratch/tree/macos/channels/staging.env"
echo "TC_BUNDLE_ID=ai.example" >>"$scratch/tree/macos/channels/staging.env"
if env -u TC_SPARKLE_PUBLIC_ED_KEY TC_CHANNEL=staging \
  bash "$scratch/tree/macos/scripts/info-plist.sh" 1.2.3 45 >/dev/null 2>&1; then
  fail "a staging file setting TC_BUNDLE_ID twice rendered a plist"
else
  pass "a staging file setting TC_BUNDLE_ID twice is refused"
fi
sed 's/^TC_BUNDLE_ID=.*/TC_BUNDLE_ID=/' "$repo/macos/channels/staging.env" \
  >"$scratch/tree/macos/channels/staging.env"
if env -u TC_SPARKLE_PUBLIC_ED_KEY TC_CHANNEL=staging \
  bash "$scratch/tree/macos/scripts/info-plist.sh" 1.2.3 45 >/dev/null 2>&1; then
  fail "a staging file with an empty TC_BUNDLE_ID rendered a plist"
else
  pass "a staging file with an empty TC_BUNDLE_ID is refused"
fi

# The bundle name becomes a path the build script deletes and recreates, so
# it may not carry a slash or start with a dot.
for name in "../Near.ai" "Near/ai" ".Near.ai" "Near.ai;x"; do
  sed "s|^TC_BUNDLE_NAME=.*|TC_BUNDLE_NAME=$name|" "$repo/macos/channels/staging.env" \
    >"$scratch/tree/macos/channels/staging.env"
  if env -u TC_SPARKLE_PUBLIC_ED_KEY TC_CHANNEL=staging \
    bash "$scratch/tree/macos/scripts/info-plist.sh" 1.2.3 45 >/dev/null 2>&1; then
    fail "a staging bundle name '$name' rendered a plist"
  else
    pass "a staging bundle name '$name' is refused"
  fi
done
cp "$repo/macos/channels/staging.env" "$scratch/tree/macos/channels/staging.env"

# --- the bundle channel check (spec 4.3) -----------------------------------
#
# assert_bundle_channel runs only inside a real bundle build, which CI makes
# only on a release tag. Here it runs against a fake .app: a plist, a stub
# near-ai printing a --version line, and a stub library exporting the two
# symbols the check calls. The chosen channel, the plist, the CLI and the
# library must all agree, and the library's URL scheme must be the channel's.
if command -v cc >/dev/null 2>&1 && command -v python3 >/dev/null 2>&1; then
  fake_app() {
    local app="$1" plist_channel="$2" cli_channel="$3" lib_json="$4"
    rm -rf "$app"
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Frameworks"
    python3 -I -c 'import plistlib, sys
with open(sys.argv[1], "wb") as f:
    plistlib.dump({"TCChannel": sys.argv[2]}, f)' "$app/Contents/Info.plist" "$plist_channel"
    printf '#!/bin/sh\necho "near-ai 1.2.3 (0000000) channel %s"\n' "$cli_channel" \
      >"$app/Contents/MacOS/near-ai"
    chmod +x "$app/Contents/MacOS/near-ai"
    if [[ -n "$lib_json" ]]; then
      python3 -I -c 'import json, sys
body = json.dumps(sys.argv[2])
open(sys.argv[1], "w").write(
    "#include <stdlib.h>\n#include <string.h>\n"
    "char *tc_build_channel_json(void) { return strdup(%s); }\n"
    "void tc_string_free(char *s) { free(s); }\n" % body)' "$scratch/stub.c" "$lib_json"
      cc -shared -fPIC -o "$app/Contents/Frameworks/libtrace_commons_contributor_ffi.dylib" \
        "$scratch/stub.c"
    fi
  }
  bundle_check() {
    env -u TC_SPARKLE_PUBLIC_ED_KEY TC_CHANNEL=staging bash -c '
      source "$1/macos/scripts/channel.sh"
      load_channel && assert_bundle_channel "$2"' _ "$repo" "$1" >"$scratch/bundle.out" 2>&1
  }
  agree='{"channel":"staging","url_scheme":"tracecommons-staging"}'
  app="$scratch/Fake Staging.app"

  fake_app "$app" staging staging "$agree"
  if bundle_check "$app"; then
    pass "a bundle whose plist, CLI and library all say staging is accepted"
  else
    fail "a bundle whose plist, CLI and library all say staging was refused"
    cat "$scratch/bundle.out" >&2
  fi
  refused() {
    local label="$1"
    if bundle_check "$app"; then
      fail "$label was accepted"
    else
      pass "$label is refused"
    fi
  }
  fake_app "$app" staging production "$agree"
  refused "a staging plist around a production CLI"
  fake_app "$app" production staging "$agree"
  refused "a production plist around a staging CLI"
  fake_app "$app" staging staging '{"channel":"production","url_scheme":"tracecommons-staging"}'
  refused "a staging plist and CLI around a production library"
  fake_app "$app" staging staging '{"channel":"staging","url_scheme":"tracecommons"}'
  refused "a staging library reporting production's URL scheme"
  fake_app "$app" staging staging ""
  refused "a bundle with no library"
else
  fail "cc and python3 are needed to check the bundle channel against a fake bundle"
fi

if ((failures > 0)); then
  echo "$failures channel plist check(s) failed." >&2
  exit 1
fi
echo "the macOS Info.plist renders per channel"
