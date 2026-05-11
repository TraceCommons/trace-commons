# TraceDAO Server

Hosted server-side control plane for Trace Commons / TraceDAO.

This repository is the extraction point for the pieces that should not live
inside Ironclaw long term: public/private ingest, review queues, tenant access
grants, audit/retention/revocation workers, export jobs, upload-claim issuing,
object storage, and the production database schema.

## Where Trace Commons Is

Trace Commons is past the local-only MVP. The hosted server now owns its
database, object-storage abstraction, upload-claim issuer, and shared protocol
surface. The current shape:

| Area | State |
|------|-------|
| Local capture (Ironclaw side) | Stable. Opt-in, redaction-first, atomic queue writes, in-process queue worker, sanitized telemetry. |
| Upload-claim auth | EdDSA/Ed25519 managed keysets with `kid` selection, guarded HTTPS refresh, optional max-stale fail-closed; static tokens and HS256 are bridge-only. A standalone Ed25519-only issuer MVP ships in this repo. |
| Tenant access grants | Durable principal/role/scope/use grants with admin create/list/revoke. `TRACE_COMMONS_REQUIRE_TENANT_ACCESS_GRANTS=true` makes them fail-closed across ingest/review/export/audit. |
| PostgreSQL store | `TraceCorpusStore` with the `PgBackend` implementation. DB dual-write available; surface-scoped DB-read flags cover contributor, reviewer, replay, audit, and ranking lists. RLS is forced on all Trace Commons tables and centralized behind `trace_current_tenant_id()`. |
| Encrypted artifact store | Local-encrypted (dev default) and filesystem-remote (rehearsal). The trait surface is production-shaped; the cloud provider is the open gap (see below). |
| Revocation / retention | Tombstones, exact delayed-credit reversal, worker-cache invalidation, service-owned object deletion for every current artifact kind, configured remote-deleter for disabled cloud refs. |
| Observability | Admin operational summary + Prometheus-text `/v1/admin/operational-metrics`, structured per-gate warning logs, hash-only error/reason logging across workers. |
| Smoke / rollout | Required-check rollout-smoke gate with `/v1/admin/rollout-smoke/{evidence,preflight}`, dedicated drills for every required check, 24h staleness window. |
| Trace Credits | Server-side settlement with hash-only utility attestations, dry-run + central-issuer-approved live batches, NEAR receipt outbox, credit holds, scoped credit-cycle scheduler. Central issuer principal allowlist gates every credit-bearing route. |

The roadmap is `docs/trace-commons-roadmap.md`; the storage contract is
`docs/trace-commons-storage.md`; the envelope and threat model are
`docs/trace-commons.md`.

## Open Production Gaps

The main remaining work is production rollout hardening, not feature breadth.
The biggest concrete gaps, in priority order:

1. **Cloud object-store provider + KMS envelope encryption.** The
   `RemoteTraceArtifactProvider` trait, alias tracking, versioning gate, and
   migration drill all exist. AWS S3 + AWS KMS are not yet implemented;
   `aws_s3` / `gcs` / `azure_blob` provider names parse but resolve to a
   fail-closed disabled adapter. Migration backfill from local/filesystem-remote
   to cloud is the second half of this slice. Design draft:
   [`docs/superpowers/specs/2026-05-11-s3-trace-artifact-provider-design.md`](docs/superpowers/specs/2026-05-11-s3-trace-artifact-provider-design.md).
2. **`TenantCtx` propagation everywhere.** Envelope tenant fields are
   attribution-only and most paths already fail closed on drift, but the
   roadmap's "every ingest, review, export, worker, maintenance, and
   contributor-status path" coverage is not yet complete.
3. **PostgreSQL service-role smoke rehearsal.** RLS is forced and the
   runtime-role hash can be pinned via
   `TRACE_COMMONS_POSTGRES_RUNTIME_ROLE_SHA256`, but the full smoke suite has
   not yet been rehearsed under the pinned non-owner service role. Until that
   passes, RLS is a defense-in-depth guardrail rather than the active trust
   boundary.
4. **Per-tenant rollout of DB-primary and object-primary read flags.** Tenant
   A canary coverage exists; broader tenant rollout is gated on reconciliation
   parity remaining green.
5. **Durable private vector and benchmark/ranker workers in production.** The
   private embedder/searcher trait, vector worker, and benchmark/registry
   outbox all exist; deployed adapters and broad rollout evidence do not.

For the full slice list see the "Production Gap Queue" in
`docs/trace-commons-roadmap.md`.

## Operator Promotion Checklist (Per Tenant)

Keep every promotion tenant-scoped until reconciliation, rollback, and smoke
evidence is green:

1. DB dual-write on; backfill complete.
2. Active tenant access grants for exactly the principals and roles being
   promoted.
3. `/v1/admin/db-reconciliation-drill` returns clean (no `blocking_gaps`).
4. Promote surface-specific DB reader flags before object-ref-required modes;
   promote object-ref modes before object-primary modes.
5. Object-store rehearsal: `/v1/admin/object-store-migration-drill` clean,
   versioning required where applicable.
6. Key rotation drill, rollback drill, audit-chain drill, revocation
   propagation drill, retention dry-run drill, ranking-readiness drill — all
   with fresh (≤24h) passed evidence in `/v1/admin/rollout-smoke/evidence`.
7. Confirm `/v1/admin/rollout-smoke/preflight` shows zero missing required
   checks.

## Binaries

- `tracedao-ingest`: hosted ingest/review/admin/worker API.
- `tracedao-upload-claim-issuer`: EdDSA/Ed25519 upload-claim issuer for hosted
  contributors.

## Repository Layout

- `crates/tracedao-protocol`: shared TraceDAO protocol DTOs and redaction helpers.
- `crates/tracedao-server`: Rust server binaries.
- `migrations`: TraceDAO server database schema, renumbered as this repo's
  first migration.
- `docs`: Trace Commons design (`trace-commons.md`), storage contract
  (`trace-commons-storage.md`), and roadmap (`trace-commons-roadmap.md`).
- `docs/superpowers/specs`: per-slice design specs (e.g. the S3 provider
  spec linked above).

## Local Development

```bash
cargo check -p tracedao-server --bins
cargo test -p tracedao-server --test trace_corpus_storage_contract --test trace_corpus_pg_store
```

This repo builds without an Ironclaw path dependency. Ironclaw should depend on
the shared `tracedao-protocol` crate when the client-side integration is
rewired.

---

## Reference: Trace Credits

Trace Credits are non-transferable account credits backed by reviewed utility
evidence. Uploads and ranker scores do not settle credit directly. Utility
workers record hash-only attestations for accepted traces, admins run
settlement batches, and optional NEAR receipt calls are queued only after
off-chain settlement finalizes.

The credit path is gated by a stack of fail-closed controls. Most production
deployments will want the full central-issuer profile via
`TRACE_COMMONS_CREDIT_SETTLEMENT_REQUIRE_CENTRAL_ISSUER_PROFILE=true`, which
enforces:

- DB mirror + RLS readiness fail-closed
- Pinned PostgreSQL runtime role
  (`TRACE_COMMONS_POSTGRES_RUNTIME_ROLE_SHA256`)
- Managed EdDSA signed-token enforcement + tenant-access-grant enforcement
- Pinned central issuer principal refs
  (`TRACE_COMMONS_CREDIT_SETTLEMENT_CENTRAL_ISSUER_PRINCIPAL_REFS`)
- Fresh central source-list approvals
  (`TRACE_COMMONS_CREDIT_SETTLEMENT_REQUIRE_ISSUER_APPROVAL=true`,
  `TRACE_COMMONS_CREDIT_SETTLEMENT_ALLOWED_POLICY_VERSIONS`, optional
  `TRACE_COMMONS_CREDIT_SETTLEMENT_ISSUER_APPROVAL_MAX_AGE_HOURS`)
- Pinned NEAR credit contract + configured submit/confirm adapters with
  bearer-auth required
  (`TRACE_COMMONS_NEAR_CREDIT_REQUIRE_ADAPTER_AUTH=true`)
- Fresh rollout-smoke readiness
  (`TRACE_COMMONS_CREDIT_SETTLEMENT_REQUIRE_ROLLOUT_SMOKE_READY=true`)

When the central issuer profile is enabled, unlisted principals can still
inspect dry-runs but cannot record issuer approvals, finalize live settlement,
manually mark NEAR outbox rows, or trigger NEAR account freeze/unfreeze.
Unlisted reviewer/admin/worker principals cannot create positive credit
through manual mutation, utility credit/attestation, prediction-credit,
process-evaluation utility credit, or credit-bearing export generation;
unlisted utility-worker principals cannot start live settlement, credit-cycle,
or NEAR credit outbox schedulers.

Credit holds enqueue `freeze_credit_account` / `unfreeze_credit_account` NEAR
outbox rows around the active-hold transition. Revocation propagation appends
deterministic negative ledger rows plus `reverse_credit_receipt` calls for
settled revoked sources. All hold/approval/outbox audit rows are hash-only
and typed.

`/v1/admin/config-status` and `/v1/admin/operational-summary` expose only safe
readiness fields: configured / not-configured booleans, timeouts, bounds, hashed
key refs, and missing-control names. They never return ARNs, URLs, bearer
tokens, account references, approval evidence, or transaction hashes.

## Reference: Ranking Evidence

Ranking evidence is stored separately from settlement. Admins register
hash-only holdout calibration dataset manifests; workers register feature
hashes, model predictions, lab/reviewer/evaluator labels, and calibration runs.
All client-supplied hashes must be canonical lowercase `sha256:<64 hex>`;
symbolic placeholders are rejected.

Key invariants enforced server-side:

- For a `(calibration_dataset_hash, target_use, policy_version)` holdout key,
  lifecycle updates must keep source manifest hash and counts unchanged.
- Predictions must name a registered active or candidate model and reference
  an existing feature vector hash.
- Calibration treats repeated labels from the same source on the same
  submission and target use as corrections; a latest `disputed` label
  suppresses that source's evidence until a newer non-disputed label arrives.
- Label-source authority: utility workers write `frontier_lab`, reviewers write
  `reviewer`, benchmark workers write `benchmark`, process-evaluation workers
  write `system`, admins are the explicit override.
- Settlement of `ranking_utility` credit requires an active model with a
  fresh promotable calibration run for the same policy / target / dataset,
  every credit event bound to a `ranking_prediction:<uuid>` with matching
  score, and zero uncleared model-risk codes.

Production gates:

- `TRACE_COMMONS_RANKING_REQUIRE_SERVER_FEATURE_PROVENANCE=true` — requires
  server-derived feature evidence before ranking credit can mint or settle.
- `TRACE_COMMONS_RANKING_REQUIRE_CALIBRATION_DATASET_REGISTRY=true` —
  calibration runs fail closed unless the holdout hash has a matching
  registered non-retired dataset row.
- `TRACE_COMMONS_RANKING_REQUIRE_ACTIVE_CALIBRATION_DATASET=true` — that
  registered row must be `active` (requires the registry gate above).

Admin readiness surfaces: `/v1/admin/ranking/readiness-drill`,
`/v1/admin/ranking/adjudication-report`,
`/v1/admin/ranking/labeler-reliability-report`,
`/v1/admin/ranking/model-backtest-report`,
`/v1/admin/ranking/calibration-dataset-conflicts`.

## Reference: External Adapters

Operator-owned adapters are pluggable; the server records only safe configured
/ not-configured readiness fields in config status.

| Purpose | URL env | Bearer env | Timeout env |
|---------|---------|------------|-------------|
| Benchmark evaluator | `TRACE_COMMONS_BENCHMARK_EVALUATOR_URL` | `..._BEARER_TOKEN` | `..._TIMEOUT_MS` |
| Process evaluator | `TRACE_COMMONS_PROCESS_EVALUATOR_URL` | `..._BEARER_TOKEN` | `..._TIMEOUT_MS` |
| Benchmark registry submit | `TRACE_COMMONS_BENCHMARK_REGISTRY_SUBMITTER_URL` | `..._BEARER_TOKEN` | `..._TIMEOUT_MS` |
| Benchmark registry confirm | `TRACE_COMMONS_BENCHMARK_REGISTRY_CONFIRMATION_URL` | `..._BEARER_TOKEN` | `..._TIMEOUT_MS` |
| NEAR credit submit | `TRACE_COMMONS_NEAR_CREDIT_SUBMITTER_URL` | (operator-owned) | (operator-owned) |
| NEAR credit confirm | `TRACE_COMMONS_NEAR_CREDIT_CONFIRMATION_URL` | (operator-owned) | (operator-owned) |

Production deployments enforce bearer-auth presence with
`TRACE_COMMONS_BENCHMARK_REGISTRY_REQUIRE_ADAPTER_AUTH=true` and
`TRACE_COMMONS_NEAR_CREDIT_REQUIRE_ADAPTER_AUTH=true`.

The NEAR outbox is intentionally a deterministic method-call payload set for
a non-transferable receipt contract: only `settle_credit_receipt`,
`reverse_credit_receipt`, `freeze_credit_account`, and
`unfreeze_credit_account` are emitted, the server ledger remains
authoritative, and payloads carry batch ids, account hashes, source-list
hashes, policy versions, attestation/signature hashes, amounts, and
issuer-signature hashes — never trace bodies or raw contributor identity.
