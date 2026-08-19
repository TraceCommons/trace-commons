#!/usr/bin/env bash
#
# Gate a release on a completed client end-to-end verification pass.
#
# Usage: check-verification-record.sh <version>
#   e.g. check-verification-record.sh 0.4.0
#
# Reads docs/operator/verification-records/app-v<version>.md and asserts the
# campaign recorded there actually completed. The runbook that produces the
# record is docs/operator/client-end-to-end-verification.md.
#
# Why this exists as a script and not as a paragraph in the release runbook:
# docs/release-runbook.md still says no release has been published and that
# the cask carries placeholder checksums, while app-v0.2.0, app-v0.2.1 and
# app-v0.3.0 have all shipped. A gate that lives only in prose goes stale
# exactly when it is needed. This one cannot.
#
# It does NOT verify that the pass was honest -- nothing can. It makes
# skipping verification a deliberate act rather than an oversight.
#
# Asserts:
#   1. A record file exists for the version.
#   2. Its pass-record block parses and carries every required key.
#   3. No key is still a template placeholder.
#   4. All three platforms are marked pass.
#   5. Cleanup counts are present, numeric, and reconciled.
#   6. Every update channel offered the current version.
#
# Exits 0 with `VerificationRecordOK: <details>` on success;
# exits 1 with `VerificationRecordFailure:<label>` on first failure.
# Labels are stable and content-free -- safe to paste into an issue.
# Stdlib only, bash 3 compatible (works on macOS); safe to re-run.

set -euo pipefail

fail() {
  echo "VerificationRecordFailure:$1" >&2
  exit 1
}

VERSION="${1:-}"
[ -n "$VERSION" ] || fail "MissingVersionArgument"

# Resolve the repo root from this script's own location so the check behaves
# the same from CI, from the repo root, and from anywhere else. Never assume
# the caller's working directory.
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# The records directory is overridable so the self-test can exercise every
# failure label without writing throwaway records into the committed
# directory. Releases never set it; CI never sets it.
RECORDS_DIR="${TRACE_VERIFICATION_RECORDS_DIR:-$REPO_ROOT/docs/operator/verification-records}"

RECORD="$RECORDS_DIR/app-v$VERSION.md"
if [ -n "${TRACE_VERIFICATION_RECORDS_DIR:-}" ]; then
  RECORD_REL="$RECORD"
else
  RECORD_REL="docs/operator/verification-records/app-v$VERSION.md"
fi

if [ ! -f "$RECORD" ]; then
  echo "no verification pass record at $RECORD_REL" >&2
  echo "Run docs/operator/client-end-to-end-verification.md against this" >&2
  echo "candidate build and commit the record before tagging." >&2
  fail "RecordMissing"
fi

# The record's machine-readable half is one fenced block labelled
# `pass-record`. It is a fenced block rather than an HTML comment so it stays
# visible when the file is read as documentation -- an invisible gate input is
# a gate nobody maintains.
BLOCK="$(awk '
  /^```pass-record[[:space:]]*$/ { inblock = 1; next }
  /^```[[:space:]]*$/            { if (inblock) exit }
  inblock                        { print }
' "$RECORD")"

[ -n "$BLOCK" ] || fail "PassRecordBlockMissing"

# Read one key out of the block. Trailing whitespace trimmed; a key that
# appears twice takes its first value, so an appended correction cannot
# silently override the original.
value_of() {
  printf '%s\n' "$BLOCK" \
    | awk -v key="$1" '
        $0 ~ "^" key ":" {
          sub("^" key ":[[:space:]]*", "")
          sub("[[:space:]]+$", "")
          print
          exit
        }'
}

REQUIRED_KEYS="version date operator
artifact_sha256_macos artifact_sha256_linux artifact_sha256_windows
invite_hash
platform_macos platform_linux platform_windows
submissions_withdrawn quarantined_found quarantined_resolved
update_channel_macos_brew update_channel_macos_dmg
update_channel_linux_flatpak update_channel_windows_appinstaller
defects_filed"

for key in $REQUIRED_KEYS; do
  v="$(value_of "$key")"
  [ -n "$v" ] || fail "MissingKey_$key"
  # The template ships every value as FILLME so an unedited copy cannot pass.
  case "$v" in
    FILLME | TBD | TODO | "<fill>") fail "PlaceholderValue_$key" ;;
  esac
done

RECORD_VERSION="$(value_of version)"
[ "$RECORD_VERSION" = "$VERSION" ] || fail "VersionMismatch"

# Artifact provenance is recorded as a hash, never a URL or a path.
for key in artifact_sha256_macos artifact_sha256_linux artifact_sha256_windows; do
  v="$(value_of "$key")"
  printf '%s' "$v" | grep -Eq '^[0-9a-f]{64}$' || fail "MalformedHash_$key"
done

# The invite is identified by its hash for the same reason. A raw invite code
# in a committed file is a live credential.
printf '%s' "$(value_of invite_hash)" | grep -Eq '^[0-9a-f]{64}$' \
  || fail "MalformedHash_invite_hash"

# All three platforms must pass. A partial campaign is recorded as a partial
# campaign; there is no "mostly passed".
for key in platform_macos platform_linux platform_windows; do
  v="$(value_of "$key")"
  case "$v" in
    pass) ;;
    fail | not-run) fail "PlatformNotPassed_$key" ;;
    *) fail "MalformedPlatformResult_$key" ;;
  esac
done

# Cleanup counts. Verification traces are real traces on a real server; a
# campaign that leaves them behind has not finished.
for key in submissions_withdrawn quarantined_found quarantined_resolved; do
  v="$(value_of "$key")"
  printf '%s' "$v" | grep -Eq '^[0-9]+$' || fail "MalformedCount_$key"
done

FOUND="$(value_of quarantined_found)"
RESOLVED="$(value_of quarantined_resolved)"
# Recorded whether or not anything was found, and reconciled when it was.
# A verification design that silently grows the quarantine queue would be
# creating the exact problem it exists to catch: that queue sat at 48 with
# zero reviews for 71 days. See docs/operator/quarantine-review.md.
[ "$FOUND" -eq "$RESOLVED" ] || fail "QuarantineUnreconciled"

# Update channels. An installed app that cannot reach the next version is a
# defect of the same class as one that cannot start, and it is invisible to
# any check that only looks at a fresh install. Both macOS channels are
# currently dead for a Homebrew contributor -- Sparkle is correctly disabled
# under Homebrew, and the cask has not been bumped in three releases -- which
# is why each channel is asserted separately rather than as "an update
# mechanism exists".
for key in update_channel_macos_brew update_channel_macos_dmg \
  update_channel_linux_flatpak update_channel_windows_appinstaller; do
  v="$(value_of "$key")"
  case "$v" in
    current) ;;
    stale | not-run) fail "UpdateChannelNotCurrent_$key" ;;
    *) fail "MalformedUpdateChannel_$key" ;;
  esac
done

echo "VerificationRecordOK: $RECORD_REL version=$VERSION platforms=3/3 \
withdrawn=$(value_of submissions_withdrawn) quarantine=$FOUND/$RESOLVED"
