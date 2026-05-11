# Trace Commons Roadmap

This document tracks open work on the standalone `tracedao-server`. It is
intentionally short and honest. The repo has zero current users — the previous
version of this doc was sized for a live Ironclaw pilot whose data extraction
no longer applies. Per-tenant rollout flags, promotion gates, rollout-smoke
evidence, and the "Phase 0 through Phase 6" cutover apparatus all assumed real
tenants who don't exist. When a deployment does appear, those sections will
come back tied to its specific constraints — not invented blind.

Companion docs (authoritative):

- `docs/trace-commons.md` — envelope contract and threat model
- `docs/trace-commons-storage.md` — storage contract
- `README.md` — current capability surface + binaries + local-dev commands

## Current State

What the standalone server has today:

- Hosted ingest, review, audit, retention, revocation, export, and worker APIs
  over PostgreSQL with forced row-level security on every Trace Commons table.
- Encrypted artifact store with three concrete backends: local-encrypted
  (dev default), filesystem-remote (rehearsal), and GCS behind a `gcs-client`
  build feature.
- Pluggable `KmsKeyWrapper` trait for envelope-encrypted per-object DEKs.
  Only `LocalMasterKeyWrapper` (dev-only) is implemented. Production
  deployments fail closed via `TRACE_COMMONS_KEK_REQUIRE_PRODUCTION_TRUST_BOUNDARY`.
- Tenant access grant storage, EdDSA-managed upload claims with guarded
  remote keyset refresh, standalone Ed25519 upload-claim issuer MVP.
- Trace Credits settlement: hash-only utility attestations, dry-run + signed
  central-issuer-approved live batches, NEAR receipt outbox, credit holds,
  scoped credit-cycle scheduler. Central-issuer principal allowlist gates
  every credit-bearing route.
- Ranking evidence storage with calibration / promotion / model-risk gates.
- Revocation propagation worker for object refs, credit ledger reversals,
  worker-queue invalidation, and service-owned artifact deletion.
- Hash-only audit and operational surfaces. `/v1/admin/config-status` and
  `/v1/admin/operational-summary` expose only safe label / hash / boolean fields.

What it does not have:

- Any clients. The Ironclaw extraction is not finished — Ironclaw should depend
  on `crates/tracedao-protocol`, but doesn't yet. **Until that happens, this
  server has no users at all.**
- A real KEK. The build refuses production startup. See "What Blocks First
  Real Use" below.

## What Blocks First Real Use

These are the only items that need to land before someone could actually
deploy this for real. Everything else is polish.

### 1. KEK strategy and implementation

The single architecturally load-bearing decision. Trace Commons already
operates with a constrained-operator threat model (hash-only audit, central
issuer fail-closed, no plaintext fallback). The KEK choice decides whether the
operator can read user content at all.

Candidates: TEE-rooted (dstack / Confidential Space / Nitro Enclaves), cloud
KMS (GCP Cloud KMS or AWS KMS), or hybrid (cloud KMS releases a sealing key
only to attested code). The TEE direction is the most consistent with the rest
of the system; cloud KMS is the smaller lift.

Needs a dedicated spec at `docs/superpowers/specs/<date>-trace-kek-strategy-design.md`
followed by a concrete `KmsKeyWrapper` impl that returns
`is_production_trust_boundary() = true`. Without this, the GCS build cannot
start in production.

### 2. Complete the Ironclaw extraction

`README.md` and earlier-extraction notes both flag that Ironclaw should depend
on the shared `tracedao-protocol` crate. Until that wiring lands, this server
has nothing talking to it. The work is on the Ironclaw side, not here, but
nothing in this repo matters until it happens. Worth coordinating before
investing further in server-side polish.

## Worth Doing Without Users

Real correctness / security improvements that hold value with zero users.
These tighten the trust model rather than the deployment runbook.

### Auth-derived `TenantCtx` propagation

Most paths already fail closed when an envelope's self-reported tenant
disagrees with the authenticated tenant. The roadmap goal of "every ingest,
review, export, worker, maintenance, and contributor-status path uses
auth-derived `TenantCtx` and treats envelope fields as attribution only" is
partially complete — the remaining work is a finite list of handlers. Pure
correctness; no users needed to land it.

### Privileged-action ABAC

Review override, destructive purge, and tombstone changes still lean partly
on static token roles. Tightening these to tenant-policy + signed-claim
allowed scopes/uses removes a real authorization shortcut. Bounded scope,
straightforward to land.

### Production-grade audit append/read

Hash-chain verification across audit rows, per-source content-read rows,
reason enforcement, sampled reconciliation. The plumbing exists; the
chain-verification + reconciliation pieces are partial. Integrity work, not
rollout work.

### Private vector infrastructure

Replace the deterministic vector similarity placeholder with a private
embedder + private search adapter that reads only approved redacted
projections, writes tenant-scoped vector metadata, and invalidates on
revocation/retention. Trait surface exists; the concrete impls do not.
Genuinely novel; aligns with the privacy-first design ethos.

### PostgreSQL `TraceCorpusStore` coverage

A handful of `TraceCorpusStore` slices still lack PostgreSQL integration
coverage. Always valuable to lock down behavior at the store boundary.

### Standalone upload-claim issuer hardening

The Ed25519 upload-claim issuer MVP exists. Making it production-shaped (key
rotation rehearsal, deploy story, basic CLI) gives a future deployment a real
entry point. Smaller than the others; shippable on its own.

## Deferred Until There Is a User

This work is explicitly *not* in scope right now. Each item is shaped for a
specific deployment that doesn't exist, and building it speculatively would
violate the project YAGNI rule. When a real user appears, build for that
user's actual constraints — not for the abstract pilot-rollout shape.

- **Per-tenant rollout flags / canary promotion.** No tenants → no rollout.
- **Rollout-smoke required-check apparatus** beyond what already exists for
  the drill surfaces. The existing drills are useful as caller-tests; the
  "fresh evidence within 24h" gate apparatus is not.
- **Operational dashboards.** The `operational-summary` API and
  `operational-metrics` Prometheus exporter already cover the readable
  surface. Grafana boards belong to a deployment, not this repo.
- **Runbooks for production rollback, key rotation, retention purge.** Same.
- **Migration / backfill tooling between object-store backends.** No pilot
  data to migrate. If a real deployment ever needs to move bytes, build a
  one-off CLI against the actual data shape — not generic worker routes
  invented blind.
- **"Phase 6: Production Cutover and Tenant Rollout"** in its entirety.
- **Cross-region replication, multi-bucket sharding, customer-managed
  bucket policies** for the object store. Operator concerns.

## Roadmap Principles

These are timeless:

- Contribution is opt-in and local-first. Uploads are always redacted
  `ironclaw.trace_contribution.v1` envelopes.
- Envelope contributor and tenant fields are attribution only. Authorization
  comes from request identity, tenant policy, and DB row scope.
- Metadata, object refs, hashes, indexes, ledgers, and workflow state live
  in PostgreSQL. Trace bodies and large artifacts live in encrypted object
  storage. Vector payloads live in a vector backend with relational metadata
  as the source of truth.
- Every derived artifact is versioned by input hash, worker version, policy
  version, and output artifact id.
- Hash-only audit, error logs, and operational surfaces. No URLs, ARNs,
  bearer tokens, account refs, transaction hashes, contributor identity, or
  trace bodies in stored rows or log strings.
- Fail closed by default. When a required gate is configured but its
  dependency is missing, refuse the path with a safe missing-control name.
  Never silently fall back to plaintext or a less-restricted backend.
- Tests prove tenant id, actor principal, object ref, and submission id
  propagation through callers — not just through unit-tested helpers.

## Verification Gates

The correctness criteria. None of these depend on having users.

- **Redaction.** Accepted envelopes and derived summaries never contain raw
  trace text, raw sidecar spans, secrets, local paths, bearer tokens, or
  raw tool payloads outside explicit policy.
- **Tenant.** Every read/write/mutation/export path is driven by auth-derived
  tenant and actor context, with same-id cross-tenant tests.
- **Object.** Every trace body read verifies object-ref tenant linkage,
  ciphertext hash, KEK context binding, decryptability, source status,
  consent scope, and allowed use.
- **KEK trust boundary.** Production deployments refuse `LocalMasterKeyWrapper`.
  v1 records reject any embedded `wrapped_dek`; v2 records require one;
  unknown schema versions are refused with `KekDowngradeRejected`.
- **Audit.** Privileged mutations and content reads emit typed, tenant-scoped,
  append-only audit events with reason, purpose, and decision input hashes
  where needed. All metadata is hash-only.
- **Revocation.** Revoke and retention flows invalidate or block submissions,
  object refs, derived rows, vectors, benchmarks, exports, worker queues,
  and credit settlement.

## When the World Changes

If a real first user appears, the right move is to rewrite this doc against
that user's actual constraints. The Phase 0-6 framing, the parallelization
lanes table, the per-tenant rollout checklist, and the rollout-smoke evidence
apparatus all become useful again at that point — but each will look
different depending on what kind of deployment the user is.

Do not restore those sections from the pre-2026-05 version of this file. The
pre-rewrite scope was sized for the Ironclaw `gecko-pass` extraction
specifically and most of it does not generalize.
