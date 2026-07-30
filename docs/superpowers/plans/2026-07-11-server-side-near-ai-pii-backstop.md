# Server-side NEAR AI PII backstop — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax. Sonnet-class subagents implement tasks; the planning session reviews between tasks.

**Goal:** An async server-side pass that re-runs the NEAR AI prose-PII classifier over already-ingested trace envelopes, re-redacts residual PII, and holds each trace out of consumer reach until the pass completes.

**Architecture:** A new driver stage folded into the same in-process task family as the perplexity scoring driver. Ingest stores message-text traces in a new `AwaitingPiiBackstop` corpus status; a background tick loads each, runs the (already-fixed) chunked NEAR AI adapter over its prose fields, re-redacts, re-stores a rescrubbed envelope, then transitions status to `Accepted`/`Quarantined`. Because the hold is a non-`Accepted` status, every existing `status == Accepted` consumer gate holds it with no per-site edits.

**Tech Stack:** Rust (edition 2024), tokio, deadpool-postgres, `trace-commons-protocol` (feature `near-ai-privacy-filter`), PostgreSQL with forced RLS.

## Global Constraints

- PostgreSQL-only. Verify with `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins` and `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run`. Clippy is CI-enforced: `cargo clippy -p trace-commons-server --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`.
- Hash-only audit/logging: never log raw PII, span text, bearer tokens, URLs, or message bodies. Error surfaces use hashes or fixed labels.
- Fail-closed: enabled-but-misconfigured → refuse at boot with a safe missing-control name; a held trace never reaches `Accepted` until the backstop completes.
- Forced RLS on every new table; the cross-tenant reader role is `NOBYPASSRLS` + permissive `FOR SELECT ... USING (true)` policies (mirror V36). Writes go through the tenant-scoped runtime pool with tenant context.
- No emojis in commits/PRs/code. Short imperative commit subjects, no `feat:`/`fix:` prefix. Co-author trailer per repo convention.
- The server crate must enable the protocol `near-ai-privacy-filter` feature for this code.
- Next free migration number is **V38**.

---

### Task 1: Protocol — async prose re-redaction helper

**Files:**
- Modify: `crates/trace-commons-protocol/src/trace_contribution.rs`
- Modify: `crates/trace-commons-protocol/Cargo.toml` (only if the helper needs the feature gate; place the fn behind `#[cfg(feature = "near-ai-privacy-filter")]` — no dep change).

**Interfaces:**
- Produces: `pub const NEAR_AI_PII_BACKSTOP_PIPELINE_SUFFIX: &str = "near-ai-pii-backstop-v1";`
- Produces: `pub async fn rescrub_envelope_prose_pii_with(adapter: &dyn PrivacyFilterAdapter, envelope: &mut TraceContributionEnvelope) -> Result<(), TraceContributionError>` — runs `adapter.redact_text` over `events[*].redacted_content` and `outcome.human_correction` (message text only; NOT `structured_payload`), replaces each with the redaction's `redacted_text`, merges `summary`/`report` into `envelope.privacy` (redaction_counts, pii_labels_present), bumps `residual_pii_risk` monotonically (`max_residual_risk`), appends `+near-ai-pii-backstop-v1` to `redaction_pipeline_version` (idempotent — don't double-append), recomputes `redaction_hash`. On any adapter error, returns it (caller decides retry) and leaves the envelope unmutated.

This mirrors `rescrub_trace_envelope_with` (trace_contribution.rs:2703-2778) but async + prose-filter instead of the deterministic redactor. Reuse the existing helpers it calls: `max_residual_risk`, the pipeline-suffix append logic, and the redaction-hash recompute (see SERVER_RESCRUB_PIPELINE_SUFFIX usage at :2755-2777).

- [ ] **Step 1: Write the failing test** in the `#[cfg(test)]` module (feature-gated). Use a stub `PrivacyFilterAdapter` that redacts a known email to `[private_email]`:

```rust
#[cfg(feature = "near-ai-privacy-filter")]
#[tokio::test]
async fn backstop_reredacts_prose_and_marks_pipeline() {
    use crate::trace_contribution::*;
    struct Stub;
    #[async_trait::async_trait]
    impl PrivacyFilterAdapter for Stub {
        async fn redact_text(&self, text: &str)
            -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
            if text.contains("jane@example.com") {
                Ok(Some(SafePrivacyFilterRedaction {
                    redacted_text: text.replace("jane@example.com", "[REDACTED:private_email]"),
                    summary: SafePrivacyFilterSummary {
                        schema_version: 1, output_mode: "redacted_text_only".into(),
                        span_count: 1,
                        by_label: std::collections::BTreeMap::from([("private_email".into(), 1)]),
                        decoded_mismatch: false,
                    },
                    report: RedactionReport::default(),
                }))
            } else { Ok(None) }
        }
    }
    let mut env = sample_envelope_with_event_content("email jane@example.com now");
    rescrub_envelope_prose_pii_with(&Stub, &mut env).await.unwrap();
    assert!(env.events[0].redacted_content.as_deref().unwrap().contains("[REDACTED:private_email]"));
    assert!(env.privacy.redaction_pipeline_version.contains("near-ai-pii-backstop-v1"));
    assert!(env.privacy.pii_labels_present.iter().any(|l| l == "private_email"));
    // Idempotent suffix: running again does not double-append.
    rescrub_envelope_prose_pii_with(&Stub, &mut env).await.unwrap();
    assert_eq!(env.privacy.redaction_pipeline_version.matches("near-ai-pii-backstop-v1").count(), 1);
}
```
Add a `sample_envelope_with_event_content(&str) -> TraceContributionEnvelope` test helper if one does not already exist (mirror existing envelope test fixtures in this module).

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p trace-commons-protocol --features near-ai-privacy-filter --lib backstop_reredacts -v`
Expected: FAIL — `rescrub_envelope_prose_pii_with` not found.

- [ ] **Step 3: Implement** `rescrub_envelope_prose_pii_with` + the suffix const per the Interfaces block. Extract the "append suffix if absent + recompute hash + bump risk + merge counts" tail into a shared private helper if it cleanly factors out of `rescrub_trace_envelope_with`; otherwise inline, matching that function's exact field updates.

- [ ] **Step 4: Run tests**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-protocol --features near-ai-privacy-filter --lib`
Expected: PASS (all, including the new test).

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-protocol/src/trace_contribution.rs
git commit -m "Add async prose-PII re-redaction helper for the server backstop"
```

---

### Task 2: Migration V38 — attempts table, reader role, RLS

**Files:**
- Create: `migrations/V38__trace_pii_backstop.sql`

**Interfaces:**
- Produces: table `trace_pii_backstop` (tenant_id, submission_id, attempts, last_attempt_at, last_error_label; PK (tenant_id, submission_id)); role `trace_pii_backstop_driver`; permissive `FOR SELECT ... TO trace_pii_backstop_driver USING (true)` policies on `trace_submissions`, `trace_object_refs`, `trace_pii_backstop`.

- [ ] **Step 1: Write the migration** — clone `migrations/V36__trace_gate_driver.sql` verbatim structure, renaming table→`trace_pii_backstop`, role→`trace_pii_backstop_driver`. Include:

```sql
-- Server-side NEAR AI PII backstop: attempt bookkeeping table + cross-tenant reader role.

CREATE TABLE IF NOT EXISTS trace_pii_backstop (
    tenant_id        TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    submission_id    UUID NOT NULL,
    attempts         INT NOT NULL DEFAULT 0,
    last_attempt_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_error_label TEXT,
    PRIMARY KEY (tenant_id, submission_id)
);

ALTER TABLE trace_pii_backstop ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_pii_backstop FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_pii_backstop;
CREATE POLICY trace_corpus_tenant_isolation ON trace_pii_backstop
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_pii_backstop_driver') THEN
        CREATE ROLE trace_pii_backstop_driver NOLOGIN NOBYPASSRLS;
    END IF;
END $$;
ALTER ROLE trace_pii_backstop_driver SET statement_timeout = '5s';

GRANT SELECT ON trace_submissions TO trace_pii_backstop_driver;
GRANT SELECT ON trace_object_refs TO trace_pii_backstop_driver;
GRANT SELECT ON trace_pii_backstop TO trace_pii_backstop_driver;

DROP POLICY IF EXISTS trace_pii_backstop_driver_cross_tenant_read ON trace_submissions;
CREATE POLICY trace_pii_backstop_driver_cross_tenant_read ON trace_submissions
    FOR SELECT TO trace_pii_backstop_driver USING (true);
DROP POLICY IF EXISTS trace_pii_backstop_driver_cross_tenant_read ON trace_object_refs;
CREATE POLICY trace_pii_backstop_driver_cross_tenant_read ON trace_object_refs
    FOR SELECT TO trace_pii_backstop_driver USING (true);
DROP POLICY IF EXISTS trace_pii_backstop_driver_cross_tenant_read ON trace_pii_backstop;
CREATE POLICY trace_pii_backstop_driver_cross_tenant_read ON trace_pii_backstop
    FOR SELECT TO trace_pii_backstop_driver USING (true);
```

- [ ] **Step 2: Register RLS coverage.** Add `"trace_pii_backstop"` to `expected_trace_rls_tables()` in `crates/trace-commons-server/tests/trace_corpus_pg_rls.rs:168-202`, and add `"V38__trace_pii_backstop.sql"` to the migration-file list read at `trace_corpus_pg_rls.rs:1436-1459`, so the new table's FORCE-RLS is actually asserted (V36 was omitted from these — do NOT repeat that omission).

- [ ] **Step 3: Verify** the migration parses and the RLS test compiles.

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run`
Expected: compiles. (The pg RLS test itself requires a live PostgreSQL; CI does not run it. If a local PostgreSQL with the shared test DB is available, run `cargo test -p trace-commons-server --test trace_corpus_pg_rls` and confirm the new table asserts.)

- [ ] **Step 4: Commit**

```bash
git add migrations/V38__trace_pii_backstop.sql crates/trace-commons-server/tests/trace_corpus_pg_rls.rs
git commit -m "Add V38 trace_pii_backstop table, reader role, and RLS coverage"
```

---

### Task 3: `AwaitingPiiBackstop` corpus status variant

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` (`TraceCorpusStatus` enum + all match arms)
- Modify: `crates/trace-commons-server/src/trace_corpus_storage.rs` (`StorageTraceCorpusStatus` twin, if present)
- Modify wherever the status serializes to/from its DB/string form.

**Interfaces:**
- Produces: `TraceCorpusStatus::AwaitingPiiBackstop` (+ storage twin) with wire string `"awaiting_pii_backstop"`. It is **not** consumer-visible (never satisfies `status == Accepted`), and is distinct from `Quarantined` (reviewer surfaces must not pick it up).

- [ ] **Step 1: Write the failing test** — round-trip + gating:

```rust
#[test]
fn awaiting_pii_backstop_status_roundtrips_and_is_not_accepted() {
    let s = TraceCorpusStatus::AwaitingPiiBackstop;
    assert_eq!(s.as_wire_str(), "awaiting_pii_backstop"); // match the enum's existing accessor name
    assert_eq!(TraceCorpusStatus::from_wire_str("awaiting_pii_backstop"), Some(s));
    assert_ne!(s, TraceCorpusStatus::Accepted);
}
```
Use whatever the enum's actual (de)serialization accessors are — find them by reading the existing `TraceCorpusStatus` impl (search `enum TraceCorpusStatus`); mirror the pattern used by `Quarantined`.

- [ ] **Step 2: Run to verify failure** — Run the test; expect a non-exhaustive-match compile error listing every arm to update. That error list is your checklist.

- [ ] **Step 3: Implement** — add the variant and handle it in every match the compiler flags: serialization, `status_for_risk` (leave it as an internal state, never produced by risk directly), reviewer-eligibility (`ensure_review_decision_eligible` must continue to require `Quarantined` and reject `AwaitingPiiBackstop`), quarantine-queue filters (must not include it), and any `TraceCorpusStatus` → `StorageTraceCorpusStatus` conversions. Do NOT add it to any `is_export_eligible`/consumer path — its absence there is the hold.

- [ ] **Step 4: Run** — `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run` and the unit test above. Expected: compiles + passes, no non-exhaustive warnings.

- [ ] **Step 5: Commit**

```bash
git commit -am "Add AwaitingPiiBackstop corpus status held out of consumer paths"
```

---

### Task 4: Config — env knobs, driver config, reader pool

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` (env consts ~711, defaults ~928, config struct ~1322, parse fn ~5450, AppState field ~1159, spawn call ~1039)
- Modify: `crates/trace-commons-server/src/config.rs` (URL accessor, mirror `gate_driver_url_from_env` at :95)
- Modify: `crates/trace-commons-server/src/db/postgres.rs` (reader pool, mirror `gate_driver_pool` at :94/:236)

**Interfaces:**
- Produces: `PiiBackstopDriverConfig { interval: StdDuration, batch_size: i64, max_attempts: i32, backoff_base_seconds: i64 }`; `parse_pii_backstop_driver_config_from_env() -> anyhow::Result<Option<PiiBackstopDriverConfig>>` returning `None` when `TRACE_COMMONS_PII_BACKSTOP_ENABLED` is falsy OR the driver URL / NEAR AI key is unset; erroring at boot only when `_ENABLED=1` but a required secret is blank (fail-closed). AppState gains `pii_backstop_driver: Option<PiiBackstopDriverConfig>` and a `pii_backstop_driver_pool` (via the Database impl). Config accessor `pii_backstop_driver_url_from_env()`.

- [ ] **Step 1: Write the failing test** for the parse fn (env-var-driven; use a serial guard if the test suite has one for env tests):

```rust
#[test]
fn pii_backstop_config_off_when_disabled() {
    // With _ENABLED unset, config is None regardless of other vars.
    assert!(parse_pii_backstop_driver_config_from_env().unwrap().is_none());
}
```

- [ ] **Step 2: Run to verify failure** — function not found.

- [ ] **Step 3: Implement** the env consts (`TRACE_COMMONS_PII_BACKSTOP_ENABLED`, `_TICK_INTERVAL_SECONDS`, `_BATCH_SIZE`, `_MAX_ATTEMPTS`, `_BACKOFF_BASE_SECONDS`), defaults (interval 45, batch 5, attempts 5, backoff 30 — mirror perplexity), the config struct, and `parse_pii_backstop_driver_config_from_env` mirroring `parse_perplexity_score_driver_config_from_env` (ingest.rs:5450) exactly, using `env_truthy` + `parse_optional_scheduler_{u64,i64}_env`. Add the reader pool mirroring `gate_driver_pool` (postgres.rs:236-249) keyed on `TRACE_COMMONS_PII_BACKSTOP_DRIVER_DATABASE_URL`. Add the AppState field and populate it in the constructor. Enforce the fail-closed boot check (enabled ⇒ URL + `TRACE_NEAR_AI_PRIVACY_API_KEY` present, else `anyhow::bail!` with a safe label).

- [ ] **Step 4: Run** — `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run` + the unit test. Expected: PASS.

- [ ] **Step 5: Commit** — `git commit -am "Add PII backstop driver config, env knobs, and reader pool"`

---

### Task 5: DB — enumeration + attempt/status writes

**Files:**
- Modify: `crates/trace-commons-server/src/db/mod.rs` (trait methods)
- Modify: `crates/trace-commons-server/src/db/postgres.rs` (enumeration on reader pool)
- Modify: `crates/trace-commons-server/src/db/trace_corpus_pg.rs` (attempt bump + status transition on tenant pool)

**Interfaces:**
- Produces on the `Database` trait:
  - `async fn list_submissions_awaiting_pii_backstop(&self, now, max_attempts: i32, backoff_base_seconds: i64, limit: i64) -> Result<Vec<GateWorkItem>, DatabaseError>` — mirrors `list_submissions_needing_gate_decision` (postgres.rs:3581) but selects `trace_submissions` rows with `status = 'awaiting_pii_backstop'` LEFT JOIN `trace_pii_backstop` for the same attempts/backoff predicate; runs on `pii_backstop_driver_pool`, no tenant context.
  - `async fn bump_pii_backstop_attempt(&self, tenant_id, submission_id, now, error_label) -> Result<i32, DatabaseError>` — clone of `bump_gate_evaluation_attempt` (trace_corpus_pg.rs:5505) targeting `trace_pii_backstop`, tenant-scoped pool.
  - `async fn set_submission_status(&self, tenant_id, submission_id, status: StorageTraceCorpusStatus) -> Result<(), DatabaseError>` — if an equivalent doesn't already exist; used to transition `awaiting_pii_backstop` → `accepted`/`quarantined`. Search first — a status-update path likely exists for the reviewer flow; reuse it.

- [ ] **Step 1: Write failing tests** — pure SQL-shape unit tests aren't feasible without a DB; instead add a `#[cfg(test)]` test in `trace_corpus_pg` gated behind the pg-test harness (mirror existing `trace_corpus_pg` integration tests that require `TRACE_COMMONS_TEST_DATABASE_URL`). If no local DB, the compile-level gate is the deliverable; assert the trait methods exist via a `--no-run` build.

- [ ] **Step 2: Run** — `cargo test -p trace-commons-server --no-run`; expect missing-method errors.

- [ ] **Step 3: Implement** the three methods per the verbatim templates from the referenced line numbers.

- [ ] **Step 4: Run** — `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run`. Expected: compiles. Run `cargo test -p trace-commons-server --test trace_corpus_pg_store` if a local PostgreSQL is configured.

- [ ] **Step 5: Commit** — `git commit -am "Add PII backstop enumeration and attempt/status DB methods"`

---

### Task 6: Driver stage — tick + process-one

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs`

**Interfaces:**
- Consumes: Task 1 `rescrub_envelope_prose_pii_with`; Task 4 config + pool; Task 5 DB methods; `read_submission_record` (:52773), `read_envelope_by_record` (:52812), `store_envelope` (:49481), the reviewer-approve re-store pointer-update pattern (:33882-33892), `status_for_risk` (:49305).
- Produces: `spawn_pii_backstop_driver_task(&Arc<AppState>, Option<PiiBackstopDriverConfig>)` (mirror :8072) and `async fn run_pii_backstop_driver_tick(Arc<AppState>, &PiiBackstopDriverConfig) -> anyhow::Result<PiiBackstopDriverTickSummary>` (mirror :35551). Per item: build the NEAR AI adapter from env (`NearAiPrivacyFilterAdapter::build_from_env`) once per tick + run `run_privacy_filter_canary` before processing any real item (abort the tick on canary failure); load envelope → `rescrub_envelope_prose_pii_with` → on Ok: re-store (`store_envelope(.., "rescrubbed-envelope", ..)`) + mirror `RescrubbedEnvelope` ref (Task 7) + transition status via `status_for_risk(post_risk, accept_medium)` + delete/settle the `trace_pii_backstop` row (done); on Err: `bump_pii_backstop_attempt` (hash-only error label), leave status `AwaitingPiiBackstop`.

- [ ] **Step 1: Write the failing test** — an integration test using a wiremock NEAR AI server + the pg-test harness, asserting a seeded `AwaitingPiiBackstop` submission with residual PII becomes `Accepted` with a rescrubbed envelope after one tick, and a 5xx NEAR AI response leaves it `AwaitingPiiBackstop` with `attempts` bumped. Gate behind the same env the other pg integration tests use. (If no local DB, write the test now, mark it `#[ignore]` with a comment that it needs `TRACE_COMMONS_TEST_DATABASE_URL`, and rely on Task 8's in-memory-DB variant for CI coverage.)

- [ ] **Step 2: Run** — expect failure (functions missing / behavior absent).

- [ ] **Step 3: Implement** the spawn + tick + process-one, plus a `PiiBackstopDriverTickSummary { done, failed, held }` struct. Wire `spawn_pii_backstop_driver_task(&state, state.pii_backstop_driver.clone())` next to the perplexity spawn (:1039).

- [ ] **Step 4: Run** — `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run` + the test (or its non-DB variant). Expected: PASS.

- [ ] **Step 5: Commit** — `git commit -am "Add PII backstop driver tick and process-one loop"`

---

### Task 7: Ingest hold + RescrubbedEnvelope ref wiring

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs`

**Interfaces:**
- At submit (:11204-11236), after `rescrub_trace_envelope` + `status_for_risk`: if the computed status is `Accepted`, the backstop is enabled, and `envelope.consent.message_text_included`, override the stored status to `AwaitingPiiBackstop` and insert a `trace_pii_backstop` `pending` row (attempts=0) in the same tenant transaction. Otherwise unchanged.
- Wire the `RescrubbedEnvelope` object-ref write (Task 6's re-store) via `trace_object_ref_write_from_record(.., StorageTraceObjectArtifactKind::RescrubbedEnvelope, record, &envelope)` (the currently-unwritten enum, :42242).

- [ ] **Step 1: Write the failing test** — submit-path unit/integration: a Low-risk, message-text-included envelope with backstop enabled lands as `AwaitingPiiBackstop` (not `Accepted`) with a pending backstop row; with backstop disabled it lands `Accepted` as today; a Quarantined-risk envelope is unaffected.

- [ ] **Step 2: Run** — expect failure.

- [ ] **Step 3: Implement** the ingest override + the RescrubbedEnvelope ref write.

- [ ] **Step 4: Run** — `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server` (targeted) + `--no-run`. Expected: PASS.

- [ ] **Step 5: Commit** — `git commit -am "Hold message-text traces in AwaitingPiiBackstop and write rescrubbed refs"`

---

### Task 8: End-to-end + release-gate regression coverage

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs`

**Interfaces:** in-memory `Database` (the existing test double) variants so CI (no PostgreSQL) still covers the flow.

- [ ] **Step 1: Write tests** — (a) full happy path via the in-memory DB + wiremock NEAR AI: submit → `AwaitingPiiBackstop` → tick → `Accepted` + rescrubbed envelope carries `+near-ai-pii-backstop-v1` and the PII is gone; (b) **release-gate regression**: an `AwaitingPiiBackstop` submission is excluded from `is_export_eligible`, the ranker/utility/process-eval selection queries, and the export manifest — assert each returns empty for the held submission and non-empty once transitioned to `Accepted`; (c) fail path: NEAR AI 5xx → held + attempts bumped; (d) canary failure aborts the tick without mutating any submission.
- [ ] **Step 2: Run** — expect failures where behavior is missing (should mostly pass if Tasks 1-7 are correct; (b) is the guard that no consumer path leaks a held trace).
- [ ] **Step 3: Fix** any leak (b) reveals (a consumer path that treats `AwaitingPiiBackstop` as visible — should not happen if Task 3 kept it out of `Accepted`, but assert it).
- [ ] **Step 4: Run** — `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server`. Expected: PASS.
- [ ] **Step 5: Commit** — `git commit -am "Cover PII backstop end-to-end and release-gate hold"`

---

### Task 9: Config docs, env template, operator runbook

**Files:**
- Modify: `deploy/pilot-gcp/ingest.env.template`
- Modify: `docs/operator/env-reference.md`
- Create: `docs/operator/pii-backstop.md` (enable checklist: migration applied, `trace_pii_backstop_driver` role + login grant, `TRACE_COMMONS_PII_BACKSTOP_DRIVER_DATABASE_URL`, `TRACE_NEAR_AI_PRIVACY_API_KEY`, drill: seed a residual-PII envelope → confirm it round-trips `AwaitingPiiBackstop` → `Accepted`; re-drive stuck rows by resetting `trace_pii_backstop.attempts`).

- [ ] **Step 1: Write** the template additions (all new env vars, defaulted OFF) + the runbook.
- [ ] **Step 2: Verify** the pilot-bootstrap smoke script still passes: `scripts/operator/pilot-bootstrap-smoke.sh` (or the CI job) is unaffected (backstop OFF by default).
- [ ] **Step 3: Commit** — `git commit -am "Document PII backstop config and operator enable checklist"`

---

## Self-review notes

- **Spec coverage**: Task 1 (async re-redaction) ↔ spec "Data flow step 2" + "async prose-filter path"; Task 2 (migration/role/RLS) ↔ "State"/"Reader role"; Task 3 (`AwaitingPiiBackstop`) ↔ "Release mechanism"; Task 4 (config) ↔ "Configuration"; Tasks 5-6 (DB+driver) ↔ "Architecture"/"Fail posture"; Task 7 (ingest hold) ↔ "Data flow step 1"/"Hold"; Task 8 ↔ "Testing"; Task 9 ↔ "Rollout".
- **Fail-closed**: held via non-`Accepted` status (Task 3/7); boot refusal on missing secret (Task 4); canary-abort (Task 6); release-gate regression proof (Task 8b).
- **Naming consistency**: `rescrub_envelope_prose_pii_with`, `NEAR_AI_PII_BACKSTOP_PIPELINE_SUFFIX`, `AwaitingPiiBackstop`, `trace_pii_backstop`, `trace_pii_backstop_driver`, `PiiBackstopDriverConfig`, `run_pii_backstop_driver_tick`, `list_submissions_awaiting_pii_backstop`, `bump_pii_backstop_attempt` — used identically across tasks.
- **Open verification for the implementer**: confirm the exact `TraceCorpusStatus` (de)serialization accessor names before Task 3; confirm whether a submission status-update DB method already exists before adding one in Task 5.
