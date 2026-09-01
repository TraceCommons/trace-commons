# Admin Drills

Every `/v1/admin/*-drill` endpoint, what it validates, and how often to
run. Each drill emits hash-only audit rows on success and contributes to
`rollout-smoke/evidence`.

All drills:
- Require an admin bearer token.
- Are `POST` with empty body.
- Return HTTP 200 with `success: true` on pass.
- Are idempotent — re-running has no side effects beyond a fresh audit
  row.

| Drill | Path | Validates | Required for promotion? | Recommended cadence |
|---|---|---|---|---|
| Key rotation | `/v1/admin/key-rotation-drill` | KEK wrap+unwrap round trip with tenant_ctx binding | yes | After each rotation; daily otherwise |
| Audit chain | `/v1/admin/audit-chain-drill` | Hash chain over `trace_audit_events` is intact | yes | Daily |
| DB reconciliation | `/v1/admin/db-reconciliation-drill` | No drift between submission rows, object refs, audit rows | yes | Daily |
| Postgres RLS | `/v1/admin/postgres-rls-drill` | RLS policies present and force_row_level_security on | yes | Daily |
| Retention dry-run | `/v1/admin/retention-dry-run-drill` | Retention scheduler can identify what would be pruned without doing it | yes | Weekly |
| Vector index | `/v1/admin/vector-index-drill` | Vector index is open, writable, can invalidate an entry | yes | After each gate model swap; daily otherwise |
| Analytics release | `/v1/admin/analytics-release-drill` | k-anon + DP noise gates apply correctly | nice-to-have | Weekly |
| Benchmark readiness | `/v1/admin/benchmark-readiness-drill` | Benchmark pipeline submitter/evaluator configured | nice-to-have | Weekly |
| Revocation propagation | `/v1/admin/revocation-propagation-drill` | Outbox + scheduler can drain queued revocations | yes | After a revocation event; daily |
| Revocation effects | `/v1/admin/revocation-effects-drill` | All revocation-effect handlers report ready | yes | Daily |
| Canary read | `/v1/admin/canary-read-drill` | A known-canary object can be read end-to-end | yes | Hourly during business |
| Object primary read | `/v1/admin/object-primary-read-drill` | Configured primary store for each route is reachable | yes | Daily |
| Object store migration | `/v1/admin/object-store-migration-drill` | Migration steps (if any) idempotent | nice-to-have | Before promotions |
| Rollback | `/v1/admin/rollback-drill` | Rollback path documented + dry-run executable | yes | Before each deploy |
| Credit settlement | `/v1/admin/credit-settlement-drill` | Settlement scheduler can resolve allowed policy version + issuer approval, can build a settlement event without committing | yes | Daily; before each settlement cycle |
| NEAR AI attestation | `/v1/admin/near-attestation-drill` | The inference endpoint is a TDX enclave running a pinned image, and the key signing its receipts is the key that enclave attests. See [`near-attestation-drill.md`](near-attestation-drill.md) | yes, **when a NEAR AI endpoint is configured**; reported not-applicable otherwise | Daily. **Costs one minimal paid completion per run** |

## Failure modes feeding `rollout-smoke/evidence`

`POST /v1/admin/rollout-smoke/preflight` walks the "required for
promotion" drills above and writes evidence rows. `GET
/v1/admin/rollout-smoke/evidence` returns a list of
`{ check_id, passed, last_run_at, evidence_hash }`. The deploy gate
must inspect this and refuse promotion if any required check is not
`passed: true`.

## How to add a new drill

When a future PR adds a new `/v1/admin/*-drill`:

1. Add the row to this table.
2. Add the curl call to [`smoke-test.md`](smoke-test.md).
3. Add it to the for-loop in
   [`scripts/operator/smoke-gate.sh`](../../scripts/operator/smoke-gate.sh).
4. Wire it into the `rollout_smoke_evidence` required-check list (in
   the binary).
