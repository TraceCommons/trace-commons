# Cross-trace dedup — design

## Goal

Assign every gate decision to a duplicate-cluster and persist a per-decision
duplication penalty `dup_pen = 1 / dedup_cluster_size`, computed from
corpus-wide clustering over two signals (bge-large embedding + a token simhash).
`dup_pen` is the second multiplier in the credit pipeline (`raw = q · dup_pen ·
…`), and it closes the anti-gaming hole the shadow score cannot touch: semantic
duplication / paraphrase farms (perplexity is blind to rewording) and the
distinctive-token "shim" that defeats embedding-novelty. Shadow-only: `dup_pen`
is persisted/derivable but multiplied into nothing that pays until the
settlement sub-project.

This is sub-project #2 of the credit pipeline. Sub-project #1 (the shadow credit
quality score `q ∈ [0,1]`) shipped in PR #168. Remaining after this: #3
per-contributor accounting + concave caps, #4 reputation weighting, #5 delayed
settlement + clawback → NEAR on-chain cutover, #6 execution/replay grounding.

## Background: why two signals, and why cross-tenant

The 2026-07-12 credit-farming red-team showed the credit score's two signals are
both content-surface metrics that fail in the same direction. Dedup adds an
orthogonal defense: it credits a *cluster* of near-duplicate work once and splits
diminishing credit across its members (`dup_pen = 1/size`), so a farm that
resubmits N rewordings of one unit of work collects ~one unit of credit, not N.

- **simhash** (64-bit, over the canonical token stream) catches near-identical
  token sequences: literal duplication, light rewording, and the distinctive-
  token shim (A6) where injected nonce tokens shift the embedding but the bulk
  tokens are unchanged.
- **embedding** (bge-large, via a separate dedup vector index) catches semantic
  duplication where the tokens differ substantially but the meaning is the same
  (heavy paraphrase / translation, A7).

The clustering runs **cross-tenant/global**, following the same model the novelty
gate already uses (a global usearch index over the whole corpus — dedup gets its
own such index). This is what catches sybil — the same or similar content
resubmitted under different identities/tenants clusters together. No new privacy
boundary is crossed: novelty already compares content-derived representations
across tenants in a global index; dedup does the same with the same
representations.

## Non-goals

- No settlement, payout, or gating. `dup_pen` multiplies nothing that pays.
- No AST/structural code hashing (deferred to a follow-up; embedding + simhash is
  the v1 signal set).
- No cross-tenant cluster *registry* table (would fight forced RLS). Cluster
  membership is a per-decision column; cluster size is a cross-tenant count.
- No change to `q`/credit_quality, perplexity, novelty, tail-fraction, gate
  status, credit, or the novelty vector index (dedup uses its OWN separate
  vector index instance).
- No LSH banding in v1 (linear Hamming scan is fine at pilot scale; banding is
  the documented scale-out).

## Architecture

Each gate decision is assigned to a cluster and carries a snapshot of its
cluster's size. `dup_pen = 1 / dedup_cluster_size`.

**Trace-level representation:** a mean-pooled bge-large embedding (derived from
the chunk vectors the novelty gate already computes) added as one trace-level
entry to a **separate dedup vector index** — its own usearch index instance,
NOT the novelty index. Reusing the novelty index would pollute its
nearest-neighbor results with trace-level dedup vectors and silently change
novelty scoring; a distinct index reuses the usearch infrastructure and the
embedder while keeping novelty behavior untouched. Plus a 64-bit token simhash
over the full canonical token stream.

**Matching = OR semantics (anti-gaming stance):** a trace joins a cluster if it
is within threshold of that cluster's representative on **either** signal —
embedding cosine ≤ `τ_e` OR simhash Hamming ≤ `τ_h`. The two cover each other's
blind spots. On a tie between two candidate clusters, join the larger
(deterministic); on no match, open a new singleton cluster. False-merge risk
(over-clustering distinct work) is a threshold-calibration concern that shadow
mode exists to tune before it pays.

**Dual compute path, mirroring sub-project #1:**
- **Inline at gate time** — when a decision is recorded, compute its simhash
  (embedding already in hand), query the dedup vector index for embedding
  neighbors and scan the `dedup_simhash` column (cross-tenant, gate-driver reader) for simhash
  neighbors, OR-match to a cluster or open a singleton, and snapshot the size.
- **Batch admin route** `POST /v1/admin/recluster-dedup` — recompute memberships
  and size snapshots over the whole corpus; backfills existing decisions and
  re-runs after a threshold/version change. Mirrors #1's
  `POST /v1/admin/score-credit-quality` (admin credential reuse, hash-only ack,
  background task, idempotent, resumable, `?limit=N`).

## Persistence

**New columns on `trace_gate_decisions`** (RLS-scoped writes via the tenant pool;
cross-tenant reads via the gate-driver reader pool):
- `dedup_simhash BIGINT` — the trace's 64-bit token simhash.
- `dedup_cluster_id UUID` — the assigned cluster.
- `dedup_cluster_size INTEGER` — snapshot of the cluster's size at
  assignment/recluster; `dup_pen = 1 / dedup_cluster_size`.

No cluster registry table: cluster *membership* is `dedup_cluster_id` on the row,
and cluster *size* is the cross-tenant `COUNT(*)` of decisions sharing that
`cluster_id` (read via the gate-driver reader, snapshotted onto the row). This
keeps every new table-column on an already-RLS-forced table and adds no
cross-tenant registry.

Thresholds `τ_e` / `τ_h` and the signal set are pinned, versioned constants
(a `DEDUP_CONSTANTS_V1` with a `version` stamp, mirroring #1's
`CREDIT_QUALITY_CONSTANTS_V1`), calibrated on the corpus in shadow. A
`dedup_calibration_version` is stamped so a recluster is a deliberate, versioned
event.

**Cluster size is time-varying** — a trace's `dup_pen` tightens as later
duplicates arrive; the current snapshot is what a future settlement reads, and
the batch route keeps snapshots fresh. This is correct behavior: a farm's credit
should shrink as its duplicates accumulate.

## Config / gating

- Inline clustering is always on (shadow); it writes only the `dedup_*` columns.
- Batch route behind the admin credential (`require_admin`, no new gate); fails
  closed if the DB mirror or the vector index is absent.
- Hash-only audit: counts, decision ids as existing gate code logs them, error
  hashes. Never raw trace text, tokens, the simhash-input content, or contributor
  identity.

## Testing

**Unit — simhash + assignment logic** (pure, no DB/index):
- simhash deterministic; near-identical token streams → small Hamming distance,
  unrelated → large; canonicalization stable.
- assignment: OR-match joins on either signal; tie → join the larger cluster
  (deterministic); no match → new singleton.

**Anti-gaming guarantees as assertions** — synthetic decisions per dup attack:
- identical resubmission → same cluster, size 2, `dup_pen` 0.5.
- light reword / paraphrase → clusters together (simhash bulk-token overlap).
- distinctive-token shim (A6): same content + injected nonce tokens → still
  clusters via simhash despite embedding drift.
- genuinely distinct traces → separate clusters, `dup_pen` 1.0.
- sybil: the same trace under two tenants → one cross-tenant cluster, size 2
  (proves size counts across tenants).

**Column isolation** — a real-Postgres test proving the dedup write touches only
`dedup_*` columns and leaves `credit_quality`/perplexity/novelty/status
byte-identical before/after (mirrors #1). The embedding-index side is tested via
an injected neighbor-provider in unit tests and validated for real on the pilot.

**Distribution validation on the 349** — run the batch recluster; inspect cluster
count, size distribution, and `dup_pen` distribution. Confirms non-degeneracy and
answers the operational question: how much duplication is actually in the pilot
corpus? Plus batch idempotency: re-running yields stable clusters.

## Rollout

Ships with inline clustering active (shadow only) and the batch route behind the
admin credential. After deploy: run the batch recluster over the 349
now-27B-consistent decisions, inspect the cluster/size/`dup_pen` distributions,
calibrate `τ_e`/`τ_h`, bump to a V2 constant set, re-run. No payout, no gating.

## Residual risks (not covered here)

- False merges (distinct work clustered together) shrink legit credit; mitigated
  by shadow-mode threshold calibration, not eliminated.
- AST-level code duplication (structurally identical code with renamed
  identifiers that also diverges in tokens and embedding) is not caught until the
  deferred AST-hash follow-up.
- Collusion rings that deliberately keep submissions just outside both thresholds
  evade clustering; a graph-level anomaly detector (out of scope) is the eventual
  answer.
- Simhash linear scan is O(N) per assignment; fine at pilot scale, needs LSH
  banding before large-corpus scale.
