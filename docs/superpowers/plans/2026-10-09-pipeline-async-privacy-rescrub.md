# Pipeline privacy rescrub out of the receipt: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** draft, 2026-10-09. Written against branch `pipeline-async-privacy-rescrub` at `1e7ea3cc2` (spec commit on top of `main` `3c7d4a239`). Every line number below is a line in this worktree at that commit.

**Goal:** Move the prose-PII classifier out of `PipelineService::submit` (the HTTP receipt) and into a server-owned privacy pass at the start of the Review dispatch. The receipt keeps the bounded, local deterministic redactor only. The pass stores the classifier's output as its own encrypted object, records it on the run (V117), holds an escalated run for a human, retries classifier errors on a bounded budget, and hands the Review policy the scrubbed bytes. Score and exports keep reading approved content only.

**Architecture:** `PipelinePrivacyBoundary` splits into `rescrub_deterministic` (receipt) and `rescrub_classifier` (pass). The pass runs inside `process_claimed`'s `Phase::Review` arm, under Review's lease and attempt budget, before `bundle.review.execute`. Its record is six new nullable columns on `pipeline_runs` (storage option (a)). Its object uses the existing per-attempt key, staging row and sweep (FR1), with a run-derived object ref id. Escalation is the server's: it parks the run `awaiting_review` with `privacy_review_required`, the review queue predicate learns the pass outcome, and a human rejection of an escalated Admit run is committed by the server. The gate-api `Phase` enum, `ReviewInput`, the bundle and its qualification fixtures do not change.

**Tech Stack:** Rust (tokio, tokio-postgres, serde, anyhow; existing dependencies only), PostgreSQL 16 with forced RLS, Python 3 standard library (`scripts/operator/`).

**Spec:** `docs/superpowers/specs/2026-10-09-pipeline-async-privacy-rescrub-design.md` (cited as "spec:N"). Contracts: `docs/superpowers/specs/2026-09-11-versioned-pipeline-behavioral-contracts.md` (SUB-005 at :386-392, REV-001..004 at :535-585, RUN-004 at :821-833, CMP-002 at :1340, SCN-003 at :1471-1482).

**Abbreviations:** VP = `crates/trace-commons-server/src/versioned_pipeline.rs`; VPA = `crates/trace-commons-server/src/versioned_pipeline_authority.rs`; ING = `crates/trace-commons-server/src/bin/trace-commons-ingest.rs`; RT = `crates/trace-commons-server/tests/versioned_pipeline_runtime_pg.rs`; HTTP = `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_http_pg_tests.rs`; RESTORE = `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_restore_pg_tests.rs`; UPG = `crates/trace-commons-server/src/db/postgres/pipeline_upgrade_tests.rs`; BUNDLE = `crates/trace-commons-server/src/versioned_pipeline_bundle.rs`.

**Scope:** server crate, one migration (V117), docs and contracts. No protocol, client or gate-api change. Stage 3 stays as it is (see "Stage 3").

## Owner decisions applied (defaults given with the task)

- **D0 (open question 0, SUB-005).** Read as the spec reads it: "synchronous privacy-risk handling" is the bounded, local deterministic redactor. The contracts document gets a clarifying sentence, not an amendment (Task 10).
- **D1 (open question 1, High).** High is held for a human like Medium, never rejected by the pass. Note the asymmetry this leaves on purpose: `MinimalAdmissionPolicy` still rejects a receipt-time High with `privacy_risk_rejected` (BUNDLE:150-156); only a pass-time High is held.
- **D2 (open question 2, client status).** The receipt keeps `status: "processing"`. No client change and no new status value. The contributor-facing mapping (`main_status_for_pipeline`, ING:17405-17421) is left as it is; Task 10 documents what it shows (see "Compatibility mapping").
- **D3 (open question 3, retry budget).** Reuse the pipeline's own mechanism: the charged `mark_retry` and the run's `max_attempts` (V93 default 5, `migrations/V93__versioned_pipeline_durability.sql:16`). That count equals legacy's `TRACE_PII_BACKSTOP_DEFAULT_MAX_ATTEMPTS: i32 = 5` (ING:1153). The pipeline's backoff, however, is `DEFAULT_RETRY_MILLISECONDS = 50` doubling (VP:453, VP:5054-5056), which spends five attempts in about a second of classifier outage. So `mark_retry` gets a third label-keyed backoff branch, beside the existing hourly branch for `artifact_integrity_failed` (VP:5057), with legacy's shape: `30 s x 2^(attempt_count-1)`, from `TRACE_PII_BACKSTOP_DEFAULT_BACKOFF_BASE_SECONDS: i64 = 30` (ING:1154) and legacy's `make_interval(secs => base * POWER(2, attempts))` (`crates/trace-commons-server/src/db/postgres.rs:5470-5471`). The pass call is bounded by legacy's per-submission bound, `TRACE_PII_BACKSTOP_DEFAULT_PER_SUBMISSION_TIMEOUT_SECONDS: i64 = 900` (ING:920), which sits inside Review's renewal cap (5 min lease, VP:777, times `PIPELINE_LEASE_RENEWAL_CAP_FACTOR = 4`, VP:785). Legacy's per-tick canary and its consecutive-failure breaker (`MAX_CONSECUTIVE_PII_BACKSTOP_FAILURES = 3`, ING:45674) are tick-scoped and are not ported.
- **Storage option (a).** Six new columns on `pipeline_runs` in V117.

## Spec corrections (what the code map shows is wrong or impossible)

1. **"Keyed by content hash, so a repeated store is idempotent" (spec:110-111, 169-170) is impossible.** Ruling FR1 (VP:14587-14603) forbids content-addressed object keys: every encrypt uses a fresh salt and nonce, so two writes of equal plaintext differ in ciphertext, and a key shared across attempts would let a later write stop matching a committed ref's `content_sha256`. Resolution: the pass object key is `pipeline_attempt_object_id("privacy-pass", run_id, lease_token)`; the object ref id is derived from the run id alone (as `approved_object_ref`, VP:14814-14824); the attempt is staged in `pipeline_attempt_artifacts` and an uncommitted object is deleted by `sweep_attempt_artifacts`. Idempotency comes from "a pass recorded on the run is never re-run" plus the deterministic ref id. REV-002's "stored by content hash" is met by recording the plaintext hash (`privacy_pass_content_hash`), exactly as `approved_content_hash` does today. The spec test "stores no second object" becomes "commits one object ref; the orphaned attempt object is swept".
2. **"Through the same state and queue as an Admission quarantine" (spec:143-144) does not work as written.** The queue, claim and assessment paths all use `run_waiting_for_review_sql!`, which requires `p.admission_decision = 'quarantine'` (VP:120-124), and `admission_decision` is not updatable by the runtime (`migrations/V92__versioned_pipeline_runs.sql:149-157`). An escalated Admit run would park forever. Resolution: Task 6 widens the predicate with `OR p.privacy_pass_outcome = 'escalated'`.
3. **"A rejection ends the run as the reviewer's rejection does today" (spec:149-150) does not happen by calling the policy.** `MinimalReviewPolicy` ignores `human_assessment` when Admission admitted (`AdmissionDecision::Admit => (None, Vec::new())`, BUNDLE:192-194), so an escalated Admit run with a Reject assessment would be approved. Resolution: the server commits the rejection itself (Task 7), mirroring BUNDLE:204-221 with a server rule id. The server does not feed the policy a synthetic `AdmissionDecision::Quarantine`, because that would record an Admission reason Admission never gave.
4. **"After the retry budget is spent, the run fails with `privacy_classification_failed`" (spec:163-164) needs new code.** `mark_retry` always writes `attempts_exhausted` on exhaustion (VP:5076-5079, VP:5090), and the `claim_next` sweep does too (VP:2085). Also, `privacy_classification_failed` is not on the P2 allowlist (VP:10954-10966), so a bare error with that label would be recorded as `PIPELINE_OPERATIONAL_ERROR_LABEL` = `"minimal_policy_failed"` (VP:136). Resolution: Task 8.
5. **Option (b) "no schema change" (spec:196-197) is not available.** Staging the pass object needs a new `pipeline_attempt_artifacts.artifact` value, and V108 pins the set in a CHECK (`migrations/V108__versioned_pipeline_attempt_artifacts.sql:23`). Option (a) is taken anyway.
6. **Pass step 2 says `rescrub` (spec:109).** It is `rescrub_classifier` after the split.
7. **Line drift.** Spec "VPA:86" is the trait at VPA:87; spec "VP:11415" (Score's `load_approved_bytes` call) is correct in this worktree, but the loader itself is at VP:10300-10312.
8. **"Exports are not confirmed yet" (spec:117-119) is now confirmed.** The pipeline export snapshot selects by `r.approved_object_ref_id` and `approved_content_hash` (`crates/trace-commons-server/src/versioned_pipeline_product.rs:448-480`) and reads no bytes; the index rebuild reads only sealed index commands (VP:10441-10466); Score reads `load_approved_bytes` (VP:11415). Legacy readers that can reach a pipeline submission (`get_latest_active_envelope_object_ref`, ING:56028-56049) fail closed on the P1 wrapper (ING:68039-68044). No reader takes the source. One gap remains and is fixed in Task 9: the export filter `s.privacy_risk = $8` (`versioned_pipeline_product.rs:477`) reads the submission row, which the receipt now writes from the deterministic envelope only.

## Decisions made while writing this plan

- **P1. Receipt refusal label for a deterministic failure.** VP:9099 maps every boundary error to `privacy_classification_failed`. A deterministic failure is a `PrivacyFilterConfigError`, which legacy reports as "privacy filter config invalid" (ING:15317-15318). New constant `PIPELINE_PRIVACY_RESCRUB_FAILED_LABEL = "privacy_rescrub_failed"` in VPA beside VPA:22-23. `privacy_classification_failed` becomes a Review-phase run failure label only. pipeline-activation.md's refusal table (1620-1624) changes with it (Task 10).
- **P2. Pass object artifact kind: `ReviewSnapshot`, with `created_by_job_id = Some(run_id)`.** Discriminators: (a) the deletion worker must verify it as `ContributionEnvelope` (ING:65689-65696): `WorkerIntermediate` maps to `VectorPayload` and is then rejected unless `is_pipeline_score_object_ref` knows it (VP:14924-14930); (b) no legacy by-name selector may pick it up: `rescrubbed_envelope` is selected with `status = 'quarantined'` by `requeue_quarantined_for_pii_backstop` and the residual-survivor reset (`crates/trace-commons-server/src/db/trace_corpus_pg.rs:2861-2875, 2890-2915`), and the pipeline writes `quarantined` for an Admission quarantine (VP:6396-6412), so it would push pipeline submissions into legacy's backstop; `submitted_envelope` is the "latest active object" selector (`trace_corpus_pg.rs:2249`, and `postgres.rs:5332, 5387, 5464`); (c) a new `TraceObjectArtifactKind` variant is decoded by `enum_from_storage` (`trace_corpus_pg.rs:480, 993`), so a Route B rollback to the previous binary would fail to read any ref row the new binary wrote. `ReviewSnapshot` meets all three: legacy selects it by name nowhere ("review_snapshot" appears only at VP:2409 and ING:52944 outside tests). `created_by_job_id = Some(run_id)` plus the run-derived id tell it apart from the approved ref (which has `None`, VP:14836).
- **P3. Escalation is defined for Admission-admitted runs only.** `escalated` = Admission decided Admit AND the pass's mapped privacy risk is Medium or High. An Admission-quarantined run is already held by the policy (`review_assessment_required`, BUNDLE:196-203); its pass record is `cleared` or `escalated` by the same risk rule but the server does not double-park it, and its approval still resolves the Admission reason.
- **P4. Risk mapping is factored and reproduces the ConsentContentFlag downgrade.** The receipt's mapping (VP:9203-9216: `Medium` with basis exactly `[ConsentContentFlag]` maps to `Low`) moves into `fn pipeline_privacy_risk(risk: &ResidualPiiRisk, basis: &[ResidualRiskCondition]) -> PrivacyRisk`. The pass merges the receipt's basis (read back from `trace_submissions.residual_risk_basis` and parsed with `ResidualRiskCondition::from_label`, `crates/trace-commons-protocol/src/trace_contribution.rs:6348`) with the classifier's conditions, and maps the pass envelope's `residual_pii_risk` through the same helper. Classifier conditions defeat the downgrade exactly as they did when both passes ran at the receipt.
- **P5. The pass writes back to `trace_submissions`.** In the pass transaction, `privacy_risk` and `residual_risk_basis` are set from the pass envelope and merged basis (through `safe_residual_risk_basis_labels`, `crates/trace-commons-server/src/trace_corpus_storage.rs:1083`), so the export filter (versioned_pipeline_product.rs:477) and every status reader see post-classifier risk. `status` is not changed (an escalated run stays `received`; `main_status_for_pipeline` already reports `AwaitingReview` as `quarantined`, ING:17414-17418). The value written is the composed envelope risk the receipt used to store, so the semantics of that column return to what they were before this change.
- **P6. A worker without a privacy boundary.** The pass needs `self.privacy`. A missing boundary is a deployment gap, not the trace's fault: `privacy_control_missing` goes to the uncharged `mark_transient_retry` (FR3 shape, beside `PIPELINE_DEPENDENCY_MISSING_LABEL` at VP:10757-10759). It never falls back to running Review on the unscrubbed source.
- **P7. The pass runs once per run and is never re-run after it is recorded,** including after an assessment and after a restore. A crash before the record transaction repeats the classifier call (the spec allows this, spec:169).

## Stage 3 and rollout

- "Keep what we have for stage 3": nothing in this plan changes the stage 3 artifacts, the pilot (`a14eaff0`/V116, pipeline off) or the in-flight promote of `q5be633d6`. In particular the restore seed's existing two lifetimes and the `("leased", "settle", true)` assertion (RESTORE:2158-2165), which is the current promote blocker, are not edited. Task 11 adds a run beside them and is sequenced after that blocker is fixed on its own branch.
- The change ships as one promote cycle after stage 3: build, Route B deploy (V117), `qualify`, promote checks, sign, assemble, then requalify the pipeline tenant's bundle on the new revision (spec:244-250).
- Stage 3 acceptance for this change (spec:240-242): `long-chunk-capped` stored with status `processing` in under 2 s and reaching a terminal state; `pii-residual` still reads `quarantined` (now via the pass's `AwaitingReview`, which `main_status_for_pipeline` maps to `quarantined`), but only after at least one worker dispatch has run the pass. Between the receipt and that dispatch it reads `accepted` (`_ => "accepted"`, ING:17421), so a smoke check that reads status immediately after upload must wait for the worker first; otherwise the rerun looks like a regression.
- V117 adds nullable columns only, so existing pilot runs are valid. A run already past Review keeps NULL pass columns. A run at Review with no pass recorded runs the pass on its next dispatch. No constraint ties "approved" to "pass recorded" (it would fail on existing rows).

## Compatibility mapping (D2, documented, not changed)

`main_status_for_pipeline` (ING:17405-17421) maps: an escalated run parked `awaiting_review` to `quarantined` (good); a run waiting for its pass (state `pending`/`retry`, Admission admit) to `accepted`, which was already the case for any undecided admitted run; a run failed with `privacy_classification_failed` (submission `received`) to `accepted` (`_ => "accepted"`). The last one reads wrong to a contributor, but changing it is a vocabulary decision under D2. Task 10 documents it in pipeline-activation.md:2324 and lists it under "Questions for the owner".

## Global constraints

- TDD: each task writes its failing test first, runs it red, then implements.
- `RUSTFLAGS="-D warnings"` on every check and test build. Clippy with the repo allow-list only: `cargo clippy -p trace-commons-server --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`.
- `cargo fmt --all` before each commit (the repo is not rustfmt-clean everywhere; check the diff is confined to touched files).
- Hash-only and label-only: the pass record and every log line carry hashes, ref ids and labels; never envelope text, never classifier spans.
- Fail-closed: a classifier error never falls back to the deterministic result; a missing boundary never runs Review on the source.
- `trace-commons-gate-api` (`Phase`, `ReviewInput`, `AdmissionDecision`) and `versioned_pipeline_bundle.rs` are not edited.
- New `.rs` files (none planned) would need the AGPL header.
- PostgreSQL test database: a fresh database per session, on the literal host `127.0.0.1`:

```bash
createdb -h 127.0.0.1 admission_test_async_rescrub_1
export TRACE_COMMONS_PG_TEST_DATABASE_URL=postgres://$USER@127.0.0.1:5432/admission_test_async_rescrub_1
export TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL=postgres://tc_login_resolver_login@127.0.0.1:5432/admission_test_async_rescrub_1
```

## File map

| File | Change |
|---|---|
| `crates/trace-commons-server/src/versioned_pipeline_authority.rs` | Trait split (VPA:86-106), both impls (VPA:108-183), new label, unit tests (VPA:267-345) |
| `crates/trace-commons-server/src/versioned_pipeline.rs` | Receipt step 3 (VP:9081-9117) and docs (VP:7875-7877, 8968-8984, 9388-9389); risk helper (VP:9203-9216); `PipelineRunRecord` (VP:1013-1051) and `pipeline_run_from_row` (VP:7220-7256); crash points (VP:918-942); `PipelineAttemptArtifact` (VP:14476-14507); pass object ref helper (beside VP:14814); store `record_privacy_pass` (beside `commit_review`, VP:2360); review predicate (VP:115-125) and approve check (VP:2817-2832); `commit_review` derived record (VP:2425-2441); Review arm (VP:11207-11253); error routing (VP:10880-10900); `mark_retry` (VP:5048-5101) |
| `migrations/V117__pipeline_privacy_pass.sql` | New |
| `crates/trace-commons-server/src/db/postgres.rs` | `MIGRATIONS` row after the V116 row (postgres.rs:1758-1767); optional shape pin (postgres.rs:8073+) |
| `crates/trace-commons-server/src/db/postgres/pipeline_upgrade_tests.rs` | `RUNTIME_PIPELINE_GRANTS` (UPG:136-167); new `v117_*` test beside UPG:1564 |
| `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` | Review queue item (ING:~43404-43433) |
| `crates/trace-commons-server/tests/versioned_pipeline_runtime_pg.rs` | Doubles (RT:2363, 5713, 5761, 30387); inverted tests (RT:5723-5753, 5806); crash matrix (RT:14086-14096); new pass tests |
| `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_http_pg_tests.rs` | Doubles (HTTP:211, 7037, 7054); inverted leg (HTTP:7551-7598) |
| `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/production_assembly_tests.rs` | `ClassifyingPrivacy` (:638) |
| `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs` | `QualifiedTestPrivacy` (:10990) |
| `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_restore_pg_tests.rs` | Task 11 only |
| `docs/operator/pipeline-activation.md`, `docs/operator/deployment.md`, `docs/operator/pipeline-qualification.md` | Task 10 / 11 |
| `docs/superpowers/specs/2026-09-11-versioned-pipeline-behavioral-contracts.md`, `docs/superpowers/specs/2026-09-09-versioned-pipeline-design.md`, `docs/superpowers/specs/2026-09-11-versioned-pipeline-contract-test-manifest.json` | Task 10 |

No change: `versioned_pipeline_production.rs` (passes the boundary through; reads `privacy_classifies_prose_pii`, :907-909), `versioned_pipeline_production/gate_env.rs:484-505`, `versioned_pipeline_harness.rs:239-262`, `pipeline_runtime.rs:330-345`, `versioned_pipeline_bundle.rs`, gate-api.

## Interfaces (the names every task uses)

```rust
// VPA
pub const PIPELINE_PRIVACY_RESCRUB_FAILED_LABEL: &str = "privacy_rescrub_failed";
#[async_trait]
pub trait PipelinePrivacyBoundary: Send + Sync {
    /// Bounded, local; called at the receipt.
    async fn rescrub_deterministic(&self, envelope: &mut TraceContributionEnvelope)
        -> anyhow::Result<Vec<ResidualRiskCondition>>;
    /// The prose-PII classifier; called by the Review-start privacy pass only.
    async fn rescrub_classifier(&self, envelope: &mut TraceContributionEnvelope)
        -> anyhow::Result<Vec<ResidualRiskCondition>>;
    fn production_qualified(&self) -> bool { false }
    fn classifies_prose_pii(&self) -> bool { false }
}

// VP
pub const PIPELINE_PRIVACY_REVIEW_REQUIRED_LABEL: &str = "privacy_review_required";
const PIPELINE_PRIVACY_PASS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(900);
const PIPELINE_PRIVACY_RETRY_BASE_SECONDS: i64 = 30;
pub enum PipelineCrashPoint { /* existing */ AfterPrivacyPassArtifactStorage, AfterPrivacyPassCommit }
pub enum PipelineAttemptArtifact { Approved, IndexCommand, ScoreNeighbors, PrivacyPass } // "privacy-pass"
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyPassOutcome { Cleared, Escalated }
// PipelineRunRecord gains:
pub privacy_pass_object_ref_id: Option<Uuid>,
pub privacy_pass_content_hash: Option<String>,   // sha256: of the pass output (plaintext)
pub privacy_pass_source_hash: Option<String>,    // sha256: of the source bytes the pass read
pub privacy_pass_residual_risk_basis: Option<Vec<String>>, // labels, safe_residual_risk_basis_labels
pub privacy_pass_outcome: Option<PrivacyPassOutcome>,
pub privacy_pass_recorded_at: Option<DateTime<Utc>>,
fn pipeline_privacy_risk(risk: &ResidualPiiRisk, basis: &[ResidualRiskCondition]) -> PrivacyRisk;
fn privacy_pass_object_ref(run, receipt, size_bytes, object_store) -> TraceObjectRefWrite; // ReviewSnapshot, created_by_job_id Some(run_id)
impl PgPipelineStore {
    pub async fn record_privacy_pass(&self, run: &PipelineRunRecord, pass: PrivacyPassRecord<'_>)
        -> Result<PipelineRunRecord, DatabaseError>;
    pub async fn load_submission_residual_risk_basis(&self, tenant_id: &str, submission_id: Uuid)
        -> Result<Vec<String>, DatabaseError>;
}
impl PipelineService {
    async fn ensure_privacy_pass(&self, run: &PipelineRunRecord, admission: &AdmissionDecision)
        -> anyhow::Result<(PipelineRunRecord, Vec<u8>)>; // (run as recorded, pass output bytes)
    async fn load_privacy_pass_bytes(&self, run: &PipelineRunRecord) -> anyhow::Result<Vec<u8>>;
}
```

---

### Task 1: Split the privacy boundary trait

**Files:**
- Modify: VPA:86-106 (trait), VPA:108-118 (`DeterministicPipelinePrivacyBoundary`), VPA:120-183 (`ClassifierRedactorPipelinePrivacyBoundary` and its docs at :120-126, :175-178), VPA:22-23 (new label), VPA tests at :267-345.
- Modify (mechanical, compile only): RT:2363 `PassThroughPipelinePrivacyBoundary`, RT:30387 `QualifiedProductionPrivacy`, HTTP:211 `PassThroughPipelinePrivacyBoundary`, `production_assembly_tests.rs:638` `ClassifyingPrivacy`, `tests.rs:10990` `QualifiedTestPrivacy`: both methods `Ok(Vec::new())`.
- Modify (behavioural doubles): RT:5713 and HTTP:7054 `FailingPrivacyBoundary`: deterministic `Ok(Vec::new())`, classifier bails. RT:5761 and HTTP:7037 `MarkerRedactingBoundary`: deterministic no-op, the MARKER_SECRET replacement moves into `rescrub_classifier`.
- Modify: VP:9095-9105 call site, temporarily: `privacy.rescrub_deterministic(&mut envelope)` (Task 2 finishes the receipt).

Both methods are required, with no default and no provided composite `rescrub()`: a default no-op `rescrub_classifier` would let a classifying boundary skip the classifier while reporting `classifies_prose_pii() == true`, and a composite would let a future call site put the classifier back on the receipt.

- [ ] **Step 1: Write the failing tests** (in the VPA test module):
  - `deterministic_boundary_classifier_is_a_noop`: an envelope with a prose name ("Jane Doe") through `DeterministicPipelinePrivacyBoundary.rescrub_classifier` returns `Ok(vec![])` and leaves the envelope byte-identical (`serde_json::to_vec` equal before and after).
  - `classifier_boundary_deterministic_half_never_calls_the_adapter`: a `ClassifierRedactorPipelinePrivacyBoundary` over a counting adapter; `rescrub_deterministic` returns Ok and the adapter's call count is 0.
  - Rewrite `ordinary_identifier_prefixes_do_not_add_privacy_findings` (:284, call at :293): call `rescrub_deterministic` then `rescrub_classifier`, merge bases without duplicates, keep the assertions (basis `[ConsentContentFlag]`, Medium).
  - Rewrite `classifier_pii_is_transformed_and_quarantinable` (:306, call at :308): the redaction of "Jane Doe" and risk >= Medium come from `rescrub_classifier` on an envelope that already went through `rescrub_deterministic`.
  - Rewrite `classifier_failure_fails_closed` (:325, assert at :344): `rescrub_deterministic` is Ok; `rescrub_classifier` is Err with message `privacy_classification_failed`.
  - Keep `each_privacy_boundary_reports_what_it_is` (:267) unchanged; it must still pass.
- [ ] **Step 2: Run red.** `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --lib versioned_pipeline_authority` -> FAIL (no such methods).
- [ ] **Step 3: Implement.** Trait per "Interfaces"; doc on `classifies_prose_pii` (VPA:97-102) says "whether `rescrub_classifier` runs a prose-PII classifier". `Deterministic`: `rescrub_deterministic` = `rescrub_trace_envelope(envelope).map_err(Into::into)`; `rescrub_classifier` = `Ok(Vec::new())`. `ClassifierRedactor`: `rescrub_deterministic` = `rescrub_trace_envelope(envelope)?`; `rescrub_classifier` = `rescrub_envelope_prose_pii_with(self.adapter.as_ref(), envelope, self.policy).await.map_err(|_| anyhow!(PIPELINE_PRIVACY_CLASSIFICATION_FAILED_LABEL))`. The basis merge at VPA:160-164 moves to the callers. The classifier half does not re-run the deterministic redactor: `rescrub_envelope_prose_pii_with` already reconciles consent declarations, sweeps secrets over its own output and reads `envelope.privacy.residual_pii_risk` as its prior (trace_contribution.rs:5527-5529, 5566-5569, 5650-5672), so running it on the stored post-deterministic envelope reproduces today's in-order composition. Add `PIPELINE_PRIVACY_RESCRUB_FAILED_LABEL` beside VPA:23. Update every double listed under Files.
- [ ] **Step 4: Run green.** Same command -> PASS. Then:

```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
```

  Grep check: `grep -rn "\.rescrub(" crates/` returns nothing, and `grep -rn "impl PipelinePrivacyBoundary" crates/ | wc -l` is 9 (2 production + 7 doubles; VPA:111,148; RT:2363,5713,5761,30387; HTTP:211,7037,7054; production_assembly_tests.rs:638; tests.rs:10990 -- the HTTP file's three share names with RT's).
- [ ] **Step 5: Commit** `Split the pipeline privacy boundary into deterministic and classifier halves`.

### Task 2: The receipt runs the deterministic half only

**Files:**
- Modify: VP:9081-9105 (step 3 and its comment), VP:8968-8984 (`submit` doc), VP:9388-9389 (`precheck_receipt` comment), VP:7875-7877 (`with_privacy` doc), VP:9203-9216 (factor `pipeline_privacy_risk`).
- Test: RT (new tests beside `privacy_boundary_failure_fails_closed`, RT:5723), RT:5723-5753 (inverted), RT:5806 doc at :5800-5804 and comment at :5891-5893, RT:22193 doc.

- [ ] **Step 1: Write the failing tests** (RT):
  - `receipt_makes_no_classifier_call`: a double whose `rescrub_classifier` increments a counter and sleeps 60 s and whose `rescrub_deterministic` increments another counter. `tokio::time::timeout(Duration::from_secs(5), service.submit(..))` completes, returns the `processing` receipt, the classifier counter is 0, the deterministic counter is 1. A replay of the same key returns the stored receipt and both counters are unchanged.
  - `receipt_deterministic_failure_refuses_with_privacy_rescrub_failed`: a double whose `rescrub_deterministic` bails. `submit` errors with `privacy_rescrub_failed`; 0 runs, 0 staged artifacts, 0 files (the assertions `privacy_boundary_failure_fails_closed` makes today at RT:5747-5752, moved here). This keeps the receipt's fail-closed coverage.
  - `pipeline_privacy_risk_maps_like_the_receipt` (VP unit test): Low->Low; Medium + `[ConsentContentFlag]`->Low; Medium + `[ConsentContentFlag, FoundAndRemoved]`->Medium; High->High.
- [ ] **Step 2: Run red.**

```bash
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --lib pipeline_privacy_risk_maps_like_the_receipt
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --test versioned_pipeline_runtime_pg receipt_ -- --test-threads=1
```

  Expected: the first fails to compile (no helper); `receipt_makes_no_classifier_call` passes already after Task 1 (record that; it guards the regression), `receipt_deterministic_failure_refuses_with_privacy_rescrub_failed` fails on the label.
- [ ] **Step 3: Implement.** Map the deterministic error to `PIPELINE_PRIVACY_RESCRUB_FAILED_LABEL`. Factor VP:9203-9216 into `pipeline_privacy_risk` and call it. Rewrite the comments: the receipt never calls the classifier; the stored source is the post-deterministic envelope; the classifier runs in the Review-start pass. Rewrite `privacy_boundary_failure_fails_closed` (RT:5723-5753) as a Review-phase test in Task 8 (mark it `#[ignore = "rewritten in Task 8"]` only if Task 8 lands in the same PR; otherwise rewrite it now to assert the receipt succeeds with `processing` and creates one run). Fix the docs at RT:5800-5804, RT:5891-5893, RT:22193 ("transformed at receipt" -> "transformed by the privacy pass").
- [ ] **Step 4: Run green.** Commands of Step 2 -> PASS; then `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins`.
- [ ] **Step 5: Commit** `Run only the deterministic rescrub at the pipeline receipt`.

Note: after Task 2 and before Task 5, Review is fed the deterministic-only source. Do not ship between them; Tasks 2-9 land in one PR.

### Task 3: V117, the pass record on `pipeline_runs`

**Files:**
- Create: `migrations/V117__pipeline_privacy_pass.sql`.
- Modify: `crates/trace-commons-server/src/db/postgres.rs` (append the `(117, "pipeline_privacy_pass", include_str!(..))` row after the V116 row at :1762-1766, with a comment in the style of :1758-1761; optionally a V117 block in `versioned_pipeline_migration_shape_is_pinned`, :8073+, next to the V108 pin at :8228).
- Modify: UPG `RUNTIME_PIPELINE_GRANTS` (:136-167, add a `// V117` group of six columns to the `pipeline_runs` UPDATE list); check what the version list at UPG:406 means (`[92, ..., 113]`, V116 is absent) before adding 117 to it.
- Modify: VP:1013-1051 `PipelineRunRecord`, VP:7220-7256 `pipeline_run_from_row`.

- [ ] **Step 1: Look up V108's CHECK name** on a migrated test database (it is an inline column CHECK at V108:23, so PostgreSQL names it; expected `pipeline_attempt_artifacts_artifact_check`):

```bash
psql "$TRACE_COMMONS_PG_TEST_DATABASE_URL" -c "SELECT conname FROM pg_constraint WHERE conrelid = 'pipeline_attempt_artifacts'::regclass AND contype = 'c'"
```

- [ ] **Step 2: Write the failing test** `v117_adds_the_privacy_pass_record` in UPG beside `v116_adds_source_and_pipeline_run_id_with_defaults` (:1564), `#[ignore]` like its neighbours (CI selects them with `pipeline_upgrade -- --ignored`, `.github/workflows/ci.yml:416`). It asserts: the six columns exist and are NULL on a run inserted before V117; an UPDATE setting only some of them fails the shape CHECK; a `privacy_pass_outcome` outside `('cleared','escalated')` fails; a malformed hash fails; the FK to `trace_object_refs` is deferred (an insert of the ref after the run update in one transaction commits); `trace_ingest_runtime` holds UPDATE on the six columns (via the `RUNTIME_PIPELINE_GRANTS` check that `pipeline_upgrade_from_v91_installs_forced_rls_storage`, UPG:378, already runs); a `pipeline_attempt_artifacts` row with `artifact = 'privacy-pass'` inserts, and `'other'` still fails.
- [ ] **Step 3: Run red.**

```bash
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --lib pipeline_upgrade -- --ignored --test-threads=1
cargo test -p trace-commons-server --lib every_migration_is_wired_into_run_migrations
```

- [ ] **Step 4: Write the migration** (template: V95:4-26 and V95:52-60):

```sql
-- The Review-start privacy pass's record (spec 2026-10-09). A pass record,
-- not a classifier verdict (CMP-002): the object it stored, the hashes of
-- its input and output, the merged residual-risk labels, and whether it
-- held the run for a human. All six are NULL until the pass commits.
ALTER TABLE pipeline_runs
    ADD COLUMN privacy_pass_object_ref_id UUID,
    ADD COLUMN privacy_pass_content_hash TEXT CHECK (
        privacy_pass_content_hash IS NULL OR privacy_pass_content_hash ~ '^sha256:[0-9a-f]{64}$'),
    ADD COLUMN privacy_pass_source_hash TEXT CHECK (
        privacy_pass_source_hash IS NULL OR privacy_pass_source_hash ~ '^sha256:[0-9a-f]{64}$'),
    ADD COLUMN privacy_pass_residual_risk_basis JSONB CHECK (
        privacy_pass_residual_risk_basis IS NULL
        OR jsonb_typeof(privacy_pass_residual_risk_basis) = 'array'),
    ADD COLUMN privacy_pass_outcome TEXT CHECK (
        privacy_pass_outcome IS NULL OR privacy_pass_outcome IN ('cleared', 'escalated')),
    ADD COLUMN privacy_pass_recorded_at TIMESTAMPTZ,
    ADD CONSTRAINT pipeline_runs_privacy_pass_shape CHECK (
        (privacy_pass_object_ref_id IS NULL) = (privacy_pass_content_hash IS NULL)
        AND (privacy_pass_object_ref_id IS NULL) = (privacy_pass_source_hash IS NULL)
        AND (privacy_pass_object_ref_id IS NULL) = (privacy_pass_residual_risk_basis IS NULL)
        AND (privacy_pass_object_ref_id IS NULL) = (privacy_pass_outcome IS NULL)
        AND (privacy_pass_object_ref_id IS NULL) = (privacy_pass_recorded_at IS NULL)),
    -- NO ACTION + DEFERRABLE for the reason V95 gives for approved_object_ref_fk.
    ADD CONSTRAINT pipeline_runs_privacy_pass_object_ref_fk
        FOREIGN KEY (tenant_id, submission_id, privacy_pass_object_ref_id)
        REFERENCES trace_object_refs (tenant_id, submission_id, object_ref_id)
        ON DELETE NO ACTION
        DEFERRABLE INITIALLY DEFERRED;

ALTER TABLE pipeline_attempt_artifacts
    DROP CONSTRAINT <name from Step 1>,
    ADD CONSTRAINT pipeline_attempt_artifacts_artifact_check CHECK (
        artifact IN ('approved', 'index-command', 'score-neighbors', 'privacy-pass'));

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_ingest_runtime') THEN
        RAISE EXCEPTION 'V117: trace_ingest_runtime is missing; V90 creates it';
    END IF;
END $$;

-- pipeline_runs: the privacy pass records its result.
GRANT UPDATE (privacy_pass_object_ref_id, privacy_pass_content_hash,
              privacy_pass_source_hash, privacy_pass_residual_risk_basis,
              privacy_pass_outcome, privacy_pass_recorded_at)
    ON pipeline_runs TO trace_ingest_runtime;
```

  No RLS change: a column on an already-forced table inherits the tenant predicate (V52:40-44), and `TRACE_COMMONS_RLS_TABLES` (postgres.rs:188) lists tables only. No gate-driver grant. No new table, so `PIPELINE_TABLES` (RESTORE:~222-243) is unchanged. Before writing, check whether V108's `pipeline_attempt_artifacts_approved_hash` constraint (V108:42) ties a hash rule to `artifact = 'approved'` and whether the new value needs a matching clause.
- [ ] **Step 5: Wire the run record.** Add the six fields (Interfaces) and map them in `pipeline_run_from_row` (JSONB labels decode the way the V52 reader does). Every store read is `SELECT *`/`RETURNING *` (VP:1990, 2182, 2515, 3016, ...), so nothing else changes; the one test literal (RT:22291) uses `..run.clone()`. `PipelineRunRecord` derives `Serialize`: check every JSON surface that serialises it carries only hashes and labels (it does: the new fields are ids, hashes, labels, a timestamp).
- [ ] **Step 6: Run green.** Commands of Step 3 -> PASS; `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run`.
- [ ] **Step 7: Commit** `Add the privacy pass record to pipeline_runs (V117)`.

### Task 4: Pass object, crash points, store method

**Files:**
- Modify: VP:14476-14507 (`PipelineAttemptArtifact::PrivacyPass`, `as_str` "privacy-pass", `from_db_str`, `store_kind` `ContributionEnvelope`), the sweep comment at VP:10050-10057 ("the three known artifacts"), VP:14587 doc (lists the artifact names).
- Modify: beside VP:14814, `privacy_pass_object_ref` (id `Uuid::new_v5(&Uuid::NAMESPACE_URL, format!("tracecommons:pipeline-privacy-pass-object:{run_id}"))`, kind `ReviewSnapshot`, `created_by_job_id: Some(run.run_id)`).
- Modify: VP:918-942 `PipelineCrashPoint` (+2 variants; fix the stale doc at :918-921).
- Modify: new `PgPipelineStore::record_privacy_pass` beside `commit_review` (VP:2360) and `load_submission_residual_risk_basis`.

- [ ] **Step 1: Write the failing tests.**
  - VP unit: `privacy_pass_artifact_round_trips` (`from_db_str(as_str())`, store kind `ContributionEnvelope`); `privacy_pass_object_ref_is_derived_from_the_run` (same id for two receipts of one run; differs from the approved ref id; kind `ReviewSnapshot`; `created_by_job_id == Some(run_id)`); `privacy_pass_ref_is_not_a_score_object` (`!is_pipeline_score_object_ref(id, Some(run_id))`, so the deletion worker takes the `ContributionEnvelope` arm, ING:65689-65696).
  - RT `record_privacy_pass_commits_once_under_the_lease`: claim a Review run, stage a `PrivacyPass` attempt artifact, call `record_privacy_pass`; the run row has all six columns, the ref row exists, the staged row is `committed`, `attempt_count` and `next_phase` unchanged. A second call with the same lease is refused (`privacy_pass_object_ref_id IS NULL` fence) and changes nothing. A call with a stale lease token is refused. A call for a withdrawn submission is refused with `submission_inoperable`. It also writes `trace_submissions.privacy_risk` and `residual_risk_basis` (P5).
- [ ] **Step 2: Run red.** `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --lib privacy_pass_` and `cargo test -p trace-commons-server --test versioned_pipeline_runtime_pg record_privacy_pass -- --test-threads=1` -> FAIL.
- [ ] **Step 3: Implement** `record_privacy_pass`, modelled on `commit_review`'s single tenant transaction (VP:2360-2560): `ensure_current_lease` (lock order: run row, then submission row, as at VP:2374-2376); `review_submission_is_operable`; `INSERT INTO trace_object_refs ... ON CONFLICT DO NOTHING` (as VP:2410); `UPDATE trace_submissions SET privacy_risk = $, residual_risk_basis = $, updated_at = NOW()` (labels through `safe_residual_risk_basis_labels`); `UPDATE pipeline_runs SET privacy_pass_* = ..., privacy_pass_recorded_at = NOW(), updated_at = NOW() WHERE tenant_id AND run_id AND lease_token = $ AND lease_expires_at > NOW() AND privacy_pass_object_ref_id IS NULL RETURNING *`; mark the `privacy-pass` staged row committed (as `commit_review` does after VP:2528). No `attempt_count` reset, no phase change, no `lock_runnable_policy` (the pass is a server control, not a bundle policy). `load_submission_residual_risk_basis` reads `trace_submissions.residual_risk_basis` in a tenant transaction.
- [ ] **Step 4: Run green**, then `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins`.
- [ ] **Step 5: Commit** `Add the privacy pass object, crash points and record transaction`.

### Task 5: The privacy pass in the Review dispatch

**Files:**
- Modify: VP:11216-11244 (Review arm), new `ensure_privacy_pass` and `load_privacy_pass_bytes` beside `load_source_bytes` (VP:10292-10294); `load_privacy_pass_bytes` goes through `load_object_bytes` (VP:10238-10289, which already enforces operability, invalidation and deletion) and checks `sha256_prefixed(bytes) == privacy_pass_content_hash` like `load_approved_bytes` (VP:10300-10312), failing with `artifact_integrity_failed`.
- Modify: VP:2425-2441 (`commit_review`'s `trace_derived_records` insert: `input_object_ref_id` becomes `run.privacy_pass_object_ref_id`, which is always set by then, so it names the object whose hash it stores in `input_hash`).
- Test: RT, MarkerRedacting tests (RT:5806, RT:~22201, HTTP:7223/7272/7333).

Flow of `ensure_privacy_pass(run, admission)`:
1. If `run.privacy_pass_object_ref_id` is set: return `load_privacy_pass_bytes(run)`; no classifier call.
2. Else: `privacy = self.privacy.as_ref()` or `privacy_control_missing` (P6). `source = load_source_bytes(run)`; `source_hash = sha256_prefixed(&source)`; deserialize `TraceContributionEnvelope` from `source` (the receipt stores `serde_json::to_vec(envelope)`, VP:9123); `tokio::time::timeout(PIPELINE_PRIVACY_PASS_TIMEOUT, privacy.rescrub_classifier(&mut envelope))`; an error or timeout raises `PolicyError::permanent(privacy_classification_failed)` (Task 8 routes it).
3. Merge: receipt basis (`load_submission_residual_risk_basis`, parsed with `ResidualRiskCondition::from_label`; an unknown label fails closed with `privacy_classification_failed`) plus classifier conditions, no duplicates. `risk = pipeline_privacy_risk(&envelope.privacy.residual_pii_risk, &merged)`. `outcome = Escalated` if `risk` is Medium or High, else `Cleared` (P3 limits the hold, not the record).
4. `bytes = serde_json::to_vec(&envelope)`; `content_hash = sha256_prefixed(&bytes)`; prepare under `pipeline_attempt_object_id("privacy-pass", run_id, lease_token)`; `stage_attempt_artifact(run, PrivacyPass, .., self.attempt_artifact_cleanup_after(Phase::Review))`; publish (both on the blocking pool via `artifact_store_call`, as VP:11265-11289); `inject_crash(AfterPrivacyPassArtifactStorage)`.
5. `record_privacy_pass(...)`; on a refused commit delete this attempt's object (as VP:11323-11330); `inject_crash(AfterPrivacyPassCommit)`; return the updated run and `bytes`.

Then the Review arm passes `bytes` as `source_artifact` and `sha256_prefixed(&bytes)` as `source_content_hash` (VP:11221-11222). `MinimalReviewPolicy` only requires `dependency_content_hash(source_artifact) == source_content_hash` (BUNDLE:188-191), so the bundle does not change.

- [ ] **Step 1: Write the failing tests** (RT):
  - `privacy_pass_runs_once_and_feeds_review` (double: counting `rescrub_classifier` that replaces a planted span): one `process_one` takes the run through Review; the classifier counter is 1; the run has a pass record with `outcome = cleared`, `privacy_pass_source_hash == sha256(stored source bytes)`, `privacy_pass_content_hash == review outcome's source_content_hash`; one `review_snapshot` ref with `created_by_job_id = run_id`; the `trace_derived_records` row's `input_object_ref_id` is the pass ref.
  - `score_never_sees_planted_prose_pii`: the source carries `MARKER_SECRET`; the stored source object still contains it (the receipt did not run the classifier); `load_approved_bytes` for the run does not; Score completes.
  - `stored_source_is_post_deterministic`: a source with a deterministic secret (an API-key-shaped token) and a prose marker; the stored source lacks the token and keeps the marker.
  - Update `transformed_content_flows_to_score_and_replay_stays_exact` (RT:5806) and the export test (RT:~22201): assertions on approved content and export output stay; any assertion that the stored source is redacted moves to the pass object.
- [ ] **Step 2: Run red.** `cargo test -p trace-commons-server --test versioned_pipeline_runtime_pg privacy_pass_ score_never_sees stored_source_is transformed_content_flows -- --test-threads=1` (with `RUSTFLAGS="-D warnings"`).
- [ ] **Step 3: Implement** per the flow above.
Note: a crash after `record_privacy_pass` and before the policy costs one Review attempt (the pass transaction does not refund it; `mark_awaiting_review` does, VP:5248). Acceptable at 5 attempts, which `commit_review` resets on approval.

- [ ] **Step 4: Audit object-count assertions.** Every Review dispatch now stores one more object and one more ref, also with the no-op doubles. Run `grep -n "count_staged_artifacts\|count_files_under" crates/trace-commons-server/tests/versioned_pipeline_runtime_pg.rs | wc -l` (62 at the plan's commit) and the same over `src/bin/trace_commons_ingest_internal/`; fix each count on a path that reached Review. Re-check the two store-level `review_snapshot` counts (RT:8573, RT:8884): they call `commit_review` directly with no pass, so they should hold; if not, filter by the approved ref id.
- [ ] **Step 5: Audit services built without a boundary** (P6). `grep -n "privacy: None" crates/trace-commons-server/tests crates/trace-commons-server/src/bin/trace_commons_ingest_internal` (one hit at RT:3187, a production-assembly readiness test) and every `test_service_with_controls(.., None)` call (RT:2638-2643): a service that drives Review must get `default_privacy_boundary()` (RT:2374).
- [ ] **Step 6: Run green** and `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run`.
- [ ] **Step 7: Commit** `Run the classifier as a server-owned privacy pass at the start of Review`.

### Task 6: Escalation hold and review-queue visibility

**Files:**
- Modify: VP:115-125 `run_waiting_for_review_sql!` -> `p.next_phase = 'review' AND (p.admission_decision = 'quarantine' OR p.privacy_pass_outcome = 'escalated') AND p.state IN (...)`; update its doc and `claim_review`'s doc (VP:2596-2605, which calls claiming an Admit run a bug).
- Modify: VP:2817-2832 approve check: the reason to resolve is `admission_reason`, or `privacy_review_required` when `admission_reason` is NULL and `privacy_pass_outcome = 'escalated'` (select the column at the row read above :2812). An approval of an escalated run that does not list `privacy_review_required` is refused with the existing `quarantine reason is unresolved` (422 via ING:~43388).
- Modify: VP:10891-10896 parking branch: also park on `PIPELINE_PRIVACY_REVIEW_REQUIRED_LABEL` when `next_phase == Review`.
- Modify: Review arm, after `ensure_privacy_pass` and the assessment load (VP:11229-11232): if Admission is Admit, the pass outcome is `Escalated` and there is no assessment, return `PolicyError::transient(privacy_review_required)`; the policy is not called.
- Modify: ING `PipelineReviewQueueItem` (~ING:43404, `admission_reason` at ~:43407, set at ~:43433): add `hold_reason: String` = `admission_reason` or `privacy_review_required` for an escalated run (additive JSON field; label only).

- [ ] **Step 1: Write the failing tests** (RT; escalation double: `rescrub_classifier` sets `residual_pii_risk = Medium` and returns `[FoundAndRemoved]`; a High variant for D1):
  - `escalated_admit_run_parks_for_a_human`: Admission admits at Low; one dispatch leaves the run `awaiting_review`, `last_error_label = privacy_review_required`, attempt refunded (`mark_awaiting_review`, VP:5236-5260), outcome `escalated`, no Review outcome row, a counting Review policy wrapper is not called; `list_review_queue` lists it; `claim_review` claims it. Repeat with High: same result, never rejected (D1).
  - `escalated_approval_must_resolve_privacy_review_required`: an Approve listing nothing, or listing another reason, is refused; listing `privacy_review_required` is accepted and releases the run to `pending`.
  - `escalated_approval_resumes_without_a_second_classifier_call`: after the approval, one dispatch completes Review; classifier counter still 1; Review policy called once with the pass bytes.
  - `admission_quarantine_is_not_double_parked`: Admission quarantines (Medium deterministic); the pass runs once and the run parks with `review_assessment_required` (the existing label), not the pass label; approval must resolve the Admission reason as today.
  - `cleared_consent_flag_only_run_goes_straight_to_review`: deterministic basis `[ConsentContentFlag]`, Medium, classifier finds nothing: outcome `cleared`, no hold.
  - ING (`pipeline_http_pg_tests.rs`): the quarantine queue route returns `hold_reason = privacy_review_required` for an escalated run.
- [ ] **Step 2: Run red.** `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --test versioned_pipeline_runtime_pg escalat admission_quarantine_is_not cleared_consent -- --test-threads=1`.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run green**; also the existing queue tests near RT:4824 and RT:4840 and the corpus test at `pipeline_corpus_pg_tests.rs:1205` ("quarantine a Medium one as privacy_review_required"): a prose-only Medium is now admitted at the receipt and parked after the pass, so move its expectation from receipt-time quarantine to post-pass `awaiting_review`.
- [ ] **Step 5: Commit** `Hold a run the privacy pass escalates for a human review`.

### Task 7: Human rejection of an escalated Admit run

**Files:** Review arm (VP:11233 onwards); a server-side `PhaseResult` builder next to it.

- [ ] **Step 1: Write the failing test** `escalated_rejection_ends_the_run_rejected` (RT): after a Reject assessment, one dispatch commits Review with `ReviewDecision::Rejected { reason: <assessment reason> }`, evidence `source_content_hash` = pass content hash, `human_assessment_hash` = the assessment's, `rule_id = "privacy_pass_human_review_rejected_v1"`; the submission is `rejected`, the run `complete` with no next phase (as commit_review's rejection, VP:2463-2477, 2491-2497); no approved object; the Review policy is not called; the classifier counter stays 1.
- [ ] **Step 2: Run red.**
- [ ] **Step 3: Implement.** When Admission is Admit, the outcome is `Escalated`, and the assessment is Reject: build the `PhaseResult` mirroring BUNDLE:204-221 with the server rule id, then `commit_review(run, StoredPhaseResult::from_result(Phase::Review, &result)?, None)`. An Approve falls through to the bundle policy with the pass bytes (it approves an Admit run, BUNDLE:192-194). Provenance needs nothing extra: `insert_outcome` writes `outcome_schema_id`/`version` from the fixed `SchemaRef::pipeline_v1()` and `bundle_id` from the run (VP:6284-6310), so the export join on `review.outcome_schema_id` (versioned_pipeline_product.rs:450-466) is unaffected; only `evaluation.rule_id` marks the outcome as the server's. Document in pipeline-activation.md (Task 10) that this one Review outcome's rule id comes from the server, not the bundle.
- [ ] **Step 4: Run green; commit** `Commit a reviewer's rejection of an escalated run`.

### Task 8: Classifier failure: charged retry, legacy backoff, terminal label

**Files:** VP:5048-5101 `mark_retry`; VP:10880-10900 (routing); RT:5710-5753; HTTP:7051-7061 and the section at HTTP:7551-7598.

Routing: the pass raises the error as a `PolicyError` value, `anyhow::Error::from(PolicyError::permanent(PIPELINE_PRIVACY_CLASSIFICATION_FAILED_LABEL)?)`, never `anyhow!(label)`: the dispatch matches on `error.downcast_ref::<PolicyError>()` (VP:10880-10882), and a bare string falls through to the P2 allowlist and is recorded as `minimal_policy_failed`. With the value, so the existing non-transient branch at VP:10900-10902 takes it to the charged `mark_retry_or_record_lease_expired` under its own label; no allowlist edit. It must never be `transient` (that goes to the uncharged `mark_transient_retry`, VP:5108-5152, which has no terminal bound). `privacy_control_missing` (P6) goes to `mark_transient_retry` beside the FR3 gaps at VP:10913-10923.

`mark_retry` changes (one place, no new primitive): (a) for `error_label == PIPELINE_PRIVACY_CLASSIFICATION_FAILED_LABEL`, the delay is `PIPELINE_PRIVACY_RETRY_BASE_SECONDS * 2^(attempt_count-1)` seconds (30, 60, 120, 240 s before the fifth attempt; legacy's ING:1154 and postgres.rs:5470-5471); (b) on exhaustion that label is written as itself instead of `attempts_exhausted` (parameter `$4` becomes the error label for this one label). A worker that crashes mid-pass and is swept by `claim_next` (VP:2068-2086) still gets `attempts_exhausted`; that is a crash, not a classifier verdict, and is documented.

- [ ] **Step 1: Write the failing tests.**
  - Rewrite `privacy_boundary_failure_fails_closed` (RT:5723) as `classifier_failure_retries_then_fails_closed`: the receipt succeeds (`processing`, one run); each Review dispatch leaves the run `retry` with `last_error_label = privacy_classification_failed`, `attempt_count` incremented, `next_attempt_at - NOW()` within 30 s x 2^(n-1) plus tolerance (assert on the stored interval, not wall-clock sleeps; see the CI wall-clock flake note); after `max_attempts` the run is `failed` with `privacy_classification_failed`; no pass record, no approved object, no Review outcome; `trace_submissions.status` still `received`. Drive the clock by setting `next_attempt_at = NOW()` in SQL between dispatches.
  - `classifier_timeout_is_a_classifier_failure`: a double that sleeps past a test-shortened timeout (make `PIPELINE_PRIVACY_PASS_TIMEOUT` a builder knob with the 900 s default) gives the same `retry` state.
  - `missing_boundary_is_an_uncharged_wait`: a service built without `with_privacy` leaves the run `retry` with `privacy_control_missing` and `attempt_count` unchanged.
  - HTTP leg (HTTP:7551-7598, "A privacy boundary that fails"): the POST now answers 200 `processing` and creates a run (the old asserts at :7576 and :7597 invert); a worker pass leaves it in `retry` with `privacy_classification_failed`.
- [ ] **Step 2: Run red.** `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --test versioned_pipeline_runtime_pg classifier_ missing_boundary -- --test-threads=1`.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run green**; plus the whole ingest bin (it holds the HTTP leg):

```bash
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --bin trace-commons-ingest -- --test-threads=1
```

- [ ] **Step 5: Commit** `Retry a failed privacy classification on the legacy backoff and fail closed`.

### Task 9: Crash safety, downstream readers, withdrawal

**Files:** RT:14086-14096 crash matrix; withdrawal and retention-purge tests.

- [ ] **Step 1: Write the failing tests.**
  - Add `AfterPrivacyPassArtifactStorage` and `AfterPrivacyPassCommit` to `crash_matrix_produces_one_logical_effect_per_point` (RT:14080; its evidence records a count, RT:14290-14299, so no fixed number moves).
  - `privacy_pass_crash_before_commit_repeats_the_call_and_commits_one_ref`: crash at `AfterPrivacyPassArtifactStorage`; restart (new service, same DB); the next dispatch calls the classifier a second time, commits exactly one pass ref with the run-derived id, and the first attempt's staged row is still `staged`; after `cleanup_after`, `sweep_attempt_artifacts` deletes the orphaned object (this is the corrected spec test, correction 1).
  - `privacy_pass_crash_after_commit_does_not_call_again`: crash at `AfterPrivacyPassCommit`; the next dispatch makes no classifier call and Review completes with the recorded bytes.
  - `withdrawal_deletes_the_privacy_pass_object`: withdraw a run after its pass; the pass ref is invalidated and queued for `delete_object_payload` (the generic enumeration at VP:6778-6835 covers it), and the deletion worker's verification accepts it (`ReviewSnapshot` -> `ContributionEnvelope`, ING:65689-65696). Same for the retention purge (VP:6984-7008).
- [ ] **Step 2: Run red; Step 3: implement only what fails** (the generic paths should already hold; the tests pin them). `inject_crash` errors propagate unchanged (VP:10770).
- [ ] **Step 4: Run green** with `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --test versioned_pipeline_runtime_pg crash_matrix privacy_pass_crash withdrawal_deletes -- --test-threads=1`.
- [ ] **Step 5: Commit** `Pin the privacy pass's crash points and its deletion`.

Optional, recommended (not required by the spec): legacy benchmark, ranker and process-evaluation exports read the unfiltered reviewer view (ING:54070, 54684, 55031, 41935) and can reach an `accepted` pipeline submission, safe today only because the P1 wrapper does not decode. Track as a follow-up issue to switch them to `read_mains_reviewer_metadata_view` (ING:61874-61883); not in this PR.

### Task 10: Docs and contracts

**Files and edits:**
- `docs/operator/pipeline-activation.md`:
  - 1595-1634 ("Authority and privacy at the receipt"): the receipt runs only the deterministic rescrub before staging (1597-1600); a failing classifier no longer refuses the receipt (1608-1612); "all three refusals" (1614) and the error-hash table (1620-1624) swap `privacy_classification_failed` for `privacy_rescrub_failed`; 1629-1631 becomes the spec's sentence: "the stored source is the content after the deterministic rescrub; the approved content, which is all that Score and exports read, is after the classifier"; replay calls neither (1632).
  - New sub-section "The privacy pass at the start of Review": load, classify, store (per-attempt key, run-derived ref, sweep), record (V117 columns), hand off; escalation; failure; crash safety; server-committed rejection rule id.
  - 1570-1574: the pass spends Review's attempt budget; exhaustion fails with `privacy_classification_failed`.
  - 1652-1682: second parking cause (`privacy_review_required`), queue lists escalated runs with `hold_reason`, what an approval must resolve, no re-run after an assessment.
  - 1703-1751: a bullet for the classifier error (charged, 30 s doubling, terminal label).
  - 1888-1900, 2500-2502, 2573-2576: the pass object in the withdrawal/purge payload lists and the attempt-artifact sweep list.
  - 2124-2127: the export guardrail reads the submission's risk, which the pass now writes.
  - 2322-2324: escalation reads `quarantined`; a `privacy_classification_failed` run reads `accepted` (D2, documented).
  - 196-197 and 876-877: "costs no classifier call" is vacuous; reword to "costs no rescrub".
- `docs/operator/pipeline-activation.md` rollback section: before rolling back below this revision, list runs at Review with no pass recorded (`SELECT run_id FROM pipeline_runs WHERE next_phase = 'review' AND privacy_pass_object_ref_id IS NULL AND state <> 'failed'`) and contain the tenant or drain them first; an older binary would run Review on their deterministic-only source. Also require `SELECT count(*) FROM pipeline_attempt_artifacts WHERE artifact = 'privacy-pass' AND state = 'staged'` to be zero (run the sweep first): the old binary's `PipelineAttemptArtifact::from_db_str` returns `None` for `privacy-pass` and its sweep then fails the whole pass with `pipeline_attempt_artifact_kind_unrecognized` (VP:10054-10057).
- `docs/operator/deployment.md:673-682`: V117 section and the six `pipeline_runs` UPDATE columns.
- `docs/superpowers/specs/2026-09-11-versioned-pipeline-behavioral-contracts.md`: SUB-005 (:392) clarifying sentence per D0; REV-001 (:535-536) and REV-002 (:547-548): Review's source artifact is the pass output, the pass record keeps the source hash; REV-003 (:569): an approval also resolves a server escalation (`privacy_review_required`); REV-004 (:577-585) or new REV-005: the server's pass runs before the Review policy, a bundle cannot opt out, a classifier failure never falls back; RUN-004 (:821-833): two crash boundaries; SCN-003 (:1471-1482): a sibling scenario (prose-only PII admitted Low, escalated, held, approved or rejected).
- `docs/superpowers/specs/2026-09-09-versioned-pipeline-design.md`: Review (:397-407), receipt path (:742-752, step 4 stores the post-deterministic envelope), `pipeline_runs` schema (:668-681).
- `docs/superpowers/specs/2026-09-11-versioned-pipeline-contract-test-manifest.json` (hygiene only; the cross-check is disabled, `scripts/operator/pipeline-deployment-inventory.py:275-280`): new test ids under SUB-005 (:57-62), REV (:79-84), RUN (:108-113) and the scenario.
- Spec doc: add a "Corrections from the implementation plan" note pointing here.

- [ ] **Step 1:** make the edits. **Step 2: verify.**

```bash
python3 scripts/operator/test_pipeline_tooling.py
python3 -c "import json;json.load(open('docs/superpowers/specs/2026-09-11-versioned-pipeline-contract-test-manifest.json'))"
grep -n "privacy_classification_failed" docs/operator/pipeline-activation.md
```

  `test_pipeline_tooling.py` compares the manifest digest to the live file (`scripts/operator/test_pipeline_tooling.py:3508`), so it passes after the edit. No required check is added, so the counts in pipeline-qualification.md (:69-135) do not move.
- [ ] **Step 3: Commit** `Document the Review-start privacy pass`.

### Task 11: Restore drill covers a run stopped after the pass (after the stage 3 restore_seed fix)

Sequenced last and gated: do not start until the stage 3 `restore_seed` blocker is fixed on its own branch and merged, so this task never edits the seed's existing lifetimes.

**Files:** RESTORE (header :15-25, seed near :2018-2170, `AUTHORITATIVE_FINGERPRINT_SQL` :247-260); `docs/operator/pipeline-qualification.md:141-152, 188`.

- [ ] **Step 1: Write the failing assertions.** The seed adds a third receipt run with crash point `AfterPrivacyPassCommit`, beside (not replacing) the `("leased", "settle", true)` run; its own label, `restore_seed_run_not_stopped_after_privacy_pass`, asserts `next_phase = review`, pass record present, no Review outcome. `AUTHORITATIVE_FINGERPRINT_SQL` adds `COALESCE(privacy_pass_object_ref_id::text,'')`, `COALESCE(privacy_pass_content_hash,'')`, `COALESCE(privacy_pass_outcome,'')` to the `concat_ws`. The resume asserts the run completes Review with a classifier call count of 0 after restore (the remote restore copies every object key generically, `versioned_pipeline_remote_restore.rs:113-157`).
- [ ] **Step 2: Run red / implement / green.**

```bash
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --bin trace-commons-ingest restore -- --test-threads=1
python3 scripts/operator/test_pipeline_tooling.py
```

- [ ] **Step 3:** update pipeline-qualification.md (drill description; sample line `pending_runs_resumed=1` -> `2`). **Commit** `Restore drill covers a run stopped after the privacy pass`.

### Task 12: Gate (no commit)

```bash
cargo fmt --all -- --check
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --features local-gpu-models   # non-CUDA CI config; may not link locally
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --features near-ai-scorer
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
cargo clippy -p trace-commons-server --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
RUSTFLAGS="-D warnings" cargo test --workspace
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --lib pipeline_upgrade -- --ignored --test-threads=1
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --test versioned_pipeline_runtime_pg -- --test-threads=1
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --bin trace-commons-ingest -- --test-threads=1
python3 scripts/operator/test_pipeline_tooling.py
grep -rn "\.rescrub(" crates/ | wc -l    # 0
```

Capture a baseline of the runtime_pg and ingest-bin failure counts on the base commit before Task 1, and compare the final counts against it (fresh `admission_test_*` database for each run).

## Review focus

- No path runs the classifier at the receipt (`grep -rn "rescrub_classifier" crates/trace-commons-server/src` shows only VPA and the pass).
- No path hands the Review policy the raw source once a boundary exists; no fallback on classifier error.
- The pass record and logs are label/hash only.
- Escalated runs are reachable by the queue, claim and assessment paths, and an approval must name `privacy_review_required`.
- A rollback binary (V116 code on a V117 database) reads every row the new binary writes: `pipeline_run_from_row` reads by column name (`row.get("tenant_id")`, VP:7224), so extra columns are ignored, and the pass ref uses an existing artifact kind (P2). Rollback hazard: the old binary has no pass, so it would run Review on a run received by the new binary (whose source is deterministic-only) and approve unclassified content. The runbook (Task 10) must require, before a rollback below this revision, that every `next_phase = 'review'` run with `privacy_pass_object_ref_id IS NULL` and `created_at` after the deploy is contained or drained first; Task 10 adds the query, and the same for staged `privacy-pass` attempt rows, which make the old binary's sweep fail its whole pass (VP:10054-10057).

## Questions for the owner

1. D2 follow-up: should `main_status_for_pipeline` map a run failed with `privacy_classification_failed` to `quarantined` instead of `accepted`? Server-only, existing vocabulary.
2. Spec correction 3: is a server-committed Review rejection (rule id `privacy_pass_human_review_rejected_v1`) acceptable, or should the bundle's Review policy learn to honour an assessment on an Admit run (a bundle and qualification change, which the spec rules out)?
3. Legacy exports' implicit fail-closed on pipeline submissions (Task 9 note): file the follow-up?
