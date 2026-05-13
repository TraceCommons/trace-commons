#!/usr/bin/env bash
#
# Smoke test the gate path against a running trace-commons-ingest.
#
# - Calls every required /v1/admin/*-drill.
# - Records rollout-smoke preflight + reads the evidence list.
# - POSTs a fixture submission_id to /v1/workers/gate/evaluate and
#   asserts the response shape.
# - Exits 0 on success; 1 on first failure with a hash-only diagnostic.
#
# Default mode is DRY-RUN: the smoke flow exercises the gate worker but
# whether credit actually mints depends on
# TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA on the server. Pass
# --enable-credit to make this script also verify that a credit row was
# emitted (only meaningful when the server is configured for live
# emission).

set -euo pipefail

TARGET=""
ADMIN_TOKEN=""
WORKER_TOKEN=""
ENABLE_CREDIT=0
FIXTURE_SUBMISSION_ID="${TRACE_COMMONS_SMOKE_FIXTURE_SUBMISSION_ID:-}"

while [ $# -gt 0 ]; do
  case "$1" in
    --target=*)       TARGET="${1#*=}";       shift ;;
    --admin-token=*)  ADMIN_TOKEN="${1#*=}";  shift ;;
    --worker-token=*) WORKER_TOKEN="${1#*=}"; shift ;;
    --fixture=*)      FIXTURE_SUBMISSION_ID="${1#*=}"; shift ;;
    --enable-credit)  ENABLE_CREDIT=1;        shift ;;
    *) echo "SmokeGateUnknownArg: $1" >&2; exit 1 ;;
  esac
done

bail() {
  echo "SmokeGateFailure: $1" >&2
  exit 1
}

[ -n "$TARGET" ]       || bail "target_unset"
[ -n "$ADMIN_TOKEN" ]  || bail "admin_token_unset"
[ -n "$WORKER_TOKEN" ] || bail "worker_token_unset"

command -v curl >/dev/null 2>&1 || bail "curl_not_installed"
command -v jq   >/dev/null 2>&1 || bail "jq_not_installed"

REQUIRED_DRILLS=(
  key-rotation
  audit-chain
  db-reconciliation
  postgres-rls
  retention-dry-run
  vector-index
  analytics-release
  benchmark-readiness
  revocation-propagation
  revocation-effects
  canary-read
  object-primary-read
  object-store-migration
  rollback
  credit-settlement
)

echo "SmokeGate: config-status"
code=$(curl -sS -o /tmp/config-status.json -w "%{http_code}" \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  "$TARGET/v1/admin/config-status")
[ "$code" = "200" ] || bail "config_status_http_$code"
ready=$(jq -r '.gate_service_status.ready // false' /tmp/config-status.json)
[ "$ready" = "true" ] || bail "gate_service_not_ready"

echo "SmokeGate: drills"
for D in "${REQUIRED_DRILLS[@]}"; do
  code=$(curl -sS -o /tmp/drill.json -w "%{http_code}" -X POST \
    -H "Authorization: Bearer $ADMIN_TOKEN" \
    "$TARGET/v1/admin/$D-drill")
  if [ "$code" != "200" ]; then
    bail "drill_${D}_http_$code"
  fi
  ok=$(jq -r '.success // false' /tmp/drill.json)
  if [ "$ok" != "true" ]; then
    bail "drill_${D}_not_success"
  fi
done

echo "SmokeGate: rollout-smoke preflight"
code=$(curl -sS -o /tmp/preflight.json -w "%{http_code}" -X POST \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  "$TARGET/v1/admin/rollout-smoke/preflight")
[ "$code" = "200" ] || bail "preflight_http_$code"

echo "SmokeGate: rollout-smoke evidence"
code=$(curl -sS -o /tmp/evidence.json -w "%{http_code}" \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  "$TARGET/v1/admin/rollout-smoke/evidence")
[ "$code" = "200" ] || bail "evidence_http_$code"
failed=$(jq -r '.required_checks // [] | map(select(.passed != true)) | length' /tmp/evidence.json)
if [ "$failed" != "0" ]; then
  bail "rollout_smoke_required_check_not_passed:$failed"
fi

if [ -n "$FIXTURE_SUBMISSION_ID" ]; then
  echo "SmokeGate: fixture gate evaluate"
  code=$(curl -sS -o /tmp/eval.json -w "%{http_code}" -X POST \
    -H "Authorization: Bearer $WORKER_TOKEN" \
    -H "Content-Type: application/json" \
    -d "{\"submission_id\":\"$FIXTURE_SUBMISSION_ID\"}" \
    "$TARGET/v1/workers/gate/evaluate")
  [ "$code" = "200" ] || bail "evaluate_http_$code"

  # Required response fields per OrchestrationDecision shape.
  for f in gate_policy_version gate_version_hash perplexity_micros \
           tail_fraction_micros novelty_score_micros \
           embedding_evidence_hash attestation_chain_hash; do
    val=$(jq -r ".${f} // \"\"" /tmp/eval.json)
    if [ -z "$val" ] || [ "$val" = "null" ]; then
      bail "evaluate_missing_field:$f"
    fi
  done

  if [ "$ENABLE_CREDIT" = "1" ]; then
    # If the operator passed --enable-credit, also verify a fresh credit
    # row landed. We don't query PG from this script; instead we expect
    # the gate-evaluate response to expose `credit_event_emitted: true`
    # when the server-side delta is non-zero and ABAC passed.
    emitted=$(jq -r '.credit_event_emitted // false' /tmp/eval.json)
    if [ "$emitted" != "true" ]; then
      bail "credit_not_emitted_with_enable_credit"
    fi
  fi
else
  echo "SmokeGate: no fixture submission supplied; skipping evaluate"
fi

echo "SmokeGateOK"
