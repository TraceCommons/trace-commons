# Graded Credit Scoring Function Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Compute a pure, deterministic per-decision credit-quality score `q ∈ [0,1]` from the gate's stored numeric signals (representative perplexity, peak perplexity, representative novelty) and persist it on `trace_gate_decisions` — shadow-only, no settlement.

**Architecture:** A pure function `credit_quality(...)` in a new `credit_quality` module (multiplicative, log-concave transforms of perplexity and novelty, times a peak-vs-representative anomaly term). It is written to new `credit_quality_*` columns by two paths that share the function: inline right after each gate decision is inserted, and a batch admin route `POST /v1/admin/score-credit-quality` that mirrors the shipped perplexity re-score route (enumerate cross-tenant via the gate-driver reader pool, update per-decision touching only the credit columns).

**Tech Stack:** Rust, PostgreSQL (postgres-only), axum routes, `async_trait` DB traits, tokio. No new dependencies.

## Global Constraints

- **Postgres-only.** No libsql, no dual-backend. `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins` and `... test -p trace-commons-server --no-run` must be clean; clippy with the repo allow-list (`-A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`) must be clean; `cargo fmt` applied.
- **Shadow-only.** Nothing consumes `q` for payment/gating in this plan. Do not touch gate status, credit issuance, the vector/embedding index, novelty, tail-fraction, or the perplexity columns.
- **Isolation invariant.** Writes touch ONLY the new `credit_quality_micros`, `credit_quality_anomaly_ratio_micros`, `credit_quality_calibration_version` columns.
- **Hash-only audit/logging.** Counts, submission/decision ids as the existing gate code logs them, error hashes only. Never raw trace text, perplexity/novelty values, keys, URLs, or contributor identity in log strings.
- **Admin route reuses `require_admin`** (no new bearer gate). Fail closed if the DB mirror is absent.
- **Determinism / versioning.** `q` is deterministic per `(inputs, calibration_version)`. Constants are pinned in code and stamped as `credit_quality_calibration_version`; never a live percentile.
- **No emojis** in commits/code/comments. Short imperative commit subjects, no `feat:`/`fix:` prefix.
- **Migration number:** use **V39** (`migrations/V39__trace_credit_quality.sql`). V37 is the current head on this branch; V38 is claimed by open PR #166 (backstop). Confirm V38 is still not present on `main` at implementation time; if #166 merged, bump to the next free number consistently across the SQL filename and any test that references it.

---

### Task 1: Pure credit-quality function + constants

**Files:**
- Create: `crates/trace-commons-server/src/credit_quality.rs`
- Modify: `crates/trace-commons-server/src/lib.rs` (add `pub mod credit_quality;` — place it alphabetically near the other `pub mod` lines)

**Interfaces:**
- Produces (used by Tasks 5 and 6):
  - `pub struct CreditQualityConstants { pub ppl_floor_micros: i64, pub ppl_ceil_micros: i64, pub nov_floor_micros: i64, pub nov_ceil_micros: i64, pub anomaly_soft_ratio_micros: i64, pub anomaly_hard_ratio_micros: i64, pub version: i32 }`
  - `pub const CREDIT_QUALITY_CONSTANTS_V1: CreditQualityConstants` (pinned defaults, `version: 1`)
  - `pub struct CreditQualityScore { pub q_micros: i64, pub anomaly_ratio_micros: i64, pub anomaly_withheld: bool }`
  - `pub fn credit_quality(ppl_rep_micros: i64, ppl_peak_micros: i64, nov_rep_micros: i64, k: &CreditQualityConstants) -> CreditQualityScore`

- [ ] **Step 1: Write the failing tests**

Create `crates/trace-commons-server/src/credit_quality.rs` with a `#[cfg(test)] mod tests` containing these tests (and the type/`use` stubs needed to compile — the impl comes in Step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // Helper: real perplexity/novelty -> micros
    fn m(x: f64) -> i64 { (x * 1_000_000.0).round() as i64 }
    const K: CreditQualityConstants = CREDIT_QUALITY_CONSTANTS_V1;

    #[test]
    fn below_floor_scores_zero() {
        // perplexity below floor (6.0) -> f = 0 -> q = 0, regardless of novelty
        let s = credit_quality(m(5.9), m(6.0), m(0.9), &K);
        assert_eq!(s.q_micros, 0);
        // novelty below floor (0.5) -> g = 0 -> q = 0, regardless of perplexity
        let s = credit_quality(m(20.0), m(21.0), m(0.49), &K);
        assert_eq!(s.q_micros, 0);
    }

    #[test]
    fn at_or_above_ceiling_saturates_to_one_per_term() {
        // both signals at/above ceiling, low spikiness -> q == 1_000_000
        let s = credit_quality(K.ppl_ceil_micros, K.ppl_ceil_micros, K.nov_ceil_micros, &K);
        assert_eq!(s.q_micros, 1_000_000);
        // the 1642 outlier saturates the same as p90 (log ceiling is the winsorizer)
        let outlier = credit_quality(m(1642.0), m(1642.0), K.nov_ceil_micros, &K);
        let at_ceil = credit_quality(K.ppl_ceil_micros, K.ppl_ceil_micros, K.nov_ceil_micros, &K);
        assert_eq!(outlier.q_micros, at_ceil.q_micros);
    }

    #[test]
    fn multiplicative_collapse_one_signal_low() {
        // high perplexity but novelty just above floor -> g near 0 -> q small
        let s = credit_quality(m(30.0), m(31.0), m(0.51), &K);
        assert!(s.q_micros < 100_000, "expected collapse, got {}", s.q_micros);
    }

    #[test]
    fn monotonic_nondecreasing_in_perplexity() {
        let a = credit_quality(m(8.0), m(8.0), m(0.9), &K).q_micros;
        let b = credit_quality(m(12.0), m(12.0), m(0.9), &K).q_micros;
        assert!(b >= a, "q must not decrease as perplexity rises: {a} then {b}");
    }

    #[test]
    fn concave_diminishing_returns() {
        // equal input steps yield non-increasing output steps (concavity)
        let q = |p: f64| credit_quality(m(p), m(p), m(0.9), &K).q_micros;
        let d1 = q(10.0) - q(8.0);
        let d2 = q(12.0) - q(10.0);
        assert!(d2 <= d1, "expected concavity: d1={d1} d2={d2}");
    }

    #[test]
    fn anomaly_soft_no_penalty_hard_withholds() {
        // r <= soft -> a = 1 (no penalty): peak == rep
        let no_pen = credit_quality(m(20.0), m(20.0), m(0.9), &K);
        // r >= hard -> a = 0 -> q = 0 + withheld flag: huge peak vs tiny rep
        let hard_r = (K.anomaly_hard_ratio_micros as f64 / 1_000_000.0) + 1.0;
        let withheld = credit_quality(m(7.0), m(7.0 * hard_r), m(0.9), &K);
        assert!(no_pen.q_micros > 0);
        assert_eq!(withheld.q_micros, 0);
        assert!(withheld.anomaly_withheld);
        assert!(!no_pen.anomaly_withheld);
    }

    #[test]
    fn anomaly_ratio_is_reported() {
        let s = credit_quality(m(10.0), m(25.0), m(0.9), &K);
        // ratio = peak/rep = 2.5 -> 2_500_000 micros (allow rounding slack)
        assert!((s.anomaly_ratio_micros - 2_500_000).abs() <= 2);
    }

    #[test]
    fn zero_or_negative_rep_is_safe() {
        // rep == 0 -> below floor -> q = 0, ratio defaults to 1.0 (no divide-by-zero)
        let s = credit_quality(0, 0, m(0.9), &K);
        assert_eq!(s.q_micros, 0);
        assert_eq!(s.anomaly_ratio_micros, 1_000_000);
    }

    #[test]
    fn deterministic() {
        let a = credit_quality(m(11.0), m(14.0), m(0.8), &K);
        let b = credit_quality(m(11.0), m(14.0), m(0.8), &K);
        assert_eq!(a.q_micros, b.q_micros);
        assert_eq!(a.anomaly_ratio_micros, b.anomaly_ratio_micros);
    }

    #[test]
    fn genuine_beats_every_gamed_variant() {
        // genuine: both mid-high, low spikiness
        let genuine = credit_quality(m(15.0), m(18.0), m(0.85), &K).q_micros;
        // rare-token pump: very high ppl, novelty just above floor
        let pump = credit_quality(m(1642.0), m(1642.0), m(0.51), &K).q_micros;
        // distinctive-token shim: high novelty, ppl just above floor
        let shim = credit_quality(m(6.2), m(6.4), m(0.99), &K).q_micros;
        // peak parasite: low rep, huge peak
        let parasite = credit_quality(m(6.5), m(120.0), m(0.85), &K).q_micros;
        assert!(genuine > pump, "genuine {genuine} !> pump {pump}");
        assert!(genuine > shim, "genuine {genuine} !> shim {shim}");
        assert!(genuine > parasite, "genuine {genuine} !> parasite {parasite}");
    }

    // Property-based layer (loop-sampled, no new dependency).
    #[test]
    fn property_monotonic_and_bounded() {
        let novs = [m(0.6), m(0.8), m(1.0)];
        for &nov in &novs {
            let mut prev = -1i64;
            let mut p = 6.0_f64;
            while p <= 60.0 {
                let q = credit_quality(m(p), m(p), nov, &K).q_micros;
                assert!((0..=1_000_000).contains(&q), "q out of range: {q}");
                assert!(q >= prev, "not monotonic at p={p} nov={nov}: {prev} then {q}");
                prev = q;
                p += 0.5;
            }
        }
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p trace-commons-server --lib credit_quality 2>&1 | tail -20`
Expected: FAIL to compile (`credit_quality` / types not defined).

- [ ] **Step 3: Write the implementation**

At the top of `crates/trace-commons-server/src/credit_quality.rs` (above the test module):

```rust
//! Pure, deterministic per-decision credit-quality score `q in [0,1]`, computed
//! from the gate's stored numeric signals. Multiplicative and log-concave
//! (anti-Goodhart), with a peak-vs-representative anomaly term used ONLY as a
//! fraud flag — never a bonus. Shadow-only: nothing here settles or pays.

/// Pinned, versioned calibration constants. `*_CEIL` and `anomaly_*` are
/// calibration outputs; the V1 defaults are seeded from the 2026-07-12 27B
/// distribution (perplexity p90 ~= 38.5) and refined by the on-pilot
/// distribution run (see the design spec's rollout). Bumping any value MUST
/// bump `version`.
#[derive(Debug, Clone, Copy)]
pub struct CreditQualityConstants {
    pub ppl_floor_micros: i64,
    pub ppl_ceil_micros: i64,
    pub nov_floor_micros: i64,
    pub nov_ceil_micros: i64,
    pub anomaly_soft_ratio_micros: i64,
    pub anomaly_hard_ratio_micros: i64,
    pub version: i32,
}

pub const CREDIT_QUALITY_CONSTANTS_V1: CreditQualityConstants = CreditQualityConstants {
    ppl_floor_micros: 6_000_000,
    ppl_ceil_micros: 38_500_000,
    nov_floor_micros: 500_000,
    nov_ceil_micros: 1_000_000,
    anomaly_soft_ratio_micros: 3_000_000,   // r <= 3.0 -> no penalty
    anomaly_hard_ratio_micros: 10_000_000,  // r >= 10.0 -> withhold
    version: 1,
};

/// Result of scoring one decision row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreditQualityScore {
    /// `q` * 1e6, clamped to [0, 1_000_000].
    pub q_micros: i64,
    /// peak/rep spikiness ratio * 1e6 (>= 0; defaults to 1_000_000 when rep <= 0).
    pub anomaly_ratio_micros: i64,
    /// True when the spikiness ratio reached the hard threshold (a == 0).
    pub anomaly_withheld: bool,
}

/// Concave, saturating map of a micros signal onto [0,1]:
/// `log(1 + max(0, x - floor)) / log(1 + ceil - floor)`, in real (non-micros)
/// units. Below floor -> 0; at/above ceil -> 1.
fn saturating_term(value_micros: i64, floor_micros: i64, ceil_micros: i64) -> f64 {
    if value_micros <= floor_micros {
        return 0.0;
    }
    let x = (value_micros - floor_micros) as f64 / 1_000_000.0;
    // max(1) guards a degenerate ceil <= floor against divide-by-zero.
    let span = ((ceil_micros - floor_micros).max(1)) as f64 / 1_000_000.0;
    ((1.0 + x).ln() / (1.0 + span).ln()).clamp(0.0, 1.0)
}

pub fn credit_quality(
    ppl_rep_micros: i64,
    ppl_peak_micros: i64,
    nov_rep_micros: i64,
    k: &CreditQualityConstants,
) -> CreditQualityScore {
    let f = saturating_term(ppl_rep_micros, k.ppl_floor_micros, k.ppl_ceil_micros);
    let g = saturating_term(nov_rep_micros, k.nov_floor_micros, k.nov_ceil_micros);

    // Spikiness ratio r = peak / rep (real units); rep <= 0 -> ratio 1.0 (no signal).
    let (ratio, anomaly_ratio_micros) = if ppl_rep_micros <= 0 {
        (1.0_f64, 1_000_000_i64)
    } else {
        let r = ppl_peak_micros.max(0) as f64 / ppl_rep_micros as f64;
        (r, (r * 1_000_000.0).round() as i64)
    };

    let soft = k.anomaly_soft_ratio_micros as f64 / 1_000_000.0;
    let hard = k.anomaly_hard_ratio_micros as f64 / 1_000_000.0;
    let (a, withheld) = if ratio <= soft {
        (1.0, false)
    } else if ratio >= hard {
        (0.0, true)
    } else {
        (1.0 - (ratio - soft) / (hard - soft), false)
    };

    let q = (f * g * a).clamp(0.0, 1.0);
    CreditQualityScore {
        q_micros: (q * 1_000_000.0).round() as i64,
        anomaly_ratio_micros,
        anomaly_withheld: withheld,
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p trace-commons-server --lib credit_quality 2>&1 | tail -20`
Expected: PASS (all tests). Then `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins` clean and `cargo clippy -p trace-commons-server --lib -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching` clean.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add crates/trace-commons-server/src/credit_quality.rs crates/trace-commons-server/src/lib.rs
git commit -m "Add pure credit-quality scoring function"
```

---

### Task 2: Migration V39 — credit_quality columns

**Files:**
- Create: `migrations/V39__trace_credit_quality.sql`

**Interfaces:**
- Produces: three nullable columns on `trace_gate_decisions`: `credit_quality_micros BIGINT`, `credit_quality_anomaly_ratio_micros BIGINT`, `credit_quality_calibration_version INTEGER`.

- [ ] **Step 1: Write the migration**

First confirm the head and the exact table name/column style by reading the perplexity columns' migration:

Run: `grep -rl "peak_perplexity_micros" migrations/` and read that file to match the `ALTER TABLE trace_gate_decisions ADD COLUMN ...` style and any RLS/grant conventions used for added columns.

Then create `migrations/V39__trace_credit_quality.sql`:

```sql
-- Shadow-mode graded-credit quality score persisted per gate decision.
-- All nullable/backfillable: q is computed inline for new decisions and by the
-- POST /v1/admin/score-credit-quality batch route for existing rows. No RLS
-- change is needed (columns on an already-RLS-forced table inherit the tenant
-- predicate); no new grants beyond the existing trace_gate_decisions grants.
ALTER TABLE trace_gate_decisions
    ADD COLUMN IF NOT EXISTS credit_quality_micros BIGINT,
    ADD COLUMN IF NOT EXISTS credit_quality_anomaly_ratio_micros BIGINT,
    ADD COLUMN IF NOT EXISTS credit_quality_calibration_version INTEGER;
```

- [ ] **Step 2: Apply against the shared test DB and verify columns exist**

Run (requires the local `trace_commons_test` PostgreSQL used by the pg tests — see CLAUDE.md):
`psql "$TRACE_COMMONS_TEST_DATABASE_URL" -f migrations/V39__trace_credit_quality.sql && psql "$TRACE_COMMONS_TEST_DATABASE_URL" -c "\d trace_gate_decisions" | grep credit_quality`
Expected: the three `credit_quality_*` columns listed. (If no local DB is available, note it and rely on the Task 3/6 pg tests, which the migration must let compile/run.)

- [ ] **Step 3: Commit**

```bash
git add migrations/V39__trace_credit_quality.sql
git commit -m "Add credit_quality columns to trace_gate_decisions (V39)"
```

---

### Task 3: DB update method — `update_trace_gate_decision_credit_quality`

**Files:**
- Modify: `crates/trace-commons-server/src/trace_corpus_storage.rs` (add trait method with a default no-op+warn, mirroring `update_trace_gate_decision_perplexity` at ~line 2408)
- Modify: `crates/trace-commons-server/src/db/trace_corpus_pg.rs` (real impl, mirroring the perplexity impl at ~line 5505)
- Modify: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs` (extend the in-memory double at ~line 63973 and add the isolation test near ~64255)
- Test: `crates/trace-commons-server/tests/trace_corpus_pg_store.rs` (real-Postgres isolation test, mirroring `pg_store_update_trace_gate_decision_perplexity_scopes_to_latest_decision_row`)

**Interfaces:**
- Consumes: `credit_quality::CreditQualityScore` (Task 1) at call sites (Tasks 5, 6).
- Produces: `async fn update_trace_gate_decision_credit_quality(&self, tenant_id: &str, decision_id: Uuid, q_micros: i64, anomaly_ratio_micros: i64, calibration_version: i32) -> Result<(), DatabaseError>` on the `TraceCorpusStore` trait.

- [ ] **Step 1: Write the failing isolation test (in-memory double)**

In `tests.rs`, next to `update_trace_gate_decision_perplexity_touches_only_perplexity_columns`, add a test that seeds a decision row, calls `update_trace_gate_decision_credit_quality`, and asserts ONLY the three credit_quality columns changed while perplexity/novelty/status/credit columns are byte-identical. Reuse the same in-memory decision-row fixture the perplexity test uses; extend the double's stored row struct with the three `credit_quality_*` fields (default `None`).

```rust
#[tokio::test]
async fn update_trace_gate_decision_credit_quality_touches_only_credit_columns() {
    let db = /* same in-memory double setup as the perplexity isolation test */;
    // seed one decision row with known perplexity + novelty values
    let before = db.debug_get_decision(TENANT, DECISION_ID).await.unwrap();
    db.update_trace_gate_decision_credit_quality(TENANT, DECISION_ID, 730_000, 2_500_000, 1)
        .await
        .unwrap();
    let after = db.debug_get_decision(TENANT, DECISION_ID).await.unwrap();
    assert_eq!(after.credit_quality_micros, Some(730_000));
    assert_eq!(after.credit_quality_anomaly_ratio_micros, Some(2_500_000));
    assert_eq!(after.credit_quality_calibration_version, Some(1));
    // isolation: every non-credit column unchanged
    assert_eq!(after.perplexity_micros, before.perplexity_micros);
    assert_eq!(after.peak_perplexity_micros, before.peak_perplexity_micros);
    assert_eq!(after.perplexity_passed, before.perplexity_passed);
    assert_eq!(after.novelty_score_micros, before.novelty_score_micros);
    assert_eq!(after.novelty_passed, before.novelty_passed);
    assert_eq!(after.nearest_neighbor_hash, before.nearest_neighbor_hash);
    assert_eq!(after.credit_withheld_reason, before.credit_withheld_reason);
}
```
(Use the exact field/accessor names the perplexity isolation test uses on the double; if the double exposes rows differently, match that shape.)

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p trace-commons-server --bin trace-commons-ingest update_trace_gate_decision_credit_quality_touches_only_credit_columns 2>&1 | tail -20`
Expected: FAIL to compile (method + fields not defined).

- [ ] **Step 3: Add the trait method (default no-op+warn)**

In `trace_corpus_storage.rs`, immediately after `update_trace_gate_decision_perplexity`, add — mirroring its doc + default exactly:

```rust
/// Update ONLY the credit-quality columns for the decision row identified by
/// `(tenant_id, decision_id)`. Perplexity, novelty, tail-fraction, vector,
/// gate status, and credit are left untouched. Implementations MUST scope by
/// `tenant_id` (forced RLS). Defaults to a log-once warning + no-op so a
/// backend without a real impl cannot silently drop the write.
async fn update_trace_gate_decision_credit_quality(
    &self,
    _tenant_id: &str,
    _decision_id: Uuid,
    _q_micros: i64,
    _anomaly_ratio_micros: i64,
    _calibration_version: i32,
) -> Result<(), DatabaseError> {
    static WARNED: std::sync::Once = std::sync::Once::new();
    WARNED.call_once(|| {
        tracing::warn!(
            "update_trace_gate_decision_credit_quality called on a backend without a real impl"
        );
    });
    Ok(())
}
```

- [ ] **Step 4: Add the Postgres impl**

In `db/trace_corpus_pg.rs`, after the perplexity impl (~5505), add — note it targets the exact PK `(tenant_id, decision_id)` (no "latest" subquery: the caller supplies the decision_id directly), and touches only the three credit columns:

```rust
async fn update_trace_gate_decision_credit_quality(
    &self,
    tenant_id: &str,
    decision_id: Uuid,
    q_micros: i64,
    anomaly_ratio_micros: i64,
    calibration_version: i32,
) -> Result<(), DatabaseError> {
    let mut client = self.trace_pool().get().await?;
    let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
    // Update ONLY the three credit_quality columns on exactly this decision
    // row. Perplexity, novelty, tail-fraction, vector-entry, gate status, and
    // credit are left exactly as-is.
    tx.execute(
        "UPDATE trace_gate_decisions
            SET credit_quality_micros = $3,
                credit_quality_anomaly_ratio_micros = $4,
                credit_quality_calibration_version = $5
         WHERE tenant_id = $1 AND decision_id = $2",
        &[
            &tenant_id,
            &decision_id,
            &q_micros,
            &anomaly_ratio_micros,
            &calibration_version,
        ],
    )
    .await
    .map_err(DatabaseError::Postgres)?;
    tx.commit().await.map_err(DatabaseError::Postgres)?;
    Ok(())
}
```

- [ ] **Step 5: Implement the double + make the test pass**

Extend the in-memory double (~63973) with a real `update_trace_gate_decision_credit_quality` that sets the three fields on the stored decision row keyed by `(tenant_id, decision_id)`; add the three `Option`-typed fields to the double's decision-row struct (default `None`) and to whatever constructor/seed the isolation test uses.

Run: `cargo test -p trace-commons-server --bin trace-commons-ingest update_trace_gate_decision_credit_quality_touches_only_credit_columns 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 6: Add the real-Postgres isolation test**

In `tests/trace_corpus_pg_store.rs`, mirroring `pg_store_update_trace_gate_decision_perplexity_scopes_to_latest_decision_row`, add `pg_store_update_trace_gate_decision_credit_quality_touches_only_credit_columns`: seed a decision row, call the method, assert the three credit columns changed and perplexity/novelty/status columns are byte-identical.

Run: `cargo test -p trace-commons-server --test trace_corpus_pg_store credit_quality 2>&1 | tail -20` (requires local PostgreSQL; if unavailable, note it — CI does not run pg tests, and the compile must still be clean).
Expected: PASS (or documented skip if no DB), and `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run` clean.

- [ ] **Step 7: Commit**

```bash
cargo fmt
git add -A
git commit -m "Add tenant-scoped credit-quality decision UPDATE (trait, pg, double, tests)"
```

---

### Task 4: DB enumeration — `list_gate_decisions_for_credit_scoring`

**Files:**
- Modify: `crates/trace-commons-server/src/db/mod.rs` (add trait method with an empty-Vec default, mirroring `list_submissions_with_gate_decision` at ~957)
- Modify: `crates/trace-commons-server/src/db/postgres.rs` (real impl via `gate_driver_pool`, mirroring the impl at ~3639)
- Modify: `crates/trace-commons-server/src/trace_corpus_storage.rs` (add the row struct next to `GateWorkItem` at ~1829)
- Modify: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs` (double default is fine; add a unit test of the mapping if the double supports it, else rely on Task 6's integration test)

**Interfaces:**
- Produces:
  - `pub struct GateCreditInput { pub tenant_id: String, pub decision_id: Uuid, pub perplexity_micros: i64, pub peak_perplexity_micros: i64, pub novelty_score_micros: i64 }` (in `trace_corpus_storage.rs`, next to `GateWorkItem`)
  - `async fn list_gate_decisions_for_credit_scoring(&self, limit: i64) -> Result<Vec<GateCreditInput>, DatabaseError>` on the `Database` trait (`db/mod.rs`).

- [ ] **Step 1: Add the struct**

In `trace_corpus_storage.rs`, right after `GateWorkItem`:

```rust
/// Numeric inputs for shadow credit-quality scoring of one decision row, read
/// cross-tenant through the narrow `trace_gate_driver` pool (no tenant GUC).
/// The peak/novelty are stored micros; NULLs map to 0 (below-floor -> q 0).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateCreditInput {
    pub tenant_id: String,
    pub decision_id: Uuid,
    pub perplexity_micros: i64,
    pub peak_perplexity_micros: i64,
    pub novelty_score_micros: i64,
}
```

- [ ] **Step 2: Add the trait default (db/mod.rs)**

Immediately after `list_submissions_with_gate_decision`'s default, add:

```rust
/// Enumerate decision rows for shadow credit-quality scoring, cross-tenant,
/// oldest-decided first, capped at `limit`. Reads through the gate-driver
/// reader pool with NO tenant GUC (the trace_gate_driver role's permissive
/// cross-tenant SELECT policies authorize it). Default: empty (test doubles
/// / backends without a gate-driver pool).
async fn list_gate_decisions_for_credit_scoring(
    &self,
    _limit: i64,
) -> Result<Vec<crate::trace_corpus_storage::GateCreditInput>, DatabaseError> {
    Ok(Vec::new())
}
```

- [ ] **Step 3: Add the Postgres impl (postgres.rs)**

After `list_submissions_with_gate_decision` (~3639), add — reading directly from `trace_gate_decisions` (the perplexity enumeration already proves the gate-driver role can read this table cross-tenant via its JOIN). Use `COALESCE(...,0)` so NULL micros map to 0:

```rust
async fn list_gate_decisions_for_credit_scoring(
    &self,
    limit: i64,
) -> Result<Vec<crate::trace_corpus_storage::GateCreditInput>, DatabaseError> {
    let pool = self
        .gate_driver_pool
        .as_ref()
        .ok_or_else(|| DatabaseError::Pool("gate-driver pool not configured".to_string()))?;
    let client = pool.get().await.map_err(DatabaseError::from)?;
    // No tenant GUC: the trace_gate_driver role's permissive cross-tenant
    // SELECT policies authorize this read across every tenant's decisions.
    let rows = client
        .query(
            "SELECT tenant_id, decision_id,
                    COALESCE(perplexity_micros, 0)      AS perplexity_micros,
                    COALESCE(peak_perplexity_micros, 0) AS peak_perplexity_micros,
                    COALESCE(novelty_score_micros, 0)   AS novelty_score_micros
             FROM trace_gate_decisions
             ORDER BY decided_at ASC
             LIMIT $1",
            &[&limit],
        )
        .await
        .map_err(DatabaseError::Postgres)?;
    Ok(rows
        .into_iter()
        .map(|row| crate::trace_corpus_storage::GateCreditInput {
            tenant_id: row.get("tenant_id"),
            decision_id: row.get("decision_id"),
            perplexity_micros: row.get("perplexity_micros"),
            peak_perplexity_micros: row.get("peak_perplexity_micros"),
            novelty_score_micros: row.get("novelty_score_micros"),
        })
        .collect())
}
```

Confirm the column names `decision_id`, `decided_at`, `perplexity_micros`, `peak_perplexity_micros`, `novelty_score_micros` against the V23 `trace_gate_decisions` migration before finalizing; match them exactly.

- [ ] **Step 4: Verify compile**

Run: `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins 2>&1 | tail -20`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add -A
git commit -m "Add cross-tenant credit-scoring decision enumeration"
```

---

### Task 5: Inline-at-gate credit-quality write

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` (in `evaluate_and_record_gate`, after the decision row is inserted)
- Test: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs`

**Interfaces:**
- Consumes: `credit_quality::{credit_quality, CREDIT_QUALITY_CONSTANTS_V1}` (Task 1), `update_trace_gate_decision_credit_quality` (Task 3), the decision id + perplexity/peak/novelty micros produced during gate evaluation.

- [ ] **Step 1: Locate the insert + read the decision id**

Read `evaluate_and_record_gate` and `insert_trace_gate_decision` in `trace-commons-ingest.rs`. Identify (a) the just-inserted decision's `decision_id` (the insert returns the decision record — capture it), and (b) the representative perplexity, peak perplexity, and representative novelty micros already computed in that function. Write down their exact variable names/paths for use below.

- [ ] **Step 2: Write the failing test**

Add a test (in `tests.rs`, using the in-memory double + the existing gate-evaluation test harness) asserting that after a gate decision is recorded, the decision row carries a non-null `credit_quality_micros` and `credit_quality_calibration_version == 1`. Pick input signals that produce a known non-zero `q` (e.g. perplexity 15.0, peak 18.0, novelty 0.85) and assert the stored `credit_quality_micros` equals `credit_quality(m(15.0), m(18.0), m(0.85), &CREDIT_QUALITY_CONSTANTS_V1).q_micros`.

- [ ] **Step 3: Run to verify it fails**

Run: `cargo test -p trace-commons-server --bin trace-commons-ingest <new_test_name> 2>&1 | tail -20`
Expected: FAIL (credit_quality_micros is None).

- [ ] **Step 4: Implement the inline write**

In `evaluate_and_record_gate`, immediately after the decision insert succeeds, compute and persist `q` (reuse Task 3's method — do NOT change the insert SQL). Use the exact variable names captured in Step 1:

```rust
// Shadow-mode credit-quality score (no settlement, no gating). Pure arithmetic
// on the signals just recorded; failure is non-fatal and hash-only logged so a
// scoring hiccup never blocks the gate decision itself.
let cq = crate::credit_quality::credit_quality(
    perplexity_micros,      // representative perplexity (i64 micros) from this fn
    peak_perplexity_micros, // peak (i64 micros)
    novelty_micros,         // representative novelty (i64 micros)
    &crate::credit_quality::CREDIT_QUALITY_CONSTANTS_V1,
);
if let Err(error) = db
    .update_trace_gate_decision_credit_quality(
        tenant_id,
        decision_id,
        cq.q_micros,
        cq.anomaly_ratio_micros,
        crate::credit_quality::CREDIT_QUALITY_CONSTANTS_V1.version,
    )
    .await
{
    tracing::warn!(
        tenant_hash = %sha256_prefixed(tenant_id),
        error_hash = %safe_display_error_hash(&error),
        "shadow credit-quality inline write failed (non-fatal)"
    );
}
```
Adapt `perplexity_micros`/`peak_perplexity_micros`/`novelty_micros`/`decision_id`/`tenant_id` to the real bindings. If any signal is only available as `Option`/`u64`, convert with the same `i64::try_from(...).unwrap_or(...)`/`unwrap_or(0)` pattern the re-score uses, so a missing novelty maps to 0 (→ q 0).

- [ ] **Step 5: Run to verify it passes**

Run: `cargo test -p trace-commons-server --bin trace-commons-ingest <new_test_name> 2>&1 | tail -20`
Expected: PASS. Then `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins` clean.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add -A
git commit -m "Compute shadow credit-quality inline at gate time"
```

---

### Task 6: Batch admin route `POST /v1/admin/score-credit-quality`

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` (route registration near the `rescore-perplexity` line ~6566; handler/pass/one + ack/summary/query types near the re-score handler ~45118-45240)
- Test: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs`

**Interfaces:**
- Consumes: `list_gate_decisions_for_credit_scoring` (Task 4), `credit_quality` (Task 1), `update_trace_gate_decision_credit_quality` (Task 3).

- [ ] **Step 1: Write the failing integration test**

In `tests.rs`, mirroring the re-score end-to-end test, seed several gate decisions with varied perplexity/novelty on the in-memory double, run the credit-quality pass (call the pass function directly with the test `AppState`), and assert: (a) every decision row now has `credit_quality_micros == credit_quality(...).q_micros` for its inputs; (b) perplexity/novelty/status columns are byte-identical to before the pass (isolation). Include one row whose signals make `q == 0` (novelty below floor) to prove collapse persists as 0, not NULL.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p trace-commons-server --bin trace-commons-ingest score_credit_quality 2>&1 | tail -20`
Expected: FAIL to compile (pass fn + types not defined).

- [ ] **Step 3: Add types + pass + one + handler**

Mirror the re-score structures. Add near the re-score handler:

```rust
#[derive(Debug, Default)]
struct ScoreCreditQualitySummary {
    scored: u64,
    failed: u64,
}

#[derive(serde::Deserialize)]
struct ScoreCreditQualityQuery {
    limit: Option<i64>,
}

#[derive(serde::Serialize)]
struct ScoreCreditQualityAck {
    accepted: bool,
    limit: Option<i64>,
}

/// Score ONE decision row's shadow credit-quality from its stored signals and
/// update only the credit_quality columns. Pure arithmetic — no decrypt, no
/// scorer, no index.
async fn score_credit_quality_one(
    state: &AppState,
    input: &crate::trace_corpus_storage::GateCreditInput,
) -> anyhow::Result<()> {
    let db = state
        .db_mirror
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("credit-quality scoring requires a configured DB mirror"))?;
    let cq = crate::credit_quality::credit_quality(
        input.perplexity_micros,
        input.peak_perplexity_micros,
        input.novelty_score_micros,
        &crate::credit_quality::CREDIT_QUALITY_CONSTANTS_V1,
    );
    db.update_trace_gate_decision_credit_quality(
        &input.tenant_id,
        input.decision_id,
        cq.q_micros,
        cq.anomaly_ratio_micros,
        crate::credit_quality::CREDIT_QUALITY_CONSTANTS_V1.version,
    )
    .await?;
    tracing::info!(
        tenant_hash = %sha256_prefixed(&input.tenant_id),
        decision_hash = %sha256_prefixed(&input.decision_id.to_string()),
        "shadow credit-quality scored one decision"
    );
    Ok(())
}

async fn run_score_credit_quality_pass(
    state: Arc<AppState>,
    limit: Option<i64>,
) -> anyhow::Result<ScoreCreditQualitySummary> {
    let db = state
        .db_mirror
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("credit-quality scoring requires a configured DB mirror"))?;
    let effective_limit = limit.unwrap_or(i64::MAX).max(0);
    let inputs = db.list_gate_decisions_for_credit_scoring(effective_limit).await?;
    let mut summary = ScoreCreditQualitySummary::default();
    for input in &inputs {
        match score_credit_quality_one(state.as_ref(), input).await {
            Ok(()) => summary.scored += 1,
            Err(error) => {
                summary.failed += 1;
                tracing::warn!(
                    tenant_hash = %sha256_prefixed(&input.tenant_id),
                    decision_hash = %sha256_prefixed(&input.decision_id.to_string()),
                    error_hash = %safe_display_error_hash(&error),
                    "shadow credit-quality skipped one decision"
                );
            }
        }
    }
    Ok(summary)
}

async fn score_credit_quality_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ScoreCreditQualityQuery>,
) -> ApiResult<Json<ScoreCreditQualityAck>> {
    let tenant = authenticate_with_tenant_access_grant(state.as_ref(), &headers).await?;
    require_admin(&tenant)?;
    if state.db_mirror.is_none() {
        return Err(api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "trace credit-quality scoring requires a configured DB mirror",
        ));
    }
    let limit = query.limit;
    let task_state = state.clone();
    tokio::spawn(async move {
        match run_score_credit_quality_pass(task_state, limit).await {
            Ok(summary) => tracing::info!(
                scored = summary.scored,
                failed = summary.failed,
                "Trace Commons credit-quality pass completed"
            ),
            Err(error) => tracing::warn!(
                error_hash = %safe_display_error_hash(&error),
                "Trace Commons credit-quality pass failed"
            ),
        }
    });
    Ok(Json(ScoreCreditQualityAck { accepted: true, limit }))
}
```

Register the route next to `rescore-perplexity` (~6566):

```rust
.route(
    "/v1/admin/score-credit-quality",
    post(score_credit_quality_handler),
)
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p trace-commons-server --bin trace-commons-ingest score_credit_quality 2>&1 | tail -20`
Expected: PASS. Then the full gate: `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins` and `... test -p trace-commons-server --no-run` clean; `cargo clippy -p trace-commons-server --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching` clean.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add -A
git commit -m "Add credit-quality batch admin route"
```

---

## Post-implementation (operational, not a code task)

After merge + pilot deploy, run the batch route (`POST /v1/admin/score-credit-quality`) over the 349 now-27B-consistent decisions using the signed admin token flow (see the pilot admin-token memory), inspect the `q` distribution and the `anomaly_withheld` set, calibrate `PPL_CEIL`/`NOV_CEIL`/`anomaly_soft`/`anomaly_hard` from the observed distributions, bump `CREDIT_QUALITY_CONSTANTS_V1` → V2 (new `version`), and re-run. Assert the distribution is non-degenerate (spreads across `[0,1]`), unlike the tail-fraction metric.
