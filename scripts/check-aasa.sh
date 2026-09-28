#!/usr/bin/env bash
# Validates the apple-app-site-association file as Apple's CDN fetcher would see
# it: HTTP 200 with no redirect, Content-Type application/json, and a body of
# exactly {"webcredentials":{"apps":["<TEAMID>.<bundle id>"]}}.
#
# Usage:
#   scripts/check-aasa.sh <base-url> [expected-app-id]
#
# <base-url> is the site root, e.g. https://tracecommons.ai (post-deploy smoke)
# or http://127.0.0.1:8788. With expected-app-id (or TC_EXPECTED_AASA_APP_ID)
# the single apps entry must equal it; otherwise it must match
# ^[A-Z0-9]{10}\.ai\.tracecommons\.shell$ (override the bundle id with
# TC_MACOS_BUNDLE_ID). Read-only; it never follows redirects, on purpose.
set -euo pipefail

base="${1:-}"
expected="${2:-${TC_EXPECTED_AASA_APP_ID:-}}"
bundle="${TC_MACOS_BUNDLE_ID:-ai.tracecommons.shell}"

if [ -z "$base" ]; then
  echo "usage: $0 <base-url> [expected-app-id]" >&2
  exit 2
fi
for tool in curl jq; do
  command -v "$tool" >/dev/null || { echo "check-aasa: $tool is required" >&2; exit 2; }
done

url="${base%/}/.well-known/apple-app-site-association"
hdr="$(mktemp)"
body="$(mktemp)"
trap 'rm -f "$hdr" "$body"' EXIT

# No -L: a redirect is a failure for AASA, not something to follow.
status="$(curl -sS --max-time 20 -o "$body" -D "$hdr" -w '%{http_code}' "$url")"

fail() { echo "check-aasa: FAIL $url: $1" >&2; exit 1; }

[ "$status" = "200" ] || fail "status $status, want 200 with no redirect"
if grep -qi '^location:' "$hdr"; then
  fail "response carries a Location header"
fi
ctype="$(grep -i '^content-type:' "$hdr" | tail -n1 | tr -d '\r' | cut -d: -f2- | tr 'A-Z' 'a-z' | sed 's/^ *//; s/ *;.*$//; s/ *$//')"
[ "$ctype" = "application/json" ] || fail "content-type '$ctype', want application/json"

jq -e . "$body" >/dev/null 2>&1 || fail "body is not valid JSON (an index.html fallback looks like this)"
jq -e '(keys == ["webcredentials"]) and (.webcredentials | keys == ["apps"]) and (.webcredentials.apps | type == "array" and length == 1 and all(type == "string"))' "$body" >/dev/null \
  || fail 'body must be exactly {"webcredentials":{"apps":["<app id>"]}}'

app="$(jq -r '.webcredentials.apps[0]' "$body")"
if [ -n "$expected" ]; then
  [ "$app" = "$expected" ] || fail "app id does not match the expected value"
else
  bundle_re="${bundle//./\\.}"
  printf '%s' "$app" | grep -Eq "^[A-Z0-9]{10}\\.${bundle_re}\$" \
    || fail "app id is not <10 uppercase alphanumerics>.$bundle"
fi

echo "check-aasa: OK $url (200, application/json, no redirect, app id $app)"
