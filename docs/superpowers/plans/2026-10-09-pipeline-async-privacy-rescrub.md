# Pipeline privacy rescrub out of the receipt: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** revision 2, 2026-10-09. Revises the first draft (`858842040`) against the 29 critiques in `.local/plan-critiques.json` and the owner decisions Q1-Q3 (see "Owner decisions"). Written against branch `pipeline-async-privacy-rescrub` at `1e7ea3cc2` (spec commit on top of `main` `3c7d4a239`). Every line number below is a line in this worktree at that commit.

**Goal:** Move the prose-PII classifier out of `PipelineService::submit` (the HTTP receipt) and into a server-owned privacy pass at the start of the Review dispatch. The receipt keeps the bounded, local deterministic redactor only. The pass stores the classifier's output as its own encrypted object, records it on the run (V117), holds an escalated run for a human, retries classifier errors on a bounded budget, and hands the Review policy the scrubbed bytes. Score and exports keep reading approved content only, and the database refuses an approval of a run that needed a pass and has none.

**Architecture:** `PipelinePrivacyBoundary` splits into `rescrub_deterministic` (receipt) and `rescrub_classifier` (pass). The pass runs inside `process_claimed`'s `Phase::Review` arm, under Review's lease and attempt budget, before `bundle.review.execute`. Its record is new nullable columns on `pipeline_runs` (storage option (a); option (b) is impossible, spec correction 5). Its object uses the existing per-attempt key, staging row and sweep (FR1), with a run-derived object ref id. A run received after V117 carries `privacy_pass_required = TRUE`, a CHECK refuses its approval without a pass, and it is claimable for human review only once its pass is recorded. Escalation is the server's: it parks the run `awaiting_review` with `privacy_pass_review_required` (its own label, distinct from Admission's `privacy_review_required`), the review queue predicate learns the pass outcome, a human rejection of an escalated Admit run is committed by the server, and a human approval of one is linked to the run's pass record. The gate-api `Phase` enum, `ReviewInput`, the bundle and its qualification fixtures do not change.

**Tech Stack:** Rust (tokio, tokio-postgres, serde, anyhow; existing dependencies only), PostgreSQL 16 with forced RLS, Python 3 standard library (`scripts/operator/`).

**Spec:** `docs/superpowers/specs/2026-10-09-pipeline-async-privacy-rescrub-design.md` (cited as "spec:N"). Contracts: `docs/superpowers/specs/2026-09-11-versioned-pipeline-behavioral-contracts.md` (SUB-005 at :386-392, REV-001..004 at :535-585, REV-003's bullets at :560-575, RUN-004 at :821-833, CMP-002 at :1340, SCN-003 at :1471-1482).

**Abbreviations:** VP = `crates/trace-commons-server/src/versioned_pipeline.rs`; VPA = `crates/trace-commons-server/src/versioned_pipeline_authority.rs`; VPP = `crates/trace-commons-server/src/versioned_pipeline_product.rs`; ING = `crates/trace-commons-server/src/bin/trace-commons-ingest.rs`; RT = `crates/trace-commons-server/tests/versioned_pipeline_runtime_pg.rs`; HTTP = `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_http_pg_tests.rs`; RESTORE = `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_restore_pg_tests.rs`; CORPUS = `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_corpus_pg_tests.rs`; UPG = `crates/trace-commons-server/src/db/postgres/pipeline_upgrade_tests.rs`; BUNDLE = `crates/trace-commons-server/src/versioned_pipeline_bundle.rs`; PROTO = `crates/trace-commons-protocol/src/trace_contribution.rs`.

**Scope:** server crate, one migration (V117), docs and contracts. No protocol, client or gate-api change. Stage 3 artifacts are untouched (see "Stage 3 and rollout").

## Owner decisions

From the spec's open questions (defaults confirmed):

- **D0 (open question 0, SUB-005).** Read as the spec reads it: "synchronous privacy-risk handling" is the bounded, local deterministic redactor. The contracts document gets a clarifying sentence, not an amendment (Task 10).
- **D1 (open question 1, High).** High is held for a human like Medium, never rejected by the pass. The asymmetry stays on purpose: `MinimalAdmissionPolicy` still rejects a receipt-time High with `privacy_risk_rejected` (BUNDLE:150-156); only a pass-time High is held.
- **D2 (open question 2, client status).** The receipt keeps `status: "processing"`. No client change and no new status value. Q1 below changes one server-side mapping inside the existing vocabulary.
- **D3 (open question 3, retry budget).** Reuse the pipeline's own mechanism: the charged `mark_retry` and the run's `max_attempts` (V93 default 5, `migrations/V93__versioned_pipeline_durability.sql:16`), which equals legacy's `TRACE_PII_BACKSTOP_DEFAULT_MAX_ATTEMPTS: i32 = 5` (ING:1153). The pipeline's backoff, `DEFAULT_RETRY_MILLISECONDS = 50` doubling (VP:453, VP:5054-5056), would spend five attempts in about a second of classifier outage, so `mark_retry` gets a third label-keyed branch beside the hourly `artifact_integrity_failed` one (VP:5057), with legacy's shape `30 s x 2^(attempt_count-1)` (`TRACE_PII_BACKSTOP_DEFAULT_BACKOFF_BASE_SECONDS: i64 = 30`, ING:1154; `make_interval(secs => base * POWER(2, attempts))`, `crates/trace-commons-server/src/db/postgres.rs:5470-5471`). The pass call is bounded by `min(900 s, review_lease x PIPELINE_LEASE_RENEWAL_CAP_FACTOR - PIPELINE_PRIVACY_PASS_COMMIT_MARGIN)` (see P8). Legacy's per-tick canary and consecutive-failure breaker (`MAX_CONSECUTIVE_PII_BACKSTOP_FAILURES = 3`, ING:45674) are tick-scoped and are not ported.
- **Storage option (a).** New columns on `pipeline_runs` in V117.

From the owner's review of the first draft:

- **Q1.** A run failed with `privacy_classification_failed` maps to contributor status `quarantined` (existing vocabulary), not `accepted`. Task 8 adds the arm to `main_status_for_pipeline` (ING:17406-17422).
- **Q2.** A server-committed Review rejection of an escalated Admit run is accepted, rule id `privacy_pass_human_review_rejected_v1` (Task 7). For an escalated approval, the server records the approving assessment's `evidence_hash` and resolved reasons on the pass record (two V117 columns, written in `commit_review`'s transaction), so the approved outcome is linked to the human decision (Task 7, with its test).
- **Q3.** Legacy readers' and exports' refusal of a pipeline submission is pinned with a test now (Task 9). The follow-up issue to move them onto `read_mains_reviewer_metadata_view` is filed separately, outside this plan.
- **Critique 0.** Adopt `privacy_pass_required BOOLEAN NOT NULL DEFAULT FALSE`, then `SET DEFAULT TRUE`, plus the CHECK, plus a pass predicate in `commit_review`'s approval UPDATE with a safe label, with a refusal test (Task 5).
- **Critique 1.** Adopt both halves: (1) a run that requires the pass is claimable for review only once its pass is recorded (Task 6); (2) an assessment recorded before `privacy_pass_recorded_at` is ignored when the pass escalates (Task 6). The pass's hold label is `privacy_pass_review_required`.
- **Critiques 7/11.** Update `redaction_counts` and `redaction_pipeline_version` from the pass envelope; keep `redaction_hash` deterministic (P5).
- **Critiques 10/19.** The restore drill keeps exactly one pending run; no third seeded run. Pass resume is proved by runtime crash-matrix tests (Task 5). The stage 3 `restore_seed` blocker is already fixed on `main` (#1315, `e4a788ec4`, in this branch's history), so that gate is removed. Task 11 keeps only a fingerprint extension and runs `pipeline.py restore-drill` against a scratch server.
- **Critiques 20/21.** Task 1 is a pure refactor: the receipt calls `rescrub_deterministic` then `rescrub_classifier` and merges, so Task 2's receipt test is red first. Each green step also runs the existing tests its task touches.
- **Critique 17, 18, 22, 24.** The upgrade-test database is in the global constraints; every multi-filter cargo command puts its filters after `--`; the crash tests are red tests in Task 5; `#[ignore]` is never used as a placeholder.

## Spec corrections (what the code map shows is wrong or impossible)

1. **"Keyed by content hash, so a repeated store is idempotent" (spec:110-111, 169-170) is impossible.** Ruling FR1 (VP:14588-14600) forbids content-addressed object keys: every encrypt uses a fresh salt and nonce, so two writes of equal plaintext differ in ciphertext, and a key shared across attempts would let a later write stop matching a committed ref's `content_sha256`. Resolution: the pass object key is `pipeline_attempt_object_id("privacy-pass", run_id, lease_token)` (VP:14601-14603); the object ref id is derived from the run id alone (as `approved_object_ref`, VP:14814-14824); the attempt is staged in `pipeline_attempt_artifacts` and an uncommitted object is deleted by `sweep_attempt_artifacts`. Idempotency comes from "a pass recorded on the run is never re-run" plus the deterministic ref id. REV-002's "stored by content hash" is met by recording the plaintext hash (`privacy_pass_content_hash`), as `approved_content_hash` does today. The spec test "stores no second object" becomes "commits one object ref; the orphaned attempt object is swept".
2. **"Through the same state and queue as an Admission quarantine" (spec:143-144) does not work as written.** The queue, claim and assessment paths all use `run_waiting_for_review_sql!`, which requires `p.admission_decision = 'quarantine'` (VP:120-124), and `admission_decision` is not updatable by the runtime (`migrations/V92__versioned_pipeline_runs.sql:149-157`). Resolution: Task 6 widens the predicate with `OR p.privacy_pass_outcome = 'escalated'` and narrows it with the pass-recorded condition (critique 1).
3. **"A rejection ends the run as the reviewer's rejection does today" (spec:149-150) does not happen by calling the policy.** `MinimalReviewPolicy` ignores `human_assessment` when Admission admitted (`AdmissionDecision::Admit => (None, Vec::new())`, BUNDLE:192-194), so an escalated Admit run with a Reject assessment would be approved, and an approved one would carry no assessment hash. Resolution: the server commits the rejection itself (Task 7, Q2), and records the approving assessment on the pass record (Task 7, Q2). The server does not feed the policy a synthetic `AdmissionDecision::Quarantine`, which would record an Admission reason Admission never gave.
4. **"After the retry budget is spent, the run fails with `privacy_classification_failed`" (spec:163-164) needs new code.** `mark_retry` always writes `attempts_exhausted` on exhaustion (VP:5076-5079, VP:5090), and the `claim_next` sweep does too (VP:2085). `privacy_classification_failed` is not on the P2 allowlist (VP:10954-10966), so a bare error with that label is recorded as `PIPELINE_OPERATIONAL_ERROR_LABEL` = `"minimal_policy_failed"` (VP:136). Resolution: Task 8.
5. **Option (b) "no schema change" (spec:196-197) is not available.** Staging the pass object needs a new `pipeline_attempt_artifacts.artifact` value, and V108 pins the set in a CHECK (`migrations/V108__versioned_pipeline_attempt_artifacts.sql:23`). Option (a) is taken.
6. **Pass step 2 says `rescrub` (spec:109).** It is `rescrub_classifier` after the split.
7. **Line drift.** Spec "VPA:86" is the trait at VPA:87; spec "VP:11415" (Score's `load_approved_bytes` call) is correct, but the loader itself is at VP:10300-10312.
8. **"Exports are not confirmed yet" (spec:117-119) is now confirmed.** The pipeline export snapshot selects by `r.approved_object_ref_id` and `approved_content_hash` (VPP:448-480) and reads no bytes; the index rebuild reads only sealed index commands (VP:10441-10466); Score reads `load_approved_bytes` (VP:11415). No pipeline reader takes the source. Legacy readers that can reach a pipeline submission (`get_latest_active_envelope_object_ref`, ING:56028-56049; the benchmark, ranker and process-evaluation jobs at ING:54057, 54654, 54996, 41896) fail closed only because the P1 wrapper does not decode as an envelope (ING:68039-68044); Task 9 pins that with a test (Q3). One gap is fixed: the export filter `s.privacy_risk = $8` (VPP:477) reads the submission row, which the receipt now writes from the deterministic envelope only, so the pass writes it back (P5).
9. **"High is held" needs a definition of escalation for Admission-quarantined runs.** See P3: the pass escalates when its risk is above the risk the receipt stored.

## Decisions made while writing this plan

- **P1. Receipt refusal label for a deterministic failure.** VP:9099 maps every boundary error to `privacy_classification_failed`. A deterministic failure is a `PrivacyFilterConfigError`, which legacy reports as "privacy filter config invalid" (ING:15317-15318). New constant `PIPELINE_PRIVACY_RESCRUB_FAILED_LABEL = "privacy_rescrub_failed"` in VPA beside VPA:22-23. `privacy_classification_failed` becomes a Review-phase run failure label only. pipeline-activation.md's refusal table (1620-1624) changes with it (Task 10).
- **P2. Pass object artifact kind: `ReviewSnapshot`, with `created_by_job_id = Some(run_id)`.** (a) The pipeline withdrawal queues `delete_object_payload` items (VP:7011-7060), which the revocation-propagation worker deletes through `delete_object_payload_for_revocation_propagation` (ING:65634), whose mapping sends `ReviewSnapshot` to `ContributionEnvelope` (ING:65689-65692); `WorkerIntermediate` would map to `VectorPayload` and be rejected unless `is_pipeline_score_object_ref` knows it (VP:14924-14930). The other mapping, `trace_artifact_kind_from_storage` (ING:19412-19421, `ReviewSnapshot -> Other`), is legacy's whole-submission delete (ING:19477); Task 9's test pins which one a pipeline withdrawal takes. (b) No legacy by-name selector picks it up: `rescrubbed_envelope` is selected with `status = 'quarantined'` by `requeue_quarantined_for_pii_backstop` and the residual-survivor reset (`crates/trace-commons-server/src/db/trace_corpus_pg.rs:2861-2875, 2890-2915`), and the pipeline writes `quarantined` for an Admission quarantine (VP:6396-6412); `submitted_envelope` is the "latest active object" selector (`trace_corpus_pg.rs:2249`; `postgres.rs:5332, 5387, 5464`). (c) A new `TraceObjectArtifactKind` variant is decoded by `enum_from_storage` (`trace_corpus_pg.rs:480, 993`), so a Route B rollback would fail to read any ref row the new binary wrote. `ReviewSnapshot` meets all three: legacy selects it by name nowhere outside tests ("review_snapshot" appears only at VP:2409 and ING:52944). `created_by_job_id = Some(run_id)` plus the run-derived id tell it apart from the approved ref (`None`, VP:14836).
- **P3. Escalation is "the pass's risk is above the risk the receipt stored".** The pass reads the submission's receipt-time `privacy_risk` and `residual_risk_basis` before it overwrites them (P5). Both sides of the comparison are on Admission's `PrivacyRisk` scale. The column holds the envelope's raw `ResidualPiiRisk` (`privacy_risk: enum_string(&envelope.privacy.residual_pii_risk)`, VP:9726); the ConsentContentFlag downgrade happens only in memory, for `AdmissionInput` (VP:9206-9216). So the receipt-time risk is `receipt_risk = pipeline_privacy_risk(&stored raw risk, &stored basis parsed with ResidualRiskCondition::from_label)` (`safe_residual_risk_basis_labels` writes each condition's `as_label()`, `trace_corpus_storage.rs:1083-1092`, and `from_label` reads `ALL` back by `as_label()`, PROTO:6348-6353, so the basis round-trips). `outcome = Escalated` iff `pipeline_privacy_risk(pass envelope risk, merged basis)` is strictly above `receipt_risk`; else `Cleared`. Comparing against the raw column would be wrong: a consent-flag-only trace is stored as raw `medium` and admitted at Low, and if the classifier then finds prose PII the pass's mapped Medium is not above raw `medium`, so the run would be `Cleared` and auto-approved with PII in it. For an Admission-admitted run (receipt Low) that is Medium or High. For an Admission-quarantined run (receipt Medium, `privacy_review_required`) it is High only. For a run received before V117 (whose stored source already went through the classifier) it is a rise past the classifier's own earlier verdict, which is rare. An escalated run is held by the server with `privacy_pass_review_required` whatever Admission decided; a non-escalated Admission-quarantined run is held by the policy with `review_assessment_required`, as today. An approval must resolve every hold that applies: the Admission reason if there is one, and `privacy_pass_review_required` if the pass escalated (Task 6).
- **P4. Risk mapping is factored and reproduces the ConsentContentFlag downgrade.** The receipt's mapping (VP:9203-9216: `Medium` with basis exactly `[ConsentContentFlag]` maps to `Low`) moves into `fn pipeline_privacy_risk(risk: &ResidualPiiRisk, basis: &[ResidualRiskCondition]) -> PrivacyRisk`. The pass merges the receipt's basis (read back from `trace_submissions.residual_risk_basis`, parsed with `ResidualRiskCondition::from_label`, PROTO:6348) with the classifier's conditions, and maps the pass envelope's `residual_pii_risk` through the same helper.
- **P5. The pass writes back to `trace_submissions`, except `redaction_hash`.** In the pass transaction it sets `privacy_risk` (the pass envelope's raw `residual_pii_risk`, through `enum_string` as the receipt writes it at VP:9726, never the mapped `PrivacyRisk`: the column holds the raw residual risk everywhere else, and a consent-only Medium written as mapped `low` would pass the export filter at VPP:477), `residual_risk_basis` (through `safe_residual_risk_basis_labels`, `crates/trace-commons-server/src/trace_corpus_storage.rs:1083`), `redaction_counts` (the classifier adds to them, PROTO:5612-5617) and `redaction_pipeline_version` (the classifier appends `+near-ai-pii-backstop-v1`, PROTO:5713-5724), so the export filter (VPP:477), the public run record (`public_run.rs:139` returns `redaction_pipeline_version`) and every status reader see post-classifier values, as they did when the receipt stored the composed envelope (VP:9726-9730). `redaction_hash` stays the deterministic envelope's on purpose, for two reasons: (i) tombstone matching. A pipeline withdrawal writes its tombstone from the submission row's `redaction_hash` (VP:6797-6824), and a later receipt checks tombstones against the hash of its own deterministic envelope (VP:6102-6110); a post-classifier hash on the row would stop a resubmission of a withdrawn trace from matching. (ii) The V68 trigger revokes every token bundle of a submission whose `redaction_hash` changes (`migrations/V68__token_rescrub_revocation.sql:4-7`). Legacy's backstop does refresh the hash (ING:~46022-46035, "refreshes the redaction hash / counts / privacy risk"), so this is a deliberate pipeline-only difference, documented in Task 10. Task 4 checks that no reader recomputes `redaction_hash` from the stored counts. `status` is not changed. The runtime already updates `trace_submissions` (VP:2451, VP:2471; table-wide since V62, `migrations/V90__ingest_runtime_grants.sql:5-7`), so V117 grants nothing on it.
- **P6. A worker without a privacy boundary.** The pass needs `self.privacy`. A missing boundary is a deployment gap, not the trace's fault: `privacy_control_missing` goes to the uncharged `mark_transient_retry` (FR3 shape, beside VP:10913-10923). It never falls back to running Review on the unscrubbed source.
- **P7. The pass is recorded once per run and never re-run after it is recorded,** including after an assessment and after a restore. The classifier call itself can run more than once: a crash before the record transaction repeats it, and so does a live worker that loses its lease mid-pass (VP:765-770) while a second worker reclaims the run. Exactly one result is recorded (the `privacy_pass_object_ref_id IS NULL` fence and the lease predicate); the loser's commit is refused and its object deleted, or swept from its staged row. Tested in Task 5; stated in pipeline-activation.md (Task 10).
- **P8. Pass timeout fits inside the Review lease cap.** The cap is `claim time + review_lease x 4` (VP:8081-8082) and the Review lease is configurable down to 1 s (VP:775-777, ING:664-665). The effective timeout is `min(900 s, review_lease x PIPELINE_LEASE_RENEWAL_CAP_FACTOR - PIPELINE_PRIVACY_PASS_COMMIT_MARGIN)` with a 60 s margin (source load, store, record, Review commit), computed when the service is built. When the boundary reports `classifies_prose_pii()` and that leaves under 60 s for the classifier (a Review lease under 30 s), the build refuses with `PIPELINE_LEASE_CONFIG_INVALID_LABEL`. When it does not (pass-through and test doubles, whose call is local), the bound is the 900 s ceiling, never the lease-derived value, so a short test lease never times the pass out. A test-only builder knob overrides the 900 s ceiling.
- **P9. A superseded assessment has no re-assessment path in this change.** `pipeline_review_assessments` is append-only with one row per run (`UNIQUE (tenant_id, run_id)`, V105:47; the runtime holds `SELECT, INSERT` only, V105:240). An assessment ignored under critique 1 (2) therefore cannot be replaced. The run is held fail-closed: `awaiting_review`, `privacy_pass_review_required`, listed in the review queue with `assessment_superseded = true`, not claimable; its exits are withdrawal or operator containment. Because half (1) makes every `privacy_pass_required` run unassessable before its pass, only a run received before V117 can reach this. Task 10 gives the pre-deploy query that counts them (it runs on the V116 database, per tenant, so it names no V117 column). Listed as owner question 1.

## Stage 3 and rollout

- Nothing in this plan changes the stage 3 artifacts, the pilot (`a14eaff0`/V116, pipeline off) or the in-flight promote of `q5be633d6`. The stage 3 `restore_seed` blocker is fixed on `main` by #1315 (`e4a788ec4`), which is in this branch's history, so no task waits on it. The restore seed keeps its two lifetimes and its one pending run (RESTORE:2141-2165).
- The change ships as one promote cycle after stage 3: build, Route B deploy (V117), `qualify`, promote checks, sign, assemble, then requalify the pipeline tenant's bundle on the new revision (spec:244-250).
- Stage 3 acceptance (spec:240-242): `long-chunk-capped` stored with status `processing` in under 2 s and reaching a terminal state; `pii-residual` reads `quarantined` (via the pass's `AwaitingReview`, which `main_status_for_pipeline` maps to `quarantined`) only after at least one worker dispatch has run the pass. Between the receipt and that dispatch it reads `accepted` (`_ => "accepted"`, ING:17421); a smoke check that reads status immediately after upload must wait for the worker first.
- V117 adds nullable columns, plus `privacy_pass_required` which is FALSE on every existing row, so existing pilot runs stay valid. A run already past Review keeps NULL pass columns. A run at Review with no pass recorded runs the pass on its next dispatch. A run received after V117 by either binary gets `privacy_pass_required = TRUE` (the receipt's INSERT lists its columns explicitly, VP:6498-6502, so the default applies), and its approval without a pass fails the CHECK: a V116 binary rolled back onto a V117 database cannot approve unclassified content. It does not make such a run wait, though. The old binary's `commit_review` gets the CHECK violation (SQLSTATE 23514) back from its `query_one` UPDATE (VP:2503-2526) as a `DatabaseError::Postgres`, which the Review arm re-raises (`error => error.into()`, VP:11348). 23514 is not in `is_transient_sqlstate` (VP:7168-7175) and the error is not a `PolicyError`, so it falls to the P2 allowlist's `_ => PIPELINE_OPERATIONAL_ERROR_LABEL` (`minimal_policy_failed`, VP:10930-10941) and the charged `mark_retry` (VP:5048-5101), at `DEFAULT_RETRY_MILLISECONDS = 50` doubling (VP:453). `phase_commit_refused` is true for a database error (VP:7116-7121), so each attempt's approved object is deleted too. Five attempts take about 750 ms of backoff, and the run ends `failed` with `attempts_exhausted`, which is terminal. Every `privacy_pass_required` run the old binary would approve at Review (every Admission-admitted run, and every Admission-quarantined run once it has an Approve assessment) is lost that way. A rollback below this revision therefore needs containment first: the Task 10 runbook's rollback section gives the count query and the steps (suspend the Review policy, or stop the pipeline workers).

## Compatibility mapping (D2 and Q1)

`main_status_for_pipeline` (ING:17406-17422) maps: an escalated run parked `awaiting_review` to `quarantined`; a run waiting for its pass (state `pending`/`retry`, Admission admit) to `accepted`, as for any undecided admitted run today; and, after Task 8 (Q1), a run whose processing is `Failed` with `reason_label` (`last_error_label`, VPP:1225) `privacy_classification_failed` to `quarantined`. `PipelineProcessingStatus::Failed` exists (VPP:139). Task 10 documents all three in pipeline-activation.md:2322-2324.

## Global constraints

- TDD: each task writes its failing test first, runs it red, then implements. A test that pins behaviour which already holds is labelled **regression pin (expected green)**, not red.
- `RUSTFLAGS="-D warnings"` on every check and test build. Clippy with the repo allow-list only: `cargo clippy -p trace-commons-server --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`.
- Every cargo command with more than one test-name filter puts the filters after `--` (cargo accepts one `TESTNAME` positional; libtest accepts several): `cargo test -p trace-commons-server --test versioned_pipeline_runtime_pg -- filter_a filter_b --test-threads=1`.
- Never use `#[ignore]` as a placeholder. In this repo `#[ignore]` is a CI selector (the upgrade and restore suites run with `--ignored`; the RT step does not, `.github/workflows/ci.yml:419-420`). A test that stops applying is rewritten or deleted in the task that changes its behaviour.
- New RT tests use a fresh `uuid` tenant and per-test counter doubles (no global counters), because CI runs RT in parallel (ci.yml:419-420); local runs use `--test-threads=1` only for speed of diagnosis, and Task 12 runs RT without it.
- Each green step also runs the existing tests its task touches (named in the step).
- `cargo fmt --all` before each commit; check the diff is confined to touched files.
- Hash-only and label-only: the pass record and every log line carry hashes, ref ids and labels; never envelope text, never classifier spans.
- Fail-closed: a classifier error never falls back to the deterministic result; a missing boundary never runs Review on the source; a run that needs a pass is never approved without one.
- `trace-commons-gate-api` (`Phase`, `ReviewInput`, `AdmissionDecision`) and `versioned_pipeline_bundle.rs` are not edited.
- New `.rs` files (none planned) would need the AGPL header.
- PostgreSQL test databases: fresh per session, on the literal host `127.0.0.1`. The runtime and ingest-bin tests read `TRACE_COMMONS_PG_TEST_DATABASE_URL` (an `admission_test_*` database). The upgrade tests read `TRACE_COMMONS_PIPELINE_PG_UPGRADE_TEST_URL` and require a database named `pipeline_test_*` (UPG:27-28, UPG:51-54; CI sets it at ci.yml:416-418); without it every `--ignored` upgrade test panics on the env lookup, which is a setup failure, not a red step. V117 is edited in Task 5 after Task 3 creates it, so drop and recreate both databases after any edit to V117.

```bash
createdb -h 127.0.0.1 admission_test_async_rescrub_1
createdb -h 127.0.0.1 pipeline_test_async_rescrub
export TRACE_COMMONS_PG_TEST_DATABASE_URL=postgres://$USER@127.0.0.1:5432/admission_test_async_rescrub_1
export TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL=postgres://tc_login_resolver_login@127.0.0.1:5432/admission_test_async_rescrub_1
export TRACE_COMMONS_PIPELINE_PG_UPGRADE_TEST_URL=postgres://$USER@127.0.0.1:5432/pipeline_test_async_rescrub
```

- Baseline before Task 1: run RT, the ingest bin and the upgrade suite on the base commit and record pass/fail counts; Task 12 compares against them.

## File map

| File | Change |
|---|---|
| `crates/trace-commons-server/src/versioned_pipeline_authority.rs` | Trait split (VPA:86-106), both impls (VPA:108-183), new label, unit tests (VPA:267-345) |
| `crates/trace-commons-server/src/versioned_pipeline.rs` | Receipt step 3 (VP:9081-9117) and docs (VP:7875-7877, 8968-8984, 9388-9389); risk helper (VP:9203-9216); `PipelineRunRecord` (VP:1013-1051) and `pipeline_run_from_row` (VP:7220-7256); crash points (VP:918-942); `PipelineAttemptArtifact` (VP:14476-14507); pass object ref helper (beside VP:14814); store `record_privacy_pass` and `load_submission_receipt_privacy` (beside `commit_review`, VP:2360); `commit_review` approval predicate and pass-approval columns (VP:2503-2526) and derived record (VP:2425-2441); review predicate (VP:115-125), claim (VP:2596-2605) and approve check (VP:2817-2832); `load_review_assessment` (VP:2899-2930, add `recorded_at`); `list_review_queue` (VP:3005-3020, hold reason and superseded flag); Review arm (VP:11207-11253); error routing (VP:10880-10925); `mark_retry` (VP:5048-5101); pass timeout (beside VP:775-790) |
| `migrations/V117__pipeline_privacy_pass.sql` | New |
| `crates/trace-commons-server/src/db/postgres.rs` | `MIGRATIONS` row after the V116 row (postgres.rs:1758-1767); optional shape pin (postgres.rs:8073+) |
| `crates/trace-commons-server/src/db/postgres/pipeline_upgrade_tests.rs` | `RUNTIME_PIPELINE_GRANTS` (UPG:136-167); new `v117_*` test beside UPG:1564 |
| `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` | Review queue item (ING:43404-43433); `main_status_for_pipeline` (ING:17406-17422, Q1) |
| `crates/trace-commons-server/tests/versioned_pipeline_runtime_pg.rs` | Doubles (RT:2363, 5713, 5761, 30387); rewritten or deleted tests (RT:5723-5753, 5806); crash matrix (RT:14086-14096); direct `commit_review` and `claim_review` callers (7 and 17 sites); new pass tests |
| `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_http_pg_tests.rs` | Doubles (HTTP:211, 7037, 7054); inverted leg (HTTP:7551-7598); queue `hold_reason` test; legacy-reader pin |
| `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/production_assembly_tests.rs` | `ClassifyingPrivacy` (:637-639) |
| `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs` | `QualifiedTestPrivacy` (:10989-10991); `main_status_for_pipeline` unit test |
| `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_restore_pg_tests.rs` | Task 11: `AUTHORITATIVE_FINGERPRINT_SQL` (:247-260) only |
| `docs/operator/pipeline-activation.md`, `docs/operator/deployment.md`, `docs/operator/pipeline-qualification.md` | Task 10 / 11 |
| `docs/superpowers/specs/2026-09-11-versioned-pipeline-behavioral-contracts.md`, `docs/superpowers/specs/2026-09-09-versioned-pipeline-design.md`, `docs/superpowers/specs/2026-09-11-versioned-pipeline-contract-test-manifest.json` | Task 10 |

No change: `versioned_pipeline_production.rs` (passes the boundary through; reads `privacy_classifies_prose_pii`, :907-909), `versioned_pipeline_production/gate_env.rs:484-505`, `versioned_pipeline_harness.rs:239-262`, `pipeline_runtime.rs:330-345`, `versioned_pipeline_bundle.rs`, gate-api, CORPUS (its harness sets `residual_pii_risk` on the fixture envelope directly, CORPUS:862-873, with a pass-through boundary, CORPUS:651, and reviews only after the run reaches `awaiting_review`, CORPUS:1094-1106, so its expectations hold; Task 12 runs `pipeline.py qualify` to confirm).

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
pub const PIPELINE_PRIVACY_PASS_REVIEW_REQUIRED_LABEL: &str = "privacy_pass_review_required";
pub const PIPELINE_PRIVACY_PASS_MISSING_LABEL: &str = "privacy_pass_missing";
pub const PIPELINE_PRIVACY_PASS_REJECTED_RULE_ID: &str = "privacy_pass_human_review_rejected_v1";
pub const PIPELINE_PRIVACY_PASS_ALREADY_RECORDED_LABEL: &str = "privacy_pass_already_recorded"; // Task 4: the fence
const PIPELINE_PRIVACY_PASS_MAX_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(900);
const PIPELINE_PRIVACY_PASS_COMMIT_MARGIN: std::time::Duration = std::time::Duration::from_secs(60);
const PIPELINE_PRIVACY_RETRY_BASE_SECONDS: i64 = 30;
pub enum PipelineCrashPoint { /* existing */ AfterPrivacyPassArtifactStorage, AfterPrivacyPassCommit }
pub enum PipelineAttemptArtifact { Approved, IndexCommand, ScoreNeighbors, PrivacyPass } // "privacy-pass"
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyPassOutcome { Cleared, Escalated }
// PipelineRunRecord gains:
pub privacy_pass_required: bool,
pub privacy_pass_object_ref_id: Option<Uuid>,
pub privacy_pass_content_hash: Option<String>,   // sha256: of the pass output (plaintext)
pub privacy_pass_source_hash: Option<String>,    // sha256: of the source bytes the pass read
pub privacy_pass_residual_risk_basis: Option<Vec<String>>, // labels, safe_residual_risk_basis_labels
pub privacy_pass_outcome: Option<PrivacyPassOutcome>,
pub privacy_pass_recorded_at: Option<DateTime<Utc>>,
pub privacy_pass_approval_assessment_hash: Option<String>, // Q2: the approving assessment's evidence_hash
pub privacy_pass_approval_resolved_reasons: Option<Vec<String>>, // Q2
// HumanReviewAssessment as loaded by the store gains `recorded_at: DateTime<Utc>`
// (a store-side wrapper if the gate-api type cannot change: `StoredReviewAssessment { assessment, recorded_at }`).
// Both `pub` (Task 4): RT calls them, and `privacy_pass_object_ref` has no
// non-test caller until Task 5, so a private fn would fail `-D warnings`.
pub fn pipeline_privacy_risk(risk: &ResidualPiiRisk, basis: &[ResidualRiskCondition]) -> PrivacyRisk;
pub fn privacy_pass_object_ref(run, receipt, size_bytes, object_store) -> TraceObjectRefWrite; // ReviewSnapshot, created_by_job_id Some(run_id)
// Fields are `pub`: RT is a separate integration-test crate that builds a
// PrivacyPassRecord (Task 4) and reads a SubmissionReceiptPrivacy (Task 4).
// `ciphertext_sha256` is the bare hex the staged row holds (a `sha256:`
// prefix is stripped); it must match `object_ref.content_sha256`.
pub struct PrivacyPassRecord<'a> { pub object_ref: &'a TraceObjectRefWrite, pub ciphertext_sha256: &'a str,
    pub content_hash: &'a str, pub source_hash: &'a str, pub basis_labels: &'a [String],
    pub outcome: PrivacyPassOutcome,
    pub residual_pii_risk: ResidualPiiRisk, // raw envelope risk, written to trace_submissions.privacy_risk (P5)
    pub redaction_counts: &'a BTreeMap<String, u32>, pub redaction_pipeline_version: &'a str }
// Raw, as stored: compare through pipeline_privacy_risk(&residual_pii_risk, &parsed basis) (P3).
pub struct SubmissionReceiptPrivacy { pub residual_pii_risk: ResidualPiiRisk, pub residual_risk_basis: Vec<String> }
// list_review_queue's row (Task 6): the run plus what holds it.
pub struct PipelineReviewQueueEntry { pub run: PipelineRunRecord, pub hold_reason: Option<String>,
    pub assessment_superseded: bool }
impl PgPipelineStore {
    // list_review_queue changes its return type (Task 6):
    pub async fn list_review_queue(&self, tenant_id: &str, limit: usize)
        -> Result<Vec<PipelineReviewQueueEntry>, DatabaseError>;
    pub async fn record_privacy_pass(&self, run: &PipelineRunRecord, pass: PrivacyPassRecord<'_>)
        -> Result<PipelineRunRecord, DatabaseError>;
    pub async fn load_submission_receipt_privacy(&self, tenant_id: &str, submission_id: Uuid)
        -> Result<SubmissionReceiptPrivacy, DatabaseError>;
    // commit_review gains a fourth argument in Task 7 (Task 5 leaves the
    // signature unchanged: an unused parameter fails `-D warnings`):
    pub async fn commit_review(&self, run: &PipelineRunRecord, result: StoredPhaseResult,
        approved: Option<ApprovedArtifactWrite>, pass_approval: Option<&HumanReviewAssessment>)
        -> Result<PipelineRunRecord, DatabaseError>;
}
impl PipelineService {
    async fn ensure_privacy_pass(&self, run: &PipelineRunRecord)
        -> anyhow::Result<(PipelineRunRecord, Vec<u8>)>; // (run as recorded, pass output bytes)
    async fn load_privacy_pass_bytes(&self, run: &PipelineRunRecord) -> anyhow::Result<Vec<u8>>;
    fn privacy_pass_timeout(&self) -> std::time::Duration; // P8
}
```

(Use the real parameter types of `commit_review` at VP:2360; the fourth argument is the only change.)

---

### Task 1: Split the privacy boundary trait (pure refactor)

No behaviour changes in this task: the receipt still runs both halves, in the same order, and merges their bases as `ClassifierRedactorPipelinePrivacyBoundary::rescrub` does today (VPA:149-168).

**Files:**
- Modify: VPA:86-106 (trait), VPA:108-118 (`DeterministicPipelinePrivacyBoundary`), VPA:120-183 (`ClassifierRedactorPipelinePrivacyBoundary` and its docs at :120-126, :175-178), VPA:22-23 (new label), VPA tests at :267-345.
- Modify: VP:9095-9105 call site: `privacy.rescrub_deterministic(&mut envelope)` then `privacy.rescrub_classifier(&mut envelope)`, both mapped to `PIPELINE_PRIVACY_CLASSIFICATION_FAILED_LABEL` as today, merging each half's conditions into `residual_risk_basis` without duplicates.
- Modify (mechanical): the 11 implementations. Production: VPA:111, VPA:148. Doubles with both methods `Ok(Vec::new())`: RT:2363 `PassThroughPipelinePrivacyBoundary`, RT:30387 `QualifiedProductionPrivacy`, HTTP:211 `PassThroughPipelinePrivacyBoundary`, `production_assembly_tests.rs:637-639` `ClassifyingPrivacy`, `tests.rs:10989-10991` `QualifiedTestPrivacy`. Behavioural doubles: RT:5713 and HTTP:7054 `FailingPrivacyBoundary` (deterministic `Ok(Vec::new())`, classifier bails; Task 2 makes RT's copy configurable so it stays constructed); RT:5761 and HTTP:7037 `MarkerRedactingBoundary` (deterministic no-op, the MARKER_SECRET replacement moves into `rescrub_classifier`). Because the call site still runs both halves, these doubles behave exactly as before.

Both methods are required, with no default and no provided composite `rescrub()`: a default no-op `rescrub_classifier` would let a classifying boundary skip the classifier while reporting `classifies_prose_pii() == true`, and a composite would let a future call site put the classifier back on the receipt.

- [ ] **Step 1: Write the failing tests** (VPA test module):
  - `deterministic_boundary_classifier_is_a_noop`: an envelope with a prose name ("Jane Doe") through `DeterministicPipelinePrivacyBoundary.rescrub_classifier` returns `Ok(vec![])` and leaves the envelope byte-identical.
  - `classifier_boundary_deterministic_half_never_calls_the_adapter`: a `ClassifierRedactorPipelinePrivacyBoundary` over a counting adapter; `rescrub_deterministic` is Ok and the adapter's call count is 0.
  - Rewrite `ordinary_identifier_prefixes_do_not_add_privacy_findings` (:284, call at :293): `rescrub_deterministic` then `rescrub_classifier`, merged without duplicates; assertions kept (basis `[ConsentContentFlag]`, Medium).
  - Rewrite `classifier_pii_is_transformed_and_quarantinable` (:306, call at :308): the redaction of "Jane Doe" and risk >= Medium come from `rescrub_classifier` on an envelope that already went through `rescrub_deterministic`.
  - Rewrite `classifier_failure_fails_closed` (:325, assert at :344): `rescrub_deterministic` is Ok; `rescrub_classifier` is Err with message `privacy_classification_failed`.
  - `each_privacy_boundary_reports_what_it_is` (:267) unchanged.
- [ ] **Step 2: Run red.** `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --lib versioned_pipeline_authority` -> FAIL (no such methods).
- [ ] **Step 3: Implement.** Trait per "Interfaces"; doc on `classifies_prose_pii` (VPA:97-102) says "whether `rescrub_classifier` runs a prose-PII classifier". `Deterministic`: `rescrub_deterministic` = `rescrub_trace_envelope(envelope).map_err(Into::into)`; `rescrub_classifier` = `Ok(Vec::new())`. `ClassifierRedactor`: `rescrub_deterministic` = `rescrub_trace_envelope(envelope)?`; `rescrub_classifier` = `rescrub_envelope_prose_pii_with(self.adapter.as_ref(), envelope, self.policy).await.map_err(|_| anyhow!(PIPELINE_PRIVACY_CLASSIFICATION_FAILED_LABEL))`. The basis merge at VPA:160-164 moves to the call site. The classifier half does not re-run the deterministic redactor: `rescrub_envelope_prose_pii_with` already reconciles consent declarations, sweeps secrets over its own output and reads `envelope.privacy.residual_pii_risk` as its prior (PROTO:5527-5529, 5566-5569, 5650-5672), so running it on a post-deterministic envelope reproduces today's in-order composition. Add `PIPELINE_PRIVACY_RESCRUB_FAILED_LABEL` beside VPA:23 (unused until Task 2; mark it `pub` so `-D warnings` does not flag it). Update every implementation listed under Files.
- [ ] **Step 4: Run green.** Same command -> PASS. Then the compile gates and the existing tests this task touches (all must pass unchanged, which is what makes this a pure refactor):

```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --test versioned_pipeline_runtime_pg -- privacy_boundary_failure_fails_closed transformed_content_flows_to_score_and_replay_stays_exact --test-threads=1
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --bin trace-commons-ingest compatibility_bundle_through_http_with_review_privacy_withdrawal_and_export
```

  Grep checks: `grep -rn "\.rescrub(" crates/` returns nothing; `grep -rn "async fn rescrub_deterministic" crates/ | wc -l` is 12 and `grep -rn "async fn rescrub_classifier" crates/ | wc -l` is 12 (the trait plus 11 implementations: 2 production + 9 doubles). `cargo test --no-run` above already fails if any implementation lacks a method; the counts guard against an implementation that was deleted instead.
- [ ] **Step 5: Commit** `Split the pipeline privacy boundary into deterministic and classifier halves`.

### Task 2: The receipt runs the deterministic half only

**Files:**
- Modify: VP:9081-9105 (step 3 and its comment: drop the `rescrub_classifier` call), VP:8968-8984 (`submit` doc), VP:9388-9389 (`precheck_receipt` comment), VP:7875-7877 (`with_privacy` doc), VP:9203-9216 (factor `pipeline_privacy_risk`).
- Test: RT (new tests beside RT:5723); RT:5723-5753 `privacy_boundary_failure_fails_closed` (deleted, see Step 3); RT:5708-5720 `FailingPrivacyBoundary` (made configurable, see Step 3); RT:5800-5804, RT:5891-5893, RT:22193 (docs).

- [ ] **Step 1: Write the failing tests.**
  - RT `receipt_makes_no_classifier_call`: a double whose `rescrub_classifier` increments a per-test counter and sleeps 60 s and whose `rescrub_deterministic` increments another. `tokio::time::timeout(Duration::from_secs(5), service.submit(..))` completes, returns the `processing` receipt; classifier counter 0, deterministic counter 1. A replay of the same key returns the stored receipt and both counters are unchanged.
  - RT `receipt_deterministic_failure_refuses_with_privacy_rescrub_failed`: RT's `FailingPrivacyBoundary` built to fail its deterministic half (`FailingPrivacyBoundary { fail_deterministic: true }`, Step 3), whose `rescrub_deterministic` bails. `submit` errors with `privacy_rescrub_failed`; 0 runs, 0 staged artifacts, 0 files (the assertions `privacy_boundary_failure_fails_closed` makes today at RT:5747-5752, moved here).
  - VP unit `pipeline_privacy_risk_maps_like_the_receipt`: Low->Low; Medium + `[ConsentContentFlag]`->Low; Medium + `[ConsentContentFlag, FoundAndRemoved]`->Medium; High->High.
- [ ] **Step 2: Run red.**

```bash
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --lib pipeline_privacy_risk_maps_like_the_receipt
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --test versioned_pipeline_runtime_pg -- receipt_makes_no_classifier_call receipt_deterministic_failure_refuses_with_privacy_rescrub_failed --test-threads=1
```

  Expected: the unit test fails to compile (no helper); `receipt_makes_no_classifier_call` fails because the receipt still calls the classifier (the 5 s timeout fires, or the counter is 1); the deterministic-failure test fails on the label (`privacy_classification_failed`).
- [ ] **Step 3: Implement.** Remove the classifier call from the receipt. Map the deterministic error to `PIPELINE_PRIVACY_RESCRUB_FAILED_LABEL`. Factor VP:9203-9216 into `pipeline_privacy_risk` and call it. Rewrite the comments: the receipt never calls the classifier; the stored source is the post-deterministic envelope; the classifier runs in the Review-start pass. Delete `privacy_boundary_failure_fails_closed` (RT:5723-5753): its receipt-side assertions now live in the deterministic-failure test, and Task 8 adds `classifier_failure_retries_then_fails_closed` for the Review side. That test is the only construction site of RT's `FailingPrivacyBoundary` (RT:5710, constructed only at RT:5736; HTTP:7051 is a separate struct, still constructed at HTTP:7557), so deleting it alone would leave the struct unconstructed and `-D warnings` would fail the RT build (`struct FailingPrivacyBoundary is never constructed`). Make the struct configurable instead: `struct FailingPrivacyBoundary { fail_deterministic: bool }`, whose `rescrub_deterministic` bails when `fail_deterministic` and is `Ok(Vec::new())` otherwise, and whose `rescrub_classifier` bails when `!fail_deterministic` and is `Ok(Vec::new())` otherwise. Task 2's deterministic-failure test constructs it with `true`; Task 8's `classifier_failure_retries_then_fails_closed` constructs it with `false`. Update its doc comment (RT:5708-5709) and the reference at RT:2358. Fix the docs at RT:5800-5804, RT:5891-5893, RT:22193 ("transformed at receipt" -> "transformed by the privacy pass").
- [ ] **Step 4: Run green.** Commands of Step 2 -> PASS; then `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins` and `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run` (builds every test target, so an unconstructed double fails here, not in Task 8).
- [ ] **Step 5: Commit** `Run only the deterministic rescrub at the pipeline receipt`.

**Expected red between Task 2 and later tasks** (Tasks 2-9 land in one PR; do not ship between them):
- Until Task 5: RT `transformed_content_flows_to_score_and_replay_stays_exact` (RT:5806; Review is fed the deterministic-only source, so MARKER_SECRET reaches approved content), the export test near RT:22201, and HTTP `compatibility_bundle_through_http_with_review_privacy_withdrawal_and_export` at its MARKER_SECRET assertion (HTTP:7333). The last is a `1 passed`-gated CI step (ci.yml:446-450).
- Until Task 8: the same HTTP test's failing-boundary leg (HTTP:7551-7598), which expects the receipt to refuse (HTTP:7576, :7597).
- From Task 5 until Task 5 Step 4 fixes it: HTTP `real_http_receipt_completes_and_resumes_after_restart` (HTTP:802), whose exact `artifact_kind` list (HTTP:1025-1054: `[review_snapshot, submitted_envelope, worker_intermediate]`) gains a second `review_snapshot` row (the pass ref) once every Review dispatch stores a pass. It is not red after Task 2; list it here so a red run of it after Task 5 is attributed to Task 5, not to Task 8's whole-bin run.
Record the observed red set after Task 2 and check it equals this list; anything else is a regression of Task 2.

### Task 3: V117, the pass record on `pipeline_runs`

**Files:**
- Create: `migrations/V117__pipeline_privacy_pass.sql`.
- Modify: `crates/trace-commons-server/src/db/postgres.rs` (append the `(117, "pipeline_privacy_pass", include_str!(..))` row after the V116 row at :1762-1766, with a comment in the style of :1758-1761; optionally a V117 block in `versioned_pipeline_migration_shape_is_pinned`, :8073+, next to the V108 pin at :8228).
- Modify: UPG `RUNTIME_PIPELINE_GRANTS` (:136-167, a `// V117` group of the eight updatable columns on `pipeline_runs`); check what the version list at UPG:406 means (`[92, ..., 113]`, V116 is absent) before adding 117 to it.
- Modify: VP:1013-1051 `PipelineRunRecord`, VP:7220-7256 `pipeline_run_from_row`.

`privacy_pass_required` and its CHECK are added to V117 in Task 5, together with the code that records a pass; adding the CHECK here would fail every existing Review approval until Task 5. V117 is unreleased until the PR merges, so editing it in Task 5 is safe; recreate the test databases after that edit (Global constraints).

- [ ] **Step 1: Look up V108's CHECK names** on a migrated test database (inline column CHECK at V108:23, so PostgreSQL names it; expected `pipeline_attempt_artifacts_artifact_check`; and the named `pipeline_attempt_artifacts_approved_hash`, V108:42-43):

```bash
psql "$TRACE_COMMONS_PG_TEST_DATABASE_URL" -c "SELECT conname, pg_get_constraintdef(oid) FROM pg_constraint WHERE conrelid = 'pipeline_attempt_artifacts'::regclass AND contype = 'c'"
```

- [ ] **Step 2: Write the failing test** `v117_adds_the_privacy_pass_record` in UPG beside `v116_adds_source_and_pipeline_run_id_with_defaults` (:1564), `#[ignore]` like its neighbours (this is the suite's CI selector, `pipeline_upgrade -- --ignored`, ci.yml:416; not a placeholder). It asserts: the eight columns exist and are NULL on a run inserted before V117; an UPDATE setting only some of the six pass columns fails the shape CHECK; a `privacy_pass_outcome` outside `('cleared','escalated')` fails; a malformed hash fails; the two approval columns can be set only together and only when `privacy_pass_outcome = 'escalated'`; the FK to `trace_object_refs` is deferred (an insert of the ref after the run update in one transaction commits); `trace_ingest_runtime` holds UPDATE on the eight columns (via the `RUNTIME_PIPELINE_GRANTS` check that `pipeline_upgrade_from_v91_installs_forced_rls_storage`, UPG:378, runs); a `pipeline_attempt_artifacts` row with `artifact = 'privacy-pass'` and a ciphertext hash inserts, one with no ciphertext hash fails, and `'other'` still fails.
- [ ] **Step 3: Run red.** With `TRACE_COMMONS_PIPELINE_PG_UPGRADE_TEST_URL` set (Global constraints):

```bash
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --lib pipeline_upgrade -- --ignored --test-threads=1
```

  Expected: `v117_adds_the_privacy_pass_record` fails on its first column assertion (no such column), not on the env lookup. `every_migration_is_wired_into_run_migrations` (postgres.rs:7833) compares `MIGRATIONS` with the files on disk and is green before and after; it is a **regression pin (expected green)** for Step 6.
- [ ] **Step 4: Write the migration** (template: V95:4-26 and V95:52-60):

```sql
-- The Review-start privacy pass's record (spec 2026-10-09). A pass record,
-- not a classifier verdict (CMP-002): the object it stored, the hashes of
-- its input and output, the merged residual-risk labels, and whether it
-- held the run for a human. All six are NULL until the pass commits. The
-- two approval columns link a human approval of an escalated run to it.
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
    ADD COLUMN privacy_pass_approval_assessment_hash TEXT CHECK (
        privacy_pass_approval_assessment_hash IS NULL
        OR privacy_pass_approval_assessment_hash ~ '^sha256:[0-9a-f]{64}$'),
    ADD COLUMN privacy_pass_approval_resolved_reasons JSONB CHECK (
        privacy_pass_approval_resolved_reasons IS NULL
        OR jsonb_typeof(privacy_pass_approval_resolved_reasons) = 'array'),
    ADD CONSTRAINT pipeline_runs_privacy_pass_shape CHECK (
        (privacy_pass_object_ref_id IS NULL) = (privacy_pass_content_hash IS NULL)
        AND (privacy_pass_object_ref_id IS NULL) = (privacy_pass_source_hash IS NULL)
        AND (privacy_pass_object_ref_id IS NULL) = (privacy_pass_residual_risk_basis IS NULL)
        AND (privacy_pass_object_ref_id IS NULL) = (privacy_pass_outcome IS NULL)
        AND (privacy_pass_object_ref_id IS NULL) = (privacy_pass_recorded_at IS NULL)),
    ADD CONSTRAINT pipeline_runs_privacy_pass_approval_shape CHECK (
        (privacy_pass_approval_assessment_hash IS NULL)
            = (privacy_pass_approval_resolved_reasons IS NULL)
        AND (privacy_pass_approval_assessment_hash IS NULL
             OR privacy_pass_outcome IS NOT DISTINCT FROM 'escalated')),
    -- NO ACTION + DEFERRABLE for the reason V95 gives for approved_object_ref_fk.
    ADD CONSTRAINT pipeline_runs_privacy_pass_object_ref_fk
        FOREIGN KEY (tenant_id, submission_id, privacy_pass_object_ref_id)
        REFERENCES trace_object_refs (tenant_id, submission_id, object_ref_id)
        ON DELETE NO ACTION
        DEFERRABLE INITIALLY DEFERRED;

ALTER TABLE pipeline_attempt_artifacts
    DROP CONSTRAINT <artifact CHECK name from Step 1>,
    ADD CONSTRAINT pipeline_attempt_artifacts_artifact_check CHECK (
        artifact IN ('approved', 'index-command', 'score-neighbors', 'privacy-pass')),
    DROP CONSTRAINT pipeline_attempt_artifacts_approved_hash,
    ADD CONSTRAINT pipeline_attempt_artifacts_approved_hash CHECK (
        ciphertext_sha256 IS NOT NULL OR artifact NOT IN ('approved', 'privacy-pass'));

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_ingest_runtime') THEN
        RAISE EXCEPTION 'V117: trace_ingest_runtime is missing; V90 creates it';
    END IF;
END $$;

-- pipeline_runs: the privacy pass records its result; commit_review links an
-- escalated run's approval to it.
GRANT UPDATE (privacy_pass_object_ref_id, privacy_pass_content_hash,
              privacy_pass_source_hash, privacy_pass_residual_risk_basis,
              privacy_pass_outcome, privacy_pass_recorded_at,
              privacy_pass_approval_assessment_hash,
              privacy_pass_approval_resolved_reasons)
    ON pipeline_runs TO trace_ingest_runtime;
```

  The approval-shape CHECK uses `IS NOT DISTINCT FROM`, not `=` (corrected while implementing Task 3): with no pass the outcome is NULL, `NULL = 'escalated'` is NULL, and a CHECK lets NULL through, so the first draft accepted both approval columns on a run with no pass at all. `v117_adds_the_privacy_pass_record` pins it ("no approval link without a pass").

  The new `pipeline_attempt_artifacts_approved_hash` renders as `artifact <> ALL (ARRAY['approved'::text, 'privacy-pass'::text])`, so the definition pin in `pipeline_upgrade_from_v91_installs_forced_rls_storage` (which looked for `artifact <> 'approved'::text`) is updated to the new form in the same commit, and that test's recorded-version list gains 117 (UPG:406 lists the migrations that touch pipeline tables; V114-V116 touch none, V117 does).

  No RLS change: a column on an already-forced table inherits the tenant predicate (V52:40-44), and `TRACE_COMMONS_RLS_TABLES` (postgres.rs:188) lists tables only. No gate-driver grant. No new table, so `PIPELINE_TABLES` (RESTORE:~222-243) is unchanged. No grant on `trace_submissions` (P5).
- [ ] **Step 5: Wire the run record.** Add the eight fields (Interfaces; `privacy_pass_required` comes in Task 5) and map them in `pipeline_run_from_row` (JSONB labels decode the way the V52 reader does). Every store read is `SELECT *`/`RETURNING *` (VP:1990, 2182, 2515, 3016, ...), so nothing else changes; the one test literal (RT:22291) uses `..run.clone()`. `PipelineRunRecord` derives `Serialize`: the new fields are ids, hashes, labels and timestamps only.
- [ ] **Step 6: Run green.** Step 3's command -> PASS; `cargo test -p trace-commons-server --lib every_migration_is_wired_into_run_migrations` -> PASS; `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run`.
- [ ] **Step 7: Commit** `Add the privacy pass record to pipeline_runs (V117)`.

### Task 4: Pass object, crash points, store method

**Files:**
- Modify: VP:14476-14507 (`PipelineAttemptArtifact::PrivacyPass`, `as_str` "privacy-pass", `from_db_str`, `store_kind` `ContributionEnvelope`), the sweep comment at VP:10050-10057 ("the three known artifacts"), VP:14587 doc (lists the artifact names).
- Modify: beside VP:14814, `privacy_pass_object_ref` (id `Uuid::new_v5(&Uuid::NAMESPACE_URL, format!("tracecommons:pipeline-privacy-pass-object:{}", run.run_id).as_bytes())`, as `approved_object_ref` passes `.as_bytes()` at VP:14821-14824, kind `ReviewSnapshot`, `created_by_job_id: Some(run.run_id)`).
- Modify: VP:918-942 `PipelineCrashPoint` (+2 variants; fix the stale doc at :918-921).
- Modify: new `PgPipelineStore::record_privacy_pass` and `load_submission_receipt_privacy` beside `commit_review` (VP:2360).

- [ ] **Step 1: Write the failing tests.**
  - VP unit: `privacy_pass_artifact_round_trips` (`from_db_str(as_str())`, store kind `ContributionEnvelope`); `privacy_pass_object_ref_is_derived_from_the_run` (same id for two receipts of one run; differs from the approved ref id; kind `ReviewSnapshot`; `created_by_job_id == Some(run_id)`); `privacy_pass_ref_is_not_a_score_object` (`!is_pipeline_score_object_ref(id, Some(run_id))`, so the deletion worker takes the `ContributionEnvelope` arm, ING:65689-65692).
  - RT `record_privacy_pass_commits_once_under_the_lease`: claim a Review run, stage a `PrivacyPass` attempt artifact, call `record_privacy_pass`. The run row has all six pass columns; the ref row exists; the staged `privacy-pass` row is `committed`; `attempt_count` and `next_phase` unchanged. `trace_submissions` has the pass's raw `residual_pii_risk` as `privacy_risk` (the `enum_string` form, not the mapped `PrivacyRisk`; P5), `residual_risk_basis`, `redaction_counts` and `redaction_pipeline_version`; its `redaction_hash` is unchanged, and a `committed` `trace_token_bundles` row the test inserts for the submission before the call (V65 shape; inserted through `owner_tenant_tx`, RT:7223, with the tenant set, because the runtime login holds only SELECT and UPDATE(state, processing_state, processing_summary) on that table, V90:99-101) is still `committed`, not `revoked` (P5, V68; without the inserted row the assertion would pass vacuously). A second call with the same lease is refused (`privacy_pass_object_ref_id IS NULL` fence) and changes nothing. A stale lease token is refused. A withdrawn submission is refused with `submission_inoperable`. A call whose staged row is missing, or names another object key or ciphertext hash, is refused with `pipeline_attempt_artifact_missing` and rolls back (no ref row, no run columns).
  - RT `load_submission_receipt_privacy_reads_the_receipt_values`: after a receipt, returns the stored raw `residual_pii_risk` and basis labels. Include a consent-flag-only receipt: it reads back raw `Medium` with basis `[consent_content_flag]`, and `pipeline_privacy_risk` of those is Low, Admission's value.
- [ ] **Step 2: Run red.**

```bash
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --lib privacy_pass_
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --test versioned_pipeline_runtime_pg -- record_privacy_pass load_submission_receipt_privacy --test-threads=1
```

  Expected: compile failure (no variant, no helper, no store method).
- [ ] **Step 3: Implement** `record_privacy_pass`, modelled on `commit_review`'s single tenant transaction (VP:2360-2580): `ensure_current_lease` (lock order: run row, then submission row, as at VP:2374-2376); `review_submission_is_operable`; `INSERT INTO trace_object_refs ... ON CONFLICT DO NOTHING` (as VP:2410); `UPDATE trace_submissions SET privacy_risk = $, residual_risk_basis = $, redaction_counts = $, redaction_pipeline_version = $, updated_at = NOW()` (labels through `safe_residual_risk_basis_labels`; never `redaction_hash`); `UPDATE pipeline_runs SET privacy_pass_* = ..., privacy_pass_recorded_at = NOW(), updated_at = NOW() WHERE tenant_id AND run_id AND next_phase = 'review' AND lease_token = $ AND lease_expires_at > NOW() AND privacy_pass_object_ref_id IS NULL RETURNING *` (zero rows -> refused with `PIPELINE_PRIVACY_PASS_ALREADY_RECORDED_LABEL`; `ensure_current_lease` already passed under the run lock, so a missing row is the fence); then

```sql
UPDATE pipeline_attempt_artifacts
   SET state = 'committed', committed_at = NOW()
 WHERE tenant_id = $1 AND run_id = $2 AND lease_token = $3
   AND artifact = 'privacy-pass' AND state = 'staged'
   AND object_key = $4
   AND (ciphertext_sha256 IS NULL OR ciphertext_sha256 = $5)
```

  and require exactly one row moved, else `DatabaseError::Constraint(PIPELINE_ATTEMPT_ARTIFACT_MISSING_LABEL)` and roll back (the shape `commit_review` uses, VP:2541-2579). Without this, a `privacy-pass` row left `staged` under the lease would make a same-dispatch rejection fail, because `commit_review`'s rejection branch moves every staged row of the lease and demands none moved (VP:2566-2580). No `attempt_count` reset, no phase change, no `lock_runnable_policy` (the pass is a server control). `load_submission_receipt_privacy` reads `trace_submissions.privacy_risk` (raw `ResidualPiiRisk`) and `residual_risk_basis` in a tenant transaction; the caller maps them (P3). Then grep: `grep -rn "redaction_hash(" crates/trace-commons-server/src` shows no reader that recomputes the hash from stored `redaction_counts` (PROTO:1543 checks only the prefix); record the result in the commit message.
- [ ] **Step 4: Run green.** Step 2's commands -> PASS; `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins`.
- [ ] **Step 5: Commit** `Add the privacy pass object, crash points and record transaction`.

### Task 5: The privacy pass in the Review dispatch, and the approval guard

**Files:**
- Modify: VP:11216-11244 (Review arm), new `ensure_privacy_pass` and `load_privacy_pass_bytes` beside `load_source_bytes` (VP:10292-10294). `load_privacy_pass_bytes` goes through `load_object_bytes` (VP:10238-10289, which enforces operability, invalidation and deletion and ends in `decode_pipeline_artifact_bytes`, VP:10289) and checks `sha256_prefixed(bytes) == privacy_pass_content_hash` like `load_approved_bytes` (VP:10300-10312), failing with `artifact_integrity_failed`.
- Modify: VP:2425-2441 (`commit_review`'s `trace_derived_records` insert: `input_object_ref_id` becomes `run.privacy_pass_object_ref_id` when set, so it names the object whose hash it stores in `input_hash`; a run with `privacy_pass_required = FALSE` and no pass keeps the source ref).
- Modify: VP:2503-2526 `commit_review` approval UPDATE (critique 0) and `migrations/V117__pipeline_privacy_pass.sql` (append the `privacy_pass_required` block).
- Test: RT, MarkerRedacting tests (RT:5806, RT:~22201, HTTP:7223/7272/7333), crash matrix (RT:14080-14100).

Flow of `ensure_privacy_pass(run)`:
1. If `run.privacy_pass_object_ref_id` is set: return `load_privacy_pass_bytes(run)`; no classifier call.
2. Else: `privacy = self.privacy.as_ref()` or `privacy_control_missing` (P6). `source = load_source_bytes(run)`; `source_hash = sha256_prefixed(&source)`; deserialize `TraceContributionEnvelope` from `source` (the receipt stores `serde_json::to_vec(envelope)` inside the P1 wrapper, VP:9123-9124; `load_source_bytes` unwraps it); `receipt = load_submission_receipt_privacy(..)`; `tokio::time::timeout(self.privacy_pass_timeout(), privacy.rescrub_classifier(&mut envelope))`; an error or timeout raises `PolicyError::permanent(privacy_classification_failed)` (Task 8 routes it).
3. Merge: the receipt basis (parsed with `ResidualRiskCondition::from_label`; an unknown label fails closed with `privacy_classification_failed`) plus classifier conditions, no duplicates. `risk = pipeline_privacy_risk(&envelope.privacy.residual_pii_risk, &merged)`; `receipt_risk = pipeline_privacy_risk(&receipt.residual_pii_risk, &parsed receipt basis)`. `outcome = Escalated` iff `risk > receipt_risk` (P3), else `Cleared`. Never compare against the raw stored value: it is on a different scale for a consent-flag-only trace.
4. `bytes = serde_json::to_vec(&envelope)`; `content_hash = sha256_prefixed(&bytes)`; `let wrapper = encode_pipeline_artifact_bytes(&bytes)?` (as VP:9124 and VP:11263 do); `prepare_serialized_json(.., TraceArtifactKind::ContributionEnvelope, &pipeline_attempt_object_id("privacy-pass", run_id, lease_token), &wrapper)`; `stage_attempt_artifact(run, PrivacyPass, .., self.attempt_artifact_cleanup_after(Phase::Review))`; publish (both on the blocking pool via `artifact_store_call`, as VP:11265-11289); `inject_crash(AfterPrivacyPassArtifactStorage)`.
5. `record_privacy_pass(...)` with the envelope's raw `residual_pii_risk` (not `risk`, P5), merged labels, `envelope.privacy.redaction_counts` and `redaction_pipeline_version`; on a refused commit delete this attempt's object (as VP:11323-11330); `inject_crash(AfterPrivacyPassCommit)`; return the updated run and `bytes`.

The Review arm then passes `bytes` as `source_artifact` and `sha256_prefixed(&bytes)` as `source_content_hash` (VP:11221-11222). `MinimalReviewPolicy` only requires `dependency_content_hash(source_artifact) == source_content_hash` (BUNDLE:188-191), so the bundle does not change.

V117 addition (critique 0):

```sql
-- A run received from here on needs a privacy pass before Review may
-- approve it. Existing rows are exempt (FALSE); the default then flips, so
-- a receipt written by either binary gets TRUE, and an approval by a binary
-- that has no pass fails this CHECK instead of approving unclassified bytes.
ALTER TABLE pipeline_runs
    ADD COLUMN privacy_pass_required BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE pipeline_runs
    ALTER COLUMN privacy_pass_required SET DEFAULT TRUE,
    ADD CONSTRAINT pipeline_runs_privacy_pass_before_approval CHECK (
        NOT privacy_pass_required
        OR approved_object_ref_id IS NULL
        OR privacy_pass_object_ref_id IS NOT NULL);
```

`commit_review`'s approval UPDATE (VP:2503-2526) gains `AND ($6::uuid IS NULL OR NOT privacy_pass_required OR privacy_pass_object_ref_id IS NOT NULL)` and becomes `query_opt`; `None` after `ensure_current_lease` has passed under the run lock means the pass is missing, and returns `DatabaseError::Constraint(PIPELINE_PRIVACY_PASS_MISSING_LABEL)` (a safe label, never the raw CHECK error). The CHECK stays as the backstop for a binary that lacks the predicate.

- [ ] **Step 1: Write the failing tests** (RT; per-test tenant and counters):
  - `privacy_pass_runs_once_and_feeds_review` (counting `rescrub_classifier` that replaces a planted span): one `process_one` takes the run through Review; classifier counter 1; the run has a pass record with `outcome = cleared`, `privacy_pass_source_hash == sha256(stored source bytes)`, `privacy_pass_content_hash == review outcome's source_content_hash`; one `review_snapshot` ref with `created_by_job_id = run_id`; the stored pass object satisfies `is_pipeline_artifact_wrapper`; the `trace_derived_records` row's `input_object_ref_id` is the pass ref.
  - `score_never_sees_planted_prose_pii`: the source carries `MARKER_SECRET`; the stored source object still contains it; `load_approved_bytes` for the run does not; Score completes.
  - `privacy_pass_crash_before_commit_repeats_the_call_and_commits_one_ref`: crash at `AfterPrivacyPassArtifactStorage` (service A); expire the lease; service B on the same database: the next dispatch calls the classifier a second time, commits exactly one pass ref with the run-derived id; the first attempt's staged row is still `staged`; after its `cleanup_after`, `sweep_attempt_artifacts` deletes the orphaned object (corrected spec test, correction 1). Red: the crash point does not exist or never fires, and the counter is wrong.
  - `privacy_pass_crash_after_commit_does_not_call_again`: crash at `AfterPrivacyPassCommit`; the next dispatch makes no classifier call and Review completes with the recorded bytes. This is the pass-resume proof that replaces the restore-drill run (critiques 10/19).
  - `privacy_pass_lease_loss_records_one_result` (P7, critique 8): worker A's classifier double blocks on a per-test `Notify`; while it blocks, expire A's lease in SQL and let worker B reclaim and complete the pass; release A. Assert: classifier counter 2, exactly one recorded `privacy_pass_object_ref_id` (B's), A's `record_privacy_pass` refused, A's object deleted or left `staged` for the sweep, and after the sweep only the committed object remains.
  - `approval_without_a_pass_is_refused` (critique 0): a run received after V117 (`privacy_pass_required = TRUE`), claimed for Review, then `commit_review` called directly with an approved artifact and no pass: refused with `privacy_pass_missing`; the run is unchanged and no approved ref exists. A raw `UPDATE pipeline_runs SET approved_object_ref_id = ...` as the migrator (`owner_client`, RT:14718) on that run fails the CHECK; this is the one place the raw-owner write is the point, because the CHECK itself is under test. The same approval on a run with `privacy_pass_required = FALSE` (set by fixture SQL through `owner_tenant_tx`, RT:7223, with the tenant set: the runtime login connects as `RUNTIME_ROLE`, RT:126-148, and V117 grants it UPDATE on the eight pass columns only, not on `privacy_pass_required`; modelling a pre-V117 row) commits.
  - Add `AfterPrivacyPassArtifactStorage` and `AfterPrivacyPassCommit` to `crash_matrix_produces_one_logical_effect_per_point` (RT:14080; its evidence records a count, RT:14290-14299). This one cannot fail on its own (the matrix asserts per-run effects); it is a **regression pin**, red only through the compile error until the variants exist.
  - **Regression pins (expected green after Task 5, already red since Task 2):** `transformed_content_flows_to_score_and_replay_stays_exact` (RT:5806) and the export test (RT:~22201): assertions on approved content and export output stay; an assertion that the stored source is redacted moves to the pass object. `stored_source_is_post_deterministic` (a deterministic secret is gone from the stored source, a prose marker is kept) holds since Task 2 and is a **regression pin (expected green)**.
- [ ] **Step 2: Run red.**

```bash
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --test versioned_pipeline_runtime_pg -- privacy_pass_ score_never_sees approval_without_a_pass crash_matrix --test-threads=1
```

  Expected: compile errors first (no crash points, no label); once those exist, the pass tests fail on the missing record and classifier count, and `approval_without_a_pass_is_refused` fails because the approval commits.
- [ ] **Step 3: Implement** the flow, the Review arm hand-off, the V117 block and the `commit_review` predicate. Recreate the test databases (V117 changed).
  Note: a crash after `record_privacy_pass` and before the policy costs one Review attempt (the pass transaction does not refund it; `mark_awaiting_review` does, VP:5248). Acceptable at 5 attempts, which `commit_review` resets on approval.
- [ ] **Step 4: Audit object-count assertions.** Every Review dispatch now stores one more object and one more ref, also with the no-op doubles. `grep -n "count_staged_artifacts\|count_files_under" crates/trace-commons-server/tests/versioned_pipeline_runtime_pg.rs | wc -l` (62 at the plan's commit) and the same over `src/bin/trace_commons_ingest_internal/`; fix each count on a path that reached Review. Also audit assertions on the kinds and number of object refs, which the count helpers do not cover: `grep -n "FROM trace_object_refs\|artifact_kind\|review_snapshot" crates/trace-commons-server/tests/versioned_pipeline_runtime_pg.rs crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_http_pg_tests.rs crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs` (214 lines over the three files at the plan's commit, most of them `artifact_kind` in unrelated legacy tests; the narrower `FROM trace_object_refs\|review_snapshot` gives 22). Known sites: HTTP `real_http_receipt_completes_and_resumes_after_restart` (HTTP:802), whose exact kind list (HTTP:1025-1054) becomes `[review_snapshot, review_snapshot, submitted_envelope, worker_intermediate]` (or filter the pass ref out by `created_by_job_id` and assert it separately); RT:8572-8589 and RT:8881-8891, which assert zero `review_snapshot` refs on store-level paths (`withdrawal_during_review_refuses_commit_and_stays_revoked`, RT:8454, and `withdrawal_during_the_review_commit_race_fails_the_run_and_deletes_the_object`, RT:8824; RT:8454 seeds and claims with no dispatch, so no pass runs and 0 holds; check RT:8824 the same way; if Step 5's helper records a pass on such a run first, the assertion must exclude the pass ref by `created_by_job_id = run_id` or expect 1). The restore drill's object and artifact fingerprints are computed from the seed, not fixed (`pipeline.py`:820-840; RESTORE:3224's `object_count: 3` is a directory-double fixture of its own and does not move).
- [ ] **Step 5: Audit direct `commit_review` callers.** `grep -n "\.commit_review(" crates/trace-commons-server/tests/versioned_pipeline_runtime_pg.rs` (7 sites, including the store-level `review_snapshot` counts at RT:8573 and RT:8884): each approval on a fresh run now needs a pass. Record one through a test helper that stages and calls `record_privacy_pass`, or, where the test is about a pre-V117 run, set `privacy_pass_required = FALSE` in fixture SQL through `owner_tenant_tx` (RT:7223; the runtime login cannot update that column) and say so in a comment. (Implemented: RT helper `mark_received_before_v117(tenant, run_id)`; the two store-level approvals on seeded runs, `the_sweep_deletes_the_objects_a_refused_score_commit_left_stored` and `a_committed_attempt_keeps_its_objects_and_a_stale_attempt_loses_them`, call it. The other five direct callers are rejections or refused before the approval UPDATE and need nothing.) The fourth argument is added in Task 7, where it is first used, with `None` at every existing caller then.
- [ ] **Step 6: Audit services built without a boundary** (P6). `grep -n "privacy: None" crates/trace-commons-server/tests crates/trace-commons-server/src/bin/trace_commons_ingest_internal` (one hit at RT:3187, a production-assembly readiness test) and every `test_service_with_controls(.., None)` call (RT:2638-2643): a service that drives Review must get `default_privacy_boundary()` (RT:2374). The restore seed already uses `PassThroughPipelinePrivacyBoundary` (RESTORE:1407).
- [ ] **Step 7: Run green**: Step 2's command; the existing tests this task touches:

```bash
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --test versioned_pipeline_runtime_pg -- transformed_content_flows stored_source_is_post_deterministic --test-threads=1
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --bin trace-commons-ingest compatibility_bundle_through_http_with_review_privacy_withdrawal_and_export
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --bin trace-commons-ingest real_http_receipt_completes_and_resumes_after_restart
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --lib pipeline_upgrade -- --ignored --test-threads=1
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
```

  The HTTP test is still red at the failing-boundary leg (HTTP:7551-7598) until Task 8, and must be green at its MARKER_SECRET assertion (HTTP:7333). Extend `v117_adds_the_privacy_pass_record` with the `privacy_pass_required` assertions (FALSE on a pre-V117 row, TRUE on a new insert, CHECK refusal).
- [ ] **Step 8: Commit** `Run the classifier as a server-owned privacy pass at the start of Review`.

Recorded while implementing Task 5:
- `commit_review`'s fourth argument (`pass_approval`) moves to Task 7. In Task 5 it would be an unused parameter (`-D warnings`); Task 7 adds it, writes the two approval columns, and passes `None` at the existing callers.
- `privacy_pass_timeout()` exists from Task 5 and returns the 900 s ceiling (`PIPELINE_PRIVACY_PASS_MAX_TIMEOUT`) only; Task 8 replaces its body with the P8 lease-derived bound. A classifier error or timeout is raised as `PolicyError::permanent(privacy_classification_failed)` from Task 5, so the existing non-transient branch already charges it under its own label; Task 8 adds only the backoff, the terminal label and the timeout bound.
- A missing boundary is raised as a bare `privacy_control_missing` in Task 5 (recorded as `minimal_policy_failed`, charged) until Task 8 routes it to `mark_transient_retry`.
- A stored source that does not decode as an envelope fails the pass with `artifact_integrity_failed` (allowlisted, charged); an unknown receipt basis label fails it with `privacy_classification_failed`, as planned.
- The tests drive dispatches with `process_run` (there is no `process_one`). `stored_source_is_post_deterministic` did not exist; it is written new in Task 5 (a double whose deterministic half removes `DETERMINISTIC_SECRET` and whose classifier half removes `MARKER_SECRET`) and passed in the red run, as a regression pin should.
- Step 4's audit, by the full RT run and the whole ingest bin: object counts move by one (the pass object) in `a_refused_score_commit_deletes_the_objects_its_attempt_wrote`, `a_review_commit_refused_for_a_stale_lease_leaves_no_object`, `a_score_commit_refused_for_a_stale_lease_leaves_no_object`, `a_failed_score_write_deletes_the_object_written_before_it`, `a_score_commit_missing_its_attempt_row_is_refused_and_leaves_no_object`, `a_review_commit_missing_its_attempt_row_is_refused_and_leaves_no_object`, `a_score_commit_whose_lease_ends_before_its_run_update_deletes_its_objects`, `a_policy_suspended_during_a_phase_cannot_commit_it`, `withdrawal_during_the_review_commit_race_fails_the_run_and_deletes_the_object` (its `review_snapshot` count now excludes the pass ref by `created_by_job_id IS NULL` and asserts the pass ref separately); attempt-row lists gain a committed `privacy-pass` row in `a_crashed_score_attempt_leaves_staged_objects_the_sweep_removes`, `a_refused_score_commit_leaves_staged_rows_the_sweep_removes` and `the_attempt_sweep_calls_the_store_off_the_runtime_workers`; `score_attempt_rows_with_hashes` and `score_attempt_lease_tokens` exclude `privacy-pass` like `approved`. HTTP: `real_http_receipt_completes_and_resumes_after_restart` lists two `review_snapshot` refs; `the_revocation_worker_deletes_every_object_of_a_withdrawn_complete_run` and `the_revocation_worker_deletes_the_score_objects_of_a_withdrawn_run` count five refs and five completed deletions (the pass object included). RT:8572 (`withdrawal_during_review_refuses_commit_and_stays_revoked`) is refused before any pass and is unchanged.

### Task 6: Escalation hold, review-queue visibility, assessment timing

**Files:**
- Modify: VP:115-125 `run_waiting_for_review_sql!` ->

```sql
p.next_phase = 'review'
AND (p.admission_decision = 'quarantine' OR p.privacy_pass_outcome = 'escalated')
AND (NOT p.privacy_pass_required OR p.privacy_pass_object_ref_id IS NOT NULL)
AND p.state IN ('pending', 'retry', 'awaiting_review')
```

  The third line is critique 1 (1): a run that requires the pass is claimable, listable and assessable only once its pass is recorded. The macro's only users are `claim_review` (VP:2644), `record_review_assessment` (VP:2782) and `list_review_queue` (VP:3020); no worker claim path uses it, so dispatch is unaffected. Pre-V117 runs (`privacy_pass_required = FALSE`) keep today's behaviour. Update its doc and `claim_review`'s doc (VP:2596-2605, which calls claiming an Admit run a bug).
- Modify: VP:2817-2832 approve check: the reasons to resolve are `admission_reason` (if set) and `privacy_pass_review_required` (if `privacy_pass_outcome = 'escalated'`; select it at the row read above :2812). An Approve that does not list every one is refused with the existing `quarantine reason is unresolved` (422 via ING:~43388).
- Modify: VP:2899-2930 `load_review_assessment`: also select `recorded_at`.
- Modify: VP:10891-10896 parking branch: also park on `PIPELINE_PRIVACY_PASS_REVIEW_REQUIRED_LABEL` when `next_phase == Review`.
- Modify: Review arm, after `ensure_privacy_pass` and the assessment load (VP:11229-11232): if the pass outcome is `Escalated` and there is no assessment, or the assessment's `recorded_at` is before `privacy_pass_recorded_at` (critique 1 (2), P9), return `PolicyError::transient(privacy_pass_review_required)`; the policy is not called.
- Modify: `list_review_queue` (VP:3005-3031) returns `Vec<PipelineReviewQueueEntry>` (Interfaces) instead of `Vec<PipelineRunRecord>`, which cannot carry these fields: `hold_reason: Option<String>` (`privacy_pass_review_required` when the pass escalated, else `admission_reason`) and `assessment_superseded: bool` (P9), computed in SQL from a LEFT JOIN on `pipeline_review_assessments` (its `recorded_at` against `p.privacy_pass_recorded_at`). Its `AND NOT EXISTS (SELECT 1 FROM pipeline_review_assessments a ...)` (VP:3021-3022) becomes "no assessment, or, on a run whose `privacy_pass_outcome = 'escalated'`, an assessment recorded before `p.privacy_pass_recorded_at`", so a superseded run is listed and an operator sees it. ING `PipelineReviewQueueItem` (ING:43404-43409) gains `hold_reason` and `assessment_superseded`, and the handler's `.map(|run| ...)` (ING:43430-43435) maps the entry (`entry.run.run_id`, ...). Additive JSON fields; labels only. Every other caller of `list_review_queue` (grep `list_review_queue(` in RT, HTTP and `tests.rs`) reads `.run`.

- [ ] **Step 1: Write the failing tests** (RT; escalation double: `rescrub_classifier` sets `residual_pii_risk = Medium` and returns `[FoundAndRemoved]`; a High variant):
  - `escalated_admit_run_parks_for_a_human`: Admission admits at Low; one dispatch leaves the run `awaiting_review`, `last_error_label = privacy_pass_review_required`, attempt refunded (`mark_awaiting_review`, VP:5236-5260), outcome `escalated`, no Review outcome row, a counting Review policy wrapper not called; `list_review_queue` lists it with `hold_reason = privacy_pass_review_required`; `claim_review` claims it. Repeat with High: same result, never rejected (D1).
  - `escalated_approval_must_resolve_privacy_pass_review_required`: an Approve listing nothing, or listing another reason, is refused; listing `privacy_pass_review_required` is accepted and releases the run to `pending`.
  - `escalated_approval_resumes_without_a_second_classifier_call`: after the approval, one dispatch completes Review; classifier counter still 1; the Review policy is called once with the pass bytes.
  - `quarantined_run_is_not_claimable_before_its_pass` (critique 1 (1)): a deterministic Medium receipt (Admission quarantines); before any dispatch, `list_review_queue` does not list it and `claim_review` is refused; after one dispatch (pass recorded, policy parks with `review_assessment_required`), both succeed.
  - `quarantined_run_escalated_to_high_is_held_for_both_reasons` (the critic's scenario, with (1) in place): deterministic Medium, one dispatch with a High classifier: parked with `privacy_pass_review_required`, outcome `escalated`; an Approve resolving only `privacy_review_required` is refused; one resolving both is accepted; the next dispatch approves through the policy. Never approved on an assessment recorded before the pass.
  - `assessment_recorded_before_the_pass_is_ignored` (critique 1 (2), P9): a run with `privacy_pass_required = FALSE` (fixture SQL through `owner_tenant_tx`, RT:7223, with the tenant set: V117 grants the runtime UPDATE on the eight pass columns only, not on `privacy_pass_required`, so the RT runtime login gets permission denied; modelling a pre-V117 row), Admission quarantine, an Approve assessment recorded before any dispatch, then a dispatch whose classifier returns High: the run is held `awaiting_review` with `privacy_pass_review_required`, not approved; the queue lists it with `assessment_superseded = true`; `claim_review` is refused.
  - `admission_quarantine_not_escalated_parks_as_today`: deterministic Medium, classifier finds nothing more: outcome `cleared`, parked by the policy with `review_assessment_required`; approval must resolve the Admission reason as today.
  - `consent_flag_only_run_escalated_by_the_classifier_is_held` (P3 scale): deterministic basis exactly `[ConsentContentFlag]` at Medium, so the submission row stores raw `medium` and Admission admits at Low; the classifier sets Medium and adds `FoundAndRemoved`. Expected: outcome `escalated`, the run `awaiting_review` with `privacy_pass_review_required`, the Review policy not called, and `trace_submissions.privacy_risk` = raw `medium` (not mapped `low`). Red until the pass maps the receipt-time risk: a raw-column comparison gives Medium > `medium` false, `cleared`, and the run is approved.
  - **Regression pin (expected green):** `cleared_consent_flag_only_run_goes_straight_to_review`: deterministic basis `[ConsentContentFlag]`, Medium, classifier finds nothing: outcome `cleared`, no hold.
  - HTTP `pipeline_review_queue_lists_an_escalated_run_with_its_hold_reason`: the quarantine queue route returns `hold_reason = privacy_pass_review_required` for an escalated run.
- [ ] **Step 2: Run red.**

```bash
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --test versioned_pipeline_runtime_pg -- escalated quarantined_run assessment_recorded_before_the_pass admission_quarantine_not_escalated cleared_consent consent_flag_only_run_escalated --test-threads=1
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --bin trace-commons-ingest pipeline_review_queue_lists_an_escalated_run_with_its_hold_reason -- --test-threads=1
```

  Expected: the escalated runs are not parked (the policy approves them), including `consent_flag_only_run_escalated_by_the_classifier_is_held`; the quarantined run is claimable before its pass; the HTTP test fails on the missing field. `cleared_consent_flag_only_run_goes_straight_to_review` passes.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Audit existing review flows** (critique 1 (1) changes when a quarantined run becomes claimable): `grep -n "\.claim_review(\|record_review_assessment(" crates/trace-commons-server/tests/versioned_pipeline_runtime_pg.rs` (17 `claim_review` sites) and the review routes in HTTP and `tests.rs` (`grep -n "review/pipeline" ...`). Every flow that claims a run received after V117 must dispatch once first (so the pass is recorded); CORPUS already waits for `awaiting_review` (CORPUS:1094-1106).
- [ ] **Step 5: Run green**: Step 2's commands; the existing queue tests near RT:4824 and RT:4840; `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --test versioned_pipeline_runtime_pg -- review --test-threads=1`.
- [ ] **Step 6: Commit** `Hold a run the privacy pass escalates for a human review`.

Recorded while implementing Task 6:
- `load_review_assessment` returns `Option<StoredReviewAssessment>` (`{ assessment, recorded_at }`), the store-side wrapper the Interfaces section allows: `HumanReviewAssessment` is a gate-api type. Its two callers (the Review arm, one HTTP test) read `.assessment` or `.is_some()`.
- The hold check in the Review arm applies only when the recorded pass outcome is `escalated`. A `cleared` run keeps using an assessment recorded before its pass (Task 7's `quarantined_rejection_same_dispatch_commits_after_the_pass` depends on it).
- RT has no seam that injects a counting Review policy (the bundle is built from the package). "The Review policy is not called" is asserted structurally instead: no `Phase::Review` outcome row, no approved ref, and the run parked under `privacy_pass_review_required`. On an Admission-admitted run `MinimalReviewPolicy` approves unconditionally, so a call would commit an outcome.
- Red run: every escalation test failed because the run was approved (`Pending` at Score) instead of parked. That includes `consent_flag_only_run_escalated_by_the_classifier_is_held`, whose receipt-time mapping Task 5 already does. `quarantined_run_is_not_claimable_before_its_pass` failed because the run was listed before its pass. `admission_quarantine_not_escalated_parks_as_today` and `cleared_consent_flag_only_run_goes_straight_to_review` passed (regression pins). The HTTP queue test failed at the parked-state assertion, before it reached the missing field.
- Step 4 audit, by the full RT run and the whole ingest bin: RT `claim_and_assessment_refuse_a_run_a_worker_holds_leased` needs a run that is still `pending` and that a reviewer can claim, so it calls `mark_received_before_v117`. HTTP gains `parked_pipeline_run` (the receipt plus one dispatch). The review-route tests use it: `pipeline_review_routes_answer_409_422_and_404_for_an_inoperable_run`, `pipeline_review_routes_append_hash_only_audit_rows`, `a_failed_review_audit_append_answers_the_committed_result` and `the_pipeline_assessment_route_applies_mains_privileged_action_consent_check`. No other test changed.

### Task 7: Human decisions on an escalated Admit run (Q2)

**Files:** Review arm (VP:11233 onwards); a server-side `PhaseResult` builder next to it; `commit_review` gains its fourth argument `pass_approval` here (moved from Task 5; `None` at every existing caller) and its approval UPDATE (VP:2503-2526) writes the two approval columns.

- [ ] **Step 1: Write the failing tests** (RT):
  - `escalated_rejection_ends_the_run_rejected`: escalated Admit run, Reject assessment; one dispatch commits Review with `ReviewDecision::Rejected { reason: <assessment reason> }`, evidence `source_content_hash` = pass content hash, `human_assessment_hash` = the assessment's `evidence_hash`, `rule_id = "privacy_pass_human_review_rejected_v1"`; the submission is `rejected`, the run `complete` with no next phase (as `commit_review`'s rejection, VP:2463-2477, 2491-2497); no approved object; the Review policy is not called; the classifier counter stays 1.
  - `escalated_approval_links_the_assessment`: escalated Admit run, Approve assessment resolving `privacy_pass_review_required`; after the approving dispatch the run row has `privacy_pass_approval_assessment_hash` = the assessment's `evidence_hash` and `privacy_pass_approval_resolved_reasons = ["privacy_pass_review_required"]`, and the approved Review outcome's run carries them (the outcome itself is the bundle policy's, `minimal_review_passthrough_v1`). A non-escalated run's approval leaves both NULL.
  - `quarantined_rejection_same_dispatch_commits_after_the_pass` (critique 4): a pre-V117-style run (`privacy_pass_required = FALSE` by fixture SQL through `owner_tenant_tx`, RT:7223; the runtime login holds no UPDATE on that column), Admission quarantine, a Reject assessment already recorded, no pass: one dispatch runs the pass (recording and committing its staged row) and the policy's rejection commits in the same dispatch; no `pipeline_attempt_artifact_missing`.
- [ ] **Step 2: Run red.**

```bash
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --test versioned_pipeline_runtime_pg -- escalated_rejection escalated_approval_links --test-threads=1
```

  Expected: `escalated_rejection_ends_the_run_rejected` fails because the run is approved (`MinimalReviewPolicy` ignores the assessment on Admit, BUNDLE:192-194); `escalated_approval_links_the_assessment` fails on NULL columns. `quarantined_rejection_same_dispatch_commits_after_the_pass` is a **regression pin (expected green)** of Task 4's `moved == 1` rule.
- [ ] **Step 3: Implement.** When Admission is Admit, the outcome is `Escalated`, and the assessment is a current Reject: build the `PhaseResult` mirroring BUNDLE:204-221 with `PIPELINE_PRIVACY_PASS_REJECTED_RULE_ID`, then `commit_review(run, StoredPhaseResult::from_result(Phase::Review, &result)?, None, None)`. A current Approve goes to the bundle policy with the pass bytes, and the Review arm passes the assessment as `commit_review`'s `pass_approval`, which adds `privacy_pass_approval_assessment_hash = $, privacy_pass_approval_resolved_reasons = $` to the approval UPDATE when `privacy_pass_outcome = 'escalated'`. An Admission-quarantined escalated run goes through the policy as today (it records the assessment hash itself, BUNDLE:255-256), and also gets the two columns. Provenance needs nothing extra: `insert_outcome` writes `outcome_schema_id`/`version` from `SchemaRef::pipeline_v1()` and `bundle_id` from the run (VP:6284-6310), so the export join on `review.outcome_schema_id` (VPP:450-466) is unaffected; only `evaluation.rule_id` marks the rejection as the server's.
- [ ] **Step 4: Run green**: Step 2's command plus `quarantined_rejection_same_dispatch_commits_after_the_pass` and Task 6's tests (`-- escalated quarantined_run --test-threads=1`).
- [ ] **Step 5: Commit** `Commit a reviewer's decision on an escalated run`.

Recorded while implementing Task 7:
- The server rejection is `PipelineService::commit_privacy_pass_rejection`. Its `PhaseResult` goes through `ReviewOutput::rejected` (the contract's own shape check) before `commit_review(run, .., None, None)`. The refusal re-raise that the Review arm did inline (bare safe labels) is factored into `review_commit_refusal`, which both paths use.
- `commit_review` writes the two approval columns with `CASE WHEN privacy_pass_outcome = 'escalated' THEN $n ELSE <column> END`, so a `pass_approval` passed for a run whose pass did not escalate is ignored rather than tripping `pipeline_runs_privacy_pass_approval_shape`; it is also ignored on a rejection. The Review arm passes the current assessment only when the pass escalated. The stored reasons are the assessment's `resolved_quarantine_reasons` as `record_review_assessment` stored them (sorted, deduplicated).
- `quarantined_run_escalated_to_high_is_held_for_both_reasons` (Task 6) gains the Q2 assertion for an Admission-quarantined escalated run: both columns set, reasons `["privacy_pass_review_required", "privacy_review_required"]`.
- Red run: `escalated_rejection_ends_the_run_rejected` failed with the run `Pending` at Score (approved); `escalated_approval_links_the_assessment` failed on a NULL `privacy_pass_approval_assessment_hash`; `quarantined_rejection_same_dispatch_commits_after_the_pass` passed (regression pin). The tests use the rule id literal, so they compiled before the constant existed.

### Task 8: Classifier failure: charged retry, legacy backoff, terminal label, pass timeout, status

**Files:** VP:5048-5101 `mark_retry`; VP:10880-10925 (routing); pass timeout (P8) beside VP:775-790 and the service builder; RT:5710-5760; HTTP:7051-7061 and the section at HTTP:7551-7598; ING:17406-17422 `main_status_for_pipeline` (Q1).

Routing: the pass raises the error as a `PolicyError` value, `anyhow::Error::from(PolicyError::permanent(PIPELINE_PRIVACY_CLASSIFICATION_FAILED_LABEL)?)`, never `anyhow!(label)`: the dispatch matches on `error.downcast_ref::<PolicyError>()` (VP:10881), and a bare string falls through to the P2 allowlist and is recorded as `minimal_policy_failed`. The existing non-transient branch (VP:10900-10902) takes it to the charged `mark_retry_or_record_lease_expired` under its own label; no allowlist edit. It must never be `transient` (that goes to the uncharged `mark_transient_retry`, VP:5108-5152, which has no terminal bound). `privacy_control_missing` (P6) goes to `mark_transient_retry` beside the FR3 gaps at VP:10913-10923.

`mark_retry` changes (one place, no new primitive): (a) for `error_label == PIPELINE_PRIVACY_CLASSIFICATION_FAILED_LABEL`, the delay is `PIPELINE_PRIVACY_RETRY_BASE_SECONDS * 2^(attempt_count-1)` seconds (30, 60, 120, 240 s before the fifth attempt); (b) on exhaustion that label is written as itself instead of `attempts_exhausted`. A worker that crashes mid-pass and is swept by `claim_next` (VP:2068-2086) still gets `attempts_exhausted`; that is a crash, not a classifier verdict, and is documented.

- [ ] **Step 1: Write the failing tests.**
  - RT `classifier_failure_retries_then_fails_closed` (boundary `FailingPrivacyBoundary { fail_deterministic: false }`, Task 2): the receipt succeeds (`processing`, one run); each Review dispatch leaves the run `retry` with `last_error_label = privacy_classification_failed`, `attempt_count` incremented, `next_attempt_at - NOW()` within 30 s x 2^(n-1) plus tolerance (assert on the stored interval, not wall-clock sleeps); after `max_attempts` the run is `failed` with `privacy_classification_failed`; no pass record, no approved object, no Review outcome; `trace_submissions.status` still `received`. Drive the clock by setting `next_attempt_at = NOW()` in SQL between dispatches.
  - RT `classifier_timeout_is_a_classifier_failure`: a double that sleeps past a test-shortened ceiling (the builder knob, P8) gives the same `retry` state.
  - VP unit `privacy_pass_timeout_fits_inside_the_review_lease_cap` (P8): Review lease 300 s -> 900 s; 120 s -> 420 s; with a classifying boundary, 20 s -> the build refuses with `pipeline_lease_config_invalid`; with a non-classifying boundary, 20 s -> builds with a 900 s bound; 5 s -> builds with a 900 s bound (no negative value).
  - RT `missing_boundary_is_an_uncharged_wait`: submit through service A built with `default_privacy_boundary()`, then dispatch Review through service B on the same database built with `privacy: None` (the crash matrix's two-service pattern, RT:14148-14168; a service without a boundary cannot take the receipt, VP:9064-9067). The run is left `retry` with `privacy_control_missing` and `attempt_count` unchanged.
  - ING unit (`tests.rs`) `main_status_maps_a_failed_privacy_classification_to_quarantined` (Q1): a `PipelineContributorStatus` with `submission_status = "received"`, `processing = Failed`, `reason_label = Some("privacy_classification_failed")`, `admission_decision = "admit"` maps to `quarantined`; with `reason_label = Some("attempts_exhausted")` it still maps to `accepted`.
  - HTTP leg (HTTP:7551-7598, "A privacy boundary that fails"): the POST now answers 200 `processing` and creates a run (the old asserts at :7576 and :7597 invert); a worker pass leaves it in `retry` with `privacy_classification_failed`.
- [ ] **Step 2: Run red.**

```bash
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --lib privacy_pass_timeout_fits_inside_the_review_lease_cap
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --test versioned_pipeline_runtime_pg -- classifier_ missing_boundary --test-threads=1
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --bin trace-commons-ingest -- main_status_maps_a_failed_privacy_classification compatibility_bundle_through_http_with_review_privacy_withdrawal_and_export --test-threads=1
```

  Expected: the retry test fails on the backoff interval and the terminal label (`attempts_exhausted`); the timeout unit test fails to compile; the status test maps to `accepted`; the HTTP test fails at the inverted leg.
- [ ] **Step 3: Implement** routing, `mark_retry`, the timeout (P8) and the `main_status_for_pipeline` arm (`_ if status.processing == PipelineProcessingStatus::Failed && status.reason_label.as_deref() == Some(PIPELINE_PRIVACY_CLASSIFICATION_FAILED_LABEL) => "quarantined"`, before `_ => "accepted"`; update its doc at ING:17400-17405). Grep the ingest tests for any legacy-status assertion that pins `accepted` for a failed pipeline run and update it.
- [ ] **Step 4: Run green**: Step 2's commands; then the whole ingest bin (it holds the HTTP leg and the status tests):

```bash
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --bin trace-commons-ingest -- --test-threads=1
```

- [ ] **Step 5: Commit** `Retry a failed privacy classification on the legacy backoff and fail closed`.

Recorded while implementing Task 8:
- P8 is a pure function, `privacy_pass_timeout_for(review_lease, classifies_prose_pii, ceiling)`, so the VP unit test needs no built service. `build()` calls it with `self.privacy.is_some_and(classifies_prose_pii)` and stores the result on the service; `privacy_pass_timeout()` returns it. The refusal is "less than `PIPELINE_PRIVACY_PASS_COMMIT_MARGIN` left for the classifier", so a 30 s Review lease is the shortest accepted (60 s bound) and 20 s refuses. The test-only knob is `PipelineServiceBuilder::with_privacy_pass_timeout_ceiling` (`#[doc(hidden)]`); it replaces the 900 s ceiling for both kinds of boundary.
- `mark_retry` takes two more parameters (`$8` the label is `privacy_classification_failed`, `$9` the delay in seconds, `30 << min(attempt_count - 1, 9)`); the exhaustion arm writes `attempts_exhausted` only when `$8` is false. Nothing else in `src/` reads `attempts_exhausted`.
- `privacy_control_missing` joins the FR3 deployment-gap array beside the two settlement gaps (uncharged `mark_transient_retry`); no allowlist edit.
- RT gains `privacy_pass_test_builder` (the builder `privacy_pass_test_service` builds) for the timeout test's knob, and `scheduled_retry_seconds(run)` = `next_attempt_at - updated_at`, which `mark_retry` sets from one `NOW()`, so the backoff is asserted exactly. The timeout test reuses `CountingSlowClassifierBoundary` with a 200 ms ceiling.
- The HTTP leg also asserts the stored first backoff is 30 s. Without it the leg could pass before the fix by catching the run in its first 50 ms `retry`.
- Red run: `classifier_failure_retries_then_fails_closed` failed on the first backoff (0 s against 30 s); `missing_boundary_is_an_uncharged_wait` on the label (`minimal_policy_failed`); the VP unit test and RT's timeout test failed to compile (no `privacy_pass_timeout_for`, no `with_privacy_pass_timeout_ceiling`); `main_status_maps_a_failed_privacy_classification_to_quarantined` mapped to `accepted`; the HTTP test failed at the leg's backoff assertion (0 against 30). The whole ingest bin had no test pinning `accepted` for a failed pipeline run.

### Task 9: Withdrawal, purge, and legacy readers (Q3)

**Files:** RT withdrawal and retention-purge tests; HTTP (end-to-end withdrawal through the revocation-propagation worker; legacy-reader pin).

- [ ] **Step 1: Write the tests.** All three are **regression pins (expected green)**: the generic paths should already hold, and these tests fix them so a later change cannot break them silently. If one is red, fix the code it names.
  - HTTP `pipeline_withdrawal_deletes_the_privacy_pass_object`: drive a run through Review (pass recorded) over HTTP, withdraw it through the contributor route, then run the revocation-propagation worker that the withdrawal's `delete_object_payload` items feed (VP:7011-7060 -> `delete_object_payload_for_revocation_propagation`, ING:65634). Assert: the pass ref is invalidated (VP:6826-6832); a `delete_object_payload` item exists for the pass ref; after the worker the object is physically gone from the store; and the item was verified under `TraceArtifactKind::ContributionEnvelope` (the ING:65689-65692 mapping), not `Other` (ING:19412-19421).
  - RT `retention_purge_deletes_the_privacy_pass_object`: the retention purge (VP:6984-7008) queues the pass ref like the source and approved refs.
  - HTTP `legacy_readers_never_emit_a_pipeline_source` (Q3, critique 2): a pipeline submission whose stored source carries a planted prose marker (MarkerRedacting double), driven to `accepted`. Assert that `get_latest_active_envelope_object_ref`-based reads (ING:56028-56049, read through ING:68039-68044) refuse it, and that `run_benchmark_conversion_job` (ING:54057), `run_ranker_training_candidates_export_job` (ING:54654), `run_ranker_training_pairs_export_job` (ING:54996) and `run_process_evaluation_worker` (ING:41896), run for the tenant, either refuse the submission or produce output that does not contain the marker. Name in a comment the follow-up issue that moves them to `read_mains_reviewer_metadata_view` (ING:61874-61883).
- [ ] **Step 2: Run.**

```bash
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --test versioned_pipeline_runtime_pg -- retention_purge_deletes_the_privacy_pass_object --test-threads=1
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --bin trace-commons-ingest -- pipeline_withdrawal_deletes_the_privacy_pass_object legacy_readers_never_emit_a_pipeline_source --test-threads=1
```

  Expected green. Then check each test can fail: temporarily map `ReviewSnapshot` to `Other` at ING:65691 (the withdrawal test must go red) and temporarily skip the P1 unwrap check in the legacy reader (the legacy-reader test must go red); revert both by hand-editing back, not with `git checkout` (it would discard uncommitted work).
- [ ] **Step 3: Commit** `Pin the privacy pass's deletion and the legacy readers' refusal`.

Recorded while implementing Task 9:
- The withdrawal test as first written was wrong about the contributor route. `POST /v1/contributors/me/pipeline-submissions/{id}/withdraw` runs `pipeline_account_trace_withdrawal`, which calls `main`'s whole-submission delete (`delete_withdrawn_trace_objects`, ING:19452) in the request: every live ref's payload is deleted by its object key under `trace_artifact_kind_from_storage`'s receipt kind (`ReviewSnapshot -> Other`; the local store deletes by key) and the refs are marked deleted. The `pipeline_withdrawal` items the pipeline queues are then completed by the worker as already deleted (ING:65683-65689), not verified. The worker's `ContributionEnvelope` mapping and verification (ING:65707-65714, `ReviewSnapshot` at :65710; the plan's ING:65691 before line drift) is the path of `main`'s revocation routes (`follow_up_revocation`, ING:15866) and of the retention purge (`follow_up_retention`, ING:73045), which delete nothing in the request. So `pipeline_withdrawal_deletes_the_privacy_pass_object` has two legs, a fresh run each: the contributor withdrawal route (the object is gone after the request, the item is queued and the worker completes it) and `POST /v1/traces/{id}/revoke` (the object stays until the worker, which verifies it under `ContributionEnvelope` and deletes it). Both legs assert one `delete_object_payload` item for the pass ref, `failed = skipped = 0`, `completed = checked`, the object gone and the ref marked deleted. The Step 2 mutation (`ReviewSnapshot` mapped to `Other` at ING:65710) turns the revocation leg red.
- The receipt goes through the router (`route_trace`, `route_request`), and the phases are driven by `process_run`; the test does not start a listener and a live worker.
- `legacy_readers_never_emit_a_pipeline_source`: `read_envelope_from_active_db_object_ref` refuses (the active `submitted_envelope` ref holds the P1 wrapper), and `read_envelope_from_object_ref` refuses every envelope-kind ref of the submission, the pass and approved `review_snapshot` refs included. Of the four jobs, observed on the run: benchmark conversion and both ranker exports answer 200 with `item_count = 0` (the pipeline submission is filtered out before any body read), and the process-evaluation worker answers 500 (`trace commons operation failed`). The test asserts no answer carries the marker and none is a 400 (each request is well formed). There is no explicit P1 check in the legacy reader to skip: it refuses because the typed decode of the wrapper fails, so the mutation that must turn it red adds an unwrap of the wrapper.
- No follow-up issue number exists yet (Q3: filed separately); the test comment names the follow-up without a number.

### Task 10: Docs and contracts

**Files and edits:**
- `docs/operator/pipeline-activation.md`:
  - 1595-1634 ("Authority and privacy at the receipt"): the receipt runs only the deterministic rescrub before staging (1597-1600); a failing classifier no longer refuses the receipt (1608-1612); "all three refusals" (1614) and the error-hash table (1620-1624) swap `privacy_classification_failed` for `privacy_rescrub_failed`; 1629-1631 becomes the spec's sentence: "the stored source is the content after the deterministic rescrub; the approved content, which is all that Score and exports read, is after the classifier"; replay calls neither (1632).
  - New sub-section "The privacy pass at the start of Review": load, classify, store (per-attempt key, run-derived ref, sweep), record (V117 columns), hand off; escalation rule (P3); `privacy_pass_required` and the approval guard; failure; crash safety and lease loss (P7: the classifier may run more than once, one result is recorded); timeout bound (P8); the server-committed rejection rule id and the approval link (Q2); the `redaction_hash` difference from legacy (P5).
  - 1570-1574: the pass spends Review's attempt budget; exhaustion fails with `privacy_classification_failed`.
  - 1652-1682: second parking cause (`privacy_pass_review_required`); a run that needs the pass is not claimable until the pass is recorded; the queue's `hold_reason` and `assessment_superseded`; what an approval must resolve; no re-run after an assessment; P9's held runs and their exits.
  - 1703-1751: a bullet for the classifier error (charged, 30 s doubling, terminal label).
  - 1888-1900, 2500-2502, 2573-2576: the pass object in the withdrawal/purge payload lists and the attempt-artifact sweep list.
  - 2124-2127: the export guardrail reads the submission's risk, which the pass now writes.
  - 2322-2324: escalation reads `quarantined`; a run waiting for its pass reads `accepted`; a `privacy_classification_failed` run reads `quarantined` (Q1).
  - 196-197 and 876-877: "costs no classifier call" is vacuous; reword to "costs no rescrub".
  - Rollback section: before rolling back below this revision, require `SELECT count(*) FROM pipeline_attempt_artifacts WHERE artifact = 'privacy-pass' AND state = 'staged'` to be zero (run the sweep first): the old binary's `PipelineAttemptArtifact::from_db_str` returns `None` for `privacy-pass` and its sweep fails the whole pass with `pipeline_attempt_artifact_kind_unrecognized` (VP:10054-10057). Runs received after V117 do need containment. The CHECK refuses the old binary's approval of them, but that refusal is a raw check violation the old dispatch charges as `minimal_policy_failed` on the 50 ms doubling backoff, so each run reaching an approval at Review ends `failed`/`attempts_exhausted` (terminal) in about a second, its approved object deleted (Stage 3 and rollout gives the code path). Before rolling back below this revision: (1) with the new binary still running, count the exposed runs per tenant: `SELECT count(*) FROM pipeline_runs WHERE next_phase = 'review' AND privacy_pass_required AND privacy_pass_object_ref_id IS NULL` (the database is at V117, so these columns exist; read the RLS note below); (2) suspend the Review policy (`POST /v1/admin/pipeline/policy-interventions`, `phase: review`, `action: suspend`, "Suspend a policy") of every bundle that any of those runs is bound to, and of the tenant's active bundle, because a receipt the old binary takes still gets `privacy_pass_required = TRUE` from the column default and reaches Review; a run under a suspended Review policy waits in `retry` with `bundle_policy_not_runnable` (`PIPELINE_POLICY_NOT_RUNNABLE_LABEL`, VP:139), uncharged. Or stop every process that runs the pipeline workers for the length of the rollback. `contain` is not a substitute: it refuses new receipts and does not stop dispatch. (3) After the roll-forward, resume the suspended policies. Separately, before deploying, count the runs P9 can hold, per tenant: `SELECT count(*) FROM pipeline_runs r JOIN pipeline_review_assessments a USING (tenant_id, run_id) WHERE r.next_phase = 'review' AND r.state IN ('pending', 'retry', 'awaiting_review')`. This one runs on the V116 database, so it names no V117 column (before V117 no run has a pass, so the old `privacy_pass_object_ref_id IS NULL` predicate was redundant and fails there with "column does not exist"). RLS note for both queries: `pipeline_runs` has forced RLS, so a plain operator session with no tenant context sees no rows. Run each query per tenant in one session after `SELECT set_config('trace_commons.trace_tenant_id', '<tenant>', false)`, or under a role with BYPASSRLS.
- `docs/operator/deployment.md:673-682`: V117 section, the eight `pipeline_runs` UPDATE columns, `privacy_pass_required` and its CHECK. `docs/operator/deployment.md:836-846` (V108 paragraph): Review's privacy-pass object is now staged too, and V117 replaces V108's artifact CHECK and `pipeline_attempt_artifacts_approved_hash`.
- `docs/superpowers/specs/2026-09-11-versioned-pipeline-behavioral-contracts.md`: SUB-005 (:392) clarifying sentence per D0; REV-001 (:535-536) and REV-002 (:547-548): Review's source artifact is the pass output, the pass record keeps the source hash; REV-003: :565 (a decision applies to a quarantined trace or one the privacy pass escalated, and only once its pass is recorded), :566-567 (human action on an escalated Admit run becomes server-generated evidence: the rejection's `human_assessment_hash`, and the approval's link on the pass record), :568 and :570 (the one exception: the server commits the rejection of an escalated Admit run under `privacy_pass_human_review_rejected_v1`; every approval is still the bound policy's), :569 (an approval also resolves `privacy_pass_review_required`); REV-004 (:577-585) or new REV-005: the server's pass runs before the Review policy, a bundle cannot opt out, a classifier failure never falls back, an approval of a run that needs a pass requires one; RUN-004 (:821-833): two crash boundaries; SCN-003 (:1471-1482): a sibling scenario (prose-only PII admitted Low, escalated, held, approved or rejected).
- `docs/superpowers/specs/2026-09-09-versioned-pipeline-design.md`: Review (:397-407), receipt path (:742-752, step 4 stores the post-deterministic envelope), `pipeline_runs` schema (:668-681).
- `docs/superpowers/specs/2026-09-11-versioned-pipeline-contract-test-manifest.json` (hygiene only; the cross-check is disabled, `scripts/operator/pipeline-deployment-inventory.py:275-280`): new test ids under SUB-005 (:57-62), REV (:79-84), RUN (:108-113) and the scenario.

- [ ] **Step 1:** make the edits. **Step 2: verify.**

```bash
python3 scripts/operator/test_pipeline_tooling.py
python3 -c "import json;json.load(open('docs/superpowers/specs/2026-09-11-versioned-pipeline-contract-test-manifest.json'))"
grep -n "privacy_classification_failed\|privacy_review_required\|privacy_pass_review_required" docs/operator/pipeline-activation.md
```

  `test_pipeline_tooling.py` compares the manifest digest to the live file (`scripts/operator/test_pipeline_tooling.py:3508`), so it passes after the edit. No required check is added, so the counts in pipeline-qualification.md (:69-135) do not move.
- [ ] **Step 3: Commit** `Document the Review-start privacy pass`.

### Task 11: Restore drill fingerprints the pass record

The drill keeps its one pending run (RESTORE:2141-2143, 2401-2416, 2553; `pipeline.py`:129, 830, 860; `promote.py`:957; the `test_pipeline_tooling.py` fixtures): none of them changes. After Task 5 both seeded runs pass Review through `PassThroughPipelinePrivacyBoundary` (RESTORE:1407), so both carry a pass record and a pass object, and the drill's generic object copy and artifact fingerprint already cover the object (`versioned_pipeline_remote_restore.rs:113-157`). This task only makes the database fingerprint name the pass record, so a restore that dropped it would be caught.

**Files:** RESTORE `AUTHORITATIVE_FINGERPRINT_SQL` (:247-260); `docs/operator/pipeline-qualification.md:141-152` (one sentence: the fingerprint covers the pass record).

- [ ] **Step 1: Edit.** Add `COALESCE(privacy_pass_object_ref_id::text,'')`, `COALESCE(privacy_pass_content_hash,'')`, `COALESCE(privacy_pass_outcome,'')`, `privacy_pass_required::text` to the `concat_ws` over `pipeline_runs`. This is a **coverage extension, not a red test**: the seed and resumed fingerprints move together, so it cannot fail before the edit.
- [ ] **Step 2: Run the drill** (the seed and resume tests are `#[ignore]`d, RESTORE:2008 and :2281, and run only through `pipeline.py`, which passes `ignored=True`; a plain `cargo test ... restore` executes none of them):

```bash
python3 scripts/operator/pipeline.py restore-drill --postgres-admin-url postgres://$USER@127.0.0.1:5432/postgres
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --bin trace-commons-ingest production_restore_drill_resumes_once -- --ignored --test-threads=1
python3 scripts/operator/test_pipeline_tooling.py
```

  Expected: `PipelineRestoreOK: ... pending_runs_resumed=1 duplicate_effects=0`, and the drill's "executed nothing" guard does not fire. Then confirm the seed's fingerprint text contains a non-empty pass hash for both runs (print it once from the seed test, or query `pipeline_runs` in the scratch database), so the new fields are not all empty strings. This coverage runs in the non-required `pipeline qualification and restore` CI job (ci.yml:845-848).
- [ ] **Step 3: Commit** `Fingerprint the privacy pass record in the restore drill`.

### Task 12: Gate (no commit)

```bash
cargo fmt --all -- --check
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --features local-gpu-models   # non-CUDA CI config; may not link locally
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --features near-ai-scorer
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
cargo clippy -p trace-commons-server --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
RUSTFLAGS="-D warnings" cargo test --workspace
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --lib pipeline_upgrade -- --ignored --test-threads=1   # needs TRACE_COMMONS_PIPELINE_PG_UPGRADE_TEST_URL
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --test versioned_pipeline_runtime_pg     # parallel, as CI runs it
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --bin trace-commons-ingest -- --test-threads=1
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --bin trace-commons-ingest compatibility_bundle_through_http_with_review_privacy_withdrawal_and_export   # CI requires "1 passed"
python3 scripts/operator/test_pipeline_tooling.py
python3 scripts/operator/pipeline.py qualify --postgres-admin-url postgres://$USER@127.0.0.1:5432/postgres
grep -rn "\.rescrub(" crates/ | wc -l    # 0
```

`pipeline.py qualify` runs the corpus harness (CORPUS:1822, ignored, run through the command) and the restore drill. Check `privacy_quarantine_approved` (CORPUS:2869-2880) still observes `admission_decision = quarantine`, `privacy_state = medium`; `observed_privacy_state` (CORPUS:1203-1214) reads Admission's decision and needs no pass case, because no corpus fixture escalates (the pass-through boundary leaves risk unchanged). Compare the RT, ingest-bin and upgrade counts against the baseline taken before Task 1 (fresh databases for each run).

## Review focus

- No path runs the classifier at the receipt (`grep -rn "rescrub_classifier" crates/trace-commons-server/src` shows only VPA, the doubles' definitions and the pass).
- No path hands the Review policy the raw source once a boundary exists; no fallback on classifier error; no approval of a `privacy_pass_required` run without a pass (predicate and CHECK).
- The pass record and logs are label/hash only.
- Escalated runs are reachable by the queue, claim and assessment paths only after the pass is recorded; an approval must name every hold; an assessment that predates the pass never releases an escalated run.
- `redaction_hash` is never written by the pass; `redaction_counts` and `redaction_pipeline_version` are.
- A rollback binary (V116 code on a V117 database) reads every row the new binary writes: `pipeline_run_from_row` reads by column name (`row.get("tenant_id")`, VP:7224), so extra columns are ignored, and the pass ref uses an existing artifact kind (P2). It cannot approve a run received after V117 without a pass (the CHECK), but its refused approval is a charged, non-transient error that exhausts the run's attempts in about a second and fails it terminally with `attempts_exhausted` (Stage 3 and rollout); the runbook requires the Review policy suspended, or the pipeline workers stopped, before such a rollback (Task 10). Its sweep fails on a staged `privacy-pass` row, which the runbook drains first (Task 10).

## Questions for the owner

1. P9: a run received before V117 whose assessment predates its escalating pass is held with no re-assessment path, because assessments are append-only and one per run (V105:47, 240). Is the fail-closed hold (exits: withdrawal or containment) acceptable, or should V117 also let a superseded assessment be replaced (a DELETE grant or a supersession column on `pipeline_review_assessments`)? The pre-deploy count in Task 10 says how many runs this concerns.

## Critique resolution index

| # | Resolution |
|---|---|
| 0 | `privacy_pass_required` + CHECK + `commit_review` predicate with `privacy_pass_missing`; `approval_without_a_pass_is_refused` (Task 5) |
| 1 | Claimable only after the pass; superseded assessment ignored (P9); label `privacy_pass_review_required`; escalation defined by P3 (Task 6). Partly: "park for a fresh one" becomes a fail-closed hold with no re-assessment path (V105:47, :240), owner question 1 |
| 2 | `legacy_readers_never_emit_a_pipeline_source` (Task 9, Q3) |
| 3, 16 | Timeout bounded by the lease cap, refused at build for a classifying boundary (P8, Task 8) |
| 4 | `record_privacy_pass` requires exactly one moved row; same-dispatch test (Tasks 4, 7) |
| 5 | Explicit `encode_pipeline_artifact_bytes` and `is_pipeline_artifact_wrapper` assertion (Task 5) |
| 6, 12 | Approval link columns (Q2); REV-003 :565-570 edits (Tasks 3, 7, 10) |
| 7, 11 | Counts and pipeline version written back; `redaction_hash` deterministic with reasons (P5, Task 4) |
| 8 | P7 restated; lease-loss test (Task 5) |
| 9 | End-to-end withdrawal through the propagation worker, mapping asserted (Task 9) |
| 10, 19 | One pending run kept; fingerprint-only drill change, run through `pipeline.py restore-drill` (Task 11). "Generalise to N pending runs" declined per the owner; pass resume is proved by crash-matrix tests (Task 5) |
| 13 | Distinct label and `hold_reason` (Task 6) |
| 14, 28 | Grep counts on `async fn rescrub_*`, 12 each (Task 1) |
| 15 | deployment.md:836-846 added (Task 10) |
| 17 | Upgrade-test database in Global constraints (Task 3) |
| 18 | Filters after `--` everywhere |
| 20, 21 | Task 1 pure refactor; existing tests in each green step; expected-red list after Task 2 |
| 22 | Crash tests are red tests in Task 5 |
| 23 | No corpus expectation moved; `pipeline.py qualify` in Task 12 |
| 24 | No `#[ignore]` placeholders; the old test is deleted in Task 2 (its double made configurable so it stays constructed), its Review side added in Task 8 |
| 25 | Two services on one database (Task 8) |
| 26 | Fresh tenant and per-test counters; parallel RT run in Task 12 |
| 27 | Regression pins labelled; real red commands for Tasks 3, 6, 7 |
