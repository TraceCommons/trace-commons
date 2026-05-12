# Private Vector System — Design

Date: 2026-05-11 (initial), 2026-05-12 (rephased)
Status: Phase A in-progress (cloud-KMS-backed on regular hardware); Phase B (dstack migration) deferred.
Owner: Trace Commons / Datasets + Auth-Keying lanes
Predecessors:
- `2026-05-11-cloud-trace-artifact-provider-design.md` (shipped `KmsKeyWrapper` trait + GCS backend)
- `2026-05-11-trace-kek-strategy-design.md` (chose cloud KMS for pilot, dstack-based KEK for the post-pilot trust upgrade)

> **Update 2026-05-12 — rephased.** The original spec assumed everything
> ran inside a single dstack-attested enclave. After scoping the
> dstack-GPU operational story against the pilot timeline, the work
> splits into two phases. Phase A builds the **same components on
> regular GPU hardware** (real perplexity model, real embedder, real
> usearch-backed vector index, no sealed snapshots, no in-process
> attestation) with cloud KMS as the KEK. Phase B does the trust
> upgrade — move the binary inside an attested dstack enclave, swap KEK
> impl, re-wrap DEKs. The trait surface, schema, envelope format, and
> gate-decision row shape are unchanged across the two phases — only
> the binary's hosting and the KEK impl differ. Sections below that
> describe enclave-resident specifics (sealed snapshots, in-enclave
> attestation, in-enclave key derivation) apply to Phase B only;
> Phase A treats those as future work and runs the same components
> as ordinary services.

## Goal

Ship the dstack-resident attested workload that holds the production KEK, runs a
prefill-perplexity credit gate, and operates a private vector index over
redacted trace embeddings. The workload emits a new hash-only credit event
kind (`novelty_utility`) when a submitted trace passes both the perplexity
gate and the dedup-novelty gate. All trace plaintext stays inside the enclave;
operators can prove the gate logic ran without seeing trace content.

This is the single slice that turns `trace-commons-server` from "shippable in dev"
into "deployable with a constrained-operator trust model." It replaces both
the open KEK strategy item and the open private-vector item from the roadmap;
they collapse into one trust boundary.

## Non-goals

- Homomorphic-encrypted or MPC vector ops. Privacy is provided by the TEE,
  not by cryptographic computation. If TEE assumption is later replaced or
  augmented, this becomes a separate slice.
- ZK proofs of gate execution. The dstack attestation token plus the
  hash-only audit row are the integrity story for v1. ZK-of-gate is a
  natural future addition; out of scope here.
- Cross-enclave federation. One enclave per deployment.
- Cross-region replication of the sealed index. Operator concern.
- Replacement of `ranking_utility`. The new credit kind is parallel, not a
  substitute.
- A learned perplexity classifier. Single floor + tail metric only in v1.
- TEE platforms other than dstack. Nitro / Confidential Space land later if
  ever, as separate `KmsKeyWrapper` impls.

## Trust model

The operator is constrained, not trusted. Concretely:

| Surface | Operator can read? |
|---------|--------------------|
| Encrypted trace bytes in GCS | yes (ciphertext) |
| Wrapped DEKs in PostgreSQL | yes (wrapped form) |
| Audit rows, credit events | yes (hash-only) |
| dstack enclave RAM | no |
| Embedder weights inside enclave | no (loaded from sealed bundle at boot) |
| Prefill model weights inside enclave | no (same) |
| Vector index inside enclave | no (in RAM); sealed snapshots only |
| `KmsKeyWrapper::unwrap_dek` results | no (returned only inside enclave) |
| Network traffic to/from enclave | yes (ciphertext + authenticated metadata) |
| dstack attestation chain | yes (designed to be auditable) |

The operator can see *that* the enclave ran the gate, by verifying the
attestation token and the audit row. They cannot see *what* the gate ran on.

## Architecture

```
Outside enclave                        Inside enclave (dstack workload)
+--------------------------+           +-------------------------------------+
| ingest handler           |  POST     |                                     |
| (axum, plaintext         | --------> | TraceGateService                    |
| trace via attested TLS)  |  /gate    |   1. KEK unwrap (DstackKekWrapper)  |
+--------------------------+           |   2. Decrypt ciphertext             |
                                       |   3. Embed (BGE / gte-large)        |
+--------------------------+           |   4. Perplexity prefill (base LLM)  |
| vector worker            |  GET      |   5. Vector search (usearch)        |
| (settlement, retention,  | --------> |   6. Decide pass/fail               |
|  revocation)             |  /status  |   7. Emit hash-only response        |
+--------------------------+           |        - gate_version               |
                                       |        - perplexity, tail_metric    |
+--------------------------+           |        - novelty_score              |
| credit settlement        |           |        - novelty_utility evidence   |
| (existing pipeline)      |  reads    |          (hash-only attestation)    |
|                          | <-------- |   8. Emit dstack attestation token  |
+--------------------------+           +-------------------------------------+
                                                |
                                                | sealed snapshot
                                                v
                                       +--------------------+
                                       | sealed index file  |
                                       | (encrypted under   |
                                       |  enclave-derived   |
                                       |  sealing key)      |
                                       +--------------------+
```

The enclave is the only place where plaintext, embeddings, and the
vector index coexist. Everything else sees only the gate's hash-only
outputs.

## Components inside the enclave

### 1. `DstackKekWrapper` (`KmsKeyWrapper` impl)

Same trait surface as today's `LocalMasterKeyWrapper`. Differences:

- Wrapping key derived from `dstack` sealing primitives (TDX measurement-bound).
  The exact derivation is platform-specific; the trait does not care.
- `is_production_trust_boundary()` returns `true`.
- `safe_status()` reports `kind = "dstack_kek"`, a `key_ref_hash` derived from
  the enclave measurement (the same value other components use as their
  audit identifier), and the production-trust-boundary flag.
- `wrap_dek` / `unwrap_dek` use the same `KekContext` binding as the local
  impl. The wrapped format includes a discriminator (`wrapper_kind`) so v2
  envelopes wrapped by `DstackKekWrapper` cannot be unwrapped by the local
  wrapper, and vice versa. (The schema already supports this — the existing
  envelope reader cross-checks `wrapper_kind`.)

When this trait return type changes from `[u8; 32]` to `Zeroizing<[u8; 32]>`
(deferred from the cloud artifact provider work), this is the spec that
ships the change. All existing impls update at the same commit.

### 2. Prefill perplexity gate

A configured base model running prefill-only — never generation.

- **Model:** Llama-3.1-8B-Instruct or equivalent (operator-configurable at
  enclave build time, baked into the sealed bundle). Chosen for: well-studied
  perplexity behavior, fits comfortably on a single confidential H100
  alongside the embedder, open weights so the operator can rebuild and
  re-attest deterministically.
- **Procedure:** Tokenize the decrypted trace, run one forward pass, recover
  per-token logprobs. No generation.
- **Aggregate metric:** mean per-token negative log-likelihood (NLL) over the
  trace, normalized to perplexity (`exp(mean_nll)`).
- **Tail metric:** fraction of tokens with logprob below a configured tail
  cutoff. Catches "uniformly easy with one rare span."
- **Gate decision:**
  - `perplexity >= floor_perplexity[policy_version]`
  - AND `tail_fraction >= floor_tail_fraction[policy_version]`
- **Floors:** configured per `gate_policy_version`, sealed into the bundle.
  Changing floors requires a new policy version. The active policy version
  is reported in `safe_status()`.

Hash-only outputs only:
- `gate_policy_version` (string)
- `perplexity_micros` (`u64` — perplexity * 1e6, fixed-point to keep audit
  rows free of float weirdness)
- `tail_fraction_micros` (`u64`)
- `passed` (bool)

The actual logprob vector and per-token detail never leave the enclave.

### 3. Local embedder

- **Model:** BGE-large-en-v1.5 or gte-large, with matryoshka support
  preferred. ~300-400M params, fits comfortably alongside the prefill model
  on a confidential H100.
- **Output:** matryoshka embeddings at three nested dims (e.g., 256 / 512 /
  1024). The index stores the full-dim vector; coarse-search uses the
  truncated vector for speed.
- **Hashing:** the operator-visible `embedding_evidence_hash` is a SHA-256
  of `(model_id || policy_version || quantized_embedding_bytes)`. Used to
  bind credit events to embedding evidence in the audit trail.

### 4. Private vector index

- **Library:** `usearch` (Apache-2.0, single-file C++ with Rust bindings,
  HNSW-based, SIMD-fast, supports both insert and delete) for v1.
  `instant-distance` is the fallback if `usearch` proves operationally
  awkward inside the enclave.
- **Storage shape:** per-tenant index, keyed by `tenant_storage_ref`. Index
  ID is `sha256(tenant_storage_ref || gate_policy_version || embedder_model_id)`
  so a policy or model bump produces a fresh index — no cross-version
  contamination.
- **Snapshots:** every N inserts (configurable, default 1000), the index is
  serialized, encrypted under a sealing key derived from the enclave
  measurement, and written to a service-owned object store path (the
  existing `TraceArtifactStore` infrastructure handles this — sealed index
  is an `artifact_kind = SealedVectorIndex`).
- **Recovery:** on enclave restart, load the latest sealed snapshot, replay
  vector inserts from the audit trail since the snapshot timestamp.
- **Revocation / retention:** on revocation, the enclave deletes the entry
  by `vector_entry_id`. The next snapshot persists the deletion. If a
  snapshot from before the deletion is ever loaded, the replay phase replays
  the same deletion event from the audit trail.

### 5. Novelty gate

After embedder + perplexity, the trace gets a novelty score:

- Query the index for top-`k` nearest neighbors (default `k=5`) by cosine
  similarity.
- Compute `novelty_score = 1 - max_cosine_similarity_over_top_k`. Range
  `[0, 1]`; 1 means farthest from anything in the corpus.
- **Gate:** `novelty_score >= floor_novelty[policy_version]`.

Hash-only outputs only:
- `novelty_score_micros` (`u64`)
- `nearest_neighbor_hash` (hash of the closest neighbor's vector id; never
  raw)
- `passed` (bool)

### 6. `TraceGateService` orchestration

Wraps all four into one synchronous request handler. Inputs: encrypted trace
bytes + `TenantCtx` + wrapped DEK. Outputs: hash-only `GateDecision`.

```rust
pub struct GateDecision {
    pub gate_policy_version: String,
    pub gate_version_hash: String,         // sha256(model_ids + policy + thresholds)
    pub perplexity_micros: u64,
    pub tail_fraction_micros: u64,
    pub perplexity_passed: bool,
    pub novelty_score_micros: u64,
    pub nearest_neighbor_hash: String,
    pub novelty_passed: bool,
    pub embedding_evidence_hash: String,
    pub attestation_token: String,         // dstack attestation, base64
}
```

If both gates pass, the enclave appends a `novelty_utility` credit-event
draft and inserts the embedding into the vector index. If either fails, the
enclave still emits the `GateDecision` (for auditability) but does not insert
or credit. The audit-row contains the same `GateDecision` minus the
attestation token (which goes into a separate attestation log).

## Components outside the enclave

### Modified: `vector_worker_*` route handlers

Today the vector worker computes deterministic similarity in-process. After
this spec, the worker calls the enclave's `/gate` endpoint, receives the
hash-only `GateDecision`, verifies the dstack attestation token, and persists
the decision.

The worker:
1. Pages new `SubmissionRecord` rows.
2. Loads the wrapped DEK + GCS object reference.
3. Streams the ciphertext + DEK + `TenantCtx` to the enclave's `/gate`
   endpoint over an attested TLS channel (dstack supports attested TLS via
   its own primitives — channel setup happens once per worker run).
4. On response, verifies the attestation token (RA via the dstack verifier
   or a pinned verifier key).
5. Writes a new `gate_decision` row (audit-grade, hash-only).
6. If `perplexity_passed && novelty_passed`, emits a `novelty_utility`
   credit event into the existing settlement-source pipeline.
7. If revoked or retention-expired, emits a delete-from-index request to
   the enclave's `/index/delete` endpoint and tombstones the vector entry.

The worker never sees plaintext. Its only sensitive surface is the wrapped
DEK, which it forwards but cannot unwrap.

### Modified: credit settlement

New `CreditEventKind::NoveltyUtility`. Parallel to `RankingUtility`. The
existing settlement pipeline (utility attestation → dry-run → central-issuer
approval → live settlement → NEAR outbox) is unchanged structurally —
settlement batches just select by `event_kind`.

Settlement source-list hashes for `novelty_utility` include
`gate_version_hash` in the canonicalization so a gate-version change is
visible in the central-issuer approval flow.

### Modified: revocation propagation

Existing revocation worker gains a step: after marking a submission revoked,
call the enclave's `/index/delete` endpoint with the `vector_entry_id`.
Failure here surfaces as a `revocation_propagation` failure (existing audit
shape).

### New: attestation log

A new table or audit-row class storing dstack attestation tokens received
from the enclave, keyed by `(gate_decision_id, attestation_chain_hash)`.
The operator can verify the chain offline. The tokens themselves are not
required for correctness — they're integrity evidence.

## Schema changes

### `credit_events`: new kind + gate_version

```sql
ALTER TABLE trace_credit_events ADD COLUMN gate_version_hash TEXT;
ALTER TABLE trace_credit_events ADD COLUMN gate_policy_version TEXT;
```

`event_kind` gains the variant `'novelty_utility'` (existing enum / column
type; check whether it's a Rust-side enum hashed into a TEXT column or a
PostgreSQL enum — match the existing pattern).

`gate_version_hash` and `gate_policy_version` are NULL for legacy event
kinds. Required (NOT NULL) for `novelty_utility`.

### `trace_vector_entries`: minor

Existing table. Add columns:

```sql
ALTER TABLE trace_vector_entries
  ADD COLUMN gate_policy_version TEXT NOT NULL DEFAULT 'legacy_deterministic',
  ADD COLUMN embedder_model_id TEXT NOT NULL DEFAULT 'legacy_deterministic',
  ADD COLUMN attestation_chain_hash TEXT;
```

`attestation_chain_hash` is NULL for rows written by the legacy deterministic
similarity worker; required for rows written by the enclave.

### New: `trace_gate_decisions`

```sql
CREATE TABLE trace_gate_decisions (
    decision_id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    submission_id UUID NOT NULL,
    gate_policy_version TEXT NOT NULL,
    gate_version_hash TEXT NOT NULL,
    perplexity_micros BIGINT NOT NULL,
    tail_fraction_micros BIGINT NOT NULL,
    perplexity_passed BOOLEAN NOT NULL,
    novelty_score_micros BIGINT NOT NULL,
    nearest_neighbor_hash TEXT NOT NULL,
    novelty_passed BOOLEAN NOT NULL,
    embedding_evidence_hash TEXT NOT NULL,
    attestation_chain_hash TEXT NOT NULL,
    decided_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
-- RLS forced as on every other Trace Commons table.
```

This is the audit-grade record. The credit event references it via
`embedding_evidence_hash` and `gate_version_hash`.

## Policy versioning and calibration

Each `gate_policy_version` is a sealed bundle inside the enclave containing:
- Configured perplexity floor + tail-fraction floor + tail logprob cutoff
- Configured novelty score floor + index top-k
- Configured embedder model id + prefill model id

A new `gate_policy_version` is a redeploy of the enclave with a new sealed
bundle. The new version's `gate_version_hash` differs from the old; the
audit trail records which version gated which decision.

**Calibration:** before promoting a new policy version, run it in dry-run
mode against a held-out set of historical traces. Compare pass-rate, mean
perplexity, mean novelty score. The central-issuer approves the new policy
version by recording its `gate_version_hash` into the existing
`TRACE_COMMONS_CREDIT_SETTLEMENT_ALLOWED_POLICY_VERSIONS` allowlist (or a
parallel one if we want separate governance — see open questions).

Settlement refuses `novelty_utility` events whose `gate_version_hash` is
not on the allowlist.

## Credit-event lifecycle

Below uses the recommended grandfather-settled policy:

1. **Mint** (enclave): gate passes → enclave emits `novelty_utility` event
   draft with `gate_version_hash` stamp.
2. **Cooling-off** (existing pipeline): event lives in the settlement-source
   pool. If the gate version is rolled back during this window, the event
   is reversed via the existing revocation propagation path.
3. **Central-issuer approval** (operator): issuer reviews the dry-run for a
   bounded source-list, approves the canonical source-list hash. New
   `gate_version_hash` values must be allowlisted before approval.
4. **Live settlement** (existing pipeline): batches settle approved events;
   NEAR outbox emits receipt calls.
5. **Post-settlement** (immutable): settled credit stays. Subsequent
   gate-version rollbacks do not claw back. New traces are evaluated under
   the new gate; old credit remains tied to its original `gate_version_hash`.

## Wiring into existing code

| Existing surface | Change |
|------------------|--------|
| `KmsKeyWrapper` trait | `unwrap_dek` return type changes to `Zeroizing<[u8; 32]>` (deferred from cloud artifact provider work). New impl: `DstackKekWrapper`. |
| `vector_worker_run` route | Calls enclave instead of computing deterministic similarity. |
| `vector_worker_status` route | Reports enclave readiness + last attestation hash. |
| Revocation worker | Calls enclave `/index/delete` after marking revoked. |
| Credit settlement | New `CreditEventKind::NoveltyUtility`. Settlement batches select by kind. |
| `config-status` | Reports `gate.policy_version`, `gate.version_hash`, `gate.is_production_trust_boundary` (always true when configured). No model ids, no thresholds, no perplexity floors. |
| `TRACE_COMMONS_KEK_REQUIRE_PRODUCTION_TRUST_BOUNDARY` startup gate | Satisfied once `DstackKekWrapper` is wired. |
| `TRACE_COMMONS_VECTOR_WORKER_*` env (existing) | Gains `TRACE_COMMONS_GATE_ENCLAVE_ENDPOINT`, `TRACE_COMMONS_GATE_ATTESTATION_VERIFIER`. |

## Tests and verification

- **Unit (outside enclave):** mock `TraceGateService` returning canned
  decisions; assert credit events, audit rows, and revocation calls all
  thread the correct values.
- **Unit (inside enclave):** the enclave code itself is a separate crate
  (probably `crates/trace-commons-gate-enclave`). Standard Rust unit tests for
  the perplexity-aggregation math, the novelty-score math, and the
  index-snapshot serialization round-trip.
- **Integration (mock enclave):** an `InMemoryGateService` impl that
  produces deterministic gate decisions from a config — runs everything
  except the actual TEE. Useful for CI without TDX hardware.
- **Integration (real enclave):** opt-in, gated by env var, runs against a
  real dstack instance. Verifies attestation tokens, sealed-snapshot round-
  trip, and end-to-end gate decisions for a fixture corpus.
- **Calibration test:** a fixture set of traces tagged "should-pass" and
  "should-fail" runs through the dry-run pipeline; assert pass-rates within
  expected bands.

Hash-only verification gates (extending the existing list):

- **Gate.** Every `novelty_utility` credit event carries a `gate_version_hash`
  matching an allowlisted version. Events without a matching attestation
  chain hash in `trace_gate_decisions` are refused at settlement.
- **Attestation.** Every `trace_gate_decisions` row carries an attestation
  chain hash that verifies against the configured dstack verifier.

## Rollout

This is one long slice. Suggested ordering:

1. **Land schema changes** (additive, behind feature flag). Tests for the
   new tables, audit rows, credit-event kind.
2. **Land `DstackKekWrapper` skeleton** as a new module gated by feature
   `gate-enclave`. Wire `unwrap_dek -> Zeroizing<[u8; 32]>` trait change.
   Build a stub that returns hardcoded responses; tests verify the wiring.
3. **Land `trace-commons-gate-enclave` crate** as a separate binary. Inside-
   enclave logic: model loading from sealed bundle, perplexity, embedder,
   index, gate decision. Tests pass on dev hardware with mock models.
4. **Land `InMemoryGateService`** for outside-enclave CI tests.
5. **Land the worker route changes:** vector_worker now calls the enclave
   (real or in-memory) and writes `trace_gate_decisions` / `novelty_utility`
   events.
6. **Land calibration tooling:** dry-run mode, source-list hashing, central-
   issuer-approval allowlist for gate versions.
7. **Wire `DstackKekWrapper` end-to-end** with real attestation. Build
   reproducibility for the sealed bundle. First-version calibration runs
   against a fixture set.
8. **Update README + roadmap.** This roadmap item becomes "shipped."

Each step is its own PR. Step 7 is the gate where production startup becomes
satisfiable; everything before that ships behind feature flags without
changing default behavior.

## Open questions

These need answers before implementation starts, in priority order:

1. **Separate allowlist for `novelty_utility` policy versions, or share
   with the existing settlement allowlist?** Sharing keeps the
   central-issuer-approval shape uniform; separating gives operations the
   ability to allowlist gate-version rolls without re-approving settlement
   policies. Recommendation: share for v1, separate if it becomes painful.

2. **GPU memory budget.** Llama-3.1-8B in bf16 is ~16GB, BGE-large is ~1GB,
   `usearch` index is small in RAM. Confidential H100 has 80GB usable.
   Should fit; confirm against the actual confidential-compute reserved
   overhead.

3. **Attested TLS between worker and enclave** — does dstack provide this
   directly, or does the worker have to wrap a regular TLS connection with
   an attestation handshake? The worker side needs to refuse to send DEKs
   if attestation fails. Confirm against current dstack documentation.

4. **Sealed-bundle reproducibility.** Operators need to rebuild the enclave
   binary deterministically to re-attest after a model swap. Standard Rust
   builds aren't deterministic out of the box. Plan: use a pinned
   `rust-toolchain` + `--frozen` cargo + a documented `nix`-flake or
   container build that the operator runs offline.

5. **What happens to `legacy_deterministic` vector entries after rollout?**
   Options: keep them, retire them, re-embed them. Recommendation: keep
   them with their `legacy_deterministic` policy version stamp; they are
   read-only as far as the new gate is concerned (the index ID partition
   keeps them from interfering with new entries).

6. **Per-tenant gate policy version, or deployment-wide?** Recommendation:
   deployment-wide for v1. Per-tenant adds calibration complexity and a
   per-tenant settlement allowlist surface that nobody is asking for.

## Out of scope

- **ZK proofs of gate execution.** Future-spec material. The dstack
  attestation token is the integrity story for v1.
- **MPC / homomorphic vector ops.** As above.
- **Cross-enclave federation.** One enclave per deployment.
- **A learned perplexity gate.** Single floor + tail metric only. Door
  open to swap in a learned classifier later if calibration data
  accumulates and proves the simple gate is leaving signal on the table.
- **Replacing `ranking_utility`.** They are parallel kinds.
- **Multiple TEE platforms simultaneously.** dstack first; Nitro /
  Confidential Space land as separate `KmsKeyWrapper` impls if ever needed.
- **Streaming gate decisions** (real-time as the trace uploads). The
  perplexity model needs the full token sequence to compute the aggregate;
  there's no useful "partial" gate decision. Always batch.

## Cost estimate

Honest rough estimate, assuming the implementer is familiar with dstack and
the inside-enclave model-loading story:

- Schema + `DstackKekWrapper` skeleton + worker wiring: 1 week
- `trace-commons-gate-enclave` crate (perplexity + embedder + index + gate
  decision logic): 2-3 weeks
- Attestation, sealed-snapshot serialization, attested-TLS to worker:
  1-2 weeks
- Calibration tooling + central-issuer-approval integration: 1 week
- End-to-end testing + deployment rehearsal on a real dstack host:
  1-2 weeks

**Total: ~6-9 weeks for a serious implementation.** This is the dominant
work item on the roadmap.

## What this spec does not commit to

- A specific perplexity model or embedder version (only their class).
- A specific dstack host configuration.
- Specific perplexity / tail / novelty floors (those are calibrated, not
  designed).
- The exact attested-TLS handshake shape (depends on current dstack
  primitives — confirm during step 7).
