#!/usr/bin/env bash
#
# Self-test for check-verification-record.sh.
#
# The release gate is only worth having if it actually refuses. Each case
# below builds a record that is wrong in exactly one way and asserts the
# specific failure label, then asserts that a complete record passes. Without
# the negative cases a gate that accidentally returned 0 for everything would
# look identical to a working one.
#
# Records are written under a temporary TRACE_VERIFICATION_RECORDS_DIR, so
# this never touches docs/operator/verification-records/.
#
# Exits 0 with `TestCheckVerificationRecordOK: <n> cases` on success;
# exits 1 with the first mismatch on failure.
# Stdlib only, bash 3 compatible (works on macOS); safe to re-run.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
GATE="$SCRIPT_DIR/check-verification-record.sh"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
export TRACE_VERIFICATION_RECORDS_DIR="$TMP"

CASES=0
FAILURES=0

# A complete, passing record. Every negative case is this with one edit.
# The hashes are obviously-fake repeated digits, not real artifact hashes.
GOOD_MACOS_HASH="1111111111111111111111111111111111111111111111111111111111111111"
GOOD_LINUX_HASH="2222222222222222222222222222222222222222222222222222222222222222"
GOOD_WINDOWS_HASH="3333333333333333333333333333333333333333333333333333333333333333"
GOOD_INVITE_HASH="4444444444444444444444444444444444444444444444444444444444444444"

write_record() {
  # write_record <version> [sed-expression]
  local version="$1"
  local edit="${2:-}"
  local path="$TMP/app-v$version.md"
  cat >"$path" <<RECORD
# Pass record

\`\`\`pass-record
version: $version
date: 2026-01-01
operator: test-operator
artifact_sha256_macos: $GOOD_MACOS_HASH
artifact_sha256_linux: $GOOD_LINUX_HASH
artifact_sha256_windows: $GOOD_WINDOWS_HASH
invite_hash: $GOOD_INVITE_HASH
platform_macos: pass
platform_linux: pass
platform_windows: pass
submitted_set_transcripts_only: pass
submissions_withdrawn: 6
quarantined_found: 2
quarantined_resolved: 2
update_channel_macos_brew: current
update_channel_macos_dmg: current
update_channel_linux_flatpak: current
update_channel_windows_appinstaller: current
defects_filed: none
\`\`\`
RECORD
  if [ -n "$edit" ]; then
    sed "$edit" "$path" >"$path.tmp" && mv "$path.tmp" "$path"
  fi
}

expect_failure() {
  # expect_failure <label> <version> [sed-expression]
  local want="$1" version="$2" edit="${3:-}"
  CASES=$((CASES + 1))
  [ "$want" = "RecordMissing" ] || write_record "$version" "$edit"
  local out status=0
  out="$("$GATE" "$version" 2>&1)" || status=$?
  if [ "$status" -eq 0 ]; then
    echo "case $want: expected failure, gate returned 0" >&2
    FAILURES=$((FAILURES + 1))
    return
  fi
  if ! printf '%s' "$out" | grep -q "VerificationRecordFailure:$want"; then
    echo "case $want: wrong label. got: $out" >&2
    FAILURES=$((FAILURES + 1))
  fi
}

expect_success() {
  local version="$1"
  CASES=$((CASES + 1))
  write_record "$version"
  local out status=0
  out="$("$GATE" "$version" 2>&1)" || status=$?
  if [ "$status" -ne 0 ]; then
    echo "complete record: expected pass, got: $out" >&2
    FAILURES=$((FAILURES + 1))
    return
  fi
  if ! printf '%s' "$out" | grep -q "VerificationRecordOK"; then
    echo "complete record: missing OK line. got: $out" >&2
    FAILURES=$((FAILURES + 1))
  fi
}

# No argument at all.
CASES=$((CASES + 1))
if out="$("$GATE" 2>&1)"; then
  echo "no-argument: expected failure, gate returned 0" >&2
  FAILURES=$((FAILURES + 1))
elif ! printf '%s' "$out" | grep -q "VerificationRecordFailure:MissingVersionArgument"; then
  echo "no-argument: wrong label. got: $out" >&2
  FAILURES=$((FAILURES + 1))
fi

# The case that matters most: tagging a version nobody verified.
expect_failure RecordMissing 9.9.9

# An unedited template copy must not pass.
CASES=$((CASES + 1))
sed 's/app-vFILLME/app-v8.8.8/' \
  "$SCRIPT_DIR/../../docs/operator/verification-records/TEMPLATE.md" \
  >"$TMP/app-v8.8.8.md"
if out="$("$GATE" 8.8.8 2>&1)"; then
  echo "template copy: expected failure, gate returned 0" >&2
  FAILURES=$((FAILURES + 1))
elif ! printf '%s' "$out" | grep -q "VerificationRecordFailure:"; then
  echo "template copy: wrong label. got: $out" >&2
  FAILURES=$((FAILURES + 1))
fi

# A record with no parseable block.
CASES=$((CASES + 1))
printf '# Pass record\n\nNothing machine readable here.\n' >"$TMP/app-v7.7.7.md"
if out="$("$GATE" 7.7.7 2>&1)"; then
  echo "no block: expected failure, gate returned 0" >&2
  FAILURES=$((FAILURES + 1))
elif ! printf '%s' "$out" | grep -q "VerificationRecordFailure:PassRecordBlockMissing"; then
  echo "no block: wrong label. got: $out" >&2
  FAILURES=$((FAILURES + 1))
fi

expect_failure MissingKey_operator 1.0.1 '/^operator:/d'
expect_failure PlaceholderValue_operator 1.0.2 's/^operator:.*/operator: FILLME/'
expect_failure VersionMismatch 1.0.3 's/^version:.*/version: 0.0.1/'
expect_failure MalformedHash_artifact_sha256_macos 1.0.4 \
  's/^artifact_sha256_macos:.*/artifact_sha256_macos: not-a-hash/'
expect_failure MalformedHash_invite_hash 1.0.5 \
  's/^invite_hash:.*/invite_hash: TC-INVITE-PLACEHOLDER/'
expect_failure PlatformNotPassed_platform_macos 1.0.6 \
  's/^platform_macos:.*/platform_macos: not-run/'
expect_failure PlatformNotPassed_platform_windows 1.0.7 \
  's/^platform_windows:.*/platform_windows: fail/'
expect_failure MalformedPlatformResult_platform_linux 1.0.8 \
  's/^platform_linux:.*/platform_linux: probably/'
expect_failure MalformedCount_submissions_withdrawn 1.0.9 \
  's/^submissions_withdrawn:.*/submissions_withdrawn: several/'
expect_failure SubmittedSetNotTranscriptsOnly 1.0.10 \
  's/^submitted_set_transcripts_only:.*/submitted_set_transcripts_only: fail/'
expect_failure MalformedTranscriptsOnly 1.0.11 \
  's/^submitted_set_transcripts_only:.*/submitted_set_transcripts_only: mostly/'
expect_failure QuarantineUnreconciled 1.1.0 \
  's/^quarantined_resolved:.*/quarantined_resolved: 1/'
expect_failure UpdateChannelNotCurrent_update_channel_macos_brew 1.1.1 \
  's/^update_channel_macos_brew:.*/update_channel_macos_brew: stale/'
expect_failure UpdateChannelNotCurrent_update_channel_linux_flatpak 1.1.2 \
  's/^update_channel_linux_flatpak:.*/update_channel_linux_flatpak: not-run/'
expect_failure MalformedUpdateChannel_update_channel_macos_dmg 1.1.3 \
  's/^update_channel_macos_dmg:.*/update_channel_macos_dmg: probably/'

# Zero quarantined rows is a normal, recordable outcome, not an absence.
CASES=$((CASES + 1))
write_record 1.2.0 's/^quarantined_found:.*/quarantined_found: 0/'
sed 's/^quarantined_resolved:.*/quarantined_resolved: 0/' "$TMP/app-v1.2.0.md" \
  >"$TMP/app-v1.2.0.tmp" && mv "$TMP/app-v1.2.0.tmp" "$TMP/app-v1.2.0.md"
if ! "$GATE" 1.2.0 >/dev/null 2>&1; then
  echo "zero quarantine: expected pass" >&2
  FAILURES=$((FAILURES + 1))
fi

expect_success 1.3.0

if [ "$FAILURES" -ne 0 ]; then
  echo "TestCheckVerificationRecordFailure: $FAILURES of $CASES cases" >&2
  exit 1
fi

echo "TestCheckVerificationRecordOK: $CASES cases"
