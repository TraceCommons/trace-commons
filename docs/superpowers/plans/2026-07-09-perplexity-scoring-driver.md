# Perplexity Scoring Driver Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax. Execution model: Sonnet 5 implementers, controller (Fable) reviews between tasks. PROCESS RULE for every implementer: run all cargo commands in the FOREGROUND and wait inline; never background builds/tests, never arm monitors, never end a turn to "wait". NEVER run bare `cargo fmt` on the server crate (it reformats 60k-line files) — format only your touched files and confirm `git status` shows only intended files before committing.

**Goal:** Build an in-process background loop in `trace-commons-ingest` that finds submissions lacking a gate decision, runs the enclave gate (NEAR AI Qwen3.6-35B perplexity + local novelty) per submission, and records the decision — so perplexity scoring actually runs on the pilot and backfills existing submissions. Floor stays 0 (non-gating).

**Architecture:** A dedicated `trace_gate_driver` Postgres role + pool answers the one cross-tenant question ("which submissions lack a gate decision"); everything else runs per-tenant via the existing `db_mirror`. The HTTP gate worker's core is extracted into a shared `evaluate_and_record_gate` fn used by both the handler and the loop. Cost controls: skip the 35B pass on precheck near-duplicates and reuse scores for content-identical resubmissions. Fail-safe: a scoring error records nothing, bumps a bounded attempt counter, and retries next tick — never rejects.

**Tech Stack:** Rust edition 2024, PostgreSQL, tokio, axum. Existing deps only. One new migration (V36). No new crates.

## Global Constraints

- No new external dependencies. Exactly one new migration file, `V36`, registered in `crates/trace-commons-server/src/db/postgres.rs` following the V23/V24/V25 guard+`batch_execute(include_str!)`+`INSERT` pattern.
- Every task verifies with `RUSTFLAGS="-D warnings"` for check/test on `trace-commons-server`; CI clippy allow-list: `-A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`; `cargo fmt -p trace-commons-server -- --check` on touched files only.
- Fail-safe, not fail-closed: a scoring failure records nothing, retries with backoff, and never changes a submission's accept/quarantine status. (Distinct from the gate's own fail-closed-for-gating behavior.)
- Hash-only/label-only logging: submission-id hash, tenant hash, attempt count, fixed error label. Never trace content, envelope bytes, or the NEAR AI response body. Reuse `safe_display_error_hash`.
- Driver is DISABLED by default (`TRACE_COMMONS_PERPLEXITY_DRIVER_ENABLED` unset/false) so CI and other deployments are unaffected. Floor calibration / gating is explicitly out of scope.
- The `trace_gate_driver` role stays `NOLOGIN NOBYPASSRLS`; its cross-tenant read is authorized only by a role-scoped permissive `USING (true)` SELECT policy, never BYPASSRLS.
- No emojis. Commit subjects: short imperative, no `feat:`/`fix:` prefixes.
- PG-gated tests use the `TRACE_COMMONS_PG_TEST_DATABASE_URL`/`DATABASE_URL` skip pattern (skip, not fail, when unset); CI never runs them but they must compile under `-D warnings`.

## Key facts (single source of truth; file:line at branch head, ingest = crates/trace-commons-server/src/bin/trace-commons-ingest.rs, store = crates/trace-commons-server/src/trace_corpus_storage.rs)

- Gate worker handler `gate_evaluate_worker_handler` (ingest.rs:44568-44711). Its non-auth core (to extract): `db.get_trace_submission(tenant_id, submission_id)`; `db.get_latest_active_trace_object_ref(tenant_id, submission_id, StorageTraceObjectArtifactKind::SubmittedEnvelope)`; build `EncryptedTraceArtifactReceipt { tenant_storage_ref: tenant_storage_ref(tenant_id), artifact_kind: TraceArtifactKind::ContributionEnvelope, object_key: object_ref.object_key, ciphertext_sha256: object_ref.content_sha256.strip_prefix("sha256:").unwrap_or(&..).to_string(), encrypted_at: object_ref.created_at }`; `artifact_store.read_encrypted_artifact(&tenant_storage_ref(tenant_id), &receipt)` (SYNC); base64 STANDARD decode `artifact.ciphertext_base64`; `artifact.wrapped_dek.clone().ok_or(...)`; `GateTenantCtx::from_canonical(tenant_storage_ref(tenant_id))`; `state.gate_service.evaluate_trace(&tenant_ctx, &ciphertext, &wrapped_dek, TraceArtifactKind::ContributionEnvelope)` (SYNC → `anyhow::Result<GateDecision>`); build `StorageTraceGateDecisionRow` (fields below); `db.insert_trace_gate_decision(tenant_id, row)`.
- `GateDecision` (trace_gate_service.rs:73-91): `gate_policy_version, gate_version_hash, perplexity_micros: u64, tail_fraction_micros: u64, perplexity_passed: bool, novelty_score_micros: u64, nearest_neighbor_hash: String, novelty_passed: bool, embedding_evidence_hash: String, attestation_chain_hash: String, vector_entry_id: Option<Uuid>`.
- `StorageTraceGateDecisionRow` = `TraceGateDecisionRow` (store:1770-1797): `decision_id: Uuid, submission_id: Uuid, gate_policy_version, gate_version_hash, perplexity_micros: i64, tail_fraction_micros: i64, perplexity_passed, novelty_score_micros: i64, nearest_neighbor_hash, novelty_passed, embedding_evidence_hash, attestation_chain_hash, decided_at: DateTime<Utc>, vector_entry_id: Option<Uuid>, credit_withheld_reason: Option<String>`. u64→i64 via `i64::try_from(x).unwrap_or(i64::MAX)`. NO tenant_id field (passed separately).
- `AppState` (ingest.rs:941-1086): `db_mirror: Option<Arc<dyn Database>>` (:951), `artifact_store: Option<ConfiguredTraceArtifactStore>` (:987), `gate_service: Arc<dyn TraceGateService>` (:1042, not Option). No cross-tenant pool exists — this plan adds one.
- Scheduler pattern (copy this): config struct `TraceVectorIndexSchedulerConfig` (:1173-1180); env parse `parse_trace_vector_index_scheduler_config_from_env()` (:5090-5125, returns `Ok(None)` when disabled); spawn `spawn_trace_vector_index_scheduler_task(state: &Arc<AppState>, config: Option<..>)` (:7775-7814) → `let Some(config)=config else {return}; let state=state.clone(); tokio::spawn(async move { loop { tokio::time::sleep(config.interval).await; match run_..._tick(state.clone(), &config).await { Ok(_)=>info, Err(_)=>warn(error_hash=safe_display_error_hash(..)) } } })`; bootstrap wiring at :899-925, config assembled into AppState at :2924-3182. Env helpers: `env_truthy`, `parse_optional_scheduler_u64_env(name, default, min, max)`.
- Store trait split: `TraceCorpusStore` (store:1806) has corpus CRUD; `Database: TraceCorpusStore` (db/mod.rs:110) adds `run_migrations` etc. `db_mirror: Arc<dyn Database>` has both. Existing reads: `get_trace_submission` (:1812), `list_trace_submissions` (:1818), `get_latest_active_trace_object_ref` (:1910), `insert_trace_gate_decision` (:2306), `list_trace_derived_records(tenant_id)` (:1922). NO existing "submissions without a gate decision" method.
- Precheck duplicate score: `trace_derived_records.duplicate_score: Option<f32>` (`TraceDerivedRecord`, store:1086-1111), read via `list_trace_derived_records(tenant_id)` filtered by `submission_id`. `novelty_score` also there.
- Migrations: 35 files, highest `V35__trace_instance_enrollments.sql`. Each is `include_str!`'d + version-gated in `db/postgres.rs` (V23 :838-849, V24 :858-869, V25 :878-889). RLS resolver fn `trace_current_tenant_id()` (V18). Table RLS pattern (V23 trace_gate_decisions:34-63): FK to `trace_tenants(tenant_id) ON DELETE CASCADE`, tenant_id in PK, `ENABLE`+`FORCE ROW LEVEL SECURITY`, policy `trace_corpus_tenant_isolation USING (tenant_id = trace_current_tenant_id()) WITH CHECK (...)`. Resolver-role + permissive-policy pattern (V30:144-179, V33:61-71): `CREATE ROLE <r> NOLOGIN NOBYPASSRLS; GRANT SELECT (...) ON <t> TO <r>; CREATE POLICY <p> ON <t> FOR SELECT TO <r> USING (true); ALTER ROLE <r> SET statement_timeout='2s';`.
- PG-gated test pattern (tests/trace_corpus_pg_rls.rs): `postgres_test_config()` reads `TRACE_COMMONS_PG_TEST_DATABASE_URL`/`DATABASE_URL`, returns None→skip. Tenant context via `tx.execute("SELECT set_config('trace_commons.trace_tenant_id', $1, true)", &[&tenant])`. Role-bypass guard `current_role_bypasses_trace_rls` skips when the connection bypasses RLS. `DatabaseConfig` has `login_resolver_url: Option<..>` (config::DatabaseConfig::login_resolver_url_from_env()).
- Login-resolver pool wiring (the pattern for the new driver pool): `DatabaseConfig.login_resolver_url`, `PgBackend` opens a second pool as the resolver role. Find it in `crates/trace-commons-server/src/db/trace_corpus_pg.rs` / `config.rs` and mirror it for `gate_driver_url`.

---

### Task 1: Migration V36 — attempts table + gate-driver role + permissive policies

**Files:**
- Create: `migrations/V36__trace_gate_driver.sql`
- Modify: `crates/trace-commons-server/src/db/postgres.rs` (register V36 beside V35's block)
- Test: `crates/trace-commons-server/tests/trace_corpus_pg_rls.rs` (PG-gated RLS test for the new table + role)

**Interfaces:**
- Produces: table `trace_gate_evaluation_attempts (tenant_id TEXT, submission_id UUID, attempts INT NOT NULL DEFAULT 0, last_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(), last_error_label TEXT, PRIMARY KEY (tenant_id, submission_id), FK tenant_id → trace_tenants ON DELETE CASCADE)` with `ENABLE`+`FORCE ROW LEVEL SECURITY` and the `trace_corpus_tenant_isolation` policy. Role `trace_gate_driver NOLOGIN NOBYPASSRLS` with `GRANT SELECT` + a `FOR SELECT TO trace_gate_driver USING (true)` permissive policy on each of `trace_submissions`, `trace_gate_decisions`, `trace_object_refs`, `trace_gate_evaluation_attempts`; `ALTER ROLE trace_gate_driver SET statement_timeout='5s'`.

- [ ] **Step 1: Write the migration SQL** `migrations/V36__trace_gate_driver.sql`:

```sql
-- Perplexity scoring driver: attempt bookkeeping table + cross-tenant reader role.

CREATE TABLE IF NOT EXISTS trace_gate_evaluation_attempts (
    tenant_id        TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    submission_id    UUID NOT NULL,
    attempts         INT NOT NULL DEFAULT 0,
    last_attempt_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_error_label TEXT,
    PRIMARY KEY (tenant_id, submission_id)
);

ALTER TABLE trace_gate_evaluation_attempts ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_gate_evaluation_attempts FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_gate_evaluation_attempts;
CREATE POLICY trace_corpus_tenant_isolation ON trace_gate_evaluation_attempts
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

-- Cross-tenant reader role for the perplexity scoring driver's enumeration query.
-- NOBYPASSRLS: the permissive USING(true) SELECT policies below are what authorize
-- reads, so the runtime/PUBLIC role stays fully tenant-isolated.
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_gate_driver') THEN
        CREATE ROLE trace_gate_driver NOLOGIN NOBYPASSRLS;
    END IF;
END $$;
ALTER ROLE trace_gate_driver SET statement_timeout = '5s';

GRANT SELECT ON trace_submissions TO trace_gate_driver;
GRANT SELECT ON trace_gate_decisions TO trace_gate_driver;
GRANT SELECT ON trace_object_refs TO trace_gate_driver;
GRANT SELECT ON trace_gate_evaluation_attempts TO trace_gate_driver;

DROP POLICY IF EXISTS trace_gate_driver_cross_tenant_read ON trace_submissions;
CREATE POLICY trace_gate_driver_cross_tenant_read ON trace_submissions
    FOR SELECT TO trace_gate_driver USING (true);
DROP POLICY IF EXISTS trace_gate_driver_cross_tenant_read ON trace_gate_decisions;
CREATE POLICY trace_gate_driver_cross_tenant_read ON trace_gate_decisions
    FOR SELECT TO trace_gate_driver USING (true);
DROP POLICY IF EXISTS trace_gate_driver_cross_tenant_read ON trace_object_refs;
CREATE POLICY trace_gate_driver_cross_tenant_read ON trace_object_refs
    FOR SELECT TO trace_gate_driver USING (true);
DROP POLICY IF EXISTS trace_gate_driver_cross_tenant_read ON trace_gate_evaluation_attempts;
CREATE POLICY trace_gate_driver_cross_tenant_read ON trace_gate_evaluation_attempts
    FOR SELECT TO trace_gate_driver USING (true);
```

(Verify the real table names for the object-refs table via `grep -n "CREATE TABLE.*trace_object" migrations/*.sql` — use the actual name, likely `trace_object_refs`; if different, use that name in the GRANT/POLICY and in Task 2's query.)

- [ ] **Step 2: Register V36 in `db/postgres.rs`.** Find the V35 registration block (`grep -n "V35\|35" crates/trace-commons-server/src/db/postgres.rs`) and add an identical guarded block for version 36:

```rust
if run_migration_guard(&client, 36).await? {
    client
        .batch_execute(include_str!("../../../../migrations/V36__trace_gate_driver.sql"))
        .await
        .context("applying V36__trace_gate_driver")?;
    record_migration(&client, 36, "trace_gate_driver").await?;
}
```

Match the EXACT helper names / include path the V35 block uses (the snippet above is the shape; copy V35's precise calls and relative `include_str!` path).

- [ ] **Step 3: Verify it compiles + apply against the local test DB**

Run: `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins`
Then, if `TRACE_COMMONS_PG_TEST_DATABASE_URL` is set locally, run the migration path via a PG test or `cargo test -p trace-commons-server --test trace_corpus_pg_rls -- --nocapture` and confirm no error. Report whether the PG path ran.

- [ ] **Step 4: Write the PG-gated RLS test** in `tests/trace_corpus_pg_rls.rs` (skip-when-unset pattern): insert a tenant + a `trace_gate_evaluation_attempts` row under tenant context; assert a connection with `SET ROLE trace_gate_driver` (and no tenant context) CAN `SELECT` it (permissive policy), while the default role with no tenant context canNOT (forced isolation). Mirror `assert_raw_sql_rls_filters_by_tenant_context` (:757) for structure; guard with `current_role_bypasses_trace_rls` skip.

- [ ] **Step 5: Run the test**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --test trace_corpus_pg_rls`
Expected: PASS or SKIP (no PG). Must compile clean under `-D warnings`.

- [ ] **Step 6: Commit**

```bash
git add migrations/V36__trace_gate_driver.sql crates/trace-commons-server/src/db/postgres.rs crates/trace-commons-server/tests/trace_corpus_pg_rls.rs
git commit -m "Add gate-driver role, attempts table, and permissive read policies"
```

---

### Task 2: Ungated-set enumeration on the gate-driver pool

**Files:**
- Modify: `crates/trace-commons-server/src/config.rs` (add `gate_driver_url` to `DatabaseConfig` + env reader)
- Modify: `crates/trace-commons-server/src/db/trace_corpus_pg.rs` (open the gate-driver pool; implement the enumeration query)
- Modify: `crates/trace-commons-server/src/trace_corpus_storage.rs` (declare the store method + `GateWorkItem`)
- Test: `tests/trace_corpus_pg_rls.rs` (PG-gated) + a unit test for the SQL-less parts if any

**Interfaces:**
- Produces:
  ```rust
  // trace_corpus_storage.rs
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct GateWorkItem { pub tenant_id: String, pub submission_id: Uuid }

  // on Database (db/mod.rs) — cross-tenant, uses the gate-driver pool:
  async fn list_submissions_needing_gate_decision(
      &self,
      now: DateTime<Utc>,
      max_attempts: i32,
      backoff_base_seconds: i64,
      limit: i64,
  ) -> Result<Vec<GateWorkItem>, DatabaseError>;
  ```
- Consumes: the `trace_gate_driver` role/policies from Task 1.
- `DatabaseConfig` gains `pub gate_driver_url: Option<SecretString>` set from `TRACE_COMMONS_GATE_DRIVER_DATABASE_URL` (mirror `login_resolver_url_from_env`). `PgBackend` opens a second pool as that role when the URL is set (mirror the login-resolver pool). If unset, `list_submissions_needing_gate_decision` returns `Err(DatabaseError::...)` with a clear "gate-driver pool not configured" message (the driver only runs when configured, so this is a startup misconfig).

**Enumeration SQL** (runs on the gate-driver pool, no tenant context — the permissive policy authorizes the cross-tenant read):

```sql
SELECT s.tenant_id, s.submission_id
FROM trace_submissions s
JOIN trace_object_refs o
  ON o.tenant_id = s.tenant_id
 AND o.submission_id = s.submission_id
 AND o.artifact_kind = 'submitted_envelope'
 AND o.invalidated_at IS NULL
 AND o.deleted_at IS NULL
LEFT JOIN trace_gate_decisions d
  ON d.tenant_id = s.tenant_id AND d.submission_id = s.submission_id
LEFT JOIN trace_gate_evaluation_attempts a
  ON a.tenant_id = s.tenant_id AND a.submission_id = s.submission_id
WHERE d.decision_id IS NULL
  AND COALESCE(a.attempts, 0) < $1
  AND (a.last_attempt_at IS NULL
       OR a.last_attempt_at + make_interval(secs => $2 * POWER(2, COALESCE(a.attempts,0))) <= $3)
ORDER BY s.received_at ASC
LIMIT $4;
```

Bind `$1=max_attempts`, `$2=backoff_base_seconds`, `$3=now`, `$4=limit`. Verify the exact artifact_kind enum text (`'submitted_envelope'`) and the object-refs column names (`invalidated_at`/`deleted_at`) against the schema; adjust to the real names.

- [ ] **Step 1: Write the failing PG-gated test** in `tests/trace_corpus_pg_rls.rs`: seed two tenants each with a submission + submitted-envelope object ref; give tenant A's submission a gate decision, leave tenant B's without one; assert `list_submissions_needing_gate_decision(now, 5, 30, 10)` (via the gate-driver pool) returns exactly tenant B's `GateWorkItem` and not tenant A's. Add a second submission with `attempts >= max_attempts` and assert it is excluded. Skip when no PG / role bypasses RLS.

- [ ] **Step 2: Run to verify failure**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --test trace_corpus_pg_rls list_submissions_needing -- --nocapture`
Expected: FAIL — method/pool not implemented (or SKIP if no PG; in that case rely on Step 4's compile check + implement, then note the PG path was not exercised locally).

- [ ] **Step 3: Implement** the config field + env reader, the second pool in `PgBackend`, the `GateWorkItem` type, the trait method on `Database`, and the query. Add the same `todo!`/unimplemented stub to any other `Database` impls (test doubles) so the workspace compiles — search `impl Database for` and `impl.*TraceCorpusStore`.

- [ ] **Step 4: Run to green + full compile**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --test trace_corpus_pg_rls list_submissions_needing && RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins && RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run`
Expected: PASS/SKIP; everything compiles (all `Database` impls updated).

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-server/src/config.rs crates/trace-commons-server/src/db/trace_corpus_pg.rs crates/trace-commons-server/src/trace_corpus_storage.rs crates/trace-commons-server/src/db/mod.rs crates/trace-commons-server/tests/trace_corpus_pg_rls.rs
git commit -m "Enumerate submissions needing a gate decision via the gate-driver pool"
```

---

### Task 3: Extract the shared `evaluate_and_record_gate` helper

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` (extract from `gate_evaluate_worker_handler` :44568-44711; refactor the handler to call it)
- Test: ingest test module (`trace_commons_ingest_internal/tests.rs`) — a unit test with a mock gate service

**Interfaces:**
- Produces:
  ```rust
  enum GateOutcome {
      Scored { decision_id: Uuid, perplexity_passed: bool, novelty_passed: bool, vector_entry_id: Option<Uuid> },
      SkippedDuplicate { decision_id: Uuid },
      Cached { decision_id: Uuid },
      Failed { label: String },
  }

  /// Non-auth core of the gate worker: fetch → decrypt → score → record.
  /// Used by both the HTTP handler and the in-process driver. Cost controls
  /// (skip_duplicates, cache) are applied by the caller-facing wrapper in Task 4;
  /// this fn always scores.
  async fn evaluate_and_record_gate(
      state: &AppState,
      tenant_id: &str,
      submission_id: Uuid,
  ) -> anyhow::Result<GateOutcome>
  ```
- The handler `gate_evaluate_worker_handler` keeps its auth (`authenticate_with_tenant_access_grant` + `require_vector_operator`) and credit-emission, then calls `evaluate_and_record_gate(state, &tenant.tenant_id, body.submission_id)` for the fetch→score→record core, mapping the `Scored`/error into the existing `TraceGateEvaluateWorkerResponse`. Behavior of the HTTP endpoint is preserved (same decision row written, same response). The credit-emission call (`attempt_emit_novelty_utility_credit`) stays in the handler (it needs `TenantAuth`); `evaluate_and_record_gate` returns the `decision`/passed flags the handler needs to decide credit. If threading credit cleanly is awkward, have `evaluate_and_record_gate` return the built `GateDecision` alongside the outcome; keep the row insert inside the helper.

- [ ] **Step 1: Write the failing test** in `trace_commons_ingest_internal/tests.rs`: build an `AppState` with an in-memory `Database`, an in-memory artifact store holding one encrypted fixture envelope, and `EnclaveGateService::mock_with_decryptor(...)` (or `InMemoryGateService`) as `gate_service`; call `evaluate_and_record_gate(&state, tenant_id, submission_id).await` and assert it returns `GateOutcome::Scored { .. }` AND that a `trace_gate_decisions` row now exists for the submission with `perplexity_passed == true` (floor 0). Reuse the existing gate-worker test's fixture wiring (search `gate_evaluate_worker` / `mock_with_decryptor` in the tests module for the setup to copy).

- [ ] **Step 2: Run to verify failure**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server evaluate_and_record_gate -- --nocapture`
Expected: FAIL — fn does not exist.

- [ ] **Step 3: Implement** the extraction: move the fetch→object-ref→decrypt→`evaluate_trace`→build-row→`insert_trace_gate_decision` block into `evaluate_and_record_gate`, returning `GateOutcome::Scored` (or `Failed { label }` on error, where `label` is a fixed string like `"gate-scoring-failed"` — never the raw error). Refactor the handler to call it. `SkippedDuplicate`/`Cached` variants are constructed only by Task 4's wrapper; here the fn always produces `Scored`/`Failed`.

- [ ] **Step 4: Run to green (helper test + the pre-existing gate-worker test)**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server evaluate_and_record_gate gate_evaluate -- --nocapture && RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins`
Expected: PASS — including any pre-existing gate-worker handler test (behavior preserved).

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-server/src/bin/trace-commons-ingest.rs crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs
git commit -m "Extract shared evaluate_and_record_gate from the gate worker handler"
```

---

### Task 4: Cost controls + attempt bookkeeping wrapper

**Files:**
- Modify: `crates/trace-commons-server/src/trace_corpus_storage.rs` + `db/mod.rs` + `db/trace_corpus_pg.rs` (attempts upsert method; a duplicate-score read helper if not already easy)
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` (the wrapper `score_one_submission`)
- Test: ingest test module

**Interfaces:**
- Produces:
  ```rust
  // Database method (per-tenant, RLS-scoped via db_mirror):
  async fn bump_gate_evaluation_attempt(&self, tenant_id: &str, submission_id: Uuid, now: DateTime<Utc>, error_label: &str) -> Result<i32, DatabaseError>; // upsert attempts+1, returns new attempts

  // ingest.rs wrapper that applies cost controls then delegates:
  struct PerplexityDriverKnobs { skip_duplicates: bool, skip_duplicate_threshold_micros: i64, max_attempts: i32 }
  async fn score_one_submission(state: &AppState, item: &GateWorkItem, knobs: &PerplexityDriverKnobs) -> GateOutcome
  ```
- `score_one_submission` logic:
  1. If `knobs.skip_duplicates`: read the submission's derived record (`list_trace_derived_records(tenant_id)`, find by `submission_id`); if its `duplicate_score` (as micros = `(dup * 1_000_000.0) as i64`) `>= knobs.skip_duplicate_threshold_micros` → insert a decision row marking a skipped-duplicate (perplexity_micros/tail 0, `perplexity_passed=true`, `credit_withheld_reason=Some("skipped_duplicate")`, novelty from the derived record) and return `SkippedDuplicate`. (Recording a decision removes it from the ungated set.)
  2. Else cache: look for an existing gate decision in the same tenant whose submission shares this submission's `canonical_summary_hash` (via `list_trace_submissions`/derived lookup + `list_trace_gate_decisions`... use the simplest correct path: if the tenant already has a decision for a submission with the same `canonical_summary_hash`, copy its perplexity/novelty into a new row) → `Cached`. If cache lookup is non-trivial to do correctly, implement it as: skip cache in v1 and return `Scored` by delegating (leave a `// cache: v1 delegates; see spec` note) — BUT the spec requires the cache, so implement the straightforward version: add a `find_gate_decision_by_canonical_hash(tenant_id, canonical_summary_hash)` read and copy on hit.
  3. Else delegate to `evaluate_and_record_gate(state, &item.tenant_id, item.submission_id)`. On `Failed { label }`, call `bump_gate_evaluation_attempt(tenant_id, submission_id, now, &label)` and return the `Failed` outcome; on success return it unchanged.

  Given the cache's added surface, if `find_gate_decision_by_canonical_hash` balloons the task, split the cache into its own follow-up and have v1 do skip-duplicate + delegate + attempt-bump, recording the cache as a Minor deferral in the report. Decide based on how cleanly the read fits the store trait.

- [ ] **Step 1: Write failing tests** (ingest test module): (a) a submission whose derived `duplicate_score` is above threshold → `score_one_submission` returns `SkippedDuplicate` and writes a decision row with `credit_withheld_reason == Some("skipped_duplicate")`, WITHOUT calling the scorer (use a mock gate service whose `evaluate_trace` panics/counts calls to prove it wasn't invoked); (b) a failing mock scorer → `score_one_submission` returns `Failed` and `bump_gate_evaluation_attempt` incremented attempts to 1.

- [ ] **Step 2: Run to verify failure**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server score_one_submission -- --nocapture`
Expected: FAIL.

- [ ] **Step 3: Implement** `bump_gate_evaluation_attempt` (upsert `ON CONFLICT (tenant_id, submission_id) DO UPDATE SET attempts = attempts+1, last_attempt_at = $now, last_error_label = $label`), the duplicate-score read, the optional cache read, and `score_one_submission`.

- [ ] **Step 4: Run to green**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server score_one_submission bump_gate_evaluation -- --nocapture && RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-server/src
git commit -m "Add cost controls and attempt bookkeeping to per-submission scoring"
```

---

### Task 5: The driver loop, config, and spawn wiring

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` (config struct + env parse + spawn + tick, following the vector-index scheduler pattern; wire into bootstrap + AppState)
- Test: ingest test module (loop-drains-backlog integration test with in-memory store + mock scorer)

**Interfaces:**
- Produces:
  ```rust
  #[derive(Clone)]
  struct PerplexityScoreDriverConfig {
      interval: StdDuration,
      batch_size: i64,
      knobs: PerplexityDriverKnobs,      // from Task 4
      backoff_base_seconds: i64,
  }
  fn parse_perplexity_score_driver_config_from_env() -> anyhow::Result<Option<PerplexityScoreDriverConfig>>;
  fn spawn_perplexity_score_driver_task(state: &Arc<AppState>, config: Option<PerplexityScoreDriverConfig>);
  async fn run_perplexity_score_driver_tick(state: Arc<AppState>, config: &PerplexityScoreDriverConfig) -> anyhow::Result<PerplexityDriverTickSummary>;
  struct PerplexityDriverTickSummary { scored: usize, skipped_duplicate: usize, cached: usize, failed: usize }
  ```
- Env (mirror the scheduler helpers): `TRACE_COMMONS_PERPLEXITY_DRIVER_ENABLED` (env_truthy; `Ok(None)` when unset), `..._INTERVAL_SECONDS` (default 45, min 5, max 86400 via `parse_optional_scheduler_u64_env`), `..._BATCH_SIZE` (default 5), `..._MAX_ATTEMPTS` (default 5), `..._SKIP_DUPLICATES` (env_truthy, default true), `..._SKIP_DUPLICATE_THRESHOLD_MICROS` (default 900000), `..._BACKOFF_BASE_SECONDS` (default 30). When `ENABLED` is set, also require `db_mirror` present and the gate-driver pool configured — else the tick logs a warning and no-ops (fail-safe; do not panic the process).
- Tick: `let items = state.db_mirror....list_submissions_needing_gate_decision(Utc::now(), max_attempts, backoff_base_seconds, batch_size).await?;` then `for item in items { let outcome = score_one_submission(state, &item, &config.knobs).await; tally }`; return the summary. Sequential (no concurrency). Spawn/loop copies the vector-index shape (`tokio::spawn`, `loop { sleep(interval); match run_tick {...} }`), logging summary on Ok and `error_hash` on Err.
- Wire: add `perplexity_score_driver: Option<PerplexityScoreDriverConfig>` to AppState (assembled in the env block like the other scheduler configs), call `spawn_perplexity_score_driver_task(&state, state.perplexity_score_driver.clone())` in the bootstrap beside the other `spawn_*` calls.

- [ ] **Step 1: Write the failing integration test** (ingest test module): AppState with in-memory store + mock scorer + a gate-driver enumeration returning 3 ungated submissions; call `run_perplexity_score_driver_tick(state, &config)` once with `batch_size=5`; assert the summary is `scored: 3` (or scored+skipped totaling 3) and that a second tick returns `scored: 0` (backlog drained — the first tick's decisions removed them from the ungated set). Use a call-counting mock scorer to assert it was invoked exactly 3 times (or fewer if some were skip-duplicate).

Note: the in-memory `Database` test double must implement `list_submissions_needing_gate_decision` sensibly (return items lacking a decision) for this test — implement that in the double as part of this task.

- [ ] **Step 2: Run to verify failure**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server perplexity_score_driver -- --nocapture`
Expected: FAIL — config/tick/spawn not implemented.

- [ ] **Step 3: Implement** the config struct, env parse, tick, spawn, AppState field, and bootstrap wiring.

- [ ] **Step 4: Run to green + full server verification**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server perplexity_score_driver && RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins && RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run`
Expected: PASS; everything compiles.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-server/src/bin/trace-commons-ingest.rs crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs
git commit -m "Add the in-process perplexity scoring driver loop and wiring"
```

---

### Task 6: Docs, env reference, and verification sweep

**Files:**
- Modify: `docs/operator/env-reference.md` (or the issuer/ingest env docs) — document the new env vars and the `TRACE_COMMONS_GATE_DRIVER_DATABASE_URL` pool
- Create/modify: `docs/operator/perplexity-scoring-driver.md` — a short runbook (enable flag, the gate-driver role/pool setup, how to verify scoring via `trace_gate_decisions`, that gating stays off until calibration)
- Modify: `docs/superpowers/specs/2026-07-09-perplexity-scoring-driver-design.md` (Status: Implemented)

**Steps:**

- [ ] **Step 1: Write the operator runbook** `docs/operator/perplexity-scoring-driver.md`. Pool/role setup (mirror `docs/operator/login-resolver-role.md`, which documents the identical `trace_login_resolver` mechanism): `trace_gate_driver` is `NOLOGIN`, so `TRACE_COMMONS_GATE_DRIVER_DATABASE_URL` connects as a dedicated LOGIN role that has been granted membership (`GRANT trace_gate_driver TO <login_role>`), and the pool issues `SET ROLE trace_gate_driver` on each connection (exactly as the login-resolver pool does — Task 2 mirrors that pool's connect logic). Then: the env vars; enabling on the pilot (`..._ENABLED=1`, restart ingest); verification query (`SELECT count(*) FROM trace_gate_decisions`); the explicit note that the floor stays 0 (scoring only, no gating) until the a27 calibration. Confirm from Task 2's implementation whether the pool does `SET ROLE` itself or relies on the URL's role — document what the code actually does.

- [ ] **Step 2: Document the env vars** in `docs/operator/env-reference.md` (or wherever ingest env is documented): all `TRACE_COMMONS_PERPLEXITY_DRIVER_*` vars with defaults, and `TRACE_COMMONS_GATE_DRIVER_DATABASE_URL`.

- [ ] **Step 3: Full verification sweep** (all must pass; paste outputs in the report):

```bash
cargo fmt -p trace-commons-server -- --check
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server evaluate_and_record_gate score_one_submission perplexity_score_driver
cargo clippy --workspace --all-targets -- -D warnings -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
```

- [ ] **Step 4: Mark the spec Implemented and commit**

```bash
git add docs/
git commit -m "Document the perplexity scoring driver and complete verification"
```
