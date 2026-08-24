# Vector Index — Design (Phase A4)

Date: 2026-05-13
Status: Draft (pre-implementation)
Owner: Trace Commons / Datasets lane
Predecessor: `2026-05-11-private-vector-system-design.md` (rephased; this spec
implements its `VectorIndex` slot for Phase A)
Siblings:
- `2026-05-12-perplexity-scorer-design.md` (A2)
- `2026-05-13-embedder-design.md` (A3)

## Goal

Replace `MockVectorIndex` in `trace-commons-gate-enclave` with a real
implementation that holds per-tenant HNSW indexes over redacted-trace
embeddings, supports insert / nearest / delete, and persists to disk
across restarts. Drives the novelty half of the gate decision.

Phase A target: regular GPU host. Phase B (dstack): same impl, plus
sealed snapshots so the on-disk file can survive enclave restarts
without compromising the trust boundary. The sealing layer is
explicitly Phase B work; v1 ships with regular at-rest disk encryption
(operator's filesystem responsibility).

## Decisions baked in

| Decision | Value |
|----------|-------|
| Library | **`usearch`** (unum-cloud, 2.25.2 May 2026, HNSW, single-file C++ core via `cxx`) |
| Distance metric | **Cosine** (over unit-normalized vectors → equivalent to dot product) |
| Persistence | **One file per tenant** under a configured root directory |
| Index dim | **1024** (matches BGE-large from A3; configurable for matryoshka-truncated variants) |
| `M` (HNSW out-degree) | **16** (default; good quality/space trade-off) |
| `ef_construction` | **200** |
| `ef_search` | **50** |
| Cargo feature | **`local-gpu-models`** (shared with A2 / A3 — single feature) |

## Update (2026-05-14)

This spec remains current. The A2.3 mistralrs migration
(`2026-05-13-mistralrs-migration-design.md`) and the A2.2 candle
arch-dispatch work (`2026-05-13-bakeoff-arch-dispatch-design.md`) only
touched the perplexity-scorer path; usearch + HNSW are unchanged. The
A2.5 gate-floor recalibration
(`2026-05-14-gate-floor-recalibration-design.md`) ships the novelty
floor at `500000` (cosine novelty 0.5) as the **primary active gate**
at pilot launch — the perplexity and tail-fraction floors ship at 0.
This makes the vector-index path load-bearing for credit decisions
from day one rather than a secondary signal alongside perplexity.

**Why these:**

- `usearch` is the lightest HNSW library in the Rust ecosystem.
  Single-file C++ core via `cxx` keeps the dependency footprint small
  (4 direct deps vs `instant-distance`'s pure-Rust larger surface).
  Healthy release cadence (May 2026), Ash Vardanian / unum-cloud
  maintainership.
- Cosine over unit-normalized vectors maps cleanly to BGE's output
  shape and keeps the math interpretable: `novelty_score = 1 -
  max_cosine_similarity` per the rephased private-vector spec.
- One-file-per-tenant persistence is the simplest shape that
  preserves RLS-style tenant isolation at the filesystem level. The
  alternative — one index with tenant_id as a filter key — adds
  complexity without operational benefit at Phase A scale.

## Non-goals

- IVF-PQ or other quantized indexes. Phase A scale (thousands to tens
  of thousands of vectors per tenant) doesn't need them. HNSW alone
  is fast enough.
- Distributed / sharded vector storage. One file, one process.
- Approximate-vs-exact mode switching. usearch supports both; we use
  approximate (HNSW) only.
- Real-time index rebuilds. Inserts and deletes happen incrementally;
  full rebuilds are a maintenance operation, not online.
- Cross-tenant deduplication. Tenant isolation is load-bearing.
- Sealed snapshots. Phase B work — Phase A relies on operator-managed
  at-rest disk encryption (LUKS, GCP CMEK on the persistent volume,
  etc.).
- WAL / crash-recovery beyond what usearch ships natively. If a tenant
  index file is corrupted, the orchestrator rebuilds from the audit
  trail (insert events live in `trace_gate_decisions` with their
  `vector_entry_id`).

## Architecture

```
                  EnclaveGateOrchestrator
                  +--------------------+
                  | evaluate(plaintext, tenant_storage_ref)
                  +---+------+---------+
                      |      |
              embed   |      |  query_then_insert
                      v      v
                  +----------+--------+
                  | UsearchVectorIndex|
                  | - per-tenant Index handles, lazily loaded |
                  | - LRU cache of open handles               |
                  | - persistence: disk file per tenant       |
                  +-------------------+
                                      |
                                      v
                  +-------------------+--------------+
                  | <root>/<tenant_storage_ref_hash>.usearch
                  +----------------------------------+
```

Per-tenant index is opened lazily on first use, then held in a
size-bounded LRU cache (default 32 hot tenants). Eviction does a
synchronous flush before closing.

## `UsearchVectorIndex`

New impl of the existing `VectorIndex` trait in
`crates/trace-commons-gate-enclave/src/vector_index.rs`, gated by
`#[cfg(feature = "local-gpu-models")]`.

### Type sketch

```rust
#[cfg(feature = "local-gpu-models")]
pub struct UsearchVectorIndex {
    root_dir: PathBuf,
    dim: usize,                              // 1024 for BGE-large
    metric: usearch::MetricKind,             // Cosine
    hnsw_m: usize,                            // 16
    ef_construction: usize,                   // 200
    ef_search: usize,                         // 50
    open_indexes: Mutex<lru::LruCache<String, Arc<Mutex<usearch::Index>>>>,
    max_open: usize,                          // 32
}

#[cfg(feature = "local-gpu-models")]
impl VectorIndex for UsearchVectorIndex {
    fn insert(&self, entry_id: Uuid, tenant_storage_ref: &str, embedding: &[f32]) -> anyhow::Result<()>;
    fn nearest(&self, tenant_storage_ref: &str, embedding: &[f32], k: usize) -> anyhow::Result<Vec<NearestNeighbor>>;
    fn delete(&self, entry_id: Uuid) -> anyhow::Result<bool>;
}
```

`Mutex<usearch::Index>` because usearch's `Index` is not natively
`Sync` for concurrent writes (concurrent reads are supported but the
API is awkward to compose with concurrent inserts). One mutex per
tenant index — cross-tenant operations don't block each other.

`delete(entry_id)` ignores `tenant_storage_ref` and instead requires
that the orchestrator pass it through — refactor needed: extend the
trait to take a tenant ref on delete. The current trait shape (PR #12)
does not include tenant in delete because the mock index uses a global
keyspace. Real index needs per-tenant routing.

### Tenant-file naming

```rust
fn tenant_file_path(&self, tenant_storage_ref: &str) -> PathBuf {
    let hash = sha256_text_hex(tenant_storage_ref);
    self.root_dir.join(format!("{}.usearch", hash))
}
```

Hash to prevent raw `tenant_storage_ref` (e.g., `tenant_sha256:<hex>`)
from appearing as filesystem paths. Even though those refs are
themselves hashes, double-hashing keeps consistency with the rest of
the codebase's hash-only operational surface.

### insert / nearest / delete

**insert:**
1. Resolve or load the per-tenant index handle.
2. Build the usearch key from the UUID: `u64` derived from the
   high 8 bytes of the UUID (collision-safe at our scale).
3. Add the embedding to the index.
4. Mark the index dirty; schedule a flush.

**nearest:**
1. Resolve or load the handle.
2. Run `index.search(embedding, k)` with `ef_search`.
3. For each match, look up the original UUID from the in-index
   metadata. Return a `Vec<NearestNeighbor>`.

**delete:**
1. Resolve the handle.
2. Call `index.remove(key)`.
3. Returns `Ok(true)` if the key existed, `Ok(false)` if not.
4. Mark dirty; flush.

### Persistence and flush strategy

- Open: `usearch::Index::load_from_file` lazily on first access. If
  no file exists, construct a new empty index.
- Flush: after every N inserts/deletes (default 32) or on a periodic
  timer (every 60 s). The flush is a synchronous `index.save_to_file`
  call inside the per-tenant mutex. **Both legs are load-bearing.** The
  count leg alone leaves a deployment with few tenants and modest
  traffic — where neither `flush_every` nor LRU eviction is reached
  between restarts — with a corpus that exists only in process memory.
  Drop-time flushing does not rescue it: SIGTERM terminates without
  unwinding, so `Drop` never runs. `trace-commons-ingest` therefore also
  handles SIGTERM and flushes explicitly before exiting.
- Close: on LRU eviction, flush + drop. No background thread; the
  evicting call pays the flush cost.
- Recovery: if `load_from_file` fails (corrupted file), the
  orchestrator's startup hook can be told to skip and rebuild. The
  rebuild path is `for each gate_decision in trace_gate_decisions
  where vector_entry_id IS NOT NULL: re-insert`. v1 ships with this
  rebuild as a manual operator command, not an automatic startup
  step. Document it.

### Failure modes

| Situation | Behavior |
|-----------|----------|
| `root_dir` not writable | Constructor fails with `VectorIndexInit: root dir not writable` |
| `load_from_file` fails | `anyhow::bail!("VectorIndexLoadFailed: <tenant_storage_ref_hash>")` propagating up. Caller fails the gate evaluation. Operator runs the rebuild command. |
| `save_to_file` fails | Log a hash-only warning; keep the in-memory index. Next flush attempt may succeed; if not, eviction will lose recent inserts. Acceptable — the audit trail is the durable record; the index is the cache. |
| Wrong dim on insert | `anyhow::bail!("VectorIndexDimMismatch: expected <N>, got <M>")` |
| Disk full | Save fails as above. Operator concern. |
| Concurrent insert and search to the same tenant | Serialized by the per-tenant mutex. Acceptable latency hit at Phase A scale. |

### Hash-only logging

Tenant storage refs are hashed before logging. Index sizes (number of
vectors per tenant) are safe operator metrics. The embedding vectors
themselves never log.

## Configuration

| Env | Default | Notes |
|-----|---------|-------|
| `TRACE_COMMONS_VECTOR_INDEX_ROOT` | `/var/lib/trace-commons-vector-index` | Filesystem root for per-tenant index files |
| `TRACE_COMMONS_VECTOR_INDEX_DIM` | `1024` | Must match A3's embedder output dim |
| `TRACE_COMMONS_VECTOR_INDEX_MAX_OPEN` | `32` | LRU bound for hot tenant indexes |
| `TRACE_COMMONS_VECTOR_INDEX_FLUSH_EVERY` | `32` | Inserts/deletes between disk flushes |
| `TRACE_COMMONS_VECTOR_INDEX_FLUSH_INTERVAL_SECONDS` | `60` | Periodic-flush cadence (the timer leg of the flush strategy above); `0` disables it |
| `TRACE_COMMONS_VECTOR_INDEX_HNSW_M` | `16` | HNSW out-degree |
| `TRACE_COMMONS_VECTOR_INDEX_EF_CONSTRUCTION` | `200` | HNSW build-quality knob |
| `TRACE_COMMONS_VECTOR_INDEX_EF_SEARCH` | `50` | HNSW recall/speed knob |

None of the HNSW parameters participate in `gate_version_hash` — they
affect index quality but not the decision semantics. Operators tuning
recall/speed shouldn't invalidate existing credit.

## Trait shape changes

The current `VectorIndex::delete(entry_id) -> bool` (from PR #12) is
incomplete for a real per-tenant impl — there's no tenant routing in
the signature. Two options:

- **Option A:** Extend the trait to take `tenant_storage_ref` on
  delete. Update the orchestrator + `EnclaveGateService` + the
  revocation hook (when it lands in A6) to thread the tenant ref
  through.
- **Option B:** Store the tenant ref alongside `entry_id` somewhere
  (e.g., in an external lookup map maintained by the orchestrator).
  Lookup before delete.

**Recommendation: Option A.** Cleaner contract, smaller code surface.
The signature change is small. Update the `MockVectorIndex` (currently
keyed by a global map) to also take tenant ref — it can keep its
in-memory `BTreeMap<(String, Uuid), Vec<f32>>` shape.

This trait change is part of A4's PR.

## Build feature

Shared with A2 / A3:

```toml
[features]
local-gpu-models = [
    # ...A2 candle deps...
    # ...A3 fastembed deps...
    "dep:usearch",
    "dep:lru",
]

[dependencies]
usearch = { version = "2", optional = true }
lru = { version = "0.16", optional = true }
```

Pin specific versions at implementation time.

**Hard dependency-policy gate:** `usearch` and `lru` are not on
`~/.claude/approved-dependencies.md`. Disclose:
- `usearch` 2.25.2 (2026-05-02), unum-cloud, 4 direct deps including
  `cxx` for C++ FFI
- `lru` is a widely-used pure-Rust LRU cache (small dep, well-vetted)
- usearch's C++ core ships in-tree; build requires a C++ toolchain
  (likely already present from `ring` and other crates)

Get explicit approval before `cargo add`.

## Memory budget

| Component | Footprint |
|-----------|-----------|
| Per-tenant index (10k vectors × 1024 f32) | ~40 MB |
| 32 hot tenant indexes | ~1.3 GB |
| HNSW graph overhead (~16 edges × 8 bytes × 10k) | ~5 MB per tenant, ~150 MB total |
| **Vector index subtotal** | ~1.5 GB |

Comfortably fits alongside A2's perplexity model + A3's embedder.

If a tenant's corpus grows past ~100k vectors per tenant, single-file
HNSW becomes slow; that's the point at which we'd consider IVF-PQ
sharding. Out of scope for Phase A.

## Latency budget

usearch HNSW search at 10k vectors with `ef_search=50`: ~1-3 ms per
query on commodity CPU (the index is CPU-resident even though the
embedder is on GPU). Insert: ~5-10 ms.

Total contribution to gate latency: trivial (single-digit ms).
Dominated by A2's perplexity prefill (~1-2 s).

## Testing

### Unit tests (no external dep)

The mock `MockVectorIndex` shipped in PR #10 stays for orchestrator
tests. No new unit tests required for the trait surface.

For the real impl, add unit tests inside `trace-commons-gate-enclave`:

- Insert then search: assert the inserted vector is its own nearest
  neighbor with similarity 1.0.
- Insert N vectors, search for one of them, assert it's in the top-k
  with similarity 1.0 and others are not.
- Delete then search: assert removed entries don't appear.
- Persistence: insert, drop the index, reload from file, search,
  assert results match.
- Tenant isolation: two tenants with the same vector — search in
  tenant A's index should not return tenant B's entry. (This is the
  load-bearing test for the one-file-per-tenant design.)
- Dim mismatch: insert wrong-dim vector → expected error.

### Integration tests

usearch's tests run on CPU and are fast enough to be unit tests, so
there's no separate integration suite. The H100 GPU is not needed for
the vector index itself.

## Migration to Phase B

Move the same code path inside the dstack enclave. The persistent
on-disk files become sealed snapshots:

- New `SealedVectorIndexStore` wrapper that wraps usearch save/load
  with enclave sealing (encrypt with an enclave-derived key on save,
  decrypt on load).
- The sealing key is enclave-measurement-bound, so the file is
  unreadable outside the attested binary.
- Trait surface unchanged.

This is Phase B work, not Phase A.

## Open questions

1. **HNSW parameter defaults.** `M=16`, `ef_construction=200`,
   `ef_search=50` are reasonable defaults but should be tuned per
   deployment based on corpus size and recall requirements. Document
   the tuning procedure.

2. **Index file format stability across usearch upgrades.** usearch's
   on-disk format may change between major versions. Pin the version
   strictly; on upgrade, document a rebuild step.

3. **Rebuild from audit trail.** v1 ships with manual rebuild only.
   At what corpus size does automatic startup-rebuild become
   acceptable? Decision: ship manual; add automatic when the first
   real index loss happens.

4. **Concurrent insert + search.** Per-tenant mutex serializes them.
   For Phase A scale (one gate worker per deployment), this is fine.
   If we ever run multiple gate workers against the same persistence
   root, we need either filesystem-level locking or a single
   orchestrator process. Decision: one process per deployment in
   Phase A; flag for review at Phase A → B transition.

5. **`delete` requires a trait signature change** (see Trait shape
   changes section above). Confirm Option A is the path.

## Cost estimate

| Item | Estimate |
|------|----------|
| Disclose `usearch` + `lru` deps, get approval | <1 day |
| Trait signature change + mock update | <1 day |
| `UsearchVectorIndex` impl | 2-3 days |
| Unit tests (incl. tenant isolation) | 1 day |
| Rebuild-from-audit-trail operator command | 1 day |
| Documentation | <1 day |
| **Total** | **~5-7 days of focused work** |

Comparable to A3, larger than A3 because of the persistence layer and
the trait change.

## What this spec does not commit to

- A specific usearch version (pinned at implementation time)
- A specific embedding dim (operator-configurable; default 1024)
- Distributed / multi-process index management
- Sealed snapshots (Phase B)
- Automatic rebuild on corruption (manual operator command in v1)
