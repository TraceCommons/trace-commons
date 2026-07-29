# Devfolio Score Read-Back Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** A server-side worker route that returns per-`submission_id` scores across tenants, so devfolio can rank the hackathon competition.

**Architecture:** New scoped `CompetitionReadWorker` credential + gate; a cross-tenant `gate_driver_pool` store method reading the latest gate decision per submission; a `POST /v1/admin/scores-by-submission` handler that authenticates, gates, guards backend availability, reads, hash-only audits, and returns a score bundle. No migration (score columns V23/V37/V39 and gate-driver grants V36 already exist).

**Tech Stack:** Rust, axum, tokio-postgres, serde. Crate: `crates/trace-commons-server`. Main file `src/bin/trace-commons-ingest.rs` (abbrev **ingest.rs**), store `src/db/postgres.rs`, trait `src/trace_corpus_storage.rs`, extracted tests `src/bin/trace_commons_ingest_internal/tests.rs`.

## Global Constraints

- PostgreSQL-only. Verify with `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins` and `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run`. Clippy allow-list: `-A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`.
- Hash-only / label-only audit and logs: never log or store scores, submission ids, tenant, or content. Audit is surface label + count only.
- Fail-closed: FORBIDDEN on wrong role; 503 SERVICE_UNAVAILABLE when the cross-tenant backend is unconfigured.
- Scoped credential: `CompetitionReadWorker` is its own gate; do NOT mix it with export/utility/etc.
- Cross-tenant reads go ONLY through `gate_driver_pool` (no tenant GUC, SELECT-only). Never read cross-tenant through the runtime tenant pool.
- No new dependency. No emoji. Short imperative commit subjects, no `feat:`/`fix:` prefix. No new migration.
- Do NOT re-inline the extracted ingest test module; add tests to `trace_commons_ingest_internal/tests.rs`.

---

### Task 1: Add the `CompetitionReadWorker` role + gate

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` (`TokenRole` enum ~2616-2629; `TokenRole::parse` ~2632-2649; `storage_name` ~2667-2680; `trace_tenant_access_grant_role_for_token` ~47783-47795; add `require_competition_operator` ~after 48844)
- Modify: `crates/trace-commons-server/src/trace_corpus_storage.rs` (`TraceTenantAccessGrantRole` enum ~876-889)
- Modify (tests): `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs` (token seeding ~3891-3908; parse table `token_role_parses_worker_roles` ~4228-4282; add a gate test modeled on `benchmark_worker_routes_reject_reviewer_tokens_before_preconditions` ~19068)

**Interfaces:**
- Produces: `TokenRole::CompetitionReadWorker`; `fn require_competition_operator(auth: &TenantAuth) -> ApiResult<()>`; `TraceTenantAccessGrantRole::CompetitionReadWorker`.

- [ ] **Step 1: Write the failing parse test**

In `tests.rs`, extend `token_role_parses_worker_roles` (~4228) with rows asserting `TokenRole::parse("competition_read_worker")` and `parse("competition-read-worker")` both give `TokenRole::CompetitionReadWorker`, and `storage_name()` returns `"competition_read_worker"`.

- [ ] **Step 2: Run it — expect fail**

Run: `cargo test -p trace-commons-server --bin trace-commons-ingest token_role_parses_worker_roles 2>&1 | tail -20`
Expected: FAIL — no variant `CompetitionReadWorker`.

- [ ] **Step 3: Add the enum variant, parse arm, storage_name arm**

In ingest.rs `TokenRole` enum (~2628) add `CompetitionReadWorker,`. In `parse` (~2646) add before the fallback: `"competition_read_worker" | "competition-read-worker" => Ok(Self::CompetitionReadWorker),`. In `storage_name` (~2678) add `Self::CompetitionReadWorker => "competition_read_worker",`.

- [ ] **Step 4: Add the grant-role variant + map arm**

In `trace_corpus_storage.rs` `TraceTenantAccessGrantRole` (~888) add `CompetitionReadWorker` (keep the existing `#[serde(rename_all = "snake_case")]`; place the variant consistently with siblings). In ingest.rs `trace_tenant_access_grant_role_for_token` (~47794) add `TokenRole::CompetitionReadWorker => StorageTraceTenantAccessGrantRole::CompetitionReadWorker,`.

- [ ] **Step 5: Add the gate**

In ingest.rs, beside `require_process_evaluation_operator` (~48844), add:

```rust
fn require_competition_operator(auth: &TenantAuth) -> ApiResult<()> {
    if auth.role.can_admin() || auth.role == TokenRole::CompetitionReadWorker {
        Ok(())
    } else {
        Err(api_error(
            StatusCode::FORBIDDEN,
            "admin or competition read worker token required",
        ))
    }
}
```

Because `require_competition_operator` is not yet called by any route in this task, add `#[allow(dead_code)]` above it (Task 3 removes the allow when it wires the route) — OR skip the allow if the gate test in Step 7 references it (a test reference counts as a use; prefer that and omit the allow).

- [ ] **Step 6: Run parse test — expect pass; build under CI flags**

Run: `cargo test -p trace-commons-server --bin trace-commons-ingest token_role_parses_worker_roles 2>&1 | tail -20` (PASS)
Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run 2>&1 | tail -20` — the compiler flags any other exhaustive `match` over `TokenRole`/`TraceTenantAccessGrantRole` missing an arm (grep `TokenRole::RevocationWorker` and `TraceTenantAccessGrantRole::RevocationWorker` to find them); add the mirror arm at each. Expected: clean.

- [ ] **Step 7: Add the gate deny/admit test**

In `tests.rs`, seed a competition token in the `test_state` token map (~3908, mirroring `insert_token(&mut tokens, "tenant-a", "competition-read-worker-token-a", TokenRole::CompetitionReadWorker)`). Add a `#[test]` (modeled on the FORBIDDEN assertions at ~19087) that calls `require_competition_operator` directly with: a `TenantAuth` carrying `TokenRole::Reviewer` → `expect_err`, `.0 == StatusCode::FORBIDDEN`; one carrying `TokenRole::CompetitionReadWorker` → `Ok`; one carrying `TokenRole::Admin` → `Ok`. (Build a `TenantAuth` the way the neighboring gate tests do.)

- [ ] **Step 8: Run tests + commit**

Run: `cargo test -p trace-commons-server --bin trace-commons-ingest competition 2>&1 | tail -20` (PASS)
```bash
git add crates/trace-commons-server/src/bin/trace-commons-ingest.rs crates/trace-commons-server/src/trace_corpus_storage.rs crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs
git commit -m "Add CompetitionReadWorker role and gate"
```

---

### Task 2: Cross-tenant store method `list_scores_by_submission_ids`

**Files:**
- Modify: `crates/trace-commons-server/src/trace_corpus_storage.rs` (add trait method near `list_gate_decisions_for_credit_scoring`; add a result row struct)
- Modify: `crates/trace-commons-server/src/db/postgres.rs` (gate_driver impl near `list_gate_decisions_for_credit_scoring` ~3738)
- Modify: the in-memory test double that implements the storage trait (find it via the compiler error after adding the trait method; it is the double used by `test_state` — mirror how it implements `list_contributor_cap_signals`)
- Test: a store-level unit test beside the existing gate-driver store tests

**Interfaces:**
- Produces:
  ```rust
  #[derive(Debug, Clone)]
  pub struct TraceScoreBySubmissionRow {
      pub submission_id: Uuid,
      pub credit_quality_micros: Option<i64>, // q; None until the credit-quality pass runs
      pub perplexity_micros: Option<i64>,
      pub novelty_score_micros: Option<i64>,
      pub gate_passed: bool, // perplexity_passed && novelty_passed
  }
  ```
  Match the exact Rust types the existing gate-decision columns already use (check `list_gate_decisions_for_credit_scoring` for whether these are `i64`/`Option<i64>`/`bool` on the row it reads — mirror them). If `perplexity_passed`/`novelty_passed` are `Option<bool>`, treat NULL as `false` when computing `gate_passed`.
- Consumes: the `gate_driver_pool` client-acquisition pattern used by `list_gate_decisions_for_credit_scoring` (postgres.rs:3738) and `list_contributor_cap_signals` (:3806).

- [ ] **Step 1: Write the failing store test**

Add a `#[tokio::test]` (or the sync test style the sibling gate-driver tests use) that seeds the in-memory double with gate-decision rows for two submission ids under DIFFERENT tenants (one fully scored with `q`, one gated but `q` NULL) plus one id with two decisions at different `decided_at` (assert the LATEST is returned), then calls `list_scores_by_submission_ids(&[id1, id2, id_unknown])` and asserts: id1/id2 returned with correct fields, the latest decision wins, `q` is `Some`/`None` as seeded, and `id_unknown` is absent. Mirror the seeding/harness of the existing `list_contributor_cap_signals` test.

- [ ] **Step 2: Run it — expect fail**

Run: `cargo test -p trace-commons-server list_scores_by_submission 2>&1 | tail -20`
Expected: FAIL — method does not exist.

- [ ] **Step 3: Add the trait method + row struct**

In `trace_corpus_storage.rs`, add `TraceScoreBySubmissionRow` (shape above) and the trait method to the storage trait (beside `list_gate_decisions_for_credit_scoring`'s declaration):
```rust
async fn list_scores_by_submission_ids(
    &self,
    submission_ids: &[Uuid],
) -> Result<Vec<TraceScoreBySubmissionRow>, StorageError>;
```
(Use the exact `Result`/error alias the neighboring trait methods use.)

- [ ] **Step 4: Implement it on the Postgres gate-driver path**

In `postgres.rs`, near `list_gate_decisions_for_credit_scoring` (~3738), implement it: acquire a client from `gate_driver_pool` (NOT the runtime pool), set NO tenant GUC (copy the sibling's client-acquire lines and the "No tenant GUC: the trace_gate_driver role's permissive cross-tenant SELECT policies authorize this read" comment), and run:
```sql
SELECT DISTINCT ON (submission_id)
    submission_id,
    credit_quality_micros,
    perplexity_micros,
    novelty_score_micros,
    perplexity_passed,
    novelty_passed
FROM trace_gate_decisions
WHERE submission_id = ANY($1)
ORDER BY submission_id, decided_at DESC
```
Bind `submission_ids` as `&[Uuid]`. Map each row into `TraceScoreBySubmissionRow`, computing `gate_passed = perplexity_passed.unwrap_or(false) && novelty_passed.unwrap_or(false)` (adjust `.unwrap_or(false)` only if the columns are non-null `bool`). Return `Vec`. If `submission_ids` is empty, return `Ok(vec![])` without a query.

- [ ] **Step 5: Implement it on the in-memory test double**

The trait method addition breaks the in-memory double (compiler will point at it). Implement `list_scores_by_submission_ids` there mirroring how the double implements `list_contributor_cap_signals`/`list_gate_decisions_for_credit_scoring`: filter its stored gate-decision rows to the requested ids, keep the latest per submission by `decided_at`, project the score fields, ignore tenant (the real method is cross-tenant).

- [ ] **Step 6: Run the store test — expect pass; build under CI flags**

Run: `cargo test -p trace-commons-server list_scores_by_submission 2>&1 | tail -20` (PASS)
Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run 2>&1 | tail -20` (clean)

- [ ] **Step 7: Commit**

```bash
git add crates/trace-commons-server/src/trace_corpus_storage.rs crates/trace-commons-server/src/db/postgres.rs
git add -A crates/trace-commons-server/src
git commit -m "Add cross-tenant list_scores_by_submission_ids gate-driver read"
```

---

### Task 3: `POST /v1/admin/scores-by-submission` handler + route

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` (request/response types; `scores_by_submission_handler`; router registration ~6841-6847; remove any `#[allow(dead_code)]` on `require_competition_operator` from Task 1)
- Test: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs` (handler test)

**Interfaces:**
- Consumes: `require_competition_operator` (Task 1), `list_scores_by_submission_ids` (Task 2), `authenticate_with_tenant_access_grant`, `append_control_plane_read_audit`.
- Produces:
  ```rust
  #[derive(Debug, Deserialize)]
  struct ScoresBySubmissionRequest { submission_ids: Vec<Uuid> }

  #[derive(Debug, Serialize)]
  struct ScoreBySubmission {
      submission_id: Uuid,
      credit_quality_micros: Option<i64>,
      perplexity_micros: Option<i64>,
      novelty_score_micros: Option<i64>,
      gate_passed: bool,
  }
  #[derive(Debug, Serialize)]
  struct ScoresBySubmissionResponse { scores: Vec<ScoreBySubmission> }
  ```

- [ ] **Step 1: Write the failing handler test**

In `tests.rs`, add a `#[tokio::test]` modeled on the scoped-worker route tests (~19068) and the status-handler test pattern:
- Seed the in-memory double with a scored decision for `id_a` (some tenant) and no decision for `id_b`.
- Call `scores_by_submission_handler(State(state.clone()), auth_headers("competition-read-worker-token-a"), Json(ScoresBySubmissionRequest { submission_ids: vec![id_a, id_b] }))`.
- Assert: `Ok(Json(resp))`, `resp.scores` has exactly one entry (`id_a`) with the seeded score fields; `id_b` absent.
- Add a second assertion: calling with `auth_headers("review-token-a")` returns `expect_err` with `.0 == StatusCode::FORBIDDEN`.

- [ ] **Step 2: Run it — expect fail**

Run: `cargo test -p trace-commons-server --bin trace-commons-ingest scores_by_submission 2>&1 | tail -20`
Expected: FAIL — handler undefined.

- [ ] **Step 3: Add the types + handler**

In ingest.rs, add the request/response types (above) and the handler, modeled on `submission_status_handler` (~12360) for the id-cap + audit and on `recompute_contributor_caps_handler` (~46029) for the backend guard:

```rust
async fn scores_by_submission_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<ScoresBySubmissionRequest>,
) -> ApiResult<Json<ScoresBySubmissionResponse>> {
    let tenant = authenticate_with_tenant_access_grant(state.as_ref(), &headers).await?;
    require_competition_operator(&tenant)?;

    if body.submission_ids.len() > 500 {
        return Err(api_error(StatusCode::BAD_REQUEST, "too many submission ids (max 500)"));
    }
    if state.db_mirror.is_none() {
        return Err(api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "score read-back requires a configured cross-tenant read backend",
        ));
    }

    let rows = state
        .db_for_cross_tenant_reads()   // use the SAME accessor recompute uses to get the gate-driver-backed Database
        .list_scores_by_submission_ids(&body.submission_ids)
        .await
        .map_err(|_| api_error(StatusCode::INTERNAL_SERVER_ERROR, "score read-back failed"))?;

    append_control_plane_read_audit(state.as_ref(), &tenant, "scores_by_submission", rows.len()).await?;

    let scores = rows.into_iter().map(|r| ScoreBySubmission {
        submission_id: r.submission_id,
        credit_quality_micros: r.credit_quality_micros,
        perplexity_micros: r.perplexity_micros,
        novelty_score_micros: r.novelty_score_micros,
        gate_passed: r.gate_passed,
    }).collect();

    Ok(Json(ScoresBySubmissionResponse { scores }))
}
```
NOTE: use the exact accessor `recompute_contributor_caps_handler` uses to obtain the Database that routes to the gate driver (it references `db.list_contributor_cap_signals` — copy that `db` acquisition verbatim; do not invent `db_for_cross_tenant_reads`). Match the real `AppState` field name, the real `authenticate_with_tenant_access_grant` signature, and the real `append_control_plane_read_audit` signature/`.await?` usage.

- [ ] **Step 4: Register the route**

In the admin router chain (~6841-6847, beside `/v1/admin/recompute-contributor-caps`):
```rust
.route("/v1/admin/scores-by-submission", post(scores_by_submission_handler))
```
Remove any `#[allow(dead_code)]` left on `require_competition_operator`.

- [ ] **Step 5: Run the handler test — expect pass**

Run: `cargo test -p trace-commons-server --bin trace-commons-ingest scores_by_submission 2>&1 | tail -20` (PASS)

- [ ] **Step 6: Full verification**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run 2>&1 | tail -20` (clean)
Run: `cargo test -p trace-commons-server --bin trace-commons-ingest scores_by_submission competition 2>&1 | tail -20` (PASS)
Run: `cargo clippy -p trace-commons-server --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching 2>&1 | tail -5` (clean)

- [ ] **Step 7: Commit**

```bash
git add crates/trace-commons-server/src/bin/trace-commons-ingest.rs crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs
git commit -m "Add scores-by-submission read-back route for devfolio"
```

## Self-Review notes

- Spec coverage: role+gate → Task 1; cross-tenant score read → Task 2; route+audit+guard → Task 3. Auth (new scoped credential), cross-tenant (gate_driver only), fail-closed (FORBIDDEN + 503), hash-only audit, no migration — all honored.
- Score bundle: q headline nullable + perplexity/novelty + gate_passed; unknown ids omitted; gated-but-unscored distinguishable via `credit_quality_micros: null`.
- Risk to watch in review: exact `AppState`/accessor/signature names (the plan points at siblings rather than pasting unread 61k-LOC code — the implementer must match the real names, and the CI build enforces it).
