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

## Open Work

This repo has zero contributor-facing deployments — see
`docs/trace-commons-roadmap.md` for the full framing. As of 2026-05-12 the
work is phased: Phase A is the
pilot-readiness slice (real gate service on regular GPU hardware with cloud
KMS as the KEK, accepting an operator-trusted model); Phase B is the trust
upgrade to a dstack-attested enclave, deferred until pilot operational
learning is in hand. The KEK trust-model regression is intentional and
documented — contributor-facing language must be honest that TEE-rooted
privacy is a planned upgrade, not a current property.

### Phase A — blocks pilot

1. **Real gate service on regular GPU hardware.** The trait surface
   (`TraceGateService`, `KmsKeyWrapper`) is shipped (PRs #9–#12). Mock
   perplexity / embedder / vector-index impls exist in
   `tracedao-gate-enclave`. What's still needed:
   - `CloudKmsKeyWrapper` (GCP KMS first) wrapping per-object DEKs. Satisfies
     the existing `TRACE_COMMONS_KEK_REQUIRE_PRODUCTION_TRUST_BOUNDARY`
     startup gate by convention.
   - Real `PerplexityScorer` (Llama-class via candle / mistralrs / ort —
     decision spec needed first).
   - Real `Embedder` (BGE-large / gte-large, matryoshka variants).
   - Real `VectorIndex` (usearch with on-disk persistence; no sealing).
   - `novelty_utility` credit-event emission through the existing
     central-issuer ABAC + audit-hashing pipeline.
   - Revocation worker hook calling `invalidate_vector_entry`.

   See `docs/superpowers/specs/2026-05-11-trace-kek-strategy-design.md`
   (chosen-path note) and
   `docs/superpowers/specs/2026-05-11-private-vector-system-design.md`
   (rephased).

2. **Complete the Ironclaw extraction** — *partial mitigation now exists.*
   Ironclaw should depend on the shared `tracedao-protocol` crate for the
   contributor UX, but that wiring lives on the Ironclaw side and is not
   under this repo's control. As of 2026-05-14, the `tracedao-pilot-bootstrap`
   binary (`crates/tracedao-server/src/bin/tracedao-pilot-bootstrap.rs`,
   landed in PR #47 and rewritten end-to-end in PR #67 against the real
   HuggingFace agent-traces schema) provides a bootstrap path: it replays
   HF agent-traces sessions through the existing `/v1/traces` ingest API,
   generating realistic submissions for gate-floor calibration, audit-chain
   validation, and embedder + vector-index seeding without requiring real
   contributors. The PR #62 dry-run surfaced parquet-only loading and
   fictional translator schemas; PR #67 replaces those with a JSONL session
   loader and three working translators (swival, pi-mono, deepseek) verified
   end-to-end with 5/5 idempotent submissions against real swival, and drops
   the `parquet` / `arrow-*` deps. PR #51 adds a local smoke harness, PR #54
   fixed the swival schema in the corpus builder, and PR #52 indexes the
   operator runbooks. See `docs/operator/pilot-bootstrap.md` for operator
   usage. This is not a substitute for Ironclaw — it is a load-generation
   harness — but it is now real-data-capable, so the only remaining blocker
   on first real use is operator-side: provision a host, run the binary,
   watch the sidecar, and decide when to flip live for the Ironclaw rewire.

### Phase B — trust upgrade (after pilot)

Move the gate-service binary inside an attested dstack enclave. Swap
`CloudKmsKeyWrapper` for `DstackKekWrapper`. Re-wrap every DEK under the
new wrapper (one batch pass; v2 envelope format already supports it via
the `wrapper_kind` field). No schema, envelope-format, or trait changes —
roughly 2 weeks of integration work assuming dstack-GPU primitives have
stabilized by then.

### Worth doing without users

Real correctness / security work that holds value with zero users — these
tighten the trust model, not the deployment runbook.

- **Auth-derived `TenantCtx` propagation** into every ingest / review /
  export / worker / maintenance path. Most paths already fail closed on
  drift; the remaining surface is a finite list of handlers.
- **Privileged-action ABAC** for review override, destructive purge, and
  tombstone changes. Tightens authorization away from static token roles.
- **Production-grade audit append/read** with hash-chain verification,
  per-source content-read rows, sampled reconciliation. Partial today.
- **Private vector infrastructure** — replace the deterministic placeholder
  with a real private embedder + search adapter over redacted projections.
- **PostgreSQL `TraceCorpusStore` integration coverage** for remaining slices.
- **Standalone upload-claim issuer hardening** — key rotation rehearsal,
  deploy story, basic CLI on the existing Ed25519 MVP.

### Deferred until there is a user

Explicit non-goals while the repo has zero deployments — per-tenant rollout
flags, smoke-evidence apparatus, operator runbooks, migration tooling between
object-store backends, and the full Phase 6 cutover machinery. When a real
deployment names its constraints, that work returns shaped to its needs. See
the roadmap for the full deferred list.

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
- `docs/superpowers/specs`: per-slice design specs (e.g. the cloud trace
  artifact provider spec linked above).

## Local Development

```bash
cargo check -p tracedao-server --bins
cargo test -p tracedao-server --test trace_corpus_storage_contract --test trace_corpus_pg_store
```

This repo builds without an Ironclaw path dependency. Ironclaw should depend on
the shared `tracedao-protocol` crate when the client-side integration is
rewired.

## CI

GitHub Actions runs on every push to `main` and every pull request against
`main` (see `.github/workflows/ci.yml`). The pipeline exercises three jobs:

- `cargo check -p tracedao-server --bins` (default features).
- `cargo check -p tracedao-server --bins --features local-gpu-models`
  (non-CUDA variant; the `local-gpu-models-cuda` variant is operator-tested
  on GPU hosts only).
- `cargo test -p tracedao-server` (default features; the existing integration
  test suite).

`cargo fmt --check` is wired into CI as of PR #56 (repo-wide `cargo fmt --all`
sweep landed in the same PR). Warnings-as-errors is now green on `main`:
PR #57 gated dead-code (`ThroughputRecord`, `VramRecord`) in
`tracedao-gate-calibrate` behind `#[allow(dead_code)]`, and PR #59 gated
`tracedao-ingest` test-only items behind `#[cfg(test)]`. `cargo clippy -D
warnings` is still not wired into CI; that remains the next CI gate to
land. Run the same commands locally before pushing.

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
