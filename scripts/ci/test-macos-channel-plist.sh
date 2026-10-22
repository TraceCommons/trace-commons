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
# - an unknown channel, or a channel file missing a key, renders nothing.
#
# Seconds, no build: bash only, plus plutil where it exists.
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
  has staging "$s" '<key>CFBundleURLSchemes</key><array><string>tracecommons-staging</string></array>'
  has staging "$s" '<key>CFBundleURLName</key><string>ai.tracecommons.shell.staging.invite</string>'
  has staging "$s" '<key>SUFeedURL</key><string>https://storage.googleapis.com/tc-staging-artifacts-20261021/staging/appcast.xml</string>'
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

if ((failures > 0)); then
  echo "$failures channel plist check(s) failed." >&2
  exit 1
fi
echo "the macOS Info.plist renders per channel"
