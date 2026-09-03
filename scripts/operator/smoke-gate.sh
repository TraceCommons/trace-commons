#!/usr/bin/env bash
#
# Smoke test the gate path against a running trace-commons-ingest.
#
# - Calls every required /v1/admin/*-drill and asserts `ready: true`.
#   Four of them need operator-supplied inputs; see the flags below. Without
#   those the drills run but report blocking gaps, so the script refuses to
#   start rather than reporting a false pass.
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
#
# Required inputs:
#   --target=            base URL of the ingest service
#   --admin-token=       admin bearer token
#   --worker-token=      worker bearer token with gate scope
#   --canary-submission= submission_id of the seeded canary trace
#                        (canary-read, object-primary-read)
#   --canary-isolation-tenant=
#                        a tenant that must NOT be able to see the canary;
#                        canary-read cannot prove isolation without it
#   --object-primary-fallback-tenant=
#                        a tenant with object-primary routing disabled;
#                        object-primary-read cannot prove fallback without it
#   --revoked-submission=
#                        submission_id of a canary trace that has actually
#                        been revoked (revocation-effects)
#   --settlement-policy-version=
#                        a policy version listed in the server's
#                        TRACE_COMMONS_CREDIT_SETTLEMENT_ALLOWED_POLICY_VERSIONS
# Optional inputs:
#   --fixture=           submission_id for the gate/evaluate step
#   --issuer-approval-evidence-hash=
#                        sha256:<64 hex> approval bound to the current
#                        settlement source list; needed only when the server
#                        requires issuer approval

set -euo pipefail

TARGET=""
ADMIN_TOKEN=""
WORKER_TOKEN=""
ENABLE_CREDIT=0
FIXTURE_SUBMISSION_ID="${TRACE_COMMONS_SMOKE_FIXTURE_SUBMISSION_ID:-}"

# Four of the fifteen required drills take a request body with at least one
# field the server cannot infer. They are supplied the same way as the gate
# fixture: a flag, or the matching env var.
CANARY_SUBMISSION_ID="${TRACE_COMMONS_SMOKE_CANARY_SUBMISSION_ID:-}"
CANARY_ISOLATION_TENANT_ID="${TRACE_COMMONS_SMOKE_CANARY_ISOLATION_TENANT_ID:-}"
OBJECT_PRIMARY_FALLBACK_TENANT_ID="${TRACE_COMMONS_SMOKE_OBJECT_PRIMARY_FALLBACK_TENANT_ID:-}"
REVOKED_SUBMISSION_ID="${TRACE_COMMONS_SMOKE_REVOKED_SUBMISSION_ID:-}"
SETTLEMENT_POLICY_VERSION="${TRACE_COMMONS_SMOKE_SETTLEMENT_POLICY_VERSION:-}"
# Optional: only needed when the server sets
# TRACE_COMMONS_CREDIT_SETTLEMENT_REQUIRE_ISSUER_APPROVAL=true. When unset the
# drill falls back to the server's own configuration.
ISSUER_APPROVAL_EVIDENCE_HASH="${TRACE_COMMONS_SMOKE_ISSUER_APPROVAL_EVIDENCE_HASH:-}"

while [ $# -gt 0 ]; do
  case "$1" in
    --target=*)       TARGET="${1#*=}";       shift ;;
    --admin-token=*)  ADMIN_TOKEN="${1#*=}";  shift ;;
    --worker-token=*) WORKER_TOKEN="${1#*=}"; shift ;;
    --fixture=*)      FIXTURE_SUBMISSION_ID="${1#*=}"; shift ;;
    --canary-submission=*)  CANARY_SUBMISSION_ID="${1#*=}"; shift ;;
    --canary-isolation-tenant=*) CANARY_ISOLATION_TENANT_ID="${1#*=}"; shift ;;
    --object-primary-fallback-tenant=*)
      OBJECT_PRIMARY_FALLBACK_TENANT_ID="${1#*=}"; shift ;;
    --revoked-submission=*) REVOKED_SUBMISSION_ID="${1#*=}"; shift ;;
    --settlement-policy-version=*) SETTLEMENT_POLICY_VERSION="${1#*=}"; shift ;;
    --issuer-approval-evidence-hash=*)
      ISSUER_APPROVAL_EVIDENCE_HASH="${1#*=}"; shift ;;
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

# These are not optional conveniences. canary-read, object-primary-read,
# revocation-effects and credit-settlement are all required-for-promotion
# drills, and each reports a blocking gap -- never `ready` -- without the
# corresponding input. Refuse up front rather than failing mid-loop.
[ -n "$CANARY_SUBMISSION_ID" ] || bail "canary_submission_id_unset"
[ -n "$CANARY_ISOLATION_TENANT_ID" ] || bail "canary_isolation_tenant_id_unset"
[ -n "$OBJECT_PRIMARY_FALLBACK_TENANT_ID" ] \
  || bail "object_primary_fallback_tenant_id_unset"
[ -n "$REVOKED_SUBMISSION_ID" ] || bail "revoked_submission_id_unset"
[ -n "$SETTLEMENT_POLICY_VERSION" ] || bail "settlement_policy_version_unset"

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

# Every drill handler takes `Json<T>`, which is a required extractor: a body-
# less POST is rejected by axum before the handler runs. Eleven of the fifteen
# have all-defaulted request structs and take `{}`; the four below have at
# least one required field.
drill_body() {
  case "$1" in
    canary-read)
      jq -nc --arg submission_id "$CANARY_SUBMISSION_ID" \
             --arg isolation_tenant_id "$CANARY_ISOLATION_TENANT_ID" \
             '{submission_id: $submission_id,
               isolation_tenant_id: $isolation_tenant_id}'
      ;;
    object-primary-read)
      jq -nc --arg submission_id "$CANARY_SUBMISSION_ID" \
             --arg fallback_tenant_id "$OBJECT_PRIMARY_FALLBACK_TENANT_ID" \
             '{submission_id: $submission_id,
               fallback_tenant_id: $fallback_tenant_id}'
      ;;
    revocation-effects)
      jq -nc --arg submission_id "$REVOKED_SUBMISSION_ID" \
             '{submission_id: $submission_id}'
      ;;
    credit-settlement)
      jq -nc --arg policy_version "$SETTLEMENT_POLICY_VERSION" \
             --arg approval_hash "$ISSUER_APPROVAL_EVIDENCE_HASH" \
             '{policy_version: $policy_version}
              + (if $approval_hash == "" then {}
                 else {issuer_approval_evidence_hash: $approval_hash} end)'
      ;;
    *)
      printf '{}'
      ;;
  esac
}

echo "SmokeGate: drills"
for D in "${REQUIRED_DRILLS[@]}"; do
  body=$(drill_body "$D")
  code=$(curl -sS -o /tmp/drill.json -w "%{http_code}" -X POST \
    -H "Authorization: Bearer $ADMIN_TOKEN" \
    -H "Content-Type: application/json" \
    --data "$body" \
    "$TARGET/v1/admin/$D-drill")
  if [ "$code" != "200" ]; then
    bail "drill_${D}_http_$code"
  fi
  # Every drill response carries `ready: bool` plus a label-only
  # `blocking_gaps` list. There is no `success` field on any of them.
  ok=$(jq -r '.ready // false' /tmp/drill.json)
  if [ "$ok" != "true" ]; then
    gaps=$(jq -r '(.blocking_gaps // []) | join(",")' /tmp/drill.json)
    bail "drill_${D}_not_ready:${gaps:-no_blocking_gaps_reported}"
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
