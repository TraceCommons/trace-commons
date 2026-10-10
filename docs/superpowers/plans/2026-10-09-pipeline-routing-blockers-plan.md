# Routing blockers of #1185 implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** Approved by the owner on 2026-10-09 ("plan approved, all recommendations accepted; start Phase B"). Each question of the section "Questions for the owner" has its recommended answer.

**Goal:** Close the 12 open boxes of TraceCommons/trace-commons#1185 with one PR against `main` (`vp/pipeline-routing-blockers`), so that no item of #1185 blocks the routing of a tenant or the NEAR payout of the pipeline.

**Architecture:** Each box gets the smallest change that closes it. Five boxes are filters or one-statement corrections on `main` routes and on one pipeline read. The audit boxes get one event that the receipt route appends, and one marker column on `pipeline_runs` (V117) with one worker pass that appends the review events, as the `CreditMutate` audit pass does for credit. One box changes how Settle charges a store failure. Two boxes change the pipeline payout. Two boxes are runbook text only.

**Tech Stack:** Rust (axum, tokio, tokio-postgres; no new dependency), PostgreSQL 16 with forced RLS, Markdown runbooks.

**Spec:** There is no separate specification (brief, section 6, step 4). The binding text is the item text of TraceCommons/trace-commons#1185 (`gh issue view 1185 --repo TraceCommons/trace-commons`), finding 5 of zmanian's review of #1283 (review id `5458497356`), the owner decisions in `.superpowers/workstreams/routing-blockers.md` (sections 4 and 5), and the shared rules in `.superpowers/workstreams/common.md`.

**Test bodies:** As in PR 2 to PR 5 (owner decision in the PR 2 ledger), a test that is given as a list of assertions is written by the implementer as real calls. Each listed assertion is necessary.

## State on 2026-10-09

- The branch is at `6c3932b8`, equal to `upstream/main`. `main`'s migrations end at V116. No open upstream PR adds a migration.
- All 12 boxes are open at `6c3932b8`. The evidence is in the ledger (`.superpowers/sdd/2026-10-09-pipeline-routing-blockers-plan/research-a.md` to `research-d.md`). Line numbers in this plan are hints at `6c3932b8`. Find each site by its function name.
- Short names: `ingest` = `crates/trace-commons-server/src/bin/trace-commons-ingest.rs`; `internal/` = `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/`; `runtime` = `internal/pipeline_runtime.rs`; `http_tests` = `internal/pipeline_http_pg_tests.rs`; `act_tests` = `internal/pipeline_activation_pg_tests.rs`; `tests` = `internal/tests.rs`; `vp` = `crates/trace-commons-server/src/versioned_pipeline.rs`; `credit` = `.../versioned_pipeline_credit.rs`; `product` = `.../versioned_pipeline_product.rs`; `rt_tests` = `crates/trace-commons-server/tests/versioned_pipeline_runtime_pg.rs`; `runbook` = `docs/operator/pipeline-activation.md`.

## Update of 2026-10-10: `main` at `5a85898d`

The 11 tasks below are done as written. Then `main` moved by 9 commits, after the pilot deployment of 8 to 10 October. Two of them change the code of this plan, and the branch holds a merge of `main` (no rebase). The task text below is the record of what was built at `6c3932b8`; read it with these changes.

- **The migration is V118.** #1324 took V117 for the privacy pass record. Each `V117` in the text below means `migrations/V118__pipeline_review_audit_marker.sql` now. V118 applies after V117 and adds one more column to `pipeline_runs`.
- **The receipt no longer runs the classifier (#1324).** It runs the deterministic redactor only. The classifier runs in a privacy pass at the start of Review, and that pass rewrites `trace_submissions.privacy_risk`. The `submitted` audit row of Task 7 keeps the risk of the receipt. `main`'s reconciliation therefore leaves the risk comparison out for a pipeline submission whose run has a recorded privacy pass (`pipeline_runs.privacy_pass_recorded_at IS NOT NULL`), as it leaves out the other comparisons that a pipeline row would fail. A run with no recorded pass (pending, or quarantined at Admission) had its risk rewritten by nothing and keeps the comparison.
- **Review has more paths (#1324).** A run that the pass escalates is held `awaiting_review` with `privacy_pass_review_required`; the server commits a reviewer's rejection of such a run itself; a classifier that keeps failing ends the run with `privacy_classification_failed`. The rejection goes through `commit_review`, so the marker of Task 8 is set and the worker appends its event. The hold and the failure commit no Review decision. Task 13 (owner decision of 2026-10-10) sets the marker for each, and the worker appends one `lifecycle_status_change` event with the status `quarantined`.
- **A review claim needs the recorded pass (#1324).** The review audit tests of Task 8 start from a parked run.
- **`main` found L1-2 again (#1324, plan question Q3).** Its test `legacy_readers_never_emit_a_pipeline_source` pins that `main`'s four jobs select a pipeline submission and are refused at the envelope read. It names the move onto `read_mains_reviewer_metadata_view` as a follow-up. Task 2 is that move. The test still passes, because its fixture has no pipeline store; only its comment changed.
- **Settle decides duplicates (#1325).** A duplicate's NoveltyUtility leg is withheld. Tasks 6, 10 and 11 need no change: a withheld leg has no payout line.

Known limits that this merge adds (they are also in the list "Known limits"):

- A run that the privacy pass holds, or that fails with `privacy_classification_failed`, keeps the stored status `received`, so `main`'s `review_sla` and `urgent_reviews` do not count it. The pipeline review queue lists it.

## Questions for the owner (answered on 2026-10-09: each recommendation is accepted)

Each question has a recommendation. The plan is written for the recommended answers.

**Q1. One PR, or two?** The two payout boxes (L2-3, L5-5) cannot occur on the production assembly, and no adapter in this repository can answer "failed on chain". They are the last two code tasks (Tasks 10 and 11), so a split costs no rework. **Recommendation: one PR with all 12 boxes**, as the owner prefers. If the review of the payout tasks delays the PR, or if the owner does not want the edit of Q4 (b) in the payout dispatch function now, move Tasks 10 and 11 to a second PR.

**Q2. C5 residual: how Settle charges a store failure of its command read (Task 6).** Today each failure is charged with the 50 ms backoff, so a store fault of two seconds fails each run in Settle that has a stored command and forfeits its credit legs. Proposed: the charge stays, but a failed store call of that read waits one hour before the next attempt, as an integrity failure does in Review and Score. The operator then has about four hours to correct the store. The cost: when one stored command cannot be read and the store is in order, the compatibility Scores of that tenant wait up to about five hours (today: about one second): four hours until the run fails, then up to one hour until each waiting Score's next attempt. No credit is lost by that wait. Ruling RB-35 protected against a wait with no end; this keeps an end. **Recommendation: accept.** The alternative is no code change: the runbook sentence that exists stays, the owner ticks the box as accepted, and Task 6 goes.

**Q3. Audit events (L1-7 and finding 5; Tasks 7 and 8).** One fact applies to (b) and (c). A routed tenant requires the database mirror, and then the audit row is written before the file line. One failed file append (or a process stop between the two) leaves the database one event ahead, and each later audit append of the tenant is refused until an operator runs the audit-chain repair. `main` has the same state.
- (a) **A migration.** The automatic-Review event and the assessment event need a bounded list of "committed, not yet appended". The tables that hold the two decisions reject each `UPDATE`, so the marker is one nullable column on `pipeline_runs` (V117, with one partial index and one column grant). **Recommendation: accept V117.** The form with no migration gives the Review event `main`'s guarantee only (a failed append is logged and lost) and makes the assessment route answer 500 and accept an identical retry; finding 5 asks for a repair pass.
- (b) **The claim route does not change.** No pass can rebuild a missed claim event: the claim row is overwritten or deleted later. The other choice is to answer 500 when the append fails, as `main`'s lease claim does. In the state above, each claim of the tenant then answers 500 and locks its run against other reviewers for 30 minutes, so the human review of the tenant stops until the repair. **Recommendation: keep the present answer (poldsam P-7), and accept that a claim whose append fails has no `review_lease` event (it is logged).** The decision's own event is the one that the pass makes certain. With this answer, the claim half of the finding-5 box closes as an accepted gap.
- (c) **The `submitted` event gets `main`'s guarantee.** The receipt route appends it after the receipt commits and answers 500 when the append fails, as `main`'s upload does. In the state above, each new upload of the tenant answers 500 with its run created; the retry answers 409 until the admission lease ends and is then a replay; a bounded account's retry is charged a second time; and none of these receipts gets a `submitted` event (`main` leaves an `idempotent_submit` row for a retry; the pipeline replay appends nothing). **Recommendation: accept (no added code, equal to `main`'s upload).** The other choice costs about 35 lines: the receipt answers success and logs, the marker is set in the receipt transaction, and the pass appends a missed event from the stored rows (without the token role and the auth method).
- (d) **`main`'s account trust recorder reads the new Review rows.** The recorder (V85, run by an operator route) reads review audit rows of an accepted submission. The worker's row for an automatic Review looks like an approval row to it, as the row of `main`'s own automatic acceptance does, so a fact that reads `accepted_approver_unknown` today reads `accepted`. This needs a bundle that writes an `accepted` ledger event; the production assembly's compatibility bundle writes none. **Recommendation: accept.**

**Q4. Payout (money; payout only; Tasks 10 and 11).**
- (a) When the adapter reports that a submitted transaction failed on chain, the pipeline marks the line and the leg `failed` and does not submit again. `main` submits a `failed` line again automatically; a second submit with the same idempotency key gives the same failed transaction by the adapter contract. Such a line then has no exit in this release (no route retries a `failed` payout, as for `near_submit_failed` today); the operator pays by hand and keeps a record. **Recommendation: accept.**
- (b) The payout audit rows have `main`'s two kinds. A row is written only for a pass that submitted, confirmed or failed one line or more, with those counts. The counts come from a counter that the worker gives to the dispatch function (about 20 lines in money code). `main` writes a row for each pass, also an empty one; for the pipeline that gives one row each minute, with no end, for each leg that waits for a NEAR account. **Recommendation: accept.**

## Global Constraints

- Work only in this worktree. Do not use `git stash`. Do not rebase.
- Do not set `RUSTFLAGS` or `CARGO_TARGET_DIR`. Do not pass `-j`. Do not use `cargo +<toolchain>`. Serialize cargo commands. Run long commands in the background.
- Three check levels (`common.md`):
  - In each task and each fix round: `cargo fmt --all -- --check`, `cargo check -p trace-commons-server --all-targets`, and only the focused tests that the task names.
  - After Tasks 5, 8 and 11, and before the review of the whole branch: clippy with the CI allow-list (`CLAUDE.md`) and `cargo test -p trace-commons-server --test license_boundary`.
  - At the gate only (Task 12): `cargo test -p trace-commons-server --no-run`. The upstream CI runs the suites.
- PostgreSQL: only the container `tc-pipeline-pg-rb` (127.0.0.1:55435, user `trace`). Make a fresh database for each run of a test command:
  `dropdb -h 127.0.0.1 -p 55435 -U trace --if-exists admission_test_rb && createdb -h 127.0.0.1 -p 55435 -U trace admission_test_rb`.
  - Tests of the ingest binary (`http_tests`, `act_tests`, `tests`): `TRACE_COMMONS_PG_TEST_DATABASE_URL=postgres://trace@127.0.0.1:55435/admission_test_rb TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL=postgres://tc_login_resolver_login@127.0.0.1:55435/admission_test_rb cargo test -p trace-commons-server --bin trace-commons-ingest <test name> -- --test-threads=1`.
  - `rt_tests`: the same first variable, `cargo test -p trace-commons-server --test versioned_pipeline_runtime_pg <test name>`.
  - Upgrade tests: make `pipeline_test_rb` new and empty with the same `dropdb` and `createdb` pair, then `TRACE_COMMONS_PIPELINE_PG_UPGRADE_TEST_URL=postgres://trace@127.0.0.1:55435/pipeline_test_rb cargo test -p trace-commons-server --lib pipeline_upgrade_tests -- --ignored --test-threads=1` (the database name must start with `pipeline_test_`).
- A new test of an audit append sets `state.require_db_mirror_writes = true` (in the `Arc::make_mut(&mut fixture.state)` block), because a routed tenant in production has that order: the row first, then the file line. Models: `real_http_pipeline_receipt_replays_on_retry` and `a_credit_audit_append_with_a_required_mirror_does_not_read_the_file_log` (`http_tests`).
- Each new test shows one failing run before the change (each test step gives the expected failure). After the change, the new tests and the present tests that the task names must pass. A mutation check is necessary only where a task says so.
- Hash-only and label-only output. Each new label matches `^[a-z0-9_]{1,64}$`. Fail closed. No new dependency. Do not edit the expected sets in `tests/license_boundary.rs`.
- Commit subjects are short and imperative, with no prefix. No Claude attribution in a commit or in the PR text. No emoji.
- Runbook text: short sentences, active voice, one fact in each sentence. Do not rename a section title (other documents link to the titles).
- No push, no PR, no GitHub comment and no issue edit without the owner's approval.

## Decisions that no task explains

- **RB-D1 (L1-1).** `read_trace_operational_summary` does not change to `read_mains_reviewer_metadata_view`: that also changes `submissions.*`, `review_sla.*`, the gate `urgent_reviews` and `delayed_credit.*`, which #1185 does not ask for. Only the derived list is filtered.
- **RB-D2 (L1-2).** The 409 is for the direct job route only. A 409 inside the process-evaluation worker is one more failed worker run, so the worker takes the filter and does not reach the refusal.
- **RB-D6 (C5 residual).** The classification of Review and Score is not used for Settle's command read: an uncharged retry of one unreadable command would hold the tenant's compatibility Scores with no end, which RB-35 forbids. A new label is used, and not the hourly wait for `index_command_invalid` as a whole: a stored command with wrong content cannot be corrected by a wait, and it would hold the tenant's Scores for four hours for nothing.
- **RB-D8 (V117).** The marker is on `pipeline_runs`. `phase_outcomes` and `pipeline_review_assessments` reject each `UPDATE` by trigger, and control health checks those triggers. A list with no marker (all Review outcomes that have no audit row) reads each run of the tenant on each pass.
- **RB-D10 (finding 5, assessment).** The assessment route keeps its append and its answer, and gives the event the id `assessment_id`; the pass is the repair only. The worker is not the only appender, because a rebuilt event names the reviewer by the stored `reviewer_sha256:` reference, and `main`'s account trust recorder (V85) finds a self-review by a comparison of the event's actor with the account's principals.
- **RB-D12 (L2-3).** The trait method changes its return type; it gets no provided default. A default would let an adapter keep the defect with no compile error.
- **RB-D14 (L5-3).** The drain-list rule has one exception that the code enforces: the index rebuild refuses a listed tenant, and `backup-restore.md` steps 3 to 5 unset both lists for it.

## Known limits

- A receipt whose `submitted` append fails, or whose process stops between the commit and the append, has no `submitted` event and no other Submit audit row (Q3 c).
- A claim whose audit append fails has no `review_lease` event. It is logged under `pipeline_review_audit_append_failed` and is not repaired (Q3 b).
- A Review decision from before V117, or from a replica that still runs the older build during the deploy, gets no event. An assessment that the older build recorded, and whose Review a newer build commits, gets a second `review_decision` event. A binary rollback has the same window.
- An assessment event that the pass appended names the reviewer by the hashed reference and has no token role, so `main`'s trust recorder cannot show a self-review for it. The time of each event of the pass is its append time; the time of the decision is in `phase_outcomes` and `pipeline_review_assessments`.
- A review audit item that the database refuses stays pending and is tried again on each pass; 32 or more such items of one tenant hold its later items (as for `CreditMutate`, item C9e, accepted in #1185). With two replicas, the clear of a marker can wait up to 30 seconds for a run row that the other replica's Settle dispatch holds.
- The pass decides "the event exists" from the database row, so it runs only in a process that requires the database mirror. In another process (such a configuration cannot be activated) the pass appends nothing, keeps the markers, and logs `pipeline_worker_review_audit_failed` each interval.
- A pipeline payout audit row whose append fails is lost (as on `main`).
- L4-7: a follow-up that is lost to a process stop, for a withdrawal with no source session and a run with no index work and no export item, is not found by the recovery pass. The reconciler covers a version that has a source session.
- A pipeline submission that Admission quarantined has the stored status `quarantined`, so `main`'s operational summary counts it in `review_sla` and, when it is old, in the gate `urgent_reviews`. The count is true, and the pipeline review routes clear it. #1185 does not name it, and this plan does not change it. Since #1324 a run that the privacy pass holds keeps the stored status `received` and is in no count of `main` (update of 2026-10-10).

---

### Task 1: Leave pipeline submissions out of the vector summary's derived list (L1-1)

**Files:**
- Modify: `ingest`, `read_trace_operational_summary` (about line 46870)
- Modify: `runbook`, the paragraph that says the rollback drill and the replay export leave pipeline rows out (about lines 2512 to 2521)
- Test: `http_tests`

- [ ] **Step 1: Write the failing test** `mains_operational_summary_leaves_pipeline_submissions_out_of_the_vector_gate` in `http_tests`. Copy the setup of `mains_operational_summary_leaves_a_failed_pipeline_payout_line_out` (same file). Use `withdrawal_fixture_with(.., true)` (database reviewer reads on). Set `state.pipeline_store = Some(Arc::new(PgPipelineStore::new(..)))` as `mains_legacy_review_routes_leave_pipeline_submissions_out` does (the test state has no store, and the filter is then a no-op). Set `state.require_derived_export_object_refs = true`. Make one accepted pipeline submission (`completed_pipeline_run`) and one accepted legacy submission of the same tenant that has a current derived record and no vector entry. Call `GET /v1/admin/operational-summary`. Assertions:
  1. `promotion_gates.blocking_gates` contains `missing_active_vectors=1` (the legacy record);
  2. `vectors.accepted_current_derived` is 1;
  3. `submissions.total` is 2 (the record list is not filtered).
  Expected failure before the change: assertion 1 (`missing_active_vectors=2`).
- [ ] **Step 2: Implement.** In `read_trace_operational_summary`, after the view read: when `state.db_reviewer_reads_for_tenant(&tenant_id)` and `state.pipeline_store` is `Some`, read `store.pipeline_submission_ids(&tenant_id)` (map an error as the function's other reads do) and `derived.retain(..)` the records whose `submission_id` is not in the set. Do not change `records`. About 10 lines.
- [ ] **Step 3: Run** the new test and `operational_summary_blocks_vector_nearest_neighbor_policy_gaps` (`tests`). Expected: PASS.
- [ ] **Step 4: Runbook.** Add one sentence to the paragraph named above: `main`'s operational summary leaves the derived records of pipeline submissions out of its vector counts and its `missing_active_vectors` gate; its submission counts and review counts include pipeline submissions.
- [ ] **Step 5: Commit.** `Leave pipeline submissions out of main's vector gate`

### Task 2: One pipeline filter for the benchmark export, the ranker exports and process evaluation (L1-2)

**Files:**
- Modify: `ingest`: `run_benchmark_conversion_job` (about 54046), `collect_ranker_training_candidates` (about 59263), `run_process_evaluation_worker` (about 41911), `run_process_evaluation_job` (about 41687)
- Modify: `runbook`, the same paragraph as Task 1
- Test: `http_tests`

**Consumer sweep (review rule 1).** `read_reviewer_metadata_view` has 12 unfiltered call sites at `6c3932b8` (one row corrected after the merge of `main`):

| Function | Takes the filter in this plan? |
| --- | --- |
| `run_benchmark_conversion_job`, `collect_ranker_training_candidates`, `run_process_evaluation_worker` | Yes (this task). Each one decodes the stored envelope and fails on the pipeline wrapper. |
| `read_trace_operational_summary` | The derived list only (Task 1). |
| `run_credit_settlement_unlocked`, `build_credit_risk_summary` | No, and they must not: the list is a lookup for ledger events, and a filter would make credit from `main`'s manual routes on a pipeline submission impossible to settle (money rule, C24). |
| `analytics_handler`, `list_traces_handler`, `run_analytics_release_drill` | No. They count or list metadata. They read no body. |
| `ranking_feature_run_handler` | Yes (review of #1331, finding m2). Planned as No, because the pipeline's `summary` record had no `canonical_summary_hash`; since #1325 the pipeline's Review commit stores one, so a pipeline submission became a candidate and got server ranking features the ranker exports then leave out. It reads `read_mains_reviewer_metadata_view`. |
| `run_canary_read_drill`, `trace_vector_nearest_neighbor_policy_gap_count_for_tenant` | No. Neither reads a pipeline body or counts a pipeline row. |

- [ ] **Step 1: Write three failing tests** in `http_tests`. Each uses one accepted pipeline submission and one accepted legacy submission of one tenant, database reviewer reads on, and `state.pipeline_store` set. Copy `mains_database_replay_export_leaves_pipeline_submissions_out` (`product_fixture()` has an export worker token).
  - `mains_benchmark_export_leaves_pipeline_submissions_out`: the benchmark conversion route answers success, and its result names only the legacy submission. Expected failure: a 500.
  - `mains_ranker_export_leaves_pipeline_submissions_out`: the same for `GET /v1/ranker/training-candidates` (the pairs export calls the same function). Expected failure: a 500.
  - `mains_process_evaluation_leaves_pipeline_submissions_out`, assertions:
    1. the worker run route in dry-run mode (copy `process_evaluation_scheduler_tick_dry_run_skips_evaluator_and_trace_writes` in `tests` for the request) answers success, and `evaluated_submission_ids` holds the legacy id only;
    2. the job route `POST /v1/workers/process-evaluation` for the pipeline submission answers `409` with `error == "pipeline_run_owns_submission"`;
    3. the job route for the legacy submission is not refused with that label.
    Expected failure: assertion 1 (two ids).
- [ ] **Step 2: Implement.** Replace `read_reviewer_metadata_view` with `read_mains_reviewer_metadata_view` at the three sites. In `run_process_evaluation_job`, call `refuse_a_pipeline_submission(state, &tenant.tenant_id, body.submission_id).await?` immediately before `read_utility_submission_record` (after the input checks, so that a wrong request keeps its 400).
- [ ] **Step 3: Run** the three tests and `process_evaluation_worker_attaches_labels_to_accepted_trace` (`tests`). Expected: PASS.
- [ ] **Step 4: Runbook.** Add to the paragraph of Task 1: `main`'s benchmark export, its two ranker exports and its process-evaluation worker leave pipeline submissions out; the process-evaluation job route refuses one with `409` `pipeline_run_owns_submission`.
- [ ] **Step 5: Commit.** `Leave pipeline submissions out of main's exports and process evaluation`

### Task 3: Take the account trace list's cursor from the raw page (L1-3)

**Files:**
- Modify: `ingest`, `account_traces_list_handler` (about 18843 to 18864) and the doc comment of `AccountTracesPage` (about 18707)
- Test: `tests`

**Sibling sites (review rule 2).** `account_credit_totals` (same file) is correct and is the model. No other handler in `ingest` or `internal/` computes a page cursor from a filtered list.

- [ ] **Step 1: Write the failing test** `account_traces_list_pages_past_a_received_row` in `tests`. Copy `account_traces_list_cursor_pages_are_disjoint_and_ordered`. Insert rows for one account with `insert_account_test_submission_with_status`, newest first: accepted, `Received`, accepted, accepted. Read with `limit: Some(2)` until no cursor is returned. Assertions: the first page has one item and a `next_cursor`; the pages together hold the three accepted rows, each one time. Expected failure: the first page has no cursor.
- [ ] **Step 2: Implement.** Before the `filter_map`, keep the raw page length and the `(received_at, submission_id)` of the last raw row. Compute `next_cursor` from them: `Some` when the raw length equals `limit`. Correct the two comments: a cursor is present when the raw page was full, so a page can hold fewer items than `limit`, or none, and have a cursor.
- [ ] **Step 3: Run** the new test and `account_traces_list_cursor_pages_are_disjoint_and_ordered`. Expected: PASS.
- [ ] **Step 4: Commit.** `Take the account trace list's cursor from the raw page`

### Task 4: Count only the pipeline's lines in `near_outbox_by_state` (1186-d)

**Files:**
- Modify: `product`, `PipelineProductStore::operational_summary` (the query at about 1208)
- Modify: `runbook`, three places (below)
- Test: `act_tests`

- [ ] **Step 1: Write the failing assertion.** In `a_submission_with_a_pipeline_run_is_never_counted_whatever_its_rows_say` (`act_tests`), after the seed of five `main` outbox rows and one pipeline row (`pending`, `trace_credit`), add: `PipelineProductStore::new(..).operational_summary(tenant).near_outbox_by_state` equals `{"pending": 1}`. Expected failure: `pending` is 2, and four more keys are present.
- [ ] **Step 2: Implement.** Add `AND instrument_id IS NOT NULL` to the query. (The legacy drain report has `instrument_id IS NULL`, so the two reports divide the rows with no overlap.)
- [ ] **Step 3: Run** the test. Expected: PASS.
- [ ] **Step 4: Runbook.** (a) At the sentence that names `near_outbox_by_state` as the place to see a `failed` line (about 2089 to 2096), add: the field counts only the pipeline's outbox lines (the rows with an `instrument_id`); `main`'s lines are in `main`'s operational summary. (b) Replace "The operational summary's NEAR outbox counts still include them." (about 2078) with: only the pipeline operational summary's `near_outbox_by_state` counts them; `main`'s operational summary does not. (c) At "the NEAR outbox by state" (about 1081), write "the pipeline's NEAR outbox lines by state".
- [ ] **Step 5: Commit.** `Count only the pipeline's lines in the pipeline summary's NEAR outbox`

### Task 5: Always run the pipeline follow-up of an account withdrawal (L4-7)

**Files:**
- Modify: `ingest`, `account_trace_withdraw_handler` (the block `if state.pipeline_service.is_none()`, about 19885)
- Modify: `runbook`, the account withdrawal paragraph (about 1863 to 1871)
- Test: `http_tests`

**Sibling sites.** The revocation route (about `ingest` 15840) and the source-session reconciler (about 22914) have the correct shape already. Do not change them. The unlocked read `withdrawal_reaches_a_pipeline_run` stays: it selects the one-transaction pipeline path, and a wrong answer now costs nothing.

- [ ] **Step 1: Write the failing test** `an_account_withdrawal_follow_up_tombstones_a_run_with_no_index_work` in `http_tests`. With a runtime injected (`withdrawal_fixture`), make a pipeline run that is still before Review (no index work, no export item) for a submission of the account. Call the new function `follow_up_account_withdrawal(&state, &ctx, &[submission_id])` directly (the route reaches this line with a runtime only in a race, so the test calls the function). Assertions: a `trace_tombstones` row exists for the submission; a second call succeeds and adds no row. Expected failure: the function does not exist.
- [ ] **Step 2: Implement.** Add `async fn follow_up_account_withdrawal(state: &AppState, ctx: &AccountCtx, affected_ids: &[Uuid]) -> anyhow::Result<()>`: with the actor `account_audit_tenant(ctx)`, for each id call `pipeline.follow_up_withdrawal(..)` when `state.pipeline_service` is `Some`, else `store.follow_up_withdrawal(..)` when `state.pipeline_store` is `Some`, else nothing. In the handler, replace the `is_none()` block with one call and map its error with `withdrawal_failed`, as the block does now. Keep its position: after the tombstones, before `finish_account_trace_withdrawal`.
- [ ] **Step 3: Run** the new test, `a_runtime_less_build_queues_the_pipeline_follow_up_through_the_database`, `legacy_withdrawal_route_keeps_mains_path_without_a_pipeline_run` and `legacy_withdrawal_route_uses_the_pipeline_for_a_session_with_a_run`. Expected: PASS.
- [ ] **Step 4: Mutation check (a deletion guard).** Remove the call in the handler: `a_runtime_less_build_queues_the_pipeline_follow_up_through_the_database` must fail. Restore the call.
- [ ] **Step 5: Runbook.** In the account withdrawal paragraph, state: `main`'s path runs the pipeline follow-up for each withdrawn submission, with or without a runtime.
- [ ] **Step 6: Commit.** `Always run the pipeline follow-up of an account withdrawal`
- [ ] **Step 7: Package check** (Tasks 1 to 5): clippy with the CI allow-list; `cargo test -p trace-commons-server --test license_boundary`.

### Task 6: Give a store failure of Settle's command read one hour between charged attempts (C5 residual)

Do this task only with the owner's answer to Q2.

**Files:**
- Modify: `vp`: `read_index_command` (about 10355; the `map_err` of the store call at about 10386), `mark_retry` (`hourly`, about 5057), the label list in `process_claimed_run` (about 10930)
- Modify: `runtime`, `pipeline_index_rebuild_error` (about 707)
- Modify: `runbook`, "Settle failures and settlement legs" (about 1733 to 1750) and the Score stall sentence (about 2244 to 2248); `docs/operator/backup-restore.md`, step 4 (the list of the index rebuild's answers)
- Test: `rt_tests`; `tests` (`pipeline_index_rebuild_errors_map_to_fixed_labels`)

**Design.** `pub const PIPELINE_INDEX_COMMAND_UNREADABLE_LABEL: &str = "index_command_unreadable";`. Only a failed store call of `read_index_command` that is not an integrity failure raises it (`is_trace_artifact_integrity_error`, the split of `artifact_store_call`); an integrity failure the store reports (the object missing or corrupt, or bound to another tenant) is wrong content and keeps `index_command_invalid` (review of #1331, finding m1). `mark_retry` sets `hourly` for it also. The label joins the list of labels that `process_claimed_run` passes through (without that, the run is charged under the generic label). The decode, parse, hash, revision and evidence checks keep `index_command_invalid` and the short backoff. Score's read of unapplied commands maps each error to `index_unavailable` and does not change. `pipeline_index_rebuild_error` maps the new label to `503` with the label (the rebuild reaches the same read).

- [ ] **Step 1: Write the failing test** `an_unreadable_settle_command_waits_an_hour_and_ends_the_run` in `rt_tests`. Copy `a_missing_source_object_is_charged_and_ends_the_run` with `run_to_settle_ready`, `force_due` and `assert_integrity_retry_waits_an_hour`. Turn the store's reads off (`OutageArtifactStore`), then process the run. Assertions: after the first attempt the run is `retry` with `last_error_label == "index_command_unreadable"`, the attempt is charged, and the next attempt is one hour later; after five attempts (with `force_due`) the run is `failed` / `attempts_exhausted` and its open legs are `forfeited`. Expected failure: the label is `index_command_invalid` and the wait is 50 ms.
- [ ] **Step 2: Implement** the design.
- [ ] **Step 3: Correct** `stored_command_binding_failures_fail_closed` (`rt_tests`): every case keeps `index_command_invalid`: a deleted object, an overwritten object and another tenant's reference are integrity failures the store reports, and the case that reads another run's command fails a later check. (As first implemented, the three store cases expected `index_command_unreadable`; review of #1331, finding m1, corrected that.) Add the new label and its `503` to the table of `pipeline_index_rebuild_errors_map_to_fixed_labels` (`tests`). Run both, the new test, and `an_artifact_store_outage_is_an_uncharged_suspension_in_every_phase` (if it asserts Settle's old label, correct that assertion). Expected: PASS.
- [ ] **Step 4: Runbook.** Replace the sentences that say the four-hour time holds for Review and Score only and that a run in Settle can fail in about one second. State: a failed store call of Settle's command read is charged under `index_command_unreadable` with one hour between attempts, so the run fails about four hours after the first failure; a stored command with wrong content keeps `index_command_invalid` and the short backoff. At the Score stall sentence, add: while such a command cannot be read, the tenant's compatibility Scores wait (uncharged), for up to about five hours (four until the run fails, then up to one until each Score's next attempt); after the operator corrects the store, each Score continues at its next attempt. `backup-restore.md` step 4: the index rebuild answers `503` `index_command_unreadable` when the store call for a stored command fails for a reason other than integrity, and `409` `index_command_invalid` for a missing or corrupt command; a second run helps only in the first case.
- [ ] **Step 5: Commit.** `Space a store failure of Settle's command read one hour apart`

### Task 7: Append `main`'s `submitted` audit event for a pipeline receipt (L1-7, first half)

**Files:**
- Modify: `ingest`: `route_pipeline_receipt` (the `Created` arm, about 14890), `normalize_audit_event_metadata` (about 72113), `audit_backfill_storage_projection` (the metadata arm for `"submitted"`, about 74698)
- Modify: `runbook`, "Authority and privacy at the receipt"
- Test: `http_tests`

**Design.** In the `Created(run)` arm, build the event by hand (no submission record is in scope): kind `submitted`; `actor_role`, `actor_principal_ref` and the `auth_method=` reason from `tenant`, as `TraceCommonsAuditEvent::submitted` sets them; `status` from `run.admission_decision` (`quarantine` gives `Quarantined`, `reject` gives `Rejected`, `admit` gives `None`, because an admitted receipt has the stored status `received`, which `main`'s audit status type does not have). Row: `action: Submit`, `metadata: Submission { status, privacy_risk }` with the stored status (`Received` for `admit`) and the same risk text that the receipt stored in `trace_submissions.privacy_risk`, `object_ref_id: Some(run.source_object_ref_id)`. Append with `append_audit_event_mirrored`; on an error answer 500 (`internal_error`), as `main`'s upload does. `normalize_audit_event_metadata` accepts a `submitted` event with no status only when the metadata status is `Received`. `audit_backfill_storage_projection` gives a `submitted` event with no status the metadata `Submission { status: Received, privacy_risk: "unknown" }`: after a database restore the backfill is the only repair of the audit rows, and one event that it cannot write blocks each later event of the tenant. The two replay answers (`pipeline_owned_submission_receipt`, and the `Replayed` arm of `route_pipeline_receipt`) append nothing, as before.

- [ ] **Step 1: Write the failing test** `a_pipeline_receipt_appends_mains_submitted_audit_event` in `http_tests`. Upload one admitted trace of a routed tenant through the route. Assertions:
  1. the tenant's audit log holds one event of kind `submitted` for the submission, with the uploader's principal reference and no status;
  2. its database row has action `submit`, metadata status `received`, and a `privacy_risk` equal to `trace_submissions.privacy_risk` of the submission;
  3. `main`'s audit verification for the tenant (use the route or helper that a present audit test uses for `verify_db_audit_projection`) reports no mismatch;
  4. a second upload of the same body (a replay) answers success and adds no `submitted` event;
  5. the appended event, projected with `audit_backfill_storage_projection` and passed through `normalize_audit_event_metadata`, is accepted (no database is necessary for this assertion);
  6. with `fail_next_audit_file_append` set before a new upload, the route answers 500 and the pipeline run exists.
  A second test with a receipt that Admission quarantines: the event has the status `quarantined`. Expected failure: no `submitted` event.
- [ ] **Step 2: Implement** the design.
- [ ] **Step 3: Run** the two tests. Expected: PASS.
- [ ] **Step 4: Runbook.** Add: a new pipeline receipt appends `main`'s `submitted` audit event after the receipt commits; an admitted receipt's event has no status (its row says `received`). When the append fails, the upload answers 500 and the run exists and is processed. Such a failure usually leaves the tenant's audit chain one event ahead in the database: until the audit-chain repair, each new upload of the tenant answers 500 with its run created. A retry answers 409 until the admission lease ends and is then a replay, which appends nothing; a bounded account's retry is charged again.
- [ ] **Step 5: Commit.** `Append the submitted audit event for a pipeline receipt`

### Task 8: Review audit events: the marker (V117) and the worker pass (L1-7 second half, finding 5)

Do this task only with the owner's answers to Q3 (a), (b) and (d).

**Files:**
- Create: `migrations/V117__pipeline_review_audit_marker.sql`
- Modify: `crates/trace-commons-server/src/db/postgres.rs` (register V117 after V116, as V116 is registered); `.../db/postgres/pipeline_upgrade_tests.rs` (the expected column grants of the runtime role on `pipeline_runs` and the doc comment above them, the expected columns, an assertion `last >= 117`)
- Modify: `vp`: `commit_review` (its `UPDATE pipeline_runs`, about 2505), `record_review_assessment` (about 2754), two new store functions
- Modify: `runtime`: the pass, and its call in `drain_pipeline_tenant` inside the `due.credit_audits` block (about 1171)
- Modify: `ingest`: `pipeline_review_assessment_handler` (about 43610): one line
- Modify: `docs/operator/deployment.md`, `runbook` (Step 4)
- Test: `http_tests`, `pipeline_upgrade_tests.rs`

**Migration (complete).** It has V116's idempotent form.

```sql
-- V117: the marker of a review audit event that the worker must still append.
-- A transaction that commits a Review decision or a human review assessment
-- sets it; the worker's review audit pass clears it. NULL for every run that
-- exists now: no event is appended for a decision made before this migration.
ALTER TABLE pipeline_runs
    ADD COLUMN IF NOT EXISTS review_audit_pending_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_pipeline_runs_review_audit_work
    ON pipeline_runs (tenant_id, review_audit_pending_at)
    WHERE review_audit_pending_at IS NOT NULL;

GRANT UPDATE (review_audit_pending_at) ON pipeline_runs TO trace_ingest_runtime;
```

Before you write the file, read V116 and use its header form. Search `crates/` and `docs/operator/` for `V116` and for `credit_audited_at`: each list that pins a migration, a column, or a column grant gets its V117 entry.

**Store (`vp`).**
- `commit_review` adds `review_audit_pending_at = clock_timestamp()` to its `UPDATE`. `record_review_assessment` adds, after its insert, `UPDATE pipeline_runs SET review_audit_pending_at = clock_timestamp() WHERE tenant_id = $1 AND run_id = $2`.
- `list_pending_review_audits(&self, tenant_id: &str, limit: i64) -> Result<Vec<PipelineReviewAuditItem>, DatabaseError>`: the runs of the tenant with a marker, oldest marker first, `LIMIT $2`, with a `LEFT JOIN` to `phase_outcomes` on `phase = 'review'` and a `LEFT JOIN` to `pipeline_review_assessments`. The item holds `run_id`, `submission_id`, `pending_at`, the Review outcome if one exists (`outcome_id`, approved or rejected from the `kind` of `decision`), and the assessment if one exists (`assessment_id`, approved or rejected from `recommendation`, `reason_code`, `reviewer_principal_ref`).
- `clear_review_audit_pending(&self, tenant_id: &str, item: &PipelineReviewAuditItem) -> Result<(), DatabaseError>`: `UPDATE pipeline_runs SET review_audit_pending_at = NULL WHERE tenant_id = $1 AND run_id = $2 AND review_audit_pending_at = $3` (a newer marker stays).

**Worker (`runtime`).** `append_pipeline_review_audit_events(state, service, tenant_id, limit) -> anyhow::Result<PipelineCreditAuditPass>` lists up to `limit` items. For each item, and for each of its events (the assessment event first), it reads the event by its id (`get_trace_audit_event_by_id`) and appends it with that id when it is absent (`append_audit_event_mirrored`). It clears the marker only when each event of the item exists. An item is failed when a read or an append of one of its events fails; a failed item keeps its marker, and it does not stop the next item. The two events:
- Assessment (the repair of a missed route append): id `assessment_id`; kind `review_decision` and row `Review` / `ReviewDecision`, built as `pipeline_review_assessment_handler` builds them (status `Accepted` for an approval, `Rejected` for a rejection; the reason from `reason_code`); `actor_principal_ref` = the stored `reviewer_principal_ref`, no `actor_role`, `actor_role_label: Some("reviewer")`.
- Automatic Review: id `outcome_id`; `TraceCommonsAuditEvent::lifecycle_status_change` with `system_audit_tenant(tenant_id, PIPELINE_WORKER_AUDIT_ACTOR_REF)`, `LifecycleAuditActor::System`, status `Accepted` or `Rejected`, reason label `pipeline_review_approved` or `pipeline_review_rejected`; row as `append_lifecycle_status_audit` builds it, `actor_role_label: Some("system")`.

Log labels: `pipeline_worker_review_audit_failed` (pass), `pipeline_worker_review_audit_item_failed` (item, with a hash of the run id). Call the pass after the credit pass in the `due.credit_audits` block, with the limit 32, and `run_again` when it used its limit. The credit pass does not change.

**Route (`ingest`).** `pipeline_review_assessment_handler` sets `event.event_id = assessment.assessment_id` before its append (use `append_audit_event_mirrored`, which takes the event). Its log and its answer on a failed append stay. The claim handler does not change (Q3 b).

- [ ] **Step 1: Write the failing tests** in `http_tests`. Each sets `require_db_mirror_writes` (Global Constraints). Copy `a_credit_audit_append_with_a_required_mirror_does_not_read_the_file_log` for the pass, and the pipeline review route tests that use `quarantined_pipeline_run` for the route.
  - `the_worker_appends_one_audit_event_for_an_automatic_review`: (1) after a run's Review commits (approved) and the pass runs, the audit log holds one `lifecycle_status_change` event for the submission with the id of the Review `phase_outcomes` row, status `accepted`, actor `pipeline_worker`, and the marker is NULL; (2) the same for a rejected Review, with status `rejected`; (3) set the marker again by SQL and run the pass again: still one event, and the marker is NULL; (4) `main`'s audit verification for the tenant reports no mismatch.
  - `a_review_assessment_event_is_appended_one_time`: post an assessment through the route, then run the pass. One `review_decision` event exists, with the id `assessment_id` and the reviewer's principal reference as its actor.
  - `the_worker_repairs_a_missed_review_assessment_event`: record an assessment through the service (`record_review_assessment`, no route, so no event is appended), then run the pass. One `review_decision` event exists, with the id `assessment_id`, the actor `reviewer_sha256:..` and the status that the recommendation leads to; a second pass adds none; `main`'s audit verification reports no mismatch.
  - `a_review_audit_item_that_fails_keeps_its_marker`: make the append fail for one item (`fail_next_audit_file_append`); the pass counts one failed item, and the item's marker is not NULL.
  - In `pipeline_upgrade_tests.rs`: the column exists after the upgrade, the runtime role has `UPDATE (review_audit_pending_at)`, and `last >= 117`.
  Expected failure: no column; no `lifecycle_status_change` event; the assessment event has a random id.
- [ ] **Step 2: Implement** the migration, the registration, the store functions, the pass and the one route line.
- [ ] **Step 3: Run** the new tests, the upgrade tests, `cargo test -p trace-commons-server --lib -- migration`, `a_pipeline_credit_event_carries_its_witness_label_and_mains_audit_event`, and the present tests of the assessment route (search `http_tests` for `review_decision`). Expected: PASS.
- [ ] **Step 4: Docs.**
  - The grants of `pipeline_runs` are pinned in `deployment.md` (the V105 table with the `credit_audited_at` row, and the list at about line 682) and in `runbook` (about line 350): add `UPDATE (review_audit_pending_at)` (V117) to each. Add one sentence to `deployment.md`: V117 locks `pipeline_runs` until it commits; apply it before a tenant is routed, or with the worker stopped.
  - Runbook, the review audit paragraph (about 1689 to 1698): replace the sentences that call a failed append of the assessment event a permanent gap and "an open item in #1185". State: the worker's review audit pass appends a missed assessment event (id `assessment_id`; the reviewer is then named by the stored `reviewer_sha256:` reference) and one `lifecycle_status_change` event (actor `pipeline_worker`) for each automatic Review decision, in the step and at the cadence of the credit audit events; the marker is `review_audit_pending_at`; the two log labels; the event's time is the append time; a decision from before V117 gets no event; a claim whose audit append fails stays logged under `pipeline_review_audit_append_failed` and is not repaired (accepted in #1185).
- [ ] **Step 5: Commit.** `Append the review audit events from the worker`
- [ ] **Step 6: Package check** (Tasks 6 to 8): clippy with the CI allow-list; `cargo test -p trace-commons-server --test license_boundary`.

### Task 9: Runbook corrections (L5-3, L5-6, L5-8, the money rule, the NEAR mode row)

**Files:**
- Modify: `runbook`; `docs/operator/deployment.md`; one doc comment in `vp`
- Test: none (no test or script reads these files)

Find each place by the quoted words. Write the replacement in the runbook's style. The facts are verified at `6c3932b8` (ledger, `research-d.md`).

- [ ] **Step 1: L5-3 (drain list).**
  - "Keep the tenant on the drain list until the operational summary shows no pending runs" (about 277): replace the condition with a rule. Do not remove a tenant from the drain list while it has a pipeline run in any state. Reason: a withdrawal, a revocation, a retention expiry or a purge of a pipeline submission queues an index invalidation, with or without a runtime; only the worker processes it, and only for a tenant on one of its two lists; so the index entries of that content stay. State the rule for the fleet: one running process with a runtime lists the tenant. Two exceptions, which the code enforces: the index rebuild after a restore (`backup-restore.md`, steps 3 to 5) unsets both lists and sets them back; and a build with no runtime refuses to start while the drain list is set, so a rollback to such a build unsets it. In the same paragraph, correct "(the withdrawal needs only a runtime)": `main`'s routes queue the invalidation with no runtime also.
  - "A runtime processes them when it runs." (about 1871), "a runtime processes the queued invalidations when it runs" (about 2509), "for a later runtime to process" (about 1909): the worker of a process with a runtime processes them, and only for a tenant on its receipts list or its drain list.
  - `deployment.md`, "or off both lists" (about 1122) and "keep the tenant off both lists" (about 1134): these contradict the new rule. Add the cost (no index invalidation is processed for a tenant on neither list) and the instruction to put the tenant back on the drain list when a build with the guards runs.
- [ ] **Step 2: L5-6 (timing).** At "waits up to 10 seconds" (about 1964), state the bound one time: each interval is a minimum distance between two runs of a step; a due step runs when the worker reaches it; with a backlog it waits for up to 32 phase steps of its tenant and for the turn of each listed tenant before it. Then add "while the worker has no runs to process" to: "is paid within one interval after the contributor enrolls" (about 2052; write "two intervals": the pass and the line have independent clocks); "is paid within one interval." (about 2066; add "and the tenant's NEAR submit lock is free"); "within 10 seconds on any other" (about 2206); "waits at most about a minute once a worker runs" (about 1940). At "Other tenants and the other phases are not serialized." (about 2241), state that this is true of the Score lock, and that one worker does one phase step at a time.
- [ ] **Step 3: L5-8 (stale lines).**
  - "Payout takes only a complete run's `trace_credit` legs" (about 1994) and "the payout pass pays the complete runs of every bundle" (about 1500): the payout takes a leg that is `complete`; the state of the run does not matter.
  - "Fail-closed dependency qualification" (about 1460 to 1481): the refusal also applies when `TRACE_COMMONS_PIPELINE_DRAIN_TENANT_IDS` lists a tenant; and the check is the bundle qualification of the default package (the list that the section gives lower down), not "scorer, embedder, index, or any registered settlement adapter".
  - The doc comment of `process_payout` in `vp` ("once the run is complete ... a run that is not complete is returned untouched"): it pays the completed legs, whatever the state of the run, and it takes up each `failed` line again.
- [ ] **Step 4: The money rule.** After the paragraph on `main`'s gate evaluate route (about 2335), add: `main`'s manual credit route (`POST /v1/review/{id}/credit-events`) and `main`'s utility credit routes (`POST /v1/workers/utility-credit`, `POST /v1/workers/utility-attestations`) stay open for a pipeline submission, as for a legacy submission (owner decision of 2026-10-09). The credit is `main`'s ledger row. The pipeline's own reads and reports do not show it. `main`'s status document counts it as ledger credit.
- [ ] **Step 5: The NEAR mode row.** Replace "Both windows are open items in #1185." (about 2037) with: both windows are accepted and are not tracked (#1185, C4 and finding 3b); the check before a mode change is the control.
- [ ] **Step 6: Check and commit.** `cargo fmt --all -- --check`; `cargo check -p trace-commons-server --all-targets`. Commit: `Correct the pipeline runbook's drain rule, timing figures and stale lines`

### Task 10: A three-way payout confirmation (L2-3)

This task and Task 11 are the two payout boxes (Q1).

**Files:**
- Modify: `credit`: the trait `NearPayoutAdapter`, `DryRunNearPayoutAdapter`, `RecordingNearAdapter` (add `record_failure`), the unit test of `confirmation`
- Modify: `vp`: `dispatch_near_settlements` (the confirmation arm, about 13975), a new label, the doc comment above it
- Modify: test doubles: `CountingNearAdapter`, `BadEvidenceNearAdapter`, `HoldingNearAdapter`, `ConfirmationHoldingNearAdapter` (`rt_tests`), `QualifiedTestNearAdapter` (`tests`)
- Modify: `runbook`, the paragraph "This release has no operator route or tool that retries a `failed` payout" (about 2089)
- Test: `rt_tests`

**Design.** In `credit`: `pub enum NearPayoutConfirmation { Pending, Confirmed(NearConfirmationEvidence), Failed }`, and `async fn confirmation(&self, idempotency_key: &str) -> NearPayoutConfirmation`. In the dispatch: `Pending` is as `None` today; `Confirmed(evidence)` is the present code; `Failed` is one statement, then the next line:

```sql
UPDATE trace_near_credit_outbox
   SET status = 'failed', confirmed_at = NULL, last_error_hash = $3
 WHERE tenant_id = $1 AND near_outbox_id = $2 AND status = 'submitted'
   AND COALESCE(near_call_json ->> 'pipeline_submission_mode', $5::TEXT) = $4
```

with the mode guard of the confirm statement (same parameters; a money guard: a line is failed only in the mode that submitted it) and `last_error_hash` = the hash of the new label `near_transaction_failed` (`pub const`, beside `near_submit_failed`). The leg takes its label from the line, not from a flag of the call (a flag is lost when the process stops between the two writes, or when a second replica reads the line later): `record_payout_state_on` reads `last_error_hash` with `status`, and gives a leg with a `failed` line of that hash the label `near_transaction_failed` (each other `failed` line keeps `near_submit_failed`). The pass does not submit the line again: a `failed` leg is not in the work list. `near_transaction_hash` stays. No migration: the two CHECK constraints allow `failed`.

- [ ] **Step 1: Write the failing test** `a_payout_that_fails_on_chain_is_marked_failed_and_not_submitted_again` in `rt_tests`. Copy `payout_submits_once_and_confirms`; use `RecordingNearAdapter::record_failure(key)`. Assertions: after the pass that polls the line, the outbox line is `failed` and keeps its transaction hash, and the leg's payout is `failed` with the label `near_transaction_failed`; the next pass lists nothing, and the adapter saw one `submit` only. Expected failure: `record_failure` does not exist.
- [ ] **Step 2: Implement** the enum, the trait change, the two adapters, the five test doubles (each maps its present `Some` to `Confirmed` and `None` to `Pending`), and the arm.
- [ ] **Step 3: Run** the new test, `payout_submits_once_and_confirms`, `a_payout_error_on_one_run_does_not_stop_the_pass`, `a_payout_that_fails_after_the_pass_listed_it_is_not_submitted_again`, `a_line_submitted_under_http_is_not_confirmed_under_dry_run`, and the unit tests of `credit` (`cargo test -p trace-commons-server --lib versioned_pipeline_credit`). Expected: PASS.
- [ ] **Step 4: Runbook.** Add to the paragraph: a submitted line whose transaction the adapter reports as failed on chain becomes `failed` under `near_transaction_failed`; the contributor is not paid; the pass does not submit the line again, and no route pays or closes it in this release; a payment by hand goes into the operator's own record. At the check before a NEAR mode change (the NEAR mode row), add: a line that is `failed` under `near_transaction_failed` is resolved when that record exists.
- [ ] **Step 5: Commit.** `Mark a NEAR payout that failed on chain as failed`

### Task 11: Audit rows and the missing controls for the pipeline payout (L5-5)

**Files:**
- Modify: `vp`: `dispatch_near_settlements`, `pay_out_runs_on`, `process_payout_on`, `process_payouts` (about 13533), the new type and method
- Modify: `runtime`: the payout step of `drain_pipeline_tenant` (about 1195)
- Modify: `runbook`: the payout controls table (about 2027 to 2040), the NEAR payout list
- Test: `http_tests`

**Design.** `pub struct PipelinePayoutTally { pub submitted: u64, pub submit_failed: u64, pub confirmed: u64, pub chain_failed: u64 }` in `vp`. `dispatch_near_settlements` takes `&mut PipelinePayoutTally` (through `pay_out_runs_on` and `process_payout_on`) and adds one: to `submitted` after `mark_near_outbox_line_submitted` returns; to `submit_failed` after `mark_near_outbox_line_failed` returns; to `confirmed` and to `chain_failed` from the row count of the confirm `UPDATE` and of Task 10's `UPDATE` (a statement that changed no row counts nothing: two replicas can poll one line). A new `PipelineService::process_payouts_tallied(&self, tenant_id: &str, limit: i64, tally: &mut PipelinePayoutTally) -> anyhow::Result<usize>` holds the present body of `process_payouts`; `process_payouts` stays and calls it with a tally that it drops, so its tests do not change. The worker's payout step owns the tally, calls `process_payouts_tallied`, and appends the rows after the call in the `Ok` arm and in the `Err` arm (an error in the submit part must not lose the counts of the confirm part). When `confirmed + chain_failed > 0` it appends one event of kind `near_credit_outbox_confirm` with the counts `confirmed` and `failed`; when `submitted + submit_failed > 0`, one of kind `near_credit_outbox_submit` with the counts `submitted` and `failed`. A pass that changed no line appends nothing (a leg that waits for a NEAR account is listed again each interval and changes no line). Each event and row has the shape that `append_near_credit_outbox_confirm_audit` and `append_near_credit_outbox_submit_audit` build (`submission_id` nil, row `Retain` / `Maintenance { surface, purpose_hash, dry_run: false, action_counts }`), with the purpose label `pipeline_near_payout`, the actor `system_audit_tenant(tenant_id, PIPELINE_WORKER_AUDIT_ACTOR_REF)`, `actor_role: None`, and `actor_role_label: Some("system")`. A failed append is logged under `pipeline_worker_payout_audit_failed` and the step continues.

- [ ] **Step 1: Write the failing test** `a_pipeline_payout_pass_appends_mains_payout_audit_rows` in `http_tests`. Use `trace_credit_payout_service` (as `mains_operational_summary_leaves_a_failed_pipeline_payout_line_out` does) and run the worker's payout step. Assertions:
  1. after the pass that submits one line, the audit log holds one `near_credit_outbox_submit` event with `submitted=1` in its row's counts and the actor `pipeline_worker`;
  2. after the pass that confirms it, one `near_credit_outbox_confirm` event with `confirmed=1`;
  3. a pass over a leg whose account has no NEAR payout target (a held leg) adds no event;
  4. `main`'s audit verification for the tenant reports no mismatch.
  Expected failure: no event.
- [ ] **Step 2: Implement** the design.
- [ ] **Step 3: Run** the new test, the tests of Task 10, and `near_credit_outbox_workers_never_touch_a_pipeline_payout_row` (`tests`). Expected: PASS.
- [ ] **Step 4: Runbook.** Controls table: three rows, each "not applied", for `TRACE_COMMONS_NEAR_CREDIT_OUTBOX_SCHEDULER_ENABLED` (it starts `main`'s scheduler; it does not stop the pipeline payout; `TRACE_COMMONS_NEAR_SETTLEMENT_MODE=disabled` stops it, and `main`'s payouts too), `..._SCHEDULER_SUBMIT_LIMIT` (`main`'s rows for each tick; the pipeline pass takes at most 32 legs of a tenant and runs again when it took 32, so no setting bounds its submits; the same is true of `..._SCHEDULER_CONFIRM_LIMIT`), and `..._SCHEDULER_DRY_RUN` (`main`'s preview; the pipeline's dry run is the mode `dry_run`). NEAR payout list: a pass that submitted, confirmed or failed one line or more appends `main`'s two payout audit kinds under the actor `pipeline_worker` and the purpose `pipeline_near_payout`, with the counts of those lines; a pass that changed no line appends none; the outbox lines hold the mode of each line; the log label; a lost row is not repaired.
- [ ] **Step 5: Commit.** `Append main's payout audit rows for a pipeline payout pass`
- [ ] **Step 6: Package check** (Tasks 9 to 11): clippy with the CI allow-list; `cargo test -p trace-commons-server --test license_boundary`.

### Task 12: Branch gate (no commit)

- [ ] `cargo fmt --all -- --check`.
- [ ] `cargo check -p trace-commons-server --all-targets`, then `cargo check -p trace-commons-server --features near-ai-scorer --all-targets` (the production assembly is behind that feature and uses the changed types).
- [ ] Clippy with the CI allow-list; `cargo test -p trace-commons-server --test license_boundary`; `cargo test -p trace-commons-server --no-run`.
- [ ] The multi-lens review of the whole branch (`common.md`, review rule 5). No Critical finding may stay open.
- [ ] Write the PR text to `replies.md` in the ledger. It names each box of #1185 that the PR closes, the migration V117, and the accepted gap of the claim event. Ask the owner for the push. The upstream CI runs the suites; before the merge, read also the results of the two jobs that are not required checks (the whole-bin `near-ai-scorer` job and `pipeline qualification and restore`).
