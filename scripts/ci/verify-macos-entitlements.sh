#!/usr/bin/env bash
# Assertions about a signed TraceCommons.app that its own signature cannot make.
#
# The failure this guards against is total: an app carrying the keychain
# entitlement without a profile that grants it is killed at exec. Every check
# that existed before this one stayed green through that.
set -euo pipefail

APP="${1:?usage: verify-macos-entitlements.sh <path to TraceCommons.app>}"
PROFILE="$APP/Contents/embedded.provisionprofile"

echo "--- the profile is embedded"
test -f "$PROFILE" || { echo "FAIL: no embedded.provisionprofile in the bundle"; exit 1; }

echo "--- the entitlement is present"
ENTS="$(codesign -d --entitlements - --xml "$APP" 2>/dev/null | plutil -convert xml1 -o - -)"
echo "$ENTS" | grep -q "KXSWJN7WY8.ai.tracecommons.shell" \
  || { echo "FAIL: keychain access group absent from the signature"; exit 1; }

echo "--- the profile grants what the entitlement requests"
GRANTED="$(security cms -D -i "$PROFILE" | plutil -extract Entitlements.keychain-access-groups xml1 -o - -)"
echo "$GRANTED" | grep -qE "KXSWJN7WY8\.(\*|ai\.tracecommons\.shell)" \
  || { echo "FAIL: profile does not grant the requested access group"; exit 1; }

echo "--- the profile is not near expiry"
EXPIRES="$(security cms -D -i "$PROFILE" | plutil -extract ExpirationDate raw -o - -)"
EXPIRES_EPOCH="$(date -j -f "%Y-%m-%dT%H:%M:%SZ" "$EXPIRES" +%s 2>/dev/null || date -d "$EXPIRES" +%s)"
DAYS_LEFT=$(( (EXPIRES_EPOCH - $(date +%s)) / 86400 ))
echo "profile expires $EXPIRES ($DAYS_LEFT days)"
test "$DAYS_LEFT" -gt 180 \
  || { echo "FAIL: under 180 days left; renew the profile or the certificate"; exit 1; }

echo "--- the signed app can actually reach its store"
OUT="$(mktemp)"
TRACE_COMMONS_CREDENTIAL_STORE_CHECK_OUT="$OUT" "$APP/Contents/MacOS/TraceCommonsApp" &
PID=$!
for _ in $(seq 1 30); do
  [ -s "$OUT" ] && break
  sleep 1
done
kill "$PID" 2>/dev/null || true
# An empty file means the app never got far enough to write one, which is what
# a kernel kill at exec looks like from here. That is the case this exists for.
test -s "$OUT" || { echo "FAIL: the signed app wrote nothing; it may not have launched"; exit 1; }
grep -q "^reachable" "$OUT" || { echo "FAIL: $(cat "$OUT")"; exit 1; }
echo "PASS: signed app reached the data-protection keychain"
