# Versioned pipeline PR 4 (qualification, restore, and CI) implementation plan

> Amended 2026-10-01: main took V101 to V104 and PR 3 moved to V105 and V106, so PR 4's migrations are V107 (qualification) and V108 (attempt artifacts). The text below keeps the numbers it was written with.
> Amended 2026-10-01: after PR 3's df1550dd, the required check pipeline_orphan_sweep is emitted by a_crashed_score_attempt_leaves_staged_objects_the_sweep_removes.
> Amended 2026-10-01 (owner option D): a compatibility Score stages its `pipeline_attempt_artifacts` rows without a hash before PR 3's tenant Score lock, and its commit sets the hash; V108's grant is `UPDATE (state, committed_at, ciphertext_sha256)`; the sweep deletes a no-hash row's object at the key it recomputes from the row's artifact, run and lease token, and only when the row names that key; a compatibility Score publishes nothing past four Score leases after its rows are staged. This replaces the staging and the grant P4-D10 describes for those rows.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build delivery PR 4 (`vp/pipeline-qualification`), stacked on PR 3 (`vp/pipeline-compatibility-product`): one tooling entry point (`scripts/operator/pipeline.py` with `test`, `run`, `qualify`, and `restore-drill`), executed check results, mandatory database checks, a restore drill, qualification for each bundle, lease renewal, orphan cleanup, index rebuild, and the CI job and runbooks. Production routing stays off.

**Architecture:** Python owns setup, subprocesses, cleanup, and result validation. Rust owns policy, authorization, effects, and assertions. Each mandatory database check is one Rust test. The test runs against a fresh database in a digest-pinned PostgreSQL container that `pipeline.py` starts, and it writes a `trace_commons.pipeline_check_result.v1` result only after its assertions pass. `run` and `restore-drill` drive the shared ingest app (`pipeline_runtime::run_pipeline_app`) in-process on a loopback port, with test dependencies injected through `IngestPipelineRuntimeAssembler`, the harness that PR 2 and PR 3 use. Qualification inspects the objects that the constructor receives for one bundle, and ignores unrelated installed dependencies.

**Tech Stack:** Rust (axum, tokio, tokio-postgres, deadpool-postgres, serde, ring, base64, clap, hf-hub; all are existing dependencies), PostgreSQL 16 with forced RLS, the existing encrypted artifact store, Python 3 standard library only, Docker, the PostgreSQL client tools (`psql`, `pg_dump`, `pg_restore`), GitHub Actions.

**Spec:** `docs/superpowers/plans/2026-09-22-versioned-pipeline-implementation-brief.md` on local `stack/07-pipeline-activation` (binding; read with `git show stack/07-pipeline-activation:<path>`): section 3A (the qualification row), section 4 (one entry point, shared environment, one corpus path, executed result contract, command transition), section 6 (delivery row 4; each delivery PR carries its own migrations), section 7 (gates). Also read: `docs/superpowers/specs/2026-09-14-versioned-pipeline-package-qualification.md` (on `main`), the PR 3 plan `docs/superpowers/plans/2026-09-25-versioned-pipeline-pr3-compatibility-product-plan.md` (untracked in the main checkout; this plan uses its form), and the ledgers in `.superpowers/sdd/` (PR 2: `2026-09-23-versioned-pipeline-pr2-runtime-plan/`, PR 2 review: `2026-09-27-pr7-review1/`, PR 3: `2026-09-25-versioned-pipeline-pr3-compatibility-product-plan/`).

**Port source:** `ef97a459` (origin `stack/07-pipeline-activation`). Read a port file with `git show ef97a459:<path>`.

**Scope:** PR 4 only. PR 5 (activation) gets its own plan. "Deferred to PR 5" lists what PR 4 must not port.

**Test bodies:** as in PR 2 and PR 3 (owner decision recorded in the PR 2 ledger), a test given as a list of assertions is written by the implementer as real calls. Every listed assertion is required.

## Owner decisions (2026-09-29, plan approval)

The owner approved this plan on 2026-09-29 and answered its three questions:

1. **Required check (P4-D17):** `pipeline qualification and restore` is **not** a required status check at the merge of PR 4.
2. **HF pin location (P4-D15):** yes, the pin file under `docs/superpowers/specs/` is a contract text change. It goes to `main` as its own small upstream PR (flow rule 1). PR 4 does not add it.
3. **Run shape (P4-D3):** confirmed. `run` and `restore-drill` host the shared app inside an ignored Rust test that `pipeline.py` starts. There is no launcher binary and no new cargo feature.

## Preconditions (not tasks of this plan)

1. The owner approves this plan (end of phase A). Done on 2026-09-29.
2. The PR 3 session rebases `vp/pipeline-compatibility-product` onto `main` one time, and the owner says that this is done. Done: PR 3 is published as TraceCommons/trace-commons#1143; the owner started phase B on 2026-09-30.
3. Phase B: create the worktree (brief command), record the PR 3 head as `BASE_PR4` in the ledger, check the migration numbers (P4-D2), and make the first commit: this plan as `docs/superpowers/plans/2026-09-29-versioned-pipeline-pr4-qualification-plan.md`. `BASE_PR4` = `4e7a6990` (PR 3 head, equal to `origin/vp/pipeline-compatibility-product`, containing `upstream/main` `ad9f3ac6`). PR 3 is published, so a later sync uses `origin/vp/pipeline-compatibility-product` (common.md sync steps).

## Task order and the PR 3 dependency

Do the **needs PR 2 only** tasks first. Do a **needs PR 3** task only after the owner confirms that PR 3's review has not changed the PR 3 code that the task uses (the last column), or after PR 4 is rebased onto the changed PR 3.

| Task | Needs | PR 3 code the task uses |
| --- | --- | --- |
| 1 Check result contract and package trust | PR 2 only | none |
| 2 Lease renewal | PR 2 only | none (it wraps `process_claimed_run`; it does not change its body) |
| 3 `pipeline.py` foundation and `test` | PR 2 only | the `test --check postgres` list names two PR 3 tests; Task 11 checks the list again |
| 4 HF corpus export | PR 2 only | none |
| 5 Qualification for each bundle (FR5 P4) | PR 3 | `PipelineDependencyQualification`, `with_authority`, `with_privacy`, `with_payout`, `payout_enabled`, `pipeline_runtime_is_production_qualified` (Ruling T2-2) |
| 6 Qualification storage and store | PR 3 | V101 and V102 numbers, `pipeline_upgrade_tests.rs` pins, `operational_summary` table list (Ruling T13-1), the compatibility implementation ids |
| 7 Attempt artifacts and orphan cleanup | PR 3 | `commit_review` and `commit_score_phase` write sites, `withdraw_submission`, `drain_pipeline_tenant` |
| 8 Index rebuild | PR 3 | guard predicate (P3-D9), `index_invalidation_state`, `load_index_command` |
| 9 Corpus run harness and `run` | PR 3 | review routes, status route, authority and privacy test doubles, `compatibility_package`, the HTTP test assembler |
| 10 Restore drill | PR 3 | Task 9 harness, Task 8 rebuild |
| 11 Results in existing suites and `qualify` | PR 3 | the test names in the check table |
| 12 CI job and runbooks | PR 3 | `docs/operator/pipeline-activation.md` as PR 3 leaves it |
| 13 PR 4 gate | PR 3 | everything |

Migration numbers (checked in phase B, 2026-09-30): `upstream/main` (`ad9f3ac6`) holds V95, V97, V98, and V100; open upstream PRs reserve V96 (#1121) and V99 (#1127); PR 3 (#1143, head `4e7a6990`) holds V101 and V102. No open upstream PR claims V103 or V104, so PR 4 takes **V103** (qualification) and **V104** (attempt artifacts). At each rebase of PR 4, check again: if `main` or PR 3 took either number, take the next two free numbers after PR 3's, and rename the files, the registry entries, the upgrade test's expected version, and the grant pins together. Write the numbers in the ledger.

## Global Constraints

- Base: `BASE_PR4` (precondition 3). Changes flow down the stack only (common.md flow rules). A contract change (a type or trait in `crates/trace-commons-gate-api`, or text in `docs/superpowers/specs/`) goes to `main` as its own small upstream PR. A defect in PR 3 code is fixed on PR 3. A session that finds work for another PR writes it down in the ledger and tells the owner; it does not make the change on `vp/pipeline-qualification`.
- PR 4 is rebased onto PR 3 after each PR 3 review round, between two tasks, never in the middle of a task. When PR 3 is squash-merged, PR 4 is rebased onto `main` one time.
- Port source: `ef97a459` only. Never port from `stack/02` to `stack/06`, or from `16b73b2d`.
- Every new `.rs` file in `trace-commons-server`, `trace-commons-gate-api`, or `trace-commons-gate-enclave` starts with `// Copyright (C) 2026 K&Z Partners LLC` and `// SPDX-License-Identifier: AGPL-3.0-or-later`.
- No new third-party dependency. Rust uses only existing dependencies. Python uses only the standard library (no `psycopg`, no `pytest`). Do not edit the expected sets in `tests/license_boundary.rs`.
- Operational output is hash-only or label-only. New labels, including every check id, match `^[a-z0-9_]{1,64}$`. No raw tenant id, principal, URL, token, key, or trace body in a log line, an error string, a stored row, a result, an evidence file, or a report. `pipeline.py` never prints a child's output; it prints the step label, the exit code, and the protected log path.
- Every hashed JSON value goes through `trace_commons_protocol::canonical_json::to_canonical_vec` (or `canonicalize`). The server's build graph has `serde_json/preserve_order` on (through `dcap-qvl`'s `std` feature; see `crates/trace-commons-protocol/src/canonical_json.rs`), so `serde_json::to_vec` of a `Value` keeps insertion order and is not canonical. Python's canonical form (`json.dumps(sort_keys=True, separators=(",", ":"), ensure_ascii=False)`) matches `to_canonical_vec` for the ASCII keys this work uses. Found on 2026-09-29: the port's corpus export hashed insertion-ordered JSON.
- Every new table: `ENABLE` and `FORCE ROW LEVEL SECURITY`, policy `trace_corpus_tenant_isolation` with `tenant_id = trace_current_tenant_id()`, explicit grants to `trace_ingest_runtime` on V92's terms (what the code reads and writes, nothing broader), and an append-only trigger with the `pg_trigger_depth() > 1` cascade exception where rows are immutable (copy the V101 comment and function shape). No general RLS bypass. No `SECURITY DEFINER` function. No cross-tenant claim (PR 2 decision D2).
- Migrations V103 and V104 belong to PR 4. They apply as a non-superuser `CREATEROLE` owner (#974): no function-level `SET`, no `ALTER ROLE ... SUPERUSER/BYPASSRLS`, grants after memberships. `migration_atomicity_pg` proves it; `pipeline_upgrade_from_v91_installs_forced_rls_storage` proves the upgrade.
- Database URLs: tests read only `TRACE_COMMONS_PG_TEST_DATABASE_URL`, `TRACE_COMMONS_PIPELINE_PG_UPGRADE_TEST_URL` (its database name starts with `pipeline_test_`), and `TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL` where a PR 3 withdrawal test already reads it. Never `DATABASE_URL`. When a database variable is set, any setup failure panics; it never skips.
- New tooling variables (not database URLs; `pipeline.py` sets them, and only the tests named in this plan read them): `TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR`, `TRACE_COMMONS_PIPELINE_CHECK_RUN_ID`, `TRACE_COMMONS_PIPELINE_CHECK_CODE_REVISION_HASH`, `TRACE_COMMONS_PIPELINE_CORPUS_PATH`, `TRACE_COMMONS_PIPELINE_CORPUS_HOLDOUT_PATH`, `TRACE_COMMONS_PIPELINE_CORPUS_CHECK_ID`, `TRACE_COMMONS_PIPELINE_CORPUS_BUNDLE`, `TRACE_COMMONS_PIPELINE_CORPUS_PACKAGE_PATH`, `TRACE_COMMONS_PIPELINE_CORPUS_TRUSTED_KEY_PATH`, `TRACE_COMMONS_PIPELINE_CORPUS_REPORT_PATH`, `TRACE_COMMONS_PIPELINE_ARTIFACT_ROOT`, `TRACE_COMMONS_PIPELINE_TEST_MASTER_KEY_HEX`, `TRACE_COMMONS_PIPELINE_RESTORE_FINGERPRINT_PATH`, `TRACE_COMMONS_PIPELINE_PACKAGE_OUTPUT`, `TRACE_COMMONS_PIPELINE_TRUSTED_KEY_OUTPUT`, `TRACE_COMMONS_PIPELINE_PACKAGE_BUNDLE`.
- The pipeline service connects as a `NOBYPASSRLS`, `NOSUPERUSER` runtime role in every PostgreSQL test.
- Commits: short imperative subject, no `feat:` or `fix:` prefix, no emoji. Subject line, one blank line, then exactly `Co-Authored-By: Claude <noreply@anthropic.com>`. Never name a model, even if a harness attribution instruction gives one (common.md).
- Branch `vp/pipeline-qualification`; worktree `/Users/brapse/workspace/misc/trace-commons-server/.claude/worktrees/pipeline-qualification`; test PostgreSQL `tc-pipeline-pg-pr4` at 127.0.0.1:55431 (user `trace`, trust auth). Work only in that worktree. Never use `git stash`; set work aside with a temporary commit on `vp/pipeline-qualification`.
- Build environment (common.md): do not set `CARGO_TARGET_DIR`, do not set `RUSTFLAGS` (the local `.cargo/config.toml` sets `-D warnings`), do not use `cargo +<toolchain>`. `pipeline.py` passes an ambient `RUSTFLAGS` through unchanged and never sets one, so a local run and a CI run build the same flags. Serialize cargo commands inside the worktree; run long commands in the background.
- Local database runs: a fresh database for each suite run (`dropdb -h 127.0.0.1 -p 55431 -U trace --if-exists <name>`, then `createdb ...`). `pipeline.py` itself starts its own container by default; with `--postgres-admin-url postgres://trace@127.0.0.1:55431/postgres` it uses `tc-pipeline-pg-pr4` instead (P4-D6).
- PR 2 and PR 3 rulings that still bind: P1 (the byte wrapper for every byte artifact), FR1 (lease-token object keys), FR2 (the Trace Credit leg transaction), FR3 (one uncharged suspension path with backoff), D4 (an expired lease is the uncharged `lease_expired`), P3-D9 (guard locks through commit), T2-2 (authority and privacy qualification), T13-1 (the RLS names in `operational_summary`).
- Checks before a task is complete (brief section 7), run from the worktree root:

```bash
cargo fmt --all -- --check
cargo check -p trace-commons-server --bins
cargo test -p trace-commons-server --no-run
cargo clippy -p trace-commons-server --all-targets -- -D warnings -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
cargo test -p trace-commons-server --test license_boundary
```

  A task that touches Python also runs `python3 -m py_compile scripts/operator/pipeline.py scripts/operator/pipeline_tooling/*.py scripts/operator/pipeline-deployment-inventory.py` and `python3 scripts/operator/test_pipeline_tooling.py`. `--no-run` is not test execution; each task names the tests it must execute.

## Review Focus

These five conditions are the ones most likely to hurt a person who uses this software. Each has a test in the task that owns the code.

1. **A qualification run that passes without running a check.** A test filter that matches nothing, a Rust test that self-skips because a variable name is wrong, a result file left from an earlier run, or a result for another code revision must fail `qualify`. Tests: `test_results_reject_missing_stale_foreign_and_tampered` and `test_zero_match_filter_fails` (Task 3), `test_qualify_fails_when_a_required_check_emits_nothing` (Task 11).
2. **Cleanup that hides a failure or leaks a container.** An earlier failure keeps its exit code when cleanup also fails; a leaked container or process fails an otherwise green run. Test: `test_cleanup_failure_fails_a_passing_run_and_keeps_an_earlier_failure` (Task 3).
3. **Secret or trace text in bounded output.** No fixture `secret_probe`, token-shaped value, e-mail address, or trace text appears in a result, an evidence file, a corpus report, the qualification report, or `pipeline.py` terminal output. Tests: `test_evidence_validator_refuses_secret_like_values` and `test_failure_output_is_label_only` (Task 3), the probe assertions in `pipeline_corpus_run` (Task 9), `test_report_contains_no_private_fields` (Task 11).
4. **A restore that duplicates an effect or loses the RLS boundary.** After restore and restart, the pending run completes once, completed legs are not dispatched again, no second outcome or credit event exists, and the resumed service still connects as a `NOBYPASSRLS` role on tables that force RLS. Test: `pipeline_restore_resume` (Task 10).
5. **Lease renewal that keeps a hung worker alive or renews a lost lease.** Renewal stops at the cap, and a worker whose lease another claim took never extends it. Tests: `renewal_stops_at_the_lease_cap` and `a_lost_lease_is_never_renewed` (Task 2).

## Decisions made while writing this plan

Executors: copy each line into the PR 4 ledger as `Plan: Ruling: ...` at setup. Each is binding for this plan. The cost line says what changes if the owner disagrees.

- **P4-D1. Branch and order.** One local branch, `vp/pipeline-qualification`, created from `BASE_PR4` in phase B. Tasks 1 to 4 need PR 2 only and run first; Tasks 5 to 13 need PR 3 (table above). Nothing is pushed until the owner decides to publish. Cost if wrong: none.
- **P4-D2. Migrations.** V103 `versioned_pipeline_qualification.sql` holds `pipeline_bundle_qualifications` (port V79, with a cascade foreign key and the V101 trigger shape instead of `RESTRICT` and an unconditional delete refusal, so a tenant with a qualified package can still be deleted). V104 `versioned_pipeline_attempt_artifacts.sql` holds `pipeline_attempt_artifacts` (P4-D10). No PR 5 table. The numbers are working names (see "Task order and the PR 3 dependency"). Cost if wrong: move one table between two new files.
- **P4-D3. Run shape (owner confirmed 2026-09-29).** There is no local launcher binary and no new cargo feature. `pipeline.py run` and `restore-drill` start `cargo test -p trace-commons-server --bin trace-commons-ingest <ignored test> -- --ignored --exact`. The test builds `AppState` the way the PR 2 and PR 3 real HTTP tests do, injects test dependencies through `IngestPipelineRuntimeAssembler`, serves `run_pipeline_app` on a loopback port, and drives it through real HTTP with production auth and static test credentials. Reason: the stock binary injects no runtime (PR 2 decision D3), and a second binary cannot include `trace-commons-ingest.rs` because its modules use `crate::` paths (`rewards.rs`). Cost if wrong: add a feature-gated launcher (`pipeline-local`, `required-features`) that calls `run_ingest(Some(&LocalTestAssembler))`, plus a Rust corpus client; the assertions and the result contract stay.
- **P4-D4. Result contract.** The result schema is exactly the brief's JSON object (`trace_commons.pipeline_check_result.v1`, eleven fields, no others). Each check id is a label (common.md: `^[a-z0-9_]{1,64}$`, so the brief's dotted example becomes `pipeline_settle_crash_matrix`). One Rust test emits one check id, after its assertions. Beside `<check_id>.result.json` it writes `<check_id>.evidence.json`, the bounded observed values; `evidence_hash` is the SHA-256 of that file's canonical JSON, and Python recomputes it and runs the evidence validator on it. The emitter is library code (`PipelineCheckEmitter`, `#[doc(hidden)]`) in `versioned_pipeline_qualification.rs`, so lib, integration, and ingest tests share it; it does nothing when the three check variables are unset, and it panics when only some are set. Cost if wrong: move the emitter into `tests/support/`.
- **P4-D5. Required check ids.** `qualify` requires the database checks in Task 11's table, the corpus checks `pipeline_http_corpus_minimal`, `pipeline_http_corpus_compatibility`, `pipeline_http_corpus_hf_local`, and `pipeline_restore_drill`. Rust holds `PROMOTION_REQUIRED_CHECKS`: the CI set plus `pipeline_production_adapters`, `pipeline_remote_restore`, and `pipeline_hf_network_canary`, which no local or CI run can pass. PR 5 adds `pipeline_activation_containment`, `pipeline_activation_rollback`, and `pipeline_legacy_drain` to both lists. A Python test keeps the CI list a subset of the Rust list. Cost if wrong: edit two lists.
- **P4-D6. Environment and the fixed test role names.** `pipeline.py` starts one container per command from `postgres:16@sha256:a3b7f434b2dc57ce85a67e171163eb8ab1a1ebcb39d27484661f26b1dfbe30d6` (the local `postgres:16` repository digest on 2026-09-29; Task 3 confirms it is the multi-architecture index digest), with trust auth on a loopback port that Docker allocates, as the CI service does. Scenarios run one after another, each with its own databases and artifact root; the environment creates the login-resolver roles once, as CI does. With the explicit `--postgres-admin-url`, it uses that server and holds a lock database (`pipeline_tooling_lock`) for the whole command, so two `pipeline.py` commands on one server stop instead of colliding. This settles the PR 2 review minor (Task F2): the suites keep their fixed cluster-global role names, and the tooling never runs two suites on one cluster at once. It never reads `DATABASE_URL` and never passes it to a child process. Cost if wrong: suffix the two test login names with the database name in two harnesses.
- **P4-D7. Qualification for each bundle (PR 2 FR5 P4).** `PipelineService::bundle_qualification(&BundlePackage)` resolves the scorer and embedder with the same function that construction uses (`resolve_score_dependencies`, extracted from `construct`), and reports only the dependencies that bundle uses: that scorer, that embedder, the index reader and writer, the settlement adapter of each instrument the package pins, the authority provider and privacy boundary (receipt controls, Ruling T2-2), and the payout adapter only when payout is enabled and the package pins `trace_credit`. Other held scorers, embedders, and adapters are ignored (spec 3A). Startup qualification (`pipeline_runtime_is_production_qualified`) checks the default package this way. Runs bound to an older package are not checked again at load; PR 5 activates only qualified packages. Cost if wrong: return to the whole-runtime check.
- **P4-D8. Qualification store scope.** Port from `versioned_pipeline_qualification.rs`: the signed package and trust store, `validate_production_package` (known ids become the four PR 3 compatibility ids; the minimal family is test-only), `ProductionDependencyProfile` (built from `bundle_qualification`, not from whole-runtime flags), `BundleQualificationMetadata`, `PipelineQualificationStore::qualify_bundle`, and promotion evaluation. Not ported: `activate_qualified_bundle` (it writes `pipeline_active_bundles` and reads the operational status; PR 5), an admin route that records a qualification from the server's own profile (PR 5, with its only reader), and a derivation of `ProductionInfrastructureProfile` from ingest configuration (PR 5). Cost if wrong: one route in PR 4.
- **P4-D9. Lease renewal (PR 2 final review M7, D4 owner note).** PR 4 implements it. While a phase runs, a task renews the claim's lease every third of the phase lease (at least every 100 ms), with a fenced update (`lease_token` equal, lease not yet expired, never shortened). Renewal stops at a cap of four phase leases after the claim, when the phase ends, or when the lease is lost. The existing commit fences stay the only authority. Reason: with two or more ingest replicas, a Score that always runs longer than its lease reaches `attempts_exhausted` today, and PR 5 puts several replicas in front of real receipts. Cost if wrong: remove Task 2; operators must set the Score lease above the slowest Score.
- **P4-D10. Orphan cleanup (P3-D15, PR 3 Task 6 and Task 7 notes).** The port's `list_cleanup_orphans` lists staged receipts only, which PR 2's `sweep_staged_receipts` already removes; it is not ported. The three attempt objects (`approved`, `index-command`, `score-neighbors`) get rows in `pipeline_attempt_artifacts`: staged in their own transaction before the object is published (the PR 2 receipt pattern), marked committed inside the phase commit, and swept by the worker: a staged row past `cleanup_after` (four phase leases plus one hour) loses its object and its row. That covers a crash, a stale worker, and a refused commit, including Score objects after a refused Score commit. A committed `index-command` or `score-neighbors` object of a withdrawn submission is deleted when its run is terminal and `index_invalidation_state` is `none` or `complete`; the row moves to `deleted`. The approved object keeps the existing withdrawal path through `trace_object_refs`. Cost if wrong: drop V104 and Task 7; orphan objects stay, as PR 2 accepted. **Amended 2026-09-30 (ruling R2-1, after PR 3's `e54ac541`):** PR 3 now records Score's `index-command` and neighbour objects as object refs of the submission, the withdrawal invalidates them and queues their deletion, main's revocation-propagation worker deletes them, and a refused Score commit deletes the objects it wrote. So deleting a withdrawn submission's objects is owned by PR 3 and main, not by this sweep: selection (b) and the `deleted` row state are removed, V104's grant is `UPDATE (state, committed_at)`, and the sweep keeps only staged rows past `cleanup_after` (crashed, stale, and refused attempts; a refused attempt's objects are already gone, so the sweep drops its rows).
- **P4-D11. Index rebuild (P3-D15).** Port `list_rebuildable_index_runs` and `rebuild_index_from_authoritative_commands` onto PR 2 and PR 3: the runs are complete, included, `index_write_state = 'complete'`, `index_invalidation_state = 'none'`, with an operable submission under the PR 3 guard predicate; each command loads through `load_index_command` and is written with Settle's own entry keys. A worker route `POST /v1/workers/pipeline/index-rebuild` behind the vector worker credential runs it with the service's own index writer and returns a hash-only report. Cost if wrong: drop the route; the runbook then says rebuild waits for PR 5.
- **P4-D12. `default_package_hash`.** Not ported. Its only port user was the local binary's readiness handler, which does not exist on `main`. PR 4 adds `PipelineService::default_package()` for startup qualification, and check results carry the package hash (`package_digests`). `/v1/pipeline/readiness` stays unauthenticated and label-only; it does not list production blockers. Cost if wrong: one accessor.
- **P4-D13. Old commands.** `main` has none of the port's shell runners, `scripts/operator/lab/`, or `run-cargo-test-filter.sh`. PR 4 does not add them; they are logic sources for `pipeline.py`. The zero-match guard lives in `pipeline_tooling/cargo.py`. The existing `postgres-suites` pipeline steps stay; the brief allows their removal only after a maintainer checks coverage and required check names. Cost if wrong: add thin wrappers.
- **P4-D14. Inventory lint.** Port `pipeline-deployment-inventory.py` without the contract-manifest check and without the `trace-commons-pipeline-local.rs` component. On the PR 3 tree it classifies all 217 routes and 110 tables. The manifest check fails on `main`'s manifest (test ids such as `versioned_pipeline_pg::...` do not exist); refreshing those ids is a change to `docs/superpowers/specs/`, so it is written down for an upstream PR (flow rule 1). Cost if wrong: re-enable one function after that PR.
- **P4-D15. HF pin descriptor (owner decision 2026-09-29, reversed 2026-09-30: no separate PR).** The network pin goes to `docs/superpowers/specs/versioned-pipeline-hf-corpus-v1.json` through a small upstream PR (flow rule 1). PR 4 carries the local loader fixtures and their own descriptor under `crates/trace-commons-server/tests/fixtures/pipeline-hf-jsonl/` (tests, not specs), so CI never needs the network or the upstream PR. `run --corpus <pin>` accepts any pin descriptor path. The owner first confirmed the upstream PR route (2026-09-29). Found on 2026-09-29 while drafting that PR: the dataset `jedisct1/agent-traces-swival` now redirects to `jedisct1/security-audits` (revision `6d527ff0` exists there), and the port pin's `configuration_digest` is an insertion-order hash. **Amended 2026-09-30 (owner):** pinning a dataset is trivial for now, so there is no separate upstream PR; the network pin is folded into the first existing PR that needs the dataset. PR 4 does not add it: every PR 4 check and the CI job use the local fixtures. The verified content (dataset name `jedisct1/security-audits` and the five digests from the Task 4 network export) is kept in the PR 4 ledger as `hf-pin-final.json`.
- **P4-D16. Corpus fixture expectations.** The harness reads `main`'s `versioned-pipeline-minimal-corpus-v1.json` unchanged (the #971 text) and derives missing expectations: privacy state from `privacy_risk`; scoring and settlement `complete` when `expected_outcome_count` is 4, else `missing` and `incomplete`; consent `allowed`; instrument count equal to the bundle's award count when the outcome count is 4, else 0. Cost if wrong: a small upstream PR that adds the port's explicit fields.
- **P4-D17. CI job (owner decision 2026-09-29: not required).** New job `pipeline-qualification`, name `pipeline qualification and restore`, in `.github/workflows/ci.yml`, which already triggers on `pull_request` and `merge_group`. It runs the tooling self-tests and `python3 scripts/operator/pipeline.py qualify`, and ends with `./.github/actions/trim-cargo-cache`. The owner decided that it is not required at the merge of PR 4. A later promotion is a separate owner decision (then README and CLAUDE.md change from eleven to twelve required checks, and a maintainer edits branch protection).
- **P4-D18. Credentials in harnesses.** Production auth with static tenant tokens that the test state configures (as the PR 2 and PR 3 HTTP tests do), the review credential for review routes, the admin credential for forensic reads, and the vector worker credential for the rebuild route. The test artifact master key is random for each `pipeline.py` command and reaches only the child environment (`TRACE_COMMONS_PIPELINE_TEST_MASTER_KEY_HEX`), so the restore drill can decrypt across two processes. Cost if wrong: none.
- **P4-D19. Catalog.** Port the lab catalog (`trace_commons.pipeline_lab_catalog.v1`, `.local/pipeline-lab-catalog.json`, content-addressed `lab-records/`) into `pipeline_tooling/catalog.py`. Only `qualify --archive` and `run --archive` write it. Routine runs write the fresh run directory and the latest bounded report under `.local/` only. Cost if wrong: none.
- **P4-D20. `.local/`.** `/.local/` goes into `.gitignore`: evidence, logs, and dumps live there, and today only this checkout's `.git/info/exclude` hides it. Cost if wrong: one line.
- **P4-D21. RLS names (PR 3 Ruling T13-1).** Tasks 6 and 7 add `pipeline_bundle_qualifications` and `pipeline_attempt_artifacts` to `TRACE_COMMONS_RLS_TABLES`, to `PIPELINE_TABLES` in the upgrade test, and to the `operational_summary` RLS list. The query form (`COUNT(*) = 0 ... relname = ANY($2)` passes for a misspelled name) is PR 3 code: if PR 3's final review did not change it, write it down for PR 3. Cost if wrong: none.

## Deferred to PR 5 (and later)

Do not port these in PR 4. A task that needs one of them has a defect in this plan; stop and ask.

| Item (port source symbol or file) | Goes to |
| --- | --- |
| `activate_qualified_bundle`, the qualified gate on `pipeline_active_bundles`, an admin route that records a qualification from the server's runtime profile, `ProductionInfrastructureProfile` from ingest configuration | PR 5 |
| Activation, routing store, receipt ownership, containment, rollback, legacy drain (`versioned_pipeline_activation.rs`, port `V80__versioned_pipeline_activation.sql`) | PR 5 |
| Policy interventions (`intervene_policy`, `list_policy_interventions`, `pipeline_policy_interventions`, `PolicyOperationalStatus`, `operational_status`) | PR 5 |
| Check ids `pipeline_activation_containment`, `pipeline_activation_rollback`, `pipeline_legacy_drain`, and the PR 5 table names in `operational_summary` | PR 5 |
| Real adapter evidence, remote-provider restore, the pinned network HF canary, operator approval | Promotion (operator work, not a PR) |
| Removal of overlapping `postgres-suites` pipeline steps | After a maintainer checks coverage (brief section 4) |

## Items for other PRs (write down, tell the owner)

- Upstream contract PR: refresh the test ids in `docs/superpowers/specs/2026-09-11-versioned-pipeline-contract-test-manifest.json` (P4-D14).
- HF network pin (P4-D15): no separate PR (owner, 2026-09-30). The first PR that needs the network dataset carries it, using the verified content in the PR 4 ledger's `hf-pin-final.json`.
- PR 3: the `operational_summary` RLS query form (P4-D21), if not already changed.
- `main` (pilot bootstrap, not the pipeline): `auto_detect_translator` in `crates/trace-commons-server/src/bin/pilot_bootstrap/mod.rs` matches only `jedisct1/agent-traces-swival`; with the dataset renamed to `jedisct1/security-audits`, `--source jedisct1/security-audits` does not select the swival translator. Found 2026-09-29.
- PR 3: only if Task 7's test shows it, an approved object that survives a pipeline withdrawal.

## File map

| File | Responsibility | Tasks |
| --- | --- | --- |
| `crates/trace-commons-server/src/versioned_pipeline_qualification.rs` (new) | Check result, emitter, package digests, promotion, package trust, production profile, qualification store | 1, 6 |
| `crates/trace-commons-server/src/lib.rs` | `pub mod versioned_pipeline_qualification;` | 1 |
| `crates/trace-commons-server/src/versioned_pipeline.rs` | Lease renewal; bundle qualification; attempt artifacts and sweep; rebuild | 2, 5, 7, 8 |
| `crates/trace-commons-server/src/versioned_pipeline_index.rs` | `IsolatedPipelineIndex::entry_set_hash` | 8 |
| `crates/trace-commons-server/src/versioned_pipeline_product.rs` | RLS names (T13-1) | 6, 7 |
| `migrations/V103__versioned_pipeline_qualification.sql` (new) | Qualification identities | 6 |
| `migrations/V104__versioned_pipeline_attempt_artifacts.sql` (new) | Attempt artifact rows | 7 |
| `crates/trace-commons-server/src/db/postgres.rs` | Migration registry, `TRACE_COMMONS_RLS_TABLES`, static migration tests | 6, 7 |
| `crates/trace-commons-server/src/db/postgres/pipeline_upgrade_tests.rs` | Upgrade to V104, grant pins, result emission | 6, 7, 11 |
| `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_runtime.rs` | Startup qualification; attempt sweep in the drain; rebuild handler | 5, 7, 8 |
| `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` | Rebuild route registration | 8 |
| `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs` | Module declarations; startup qualification tests; artifact store with a fixed key | 5, 9, 10 |
| `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_http_pg_tests.rs` | `pub(super)` helpers; result emission | 9, 11 |
| `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_corpus_pg_tests.rs` (new) | Corpus run harness, package writer | 9 |
| `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_restore_pg_tests.rs` (new) | Restore seed and resume | 10 |
| `crates/trace-commons-server/tests/versioned_pipeline_runtime_pg.rs` | Renewal, qualification, attempt artifact, rebuild tests; result emission | 2, 5, 6, 7, 8, 11 |
| `crates/trace-commons-server/src/bin/trace-commons-pipeline-corpus-export.rs` (new) | HF JSONL to direct corpus | 4 |
| `crates/trace-commons-server/src/bin/pilot_bootstrap/hf_dataset.rs` | `open_at_revision` | 4 |
| `crates/trace-commons-server/tests/fixtures/pipeline-hf-jsonl/` (new) | Local loader fixtures and their descriptor | 4 |
| `scripts/operator/pipeline.py` (new) | CLI: `test`, `run`, `package`, `qualify`, `restore-drill` | 3, 9, 10, 11 |
| `scripts/operator/pipeline_tooling/` (new) | `__init__.py`, `errors.py`, `environment.py`, `cargo.py`, `results.py`, `corpus.py`, `catalog.py`, `checks.py`, `report.py` | 3, 4, 9, 10, 11 |
| `scripts/operator/test_pipeline_tooling.py` (new) | Standard-library self-tests | 3, 4, 9, 10, 11 |
| `scripts/operator/pipeline-deployment-inventory.py` (new) | Inventory lint | 3 |
| `.gitignore` | `/.local/` | 3 |
| `.github/workflows/ci.yml` | `pipeline qualification and restore` job | 12 |
| `docs/operator/pipeline-qualification.md`, `pipeline-lab.md`, `pipeline-activation.md`, `backup-restore.md`, `README.md` | Runbooks and index | 12 |
| `README.md`, `CLAUDE.md` | CI job counts | 12 |

## Interfaces (the names every task uses)

> Amended 2026-10-01 (wave 2 and its fix round 1; the code is authoritative where this section differs): `evaluate_promotion` adds the blockers `qualification_evidence_mixed_revision` and `qualification_evidence_mixed_package`, and `PromotionDecision` gains `code_revision_hash: Option<String>` and `package: Option<PromotionPackage>`; its `evidence_hash` is canonical JSON under the schema string `trace_commons.pipeline_promotion.v2` over the blockers and, per check id, the run id, code revision, three package digests and evidence hash (not the evaluation time). `qualify_bundle(tenant_id, signed, trust, metadata, dependencies, evidence: &[DrillEvidence])` evaluates the evidence itself at the time of the call and reads the package's configuration term itself (`package_configuration_is_qualifiable`); `PipelineBundleQualification` gains `configuration_qualifiable`, and `PipelineDependencyQualification` loses `bundle`. `PipelineService::sweep_attempt_artifacts_from(tenant_id, limit, resume_after) -> AttemptSweepPass` pages past kept rows and resumes where a pass stopped; `sweep_attempt_artifacts` is its `resume_after = None` form. A missing or mismatched attempt row refuses a commit as `pipeline_attempt_artifact_missing`.

`versioned_pipeline_qualification.rs` (Task 1, Task 6):

```rust
pub const PIPELINE_CHECK_RESULT_SCHEMA: &str = "trace_commons.pipeline_check_result.v1";
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)] #[serde(rename_all = "snake_case")]
pub enum PipelineCheckStatus { Pass, Fail, Blocked }
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)] #[serde(deny_unknown_fields)]
pub struct PipelineCheckResult {
    pub schema: String, pub run_id: String, pub check_id: String, pub status: PipelineCheckStatus,
    pub code_revision_hash: String, pub package_hash: Option<String>, pub configuration_digest: Option<String>,
    pub dependency_digest: Option<String>, pub observed_at: DateTime<Utc>, pub evidence_hash: String,
    pub safe_blockers: Vec<String>,
}
impl PipelineCheckResult { pub fn validate(&self) -> Result<(), String>; }
pub struct PipelinePackageDigests { pub package_hash: String, pub configuration_digest: String, pub dependency_digest: String }
pub fn package_digests(package: &BundlePackage) -> Result<PipelinePackageDigests, String>;
pub fn evidence_hash(observed: &serde_json::Value) -> Result<String, String>;
#[doc(hidden)] pub struct PipelineCheckEmitter;
// new(dir: PathBuf, run_id: &str, code_revision_hash: &str) -> Result<Self, String>
// from_vars(dir: Option<String>, run_id: Option<String>, code_revision_hash: Option<String>) -> Result<Option<Self>, String>
// from_env() -> Result<Option<Self>, String>
// emit(&self, check_id: &str, status: PipelineCheckStatus, package: Option<&BundlePackage>, safe_blockers: &[&str], observed: serde_json::Value) -> Result<(), String>
// emit_from_env(check_id, status, package, safe_blockers, observed)   // no-op when unset; panics on a partial set or an emit error
// emit_pass_from_env(check_id, package, observed)                     // emit_from_env with Pass and no blockers
pub const PROMOTION_REQUIRED_CHECKS: &[&str];
pub struct DrillEvidence { pub check: PipelineCheckResult, pub maximum_age_seconds: u64 }
pub struct PromotionDecision { pub ready: bool, pub evaluated_at: DateTime<Utc>, pub evidence_hash: String, pub safe_blockers: Vec<String> }
pub fn evaluate_promotion(evidence: &[DrillEvidence], now: DateTime<Utc>) -> Result<PromotionDecision, String>;
pub struct BundlePackageSignature; pub struct SignedBundlePackage; pub struct TrustedBundleKey; // port lines 76-94
pub struct BundlePackageTrustStore; // new(keys) -> Result<Self, String>, verify(&SignedBundlePackage) -> Result<(), String>
pub fn sign_bundle_package(package: BundlePackage, key_id: &str, pkcs8: &[u8]) -> anyhow::Result<SignedBundlePackage>;
pub fn trusted_key_for_pkcs8(key_id: &str, pkcs8: &[u8]) -> anyhow::Result<TrustedBundleKey>;
// Task 6:
pub enum ProductionAdapterKind { Production, Development, Synthetic, Missing }
pub struct ProductionInfrastructureProfile; // port lines 284-313
pub struct ProductionDependencyProfile; // new(bundle: PipelineBundleQualification, infrastructure) -> Self, for_bundle(service, package, infrastructure) -> Result<Self, String>, runtime_identity_digest() -> Result<String, String>, blockers() -> Vec<String>
pub fn validate_production_package(package: &BundlePackage) -> Result<(), String>;
pub struct BundleQualificationMetadata; pub struct BundleQualificationRecord; // port lines 315-353
pub struct PipelineQualificationStore; // new(backend), qualify_bundle(tenant_id, signed, trust, metadata, dependencies)
```

`versioned_pipeline.rs` (Tasks 2, 5, 7, 8):

```rust
pub const PIPELINE_LEASE_RENEWAL_CAP_FACTOR: i32 = 4;
impl PipelineLeaseConfig { pub fn for_phase(&self, phase: Phase) -> Duration; }
impl PgPipelineStore {
    pub async fn renew_lease(&self, tenant_id: &str, run_id: Uuid, lease_token: Uuid, extension: Duration, lease_cap: DateTime<Utc>) -> Result<Option<DateTime<Utc>>, DatabaseError>;
    pub async fn stage_attempt_artifact(&self, run: &PipelineRunRecord, artifact: PipelineAttemptArtifact, object_key: &str, ciphertext_sha256: &str, cleanup_after: DateTime<Utc>) -> Result<(), DatabaseError>;
    pub async fn list_rebuildable_index_runs(&self, tenant_id: &str) -> Result<Vec<PipelineRunRecord>, DatabaseError>;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum PipelineAttemptArtifact { Approved, IndexCommand, ScoreNeighbors } // as_str: "approved", "index-command", "score-neighbors"
pub struct PipelineDependencyCheck { pub identity: String, pub production_qualified: bool }
pub struct PipelineBundleQualification { pub bundle_id: String, pub scorer: PipelineDependencyCheck, pub embedder: PipelineDependencyCheck, pub index_reader: PipelineDependencyCheck, pub index_writer: PipelineDependencyCheck, pub settlement_adapters: BTreeMap<String, PipelineDependencyCheck>, pub authority: bool, pub privacy: bool, pub payout: Option<bool>, pub dependency_digest: String }
impl PipelineBundleQualification { pub fn blockers(&self) -> Vec<&'static str>; pub fn is_production_qualified(&self) -> bool; }
pub struct PipelineIndexRebuildReport { pub command_count: usize, pub entry_count: usize, pub unchanged_entry_count: usize, pub command_set_hash: String }
impl PipelineService {
    pub fn default_package(&self) -> &BundlePackage;
    pub fn bundle_qualification(&self, package: &BundlePackage) -> Result<PipelineBundleQualification, &'static str>;
    pub fn index_writer(&self) -> Arc<dyn IdentifiedIndexWriter>;
    pub async fn sweep_attempt_artifacts(&self, tenant_id: &str, limit: usize) -> anyhow::Result<usize>;
    pub async fn rebuild_index_from_authoritative_commands(&self, tenant_id: &str, writer: Arc<dyn IdentifiedIndexWriter>) -> anyhow::Result<PipelineIndexRebuildReport>;
}
```

`pipeline_tooling` (Tasks 3, 4, 9, 10, 11), Python:

```python
class ToolingError(Exception): ...            # errors.py: a safe label only
class StepFailed(ToolingError): ...           # step label, exit code, protected log path
def require(condition, label): ...
def child_environment(extra: dict[str, str]) -> dict[str, str]: ...   # environment.py
class Run: ...          # run_id, started_at, run_dir, code_revision_hash, cleanup_failed, log_path(step), results_dir
class Environment: ...  # context manager; scenario(label) -> Scenario; dump(db, path); restore(path, db)
class Scenario: ...     # runtime_url, upgrade_url, login_resolver_url, pilot_database, artifact_root
def cargo_test(run, step, cargo_args, test_filter, env, *, exact=False, ignored=False) -> None: ...   # cargo.py
def load_results(run) -> dict[str, CheckResult]: ...                 # results.py
def require_current_pass_results(run, results, required: dict[str, CheckSpec]) -> None: ...
def validate_evidence(value) -> None: ...
def canonical(value) -> bytes: ...
def load_direct_corpus(path, expected_digest=None) -> tuple[dict, str]: ...   # corpus.py
def export_hf_corpus(run, pin_path, env, *, local_dir=None) -> list[Path]: ...
def update_catalog(catalog_path, report_path, records=()) -> dict: ...        # catalog.py
REQUIRED_DATABASE_CHECKS: tuple[DatabaseCheck, ...]; REQUIRED_CHECK_IDS: frozenset[str]   # checks.py
def write_report(run, results, inputs) -> Path: ...                           # report.py
```

---

### Task 1: Check result contract, promotion, and package trust (needs PR 2 only)

**Files:**
- Create: `crates/trace-commons-server/src/versioned_pipeline_qualification.rs`
- Modify: `crates/trace-commons-server/src/lib.rs` (add `pub mod versioned_pipeline_qualification;` beside the other `versioned_pipeline*` modules)

**Interfaces:**
- Consumes: `trace_commons_gate_api::pipeline::BundlePackage` (`package_hash`, `validate`, `manifest`), `MinimalPolicyBundle::minimal_package` and the reference dependencies (tests only).
- Produces: the Task 1 part of the `versioned_pipeline_qualification.rs` interface block.

- [ ] **Step 1: Write the failing unit tests** in the module's `#[cfg(test)] mod tests` (assertion lists):
  - `check_result_round_trips_and_refuses_unknown_fields`: a result serializes and deserializes to the same value; JSON with one extra key fails to deserialize; `status: "skipped"` fails to deserialize; `validate` refuses the check id `pipeline.settle.recovery`, a `run_id` of 65 characters, an `evidence_hash` without `sha256:`, a `package_hash` of `sha256:` plus 63 hex characters, and a safe blocker `Bad Label`.
  - `package_digests_follow_the_manifest`: for a minimal package, `package_hash == package.package_hash()`; `configuration_digest == evidence_hash(json!({"admission": h_a, "review": h_r, "score": h_s, "settle": h_t}))`, where each `h` is that phase's `configuration_hash`; `dependency_digest == evidence_hash(json!(sorted score.data_artifact_hashes))`; a package whose score `configuration_hash` differs has a different `configuration_digest`.
  - `evidence_hash_is_canonical_json`: `evidence_hash(&json!({"b": 1, "a": "x", "c": [true, null, "sha256:<64 zeros>"]}))` equals `"sha256:5c4967af7bf59c091e1b5d2c1a70e9d927f642ef996d1d3dc154221175a1ef6b"` (the value of Python's `json.dumps(sort_keys=True, separators=(",", ":"), ensure_ascii=False)`); a `serde_json::Map` built by inserting `"b"` before `"a"` gives the same hash as one built the other way (under this build's `preserve_order` the two maps serialize differently with a plain `to_vec`, so this assertion fails if `evidence_hash` stops canonicalizing); a float value is refused with `evidence_value_invalid`.
  - `emitter_writes_result_and_evidence_once`: in a `tempfile::TempDir`, `PipelineCheckEmitter::new(dir, "q0123abcd", code_hash)`; `emit("pipeline_crash_matrix", PipelineCheckStatus::Pass, Some(&package), &[], json!({"runs": 3}))` writes `pipeline_crash_matrix.result.json` and `pipeline_crash_matrix.evidence.json`; the result validates, carries the run id, the code hash, and the three package digests; its `evidence_hash` equals `evidence_hash` of the evidence file's parsed value; a second emit of the same check id returns `Err("pipeline_check_already_emitted")` and leaves the first files unchanged.
  - `emitter_from_vars_requires_all_three_values`: `from_vars(None, None, None)` is `Ok(None)`; all three set is `Ok(Some(_))`; any one or two set is `Err("pipeline_check_environment_incomplete")`; a run id that is not a label is `Err("pipeline_check_environment_invalid")`.
  - `package_signature_binds_canonical_package_and_trusted_key` and `package_trust_rejects_artifact_tampering_unknown_keys_and_algorithms`: port `ef97a459:crates/trace-commons-server/src/versioned_pipeline_qualification.rs` lines 751 to 792, with a minimal package instead of `compatibility_package()` (the compatibility family is PR 3; Task 6 adds a compatibility case).
  - `sign_bundle_package_round_trips_through_the_trust_store`: a generated PKCS#8 key signs a package; `BundlePackageTrustStore::new([trusted_key_for_pkcs8(id, pkcs8)?])?.verify(&signed)` is `Ok`; another key's store answers `bundle_package_signer_untrusted`.
  - `promotion_requires_current_passing_evidence_for_every_check`: port lines 847 to 887, with `DrillEvidence { check, maximum_age_seconds: 3_600 }` for every id in `PROMOTION_REQUIRED_CHECKS`; add: evidence whose `check.status` is `Blocked` gives a `qualification_evidence_failed:<id>` blocker; an unknown check id is `Err("qualification_evidence_invalid")`.

- [ ] **Step 2: Run them to see them fail.** `cargo test -p trace-commons-server --lib versioned_pipeline_qualification`. Expected: FAIL (the module does not exist).

- [ ] **Step 3: Implement.** Start the file with the AGPL header and a module comment from port lines 4 to 9. Then:

```rust
pub const PIPELINE_CHECK_RESULT_SCHEMA: &str = "trace_commons.pipeline_check_result.v1";
const PIPELINE_CHECK_RESULT_DIR_VAR: &str = "TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR";
const PIPELINE_CHECK_RUN_ID_VAR: &str = "TRACE_COMMONS_PIPELINE_CHECK_RUN_ID";
const PIPELINE_CHECK_CODE_REVISION_VAR: &str = "TRACE_COMMONS_PIPELINE_CHECK_CODE_REVISION_HASH";

pub const PROMOTION_REQUIRED_CHECKS: &[&str] = &[
    "pipeline_storage_upgrade_rls",
    "pipeline_http_corpus_minimal",
    "pipeline_http_corpus_compatibility",
    "pipeline_http_corpus_hf_local",
    "pipeline_crash_matrix",
    "pipeline_http_restart_recovery",
    "pipeline_independent_instruments",
    "pipeline_stale_lease_fence",
    "pipeline_lease_renewal",
    "pipeline_receipt_replay_exact",
    "pipeline_http_receipt_ownership",
    "pipeline_payout_recovery",
    "pipeline_index_rebuild",
    "pipeline_orphan_sweep",
    "pipeline_bundle_qualification",
    "pipeline_restore_drill",
    // Promotion only: no local or CI run passes these.
    "pipeline_production_adapters",
    "pipeline_remote_restore",
    "pipeline_hf_network_canary",
];

pub(crate) fn is_safe_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

pub fn evidence_hash(observed: &serde_json::Value) -> Result<String, String> {
    fn check(value: &serde_json::Value) -> Result<(), String> {
        match value {
            serde_json::Value::Number(n) if n.is_f64() => Err("evidence_value_invalid".into()),
            serde_json::Value::Array(items) => items.iter().try_for_each(check),
            serde_json::Value::Object(map) => map.values().try_for_each(check),
            _ => Ok(()),
        }
    }
    check(observed)?;
    // This build has serde_json/preserve_order on (dcap-qvl), so a plain
    // to_vec keeps insertion order. to_canonical_vec sorts every object's
    // keys: the same compact bytes as Python's canonical form.
    let bytes = trace_commons_protocol::canonical_json::to_canonical_vec(observed)
        .map_err(|_| "evidence_value_invalid".to_string())?;
    Ok(sha256_prefixed(&bytes))
}
```

  `PipelineCheckResult::validate` checks: `schema` equal to the constant; `run_id` and `check_id` are safe labels; `code_revision_hash` and `evidence_hash` are `sha256:` plus 64 lowercase hex; each `Some` digest has the same form; each safe blocker is a label. `package_digests` computes the three values as the Step 1 test states. `PipelineCheckEmitter` holds `dir: PathBuf, run_id: String, code_revision_hash: String`; `from_vars` validates and `from_env` reads the three variables; `emit` refuses an existing `<check_id>.result.json` (`create_new` open), writes each file to a temporary name in the same directory and renames it, and writes the result last; `emit_from_env(check_id, status, package, safe_blockers, observed)` calls `from_env()`, panics on `Err` (a partial or invalid set, so a misspelled variable never passes silently), does nothing on `Ok(None)`, and panics when `emit` fails; `emit_pass_from_env(check_id, package, observed)` is `emit_from_env` with `Pass` and no blockers. Port the signature types and trust store (port lines 76 to 146) unchanged, plus `sign_bundle_package` and `trusted_key_for_pkcs8` (from the port test helper, lines 725 to 749). Port `DrillStatus` as `PipelineCheckStatus`, and `evaluate_promotion` (lines 654 to 706) with `DrillEvidence { check, maximum_age_seconds }`: the check's `status`, `observed_at`, `evidence_hash`, and `safe_blockers` replace the port fields, and `PROMOTION_REQUIRED_CHECKS` replaces `REQUIRED_PROMOTION_DRILLS`. Reuse `crate::versioned_pipeline::sha256_prefixed` (it is `pub(crate)`) instead of a second copy.

- [ ] **Step 4: Run the tests.** `cargo test -p trace-commons-server --lib versioned_pipeline_qualification`. Expected: PASS (9 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-server/src/versioned_pipeline_qualification.rs crates/trace-commons-server/src/lib.rs
git commit -m "Define the pipeline check result and package trust" -m "Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: Lease renewal (needs PR 2 only)

**Files:**
- Modify: `crates/trace-commons-server/src/versioned_pipeline.rs` (`PipelineLeaseConfig`, `PgPipelineStore`, `process_one`, `process_run`, the doc comments that say "Lease renewal (PR 4)")
- Modify: `crates/trace-commons-server/tests/versioned_pipeline_runtime_pg.rs`

**Interfaces:**
- Consumes: `PgPipelineStore::claim_next`, `claim_run_with_lease_config`, `record_lease_expired`, `ensure_live_lease`; the slow scorer double of `score_lease_test_service`; Task 1 `PipelineCheckEmitter::emit_pass_from_env`.
- Produces: `PIPELINE_LEASE_RENEWAL_CAP_FACTOR`, `PipelineLeaseConfig::for_phase`, `PgPipelineStore::renew_lease`, the private `PipelineLeaseRenewal`.

- [ ] **Step 1: Write the failing tests** in `versioned_pipeline_runtime_pg.rs` (assertion lists; build services with `score_lease_test_service`, leases `PipelineLeaseConfig::new(1 s, 2 s, 1 s)`):
  - `a_score_longer_than_its_lease_completes_once_with_two_workers`: service A and service B share one database and one tenant; A's scorer sleeps 5 s; A runs `process_one` until Score; while A scores, B calls `claim_next` every 250 ms; B never gets `Some`; the run completes with `attempt_count` 1 for the Score claim, no `lease_expired` label, and exactly one Score outcome. Then call `PipelineCheckEmitter::emit_pass_from_env("pipeline_lease_renewal", Some(&package), json!({"score_claims": 1, "competing_claims": 0, "score_outcomes": 1}))`, where `package` is the package the test service was built from.
  - `renewal_stops_at_the_lease_cap`: Score lease 1 s (cap 4 s); the scorer sleeps 7 s; a reader samples `lease_expires_at` every 200 ms and never sees it later than the claim time plus 4 s plus 1 s of tolerance; B's `claim_next` returns `Some` after the cap; A's Score commit is refused, and the run carries B's lease token.
  - `a_lost_lease_is_never_renewed`: claim a run (token T1); expire the lease and claim it again with `claim_next` (token T2); `renew_lease(T1, ...)` returns `Ok(None)`; `lease_expires_at` is unchanged.
  - `renewal_never_shortens_a_lease`: claim with a 60 s lease; `renew_lease` with a 1 s extension returns the original expiry.
  - Unit test (no database) `lease_for_phase_matches_the_config`: `for_phase(Phase::Score) == config.score()`, and the same for Review and Settle; `for_phase(Phase::Admission)` is `review()` (the claim SQL's `ELSE` arm).

- [ ] **Step 2: Run them to see them fail.** `dropdb`/`createdb` a fresh `admission_test_pipeline_renewal` on 127.0.0.1:55431, then `TRACE_COMMONS_PG_TEST_DATABASE_URL=postgres://trace@127.0.0.1:55431/admission_test_pipeline_renewal cargo test -p trace-commons-server --test versioned_pipeline_runtime_pg renew lease_cap lost_lease shortens two_workers`. Expected: FAIL (`renew_lease` does not exist).

- [ ] **Step 3: Implement.**

```rust
/// A lease is renewed at most until this many phase leases after its claim.
pub const PIPELINE_LEASE_RENEWAL_CAP_FACTOR: i32 = 4;

impl PipelineLeaseConfig {
    /// The lease a claim of `phase` gets; Admission never runs as a claim and
    /// falls back to Review, as the claim SQL's `ELSE` arm does.
    pub fn for_phase(&self, phase: Phase) -> Duration {
        match phase {
            Phase::Score => self.score,
            Phase::Settle => self.settle,
            Phase::Admission | Phase::Review => self.review,
        }
    }
}

impl PgPipelineStore {
    /// Extends a lease this claim still holds. Never shortens it, never
    /// extends past `lease_cap`, and changes nothing when the token no longer
    /// matches or the lease already expired.
    pub async fn renew_lease(
        &self,
        tenant_id: &str,
        run_id: Uuid,
        lease_token: Uuid,
        extension: Duration,
        lease_cap: DateTime<Utc>,
    ) -> Result<Option<DateTime<Utc>>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let extension_ms = extension.num_milliseconds();
        let row = tx
            .query_opt(
                "UPDATE pipeline_runs
                    SET lease_expires_at = GREATEST(
                            lease_expires_at,
                            LEAST(NOW() + ($4::bigint * INTERVAL '1 millisecond'), $5)
                        ),
                        updated_at = NOW()
                  WHERE tenant_id = $1 AND run_id = $2 AND state = 'leased'
                    AND lease_token = $3 AND lease_expires_at > NOW()
                  RETURNING lease_expires_at",
                &[&tenant_id, &run_id, &lease_token, &extension_ms, &lease_cap],
            )
            .await?;
        tx.commit().await?;
        Ok(row.map(|row| row.get(0)))
    }
}

/// Renews a claim's lease while its phase runs. Stopped when the phase ends,
/// at the cap, or when the lease is lost. The commit fences stay the only
/// authority: renewal only keeps an honest slow phase from being reclaimed.
struct PipelineLeaseRenewal {
    stop: tokio::sync::watch::Sender<bool>,
    join: tokio::task::JoinHandle<()>,
}

impl PipelineLeaseRenewal {
    fn start(store: PgPipelineStore, run: &PipelineRunRecord, config: PipelineLeaseConfig) -> Option<Self> {
        let phase = run.next_phase?;
        let lease_token = run.lease_token?;
        let lease = config.for_phase(phase);
        let lease_cap = Utc::now() + lease * PIPELINE_LEASE_RENEWAL_CAP_FACTOR;
        let interval = std::cmp::max(lease / 3, Duration::milliseconds(100)).to_std().ok()?;
        let tenant_id = run.tenant_id.clone();
        let run_id = run.run_id;
        let (stop, mut stopped) = tokio::sync::watch::channel(false);
        let join = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(interval) => {}
                    _ = stopped.changed() => return,
                }
                match store.renew_lease(&tenant_id, run_id, lease_token, lease, lease_cap).await {
                    Ok(Some(expires_at)) if expires_at < lease_cap => {}
                    Ok(_) => return,
                    Err(_) => tracing::warn!(
                        label = "pipeline_lease_renewal_failed",
                        "a lease renewal failed; the next interval tries again"
                    ),
                }
            }
        });
        Some(Self { stop, join })
    }

    async fn stop(self) {
        let _ = self.stop.send(true);
        let _ = self.join.await;
    }
}
```

  Add `#[derive(Clone)]` to `PgPipelineStore` if it does not have it (it holds only `Arc<PgBackend>`). In `process_one` and `process_run`, after the claim:

```rust
        let renewal = PipelineLeaseRenewal::start(self.store.clone(), &run, self.lease_config);
        let result = self.process_claimed_run(run).await;
        if let Some(renewal) = renewal {
            renewal.stop().await;
        }
        result
```

  The renewal task takes its own short connection checkout and never runs inside another transaction. Update the doc comments that say renewal is PR 4 (search `Lease renewal`) to describe this behavior. Check that V92 grants `UPDATE (lease_expires_at, updated_at)` on `pipeline_runs` to `trace_ingest_runtime` (the claim already writes both); if it does not, stop and ask (a grant change in a PR 2 migration is PR 2's).

- [ ] **Step 4: Run the tests.** The five tests, then the whole runtime suite on a fresh database (`cargo test -p trace-commons-server --test versioned_pipeline_runtime_pg`). Expected: PASS, including `a_slow_score_completes_under_its_own_longer_configured_lease`, `a_score_lease_that_always_expires_records_the_expiry_and_never_exhausts`, `index_dispatch_never_holds_two_pooled_connections`, and the crash matrix.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-server/src/versioned_pipeline.rs crates/trace-commons-server/tests/versioned_pipeline_runtime_pg.rs
git commit -m "Renew a pipeline lease while its phase runs" -m "Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: `pipeline.py` foundation, `test`, and the inventory lint (needs PR 2 only)

**Files:**
- Create: `scripts/operator/pipeline.py`
- Create: `scripts/operator/pipeline_tooling/__init__.py`, `errors.py`, `environment.py`, `cargo.py`, `results.py`, `checks.py`
- Create: `scripts/operator/test_pipeline_tooling.py`
- Create: `scripts/operator/pipeline-deployment-inventory.py`
- Modify: `.gitignore`

**Interfaces:**
- Consumes: Task 1 result schema and canonical form.
- Produces: the `pipeline_tooling` block in "Interfaces"; `python3 scripts/operator/pipeline.py test [--check contracts|runtime|postgres] [--postgres-admin-url URL]`.

- [ ] **Step 1: Write the failing self-tests** in `scripts/operator/test_pipeline_tooling.py` (`unittest`, no Docker, no cargo; subprocess calls go through an injectable runner that the tests replace):
  - `test_child_environment_drops_ambient_database_urls`: with `DATABASE_URL`, `PGPASSWORD`, `AWS_SECRET_ACCESS_KEY`, and `TRACE_COMMONS_TENANT_TOKENS` in `os.environ`, `child_environment({"TRACE_COMMONS_PG_TEST_DATABASE_URL": u})` contains `PATH` and `TRACE_COMMONS_PG_TEST_DATABASE_URL` and none of the four; an extra key without the `TRACE_COMMONS_` prefix raises `ToolingError("child_environment_key_invalid")`; `RUSTFLAGS` passes through when present and is absent when absent.
  - `test_zero_match_filter_fails`: a fake `cargo test ... -- --list` output with no `: test` line makes `cargo_test` raise `ToolingError("cargo_filter_matched_zero_tests")` before the real run; a listing with `a::b: test` runs the real command once, with `--exact` and `--ignored` after `--` when requested.
  - `test_results_reject_missing_stale_foreign_and_tampered`: write results with a helper; `require_current_pass_results` fails with a distinct label for each case: a required id with no file (`check_result_missing:<id>`), a `run_id` of another run (`check_result_foreign_run`), another `code_revision_hash` (`check_result_foreign_revision`), `observed_at` before `run.started_at` (`check_result_stale`), `observed_at` in the future (`check_result_stale`), `status: blocked` for a required id (`check_result_blocked:<id>`), `status: fail` for any id (`check_result_failed:<id>`), an evidence file whose canonical hash differs (`check_evidence_hash_mismatch`), a missing evidence file (`check_evidence_missing`), an extra key (`check_result_schema_invalid`), a dotted check id (`check_result_schema_invalid`), a required digest that is `null` (`check_result_digest_missing:<id>`); a complete, current set passes.
  - `test_evidence_validator_refuses_secret_like_values`: `validate_evidence` refuses a value starting `ghp_`, `github_pat_`, or `sk-`, a value containing `@`, a key in `{"input", "text", "trace_text", "secret", "secret_probe", "token", "account_id", "email"}`, a string longer than 128 characters, and a float; it accepts labels, `sha256:` hashes, integers, booleans, `null`, and lists and objects of those.
  - `test_cleanup_failure_fails_a_passing_run_and_keeps_an_earlier_failure`: `pipeline.main` with a fake command that succeeds and a fake environment whose cleanup leaves the container listed returns 1; with a command that fails with exit code 7 and a cleanup that also fails, it returns 7; with both clean, 0.
  - `test_environment_names_are_valid_test_databases`: scenario database names start with `admission_test_`, upgrade names with `pipeline_test_`, all are at most 63 characters and match `^[a-z0-9_]+$`, and the URLs use the literal host `127.0.0.1`.
  - `test_admin_url_mode_holds_the_lock_database`: with a fake server, entering the environment runs `CREATE DATABASE pipeline_tooling_lock` and exiting runs `DROP DATABASE pipeline_tooling_lock`; when the create fails, entering raises `ToolingError("pipeline_tooling_server_busy")`.
  - `test_failure_output_is_label_only`: when a step fails, `main` writes one line to standard error that holds the step label, the exit code, and the log path relative to the repository, and nothing from the child's command line, environment, or output (the fake child prints a URL and a token-shaped string; neither appears).

- [ ] **Step 2: Run them to see them fail.** `python3 scripts/operator/test_pipeline_tooling.py`. Expected: FAIL (the modules do not exist).

- [ ] **Step 3: Implement.**
  - `errors.py`:

```python
class ToolingError(Exception):
    """A safe label. Never carries a URL, token, command line, or trace text."""


class StepFailed(ToolingError):
    def __init__(self, step, exit_code, log_path):
        super().__init__(f"step_failed:{step}")
        self.step, self.exit_code, self.log_path = step, exit_code, log_path


def require(condition, label):
    if not condition:
        raise ToolingError(label)
```

  - `environment.py`: `ROOT`, `POSTGRES_IMAGE` (P4-D6), `CHILD_ENV_ALLOWLIST = ("PATH", "HOME", "USER", "LOGNAME", "TMPDIR", "CARGO_HOME", "RUSTUP_HOME", "RUSTUP_TOOLCHAIN", "CARGO_TERM_COLOR", "CARGO_INCREMENTAL", "RUSTFLAGS", "SystemRoot")`, `child_environment`, `Run` (run id `q` plus 8 random hex characters; `started_at` in UTC; `run_dir = ROOT / ".local/pipeline/runs" / run_id` created with mode 0700; `code_revision_hash` from the port's tree hash, `ef97a459:scripts/operator/run-pipeline-qualification.sh` lines 70 to 88, with `sha256:` prefix; `log_path(step)` creates `logs/<step>.log` with mode 0600), `run_child(run, step, command, env)` (stdout and stderr to the log; a nonzero exit raises `StepFailed`), and `Environment`:
    - Container mode: `docker run --detach --rm --name tc-pipeline-<run_id> -e POSTGRES_USER=trace -e POSTGRES_HOST_AUTH_METHOD=trust -p 127.0.0.1::5432 <POSTGRES_IMAGE>`; wait for `docker exec <c> pg_isready -U trace` (60 tries, 0.25 s); read the port with `docker port <c> 5432/tcp` and require `127.0.0.1:<digits>`; `psql` through `docker exec -i <c> psql -v ON_ERROR_STOP=1 -U trace -qtA`.
    - Admin-url mode (`--postgres-admin-url`): require host `127.0.0.1` and user `trace`; `psql <url> -v ON_ERROR_STOP=1 -qtA`; create the lock database on enter and drop it on exit.
    - On enter, create the login-resolver roles exactly as `.github/workflows/ci.yml` does (`trace_login_resolver NOLOGIN NOBYPASSRLS`, `tc_login_resolver_login LOGIN NOBYPASSRLS`, the grant), skipping each that exists.
    - `scenario(label)`: databases `admission_test_<run8>_<n>` and `pipeline_test_<run8>_<n>` (a counter `n`, two digits), an artifact root under the run directory; on exit, `DROP DATABASE IF EXISTS <name> WITH (FORCE)` for the database, `<database>_pilot`, `<database>_restored`, and the upgrade database. A scenario also offers `committed_transactions(*databases)` (sum of `pg_stat_database.xact_commit`).
    - `__exit__`: remove the container (`docker rm -f`) and confirm with `docker ps -a --filter name=tc-pipeline-<run_id> -q` that nothing is left; any failure sets `run.cleanup_failed = True`. It never suppresses the primary exception.
  - `cargo.py`: `cargo_test` lists with `cargo test <args> <filter> -- --list [--exact] [--ignored]`, counts lines ending in `: test`, requires at least one, then runs `cargo test <args> <filter> -- [--exact] [--ignored]` through `run_child`.
  - `results.py`: `canonical(value)` (`json.dumps(sort_keys=True, separators=(",", ":"), ensure_ascii=False, allow_nan=False).encode()`), `validate_evidence` (Step 1 rules; port `ef97a459:scripts/operator/lab/lab.py` lines 219 to 235, tightened to labels, hashes, and ISO times), `CheckResult`, `load_results(run)` (each `results/*.result.json`, exact key set of the Rust schema, formats as Task 1's `validate`, evidence file present and hash equal), and `require_current_pass_results(run, results, required)` with the Step 1 labels.
  - `checks.py`: `CheckSpec(check_id, digests_required: bool)` and, for now, `TEST_CHECKS` for `pipeline.py test`:
    - `contracts`: `cargo test -p trace-commons-gate-api --lib pipeline::`, `cargo test -p trace-commons-server --lib versioned_pipeline_bundle`, `cargo test -p trace-commons-server --lib versioned_pipeline_qualification`, and `python3 scripts/operator/pipeline-deployment-inventory.py --check`.
    - `runtime`: `cargo test -p trace-commons-server --lib versioned_pipeline`, `cargo test -p trace-commons-server --bin trace-commons-ingest pipeline_runtime`, and `python3 scripts/operator/test_pipeline_tooling.py`.
    - `postgres` (needs the environment; each step in its own scenario, with the variables the CI step uses): the upgrade test (`--lib pipeline_upgrade -- --ignored`), `--test migration_atomicity_pg`, `--test versioned_pipeline_runtime_pg`, and the ingest tests `real_http_receipt_completes_and_resumes_after_restart`, `compatibility_bundle_through_http_with_review_privacy_withdrawal_and_export`, and `legacy_and_pipeline_tenants_match_under_equivalent_configuration` (the last two with the login-resolver URL). After each step, require at least 5 committed transactions in its databases (`database_check_executed_nothing:<step>`).
  - `pipeline.py`: `argparse` subcommands (`test` now; later tasks add the others), one `Run` per command, and `main(argv) -> int` with the exit rule:

```python
def main(argv=None):
    args = parse_args(argv)
    run = Run.create()
    primary = 0
    try:
        args.handler(args, run)
    except StepFailed as failure:
        print(f"PipelineFailure: {failure} exit={failure.exit_code} log={failure.log_path.relative_to(ROOT)}", file=sys.stderr)
        primary = failure.exit_code or 1
    except ToolingError as error:
        print(f"PipelineFailure: {error}", file=sys.stderr)
        primary = 1
    except KeyboardInterrupt:
        primary = 130
    cleanup = 1 if run.cleanup_failed else 0
    if cleanup:
        print("PipelineFailure: cleanup_failed", file=sys.stderr)
    return primary if primary != 0 else cleanup
```

    The default `test` runs `contracts` and `runtime`. `pipeline.py` refuses any argument that looks like a URL except `--postgres-admin-url`, and never prints a child command line.
  - `pipeline-deployment-inventory.py`: port `ef97a459:scripts/operator/pipeline-deployment-inventory.py` (388 lines) with two changes: `build_inventory` reads only `trace-commons-ingest.rs`; `main` does not call `validate_contract_manifest`, and `validate_contract_manifest`, `resolve_evidence_id`, `SCRIPT_EVIDENCE`, `CORPUS_EVIDENCE`, and `RUST_MODULES` are removed (P4-D14). Add a comment at `main` that names the upstream manifest refresh as the condition to add the check back.
  - `.gitignore`: add `/.local/` with a comment ("pipeline tooling evidence, logs, and database dumps").
  - Confirm the image digest: `docker buildx imagetools inspect postgres:16` shows the index digest; if it differs from `a3b7f434...`, use the index digest and record it in the ledger.

- [ ] **Step 4: Run the tests.** `python3 scripts/operator/test_pipeline_tooling.py` (PASS), `python3 scripts/operator/pipeline-deployment-inventory.py --check` (prints `PipelineDeploymentInventoryOK`), `python3 scripts/operator/pipeline.py test` (PASS), and `python3 scripts/operator/pipeline.py test --check postgres` (PASS; it starts and removes its own container; `docker ps -a` shows no `tc-pipeline-q*` container afterward). Then prove two failures by hand and restore each: change one test name in the `postgres` list to a name that does not exist (fails with `cargo_filter_matched_zero_tests`); stop Docker's container from another terminal during the run (fails, exit code nonzero, no container left).

- [ ] **Step 5: Commit**

```bash
git add scripts/operator/pipeline.py scripts/operator/pipeline_tooling scripts/operator/test_pipeline_tooling.py scripts/operator/pipeline-deployment-inventory.py .gitignore
git commit -m "Add the pipeline tooling entry point and its test command" -m "Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: HF corpus export (needs PR 2 only)

**Files:**
- Create: `crates/trace-commons-server/src/bin/trace-commons-pipeline-corpus-export.rs`
- Modify: `crates/trace-commons-server/src/bin/pilot_bootstrap/hf_dataset.rs`
- Create: `crates/trace-commons-server/tests/fixtures/pipeline-hf-jsonl/01.jsonl`, `02.jsonl`, `pin-local.json`
- Create: `scripts/operator/pipeline_tooling/corpus.py`
- Modify: `scripts/operator/test_pipeline_tooling.py`

**Interfaces:**
- Consumes: `pilot_bootstrap/hf_dataset.rs` (`list_local_jsonl_sessions`, `read_session_bytes`, `list_session_names`, `fetch_session`), `pilot_bootstrap/translators.rs` (`translator_by_name`, `passes_word_filter`).
- Produces: the binary `trace-commons-pipeline-corpus-export` (port arguments); `HfJsonlDataset::open_at_revision`; `corpus.load_direct_corpus`, `corpus.load_pin`, `corpus.export_hf_corpus`.

- [ ] **Step 1: Write the failing tests.**
  - Python (`test_pipeline_tooling.py`): `test_corpus_validation_refuses_changed_bytes_order_and_duplicates`: `load_direct_corpus` refuses an unsupported schema, an empty fixture list, a duplicate label, trace id, or submission id, an unsafe label, an empty `secret_probe`, and a digest other than the expected one; `load_pin` refuses a pin whose schema is not `trace_commons.pipeline_hf_corpus_pin.v1` or that lacks a field the port script reads; `export_hf_corpus` (with a fake runner that writes a manifest) refuses a manifest whose `source_digest`, `order_digest`, `bootstrap_corpus_digest`, or `holdout_corpus_digest` differs from the pin (`hf_<field>_mismatch`) and one that says `contains_raw_trace_text: true`.
  - Rust: the binary has no unit tests in the port; add `#[cfg(test)]` tests in the new binary file: `deterministic_uuid_is_stable_and_version_five` (same input, same UUID; version nibble 5; variant bits 10) and `fixture_never_carries_the_source_name` (the fixture JSON for a session whose name is `secret-session.jsonl` contains no `secret-session`).

- [ ] **Step 2: Run them to see them fail.** `python3 scripts/operator/test_pipeline_tooling.py` and `cargo test -p trace-commons-server --bin trace-commons-pipeline-corpus-export`. Expected: FAIL.

- [ ] **Step 3: Implement.**
  - Port `ef97a459:crates/trace-commons-server/src/bin/trace-commons-pipeline-corpus-export.rs` (294 lines), plus the two tests, with one change: `canonical(value)` returns `trace_commons_protocol::canonical_json::to_canonical_vec(value)` instead of `serde_json::to_vec(value)` (Global Constraints). This changes only `configuration_digest` (the one hashed JSON object; `order_digest` hashes an array of strings, and the corpus digests hash files). Add a third test, `configuration_digest_is_key_order_independent`: the digest for the port pin's configuration equals `"sha256:06d536617c70a36caba72ceab8ac637c75d428727dc27f92e931d07b5510d050"` with repository `jedisct1/security-audits` (computed on 2026-09-29 with Python's canonical form; the port's insertion-order value for the old name was `de772776...`).
  - Port the `hf_dataset.rs` change (`git diff upstream/main ef97a459 -- crates/trace-commons-server/src/bin/pilot_bootstrap/hf_dataset.rs`, 14 lines): `open` calls `open_at_revision(dataset_id, "main", cache_dir)`; `open_at_revision` refuses an empty revision and uses `Repo::with_revision`.
  - Copy `ef97a459:crates/trace-commons-server/tests/fixtures/pipeline-hf-jsonl/01.jsonl` and `02.jsonl`. Write `pin-local.json`: the port pin's fields (`ef97a459:docs/superpowers/specs/versioned-pipeline-hf-corpus-v1.json`) with `"repository": "jedisct1/security-audits"` (the dataset's current name), `"min_words": 1`, `"source_digest": "sha256:fdf56253bff083a19045afa91dc3ba8142d994b72c1dcc1b23bec5be4dd0e9ef"`, and `"order_digest": "sha256:6135f8f173fd9b60f13e986e1c002b91d7a32d147fd2425c66ee2b5023b7dfcb"` (the local digests of `ef97a459:scripts/operator/run-pipeline-hf-qualification.sh` lines 38 to 41), plus `"local_jsonl_dir": "crates/trace-commons-server/tests/fixtures/pipeline-hf-jsonl"`. The local sample and `min_words` differ from the network pin, so set `configuration_digest`, `bootstrap_corpus_digest`, and `holdout_corpus_digest` to the values that the first local export prints (the export is deterministic), and record them in the ledger.
  - `corpus.py`: port `validate_corpus` (`lab.py` lines 102 to 119) as `load_direct_corpus`; `load_pin`; `export_hf_corpus(run, pin_path, env, *, local_dir=None)` builds the port script's argument list (lines 43 to 60) from the pin, runs `cargo run -q -p trace-commons-server --bin trace-commons-pipeline-corpus-export -- ...` through `run_child` into `run_dir/hf/`, checks the manifest (port script lines 101 to 149, the remote digest checks apply when the pin names them), and returns `[bootstrap-corpus.json, holdout-corpus.json]`.

- [ ] **Step 4: Run the tests** (the Python self-tests and the binary's tests), then one export by hand: `python3 -c "..."` is not needed; run `cargo run -q -p trace-commons-server --bin trace-commons-pipeline-corpus-export -- <the pin-local.json arguments> --output-dir .local/pipeline/hf-check` and confirm the source and order digests match the pin. Expected: PASS. Then do steps 1 and 2 of Draft 1 in `.superpowers/sdd/2026-09-29-versioned-pipeline-pr4-qualification-plan/replies.md` (owner decision 2026-09-29, choice A): one network export with the network pin's settings and the name `jedisct1/security-audits`, then a second export with every expected digest set. Record the five digests and the port values in the ledger, fill the draft, and tell the owner. Do not push and do not open the upstream PR without the owner's yes.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-server/src/bin/trace-commons-pipeline-corpus-export.rs crates/trace-commons-server/src/bin/pilot_bootstrap/hf_dataset.rs crates/trace-commons-server/tests/fixtures/pipeline-hf-jsonl scripts/operator/pipeline_tooling/corpus.py scripts/operator/test_pipeline_tooling.py
git commit -m "Export a pinned JSONL sample as a pipeline corpus" -m "Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 5: Qualification for each bundle (FR5 P4) (needs PR 3)

**Files:**
- Modify: `crates/trace-commons-server/src/versioned_pipeline.rs` (`construct`, new `resolve_score_dependencies`, `default_package`, `bundle_qualification`, the two structs)
- Modify: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_runtime.rs` (`pipeline_runtime_is_production_qualified`)
- Modify: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs` (startup tests)
- Modify: `crates/trace-commons-server/tests/versioned_pipeline_runtime_pg.rs`

**Interfaces:**
- Consumes: `SettlementAdapterRegistry::get`, `SettlementAdapter::{adapter_identity, production_qualified}`, the `Identified*` traits (`dependency_identity`, `production_qualified`), `PipelineService::payout_enabled`, Task 1 `package_digests`.
- Produces: `PipelineDependencyCheck`, `PipelineBundleQualification`, `PipelineService::{default_package, bundle_qualification}`.

- [ ] **Step 1: Write the failing tests.**
  - Ingest unit tests beside `pipeline_runtime_starts_a_qualified_dependency_with_routed_tenants` (reuse `pipeline_runtime_fail_closed_fixture` and the qualified doubles there):
    - `an_unqualified_scorer_the_default_bundle_does_not_name_does_not_block_startup`: the service holds the qualified scorer that the default package names and one more, unqualified scorer; tenants are routed; startup succeeds.
    - `an_unqualified_adapter_for_an_instrument_the_bundle_does_not_pin_does_not_block_startup`: an extra unqualified settlement adapter for an instrument the default package does not pin; startup succeeds.
    - `an_unqualified_scorer_the_default_bundle_names_blocks_startup`: the named scorer is unqualified; startup fails with `pipeline_runtime_dependencies_not_production_qualified`.
    - The existing authority, privacy, and payout startup tests pass unchanged.
  - Runtime tests (assertion lists; `emit_pass_from_env("pipeline_bundle_qualification", ...)` at the end of the first):
    - `qualification_inspects_the_objects_the_constructor_receives`: two counting scorer doubles Q (qualified) and U (unqualified) with different descriptors; the package names Q; `bundle_qualification` reports Q's identity with `production_qualified: true` and does not report U; one run through Score calls Q's `score` and never U's.
    - `qualification_fails_closed_for_a_substituted_or_missing_dependency`: a scorer with the same identity label and changed descriptor bytes answers `bundle_dependency_missing`; a package with one artifact byte changed answers `bundle_package_invalid`; a package that pins an instrument with no registered adapter reports that adapter with identity `settlement_adapter_missing` and blocker `runtime_settlement_not_production`.
    - `qualification_reports_payout_only_when_it_applies`: payout disabled gives `payout: None`; payout enabled with a package that pins `trace_credit` gives `Some(adapter.production_qualified())`; payout enabled with a package that does not pin `trace_credit` gives `None`.

- [ ] **Step 2: Run them to see them fail.** `cargo test -p trace-commons-server --bin trace-commons-ingest pipeline_runtime` and the three runtime tests on a fresh database. Expected: FAIL.

- [ ] **Step 3: Implement.** Move the scorer and embedder lookup out of `construct` into `resolve_score_dependencies(&self, package: &BundlePackage) -> Result<(Arc<dyn IdentifiedPerplexityScorer>, Arc<dyn IdentifiedEmbedder>), &'static str>`, and call it from `construct`. Add the structs from "Interfaces" (`#[derive(Debug, Clone, PartialEq, Eq, Serialize)]`), and:

```rust
impl PipelineBundleQualification {
    pub fn blockers(&self) -> Vec<&'static str> {
        let mut blockers = Vec::new();
        for (check, label) in [
            (&self.scorer, "runtime_scorer_not_production"),
            (&self.embedder, "runtime_embedder_not_production"),
            (&self.index_reader, "runtime_index_reader_not_production"),
            (&self.index_writer, "runtime_index_writer_not_production"),
        ] {
            if !check.production_qualified {
                blockers.push(label);
            }
        }
        if self.settlement_adapters.values().any(|check| !check.production_qualified) {
            blockers.push("runtime_settlement_not_production");
        }
        if !self.authority {
            blockers.push("runtime_authority_not_production");
        }
        if !self.privacy {
            blockers.push("runtime_privacy_not_production");
        }
        if self.payout == Some(false) {
            blockers.push("runtime_payout_not_production");
        }
        blockers
    }

    pub fn is_production_qualified(&self) -> bool {
        self.blockers().is_empty()
    }
}
```

  `bundle_qualification` validates the package (`bundle_package_invalid`), resolves the scorer and embedder with `resolve_score_dependencies`, builds one `PipelineDependencyCheck` for each (identity from `dependency_identity()`), one for the index reader and writer, one for each key of `package.manifest.instruments` from `self.settlement_adapters.get(..)` (missing: identity `settlement_adapter_missing`, false), `authority` and `privacy` as `dependency_qualification` computes them today, `payout` as the third test states, and `dependency_digest` from `package_digests(package)`. Replace the body of `pipeline_runtime_is_production_qualified` with:

```rust
    service
        .bundle_qualification(service.default_package())
        .is_ok_and(|qualification| qualification.is_production_qualified())
```

  and update its doc comment (the per-bundle rule, P4-D7). Keep `dependency_qualification` (other callers read it); note in its doc comment that startup no longer uses it.

- [ ] **Step 4: Run the tests.** The ingest `pipeline_runtime` tests, the three runtime tests, and the whole runtime suite on a fresh database. Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-server/src/versioned_pipeline.rs crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_runtime.rs crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs crates/trace-commons-server/tests/versioned_pipeline_runtime_pg.rs
git commit -m "Qualify only the dependencies a pipeline bundle uses" -m "Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 6: Qualification storage and store (V103) (needs PR 3)

**Files:**
- Create: `migrations/V103__versioned_pipeline_qualification.sql`
- Modify: `crates/trace-commons-server/src/db/postgres.rs` (registry, `TRACE_COMMONS_RLS_TABLES`, static migration tests)
- Modify: `crates/trace-commons-server/src/db/postgres/pipeline_upgrade_tests.rs`
- Modify: `crates/trace-commons-server/src/versioned_pipeline_product.rs` (`operational_summary` RLS list)
- Modify: `crates/trace-commons-server/src/versioned_pipeline_qualification.rs`
- Modify: `crates/trace-commons-server/tests/versioned_pipeline_runtime_pg.rs`

**Interfaces:**
- Consumes: `pipeline_bundle_packages`, `PgPipelineStore::register_bundle`, Task 5 `bundle_qualification`, Task 1 trust store, the compatibility implementation ids and `MinimalPolicyBundle::compatibility_package`.
- Produces: table `pipeline_bundle_qualifications`; the Task 6 part of the qualification interface block.

- [ ] **Step 1: Extend the upgrade test.** Add `pipeline_bundle_qualifications` to `PIPELINE_TABLES`, raise the expected maximum version to 103, and add the V103 grant pins: `("pipeline_bundle_qualifications", "SELECT", [])`, `("pipeline_bundle_qualifications", "INSERT", [])`, and no UPDATE or DELETE.

- [ ] **Step 2: Write the failing tests** (assertion lists):
  - Unit (`versioned_pipeline_qualification.rs`): `production_package_validation_refuses_minimal_unknown_and_reference_packages` (port lines 794 to 845 with the per-bundle profile: a minimal package answers `bundle_implementation_unknown`; `compatibility_package(&CompatibilityBundleConfig::local_reference(), ..)` answers `bundle_development_dependency`; a package with an unknown score id answers `bundle_implementation_unknown`); `production_profile_blocks_on_bundle_and_infrastructure` (`ProductionDependencyProfile::new` with a hand-built `PipelineBundleQualification` whose scorer is unqualified and `ProductionInfrastructureProfile::local_test()` lists `runtime_scorer_not_production`, `artifact_store_not_production`, and `static_bearer_authentication_enabled`; `runtime_identity_digest` changes when the scorer identity changes; the "unrelated held scorer" case is proved in Task 5, because a hand-built qualification never contains one).
  - Runtime: `qualify_bundle_records_an_immutable_hash_only_identity`: with qualified doubles for every bundle dependency and an all-production infrastructure profile, `qualify_bundle` returns a record whose fields equal the inputs; a second call with the same inputs returns the same record; a call with other metadata answers `bundle_qualification_identity_conflict`; a direct `UPDATE` and a direct `DELETE` as the owner connection fail (`pipeline bundle qualifications are immutable`); tenant B reads no row for tenant A's bundle; deleting tenant A succeeds and removes the row. `qualify_bundle_refuses_untrusted_development_and_unqualified_inputs`: an untrusted signer, a tampered package, invalid metadata (`bundle_qualification_metadata_invalid`), a metadata dependency digest that differs from the profile (`runtime_dependency_identity_mismatch`), and a profile with a blocker (that label) each fail, and no row exists afterward.
  - Static migration test: V103's table forces RLS, has the tenant policy, no function-level `SET`, no `SECURITY DEFINER`.

- [ ] **Step 3: Run them to see them fail.** Fresh `pipeline_test_upgrade_pr4` for the upgrade test (`TRACE_COMMONS_PIPELINE_PG_UPGRADE_TEST_URL=postgres://trace@127.0.0.1:55431/pipeline_test_upgrade_pr4 cargo test -p trace-commons-server --lib pipeline_upgrade -- --ignored`), a fresh runtime database for the rest. Expected: FAIL.

- [ ] **Step 4: Write V103.**

```sql
-- Production package qualification for the versioned pipeline (delivery
-- PR 4). Detailed lab and drill reports stay outside the ingest database:
-- this table stores only the immutable package trust and evidence
-- identities that the PR 5 activation gate reads.

CREATE TABLE pipeline_bundle_qualifications (
    tenant_id TEXT NOT NULL,
    bundle_id TEXT NOT NULL CHECK (bundle_id ~ '^sha256:[0-9a-f]{64}$'),
    package_hash TEXT NOT NULL CHECK (package_hash ~ '^sha256:[0-9a-f]{64}$'),
    signing_key_id TEXT NOT NULL CHECK (signing_key_id ~ '^[A-Za-z0-9_.:-]{1,128}$'),
    signature_hash TEXT NOT NULL CHECK (signature_hash ~ '^sha256:[0-9a-f]{64}$'),
    corpus_digest TEXT NOT NULL CHECK (corpus_digest ~ '^sha256:[0-9a-f]{64}$'),
    input_digest TEXT NOT NULL CHECK (input_digest ~ '^sha256:[0-9a-f]{64}$'),
    configuration_digest TEXT NOT NULL CHECK (configuration_digest ~ '^sha256:[0-9a-f]{64}$'),
    code_revision_hash TEXT NOT NULL CHECK (code_revision_hash ~ '^sha256:[0-9a-f]{64}$'),
    runtime_dependency_digest TEXT NOT NULL CHECK (runtime_dependency_digest ~ '^sha256:[0-9a-f]{64}$'),
    evidence_hash TEXT NOT NULL CHECK (evidence_hash ~ '^sha256:[0-9a-f]{64}$'),
    qualified_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, bundle_id),
    -- A qualification leaves only with its package, and a package leaves
    -- only with its tenant (V93's trigger lets only a cascade through).
    FOREIGN KEY (tenant_id, bundle_id)
        REFERENCES pipeline_bundle_packages (tenant_id, bundle_id)
        ON DELETE CASCADE
);
```

  Then copy the append-only function and triggers of `migrations/V101__versioned_pipeline_review_invalidation.sql` on the PR 3 branch (lines 107 to 133) with the names `reject_pipeline_bundle_qualification_mutation`, `pipeline_bundle_qualifications_reject_update`, `pipeline_bundle_qualifications_reject_delete`, and the message `pipeline bundle qualifications are immutable`; the RLS block (port V79 lines 59 to 64); the `trace_ingest_runtime` presence check (V101 lines 158 to 162, with `V103:` in the message); and `GRANT SELECT, INSERT ON pipeline_bundle_qualifications TO trace_ingest_runtime;` with a comment in V101's style.

- [ ] **Step 5: Implement.** Register V103 after V102 in `db/postgres.rs`; add the table to `TRACE_COMMONS_RLS_TABLES` and to the `operational_summary` list (P4-D21). In `versioned_pipeline_qualification.rs`, port `ProductionAdapterKind`, `ProductionInfrastructureProfile` (lines 148 to 155 and 284 to 313), `ProductionDependencyProfile` with `new(bundle, infrastructure)` and `for_bundle(service, package, infrastructure)` (it calls `service.bundle_qualification(package)`, maps an error to `Err(label.to_string())`, and calls `new`; `blockers` returns the bundle blockers then the infrastructure part of port lines 200 to 281; `runtime_identity_digest` is `evidence_hash` of the qualification's identities and `dependency_digest` as JSON), `validate_production_package` (lines 355 to 410; the known ids are the four `COMPATIBILITY_*_IMPLEMENTATION` constants), `BundleQualificationMetadata` and `BundleQualificationRecord` (lines 315 to 353), `PipelineQualificationStore` with `qualify_bundle` (lines 412 to 510; the conflict error becomes the label `bundle_qualification_identity_conflict`), and `qualification_from_row` (lines 610 to 626). Do not port `activate_qualified_bundle` (P4-D8).

- [ ] **Step 6: Run the tests.** The upgrade test and `migration_atomicity_pg` on fresh databases, `cargo test -p trace-commons-server --lib db::postgres`, the qualification unit tests, the two runtime tests, and `operational_summary_counts_runs_by_state_and_label`. Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add migrations/V103__versioned_pipeline_qualification.sql crates/trace-commons-server/src/db/postgres.rs crates/trace-commons-server/src/db/postgres/pipeline_upgrade_tests.rs crates/trace-commons-server/src/versioned_pipeline_product.rs crates/trace-commons-server/src/versioned_pipeline_qualification.rs crates/trace-commons-server/tests/versioned_pipeline_runtime_pg.rs
git commit -m "Record immutable pipeline bundle qualifications" -m "Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 7: Attempt artifacts and orphan cleanup (V104) (needs PR 3)

> **Amended 2026-09-30 (ruling R2-1):** the parts of this task about committed Score rows of withdrawn submissions (sweep selection (b), the `deleted` state, `deleted_at`, the `UPDATE (deleted_at)` grant, and the test `withdrawal_deletes_committed_score_objects_after_the_run_ends`) were removed after PR 3's `e54ac541` took over that deletion. The test `the_revocation_worker_deletes_every_object_of_a_withdrawn_complete_run` (ingest bin) now proves that the withdrawal and main's revocation-propagation worker delete the approved object and both Score objects. See P4-D10.

**Files:**
- Create: `migrations/V104__versioned_pipeline_attempt_artifacts.sql`
- Modify: `crates/trace-commons-server/src/db/postgres.rs`, `crates/trace-commons-server/src/db/postgres/pipeline_upgrade_tests.rs`, `crates/trace-commons-server/src/versioned_pipeline_product.rs`
- Modify: `crates/trace-commons-server/src/versioned_pipeline.rs` (the three attempt object write sites, `commit_review`, the Score commit, the sweep)
- Modify: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_runtime.rs` (`drain_pipeline_tenant`)
- Modify: `crates/trace-commons-server/tests/versioned_pipeline_runtime_pg.rs`

**Interfaces:**
- Consumes: `pipeline_attempt_object_id`, the write sites (search `pipeline_attempt_object_id(` in `versioned_pipeline.rs`: `approved`, `index-command`, `score-neighbors`), `TraceArtifactStore::{prepare_serialized_json, publish_serialized_json, delete_artifact, artifact_present_by_object_key}`, `sweep_staged_receipts` (the pattern), Task 2 `PIPELINE_LEASE_RENEWAL_CAP_FACTOR`.
- Produces: table `pipeline_attempt_artifacts`; `PipelineAttemptArtifact`; `PgPipelineStore::stage_attempt_artifact`; `PipelineService::sweep_attempt_artifacts`.

- [ ] **Step 1: Extend the upgrade test.** Add `pipeline_attempt_artifacts` to `PIPELINE_TABLES`, raise the expected maximum version to 104, and pin its grants: `SELECT`, `INSERT`, `DELETE`, and `UPDATE (state, committed_at, deleted_at)`.

- [ ] **Step 2: Write the failing tests** (assertion lists; `emit_pass_from_env("pipeline_orphan_sweep", ...)` at the end of the first):
  - `a_refused_score_commit_leaves_staged_rows_the_sweep_removes`: withdraw the submission after Score's object writes and before its commit (the seam that `score_commit_refuses_a_submission_withdrawn_after_the_read` uses); two `staged` rows (`index-command`, `score-neighbors`) exist for that lease token; `sweep_attempt_artifacts` before `cleanup_after` removes nothing; after `cleanup_after` is moved into the past, it removes both objects (`artifact_present_by_object_key` false) and both rows.
  - `a_committed_attempt_keeps_its_objects_and_a_stale_attempt_loses_them`: a stale worker's Review writes an approved object under its token and its commit is refused, while the second claim commits; after the sweep the committed object is present and its row is `committed`; the stale object and row are gone.
  - `withdrawal_deletes_committed_score_objects_after_the_run_ends`: a complete, indexed run; withdraw it; while `index_invalidation_state` is `pending`, the sweep keeps the Score objects; after the invalidation completes, the sweep deletes the `index-command` and `score-neighbors` objects and marks their rows `deleted`; the approved object is not readable (the existing withdrawal path deleted it; if it is readable, stop, write it down for PR 3, and do not delete it here).
  - `the_sweep_treats_an_absent_object_as_removed`: delete a staged object by hand; the sweep removes its row and does not fail.
  - `attempt_artifact_rows_are_tenant_scoped`: tenant B's sweep never touches tenant A's rows; tenant B reads none of them.
  - The crash matrix (`crash_matrix_produces_one_logical_effect_per_point`) and `review_crash_after_artifact_storage_reuses_one_revision` pass unchanged.

- [ ] **Step 3: Run them to see them fail** on a fresh database. Expected: FAIL.

- [ ] **Step 4: Write V104.**

```sql
-- The objects each pipeline phase attempt writes (delivery PR 4). A row is
-- staged before its object is published and committed in the same
-- transaction as the phase commit, so the worker can remove the objects of
-- an attempt that crashed, lost its lease, or had its commit refused, and
-- the Score objects of a withdrawn submission.

CREATE TABLE pipeline_attempt_artifacts (
    tenant_id TEXT NOT NULL,
    run_id UUID NOT NULL,
    lease_token UUID NOT NULL,
    artifact TEXT NOT NULL CHECK (artifact IN ('approved', 'index-command', 'score-neighbors')),
    object_key TEXT NOT NULL CHECK (object_key <> ''),
    ciphertext_sha256 TEXT NOT NULL CHECK (ciphertext_sha256 ~ '^[0-9a-f]{64}$'),
    state TEXT NOT NULL DEFAULT 'staged' CHECK (state IN ('staged', 'committed', 'deleted')),
    cleanup_after TIMESTAMPTZ NOT NULL,
    staged_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    committed_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, run_id, lease_token, artifact),
    UNIQUE (tenant_id, object_key),
    FOREIGN KEY (tenant_id, run_id)
        REFERENCES pipeline_runs (tenant_id, run_id)
        ON DELETE CASCADE,
    CHECK (
        (state = 'staged' AND committed_at IS NULL AND deleted_at IS NULL)
        OR (state = 'committed' AND committed_at IS NOT NULL AND deleted_at IS NULL)
        OR (state = 'deleted' AND deleted_at IS NOT NULL)
    )
);

CREATE INDEX pipeline_attempt_artifacts_due
    ON pipeline_attempt_artifacts (tenant_id, state, cleanup_after);
```

  Then the RLS block (V101 shape), the `trace_ingest_runtime` presence check (`V104:`), and the grants with a V101-style comment: `GRANT SELECT, INSERT, DELETE ON pipeline_attempt_artifacts TO trace_ingest_runtime;` and `GRANT UPDATE (state, committed_at, deleted_at) ON pipeline_attempt_artifacts TO trace_ingest_runtime;`. Register V104 after V103, and add the table to `TRACE_COMMONS_RLS_TABLES` and the `operational_summary` list.

- [ ] **Step 5: Implement.**
  - At each of the three write sites, replace the direct `put_serialized_json` with the receipt pattern: `prepare_serialized_json` (it returns the ciphertext hash), then `stage_attempt_artifact(run, kind, object_key, ciphertext_sha256, cleanup_after)` in its own tenant transaction (`INSERT ... ON CONFLICT DO NOTHING`; a conflict with another object key is a constraint error), then `publish_serialized_json`. `cleanup_after = now + PIPELINE_LEASE_RENEWAL_CAP_FACTOR x the phase lease + 1 hour`.
  - In `commit_review` and the Score commit transaction, before commit: `UPDATE pipeline_attempt_artifacts SET state = 'committed', committed_at = NOW() WHERE tenant_id = $1 AND run_id = $2 AND lease_token = $3 AND state = 'staged'`. A refused commit rolls this back with the rest.
  - `sweep_attempt_artifacts(tenant_id, limit)`: one tenant transaction, `FOR UPDATE SKIP LOCKED`, two selections up to `limit` in total: (a) `state = 'staged' AND cleanup_after <= NOW()`; (b) `state = 'committed' AND artifact IN ('index-command', 'score-neighbors')` for runs in state `complete` or `failed`, with `index_invalidation_state IN ('none', 'complete')`, whose submission has a `trace_withdrawals` row or a non-null `withdrawn_at`. For each row: when `artifact_present_by_object_key` is true, `delete_artifact` with the kind its write site uses (`approved` and the Score kinds as written there); on a delete failure log `pipeline_attempt_sweep_delete_failed` and keep the row; then delete the row (a) or set `state = 'deleted', deleted_at = NOW()` (b).
  - In `drain_pipeline_tenant`, after `sweep_staged_receipts`, call `sweep_attempt_artifacts(&tenant_id, PIPELINE_WORKER_MAX_SWEPT_ATTEMPT_ARTIFACTS_PER_TENANT)` (a new constant, 32) and log a failure with the label `pipeline_worker_attempt_sweep_failed`, as the receipt sweep does.

- [ ] **Step 6: Run the tests.** The five new tests, the whole runtime suite, the upgrade test, `migration_atomicity_pg`, and the real HTTP test on fresh databases. Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add migrations/V104__versioned_pipeline_attempt_artifacts.sql crates/trace-commons-server/src/db/postgres.rs crates/trace-commons-server/src/db/postgres/pipeline_upgrade_tests.rs crates/trace-commons-server/src/versioned_pipeline_product.rs crates/trace-commons-server/src/versioned_pipeline.rs crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_runtime.rs crates/trace-commons-server/tests/versioned_pipeline_runtime_pg.rs
git commit -m "Track pipeline attempt objects and sweep the orphans" -m "Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 8: Index rebuild from authoritative commands (needs PR 3)

**Files:**
- Modify: `crates/trace-commons-server/src/versioned_pipeline.rs`
- Modify: `crates/trace-commons-server/src/versioned_pipeline_index.rs`
- Modify: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_runtime.rs` (handler)
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` (route)
- Modify: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs` (route tests)
- Modify: `crates/trace-commons-server/tests/versioned_pipeline_runtime_pg.rs`

**Interfaces:**
- Consumes: `load_index_command(run, evidence)`, the committed Score outcome reader (`committed_decision` and the evidence reader beside it), the Settle index write loop (its entry keys), the PR 3 guard predicate, the vector worker credential gate (find the existing vector worker route and copy its auth shape).
- Produces: `PipelineIndexRebuildReport`, `PgPipelineStore::list_rebuildable_index_runs`, `PipelineService::{rebuild_index_from_authoritative_commands, index_writer}`, `IsolatedPipelineIndex::entry_set_hash`, `POST /v1/workers/pipeline/index-rebuild`.

- [ ] **Step 1: Write the failing tests** (assertion lists; `emit_pass_from_env("pipeline_index_rebuild", ...)` at the end of the first):
  - `index_rebuild_uses_sealed_commands_without_new_credit_or_outcomes`: three indexed runs (one with two chunks); record `entry_set_hash` of the live index; rebuild into a new `IsolatedPipelineIndex`; its `entry_set_hash` equals the recorded one; `command_count` 3; `entry_count` equals the sum of chunks; no new outcome, settlement row, or ledger row; a second rebuild into the same index reports every entry unchanged.
  - `index_rebuild_skips_withdrawn_and_invalidated_runs`: a withdrawn submission's run and a run with `index_invalidation_state = 'complete'` are not rebuilt.
  - `index_rebuild_fails_closed_on_a_tampered_command`: change the stored `index_command_hash`; the rebuild fails with `index_command_invalid` and writes no entry.
  - `index_rebuild_is_tenant_scoped`: tenant B's rebuild never reads tenant A's commands.
  - Ingest unit tests: the route returns 401 without a credential and 403 with a non-vector worker credential; with no pipeline runtime it returns 404; its error bodies are labels.

- [ ] **Step 2: Run them to see them fail.** Expected: FAIL.

- [ ] **Step 3: Implement.** Port `list_rebuildable_index_runs` (`ef97a459:crates/trace-commons-server/src/versioned_pipeline.rs` lines 618 to 655) with these predicates: `p.state = 'complete'`, `p.index_membership = 'included'`, `p.index_write_state = 'complete'`, `p.index_invalidation_state = 'none'`, `p.index_command_ref IS NOT NULL`, `p.index_command_hash IS NOT NULL`, and the PR 3 operable-submission predicate (reuse its SQL fragment; do not join `trace_derived_records`, which PR 2 replaced with the `approved_*` columns). Port `PipelineIndexRebuildReport` (lines 3349 to 3355) and `rebuild_index_from_authoritative_commands` (lines 3867 to 3905): read each run's committed Score evidence, load the command with `load_index_command(run, &evidence)` (it checks the hash, revision, index, and model), and write its entries with the same key derivation that Settle's index dispatch uses; `command_set_hash = sha256_prefixed(serde_json::to_vec(&command_hashes))`. Add `index_writer()`. Add `IsolatedPipelineIndex::entry_set_hash(&self, tenant_storage_ref, index_id) -> String` (SHA-256 over the sorted `(key, content_hash, embedding as little-endian f32 bytes)`). The handler reads the tenant from the worker credential, calls the rebuild with `service.index_writer()`, and returns `{"command_count", "entry_count", "unchanged_entry_count", "command_set_hash"}`; register it beside the other `/v1/workers/` routes.

- [ ] **Step 4: Run the tests.** Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-server/src/versioned_pipeline.rs crates/trace-commons-server/src/versioned_pipeline_index.rs crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_runtime.rs crates/trace-commons-server/src/bin/trace-commons-ingest.rs crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs crates/trace-commons-server/tests/versioned_pipeline_runtime_pg.rs
git commit -m "Rebuild the pipeline index from its sealed commands" -m "Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 9: Corpus run harness, `run`, and `package` (needs PR 3)

**Files:**
- Create: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_corpus_pg_tests.rs`
- Modify: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs` (declare the module inside `tests`, as `pipeline_http_pg_tests` is declared; add `test_artifact_store_with_key(root, key_hex)`)
- Modify: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_http_pg_tests.rs` (make shared helpers `pub(super)`)
- Modify: `scripts/operator/pipeline.py`, `scripts/operator/pipeline_tooling/corpus.py`, `scripts/operator/test_pipeline_tooling.py`

**Interfaces:**
- Consumes: from `pipeline_http_pg_tests.rs`: `pipeline_http_database_url`, `runtime_backend`, `serve_pipeline_app`, `wait_for_pipeline_ready`, `tenant_tx`, and the assembler and state setup of `compatibility_bundle_through_http_with_review_privacy_withdrawal_and_export` (authority provider, privacy boundary, payout disabled); the review routes; `/v1/contributors/me/submission-status`; `/v1/admin/pipeline/runs/{run_id}/forensic`; Task 1 trust store and emitter; Task 4 corpus files.
- Produces: the ignored tests `pipeline_corpus_run` and `pipeline_package_write`; `pipeline.py run` and `pipeline.py package`; the corpus report schema `trace_commons.pipeline_corpus_report.v1`.

- [ ] **Step 1: Write the failing tests.**
  - Rust unit tests (no database) in the new file: `corpus_file_refuses_duplicate_ids_unsafe_labels_and_empty_probes` (the port's `CorpusFile::validate` rules, lines 297 to 341 of `ef97a459:crates/trace-commons-server/src/bin/trace-commons-pipeline-local.rs`); `fixture_expectations_default_from_the_outcome_count` (P4-D16: `main`'s rejected fixture gets scoring `missing` and settlement `incomplete`; an admitted fixture gets `complete` and the bundle's award count).
  - The ignored harness `pipeline_corpus_run` (it is the `run` implementation; its assertions are the test): read the corpus (`TRACE_COMMONS_PIPELINE_CORPUS_PATH`; with `TRACE_COMMONS_PIPELINE_CORPUS_HOLDOUT_PATH` also set, the first path is the HF bootstrap partition and the second the holdout partition, run in that order against one app, one database, and one index), the check id to emit (`TRACE_COMMONS_PIPELINE_CORPUS_CHECK_ID`, one of `pipeline_http_corpus_minimal`, `pipeline_http_corpus_compatibility`, `pipeline_http_corpus_hf_local`, `pipeline_http_corpus_package`; any other value fails `corpus_check_id_invalid`), the bundle (`TRACE_COMMONS_PIPELINE_CORPUS_BUNDLE`: `minimal` gives PR 2's minimal package with the `storage_rebate` award, `compatibility` gives `compatibility_package(local_reference with novelty_utility_microcredits 2_500_000)`), or a signed package (`..._PACKAGE_PATH` and `..._TRUSTED_KEY_PATH`, verified with `BundlePackageTrustStore`; a package with a bundle variable fails `corpus_package_and_bundle_conflict`). Serve the app with tenant `tenant-lab` routed and tenant `tenant-other` configured. For each fixture, in file order:
    - `POST /v1/traces` with the fixture's envelope (build a Medium or High risk envelope as the PR 3 compatibility HTTP test builds its Medium one); the response is `processing`.
    - For an expected `quarantine`, claim and assess through the review routes with the fixture's `review_recommendation`.
    - Wait (60 s bound) until the run is terminal, reading only through HTTP: the status route for the processing state and the forensic route for the phase outcomes.
    - Assert the admission decision, the outcome count, the consent, privacy, scoring, and settlement states, and the instrument count against the (derived) expectations.
    - Replay the same request bytes: same run (`replay_same_run`). Send changed content under the same idempotency key: refused (`changed_content_refused`).
    - `tenant-other`'s credential gets 404 for the fixture's status and forensic reads.
    - Write the bounded report (`..._REPORT_PATH`): schema, bundle id, package hash, corpus digest, configuration digest, fixture order, and per fixture the labels and booleans above and the outcome hashes (the port's `FixtureReport` fields, lines 418 to 470, without `run_id` values; run ids appear only as SHA-256 hashes).
    - Assert that no fixture `secret_probe` and no `server_privacy_probe` appears in the report bytes, the evidence, or any HTTP error body the harness saw.
    - Emit the check id from `TRACE_COMMONS_PIPELINE_CORPUS_CHECK_ID` once, after every partition, with evidence `{"fixtures": <count>, "completed": <count>, "replay_same_run": <count>, "changed_content_refused": <count>, "tenant_isolation": true, "report_hash": <sha256 of the report file>}`. For two partitions the report holds one section for each, and both name the same bundle id.
  - The ignored tool `pipeline_package_write`: builds the named bundle's package, signs it with the PKCS#8 key at an optional path or a generated one, and writes the signed package and the trusted key JSON to the two output variables. Assertion: the written package verifies with the written key.
  - Python: `test_run_passes_only_the_expected_variables` (with a fake runner, `run --bundle minimal --corpus X` calls `cargo_test` for `pipeline_corpus_run` with `--ignored --exact` and exactly the database, corpus, check id, bundle, report, artifact root, master key, and check variables); `test_run_refuses_package_and_bundle_together`; `test_hf_pin_runs_bootstrap_then_holdout` (a pin corpus exports first, then runs the harness one time with the bootstrap path, the holdout path, and the check id `pipeline_http_corpus_hf_local`).

- [ ] **Step 2: Run them to see them fail.** Expected: FAIL.

- [ ] **Step 3: Implement.**
  - Rust: the module above; make the listed helpers `pub(super)`; `test_artifact_store_with_key` builds `SecretsCrypto` from the given hex key (the harness uses `TRACE_COMMONS_PIPELINE_TEST_MASTER_KEY_HEX` when it is set, else a generated key). Port the fixture and report types and the request builder from the port file (lines 292 to 470 and 1580 to 1633) and the per-fixture checks from `fixture_report` (lines 1794 to 1927), reading through HTTP instead of the removed inspect route.
  - Python `pipeline.py run`: `--bundle {minimal,compatibility}` or `--package PATH --trusted-key PATH` (exactly one form; the check id is `pipeline_http_corpus_<bundle>`, `pipeline_http_corpus_package`, or for a pin `pipeline_http_corpus_hf_local`), `--corpus PATH` (default `docs/superpowers/specs/fixtures/versioned-pipeline-minimal-corpus-v1.json`), `--corpus-digest`, `--postgres-admin-url`, `--archive`. A corpus whose schema is the pin schema goes through `export_hf_corpus` and two harness runs. After the run: validate the report with the port's `safe_report_value` and `validate_report` rules (`lab.py` lines 122 to 216, adapted to the new schema), validate the check result, write `.local/pipeline-<bundle>-corpus-report.json` and `.md` (port `markdown`, lines 238 to 251), print one summary line (`PipelineRunOK: bundle=<hash> fixtures=<n>`), and archive only with `--archive`.
  - Python `pipeline.py package`: `--bundle`, `--output`, `--public-key-output`, optional `--signing-key` and `--key-id` (both or neither); it runs `pipeline_package_write`.

- [ ] **Step 4: Run the tests.** Rust unit tests, Python self-tests, then `python3 scripts/operator/pipeline.py run --bundle minimal`, `python3 scripts/operator/pipeline.py run --bundle compatibility`, `python3 scripts/operator/pipeline.py run --bundle compatibility --corpus crates/trace-commons-server/tests/fixtures/pipeline-hf-jsonl/pin-local.json`, and a signed-package run (`pipeline.py package --bundle minimal --output .local/package.json --public-key-output .local/trusted-key.json`, then `run --package .local/package.json --trusted-key .local/trusted-key.json`). Expected: PASS; no catalog file changes. Prove one failure by hand and restore: change one fixture's `expected_admission_decision` in a copy of the corpus and run it (fails; the report shows the mismatch; exit code nonzero).

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_corpus_pg_tests.rs crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_http_pg_tests.rs scripts/operator/pipeline.py scripts/operator/pipeline_tooling scripts/operator/test_pipeline_tooling.py
git commit -m "Run a pipeline corpus through the shared ingest app" -m "Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 10: Restore drill (needs PR 3)

**Files:**
- Create: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_restore_pg_tests.rs`
- Modify: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs` (declare the module)
- Modify: `scripts/operator/pipeline.py`, `scripts/operator/pipeline_tooling/environment.py` (dump and restore), `scripts/operator/test_pipeline_tooling.py`

**Interfaces:**
- Consumes: Task 9 harness helpers; Task 8 rebuild and `entry_set_hash`; `PipelineCrashPoint::AfterSettleSelection`; `provision_member_only_login`.
- Produces: the ignored tests `pipeline_restore_seed` and `pipeline_restore_resume`; `pipeline.py restore-drill`; check `pipeline_restore_drill`.

- [ ] **Step 1: Write the failing tests.**
  - `pipeline_restore_seed` (ignored): database from `pipeline_http_database_url()` (it creates `<db>_pilot`), artifact root from `TRACE_COMMONS_PIPELINE_ARTIFACT_ROOT`, key from `TRACE_COMMONS_PIPELINE_TEST_MASTER_KEY_HEX`. Submit the first minimal corpus fixture and let it complete. Submit a second receipt with the crash point `AfterSettleSelection`, and stop the app when the selection is durable (`wait_for_settle_selection`). Write `TRACE_COMMONS_PIPELINE_RESTORE_FINGERPRINT_PATH`: the authoritative fingerprint (the port's SQL, `ef97a459:scripts/operator/pipeline-backup-restore-smoke.sh` lines 134 to 173, run in tenant transactions over `pipeline_runs`, `phase_outcomes`, `pipeline_run_settlements`, and `pipeline_bundle_packages`, hashed with SHA-256), the artifact fingerprint (SHA-256 over sorted relative paths and file hashes, port lines 191 to 205), the index `entry_set_hash`, the pending run id hash, and the recording adapter's request count.
  - `pipeline_restore_resume` (ignored): `TRACE_COMMONS_PG_TEST_DATABASE_URL` names the restored database directly. A new helper `restored_database_url()` does not append `_pilot`, does not drop, and does not migrate; it requires `_trace_commons_migrations` to be at the latest version. Call `provision_member_only_login` for the runtime login (idempotent). Assertions:
    - The runtime role is not superuser and does not bypass RLS, and every `pipeline_*` table forces RLS in the restored database.
    - The database fingerprint and the artifact fingerprint equal the seed's.
    - A rebuild into a new `IsolatedPipelineIndex` gives the seed's `entry_set_hash`.
    - Start the app on the restored database and the restored root, with that index. The pending run completes (60 s bound), with no manual processor call.
    - The completed run's settlement rows and outcomes are unchanged. Each run has exactly one outcome for each phase it reached. The new recording adapter received requests only for the pending run's legs. No credit event or settlement row is duplicated.
    - Call `PipelineCheckEmitter::emit_from_env("pipeline_restore_drill", PipelineCheckStatus::Pass, Some(&package), &["filesystem_restore_local_only"], json!({"database_fingerprint": .., "artifact_fingerprint": .., "index_entry_set_hash": .., "pending_runs_resumed": 1, "duplicate_effects": 0}))`. A pass result may carry safe blockers; they are promotion blockers, and `qualify` copies them into the report.
  - Python: `test_restore_drill_order` (with fakes: seed, dump of `<db>_pilot`, `createdb <db>_restored`, restore, artifact copy with a byte comparison, resume, in that order; a byte difference fails with `restore_artifact_bytes_mismatch` before resume); `test_restore_uses_privileges_and_the_same_cluster` (the restore command has no `--no-privileges` and no `--no-owner`).

- [ ] **Step 2: Run them to see them fail.** Expected: FAIL.

- [ ] **Step 3: Implement.** The two tests; `Environment.dump(database, path)` and `Environment.restore(path, database)` (container mode: `docker exec <c> pg_dump -U trace -Fc -f /tmp/<run>.dump <db>` and `docker exec <c> pg_restore -U trace -d <dest> /tmp/<run>.dump`; admin-url mode: the host tools with `-h 127.0.0.1 -p <port> -U trace` and a dump file with mode 0600 in the run directory); `pipeline.py restore-drill` (one scenario; a random test master key for both processes; the steps in the Python test's order; print `PipelineRestoreOK` and state that filesystem restore is local evidence).

- [ ] **Step 4: Run the tests.** Python self-tests, then `python3 scripts/operator/pipeline.py restore-drill`. Expected: PASS. Prove one failure by hand and restore: flip one byte of one restored artifact file between the copy and the resume (a temporary code change in the drill); the drill fails with `restore_artifact_bytes_mismatch`.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_restore_pg_tests.rs crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs scripts/operator/pipeline.py scripts/operator/pipeline_tooling scripts/operator/test_pipeline_tooling.py
git commit -m "Restore a pipeline database and resume its pending work" -m "Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 11: Results in the existing suites, and `qualify` (needs PR 3)

**Files:**
- Modify: `crates/trace-commons-server/tests/versioned_pipeline_runtime_pg.rs`, `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_http_pg_tests.rs`, `crates/trace-commons-server/src/db/postgres/pipeline_upgrade_tests.rs` (one emit call at the end of each test in the table)
- Create: `scripts/operator/pipeline_tooling/catalog.py`, `report.py`
- Modify: `scripts/operator/pipeline_tooling/checks.py`, `scripts/operator/pipeline.py`, `scripts/operator/test_pipeline_tooling.py`

**Interfaces:**
- Consumes: Tasks 1 to 10.
- Produces: `REQUIRED_DATABASE_CHECKS`, `REQUIRED_CHECK_IDS`, `pipeline.py qualify [--archive] [--postgres-admin-url URL]`, the report `trace_commons.pipeline_qualification_report.v1` at `.local/pipeline-qualification-report.json`.

The required database checks (each row is one exact test that emits its check id after its assertions; `digests` says whether the three package digests are required):

| Check id | Target and test | Database | Digests |
| --- | --- | --- | --- |
| `pipeline_storage_upgrade_rls` | `--lib`, `db::postgres::pipeline_upgrade_tests::pipeline_upgrade_from_v91_installs_forced_rls_storage`, ignored | upgrade | no |
| `pipeline_crash_matrix` | `--test versioned_pipeline_runtime_pg`, `crash_matrix_produces_one_logical_effect_per_point` | runtime | yes |
| `pipeline_independent_instruments` | same target, `independent_instruments_retry_without_repeating_a_completed_one` | runtime | yes |
| `pipeline_stale_lease_fence` | same target, `stale_lease_cannot_commit_after_reclaim` | runtime | yes |
| `pipeline_lease_renewal` | same target, `a_score_longer_than_its_lease_completes_once_with_two_workers` | runtime | yes |
| `pipeline_receipt_replay_exact` | same target, `receipt_replay_and_conflict_are_exact` | runtime | yes |
| `pipeline_payout_recovery` | same target, `payout_crash_between_submit_and_confirm_submits_once` | runtime | yes |
| `pipeline_index_rebuild` | same target, `index_rebuild_uses_sealed_commands_without_new_credit_or_outcomes` | runtime | yes |
| `pipeline_orphan_sweep` | same target, `a_refused_score_commit_leaves_staged_rows_the_sweep_removes` | runtime | yes |
| `pipeline_bundle_qualification` | same target, `qualification_inspects_the_objects_the_constructor_receives` | runtime | yes |
| `pipeline_http_restart_recovery` | `--bin trace-commons-ingest`, `real_http_receipt_completes_and_resumes_after_restart` | runtime (`_pilot`) | yes |
| `pipeline_http_receipt_ownership` | same target, `real_http_pipeline_receipt_checks_ownership_on_replay` | runtime (`_pilot`) | yes |

Plus the corpus checks (`pipeline_http_corpus_minimal`, `pipeline_http_corpus_compatibility`, `pipeline_http_corpus_hf_local`; Task 9) and `pipeline_restore_drill` (Task 10). Before the database checks, `qualify` runs the binding checks by exit status: `pipeline.py test` (`contracts` and `runtime`), `migration_atomicity_pg` in its own scenario, and the tooling self-tests.

- [ ] **Step 1: Write the failing Python self-tests:**
  - `test_required_checks_are_a_subset_of_promotion_checks`: parse `PROMOTION_REQUIRED_CHECKS` out of `crates/trace-commons-server/src/versioned_pipeline_qualification.rs`; `REQUIRED_CHECK_IDS` is a subset; the three promotion-only ids are not in `REQUIRED_CHECK_IDS`.
  - `test_qualify_fails_when_a_required_check_emits_nothing`: with a fake runner whose cargo calls succeed but one check writes no result, `qualify` fails with `check_result_missing:<id>`, and the report says `status: fail`.
  - `test_qualify_runs_each_check_in_a_fresh_scenario`: every database check gets its own scenario, and the xact guard runs after each.
  - `test_routine_runs_do_not_archive`: `qualify` without `--archive` leaves `.local/pipeline-lab-catalog.json` absent (in a temporary root); with `--archive` it writes the catalog and the content-addressed records; archiving twice gives the same catalog; a changed record at an existing digest fails with `immutable_record_conflict` (port `lab.py` lines 254 to 323).
  - `test_report_contains_no_private_fields`: the report passes `validate_evidence`, holds `production_promotion_ready: false`, and lists the local blockers `local_reference_scorer`, `local_reference_embedder`, `synthetic_index`, `synthetic_settlement`, `static_bearer_authentication`, `filesystem_restore_local_only`, `hf_network_canary_not_run`.

- [ ] **Step 2: Run them to see them fail.** Expected: FAIL.

- [ ] **Step 3: Implement.**
  - Add one call at the end of each test in the table whose test does not emit yet (Tasks 2, 5, 7, and 8 added theirs), after its last assertion: `PipelineCheckEmitter::emit_pass_from_env("<check_id>", Some(&package), json!({...}))` with bounded observed values that the test already asserts (counts, labels, outcome hashes); the upgrade test passes `None` for the package and `{"tables": <n>, "forced_rls": <n>, "maximum_version": 104}`.
  - `checks.py`: `DatabaseCheck(check_id, cargo_args, test_name, database, ignored, digests)` and the table rows; `REQUIRED_CHECK_IDS`.
  - `report.py`: port the report shape of `ef97a459:scripts/operator/run-pipeline-qualification.sh` lines 50 to 176 with executed results: `inputs` (code revision hash, corpus digests, bundle ids and package hashes of the corpus runs, `inventory_digest` from the inventory script's output, `contract_manifest_digest` of the manifest file), `checks` (the validated results), `safe_blockers` (the list in the Step 1 test), `status`, and `evidence_hash = sha256(canonical({"inputs", "checks"}))`. Write it to the run directory and to `.local/pipeline-qualification-report.json`.
  - `catalog.py`: port `archive` and `update_catalog` (`lab.py` lines 254 to 323) with the allowed record schemas `trace_commons.pipeline_qualification_report.v1`, `trace_commons.pipeline_corpus_report.v1`, and `trace_commons.pipeline_hf_corpus_manifest.v1`.
  - `pipeline.py qualify`: the brief's orchestration.

```python
def qualify(args, run):
    run_binding_checks(run, args)
    with Environment(run, admin_url=args.postgres_admin_url) as env:
        for check in REQUIRED_DATABASE_CHECKS:
            with env.scenario(check.check_id) as scenario:
                run_database_check(run, scenario, check)
        run_corpus_checks(run, env)
        run_restore_drill(run, env)
    results = load_results(run)
    require_current_pass_results(run, results, required_specs())
    report = write_report(run, results)
    if args.archive:
        update_catalog(CATALOG_PATH, report, records=corpus_records(run))
    print(f"PipelineQualificationOK: report={report.relative_to(ROOT)}")
```

    A failure anywhere still writes the report with `status: fail` and the safe label, then raises.

- [ ] **Step 4: Run the tests.** Python self-tests, then `python3 scripts/operator/pipeline.py qualify` (PASS; no catalog change), then `python3 scripts/operator/pipeline.py qualify --archive` (PASS; catalog written). Prove each failure by hand and restore it (brief section 4 acceptance):
  1. Break database setup: make the environment skip the login-resolver roles; `qualify` fails.
  2. Break result identity: make the emitter write another run id; `qualify` fails with `check_result_foreign_run`.
  3. Break result presence: comment out one emit call; `qualify` fails with `check_result_missing:<id>`.
  4. Break corpus bytes: change one byte of a corpus copy passed with `--corpus-digest`; `run` fails with the digest mismatch.
  5. Break corpus order and bytes: swap the contents of the two local JSONL fixture files in a copy and point a copy of `pin-local.json` at it; the export binary fails its source digest check (the source digest binds each name to its bytes and order), and `run` fails with a nonzero exit.
  6. Break the secret check: make the harness copy one `secret_probe` into the report; the harness fails.
  7. Break cleanup: make `Environment.__exit__` skip `docker rm`; an otherwise green `qualify` fails with `cleanup_failed`.
  8. Confirm that test adapters stay unqualified: the report lists the local blockers and `production_promotion_ready` is false.
  Record each result in the ledger.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-server/tests/versioned_pipeline_runtime_pg.rs crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_http_pg_tests.rs crates/trace-commons-server/src/db/postgres/pipeline_upgrade_tests.rs scripts/operator/pipeline.py scripts/operator/pipeline_tooling scripts/operator/test_pipeline_tooling.py
git commit -m "Qualify the pipeline from executed check results" -m "Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 12: CI job and runbooks (needs PR 3)

**Files:**
- Modify: `.github/workflows/ci.yml`
- Modify: `docs/operator/pipeline-qualification.md`, `docs/operator/pipeline-lab.md`, `docs/operator/pipeline-activation.md`, `docs/operator/backup-restore.md`, `docs/operator/README.md`
- Modify: `README.md`, `CLAUDE.md`

**Interfaces:**
- Consumes: `pipeline.py qualify`; the owner's answer to question 1.

- [ ] **Step 1: Add the job** after `ingest-bin-postgres`:

```yaml
  pipeline-qualification:
    # Not a required status check (owner decision P4-D17): it runs on every
    # pull request and in the merge queue, and it blocks nothing until the
    # owner promotes it. `pipeline.py` starts its own digest-pinned
    # PostgreSQL container, so this job has no `services:` block.
    name: pipeline qualification and restore
    runs-on: ubuntu-latest
    timeout-minutes: 60
    steps:
      - uses: actions/checkout@v6
      - uses: dtolnay/rust-toolchain@stable
      - uses: actions/cache@v5
        with:
          path: |
            ~/.cargo/registry/index
            ~/.cargo/registry/cache
            ~/.cargo/git/db
            target
          key: ${{ runner.os }}-cargo-pipeline-qualification-${{ hashFiles('**/Cargo.lock') }}
          restore-keys: |
            ${{ runner.os }}-cargo-pipeline-qualification-
      - uses: actions/setup-python@v6
        with:
          python-version: '3.12'
      - name: pipeline tooling self-tests
        run: python3 scripts/operator/test_pipeline_tooling.py
      - name: pipeline qualify
        run: python3 scripts/operator/pipeline.py qualify
      # Last, so every step above still has its binaries; see the action for
      # what is removed and why.
      - uses: ./.github/actions/trim-cargo-cache
```

  If the owner makes it required, replace the first comment line with the required-check wording that `ingest-bin-postgres` uses, and change README and CLAUDE.md from eleven to twelve.

- [ ] **Step 2: Rewrite `docs/operator/pipeline-qualification.md`** (it describes port tooling that does not exist). Status line: the tooling exists; production routing is off until PR 5. Sections: what `test`, `qualify`, and `restore-drill` check; the environment (container digest, `--postgres-admin-url`, one command at a time on one server); the result contract and what makes a result invalid; the outputs under `.local/`; `--archive` and the catalog; what local evidence is not (the blocker list; remote-provider restore, real adapters, and the HF network canary are promotion work); package trust and `qualify_bundle` (the admin route and activation are PR 5).

- [ ] **Step 3: Rewrite `docs/operator/pipeline-lab.md`** as "Local pipeline corpus runs": `pipeline.py run` with `--bundle`, `--package`, the HF pin, `pipeline.py package`, the report fields, isolation and privacy (fixture probes never leave the run), and failure labels. Remove every reference to `scripts/operator/lab/` and `trace-commons-pipeline-local`.

- [ ] **Step 4: Update `docs/operator/pipeline-activation.md`** (keep its "design stage" line): the lease section describes renewal and its cap (remove "PR 4" wording); the dependency qualification section describes the per-bundle rule; the "Receipt staging and the orphan sweep" section adds the attempt artifact sweep and the withdrawal rule for Score objects.

- [ ] **Step 5: Update `docs/operator/backup-restore.md`**: a "Versioned pipeline" section: what the drill proves, that filesystem restore is local evidence, the rebuild route and its credential, and the order after a real restore (restore PostgreSQL and objects, rebuild the index, start ingest).

- [ ] **Step 6: Update `docs/operator/README.md`** (the index lines for the two rewritten runbooks), `README.md` (the CI job count sentence), and `CLAUDE.md` ("Eleven of the seventeen" becomes "Eleven of the eighteen", and one line under CI for the new job).

- [ ] **Step 7: Check the YAML.** Run `git diff --check`. Run `actionlint .github/workflows/ci.yml` if `actionlint` is installed; if it is not, record in the ledger that the job was checked by reading only (PR 4 is local, so no CI run exists yet). Push nothing.

- [ ] **Step 8: Commit**

```bash
git add .github/workflows/ci.yml docs/operator/pipeline-qualification.md docs/operator/pipeline-lab.md docs/operator/pipeline-activation.md docs/operator/backup-restore.md docs/operator/README.md README.md CLAUDE.md
git commit -m "Run pipeline qualification in CI and document the tooling" -m "Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 13: PR 4 gate (needs PR 3; no commit)

Owner feedback (2026-09-30, project memory "Publish, then let CI verify"): no multi-hour local gates and no per-commit test builds. When the branch compiles cleanly and its review found no Critical issue, it is ready to publish (with the owner's go-ahead), and the upstream CI runs the full matrix. Give a time estimate before any step that takes more than about 30 minutes.

- [ ] **Step 1:** Every check in Global Constraints, from the worktree root, at the tip only (about 10 minutes).
- [ ] **Step 2:** `python3 scripts/operator/pipeline.py qualify` one time, on a fresh container. It is PR 4's own deliverable, and the CI job runs the same command. Estimate its time from Task 11's run and tell the owner before it starts if it is longer than 30 minutes.
- [ ] **Step 3:** Leave to the upstream CI when PR 4 is published: `cargo test --workspace`, the feature checks, the `cargo deny` runs, the MSRV floor, and the whole ingest bin against PostgreSQL.
- [ ] **Step 4:** Record the candidate revision, every executed check, the evidence location, and the open items ("Items for other PRs") in the ledger. Do not push.

---

## Self-review (done while writing)

- **Spec coverage (brief sections 3A, 4, 6, 7):**
  - One entry point with `test`, `run`, `qualify`, `restore-drill`: Tasks 3, 9, 10, 11. Target CLI forms: `test --check contracts|runtime|postgres` (Task 3), `run --bundle`, `run --package --trusted-key`, the HF pin run (Task 9), `qualify`, `qualify --archive`, `restore-drill` (Tasks 10, 11).
  - Shared environment (one container, scenarios with their own database and artifact root, runtime role checks, cleanup that keeps the primary failure): Task 3, and the RLS assertions in Tasks 10 and 11.
  - One corpus path (direct corpus and pin, both through the same run): Tasks 4 and 9.
  - Executed result contract (emit after assertions, accept, reject, report, retain, restore): Tasks 1, 3, 10, 11.
  - Command transition and CI name and triggers: P4-D13, P4-D17, Task 12.
  - Section 4 acceptance (deliberate breaks, cleanup, no duplicates, no archive, unqualified test adapters): Task 11 Step 4, Task 10, Task 3.
  - Section 3A qualification row: Task 5 (shared objects, substitution, changed bytes, tampering, missing versions), Task 6.
  - Section 6 (own migrations): V103 and V104. Section 7 gate: Task 13, shortened on 2026-09-30 by owner feedback (the upstream CI runs the full matrix).
  - Items passed from earlier PRs: P3-D15 (Tasks 7, 8; P4-D10, P4-D11), lease renewal (Task 2; P4-D9), FR5 P4 (Task 5; P4-D7), T13-1 (Tasks 6, 7; P4-D21), `default_package_hash` (P4-D12), fixed test role names (P4-D6), withdrawn Score objects (Task 7).
  - Stale documents on `main`: `pipeline-qualification.md`, `pipeline-lab.md` rewritten; `pipeline-activation.md` and `backup-restore.md` updated (Task 12). The package qualification spec on `main` needs no change for PR 4: its promotion evidence and dependency list read correctly for each bundle; `activate_qualified_bundle` stays as PR 5 work.
- **Placeholders:** port steps name exact files and line ranges and the changes; new logic is written out; tests given as assertion lists follow the recorded owner decision. Two values are confirmed during execution, with the command given: the image index digest (Task 3) and the local HF corpus digests (Task 4).
- **Type consistency:** `PipelineCheckEmitter::emit_pass_from_env` (Task 1) is the call in Tasks 2, 5, 7, 8, 9, 11, and `emit_from_env` (with the safe blocker) in Task 10; `PipelineBundleQualification` (Task 5) is what `ProductionDependencyProfile::for_bundle` (Task 6) stores; `PIPELINE_LEASE_RENEWAL_CAP_FACTOR` (Task 2) sets `cleanup_after` in Task 7; `entry_set_hash` (Task 8) is used in Task 10; every check id in Task 11's table and in `PROMOTION_REQUIRED_CHECKS` (Task 1) is the same string.
- **Review Focus:** each of the five lines has a named test in its owning task.
