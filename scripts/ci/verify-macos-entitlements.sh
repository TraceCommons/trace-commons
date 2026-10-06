#!/usr/bin/env bash
# Assertions about a signed TraceCommons.app that its own signature cannot make.
#
# The failure this guards against is total: an app carrying the keychain
# entitlement without a profile that grants it is killed at exec. Every check
# that existed before this one stayed green through that.
#
# It serves both macOS apps: the native shell (ai.tracecommons.shell) and the
# Tauri desktop app (ai.tracecommons.desktop). Each has its own App ID and
# profile, but both must request the same keychain access group, because the
# contributor crate's ACCESS_GROUP is one constant and a credential written
# by one app has to be readable by the other. So the group below is fixed,
# and everything app-specific -- the bundle id, the executable name -- is
# read from the bundle's own Info.plist.
set -euo pipefail

APP="${1:?usage: verify-macos-entitlements.sh <path to a TraceCommons .app>}"
PROFILE="$APP/Contents/embedded.provisionprofile"
ACCESS_GROUP="KXSWJN7WY8.ai.tracecommons.shell"
TEAM_ID="KXSWJN7WY8"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
case "${TC_REQUIRE_NATIVE_PASSKEYS:-0}" in
  0|1) ;;
  *) echo "FAIL: invalid TC_REQUIRE_NATIVE_PASSKEYS (expected 0 or 1)"; exit 1 ;;
esac

INFO="$APP/Contents/Info.plist"
test -f "$INFO" || { echo "FAIL: no Contents/Info.plist in $APP"; exit 1; }
BUNDLE_ID="$(plutil -extract CFBundleIdentifier raw -o - "$INFO" 2>/dev/null)" \
  || { echo "FAIL: Info.plist carries no CFBundleIdentifier"; exit 1; }
EXECUTABLE="$(plutil -extract CFBundleExecutable raw -o - "$INFO" 2>/dev/null)" \
  || { echo "FAIL: Info.plist carries no CFBundleExecutable"; exit 1; }
echo "bundle $BUNDLE_ID, executable $EXECUTABLE"

echo "--- the profile is embedded"
test -f "$PROFILE" || { echo "FAIL: no embedded.provisionprofile in the bundle"; exit 1; }

echo "--- the entitlement is present"
# Extract the array, do not grep the whole document. The same string is also
# the com.apple.application-identifier value, so a bare grep stayed green with
# keychain-access-groups deleted outright -- only a launch caught that.
#
# Not `GROUPS`: that is bash's own array of the user's group ids. bash 3.2
# exits 1 silently on the assignment, and newer bash ignores it, so the grep
# below saw the primary gid and failed every correctly signed bundle.
ENTS="$(codesign -d --entitlements - --xml "$APP" 2>/dev/null | plutil -convert xml1 -o - -)" \
  || { echo "FAIL: could not read the signed entitlements; is the bundle signed?"; exit 1; }
SIGNED_GROUPS="$(printf '%s' "$ENTS" | plutil -extract keychain-access-groups xml1 -o - - 2>/dev/null)" \
  || { echo "FAIL: the signature carries no keychain-access-groups array"; exit 1; }
printf '%s' "$SIGNED_GROUPS" | grep -qF "<string>$ACCESS_GROUP</string>" \
  || { echo "FAIL: keychain access group absent from keychain-access-groups"; exit 1; }

echo "--- the profile grants what the entitlement requests"
PLIST="$(security cms -D -i "$PROFILE" 2>/dev/null)" \
  || { echo "FAIL: embedded.provisionprofile is not readable CMS"; exit 1; }
GRANTED="$(printf '%s' "$PLIST" | plutil -extract Entitlements.keychain-access-groups xml1 -o - - 2>/dev/null)" \
  || { echo "FAIL: profile carries no Entitlements.keychain-access-groups"; exit 1; }
echo "$GRANTED" | grep -qE "KXSWJN7WY8\.(\*|ai\.tracecommons\.shell)" \
  || { echo "FAIL: profile does not grant the requested access group"; exit 1; }

echo "--- the profile is this app's profile"
# A profile names exactly one App ID. The signed application-identifier has
# to be that App ID, and it has to be this bundle's: embed the native shell's
# profile in the desktop app (or the reverse) and every check above still
# passes, because both profiles grant $TEAM_ID.* -- and the kernel kills the
# app at exec. This is the static half of what the launch check proves.
# The dots in the key are escaped: plutil splits keypaths on unescaped dots.
EXPECTED_APP_ID="$TEAM_ID.$BUNDLE_ID"
SIGNED_APP_ID="$(printf '%s' "$ENTS" | plutil -extract 'com\.apple\.application-identifier' raw -o - - 2>/dev/null)" \
  || { echo "FAIL: the signature carries no com.apple.application-identifier"; exit 1; }
PROFILE_APP_ID="$(printf '%s' "$PLIST" | plutil -extract 'Entitlements.com\.apple\.application-identifier' raw -o - - 2>/dev/null)" \
  || { echo "FAIL: profile carries no Entitlements.com.apple.application-identifier"; exit 1; }
test "$SIGNED_APP_ID" = "$EXPECTED_APP_ID" \
  || { echo "FAIL: signed application-identifier $SIGNED_APP_ID is not $EXPECTED_APP_ID"; exit 1; }
test "$PROFILE_APP_ID" = "$EXPECTED_APP_ID" \
  || { echo "FAIL: embedded profile is for $PROFILE_APP_ID, not $EXPECTED_APP_ID"; exit 1; }

echo "--- native Associated Domains qualification"
# Pure metadata fixtures cover missing/wrong domains and grants separately.
# This path reads the ACTUAL signature and embedded CMS, never synthetic CMS.
NATIVE_METADATA="$(mktemp -d)"
trap 'rm -rf "$NATIVE_METADATA"' EXIT
printf '%s' "$ENTS" > "$NATIVE_METADATA/entitlements.plist"
printf '%s' "$PLIST" > "$NATIVE_METADATA/profile.plist"
PASSKEY_ARGS=(--bundle-id "$BUNDLE_ID")
if [ "${TC_REQUIRE_NATIVE_PASSKEYS:-0}" = 1 ]; then
  PASSKEY_ARGS+=(--require-passkeys)
fi
python3 "$ROOT/scripts/ci/native-passkey-entitlements.py" \
  --entitlements "$NATIVE_METADATA/entitlements.plist" \
  --profile "$NATIVE_METADATA/profile.plist" \
  "${PASSKEY_ARGS[@]}"

echo "--- the profile is not near expiry"
EXPIRES="$(printf '%s' "$PLIST" | plutil -extract ExpirationDate raw -o - - 2>/dev/null)" \
  || { echo "FAIL: profile carries no ExpirationDate"; exit 1; }
EXPIRES_EPOCH="$(date -j -f "%Y-%m-%dT%H:%M:%SZ" "$EXPIRES" +%s 2>/dev/null || date -d "$EXPIRES" +%s)"
DAYS_LEFT=$(( (EXPIRES_EPOCH - $(date +%s)) / 86400 ))
echo "profile expires $EXPIRES ($DAYS_LEFT days)"
test "$DAYS_LEFT" -gt 180 \
  || { echo "FAIL: under 180 days left; renew the profile or the certificate"; exit 1; }

# PR CI runs everything above against an ad-hoc-signed fixture bundle (see
# test-verify-macos-entitlements.sh). An ad-hoc signature can never be granted
# the access group, so the launch below means something only on a release
# build; the fixture stops here.
if [ "${TC_VERIFY_STATIC_ONLY:-}" = 1 ]; then
  echo "PASS: static checks (launch skipped: TC_VERIFY_STATIC_ONLY=1)"
  exit 0
fi

echo "--- the signed app can actually reach its store"
BIN="$APP/Contents/MacOS/$EXECUTABLE"
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
