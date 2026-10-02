# Versioned pipeline PR 5 (qualified routing, receipt ownership, containment, rollback, and the legacy drain rehearsal) implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** Approved by the owner on 2026-10-02 (end of phase A of `.superpowers/workstreams/pr5-activation.md`), with the answers in "Owner decisions". The owner started phase B the same day.

**Goal:** Build delivery PR 5 (`vp/pipeline-activation`), stacked on PR 4 (`vp/pipeline-qualification`): a committed routing record for each tenant, a permanent owner for each receipt, containment, rollback to an earlier qualified bundle, the qualified activation gate, policy suspension, signed check results, a committed index rebuild fence, and a rehearsal of the legacy drain against real pending legacy work. Production routing stays off: a tenant routes to the pipeline only after an operator activates a qualified bundle for it.

**Architecture:** The two tenant lists that ingest reads at start stay. They become the scope of one process: the tenants that its worker drains and that it can route. Inside that scope, a committed row in `pipeline_tenant_routing` decides where a new receipt goes (`legacy`, `pipeline`, or `contained`). The upload handler reads the row for each request, and the pipeline's receipt transaction reads it again under a row lock, with the active bundle. Each receipt gets one row in `pipeline_receipt_ownership`, committed with the run (pipeline) or claimed before the legacy write (legacy), so a retry always goes to its first owner. Activation and rollback go through one store operation that checks the stored qualification, a promotion decision that the server evaluates from signed check results, the deployed code revision, the current dependency profile, and four runnable policies, and then writes the active bundle, the routing row, and an immutable event in one transaction. The legacy drain is a read: it counts the pending work that the legacy path still owes, from `main`'s own tables.

**Tech Stack:** Rust (axum, tokio, tokio-postgres, deadpool-postgres, serde, ring, base64, chrono; all are existing dependencies), PostgreSQL 16 with forced RLS, Python 3 standard library only (`scripts/operator/pipeline.py` and `pipeline_tooling/`), GitHub Actions.

**Spec:**
- `docs/superpowers/plans/2026-09-22-versioned-pipeline-implementation-brief.md` on local `stack/07-pipeline-activation` (binding; read with `git show stack/07-pipeline-activation:<path>`): section 2 (identity, recovery, and guards), section 3A (qualification inspects the objects that the constructor was given), section 3D (containment row), section 5 (legacy writer and table retirement stays out), section 6 (delivery row 5; each delivery PR carries its own migrations), section 7 (gates: mixed legacy and pipeline receipts, exact replay, failed activation, containment, rollback for future runs, suspension and resumption, actual pending legacy work).
- On `main`: `docs/superpowers/specs/2026-09-14-versioned-pipeline-package-qualification.md` (`activate_qualified_bundle`), `docs/superpowers/specs/2026-09-11-versioned-pipeline-behavioral-contracts.md` (SUB-003, SUB-004, BND-002, BND-003, GRD-004, OPS-003, OPS-004, LAB-003, SCN-007, SCN-008, SCN-013, SCN-015), `docs/superpowers/specs/2026-09-11-versioned-pipeline-contract-test-manifest.json`.
- At PR 4's head: `docs/operator/pipeline-activation.md`, `docs/operator/pipeline-qualification.md`, `docs/operator/backup-restore.md`, and the PR 4 plan `docs/superpowers/plans/2026-09-29-versioned-pipeline-pr4-qualification-plan.md` (this plan uses its form).
- Shared rules: `.superpowers/workstreams/common.md`. Brief: `.superpowers/workstreams/pr5-activation.md`.

**Port source:** `ef97a459` (`origin/stack/07-pipeline-activation`). Read a port file with `git show ef97a459:<path>`. The PR 5 parts are `crates/trace-commons-server/src/versioned_pipeline_activation.rs` (1474 lines), `migrations/V80__versioned_pipeline_activation.sql` (200 lines), `activate_qualified_bundle` (port `versioned_pipeline_qualification.rs` lines 512 to 607), the policy intervention code (port `versioned_pipeline.rs` lines 239 to 290, 678 to 840, 3834 to 3870; port `migrations/V77__versioned_pipeline_authority_privacy.sql` lines 59 to 110 and 186 to 222), and the test `pipeline_activation_rollback_containment_and_writer_retirement` (port `tests/versioned_pipeline_runtime_pg.rs` lines 713 to 1076). The port is a source of logic, not of text: `main` changed under it (see "What the port assumed that `main` does not have").

**Scope:** PR 5 only. It does not retire a legacy writer or a legacy table (brief section 5). It does not turn production routing on.

**Test bodies:** as in PR 2, PR 3, and PR 4 (owner decision recorded in the PR 2 ledger), a test given as a list of assertions is written by the implementer as real calls. Every listed assertion is required.

## State on 2026-10-02 (verified with read-only commands)

- `upstream/main` is at `51512926`. Its highest migration is V106. PR 3 (#1143) is in it as the squash `340dd71a`.
- PR 4 is TraceCommons/trace-commons#1166 from `vp/pipeline-qualification`, head `cbe165ff`, open and not a draft, with V107 and V108. It waits for a review answer, so its code can still change.
- The PR 3 follow-up branch `vp/pipeline-compat-followups` (local; `c080db96` when this plan was written, `f8bf4b2e` in phase B) claims V109.
- No other open upstream PR adds a file under `migrations/`. So PR 5 takes V110 to V113 as working numbers (P5-D2).
- Every line number in this plan is a line of `upstream/vp/pipeline-qualification` at `cbe165ff`. Search by symbol when the line moved.

## What the port assumed that `main` does not have

Read this before any port step. Each line is a reason that a port step in this plan changes the port's logic.

1. **A toy legacy receipt.** The port's `record_legacy_receipt` inserts its own `trace_submissions` row and a `trace_credit_ledger` row. On `main` the legacy receipt is `submit_trace_handler` (`trace-commons-ingest.rs`, line 14428): account admission, a file record, a DB mirror, and a gate decision later. PR 5 does not port `record_legacy_receipt`, `complete_legacy_work`, `submit_switched`, or `submit_ingest_receipt`.
2. **A caller-written pending-work table.** The port's `pipeline_legacy_owned_work` holds rows that the caller marks `pending` and `complete`. It cannot show actual pending legacy work. PR 5 reads `main`'s tables instead (P5-D8).
3. **A caller-built promotion decision.** The port's `activate_tenant` takes a `PromotionDecision` struct from its caller. PR 4 made `qualify_bundle` evaluate the evidence itself. PR 5's activation does the same (P5-D9).
4. **A second copy of the selected bundle.** The port keeps `selected_bundle_id` in the routing row and `bundle_id` in `pipeline_active_bundles`. `main` reads only `pipeline_active_bundles` (`versioned_pipeline.rs`, lines 1413, 1431, 7974). PR 5 keeps one copy (P5-D3).
5. **`operational_status` on the policy status row.** `main`'s `pipeline_bundle_policy_status` (V93) has `runnable BOOLEAN` and `error_label`. Nothing on `main` updates it. PR 5 adds the writer, so it must also add the commit-time guard and the payout guard that `main` does not have (P5-D12).
6. **`trace_credit_ledger.ledger_source_key`.** `main` has no such column. A pipeline ledger row is unique on `(tenant_id, pipeline_run_id, score_outcome_id, instrument_id)` (V94), and its `external_ref` carries `pipeline_ledger_source_key`. PR 5 does not add the column (P5-D7).
7. **The local binary.** `trace-commons-pipeline-local.rs` does not exist on `main` (PR 4 P4-D12). Its routes become admin routes of ingest (P5-D11).
8. **A cross-tenant expansion.** The port's `expand_activation` reads one tenant's routing to activate another. Every route on `main` takes its tenant from the credential. PR 5 does not port it (P5-D6).

## Owner decisions (2026-10-02, plan approval)

The owner approved this plan on 2026-10-02, answered its questions, and started phase B while PR 4 (#1166) was still at `cbe165ff` with a review required. Each answer confirms the plan as written; no task changed.

1. **Legacy drain (P5-D8):** the counts as planned. The tenant-wide legacy NEAR outbox count blocks `drained`.
2. **Size:** one PR. Tasks 5, 8, and 11 stay in PR 5.
3. **Qualification key (P5-D10):** PR 5's V112 widens the key to `(tenant_id, bundle_id, code_revision_hash)`. PR 4's V107 does not change.
4. **Deployed code revision (P5-D17):** the build-time variable `TRACE_COMMONS_BUILD_CODE_REVISION_HASH`, with `pipeline.py revision`. A binary without it refuses each qualification and activation.
5. **CI (P5-D19):** `pipeline qualification and restore` stays not required.
6. **Routing read (P5-D4):** with a runtime injected, every new upload of every tenant reads the routing row. A replica that lacks an activated tenant in its list refuses with `503`.
7. **Routes and grants (P5-D11):** admin routes in ingest; the runtime role gets `UPDATE (bundle_id, selected_at)` on `pipeline_active_bundles` in V112; the ungated `activate_bundle` methods are deleted. `pipeline_activation_events` is the audit record; no new action in `main`'s audit log.
8. **`deactivate` (P5-D6):** accepted.
9. **`terminate` and the suspension specification (P5-D12):** PR 5 ships `suspend` and `resume` and refuses `terminate`. The CMP-003 specification opens later, with promotion; it stays in "Items for other PRs".
10. **Test-only unqualified routing (P5-D5):** accepted.
11. **Items for other PRs:** reported as TraceCommons/trace-commons#1186 (assigned to the owner). Whether delayed utility credit for a pipeline submission is `main`'s to issue is open on that issue; it does not block PR 5's tasks.
12. **Signing key (P5-D13):** the holder is decided at promotion. PR 5 documents the rule (not a holder of the admin credential), and CI runs `qualify` unsigned.

## Preconditions (not tasks of this plan)

1. The owner approves this plan (end of phase A). Done on 2026-10-02.
2. The owner says that PR 4's review allows phase B. Done on 2026-10-02 (the owner started phase B with PR 4 at `cbe165ff`).
3. Phase B: create the worktree (brief command), record the base as `BASE_PR5` in the ledger, check the migration numbers again (P5-D2), and make the first commit: this plan as `docs/superpowers/plans/2026-10-02-versioned-pipeline-pr5-activation-plan.md`. Done on 2026-10-02: `BASE_PR5` = `cbe165fffde2fec53e4e4276f21c81e7e93efbdf` (PR 4's head, equal to `upstream/vp/pipeline-qualification`; it contains `upstream/main` `51512926`). The branch has no upstream tracking link, so a later sync names `upstream/vp/pipeline-qualification` in the merge command.

## Task order and the PR 4 dependency

Do the **Group A** tasks first. They use `main` code (PR 2 and PR 3) and new PR 5 modules only. Do a **Group B** task only after the owner confirms that PR 4's review has not changed the PR 4 part that the task uses (the last column), or after PR 5 has merged the changed PR 4 (common.md, flow rule 3). At the start of each task, compare `upstream/vp/pipeline-qualification` with the head that PR 5 last merged.

| Task | Group | PR 4 part that review can still change |
| --- | --- | --- |
| 1 Routing, event, and ownership tables (V110) | A | none. It adds only new tables, and names to three lists that PR 4 also extended (`TRACE_COMMONS_RLS_TABLES`, `PIPELINE_TABLES`, the `operational_summary` test list); a PR 4 change there is a text merge only |
| 2 Routing store, containment, and deactivation | A | none |
| 3 Ownership and routing in the receipt transaction | A | none (`submit`, `insert_receipt_records`, and `replay_receipt` are PR 2 and PR 3 code on `main`) |
| 4 Upload routing in ingest | A | none (`submit_trace_handler`, `route_pipeline_receipt`, and `pipeline_owned_submission_receipt` are on `main`) |
| 5 Policy interventions and their guards (V111) | A | none |
| 6 Legacy drain report | A | none |
| 7 Evidence that names one package | B | **`evaluate_promotion`**, **the check lists** (`PROMOTION_REQUIRED_CHECKS`, `checks.py`), the emit sites in the suites |
| 8 Signed check results | B | **`qualify_bundle`**, **`evaluate_promotion`**, **`pipeline.py`**, `results.py` |
| 9 The qualified activation gate and rollback (V112) | B | **`qualify_bundle`** (the record it writes and its two statements that name the key), V107, `PromotionDecision`, `ProductionDependencyProfile` |
| 10 Infrastructure profile and admin routes | B | **`qualify_bundle`**, `ProductionInfrastructureProfile` |
| 11 Committed rebuild fence (V113) | B | **the rebuild route** (`pipeline_index_rebuild_handler`, `PipelineIndexRebuilds`) and `rebuild_index_run` |
| 12 The three PR 5 checks in `qualify` | B | **the check lists**, **`pipeline.py`**, `test_pipeline_tooling.py` |
| 13 Runbooks, inventory, and CI | B | `docs/operator/pipeline-qualification.md`, `pipeline-activation.md`, and `backup-restore.md` as PR 4 leaves them; the CI job |
| 14 PR 5 gate | B | everything |

## Global Constraints

- Base: `BASE_PR5` (precondition 3). Changes flow down the stack only (common.md flow rules). A contract change (a type or trait in `crates/trace-commons-gate-api`, or text in `docs/superpowers/specs/`) goes to `main` as its own small upstream PR. A defect in PR 4 code is fixed on PR 4 by the PR 4 session. A defect in merged PR 3 code goes to the PR 3 follow-up PR. A session that finds work for another PR writes it down in the ledger and tells the owner; it does not make the change on `vp/pipeline-activation`.
- PR 5 takes each PR 4 push by a merge of `upstream/vp/pipeline-qualification`, between two tasks, never in the middle of a task. When PR 4 is squash-merged, PR 5 merges `upstream/main` one time. Never rebase. Record each merge in the ledger.
- PR 5 is local only. Nothing is pushed until the owner decides to publish it. Each outward action (a push, a GitHub comment, a PR description) needs a draft in the ledger's `replies.md` and the owner's approval (common.md).
- Port source: `ef97a459` only. Never port from `stack/02` to `stack/06`, or from `16b73b2d`. Do not port `src/bin/trace-commons-pipeline-local.rs`.
- Every new `.rs` file in `trace-commons-server`, `trace-commons-gate-api`, or `trace-commons-gate-enclave` starts with `// Copyright (C) 2026 K&Z Partners LLC` and `// SPDX-License-Identifier: AGPL-3.0-or-later`.
- No new third-party dependency. Rust uses only existing dependencies. Python uses only the standard library. Do not edit the expected sets in `tests/license_boundary.rs`.
- Operational output is hash-only or label-only. New labels, including every check id, reason code, and refusal label, match `^[a-z0-9_]{1,64}$`. No raw tenant id, principal, URL, token, key, or trace body in a log line, an error string, a stored row, a result, an evidence file, or a report. An actor is stored as the credential's `principal_ref` (already a hash-derived reference on `main`), never as a token or a name.
- Fail closed. A missing trust store, a missing deployed code revision, a missing qualification, an unreadable routing row, or a database error on the routing read refuses the operation with a safe label. It never falls back to the legacy path for a tenant whose routing row says `pipeline` or `contained`.
- Tenant scoping: every route takes its tenant from the credential. No route and no store method takes a second tenant. No cross-tenant read (PR 2 decision D2): the worker does not list tenants from the database.
- Every hashed JSON value goes through `trace_commons_protocol::canonical_json::to_canonical_vec` (the server's build has `serde_json/preserve_order` on, so `serde_json::to_vec` of a `Value` is not canonical). Python's canonical form is `json.dumps(sort_keys=True, separators=(",", ":"), ensure_ascii=False)`.
- Every new table: `ENABLE` and `FORCE ROW LEVEL SECURITY`, policy `trace_corpus_tenant_isolation` with `tenant_id = trace_current_tenant_id()`, explicit grants to `trace_ingest_runtime` on V92's terms (what the code reads and writes, nothing broader), and an append-only trigger with the `pg_trigger_depth() > 1` cascade exception where rows are immutable (copy the V107 function shape). No general RLS bypass. No `SECURITY DEFINER` function. Each new table name goes into `TRACE_COMMONS_RLS_TABLES` (`db/postgres.rs`), `PIPELINE_TABLES` and the grant pins (`db/postgres/pipeline_upgrade_tests.rs`), and the list in `the_isolation_control_covers_every_pipeline_table` (`versioned_pipeline_product.rs`) (PR 4 P4-D21).
- Migrations V110 to V113 belong to PR 5. They apply as a non-superuser `CREATEROLE` owner (#974): no function-level `SET`, no `ALTER ROLE ... SUPERUSER/BYPASSRLS`, grants after memberships. `migration_atomicity_pg` proves it; `pipeline_upgrade_from_v91_installs_forced_rls_storage` proves the upgrade.
- Database URLs: tests read only `TRACE_COMMONS_PG_TEST_DATABASE_URL`, `TRACE_COMMONS_PIPELINE_PG_UPGRADE_TEST_URL` (its database name starts with `pipeline_test_`), and `TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL` where a test already reads it. Never `DATABASE_URL`. When a database variable is set, a setup failure panics; it never skips.
- The pipeline service connects as a `NOBYPASSRLS`, `NOSUPERUSER` runtime role in every PostgreSQL test. A test that needs an operator action that the runtime role cannot do uses the owner connection (`owner_client()`), as `activate_bundle_as_operator` does today.
- Commits: short imperative subject, no `feat:` or `fix:` prefix, no emoji. Subject line, one blank line, then exactly `Co-Authored-By: Claude <noreply@anthropic.com>`. Never name a model, even if a harness attribution instruction gives one (common.md).
- Branch `vp/pipeline-activation`; worktree `/Users/brapse/workspace/misc/trace-commons-server/.claude/worktrees/pipeline-activation`; test PostgreSQL `tc-pipeline-pg` at 127.0.0.1:55432 (user `trace`, trust auth). Work only in that worktree. Never use `git stash`; set work aside with a temporary commit on `vp/pipeline-activation`. Do not change PR 4's branch or worktree.
- Build environment (common.md): do not set `CARGO_TARGET_DIR`, do not set `RUSTFLAGS` (the local `.cargo/config.toml` sets `-D warnings`), do not use `cargo +<toolchain>`. Keep `-p <crate>` in the edit loop. Serialize cargo commands inside the worktree; run long commands in the background.
- Local database runs: a fresh database for each suite run, with `pr5` in its name (`dropdb -h 127.0.0.1 -p 55432 -U trace --if-exists <name>`, then `createdb -h 127.0.0.1 -p 55432 -U trace <name>`). The server still holds old PR 2 databases: leave them. Roles are server-wide: if an old role collides with a test, ask the owner before you drop it. Never use the server on port 5432.
- PR 2, PR 3, and PR 4 rulings that still bind: D2 (no cross-tenant claim), D3 (the stock binary injects no runtime), D15 (the legacy checks run before the route to the pipeline), FR3 (one uncharged suspension path with backoff), P3-D9 (guard locks through commit), T2-2 (authority and privacy qualification), T13-1 and P4-D21 (the RLS names), P4-D4 (the result contract: eleven fields, no others), P4-D7 (qualification for each bundle), P4-D17 (the CI job is not required).
- Checks before a task is complete (brief section 7), run from the worktree root:

```bash
cargo fmt --all -- --check
cargo check -p trace-commons-server --bins
cargo test -p trace-commons-server --no-run
cargo clippy -p trace-commons-server --all-targets -- -D warnings -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
cargo test -p trace-commons-server --test license_boundary
```

  A task that touches Python also runs `python3 -m py_compile scripts/operator/pipeline.py scripts/operator/pipeline_tooling/*.py scripts/operator/pipeline-deployment-inventory.py` and `python3 scripts/operator/test_pipeline_tooling.py`. `--no-run` is not test execution; each task names the tests it must execute.

- Every fix brief and every task review follows common.md's review rules: the consumer sweep (rule 1), sibling sites (rule 2), the efficiency and runtime lens (rule 3), and triage (rule 6: a declined or deferred item that touches money, privacy, or `main`'s behavior goes to the owner).

## Review Focus

These five conditions are the ones most likely to hurt a person who uses this software. Each has a test in the task that owns the code.

1. **A receipt with two owners.** A retry of a receipt that the legacy path accepted before the switch must return the legacy receipt and must not start a pipeline run. A retry of a pipeline receipt after a rollback to legacy must replay the pipeline receipt and must not reach the legacy upsert. Two first uploads of one submission id that race a switch must end with one owner. Tests: `a_legacy_submission_is_never_given_a_pipeline_run` and `two_owners_cannot_commit_for_one_submission` (Task 3), `mixed_receipts_replay_to_their_first_owner_across_every_switch` and `a_remediation_of_a_legacy_quarantine_stays_on_the_legacy_path` (Task 4).
2. **Containment that does not contain, or that stops too much.** After `contain` commits, no replica starts a new pipeline run for the tenant, and a receipt in flight cannot commit one. Replays are still answered, withdrawals still work, and the worker still completes the pending runs. A contained tenant's new upload never falls back to the legacy path. Tests: `a_receipt_in_flight_cannot_commit_after_containment` (Task 3), `containment_refuses_new_receipts_and_keeps_pending_work` (Task 4, check `pipeline_activation_containment`).
3. **An activation that half-commits or passes on evidence that the server did not check.** A refused activation leaves the routing row, the active bundle, and the event log as they were. An activation needs a stored qualification, a promotion decision that the server evaluates from signed results, the deployed revision, the current dependency profile, and four runnable policies. Tests: `a_refused_activation_changes_nothing` and `activation_requires_every_term_of_the_gate` (Task 9), `an_unsigned_or_altered_result_cannot_qualify_a_bundle` (Task 8).
4. **A rollback or a suspension that rebinds a run or rewrites an outcome.** A run in flight keeps its bundle through an activation, a rollback, and a suspension. A suspended policy leaves the run in `retry` with a safe label, uncharged, and the run completes under the same bundle after the policy resumes. A policy suspended during a phase cannot commit that phase. A payout does not dispatch while the Settle policy is suspended. Tests: `rollback_selects_an_earlier_bundle_for_new_runs_only` (Task 9, check `pipeline_activation_rollback`), `a_policy_suspended_during_a_phase_cannot_commit_it` and `a_payout_waits_for_a_suspended_settle_policy` (Task 5).
5. **A drain report that reads zero while the legacy path still owes work.** The report counts pending legacy work from `main`'s own tables, for submissions that no pipeline run owns, and it never counts a pipeline-owned submission. Test: `the_legacy_drain_report_counts_real_pending_work_and_reaches_zero` (Task 6, check `pipeline_legacy_drain`).

### `main` integration (common.md, review rule 1)

Routing changes how `main` paths decide. The reviewer of each task checks this table against the code. The inventory is from `trace-commons-ingest.rs` and `pipeline_runtime.rs` at `cbe165ff` (32 sites; found on 2026-10-02 with searches for `PipelineReceipts`, `pipeline_drain_tenant_ids`, `pipeline_service`, `pipeline_store`, `pipeline_product`, `submission_has_pipeline_run`). Task 4 Step 1 runs the searches again and records the result in the ledger.

Today every routing input (the two lists and the presence of the runtime, the store, and the product store) is fixed in `AppState` at start. Only the database lookups are for each request.

| Path (function, line) | Reads today | With PR 5 |
| --- | --- | --- |
| New upload: `route_pipeline_receipt` (14336), `pipeline_runtime_for_tenant` (14181) | receipts list and runtime, at start | the routing row for each request, then again in the receipt transaction (P5-D4). Task 4 |
| Retried upload: `pipeline_owned_submission_receipt` (14259; called at 14537 and 14578), `pipeline_runtime_for_replay` (14204) | both lists, runtime, store, and `submission_has_pipeline_run` | not changed. A pipeline run is the proof of pipeline ownership, whatever the routing state is. Task 4 tests it in every state |
| Legacy writes after the route returns `None`: `store_envelope` (14784), `mirror_submission_to_db_with_options` (14880, 14908), `write_submission_record` (14903, 14906), `write_derived_record`, `append_audit_event_mirrored` (14930), `attempt.finish` (14984) | nothing | for a tenant in this process's scope or with a routing row, a legacy ownership claim commits before `store_envelope` (P5-D7). Task 4 |
| Legacy checks that run before the route for a routed tenant (D15): `admission::reserve` (14464), `attempt.processing` (14681), tombstone check (14694), `enforce_submission_quota` (14709) | nothing | not changed. A contained upload is refused after these checks, so `attempt.finish` still records the failed attempt |
| Withdrawal: `revoke_submission` (15230), `account_trace_withdraw_handler` (19179, 19273), `pipeline_submission_withdraw_handler` (19509), `reconcile_source_session_withdrawals` (22090), `run_maintenance` (72153, 72262) | runtime or store, and database lookups. None reads a list | not changed. They do not depend on routing. Task 4 proves a withdrawal of a pipeline submission in each routing state |
| Legacy gate: `gate_evaluate_worker_handler` (57881), `list_submissions_needing_gate_decision` (`db/postgres.rs` 5206) | runtime and `submission_has_pipeline_run`; `NOT EXISTS pipeline_runs` | not changed. Task 6 reads the same predicate for the drain report |
| Status: `submission_status_handler` (16484) | product store and both lists | not changed |
| Admin, export, and review routes that need the runtime or the product store (17060 to 17384, 42504 to 42815) | presence only | not changed. Task 10 adds routes with the same shape |
| Worker: `spawn_pipeline_worker` and `pipeline_worker_tenant_ids` (`pipeline_runtime.rs` 1179, 1195) | both lists, at start | not changed (D2: no cross-tenant read). The worker drains a tenant in every routing state |
| Rebuild route: `pipeline_index_rebuild_handler` (`pipeline_runtime.rs` 383) | both lists | keeps its refusal; Task 11 adds the committed fence |
| Start: `validate_pipeline_receipt_rollout` (4796), `validate_pipeline_drain_tenants` (4812), `validate_pipeline_tenant_bundles` (3965), `register_default_bundles_for_rollout_tenants` (`pipeline_runtime.rs` 1255) | both lists | not changed. Task 10 adds the trust stores and the deployed revision |
| Drill evidence and status: `active_rollout_flags_for_rollback_drill` (49669), `trace_commons_config_status_response` (12902), `pipeline_readiness_handler` | list counts | not changed. The receipts list now means "in scope", not "routed"; Task 13 says so in the runbook |

## Decisions made while writing this plan

Executors: copy each line into the PR 5 ledger as `Plan: Ruling: ...` at setup. Each is binding for this plan. The cost line says what changes if the owner disagrees. A decision marked **(owner)** is also a question at the end of this file.

- **P5-D1. Branch and order.** One local branch, `vp/pipeline-activation`, created from `upstream/vp/pipeline-qualification` in phase B (from `upstream/main` if PR 4 is merged by then). Group A tasks run first. Nothing is pushed until the owner decides to publish. Cost if wrong: none.
- **P5-D2. Migrations.** Four small migrations, each owned by one task: V110 `versioned_pipeline_activation.sql` (`pipeline_tenant_routing`, `pipeline_activation_events`, `pipeline_receipt_ownership`; Task 1), V111 `versioned_pipeline_policy_interventions.sql` (`operational_status`, `pipeline_policy_interventions`; Task 5), V112 `versioned_pipeline_activation_gate.sql` (the key change of P5-D10 and the grant of P5-D11; Task 9), V113 `versioned_pipeline_rebuild_fence.sql` (`pipeline_index_rebuild_fences`; Task 11). V110 adds only new tables, so the Group A tasks change nothing that PR 4 owns. Checked in phase B (2026-10-02): `upstream/main` ends at V106, PR 4 (#1166, the only open upstream PR with a migration) holds V107 and V108, `vp/pipeline-compat-followups` holds V109, and no other local or remote branch holds V110 or later, so V110 to V113 are free. Check them again at each merge: `git ls-tree --name-only upstream/main migrations/`, every open upstream PR that adds a file under `migrations/`, and `vp/pipeline-compat-followups`. If a number is taken, take the next free numbers and rename the files, the registry entries, the upgrade test's version list, and the grant pins together. Write the numbers in the ledger. Cost if wrong: merge the four files into one.
- **P5-D3. Two levels of routing, one copy of the bundle.** `TRACE_COMMONS_PIPELINE_RECEIPTS_TENANT_IDS` and `TRACE_COMMONS_PIPELINE_DRAIN_TENANT_IDS` stay, with their validation. They are the scope of a process: the worker drains their union (read once at start, D2), and a process can route only a tenant on the receipts list. The decision inside that scope is a committed row in `pipeline_tenant_routing`: `legacy`, `pipeline`, or `contained`. No row means `legacy`. The row has no bundle column: `pipeline_active_bundles` stays the one record of the selected bundle, and an activation writes both in one transaction. Reason: the worker cannot list tenants from the database without a cross-tenant read, and a second bundle column would give `submit` two answers. Cost if wrong: add `selected_bundle_id` to the row and move `submit`'s bundle read to it.
- **P5-D4. Where the routing row is read.** When a pipeline runtime is injected, `submit_trace_handler` reads the tenant's routing row for each new upload (one primary-key read; no cache, so a containment takes effect on every replica at once). The pipeline's staging and commit transactions read it again under a shared transaction advisory lock (`pipeline-routing:<tenant>`) and refuse when it is not `pipeline`; an activation, a rollback, a containment, and a deactivation take the same lock exclusively. An advisory lock is used, not a row lock, because a tenant can have no row. Outcomes for a new submission id: `pipeline` and the tenant on this process's receipts list: the pipeline. `pipeline` and the tenant not on the list: `503` `pipeline_tenant_not_served` (never the legacy path). `contained`: `503` `pipeline_receipt_intake_contained`. `legacy` or no row: the legacy path. A failed routing read: `503` `pipeline_routing_unavailable`. The stock binary injects no runtime and makes no read. Cost if wrong: read the row only for tenants on the receipts list (one fewer read for other tenants; a replica that lacks the tenant in its list then sends an activated tenant's receipts to the legacy path, as `main` does today).
- **P5-D5. Unqualified routing is for tests only.** Test bundles (the minimal family, and a compatibility bundle with reference dependencies) cannot be qualified, and the PR 2 to PR 4 suites route them through the receipts list. So: when a tenant has no routing row, is on the receipts list, and the process started with `TRACE_COMMONS_PIPELINE_ALLOW_TEST_DEPENDENCIES`, its receipts route to the pipeline, as today. The flag already refuses to combine with `TRACE_COMMONS_PIPELINE_RUNTIME_REQUIRED`. `PipelineServiceBuilder::with_unqualified_routing(bool)` carries it into the receipt transaction; its default is `false`. Without the flag, a tenant with no row is on the legacy path until an operator activates it: production routing stays off. Cost if wrong: remove the rule and make each test harness write a routing row through the owner connection.
- **P5-D6. Actions.** `activate` (to `pipeline` with a qualified bundle), `rollback` (to `pipeline` with an earlier qualified bundle), `contain` (to `contained`), `deactivate` (to `legacy`; new in this plan: without it a contained tenant has no way back to the legacy path, because P5-D4 refuses its uploads). Not ported: `expand_activation` (it reads a second tenant), `retire_legacy_writer` and `pipeline_legacy_writer_status` (brief section 5), `switch_bound_run_bundle` (it only returned an error; the V92 trigger `reject_pipeline_run_identity_mutation` already refuses a rebind, and Task 9 tests it). Cost if wrong: one action in `PipelineActivationStore` and one route.
- **P5-D7. Receipt ownership.** `pipeline_receipt_ownership` has one immutable row for each `(tenant_id, submission_id)`: `owner` is `legacy` or `pipeline`, and a pipeline row names its run. The submission id is the key because it is the request key that ingest gives the pipeline (`route_pipeline_receipt`) and the primary key of the legacy record. The pipeline's receipt transaction inserts its row with the run; a conflict means the legacy path owns the id, and the result is `PipelineReceiptResult::LegacyOwned`. The legacy path claims its row before its first write (`store_envelope`), only for a tenant that is in this process's scope or has a routing row; a conflict there means a pipeline run owns the id, and the upload is answered as `pipeline_owned_submission_receipt` answers it. A legacy receipt from before the tenant came into scope has no row: the handler never sends a remediation of an existing legacy record to the pipeline, and the `trace_submissions` primary key refuses a pipeline receipt for it, which PR 5 turns from an internal error into `LegacyOwned`. Pipeline runs from before V110 have no row; `submission_has_pipeline_run` stays the proof of pipeline ownership, and V110 does not backfill (no deployment routes to the pipeline, and a backfill under forced RLS needs a loop over tenants). Not ported: `trace_credit_ledger.ledger_source_key`; one owner for each submission is what stops two awards. Cost if wrong: a backfill block in V110, or a claim on every legacy receipt of every tenant.
- **P5-D8. The legacy drain is a count from `main`'s tables (owner).** `pipeline_legacy_owned_work` is not ported. `PipelineActivationStore::legacy_drain_report` counts, for submissions that no pipeline run owns: `awaiting_pii_backstop` (status), `gate_decision_pending` and `gate_decision_exhausted` (no `trace_gate_decisions` row; attempts below or at the maximum), `quarantine_review_pending` (status), `vector_index_pending` (accepted, current `duplicate_precheck` derived record, no active vector entry), `delayed_credit_unsettled` (a positive delayed-credit ledger event that no finalized settlement batch carries), `revocation_propagation_pending` (items `pending`, `in_progress`, or `failed` whose source submission is legacy-owned), and the tenant-wide `near_outbox_pending` (legacy rows: `instrument_id IS NULL`, status `pending`, `failed`, or `submitted`). `drained` is true when every count is zero. Not counted, with the reason in the report's doc comment: retention (it never ends while a submission lives), export jobs and benchmark and process-evaluation work (corpus jobs, not owed to a receipt), the DB mirror backfill (the reconciliation drill measures it). The rehearsal drives real legacy receipts and real legacy routes. Cost if wrong: add or remove one count.
- **P5-D9. The qualified activation gate.** `PipelineQualificationStore::activate_qualified_bundle_in` runs inside the activation's transaction and requires: a ready promotion decision with no blocker, evaluated at most 15 minutes ago; a stored qualification of the bundle for the deployed code revision; the decision's one code revision equal to it; the decision's one package equal to the stored package's digests; a dependency profile for this bundle with no blocker and the stored `runtime_dependency_digest`; a stored package that still loads and validates; four runnable policies; and the startup checks of a tenant bundle (`check_runnable_package`: `main`'s gate configuration and the credit issuer). The server builds the decision itself: the route takes signed check results and calls `evaluate_promotion` at the time of the call, as `qualify_bundle` does. An activation also needs a passing readiness, which the server computes from the tenant's operational summary; a rollback does not (it must work when the pipeline is not healthy). Cost if wrong: accept a caller's decision in the store API only (tests), which the route never passes.
- **P5-D10. One qualification for each bundle and code revision (owner).** V107's key is `(tenant_id, bundle_id)`, and `qualify_bundle` refuses a second row with other metadata. The gate requires the deployed revision to equal the qualified revision. So after a new server revision is deployed, no bundle that was qualified before can be activated again, and a rollback to an earlier bundle is not possible. V112 changes the key to `(tenant_id, bundle_id, code_revision_hash)`, and `qualify_bundle` keeps its conflict rule inside one revision. Cost if wrong: leave the key; then a rollback works only while the deployed revision is the one that the earlier bundle was qualified on, and the runbook says "contain, then qualify a new bundle". If the owner prefers, the PR 4 session changes V107 instead (flow rule 1), and V112 drops that statement.
- **P5-D11. Routes.** Admin routes of ingest, behind `authenticate_with_tenant_access_grant` and `require_admin`, tenant from the credential: `GET /v1/admin/pipeline/routing`, `POST /v1/admin/pipeline/qualifications`, `POST /v1/admin/pipeline/activate`, `POST /v1/admin/pipeline/rollback`, `POST /v1/admin/pipeline/contain`, `POST /v1/admin/pipeline/deactivate`, `POST` and `GET /v1/admin/pipeline/policy-interventions`, `GET /v1/admin/pipeline/legacy-drain`. `pipeline_activation_events` is the audit record of a routing change (immutable, hash-only); PR 5 adds no new action to `main`'s audit log. The runtime role gets `UPDATE (bundle_id, selected_at)` on `pipeline_active_bundles` (V112), which V93 withheld because no ingest route switched a bundle. So that the grant opens no ungated switch, Task 9 deletes `PgPipelineStore::activate_bundle` and `PipelineService::activate_bundle` (no route and no test calls them; tests switch a bundle through the owner connection): after PR 5 the gate is the only statement in the crate that updates the row. Cost if wrong: a new audit action and its migration; or an operator binary with its own database role in place of the routes.
- **P5-D12. Policy interventions and their guards.** Port `intervene_policy`, `list_policy_interventions`, and `PolicyOperationalStatus`. The schema holds the three states of the port, but PR 5 accepts only `suspend` and `resume`: `terminate` is refused with `policy_intervention_not_supported`, because no specification says what happens to a run that is bound to a terminated policy (CMP-003). `operational_status` is added beside `runnable`, with a check that keeps them equal (`runnable = (operational_status = 'runnable')`), so no reader on `main` changes. Because PR 5 adds the first writer of this row, it also adds what GRD-004 and the brief's section 2 require and `main` lacks: each phase commit (`commit_review`, `commit_score_in`, `commit_settle`) locks the policy status row `FOR SHARE` and refuses the commit when the policy is not runnable; `process_payouts` checks the Settle policy before each dispatch. `PipelineOperationalSummary` gains `suspended_policy_count` (OPS-003). CMP-003 says that suspension controls need a follow-up specification before production use; that text is a contract PR (flow rule 1), not PR 5. Cost if wrong: drop Task 5; then nothing can suspend a policy, and the rollout has containment only.
- **P5-D13. Signed check results.** A result stays exactly `trace_commons.pipeline_check_result.v1` (P4-D4). A new envelope, `trace_commons.pipeline_check_attestation.v1`, holds the result, its maximum age, an optional `corpus_digest` and `input_digest`, and an Ed25519 signature over the envelope's canonical hash. `pipeline.py qualify --signing-key PATH --signing-key-id ID` writes one attestation for each required result after the run passed. `qualify_bundle` and the activation routes take attestations, verify them against a `CheckResultTrustStore` (a separate key set from the package trust store), and only then evaluate them. The server still caps the maximum age at 7 days. Who holds the signing key is operator policy; the runbook says that it must not be a holder of the admin credential. Cost if wrong: drop Task 8; the evidence stays caller input, and the runbook keeps PR 4's warning.
- **P5-D14. `corpus_digest` and `input_digest`.** `BundleQualificationMetadata` is no longer a caller's input in the route. The server computes: `corpus_digest` = the canonical hash of the map from check id to the `corpus_digest` of each attestation that carries one; `input_digest` likewise; `configuration_digest` from the package; `code_revision_hash` = the deployed revision; `runtime_dependency_digest` from the profile; `evidence_hash` from the decision. The store API keeps the metadata parameter and checks the first two against the attestations (`bundle_qualification_corpus_mismatch`, `bundle_qualification_input_mismatch`). Cost if wrong: none; it only removes caller input.
- **P5-D15. Mechanics checks name no package.** The checks that prove mechanics with their own test bundles emit no package digests: the upgrade, crash matrix, independent instruments, stale lease, lease renewal, receipt replay, payout recovery, index rebuild, orphan sweep, restart recovery, receipt ownership, the minimal corpus run, and the three PR 5 checks. The four checks that name a package (`pipeline_bundle_qualification`, `pipeline_http_corpus_compatibility`, `pipeline_http_corpus_hf_local`, `pipeline_restore_drill`) build it from one shared constructor, so a `qualify` run is not mixed. Running those four against an operator's candidate package needs the production assembly, which is not in this tree (D3): that is promotion work. Cost if wrong: run every mechanics check against the candidate (a parameter in about 12 tests).
- **P5-D16. The rebuild fence.** `pipeline_index_rebuild_fences` holds one row for a tenant while a rebuild writes: `fenced_until` is committed before the first write and moved forward before each run's writes, to the write deadline plus `PIPELINE_INDEX_WRITE_FENCE_MARGIN_SECONDS`. `claim_due_index_invalidations` claims nothing for a tenant with an unexpired fence, in the same statement that reads the invalidations. A rebuild that ends cleanly deletes its row; one that is lost leaves a row that expires. The route keeps its refusal for a tenant that its own process routes or drains (a Score must not read a partly rebuilt index), and the restore runbook keeps step 3. What changes: another replica, a lost session, an abort, and a process exit can no longer let an invalidation run before the rebuild's last write. Cost if wrong: drop Task 11; the gap stays as PR 4 documents it.
- **P5-D17. The deployed code revision (owner).** The qualified revision is `pipeline.py`'s tree hash (`_code_revision_hash`). The server knows only its commit (`trace_commons_build_info::COMMIT`). PR 5 adds `pipeline.py revision`, which prints the tree hash, and the server reads `TRACE_COMMONS_BUILD_CODE_REVISION_HASH` at build time (`option_env!`). A release build sets it to `pipeline.py revision`'s output on the same checkout. A binary built without it refuses every qualification and activation with `bundle_runtime_revision_unknown`. Cost if wrong: qualify against the commit instead (change `_code_revision_hash` to hash `git rev-parse HEAD`, which a dirty tree does not show).
- **P5-D18. Required check ids.** `pipeline_activation_containment`, `pipeline_activation_rollback`, and `pipeline_legacy_drain` go into `PROMOTION_REQUIRED_CHECKS` and into `checks.py`'s `REQUIRED_DATABASE_CHECKS` in one commit (Task 12). The existing Python test `REQUIRED_CHECK_IDS` equals the Rust list less the promotion-only ids keeps them equal. Cost if wrong: edit two lists.
- **P5-D19. CI.** The job `pipeline qualification and restore` stays not required (P4-D17). It runs `qualify`, so it runs the three new checks with no workflow edit. A promotion to a required check is a new owner decision (question 5). Cost if wrong: none.
- **P5-D20. The contract manifest.** PR 5 does not edit `docs/superpowers/specs/`. The refresh of the manifest's test ids (P4-D14) stays its own upstream PR; "Items for other PRs" lists the PR 5 test names that it needs. Cost if wrong: none.
- **P5-D21. Infrastructure profile.** `ProductionInfrastructureProfile` is derived from `AppState`, from the same fields that `GET /v1/admin/config-status` reports (Task 10 gives the table). A test deployment therefore has blockers and cannot qualify a bundle through the route. Cost if wrong: none.

## Deferred (not PR 5)

Do not build these in PR 5. A task that needs one of them has a defect in this plan; stop and ask.

| Item | Goes to |
| --- | --- |
| Retirement of a legacy writer or a legacy table (`retire_legacy_writer`, `pipeline_legacy_writer_status`, a disabled legacy upload) | A later operator change (brief section 5) |
| `expand_activation` (activation of one tenant from another tenant's readiness) | Not planned (P5-D6) |
| Real adapter evidence, a remote-provider restore, the pinned network HF canary, operator approval, and the package-bearing checks run against an operator's candidate package | Promotion (operator work, not a PR) |
| The HF network pin | The first PR that needs the network dataset (content: `hf-pin-final.json` in the PR 4 ledger) |
| A check of the deployed revision after each deploy | Promotion requirement (package qualification spec; the roadmap tracks it) |
| Promotion of `pipeline qualification and restore` to a required check | Owner decision (question 5) |
| Removal of the overlapping `postgres-suites` pipeline steps | After a maintainer checks coverage (brief section 4) |
| A cache of the routing row, or routing from the database without a tenant list | Not planned (P5-D3, P5-D4) |

## Items for other PRs (write down, tell the owner)

Found on 2026-10-02 by a search of `cbe165ff`, then checked against `main` at `51512926` by reading the code. No test reproduces them yet.

- **PR 3 follow-up PR (`main` integration of merged PR 3 code):** reported on 2026-10-02 as TraceCommons/trace-commons#1186 (four items), beside the owner's list TraceCommons/trace-commons#1185. PR 5 must close or accept each one before a tenant is routed; it fixes none of them.
  - #1186, Medium: `main`'s review queue and leases take pipeline-quarantined rows when DB reviewer reads are on (`review_quarantine_handler`, `claim_trace_review_lease` through the lease and claim-next routes, `apply_review_decision`). The decision route probably fails with a 500 at the envelope read; that was not run.
  - #1186, Medium: `POST /v1/workers/utility-credit` and `POST /v1/workers/utility-attestations` accept an accepted pipeline-owned submission (`read_utility_submission_record`), and `run_credit_settlement_unlocked` would then batch the event (no `pipeline_run_id` check). Open decision: whether delayed utility credit for a pipeline submission is `main`'s to issue.
  - #1186, Low: `gate_evaluate_worker_handler` checks `submission_has_pipeline_run` only through `state.pipeline_service`; it does not fall back to `state.pipeline_store`, as `revoke_submission` does.
  - #1186, Low: `PipelineProductStore::operational_summary`'s `near_outbox_by_state` groups every NEAR outbox row of the tenant, legacy rows included. `ActivationReadiness::from_operational_summary` (Task 2) does not read that field.
  - Already in #1185: the process evaluation and benchmark readers (L1-2), the vector summary (L1-1), a process without a runtime at the reconciliation and rollback-drill sites (L1-4), and the duplicate-row guard outside the legacy upsert (L4-8; Tasks 3 and 4 of this plan close it with the ownership row).
- **Upstream contract PR:** refresh the test ids in `docs/superpowers/specs/2026-09-11-versioned-pipeline-contract-test-manifest.json` (P4-D14), with PR 5's names for BND-003, GRD-004, LAB-003, SCN-007, SCN-008, SCN-013, and SCN-015 (the tests of Tasks 3, 4, 5, and 9); the follow-up specification for policy suspension, resumption, and termination that CMP-003 names; and, if P5-D10 stands, one sentence in the package qualification spec that a qualification is for one code revision.
- **PR 4 (only if the owner chooses it in question 3):** V107's key becomes `(tenant_id, bundle_id, code_revision_hash)`.

## File map

| File | Responsibility | Tasks |
| --- | --- | --- |
| `migrations/V110__versioned_pipeline_activation.sql` (new) | Routing, activation events, receipt ownership | 1 |
| `migrations/V111__versioned_pipeline_policy_interventions.sql` (new) | `operational_status`, `pipeline_policy_interventions` | 5 |
| `migrations/V112__versioned_pipeline_activation_gate.sql` (new) | The qualification key; `UPDATE` on `pipeline_active_bundles` | 9 |
| `migrations/V113__versioned_pipeline_rebuild_fence.sql` (new) | `pipeline_index_rebuild_fences` | 11 |
| `crates/trace-commons-server/src/db/postgres.rs` | Migration registry, `TRACE_COMMONS_RLS_TABLES`, static migration tests | 1, 5, 9, 11 |
| `crates/trace-commons-server/src/db/postgres/pipeline_upgrade_tests.rs` | Upgrade to V113, `PIPELINE_TABLES`, grant pins | 1, 5, 9, 11 |
| `crates/trace-commons-server/tests/trace_corpus_pg_rls.rs` | The RLS suite's migration includes | 1, 5, 9, 11 |
| `crates/trace-commons-server/src/versioned_pipeline_activation.rs` (new) | Routing types and store, containment, deactivation, activation and rollback, readiness, legacy claim, legacy drain report | 2, 6, 9 |
| `crates/trace-commons-server/src/lib.rs` | `pub mod versioned_pipeline_activation;` | 2 |
| `crates/trace-commons-server/src/versioned_pipeline.rs` | Routing and ownership in the receipt transactions; `LegacyOwned` and `NotRouted`; policy interventions, the commit guards, the payout guard; the rebuild fence | 3, 5, 11 |
| `crates/trace-commons-server/src/versioned_pipeline_product.rs` | RLS names; `suspended_policy_count` | 1, 5, 11 |
| `crates/trace-commons-server/src/versioned_pipeline_qualification.rs` | `activate_qualified_bundle_in`; attestations and the check trust store; `qualify_bundle_attested`; the three check ids; the deployed revision constant | 8, 9, 12 |
| `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` | Upload routing, the legacy claim, `AppState` fields, route registration, trust stores at start | 4, 10 |
| `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_runtime.rs` | The assembly check of the unqualified routing flag; the rebuild route's doc | 4, 11 |
| `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_activation.rs` (new) | The admin route handlers and the infrastructure profile | 10 |
| `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_http_pg_tests.rs` | Mixed receipts, containment, remediation, the not-served refusal, withdrawal in each state | 4 |
| `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_activation_pg_tests.rs` (new) | The legacy drain rehearsal; the admin route tests | 6, 10 |
| `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs` | Module declarations; the test state's routing flag | 4, 6, 10 |
| `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_corpus_pg_tests.rs`, `pipeline_restore_pg_tests.rs` | One shared candidate package; `corpus_digest` and `input_digest` in the corpus evidence | 7, 8 |
| `crates/trace-commons-server/tests/versioned_pipeline_runtime_pg.rs` | Routing store, receipt ownership, interventions, the gate, rollback, the fence; mechanics checks emit no package | 2, 3, 5, 7, 9, 11 |
| `scripts/operator/pipeline.py` | `revision`; `qualify --signing-key` | 8, 9 |
| `scripts/operator/pipeline_tooling/checks.py`, `results.py`, `report.py` | Mechanics checks without digests; the three check rows; attestations | 7, 8, 12 |
| `scripts/operator/test_pipeline_tooling.py` | Self-tests for the above | 7, 8, 12 |
| `scripts/operator/pipeline-deployment-inventory.py` | The new routes and tables | 13 |
| `docs/operator/pipeline-activation.md`, `pipeline-qualification.md`, `backup-restore.md`, `deployment.md`, `README.md` (operator index) | Runbooks | 13 |

## Interfaces (the names every task uses)

`versioned_pipeline_activation.rs` (Tasks 2, 6, 9):

```rust
pub const PIPELINE_RECEIPT_INTAKE_CONTAINED_LABEL: &str = "pipeline_receipt_intake_contained";
pub const PIPELINE_TENANT_NOT_SERVED_LABEL: &str = "pipeline_tenant_not_served";
pub const PIPELINE_ROUTING_UNAVAILABLE_LABEL: &str = "pipeline_routing_unavailable";
pub const ACTIVATION_ACTOR_INVALID_LABEL: &str = "activation_actor_invalid";
pub const ACTIVATION_STATE_INVALID_LABEL: &str = "activation_state_invalid";
pub const ACTIVATION_READINESS_FAILED_LABEL: &str = "activation_readiness_failed";
pub const EARLIER_QUALIFIED_BUNDLE_REQUIRED_LABEL: &str = "earlier_qualified_bundle_required";
pub const ACTIVATION_MAX_ERROR_COUNT: u64 = 0;
pub const ACTIVATION_MAX_WORK_AGE_SECONDS: u64 = 300;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)] #[serde(rename_all = "snake_case")]
pub enum RoutingState { Legacy, Pipeline, Contained }          // as_db / from_db: "legacy", "pipeline", "contained"
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)] #[serde(rename_all = "snake_case")]
pub enum ReceiptOwner { Legacy, Pipeline }                     // "legacy", "pipeline"
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)] #[serde(rename_all = "snake_case")]
pub enum ActivationAction { Activate, Rollback, Contain, Deactivate }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewReceiptRoute { Legacy, Pipeline, Contained, NotServed }
pub fn decide_new_receipt_route(routing: Option<RoutingState>, on_receipts_list: bool, unqualified_routing_allowed: bool) -> NewReceiptRoute;

pub struct TenantRouting { pub routing_state: RoutingState, pub activation_record_id: Uuid, pub actor_principal_ref: String, pub reason_code: String, pub evidence_hash: String, pub recorded_at: DateTime<Utc> }
pub struct ActivationEvent { pub event_id: Uuid, pub action: ActivationAction, pub previous_state: Option<RoutingState>, pub resulting_state: RoutingState, pub previous_bundle_id: Option<String>, pub resulting_bundle_id: Option<String>, pub actor_principal_ref: String, pub reason_code: String, pub evidence_hash: String, pub recorded_at: DateTime<Utc> }
pub struct ReceiptOwnership { pub owner: ReceiptOwner, pub submission_id: Uuid, pub run_id: Option<Uuid> }
pub struct ActivationReadiness { pub readiness_ok: bool, pub error_count: u64, pub max_work_age_seconds: u64, pub credit_reconciled: bool, pub index_consistent: bool, pub invalidation_clear: bool, pub evaluated_at: DateTime<Utc>, pub evidence_hash: String }
impl ActivationReadiness { pub fn from_operational_summary(summary: &PipelineOperationalSummary) -> Self; }
pub fn evaluate_activation_readiness(ready: &ActivationReadiness, now: DateTime<Utc>) -> Result<(), String>;
pub struct ActivationRequest<'a> { pub tenant_id: &'a str, pub bundle_id: &'a str, pub actor_principal_ref: &'a str, pub reason_code: &'a str, pub promotion: &'a PromotionDecision, pub runtime_code_revision_hash: &'a str, pub dependencies: &'a ProductionDependencyProfile }
pub struct LegacyDrainReport { pub generated_at: DateTime<Utc>, pub routing_state: Option<RoutingState>, pub pending: BTreeMap<String, u64>, pub drained: bool, pub evidence_hash: String }

pub struct PipelineActivationStore; // new(backend: Arc<PgBackend>)
impl PipelineActivationStore {
    pub async fn routing(&self, tenant_id: &str) -> Result<Option<TenantRouting>, DatabaseError>;
    pub async fn events(&self, tenant_id: &str, limit: usize) -> Result<Vec<ActivationEvent>, DatabaseError>;
    pub async fn ownership(&self, tenant_id: &str, submission_id: Uuid) -> Result<Option<ReceiptOwnership>, DatabaseError>;
    pub async fn claim_legacy_receipt(&self, tenant_id: &str, submission_id: Uuid) -> Result<ReceiptOwner, DatabaseError>;
    pub async fn contain(&self, tenant_id: &str, actor_principal_ref: &str, reason_code: &str) -> Result<TenantRouting, DatabaseError>;
    pub async fn deactivate(&self, tenant_id: &str, actor_principal_ref: &str, reason_code: &str) -> Result<TenantRouting, DatabaseError>;
    pub async fn activate_tenant(&self, request: ActivationRequest<'_>, readiness: &ActivationReadiness) -> Result<TenantRouting, DatabaseError>;   // Task 9
    pub async fn rollback_bundle(&self, request: ActivationRequest<'_>) -> Result<TenantRouting, DatabaseError>;                                  // Task 9
    pub async fn legacy_drain_report(&self, tenant_id: &str, gate_max_attempts: i32) -> Result<LegacyDrainReport, DatabaseError>;                // Task 6
}
```

`versioned_pipeline.rs` (Tasks 3, 5, 11):

```rust
pub const PIPELINE_LEGACY_RECEIPT_OWNED_LABEL: &str = "legacy_receipt_owned";
pub const PIPELINE_POLICY_INTERVENTION_NOT_SUPPORTED_LABEL: &str = "policy_intervention_not_supported";
pub enum PipelineReceiptResult { /* existing variants */ LegacyOwned, NotRouted(RoutingState) }
pub(crate) fn pipeline_routing_lock(tenant_id: &str) -> String;   // "pipeline-routing:{tenant_id}"
impl PipelineServiceBuilder { pub fn with_unqualified_routing(self, allowed: bool) -> Self; }
impl PipelineService { pub fn unqualified_routing(&self) -> bool; }
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)] #[serde(rename_all = "snake_case")]
pub enum PolicyOperationalStatus { Runnable, Suspended, Terminated }
pub struct PipelinePolicyInterventionRecord { pub intervention_id: Uuid, pub bundle_id: String, pub phase: Phase, pub action: String, pub actor_principal_ref: String, pub reason_code: String, pub previous_status: PolicyOperationalStatus, pub resulting_status: PolicyOperationalStatus, pub evidence_hash: String, pub recorded_at: DateTime<Utc> }
impl PgPipelineStore {
    pub async fn intervene_policy(&self, tenant_id: &str, bundle_id: &str, phase: Phase, action: &str, actor_principal_ref: &str, reason_code: &str) -> Result<PipelinePolicyInterventionRecord, DatabaseError>;
    pub async fn list_policy_interventions(&self, tenant_id: &str, bundle_id: &str) -> Result<Vec<PipelinePolicyInterventionRecord>, DatabaseError>;
    pub async fn set_index_rebuild_fence(&self, tenant_id: &str, fence_id: Uuid, fenced_until: DateTime<Utc>) -> Result<(), DatabaseError>;   // Task 11
    pub async fn clear_index_rebuild_fence(&self, tenant_id: &str, fence_id: Uuid) -> Result<(), DatabaseError>;                              // Task 11
}
impl PipelineService { /* the same two intervention methods, forwarding to the store */ }
```

`versioned_pipeline_qualification.rs` (Tasks 8, 9, 12):

```rust
pub const PACKAGE_QUALIFICATION_MISSING_LABEL: &str = "bundle_qualification_missing";
pub const PACKAGE_RUNTIME_REVISION_MISMATCH_LABEL: &str = "bundle_runtime_revision_mismatch";
pub const PACKAGE_RUNTIME_REVISION_UNKNOWN_LABEL: &str = "bundle_runtime_revision_unknown";
pub const ACTIVATION_PROMOTION_NOT_READY_LABEL: &str = "bundle_activation_promotion_not_ready";
pub const ACTIVATION_PROMOTION_STALE_LABEL: &str = "bundle_activation_promotion_stale";
pub const ACTIVATION_PACKAGE_MISMATCH_LABEL: &str = "bundle_activation_package_mismatch";
pub const ACTIVATION_PROMOTION_MAX_AGE_SECONDS: i64 = 15 * 60;
pub const DEPLOYED_CODE_REVISION_HASH: Option<&str> = option_env!("TRACE_COMMONS_BUILD_CODE_REVISION_HASH");
impl PipelineQualificationStore {
    pub async fn qualification(&self, tenant_id: &str, bundle_id: &str, code_revision_hash: &str) -> Result<Option<BundleQualificationRecord>, DatabaseError>;
    pub async fn activate_qualified_bundle_in(&self, tx: &Transaction<'_>, tenant_id: &str, bundle_id: &str, promotion: &PromotionDecision, runtime_code_revision_hash: &str, dependencies: &ProductionDependencyProfile, now: DateTime<Utc>) -> Result<Option<String>, DatabaseError>; // returns the bundle that was active before
    pub async fn qualify_bundle_attested(&self, tenant_id: &str, signed: &SignedBundlePackage, package_trust: &BundlePackageTrustStore, check_trust: &CheckResultTrustStore, dependencies: &ProductionDependencyProfile, attestations: &[PipelineCheckAttestation], code_revision_hash: &str) -> Result<BundleQualificationRecord, DatabaseError>;
}
pub const PIPELINE_CHECK_ATTESTATION_SCHEMA: &str = "trace_commons.pipeline_check_attestation.v1";
pub const QUALIFICATION_EVIDENCE_DEFAULT_MAX_AGE_SECONDS: u64 = 24 * 60 * 60;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)] #[serde(deny_unknown_fields)]
pub struct PipelineCheckAttestation { pub schema: String, pub result: PipelineCheckResult, pub maximum_age_seconds: u64, pub corpus_digest: Option<String>, pub input_digest: Option<String>, pub signature: BundlePackageSignature }
pub fn sign_check_result(result: PipelineCheckResult, maximum_age_seconds: u64, corpus_digest: Option<String>, input_digest: Option<String>, key_id: &str, pkcs8: &[u8]) -> anyhow::Result<PipelineCheckAttestation>;
pub struct CheckResultTrustStore; // new(keys: impl IntoIterator<Item = TrustedBundleKey>) -> Result<Self, String>
pub struct VerifiedEvidence { pub evidence: Vec<DrillEvidence>, pub corpus_digest: String, pub input_digest: String }
impl CheckResultTrustStore { pub fn verify_all(&self, attestations: &[PipelineCheckAttestation]) -> Result<VerifiedEvidence, String>; }
```

Ingest (Tasks 4, 10): `AppState` gains `pipeline_activation: Option<Arc<PipelineActivationStore>>`, `pipeline_unqualified_routing: bool`, `pipeline_package_trust: Option<Arc<BundlePackageTrustStore>>`, `pipeline_check_trust: Option<Arc<CheckResultTrustStore>>`, and `pipeline_code_revision_hash: Option<String>`. `IngestPipelineRuntimeContext` gains `unqualified_routing_allowed: bool`.

---

### Task 1: Routing, event, and ownership tables (V110) (Group A)

**Files:**
- Create: `migrations/V110__versioned_pipeline_activation.sql`
- Modify: `crates/trace-commons-server/src/db/postgres.rs` (the `MIGRATIONS` entry after V108's, `TRACE_COMMONS_RLS_TABLES`, the static migration tests beside V108's at about lines 7780 to 8062)
- Modify: `crates/trace-commons-server/src/db/postgres/pipeline_upgrade_tests.rs` (`PIPELINE_TABLES`, the grant pins, the version list `[92, 93, 94, 95, 105, 106, 107, 108]`)
- Modify: `crates/trace-commons-server/tests/trace_corpus_pg_rls.rs` (the migration include list, beside V108's at line 1431)
- Modify: `crates/trace-commons-server/src/versioned_pipeline_product.rs` (`the_isolation_control_covers_every_pipeline_table`)

**Interfaces:**
- Consumes: V92 (`pipeline_runs`), V90 (`trace_ingest_runtime`).
- Produces: the tables `pipeline_tenant_routing`, `pipeline_activation_events`, `pipeline_receipt_ownership`, with their grants.

- [ ] **Step 1: Write the failing tests.**
  - In `db/postgres.rs`, beside the V108 static test, `v110_defines_routing_events_and_ownership`: the V110 text contains each of `CREATE TABLE pipeline_tenant_routing`, `CREATE TABLE pipeline_activation_events`, `CREATE TABLE pipeline_receipt_ownership`, `CREATE TRIGGER pipeline_activation_events_reject_update`, `CREATE TRIGGER pipeline_receipt_ownership_reject_update`, one `FORCE ROW LEVEL SECURITY` line and one `CREATE POLICY trace_corpus_tenant_isolation` line for each of the three tables, `GRANT SELECT, INSERT ON pipeline_activation_events TO trace_ingest_runtime;`, and `GRANT SELECT, INSERT ON pipeline_receipt_ownership TO trace_ingest_runtime;`; it contains none of `SECURITY DEFINER`, `BYPASSRLS`, `pipeline_bundle_qualifications`, `pipeline_active_bundles`, `pipeline_legacy_owned_work`, `pipeline_legacy_writer_status`, `ledger_source_key`.
  - In `pipeline_upgrade_tests.rs`: add the three names to `PIPELINE_TABLES` (18 entries), add 110 to the version list, and add the grant pins of Step 3 to the expected grants. `pipeline_upgrade_from_v91_installs_forced_rls_storage` then fails until V110 exists. Add these assertions to it: a second `pipeline_receipt_ownership` row for one `(tenant_id, submission_id)` fails with a unique violation; an `UPDATE` and a direct `DELETE` of an ownership row and of an event row fail with the trigger's message; deleting the tenant removes its routing, event, and ownership rows.
  - In `versioned_pipeline_product.rs`: add the three names to the expected set of `the_isolation_control_covers_every_pipeline_table`.

- [ ] **Step 2: Run them to see them fail.** `cargo test -p trace-commons-server --lib v110_defines the_isolation_control_covers_every_pipeline_table`. Expected: FAIL (the file does not exist; the set differs).

- [ ] **Step 3: Write the migration.**

```sql
-- Qualified routing, activation history, and receipt ownership for the
-- versioned pipeline (delivery PR 5). Routing and ownership are explicit
-- records. Timestamps are audit metadata: they select no bundle and no owner.
-- The selected bundle stays in the active bundle table (V93); the routing row
-- holds only the state.

CREATE TABLE pipeline_tenant_routing (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    routing_state TEXT NOT NULL CHECK (
        routing_state IN ('legacy', 'pipeline', 'contained')
    ),
    activation_record_id UUID NOT NULL,
    actor_principal_ref TEXT NOT NULL CHECK (
        actor_principal_ref ~ '^[A-Za-z0-9_:.-]{1,160}$'
    ),
    reason_code TEXT NOT NULL CHECK (reason_code ~ '^[a-z0-9_]{1,64}$'),
    evidence_hash TEXT NOT NULL CHECK (evidence_hash ~ '^sha256:[0-9a-f]{64}$'),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id)
);

CREATE TABLE pipeline_activation_events (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    event_id UUID NOT NULL,
    action TEXT NOT NULL CHECK (
        action IN ('activate', 'rollback', 'contain', 'deactivate')
    ),
    previous_state TEXT NOT NULL CHECK (
        previous_state IN ('unselected', 'legacy', 'pipeline', 'contained')
    ),
    resulting_state TEXT NOT NULL CHECK (
        resulting_state IN ('legacy', 'pipeline', 'contained')
    ),
    previous_bundle_id TEXT CHECK (
        previous_bundle_id IS NULL OR previous_bundle_id ~ '^sha256:[0-9a-f]{64}$'
    ),
    resulting_bundle_id TEXT CHECK (
        resulting_bundle_id IS NULL OR resulting_bundle_id ~ '^sha256:[0-9a-f]{64}$'
    ),
    actor_principal_ref TEXT NOT NULL CHECK (
        actor_principal_ref ~ '^[A-Za-z0-9_:.-]{1,160}$'
    ),
    reason_code TEXT NOT NULL CHECK (reason_code ~ '^[a-z0-9_]{1,64}$'),
    evidence_hash TEXT NOT NULL CHECK (evidence_hash ~ '^sha256:[0-9a-f]{64}$'),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, event_id)
);

CREATE INDEX idx_pipeline_activation_events_recorded
    ON pipeline_activation_events (tenant_id, recorded_at DESC, event_id);

-- One permanent owner for each submission id. A pipeline row is committed
-- with its run; a legacy row is claimed before the legacy path's first
-- write, so no legacy submission row exists yet and there is no foreign key
-- to trace_submissions. The run foreign key is deferred because the receipt
-- transaction inserts this row before the run.
CREATE TABLE pipeline_receipt_ownership (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    submission_id UUID NOT NULL,
    owner TEXT NOT NULL CHECK (owner IN ('legacy', 'pipeline')),
    run_id UUID,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, submission_id),
    CHECK (
        (owner = 'pipeline' AND run_id IS NOT NULL)
        OR (owner = 'legacy' AND run_id IS NULL)
    ),
    FOREIGN KEY (tenant_id, run_id)
        REFERENCES pipeline_runs (tenant_id, run_id)
        ON DELETE CASCADE
        DEFERRABLE INITIALLY DEFERRED
);

-- Immutable, the V107 shape: no UPDATE and no direct DELETE; a DELETE that
-- arrives through a cascade (the tenant was deleted) is let through.
CREATE FUNCTION reject_pipeline_activation_record_mutation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF TG_OP = 'DELETE' AND pg_trigger_depth() > 1 THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION 'pipeline activation records are immutable';
END;
$$;

CREATE TRIGGER pipeline_activation_events_reject_update
    BEFORE UPDATE ON pipeline_activation_events
    FOR EACH ROW EXECUTE FUNCTION reject_pipeline_activation_record_mutation();
CREATE TRIGGER pipeline_activation_events_reject_delete
    BEFORE DELETE ON pipeline_activation_events
    FOR EACH ROW EXECUTE FUNCTION reject_pipeline_activation_record_mutation();
CREATE TRIGGER pipeline_receipt_ownership_reject_update
    BEFORE UPDATE ON pipeline_receipt_ownership
    FOR EACH ROW EXECUTE FUNCTION reject_pipeline_activation_record_mutation();
CREATE TRIGGER pipeline_receipt_ownership_reject_delete
    BEFORE DELETE ON pipeline_receipt_ownership
    FOR EACH ROW EXECUTE FUNCTION reject_pipeline_activation_record_mutation();

ALTER TABLE pipeline_tenant_routing ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline_tenant_routing FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON pipeline_tenant_routing;
CREATE POLICY trace_corpus_tenant_isolation ON pipeline_tenant_routing
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE pipeline_activation_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline_activation_events FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON pipeline_activation_events;
CREATE POLICY trace_corpus_tenant_isolation ON pipeline_activation_events
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE pipeline_receipt_ownership ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline_receipt_ownership FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON pipeline_receipt_ownership;
CREATE POLICY trace_corpus_tenant_isolation ON pipeline_receipt_ownership
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_ingest_runtime') THEN
        RAISE EXCEPTION 'V110: trace_ingest_runtime is missing; V90 creates it';
    END IF;
END $$;

-- pipeline_tenant_routing: each upload reads it; an operator action writes
-- it through an admin route (insert, or update of every column but the key).
GRANT SELECT, INSERT ON pipeline_tenant_routing TO trace_ingest_runtime;
GRANT UPDATE (routing_state, activation_record_id, actor_principal_ref,
              reason_code, evidence_hash, recorded_at)
    ON pipeline_tenant_routing TO trace_ingest_runtime;

-- pipeline_activation_events and pipeline_receipt_ownership: append-only.
GRANT SELECT, INSERT ON pipeline_activation_events TO trace_ingest_runtime;
GRANT SELECT, INSERT ON pipeline_receipt_ownership TO trace_ingest_runtime;
```

  Before the commit, confirm in the base tree that `pipeline_runs` has a unique key on `(tenant_id, run_id)` (V92), which the ownership foreign key needs.

- [ ] **Step 4: Register it.** Add `(110, "versioned_pipeline_activation", include_str!("../../../../migrations/V110__versioned_pipeline_activation.sql"))` after V108's entry, with a comment in the style of V108's. Add the three names to `TRACE_COMMONS_RLS_TABLES` after `pipeline_attempt_artifacts`. Add the include to `trace_corpus_pg_rls.rs` where V108's is. Add the grant pins to `pipeline_upgrade_tests.rs`: `("pipeline_tenant_routing", "SELECT", &[])`, `("pipeline_tenant_routing", "INSERT", &[])`, `("pipeline_tenant_routing", "UPDATE", &["routing_state", "activation_record_id", "actor_principal_ref", "reason_code", "evidence_hash", "recorded_at"])`, and `SELECT` and `INSERT` for the two append-only tables.

- [ ] **Step 5: Consumer sweep (review rule 1).** The three tables are new: `git grep -n "pipeline_tenant_routing\|pipeline_activation_events\|pipeline_receipt_ownership" -- crates scripts` finds only this task's lines. Run `python3 scripts/operator/pipeline-deployment-inventory.py`: it reports three tables without a class until Task 13 classifies them. If a check that runs before Task 13 fails on that (for example `pipeline.py test --check contracts`), classify the three tables in this task and write that in the ledger.

- [ ] **Step 6: Run the tests.** Fresh databases `admission_test_pr5_schema` and `pipeline_test_pr5_upgrade`. `cargo test -p trace-commons-server --lib v110_defines the_isolation_control_covers_every_pipeline_table`; `TRACE_COMMONS_PIPELINE_PG_UPGRADE_TEST_URL=postgres://trace@127.0.0.1:55432/pipeline_test_pr5_upgrade cargo test -p trace-commons-server --lib pipeline_upgrade_from_v91_installs_forced_rls_storage -- --ignored`; `TRACE_COMMONS_PG_TEST_DATABASE_URL=postgres://trace@127.0.0.1:55432/admission_test_pr5_schema cargo test -p trace-commons-server --test migration_atomicity_pg`. Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add migrations/V110__versioned_pipeline_activation.sql crates/trace-commons-server/src/db crates/trace-commons-server/tests/trace_corpus_pg_rls.rs crates/trace-commons-server/src/versioned_pipeline_product.rs
git commit -m "Add the routing, activation event, and receipt ownership tables" -m "Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: Routing store, containment, and deactivation (Group A)

**Files:**
- Create: `crates/trace-commons-server/src/versioned_pipeline_activation.rs`
- Modify: `crates/trace-commons-server/src/lib.rs` (add `pub mod versioned_pipeline_activation;` beside the other `versioned_pipeline*` modules)
- Modify: `crates/trace-commons-server/tests/versioned_pipeline_runtime_pg.rs`

**Interfaces:**
- Consumes: Task 1's tables; `PgBackend::trace_pool`; `crate::versioned_pipeline::sha256_prefixed`, and `pipeline_routing_lock`, which this task adds to `versioned_pipeline.rs` and Task 3 also uses; `crate::versioned_pipeline_qualification::is_safe_label`; `PipelineOperationalSummary`.
- Produces: every type of the `versioned_pipeline_activation.rs` interface block except `ActivationRequest`, `activate_tenant`, `rollback_bundle` (Task 9), `LegacyDrainReport`, and `legacy_drain_report` (Task 6).

- [ ] **Step 1: Write the failing unit tests** in the module's `#[cfg(test)] mod tests`:
  - `the_route_of_a_new_receipt_follows_the_row_then_the_scope`: `decide_new_receipt_route` for every input. `(Some(Pipeline), true, _)` is `Pipeline`; `(Some(Pipeline), false, _)` is `NotServed`; `(Some(Contained), _, _)` is `Contained`; `(Some(Legacy), _, _)` is `Legacy`; `(None, true, true)` is `Pipeline`; `(None, true, false)`, `(None, false, true)`, and `(None, false, false)` are `Legacy`.
  - `stale_or_failed_readiness_blocks_activation`: port lines 1383 to 1428 without the `drills_ready` and `corpus_evidence_current` cases and without the last four lines; a readiness whose `evidence_hash` was changed fails; one evaluated 16 minutes ago fails; one evaluated in the future fails; each of `readiness_ok = false`, `error_count = 1`, `max_work_age_seconds = 301`, `credit_reconciled = false`, `index_consistent = false`, `invalidation_clear = false` (with the hash recomputed) fails with `activation_readiness_failed`.
  - `readiness_ignores_completed_and_terminal_history_age`: port lines 1430 to 1473, with `main`'s `PipelineOperationalSummary` fields.
  - `an_actor_and_a_reason_are_validated`: `contain` with a reason `Bad Reason` and with an empty actor returns `Err` with `activation_actor_invalid` before any database call (call the private `validate_actor` directly).

- [ ] **Step 2: Write the failing database tests** in `versioned_pipeline_runtime_pg.rs` (assertion lists; `runtime_backend(4)`, a fresh tenant for each test, actor `"principal_sha256:" + 64 hex`):
  - `a_tenant_without_a_row_has_no_routing`: `routing(tenant)` is `None`; `events(tenant, 10)` is empty.
  - `containment_and_deactivation_write_the_row_and_an_event`: `contain(tenant, actor, "contain_first_rollout")` on a tenant with no row returns a routing with state `Contained`; the event is `Contain`, `previous_state: None`, `resulting_state: Contained`, both bundle ids `None`. Then `deactivate(tenant, actor, "return_to_legacy")` returns `Legacy`; the second event is `Deactivate` with `previous_state: Some(Contained)`. `events` returns the two newest first. The routing's `activation_record_id` differs between the two. A second `deactivate` on a `Legacy` tenant returns `Err` with `activation_state_invalid` and adds no event.
  - `routing_is_tenant_scoped`: tenant A contained; `routing(B)` is `None`; with the runtime role and tenant B's setting, `SELECT COUNT(*) FROM pipeline_tenant_routing` is 0.
  - `a_legacy_claim_is_permanent_and_idempotent`: `claim_legacy_receipt(tenant, id)` returns `Legacy` twice; `ownership(tenant, id)` is `Some` with `owner: Legacy`, `run_id: None`; the table holds one row for the id.
  - `the_runtime_role_cannot_change_an_event_or_an_ownership_row`: through `backend.trace_pool_for_test()` with the tenant set, an `UPDATE` and a `DELETE` of the event row and of the ownership row each fail.

- [ ] **Step 3: Run them to see them fail.** `cargo test -p trace-commons-server --lib versioned_pipeline_activation`. Expected: FAIL (the module does not exist).

- [ ] **Step 4: Implement.** Start the file with the AGPL header and this module comment:

```rust
//! Tenant routing, receipt ownership, containment, activation, and rollback.
//!
//! A committed row assigns each tenant's new receipts to the legacy path or
//! the pipeline, or stops them (containment). Each receipt has one permanent
//! owner; a retry goes to that owner whatever the routing row says. An
//! activation or a rollback selects a qualified bundle for later runs only.
//! Timestamps are audit metadata and select nothing.
```

  Port from `ef97a459:crates/trace-commons-server/src/versioned_pipeline_activation.rs`: `RoutingState` and `ReceiptOwner` with `as_db` and `from_db` (lines 64 to 117); `ActivationReadiness`, `from_operational_summary`, `activation_readiness_hash`, and `evaluate_activation_readiness` (lines 189 to 291) without `drills_ready`, `corpus_evidence_current`, and `passing` (tests build a value with a private test helper); `tenant_transaction` and `routing` (lines 320 to 347) without `selected_bundle_id`; `routing_from_row` (line 1323). Do not port lines 119 to 187 (`LegacyWriterState`, `SwitchedReceipt`), 349 to 806, 979 to 1193, or 1346 to 1352. Use `crate::versioned_pipeline::sha256_prefixed` instead of the port's private copy. New code:

```rust
pub fn decide_new_receipt_route(
    routing: Option<RoutingState>,
    on_receipts_list: bool,
    unqualified_routing_allowed: bool,
) -> NewReceiptRoute {
    match routing {
        Some(RoutingState::Pipeline) if on_receipts_list => NewReceiptRoute::Pipeline,
        Some(RoutingState::Pipeline) => NewReceiptRoute::NotServed,
        Some(RoutingState::Contained) => NewReceiptRoute::Contained,
        Some(RoutingState::Legacy) => NewReceiptRoute::Legacy,
        None if on_receipts_list && unqualified_routing_allowed => NewReceiptRoute::Pipeline,
        None => NewReceiptRoute::Legacy,
    }
}

fn validate_actor(actor_principal_ref: &str, reason_code: &str) -> Result<(), DatabaseError> {
    let actor_ok = !actor_principal_ref.is_empty()
        && actor_principal_ref.len() <= 160
        && actor_principal_ref
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b':' | b'.' | b'-'));
    if !actor_ok || !is_safe_label(reason_code) {
        return Err(DatabaseError::Constraint(ACTIVATION_ACTOR_INVALID_LABEL.to_string()));
    }
    Ok(())
}

/// Takes the tenant's routing lock exclusively for the rest of `tx`. A
/// receipt's staging and commit transactions take it shared
/// (`PipelineService`), so a routing change waits for the receipts in those
/// transactions and no later one misses it.
async fn lock_routing(tx: &Transaction<'_>, tenant_id: &str) -> Result<(), DatabaseError> {
    tx.execute(
        "SELECT pg_advisory_xact_lock(hashtextextended($1, 2))",
        &[&pipeline_routing_lock(tenant_id)],
    )
    .await?;
    Ok(())
}
```

  `write_routing_in(tx, tenant_id, resulting, action, previous_bundle_id, resulting_bundle_id, actor, reason, evidence_hash)` is the port's `write_routing` body (lines 1196 to 1300) inside the caller's transaction, after `lock_routing`: the `trace_tenants` insert, the read of the previous state (no `FOR UPDATE`; the advisory lock serializes it), the upsert of the row without a bundle column, and the event insert with the two bundle ids the caller gives. `contain` and `deactivate` open a tenant transaction, call `lock_routing`, read the active bundle (`SELECT bundle_id FROM pipeline_active_bundles WHERE tenant_id = $1`) for the event's two bundle columns (both the same value: neither action changes the bundle), refuse a `deactivate` of a tenant with no row or a `Legacy` row with `activation_state_invalid`, call `write_routing_in`, and commit. Their evidence hash is `sha256_prefixed` of `format!("trace_commons.pipeline_{action}.v1\0{tenant_id}\0{reason_code}")`, as the port's `contain_pipeline` builds it. `claim_legacy_receipt` runs `INSERT INTO pipeline_receipt_ownership (tenant_id, submission_id, owner) VALUES ($1, $2, 'legacy') ON CONFLICT (tenant_id, submission_id) DO NOTHING` and then `SELECT owner ...`, in one tenant transaction, and returns the owner it read. In `versioned_pipeline.rs` add `pub(crate) fn pipeline_routing_lock(tenant_id: &str) -> String { format!("pipeline-routing:{tenant_id}") }` beside `pipeline_receipt_lock`. Before you use seed 2 for `hashtextextended`, run `git grep -n "hashtextextended(" -- crates` and confirm that no other lock class uses it (the receipt lock uses 0; the quota and bundle registry locks use 1); if one does, take the next free seed and write it in the ledger.

- [ ] **Step 5: Run the tests.** `cargo test -p trace-commons-server --lib versioned_pipeline_activation`; fresh database `admission_test_pr5_routing`; `TRACE_COMMONS_PG_TEST_DATABASE_URL=postgres://trace@127.0.0.1:55432/admission_test_pr5_routing cargo test -p trace-commons-server --test versioned_pipeline_runtime_pg a_tenant_without_a_row containment_and_deactivation routing_is_tenant_scoped a_legacy_claim the_runtime_role_cannot_change`. Expected: PASS (4 unit tests, 5 database tests).

- [ ] **Step 6: Commit**

```bash
git add crates/trace-commons-server/src/versioned_pipeline_activation.rs crates/trace-commons-server/src/lib.rs crates/trace-commons-server/src/versioned_pipeline.rs crates/trace-commons-server/tests/versioned_pipeline_runtime_pg.rs
git commit -m "Add the pipeline routing store with containment and deactivation" -m "Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: Ownership and routing in the receipt transaction (Group A)

**Files:**
- Modify: `crates/trace-commons-server/src/versioned_pipeline.rs` (`PipelineReceiptResult`, `PipelineServiceBuilder`, `precheck_receipt` (line 7883), `stage_receipt_attempt` (7935), `commit_receipt_attempt` (8066), `insert_receipt_records` (5187), `replay_result`'s callers, the two doc notes that name PR 5 at lines 5185 and 8065, and the note at 7971)
- Modify: `crates/trace-commons-server/tests/versioned_pipeline_runtime_pg.rs` (`test_service` and `compatibility_test_service` set `with_unqualified_routing(true)`)

**Interfaces:**
- Consumes: Task 1's tables; Task 2's `RoutingState`, `pipeline_routing_lock`, `PipelineActivationStore`.
- Produces: `PipelineReceiptResult::LegacyOwned`, `PipelineReceiptResult::NotRouted(RoutingState)`, `PIPELINE_LEGACY_RECEIPT_OWNED_LABEL`, `PipelineServiceBuilder::with_unqualified_routing`, `PipelineService::unqualified_routing`.

- [ ] **Step 1: Write the failing tests** in `versioned_pipeline_runtime_pg.rs` (assertion lists). The helper `write_routing_as_operator(tenant, state)` writes the routing row through `owner_client()` with the statement of `write_routing_in` and no event; add it beside `activate_bundle_as_operator`.
  - `a_receipt_commits_its_ownership_with_its_run`: a receipt through `submit_registered` returns `Created(run)`; `PipelineActivationStore::ownership(tenant, envelope.submission_id)` is `Some` with `owner: Pipeline` and `run_id: Some(run.run_id)`; the same key and body returns `Replayed` and the table still holds one row.
  - `a_legacy_submission_is_never_given_a_pipeline_run` (Review Focus 1): (a) `claim_legacy_receipt(tenant, id)`, then a receipt for an envelope with that submission id returns `LegacyOwned`; no `pipeline_runs`, `phase_outcomes`, `trace_submissions`, or `trace_object_refs` row exists for it; `pipeline_receipt_artifacts` holds no `committed` row for the key. (b) With no ownership row, insert a `trace_submissions` row for another id through the owner connection (the columns `insert_receipt_records` writes, status `accepted`); a receipt for that id returns `LegacyOwned` (today it is the error `submission identity is already bound to another receipt`); no run exists.
  - `two_owners_cannot_commit_for_one_submission` (Review Focus 1): 20 rounds; in each round, one fresh id, and `tokio::join!` of `claim_legacy_receipt(tenant, id)` and a receipt for that id. Every round ends with exactly one ownership row. When the claim returned `Legacy`, the receipt returned `LegacyOwned` and no run exists. When the receipt returned `Created`, the claim returned `Pipeline`.
  - `a_service_without_unqualified_routing_refuses_a_tenant_with_no_row`: a service built with the flag `false` (the default) returns `NotRouted(RoutingState::Legacy)` for a tenant with no row and stores nothing; with `write_routing_as_operator(tenant, "pipeline")` the same receipt returns `Created`.
  - `a_contained_or_legacy_tenant_gets_no_new_run_and_keeps_its_replays`: tenant routed `pipeline`; receipt A is `Created`. Write `contained`: a new receipt B returns `NotRouted(Contained)` and stores nothing (no staging row, no usage row for B's key); a retry of A returns `Replayed`; a retry of A with another body returns `ContentConflict`. Write `legacy`: a new receipt returns `NotRouted(Legacy)`; a retry of A returns `Replayed`.
  - `a_receipt_in_flight_cannot_commit_after_containment` (Review Focus 2): an artifact store double, `PausingArtifactStore`, wraps the test store and blocks the write of the receipt's object on a `std::sync::mpsc` channel (the write runs on a blocking thread, between the staging transaction and the commit transaction); while it is blocked, `contain` commits; the test then releases the write; the receipt then returns `NotRouted(Contained)`; no run exists; the attempt's object and staging row are gone (`discard_receipt_attempt`).
  - `containment_waits_for_a_receipt_transaction_and_is_not_starved`: open a transaction that holds the routing lock shared (`SELECT pg_advisory_xact_lock_shared(hashtextextended($1, 2))` with `pipeline_routing_lock(tenant)` through `trace_pool_for_test`); `contain` does not return within 300 ms; commit the transaction; `contain` returns within 2 s.

- [ ] **Step 2: Run them to see them fail.** Fresh database `admission_test_pr5_receipt`. `TRACE_COMMONS_PG_TEST_DATABASE_URL=postgres://trace@127.0.0.1:55432/admission_test_pr5_receipt cargo test -p trace-commons-server --test versioned_pipeline_runtime_pg ownership_with_its_run never_given_a_pipeline_run two_owners unqualified_routing contained_or_legacy in_flight_cannot_commit containment_waits`. Expected: FAIL (the variants do not exist).

- [ ] **Step 3: Implement.**

```rust
/// Safe label of a receipt for a submission id that the legacy path owns.
pub const PIPELINE_LEGACY_RECEIPT_OWNED_LABEL: &str = "legacy_receipt_owned";

/// Why a new receipt may not start a run for `tenant_id` now, read inside
/// `tx`: the legacy path owns the submission id, or the tenant is not routed
/// to the pipeline. `None` when the receipt may go on. With `lock`, the
/// tenant's routing lock is held shared for the rest of `tx`, so a routing
/// change waits for this transaction and no later one misses the change.
async fn new_receipt_refusal_in(
    tx: &Transaction<'_>,
    tenant_id: &str,
    submission_id: Uuid,
    unqualified_routing: bool,
    lock: bool,
) -> Result<Option<PipelineReceiptResult>, DatabaseError> {
    if lock {
        tx.execute(
            "SELECT pg_advisory_xact_lock_shared(hashtextextended($1, 2))",
            &[&pipeline_routing_lock(tenant_id)],
        )
        .await?;
    }
    let legacy_owned: bool = tx
        .query_one(
            "SELECT EXISTS (
                 SELECT 1 FROM pipeline_receipt_ownership
                  WHERE tenant_id = $1 AND submission_id = $2 AND owner = 'legacy'
             ) OR EXISTS (
                 SELECT 1 FROM trace_submissions s
                  WHERE s.tenant_id = $1 AND s.submission_id = $2
                    AND NOT EXISTS (
                        SELECT 1 FROM pipeline_runs r
                         WHERE r.tenant_id = s.tenant_id
                           AND r.submission_id = s.submission_id
                    )
             )",
            &[&tenant_id, &submission_id],
        )
        .await?
        .get(0);
    if legacy_owned {
        return Ok(Some(PipelineReceiptResult::LegacyOwned));
    }
    let state = tx
        .query_opt(
            "SELECT routing_state FROM pipeline_tenant_routing WHERE tenant_id = $1",
            &[&tenant_id],
        )
        .await?
        .map(|row| RoutingState::from_db(row.get::<_, String>(0).as_str()))
        .transpose()?;
    Ok(match state {
        Some(RoutingState::Pipeline) => None,
        Some(other) => Some(PipelineReceiptResult::NotRouted(other)),
        None if unqualified_routing => None,
        None => Some(PipelineReceiptResult::NotRouted(RoutingState::Legacy)),
    })
}
```

  Make `RoutingState::from_db` `pub(crate)`. Call `new_receipt_refusal_in` in three places, each time **after** the check for an existing run of the key (a replay is answered in every routing state) and before anything else:
  1. `precheck_receipt`: with `lock: false`, after its existing-run branch. A refusal returns before the rescrub, so a contained tenant costs no classifier call.
  2. `stage_receipt_attempt`: with `lock: true`, after `receipt_key_refusal` and before the bound bundle is read. A refusal commits the transaction and returns `ReceiptStage::Refused`.
  3. `commit_receipt_attempt`: with `lock: true`, after the `existing_receipt_run` check. A refusal commits the transaction and returns `ReceiptCommit::Refused`, so `submit` discards the attempt as it does for every refusal.

  In `insert_receipt_records`, make the ownership insert the first statement, and turn the submission conflict into the same label:

```rust
    let owned = tx
        .execute(
            "INSERT INTO pipeline_receipt_ownership (tenant_id, submission_id, owner, run_id)
             VALUES ($1, $2, 'pipeline', $3)
             ON CONFLICT (tenant_id, submission_id) DO NOTHING",
            &[&run.tenant_id, &run.submission_id, &run.run_id],
        )
        .await?;
    if owned != 1 {
        return Err(DatabaseError::Constraint(
            PIPELINE_LEGACY_RECEIPT_OWNED_LABEL.to_string(),
        ));
    }
```

  and replace the `inserted != 1` error text of the `trace_submissions` insert with the same `PIPELINE_LEGACY_RECEIPT_OWNED_LABEL`. In `commit_receipt_attempt`, map a `DatabaseError::Constraint` with that label from `insert_receipt_records` to `ReceiptCommit::Refused(PipelineReceiptResult::LegacyOwned)` after the transaction is dropped (it rolls back, so the deferred run foreign key is never checked). Add `unqualified_routing: bool` to `PipelineServiceBuilder` (default `false`) and to `PipelineService`. Update every `match` on `PipelineReceiptResult` that the compiler reports; in `versioned_pipeline.rs` itself the two new variants are never built by `replay_result`. Update the doc comments of `insert_receipt_records`, `commit_receipt_attempt`, and `stage_receipt_attempt` (the three notes that name PR 5) to say what they now do.

- [ ] **Step 4: Sibling sites and the efficiency lens.** `git grep -n "PipelineReceiptResult::" -- crates` lists every match; each one handles the two new variants explicitly (no `_ =>` arm that hides them). The receipt now runs one more query in each of its three transactions and takes one shared advisory lock in two of them: both are single-row, primary-key reads. Write both facts in the ledger.

- [ ] **Step 5: Run the tests.** The Step 2 command, then `cargo test -p trace-commons-server --test versioned_pipeline_runtime_pg receipt_replay_and_conflict_are_exact replay_receipt_reads_without_writing activation_does_not_rebind_an_existing_run concurrent` on the same database. Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/trace-commons-server/src/versioned_pipeline.rs crates/trace-commons-server/src/versioned_pipeline_activation.rs crates/trace-commons-server/tests/versioned_pipeline_runtime_pg.rs
git commit -m "Commit receipt ownership and check routing in the receipt transaction" -m "Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: Upload routing in ingest (Group A)

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` (`AppState`, `from_env_with_pipeline_runtime_assembler` at about lines 3903 to 3982 and 4477, `pipeline_owned_submission_receipt` (14259), `route_pipeline_receipt` (14336), its call in `submit_trace_handler` (14724))
- Modify: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_runtime.rs` (`IngestPipelineRuntimeContext`, `assemble_ingest_pipeline_runtime`)
- Modify: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_http_pg_tests.rs`, `tests.rs` (the test assemblers pass the flag; the test state sets `pipeline_unqualified_routing`)

**Interfaces:**
- Consumes: Tasks 2 and 3.
- Produces: `AppState::pipeline_activation`, `AppState::pipeline_unqualified_routing`, `IngestPipelineRuntimeContext::unqualified_routing_allowed`, the refusal `pipeline_runtime_unqualified_routing_mismatch`, and the HTTP answers `503 pipeline_receipt_intake_contained`, `503 pipeline_tenant_not_served`, `503 pipeline_routing_unavailable`.

- [ ] **Step 1: Run the consumer sweep again.** In the worktree, run `git grep -n "PipelineReceipts\|pipeline_drain_tenant_ids\|pipeline_service\|pipeline_store\|pipeline_product\|submission_has_pipeline_run" -- crates/trace-commons-server/src/bin`. Compare with the table in "`main` integration". Write each new or moved site in the ledger with its row of the table, or with a new row. A new site that decides between the legacy path and the pipeline, and that this plan does not name, is a defect in the plan: stop and ask.

- [ ] **Step 2: Write the failing tests** in `pipeline_http_pg_tests.rs` (real HTTP through `serve_pipeline_app`, production auth with test credentials, the minimal test bundle, a tenant on the receipts list; assertion lists):
  - `mixed_receipts_replay_to_their_first_owner_across_every_switch` (Review Focus 1): (1) Write the row `legacy` (`PipelineActivationStore::deactivate` is refused for a tenant with no row, so use `write_routing_as_operator` copied into this module as `pub(super)`). Upload L1: `200`, a legacy receipt (its `status` is not `processing`); no `pipeline_runs` row; an ownership row with owner `legacy`. (2) Write `pipeline`. Upload P1: `200`, `status: "processing"`; one run; ownership `pipeline`. (3) Retry L1 with the same body: the same legacy receipt as in (1); still no run for L1. (4) Retry P1 with the same body: `processing`; still one run. Retry P1 with another body: `409` `receipt id reused with different content`. (5) Write `legacy` again. Retry P1: `processing` (the pipeline still answers its own receipt). Upload L2: a legacy receipt; no run. (6) Write `pipeline`. Retry L2: the legacy receipt; no run. (7) At the end: `SELECT owner, COUNT(*) FROM pipeline_receipt_ownership` gives `legacy` 2 and `pipeline` 1; `trace_credit_ledger` has no row with a `pipeline_run_id` for L1 or L2.
  - `a_remediation_of_a_legacy_quarantine_stays_on_the_legacy_path` (Review Focus 1): row `legacy`; upload a submission that the legacy path quarantines (the residual-risk fixture that `main`'s quarantine tests use); write `pipeline`; the same principal uploads the same submission id with a changed body. The answer is `main`'s remediation answer, not a `500` and not `processing`; no run exists for the id; the legacy record holds the new body's redaction hash.
  - `containment_refuses_new_receipts_and_keeps_pending_work` (Review Focus 2; it emits the check of Task 12): two `AppState` values on one database (two replicas), both with the tenant on the receipts list, each served as a plain router (`app(state)`, so no worker runs yet). (1) Upload P1 through replica 1: `processing`; its run is pending. (2) `contain` through the store. (3) Upload P2 through replica 1 and P3 through replica 2: each is `503` `pipeline_receipt_intake_contained`; no run, no legacy record, no `trace_submissions` row, and no ownership row exists for P2 or P3; the admission attempt of each is finished as failed (a retry of P2 after step 6 is accepted). (4) Retry P1 through replica 2: `processing`. (5) Start the worker (`serve_pipeline_app` on replica 1's state): `wait_for_run_complete` for P1 succeeds while the routing row is still `contained`. (6) Write `pipeline` again: P2 is accepted. Then call `PipelineCheckEmitter::emit_pass_from_env("pipeline_activation_containment", None, json!({"refused_new_receipts": 2, "replayed": 1, "completed_while_contained": 1, "legacy_records_for_refused": 0}))`.
  - `an_activated_tenant_off_the_receipts_list_is_refused_not_sent_to_legacy`: an app whose receipts list does not hold the tenant, with a runtime; write `pipeline` for the tenant; an upload is `503` `pipeline_tenant_not_served`; no legacy record exists.
  - `a_tenant_with_no_row_stays_on_the_legacy_path_without_the_test_flag`: an app with `pipeline_unqualified_routing: false`, the tenant on the receipts list, no row: an upload gets a legacy receipt and no run. (This is "production routing stays off".)
  - `a_withdrawal_of_a_pipeline_submission_works_in_every_routing_state`: for each of `pipeline`, `contained`, `legacy`: a completed pipeline run, the row written, then `POST /v1/contributors/me/pipeline-submissions/{id}/withdraw` answers as `pipeline_withdrawal_route_withdraws_through_the_account_session` expects, and the index invalidation is queued.
  - In `tests.rs`, unit: `the_assembly_refuses_a_service_with_another_unqualified_routing_flag`: an assembler that ignores `context.unqualified_routing_allowed` is refused with `pipeline_runtime_unqualified_routing_mismatch`.

- [ ] **Step 3: Run them to see them fail.** Fresh database `admission_test_pr5_http`, and the login resolver URL of `CLAUDE.md` on port 55432 (create the resolver login on this server if it is missing; it is a server-wide role, so ask the owner first if a role of that name exists with other settings). `TRACE_COMMONS_PG_TEST_DATABASE_URL=postgres://trace@127.0.0.1:55432/admission_test_pr5_http TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL=postgres://tc_login_resolver_login@127.0.0.1:55432/admission_test_pr5_http cargo test -p trace-commons-server --bin trace-commons-ingest pipeline_http_pg_tests -- --test-threads=1`. Expected: the new tests FAIL.

- [ ] **Step 4: Implement.**
  - `AppState`: `pipeline_activation` is `Some(Arc::new(PipelineActivationStore::new(connections.postgres.clone())))` exactly when `pipeline_store` is `Some`. `pipeline_unqualified_routing` is `pipeline_allow_test_dependencies`.
  - `IngestPipelineRuntimeContext::unqualified_routing_allowed` is the same value. `assemble_ingest_pipeline_runtime` takes it as a parameter and refuses a service whose `unqualified_routing()` differs, after the lease-config check and in its shape:

```rust
    // P5-D5: only a process started for tests routes a tenant that has no
    // routing row, and its service must agree, because the receipt
    // transaction checks the routing again.
    anyhow::ensure!(
        service.unqualified_routing() == unqualified_routing_allowed,
        "pipeline_runtime_unqualified_routing_mismatch"
    );
```

  - Replace the first lines of `route_pipeline_receipt` (it gains the parameter `remediating: bool`, and its doc comment is rewritten for the new rule):

```rust
    let in_scope = pipeline_runtime_for_replay(state, tenant).is_some();
    // A remediation rewrites a legacy record that this handler has in hand:
    // the legacy path owns the id, whatever the routing row says.
    if remediating {
        return claim_legacy_receipt(state, tenant, envelope.submission_id, raw_body, in_scope).await;
    }
    let (Some(pipeline_service), Some(activation)) =
        (state.pipeline_service.as_ref(), state.pipeline_activation.as_ref())
    else {
        return Ok(None);
    };
    let routing = activation
        .routing(tenant.tenant_id())
        .await
        .map_err(|_| api_error(StatusCode::SERVICE_UNAVAILABLE, PIPELINE_ROUTING_UNAVAILABLE_LABEL))?;
    let has_row = routing.is_some();
    match decide_new_receipt_route(
        routing.map(|row| row.routing_state),
        pipeline_runtime_for_tenant(state, tenant).is_some(),
        state.pipeline_unqualified_routing,
    ) {
        NewReceiptRoute::Legacy => {
            return claim_legacy_receipt(state, tenant, envelope.submission_id, raw_body, in_scope || has_row).await;
        }
        NewReceiptRoute::Contained => {
            return Err(api_error(StatusCode::SERVICE_UNAVAILABLE, PIPELINE_RECEIPT_INTAKE_CONTAINED_LABEL));
        }
        NewReceiptRoute::NotServed => {
            return Err(api_error(StatusCode::SERVICE_UNAVAILABLE, PIPELINE_TENANT_NOT_SERVED_LABEL));
        }
        NewReceiptRoute::Pipeline => {}
    }
```

  and add the arms of the `submit` result: `PipelineReceiptResult::LegacyOwned` returns `Ok(None)` (the legacy path goes on; its ownership row or its record exists); `NotRouted(RoutingState::Contained)` is the contained `503`; `NotRouted(RoutingState::Legacy)` calls `claim_legacy_receipt(..., true)`; `NotRouted(RoutingState::Pipeline)` is `internal_error("pipeline_routing_result_unexpected")`. In `pipeline_owned_submission_receipt`, add both new variants to the arm that answers `pipeline_replay_result_unexpected`. `claim_legacy_receipt` is new:

```rust
/// Claims `submission_id` for the legacy path before its first write, when
/// `claim` (the tenant is in this process's pipeline scope or has a routing
/// row). `Ok(None)`: the legacy path goes on. A pipeline run that owns the id
/// is answered as `pipeline_owned_submission_receipt` answers it.
async fn claim_legacy_receipt(
    state: &AppState,
    tenant: &TenantCtx,
    submission_id: Uuid,
    raw_body: &[u8],
    claim: bool,
) -> ApiResult<Option<TraceSubmissionReceipt>> {
    let (true, Some(activation)) = (claim, state.pipeline_activation.as_ref()) else {
        return Ok(None);
    };
    match activation
        .claim_legacy_receipt(tenant.tenant_id(), submission_id)
        .await
        .map_err(internal_error)?
    {
        ReceiptOwner::Legacy => Ok(None),
        ReceiptOwner::Pipeline => pipeline_owned_submission_receipt(
            state,
            tenant,
            submission_id,
            raw_body,
            "submission id already belongs to another principal",
        )
        .await?
        .map(Some)
        .ok_or_else(|| api_error(StatusCode::CONFLICT, SUBMISSION_OWNED_BY_PIPELINE_RUN)),
    }
}
```

  At the call site (line 14724) pass `remediating_prior.is_some()`. The stock binary has no `pipeline_service`, so it makes no routing read and no claim.
  - Update the comment at line 14717 ("Tenant rollout gate (D3, D15)") and the doc of `pipeline_runtime_for_tenant`: the receipts list is the scope; the routing row decides.

- [ ] **Step 5: The efficiency lens.** With a runtime injected, each new upload adds one primary-key read (`routing`), and each legacy upload of a tenant in scope adds one insert and one read in one transaction (`claim_legacy_receipt`). A replay adds nothing: both checks run before the route. Write this in the ledger and in the runbook text of Task 13.

- [ ] **Step 6: Run the tests.** The Step 3 command. Expected: PASS, with every PR 2 to PR 4 test of the module unchanged except the assemblers and the test state that now pass the flag.

- [ ] **Step 7: Commit**

```bash
git add crates/trace-commons-server/src/bin
git commit -m "Route uploads by the committed routing row" -m "Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 5: Policy interventions and their guards (V111) (Group A)

**Files:**
- Create: `migrations/V111__versioned_pipeline_policy_interventions.sql`
- Modify: `crates/trace-commons-server/src/db/postgres.rs`, `db/postgres/pipeline_upgrade_tests.rs`, `tests/trace_corpus_pg_rls.rs` (as Task 1 Step 4, for V111 and `pipeline_policy_interventions`)
- Modify: `crates/trace-commons-server/src/versioned_pipeline.rs` (the store and service methods; `commit_review` (1844), `commit_score_in` (2568), `commit_settle` (3945), `commit_receipt_attempt` (8066), `process_payouts` (11657))
- Modify: `crates/trace-commons-server/src/versioned_pipeline_product.rs` (`PipelineOperationalSummary`, the RLS name)
- Modify: `crates/trace-commons-server/tests/versioned_pipeline_runtime_pg.rs`

**Interfaces:**
- Consumes: V93's `pipeline_bundle_policy_status`; `PIPELINE_POLICY_NOT_RUNNABLE_LABEL`; `mark_transient_retry` (FR3: an uncharged retry with backoff).
- Produces: `PolicyOperationalStatus`, `PipelinePolicyInterventionRecord`, `intervene_policy`, `list_policy_interventions`, `PIPELINE_POLICY_INTERVENTION_NOT_SUPPORTED_LABEL`, `PipelineOperationalSummary::suspended_policy_count`.

- [ ] **Step 1: Write the failing tests** in `versioned_pipeline_runtime_pg.rs` (assertion lists):
  - `a_suspended_policy_leaves_the_run_retryable_and_resumes_under_the_same_bundle` (SCN-008): a run that completed Review; `intervene_policy(tenant, bundle, Phase::Score, "suspend", actor, "unsafe_bound_policy")` returns a record with `previous_status: Runnable`, `resulting_status: Suspended`; `process_one` leaves the run in `Retry` with `last_error_label == "bundle_policy_not_runnable"` and the same `attempt_count` as before the claim; the run's `bundle_id` is unchanged; `resume` returns `Suspended` to `Runnable`; the run completes under the same bundle with one Score outcome. `list_policy_interventions` returns the two records, oldest first.
  - `a_policy_suspended_during_a_phase_cannot_commit_it` (Review Focus 4): the slow scorer double pauses Score; while it is paused, suspend the Score policy; the Score commit is refused; no Score outcome exists; the run is in `Retry`, uncharged; the staged Score objects are swept as a refused attempt's are. Repeat for Review (suspend between the review assessment and its commit, through the store's `commit_review`) and for Settle (suspend after the selection is stored and before `commit_settle`): no outcome of that phase exists while the policy is suspended, and the run completes after `resume` with one outcome for each phase and one credit event.
  - `a_suspension_waits_for_a_commit_that_holds_the_policy_row`: a transaction through `trace_pool_for_test` holds `SELECT runnable FROM pipeline_bundle_policy_status ... FOR SHARE` for the Score row; `intervene_policy(... "suspend" ...)` does not return within 300 ms; commit; it returns within 2 s.
  - `a_payout_waits_for_a_suspended_settle_policy` (Review Focus 4; GRD-004): a complete run with a payable Trace Credit leg and the recording payout adapter of the payout tests; suspend the Settle policy; `process_payouts(tenant, 32)` submits nothing (the adapter saw no call) and changes no payout row; `resume`; `process_payouts` submits once.
  - `an_admission_policy_suspended_before_the_receipt_commit_refuses_the_receipt`: suspend Admission between the staging transaction and the commit transaction (Task 3's `PausingArtifactStore`): the receipt fails with `bundle_policy_not_runnable` and stores no run.
  - `terminate_is_refused_and_an_unknown_action_is_refused`: `"terminate"` returns `Err` with `policy_intervention_not_supported`; `"pause"` returns `Err` with `policy_intervention_invalid`; `"resume"` on a runnable policy returns `Err` with `policy_intervention_no_transition`; an intervention on a bundle the tenant does not have is `NotFound`; none of them adds a record.
  - `the_summary_counts_suspended_policies`: with one Score policy suspended, `operational_summary(tenant).suspended_policy_count == 1`; after `resume` it is 0.
  - In `db/postgres.rs`: `v111_defines_policy_interventions` (the table, the two triggers, forced RLS, the policy, `ADD COLUMN operational_status`, the check `runnable = (operational_status = 'runnable')`, the grants of Step 3).

- [ ] **Step 2: Run them to see them fail.** Fresh database `admission_test_pr5_policy`. `TRACE_COMMONS_PG_TEST_DATABASE_URL=postgres://trace@127.0.0.1:55432/admission_test_pr5_policy cargo test -p trace-commons-server --test versioned_pipeline_runtime_pg suspended_policy suspended_during suspension_waits payout_waits admission_policy_suspended terminate_is_refused summary_counts_suspended`. Expected: FAIL.

- [ ] **Step 3: Write the migration.**

```sql
-- Operator interventions on a bound policy (delivery PR 5). `runnable` (V93)
-- stays the column every reader uses; `operational_status` says why it is
-- false. V93 gave the runtime no UPDATE here and nothing wrote `runnable`,
-- so every existing row is runnable and satisfies the new check.

ALTER TABLE pipeline_bundle_policy_status
    ADD COLUMN operational_status TEXT NOT NULL DEFAULT 'runnable'
        CHECK (operational_status IN ('runnable', 'suspended', 'terminated')),
    ADD CONSTRAINT pipeline_bundle_policy_status_runnable_shape CHECK (
        runnable = (operational_status = 'runnable')
    );

CREATE TABLE pipeline_policy_interventions (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    intervention_id UUID NOT NULL,
    bundle_id TEXT NOT NULL CHECK (bundle_id ~ '^sha256:[0-9a-f]{64}$'),
    phase TEXT NOT NULL CHECK (phase IN ('admission', 'review', 'score', 'settle')),
    action TEXT NOT NULL CHECK (action IN ('suspend', 'resume', 'terminate')),
    actor_principal_ref TEXT NOT NULL CHECK (
        actor_principal_ref ~ '^[A-Za-z0-9_:.-]{1,160}$'
    ),
    reason_code TEXT NOT NULL CHECK (reason_code ~ '^[a-z0-9_]{1,64}$'),
    previous_status TEXT NOT NULL CHECK (
        previous_status IN ('runnable', 'suspended', 'terminated')
    ),
    resulting_status TEXT NOT NULL CHECK (
        resulting_status IN ('runnable', 'suspended', 'terminated')
    ),
    evidence_hash TEXT NOT NULL CHECK (evidence_hash ~ '^sha256:[0-9a-f]{64}$'),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, intervention_id),
    FOREIGN KEY (tenant_id, bundle_id, phase)
        REFERENCES pipeline_bundle_policy_status (tenant_id, bundle_id, phase)
        ON DELETE CASCADE
);

CREATE INDEX idx_pipeline_policy_interventions_bundle
    ON pipeline_policy_interventions (tenant_id, bundle_id, phase, recorded_at DESC);

CREATE FUNCTION reject_pipeline_policy_intervention_mutation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF TG_OP = 'DELETE' AND pg_trigger_depth() > 1 THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION 'pipeline policy interventions are immutable';
END;
$$;

CREATE TRIGGER pipeline_policy_interventions_reject_update
    BEFORE UPDATE ON pipeline_policy_interventions
    FOR EACH ROW EXECUTE FUNCTION reject_pipeline_policy_intervention_mutation();
CREATE TRIGGER pipeline_policy_interventions_reject_delete
    BEFORE DELETE ON pipeline_policy_interventions
    FOR EACH ROW EXECUTE FUNCTION reject_pipeline_policy_intervention_mutation();

ALTER TABLE pipeline_policy_interventions ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline_policy_interventions FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON pipeline_policy_interventions;
CREATE POLICY trace_corpus_tenant_isolation ON pipeline_policy_interventions
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_ingest_runtime') THEN
        RAISE EXCEPTION 'V111: trace_ingest_runtime is missing; V90 creates it';
    END IF;
END $$;

-- An intervention updates the status row (and a phase commit locks it
-- FOR SHARE, which needs UPDATE on a column) and appends its record.
GRANT UPDATE (runnable, operational_status, error_label, updated_at)
    ON pipeline_bundle_policy_status TO trace_ingest_runtime;
GRANT SELECT, INSERT ON pipeline_policy_interventions TO trace_ingest_runtime;
```

- [ ] **Step 4: Implement.** Port `PolicyOperationalStatus` and `PipelinePolicyInterventionRecord` (port `versioned_pipeline.rs` lines 239 to 281, the record without `tenant_id`), `intervene_policy` and `list_policy_interventions` (lines 678 to 829) and the service forwards (lines 3834 to 3870), with these changes: validate the actor with the rule of Task 2's `validate_actor` (move that function to `versioned_pipeline.rs` as `pub(crate)` and use it from both modules) and the reason with `is_safe_label`; the `UPDATE` sets `runnable`, `operational_status`, `error_label` (the reason code when the result is not runnable, else `NULL`), and `updated_at` (V93 has no `updated_by_principal_ref` and no `reason_code` column, and V111 adds none); `"terminate"` returns `Constraint(PIPELINE_POLICY_INTERVENTION_NOT_SUPPORTED_LABEL)`; another action returns `Constraint("policy_intervention_invalid")`; an invalid transition returns `Constraint("policy_intervention_no_transition")`. Add the commit guard:

```rust
/// Whether the policy of `phase` in `bundle_id` is runnable, with its status
/// row locked `FOR SHARE` for the rest of `tx`: an intervention waits for
/// this commit, and a commit that starts after a suspension is refused
/// (GRD-004).
async fn lock_runnable_policy(
    tx: &Transaction<'_>,
    tenant_id: &str,
    bundle_id: &str,
    phase: Phase,
) -> Result<bool, DatabaseError> {
    Ok(tx
        .query_opt(
            "SELECT runnable FROM pipeline_bundle_policy_status
              WHERE tenant_id = $1 AND bundle_id = $2 AND phase = $3
              FOR SHARE",
            &[&tenant_id, &bundle_id, &phase_as_db(Some(phase))],
        )
        .await?
        .is_some_and(|row| row.get::<_, bool>("runnable")))
}
```

  Call it in each phase commit, in the transaction that inserts the outcome, after that commit's existing lease and submission guards and before its first write: `commit_review` (`Phase::Review`), `commit_score_in` (`Phase::Score`), `commit_settle` (`Phase::Settle`), and `commit_receipt_attempt` (`Phase::Admission`, with the bundle the run is bound to). A commit that finds the policy not runnable makes no write and is refused the way that commit already refuses a guard that fails at commit time (P3-D9), with the label `PIPELINE_POLICY_NOT_RUNNABLE_LABEL`; the caller treats that label as `process_claimed_run` treats it at load (line 9084): `mark_transient_retry`, uncharged. For the receipt, the refusal is the error `bundle_policy_not_runnable` that the staging transaction already returns for a policy that is not runnable (line 7992). In `process_payouts`, before each run's dispatch, call `self.store.policy_is_runnable(tenant_id, &run.bundle_id, Phase::Settle)`; when it is false, skip that run for this pass (no state change, no charge), and do not count it as processed. Add `suspended_policy_count` to `PipelineOperationalSummary`: `SELECT COUNT(*) FROM pipeline_bundle_policy_status WHERE tenant_id = $1 AND NOT runnable`.

- [ ] **Step 5: Sibling sites (review rule 2).** `git grep -n "INSERT INTO phase_outcomes\|fn insert_outcome" -- crates/trace-commons-server/src` lists every place that writes an outcome. Each one is inside a transaction that called `lock_runnable_policy`, or it is written in the ledger with the reason that it needs no guard. `git grep -n "struct PipelineOperationalSummary" -A 20` consumers: the admin summary route serializes the new field; `ActivationReadiness::from_operational_summary` (Task 2) does not read it. Lock order: a phase commit takes the run row, the submission row, then the policy row; an intervention takes only the policy row; so no cycle. Settle holds the policy row through its index dispatch (at most its lease, or 30 s), so a suspension of Settle can wait that long: write this in the ledger and in the runbook.

- [ ] **Step 6: Register V111** as Task 1 Step 4 (registry, `TRACE_COMMONS_RLS_TABLES`, `PIPELINE_TABLES` to 19 entries, the version list, the RLS suite include, the isolation test's set, the grant pins `("pipeline_bundle_policy_status", "UPDATE", &["runnable", "operational_status", "error_label", "updated_at"])` and `SELECT` and `INSERT` on the new table).

- [ ] **Step 7: Run the tests.** The Step 2 command; the upgrade test and `migration_atomicity_pg` as in Task 1 Step 6 with fresh databases; `cargo test -p trace-commons-server --test versioned_pipeline_runtime_pg crash_matrix_produces_one_logical_effect_per_point stale_lease_cannot_commit_after_reclaim payout_crash_between_submit_and_confirm_submits_once` (the guards must not change them). Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add migrations/V111__versioned_pipeline_policy_interventions.sql crates/trace-commons-server
git commit -m "Add policy suspension with commit and payout guards" -m "Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 6: Legacy drain report (Group A)

**Files:**
- Modify: `crates/trace-commons-server/src/versioned_pipeline_activation.rs` (`LegacyDrainReport`, `legacy_drain_report`)
- Create: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_activation_pg_tests.rs`
- Modify: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs` (declare the module beside `pipeline_http_pg_tests`)

**Interfaces:**
- Consumes: `main`'s tables `trace_submissions`, `trace_gate_decisions`, `trace_gate_evaluation_attempts`, `trace_object_refs`, `trace_derived_records`, `trace_vector_entries`, `trace_credit_ledger`, `trace_credit_settlement_batches`, `trace_revocation_propagation_items`, `trace_near_credit_outbox`; `pipeline_runs`; Task 2's store; Task 4's routing.
- Produces: `LegacyDrainReport`, `PipelineActivationStore::legacy_drain_report`.

- [ ] **Step 1: Read the legacy predicates.** The report must count what the legacy workers select, so each count is the worker's own predicate plus "no pipeline run owns the submission" and the tenant filter. Read each one and copy its conditions into the query; write in the ledger the file and line of each source predicate:

| Count label | Source predicate on `main` | Pending means |
| --- | --- | --- |
| `awaiting_pii_backstop` | `list_submissions_awaiting_pii_backstop` (`db/postgres.rs`, about 5273) | `trace_submissions.status = 'awaiting_pii_backstop'` |
| `gate_decision_pending` | `list_submissions_needing_gate_decision` (`db/postgres.rs`, about 5206) | its conditions, with attempts below `gate_max_attempts` |
| `gate_decision_exhausted` | the same query | the same conditions, with attempts at or above `gate_max_attempts` (the driver no longer selects it, and no decision exists) |
| `quarantine_review_pending` | `claim_trace_review_lease` (`db/trace_corpus_pg.rs`, about 2921) | `status = 'quarantined'` |
| `vector_index_pending` | `index_vector_metadata_from_db` (`trace-commons-ingest.rs`, about 74094 to 74170) | accepted, not revoked or purged, a current `duplicate_precheck` derived record, and no active vector entry at the deterministic id |
| `delayed_credit_unsettled` | `run_credit_settlement_unlocked` (`trace-commons-ingest.rs`, about 28625 to 28640; event types at about 40680) | a positive delayed-credit ledger event on an accepted submission that no finalized settlement batch names in `source_credit_event_ids` |
| `revocation_propagation_pending` | `list_due_trace_revocation_propagation_items` (`db/trace_corpus_pg.rs`, about 6383) | items with status `pending`, `in_progress`, or `failed` whose `source_submission_id` is legacy-owned |
| `near_outbox_pending` (tenant-wide) | `read_mains_near_credit_outbox_items` (`trace-commons-ingest.rs`, about 33219 to 33290) | `instrument_id IS NULL` and status `pending`, `failed`, or `submitted` |

  "Legacy-owned" is: `NOT EXISTS (SELECT 1 FROM pipeline_runs r WHERE r.tenant_id = s.tenant_id AND r.submission_id = s.submission_id)`, the predicate that `list_submissions_needing_gate_decision` already uses. `gate_max_attempts` is the value that `run_perplexity_score_driver_tick` (`trace-commons-ingest.rs`, about 44177) passes to that query. If the runtime role lacks `SELECT` on one of these tables in the grant pins, stop and ask: this plan adds no grant on a table of `main`.

- [ ] **Step 2: Write the failing rehearsal test** in `pipeline_activation_pg_tests.rs` (real HTTP, the legacy path of `submit_trace_handler`, the helpers of `pipeline_http_pg_tests.rs`; assertion list): `the_legacy_drain_report_counts_real_pending_work_and_reaches_zero` (Review Focus 5; it emits the check of Task 12).
  1. A tenant on the receipts list with the routing row `legacy`. Upload L1, which the legacy path quarantines, and L2, which it accepts. Both are legacy receipts with no run.
  2. `legacy_drain_report`: `quarantine_review_pending == 1`; `gate_decision_pending` equals the number of the two submissions that `list_submissions_needing_gate_decision` selects for this tenant (assert the same number from that function, so the two cannot drift); `drained == false`; `routing_state == Some(Legacy)`.
  3. Write the row `pipeline`. Upload P1 (a clean submission) and P2 (one that the pipeline's Admission quarantines). Both have runs. The report's numbers are the numbers of step 2: P2's `trace_submissions` row has status `quarantined` and is not counted.
  4. Retry L1 with its first body: the legacy receipt of step 1; no run for L1.
  5. Do the legacy work through the legacy code: decide L1 through `main`'s review route (the decision route that `apply_review_decision` serves, as `main`'s review tests call it), and record L2's gate decision through `POST /v1/workers/gate/evaluate` with the test gate runtime that the handler's own tests use. If that handler cannot run in this module without a model, call the store function that `evaluate_and_record_gate` calls, and record this as a ruling in the ledger with the reason.
  6. The report: every count is 0 and `drained == true`, while P1 and P2 are still pending pipeline work (the worker is paused for this test). `evidence_hash` is the canonical hash of the `pending` map.
  7. The legacy writer is still on: write the row `legacy`; upload L3: a legacy receipt; the report's `gate_decision_pending` is again above 0 and `drained == false`.
  8. Emit: `PipelineCheckEmitter::emit_pass_from_env("pipeline_legacy_drain", None, json!({"legacy_receipts": 3, "pipeline_receipts": 2, "pending_before": <the step 2 total>, "pending_after_drain": 0, "pipeline_rows_counted": 0}))`.
  Also `a_drain_report_is_tenant_scoped`: tenant B with a quarantined legacy submission does not change tenant A's report.

- [ ] **Step 3: Run it to see it fail.** Fresh database `admission_test_pr5_drain` and the resolver URL. `cargo test -p trace-commons-server --bin trace-commons-ingest pipeline_activation_pg_tests -- --test-threads=1`. Expected: FAIL (`legacy_drain_report` does not exist).

- [ ] **Step 4: Implement.** One tenant transaction, one statement for each count (seven `SELECT COUNT(*)` statements over indexed tenant predicates and one for the NEAR outbox; no row is locked and no per-row statement runs). `pending` holds every label of the table, also when its count is 0. `drained` is `pending.values().all(|count| *count == 0)`. `evidence_hash` is `evidence_hash(&json!({"schema": "trace_commons.pipeline_legacy_drain.v1", "pending": pending}))`. `routing_state` is read in the same transaction. The doc comment of `legacy_drain_report` names what is not counted and why (P5-D8): retention, export jobs, benchmark and process-evaluation work, the DB mirror backfill, and work that the legacy path does not start by itself (`/v1/workers/utility-credit` takes its submission ids from its caller).

- [ ] **Step 5: The efficiency lens.** Run `EXPLAIN` for each of the eight statements on the test database with a tenant of at least 1,000 seeded submissions (seed them through the owner connection). Each plan starts from an index on `tenant_id`. Put the eight plans' first lines in the ledger. If one is a sequential scan of a table of `main`, do not add an index in PR 5: write it down and tell the owner (a new index on a table of `main` is a decision).

- [ ] **Step 6: Run the tests.** The Step 3 command. Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/trace-commons-server/src/versioned_pipeline_activation.rs crates/trace-commons-server/src/bin/trace_commons_ingest_internal
git commit -m "Report pending legacy work from the legacy tables" -m "Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 7: Evidence that names one package (Group B; uses `evaluate_promotion`, the check lists, and the emit sites)

**Files:**
- Modify: `crates/trace-commons-server/tests/versioned_pipeline_runtime_pg.rs` (the emit sites of the mechanics checks; today at about lines 363, 727, 3777, 9866, 11373, 13745, 22874, 28479)
- Modify: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_http_pg_tests.rs` (the emit sites at about 1012 and 1857), `pipeline_corpus_pg_tests.rs` (about 1463), `pipeline_restore_pg_tests.rs` (about 1472)
- Modify: `scripts/operator/pipeline_tooling/checks.py`, `scripts/operator/test_pipeline_tooling.py`
- Modify: `crates/trace-commons-server/src/versioned_pipeline_qualification.rs` (the doc of `evaluate_promotion`, lines 411 to 444)

**Interfaces:**
- Consumes: `PipelineCheckEmitter::emit_pass_from_env(check_id, package: Option<&BundlePackage>, observed)`; `evaluate_promotion`; `checks.DatabaseCheck.digests`; `checks.required_specs`.
- Produces: the rule "a mechanics check names no package" (P5-D15) in code; one shared candidate package for the four package-bearing checks.

- [ ] **Step 1: Measure.** Run `python3 scripts/operator/pipeline.py qualify --postgres-admin-url postgres://trace@127.0.0.1:55432/postgres` at the base (estimate the time from the PR 4 ledger and tell the owner before it starts if it is longer than 30 minutes). From the run directory, list each result's `check_id` and `package_hash` (`python3 -c 'import json,glob,sys; [print(json.load(open(p))["check_id"], json.load(open(p))["package_hash"]) for p in sorted(glob.glob(sys.argv[1] + "/results/*.result.json"))]' <run dir>`). Write the table in the ledger. It is the list that Steps 3 and 4 change.

- [ ] **Step 2: Write the failing tests.**
  - Rust unit test in `versioned_pipeline_qualification.rs`, `mechanics_results_without_a_package_do_not_mix_a_promotion`: one result for each id of `PROMOTION_REQUIRED_CHECKS`; the results of `pipeline_bundle_qualification`, `pipeline_http_corpus_compatibility`, `pipeline_http_corpus_hf_local`, and `pipeline_restore_drill` carry the three digests of one package, and every other result carries none; `evaluate_promotion` returns a decision with no `qualification_evidence_mixed_package` blocker and `package == Some(that package)`. Then give the crash-matrix result another package: the blocker is present.
  - Python, in `test_pipeline_tooling.py`, `test_only_candidate_checks_require_package_digests`: `{spec.check_id for spec in checks.required_specs().values() if spec.digests_required}` equals `{"pipeline_bundle_qualification", "pipeline_http_corpus_compatibility", "pipeline_http_corpus_hf_local", "pipeline_restore_drill"}`.
  - Python, `test_a_result_set_with_two_packages_is_reported_mixed`: `results.require_one_package(results)` (new, Step 4) raises `ToolingError("qualification_evidence_mixed_package")` for two results with different `package_hash` values, and passes when the package-bearing results agree and the others carry `None`.

- [ ] **Step 3: Change the emit sites.** For each mechanics check, pass `None` as the package: `pipeline_crash_matrix`, `pipeline_independent_instruments`, `pipeline_stale_lease_fence`, `pipeline_lease_renewal`, `pipeline_receipt_replay_exact`, `pipeline_payout_recovery`, `pipeline_index_rebuild`, `pipeline_orphan_sweep`, `pipeline_http_restart_recovery`, `pipeline_http_receipt_ownership`, and the corpus harness when its `check_id` is `pipeline_http_corpus_minimal`. (`pipeline_storage_upgrade_rls` already names none.) Keep the package for `pipeline_bundle_qualification`, the compatibility and HF-local corpus checks, and the restore drill, and make all four build it from one function: move the compatibility package constructor that the corpus harness uses for `--bundle compatibility` into one `pub(super) fn qualification_candidate_package()` in `pipeline_http_pg_tests.rs` (beside `compatibility_reference_package`), and give the runtime suite's `qualification_inspects_the_objects_the_constructor_receives` the same construction (the same configuration and the same reference scorer and embedder; a suite in `tests/` cannot import the bin's module, so it repeats the three lines, and Step 5's run proves that the hashes agree).

- [ ] **Step 4: Change `checks.py`.** `_runtime(check_id, test_name, digests=False)`, with `digests=True` only for `pipeline_bundle_qualification`; the two `_INGEST_BIN` rows get `False`; `required_specs` gives `digests_required=False` for `pipeline_http_corpus_minimal` and `True` for the other two corpus checks and the restore drill. Add `require_one_package(results)` to `results.py`: the set of `(package_hash, configuration_digest, dependency_digest)` over results whose `package_hash` is not `None` has at most one member, else `ToolingError("qualification_evidence_mixed_package")`; `require_current_pass_results` calls it. A result of a check whose spec has `digests_required=False` must carry no digest (`ToolingError("pipeline_check_digests_unexpected")`), so a later check cannot name a test bundle again without a test failure.

- [ ] **Step 5: Run.** `python3 scripts/operator/test_pipeline_tooling.py`; `cargo test -p trace-commons-server --lib versioned_pipeline_qualification`; then `pipeline.py qualify` again as in Step 1. Expected: `qualify` passes, and the Step 1 listing now shows one `package_hash` on four results and `None` on the others. Put the listing in the ledger.

- [ ] **Step 6: Update the docs of the rule.** The doc of `evaluate_promotion` (the sentence "Every runtime check names its own test bundle today, so PR 5 must have the mechanics checks name no package, or run them against the candidate") becomes the rule as built. `docs/operator/pipeline-qualification.md`, lines 216 to 223, is rewritten in Task 13.

- [ ] **Step 7: Commit**

```bash
git add crates/trace-commons-server scripts/operator
git commit -m "Name one package in a qualification run" -m "Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 8: Signed check results (Group B; uses `qualify_bundle`, `evaluate_promotion`, `pipeline.py`)

**Files:**
- Modify: `crates/trace-commons-server/src/versioned_pipeline_qualification.rs`
- Modify: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_corpus_pg_tests.rs` (the corpus evidence; a new ignored test beside `pipeline_package_write`)
- Modify: `scripts/operator/pipeline.py`, `scripts/operator/pipeline_tooling/results.py`, `scripts/operator/test_pipeline_tooling.py`
- Modify: `crates/trace-commons-server/tests/versioned_pipeline_runtime_pg.rs`

**Interfaces:**
- Consumes: `PipelineCheckResult`, `evidence_hash`, `BundlePackageSignature`, `TrustedBundleKey`, `sign_bundle_package`'s Ed25519 use of `ring`, `DrillEvidence`, `evaluate_promotion`, `qualify_bundle`, `QUALIFICATION_EVIDENCE_AGE_CEILING_SECONDS`, Task 1's key.
- Produces: the attestation part of the `versioned_pipeline_qualification.rs` interface block; `qualify_bundle_attested`; `pipeline.py qualify --signing-key PATH --signing-key-id ID [--evidence-max-age-seconds N]`, `pipeline.py keygen`, and `pipeline.py revision`; the files `<check_id>.attestation.json`.

- [ ] **Step 1: Write the failing unit tests** in `versioned_pipeline_qualification.rs`:
  - `an_attestation_round_trips_through_the_check_trust_store`: a generated PKCS#8 key signs `sample_check_result()` with `maximum_age_seconds: 3_600`; `CheckResultTrustStore::new([trusted_key_for_pkcs8(id, pkcs8)?])?.verify_all(&[attestation])` returns one `DrillEvidence` with that result and age.
  - `an_unsigned_or_altered_result_cannot_qualify_a_bundle` (Review Focus 3), unit part: each of these makes `verify_all` fail: a changed `result.observed_at`; a changed `result.status`; a changed `maximum_age_seconds`; a changed or added `corpus_digest`; a signature from another key (`check_attestation_signer_untrusted`); an algorithm other than `Ed25519`; a schema string other than the constant; an unknown JSON field (it fails to deserialize). The first four fail with `check_attestation_signature_invalid`.
  - `a_package_key_is_not_a_check_key`: an attestation signed with the key of a `BundlePackageTrustStore` is refused by a `CheckResultTrustStore` built from other keys; the two store types are different types (this compiles only because the route holds both).
  - `the_corpus_and_input_digests_cover_every_attestation_that_carries_one`: for three attestations of which two carry `corpus_digest` values `a` and `b` under check ids `x` and `y`, `VerifiedEvidence::corpus_digest` equals `evidence_hash(&json!({"x": a, "y": b}))`; with none, it equals `evidence_hash(&json!({}))`.

- [ ] **Step 2: Write the failing database tests** in `versioned_pipeline_runtime_pg.rs`:
  - `an_unsigned_or_altered_result_cannot_qualify_a_bundle`, database part: with `qualified_production_service` and a full set of signed passing results for its package, `qualify_bundle_attested` records the row; its metadata's `corpus_digest` and `input_digest` are the server's values, its `code_revision_hash` is the argument, and its `evidence_hash` is `evaluate_promotion`'s. The same call with one attestation altered after signing returns `Constraint("check_attestation_signature_invalid")` and records nothing; with a check trust store of another key, `check_attestation_signer_untrusted`; with `maximum_age_seconds` above the ceiling (signed that way), `bundle_qualification_evidence_age_above_ceiling`.
  - `qualify_bundle_refuses_metadata_that_the_attestations_do_not_back`: the store API `qualify_bundle` with metadata whose `corpus_digest` differs from the attestations' value is `bundle_qualification_corpus_mismatch`; likewise `bundle_qualification_input_mismatch`. (This needs `qualify_bundle` to take the verified digests: Step 4.)

- [ ] **Step 3: Write the failing Python tests** in `test_pipeline_tooling.py`:
  - `test_qualify_with_a_signing_key_runs_the_attestation_step`: with the cargo layer replaced by the self-tests' fake (as the existing `qualify` self-tests do), `qualify --signing-key K --signing-key-id ci_key` calls `cargo_test` one more time, with the test name `tests::pipeline_corpus_pg_tests::pipeline_check_attestations_write`, `ignored=True`, `exact=True`, and an environment that holds `TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR`, `TRACE_COMMONS_PIPELINE_CHECK_SIGNING_KEY_PATH`, `TRACE_COMMONS_PIPELINE_CHECK_SIGNING_KEY_ID`, and `TRACE_COMMONS_PIPELINE_CHECK_MAX_AGE_SECONDS`; without `--signing-key` it does not; the step runs only after `require_current_pass_results` passed (a failing result set never reaches it).
  - `test_the_signing_key_path_never_reaches_output`: the terminal output and the report of that run do not contain the key path's file name.
  - `test_revision_prints_the_tree_hash`: `pipeline.py revision` prints `Run`'s `code_revision_hash` value and one newline.

- [ ] **Step 4: Implement the Rust part.**

```rust
pub const PIPELINE_CHECK_ATTESTATION_SCHEMA: &str = "trace_commons.pipeline_check_attestation.v1";
pub const QUALIFICATION_EVIDENCE_DEFAULT_MAX_AGE_SECONDS: u64 = 24 * 60 * 60;
pub const CHECK_ATTESTATION_SIGNATURE_INVALID_LABEL: &str = "check_attestation_signature_invalid";
pub const CHECK_ATTESTATION_SIGNER_UNTRUSTED_LABEL: &str = "check_attestation_signer_untrusted";
pub const CHECK_ATTESTATION_INVALID_LABEL: &str = "check_attestation_invalid";

/// The hash an attestation's signature covers: every field but the
/// signature, as canonical JSON.
fn attestation_hash(attestation: &PipelineCheckAttestation) -> Result<String, String> {
    let result = serde_json::to_value(&attestation.result)
        .map_err(|_| CHECK_ATTESTATION_INVALID_LABEL.to_string())?;
    evidence_hash(&serde_json::json!({
        "schema": attestation.schema,
        "result": result,
        "maximum_age_seconds": attestation.maximum_age_seconds,
        "corpus_digest": attestation.corpus_digest,
        "input_digest": attestation.input_digest,
    }))
    .map_err(|_| CHECK_ATTESTATION_INVALID_LABEL.to_string())
}
```

  `evidence_hash` refuses a float; `observed_at` serializes as a string and `maximum_age_seconds` as an integer, so the value has none. `sign_check_result` builds the attestation with an empty signature, computes `attestation_hash`, and signs its ASCII bytes with the PKCS#8 key, as `sign_bundle_package` signs the package hash (same algorithm label, same base64url encoding). `CheckResultTrustStore` holds `BTreeMap<String, TrustedBundleKey>`; build it with the checks of `BundlePackageTrustStore::new`. `verify_all`: for each attestation, in order: the schema is the constant, `result.validate()` passes, `maximum_age_seconds > 0`, each `Some` digest is a `sha256:` digest (else `check_attestation_invalid`); the key id is known (else `check_attestation_signer_untrusted`); the signature verifies over `attestation_hash` (else `check_attestation_signature_invalid`). It returns the `DrillEvidence` list and the two digests of Step 1's fourth test. `qualify_bundle` gains one parameter after `evidence`: `verified: Option<(&str, &str)>` (the corpus and input digests of verified attestations); when it is `Some`, the metadata's two values must equal it. `qualify_bundle_attested` verifies the attestations, evaluates the promotion at the current time (for its evidence hash), builds the metadata as P5-D14 says, and calls `qualify_bundle` with `verified: Some(..)`. The PR 4 tests that call `qualify_bundle` pass `None`. Update the doc comments of `qualify_bundle`, `BundleQualificationMetadata`, and `QUALIFICATION_EVIDENCE_AGE_CEILING_SECONDS` (the five notes that name PR 5 at lines 53, 875, 970, 984, and 989): the route's path verifies signed results; the bare store API still takes what its caller gives and is not reachable from a route.

- [ ] **Step 5: Implement the signer and the tooling.** In `pipeline_corpus_pg_tests.rs`, beside `pipeline_package_write`, the ignored test `pipeline_check_attestations_write` (the implementation of the signing step; it needs no database): it reads the four variables of Step 3; for each `<check_id>.result.json` in the directory it parses the result, parses `<check_id>.evidence.json`, requires `evidence_hash(evidence) == result.evidence_hash`, takes `corpus_digest` and `input_digest` from the evidence object when both keys are present, signs with `sign_check_result`, and writes `<check_id>.attestation.json` (create-new, a temporary name then a rename, as the emitter writes); it then verifies every written file with a store built from the same key and asserts that the count equals the number of result files. In the corpus harness, add `"corpus_digest"` and `"input_digest"` to the evidence object that it emits (line 1463's `evidence`): the digest of the normalized direct corpus that the run loaded, and the canonical hash of the list of request-content hashes that it posted, in order; if the evidence already holds these two values under other names, add the two keys with the same values and keep the old names. In `pipeline.py`: `qualify` gains `--signing-key`, `--signing-key-id`, and `--evidence-max-age-seconds` (default `86400`, refused above `604800` with `ToolingError("evidence_max_age_above_ceiling")`); both key flags are required together (`ToolingError("signing_key_incomplete")`); the step runs after `require_current_pass_results` and before the report; the report gains `"attested": true` and the count. Add the `revision` subcommand: it prints `environment._code_revision_hash()`. Add `keygen --output PATH --key-id ID --trusted-key-output PATH2`: it starts the new ignored test `pipeline_signing_key_write` (beside the signer), which generates an Ed25519 PKCS#8 key with `ring` as `pipeline_package_write` generates one, writes it to `PATH` with mode `0600` (it refuses an existing file), and writes the `TrustedBundleKey` JSON to `PATH2`; its assertion is that a result signed with the written key verifies with the written trusted key. Add `test_keygen_runs_the_key_writer_and_prints_no_path` to Step 3's Python tests.

- [ ] **Step 6: Run the tests.** `cargo test -p trace-commons-server --lib versioned_pipeline_qualification`; fresh database `admission_test_pr5_attest`, then `cargo test -p trace-commons-server --test versioned_pipeline_runtime_pg unsigned_or_altered qualify_bundle`; `python3 scripts/operator/test_pipeline_tooling.py`; then one real signed run: `pipeline.py keygen --output .local/pr5-check-key.pk8 --key-id local_check_key --trusted-key-output .local/pr5-check-key.json`, and `pipeline.py qualify --signing-key .local/pr5-check-key.pk8 --signing-key-id local_check_key --postgres-admin-url postgres://trace@127.0.0.1:55432/postgres`. Expected: PASS, and one attestation file for each result file.

- [ ] **Step 7: Commit**

```bash
git add crates/trace-commons-server scripts/operator
git commit -m "Sign check results and verify them before a qualification" -m "Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 9: The qualified activation gate and rollback (V112) (Group B; uses `qualify_bundle`, V107, `PromotionDecision`, `ProductionDependencyProfile`)

**Files:**
- Create: `migrations/V112__versioned_pipeline_activation_gate.sql`
- Modify: the registry, the version list, the RLS suite include, and the grant pins as Task 1 Step 4, for V112 (it adds no table)
- Modify: `crates/trace-commons-server/src/versioned_pipeline_qualification.rs` (`qualify_bundle`'s two statements that name the key, `qualification`, `activate_qualified_bundle_in`, the labels, `DEPLOYED_CODE_REVISION_HASH`)
- Modify: `crates/trace-commons-server/src/versioned_pipeline_activation.rs` (`ActivationRequest`, `activate_tenant`, `rollback_bundle`)
- Modify: `crates/trace-commons-server/src/versioned_pipeline.rs` (make `check_runnable_package` `pub`; delete `PgPipelineStore::activate_bundle` at line 1357 and `PipelineService::activate_bundle` at line 6968)
- Modify: `crates/trace-commons-server/tests/versioned_pipeline_runtime_pg.rs`

**Interfaces:**
- Consumes: V107 (`pipeline_bundle_qualifications`), V93 (`pipeline_active_bundles`), Task 2 (`write_routing_in`, `lock_routing`), Task 5 (`lock_runnable_policy`), `PromotionDecision { ready, evaluated_at, evidence_hash, safe_blockers, code_revision_hash, package }`, `PromotionPackage::is`, `package_digests`, `ProductionDependencyProfile::{blockers, runtime_identity_digest}`, `load_bundle_from_transaction`, `evaluate_activation_readiness`.
- Produces: the activation part of both interface blocks; the key `(tenant_id, bundle_id, code_revision_hash)` on `pipeline_bundle_qualifications`; `UPDATE (bundle_id, selected_at)` on `pipeline_active_bundles` for the runtime role.

- [ ] **Step 1: Write the failing tests** in `versioned_pipeline_runtime_pg.rs`. Helper `two_qualified_bundles(backend, dir)`: extend `qualified_production_service` to return the service and two production-compatible packages A and B that the service can construct, each qualified for one test revision with a full set of passing evidence that names it (reuse the evidence builder of `qualify_bundle_records_an_immutable_hash_only_identity`). B differs from A in one configuration value that changes the bundle id; if every value of `production_compatible_config()` is fixed by another test, give B a second qualified scorer identity that the service also holds. Write in the ledger which one you used.
  - `activation_requires_every_term_of_the_gate` (Review Focus 3): with A qualified, `activate_tenant` succeeds: the routing is `Pipeline`; `active_bundle_id` is A; one `Activate` event with `resulting_bundle_id: Some(A)` and `evidence_hash` equal to the readiness hash. Then, each on a fresh tenant, one term removed at a time, with the label: a decision with `ready: false` (`bundle_activation_promotion_not_ready`); a decision evaluated 16 minutes ago, and one evaluated in the future (`bundle_activation_promotion_stale`); no qualification row (`bundle_qualification_missing`); a runtime revision other than the qualified one (`bundle_runtime_revision_mismatch`); a runtime revision that is not a `sha256:` digest (`bundle_runtime_revision_unknown`); a decision whose `code_revision_hash` is another revision (`bundle_runtime_revision_mismatch`); a decision whose `package` is B's digests while A is activated, and one with `package: None` (`bundle_activation_package_mismatch`); a profile built for B (`bundle_qualification_profile_mismatch`); a profile with `ProductionInfrastructureProfile::local_test()` (its first blocker, `artifact_store_not_production`); a profile whose `runtime_identity_digest` differs from the stored one (`runtime_dependency_identity_mismatch`); one Score policy suspended (`bundle_policy_not_runnable`); a stored package tampered through `tamper_stored_bundle_package` (`bundle_package_missing`); a failed readiness (`activation_readiness_failed`).
  - `a_refused_activation_changes_nothing` (Review Focus 3; "failed activation"): for each refusal of the test above, on a tenant that is already routed with bundle A: the routing row (all six columns), `pipeline_active_bundles`' row (`bundle_id` and `selected_at`), and the number of event rows are the same before and after. Also: an activation whose event insert fails after the gate passed leaves the active bundle and the routing row unchanged, which proves one transaction. To make the insert fail, the test adds a check constraint through the owner connection (`ALTER TABLE pipeline_activation_events ADD CONSTRAINT pr5_test_refuse_reason CHECK (reason_code <> 'force_event_failure') NOT VALID`), activates with that reason code, and drops the constraint at its end (also when an assertion fails: use a guard value whose `Drop` runs the `DROP CONSTRAINT`). The constraint refuses only that reason code, so tests that run at the same time on the database are not affected.
  - `a_bundle_is_qualified_once_for_each_code_revision` (P5-D10): the same bundle and the same revision with other metadata is `bundle_qualification_identity_conflict`; the same bundle with a second revision records a second row; `qualification(tenant, bundle, revision)` returns the row of that revision; the gate with the second revision as the runtime revision passes, and with a third revision is `bundle_runtime_revision_mismatch`.
  - In `db/postgres.rs`: `v113_widens_the_qualification_key_and_grants_the_bundle_switch` (the text contains `ADD PRIMARY KEY (tenant_id, bundle_id, code_revision_hash)` and `GRANT UPDATE (bundle_id, selected_at) ON pipeline_active_bundles TO trace_ingest_runtime;` and no `CREATE TABLE`). In `pipeline_upgrade_tests.rs`: 113 in the version list, the pin `("pipeline_active_bundles", "UPDATE", &["bundle_id", "selected_at"])`, and the assertion that two qualification rows for one bundle with two code revisions are accepted and a second row for one revision is a unique violation.
  - `rollback_selects_an_earlier_bundle_for_new_runs_only` (Review Focus 4; LAB-003, SCN-007; it emits the check of Task 12): activate A; receipt R1 is created under A and stops before Score (the worker is not run); activate B; R1's `bundle_id` is still A; receipt R2 is created under B; `rollback_bundle` to A: the routing is `Pipeline`, the active bundle is A, the event is `Rollback` with `previous_bundle_id: Some(B)` and `resulting_bundle_id: Some(A)`; receipt R3 is created under A; R2 keeps B. Complete R1, R2, and R3: each run's four outcomes carry the run's own bundle id; the outcome rows of R1 that existed before the rollback are byte-equal after it (compare `decision`, `evidence`, `evaluation`). Then: a rollback to the bundle that is active now is `earlier_qualified_bundle_required`; a rollback to a qualified bundle that was never active for this tenant is `earlier_qualified_bundle_required`; a rollback on a tenant with a `Legacy` row or no row is `activation_state_invalid`; a rollback from `Contained` to A succeeds and sets `Pipeline`. A direct `UPDATE pipeline_runs SET bundle_id = ...` with the runtime role fails (the V92 trigger). Emit: `PipelineCheckEmitter::emit_pass_from_env("pipeline_activation_rollback", None, json!({"runs_before_rollback_rebound": 0, "runs_after_rollback_on_earlier_bundle": 1, "outcomes_changed": 0, "refused_rollbacks": 3}))`.
  - `an_activation_races_a_receipt_without_rebinding_it` (BND-003): 20 rounds of `tokio::join!` of a receipt and an activation of B on a tenant routed with A; each created run's `bundle_id` is A or B, its Admission outcome carries the same id, and no receipt fails.
  - `a_rollback_needs_no_readiness_and_an_activation_does`: with the tenant's summary showing one failed index invalidation (seed it as `the_operator_route_requeues_the_tenants_failed_invalidations` does), `activate_tenant` is refused and `rollback_bundle` succeeds.

- [ ] **Step 2: Run them to see them fail.** Fresh database `admission_test_pr5_gate`. `TRACE_COMMONS_PG_TEST_DATABASE_URL=postgres://trace@127.0.0.1:55432/admission_test_pr5_gate cargo test -p trace-commons-server --test versioned_pipeline_runtime_pg activation_requires refused_activation rollback_selects activation_races rollback_needs_no_readiness`. Expected: FAIL.

- [ ] **Step 3: Write the migration and change `qualify_bundle`'s key.**

```sql
-- The activation gate (delivery PR 5).

-- A bundle is qualified for one code revision at a time (P5-D10): the gate
-- needs a qualification for the deployed revision, so a bundle that was
-- qualified on an earlier revision can be qualified again on a later one.
ALTER TABLE pipeline_bundle_qualifications
    DROP CONSTRAINT pipeline_bundle_qualifications_pkey,
    ADD PRIMARY KEY (tenant_id, bundle_id, code_revision_hash);

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_ingest_runtime') THEN
        RAISE EXCEPTION 'V112: trace_ingest_runtime is missing; V90 creates it';
    END IF;
END $$;

-- V93 withheld UPDATE on the active bundle because no ingest route switched
-- a tenant's bundle. The qualified activation gate now does, in a statement
-- that runs only after a qualification for the deployed revision and four
-- runnable policies were found (activate_qualified_bundle_in). No other
-- statement of the ingest runtime updates this row.
GRANT UPDATE (bundle_id, selected_at) ON pipeline_active_bundles TO trace_ingest_runtime;
```

  Confirm the constraint name first (`\d pipeline_bundle_qualifications` on a database migrated to V111) and change the text if it differs. In `qualify_bundle`, the insert becomes `ON CONFLICT (tenant_id, bundle_id, code_revision_hash) DO NOTHING` and the read-back adds `AND code_revision_hash = $3`; its conflict rule (`bundle_qualification_identity_conflict`) then holds inside one revision. These two statements are PR 4 code: change nothing else in the function. Rewrite the registry comment above V107's entry in `db/postgres.rs` (about line 1662, "PR 5's activation gate") to name the gate. Consumer sweep: `git grep -n "pipeline_bundle_qualifications" -- crates scripts` and check each reader against the new key; write the list in the ledger. Register V112 as Task 1 Step 4.

- [ ] **Step 4: Implement the gate.**

```rust
impl PipelineQualificationStore {
    /// Selects `bundle_id` as the tenant's active bundle inside `tx`, when
    /// every term of the activation gate holds (P5-D9); returns the bundle
    /// that was active before. Fails closed with the first failing term's
    /// label and writes nothing. The caller holds the tenant's routing lock
    /// and writes the routing row and the event in the same transaction.
    pub async fn activate_qualified_bundle_in(
        &self,
        tx: &Transaction<'_>,
        tenant_id: &str,
        bundle_id: &str,
        promotion: &PromotionDecision,
        runtime_code_revision_hash: &str,
        dependencies: &ProductionDependencyProfile,
        now: DateTime<Utc>,
    ) -> Result<Option<String>, DatabaseError> {
        let refuse = |label: &str| Err(DatabaseError::Constraint(label.to_string()));
        if !promotion.ready || !promotion.safe_blockers.is_empty() {
            return refuse(ACTIVATION_PROMOTION_NOT_READY_LABEL);
        }
        if promotion.evaluated_at > now
            || now - promotion.evaluated_at > Duration::seconds(ACTIVATION_PROMOTION_MAX_AGE_SECONDS)
        {
            return refuse(ACTIVATION_PROMOTION_STALE_LABEL);
        }
        if !is_sha256(runtime_code_revision_hash) {
            return refuse(PACKAGE_RUNTIME_REVISION_UNKNOWN_LABEL);
        }
        if promotion.code_revision_hash.as_deref() != Some(runtime_code_revision_hash) {
            return refuse(PACKAGE_RUNTIME_REVISION_MISMATCH_LABEL);
        }
        let qualified = tx
            .query(
                "SELECT code_revision_hash, runtime_dependency_digest
                   FROM pipeline_bundle_qualifications
                  WHERE tenant_id = $1 AND bundle_id = $2",
                &[&tenant_id, &bundle_id],
            )
            .await?;
        if qualified.is_empty() {
            return refuse(PACKAGE_QUALIFICATION_MISSING_LABEL);
        }
        let Some(row) = qualified
            .iter()
            .find(|row| row.get::<_, String>("code_revision_hash") == runtime_code_revision_hash)
        else {
            return refuse(PACKAGE_RUNTIME_REVISION_MISMATCH_LABEL);
        };
        let package = load_bundle_from_transaction(tx, tenant_id, bundle_id)
            .await
            .ok()
            .flatten()
            .ok_or_else(|| DatabaseError::Constraint(PIPELINE_BUNDLE_MISSING_LABEL.to_string()))?;
        let digests = package_digests(&package).map_err(DatabaseError::Constraint)?;
        if !promotion.package.as_ref().is_some_and(|named| named.is(&digests)) {
            return refuse(ACTIVATION_PACKAGE_MISMATCH_LABEL);
        }
        if dependencies.bundle.bundle_id != bundle_id
            || dependencies.bundle.dependency_digest != digests.dependency_digest
        {
            return refuse("bundle_qualification_profile_mismatch");
        }
        if let Some(blocker) = dependencies.blockers().into_iter().next() {
            return Err(DatabaseError::Constraint(blocker));
        }
        if row.get::<_, String>("runtime_dependency_digest")
            != dependencies.runtime_identity_digest().map_err(DatabaseError::Constraint)?
        {
            return refuse("runtime_dependency_identity_mismatch");
        }
        // Four runnable policies, each locked so a suspension waits for this
        // activation or is seen by it (Task 5's guard).
        let runnable: i64 = tx
            .query_one(
                "SELECT COUNT(*) FROM (
                     SELECT 1 FROM pipeline_bundle_policy_status
                      WHERE tenant_id = $1 AND bundle_id = $2 AND runnable
                      FOR SHARE
                 ) AS runnable_policies",
                &[&tenant_id, &bundle_id],
            )
            .await?
            .get(0);
        if runnable != 4 {
            return refuse(PIPELINE_POLICY_NOT_RUNNABLE_LABEL);
        }
        let previous: Option<String> = tx
            .query_opt(
                "SELECT bundle_id FROM pipeline_active_bundles WHERE tenant_id = $1 FOR UPDATE",
                &[&tenant_id],
            )
            .await?
            .map(|row| row.get(0));
        tx.execute(
            "INSERT INTO pipeline_active_bundles (tenant_id, bundle_id)
             SELECT $1, bundle_id FROM pipeline_bundle_packages
              WHERE tenant_id = $1 AND bundle_id = $2
             ON CONFLICT (tenant_id) DO UPDATE
                SET bundle_id = EXCLUDED.bundle_id, selected_at = NOW()",
            &[&tenant_id, &bundle_id],
        )
        .await?;
        Ok(previous)
    }
}
```

  `PIPELINE_BUNDLE_MISSING_LABEL` is `"bundle_package_missing"` (`versioned_pipeline.rs`, line 136); `load_bundle_from_transaction` validates the package, so a tampered package gives the same label (BND-002). `SELECT ... FOR UPDATE` on `pipeline_active_bundles` needs the `UPDATE` grant of V112. `qualification(tenant, bundle, revision)` reads one row with the new key and returns it through `qualification_from_row`.

- [ ] **Step 5: Implement `activate_tenant` and `rollback_bundle`.** Both: `validate_actor`; open a tenant transaction; `lock_routing`; read the current routing row. `activate_tenant`: `evaluate_activation_readiness(readiness, now)` (its label on failure); the gate; `write_routing_in(tx, tenant, RoutingState::Pipeline, ActivationAction::Activate, previous_bundle, Some(bundle), actor, reason, &readiness.evidence_hash)`; commit. `rollback_bundle`: the current state must be `Pipeline` or `Contained` (else `activation_state_invalid`); the bundle must differ from the active bundle, and an event row of this tenant with `resulting_bundle_id = $bundle` and `action IN ('activate', 'rollback')` must exist (else `earlier_qualified_bundle_required`); the gate; `write_routing_in(... ActivationAction::Rollback ..., &promotion.evidence_hash)`; commit. Neither takes or writes anything of another tenant. Make `PipelineService::check_runnable_package` `pub` (Task 10's route calls it before the store call, so the startup checks of a tenant bundle also hold for a bundle that is activated after start). Delete `PgPipelineStore::activate_bundle` (line 1357) and its forward `PipelineService::activate_bundle` (line 6968): with V112's grant they would be a switch without the gate. `git grep -n "\.activate_bundle(" -- crates` must find no caller after the deletion (today the forward is the only one); the test helper `activate_bundle_as_operator` keeps its own statement through the owner connection, and its comment and the comment at about line 13037 of the runtime suite are rewritten to say that the runtime role can switch a bundle only through the gate.

- [ ] **Step 6: Consumer sweep.** Readers of `pipeline_active_bundles`: `runnable_bundle_ids` (1408), `active_bundle_id` (1426), the staging transaction (7974), `register_default_bundle` through `activate_bundle_if_none` (1385). An activation changes the row that all four read. `validate_pipeline_tenant_bundles` ran at start only for the bundles known then: Task 10's route runs `check_runnable_package` for the new bundle. `activate_bundle_if_none` at start does not undo an activation (it inserts only when no row exists): assert this in `rollback_selects_an_earlier_bundle_for_new_runs_only` by calling `register_default_bundle(tenant)` after the rollback and reading the active bundle again.

- [ ] **Step 7: Run the tests.** The Step 2 command with `a_bundle_is_qualified_once qualify_bundle` added, plus `activation_does_not_rebind_an_existing_run startup_checks_every_bundle_a_tenant_may_run`; the upgrade test and `migration_atomicity_pg` as in Task 1 Step 6 with fresh databases. Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add migrations/V112__versioned_pipeline_activation_gate.sql crates/trace-commons-server
git commit -m "Gate activation and rollback on a qualified bundle" -m "Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 10: Infrastructure profile and admin routes (Group B; uses `qualify_bundle` and `ProductionInfrastructureProfile`)

**Files:**
- Create: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_activation.rs` (AGPL header; `use super::*;` as `pipeline_runtime.rs` does)
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` (the module declaration beside `pipeline_runtime`; the route registrations beside `/v1/admin/pipeline/operational-summary` at line 8325; `AppState`; `from_env_with_pipeline_runtime_assembler`)
- Modify: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_activation_pg_tests.rs`

**Interfaces:**
- Consumes: Tasks 2, 5, 6, 8, 9 (with `DEPLOYED_CODE_REVISION_HASH`); `authenticate_with_tenant_access_grant`, `require_admin`, `require_pipeline_service`, `api_error`, `internal_error`; `ProductionDependencyProfile::for_bundle`; `PipelineProductStore::operational_summary`; `state.pipeline_main_gate` (if `AppState` does not keep the `MainGateConfig` that line 3938 parses, add the field `pipeline_main_gate: MainGateConfig`).
- Produces: the routes of P5-D11; `infrastructure_profile_from_state(state: &AppState) -> ProductionInfrastructureProfile`; the variables `TRACE_COMMONS_PIPELINE_PACKAGE_TRUSTED_KEYS_PATH` and `TRACE_COMMONS_PIPELINE_CHECK_TRUSTED_KEYS_PATH`.

- [ ] **Step 1: Write the failing tests.** Unit tests in `pipeline_activation.rs`:
  - `the_infrastructure_profile_follows_the_configuration`: one case for each row of this table, changing one field of a test state at a time.

| Profile field | Value from `AppState` |
| --- | --- |
| `authoritative_metadata` | `Production` when `db_mirror.is_some() && require_db_mirror_writes`; `Development` when `db_mirror.is_some()` only; else `Missing` |
| `best_effort_database_mirror` | `db_mirror.is_some() && !require_db_mirror_writes` |
| `artifact_store` | from the configured store's `provider_label`: `Some("gcs")` is `Production`; `Some("file_system")` and `Some("local_encrypted")` are `Development`; `None` is `Missing` |
| `plaintext_fallback` | the configured store's `plaintext_compatibility_allowed` |
| `key_wrapper` | from the configured store's `kek_status`: the provider `gcp_cloud_kms` is `Production`; a local master key is `Development`; `None` is `Missing` |
| `authentication` | `Production` when `signed_token_verifier.is_some() && require_managed_eddsa_signed_tokens`; `Development` when a verifier exists without that requirement; else `Missing` |
| `static_bearer_authentication` | `!state.tokens.is_empty()` |
| `hs256_bridge_authentication` | the verifier's `configured_key_count()` is above its EdDSA key count (the two numbers that `trace_commons_config_status_response` reports, line 12909) |
| `unversioned_policy_dependencies` | `false`: `bundle_qualification` already fails closed for a dependency without a content hash (`bundle_dependency_missing`) |
| `live_external_payout_enabled` | `pipeline_service.payout_enabled()` |

  If `AppState` does not keep the configured store's `provider_label`, `plaintext_compatibility_allowed`, and `kek_status` after start, keep the three values in a small `PipelineInfrastructureFacts` field that `from_env_with_pipeline_runtime_assembler` fills from `ConfiguredTraceArtifactStore` (line 2577).
  - `a_trust_store_file_is_a_list_of_trusted_keys`: the loader reads a JSON array of `TrustedBundleKey`; an empty array, a file that is not JSON, and a duplicate key id fail with `pipeline_trust_store_invalid`; an unset variable gives `None`.

  HTTP tests in `pipeline_activation_pg_tests.rs` (the assembler injects `qualified_production_service`'s construction, repeated here from the runtime suite; the test state holds both trust stores and a test `pipeline_code_revision_hash`; the admin token is the test state's admin credential):
  - `the_admin_routes_require_an_admin_and_a_runtime`: each of the nine routes answers a request without a credential as `GET /v1/admin/pipeline/operational-summary` answers one, answers `403` `admin token required` to a contributor credential, and (except `GET routing` and `GET legacy-drain`, which need only the store) `404` in an app with no runtime.
  - `a_test_deployment_cannot_qualify_or_activate_through_the_route`: with the real `infrastructure_profile_from_state` of the test state (a local artifact store, static tokens), `POST qualifications` with a valid signed package and valid attestations answers `409` with the first blocker's label (`artifact_store_not_production`), and no qualification row exists.
  - `the_routes_qualify_activate_roll_back_contain_and_deactivate`: with the test state's `#[cfg(test)]` override `pipeline_infrastructure_override: Some(<all Production, all flags false>)`: `POST qualifications` for A and for B records two rows whose `code_revision_hash` is the state's; `POST activate` for A answers `200` with `routing_state: "pipeline"`; an upload is `processing`; `POST activate` for B; `POST rollback` to A; `POST contain`: an upload is `503`; `POST deactivate`: an upload gets a legacy receipt; `GET routing` returns the state `legacy`, the active bundle A, and five events, newest first, with no field outside the allow-list (`event_id`, `action`, `previous_state`, `resulting_state`, `previous_bundle_id`, `resulting_bundle_id`, `actor_principal_ref`, `reason_code`, `evidence_hash`, `recorded_at`).
  - `the_routes_refuse_without_a_trust_store_or_a_revision`: with `pipeline_check_trust: None`, `POST qualifications` and `POST activate` answer `503` `pipeline_trust_store_missing`; with `pipeline_code_revision_hash: None`, `409` `bundle_runtime_revision_unknown`. Nothing is written.
  - `an_activation_through_the_route_keeps_the_startup_checks`: a qualified bundle whose compatibility configuration does not match the state's `MainGateConfig` is refused with `409` `pipeline_runtime_main_gate_config_mismatch`; with the credit issuer unset in the service, `409` `pipeline_credit_issuer_principal_missing`.
  - `a_route_never_acts_on_another_tenant`: tenant B's admin calls `POST contain`; tenant A's routing is unchanged; no request body field named `tenant_id` is read (a body with one is refused by `deny_unknown_fields`).
  - `the_policy_routes_suspend_resume_and_list`: `POST policy-interventions` with `suspend` then `resume` for the Score policy; `GET policy-interventions?bundle_id=...` lists two records; `terminate` answers `409` `policy_intervention_not_supported`.
  - `the_legacy_drain_route_returns_the_report`: `GET legacy-drain` equals `legacy_drain_report` for the tenant, and its body holds only labels, counts, a state, a time, and a hash.
  - `no_route_response_or_log_line_holds_a_token_or_a_tenant_id`: collect every response body of the tests above and the captured log lines; none contains the admin token, a contributor token, or the tenant id string.

- [ ] **Step 2: Run them to see them fail.** Fresh database `admission_test_pr5_routes` and the resolver URL. `cargo test -p trace-commons-server --bin trace-commons-ingest pipeline_activation -- --test-threads=1`. Expected: FAIL.

- [ ] **Step 3: Implement.** Request bodies (`#[serde(deny_unknown_fields)]`): `QualifyBody { signed_package: SignedBundlePackage, attestations: Vec<PipelineCheckAttestation> }`; `ActivateBody { bundle_id: String, reason_code: String, attestations: Vec<PipelineCheckAttestation> }` (also for rollback); `ReasonBody { reason_code: String }`; `InterventionBody { bundle_id: String, phase: Phase, action: String, reason_code: String }`. Bound each list at 64 attestations (`413` `pipeline_evidence_too_large`). Every handler starts with:

```rust
    let tenant = authenticate_with_tenant_access_grant(state.as_ref(), &headers).await?;
    require_admin(&tenant)?;
```

  and uses `tenant.tenant_id` and `tenant.principal_ref` only. The activation handler, in order: the runtime (`require_pipeline_service`); the two trust stores and the revision (`503` `pipeline_trust_store_missing`; `409` `bundle_runtime_revision_unknown`); `check_trust.verify_all(&body.attestations)` (`409` with its label); `evaluate_promotion(&verified.evidence, Utc::now())` (`409` with its label on `Err`; a decision that is not ready is refused by the gate); the stored package (`pipeline_store.load_bundle`; `404` when none); `pipeline_service.check_runnable_package(&package, &state.pipeline_main_gate, true)` (`409` with its label); `ProductionDependencyProfile::for_bundle(service, &package, infrastructure_profile_from_state(state))`; `ActivationReadiness::from_operational_summary(&product.operational_summary(tenant).await?)`; then `activate_tenant`. A `DatabaseError::Constraint(label)` whose text is a safe label (`is_safe_label`) becomes `409` with that label; any other error is `internal_error`. The rollback handler is the same without the readiness. The qualification handler verifies `package_trust.verify(&signed)` through `qualify_bundle_attested`. Log one line for each successful action with `tenant_storage_ref`, the action label, and the event's `evidence_hash`; never the bundle package, a token, or the tenant id. Load the two trust stores at start from the two variables; a set variable whose file is invalid refuses the start with `pipeline_trust_store_invalid`; `pipeline_code_revision_hash` is `DEPLOYED_CODE_REVISION_HASH.map(str::to_string)`. Register the nine routes in the admin group, where `/v1/admin/pipeline/operational-summary` is registered. Rewrite the last paragraph of `qualify_bundle`'s doc comment (`versioned_pipeline_qualification.rs`, about line 991 to 996, "PR 5's activation must keep them"): the activation route keeps the two startup checks through `check_runnable_package`.

- [ ] **Step 4: Sibling sites and the lenses.** The nine handlers share one `fn activation_error(error: DatabaseError) -> (StatusCode, Json<ApiError>)`; none builds an error string from a request field. Efficiency: an activation runs one summary read and one transaction; the transaction holds the tenant's routing lock, so uploads of that tenant wait for it (the gate is a few indexed reads; no network call and no object-store call runs inside it; `check_runnable_package` and `for_bundle` run before the transaction). Write this in the ledger.

- [ ] **Step 5: Run the tests.** The Step 2 command, then the whole `pipeline_http_pg_tests` module once. Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/trace-commons-server/src/bin
git commit -m "Add the admin routes for qualification, routing, and policy interventions" -m "Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 11: Committed rebuild fence (V113) (Group B; uses the rebuild route and `rebuild_index_run`)

**Files:**
- Create: `migrations/V113__versioned_pipeline_rebuild_fence.sql`
- Modify: the registry, pins, and lists as Task 1 Step 4, for V113 and `pipeline_index_rebuild_fences`
- Modify: `crates/trace-commons-server/src/versioned_pipeline.rs` (`claim_due_index_invalidations` (3131), `rebuild_index_from_authoritative_commands` (8838), the doc of `rebuild_index_run` (8944) and the note at 8943)
- Modify: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_runtime.rs` (the doc of `pipeline_index_rebuild_handler`, lines 350 to 360)
- Modify: `crates/trace-commons-server/tests/versioned_pipeline_runtime_pg.rs`

**Interfaces:**
- Consumes: `PIPELINE_INDEX_WRITE_FENCE_MARGIN_SECONDS`, `PIPELINE_INDEX_DISPATCH_BUDGET_SECONDS`, the rebuild's per-run deadline, `process_index_invalidations`.
- Produces: `PgPipelineStore::set_index_rebuild_fence`, `clear_index_rebuild_fence`; the fence predicate in the invalidation claim.

- [ ] **Step 1: Write the failing tests** in `versioned_pipeline_runtime_pg.rs`:
  - `an_invalidation_is_not_claimed_while_a_rebuild_fence_is_set`: a complete, indexed run; `set_index_rebuild_fence(tenant, fence, now + 60 s)`; withdraw the submission (its invalidation is queued); `process_index_invalidations(tenant, 32)` returns 0 and the entry is still in the index; `clear_index_rebuild_fence(tenant, fence)`; `process_index_invalidations` returns 1 and the entry is gone.
  - `an_expired_fence_does_not_block`: a fence with `fenced_until` one second in the past does not stop the claim.
  - `a_fence_is_tenant_scoped`: tenant A's fence does not stop tenant B's invalidations.
  - `a_rebuild_sets_extends_and_clears_its_fence`: with a writer double that records the fence row at each upsert (it reads `pipeline_index_rebuild_fences` through the owner connection), a rebuild of three runs sees a row at every upsert whose `fenced_until` is later than the time of that upsert plus `PIPELINE_INDEX_WRITE_FENCE_MARGIN_SECONDS`; after the rebuild returns, no row exists. Another rebuild's `clear` with a different `fence_id` does not delete the row.
  - `a_rebuild_that_loses_its_session_cannot_be_overtaken_by_an_invalidation`: the writer double blocks in its second upsert; the test withdraws that run's submission and drops the rebuild future's transaction (the existing dropped-rebuild harness of `a_dropped_index_rebuild_request_keeps_its_run_locked_until_its_writes_commit`, with the lock released as a lost session releases it); a second service's `process_index_invalidations` returns 0 while the fence is unexpired; release the writer; after the fence's time (shorten the margin for the test through the service's test-only setter, as the deadline tests do) the invalidation runs and the entry is gone.
  - `the_rebuild_route_still_refuses_an_active_tenant` stays as PR 4 has it (no change; run it).

- [ ] **Step 2: Run them to see them fail.** Fresh database `admission_test_pr5_fence`. Expected: FAIL.

- [ ] **Step 3: Write the migration.**

```sql
-- A committed fence for the index rebuild (delivery PR 5). While a tenant's
-- row is unexpired, no worker in any process claims that tenant's index
-- invalidations, so an invalidation cannot run before a rebuild's last write.
-- A rebuild that ends deletes its row; one that is lost leaves a row that
-- expires.

CREATE TABLE pipeline_index_rebuild_fences (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    fence_id UUID NOT NULL,
    fenced_until TIMESTAMPTZ NOT NULL,
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id)
);

ALTER TABLE pipeline_index_rebuild_fences ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline_index_rebuild_fences FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON pipeline_index_rebuild_fences;
CREATE POLICY trace_corpus_tenant_isolation ON pipeline_index_rebuild_fences
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_ingest_runtime') THEN
        RAISE EXCEPTION 'V113: trace_ingest_runtime is missing; V90 creates it';
    END IF;
END $$;

GRANT SELECT, INSERT, DELETE ON pipeline_index_rebuild_fences TO trace_ingest_runtime;
GRANT UPDATE (fence_id, fenced_until, started_at)
    ON pipeline_index_rebuild_fences TO trace_ingest_runtime;
```

- [ ] **Step 4: Implement.** `set_index_rebuild_fence`: one statement in its own committed tenant transaction: `INSERT INTO pipeline_index_rebuild_fences (tenant_id, fence_id, fenced_until) VALUES ($1, $2, $3) ON CONFLICT (tenant_id) DO UPDATE SET fence_id = EXCLUDED.fence_id, fenced_until = GREATEST(pipeline_index_rebuild_fences.fenced_until, EXCLUDED.fenced_until)`. (`GREATEST` so one rebuild never shortens another's fence; the route already runs one rebuild for a tenant in a process.) `clear_index_rebuild_fence`: `DELETE ... WHERE tenant_id = $1 AND fence_id = $2`. In `claim_due_index_invalidations`, add to the `due` CTE's `WHERE`, so the fence and the invalidations are read in one statement and one snapshot:

```sql
                       AND NOT EXISTS (
                           SELECT 1 FROM pipeline_index_rebuild_fences f
                            WHERE f.tenant_id = $1 AND f.fenced_until > NOW()
                       )
```

  Why one statement is enough: an invalidation that this claim can see was committed before the claim's snapshot. If the fence is not visible in that snapshot, the withdrawal committed before the fence, so the rebuild's own lock of that run (`lock_rebuildable_index_run_on_tx`) sees the queued invalidation and skips the run. If the withdrawal committed after the fence, the claim sees the fence. Put this reasoning in the doc comment of the claim. In `rebuild_index_from_authoritative_commands`: make one `fence_id`; before the first run and again before each run's writes, call `set_index_rebuild_fence(tenant, fence_id, now + run_deadline + margin)`, where `run_deadline` is the deadline `rebuild_index_run` already computes (the smaller of the Settle lease and `PIPELINE_INDEX_DISPATCH_BUDGET_SECONDS`) and `margin` is `PIPELINE_INDEX_WRITE_FENCE_MARGIN_SECONDS`; when the rebuild returns `Ok`, or returns an error after every started write has returned, clear the fence; when it fails as `index_unavailable` with a call still in flight, leave the fence to expire. A fence write that fails stops the rebuild before the run's first write (`index_rebuild_fence_unavailable`, mapped to `503` beside `index_unavailable` in `pipeline_index_rebuild_error`). Rewrite the three doc notes that name PR 5 (`pipeline_runtime.rs` 358 to 360; `versioned_pipeline.rs` 8930 to 8943): what the fence closes (another replica; a lost session; an abort; a process exit) and what stays (the route refuses a tenant its own process routes or drains, because a Score must not read a partly rebuilt index).

- [ ] **Step 5: The lenses.** One committed upsert for each rebuilt run (a rebuild is an operator action after a restore, so the count is small); one `NOT EXISTS` on a primary key in each invalidation claim. A fence can delay a tenant's invalidations by at most the run deadline plus the margin after a lost rebuild (90 s at the defaults): write the number in the ledger and in the runbook.

- [ ] **Step 6: Register V113** as Task 1 Step 4 (`PIPELINE_TABLES` to 20 entries; pins `SELECT`, `INSERT`, `DELETE`, and `("pipeline_index_rebuild_fences", "UPDATE", &["fence_id", "fenced_until", "started_at"])`).

- [ ] **Step 7: Run the tests.** The new tests; every `index_rebuild_*` test; `the_worker_drain_removes_a_withdrawn_revision_from_the_index` and `the_index_rebuild_route_appends_an_audit_row` in the ingest bin; the upgrade test and `migration_atomicity_pg`. Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add migrations/V113__versioned_pipeline_rebuild_fence.sql crates/trace-commons-server
git commit -m "Fence index invalidations while a rebuild writes" -m "Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 12: The three PR 5 checks in `qualify` (Group B; uses the check lists and `pipeline.py`)

**Files:**
- Modify: `crates/trace-commons-server/src/versioned_pipeline_qualification.rs` (`PROMOTION_REQUIRED_CHECKS`)
- Modify: `scripts/operator/pipeline_tooling/checks.py`, `scripts/operator/test_pipeline_tooling.py`

**Interfaces:**
- Consumes: the emitting tests of Task 4 (`containment_refuses_new_receipts_and_keeps_pending_work`), Task 9 (`rollback_selects_an_earlier_bundle_for_new_runs_only`), and Task 6 (`the_legacy_drain_report_counts_real_pending_work_and_reaches_zero`).
- Produces: the check ids `pipeline_activation_containment`, `pipeline_activation_rollback`, `pipeline_legacy_drain` in both lists.

- [ ] **Step 1: Write the failing Python test** `test_the_activation_checks_are_required`: the three ids are in `checks.REQUIRED_CHECK_IDS`; each has `digests_required=False`; their rows name the tests above (`_HTTP_TESTS + "containment_refuses_new_receipts_and_keeps_pending_work"` in the ingest bin with database `pilot`; the runtime suite test; `"tests::pipeline_activation_pg_tests::the_legacy_drain_report_counts_real_pending_work_and_reaches_zero"` in the ingest bin with database `pilot`). The existing test that compares `REQUIRED_CHECK_IDS` with the Rust list (line 2296) now fails until Step 2.

- [ ] **Step 2: Change both lists in one commit.** In Rust, add the three ids after `"pipeline_restore_drill"` and before the promotion-only comment. In `checks.py`, add three rows to `REQUIRED_DATABASE_CHECKS`. Update the unit tests in `versioned_pipeline_qualification.rs` that count the list or build one result for each id (they use the constant, so most need no edit; run them).

- [ ] **Step 3: Prove that a missing emission fails.** Remove the `emit_pass_from_env` line from the rollback test in a scratch edit; `pipeline.py qualify` must fail with the missing-result label for `pipeline_activation_rollback`; restore the line. Record the failing output's label in the ledger (not a commit).

- [ ] **Step 4: Run.** `python3 scripts/operator/test_pipeline_tooling.py`; `cargo test -p trace-commons-server --lib versioned_pipeline_qualification`; `pipeline.py qualify --postgres-admin-url postgres://trace@127.0.0.1:55432/postgres` (tell the owner the estimate first). Expected: PASS, with 19 required results, of which four name the one package.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-server/src/versioned_pipeline_qualification.rs scripts/operator
git commit -m "Require the containment, rollback, and legacy drain checks" -m "Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 13: Runbooks, inventory, and CI (Group B)

**Files:**
- Modify: `docs/operator/pipeline-activation.md`, `docs/operator/pipeline-qualification.md`, `docs/operator/backup-restore.md`, `docs/operator/deployment.md`, `docs/operator/README.md`
- Modify: `scripts/operator/pipeline-deployment-inventory.py`
- Modify: `.github/workflows/ci.yml` (comment only)

- [ ] **Step 1: The inventory lint.** Run `python3 scripts/operator/pipeline-deployment-inventory.py`. It must fail for the nine new routes and five new tables until they are classified. Add them with the classes that the script uses for `/v1/admin/pipeline/operational-summary` and for `pipeline_bundle_qualifications`. Run it again: PASS, with the new counts in the ledger.

- [ ] **Step 2: `pipeline-activation.md`.** Remove the "design stage" banner. Rewrite these sections; keep every other section as PR 4 leaves it:
  - The opening list: what is true now (a routing row decides; one owner for each receipt; activation needs a qualification, signed evidence, the deployed revision, the dependency profile, and four runnable policies; rollback selects an earlier bundle that was active; containment refuses new receipts with `503` and keeps the worker; a suspended policy keeps its run). Remove "Legacy writers disable only after their pending owned work completes": PR 5 reports the drain and disables nothing.
  - "Pipeline receipt routing before activation" becomes "Scope lists and the routing row": the two lists as the process scope; the table of P5-D4's outcomes with each HTTP status and label; the test-only rule of P5-D5 and that the flag refuses to combine with `TRACE_COMMONS_PIPELINE_RUNTIME_REQUIRED`; the cost (one read for each new upload with a runtime; one claim for each legacy upload of a tenant in scope); that the receipts list count in `config-status` and in the rollback drill's flags now means "in scope".
  - New "Activate, roll back, contain, deactivate": the nine routes, their bodies, each refusal label and its meaning, the two trust store variables, `TRACE_COMMONS_BUILD_CODE_REVISION_HASH` and `pipeline.py revision`, the 15-minute rule, and the order of a first rollout (qualify, activate one tenant, watch the summary, contain on doubt, roll back or deactivate).
  - New "Suspend a policy": the routes; what a suspended run shows (`bundle_policy_not_runnable`, uncharged retry); that a Settle suspension can wait for one index dispatch; that a payout waits; that `terminate` is not supported.
  - New "Legacy drain report": each count and its source; what is not counted and why; that `drained` does not disable anything; the rehearsal's test name.
  - "Rehearse the switch": the real commands (`cargo test -p trace-commons-server --test versioned_pipeline_runtime_pg rollback_selects activation_requires`, the two ingest bin tests, and `pipeline.py qualify`).
  - "Local operator routes": delete (the binary does not exist; the routes are in ingest).
  - "Current completion": activation is in the repository; production routing is off until an operator activates a tenant; the promotion requirements that remain.
- [ ] **Step 3: `pipeline-qualification.md`.** Lines 6, 200 to 223, and 311 to 368: the three new checks in the check table; mechanics checks name no package and four checks name one; `qualify --signing-key`, the attestation file, the check trust store, and who must hold the key; `qualify_bundle` over HTTP; the metadata digests that the server now computes; one qualification for each code revision.
- [ ] **Step 4: `backup-restore.md`, steps 3 to 5.** Step 3 stays (both lists unset for the rebuild), with two changes: an upload of an activated tenant is now refused with `503` `pipeline_tenant_not_served` and does not take the legacy path; and the routing rows, events, ownership rows, and qualifications restore with the database. Step 4: the fence, what it closes, and the 90 s bound after a lost rebuild; delete the sentence that says the fence is PR 5 work. Step 5 unchanged.
- [ ] **Step 5: `deployment.md`.** A section "V110 to V113: activation, policy interventions, the activation gate, and the rebuild fence" in the form of "V107 and V108": tables, grants, the new `UPDATE` on `pipeline_active_bundles` and on `pipeline_bundle_policy_status`, and the three new variables. Add the runbook lines to `docs/operator/README.md` if a new file was created (none is planned).
- [ ] **Step 6: CI.** No job is added and none becomes required (P5-D19). Update the comment above `pipeline-qualification` in `ci.yml` only if it lists the checks. `README.md` and `CLAUDE.md` job counts do not change.
- [ ] **Step 7: Check every doc claim against the code.** For each label, route, variable, and table name in the changed docs, `git grep` it in `crates/` and `migrations/`. A name that is not found is a defect. Then:

```bash
git add docs scripts/operator/pipeline-deployment-inventory.py .github/workflows/ci.yml
git commit -m "Document activation, containment, rollback, and the legacy drain report" -m "Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 14: PR 5 gate (Group B; no commit)

Owner rule (project memory "Publish, then let CI verify", common.md "Publish"): no multi-hour local gates. When the branch compiles cleanly and its review found no Critical issue, it is ready to publish with the owner's yes, and the upstream CI runs the full suites. Give a time estimate before any step that takes more than about 30 minutes.

- [ ] **Step 1:** Every check in Global Constraints, from the worktree root, at the tip only.
- [ ] **Step 2:** The focused suites, each one time on a fresh database: the new tests of Tasks 1 to 11 by name; `python3 scripts/operator/test_pipeline_tooling.py`; `python3 scripts/operator/pipeline.py qualify` one time (it is the CI job's command).
- [ ] **Step 3:** The final review, with separate lenses (common.md, review rule 5): correctness; `main` integration (the table in Review Focus, checked site by site against the tip); efficiency (each query and lock this plan adds, listed in the ledger by Tasks 3, 4, 5, 6, 10, and 11); concurrency (the routing lock, the policy row lock, the ownership key, the fence); configuration and operations (the three variables, the two lists, the grants, the runbooks). Each lens gives candidate findings; a separate step verifies each one. Triage by review rule 6.
- [ ] **Step 4:** Leave to the upstream CI when PR 5 is published: `cargo test --workspace`, the feature checks, the `cargo deny` runs, the MSRV floor, and the whole ingest bin against PostgreSQL.
- [ ] **Step 5:** Record the candidate revision, every executed check, the evidence location, the merges of PR 4 that PR 5 took, and the open items ("Items for other PRs") in the ledger. Write the PR description draft in `replies.md` (scope, predecessor PR 4, tests, a link to the implementation brief, the licensing attestation from the PR template). Do not push. Ask the owner.

---

## Self-review (done while writing)

- **Spec coverage.**
  - Delivery row 5 (brief section 6): qualified routing (Tasks 2, 3, 4, 9, 10), receipt ownership (Tasks 1, 3, 4), containment (Tasks 2, 3, 4), rollback (Task 9), actual legacy drain rehearsal (Task 6). Production routing stays off (P5-D5; `a_tenant_with_no_row_stays_on_the_legacy_path_without_the_test_flag`). Own migrations: V110 to V113.
  - Section 5: no writer or table is retired ("Deferred"; P5-D6; the rehearsal's step 7 proves the legacy writer still accepts).
  - Section 7 gates: mixed legacy and pipeline receipts and exact replay (Task 4's first two tests; Task 3); failed activation (`a_refused_activation_changes_nothing`); containment (Task 4, check `pipeline_activation_containment`); rollback for future runs (Task 9, check `pipeline_activation_rollback`); suspension and resumption (Task 5); actual pending legacy work (Task 6, check `pipeline_legacy_drain`).
  - Section 3A: the profile at qualification and at activation is built from the service's own objects for that bundle (`ProductionDependencyProfile::for_bundle`), and the gate compares its identity digest with the stored one (Task 9).
  - Section 2: "Activation and rollback affect future runs" (Task 9), "permanent receipt ownership across legacy and new routing" (Tasks 3, 4), "Check guards before work and again in the commit transaction; lock the policy-status rows" (Task 5), "Check again before external payout dispatch" (Task 5), "Suspension leaves work retryable under the same bound package" (Task 5).
  - Package qualification spec: `activate_qualified_bundle` needs a current passing decision of at most 15 minutes, the deployed revision equal to the qualified revision, the current dependency profile, and four runnable policies (Task 9); the decision and the profile are the server's (Tasks 8, 10).
  - Behavioral contracts: BND-002 (a tampered stored package refuses activation), BND-003 and SCN-007 (Task 9), GRD-004 and SCN-008 (Task 5), LAB-003 (Task 9), SCN-013 (the refusal tests of Tasks 9 and 10: no run, no routing change), SCN-015's last line (Task 9), OPS-003's suspended policies (Task 5), SUB-003 and SUB-004 (Task 3).
  - Items that PR 4 passed to PR 5: `activate_qualified_bundle` and the qualified gate on `pipeline_active_bundles` (Task 9, V112's grant); the admin route for a qualification from the server's profile and the HTTP route for `qualify_bundle` (Task 10); `ProductionInfrastructureProfile` from the configuration (Task 10); policy interventions (Task 5; `terminate` by written decision P5-D12); the three check ids in both lists with the Python agreement test (Task 12); the PR 5 tables in the three lists (Tasks 1, 5, 11); signed check results (Task 8); `corpus_digest` and `input_digest` (Task 8, P5-D14); the committed rebuild fence (Task 11); mechanics checks and packages (Task 7, P5-D15); the two startup checks at activation (Task 10, `an_activation_through_the_route_keeps_the_startup_checks`); the ownership row and the routing lookup (Task 3); the two lists and the restore runbook's step 3 (P5-D3, P5-D4, P5-D16, Task 13); the 23 notes that name PR 5 (each one is rewritten in the task that owns its line: Tasks 3, 7, 8, 11, 13; Task 13 Step 7 runs `git grep -n "PR 5" -- crates docs/operator scripts` and expects no line that still describes PR 5 as future work).
  - Not PR 5, as the brief says: real adapter evidence, a remote-provider restore, the network HF canary, operator approval ("Deferred").
- **Placeholders.** Each migration is written out. New logic is written out (`decide_new_receipt_route`, `new_receipt_refusal_in`, the ownership insert, `claim_legacy_receipt`, the route's decision, `lock_runnable_policy`, `attestation_hash`, the gate, the fence predicate). Port steps name the port file and its line ranges and say what changes. Tests given as assertion lists follow the recorded owner decision. Values that are confirmed during execution, each with its command: the primary key constraint name (Task 9) and the advisory lock seed (Task 2), the per-check package table (Task 7), the eight `EXPLAIN` plans (Task 6), and the choice of bundle B (Task 9). Three steps tell the implementer to stop and ask instead of choosing: a missing grant on a table of `main` (Task 6), a sequential scan that needs an index on a table of `main` (Task 6), and a server-wide role collision (Global Constraints).
- **Type consistency.** `RoutingState` (Task 2) is the payload of `PipelineReceiptResult::NotRouted` (Task 3) and the input of `decide_new_receipt_route` (Tasks 2, 4). `pipeline_routing_lock` with seed 2 is taken exclusively in `lock_routing` (Task 2) and shared in `new_receipt_refusal_in` (Task 3). `write_routing_in` (Task 2) is what `activate_tenant` and `rollback_bundle` call (Task 9). `lock_runnable_policy` (Task 5) and the gate's `FOR SHARE` count (Task 9) lock the same rows. `VerifiedEvidence` (Task 8) feeds `evaluate_promotion` in the routes (Task 10) and `qualify_bundle`'s `verified` parameter. The three check ids are the same strings in Tasks 4, 6, 9, and 12. `validate_actor` lives in `versioned_pipeline.rs` after Task 5 and is used by Tasks 2, 5, and 9.
- **Review Focus.** Each of the five lines names its tests, and each test is in the task that owns the code. The `main` integration table has one row for each class of site in the inventory, and Task 4 Step 1 repeats the sweep.
- **Scope.** The plan covers one delivery PR with 13 working tasks and four migrations. Tasks 5, 8, and 11 each own one migration or one module and can be cut into a second PR without a change to the others (question 2).

## Questions for the owner (answered on 2026-10-02; see "Owner decisions")

1. **The legacy drain (P5-D8).** Is the list of counts right for "actual pending legacy work": PII backstop, gate decision (pending and exhausted), quarantine review, vector index, delayed credit, revocation propagation, and the tenant-wide legacy NEAR outbox? Should the tenant-wide NEAR outbox count block `drained`? Retention, export jobs, benchmark and process-evaluation work, and the DB mirror backfill are not counted.
2. **Size.** PR 5 as planned has 13 working tasks and four migrations. Do you want one PR, or a second PR for Task 8 (signed results), Task 11 (rebuild fence), and Task 5 (policy suspension)? My recommendation: one branch and one PR, because the reviewer was told that these items are PR 5; the three tasks are the cut lines if the review gets too large.
3. **One qualification for each code revision (P5-D10).** V107 allows one qualification for each bundle, so after a new deploy no earlier bundle can pass the gate, and a rollback is not possible. Options: V112 widens the key (planned); the PR 4 session changes V107; or the key stays and the runbook says "contain, then qualify a new bundle".
4. **The deployed code revision (P5-D17).** The qualified revision is `pipeline.py`'s tree hash; the binary knows only its commit. Planned: a build-time variable `TRACE_COMMONS_BUILD_CODE_REVISION_HASH`, with `pipeline.py revision` to compute it, and a refusal when it is missing. Is a new build-time variable acceptable for the release build?
5. **CI (P5-D19).** `pipeline qualification and restore` stays not required and runs the three new checks. Do you want to promote it to a required check with PR 5? (Then README and CLAUDE.md change from eleven to twelve required checks, and a maintainer edits branch protection.)
6. **Routing read (P5-D4).** With a runtime injected, every new upload of every tenant reads the routing row, so a replica that lacks an activated tenant in its list refuses with `503` and does not use the legacy path. The cheaper option reads the row only for tenants on the receipts list and keeps today's fallback. Which one?
7. **Routes and grants (P5-D11).** The admin routes run in ingest, so the runtime role gets `UPDATE (bundle_id, selected_at)` on `pipeline_active_bundles`, which V93 withheld on purpose. The other option is an operator binary with its own database role. And: is `pipeline_activation_events` enough as the audit record, or do you want a new action in `main`'s audit log?
8. **`deactivate` (P5-D6).** The port has no action that returns a tenant to the legacy path. The plan adds one. Do you agree?
9. **`terminate` and the suspension specification (P5-D12).** PR 5 ships `suspend` and `resume` and refuses `terminate`. CMP-003 asks for a follow-up specification before production use; that is a contract PR. Do you want that PR opened now?
10. **Test-only unqualified routing (P5-D5).** A tenant with no routing row routes to the pipeline only when the process started with `TRACE_COMMONS_PIPELINE_ALLOW_TEST_DEPENDENCIES`. Is this rule acceptable, or should every test harness write a routing row?
11. **Items for other PRs.** Reported as TraceCommons/trace-commons#1186 on 2026-10-02 (four findings where legacy code can read or act on a pipeline-owned submission; the fifth was already #1185 L1-2). Open: do you want the PR 3 follow-up session to take #1186, and is delayed utility credit for a pipeline submission `main`'s to issue?
12. **Signing key (P5-D13).** Who holds the check signing key in the pilot: the CI system, or the operator who runs the qualification? The plan only requires that it is not a holder of the admin credential.
