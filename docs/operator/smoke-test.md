# Smoke Test

5-minute checklist run after each deploy. Codified in
[`scripts/operator/smoke-gate.sh`](../../scripts/operator/smoke-gate.sh);
running the script is the preferred path. This doc is the per-step
breakdown so an operator can debug a partial failure.

The smoke test is **dry-run by default** — it exercises the gate worker
route but the configured `TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA`
is what decides whether credit mints. Set the delta to 0 during
calibration; pass `--enable-credit` to `smoke-gate.sh` only after
calibration is complete and policy is approved for live cutover.

## Prereqs

- `trace-commons-ingest` listening (you have the base URL).
- An **admin** bearer token with rights to call the `/v1/admin/*` drills.
- A **worker** bearer token with `gate` scope for
  `POST /v1/workers/gate/evaluate`.
- A seeded **canary** submission id, a tenant that must not be able to
  see it, and a tenant with object-primary routing disabled.
- A **revoked** canary submission id whose revocation has drained.
- A settlement **policy version** from the server's
  `TRACE_COMMONS_CREDIT_SETTLEMENT_ALLOWED_POLICY_VERSIONS`.

`smoke-gate.sh` takes these as `--canary-submission`,
`--canary-isolation-tenant`, `--object-primary-fallback-tenant`,
`--revoked-submission` and `--settlement-policy-version` (or the matching
`TRACE_COMMONS_SMOKE_*` env vars) and refuses to start without them,
because the four drills that need them can never report `ready` otherwise.

## Steps

Each step uses `BASE=https://ingest.example.com` and assumes
`ADMIN=<admin token>`, `WORKER=<worker token>`.

### 1. Config status

```sh
curl -s -H "Authorization: Bearer $ADMIN" "$BASE/v1/admin/config-status" | jq
```

Expect: HTTP 200 with `critical_warnings: []` and
`gate_service_status.ready: true`.

### 2. Operational summary

```sh
curl -s -H "Authorization: Bearer $ADMIN" "$BASE/v1/admin/operational-summary" | jq
```

Expect: HTTP 200. See [`operational-summary.md`](operational-summary.md)
for which fields should alarm.

### 3. Required drills

Run each of these (POST, JSON body, `Content-Type: application/json`).
All must return HTTP 200 with `ready: true`. Most take `{}`; the four
marked below need fields the server cannot infer — see
[`drills.md`](drills.md) for what each one means.

| Drill | Endpoint |
|---|---|
| Key rotation | `POST /v1/admin/key-rotation-drill` |
| Audit chain | `POST /v1/admin/audit-chain-drill` |
| DB reconciliation | `POST /v1/admin/db-reconciliation-drill` |
| Postgres RLS | `POST /v1/admin/postgres-rls-drill` |
| Retention dry-run | `POST /v1/admin/retention-dry-run-drill` |
| Vector index | `POST /v1/admin/vector-index-drill` |
| Analytics release | `POST /v1/admin/analytics-release-drill` |
| Benchmark readiness | `POST /v1/admin/benchmark-readiness-drill` |
| Revocation propagation | `POST /v1/admin/revocation-propagation-drill` |
| Revocation effects | `POST /v1/admin/revocation-effects-drill` (needs `submission_id` of a revoked canary) |
| Canary read | `POST /v1/admin/canary-read-drill` (needs `submission_id` + `isolation_tenant_id`) |
| Object primary read | `POST /v1/admin/object-primary-read-drill` (needs `submission_id` + `fallback_tenant_id`) |
| Object store migration | `POST /v1/admin/object-store-migration-drill` |
| Rollback | `POST /v1/admin/rollback-drill` |
| Credit settlement | `POST /v1/admin/credit-settlement-drill` (needs `policy_version`) |

Example:

```sh
# The eleven that take an empty JSON object.
for D in key-rotation audit-chain db-reconciliation postgres-rls \
         retention-dry-run vector-index analytics-release \
         benchmark-readiness revocation-propagation \
         object-store-migration rollback; do
  echo "=== $D ==="
  curl -s -X POST \
    -H "Authorization: Bearer $ADMIN" \
    -H "Content-Type: application/json" -d '{}' \
    "$BASE/v1/admin/$D-drill" | jq '{ready, blocking_gaps}'
done

# The four that need input.
curl -s -X POST -H "Authorization: Bearer $ADMIN" \
  -H "Content-Type: application/json" \
  -d "{\"submission_id\":\"$REVOKED_SUBMISSION\"}" \
  "$BASE/v1/admin/revocation-effects-drill" | jq '{ready, blocking_gaps}'

curl -s -X POST -H "Authorization: Bearer $ADMIN" \
  -H "Content-Type: application/json" \
  -d "{\"submission_id\":\"$CANARY_SUBMISSION\",
       \"isolation_tenant_id\":\"$ISOLATION_TENANT\"}" \
  "$BASE/v1/admin/canary-read-drill" | jq '{ready, blocking_gaps}'

curl -s -X POST -H "Authorization: Bearer $ADMIN" \
  -H "Content-Type: application/json" \
  -d "{\"submission_id\":\"$CANARY_SUBMISSION\",
       \"fallback_tenant_id\":\"$FALLBACK_TENANT\"}" \
  "$BASE/v1/admin/object-primary-read-drill" | jq '{ready, blocking_gaps}'

curl -s -X POST -H "Authorization: Bearer $ADMIN" \
  -H "Content-Type: application/json" \
  -d "{\"policy_version\":\"$SETTLEMENT_POLICY_VERSION\"}" \
  "$BASE/v1/admin/credit-settlement-drill" | jq '{ready, blocking_gaps}'
```

### 4. Record rollout-smoke evidence

```sh
curl -s -X POST -H "Authorization: Bearer $ADMIN" \
  "$BASE/v1/admin/rollout-smoke/preflight"
curl -s -H "Authorization: Bearer $ADMIN" \
  "$BASE/v1/admin/rollout-smoke/evidence" | jq '.required_checks'
```

Expect: every required-check entry has `passed: true`.

### 5. Fixture gate evaluation

```sh
curl -s -X POST -H "Authorization: Bearer $WORKER" \
  -H "Content-Type: application/json" \
  -d '{"submission_id":"<seeded fixture submission_id>"}' \
  "$BASE/v1/workers/gate/evaluate" | jq
```

Expect: response shape with `gate_policy_version`, `gate_version_hash`,
non-zero `perplexity_micros`, populated `embedding_evidence_hash`. If the
configured delta is `0`, no credit row appears in `trace_credit_ledger`.

### 6. Verify the audit chain advanced

```sql
SELECT prev_audit_event_hash, audit_event_hash, action
FROM trace_audit_events
ORDER BY occurred_at DESC LIMIT 5;
```

Each row's `prev_audit_event_hash` should equal the previous row's
`audit_event_hash`. The `audit-chain-drill` above verifies this; this
manual check is for sanity during incidents.

## Passing criteria

All five sections green. Any failure → see
[`troubleshooting.md`](troubleshooting.md) and
[`hash-only-logging.md`](hash-only-logging.md).
