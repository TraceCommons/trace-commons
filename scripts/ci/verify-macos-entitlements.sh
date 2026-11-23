#!/usr/bin/env bash
# Assertions about a signed Near.ai.app that its own signature cannot make.
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
#
# The one exception is the native shell's staging build (staging design
# 4.2), ai.tracecommons.shell.staging: it has its own group, so that it never
# reads a production credential, and it is held to that group exactly.
set -euo pipefail

APP="${1:?usage: verify-macos-entitlements.sh <path to a Near.ai or Trace Commons .app>}"
PROFILE="$APP/Contents/embedded.provisionprofile"
ACCESS_GROUP="KXSWJN7WY8.ai.tracecommons.shell"
TEAM_ID="KXSWJN7WY8"

INFO="$APP/Contents/Info.plist"
test -f "$INFO" || { echo "FAIL: no Contents/Info.plist in $APP"; exit 1; }
BUNDLE_ID="$(plutil -extract CFBundleIdentifier raw -o - "$INFO" 2>/dev/null)" \
  || { echo "FAIL: Info.plist carries no CFBundleIdentifier"; exit 1; }
EXECUTABLE="$(plutil -extract CFBundleExecutable raw -o - "$INFO" 2>/dev/null)" \
  || { echo "FAIL: Info.plist carries no CFBundleExecutable"; exit 1; }
echo "bundle $BUNDLE_ID, executable $EXECUTABLE"
if [ "$BUNDLE_ID" = "ai.tracecommons.shell.staging" ]; then
  ACCESS_GROUP="$TEAM_ID.ai.tracecommons.shell.staging"
fi

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
# The team wildcard, or this exact group: a grant of the production group
# does not cover the staging one.
echo "$GRANTED" | grep -qE "<string>($TEAM_ID\.\*|${ACCESS_GROUP//./\\.})</string>" \
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

echo "--- the profile is not near expiry"
EXPIRES="$(printf '%s' "$PLIST" | plutil -extract ExpirationDate raw -o - - 2>/dev/null)" \
  || { echo "FAIL: profile carries no ExpirationDate"; exit 1; }
EXPIRES_EPOCH="$(date -j -f "%Y-%m-%dT%H:%M:%SZ" "$EXPIRES" +%s 2>/dev/null || date -d "$EXPIRES" +%s)"
DAYS_LEFT=$(( (EXPIRES_EPOCH - $(date +%s)) / 86400 ))
echo "profile expires $EXPIRES ($DAYS_LEFT days)"
test "$DAYS_LEFT" -gt 180 \
  || { echo "FAIL: under 180 days left; renew the profile or the certificate"; exit 1; }

echo "--- every architecture slice links only portable libraries"
BIN="$APP/Contents/MacOS/$EXECUTABLE"
test -x "$BIN" || { echo "FAIL: $BIN is missing or not executable"; exit 1; }
check_portable_macho() {
  local target="$1"
  local archs arch deps dep
  archs="$(lipo -archs "$target" 2>/dev/null)" \
    || { echo "FAIL: could not read architectures of $target"; exit 1; }
  for arch in $archs; do
    deps="$(otool -arch "$arch" -L "$target" | awk 'NR > 1 && /^[[:space:]]+/ { sub(/^[[:space:]]+/, ""); sub(/ \(compatibility version .*/, ""); print }')"
    while IFS= read -r dep; do
      [ -n "$dep" ] || continue
      case "$dep" in
        /System/Library/*|/usr/lib/*|@rpath/*|@executable_path/*|@loader_path/*) ;;
        *)
          echo "FAIL: $target ($arch) links non-portable library: $dep"
          exit 1
          ;;
      esac
    done <<EOF
$deps
EOF
  done
}
check_portable_macho "$BIN"
if [ -d "$APP/Contents/Frameworks" ]; then
  while IFS= read -r -d '' dylib; do
    check_portable_macho "$dylib"
  done < <(find "$APP/Contents/Frameworks" -maxdepth 2 -type f -name '*.dylib' -print0)
fi

# PR CI runs everything above against an ad-hoc-signed fixture bundle (see
# test-verify-macos-entitlements.sh). An ad-hoc signature can never be granted
# the access group, so the launch below means something only on a release
# build; the fixture stops here.
if [ "${TC_VERIFY_STATIC_ONLY:-}" = 1 ]; then
  echo "PASS: static checks (launch skipped: TC_VERIFY_STATIC_ONLY=1)"
  exit 0
fi

probe_credential_store() {
  local label="$1"
  shift
  local out pid alive
  out="$(mktemp)"
  TRACE_COMMONS_CREDENTIAL_STORE_CHECK_OUT="$out" "$@" &
  pid=$!
  for _ in $(seq 1 30); do
    [ -s "$out" ] && break
    sleep 1
  done
  # "Produced nothing" has two causes and they are not the same failure. A
  # kernel kill at exec or a dyld missing-library abort leaves no process.
  # A first notarized launch waiting on Gatekeeper's online ticket check leaves
  # one running, and reporting that as a launch failure would block a healthy
  # ship. The timeout is not tuned; distinguishing the two is what makes it safe.
  if kill -0 "$pid" 2>/dev/null; then alive=yes; else alive=no; fi
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  if [ ! -s "$out" ]; then
    rm -f "$out"
    if [ "$alive" = yes ]; then
      echo "FAIL: the signed app ($label) was still running after 30s and wrote nothing."
      echo "      It launched, so this is not the entitlement kill. Suspect a"
      echo "      slow first-launch Gatekeeper check; re-run before raising the"
      echo "      timeout, and do not treat this as a signature failure."
    else
      echo "FAIL: the signed app ($label) exited without writing anything; it may not"
      echo "      have launched at all. That is what a kernel kill for an"
      echo "      entitlement the profile does not grant, or a dyld missing-library"
      echo "      abort, looks like from here."
    fi
    exit 1
  fi
  if ! grep -q "^reachable" "$out"; then
    echo "FAIL: ($label) $(cat "$out")"
    rm -f "$out"
    exit 1
  fi
  rm -f "$out"
  echo "PASS: signed app ($label) reached the data-protection keychain"
}

echo "--- the signed app can actually reach its store"
probe_credential_store "native" "$BIN"

EXEC_ARCHS="$(lipo -archs "$BIN" 2>/dev/null || true)"
if [ "$(uname -m)" = "arm64" ] && [[ " $EXEC_ARCHS " == *" x86_64 "* ]]; then
  if arch -x86_64 /usr/bin/true 2>/dev/null; then
    echo "--- the signed app's x86_64 slice can actually reach its store (Rosetta)"
    probe_credential_store "x86_64" arch -x86_64 "$BIN"
  else
    echo "note: Rosetta 2 is not installed on this runner; skipping x86_64 launch probe (static otool check passed)"
  fi
fi
