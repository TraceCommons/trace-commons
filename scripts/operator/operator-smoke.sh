#!/usr/bin/env bash
#
# Local smoke harness for the four operator binaries:
#   trace-commons-review, trace-commons-admin,
#   trace-commons-worker, trace-commons-tenant.
#
# Builds each binary in debug mode (release if SMOKE_OPERATOR_RELEASE=1)
# and exercises --version and --help on each, then runs a few representative
# parse-through invocations (no network — uses --help on a subcommand to
# exercise the clap derive end-to-end). Asserts:
#
#   1. Each binary builds.
#   2. --version prints non-empty output.
#   3. --help lists the expected number of subcommands.
#   4. A representative subcommand on each binary parses cleanly.
#
# Exits 0 with `SmokeOperatorBinariesOK: <details>` on success;
# exits 1 with `SmokeOperatorBinariesFailure:<label>` on first failure.
# Stdlib only, bash 3 compatible (works on macOS); safe to re-run.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

if [[ "${SMOKE_OPERATOR_RELEASE:-0}" == "1" ]]; then
    PROFILE_FLAG="--release"
    PROFILE_DIR="release"
else
    PROFILE_FLAG=""
    PROFILE_DIR="debug"
fi

BIN_DIR="$REPO_ROOT/target/$PROFILE_DIR"

# Parallel arrays: BINARIES[i] expects EXPECTED_COUNTS[i] subcommands.
# If a binary gains or loses a subcommand, update both here and the
# operator-binaries.md doc.
BINARIES=(
    "trace-commons-review"
    "trace-commons-admin"
    "trace-commons-worker"
    "trace-commons-tenant"
)
EXPECTED_COUNTS=(
    "8"
    "12"
    "8"
    "11"
)
REPRESENTATIVE_SUBCMDS=(
    "quarantine-list"
    "maintenance-run"
    "worker-utility-credit"
    "tenant-policy-get"
)

fail() {
    echo "SmokeOperatorBinariesFailure:$1" >&2
    exit 1
}

build_one() {
    local name
    name="$1"
    # shellcheck disable=SC2086
    cargo build -p trace-commons-server $PROFILE_FLAG --bin "$name" \
        > /dev/null 2>&1 || fail "build:$name"
}

check_help_subcmd_count() {
    local name want out count
    name="$1"
    want="$2"
    out="$("$BIN_DIR/$name" --help 2>&1)" || fail "help-failed:$name"
    count="$(printf '%s\n' "$out" | awk '
        /^Commands:/ { in_cmds=1; next }
        in_cmds && /^Options:/ { exit }
        in_cmds && /^[[:space:]]*$/ { exit }
        in_cmds && /^  help[[:space:]]/ { next }
        in_cmds && /^  [a-z]/ { count++ }
        END { print count+0 }
    ')"
    if [[ "$count" != "$want" ]]; then
        fail "subcmd-count:$name:want=$want:got=$count"
    fi
}

check_version() {
    local name out
    name="$1"
    out="$("$BIN_DIR/$name" --version 2>&1)" || fail "version-failed:$name"
    if [[ -z "$out" ]]; then
        fail "empty-version:$name"
    fi
}

check_subcommand_help() {
    local name sub
    name="$1"
    sub="$2"
    "$BIN_DIR/$name" "$sub" --help > /dev/null 2>&1 || fail "subcmd-help:$name:$sub"
}

main() {
    local i n
    n="${#BINARIES[@]}"

    echo "Building operator binaries..."
    for ((i = 0; i < n; i++)); do
        build_one "${BINARIES[$i]}"
    done

    echo "Checking --version on each binary..."
    for ((i = 0; i < n; i++)); do
        check_version "${BINARIES[$i]}"
    done

    echo "Checking subcommand counts via --help..."
    for ((i = 0; i < n; i++)); do
        check_help_subcmd_count "${BINARIES[$i]}" "${EXPECTED_COUNTS[$i]}"
    done

    echo "Checking representative subcommand --help on each binary..."
    for ((i = 0; i < n; i++)); do
        check_subcommand_help "${BINARIES[$i]}" "${REPRESENTATIVE_SUBCMDS[$i]}"
    done

    echo "SmokeOperatorBinariesOK: review=8 admin=12 worker=8 tenant=11"
}

main "$@"
