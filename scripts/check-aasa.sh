#!/usr/bin/env bash
# Smoke test for the apple-app-site-association file that tracecommons.ai
# serves for the macOS app's native passkeys. The file is built and deployed by
# TraceCommons/trace-commons-community; this script only reads it.
#
# Two fetches:
#   1. The origin, <base-url>/.well-known/apple-app-site-association: HTTP 200
#      with no redirect, Content-Type application/json, and a body of exactly
#      {"webcredentials":{"apps":["<TEAMID>.<bundle id>"]}}.
#   2. Apple's CDN, https://app-site-association.cdn-apple.com/a/v1/<host>,
#      which is what macOS actually fetches. It caches (about an hour, per its
#      Cache-Control), so a fresh deploy can pass (1) and fail (2) for a while.
#      It must answer 200 with the same body; its content type is not checked.
#
# The body is parsed and checked whatever the Content-Type says, so an HTML
# page relabelled as JSON fails, and so does JSON of the wrong shape.
#
# Usage:
#   scripts/check-aasa.sh <base-url> [expected-app-id]
#
# <base-url> is the site root, e.g. https://tracecommons.ai (post-deploy smoke)
# or http://127.0.0.1:8788. With expected-app-id (or TC_EXPECTED_AASA_APP_ID)
# the single apps entry must equal it; otherwise it must match
# ^[A-Z0-9]{10}\.ai\.tracecommons\.shell$ (override the bundle id with
# TC_MACOS_BUNDLE_ID).
#
# The CDN check runs for https base URLs and is skipped for http ones (a local
# preview is not something Apple can fetch). TC_AASA_CDN=0 skips it,
# TC_AASA_CDN=1 forces it, and TC_AASA_CDN_BASE overrides the CDN root (tests).
#
# Exit status: 0 both pass; 1 the origin fails; 3 the origin passes and the
# CDN does not (usually its cache; retry after max-age); 2 usage.
# Read-only; it never follows redirects, on purpose.
set -euo pipefail

base="${1:-}"
expected="${2:-${TC_EXPECTED_AASA_APP_ID:-}}"
bundle="${TC_MACOS_BUNDLE_ID:-ai.tracecommons.shell}"
cdn_base="${TC_AASA_CDN_BASE:-https://app-site-association.cdn-apple.com/a/v1}"

if [ -z "$base" ]; then
  echo "usage: $0 <base-url> [expected-app-id]" >&2
  exit 2
fi
for tool in curl jq; do
  command -v "$tool" >/dev/null || { echo "check-aasa: $tool is required" >&2; exit 2; }
done

base="${base%/}"
host="${base#*://}"
host="${host%%/*}"
host="${host%%:*}"
case "${TC_AASA_CDN:-}" in
  0) check_cdn=0 ;;
  1) check_cdn=1 ;;
  *) case "$base" in https://*) check_cdn=1 ;; *) check_cdn=0 ;; esac ;;
esac

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# fetch <url> <name>: writes $tmp/<name>.body and $tmp/<name>.hdr, prints the
# status. No -L: a redirect is a failure for AASA, not something to follow.
fetch() {
  : >"$tmp/$2.body"
  : >"$tmp/$2.hdr"
  # On a connection failure curl still prints 000 for http_code.
  curl -sS --max-time 20 -o "$tmp/$2.body" -D "$tmp/$2.hdr" -w '%{http_code}' "$1" || true
}

header() {
  grep -i "^$2:" "$tmp/$1.hdr" | tail -n1 | tr -d '\r' | cut -d: -f2- | sed 's/^ *//; s/ *$//' || true
}

# body_problem <name>: prints why the body is not an association file for the
# expected app, or nothing.
body_problem() {
  local body="$tmp/$1.body" app bundle_re
  if ! jq -e . "$body" >/dev/null 2>&1; then
    echo "body is not valid JSON (an HTML fallback looks like this)"
    return
  fi
  if ! jq -e '(type == "object") and (keys == ["webcredentials"]) and (.webcredentials | type == "object" and keys == ["apps"]) and (.webcredentials.apps | type == "array" and length == 1 and all(type == "string"))' "$body" >/dev/null; then
    echo 'body must be exactly {"webcredentials":{"apps":["<app id>"]}}'
    return
  fi
  app="$(jq -r '.webcredentials.apps[0]' "$body")"
  if [ -n "$expected" ]; then
    [ "$app" = "$expected" ] || echo "app id does not match the expected value"
  else
    bundle_re="${bundle//./\\.}"
    printf '%s' "$app" | grep -Eq "^[A-Z0-9]{10}\\.${bundle_re}\$" \
      || echo "app id is not <10 uppercase alphanumerics>.$bundle"
  fi
}

# --- 1. origin ---------------------------------------------------------------

origin_url="$base/.well-known/apple-app-site-association"
status="$(fetch "$origin_url" origin)"
problems=()
[ "$status" = "200" ] || problems+=("status $status, want 200 with no redirect")
if grep -qi '^location:' "$tmp/origin.hdr"; then
  problems+=("response carries a Location header")
fi
ctype="$(header origin content-type | tr '[:upper:]' '[:lower:]' | sed 's/ *;.*$//')"
[ "$ctype" = "application/json" ] || problems+=("content-type '$ctype', want application/json")
problem="$(body_problem origin)"
[ -z "$problem" ] || problems+=("$problem")

if [ "${#problems[@]}" -gt 0 ]; then
  for p in "${problems[@]}"; do echo "check-aasa: FAIL $origin_url: $p" >&2; done
  exit 1
fi
app="$(jq -r '.webcredentials.apps[0]' "$tmp/origin.body")"
echo "check-aasa: OK $origin_url (200, application/json, no redirect, app id $app)"

# --- 2. Apple's CDN ------------------------------------------------------------

if [ "$check_cdn" != "1" ]; then
  echo "check-aasa: SKIP Apple CDN check for $base (not https; TC_AASA_CDN=1 forces it)"
  exit 0
fi

cdn_url="${cdn_base%/}/$host"
status="$(fetch "$cdn_url" cdn)"
problems=()
[ "$status" = "200" ] || problems+=("status $status, want 200")
problem="$(body_problem cdn)"
[ -z "$problem" ] || problems+=("$problem")
if [ "${#problems[@]}" -eq 0 ] && [ "$(jq -r '.webcredentials.apps[0]' "$tmp/cdn.body")" != "$app" ]; then
  problems+=("app id differs from the origin's")
fi

if [ "${#problems[@]}" -gt 0 ]; then
  for p in "${problems[@]}"; do echo "check-aasa: FAIL $cdn_url: $p" >&2; done
  reason="$(header cdn apple-failure-reason)"
  [ -z "$reason" ] || echo "check-aasa: Apple-Failure-Reason: $reason" >&2
  age="$(header cdn age)"
  cache="$(header cdn cache-control)"
  echo "check-aasa: the origin passes; Apple's CDN caches (Cache-Control: ${cache:-none}, Age: ${age:-none}), so retry once that expires" >&2
  exit 3
fi
echo "check-aasa: OK $cdn_url (200, app id $app)"
