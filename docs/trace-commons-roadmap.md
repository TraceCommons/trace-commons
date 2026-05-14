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

### Production Gap Queue (2026-05-14, post A2.6 outcome routing)

Ordered. Top item is the active blocker.

1. **A2.7 perplexity floor calibration.** A2.6 trends toward Outcome 1
   ("at least one candidate AUC > 0.5") with three of four candidates
   complete: Llama-3.1-8B-Instruct AUC 0.342, Qwen3-8B-Base AUC 0.243,
   **Qwen 3.6 27B Dense AUC 0.936** (crosses the 0.5 threshold).
   Outcome 1 fires the full A2.7 path per the A2.6 spec's outcome map.
   The A2.7 plan stub (PR #74) is ready to promote to the active plan
   pending the final Gemma 4 31B candidate. Calibration targets a
   pilot-deployable perplexity floor from the winning model's
   distribution against the agent-traces novel slice.
2. **Gemma 4 31B Base bake-off run.** The fourth candidate is still in
   flight on Lambda H100; expected completion ~21:00 UTC. Result is
   informational for A2.7 — the Outcome 1 branch is already locked in
   by Qwen 3.6 27B's 0.936 — but the row is needed to finalize the
   report and to inform model selection if Gemma's AUC also crosses 0.5.
3. **Pilot launch with a 27B-class gate-service deployment.** Server
   side is code-complete + smoke-validated; pilot-bootstrap is real-
   data-capable (PR #67); the `pilot-bootstrap-first-100` runbook
   (PR #70) walks the operator path; the tail_floor credentials-leak
   blocker is resolved (PR #86). Remaining work is operator-side:
   provision a 27B-class GPU host, load the winning model (Qwen 3.6
   27B Dense unless Gemma 4 31B overtakes it), apply the A2.7-calibrated
   perplexity floor, then run the bootstrap. The size-pattern finding
   (small models flunk, 27B-class passes) means the pilot deployment
   goal now requires 27B-class hardware — not the 8B-class footprint
   A2.5 had assumed.
4. **Tail-fraction floor calibration.** Currently 0 (disabled) per
   A2.5. Subcommand landed in PR #66 (`tracedao-gate-calibrate
   tail-floor`). Capability-complete; awaits the first pilot run's data
   to produce a real floor. Unblocks after the pilot run starts emitting
   decision rows.
5. **Ironclaw client wiring.** Still the eventual unblock for real
   contributor traffic; out of this repo's control. Pilot-bootstrap
   harness is the work-around for everything that previously required
   real users.

Recently closed: A2.6 outcome-routing question (settled by Qwen 3.6
27B's AUC 0.936 — see "Deferred" below for the parked Phase A.5 work);
tail_floor credentials-leak pilot blocker (PR #86); CI clippy
enforcement (PR #78); Actions Node 24 + pilot-bootstrap smoke job
(PR #79); A2.6 corpus archived for A.5a reuse (PR #83); A.5a rarity
pre-flight tool (PR #84).

### Deferred

- **Phase A.5 perplexity-replacement metric.** A2.6's Outcome 1 fired
  on Qwen 3.6 27B Dense (AUC 0.936), so the metric-replacement
  escalation branch did not trigger. The size-pattern finding — see
  the memory note `project_perplexity_size_pattern.md` — explains
  why: aggregate perplexity discriminates novel reasoning only at
  27B-class capacity. The 8B-class candidates we measured remained
  inverted (Llama-3.1-8B-Instruct 0.342, Qwen3-8B-Base 0.243),
  consistent with the A2.3c/A2.4 results. The Phase A.5 plan stub
  (PR #65) and the per-token rarity bake-off path (PR #63 + A.5a
  pre-flight in PR #84) stay on `main` as an option for a future
  cost-driven retreat to an 8B gate, but are no longer on the
  critical pilot path.

### 1. Phase A — real gate service on regular hardware with cloud KMS

The pilot-readiness slice. Trace Commons aspires to an operator-constrained
threat model, but the dstack-GPU operational story isn't settled yet and
gating the pilot on it costs months. Phase A ships a real working gate
service on regular GPU hardware with **cloud KMS as the KEK**, accepting
that the operator and cloud provider can read user content via KMS
`Decrypt`. Phase B (below) does the trust upgrade once dstack is ready.

**Phase A status (2026-05-14, post A2.6 outcome routing): code-complete
+ smoke-validated; pilot-bootstrap real-data-capable; A2.6 Outcome 1
trending.** All A1–A6 work items below plus four bake-off retrofits
(A2.1, A2.2, A2.3, A2.5) and two real bake-off runs (A2.3c + A2.4)
are merged on `main`. The per-token rarity scorer landed in the Rust
bake-off binary (PR #63) on the mock-scorer path under a `--scorer
perplexity|token-rarity|both` flag; real-scorer rarity wiring is
deferred at `BakeoffRealRarityNotImplemented`. The A2.6 report skeleton
(PR #64) and a conditional Phase A.5 implementation plan stub (PR #65)
are on `main`. The pilot-bootstrap binary was rewritten end-to-end
against the real HF agent-traces schema in PR #67. The tail-fraction
floor calibration subcommand landed in PR #66 (`tracedao-gate-calibrate
tail-floor`). CI clippy enforcement landed in PR #78, the Actions
Node-24 + pilot-bootstrap smoke job in PR #79, the A2.6 corpus archive
in PR #83, the A.5a rarity pre-flight tool in PR #84, and the
tail_floor credentials-leak fix (critical pilot blocker) in PR #86.

In flight: **A2.6 agent-traces novel-slice 4-way bake-off has three of
four candidates complete and is trending Outcome 1** (at least one
candidate AUC > 0.5):

- Llama-3.1-8B-Instruct: AUC 0.342
- Qwen3-8B-Base: AUC 0.243
- **Qwen 3.6 27B Dense: AUC 0.936** (crosses the 0.5 threshold)
- Gemma 4 31B Base: in flight, ETA ~21:00 UTC

Per the A2.6 spec's outcome map, "at least one candidate AUC > 0.5"
fires the full A2.7 path. The A2.7 plan stub (PR #74) is ready to
promote pending the Gemma 4 31B row. The size-pattern finding — 8B
candidates flunk, 27B candidates pass — has a direct deployment
implication: the pilot now needs a 27B-class GPU host. Pilot launch
is the next gate, blocked on Gemma 4 31B completion, A2.7 promotion,
operator-side pilot-bootstrap execution, and post-pilot tail-fraction
calibration. Phase A.5 (perplexity replacement) is deferred — see the
Production Gap Queue "Deferred" subsection.
The binary boots green on Lambda Cloud GPU hardware
(A10 / A100 / H100); `audit-chain-drill` returns `ready: true`. The
empirical model bake-off ran four candidates (Llama-3.1-8B-Instruct,
Qwen3-8B-Base, Qwen 3.6 27B Dense, Gemma 4 31B Base) against two
distinct duplicate-slice corpora (boilerplate + Wikipedia). The
headline finding is uncomfortable: **perplexity-based novelty AUC is
inverted (< 0.5) across all candidate × corpus combinations** —
modern instruct-aligned LLMs find OASST2-style reasoning *less*
surprising than common duplicate content. A2.5 reconfigures the gate
floors to ship the perplexity floor at 0 (disabled) for pilot launch,
keep tail-fraction at 0 pending post-first-1000-trace calibration,
and rely on the novelty-embedder floor (500000) as the active primary
gate. The deeper perplexity-replacement metric design is parked under
**Phase A.5** below pending real pilot data.

- A1: `CloudKmsKeyWrapper` (GCP KMS) — done
- A2: real `PerplexityScorer` (candle + Llama-3.1-8B-Instruct as
  incumbent) — done
- A2.1: empirical model bake-off retrofit (`tracedao-gate-calibrate
  bake-off`, corpus builder, decision rule, operator runbook Phase 0) —
  done; see `docs/operator/calibration.md` § Phase 0
- A2.2: candle arch dispatch + Gemma 4 support + Qwen3 QK-Norm fix —
  done. Superseded for the production runtime path by A2.3
  (mistralrs replaces the hand-rolled candle `ScorerBackend` enum);
  retained as a documented validation pass that ground-truthed
  candle's per-arch surface and fixed the Qwen3 silent-QK-Norm bug
  before A2.3 took over.
- A2.3: mistralrs backend migration + Qwen 3.6 support — done.
  Code merged (mistralrs git-pinned to `2d4ba4f`); per-arch candle
  dispatch replaced by mistralrs auto-detection.
- A2.3c: 4-way bake-off real run (Llama-3.1-8B + Qwen3-8B + Qwen 3.6
  27B Dense + Gemma 4 31B), boilerplate-duplicate corpus — done;
  report at `docs/superpowers/reports/2026-05-13-model-bakeoff-result-a23c.{json,md}`.
  All four AUCs measured below 0.5; winner-by-rule was Qwen3-8B-Base
  (Apache-2.0 license tiebreaker inside a marginal-AUC band).
- A2.4: corpus iteration with Wikipedia-introductions duplicate slice
  (same 4 candidates, same code) — done; report at
  `docs/superpowers/reports/2026-05-14-model-bakeoff-result-a24.{json,md}`.
  AUCs moved but none crossed 0.5. Llama-Instruct +0.120, Gemma 4
  31B +0.130; Qwen base models slightly worse. A2.4's winner-by-rule
  flipped to Llama-3.1-8B-Instruct (highest in-budget AUC); the
  flip is informational since A2.5 disables the perplexity floor.
- A2.5: gate-floor recalibration after bake-off findings — done; see
  `docs/superpowers/reports/2026-05-14-gate-floor-recalibration-findings.md`
  and `docs/operator/calibration.md` Phase 1. Perplexity floor ships
  at 0 (disabled) for pilot launch; tail-fraction floor at 0
  pending post-first-1000-trace calibration; novelty floor at 500000
  is the active primary gate. Model pick stays Qwen3-8B-Base for
  cost (smallest VRAM footprint; choice no longer load-bearing).
- A2.6: agent-traces novel-slice bake-off — **largely done, trending
  Outcome 1**. Three of four candidates complete on Lambda H100:
  Llama-3.1-8B-Instruct AUC 0.342, Qwen3-8B-Base AUC 0.243, and
  **Qwen 3.6 27B Dense AUC 0.936** (crosses 0.5). Gemma 4 31B Base
  remains in flight, ETA ~21:00 UTC. Per the A2.6 spec's outcome map,
  "at least one candidate AUC > 0.5" fires the full A2.7 path; A.5
  (perplexity replacement) is deferred since the 27B run cleared the
  threshold. Report skeleton pre-written (PR #64). PR #50 landed the
  A2.7 follow-up spec stub; PR #58 refined it with outcome-branch
  decision recipes; PR #74 stubs the A2.7 plan, ready to promote
  pending the Gemma row. PR #83 archived the A2.6 corpus for A.5a
  pre-flight reuse. Spec:
  `docs/superpowers/specs/2026-05-14-agent-traces-bakeoff-design.md`;
  operator runbook: `docs/operator/agent-traces-bakeoff-run.md`.
- A2.7: perplexity floor calibration from the A2.6 winning model —
  plan stub on `main` (PR #74), ready to promote to the active plan
  once Gemma 4 31B completes. This is the next-gate work after A2.6.
- A3: real `Embedder` (fastembed + BGE-large-en-v1.5) — done
- A4: real `VectorIndex` (usearch with on-disk persistence) — done
- A5: `novelty_utility` credit-event emission — done
- A6: revocation worker hook (`invalidate_vector_entry`) plus typed
  propagation-failure audit retrofit for non-vector targets — done
- A.6: pilot-bootstrap HF-trace replay harness
  (`tracedao-pilot-bootstrap` binary) — done; real-data-capable as of
  PR #67. PR #62 dry-run surfaced parquet-only loading and fictional
  translator schemas; PR #67 rewrote the binary end-to-end against the
  real HF agent-traces schema with a JSONL session loader and three
  working translators (swival, pi-mono, deepseek), verified end-to-end
  with 5/5 idempotent submissions against real swival. `parquet` and
  `arrow-*` deps dropped. Awaits operator run for the first 30k
  submissions per A.6's "What success looks like" criteria. Spec:
  `docs/superpowers/specs/2026-05-14-pilot-bootstrap-harness-design.md`.
  Plan: `docs/superpowers/plans/2026-05-14-pilot-bootstrap-harness.md`.
  Runbook: `docs/operator/pilot-bootstrap.md`.

The current standalone foundation (PRs #9–#12) already has:

- `KmsKeyWrapper` trait, `LocalMasterKeyWrapper` (dev), `DstackKekWrapper`
  (stub that bails on calls)
- `TraceGateService` trait, `InMemoryGateService`, `EnclaveGateService`
  composing mock perplexity / embedder / vector-index from
  `tracedao-gate-enclave`
- `POST /v1/workers/gate/evaluate` worker route writing
  `trace_gate_decisions` rows
- Migration V23 (gate decision table + novelty_utility event kind) and
  V24 (vector_entry_id column)

What Phase A adds:

- **`CloudKmsKeyWrapper`** — `KmsKeyWrapper` impl wrapping per-object DEKs
  through GCP Cloud KMS (AWS KMS adapter parallel if needed). Returns
  `is_production_trust_boundary() = true` by convention; the trust model
  is operator-trusted. Satisfies the existing
  `TRACE_COMMONS_KEK_REQUIRE_PRODUCTION_TRUST_BOUNDARY` startup gate.
- **Real `PerplexityScorer`** — loads a configured base model (Llama-3.1-8B
  class or similar), runs prefill, computes per-token logprobs, aggregates
  to the perplexity + tail metric. Inference library choice (candle / mistralrs /
  ort / Python sidecar) needs its own short decision spec before
  implementation.
- **Real `Embedder`** — BGE-large / gte-large class, matryoshka variants
  preferred. Same inference path as the perplexity model.
- **Real `VectorIndex`** — `usearch` with on-disk persistence. No sealed
  snapshots in Phase A; regular at-rest disk encryption only.
- **`novelty_utility` credit-event emission** — wires the gate-pass path
  through the central-issuer ABAC + audit-row hashing pipeline (the
  previous implementer correctly flagged this as non-trivial and
  deferred it).
- **Revocation worker hook** — calls `invalidate_vector_entry` after a
  submission is revoked. Needs the propagation-failure audit-row shape
  specified first.

Strategy brief: `docs/superpowers/specs/2026-05-11-trace-kek-strategy-design.md`
(updated 2026-05-12 with the chosen-path note).
Full design: `docs/superpowers/specs/2026-05-11-private-vector-system-design.md`
(applies to Phase A with the enclave-resident framing relaxed; the same
components run on regular hardware).

### 1.5. Phase A.5 — perplexity-replacement metric (deferred per A2.6 Outcome 1)

A2.3c + A2.4 showed that aggregate perplexity does not discriminate
novel reasoning from duplicate content on any of the four 8B-class
candidates we measured (AUC < 0.5 across the board; the metric was
inverted, not noisy). A2.6 then re-ran the same four-way comparison
against the agent-traces novel slice and found a clean size pattern:
the 8B candidates remained inverted (Llama 0.342, Qwen3 0.243), but
Qwen 3.6 27B Dense scored AUC 0.936. The A2.6 outcome map routes that
to the A2.7 perplexity-floor-calibration path, not to the
metric-replacement path. Phase A.5 is therefore deferred: it remains
a documented option for a future cost-driven retreat from a 27B-class
gate, but it is no longer on the critical pilot path. The Phase A.5
plan stub (PR #65), the per-token rarity bake-off path (PR #63), and
the A.5a pre-flight tool (PR #84) stay on `main` so that an 8B-class
replacement metric can be picked up later without re-spinning the
scaffolding.

Three candidate approaches recorded in
`docs/superpowers/reports/2026-05-14-gate-floor-recalibration-findings.md`:

- **Contrastive perplexity** — delta in logprobs between two model
  checkpoints. No schema change; one extra model load.
- **Per-token rarity** — gate on the lowest-N logprob tail. A
  tighter version of `tail_fraction`; may collapse into it after
  pilot calibration. Python prototype landed in PR #55
  (`scripts/research/`); Rust bake-off integration landed in PR #63
  on the mock-scorer path under a `--scorer
  perplexity|token-rarity|both` flag in the existing
  `tracedao-gate-calibrate bake-off` binary. Real-scorer rarity
  wiring is deferred at `BakeoffRealRarityNotImplemented` — it
  activates only on the A2.6 AUC < 0.4 branch per the Phase A.5
  plan stub (PR #65).
- **Learned discriminator** — small classifier trained on labeled
  novel/duplicate exemplars. Highest ceiling, depends on labeled
  pilot data.

Dependency: first ~1000 pilot traces. Until those exist, designing a
replacement metric is premature.

### 2. Phase B — dstack migration

Once dstack-GPU operational tooling is settled and the pilot has produced
real operational learning, do the trust-model upgrade:

- New `KmsKeyWrapper` impl that replaces the cloud-KMS-rooted unseal with
  a dstack-attested unseal (path B1 or C from the strategy brief).
- Re-wrap every existing DEK under the new wrapper. The wrap format
  already carries `wrapper_kind`, so v2 envelopes are forward-compatible —
  one batch pass over `trace_credit_ledger.gate_version_hash`-stamped
  vector entries.
- Move the gate-service binary inside the attested enclave; add
  attestation token verification at its API boundary.
- No schema, envelope-format, or trait changes.

Estimated migration cost: ~2 weeks of integration work, assuming dstack-GPU
attestation primitives have stabilized.

### 3. Complete the Ironclaw extraction

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
