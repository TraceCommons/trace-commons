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
# Extract the array, do not grep the whole document. The same string is also
# the com.apple.application-identifier value, so a bare grep stayed green with
# keychain-access-groups deleted outright -- only a launch caught that.
ENTS="$(codesign -d --entitlements - --xml "$APP" 2>/dev/null | plutil -convert xml1 -o - -)" \
  || { echo "FAIL: could not read the signed entitlements; is the bundle signed?"; exit 1; }
GROUPS="$(printf '%s' "$ENTS" | plutil -extract keychain-access-groups xml1 -o - - 2>/dev/null)" \
  || { echo "FAIL: the signature carries no keychain-access-groups array"; exit 1; }
printf '%s' "$GROUPS" | grep -q "<string>KXSWJN7WY8.ai.tracecommons.shell</string>" \
  || { echo "FAIL: keychain access group absent from keychain-access-groups"; exit 1; }

echo "--- the profile grants what the entitlement requests"
PLIST="$(security cms -D -i "$PROFILE" 2>/dev/null)" \
  || { echo "FAIL: embedded.provisionprofile is not readable CMS"; exit 1; }
GRANTED="$(printf '%s' "$PLIST" | plutil -extract Entitlements.keychain-access-groups xml1 -o - - 2>/dev/null)" \
  || { echo "FAIL: profile carries no Entitlements.keychain-access-groups"; exit 1; }
echo "$GRANTED" | grep -qE "KXSWJN7WY8\.(\*|ai\.tracecommons\.shell)" \
  || { echo "FAIL: profile does not grant the requested access group"; exit 1; }

echo "--- the profile is not near expiry"
EXPIRES="$(printf '%s' "$PLIST" | plutil -extract ExpirationDate raw -o - - 2>/dev/null)" \
  || { echo "FAIL: profile carries no ExpirationDate"; exit 1; }
EXPIRES_EPOCH="$(date -j -f "%Y-%m-%dT%H:%M:%SZ" "$EXPIRES" +%s 2>/dev/null || date -d "$EXPIRES" +%s)"
DAYS_LEFT=$(( (EXPIRES_EPOCH - $(date +%s)) / 86400 ))
echo "profile expires $EXPIRES ($DAYS_LEFT days)"
test "$DAYS_LEFT" -gt 180 \
  || { echo "FAIL: under 180 days left; renew the profile or the certificate"; exit 1; }

echo "--- the signed app can actually reach its store"
BIN="$APP/Contents/MacOS/TraceCommonsApp"
# A backgrounded job's failure does not trip `set -e`, so a wrong executable
# name produces output byte-identical to a kernel kill at exec. Name it here.
test -x "$BIN" || { echo "FAIL: $BIN is missing or not executable"; exit 1; }
OUT="$(mktemp)"
TRACE_COMMONS_CREDENTIAL_STORE_CHECK_OUT="$OUT" "$BIN" &
PID=$!
for _ in $(seq 1 30); do
  [ -s "$OUT" ] && break
  sleep 1
done
# "Produced nothing" has two causes and they are not the same failure. A
# kernel kill at exec -- the case this check exists for -- leaves no process.
# A first notarized launch waiting on Gatekeeper's online ticket check leaves
# one running, and reporting that as a launch failure would block a healthy
# ship. The timeout is not tuned; distinguishing the two is what makes it safe.
if kill -0 "$PID" 2>/dev/null; then ALIVE=yes; else ALIVE=no; fi
kill "$PID" 2>/dev/null || true
if [ ! -s "$OUT" ]; then
  if [ "$ALIVE" = yes ]; then
    echo "FAIL: the signed app was still running after 30s and wrote nothing."
    echo "      It launched, so this is not the entitlement kill. Suspect a"
    echo "      slow first-launch Gatekeeper check; re-run before raising the"
    echo "      timeout, and do not treat this as a signature failure."
  else
    echo "FAIL: the signed app exited without writing anything; it may not"
    echo "      have launched at all. That is what a kernel kill for an"
    echo "      entitlement the profile does not grant looks like from here."
  fi
  exit 1
fi
grep -q "^reachable" "$OUT" || { echo "FAIL: $(cat "$OUT")"; exit 1; }
echo "PASS: signed app reached the data-protection keychain"
