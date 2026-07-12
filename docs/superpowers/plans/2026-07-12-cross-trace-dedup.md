# Cross-Trace Dedup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Assign every gate decision to a duplicate-cluster over a token simhash + a bge-large embedding (via a separate dedup vector index), OR-matched cross-tenant, and persist `dedup_simhash`/`dedup_cluster_id`/`dedup_cluster_size` on `trace_gate_decisions` so `dup_pen = 1/dedup_cluster_size` — shadow-only, no settlement.

**Architecture:** A pure simhash function and a pure cluster-assignment function (signal-agnostic: it takes candidate clusters gathered from a cross-tenant simhash scan and from the dedup vector index, OR-matches, tie-breaks to the larger, or opens a singleton). Persistence and the two compute paths (inline at gate time + a batch `recluster-dedup` admin route) mirror credit-pipeline sub-project #1 (the credit-quality feature) exactly. The embedding side uses its OWN `UsearchVectorIndex` instance so novelty's index is untouched.

**Tech Stack:** Rust, PostgreSQL (postgres-only), usearch (`UsearchVectorIndex`), the gate enclave's `embed_chunk_mean_pooled`, axum, `async_trait` DB traits, tokio. No new dependencies.

## Global Constraints

- **Postgres-only.** `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins` and `... test -p trace-commons-server --no-run` clean; clippy with the repo allow-list (`-A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`) clean; `cargo fmt` applied.
- **Shadow-only.** `dup_pen`/dedup columns are computed and persisted but consumed by nothing that pays or gates. Do not modify `q`/credit_quality, perplexity, novelty, tail-fraction, gate status, credit, or the novelty vector index.
- **Isolation invariant.** Dedup writes touch ONLY `dedup_simhash`, `dedup_cluster_id`, `dedup_cluster_size`.
- **Separate index.** The dedup embedding index is its OWN `UsearchVectorIndex` instance at its own root path — never the novelty index.
- **Cross-tenant reads** go through the gate-driver reader pool with NO tenant GUC (like `list_gate_decisions_for_credit_scoring`). **Writes** are tenant-scoped via the tenant pool + `begin_trace_tenant_transaction`.
- **Hash-only audit/logging.** Counts, decision ids as existing gate code logs them, error hashes only. Never raw trace text, tokens, simhash-input content, or contributor identity.
- **Admin route reuses `require_admin`** (no new bearer gate); fails closed if the DB mirror (or, for the embedding path, the dedup index) is absent.
- **Migration V40** (`migrations/V40__trace_dedup.sql`), wired into the hand-rolled `run_migrations` in `crates/trace-commons-server/src/db/postgres.rs` (a migration file alone is inert — see sub-project #1's final-review lesson). V39 is the current head.
- **Versioned constants.** `DEDUP_CONSTANTS_V1` (`tau_e_micros`, `tau_hamming`, `version`) pinned in code; recluster is a versioned event.
- No emojis; short imperative commit subjects (no `feat:`/`fix:` prefix).

**Reference implementation to mirror throughout:** the credit-quality feature merged in PR #168. Its parts:
- pure fn + constants: `crates/trace-commons-server/src/credit_quality.rs`
- migration wiring: the V37/V39 blocks in `crates/trace-commons-server/src/db/postgres.rs` `run_migrations`
- tenant-scoped decision UPDATE: `update_trace_gate_decision_credit_quality` (trait in `src/trace_corpus_storage.rs`, pg in `src/db/trace_corpus_pg.rs`, double in `src/bin/trace_commons_ingest_internal/tests.rs`), real-pg isolation test in `tests/trace_corpus_pg_store.rs`
- cross-tenant enumeration: `list_gate_decisions_for_credit_scoring` (trait `src/db/mod.rs`, pg `src/db/postgres.rs`)
- inline write: in `evaluate_and_record_gate` (`src/bin/trace-commons-ingest.rs`)
- batch route: `score_credit_quality_handler`/`run_score_credit_quality_pass`/`score_credit_quality_one` + route registration `POST /v1/admin/score-credit-quality`

---

### Task 1: Token simhash (pure function)

**Files:**
- Create: `crates/trace-commons-server/src/dedup_simhash.rs`
- Modify: `crates/trace-commons-server/src/lib.rs` (add `pub mod dedup_simhash;` alphabetically)

**Interfaces:**
- Produces: `pub fn trace_simhash(canonical_text: &str) -> u64`; `pub fn hamming_distance(a: u64, b: u64) -> u32`.

- [ ] **Step 1: Write the failing tests**

Create `dedup_simhash.rs` with a `#[cfg(test)] mod tests`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic() {
        assert_eq!(trace_simhash("the quick brown fox jumps"), trace_simhash("the quick brown fox jumps"));
    }

    #[test]
    fn identical_text_zero_distance() {
        let t = "fn main() { let x = compute(); println!(\"{}\", x); }";
        assert_eq!(hamming_distance(trace_simhash(t), trace_simhash(t)), 0);
    }

    #[test]
    fn near_identical_small_distance() {
        // one token changed out of many -> small Hamming distance
        let a = "the agent debugged the parser and fixed the off by one error in the loop";
        let b = "the agent debugged the parser and fixed the off by one error in the block";
        assert!(hamming_distance(trace_simhash(a), trace_simhash(b)) <= 8,
            "near-identical texts should be close: {}", hamming_distance(trace_simhash(a), trace_simhash(b)));
    }

    #[test]
    fn distinctive_token_shim_still_close() {
        // A6: same content + a few injected nonce tokens -> still close (bulk tokens unchanged)
        let base = "the agent read the config file parsed the yaml and validated every required key in order";
        let shimmed = format!("{base} zqxnonce7731 vvblorpmarker9920");
        assert!(hamming_distance(trace_simhash(base), trace_simhash(&shimmed)) <= 10,
            "shim should stay close: {}", hamming_distance(trace_simhash(base), trace_simhash(&shimmed)));
    }

    #[test]
    fn unrelated_large_distance() {
        let a = "the agent debugged the parser and fixed the off by one error in the loop";
        let b = "quarterly revenue projections exceeded forecasts across every regional market segment";
        assert!(hamming_distance(trace_simhash(a), trace_simhash(b)) >= 18,
            "unrelated texts should be far: {}", hamming_distance(trace_simhash(a), trace_simhash(b)));
    }

    #[test]
    fn empty_is_zero() {
        assert_eq!(trace_simhash(""), 0);
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p trace-commons-server --lib dedup_simhash 2>&1 | tail -20`
Expected: FAIL to compile (functions undefined).

- [ ] **Step 3: Write the implementation**

Above the test module (uses only `std`; a stable FNV-1a hash so results are reproducible across builds — do NOT use `DefaultHasher`, which is not stable):

```rust
//! Deterministic 64-bit token simhash over a trace's canonical text, for
//! cross-trace duplicate clustering. Word-shingle features, FNV-1a hashed for
//! build-stable reproducibility. Pure: no I/O.

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Lowercase, split on non-alphanumeric, drop empties. Deterministic and
/// dependency-free — a simhash does not need a linguistic tokenizer.
fn tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_ascii_lowercase())
        .collect()
}

pub fn trace_simhash(canonical_text: &str) -> u64 {
    let toks = tokens(canonical_text);
    if toks.is_empty() {
        return 0;
    }
    // Features = overlapping 2-token shingles (fall back to unigrams for a
    // single token) so word-order matters and light rewords stay close.
    let mut features: Vec<u64> = Vec::new();
    if toks.len() == 1 {
        features.push(fnv1a_64(toks[0].as_bytes()));
    } else {
        for w in toks.windows(2) {
            features.push(fnv1a_64(format!("{} {}", w[0], w[1]).as_bytes()));
        }
    }
    let mut acc = [0i32; 64];
    for f in features {
        for (bit, slot) in acc.iter_mut().enumerate() {
            if (f >> bit) & 1 == 1 {
                *slot += 1;
            } else {
                *slot -= 1;
            }
        }
    }
    let mut sig: u64 = 0;
    for (bit, slot) in acc.iter().enumerate() {
        if *slot > 0 {
            sig |= 1u64 << bit;
        }
    }
    sig
}

pub fn hamming_distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}
```

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p trace-commons-server --lib dedup_simhash 2>&1 | tail -20`
Expected: PASS. If `near_identical_small_distance` / `unrelated_large_distance` land on the boundary, do NOT loosen an assertion to hide a real problem — re-run and confirm the values; the thresholds (≤8 near-identical, ≤10 shim, ≥18 unrelated) have margin for a 64-bit shingle simhash (observed ~7 and ~9 for the near cases, well below 18). Then `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins` clean and clippy (lib) clean.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add crates/trace-commons-server/src/dedup_simhash.rs crates/trace-commons-server/src/lib.rs
git commit -m "Add token simhash for cross-trace dedup"
```

---

### Task 2: Dedup constants + pure cluster-assignment logic

**Files:**
- Create: `crates/trace-commons-server/src/dedup_assign.rs`
- Modify: `crates/trace-commons-server/src/lib.rs` (add `pub mod dedup_assign;`)

**Interfaces:**
- Consumes: `crate::dedup_simhash::hamming_distance` (Task 1).
- Produces:
  - `pub struct DedupConstants { pub tau_e_micros: i64, pub tau_hamming: u32, pub version: i32 }`
  - `pub const DEDUP_CONSTANTS_V1: DedupConstants`
  - `pub struct ClusterCandidate { pub cluster_id: uuid::Uuid, pub size: i64, pub simhash: u64, pub embed_cosine_micros: Option<i64> }` (one per already-clustered decision the callers gathered from the simhash scan and/or the dedup index; `embed_cosine_micros` is `Some` when this candidate came from the embedding-index neighbor query, `None` when only the simhash scan surfaced it)
  - `pub enum ClusterAssignment { Existing(uuid::Uuid), New }`
  - `pub fn assign_cluster(new_simhash: u64, candidates: &[ClusterCandidate], k: &DedupConstants) -> ClusterAssignment`

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    const K: DedupConstants = DEDUP_CONSTANTS_V1;
    fn cand(id: Uuid, size: i64, simhash: u64, cos: Option<i64>) -> ClusterCandidate {
        ClusterCandidate { cluster_id: id, size, simhash, embed_cosine_micros: cos }
    }

    #[test]
    fn no_candidates_is_new_singleton() {
        assert_eq!(assign_cluster(42, &[], &K), ClusterAssignment::New);
    }

    #[test]
    fn simhash_within_threshold_joins() {
        let id = Uuid::from_u128(1);
        // identical simhash -> Hamming 0 <= tau_hamming
        let c = cand(id, 1, 42, None);
        assert_eq!(assign_cluster(42, &[c], &K), ClusterAssignment::Existing(id));
    }

    #[test]
    fn simhash_far_and_no_embedding_is_new() {
        let id = Uuid::from_u128(1);
        // Hamming distance >> tau_hamming, no embedding signal
        let c = cand(id, 1, u64::MAX, None);
        assert_eq!(assign_cluster(0, &[c], &K), ClusterAssignment::New);
    }

    #[test]
    fn embedding_within_threshold_joins_even_if_simhash_far() {
        // heavy paraphrase: simhash far, but embedding cosine distance below tau_e
        let id = Uuid::from_u128(2);
        let c = cand(id, 1, u64::MAX, Some(K.tau_e_micros - 1));
        assert_eq!(assign_cluster(0, &[c], &K), ClusterAssignment::Existing(id));
    }

    #[test]
    fn embedding_over_threshold_does_not_join_on_embedding_alone() {
        let id = Uuid::from_u128(2);
        let c = cand(id, 1, u64::MAX, Some(K.tau_e_micros + 1));
        assert_eq!(assign_cluster(0, &[c], &K), ClusterAssignment::New);
    }

    #[test]
    fn tie_breaks_to_larger_cluster() {
        // two clusters both match on simhash; join the larger
        let small = Uuid::from_u128(10);
        let large = Uuid::from_u128(20);
        let cands = [cand(small, 2, 42, None), cand(large, 9, 42, None)];
        assert_eq!(assign_cluster(42, &cands, &K), ClusterAssignment::Existing(large));
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p trace-commons-server --lib dedup_assign 2>&1 | tail -20`
Expected: FAIL to compile.

- [ ] **Step 3: Write the implementation**

```rust
//! Pure cluster-assignment logic for cross-trace dedup. Signal-agnostic: the
//! caller gathers candidate clusters from the cross-tenant simhash scan and/or
//! the dedup vector index and hands them here. OR-match on either signal; tie
//! -> larger cluster (deterministic); no match -> new singleton.

use crate::dedup_simhash::hamming_distance;
use uuid::Uuid;

#[derive(Debug, Clone, Copy)]
pub struct DedupConstants {
    /// Embedding cosine-DISTANCE threshold in micros (join when a candidate's
    /// cosine distance <= this). Calibrated in shadow; V1 is a starting value.
    pub tau_e_micros: i64,
    /// simhash Hamming-distance threshold (join when <= this).
    pub tau_hamming: u32,
    pub version: i32,
}

pub const DEDUP_CONSTANTS_V1: DedupConstants = DedupConstants {
    tau_e_micros: 150_000, // cosine distance 0.15
    tau_hamming: 3,
    version: 1,
};

#[derive(Debug, Clone, Copy)]
pub struct ClusterCandidate {
    pub cluster_id: Uuid,
    pub size: i64,
    pub simhash: u64,
    pub embed_cosine_micros: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClusterAssignment {
    Existing(Uuid),
    New,
}

pub fn assign_cluster(
    new_simhash: u64,
    candidates: &[ClusterCandidate],
    k: &DedupConstants,
) -> ClusterAssignment {
    // A candidate matches if EITHER signal is within threshold (OR semantics).
    let mut best: Option<(Uuid, i64)> = None; // (cluster_id, size)
    for c in candidates {
        let simhash_match = hamming_distance(new_simhash, c.simhash) <= k.tau_hamming;
        let embed_match = c
            .embed_cosine_micros
            .is_some_and(|d| d <= k.tau_e_micros);
        if simhash_match || embed_match {
            // tie-break: larger cluster wins; on equal size, lower uuid wins for determinism
            let take = match best {
                None => true,
                Some((bid, bsize)) => c.size > bsize || (c.size == bsize && c.cluster_id < bid),
            };
            if take {
                best = Some((c.cluster_id, c.size));
            }
        }
    }
    match best {
        Some((id, _)) => ClusterAssignment::Existing(id),
        None => ClusterAssignment::New,
    }
}
```

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p trace-commons-server --lib dedup_assign 2>&1 | tail -20`
Expected: PASS. Then `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins` and clippy (lib) clean.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add crates/trace-commons-server/src/dedup_assign.rs crates/trace-commons-server/src/lib.rs
git commit -m "Add pure cluster-assignment logic for dedup"
```

---

### Task 3: Migration V40 — dedup columns (wired into run_migrations)

**Files:**
- Create: `migrations/V40__trace_dedup.sql`
- Modify: `crates/trace-commons-server/src/db/postgres.rs` (`run_migrations`, add a V40 block after the V39 block)

**Interfaces:**
- Produces: nullable columns `dedup_simhash BIGINT`, `dedup_cluster_id UUID`, `dedup_cluster_size INTEGER` on `trace_gate_decisions`.

- [ ] **Step 1: Write the migration**

Read `migrations/V39__trace_credit_quality.sql` to match style. Create `migrations/V40__trace_dedup.sql`:

```sql
-- Cross-trace dedup: per-decision duplicate-cluster assignment (shadow mode).
-- dedup_simhash = 64-bit token simhash (stored as BIGINT; bit pattern, may be
-- negative when interpreted as signed). dedup_cluster_id = assigned cluster.
-- dedup_cluster_size = snapshot of the cluster's cross-tenant member count;
-- dup_pen = 1 / dedup_cluster_size. All nullable/backfillable. No RLS change
-- (columns on an already-RLS-forced table inherit the tenant predicate); no new
-- grants beyond the existing trace_gate_decisions grants.
ALTER TABLE trace_gate_decisions
    ADD COLUMN IF NOT EXISTS dedup_simhash BIGINT,
    ADD COLUMN IF NOT EXISTS dedup_cluster_id UUID,
    ADD COLUMN IF NOT EXISTS dedup_cluster_size INTEGER;
```

Note: simhash is a `u64`; Postgres BIGINT is signed `i64`. Store the bit pattern via `i64::from_ne_bytes(u64.to_ne_bytes())` (i.e. `as i64`) and read back with `as u64`. The Task 4 code owns this cast; the column just holds the 64-bit pattern.

- [ ] **Step 2: Wire it into run_migrations**

In `crates/trace-commons-server/src/db/postgres.rs`, immediately AFTER the V39 block (which ends just before the final `Ok(())` of `run_migrations`), add — mirroring the V39/V37 block exactly with `39`->`40`:

```rust
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&40_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V40__trace_dedup.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&40_i32, &"trace_dedup"],
                )
                .await?;
        }
```

- [ ] **Step 3: Verify wiring by drop-then-migrate (defeats false-green)**

Find the test DB connection var the pg tests use (grep `tests/trace_corpus_pg_store.rs` for the env var, e.g. `TRACE_COMMONS_TEST_DATABASE_URL`). Then:
```
psql "$URL" -c "ALTER TABLE trace_gate_decisions DROP COLUMN IF EXISTS dedup_simhash, DROP COLUMN IF EXISTS dedup_cluster_id, DROP COLUMN IF EXISTS dedup_cluster_size;"
psql "$URL" -c "DELETE FROM _trace_commons_migrations WHERE version = 40;"
```
Then run any pg test that calls `run_migrations` (e.g. the Task 4 test once it exists, or a credit_quality pg test now): it must recreate the columns. Confirm: `psql "$URL" -c "\d trace_gate_decisions" | grep dedup`. If you cannot reach the DB, report it — do not fake it.

- [ ] **Step 4: Commit**

```bash
git add migrations/V40__trace_dedup.sql crates/trace-commons-server/src/db/postgres.rs
git commit -m "Add dedup columns to trace_gate_decisions (V40) and wire into run_migrations"
```

---

### Task 4: DB methods — dedup UPDATE + cross-tenant signal enumeration

**Files:**
- Modify: `crates/trace-commons-server/src/trace_corpus_storage.rs` (add `update_trace_gate_decision_dedup` trait default, mirroring `update_trace_gate_decision_credit_quality`; add the `DedupSignalRow` struct next to `GateCreditInput`)
- Modify: `crates/trace-commons-server/src/db/trace_corpus_pg.rs` (pg impl of the UPDATE, mirroring the credit-quality UPDATE)
- Modify: `crates/trace-commons-server/src/db/mod.rs` (add `list_dedup_signals` trait default returning empty Vec, mirroring `list_gate_decisions_for_credit_scoring`)
- Modify: `crates/trace-commons-server/src/db/postgres.rs` (pg impl of `list_dedup_signals` via gate-driver pool)
- Modify: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs` (extend the in-memory double with a dedup side-map + accessor, mirroring the credit-quality side-map; add the in-memory isolation test)
- Test: `crates/trace-commons-server/tests/trace_corpus_pg_store.rs` (real-pg isolation test)

**Interfaces:**
- Produces:
  - `pub struct DedupSignalRow { pub tenant_id: String, pub decision_id: uuid::Uuid, pub dedup_cluster_id: Option<uuid::Uuid>, pub dedup_simhash: Option<i64> }` (in `trace_corpus_storage.rs`, next to `GateCreditInput`)
  - `async fn update_trace_gate_decision_dedup(&self, tenant_id: &str, decision_id: Uuid, dedup_simhash: i64, dedup_cluster_id: Uuid, dedup_cluster_size: i32) -> Result<(), DatabaseError>` (TraceCorpusStore trait)
  - `async fn list_dedup_signals(&self, limit: i64) -> Result<Vec<DedupSignalRow>, DatabaseError>` (Database trait) — cross-tenant, `ORDER BY decided_at ASC LIMIT $1`, gate-driver pool, no tenant GUC.
- Consumes: nothing from Tasks 1-2 directly (this is plumbing).

- [ ] **Step 1: Write the failing in-memory isolation test**

Mirror `update_trace_gate_decision_credit_quality_touches_only_credit_columns` in `tests.rs`. Extend the double with a `dedup: RwLock<HashMap<(String, Uuid), (i64, Uuid, i32)>>` side-map + an accessor `gate_decision_with_dedup_by_id(tenant_id, decision_id) -> Option<DecisionRowWithDedup>` (mirror the credit-quality side-map + `gate_decision_with_credit_quality_by_id` exactly). Test: seed a decision row, call `update_trace_gate_decision_dedup`, assert the three dedup values set AND ~15 base-row fields (perplexity/novelty/status/credit_quality/…) byte-identical.

```rust
#[tokio::test]
async fn update_trace_gate_decision_dedup_touches_only_dedup_columns() {
    // (same fixture setup as the credit-quality isolation test)
    let before = db.debug_get_decision(TENANT, DECISION_ID).await.unwrap();
    let cluster = Uuid::from_u128(7);
    db.update_trace_gate_decision_dedup(TENANT, DECISION_ID, 42i64, cluster, 3)
        .await
        .unwrap();
    let after = db.gate_decision_with_dedup_by_id(TENANT, DECISION_ID).unwrap();
    assert_eq!(after.dedup_simhash, Some(42));
    assert_eq!(after.dedup_cluster_id, Some(cluster));
    assert_eq!(after.dedup_cluster_size, Some(3));
    // isolation
    assert_eq!(after.base.perplexity_micros, before.perplexity_micros);
    assert_eq!(after.base.novelty_score_micros, before.novelty_score_micros);
    assert_eq!(after.base.credit_quality_micros, before.credit_quality_micros);
    assert_eq!(after.base.perplexity_passed, before.perplexity_passed);
    // ...(assert the rest of the non-dedup columns unchanged, as the credit-quality test does)
}
```
(Match the double's actual accessor shape; if credit-quality's side-map row is `DecisionRowWithCreditQuality`, model `DecisionRowWithDedup` the same way.)

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p trace-commons-server --bin trace-commons-ingest update_trace_gate_decision_dedup_touches_only_dedup_columns 2>&1 | tail -20`
Expected: FAIL to compile.

- [ ] **Step 3: Add the trait method + pg impl + struct**

Trait default in `trace_corpus_storage.rs` (mirror the credit-quality default's log-once warn + no-op):

```rust
async fn update_trace_gate_decision_dedup(
    &self,
    _tenant_id: &str,
    _decision_id: Uuid,
    _dedup_simhash: i64,
    _dedup_cluster_id: Uuid,
    _dedup_cluster_size: i32,
) -> Result<(), DatabaseError> {
    static WARNED: std::sync::Once = std::sync::Once::new();
    WARNED.call_once(|| {
        tracing::warn!("update_trace_gate_decision_dedup called on a backend without a real impl");
    });
    Ok(())
}
```

pg impl in `trace_corpus_pg.rs` (mirror the credit-quality UPDATE — exact PK scope, tenant pool, only the three dedup columns):

```rust
async fn update_trace_gate_decision_dedup(
    &self,
    tenant_id: &str,
    decision_id: Uuid,
    dedup_simhash: i64,
    dedup_cluster_id: Uuid,
    dedup_cluster_size: i32,
) -> Result<(), DatabaseError> {
    let mut client = self.trace_pool().get().await?;
    let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
    tx.execute(
        "UPDATE trace_gate_decisions
            SET dedup_simhash = $3,
                dedup_cluster_id = $4,
                dedup_cluster_size = $5
         WHERE tenant_id = $1 AND decision_id = $2",
        &[&tenant_id, &decision_id, &dedup_simhash, &dedup_cluster_id, &dedup_cluster_size],
    )
    .await
    .map_err(DatabaseError::Postgres)?;
    tx.commit().await.map_err(DatabaseError::Postgres)?;
    Ok(())
}
```

Add `DedupSignalRow` in `trace_corpus_storage.rs` next to `GateCreditInput`.

- [ ] **Step 4: Add the enumeration (trait default + pg impl)**

`db/mod.rs` trait default (mirror `list_gate_decisions_for_credit_scoring`):

```rust
async fn list_dedup_signals(
    &self,
    _limit: i64,
) -> Result<Vec<crate::trace_corpus_storage::DedupSignalRow>, DatabaseError> {
    Ok(Vec::new())
}
```

`db/postgres.rs` pg impl (gate-driver pool, no tenant GUC):

```rust
async fn list_dedup_signals(
    &self,
    limit: i64,
) -> Result<Vec<crate::trace_corpus_storage::DedupSignalRow>, DatabaseError> {
    let pool = self
        .gate_driver_pool
        .as_ref()
        .ok_or_else(|| DatabaseError::Pool("gate-driver pool not configured".to_string()))?;
    let client = pool.get().await.map_err(DatabaseError::from)?;
    let rows = client
        .query(
            "SELECT tenant_id, decision_id, dedup_cluster_id, dedup_simhash
             FROM trace_gate_decisions
             ORDER BY decided_at ASC
             LIMIT $1",
            &[&limit],
        )
        .await
        .map_err(DatabaseError::Postgres)?;
    Ok(rows
        .into_iter()
        .map(|row| crate::trace_corpus_storage::DedupSignalRow {
            tenant_id: row.get("tenant_id"),
            decision_id: row.get("decision_id"),
            dedup_cluster_id: row.get("dedup_cluster_id"),
            dedup_simhash: row.get("dedup_simhash"),
        })
        .collect())
}
```

- [ ] **Step 5: Implement the double + pass the in-memory test**

Extend the double with the dedup side-map, the `update_trace_gate_decision_dedup` override, and `gate_decision_with_dedup_by_id`. Run:
`cargo test -p trace-commons-server --bin trace-commons-ingest update_trace_gate_decision_dedup_touches_only_dedup_columns 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 6: Add the real-Postgres isolation test**

In `tests/trace_corpus_pg_store.rs`, mirror `pg_store_update_trace_gate_decision_credit_quality_touches_only_credit_columns`: seed a decision (via the existing `sample_gate_decision` helper), call `update_trace_gate_decision_dedup`, assert the three dedup columns changed and perplexity/novelty/status/credit_quality columns byte-identical.

Run: `cargo test -p trace-commons-server --test trace_corpus_pg_store dedup 2>&1 | tail -20` (local `trace_commons_test` DB; if unavailable report it). Expected: PASS. Then `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run` clean.

- [ ] **Step 7: Commit**

```bash
cargo fmt
git add -A
git commit -m "Add dedup decision UPDATE and cross-tenant signal enumeration"
```

---

### Task 5: Separate dedup vector index + trace embedding

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` (instantiate a second `UsearchVectorIndex` for dedup at its own root; add it to `AppState`; add a helper to embed a trace mean-pooled and query/insert the dedup index)

**Interfaces:**
- Consumes: `UsearchVectorIndex::try_new` (see the novelty index construction at `trace-commons-ingest.rs:4667`/`:4937`), `trace_commons_gate_enclave::embedder::embed_chunk_mean_pooled`.
- Produces: an `AppState` field `dedup_vector_index: Option<Arc<UsearchVectorIndex>>` and two helpers: `dedup_index_query(embedding: &[f32], k: usize) -> Vec<(Uuid, i64 /*cosine distance micros*/)>` and `dedup_index_insert(id: Uuid, embedding: &[f32])`. Both no-op/empty when the index is `None` (fail-open for the embedding signal; simhash still works).

- [ ] **Step 1: Read the novelty index construction and AppState**

Read `trace-commons-ingest.rs` around lines 4660-4940 (both `UsearchVectorIndex::try_new` sites) and the `AppState` struct definition. Note the exact constructor arg order (`try_new(root, dim, m, ef_construction, ef_search, …)` per `vector_index_usearch.rs`), the env vars (`TRACE_COMMONS_VECTOR_INDEX_*`), and how the novelty index is stored on `AppState`. Write down the real signatures.

- [ ] **Step 2: Write the failing test**

Add a unit test that constructs a temp dedup index (mirror `vector_index_usearch.rs`'s test ctor `UsearchVectorIndex::try_new(tmp.path(), dim, 16, 200, 50, 2, 32)`), inserts two vectors, and asserts `dedup_index_query` returns the near vector within a small cosine distance and ranks it first. Keep it in `tests.rs` or a module test near the helpers.

```rust
#[test]
fn dedup_index_query_finds_near_vector() {
    // build a temp UsearchVectorIndex, insert id_a with vec_a and id_b with an
    // orthogonal vec_b; query with a vector ~= vec_a; assert id_a returned with
    // cosine-distance micros < DEDUP_CONSTANTS_V1.tau_e_micros and ranked first.
}
```

- [ ] **Step 3: Run to verify it fails**

Run: `cargo test -p trace-commons-server --bin trace-commons-ingest dedup_index_query 2>&1 | tail -20`
Expected: FAIL to compile (helper undefined).

- [ ] **Step 4: Implement the index instance + helpers**

Instantiate the dedup index next to the novelty index construction, at a distinct root from a new env var `TRACE_COMMONS_DEDUP_VECTOR_INDEX_ROOT` (default: sibling dir of the novelty index root, e.g. `<novelty_root>/../dedup-index`), same `dim`/params as novelty. Store `Option<Arc<UsearchVectorIndex>>` on `AppState` (mirror the novelty index field). Implement:
- `dedup_index_query`: if index `None` -> `vec![]`; else search top-k, convert usearch key back to `Uuid` (`UsearchVectorIndex::uuid_to_key` inverse — see the round-trip in `vector_index_usearch.rs` tests), convert cosine similarity to cosine-distance micros (`((1.0 - sim) * 1e6).round() as i64`).
- `dedup_index_insert`: if `Some`, insert the vector under the decision's uuid key.
Both hash-only logging on error; both non-fatal.

- [ ] **Step 5: Run to verify it passes**

Run: `cargo test -p trace-commons-server --bin trace-commons-ingest dedup_index_query 2>&1 | tail -20`
Expected: PASS. Then `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins` clean; clippy clean.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add -A
git commit -m "Add separate dedup vector index and trace embedding helpers"
```

---

### Task 6: Inline-at-gate dedup assignment

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` (`evaluate_and_record_gate`, after the decision insert — reuse the same insertion point the inline credit-quality write uses)
- Test: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs`

**Interfaces:**
- Consumes: `dedup_simhash::trace_simhash` (T1), `dedup_assign::{assign_cluster, ClusterCandidate, DEDUP_CONSTANTS_V1}` (T2), `update_trace_gate_decision_dedup` + `list_dedup_signals` (T4), `dedup_index_query`/`dedup_index_insert` + `embed_chunk_mean_pooled` (T5), the just-inserted decision id + tenant + the canonical trace text used for scoring.

- [ ] **Step 1: Locate the canonical text + decision id at the insertion point**

Read `evaluate_and_record_gate`; find (a) the just-inserted `decision_id`, (b) `tenant_id`, (c) the canonical trace text (the text the chunker/scorer consumed). Write down the exact bindings (as sub-project #1's Task 5 did).

- [ ] **Step 2: Write the failing test**

In `tests.rs`, drive two gate evaluations with the same canonical text (in-memory double) and assert both decisions land in the SAME `dedup_cluster_id` with `dedup_cluster_size == 2` (via `gate_decision_with_dedup_by_id`), and a third with unrelated text gets a distinct cluster with size 1. Use the double's dedup accessor; the dedup index will be `None` in the test, so this exercises the simhash path (which is what makes the two identical texts cluster).

- [ ] **Step 3: Run to verify it fails**

Run: `cargo test -p trace-commons-server --bin trace-commons-ingest <new_test_name> 2>&1 | tail -20`
Expected: FAIL (dedup fields None / clusters differ).

- [ ] **Step 4: Implement the inline assignment**

After the decision insert (non-fatal, hash-only on error — mirror the inline credit-quality write):

```rust
// Shadow-mode cross-trace dedup (no settlement, no gating). Best-effort; a
// failure logs hash-only and never blocks the gate decision.
let simhash = crate::dedup_simhash::trace_simhash(canonical_text);
// gather candidates: cross-tenant simhash scan + embedding neighbors
let signals = db.list_dedup_signals(i64::MAX).await.unwrap_or_default();
let mut candidates: Vec<crate::dedup_assign::ClusterCandidate> = Vec::new();
// simhash candidates (only rows already assigned to a cluster)
// size per cluster computed from the signal snapshot:
let mut sizes: std::collections::HashMap<uuid::Uuid, i64> = std::collections::HashMap::new();
for s in &signals {
    if let Some(cid) = s.dedup_cluster_id { *sizes.entry(cid).or_insert(0) += 1; }
}
for s in &signals {
    if let (Some(cid), Some(sh)) = (s.dedup_cluster_id, s.dedup_simhash) {
        candidates.push(crate::dedup_assign::ClusterCandidate {
            cluster_id: cid,
            size: *sizes.get(&cid).unwrap_or(&0),
            simhash: sh as u64,
            embed_cosine_micros: None,
        });
    }
}
// embedding candidates (fail-open if index absent)
if let Ok(embedding) = trace_commons_gate_enclave::embedder::embed_chunk_mean_pooled(embedder, canonical_text) {
    for (neighbor_decision_id, cos_micros) in state.dedup_index_query(&embedding, 8) {
        if let Some(s) = signals.iter().find(|s| s.decision_id == neighbor_decision_id) {
            if let Some(cid) = s.dedup_cluster_id {
                candidates.push(crate::dedup_assign::ClusterCandidate {
                    cluster_id: cid,
                    size: *sizes.get(&cid).unwrap_or(&0),
                    simhash: s.dedup_simhash.unwrap_or(0) as u64,
                    embed_cosine_micros: Some(cos_micros),
                });
            }
        }
    }
    state.dedup_index_insert(decision_id, &embedding);
}
let assignment = crate::dedup_assign::assign_cluster(simhash, &candidates, &crate::dedup_assign::DEDUP_CONSTANTS_V1);
let cluster_id = match assignment {
    crate::dedup_assign::ClusterAssignment::Existing(id) => id,
    crate::dedup_assign::ClusterAssignment::New => uuid::Uuid::new_v4(),
};
let new_size = i32::try_from(sizes.get(&cluster_id).copied().unwrap_or(0) + 1).unwrap_or(i32::MAX);
if let Err(error) = db
    .update_trace_gate_decision_dedup(tenant_id, decision_id, simhash as i64, cluster_id, new_size)
    .await
{
    tracing::warn!(tenant_hash = %sha256_prefixed(tenant_id), error_hash = %safe_display_error_hash(&error), "shadow dedup inline write failed (non-fatal)");
}
```
Adapt `canonical_text`/`embedder`/`decision_id`/`tenant_id`/`state`/`db` to the real bindings. Note: this does NOT update existing cluster members' size snapshots — that is the batch route's job (Step in Task 7); inline only sets the new row's size. Add a one-line comment saying so.

- [ ] **Step 5: Run to verify it passes**

Run: `cargo test -p trace-commons-server --bin trace-commons-ingest <new_test_name> 2>&1 | tail -20`
Expected: PASS. Then full `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins` + `... test --no-run` clean; clippy clean.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add -A
git commit -m "Assign cross-trace dedup cluster inline at gate time"
```

---

### Task 7: Batch recluster admin route

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` (route + handler/pass/one + ack/summary/query types, mirroring the credit-quality batch route)
- Test: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs`

**Interfaces:**
- Consumes: `list_dedup_signals` (T4), `assign_cluster` (T2), `update_trace_gate_decision_dedup` (T4). Recompute is simhash-based over the enumerated snapshot (the batch pass does not decrypt/re-embed in v1 — document this; embedding-side recluster is a follow-up).

- [ ] **Step 1: Write the failing integration test**

Mirror `score_credit_quality_pass_...`. Seed several decisions with `dedup_simhash` already set (two identical simhashes across two tenants, one distinct), run the pass, assert the two identical ones share a `cluster_id` with `dedup_cluster_size == 2` (cross-tenant) and the distinct one has size 1; assert perplexity/novelty/credit_quality columns byte-identical before/after.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p trace-commons-server --bin trace-commons-ingest recluster_dedup 2>&1 | tail -20`
Expected: FAIL to compile.

- [ ] **Step 3: Implement types + pass + one + handler + route**

Mirror the credit-quality route exactly. `ReclusterDedupSummary { reclustered: u64, failed: u64 }`, `ReclusterDedupQuery { limit: Option<i64> }`, `ReclusterDedupAck { accepted: bool, limit: Option<i64> }`.

`run_recluster_dedup_pass`: enumerate via `list_dedup_signals(limit)`; recompute clusters over the snapshot — greedily, in `decided_at` order, using `assign_cluster` against the clusters formed so far (simhash-only candidates, `embed_cosine_micros: None`); track each cluster's running membership; then for every decision write `update_trace_gate_decision_dedup(tenant, decision_id, simhash, assigned_cluster_id, final_cluster_size)`. Compute final sizes after the full assignment sweep so every member gets the cluster's total size (not its position-in-sweep size). One failure skips+continues (hash-only warn), never aborts.

`recluster_dedup_handler`: authenticate + `require_admin`; fail closed `SERVICE_UNAVAILABLE` if `state.db_mirror.is_none()`; spawn the pass; return the ack. Register `POST /v1/admin/recluster-dedup` next to `score-credit-quality`.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p trace-commons-server --bin trace-commons-ingest recluster_dedup 2>&1 | tail -20`
Expected: PASS. Then the full gate: `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins` + `... test --no-run` clean; `cargo clippy -p trace-commons-server --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching` clean.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add -A
git commit -m "Add dedup recluster batch admin route"
```

---

## Post-implementation (operational, not a code task)

After merge + pilot deploy, run `POST /v1/admin/recluster-dedup` over the 349 decisions (signed admin token flow — see the pilot admin-token memory), then read the cluster-size and `dup_pen` distributions via the gate-driver role. Inspect: how many clusters, largest cluster, `dup_pen` spread — the operational answer to "how much duplication is in the corpus." Calibrate `tau_e_micros`/`tau_hamming` from what over/under-clusters, bump `DEDUP_CONSTANTS_V1` -> V2, re-run. No payout, no gating.
