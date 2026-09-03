# Credit-Numbers API Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Serve a contributor their own credit figures over an authenticated endpoint, and the register's aggregate figures over an unauthenticated one, without either surface being able to leak into the other.

**Architecture:** Two endpoints that share only a posture/units helper. `GET /v1/account/credit-summary` runs on the existing account-session path and is scoped to the caller's principal set. `GET /v1/public/register-stats` has no tenant context at all, so it reads a materialised aggregate through a dedicated `NOBYPASSRLS` role with a role-scoped policy — never a superuser pool, never `BYPASSRLS`.

**Tech Stack:** Rust, axum, PostgreSQL with forced RLS, existing `AccountCtx` session auth. **No new dependencies.**

**Spec:** `docs/superpowers/specs/2026-09-01-credit-numbers-api-design.md` — read it before starting.

## Spec correction, settled before this plan

The spec lists `spent_this_period` with an open question about whether the
server can know it. **It cannot, and the field is cut.**

`grep -rn "inference_spend\|inference_cost\|spend_usd\|cost_usd" crates/trace-commons-server/src --include='*.rs'` returns nothing outside tests. The server
sees submissions; it has never seen anyone's inference bill.

So the contract carries **earned only**. The client composes "covering 30% of
your bill" from its own local inference ledger — the desktop client already has
those numbers locally from the routing overlay — and the server never learns
what anyone spends. That is a better boundary than the spec assumed, not a
worse one: an API that reported your spend back to you would have to be told it
first.

Any task that would add a spend field to a server response is out of scope and
should be raised rather than built.

## Global Constraints

- **No new dependencies.** Nothing added to any `Cargo.toml`. If you believe you need one, stop and ask.
- **AGPL crate.** Every new `.rs` file under `crates/trace-commons-server/` carries the two-line header, copied exactly from an existing file:
  ```rust
  // Copyright (C) 2026 K&Z Partners LLC
  // SPDX-License-Identifier: AGPL-3.0-or-later
  ```
- **Never `BYPASSRLS`, never a superuser pool, never dropping `FORCE`** on a table. The public path uses a role-scoped policy, and its test uses `SET ROLE` — an owner connection would hide a missing policy entirely.
- **Hash-only logging.** No account ids, principal refs, tenant ids, or figures in log lines.
- **Fail closed.** A missing configuration means the feature is absent from the response, never a default value.
- **No emojis.** Commit subject: short imperative, no `feat:`/`fix:` prefix.
- **Next migration number is `V55`** — `V54__trace_gate_decision_qualifying_mass.sql` is the highest present. `run_migrations` is hand-rolled; a new migration must be wired into it explicitly or it never runs.
- **Verification for every task:**
  ```bash
  cargo fmt --all
  RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
  cargo clippy -p trace-commons-server --all-targets -- \
    -A clippy::type_complexity -A clippy::collapsible_if \
    -A clippy::manual_option_as_slice -A clippy::useless_vec \
    -A clippy::redundant_pattern_matching
  ```
  Do **not** run `cargo test --workspace` — the controller runs it. Run the targeted tests each task names.

## File Structure

| File | Responsibility |
|---|---|
| `crates/trace-commons-server/src/credit_numbers.rs` (create) | The units/posture helper: points-to-currency gating, the posture block. Pure, no I/O, no HTTP. |
| `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` (modify) | Both handlers and their route registrations, beside the existing `/v1/account/*` and `/v1/source` routes. |
| `migrations/V55__register_stats_public_read.sql` (create) | The aggregate view, the read role, the role-scoped policy. |
| `crates/trace-commons-server/tests/register_stats_rls.rs` (create) | Proves the public read works under `SET ROLE` and reaches nothing else. |

---

### Task 1: The units and posture helper

Everything downstream depends on getting this right: `currency` must be **absent** when no rate is configured, because a client reading a zero would tell a contributor they earned nothing.

**Files:**
- Create: `crates/trace-commons-server/src/credit_numbers.rs`
- Modify: `crates/trace-commons-server/src/lib.rs` (add `pub mod credit_numbers;`)

**Interfaces:**
- Produces: `CreditPosture { settlement: String, graded: bool, explanation: String }`; `CurrencyBlock { code: String, earned_this_period: String }`; `fn currency_for(points: i64, rate: Option<&CreditRate>) -> Option<CurrencyBlock>`; `CreditRate { points_per_unit: f64, code: String }`; `fn rate_from_env() -> Option<CreditRate>`

- [ ] **Step 1: Write the failing tests**

Create `crates/trace-commons-server/src/credit_numbers.rs` with the header and only this test module:

```rust
// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_configured_rate_yields_no_currency_block_at_all() {
        // Absent, never zero. A client that sees no key shows points; a client
        // that saw `0.00` would tell a contributor they earned nothing.
        assert!(currency_for(1240, None).is_none());
    }

    #[test]
    fn a_configured_rate_converts_points_to_a_fixed_two_place_string() {
        let rate = CreditRate { points_per_unit: 100.0, code: "USD".to_string() };
        let block = currency_for(1240, Some(&rate)).expect("a rate yields a block");
        assert_eq!(block.code, "USD");
        assert_eq!(block.earned_this_period, "12.40");
    }

    #[test]
    fn zero_points_with_a_rate_is_still_a_block_reading_zero() {
        // Distinct from the no-rate case above: here the contributor really
        // did earn nothing, and saying so is correct.
        let rate = CreditRate { points_per_unit: 100.0, code: "USD".to_string() };
        let block = currency_for(0, Some(&rate)).expect("a rate yields a block");
        assert_eq!(block.earned_this_period, "0.00");
    }

    #[test]
    fn a_nonsense_rate_yields_no_currency_rather_than_a_wrong_number() {
        // Fail closed: a zero or negative divisor means the deployment is
        // misconfigured, and no figure is better than an invented one.
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let rate = CreditRate { points_per_unit: bad, code: "USD".to_string() };
            assert!(currency_for(1240, Some(&rate)).is_none(), "rate {bad} must not convert");
        }
    }

    #[test]
    fn posture_reports_ungraded_while_the_pipeline_is_shadow_mode() {
        let posture = CreditPosture::current("disabled", false);
        assert_eq!(posture.settlement, "disabled");
        assert!(!posture.graded);
        assert!(
            !posture.explanation.is_empty(),
            "a posture always states itself in words"
        );
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p trace-commons-server --lib credit_numbers
```

Expected: FAIL — the module is not declared and the types do not exist.

- [ ] **Step 3: Declare the module**

In `crates/trace-commons-server/src/lib.rs`, beside the other `pub mod` lines, in alphabetical position:

```rust
pub mod credit_numbers;
```

- [ ] **Step 4: Write the implementation**

Above the test module in `credit_numbers.rs`:

```rust
//! Units and posture for the credit-numbers endpoints.
//!
//! Pure: no I/O, no HTTP, no database. Both endpoints render their figures
//! through here so a contributor and a public reader can never be told two
//! different stories about the same deployment.

use serde::Serialize;

/// A deployment's configured points-to-currency rate.
///
/// Optional everywhere. A deployment that has not set one does not tell
/// contributors their work is worth money, which is the correct posture until
/// the graded pipeline leaves shadow mode.
#[derive(Debug, Clone)]
pub struct CreditRate {
    /// Points that make one unit of currency.
    pub points_per_unit: f64,
    /// ISO 4217 code, reported verbatim to clients.
    pub code: String,
}

/// The currency view of a points figure. Serialized only when it exists.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CurrencyBlock {
    pub code: String,
    /// Fixed two places, as a string: a float would invite a client to do
    /// arithmetic on money the server already rounded.
    pub earned_this_period: String,
}

/// What this deployment is actually doing with credit.
#[derive(Debug, Clone, Serialize)]
pub struct CreditPosture {
    /// The live value of `TRACE_COMMONS_NEAR_SETTLEMENT_MODE`.
    pub settlement: String,
    /// Whether quality, duplicate penalty and the per-contributor cap are
    /// authoritative. False while that pipeline is shadow-mode, which is what
    /// lets a client say a figure may still be revised.
    pub graded: bool,
    /// The same sentence the submission receipt gives, so two surfaces cannot
    /// describe one deployment differently.
    pub explanation: String,
}

impl CreditPosture {
    #[must_use]
    pub fn current(settlement_mode: &str, graded: bool) -> Self {
        Self {
            settlement: settlement_mode.to_string(),
            graded,
            explanation: settlement_posture_sentence(settlement_mode, graded),
        }
    }
}

/// One sentence describing the deployment's posture.
///
/// Deliberately mirrors `settlement_posture_explanation` in the ingest binary,
/// which puts the same fact on a submission receipt. If you change the wording
/// in one place, change it in the other: a receipt and an API that disagree
/// about whether credit settles is the exact defect #445 was filed for.
fn settlement_posture_sentence(settlement_mode: &str, graded: bool) -> String {
    let settlement = match settlement_mode {
        "http" => "Credit is queued for on-chain settlement.",
        "dry_run" => {
            "Settlement is running in dry-run: no on-chain credit is issued."
        }
        _ => {
            "Credit is recorded but not settled: on-chain settlement is not \
             enabled on this deployment, so this figure stays pending."
        }
    };
    if graded {
        settlement.to_string()
    } else {
        format!(
            "{settlement} It is an estimate and may be revised as your \
             submissions are scored."
        )
    }
}

/// The currency view of `points`, or `None` when this deployment has no rate.
///
/// `None` is the whole point of the signature. A deployment without a rate has
/// made no claim about what a point is worth, and an absent key cannot be
/// misread the way a `0.00` can.
#[must_use]
pub fn currency_for(points: i64, rate: Option<&CreditRate>) -> Option<CurrencyBlock> {
    let rate = rate?;
    if !rate.points_per_unit.is_finite() || rate.points_per_unit <= 0.0 {
        return None;
    }
    let units = points as f64 / rate.points_per_unit;
    if !units.is_finite() {
        return None;
    }
    Some(CurrencyBlock {
        code: rate.code.clone(),
        earned_this_period: format!("{units:.2}"),
    })
}

/// Read the configured rate, or `None`.
///
/// Both variables are required together: a rate without a currency code names
/// no unit, and a code without a rate converts nothing.
#[must_use]
pub fn rate_from_env() -> Option<CreditRate> {
    let points_per_unit = std::env::var("TRACE_COMMONS_CREDIT_POINTS_PER_CURRENCY_UNIT")
        .ok()?
        .parse::<f64>()
        .ok()?;
    let code = std::env::var("TRACE_COMMONS_CREDIT_CURRENCY_CODE").ok()?;
    if code.trim().is_empty() {
        return None;
    }
    Some(CreditRate { points_per_unit, code })
}
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cargo test -p trace-commons-server --lib credit_numbers
```

Expected: PASS, five tests.

- [ ] **Step 6: Verify and commit**

```bash
cargo fmt --all
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
cargo clippy -p trace-commons-server --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
git add crates/trace-commons-server/src/credit_numbers.rs crates/trace-commons-server/src/lib.rs
git commit -m "Render credit figures in points, and in money only when told a rate

The ledger holds points. There is no points-to-currency conversion
anywhere in this repository, nothing has settled, and the pipeline that
would make a figure mean something is still shadow-mode. So a deployment
that has not configured a rate makes no claim about what a point is
worth, and the currency block is absent rather than zero -- a client
reading 0.00 would tell a contributor they earned nothing.

A nonsense rate converts nothing for the same reason: no figure beats an
invented one."
```

---

### Task 2: The contributor's own summary

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` — new handler beside `account_traces_list_handler` (around `:15059`), route registered beside `/v1/account/traces` (around `:7175`)

**Interfaces:**
- Consumes: `credit_numbers::{CreditPosture, CurrencyBlock, currency_for, rate_from_env}` (Task 1)
- Produces: `GET /v1/account/credit-summary` returning `AccountCreditSummary`

- [ ] **Step 1: Write the failing test**

In the ingest binary's extracted test module (`crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs`), following the shape of the neighbouring account-route tests:

```rust
#[tokio::test]
async fn credit_summary_reports_only_the_calling_account() {
    // The account's own figures, summed over its whole principal set: a
    // contributor with a device key AND a passkey sees one total, not one per
    // credential — and never anything belonging to another account.
    let fixture = account_fixture_with_two_principals().await;
    let response = fixture.get("/v1/account/credit-summary").await;
    assert_eq!(response.status(), StatusCode::OK);

    let body: serde_json::Value = response.json().await;
    assert_eq!(body["points"]["earned_this_period"], 1240);
    assert!(
        body.get("currency").is_none(),
        "no rate configured, so no currency block"
    );
    assert_eq!(body["posture"]["graded"], false);
    assert!(body["posture"]["explanation"].as_str().is_some());
    assert!(
        body.get("spent_this_period").is_none()
            && body["points"].get("spent_this_period").is_none(),
        "the server does not know anyone's inference spend and must not imply it does"
    );
}

#[tokio::test]
async fn credit_summary_requires_a_session() {
    let fixture = unauthenticated_fixture().await;
    let response = fixture.get("/v1/account/credit-summary").await;
    assert_ne!(response.status(), StatusCode::OK);
}
```

Use whatever fixture helpers the neighbouring account-route tests already use — find them by reading a test near `account_traces_list_handler`'s coverage and reuse those names. Do **not** write new fixture builders.

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p trace-commons-server credit_summary_reports_only
```

Expected: FAIL — the route does not exist (404), or the test helpers do not compile.

- [ ] **Step 3: Add the response types**

Beside the other account response structs in `trace-commons-ingest.rs`:

```rust
/// One contributor's own credit figures.
///
/// Deliberately small. It carries what this account earned and nothing that
/// could address another account: no principal refs, no hashes, no ids.
///
/// It also carries NO spend figure. The server sees submissions, not anyone's
/// inference bill, so a client that wants "credit covered N% of my spend"
/// composes it from its own local ledger. An API that reported your spend back
/// to you would have to be told it first.
#[derive(Debug, Serialize)]
struct AccountCreditSummary {
    points: AccountCreditPoints,
    #[serde(skip_serializing_if = "Option::is_none")]
    currency: Option<trace_commons_server::credit_numbers::CurrencyBlock>,
    posture: trace_commons_server::credit_numbers::CreditPosture,
    period: AccountCreditPeriod,
    /// Submissions held and not yet counted, so a contributor whose figure
    /// looks low has somewhere to look rather than a mystery.
    pending_review: usize,
}

#[derive(Debug, Serialize)]
struct AccountCreditPoints {
    earned_this_period: i64,
    lifetime_earned: i64,
}

#[derive(Debug, Serialize)]
struct AccountCreditPeriod {
    /// Stated explicitly rather than implied by "this period".
    start: chrono::DateTime<chrono::Utc>,
    end: chrono::DateTime<chrono::Utc>,
}
```

- [ ] **Step 4: Write the handler**

Beside `account_traces_list_handler`:

```rust
/// A contributor's own credit figures.
///
/// Scoped to `ctx.principal_set` — the account's active principals — because a
/// contributor with several credentials is one contributor. Reuses the same
/// expansion the credit read path already applies rather than a second notion
/// of who an account is.
async fn account_credit_summary_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
) -> ApiResult<Json<AccountCreditSummary>> {
    let period_end = chrono::Utc::now();
    let period_start = period_end - chrono::Duration::days(30);

    let (earned_this_period, lifetime_earned, pending_review) =
        account_credit_totals(state.as_ref(), &ctx.principal_set, period_start, period_end).await?;

    let rate = trace_commons_server::credit_numbers::rate_from_env();
    let settlement_mode = state.near_settlement_mode_label();

    Ok(Json(AccountCreditSummary {
        points: AccountCreditPoints { earned_this_period, lifetime_earned },
        currency: trace_commons_server::credit_numbers::currency_for(
            earned_this_period,
            rate.as_ref(),
        ),
        posture: trace_commons_server::credit_numbers::CreditPosture::current(
            &settlement_mode,
            false,
        ),
        period: AccountCreditPeriod { start: period_start, end: period_end },
        pending_review,
    }))
}
```

Implement `account_credit_totals` beside it with this signature:

```rust
/// Earned in the period, earned lifetime, and submissions still held.
///
/// Sums `points_delta` over credit-ledger rows whose `auth_principal_ref` is
/// in `principals`, through the same visibility helpers the account trace
/// routes use (`visible_credit_events` / `can_access_credit_event_scoped`
/// around `:54612`). Do not write raw SQL that goes around them: those helpers
/// are where "which rows belong to this account" is decided, and a second
/// answer to that question is a second thing to get wrong.
async fn account_credit_totals(
    state: &AppState,
    principals: &AccountPrincipalSet,
    period_start: chrono::DateTime<chrono::Utc>,
    period_end: chrono::DateTime<chrono::Utc>,
) -> ApiResult<(i64, i64, usize)>
```

Points are stored as a decimal `points_delta`; convert to whole points with the
same rounding the existing credit projections use rather than inventing one —
find it by reading how `credit_points_pending` is computed for
`TraceCommonsTenantCreditResponse` (around `:71059`).

If `state` has no existing accessor for the settlement mode label, add one that returns the configured mode string — do not read the environment variable a second time inside the handler.

- [ ] **Step 5: Register the route**

Beside `/v1/account/traces` in `app()`:

```rust
        .route(
            "/v1/account/credit-summary",
            get(account_credit_summary_handler),
        )
```

It must sit inside the same router layer that applies the account-session middleware — check how `/v1/account/traces` is wrapped and match it exactly. A route registered outside that layer would have no `AccountCtx` and fail at runtime, not compile time.

- [ ] **Step 6: Run the tests to verify they pass**

```bash
cargo test -p trace-commons-server credit_summary
```

Expected: PASS, two tests.

- [ ] **Step 7: Verify and commit**

```bash
cargo fmt --all
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
cargo clippy -p trace-commons-server --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
git add crates/trace-commons-server/src/bin/
git commit -m "Serve a contributor their own credit figures

Scoped to the account's principal set, because someone with a device key
and a passkey is one contributor and should see one total.

It carries no spend figure and will not gain one. The server sees
submissions, not anyone's inference bill; a client wanting to say credit
covered some share of what they spent composes that from its own local
ledger. An API that reported your spend back to you would have to be
told it first, and this one is not."
```

---

### Task 3: The aggregate view, its role, and its policy

The public endpoint has no tenant context, so the usual RLS predicate matches nothing. This task builds the only safe way through.

**Files:**
- Create: `migrations/V55__register_stats_public_read.sql`
- Modify: the hand-rolled `run_migrations` (find it with `grep -rn "run_migrations" crates/trace-commons-server/src/`) — a migration not wired in never runs
- Create: `crates/trace-commons-server/tests/register_stats_rls.rs`

**Interfaces:**
- Produces: view `trace_register_stats`, role `trace_commons_public_read`, and a role-scoped SELECT policy

- [ ] **Step 1: Write the failing test**

```rust
// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The public read path, exercised as the role that actually serves it.
//!
//! Every assertion here runs under `SET ROLE trace_commons_public_read`. Run
//! as the owner instead and all of them pass whether or not the policy exists
//! — which is exactly how a missing policy reaches production.

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_public_role_can_read_the_aggregate() {
    let pool = test_pool().await;
    let mut conn = pool.acquire().await.expect("connection");
    sqlx::query("SET ROLE trace_commons_public_read").execute(&mut *conn).await.expect("set role");

    let row = sqlx::query("SELECT traces_accepted, contributors FROM trace_register_stats")
        .fetch_one(&mut *conn)
        .await
        .expect("the public role reads the aggregate");
    let _: i64 = row.get("traces_accepted");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_public_role_can_read_nothing_else() {
    // The role exists to serve one view. If it can reach a table with rows in
    // it, the endpoint is one query change away from a leak.
    let pool = test_pool().await;
    let mut conn = pool.acquire().await.expect("connection");
    sqlx::query("SET ROLE trace_commons_public_read").execute(&mut *conn).await.expect("set role");

    for table in ["trace_submissions", "trace_credit_ledger", "trace_accounts"] {
        let result = sqlx::query(&format!("SELECT * FROM {table} LIMIT 1"))
            .fetch_optional(&mut *conn)
            .await;
        assert!(
            result.is_err() || result.unwrap().is_none(),
            "the public role reached {table}"
        );
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_public_role_does_not_bypass_rls() {
    let pool = test_pool().await;
    let row = sqlx::query(
        "SELECT rolbypassrls FROM pg_roles WHERE rolname = 'trace_commons_public_read'",
    )
    .fetch_one(&pool)
    .await
    .expect("the role exists");
    let bypass: bool = row.get("rolbypassrls");
    assert!(!bypass, "the public role must never bypass RLS");
}
```

Use the same PostgreSQL test-pool helper the other `tests/trace_corpus_pg_store.rs`-style tests use; do not write a new one. Mark them `#[ignore]` as those do — CI does not run PostgreSQL tests, so these are run deliberately.

- [ ] **Step 2: Run to verify they fail**

```bash
cargo test -p trace-commons-server --test register_stats_rls -- --ignored
```

Expected: FAIL — the role and view do not exist. (If PostgreSQL is not configured locally, the failure is a connection error; say so in your report rather than skipping the task.)

- [ ] **Step 3: Write the migration**

`migrations/V55__register_stats_public_read.sql`:

```sql
-- Aggregate register facts, and the least-privileged way to read them.
--
-- The public endpoint has no tenant, so `trace_current_tenant_id()` matches
-- nothing and the ordinary predicate returns an empty set. The answer is a
-- role that may read one aggregate and nothing else -- NOT `BYPASSRLS`, NOT a
-- superuser pool, and NOT dropping FORCE on a table, each of which trades a
-- narrow read for a broad hole.

CREATE TABLE trace_register_stats (
    singleton          BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    traces_accepted    BIGINT      NOT NULL DEFAULT 0,
    contributors       BIGINT      NOT NULL DEFAULT 0,
    points_issued      BIGINT      NOT NULL DEFAULT 0,
    withheld           BOOLEAN     NOT NULL DEFAULT TRUE,
    as_of              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- NULL until a refresh has actually computed this row. The endpoint
    -- publishes nothing while it is NULL: zeros would be a claim about the
    -- register that nobody made.
    refreshed_at       TIMESTAMPTZ
);

INSERT INTO trace_register_stats (singleton) VALUES (TRUE)
    ON CONFLICT DO NOTHING;

ALTER TABLE trace_register_stats ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_register_stats FORCE ROW LEVEL SECURITY;

CREATE ROLE trace_commons_public_read NOLOGIN NOBYPASSRLS;

GRANT SELECT (traces_accepted, contributors, points_issued, withheld, as_of, refreshed_at)
    ON trace_register_stats TO trace_commons_public_read;

-- Role-scoped rather than blanket: this row carries no tenant, so there is no
-- tenant predicate to write, and the grant above is what bounds the columns.
CREATE POLICY trace_register_stats_public_read
    ON trace_register_stats
    FOR SELECT
    TO trace_commons_public_read
    USING (TRUE);
```

A materialised single row, refreshed on a schedule, rather than a view over the
live tables: a view would let anyone poll the endpoint to watch one submission
land. Note `refreshed_at` is nullable and starts NULL — that is what lets the
endpoint refuse to publish before anything has computed the row.

- [ ] **Step 4: Add the refresh, or the table publishes zeros forever**

The table above starts at zeros with `refreshed_at` NULL. Nothing fills it yet,
and a published zero is a false claim about the register — worse than
publishing nothing. So the refresh ships in this task, not a later one.

Add a worker route following the existing worker pattern exactly — find one
(`grep -n '"/v1/workers/' crates/trace-commons-server/src/bin/trace-commons-ingest.rs`)
and copy its shape, including its own scoped bearer-token role. Worker-route
credentials are scoped per purpose in this repo and must not be mixed, so this
is a new scope, not a reuse of the utility or retention one.

```rust
/// Recompute the public aggregate row.
///
/// Batch-only and idempotent. It writes the single `trace_register_stats` row
/// and stamps `refreshed_at`; until it has run at least once the public
/// endpoint publishes nothing, because zeros would be a claim nobody made.
async fn register_stats_refresh_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<Json<RegisterStatsRefreshResponse>>
```

Counts come from the same projections the admin surfaces already use rather
than fresh SQL — `traces_accepted` from the accepted-submission projection,
`contributors` as the distinct count of credit accounts, `points_issued` as the
sum of positive `points_delta`. Reuse, so the public number and the operator
number cannot disagree.

Add a test that the row is NULL-stamped before a refresh and stamped after:

```rust
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_refresh_stamps_the_row_that_was_never_computed() {
    let pool = test_pool().await;
    let before = fetch_register_stats_row(&pool).await;
    assert!(before.refreshed_at.is_none(), "starts uncomputed");

    run_register_stats_refresh(&pool).await.expect("refresh");

    let after = fetch_register_stats_row(&pool).await;
    assert!(after.refreshed_at.is_some(), "a refresh stamps the row");
}
```

Do **not** add an in-process scheduler in this task. An operator wires the
route to a timer; say plainly in your report that nothing schedules it yet, so
that is a decision somebody makes rather than one that happens.

- [ ] **Step 5: Wire the migration in**

`run_migrations` is hand-rolled. Find it and add `V55` in the same form as `V54`. A migration file that is not listed there is never applied, and nothing fails to tell you.

- [ ] **Step 6: Run the tests to verify they pass**

```bash
cargo test -p trace-commons-server --test register_stats_rls -- --ignored
```

Expected: PASS, four tests, against a database with the migration applied.

- [ ] **Step 7: Verify and commit**

```bash
cargo fmt --all
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
git add migrations/V55__register_stats_public_read.sql \
        crates/trace-commons-server/tests/register_stats_rls.rs \
        crates/trace-commons-server/src/
git commit -m "Give the public read path a role instead of an exception

An unauthenticated request has no tenant, so the ordinary RLS predicate
matches nothing. The tempting fixes -- BYPASSRLS, a superuser pool,
dropping FORCE -- each trade a narrow read for a broad hole. This is a
NOBYPASSRLS role with a column grant and a role-scoped policy over one
materialised row, and it can reach nothing else.

The tests run under SET ROLE deliberately. As the owner they would pass
whether or not the policy exists, which is how a missing policy reaches
production."
```

---

### Task 4: The public endpoint, with a floor and a cache

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` — handler and route beside `/v1/source` (around `:7282`)

**Interfaces:**
- Consumes: `trace_register_stats` (Task 3), `CreditPosture` (Task 1)
- Produces: `GET /v1/public/register-stats`

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn register_stats_needs_no_credential() {
    let fixture = unauthenticated_fixture().await;
    let response = fixture.get("/v1/public/register-stats").await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn register_stats_withholds_counts_below_the_floor() {
    // With few contributors an aggregate identifies people: a known cohort
    // plus "points issued this week" is one person's earnings. Below the floor
    // the counts are absent and `withheld` says so -- never a small number,
    // and never a zero that reads as "nobody contributed".
    let fixture = fixture_with_contributors(2).await;
    let body: serde_json::Value = fixture.get("/v1/public/register-stats").await.json().await;
    assert_eq!(body["withheld"], true);
    assert!(body.get("contributors").is_none());
    assert!(body.get("points_issued").is_none());
}

#[tokio::test]
async fn register_stats_reports_counts_above_the_floor() {
    let fixture = fixture_with_contributors(50).await;
    let body: serde_json::Value = fixture.get("/v1/public/register-stats").await.json().await;
    assert_eq!(body["withheld"], false);
    assert_eq!(body["contributors"], 50);
}

#[tokio::test]
async fn register_stats_carries_no_identifying_field() {
    // The response is aggregate or it is nothing. Any of these appearing means
    // a breakdown was added that can be differenced back to a person.
    let fixture = fixture_with_contributors(50).await;
    let body: serde_json::Value = fixture.get("/v1/public/register-stats").await.json().await;
    let text = body.to_string();
    for forbidden in ["tenant", "account", "principal", "submission_id", "sha256:"] {
        assert!(!text.contains(forbidden), "response leaked {forbidden}");
    }
}
```

- [ ] **Step 2: Run to verify they fail**

```bash
cargo test -p trace-commons-server register_stats_
```

Expected: FAIL — route missing.

- [ ] **Step 3: Write the response type and handler**

```rust
/// The register's aggregate figures. No identity, at any aggregation, ever.
#[derive(Debug, Serialize)]
struct RegisterStats {
    traces_accepted: i64,
    /// Absent below the contributor floor. Absent, not zero: a zero here would
    /// read as "nobody has contributed" rather than "we are not saying".
    #[serde(skip_serializing_if = "Option::is_none")]
    contributors: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    points_issued: Option<i64>,
    /// True when the floor suppressed the counts above.
    withheld: bool,
    as_of: chrono::DateTime<chrono::Utc>,
    posture: trace_commons_server::credit_numbers::CreditPosture,
}

/// Aggregate register facts, to anyone.
///
/// Reads one materialised row through the public read role. It must never
/// query a live table: a figure computed per request is a figure someone can
/// poll to watch a single submission land.
async fn register_stats_handler(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<RegisterStats>> {
    let row = state.register_stats_row().await?;
    let floor = register_stats_contributor_floor();
    // Withheld below the floor, AND while the row has never been refreshed.
    // An unrefreshed deployment must publish nothing rather than zeros: zeros
    // are a claim about the register, and a wrong one.
    let stale = row.refreshed_at.is_none();
    let withheld = stale || row.contributors < floor;

    Ok(Json(RegisterStats {
        traces_accepted: row.traces_accepted,
        contributors: (!withheld).then_some(row.contributors),
        points_issued: (!withheld).then_some(row.points_issued),
        withheld,
        as_of: row.as_of,
        posture: trace_commons_server::credit_numbers::CreditPosture::current(
            &state.near_settlement_mode_label(),
            false,
        ),
    }))
}

/// Contributors below which the counts are withheld.
///
/// Configurable because the right number depends on the real contributor
/// count, and defaulted high because the failure mode of guessing low is
/// publishing one person's earnings.
fn register_stats_contributor_floor() -> i64 {
    std::env::var("TRACE_COMMONS_REGISTER_STATS_CONTRIBUTOR_FLOOR")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(25)
}
```

`traces_accepted` is reported regardless of the floor: it counts submissions,
not people, and does not difference back to an individual. It is still
suppressed while the row is stale, because an unrefreshed zero is a false claim
either way.

Add `AppState::register_stats_row()` beside the other state accessors:

```rust
/// The one materialised aggregate row, read through the public read role.
///
/// Reads a single row by primary key. It must never join a live table: a
/// figure computed per request is one that can be polled to watch a single
/// submission land.
async fn register_stats_row(&self) -> ApiResult<RegisterStatsRow>
```

where `RegisterStatsRow` carries `traces_accepted: i64`, `contributors: i64`,
`points_issued: i64`, `as_of: DateTime<Utc>` and `refreshed_at: Option<DateTime<Utc>>`.

- [ ] **Step 4: Register the route**

Beside `/v1/source` in `app()`, and — like it — **outside** every auth layer:

```rust
        .route("/v1/public/register-stats", get(register_stats_handler))
```

- [ ] **Step 5: Add cache headers and a per-IP limit**

The handler is unauthenticated and therefore abusable. Set `Cache-Control: public, max-age=300` on the response, and apply whatever per-IP rate limiting the codebase already uses for unauthenticated paths — find it by reading how `/v1/source` and `/health` are protected and follow that. If no such mechanism exists, say so in your report rather than inventing one.

- [ ] **Step 6: Run the tests to verify they pass**

```bash
cargo test -p trace-commons-server register_stats_
```

Expected: PASS, four tests.

- [ ] **Step 7: Verify and commit**

```bash
cargo fmt --all
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
cargo clippy -p trace-commons-server --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
git add crates/trace-commons-server/src/bin/
git commit -m "Publish what the register holds, and withhold what identifies

Aggregate figures to anyone, from one materialised row through the
public read role -- never a live query, because a figure computed per
request is one somebody can poll to watch a single submission land.

Below a contributor floor the counts are absent rather than small. With
few contributors a known cohort plus a total is one person's earnings,
and a zero would read as nobody having contributed rather than as us
declining to say. Trace count is reported either way: it counts
submissions, not people."
```

---

## Not in this plan

- **Any spend figure on any server response.** The server does not know it; see the spec correction above.
- **Redemption, settlement, or anything that moves credit.** Both endpoints are `GET`.
- **A configured rate on the pilot.** Leaving it unset is correct until the graded pipeline leaves shadow mode — the endpoints ship reporting points.
- **A scheduler for the refresh.** Task 3 ships the worker route; wiring it to a
  timer is an operator decision, and the endpoint publishes nothing until it has
  run, so an unscheduled deployment is visibly quiet rather than quietly wrong.
- **`by_harness`.** The spec flags it as the field most likely to want reshaping once a real client draws it. Add it when a client needs it, from a real screen.
