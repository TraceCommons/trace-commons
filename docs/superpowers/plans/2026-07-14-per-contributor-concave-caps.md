# Per-contributor Concave Caps Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a shadow-mode per-decision `contributor_factor` that caps how much credit a single contributor identity can extract per 7-day epoch, via a saturating concave function of their in-epoch cumulative `R = Σ(q·dup_pen)`.

**Architecture:** A pure math module computes the marginal cap factor. A new migration adds four nullable columns to `trace_gate_decisions`. A cross-tenant gate-driver enumeration joins decisions to submissions for contributor identity; a single forward pass per `(auth_principal_ref, epoch)` computes and persists factors via a tenant-scoped UPDATE touching only the new columns. A batch admin route triggers it. Batch-only in v1 (no inline gate path).

**Tech Stack:** Rust, axum, tokio-postgres/deadpool, PostgreSQL with forced RLS. Mirrors the cross-trace dedup slice (PR #169) and credit-quality slice (PR #168) patterns exactly.

## Global Constraints

- **Shadow-only.** `contributor_factor` is persisted/derivable and multiplies nothing that pays or gates. Do not touch credit, settlement, or gate status.
- **PostgreSQL-only.** No libsql. A single `cargo check -p trace-commons-server` per feature set.
- **Verify with `RUSTFLAGS="-D warnings"`** for both `cargo check -p trace-commons-server --bins` and `cargo test -p trace-commons-server --no-run`. Also run the CI clippy allow-list. Plain `cargo check` hides what CI catches.
- **Hash-only audit.** Never log raw trace text, contributor identity in the clear, or join inputs. Use `sha256_prefixed(...)` for tenant/decision ids, `safe_display_error_hash(...)` for errors — exactly as `run_recluster_dedup_pass` does.
- **Isolation invariant.** Every write is a tenant-scoped UPDATE via `begin_trace_tenant_transaction`, exact PK `(tenant_id, decision_id)`, touching ONLY the `contributor_*` columns. Cross-tenant reads go through the `gate_driver_pool` with no tenant GUC.
- **`run_migrations` is hand-rolled.** A migration file is inert until wired into `run_migrations` in `db/postgres.rs` with an `include_str!` + version guard + a `_trace_commons_migrations` insert. Prove recreation by dropping the columns + the migrations row and re-running.
- **No emojis. Short imperative commit subjects** (no `feat:`/`fix:` prefixes).
- **Constants are pinned + versioned:** `CONTRIBUTOR_CAP_CONSTANTS_V1 { k_micros: 25_000_000, epoch_days: 7, version: 1 }`. Bumping any value bumps `version`.

---

### Task 1: Pure contributor-cap math module

**Files:**
- Create: `crates/trace-commons-server/src/contributor_cap.rs`
- Modify: `crates/trace-commons-server/src/lib.rs` (add `pub mod contributor_cap;` beside `pub mod credit_quality;`)

**Interfaces:**
- Produces:
  - `pub struct ContributorCapConstants { pub k_micros: i64, pub epoch_days: i64, pub version: i32 }`
  - `pub const CONTRIBUTOR_CAP_CONSTANTS_V1: ContributorCapConstants`
  - `pub fn epoch_index(decided_at_unix_secs: i64, epoch_days: i64) -> i64`
  - `pub fn increment_micros(credit_quality_micros: Option<i64>, dedup_cluster_size: Option<i32>) -> i64`
  - `pub fn contributor_factor_micros(r_before_micros: i64, r_micros: i64, k: &ContributorCapConstants) -> i64`

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    const K: ContributorCapConstants = CONTRIBUTOR_CAP_CONSTANTS_V1;
    fn m(x: f64) -> i64 { (x * 1_000_000.0).round() as i64 }

    #[test]
    fn epoch_buckets_are_seven_day_windows() {
        let day = 86_400;
        // Two timestamps 6 days apart share an epoch; 8 days apart do not,
        // when aligned to bucket boundaries.
        assert_eq!(epoch_index(0, 7), 0);
        assert_eq!(epoch_index(6 * day, 7), 0);
        assert_eq!(epoch_index(7 * day, 7), 1);
        assert_eq!(epoch_index(15 * day, 7), 2);
    }

    #[test]
    fn increment_applies_dup_pen() {
        // r = q * (1/size). q=0.8, size=4 -> 0.2
        assert_eq!(increment_micros(Some(m(0.8)), Some(4)), m(0.2));
        // NULL size -> dup_pen 1; NULL q -> 0
        assert_eq!(increment_micros(Some(m(0.8)), None), m(0.8));
        assert_eq!(increment_micros(None, Some(4)), 0);
    }

    #[test]
    fn first_trace_of_epoch_has_full_factor() {
        // R_before = 0 -> marginal factor ~ 1.0 for a small increment.
        let f = contributor_factor_micros(0, m(0.1), &K);
        assert!(f > 990_000, "expected near-1.0, got {f}");
    }

    #[test]
    fn factor_decays_as_cumulative_rises() {
        let early = contributor_factor_micros(0, m(0.1), &K);
        let mid = contributor_factor_micros(m(20.0), m(0.1), &K);
        let late = contributor_factor_micros(m(60.0), m(0.1), &K);
        assert!(early > mid && mid > late, "monotone decay: {early} {mid} {late}");
    }

    #[test]
    fn zero_increment_uses_derivative_limit() {
        // r == 0 -> exp(-R_before/K). At R_before = K, that's exp(-1) ~ 0.3679.
        let f = contributor_factor_micros(K.k_micros, 0, &K);
        assert!((f - 367_879).abs() <= 200, "exp(-1) limit, got {f}");
    }

    #[test]
    fn epoch_total_effective_is_bounded_by_k() {
        // Telescoping: sum of (r * factor) over an epoch == effective(R_final) <= K.
        // Flood 400 increments of 0.2 each (R_final = 80, well past K=25).
        let mut r_before = 0i64;
        let mut total_effective = 0.0f64;
        for _ in 0..400 {
            let r = m(0.2);
            let factor = contributor_factor_micros(r_before, r, &K) as f64 / 1_000_000.0;
            total_effective += (r as f64 / 1_000_000.0) * factor;
            r_before += r;
        }
        let k = K.k_micros as f64 / 1_000_000.0;
        assert!(total_effective <= k + 1e-6, "total {total_effective} must be <= K {k}");
        // And it should be close to K (saturated), not far below.
        assert!(total_effective > k * 0.99, "expected saturation near K, got {total_effective}");
    }

    #[test]
    fn deterministic_and_bounded() {
        let a = contributor_factor_micros(m(13.0), m(0.15), &K);
        let b = contributor_factor_micros(m(13.0), m(0.15), &K);
        assert_eq!(a, b);
        assert!((0..=1_000_000).contains(&a));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p trace-commons-server --lib contributor_cap`
Expected: FAIL to compile (module/functions not defined).

- [ ] **Step 3: Write the implementation**

```rust
//! Pure, deterministic per-decision contributor-cap factor. A saturating
//! concave function of a contributor's in-epoch cumulative raw credit
//! `R = sum(q * dup_pen)` bounds how much any one identity can earn per epoch:
//! `effective(R) = K * (1 - exp(-R/K))`, asymptoting to `K`. Each decision's
//! `contributor_factor` is the MARGINAL effective-per-raw at its point in the
//! running total, decaying from 1.0 (first trace of the epoch) toward 0.
//! Shadow-only: nothing here settles or pays. Epochs are fixed 7-day buckets.

#[derive(Debug, Clone, Copy)]
pub struct ContributorCapConstants {
    /// Per-epoch asymptote K * 1e6 (max total effective credit one identity
    /// can earn in a single epoch, regardless of how many traces it submits).
    pub k_micros: i64,
    /// Epoch length in days; `R` resets at each epoch boundary.
    pub epoch_days: i64,
    pub version: i32,
}

pub const CONTRIBUTOR_CAP_CONSTANTS_V1: ContributorCapConstants = ContributorCapConstants {
    k_micros: 25_000_000, // K = 25.0 per epoch; calibration seed, tune on backfill
    epoch_days: 7,
    version: 1,
};

/// Deterministic epoch bucket of a decision's `decided_at` (unix seconds).
/// `floor(secs / (epoch_days * 86400))` — globally consistent, no genesis anchor.
pub fn epoch_index(decided_at_unix_secs: i64, epoch_days: i64) -> i64 {
    let epoch_secs = epoch_days.max(1) * 86_400;
    decided_at_unix_secs.div_euclid(epoch_secs)
}

/// This decision's raw pipeline increment `r = q * dup_pen`, in micros.
/// `dup_pen = 1 / dedup_cluster_size` (NULL size -> 1); NULL q -> 0. Because
/// `q_micros` is already micros and `dup_pen` is a fraction, `q_micros / size`
/// stays on the micros scale.
pub fn increment_micros(credit_quality_micros: Option<i64>, dedup_cluster_size: Option<i32>) -> i64 {
    let q = credit_quality_micros.unwrap_or(0).max(0);
    let size = dedup_cluster_size.unwrap_or(1).max(1) as i64;
    q / size
}

/// Marginal cap factor (* 1e6, clamped to [0,1e6]) for a decision with
/// increment `r_micros` at prior in-epoch cumulative `r_before_micros`:
/// `(effective(R_before + r) - effective(R_before)) / r` for `r > 0`, else the
/// `r -> 0` derivative limit `exp(-R_before / K)`.
pub fn contributor_factor_micros(
    r_before_micros: i64,
    r_micros: i64,
    k: &ContributorCapConstants,
) -> i64 {
    let kf = k.k_micros.max(1) as f64 / 1_000_000.0;
    let rb = r_before_micros.max(0) as f64 / 1_000_000.0;
    let r = r_micros.max(0) as f64 / 1_000_000.0;
    let factor = if r <= 0.0 {
        (-rb / kf).exp()
    } else {
        let eff = |x: f64| kf * (1.0 - (-x / kf).exp());
        (eff(rb + r) - eff(rb)) / r
    };
    (factor.clamp(0.0, 1.0) * 1_000_000.0).round() as i64
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p trace-commons-server --lib contributor_cap`
Expected: PASS (7 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-server/src/contributor_cap.rs crates/trace-commons-server/src/lib.rs
git commit -m "Add pure per-contributor concave cap math"
```

---

### Task 2: Migration V41 columns + run_migrations wiring

**Files:**
- Create: `migrations/V41__trace_contributor_cap.sql`
- Modify: `crates/trace-commons-server/src/db/postgres.rs` (add a V41 block in `run_migrations`, mirroring the V40 block; find it by grepping `name = "trace_dedup"` / the V40 `include_str!`)

**Interfaces:**
- Produces: four nullable columns on `trace_gate_decisions`: `contributor_factor_micros INTEGER`, `contributor_cumulative_raw_micros BIGINT`, `contributor_cap_epoch BIGINT`, `contributor_cap_version INTEGER`.

- [ ] **Step 1: Write the migration**

`migrations/V41__trace_contributor_cap.sql`:
```sql
-- Per-contributor concave cap (credit pipeline sub-project #3). Shadow-only
-- per-decision snapshot: the marginal cap factor, the in-epoch cumulative raw
-- credit R it was computed against, the epoch bucket, and the calibration
-- version. All nullable; written only by the recompute-contributor-caps pass.
ALTER TABLE trace_gate_decisions
    ADD COLUMN IF NOT EXISTS contributor_factor_micros INTEGER,
    ADD COLUMN IF NOT EXISTS contributor_cumulative_raw_micros BIGINT,
    ADD COLUMN IF NOT EXISTS contributor_cap_epoch BIGINT,
    ADD COLUMN IF NOT EXISTS contributor_cap_version INTEGER;
```

- [ ] **Step 2: Find and read the V40 wiring block**

Run: `grep -n "trace_dedup\|V40\|40" crates/trace-commons-server/src/db/postgres.rs | head`
Read the V40 block in `run_migrations` — it is the exact template: a `run_single_migration(... , 40, "trace_dedup", include_str!("../../../../migrations/V40__trace_dedup.sql"))` style call (use the ACTUAL helper name and `include_str!` path shape found in that block; match it verbatim).

- [ ] **Step 3: Add the V41 block**

Immediately after the V40 block, add the analogous V41 call: version `41`, name `"trace_contributor_cap"`, `include_str!` pointing at `V41__trace_contributor_cap.sql` with the same relative-path shape as the V40 line. Preserve ordering (V41 after V40).

- [ ] **Step 4: Compile**

Run: `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins`
Expected: clean.

- [ ] **Step 5: Prove recreation (drop-then-migrate)**

If a local/pilot PostgreSQL with the `trace_commons_test` DB is reachable, run:
```bash
psql "$TEST_DB_URL" -c "ALTER TABLE trace_gate_decisions
  DROP COLUMN IF EXISTS contributor_factor_micros,
  DROP COLUMN IF EXISTS contributor_cumulative_raw_micros,
  DROP COLUMN IF EXISTS contributor_cap_epoch,
  DROP COLUMN IF EXISTS contributor_cap_version;
  DELETE FROM _trace_commons_migrations WHERE version = 41;"
cargo test -p trace-commons-server --test trace_corpus_pg_store -- --nocapture   # runs run_migrations
psql "$TEST_DB_URL" -c "\d trace_gate_decisions" | grep contributor_
```
Expected: the four columns are recreated by `run_migrations`. If no PostgreSQL is reachable, state that explicitly in the task report and rely on the wiring inspection (the V41 block matching the proven V40 shape) — do not claim the DB proof was run if it was not.

- [ ] **Step 6: Commit**

```bash
git add migrations/V41__trace_contributor_cap.sql crates/trace-commons-server/src/db/postgres.rs
git commit -m "Add contributor-cap columns (V41) and wire into run_migrations"
```

---

### Task 3: Cross-tenant enumeration + tenant-scoped UPDATE helpers

**Files:**
- Modify: `crates/trace-commons-server/src/trace_corpus_storage.rs` (add `ContributorCapSignalRow` struct + two trait methods with default no-op/err impls, mirroring `DedupSignalRow` + `update_trace_gate_decision_dedup`)
- Modify: `crates/trace-commons-server/src/db/postgres.rs` (impl `list_contributor_cap_signals` on the gate-driver pool)
- Modify: `crates/trace-commons-server/src/db/trace_corpus_pg.rs` (impl `update_trace_gate_decision_contributor_cap` on the tenant pool)
- Modify: `crates/trace-commons-server/src/db/mod.rs` if the trait methods are declared there (match where `list_dedup_signals` / `update_trace_gate_decision_dedup` are declared — put the new methods in the SAME trait, same place)

**Interfaces:**
- Consumes: `epoch_index`, `increment_micros` are NOT used here (this task is pure I/O); the enumeration returns raw fields the pass (Task 4) feeds into the math.
- Produces:
  - `pub struct ContributorCapSignalRow { pub tenant_id: String, pub decision_id: Uuid, pub auth_principal_ref: String, pub decided_at: DateTime<Utc>, pub credit_quality_micros: Option<i64>, pub dedup_cluster_size: Option<i32> }`
  - `async fn list_contributor_cap_signals(&self, limit: i64) -> Result<Vec<ContributorCapSignalRow>, DatabaseError>` — gate-driver pool, cross-tenant, joins `trace_gate_decisions` to `trace_submissions`, ordered `auth_principal_ref, decided_at ASC` so the pass can group and forward-accumulate.
  - `async fn update_trace_gate_decision_contributor_cap(&self, tenant_id: &str, decision_id: Uuid, factor_micros: i32, cumulative_raw_micros: i64, epoch: i64, version: i32) -> Result<(), DatabaseError>` — tenant pool, updates ONLY the four `contributor_*` columns.

- [ ] **Step 1: Read the exact patterns to mirror**

Read `list_dedup_signals` in `db/postgres.rs` (~line 3754) and `update_trace_gate_decision_dedup` in `db/trace_corpus_pg.rs` (~line 5582), and the `DedupSignalRow` struct + trait-default methods in `trace_corpus_storage.rs` (~line 1851). Also read `list_submissions_needing_gate_decision` (`db/postgres.rs` ~3619) — it proves `trace_submissions` is gate-driver-readable cross-tenant and shows the `s.tenant_id`/`s.submission_id` join keys. The contributor identity column on `trace_submissions` is `auth_principal_ref`.

- [ ] **Step 2: Add the struct + trait methods (with defaults)**

In `trace_corpus_storage.rs`, beside `DedupSignalRow`:
```rust
/// One row for the per-contributor cap recompute pass. Cross-tenant by
/// construction (enumerated on the gate-driver pool), joining each decision to
/// its submission for the contributor identity (`auth_principal_ref`). The pass
/// derives `r = q * dup_pen` and the epoch bucket from these fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContributorCapSignalRow {
    pub tenant_id: String,
    pub decision_id: Uuid,
    pub auth_principal_ref: String,
    pub decided_at: DateTime<Utc>,
    pub credit_quality_micros: Option<i64>,
    pub dedup_cluster_size: Option<i32>,
}
```
Add the two trait methods to the SAME trait that declares `list_dedup_signals` / `update_trace_gate_decision_dedup`, each with the SAME style of default impl those use (default `list_*` returns `Err(DatabaseError::Pool(...))` or `Ok(vec![])` — match the dedup default exactly; default `update_*` returns the same "not implemented" style the dedup update default uses, or a log-once no-op if that is the established pattern — mirror `update_trace_gate_decision_dedup`'s default verbatim in shape).

- [ ] **Step 3: Implement `list_contributor_cap_signals` (pg, gate-driver)**

In `db/postgres.rs`, mirroring `list_dedup_signals`:
```rust
async fn list_contributor_cap_signals(
    &self,
    limit: i64,
) -> Result<Vec<crate::trace_corpus_storage::ContributorCapSignalRow>, DatabaseError> {
    let pool = self
        .gate_driver_pool
        .as_ref()
        .ok_or_else(|| DatabaseError::Pool("gate-driver pool not configured".to_string()))?;
    let client = pool.get().await.map_err(DatabaseError::from)?;
    // No tenant GUC: the trace_gate_driver role's permissive cross-tenant
    // SELECT policies authorize this read across every tenant's decisions and
    // submissions. Ordered by (auth_principal_ref, decided_at) so the recompute
    // pass groups per contributor and forward-accumulates in time order.
    let rows = client
        .query(
            "SELECT d.tenant_id, d.decision_id, s.auth_principal_ref,
                    d.decided_at, d.credit_quality_micros, d.dedup_cluster_size
             FROM trace_gate_decisions d
             JOIN trace_submissions s
               ON s.tenant_id = d.tenant_id AND s.submission_id = d.submission_id
             ORDER BY s.auth_principal_ref ASC, d.decided_at ASC
             LIMIT $1",
            &[&limit],
        )
        .await
        .map_err(DatabaseError::Postgres)?;
    Ok(rows
        .into_iter()
        .map(|row| crate::trace_corpus_storage::ContributorCapSignalRow {
            tenant_id: row.get("tenant_id"),
            decision_id: row.get("decision_id"),
            auth_principal_ref: row.get("auth_principal_ref"),
            decided_at: row.get("decided_at"),
            credit_quality_micros: row.get("credit_quality_micros"),
            dedup_cluster_size: row.get("dedup_cluster_size"),
        })
        .collect())
}
```

- [ ] **Step 4: Implement `update_trace_gate_decision_contributor_cap` (pg, tenant pool)**

In `db/trace_corpus_pg.rs`, mirroring `update_trace_gate_decision_dedup`:
```rust
async fn update_trace_gate_decision_contributor_cap(
    &self,
    tenant_id: &str,
    decision_id: Uuid,
    factor_micros: i32,
    cumulative_raw_micros: i64,
    epoch: i64,
    version: i32,
) -> Result<(), DatabaseError> {
    let mut client = self.trace_pool().get().await?;
    let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
    // Update ONLY the four contributor-cap columns on exactly this decision
    // row. Perplexity, novelty, dedup, gate status, and credit are untouched.
    tx.execute(
        "UPDATE trace_gate_decisions
            SET contributor_factor_micros = $3,
                contributor_cumulative_raw_micros = $4,
                contributor_cap_epoch = $5,
                contributor_cap_version = $6
         WHERE tenant_id = $1 AND decision_id = $2",
        &[
            &tenant_id,
            &decision_id,
            &factor_micros,
            &cumulative_raw_micros,
            &epoch,
            &version,
        ],
    )
    .await
    .map_err(DatabaseError::Postgres)?;
    tx.commit().await.map_err(DatabaseError::Postgres)?;
    Ok(())
}
```

- [ ] **Step 5: Compile both feature sets**

Run:
```
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins --features near-ai-scorer
```
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/trace-commons-server/src/trace_corpus_storage.rs crates/trace-commons-server/src/db/postgres.rs crates/trace-commons-server/src/db/trace_corpus_pg.rs crates/trace-commons-server/src/db/mod.rs
git commit -m "Add contributor-cap signal enumeration and per-decision UPDATE"
```

---

### Task 4: Recompute pass + admin route + tests

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` (route registration + handler + `run_recompute_contributor_caps_pass` + query/ack/summary structs — all mirroring the dedup recluster equivalents at ~6844 and ~45890/45793)
- Modify: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs` (in-memory double for the two new DB methods + tests)

**Interfaces:**
- Consumes: `contributor_cap::{CONTRIBUTOR_CAP_CONSTANTS_V1, epoch_index, increment_micros, contributor_factor_micros}`; `list_contributor_cap_signals`; `update_trace_gate_decision_contributor_cap`.
- Produces: `POST /v1/admin/recompute-contributor-caps` route.

- [ ] **Step 1: Register the route**

Beside `.route("/v1/admin/recluster-dedup", post(recluster_dedup_handler))` (~line 6844), add:
```rust
.route(
    "/v1/admin/recompute-contributor-caps",
    post(recompute_contributor_caps_handler),
)
```

- [ ] **Step 2: Add query/ack/summary structs + handler + pass**

Mirror `ReclusterDedupQuery`/`ReclusterDedupAck`/`ReclusterDedupSummary`/`recluster_dedup_handler`/`run_recluster_dedup_pass`:
```rust
#[derive(Debug, Deserialize)]
struct RecomputeContributorCapsQuery {
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Debug, Serialize)]
struct RecomputeContributorCapsAck {
    accepted: bool,
    limit: Option<i64>,
}

#[derive(Debug, Default)]
struct RecomputeContributorCapsSummary {
    updated: u64,
    failed: u64,
}

async fn recompute_contributor_caps_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<RecomputeContributorCapsQuery>,
) -> ApiResult<Json<RecomputeContributorCapsAck>> {
    let tenant = authenticate_with_tenant_access_grant(state.as_ref(), &headers).await?;
    require_admin(&tenant)?;
    if state.db_mirror.is_none() {
        return Err(api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "contributor-cap recompute requires a configured DB mirror",
        ));
    }
    let limit = query.limit;
    let task_state = state.clone();
    tokio::spawn(async move {
        match run_recompute_contributor_caps_pass(task_state, limit).await {
            Ok(summary) => {
                tracing::info!(
                    updated = summary.updated,
                    failed = summary.failed,
                    "Trace Commons contributor-cap recompute pass completed"
                );
            }
            Err(error) => {
                tracing::warn!(
                    error_hash = %safe_display_error_hash(&error),
                    "Trace Commons contributor-cap recompute pass failed"
                );
            }
        }
    });
    Ok(Json(RecomputeContributorCapsAck { accepted: true, limit }))
}

/// One full per-contributor cap recompute pass. A single forward sweep over the
/// gate-driver enumeration (already ordered by `auth_principal_ref, decided_at`)
/// accumulates in-epoch `R` per `(auth_principal_ref, epoch_index)`, resetting at
/// each contributor or epoch boundary, and writes each decision's marginal
/// factor + cumulative snapshot. Each factor depends only on prior in-epoch
/// decisions, so one pass is correct (no dedup-style second pass).
async fn run_recompute_contributor_caps_pass(
    state: Arc<AppState>,
    limit: Option<i64>,
) -> anyhow::Result<RecomputeContributorCapsSummary> {
    use trace_commons_server::contributor_cap as cap;
    let db = state
        .db_mirror
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("contributor-cap recompute requires a configured DB mirror"))?;
    let effective_limit = limit.unwrap_or(i64::MAX).max(0);
    let rows = db.list_contributor_cap_signals(effective_limit).await?;
    let k = &cap::CONTRIBUTOR_CAP_CONSTANTS_V1;

    let mut summary = RecomputeContributorCapsSummary::default();
    // Group key = (auth_principal_ref, epoch_index); reset R when it changes.
    let mut cur_key: Option<(String, i64)> = None;
    let mut r_before: i64 = 0;
    for row in &rows {
        let epoch = cap::epoch_index(row.decided_at.timestamp(), k.epoch_days);
        let key = (row.auth_principal_ref.clone(), epoch);
        if cur_key.as_ref() != Some(&key) {
            cur_key = Some(key);
            r_before = 0;
        }
        let r = cap::increment_micros(row.credit_quality_micros, row.dedup_cluster_size);
        let factor = cap::contributor_factor_micros(r_before, r, k);
        let cumulative = r_before.saturating_add(r);
        let factor_i32 = i32::try_from(factor).unwrap_or(i32::MAX);
        match db
            .update_trace_gate_decision_contributor_cap(
                &row.tenant_id,
                row.decision_id,
                factor_i32,
                cumulative,
                epoch,
                k.version,
            )
            .await
        {
            Ok(()) => {
                summary.updated += 1;
                r_before = cumulative;
            }
            Err(error) => {
                summary.failed += 1;
                // Do NOT advance r_before on a failed write: the snapshot for
                // this decision was not persisted, so later decisions in the
                // group must accumulate as if this one is retried next pass.
                tracing::warn!(
                    tenant_hash = %sha256_prefixed(&row.tenant_id),
                    decision_hash = %sha256_prefixed(&row.decision_id.to_string()),
                    error_hash = %safe_display_error_hash(&error),
                    "shadow contributor-cap recompute skipped one decision"
                );
            }
        }
    }
    Ok(summary)
}
```

NOTE on the failed-write branch: leaving `r_before` unadvanced is the deliberate choice — a persisted snapshot and the running total stay consistent. Confirm this matches how you want partial failures to behave; it is the safe default (never counts unpersisted credit toward the cap).

- [ ] **Step 3: Extend the in-memory test double**

Find the in-memory `Database` test double used by the dedup tests (it implements `list_dedup_signals` + `update_trace_gate_decision_dedup` and exposes `gate_decision_with_dedup_by_id`). Add:
- storage for the four `contributor_*` fields (a side map keyed by `(tenant_id, decision_id)`, like the dedup side map),
- `list_contributor_cap_signals` returning the seeded decisions joined to their seeded submissions' `auth_principal_ref`, ordered `(auth_principal_ref, decided_at)`,
- `update_trace_gate_decision_contributor_cap` writing the side map,
- an accessor `gate_decision_with_contributor_cap_by_id(...) -> {..., contributor_factor_micros, contributor_cumulative_raw_micros, contributor_cap_epoch, contributor_cap_version}` mirroring `gate_decision_with_dedup_by_id`.

Reuse the existing seeded-submission plumbing (the dedup tests already seed submissions with an `auth_principal_ref`; if not, seed it) so the join has identity to return.

- [ ] **Step 4: Write the tests**

```rust
// 1. Cross-tenant: same auth_principal_ref across two tenants shares ONE
//    per-epoch running total -> the second decision's factor is < the first's.
#[tokio::test]
async fn recompute_contributor_caps_shares_running_total_across_tenants() { /* seed
    two decisions, same principal, same epoch, different tenants, each with a
    sizable r; run the pass; assert decision 2's factor < decision 1's factor and
    decision 2's cumulative == r1 + r2. */ }

// 2. Epoch reset: two decisions for one principal in DIFFERENT epochs both get
//    factor ~1.0 (both start at R_before = 0).
#[tokio::test]
async fn recompute_contributor_caps_resets_each_epoch() { /* decided_at 8 days
    apart -> different 7-day epochs -> both near 1_000_000. */ }

// 3. Column isolation: capture a decision's credit_quality/dedup/status fields
//    before, run the pass, assert those are byte-identical after and only the
//    contributor_* columns changed.
#[tokio::test]
async fn recompute_contributor_caps_touches_only_contributor_columns() { /* ... */ }

// 4. Anti-farm end to end: a single principal floods N low-q decisions in one
//    epoch; assert the sum over the epoch of (r * factor)/1e6 <= K and the last
//    decision's factor is much smaller than the first.
#[tokio::test]
async fn recompute_contributor_caps_bounds_epoch_total_by_k() { /* ... */ }
```

Use the dedup tests (`recluster_dedup_pass_clusters_cross_tenant...`) as the structural template for seeding + running + asserting.

- [ ] **Step 5: Run the tests**

Run:
```
cargo test -p trace-commons-server --bin trace-commons-ingest -- recompute_contributor_caps
```
Expected: 4 tests PASS.

- [ ] **Step 6: Full verification**

Run:
```
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins --features near-ai-scorer
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
cargo clippy -p trace-commons-server --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
cargo test -p trace-commons-server --lib contributor_cap
```
Expected: all clean/green.

- [ ] **Step 7: Commit**

```bash
git add crates/trace-commons-server/src/bin/trace-commons-ingest.rs crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs
git commit -m "Add recompute-contributor-caps batch admin route and pass"
```

---

## Self-Review

- **Spec coverage:** cap math (Task 1) · epoch bucketing (Task 1) · V41 columns + wiring (Task 2) · cross-tenant enumeration with identity join + tenant-scoped isolated UPDATE (Task 3) · single-forward-pass recompute + admin route + cross-tenant/epoch-reset/isolation/anti-farm tests (Task 4). All spec sections map to a task.
- **Type consistency:** `contributor_factor_micros` returns `i64` (clamped [0,1e6]); the DB column is `INTEGER`, so Task 4 narrows via `i32::try_from`. `cumulative`/`epoch` are `i64` → `BIGINT`. `version` is `i32` → `INTEGER`. Matches V41.
- **Shadow-only / isolation:** every write is the Task 3 UPDATE (four columns, exact PK, tenant tx); no credit/settlement/gate writes anywhere.
- **No placeholders:** every code step carries complete code except Task 4 Step 3/4 (in-memory double + tests), which reference the concrete dedup analog to mirror and specify exact assertions — the implementer has a named template and explicit expected behavior.
