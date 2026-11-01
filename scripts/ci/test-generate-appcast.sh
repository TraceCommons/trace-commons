#!/usr/bin/env bash
# scripts/updates/generate-appcast.sh for both channels (staging design 4.7).
#
# The script takes everything that differs between the production and the
# staging appcast as arguments -- output path, signing key file, feed title,
# pre-release label -- rather than gaining a staging branch. This holds:
#
# - a production invocation, unchanged from before, writes the same appcast
#   it always did (pubDate aside), and still refuses a pre-release version;
# - a staging invocation with `--prerelease staging` accepts only
#   `X.Y.Z-staging.N` and writes its own title;
# - a pre-release label never loosens the version to anything else;
# - the signing key reaches sign_update only as a file, never in argv as
#   key material, and a missing key file is refused.
#
# Seconds: a stub sign_update stands in for Sparkle's, which needs macOS.
set -euo pipefail

repo="$(cd "$(dirname "$0")/../.." && pwd)"
script="$repo/scripts/updates/generate-appcast.sh"
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

# The stub prints what Sparkle's sign_update prints, and records its
# arguments so the test can see how the key was passed.
cat >"$scratch/sign_update" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$@" >"$(dirname "$0")/sign_update.args"
echo 'sparkle:edSignature="c3R1Yi1zaWduYXR1cmU=" length="999"'
STUB
chmod +x "$scratch/sign_update"
printf 'a disk image' >"$scratch/app.dmg"
printf 'synthetic-private-key' >"$scratch/ed-key"

generate() {
  "$script" \
    --build-version 4201 \
    --dmg-url "https://example.invalid/app.dmg" \
    --dmg-path "$scratch/app.dmg" \
    --sign-update "$scratch/sign_update" \
    --ed-key-file "$scratch/ed-key" \
    "$@"
}

# pubDate is the only line that changes from run to run.
normalized() {
  sed -E 's#<pubDate>[^<]*</pubDate>#<pubDate>DATE</pubDate>#' "$1"
}

# 1. Production, invoked exactly as before: the same document as before.
out="$scratch/production/appcast.xml"
if generate --short-version 0.20.6 --out "$out" >/dev/null 2>&1; then
  expected="$(
    cat <<'XML'
<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>Trace Commons</title>
    <item>
      <title>0.20.6</title>
      <pubDate>DATE</pubDate>
      <sparkle:minimumSystemVersion>14.0</sparkle:minimumSystemVersion>
      <enclosure
        url="https://example.invalid/app.dmg"
        sparkle:version="4201"
        sparkle:shortVersionString="0.20.6"
        length="12"
        type="application/octet-stream"
        sparkle:edSignature="c3R1Yi1zaWduYXR1cmU=" />
    </item>
  </channel>
</rss>
XML
  )"
  if [[ "$(normalized "$out")" == "$expected" ]]; then
    pass "a production appcast is unchanged"
  else
    fail "a production appcast changed:"
    diff <(printf '%s\n' "$expected") <(normalized "$out") >&2 || true
  fi
else
  fail "a production appcast did not generate"
fi

if grep -qx -- '--ed-key-file' "$scratch/sign_update.args" \
  && grep -qx -- "$scratch/ed-key" "$scratch/sign_update.args" \
  && ! grep -q 'synthetic-private-key' "$scratch/sign_update.args"; then
  pass "the signing key reaches sign_update as a file, not as key material"
else
  fail "sign_update did not receive the key as --ed-key-file <path>"
fi

# A refusal must be for the stated reason, not an unknown argument or a
# crash that happens to exit non-zero.
refused() {
  local desc="$1" reason="$2" out
  shift 2
  if out="$(generate --out "$scratch/refused.xml" "$@" 2>&1)"; then
    fail "$desc was accepted"
  elif [[ "$out" != *"$reason"* ]]; then
    fail "$desc was refused for another reason: $out"
  else
    pass "refused: $desc"
  fi
}

refused "a staging version without a pre-release label" "three-part numeric" --short-version 0.20.6-staging.7
refused "a two-part version" "three-part numeric" --short-version 0.20

# 2. Staging: its own label, title and output path.
out="$scratch/staging/appcast.xml"
if generate --short-version 0.20.6-staging.7 --prerelease staging \
  --title "Trace Commons Staging" --out "$out" >/dev/null 2>&1; then
  if grep -q '<title>Trace Commons Staging</title>' "$out" \
    && grep -q '<title>0.20.6-staging.7</title>' "$out" \
    && grep -q 'sparkle:shortVersionString="0.20.6-staging.7"' "$out" \
    && grep -q 'sparkle:version="4201"' "$out" \
    && ! grep -q '<title>Trace Commons</title>' "$out"; then
    pass "a staging appcast carries its own title and version"
  else
    fail "a staging appcast has the wrong title or version:"
    cat "$out" >&2
  fi
else
  fail "a staging appcast did not generate"
fi

refused "a pre-release label with a plain version" "X.Y.Z-staging.N" --short-version 0.20.6 --prerelease staging
refused "a pre-release label with another label's version" "X.Y.Z-staging.N" \
  --short-version 0.20.6-beta.7 --prerelease staging
refused "a pre-release version with no build number" "X.Y.Z-staging.N" \
  --short-version 0.20.6-staging. --prerelease staging
refused "a pre-release version with a word for a build number" "X.Y.Z-staging.N" \
  --short-version 0.20.6-staging.x --prerelease staging
refused "a pre-release version with a trailing part" "X.Y.Z-staging.N" \
  --short-version 0.20.6-staging.7.1 --prerelease staging
refused "a pre-release label that is not a plain word" "lower-case letters" \
  --short-version '0.20.6-a.b.7' --prerelease 'a.b'
refused "an empty title" "title must be" --short-version 0.20.6 --title ''
refused "a title that would break the XML" "title must be" --short-version 0.20.6 --title 'a</title>'

# 3. The key file is required to exist when named.
if out="$("$script" --short-version 0.20.6 --build-version 1 \
  --dmg-url "https://example.invalid/app.dmg" --dmg-path "$scratch/app.dmg" \
  --sign-update "$scratch/sign_update" --ed-key-file "$scratch/absent-key" \
  --out "$scratch/absent.xml" 2>&1)"; then
  fail "a missing key file was accepted"
elif [[ "$out" != *"ed key file not found"* ]]; then
  fail "a missing key file was refused for another reason: $out"
else
  pass "refused: a missing key file"
fi

if ((failures > 0)); then
  echo "$failures appcast test(s) failed" >&2
  exit 1
fi
echo "all appcast tests passed"
