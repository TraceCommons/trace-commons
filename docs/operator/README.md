# Operator Runbook — `trace-commons-server`

This directory holds the deployment-shaped knowledge that lets an operator
take `trace-commons-server` from "compiles clean on `main`" to "first pilot
running on real hardware." It is intentionally separate from the architectural
docs (`docs/trace-commons*.md`) and per-slice design specs (`docs/superpowers/specs/`)
which describe *what* the system is. This directory describes *how to run it*.

## Status: outline (2026-05-13)

The outline below names every runbook doc and deployment script that the
pilot-prep PR will land. Brief descriptions; the actual content lands after
the audit-fixes PR merges so the env-var surface is stable.

Use this index as a contents page when the full runbook is filled in.

## Quick links by scenario

Start here. Find the row that matches what you are about to do and follow
the link.

| If you are... | Start with |
|---|---|
| First-time deploying `trace-commons-server` | [`./deployment.md`](./deployment.md) |
| Setting gate floors or calibrating thresholds | [`./calibration.md`](./calibration.md) |
| Validating a deployment before promoting | [`./smoke-test.md`](./smoke-test.md) |
| Running the model bake-off | [`./calibration.md`](./calibration.md) (Phase 0) + [`./agent-traces-bakeoff-run.md`](./agent-traces-bakeoff-run.md) |
| Handling an A2.6 bake-off result | [`./a26-bakeoff-result-handler.md`](./a26-bakeoff-result-handler.md) |
| Running the pilot bootstrap harness | [`./pilot-bootstrap.md`](./pilot-bootstrap.md) (see also [`./pilot-bootstrap-dryrun-notes.md`](./pilot-bootstrap-dryrun-notes.md) — known real-data defects) |
| Running the pilot-bootstrap first-100-traces dry run | [`./pilot-bootstrap-first-100-traces.md`](./pilot-bootstrap-first-100-traces.md) |
| Managing the HuggingFace dataset / model cache | [`./hf-dataset-cache-hygiene.md`](./hf-dataset-cache-hygiene.md) |
| Recording GPU instance spend | [`./gpu-cost-ledger.md`](./gpu-cost-ledger.md) |
| Rotating cloud-KMS keys | [`./key-rotation.md`](./key-rotation.md) |
| Swapping the gate model or embedder | [`./model-swap.md`](./model-swap.md) |
| Restoring from backup | [`./backup-restore.md`](./backup-restore.md) |
| Recovering a corrupted vector index | [`./vector-replay.md`](./vector-replay.md) |
| Investigating an audit-chain failure | [`./audit-trail-forensics.md`](./audit-trail-forensics.md) |
| Reading hash-only error classes from logs | [`./hash-only-logging.md`](./hash-only-logging.md) |
| Interpreting `/v1/admin/operational-summary` | [`./operational-summary.md`](./operational-summary.md) |
| Running or scheduling admin drills | [`./drills.md`](./drills.md) |
| Looking up an env var | [`./env-reference.md`](./env-reference.md) |
| Diagnosing a stuck or failing service | [`./troubleshooting.md`](./troubleshooting.md) |
| Understanding the deployment topology | [`./architecture.md`](./architecture.md) |

## Alphabetical reference

Every runbook in this directory, with a one-line description.

- [`./a26-bakeoff-result-handler.md`](./a26-bakeoff-result-handler.md) — post-run
  handler for the A2.6 bake-off: pull the report, fill the skeleton, route to
  the A2.7 / A2.7-partial / Phase A.5 outcome branch, tear down.
- [`./agent-traces-bakeoff-run.md`](./agent-traces-bakeoff-run.md) — operator
  activity for the A2.6 agent-traces bake-off across candidate gate models.
- [`./architecture.md`](./architecture.md) — one-page deployment topology
  (KMS, PostgreSQL, GCS, GPU host, ingest binary).
- [`./audit-trail-forensics.md`](./audit-trail-forensics.md) — how to query
  and verify the audit chain when investigating a dispute or anomaly.
- [`./backup-restore.md`](./backup-restore.md) — what is backed up where,
  restore procedures, and honest RPO/RTO targets.
- [`./calibration.md`](./calibration.md) — empirical procedure for tuning
  perplexity, tail-fraction, and novelty floors (dry-run → cutover).
- [`./deployment.md`](./deployment.md) — end-to-end first-deploy walkthrough;
  the authoritative top-of-funnel doc.
- [`./drills.md`](./drills.md) — the full set of `/v1/admin/*-drill`
  endpoints, what each validates, and cadence guidance.
- [`./env-reference.md`](./env-reference.md) — table of every
  `TRACE_COMMONS_*` env, default, required/optional, and surface touched.
- [`./gpu-cost-ledger.md`](./gpu-cost-ledger.md) — append-only ledger of
  GPU instance spend for bake-offs and corpus rebuilds.
- [`./hash-only-logging.md`](./hash-only-logging.md) — interpreting
  hash-only error classes in production logs.
- [`./hf-dataset-cache-hygiene.md`](./hf-dataset-cache-hygiene.md) — cache
  layout, disk-space requirements, and hygiene commands for the HuggingFace
  dataset and model cache used by pilot-bootstrap and the bake-off corpus
  builder.
- [`./key-rotation.md`](./key-rotation.md) — Cloud KMS key-version rotation
  procedure, including drill validation and rollback.
- [`./model-swap.md`](./model-swap.md) — procedure for upgrading the
  perplexity model or embedder and the gate-version implications.
- [`./operational-summary.md`](./operational-summary.md) — field-by-field
  meaning and alarm guidance for `/v1/admin/operational-summary`.
- [`./pilot-bootstrap.md`](./pilot-bootstrap.md) — operator runbook for the
  HF-trace replay harness used to seed pilot calibration data.
- [`./pilot-bootstrap-dryrun-notes.md`](./pilot-bootstrap-dryrun-notes.md) —
  findings from the real-HF-dataset dry-run; documents two pre-pilot defects
  in the harness's shard discovery and translators.
- [`./pilot-bootstrap-first-100-traces.md`](./pilot-bootstrap-first-100-traces.md) —
  controlled first-100 dry run against staging to verify gate decision
  distribution and audit chain row counts before scaling.
- [`./smoke-test.md`](./smoke-test.md) — post-deploy validation checklist
  that exercises every required drill plus a fixture gate evaluation.
- [`./troubleshooting.md`](./troubleshooting.md) — common failure modes by
  symptom, with hash-only signatures and fixes.
- [`./vector-replay.md`](./vector-replay.md) — operator reference for the
  `trace-commons-vector-replay` recovery binary.

## Doc lifecycle and freshness

These runbooks describe how to run the system as of the most recent
Phase A retrofit. As the system evolves, individual docs can fall behind
the implementation. When a runbook and the underlying spec disagree, treat
the spec as authoritative:

- Per-slice design specs live in [`../superpowers/specs/`](../superpowers/specs/).
- Per-slice implementation plans live in [`../superpowers/plans/`](../superpowers/plans/).
- Cross-cutting contracts (envelope, storage, threat model) live in
  [`../trace-commons.md`](../trace-commons.md),
  [`../trace-commons-storage.md`](../trace-commons-storage.md), and
  [`../trace-commons-roadmap.md`](../trace-commons-roadmap.md).

If you discover a drift while running a procedure, open an issue or PR
that updates the affected runbook in the same change set that updates the
code or spec.

## What is NOT here

This directory holds operator-facing runbooks only. The following live
elsewhere:

- **Architecture and threat-model design.** See
  [`../trace-commons.md`](../trace-commons.md),
  [`../trace-commons-storage.md`](../trace-commons-storage.md), and
  [`../trace-commons-roadmap.md`](../trace-commons-roadmap.md).
- **Per-slice spec and plan documents.** See
  [`../superpowers/specs/`](../superpowers/specs/) and
  [`../superpowers/plans/`](../superpowers/plans/).
- **Implementation reports.** See
  [`../superpowers/reports/`](../superpowers/reports/).

## Audience

A single deployment operator who:
- Owns the GCP project (or other cloud) where `trace-commons-server` will run
- Has access to a GPU host (NVIDIA H100, 80 GB, single-GPU) for the gate
  service
- Is comfortable with shell + terraform-or-equivalent, but is not a Rust
  developer
- Will calibrate the gate floors empirically on the first pilot's traces
- Is the same actor as the central credit issuer (early pilot assumption —
  the central-issuer profile is a single-actor configuration)

If the operator and the central credit issuer are different actors, see
`docs/superpowers/specs/2026-05-12-trace-kek-strategy-design.md` — the
operator-constrained trust model is Phase B work (dstack), not Phase A.

## Contents (planned)

### Orientation

- **`README.md`** *(this file)* — outline and audience.
- **`architecture.md`** — one-page deployment topology diagram: where the
  GCP project, Cloud KMS key, PostgreSQL instance, GCS bucket, GPU host,
  and (when wired) Ironclaw client live. The pieces are already specced
  individually; this stitches them into one picture.

### First deploy

- **`deployment.md`** — end-to-end walkthrough for the first deploy:
  prerequisites, GCP setup, env-var configuration, model staging,
  initial start, and what to look at in the first hour. Single
  authoritative document; everything below is referenced from here.
- **`env-reference.md`** — complete table of every `TRACE_COMMONS_*` env
  the binaries read, with default, required-or-optional flag,
  description, and which surface (KEK, gate, credit, audit) it touches.
  Generated by hand from the binary's `const TRACE_COMMONS_*` definitions
  but kept current as a stable contract.
- **`smoke-test.md`** — checklist the operator runs after first start
  AND before each subsequent deploy. Calls every `/v1/admin/*-drill`
  endpoint, runs a fixture gate evaluation, verifies all checks pass.
  Should be runnable in ~5 minutes.

### Operations

- **`key-rotation.md`** — GCP Cloud KMS key version rotation procedure.
  Covers staging a new key version, validating it via
  `/v1/admin/key-rotation-drill`, rolling the workload identity, and
  rolling back if needed. Documents the maximum claim-lifetime + refresh
  window where both keys must remain valid.
- **`model-swap.md`** — procedure for upgrading the perplexity model or
  the embedder. Covers (1) downloading + verifying the new weights,
  (2) updating `TRACE_COMMONS_PERPLEXITY_MODEL_ID` /
  `TRACE_COMMONS_EMBEDDER_MODEL_ID`, (3) what changes in the
  `gate_version_hash` and how grandfather-settled credit semantics
  apply, (4) re-calibration if the floors need adjusting.
- **`calibration.md`** — empirical procedure for tuning the perplexity,
  tail-fraction, and novelty floors. Three phases: dry-run gating
  (collect decisions without emitting credit), threshold selection
  (target a pass rate; document the trade-off), live cutover
  (`TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA` from 0 to its
  configured value). Includes guidance on collecting representative
  traces, what perplexity ranges to expect for known-novel vs
  known-duplicate inputs, and how to spot calibration drift over time.
- **`backup-restore.md`** — what's backed up where: PostgreSQL (full
  database including ledger + audit chain), GCS (encrypted artifact
  bytes), local disk (vector index files + model cache), Cloud KMS
  (out of scope — managed by GCP). Restore procedures + RPO/RTO
  targets honest about what's recoverable and what isn't.
- **`vector-replay.md`** — operator reference for the
  `trace-commons-vector-replay` recovery binary. Rebuilds a tenant's local
  vector index from `trace_gate_decisions` + the encrypted artifact
  store without touching audit/credit history. Used when the per-
  tenant `.usearch` file is corrupted, lost, or out of sync.

### Observability

- **`hash-only-logging.md`** — guide to interpreting the hash-only
  error classes (`KekUnwrapFailed`, `GateServiceUnavailable`,
  `RevocationPropagationFailure`, etc.) in production logs. For each
  class: what it means, what to check first, common root causes.
- **`operational-summary.md`** — what fields in
  `/v1/admin/operational-summary` mean and which ones should alarm.
  Includes the gate-related fields and the per-target-kind
  `revocation_propagation_terminal_failed_*` counters
  (`vector_entries`, `object_refs`, `export_manifests`,
  `export_manifest_items`, `derived_records`, `benchmark_artifacts`,
  `ranker_artifacts`, `credit_settlements`, `worker_queues`,
  `physical_delete_receipts`), gate service status reflection, etc.
- **`drills.md`** — the full set of `/v1/admin/*-drill` endpoints,
  what each one validates, and how often to run each. Calls out
  which drills are required for promotion vs nice-to-have.

### Troubleshooting

- **`troubleshooting.md`** — common failure modes by symptom. Each
  entry: observed behavior → hash-only log signature → root cause →
  fix. Includes the GPU OOM case, the `KekContextMismatch` case
  (most common cause: tenant_ctx misconfigured), the
  `EmbedderInferenceFailed` case (most common cause: model file
  missing), etc.
- **`audit-trail-forensics.md`** — how to read the audit chain when
  something went wrong: which tables to query, how to verify the
  hash chain, how to correlate a credit dispute back to its
  triggering submission and gate decision.

### Deferred (Phase B)

These docs are placeholders until the dstack migration:

- `dstack-migration.md` — operator procedure for moving from Phase A
  (cloud-KMS-rooted) to Phase B (dstack-attested-enclave-rooted).
- `attestation-verification.md` — how to validate attestation tokens
  in the live deployment.

## Deployment scripts (planned)

These live in `scripts/operator/` (separate from `scripts/` if that
already holds dev scripts). All idempotent, all hash-only-logging,
all callable from the runbook docs.

- **`scripts/operator/stage-models.sh`** — download Llama-3.1-8B-Instruct
  via `huggingface-cli` and BGE-large via `fastembed`'s cache mechanism,
  then verify SHA256 against pinned expected values. Refuses on hash
  mismatch.
- **`scripts/operator/smoke-gate.sh`** — runs every required
  `/v1/admin/*-drill` endpoint, records rollout-smoke evidence, then
  hits `POST /v1/workers/gate/evaluate` with a fixture submission
  and asserts the response shape. Exits 0 on success, 1 with a
  hash-only diagnostic on first failure.
- **`scripts/operator/rotate-kek.sh`** — GCP Cloud KMS key version
  rotation: creates new version, runs the key-rotation drill, prints
  the rollback command in case the operator needs to revert. Doesn't
  flip the active key automatically — that's an explicit operator
  decision.
- **`scripts/operator/smoke-deploy.sh`** — meta-script that runs
  stage-models + binary start + smoke-gate in sequence. Idempotent;
  safe to re-run after a partial deploy.

## What this runbook does NOT cover

- **Infrastructure-as-code.** Terraform/Pulumi/k8s manifests are
  intentionally not in the runbook — they're deployment-shape-specific
  and a real operator typically has their own IaC stack. The runbook
  references the GCP resources by purpose ("a Cloud KMS key", "a GCS
  bucket with versioning") rather than prescribing how they're
  provisioned.
- **Ironclaw client setup.** Out of scope for this repo. The runbook
  notes where the client integration points sit (upload-claim issuer,
  `POST /v1/submissions`) but the client side is a separate
  deployment story.
- **Scaling beyond single-host.** First pilot is single-GPU,
  single-binary. Multi-host scaling is a future operational concern.
- **Phase B dstack-specific procedures.** Placeholders only; full
  content lands with the dstack migration.

## When to update this runbook

- A new `TRACE_COMMONS_*` env is added: update `env-reference.md`.
- A new `/v1/admin/*-drill` endpoint is added: update `drills.md` +
  `smoke-test.md` + `scripts/operator/smoke-gate.sh`.
- A new hash-only error class is introduced: update
  `hash-only-logging.md`.
- A model is swapped in production: don't update the runbook — the
  procedure stays the same. The deployment-specific notes go in the
  operator's own change log.

## Open questions for the pilot-prep PR

When the audit-fixes PR merges and the full runbook gets written,
these are the open questions to resolve:

1. **GCP-only or cloud-agnostic in the runbook?** A1 ships GCP KMS;
   AWS / Azure are deferred. The runbook can either be GCP-specific
   (concrete, useful for the first pilot) or cloud-agnostic with a
   GCP appendix (more general, less useful). Recommendation:
   GCP-specific in v1; a cloud-agnostic refactor happens when a
   second cloud's KMS adapter exists.
2. **Should `smoke-gate.sh` actually emit credit?** Or stay in
   dry-run? Probably dry-run by default with an explicit
   `--enable-credit` flag for the first real production run.
3. **Calibration data source?** First pilot has no traces to
   calibrate against. The runbook should be honest that the first
   week's floors are educated guesses, with re-calibration after
   ~1000 real traces are gated.
