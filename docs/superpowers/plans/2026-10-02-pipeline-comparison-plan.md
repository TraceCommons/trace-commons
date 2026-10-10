# Pipeline comparison (`pipeline.py compare`) implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** Draft of 2026-10-02, revised on 2026-10-08 for the owner's annotations, for the merge of PR 5, and for the five-lens plan review (see "Plan review of 2026-10-08" near the end). All twelve owner questions have their answers (2026-10-08), and the tasks have the changes for them (PC-D18 to PC-D21). One separate check read the text that changed after the plan review, and its five findings are corrected. The owner approved the plan on 2026-10-08. From Phase B on, the workstream runs on the machine `hetzner` (owner, 2026-10-08).

**Correction of 2026-10-09 (final review):** five sentences on the baseline are corrected. The gate values are those of `CompatibilityBundleConfig::local_reference()` and not `main`'s default values (`top_k` is 5 since the owner decision PC-D23 of 2026-10-09), and a comparison run reads no dedup signal rows.

**Correction of 2026-10-10 (review of `c76306cf`):** `main` now has #1324 (the pipeline's classifier rescrub moves to a privacy pass at the start of Review) and #1325 (the pipeline applies `main`'s duplicate short-circuits to its gate decision row). The run compares neither. The report names the two gaps with two new blockers (PC-D24, PC-D25), spec section 17 point 2 no longer says that the pipeline has no duplicate step, PC-D22 says that an empty basis meets its condition, and the validator checks the distribution of each side and the exact list of excluded rules (PC-D26).

**Correction of 2026-10-11 (PC-D27):** the owner ruled that the reject of a High trace by the pipeline is intended (spec section 8.3 row 4). The rule `high_risk_admission_reject` permits it, the self-test's risk pin now passes with two permitted pairs, and the label `compare_self_test_risk_passed` is removed.

**Goal:** Build `pipeline.py compare`: a command that sends the same real traces through `main`'s old gate path and through the versioned pipeline with a production-compatible compatibility bundle, compares the content-dependent decisions for each trace, and fails on any difference that no named rule permits.

**Architecture:** A pure comparison module in the server library holds the record type, the comparison function, the alignment table, the floor calculation, and the report builder. An ignored Rust test in the ingest binary hosts one app with two tenants (baseline on the old path, candidate on the pipeline), calibrates the gate floors from the bootstrap partition, sends each trace to the two tenants in serial order over real HTTP, reads the results from the database, and writes hash-only records and a report. `pipeline.py compare` exports the pinned sample (with session events, as a JSONL corpus), starts the test in a disposable PostgreSQL environment, and validates the report.

**Tech Stack:** Rust (axum, tokio, tokio-postgres, serde, sha2, hf-hub, clap; all are existing dependencies), PostgreSQL 16, Python 3 standard library only (`scripts/operator/pipeline.py` and `pipeline_tooling/`), GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-10-02-pipeline-comparison-design.md` (binding; section numbers below refer to it). Also on `main`: `docs/superpowers/specs/2026-09-14-versioned-pipeline-compatibility-mapping.md`, `docs/operator/pipeline-lab.md`, `docs/operator/pipeline-qualification.md`. Shared rules: `.superpowers/workstreams/common.md` in the main checkout.

**Scope:** The tool, its CI check, its runbook, the network pin, a 100-trace run, and (after two owner decisions) the full run. It adds no migration, no production route, and no check to `pipeline.py qualify`.

**Test bodies:** as in PR 2 to PR 5 (owner decision recorded in the PR 2 ledger), a test given as a list of assertions is written by the implementer as real calls. Every listed assertion is required.

## What the tool compares, in plain words

The tool sends one trace to two versions of the server and checks that the two versions make the same decisions about it.

- **Baseline:** the old gate path that runs on `main` today.
- **Candidate:** the versioned pipeline, with a bundle that is configured to behave as the old path does.

For each trace, the tool compares the answers to these questions:

| Question | Compared values |
|---|---|
| How private is the trace? | The privacy risk and its basis. |
| Does the server accept it? | Admit, quarantine, or reject. |
| Is the text good enough? | The quality values and the pass or fail result. |
| Is the text new? | The novelty values and the pass or fail result. |
| How was the text divided? | The chunk counts, and whether the cap removed chunks. |
| What went into the index? | The index size at scoring time, and the chunks that became members. |
| How much credit does the contributor get? | The credit quality value and the credit events. |

Each value must be exactly equal on the two sides (decision D4). The tool does not compare averages or distributions, for these reasons:

- The two sides get the same bytes, the same scorer, the same embedder, and the same floors, and each side is deterministic. The expected difference is thus zero. A difference of one unit is a code difference.
- A decision is a threshold for one trace, and credit is paid for one trace. Two sides can have equal averages while one contributor gets credit on only one side.
- An exact comparison finds one defect in 10,000 traces. A comparison of averages cannot find it.

The alignment (spec section 8.2, Review Focus 2) follows from the exact comparison. `index_cardinality` is a compared field. If one side adds a trace to its index and the other side does not, that field differs for each later trace. One cause then gives thousands of `unexplained` results. The alignment keeps the two indexes equal, so each `unexplained` result is independent evidence, and the first result names the cause.

## Words used in this plan

- **Dataset:** the public Hugging Face repository `jedisct1/security-audits`. Each file is one recorded agent session.
- **Pin:** a small JSON file in git (schema `trace_commons.pipeline_hf_corpus_pin.v1`) that fixes one sample of the dataset. It holds the repository, the revision, the selection values (the counts and the word limits), and the digests that the selected files must have. Two runs with one pin read the same traces in the same order. A **local pin** has the field `local_jsonl_dir` and reads session files that are committed in this repository, so it needs no network. The **network pin** has no such field and reads the dataset.
- **Export:** the step that makes the corpus from a pin. The binary `trace-commons-pipeline-corpus-export` reads the files that the pin selects, translates each session, and writes the corpus files and `source-manifest.json`. For a local pin, it reads the local directory. For the network pin, it reads a cache directory and first downloads the files that are not in the cache. "The export downloads" means this download. The dataset is not the export; the export is the program step that reads the dataset.
- **Corpus:** the output of the export: `bootstrap-compare.jsonl` and `holdout-compare.jsonl`, with one trace on each line.
- **Bootstrap and holdout:** the two partitions of the sample. The run compares the traces of the two partitions. The bootstrap traces also give the floors.
- **Floors:** the three threshold values of the gate (perplexity, tail fraction, and novelty). The calibration derives them from the bootstrap traces, and the two sides get the same values.
- **Check result:** the file that a passing run writes for `pipeline.py` (`<check id>.result.json`, with its evidence file). It holds the code revision hash and the digests of the run.

## How the baseline is configured and identified

On a deployed server, environment variables configure the old path, and no record says which values produced a decision. The versioned bundle exists to end that. The comparison does not use environment variables for the baseline:

- **The harness builds the baseline state in code** (Task 5 Step 2): the base test state plus the values that the step lists. It does not start the server through the production start function, which reads the configuration from the environment.
- **A variable in the operator's shell cannot reach the harness.** `pipeline.py` gives a child process only the ambient variables of `CHILD_ENV_ALLOWLIST` (`pipeline_tooling/environment.py`; no `TRACE_COMMONS_*` variable is in it) and the harness variables of the Interfaces section. Task 5 Step 2 also searches the handlers that the harness calls for a read of an environment variable at request time.
- **One value gives the gate configuration of the two sides.** `compare_main_gate(floors)` gives one `MainGateConfig`: the gate values of `CompatibilityBundleConfig::local_reference()`, with the floors that the calibration derives from the sample. These values equal a deployed server's defaults for the chunk target, the chunk maximum, the chunk cap, the chunk minimum, the insert threshold, and `top_k` (5, PC-D23; the pre-PC-D23 text had 8). The baseline orchestrator is built from that value (`baseline_orchestrator_config`, Task 4 Step 1 test 7). The candidate bundle is built from the same value, and ingest refuses to start a compatibility bundle that does not hold it (`matches_main_gate`, PC-D8).

What identifies the baseline of one run:

1. **The code:** `code_revision_hash` in the check result. It is a hash of the files of the worktree, and `pipeline.py` fails a run if a file changed during the run (plan review G13). Task 11 commits the check result beside the report (question 11).
2. **The gate configuration:** `configuration_digest`, `bundle_id`, and `floors` in the report. Because one value gives the two sides, the candidate's digest also covers the baseline's gate values.
3. **The fixed state values outside the gate configuration:** for example `accept_medium_risk_submissions`, the credit delta, no score driver, PC-D16, and PC-D18. They are constants in the harness code, so the code revision covers them. The runbook section "Limits" lists them.

What proves that the tool finds a wrong baseline: scenario 3 changes one baseline floor and requires a failure (PC-D8).

What this does not give: the baseline is `main`'s old path with the gate values of `CompatibilityBundleConfig::local_reference()` and the derived floors. The chunk values, the insert threshold, and `top_k` equal a deployed server's defaults (PC-D23). It is not the configuration of a deployed server. A deployment that sets other values has a different old path, and this report says nothing about it.

## State on 2026-10-08 (verified with read-only commands)

- **Machine (owner, 2026-10-08): `hetzner`.** The branch was pushed from the laptop to the server's repository, and the worktree is `/home/brapse/workspace/misc/trace-commons-server/.claude/worktrees/pipeline-comparison`. The laptop session stops work on this branch (common.md, machine rule 5). Handoff: `.superpowers/workstreams/handoff-pipeline-comparison-2026-10-08.md`. The lines below give the state on the laptop before the move.
- PR 4 (#1166) and PR 5 (#1240) are merged. `upstream/main` = `0e5dffe6b`. PR 4 is `67403a212` and PR 5 is `4e484b567` on `main`.
- Branch `vp/pipeline-comparison` = `9f590a637` (the spec only) on `cbe165ff`, the last head of PR 4 before its squash merge. The branch is 72 commits behind `upstream/main`. Nothing is pushed. The worktree has no `target/`.
- The workstream has no PostgreSQL container and no ledger. No container uses port 55434 (`tc-pipeline-pg` has 55432, `tc-pipeline-pg-pr3` has 55433).
- The plan was written against the PR 4 tree. On 2026-10-08 its names were checked against `0e5dffe6b`: each existing function, type, constant, and CI step that the plan names exists there, and no file that the plan creates exists there. The check compared names and signatures only. It did not compile the plan's code. Task 5 Step 1 (the spike) is the first proof.
- PR 5 changed three facts that the plan uses. The tasks below have the new values:
  1. **Routing.** The rollout gate `PipelineReceipts` is no longer sufficient to send a tenant's receipts to the pipeline. The test state must also hold a routing store (`pipeline_activation = routing_store(&runtime)`), and `assemble_ingest_pipeline_runtime` has a new argument, `unqualified_routing_allowed` (Task 5, PC-D15).
  2. **Positions.** The parity test is at about line 7251 of `pipeline_http_pg_tests.rs`, and `review_quarantine` is at about line 829 of `pipeline_corpus_pg_tests.rs`.
  3. **Test count.** `test_pipeline_tooling.py` has 100 tests, not 48 (Task 7).

## Preconditions (not tasks of this plan)

1. The owner approves this plan and answers the questions at its end. Done on 2026-10-08.
2. Phase B, in this order:
   - `git fetch upstream && git merge upstream/main` in the worktree (one merge commit; common.md flow rule 3: the branch is on PR 4's last head, and PR 4 is squash-merged, so this branch merges `upstream/main` one time). This merge also brings PR 5 and the later commits of `main`. Write the hash of the merge commit in the ledger with the name `BASE_CMP`. `BASE_CMP` is the start point of this plan's work: the plan could not give the hash before the merge, `git diff BASE_CMP..HEAD` shows only the work of this plan, and the later merges of `main` are recorded apart from it.
     - The branch has no code of its own, but the merge stops with conflicts: 29 files at `0e5dffe6b` (11 `add/add`, 18 `content`), because the branch has PR 4's commits and `main` has PR 4's squash. Take `upstream/main` for each conflicted path as a whole file (`git checkout upstream/main -- <path>`). Do not use `-X theirs`: it keeps branch-side hunks that do not conflict.
     - After the merge commit, `git diff --stat upstream/main HEAD` must list only the spec and this plan (the plan was committed before the move to `hetzner`). If it lists another path, take that path from `upstream/main` and amend the merge commit before task 1. This check is the proof that the base is `upstream/main` plus the spec.
     - If `git fetch` moved `upstream/main` past `0e5dffe6b`, do the name check of the state section again for the new commits (`git diff --stat 0e5dffe6b upstream/main` on the files of the file map) before task 1.
   - Create the container on `hetzner`: `docker run -d --name tc-pipeline-pg-cmp -e POSTGRES_USER=trace -e POSTGRES_HOST_AUTH_METHOD=trust -p 127.0.0.1:55434:5432 postgres@sha256:a3b7f434b2dc57ce85a67e171163eb8ab1a1ebcb39d27484661f26b1dfbe30d6`. This is the PostgreSQL 16 image that `pipeline.py` pins (`POSTGRES_IMAGE`); the server has it already, and it has no `postgres:16` tag. Port 55434 was free on the server on 2026-10-08.
   - Complete the ledger directory `.superpowers/sdd/2026-10-02-pipeline-comparison-plan/` in the main checkout on `hetzner` (it has the plan review files already; add `progress.md`, `replies.md`, `plan-path`) and add it to the entry `pipeline-comparison` in `.superpowers/workstreams/workstreams.json`.
   - The plan is committed already (the last commit on the laptop, before the move). Do not commit it a second time. A later change of the plan is its own commit.
3. The worktree on `hetzner` has no `target/`, so the first build is cold. Measured on the server on 2026-10-03: about 5.5 minutes for check, clippy, and `test --no-run`. No clone of a warm directory is necessary. The server's `.cargo/config.toml` sets `jobs = 10` and the same `rustflags` as the laptop's.

## If the work stops

- Remove the container: `docker rm -f tc-pipeline-pg-cmp`.
- Remove `.local/pipeline/` in the worktree (it holds trace text of the public dataset and encrypted artifacts).
- Update the entry `pipeline-comparison` in `.superpowers/workstreams/workstreams.json`.
- The plan file and the spec exist only on this branch. Do not remove the worktree before the branch is pushed or the two files are copied.

## Global Constraints

- Base: `BASE_CMP` (precondition 2). After the base merge, `upstream/main` is the parent. Take later `main` changes by a merge, between two tasks, never in the middle of a task. Never rebase. Record each merge in the ledger.
- The branch is on the owner's two machines only (the laptop and `hetzner`), and on no published remote. Nothing is pushed to GitHub until the owner decides to publish it. Each outward action (a push, a GitHub comment, a PR description) needs a draft in the ledger's `replies.md` and the owner's approval (common.md).
- Changes flow down only (common.md flow rule 1). A defect that the comparison finds is not fixed on this branch: a defect in pipeline code on `main` goes to the PR 3 follow-up PR or to a new issue. Write it in the ledger and tell the owner. A new permitted-difference rule that changes text in `docs/superpowers/specs/2026-09-14-versioned-pipeline-compatibility-mapping.md` goes to `main` as its own small upstream PR.
- Do not change the harness to make a difference go away. The tool's job is to report it. The two admission rows of spec section 8.3 were expected candidates for findings. Row 3 has the ruling PC-D22 (2026-10-09), and the rule `medium_risk_privacy_review` permits it. Row 4 has the ruling PC-D27 (2026-10-11), and the rule `high_risk_admission_reject` permits it. The `credit_quality_micros` clamp is not: `main` clamps the value to `[0, 1]` before it stores it (`credit_quality.rs`), so the candidate's `max(0)` changes nothing. One change of the baseline's stored state is permitted: PC-D18 removes the baseline tenant's derived files after each trace (owner, 2026-10-08). It removes a cost, not a difference, and Task 5 Step 4 test 15 proves that no record changes.
- Every new `.rs` file in `trace-commons-server` starts with `// Copyright (C) 2026 K&Z Partners LLC` and `// SPDX-License-Identifier: AGPL-3.0-or-later`.
- No new third-party dependency. Rust uses only existing dependencies (the server crate has no `futures` dependency: use `tokio`). Python uses only the standard library. Do not edit the expected sets in `tests/license_boundary.rs`.
- No migration. No new table. No production route. No change to a type or trait in `crates/trace-commons-gate-api`.
- Operational output is hash-only or label-only. New labels, including every check id, rule id, and failure label, match `^[a-z0-9_]{1,64}$`. No raw trace text, UUID, tenant id, token, or dataset file name in a record, a report, a result, an evidence file, or a log line that the tooling prints. `trace_hash` and `position` identify the dataset file to a person who has the pin and the public dataset. This is intended, so that a person can reproduce a difference (owner, question 12). Do not use the comparison on a corpus that is not public.
- Every hashed JSON value goes through `trace_commons_protocol::canonical_json::to_canonical_vec` (the server's build has `serde_json/preserve_order` on). Python's canonical form is `pipeline_tooling.results.canonical`.
- The side names are `baseline` and `candidate` in every record, report field, type, and variable. Code that applies only to the old path carries the comment marker `// baseline-old-path:` on the item, so a later change can find and remove it (spec section 18).
- The v1 corpus output of the export binary must stay byte-for-byte as it is; the existing digest tests prove it. The reports of `pipeline.py run` and `pipeline.py qualify` must stay equal apart from the code revision hash and the times. The new library module runs inside `qualify` (its step `cargo test --lib versioned_pipeline` matches the module name), so `qualify` gets slower by the new unit tests and gets no new check.
- Database URLs: tests read only `TRACE_COMMONS_PG_TEST_DATABASE_URL`. Never `DATABASE_URL`. When the variable is set, a setup failure panics; it never skips.
- Commits: short imperative subject, no `feat:` or `fix:` prefix, no emoji.
- Branch `vp/pipeline-comparison`; worktree `/home/brapse/workspace/misc/trace-commons-server/.claude/worktrees/pipeline-comparison` on `hetzner`; test PostgreSQL `tc-pipeline-pg-cmp` at 127.0.0.1:55434 on that machine (user `trace`, trust auth). Work only in that worktree. Never use `git stash`.
- Build environment (common.md): do not set `CARGO_TARGET_DIR`, do not set `RUSTFLAGS`, do not use `cargo +<toolchain>`. Keep `-p trace-commons-server` in the edit loop. Serialize cargo commands inside the worktree; run long commands in the background.
- Local database runs: a fresh database for each suite run, with `cmp` in its name (`dropdb -h 127.0.0.1 -p 55434 -U trace --if-exists <name>`, then `createdb -h 127.0.0.1 -p 55434 -U trace <name>`). Never use the server on port 5432.
- Checks, in the three levels of common.md (owner, 2026-10-03). Run them from the worktree root.
  - **In a task and in a fix round:**

    ```bash
    cargo fmt --all -- --check
    cargo check -p trace-commons-server --all-targets
    ```

    and the focused tests that the task names. `--all-targets` is necessary: almost all Rust of Tasks 4 to 6 is test code of the ingest binary, and `--bins` does not compile it. A task that touches Python also runs `python3 -m py_compile scripts/operator/pipeline.py scripts/operator/pipeline_tooling/*.py` and `python3 scripts/operator/test_pipeline_tooling.py`.
  - **Once for each package of tasks** (after Task 3 for Tasks 1 to 3, after Task 7 for Tasks 4 to 7) **and before the final review:**

    ```bash
    cargo clippy -p trace-commons-server --all-targets -- -D warnings -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
    cargo test -p trace-commons-server --test license_boundary
    ```

  - **At the gate only (Task 10):** `cargo test -p trace-commons-server --no-run`, the suites of Task 10 Step 2, and `pipeline.py qualify`. The upstream CI runs the rest.

- Every task review follows common.md's review rules: the consumer sweep, sibling sites, the efficiency and runtime lens, and triage (a declined or deferred item that touches money, privacy, or `main`'s behavior goes to the owner).

## Review Focus

These five conditions are the ones most likely to give a person false confidence or a useless run. Each has a test in the task that owns the code.

1. **A comparison that passes because nothing was compared.** Two records that both have no score compare equal. A run in which no trace is scored, or in which a gate gives only one result, must not pass as activation evidence. A side that never reaches a terminal state must be a difference, not an equal pair of empty records. Tests: `two_unscored_records_are_equal` and `a_record_that_is_not_terminal_is_unexplained` (Task 1), `the_branch_check_reports_each_gap` (Task 2), and scenario 3 (the baseline floor skew) of Task 6 Step 4 and of `--self-test` (Task 7).
2. **One admission difference that hides behind a chain of novelty differences.** After one side indexes a trace that the other side does not, all later novelty values differ. The alignment must keep the two indexes equal, and the report must name the first cause. Tests: `the_alignment_table_covers_every_pair` (Task 2), and scenario 2 (the risk pin reports only `admission`) of Task 6 Step 4 and of `--self-test` (Task 7).
3. **Trace text or an identifier in a record, a report, or a printed line.** The record type permits only numbers, enumerations, hashes, and label columns, and that type is the primary guard. Tests: `a_record_holds_only_labels_numbers_and_hashes` (Task 1), `test_comparison_report_refuses_private_content` (Task 7), `an_unsafe_record_value_is_refused` (Task 6), and `no_probe_reaches_a_body_or_a_record` with its content probe (Task 5). The probe test alone is not sufficient: the `secret_probe` of an exported fixture is a label beside the text, not text inside it.
4. **A run that cannot complete at 10,000 traces.** The corpus is about 380 MB. A reader that loads the file, or a harness that keeps each HTTP body, uses memory in proportion to the sample. Tests: `the_corpus_reader_is_lazy` (Task 4), `each_session_is_written_before_the_next_is_read` (Task 3), and the measurement of Task 9.
5. **Two runs of one pin that give two different reports.** A random identifier in an envelope, a timestamp in the report, or a map with no fixed order changes the digest, and then a difference cannot be reproduced. Tests: `the_same_fixture_gives_the_same_envelope_bytes` (Task 4), `the_report_digest_is_stable` (Task 2), and scenario 4 of Task 6 Step 4 and of `--self-test` (Task 7).

## Decisions made while writing this plan

| Id | Decision | Reason |
|---|---|---|
| PC-D1 | The pure logic is a new library module, `crates/trace-commons-server/src/versioned_pipeline_comparison.rs`. | Its unit tests then run in the default test run (spec section 15), and the later two-bundle comparison can use it again. |
| PC-D2 | The calibration runs inside the harness process, before the app starts. | It needs the envelope builder and the old orchestrator, which the harness already has. One process, no extra file. |
| PC-D3 | With `--with-events`, the export writes a JSONL corpus (`trace_commons.pipeline_compare_corpus.v1`): one fixture on each line, with the recorded trace as `trace_file` and no `input` field. | The full sample is about 380 MB. A JSONL file can be written and read one trace at a time. A `TraceFile` is a serde type of the protocol crate, so the harness needs no translator code. |
| PC-D4 | `steps_for` moves from `pilot_bootstrap/submitter.rs` to `pilot_bootstrap/translators.rs` as `trace_steps_for`, with a new `trace_file_for`. `submitter.rs` calls it. | Four build targets include `translators.rs` with `#[path]`: the pilot-bootstrap binary, the export binary, and the tests `pilot_bootstrap_translators` and `pilot_bootstrap_submitter`. The export binary does not include `submitter.rs`. One implementation makes the harness envelope equal to the pilot-bootstrap envelope (D8). |
| PC-D5 | The baseline index is `MockVectorIndex` of the gate-enclave crate, in test code only. | It is the only in-memory `VectorIndex`. It is a real full scan with the same dot product as `IsolatedPipelineIndex`. The report keeps the blocker `synthetic_index`. The owner accepted this decision on 2026-10-08. |
| PC-D6 | The candidate privacy boundary is `DeterministicPipelinePrivacyBoundary`, not the pass-through test double. | It is the deterministic stage of the production boundary. A second rescrub that changes the Score input is a difference that the tool must find. |
| PC-D7 | The baseline orchestrator sets `qualifying_chunk_floor_micros` to the perplexity floor. | This is `main`'s default (`build_trace_gate_service_from_env`: `unwrap_or(perplexity_floor_micros)`), and the candidate passes the same value. It answers spec section 17, point 5. The harness does not call `build_trace_gate_service_from_env` and reads no environment variable for the baseline: see "How the baseline is configured and identified". |
| PC-D8 | The negative test changes the baseline floor, not the candidate floor. | Ingest refuses a runtime whose compatibility bundle does not hold `main`'s gate configuration (`matches_main_gate`), so a candidate with a different floor cannot start. Spec section 15 names the candidate. The owner accepted this decision on 2026-10-08. |
| PC-D9 | The traces with a declared medium or high risk are in a second local pin. `pipeline.py compare --self-test` is the CI step. It runs four scenarios (Task 7). | The two sides give different admission decisions for a trace that declares a medium or a high risk (spec section 8.3). Spec decision D10 said that no rule permits a difference. Since PC-D22 (2026-10-09) the declared medium trace is a permitted pair, and since PC-D27 (2026-10-11) the declared high trace is a permitted pair too, so the risk pin passes. The risk pin has two such traces and must pass with two permitted pairs (the medium trace by `medium_risk_privacy_review` and the high trace by `high_risk_admission_reject`) and no unexplained difference. The negative proof that the tool finds a real difference is scenario 3 (the skew). `--self-test` runs the pins and passes only when each run gives its expected result. The owner accepted this decision on 2026-10-08. |
| PC-D10 | Receipts, reviews, and the baseline gate call go through HTTP. The harness reads results from the database and from `PipelineService` (`get_run`, `load_index_command`). | The forensic route gives hashes only. The current parity test reads the Score evidence the same way (`score_shadow_credit_decision`). |
| PC-D11 | Check ids: `pipeline_comparison_local` (a pin with `local_jsonl_dir`) and `pipeline_comparison_hf`. The harness emits a check result only for a full run that passes. | A partial run and a failed run are not evidence. |
| PC-D12 | `ComparisonRule` holds the three exclusions of spec section 10.2. No `ComparisonRule` permits a difference in a compared field; PC-D22 adds `PermittedDifference` for that. The comparison function takes the permit function as a parameter. | A pair has one of three results: equal, permitted, or unexplained. Spec decision D10 said that no rule permits a difference, so production code did not give `permitted` until PC-D22. The code for `permitted` has a test all the same (Task 9 Step 3). `compare_records_with` thus takes the rule as a function argument: a unit test gives it a test rule, and `compare_records` gives it the function of `PermittedDifference` (PC-D22; before PC-D22 it permitted nothing). No rule that only a test needs is in production code. |
| PC-D13 | The export downloads in windows of 4 files (16 until 2026-10-11; see issue #1308) with `tokio::task::JoinSet` and handles each window in name order. | Bounded concurrency without a new dependency. The selection order does not change, so the digests do not change. |
| PC-D14 | The harness polls a candidate run's state every 25 ms. | The corpus harness uses 100 ms for five fixtures. The pipeline worker starts a pass each 200 ms (`pipeline_runtime.rs`), so the worker interval, not the poll, is the lower bound of the candidate wait. The 25 ms poll adds little to it. |
| PC-D15 | The candidate tenant gets the pipeline through unqualified test routing (`unqualified_routing_allowed: true` and a routing store with no routing row), as the parity test and `pipeline_corpus_run` do on `main`. The harness does not activate the tenant. | PR 5 added the routing decision. An activation needs a qualification of the bundle on the deployed code revision (`docs/operator/pipeline-activation.md`), and the comparison bundle changes with the derived floors of each pin. The harness runtime has the reference scorer and embedder, so it is not production-qualified, and `main` permits unqualified routing only for such a runtime. The owner accepted this decision on 2026-10-08. |
| PC-D16 | The harness removes the submit rate limit for its own principals: `configure_unbounded_submit_limits_for_test(&tokens)` with the harness token map. | `main` permits 30 receipts in 60 s for each principal, before the routing decision, so the limit applies to the two sides (plan review G1). The call is the test hook that the base test state uses for its own tokens. The local pins have 10 and 8 traces and do not reach the limit; the 100-trace run does. Recorded for the owner, not a question. |
| PC-D17 | Each test of Task 5 uses tenant names and tokens with a random suffix. Only `pipeline_compare_run` uses the constant names. | All PostgreSQL tests of one process share one `<database>_pilot` database, and a tenant keeps its first active bundle (`activate_bundle_if_none` is `ON CONFLICT DO NOTHING`). A second test with other floors on the same tenant gives a false difference (plan review G5). The parity test does the same. |
| PC-D18 | After the two finish steps of each trace, the harness removes each `.json` file from the baseline tenant's `derived/` directory (`clear_baseline_derived`). It removes no database row and no other file. The report has the blocker `baseline_derived_scan_removed`. | The old path's receipt reads and compares every earlier derived record of the tenant (plan review G10): about 5 x 10^7 comparisons for 10,000 traces, estimated at 5 to 19 hours in an optimized build and not measured. The result of that scan (`embedding_analysis`, `submission_score`, the derived record) is not a compared field. It is not gate input for an envelope with one or more events: the chunker scores the whole stored envelope, which holds `embedding_analysis`, only when the envelope has no event, and no pin selects such a trace (each pin has a `min_words` of 1 or more). With the removal, each baseline receipt sees an empty directory, as each candidate receipt does already, so the two sides get equal scan values (`novelty_score` 0.65, `duplicate_score` 0, no neighbor). What changes: the baseline's stored envelope metadata and its `trace_derived_records` rows hold those values and not the values that `main` stores for a tenant with earlier traces. The owner selected this on 2026-10-08 (question 9); that answer is the written ruling. The choice not taken, K tenant pairs, changes the size of the two indexes, and their decisions are compared. Test: Task 5 Step 4 test 15. |
| PC-D19 | A side whose admission is `Refused` or `Other` has `terminal: false`, and the harness makes no poll and no wait for it. A pair of two refused records is `unexplained` (`terminal`). A run with one or more refused receipts fails with `comparison_receipt_refused` and gets no check result. | Spec section 8.2 gives a rule only for a refusal on one side. All checks before the routing decision are the same code for the two tenants, so an equal refusal on the two sides is the usual form of a refusal, and nothing content-dependent was compared for such a trace (plan review G2). The owner agreed on 2026-10-08 (question 6). |
| PC-D20 | The run stops at the first pair after which the two indexes can differ (`alignment_lost`): the action is `Stop` and the candidate admitted the trace; or the action is not `Stop` and a side is not terminal; or the two sides are terminal and `member` or `member_chunks` differ. The report has the field `alignment_lost_position`, and the run fails with `comparison_alignment_lost`. | After such a pair, `index_cardinality` and the novelty values of each later trace can differ, so later results are not independent evidence (Review Focus 2). A permanent cause also makes the run wait 60 s for each remaining trace (plan review G6). The third condition is the owner's rule "stop when the indexes deviate": a real membership difference also makes the two indexes different. A difference that leaves the indexes equal (for example in `perplexity_micros` only) does not stop the run. The owner agreed on 2026-10-08 (question 7). |
| PC-D21 | `compare --corpus` always uses Cargo's optimized build (`--release`) for the harness and the export. `compare --self-test` uses the debug build. There is no option. | The optimized build is always faster to run and slower to build. A run with hundreds or thousands of traces gains hours. The CI self-test uses the two local pins (10 and 8 traces), the job has the debug build already for `qualify`, and a second build there adds build time and cached files to a cache that is near GitHub's limit (plan review G11). The owner selected this on 2026-10-08 (question 10). |
| PC-D22 | `PermittedDifference::MediumRiskPrivacyReview` (id `medium_risk_privacy_review`, source `ruling.PC-D22`, field `admission`) permits spec section 8.3 row 3. It permits a pair only when all of these hold: the field is `admission`; `privacy_risk` is `medium` on the two records; `privacy_basis` is equal on the two records and is not exactly `["consent_content_flag"]`; the baseline admission is `admit` and the candidate admission is `quarantine`. A pair of any other kind stays `unexplained`; row 4 (baseline quarantine, candidate reject) has its own rule since PC-D27. The report adds `permitted_rules`. The basis clause is "not exactly", so an empty basis also meets it (correction of 2026-10-10): a declared risk has no basis label, and the declared medium trace of the risk pin, the permitted pair of `--self-test`, meets the rule through its empty basis (`a_declared_medium_trace_is_aligned` asserts the empty basis). The rule stays as it is. | The old path accepts a medium-risk trace (`TRACE_COMMONS_ACCEPT_MEDIUM_RISK_SUBMISSIONS=true`), and the pipeline quarantines it for review. The owner ruled on 2026-10-09 that this is intended. The condition is exact so that a nearby difference (the reverse direction, another risk, another basis) is still reported. |
| PC-D23 | `top_k` is 5 on the two sides. The harness takes it from `TRACE_COMMONS_GATE_DEFAULT_TOP_K` of `main`. | `CompatibilityBundleConfig::local_reference()` has `top_k` 8, and a deployed server's default, and the pilot template, have 5. The comparison of the default configuration is the useful one. The owner decided this on 2026-10-09 before the full run. |
| PC-D24 | The report has the blocker `duplicate_short_circuits_not_compared`, and the run does not compare the duplicate short-circuits (`skipped_duplicate`, `cached`). The candidate runtime keeps `duplicate_controls: None`, and the baseline keeps the gate route. | Since #1325 the two production paths apply the short-circuits: the old path in the score driver, the pipeline at Settle. The run applies them on neither side (spec section 17 point 2), so a passing report says nothing about them. The two sides also read `credit_quality_micros` from different places (the candidate from the Score evidence, the baseline from the gate decision row), which differ for a duplicate since #1325. The owner chose on 2026-10-10 to name the gap in each report and to compare the path in a follow-up, not in this PR. The Python validator refuses a report without the blocker (`missing_local_blockers`). |
| PC-D25 | The report has the blocker `review_start_privacy_pass_not_compared`. The harness reads the candidate's privacy fields and admission at the receipt, before the Review-start privacy pass of #1324. | With the deterministic boundary of this tool the pass is `cleared` for each trace and changes no compared field. With a real classifier, an escalated pass parks the candidate run for a human, and the run stops with `comparison_alignment_lost`, which fails closed but does not name the pass (spec section 8.3). The owner asked on 2026-10-10 that the report say so. |
| PC-D27 | `PermittedDifference::HighRiskAdmissionReject` (id `high_risk_admission_reject`, source `ruling.PC-D27`, field `admission`) permits spec section 8.3 row 4. It permits a pair only when all of these hold: the field is `admission`; `privacy_risk` is `high` on the two records; `privacy_basis` is equal on the two records (its content does not matter); the baseline admission is `quarantine` and the candidate admission is `reject`. `PermittedDifference::ALL` lists `MediumRiskPrivacyReview` and then `HighRiskAdmissionReject`, and `permitted_rules` holds the two rules in this order. A pair of any other kind stays `unexplained`. The self-test's risk pin now passes: scenario 2 expects `permitted_counts == {"medium_risk_privacy_review": 1, "high_risk_admission_reject": 1}`, `permitted_total == 2`, and no unexplained difference. The label `compare_self_test_risk_passed` is removed. With this rule, a trace whose risk is High at the receipt gets no reviewer in the pipeline, while on `main` a reviewer can approve it. | `main`'s `status_for_risk` quarantines a High trace and a reviewer decides. The pipeline's admission policy rejects it (`privacy_risk_rejected`), and Review does not run. The owner ruled on 2026-10-11 that the reject is intended. The source of the intent is the design `docs/superpowers/specs/2026-10-09-pipeline-async-privacy-rescrub-design.md`, which says twice: "A receipt-time High is still rejected by Admission." The full run of 2026-10-09 (10,000 traces) had one such pair, at position 8298, and it was the only unexplained pair. |
| PC-D26 | The Python validator refuses (`comparison_count_mismatch`) a report in which a side's admission counts (`admit`, `quarantine`, `reject`, `refused`, `other`) do not add up to `compared_count`, a pair of gate counts (`quality_passed` and `quality_failed`, `novelty_passed` and `novelty_failed`, `member` and `not_member`) does not add up to `scored`, or `chunks_capped` is above `scored`. It refuses (`comparison_report_malformed`) a report whose `excluded_rules` is not exactly the three rules of `ComparisonRule::ALL`, in order. A test compares `comparison.BLOCKERS` with `COMPARISON_BLOCKERS` in the Rust source. | `SideDistribution::observe` counts each compared pair once on each side, so a report whose counts disagree was not written by the harness. `scored` is not bounded by `admit`: a quarantined trace that the review approved is scored. The review of 2026-10-10 found these checks absent, and the list of excluded rules unchecked beyond its shape. |

## Items for other PRs (write down, tell the owner)

- Any difference that the 100-trace run or the full run reports, with its count and one trace hash.
- If Task 6 step 1 shows that ingest refuses a production-compatible bundle with the reference scorer, that is an owner question, not a harness change.
- The old path's receipt cost grows with the number of file-side records of the tenant (plan review G10): each receipt reads and compares every earlier derived record (`read_all_derived_records`, `build_derived_precheck`), each audit append reads the whole `events.jsonl` three times, each gate call of a deployment reads all dedup signal rows, and each credit event reads the whole credit event file. This is `main`'s behavior and not a defect of this branch. PC-D18 removes the first of these four terms from the run. The read of the dedup signal rows is in a deployment and not in a comparison run: the harness database has no gate-driver pool. The other two stay. Tell the owner the measured values of Task 9.
- **Finding (owner, 2026-10-08): the old path has this cost in production also, not only in the harness.** On the old path the files are the primary store and PostgreSQL is a mirror of them. The receipt handler calls `read_all_derived_records(&state.root, ...)` with no condition, and it calls `write_derived_record` in each branch of the mirror condition (`state.require_db_mirror_writes || state.account_admission.is_some()`). No flag selects a database read for the receipt: the store has `list_trace_derived_records`, but the receipt handler does not call it. Thus a production tenant on the old path pays one directory read, one parse, and one comparison for each earlier trace of the tenant on each receipt, and the total cost for N traces grows with N squared. A database read of all earlier rows has the same growth with a smaller constant. The handler does this read before `route_pipeline_receipt`, so a pipeline tenant makes the same call, but the pipeline path writes no file to `derived/`, and its directory stays empty. The harness does not change the old path's code. It removes the scan's input for the baseline tenant after each trace (PC-D18), so the run does not pay this cost and does not measure it. The committed report states the finding with the plan review's estimate, marked as not measured (Task 9 Step 3, Task 11 Step 3).
- **Owner ruling (2026-10-08): this receipt code is "ridiculous" and must change.** A receipt must not read and parse one file for each earlier trace of the tenant. This is a defect of the old path, not a cost to accept in production. The correction is not in this PR: this PR changes no code of the old path, and its harness only removes the scan's input for the baseline tenant (PC-D18). The correction needs its own issue and its own PR. When all tenants are on the pipeline, the correction can be the removal of the old receipt code, because the pipeline path writes no derived file and takes its novelty decision from the gate. Until that PR merges, the finding stays in each comparison report.

## File map

| File | Change |
|---|---|
| `crates/trace-commons-server/src/versioned_pipeline_comparison.rs` | New. Records, comparison, alignment, floors, summary, report. |
| `crates/trace-commons-server/src/lib.rs` | Add `pub mod versioned_pipeline_comparison;`. |
| `crates/trace-commons-server/src/bin/pilot_bootstrap/translators.rs` | Add `trace_steps_for`, `trace_file_for` (moved from `submitter.rs`). |
| `crates/trace-commons-server/src/bin/pilot_bootstrap/submitter.rs` | Call `trace_file_for`. Remove `steps_for`. |
| `crates/trace-commons-server/src/bin/trace-commons-pipeline-corpus-export.rs` | `--with-events`, `--declared-privacy-risk`, streamed JSONL output, windowed download. |
| `crates/trace-commons-server/tests/fixtures/pipeline-compare-jsonl/` | New. 12 synthetic sessions, `pin-local.json`, `pin-local-risk.json`. |
| `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_compare_pg_tests.rs` | New. Envelope builder, corpus reader, calibration, side drivers, `pipeline_compare_run`. |
| `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs` | Declare the module. Add `fixture_gate_worker_artifact_store_with_key`. |
| `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_http_pg_tests.rs` | Make one helper (`wait_for_run_state`) and one constant `pub(super)`. |
| `scripts/operator/pipeline_tooling/comparison.py` | New. Export call, report validation, Markdown. |
| `scripts/operator/pipeline.py` | The `compare` subcommand. |
| `scripts/operator/test_pipeline_tooling.py` | Self-tests for the command and the validator. |
| `.github/workflows/ci.yml` | One step in `pipeline qualification and restore`. |
| `docs/operator/pipeline-comparison.md`, `docs/operator/README.md` | New runbook and its two index lines. |
| `docs/operator/pipeline-lab.md`, `docs/operator/pipeline-qualification.md`, `CLAUDE.md` | One sentence each: the network pin, and the new step of the CI job `pipeline qualification and restore`. |
| `docs/superpowers/specs/fixtures/versioned-pipeline-comparison-hf-pin-v1.json` | New. The network pin (Task 8). |
| `docs/superpowers/reports/` | The report of the full run, with its check result and its evidence file (Task 11). |

## Interfaces (the names every task uses)

### `versioned_pipeline_comparison.rs`

```rust
pub const COMPARISON_REPORT_SCHEMA: &str = "trace_commons.pipeline_comparison_report.v1";
pub const COMPARE_CORPUS_SCHEMA: &str = "trace_commons.pipeline_compare_corpus.v1";
pub const UNEXPLAINED_LIST_LIMIT: usize = 1_000;
pub const COMPARISON_BLOCKERS: [&str; 10] = [
    "local_test_only", "local_reference_scorer", "local_reference_embedder",
    "synthetic_index", "synthetic_settlement", "static_bearer_authentication",
    "deterministic_privacy_only",
    "baseline_derived_scan_removed",  // PC-D18
    "duplicate_short_circuits_not_compared",  // PC-D24
    "review_start_privacy_pass_not_compared",  // PC-D25
];
pub const COMPARISON_UNEXPLAINED_LABEL: &str = "comparison_has_unexplained_differences";
pub const COMPARISON_BRANCH_LABEL: &str = "comparison_gate_branch_not_exercised";
pub const COMPARISON_REFUSED_LABEL: &str = "comparison_receipt_refused";      // PC-D19
pub const COMPARISON_ALIGNMENT_LABEL: &str = "comparison_alignment_lost";     // PC-D20

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonSide { Baseline, Candidate }

/// `Refused`: the receipt was not accepted. `Other`: a state that is none of
/// the three decisions (for example `awaiting_pii_backstop`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionLabel { Admit, Quarantine, Reject, Refused, Other }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewLabel { None, Approve, Reject }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewSource { None, HashRule, Alignment }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GateValues {
    pub quality_passed: bool,
    pub novelty_passed: bool,
    pub perplexity_micros: u64,
    pub tail_fraction_micros: u64,
    pub peak_perplexity_micros: u64,
    pub novelty_score_micros: u64,
    pub peak_novelty_micros: u64,
    pub chunk_count: u32,
    pub total_chunk_count: u32,
    pub chunks_capped: bool,
    pub index_cardinality: Option<u64>,
    pub credit_quality_micros: Option<i64>,
    pub credit_quality_version: Option<i64>,  // the column is INT4 and the evidence is i32: read as i32, then widen
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CreditEvent { pub event_type: String, pub microcredits: u64 }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ComparisonRecord {
    pub position: u64,
    pub partition: String,            // "bootstrap" or "holdout"
    pub trace_hash: String,           // sha256: of the trace id text
    pub side: ComparisonSide,
    pub receipt_code: u16,
    pub terminal: bool,               // the side reached a terminal state in time
    pub privacy_risk: Option<String>, // "low" | "medium" | "high"
    pub privacy_basis: Vec<String>,   // sorted labels
    pub admission: AdmissionLabel,
    pub review: ReviewLabel,
    pub review_source: ReviewSource,
    pub gate_skipped_by_alignment: bool,
    pub scored: bool,
    pub gate: Option<GateValues>,     // Some exactly when `scored`
    pub member: bool,
    pub member_chunks: Vec<u32>,      // ascending chunk numbers
    pub credit_events: Vec<CreditEvent>,
}

/// The exclusions of spec section 10.2. No variant permits a difference in a
/// compared field (PC-D12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ComparisonRule { DeterministicIndexKeys, LedgerReasonText, ShadowValuesNotInContract }
impl ComparisonRule {
    pub const ALL: [ComparisonRule; 3];
    pub fn id(self) -> &'static str;              // "deterministic_index_keys", ...
    pub fn source(self) -> &'static str;          // a label, for example "compatibility_mapping.required_behavior_change_2"
    pub fn excluded_fields(self) -> &'static [&'static str];
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceComparison {
    Equal,
    Permitted { rules: Vec<&'static str> },
    Unexplained { fields: Vec<&'static str> },
}

/// The production comparison: only a `PermittedDifference` permits a difference.
pub fn compare_records(baseline: &ComparisonRecord, candidate: &ComparisonRecord) -> TraceComparison;

/// `permit` answers the rule id that permits a difference in `field`, or `None`.
pub fn compare_records_with(
    baseline: &ComparisonRecord,
    candidate: &ComparisonRecord,
    permit: &dyn Fn(&'static str, &ComparisonRecord, &ComparisonRecord) -> Option<&'static str>,
) -> TraceComparison;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignmentAction {
    None,
    ApproveCandidate,
    SkipBaselineGate,
    ApproveBaseline,
    HashRuleBoth(ReviewLabel),
    RejectBaseline,
    /// A side is `Refused` or `Other`: no review and no gate call.
    Stop,
}
pub fn hash_rule(trace_hash: &str) -> ReviewLabel;
pub fn alignment_action(baseline: AdmissionLabel, candidate: AdmissionLabel, trace_hash: &str) -> AlignmentAction;
/// PC-D20. True when, after this pair, the two indexes can differ:
/// 1. `action` is `Stop` and `candidate.admission` is `Admit` (the worker adds
///    the trace with no call from the harness, and the baseline adds nothing);
/// 2. `action` is not `Stop` and a side has `terminal: false` (a call failed,
///    or the candidate can still complete later);
/// 3. the two sides are terminal and `member` or `member_chunks` differ.
/// False for every other pair, also for a pair that differs in other fields.
pub fn alignment_lost(action: AlignmentAction, baseline: &ComparisonRecord, candidate: &ComparisonRecord) -> bool;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct DerivedFloors {
    pub perplexity_floor_micros: u64,
    pub tail_fraction_floor_micros: u64,
    pub novelty_floor_micros: u64,
}
pub fn lower_median(values: &[u64]) -> Option<u64>;
/// `Err("comparison_calibration_empty")` for an empty input,
/// `Err("comparison_floors_all_zero")` when every floor is zero.
pub fn derive_floors(perplexity: &[u64], tail_fraction: &[u64], novelty: &[u64]) -> Result<DerivedFloors, &'static str>;

#[derive(Debug, Default)]
pub struct ComparisonSummary { /* counts only; no record is kept except the unexplained list */ }
impl ComparisonSummary {
    pub fn observe(&mut self, baseline: &ComparisonRecord, candidate: &ComparisonRecord, result: &TraceComparison);
    pub fn unexplained_total(&self) -> u64;
    /// The sum of the `refused` counts of the two sides (PC-D19).
    pub fn refused_total(&self) -> u64;
    /// The labels of the branches with no evidence: any of "quality_passed_true",
    /// "quality_passed_false", "novelty_passed_true", "novelty_passed_false",
    /// "member_true", "member_false". Read from the baseline side.
    pub fn branch_gaps(&self) -> Vec<&'static str>;
}

pub struct ComparisonReportInput<'a> {
    pub check_id: &'a str,
    /// The number of traces in the pin. The run is partial when the
    /// summary observed fewer pairs.
    pub trace_count: u64,
    pub partial: bool,
    pub pin_digests: &'a BTreeMap<String, String>, // source, order, configuration, bootstrap_corpus, holdout_corpus
    pub package: &'a BundlePackage,
    pub floors: DerivedFloors,
    pub skew: Option<&'a str>,
    /// The position of the pair at which the run stopped (PC-D20), or `None`.
    pub alignment_lost_position: Option<u64>,
    pub records_digest: &'a str,
}
pub fn comparison_report(input: &ComparisonReportInput<'_>, summary: &ComparisonSummary) -> Result<serde_json::Value, String>;
```

The compared field names, in this order (they are the strings in an `Unexplained` list and in `unexplained_counts`): `receipt_code`, `terminal`, `privacy_risk`, `privacy_basis`, `admission`, `scored`, `quality_passed`, `novelty_passed`, `perplexity_micros`, `tail_fraction_micros`, `peak_perplexity_micros`, `novelty_score_micros`, `peak_novelty_micros`, `chunk_count`, `total_chunk_count`, `chunks_capped`, `index_cardinality`, `credit_quality_micros`, `credit_quality_version`, `member`, `member_chunks`, `credit_events`.

Not compared: `position`, `partition`, `trace_hash` (the caller pairs the records and must give two records with equal values; the function answers `Unexplained { fields: ["record_pair"] }` if they differ), `side`, `review`, `review_source`, `gate_skipped_by_alignment`.

### The compare corpus (one JSON object on each line)

```json
{"label":"hf_bootstrap_0000","trace_id":"<uuid>","submission_id":"<uuid>","created_at":"2026-01-01T00:00:00+00:00","secret_probe":"qualification_probe_bootstrap_0000","privacy_risk":"low","trace_file":{"model_name":"pilot-bootstrap/<repository>","memory_snapshot":[],"http_exchanges":[],"steps":[...]}}
```

`label`, `trace_id`, `submission_id`, `created_at`, `secret_probe`, and `privacy_risk` have the values that `fixture` writes today. Files: `bootstrap-compare.jsonl`, `holdout-compare.jsonl`.

### Pin fields for a comparison

A comparison pin has the schema `trace_commons.pipeline_hf_corpus_pin.v1` and its current fields, plus:

| Field | Content |
|---|---|
| `with_events` | `true`. `pipeline.py compare` refuses a pin without it. |
| `session_names` | optional; a list of file names. Only these files are candidates. Local pins only. |
| `declared_privacy_risk` | optional; a map from a file name to `medium` or `high`. Local pins only. |

### The report (`trace_commons.pipeline_comparison_report.v1`)

```json
{
  "schema": "trace_commons.pipeline_comparison_report.v1",
  "check_id": "pipeline_comparison_local",
  "scope": "local_test", "production_ready": false, "external_payout_enabled": false,
  "partial": false,
  "skew": null,
  "safe_blockers": ["local_test_only", "...", "deterministic_privacy_only", "baseline_derived_scan_removed", "duplicate_short_circuits_not_compared", "review_start_privacy_pass_not_compared"],
  "pin": {"source_digest": "sha256:...", "order_digest": "sha256:...", "configuration_digest": "sha256:...", "bootstrap_corpus_digest": "sha256:...", "holdout_corpus_digest": "sha256:..."},
  "bundle_id": "sha256:...", "package_hash": "sha256:...", "configuration_digest": "sha256:...", "dependency_digest": "sha256:...",
  "floors": {"perplexity_floor_micros": 0, "tail_fraction_floor_micros": 0, "novelty_floor_micros": 0},
  "trace_count": 12, "compared_count": 12,
  "equal_count": 12, "permitted_counts": {}, "permitted_total": 0, "unexplained_counts": {}, "unexplained_total": 0,
  "unexplained": [{"position": 3, "trace_hash": "sha256:...", "fields": ["admission"]}],
  "first_unexplained_position": null,
  "alignment_lost_position": null,
  "excluded_rules": [{"rule": "deterministic_index_keys", "source": "compatibility_mapping.required_behavior_change_2", "fields": ["index_entry_id", "nearest_neighbor_hash"]}],
  "distribution": {"baseline": {"admit": 0, "quarantine": 0, "reject": 0, "refused": 0, "other": 0, "scored": 0, "quality_passed": 0, "quality_failed": 0, "novelty_passed": 0, "novelty_failed": 0, "member": 0, "not_member": 0, "chunks_capped": 0}, "candidate": {"...": 0}},
  "branch_gaps": [],
  "records_digest": "sha256:...",
  "report_digest": "sha256:..."
}
```

`trace_count` is the number of traces in the pin; its source is `sample_count` of the export manifest. `compared_count` is the number that the run compared (`min(limit, trace_count)`, or fewer after an early stop). `distribution.<side>.not_member` counts scored traces only, so a run with no scored trace keeps the gap `member_false`. `skew` is `null` or `"baseline_quality_floor"`; a report with a skew never gets a check result. `alignment_lost_position` is `null`, or the position of the pair at which the run stopped (PC-D20); the pair is the last compared pair, and it is in the counts. A report with an `alignment_lost_position`, or with a `refused` count that is not zero (PC-D19), never gets a check result. `equal_count + permitted_total + unexplained_total == compared_count`. `permitted_total` is the number of pairs with the result `permitted`. A pair that several rules permit adds 1 to `permitted_total` and 1 to each rule in `permitted_counts`, so the sum of `permitted_counts` is not a count of pairs. `permitted_total <= sum(permitted_counts)`, and the two are both zero or both more than zero.

### Harness variables (set by `pipeline.py compare`)

| Variable | Content |
|---|---|
| `TRACE_COMMONS_PG_TEST_DATABASE_URL` | the scenario's runtime URL |
| `TRACE_COMMONS_PIPELINE_COMPARE_BOOTSTRAP_PATH` | `bootstrap-compare.jsonl` |
| `TRACE_COMMONS_PIPELINE_COMPARE_HOLDOUT_PATH` | `holdout-compare.jsonl` |
| `TRACE_COMMONS_PIPELINE_COMPARE_MANIFEST_PATH` | the export's `source-manifest.json` |
| `TRACE_COMMONS_PIPELINE_COMPARE_CHECK_ID` | `pipeline_comparison_local` or `pipeline_comparison_hf` |
| `TRACE_COMMONS_PIPELINE_COMPARE_REPORT_PATH` | where the harness writes the report |
| `TRACE_COMMONS_PIPELINE_COMPARE_RECORDS_PATH` | where the harness writes `comparison-records.jsonl` |
| `TRACE_COMMONS_PIPELINE_COMPARE_LIMIT` | optional; a positive integer |
| `TRACE_COMMONS_PIPELINE_COMPARE_SKEW` | optional; only `baseline_quality_floor` |
| `TRACE_COMMONS_PIPELINE_ARTIFACT_ROOT`, `TRACE_COMMONS_PIPELINE_TEST_MASTER_KEY_HEX` | as for `pipeline_corpus_run` |
| `TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR`, `..._CHECK_RUN_ID`, `..._CHECK_CODE_REVISION_HASH` | as for `pipeline_corpus_run` |

---

### Task 1: Records and the comparison function

**Files:**
- Create: `crates/trace-commons-server/src/versioned_pipeline_comparison.rs`
- Modify: `crates/trace-commons-server/src/lib.rs` (add `pub mod versioned_pipeline_comparison;` beside `versioned_pipeline_compat`)

**Interfaces:**
- Consumes: nothing from this plan.
- Produces: `ComparisonSide`, `AdmissionLabel`, `ReviewLabel`, `ReviewSource`, `GateValues`, `CreditEvent`, `ComparisonRecord`, `ComparisonRule`, `TraceComparison`, `compare_records`, `compare_records_with`, and the constants of the Interfaces section.

- [ ] **Step 1: Write the failing tests** in a `#[cfg(test)] mod tests` of the new file. Use one helper, `fn record(side: ComparisonSide) -> ComparisonRecord`, that returns a scored, admitted, member record with fixed values (`position: 7`, `partition: "holdout"`, `trace_hash: "sha256:" + 64 times "a"`, `receipt_code: 200`, `terminal: true`, `privacy_risk: Some("medium")`, `privacy_basis: vec!["consent_content_flag"]`, `gate: Some(GateValues { quality_passed: true, novelty_passed: true, perplexity_micros: 31_000_000, tail_fraction_micros: 120_000, peak_perplexity_micros: 40_000_000, novelty_score_micros: 600_000, peak_novelty_micros: 900_000, chunk_count: 3, total_chunk_count: 3, chunks_capped: false, index_cardinality: Some(12), credit_quality_micros: Some(450_000), credit_quality_version: Some(2) })`, `member: true`, `member_chunks: vec![0, 1, 2]`, `credit_events: vec![CreditEvent { event_type: "novelty_utility".into(), microcredits: 2_500_000 }]`). Tests:
  1. `two_equal_records_compare_equal`: `compare_records(&record(Baseline), &record(Candidate)) == TraceComparison::Equal`.
  2. `each_compared_field_is_reported_by_name`: for each of the 22 compared field names, change exactly that field in the candidate record and assert `Unexplained { fields: vec![<name>] }`. Write this as a table of `(name, fn(&mut ComparisonRecord))`. For the fields inside `gate`, change the value inside `Some`. Assert that the table has 22 entries and that its names equal the order of the Interfaces section.
  3. `several_differences_are_reported_in_field_order`: change `admission` and `novelty_score_micros`; the list is `["admission", "novelty_score_micros"]`.
  4. `a_scored_record_against_an_unscored_record_reports_scored_only`: the candidate has `scored: false, gate: None`; the result is `Unexplained { fields: vec!["scored"] }` (the gate value fields are not listed again).
  5. `two_unscored_records_are_equal` (Review Focus 1; Task 2's `the_branch_check_reports_each_gap` is the guard for this case): both `scored: false, gate: None, member: false, member_chunks: vec![], credit_events: vec![]`; the result is `Equal`.
  6. `a_record_that_is_not_terminal_is_unexplained`: both records have `terminal: false`; the result is `Unexplained { fields: vec!["terminal"] }`. A pair in which no side finished is never `Equal`.
  7. `records_of_different_traces_are_refused`: the candidate has `position: 8`; the result is `Unexplained { fields: vec!["record_pair"] }`. The same for a different `trace_hash` and a different `partition`.
  8. `a_permit_function_gives_permitted`: `compare_records_with` with a closure that answers `Some("test_rule")` for `"admission"`; the candidate differs in `admission` only; the result is `Permitted { rules: vec!["test_rule"] }`. With differences in `admission` and `chunk_count`, the result is `Unexplained { fields: vec!["chunk_count"] }`.
  9. `rule_ids_and_sources_are_labels`: for each `ComparisonRule::ALL`, `id()` matches `^[a-z0-9_]{1,64}$`, `source()` matches `^[A-Za-z0-9_.:-]{1,128}$`, `excluded_fields()` is not empty, and no excluded field name is one of the 22 compared names.
  10. `a_record_holds_only_labels_numbers_and_hashes` (Review Focus 3): serialize `record(Baseline)` with `to_canonical_vec`, parse it as a `serde_json::Value`, and walk it: every string matches `^[A-Za-z0-9_.:-]{1,128}$`; no object key is one of `input`, `text`, `trace_text`, `secret`, `secret_probe`, `token`, `account_id`, `email`; no string has the form of a UUID. Also: `serde_json::from_str::<ComparisonRecord>` refuses an object with an unknown field.
  11. `a_one_side_refusal_reports_receipt_code_only` (spec section 8.2): the candidate has `receipt_code: 429`, `admission: Refused`, `terminal: false` (PC-D19), `scored: false`, `gate: None`; the result is `Unexplained { fields: vec!["receipt_code"] }`, with no other field.
  12. `scored_without_gate_values_is_refused`: a side with `scored: true, gate: None`, and a side with `scored: false, gate: Some(_)`, each give `Unexplained { fields: vec!["record_pair"] }`.
  13. `two_refused_records_are_unexplained` (PC-D19; Review Focus 1): the two records have `receipt_code: 429`, `admission: Refused`, `terminal: false`, `scored: false`, `gate: None`, `member: false`, and no credit event. The result is `Unexplained { fields: vec!["terminal"] }`. Two equal refusals are never `Equal`: nothing content-dependent was compared.

  The server crate has no `regex` dependency, and this plan adds none. Where a test names a pattern: use `is_safe_label` (`versioned_pipeline_qualification.rs`) for `^[a-z0-9_]{1,64}$`, write the 128-character class as a byte check in the test module, and detect a UUID with `Uuid::parse_str`.

- [ ] **Step 2: Run the tests to see them fail.**

Run: `cargo test -p trace-commons-server --lib versioned_pipeline_comparison`
Expected: a compile failure (the types do not exist).

- [ ] **Step 3: Implement.** The types exactly as in the Interfaces section. `compare_records_with`:
  1. If `position`, `partition`, or `trace_hash` differ, or the sides are not `Baseline` and `Candidate` in that order, or `scored != gate.is_some()` on a side, answer `Unexplained { fields: vec!["record_pair"] }`.
  1a. If `receipt_code` differs, answer `Unexplained { fields: vec!["receipt_code"] }` and compare nothing more (spec section 8.2).
  2. If a side has `terminal: false`, the field `terminal` is a difference, also when the two values are equal.
  3. Compare the other fields in the listed order. If `scored` differs, do not list the fields of `gate`. If both are scored, compare each `GateValues` field.
  4. For each difference, call `permit`. Collect the rule ids of the permitted differences (sorted, no duplicates) and the names of the others.
  5. Answer `Unexplained` if a name remains, else `Permitted` if a rule was used, else `Equal`.

  `compare_records` calls `compare_records_with` with a function that always answers `None`. `ComparisonRule`: `DeterministicIndexKeys` (`"deterministic_index_keys"`, `"compatibility_mapping.required_behavior_change_2"`, `["index_entry_id", "nearest_neighbor_hash"]`), `LedgerReasonText` (`"ledger_reason_text"`, `"ruling.T15-2"`, `["ledger_reason"]`), `ShadowValuesNotInContract` (`"shadow_values_not_in_contract"`, `"compatibility_mapping.membership_and_credit_rules"`, `["dedup_penalty", "contributor_cap", "anomaly_withheld"]`). The module doc comment states the purpose, the side names, and that the module has no database, HTTP, or clock.

- [ ] **Step 4: Run the tests.** The Step 2 command. Expected: PASS, 13 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-server/src/versioned_pipeline_comparison.rs crates/trace-commons-server/src/lib.rs
git commit -m "Compare two pipeline decision records field by field"
```

---

### Task 2: Alignment, floors, summary, and the report

**Files:**
- Modify: `crates/trace-commons-server/src/versioned_pipeline_comparison.rs`

**Interfaces:**
- Consumes: Task 1's types. `trace_commons_gate_api::pipeline::BundlePackage`, `crate::versioned_pipeline_qualification::package_digests`, `trace_commons_protocol::canonical_json::to_canonical_vec`.
- Produces: `AlignmentAction`, `hash_rule`, `alignment_action`, `alignment_lost`, `DerivedFloors`, `lower_median`, `derive_floors`, `ComparisonSummary`, `ComparisonReportInput`, `comparison_report`.

- [ ] **Step 1: Write the failing tests.**
  1. `the_hash_rule_reads_the_first_byte`: `hash_rule("sha256:00...")` and `hash_rule("sha256:fe...")` are `Approve`; `hash_rule("sha256:01...")` and `hash_rule("sha256:ff...")` are `Reject`; a text without the prefix or with a first byte that is not hex is `Reject` (fail closed).
  2. `the_alignment_table_covers_every_pair` (Review Focus 2): for each of the 25 pairs of `AdmissionLabel`, assert the action. The six rows of spec section 8.2: `(Admit, Admit) -> None`, `(Admit, Quarantine) -> ApproveCandidate`, `(Admit, Reject) -> SkipBaselineGate`, `(Quarantine, Admit) -> ApproveBaseline`, `(Quarantine, Quarantine) -> HashRuleBoth(hash_rule(trace_hash))`, `(Quarantine, Reject) -> RejectBaseline`. `(Reject, Reject) -> None`. `(Reject, Admit)` and `(Reject, Quarantine) -> Stop` (the old path has no direct rejection; do not guess). Every pair with `Refused` or `Other` on a side `-> Stop`.
  3. `the_lower_median_is_exact`: `lower_median(&[]) == None`; `[5] -> 5`; `[3, 1] -> 1`; `[9, 1, 5] -> 5`; `[4, 1, 3, 2] -> 2`. The input order does not change the result.
  4. `floors_are_the_three_medians`: `derive_floors(&[10, 30, 20], &[0, 0, 0], &[7, 9, 8])` is `Ok(DerivedFloors { 20, 0, 8 })`. `derive_floors(&[], &[], &[])` is `Err("comparison_calibration_empty")`. `derive_floors(&[0, 0], &[0, 0], &[0, 0])` is `Err("comparison_floors_all_zero")`. Slices of different lengths are `Err("comparison_calibration_invalid")`.
  5. `the_summary_counts_each_result`: observe three pairs (one `Equal`, one `Permitted { rules: ["test_rule"] }`, one `Unexplained { fields: ["admission", "member"] }`). Build the report. Assert `compared_count == 3`, `equal_count == 1`, `permitted_counts == {"test_rule": 1}`, `unexplained_counts == {"admission": 1, "member": 1}`, `unexplained_total == 1`, `unexplained` has one entry with the pair's `position`, `trace_hash`, and fields, and `first_unexplained_position` is that position.
  6. `the_unexplained_list_is_bounded`: observe 1,005 unexplained pairs; `unexplained` has 1,000 entries (the first 1,000 positions) and `unexplained_total == 1005`.
  7. `the_branch_check_reports_each_gap` (Review Focus 1): observe only pairs with `scored: false`; `branch_gaps()` holds all six labels. Then observe one pair of the Task 1 helper record: `quality_passed_true`, `novelty_passed_true`, and `member_true` leave the list.
  8. `the_distribution_counts_each_side`: observe a pair whose baseline is the helper record and whose candidate is rejected and not scored; `distribution.baseline.admit == 1`, `distribution.baseline.quality_passed == 1`, `distribution.candidate.reject == 1`, `distribution.candidate.scored == 0`.
  9. `the_report_digest_is_stable` (Review Focus 5): build the report two times from the same observations; the two values are equal, `report_digest` is the SHA-256 of the canonical bytes of the report without that field, and the report has no key that names a time (`*_at`, `duration*`, `elapsed*`).
  10. `the_report_holds_its_fixed_fields`: `schema`, `scope == "local_test"`, `production_ready == false`, `external_payout_enabled == false`, `safe_blockers == COMPARISON_BLOCKERS`, `excluded_rules` has the three rules, `pin` has the five digests, `floors` has the three numbers, `skew` is `null`, `alignment_lost_position` is `null`, `bundle_id` is the field of the package, and the three other package digests are those of `package_digests(package)` (it gives three digests, not four).
  11. `stop_with_a_candidate_admit_loses_the_alignment` (PC-D20): with `AlignmentAction::Stop`, a candidate record with `admission: Admit` gives `alignment_lost == true`, for a baseline with `admission: Refused` and for one with `Other`. With `Stop`, a candidate with `admission: Quarantine`, `Reject`, or `Refused` gives `false`. The two refused records of Task 1 test 13 give `false`: the run continues, and PC-D19 fails it at its end.
  12. `the_alignment_is_lost_only_when_the_indexes_can_differ` (PC-D20; Review Focus 2): with `AlignmentAction::None`, the pair of the Task 1 helper record gives `false`. A candidate with `terminal: false` gives `true`, and a baseline with `terminal: false` gives `true`. Two terminal records with `member: true` on the baseline and `member: false` on the candidate give `true`. Two terminal member records with `member_chunks` `[0, 1]` and `[0]` give `true`. A pair that differs only in `perplexity_micros`, and a pair that differs only in `admission` (with `ApproveCandidate`, the two sides terminal and members), each give `false`: a difference that leaves the indexes equal does not stop the run.
  13. `the_report_names_the_alignment_loss` (PC-D20): build the report with `alignment_lost_position: Some(4)`; the field is `4`. The report digest differs from that of the same report with `None`.

  For tests 5 to 10 and 13, build the `BundlePackage` with `crate::versioned_pipeline_bundle::MinimalPolicyBundle::compatibility_package(&CompatibilityBundleConfig::local_reference(), &ReferencePerplexityScorer::new(), &ReferenceEmbedder::new())`, as the tests of `versioned_pipeline_compat.rs` do.

- [ ] **Step 2: Run the tests to see them fail.** `cargo test -p trace-commons-server --lib versioned_pipeline_comparison`. Expected: a compile failure.

- [ ] **Step 3: Implement.**
  - `lower_median`: copy, sort, answer the element at index `(len - 1) / 2`.
  - `ComparisonSummary`: counters in `BTreeMap`s (a fixed order), two `SideDistribution` structs, a `Vec` of at most `UNEXPLAINED_LIST_LIMIT` entries, and `first_unexplained_position: Option<u64>`. `observe` keeps no record.
  - `branch_gaps` reads the baseline distribution: a label is in the list when its count is zero.
  - `alignment_lost`: the three conditions of the Interfaces section, in that order, and nothing more. It reads only `admission`, `terminal`, `member`, and `member_chunks`. It has no database, HTTP, or clock.
  - `refused_total`: the sum of the two `refused` counts of the distribution.
  - `comparison_report` builds the object of the Interfaces section, sets `branch_gaps` to `[]` when `partial` is true, and sets the digest as `corpus_report` does in `pipeline_corpus_pg_tests.rs`.

- [ ] **Step 4: Run the tests.** Expected: PASS, 26 tests in the module.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-server/src/versioned_pipeline_comparison.rs
git commit -m "Add the comparison alignment, floors, and report"
```

---

### Task 3: Export with events, and the local fixtures

**Files:**
- Modify: `crates/trace-commons-server/src/bin/pilot_bootstrap/translators.rs` (add `trace_steps_for`, `trace_file_for`)
- Modify: `crates/trace-commons-server/src/bin/pilot_bootstrap/submitter.rs` (use them; remove `steps_for`)
- Modify: `crates/trace-commons-server/src/bin/trace-commons-pipeline-corpus-export.rs`
- Create: `crates/trace-commons-server/tests/fixtures/pipeline-compare-jsonl/` (12 `.jsonl` sessions, `pin-local.json`, `pin-local-risk.json`)

**Interfaces:**
- Consumes: `trace_commons_protocol::llm::recording::{TraceFile, TraceStep, TraceResponse, TraceToolCall, ExpectedToolResult}`, `SubmissionDraft`, `SessionEvent`.
- Produces:
  - `pub fn trace_steps_for(event: &SessionEvent) -> Vec<TraceStep>` and `pub fn trace_file_for(draft: &SubmissionDraft) -> TraceFile` in `translators.rs`.
  - Export flags: `--with-events` (bool); `--session-name <NAME>` (repeatable; when given, only the named files are candidates; refused without `--local-jsonl-dir` with the message `session names need a local directory`); `--declared-privacy-risk <NAME=RISK>` (repeatable; `RISK` is `medium` or `high`; refused without `--local-jsonl-dir` with the message `declared privacy risk needs a local directory`). One fixture directory serves two pins, so each pin names its sessions.
  - With `--with-events`: output files `bootstrap-compare.jsonl`, `holdout-compare.jsonl`, `source-manifest.json`. The manifest's `source` object has one more key, `"with_events": true`, and its two corpus digests are the SHA-256 of the two JSONL files.
  - The local pins, read by Task 6 and Task 7.

- [ ] **Step 1: Move the step builder (no behavior change).** Move `steps_for` and its doc comment from `submitter.rs` to `translators.rs` as `pub fn trace_steps_for`. Four build targets include `translators.rs` (PC-D4), and two of them call neither new function, so put `#[allow(dead_code)]` with a one-line reason on the two functions, as `translators.rs` does already for this case. Add:

```rust
/// The recorded trace that `build_envelope_from_draft` redacts: one step for
/// each session event, in order.
pub fn trace_file_for(draft: &SubmissionDraft) -> TraceFile {
    TraceFile {
        model_name: format!("pilot-bootstrap/{}", draft.source_dataset),
        memory_snapshot: Vec::new(),
        http_exchanges: Vec::new(),
        steps: draft.session_events.iter().flat_map(trace_steps_for).collect(),
    }
}
```

  `build_envelope_from_draft` now starts with `let trace = trace_file_for(draft);`. No test moves: no test calls `steps_for` directly (the three tests in `submitter.rs` test `build_envelope_from_draft`). Run `cargo test -p trace-commons-server --bin trace-commons-pilot-bootstrap`, `--test pilot_bootstrap_translators`, and `--test pilot_bootstrap_submitter`. Expected: the same pass counts as before the move (record the counts in the ledger).

- [ ] **Step 2: Write the fixture sessions.** 12 files in `tests/fixtures/pipeline-compare-jsonl/`, named `s01.jsonl` to `s10.jsonl`, `s04h.jsonl`, and `s04m.jsonl`, in the swival session shape that `translators.rs` documents (a `session` line, then `message` lines with `role` and `content`; two files also have a `tool_use` and a `tool_result`). Content rules (the reference scorer is the byte entropy of a chunk; the reference embedder is a bag of lowercase tokens):
  - `s01` to `s04` (the bootstrap partition): four different prose sessions of 300 to 500 words about different subjects, with normal English text.
  - `s05`, `s06`: two more different prose sessions (expected: quality pass, novelty pass).
  - `s07`: one message of 400 words that repeats one six-letter word (expected: low perplexity, quality failure).
  - `s08`: the text of `s05` with a different session id and one word changed (expected: novelty failure).
  - `s09`: a session with a tool call and a tool result (the result names its call id).
  - `s10`: a session of about 30,000 characters (more than one chunk).
  - `s04h`, `s04m`: two short prose sessions, used only by the risk pin. The selection is in name order, so these two names put the risk traces before `s05` and `s06` in the holdout partition (plan review G17). A scored trace then comes after each risk trace.
  - No file contains a real name, address, credential, or text copied from a dataset. Write the text yourself.

- [ ] **Step 3: Write the failing export tests** in the `tests` module of the export binary:
  1. `v1_output_is_unchanged`: run the export logic for the directory `tests/fixtures/pipeline-hf-jsonl` with the settings of its `pin-local.json` and no `--with-events`; the manifest's five digests equal the values in that pin. (Factor `main` into `async fn run(args: Args) -> anyhow::Result<Value>` that returns the manifest, so a test can call it with a temporary `output_dir`.)
  2. `with_events_writes_one_fixture_on_each_line`: run with `--with-events` on `pipeline-compare-jsonl` (bootstrap 4, holdout 6, `min_words` 1, `max_words` 20000, and `--session-name` for `s01.jsonl` to `s10.jsonl`). `bootstrap-compare.jsonl` has 4 lines and `holdout-compare.jsonl` has 6. Each line parses, has exactly the keys `label`, `trace_id`, `submission_id`, `created_at`, `secret_probe`, `privacy_risk`, `trace_file`, and has no `input` key. The line of `s09` has a step whose `response.type` is `tool_calls`. `serde_json::from_value::<TraceFile>` accepts each `trace_file`.
  3. `with_events_identities_equal_the_v1_identities`: for the same directory and settings, the `label`, `trace_id`, `submission_id`, and `created_at` of each JSONL line equal those of the v1 fixture at the same position.
  4. `with_events_changes_the_configuration_digest_only_by_its_flag`: the manifest's `source` has `with_events: true`; `source_digest` and `order_digest` equal those of the v1 run.
  5. `a_declared_risk_reaches_the_fixture`: with `--declared-privacy-risk s04m.jsonl=medium --declared-privacy-risk s04h.jsonl=high`, the two lines have `privacy_risk` `medium` and `high`, and every other line has `low`. A name that no selected session has is an error (`declared privacy risk names no selected session`). A risk other than `medium` or `high` is an error. The flag without `--local-jsonl-dir` is an error.
  6. `each_session_is_written_before_the_next_is_read` (Review Focus 4): call the local selection with a session source and a fixture writer that both append to one shared event list (`read:<name>`, `write:<label>`). For five sessions the list is `read, write, read, write, ...`: no second `read` comes before the first `write`. This proves that the `--with-events` path holds one session at a time.
  7. `no_fixture_line_carries_the_source_name`: no line contains one of the 12 file names of the fixture directory.
  8. `a_session_name_list_limits_the_candidates`: with `--session-name s01.jsonl --session-name s02.jsonl`, bootstrap 1, and holdout 1, the export selects exactly those two files, in name order. A name that is not in the directory is an error (`session name is not in the directory`). The flag without `--local-jsonl-dir` is an error.

- [ ] **Step 4: Run the tests to see them fail.** `cargo test -p trace-commons-server --bin trace-commons-pipeline-corpus-export`. Expected: a compile failure.

- [ ] **Step 5: Implement.**
  - Keep the v1 path as it is: collect, then write the two `.json` files. Its code does not change except for the move into `run`.
  - `--with-events` path: open the two JSONL writers first. For each selected session, in order: update the source hasher and the order list exactly as the v1 path does; build the fixture object (`label`, `trace_id`, `submission_id`, `created_at`, `secret_probe` from the same helpers as `fixture`; `privacy_risk` from the declared map or `low`; `trace_file` from `trace_file_for(&draft)`); write its canonical JSON and `\n` to the bootstrap writer for the first `bootstrap_count` sessions and to the holdout writer for the rest; hash the same bytes into that file's digest; drop the session. Write through a temporary file and rename, as `atomic_json` does. `atomic_json` has one temporary name for each process, and two writers are open at the same time here: give each writer its own temporary name.
  - `selected_sessions` becomes a function that calls a closure for each selected session, so the two paths share the selection loop. The translator must give `draft.session_events`; the selection keeps `draft` for the `--with-events` path.
  - Windowed download (PC-D13), remote mode only: take the next 16 names, start one `fetch_session` task for each in a `tokio::task::JoinSet` (hold the dataset in an `Arc`), wait for all 16, then handle the 16 results in name order. Stop after the window in which the count is reached. A download that fails gets one more attempt after its window, in name order (`hf-hub` makes no retry of its own); a second failure is an error, as today. The local mode does not change.
  - `--declared-privacy-risk`: parse into a `BTreeMap<String, String>`; after the selection, every key must have matched.
  - `--session-name`: parse into a `BTreeSet<String>`; in local mode, keep only the listed files before the selection loop; every name must be a file of the directory.

- [ ] **Step 6: Run the tests.** The Step 4 command. Expected: PASS (the count before this task, 24 on `main` at `0e5dffe6b`, plus 8 new tests).

- [ ] **Step 7: Write the two local pins.** Run the export two times by hand, from the worktree root, to get the digests. The output directory must be under the ignored `.local/` of the worktree, so give it as an absolute path:

```bash
cargo run -q -p trace-commons-server --bin trace-commons-pipeline-corpus-export -- \
  --repository jedisct1/security-audits --revision 6d527ff0081eec6704c2a4f00e1ef8d308ae7366 \
  --split train --translator swival --with-events \
  --local-jsonl-dir crates/trace-commons-server/tests/fixtures/pipeline-compare-jsonl \
  --session-name s01.jsonl --session-name s02.jsonl --session-name s03.jsonl --session-name s04.jsonl \
  --session-name s05.jsonl --session-name s06.jsonl --session-name s07.jsonl --session-name s08.jsonl \
  --session-name s09.jsonl --session-name s10.jsonl \
  --output-dir "$PWD/.local/pipeline/compare-pin-draft" \
  --bootstrap-count 4 --holdout-count 6 --min-words 1 --max-words 20000
```

  `pin-local.json` (schema `trace_commons.pipeline_hf_corpus_pin.v1`) holds the pin fields of `pipeline-hf-jsonl/pin-local.json`, with `bootstrap_count: 4`, `holdout_count: 6`, `min_words: 1`, `max_words: 20000`, `with_events: true`, the five digests of the manifest, `local_jsonl_dir: "crates/trace-commons-server/tests/fixtures/pipeline-compare-jsonl"`, and `session_names: ["s01.jsonl", ..., "s10.jsonl"]`.
  `pin-local-risk.json`: `session_names` `s01.jsonl` to `s04.jsonl`, `s04h.jsonl`, `s04m.jsonl`, `s05.jsonl`, `s06.jsonl`; `bootstrap_count: 4`, `holdout_count: 4`; `declared_privacy_risk: {"s04m.jsonl": "medium", "s04h.jsonl": "high"}`; its own digests from a second export with those flags. The holdout order is then `s04h` (high), `s04m` (medium), `s05`, `s06`.

- [ ] **Step 8: Prove that the old checks did not change.** `cargo test -p trace-commons-server --bin trace-commons-pilot-bootstrap`, `cargo test -p trace-commons-server --test pilot_bootstrap_translators`, `cargo test -p trace-commons-server --test pilot_bootstrap_submitter`, and `python3 scripts/operator/test_pipeline_tooling.py`. Expected: PASS. Then run the checks for the package of Tasks 1 to 3 (clippy and `license_boundary`, Global Constraints).

- [ ] **Step 9: Commit**

```bash
git add crates/trace-commons-server/src/bin/pilot_bootstrap crates/trace-commons-server/src/bin/trace-commons-pipeline-corpus-export.rs crates/trace-commons-server/tests/fixtures/pipeline-compare-jsonl
git commit -m "Export a pinned sample with its session events"
```

---

### Task 4: Envelope builder, corpus reader, and calibration

**Files:**
- Create: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_compare_pg_tests.rs`
- Modify: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs` (declare `mod pipeline_compare_pg_tests;` beside `mod pipeline_corpus_pg_tests;`)

**Interfaces:**
- Consumes: Task 2's `derive_floors`, `DerivedFloors`; Task 3's corpus format and fixtures; `trace_commons_gate_enclave`'s `EnclaveGateOrchestrator` and `MockVectorIndex` (import them as `trace_gate_service.rs` does); `EnclaveGateOrchestratorConfig`; `ReferencePerplexityScorer`, `ReferenceEmbedder`; `MainGateConfig`; `DeterministicTraceRedactor`, `RawTraceContribution`, `RecordedTraceContributionOptions`; `make_metadata_only_low_risk`, `set_metadata_only_tool_name`.
- Produces (all in the new file):

```rust
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompareFixture {
    label: String,
    trace_id: Uuid,
    submission_id: Uuid,
    created_at: DateTime<Utc>,
    secret_probe: String,
    privacy_risk: String,
    trace_file: TraceFile,
}

/// Reads one fixture for each call. It never holds more than one line.
struct CompareCorpusReader { /* a BufReader, the partition name, the next position, a SHA-256 of the bytes read */ }
impl CompareCorpusReader {
    fn open(partition: &'static str, path: &Path) -> Result<Self, &'static str>;
    fn next_fixture(&mut self) -> Result<Option<CompareFixture>, &'static str>;
    /// The digest of the whole file; reads the lines that are left.
    fn finish(self) -> Result<String, &'static str>;
}

async fn compare_envelope(fixture: &CompareFixture) -> TraceContributionEnvelope;

/// `main`'s gate configuration for the run: the derived floors and the
/// values of `CompatibilityBundleConfig::local_reference()`.
fn compare_main_gate(floors: DerivedFloors) -> MainGateConfig;

// baseline-old-path:
fn baseline_orchestrator_config(gate: &MainGateConfig) -> EnclaveGateOrchestratorConfig;

async fn calibrate_floors(bootstrap: &Path) -> Result<DerivedFloors, &'static str>;
```

- [ ] **Step 1: Write the failing tests** (no database; they run in the default test run of the ingest binary). Two test helpers build the input: `fn fixture_line(label: &str, risk: &str, steps: Vec<TraceStep>) -> String` gives one canonical JSON line with a `trace_id` and a `submission_id` that are `Uuid::new_v5` of the label, `created_at` 2026-01-01T00:00:00Z, and `secret_probe` `compare_probe_<label>`; `fn write_corpus(lines: &[String]) -> (tempfile::TempDir, PathBuf)` writes them to a file. A prose step is `TraceStep { request_hint: None, response: TraceResponse::UserInput { content }, expected_tool_results: vec![], timestamp: None }`.
  1. `the_corpus_reader_is_lazy` (Review Focus 4): write a file with two valid fixture lines and a third line that is not JSON. Two calls of `next_fixture` answer `Ok(Some(_))`. The third answers `Err("compare_corpus_line_invalid")`. A reader that parsed the whole file first would fail at the first call. Second part, which a reader that loads the whole file in `open` cannot pass: open a file with one valid line, read it, append a second valid line to the file, and require that the next call returns the second fixture. Do not copy `load_corpus` of `pipeline_corpus_pg_tests.rs`: it reads the whole file.
  2. `the_corpus_reader_refuses_bad_fixtures`: an unknown field, a `privacy_risk` that is not `low`, `medium`, or `high`, an empty `secret_probe`, and a label that does not match `^[a-z0-9_]{1,64}$` each give `Err("compare_corpus_line_invalid")`. An empty file gives `Ok(None)` and `finish` gives the SHA-256 of zero bytes.
  3. `the_reader_digest_is_the_file_digest`: for a file of three fixture lines, read one fixture and call `finish()`; the result equals `sha256_bytes(&std::fs::read(path))`. The same holds when all three fixtures were read first.
  4. `the_same_fixture_gives_the_same_envelope_bytes` (Review Focus 5): build the envelope two times for one fixture; `serde_json::to_vec` gives equal bytes. The envelope has `trace_id` and `submission_id` of the fixture, `created_at` of the fixture, more than one event for a fixture with more than one step, consent scope `ModelTraining`, and `allowed_uses == [ModelTraining]`. Use a fixture with a tool call and a tool result, and with steps that have no timestamp: the tool result's `parent_event_id` is the new id of its call in the two envelopes, and each event timestamp is the fixture's `created_at`.
  5. `a_tool_session_keeps_its_tool_events`: for a fixture whose `trace_file` has a `tool_calls` step with a tool result, the envelope has a tool event.
  6. `a_declared_risk_gives_a_metadata_only_envelope`: `privacy_risk: "medium"` gives `residual_pii_risk == Medium` and no message text; `"high"` gives `High`.
  7. `the_baseline_configuration_holds_mains_gate`: for `gate = compare_main_gate(DerivedFloors { 20, 5, 8 })`, `baseline_orchestrator_config(&gate)` has the three floors, `top_k == 5` (PC-D23), `chunk_target_tokens == 2048`, `chunk_max_tokens == 3072`, `chunk_cap == 16`, `chunk_min_tokens == 64`, `embed_insert_novelty_micros == 50_000`, and `qualifying_chunk_floor_micros == 20` (PC-D7). `CompatibilityBundleConfig::production_compatible("reference_perplexity.v1".into(), MINIMAL_PROJECTION_ID.into(), MINIMAL_INDEX_ID.into(), &gate)` is `Ok`, and `matches_main_gate(&gate)` is true. `gate.novelty_utility_microcredits == 2_500_000`.
  8. `calibration_is_deterministic_and_not_zero`: `calibrate_floors` on a file of four prose fixtures with four different texts of 200 words or more gives the same `DerivedFloors` two times, and `perplexity_floor_micros > 0`. On an empty file it gives `Err("comparison_calibration_empty")`.
  9. `the_two_indexes_give_the_same_similarity`: insert the same five unit vectors (from `ReferenceEmbedder` on five different texts) into a `MockVectorIndex` and into an `IsolatedPipelineIndex` (through `VectorIndexWriter`, as the tests of `versioned_pipeline_index.rs` do). For three query vectors, the best similarity of `nearest(.., 8)` is bit-equal (`to_bits()`) on the two indexes, and the ordered list of similarities is bit-equal.

- [ ] **Step 2: Run the tests to see them fail.** `cargo test -p trace-commons-server --bin trace-commons-ingest pipeline_compare_pg_tests`. Expected: a compile failure.

- [ ] **Step 3: Implement.**
  - `compare_envelope`: `RawTraceContribution::from_recorded_trace(&fixture.trace_file, RecordedTraceContributionOptions { include_message_text: true, include_tool_payloads: true, pseudonymous_contributor_id: Some("sha256:compare-contributor".into()), tenant_scope_ref: Some("tenant_sha256:compare".into()), ..Default::default() })`. Then set `trace_id`, `submission_id`, `created_at`, the revocation handle (`Uuid::new_v5(&Uuid::NAMESPACE_URL, "tracecommons:compare-revocation:<label>")`), and each event id (`"tracecommons:compare-event:<label>:<index>"`), as `fixture_envelope` does in `pipeline_corpus_pg_tests.rs`. Two fields need more than that list, because `fixture_envelope` builds from capture turns and this function builds from a recorded trace (plan review G19): (1) `from_recorded_trace` gives `Utc::now()` to each step that has no timestamp, so before the call give each step with `timestamp: None` the value `Some(fixture.created_at)`; (2) a tool result names its call through `parent_event_id`, and the call's event id is random, so when you replace the event ids, map each `parent_event_id` to the new id of its parent. Redact with `DeterministicTraceRedactor::try_default()`. For `medium` or `high`: `make_metadata_only_low_risk`, `set_metadata_only_tool_name(&mut envelope, &fixture.label)`, and the declared risk. Set the three consent fields. If the envelope bytes still differ between two calls, find the field with a diff of the two JSON values and make it deterministic from the label; write the field name in the ledger.
  - `calibrate_floors`: open the reader; for each fixture, build the envelope, serialize it with `serde_json::to_vec`, and call `EnclaveGateOrchestrator::evaluate(&bytes, "compare-calibration")` on one orchestrator (reference scorer, reference embedder, a new `MockVectorIndex`, `baseline_orchestrator_config(&compare_main_gate(DerivedFloors { 0, 0, 0 }))` with `gate_policy_version: "compare_calibration_v1"`). Collect `perplexity_micros`, `tail_fraction_micros`, `novelty_score_micros`; answer `derive_floors`. Run it with `tokio::task::spawn_blocking` for the scorer calls (they are synchronous).
  - The file's module doc comment explains the two sides, the serial order, and that the code marked `// baseline-old-path:` goes away with the old path.

- [ ] **Step 4: Run the tests.** Expected: PASS, 9 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-server/src/bin/trace_commons_ingest_internal
git commit -m "Build comparison envelopes and calibrate the gate floors"
```

---

### Task 5: The app with two tenants, and the two side drivers

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_compare_pg_tests.rs`
- Modify: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs` (add `fixture_gate_worker_artifact_store_with_key`)
- Modify: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_http_pg_tests.rs` (make `wait_for_run_state` and `LEGACY_NOVELTY_UTILITY_CREDIT_POINTS_DELTA` `pub(super)`; `tenant_tx` is `pub(super)` already). `run_of_submission` stays private: it panics when no run exists, and the drivers need an answer for that case. `wait_for_run_state` panics at 60 s; use it only in the spike test. `send_http` and `post_trace` panic on a transport error; the drivers call them through a wrapper that gives `Err("compare_http_failed")` (use `reqwest` directly, as those two helpers do)

**Interfaces:**
- Consumes: Task 1's `ComparisonRecord` and labels; Task 2's `alignment_action`; Task 4's envelope and gate configuration; `serve_pipeline_app`, `wait_for_pipeline_ready`, `mains_database`, `runtime_backend`, `account_owner_backend`, `post_trace`, `send_http`, `allow_all_test_authority`, `join_within` (`pipeline_http_pg_tests.rs`); `routing_store` (`tests.rs`); `assemble_ingest_pipeline_runtime`; `PipelineServiceBuilder`; `DeterministicPipelinePrivacyBoundary`; `PipelineService::store().get_run`, `PipelineService::load_index_command`; `ScoreEvidence`.
- Produces:

```rust
fn fixture_gate_worker_artifact_store_with_key(artifact_root: &Path, master_key_hex: &str)
    -> (ConfiguredTraceArtifactStore, Arc<dyn KmsKeyWrapper>, String);   // in tests.rs

struct CompareApp {
    base: String,
    client: reqwest::Client,
    owner: Arc<PgBackend>,
    runtime: Arc<PgBackend>,
    service: Arc<PipelineService>,
    package: BundlePackage,
    tenants: CompareTenants,
    root: PathBuf,   // a copy of `state.root`: the directory that `read_all_derived_records` reads (PC-D18)
    stop: /* the sender that `serve_pipeline_app` returns */,
    server: /* its join handle */,
}
/// The tenant names and the five tokens of one app. `CompareTenants::run()`
/// gives the constants below; `CompareTenants::random()` gives names with a
/// `Uuid::new_v4().simple()` suffix, for every test of this task (PC-D17).
struct CompareTenants { baseline: String, candidate: String, /* the five tokens */ }
struct CompareAppOptions<'a> {
    tenants: CompareTenants,
    skew: Option<&'a str>,
    /// `true` in the run (spec section 8.1). `false` only in the three tests
    /// that need a baseline quarantine of a medium-risk trace (plan review G16).
    accept_medium_risk: bool,
}
async fn start_compare_app(gate: MainGateConfig, artifact_root: &Path, master_key_hex: &str, options: CompareAppOptions<'_>) -> CompareApp;

/// What a driver knows after the receipt, before any review or gate call.
struct AdmissionObservation { receipt_code: u16, privacy_risk: Option<String>, privacy_basis: Vec<String>, admission: AdmissionLabel }

// baseline-old-path:
async fn baseline_admission(app: &CompareApp, probes: &Probes, body: &[u8], submission_id: Uuid) -> Result<AdmissionObservation, &'static str>;
async fn candidate_admission(app: &CompareApp, probes: &Probes, body: &[u8], submission_id: Uuid) -> Result<AdmissionObservation, &'static str>;

// baseline-old-path:
async fn baseline_finish(app: &CompareApp, probes: &Probes, fixture: &CompareFixture, seen: AdmissionObservation, action: AlignmentAction, base: RecordBase) -> Result<ComparisonRecord, &'static str>;
async fn candidate_finish(app: &CompareApp, probes: &Probes, fixture: &CompareFixture, seen: AdmissionObservation, action: AlignmentAction, base: RecordBase) -> Result<ComparisonRecord, &'static str>;

/// PC-D18. Removes each `.json` file of the baseline tenant's `derived/`
/// directory (`<app.root>/tenants/<tenant_storage_key(baseline)>/derived/`)
/// and answers the number of files that it removed. A directory that does
/// not exist answers 0. It removes nothing else: no other directory, no file
/// of the candidate tenant, no database row. An I/O error is
/// `Err("compare_baseline_derived_clear_failed")`.
// baseline-old-path:
fn clear_baseline_derived(app: &CompareApp) -> Result<u64, &'static str>;

/// `position`, `partition`, `trace_hash`: equal on the two sides.
struct RecordBase { position: u64, partition: &'static str, trace_hash: String }

/// The strings that no HTTP response, record, or report may contain.
/// `secret_probe` is a label beside the text, so it cannot find trace text.
/// The content probe can: a fixed window (the first 48 bytes, at a character
/// boundary) of the first event text of the current fixture that has 48 bytes
/// or more. It is checked against each RECEIVED body, the two records, and the
/// report. It is not checked against the sent body, which holds the text.
struct Probes { /* the harness tokens, the current fixture's secret_probe, and its content probe */ }
impl Probes {
    /// All probes. For a received body, a record, and the report. Panics with `label`.
    fn check(&self, bytes: &[u8], label: &'static str);
    /// The tokens only. For a sent body, which holds the trace text by design.
    fn check_sent(&self, bytes: &[u8], label: &'static str);
}
```

  Tenants and tokens of `CompareTenants::run()` (constants in the file, used only by `pipeline_compare_run`): `tenant-compare-baseline`, `tenant-compare-candidate`; tokens `token-compare-baseline-contributor`, `token-compare-baseline-reviewer`, `token-compare-baseline-gate` (`TokenRole::VectorWorker`), `token-compare-candidate-contributor`, `token-compare-candidate-reviewer`.

- [ ] **Step 1: Prove the two paths for one trace before any driver code (a spike in the test file, kept as a test).** Write `one_trace_reaches_both_gates` (it self-skips without `TRACE_COMMONS_PG_TEST_DATABASE_URL`, as the other PostgreSQL tests of this binary do). It starts the app with `compare_main_gate(DerivedFloors { perplexity_floor_micros: 1, tail_fraction_floor_micros: 0, novelty_floor_micros: 0 })` and `CompareTenants::random()`, builds the envelope of one prose fixture, and asserts:
  0. Before any receipt: `SELECT bundle_id FROM pipeline_active_bundles WHERE tenant_id = <candidate>` equals `app.package.bundle_id` (PC-D17: a tenant keeps its first bundle), and `state.submission_quota` is disabled (0 and 0).
  1. `POST /v1/traces` with the baseline token answers 200 with `status == "accepted"` (not `awaiting_pii_backstop`: the backstop is off in the test state; if it is on, turn it off in `start_compare_app` and name the state field in the ledger).
  2. `POST /v1/workers/gate/evaluate` with the gate token and `{"submission_id": <id>}` answers 200, and a `trace_gate_decisions` row exists for the baseline tenant with `perplexity_micros > 0`.
  3. `POST /v1/traces` with the candidate token answers 200 with `status == "processing"`, and `wait_for_run_state(.., "complete")` returns.
  4. The candidate's Score evidence (`SELECT evidence FROM phase_outcomes WHERE tenant_id = $1 AND run_id = $2 AND phase = 'score'`) parses as `ScoreEvidence` with `perplexity_micros.is_some()`.
  5. `service.load_index_command(&run, &evidence)` answers `Ok(Some(_))` or `Ok(None)` (not an error).

  **If the app refuses to start because the bundle is production-compatible with test dependencies (a readiness or qualification label, or `pipeline_unqualified_routing_with_production_runtime`), stop.** The same applies if the candidate receipt answers `503` with `pipeline_bundle_not_qualified`. Write the label in the ledger and ask the owner. Do not fall back to `local_reference()` on your own: D3 names `production_compatible`.

  Fresh database: `dropdb -h 127.0.0.1 -p 55434 -U trace --if-exists admission_test_cmp_spike; createdb -h 127.0.0.1 -p 55434 -U trace admission_test_cmp_spike`.
  Run: `TRACE_COMMONS_PG_TEST_DATABASE_URL=postgres://trace@127.0.0.1:55434/admission_test_cmp_spike cargo test -p trace-commons-server --bin trace-commons-ingest one_trace_reaches_both_gates -- --test-threads=1`
  Expected at this step: a compile failure (`start_compare_app` does not exist).

- [ ] **Step 2: Implement `start_compare_app`.** Follow `legacy_and_pipeline_tenants_match_under_equivalent_configuration` (`pipeline_http_pg_tests.rs`, about line 7251 on `main` at `0e5dffe6b`) step by step, with these differences:
  - The store is `fixture_gate_worker_artifact_store_with_key(artifact_root, master_key_hex)`. Write that function in `tests.rs` by taking the key as a parameter in the body of `fixture_gate_worker_artifact_store_with_decryptor`, and make the old function call the new one with a generated key.
  - `state.gate_service = Arc::new(EnclaveGateService::new(EnclaveGateOrchestrator::new(ReferencePerplexityScorer::new(), ReferenceEmbedder::new(), MockVectorIndex::new(), baseline_config), decryptor, "enclave_reference_compare"))`, where `baseline_config = baseline_orchestrator_config(&gate)` with `gate_policy_version: "compare_gate_v1"`. If `skew == Some("baseline_quality_floor")`, set `baseline_config.perplexity_floor_micros = u64::MAX` (PC-D8). Any other skew value panics with `compare_skew_invalid`.
  - `state.novelty_utility_credit_points_delta = LEGACY_NOVELTY_UTILITY_CREDIT_POINTS_DELTA` (2.5) and `gate.novelty_utility_microcredits == 2_500_000`.
  - `state.accept_medium_risk_submissions = options.accept_medium_risk` (`true` in the run, spec section 8.1).
  - Build the token map from `options.tenants`. After the LAST `account_owner_backend()` call of the function, call `configure_unbounded_submit_limits_for_test(&tokens)` with that map, then set `state.tokens` (PC-D16). The order matters: `account_owner_backend()` resets the limiter and removes an earlier override. The parity test and `pipeline_corpus_run` set `state.tokens` directly and call no override; they send fewer than 30 receipts. Do not copy that part.
  - After the start, assert that the candidate tenant's active bundle is `package.bundle_id` (the Step 1 assertion 0), in `start_compare_app` itself, so each caller has it.
  - The pipeline service comes from `assemble_ingest_pipeline_runtime` with a new `CompareAssembler` (in this file; copy the shape of `CompatibilityTestAssembler`): the package is `MinimalPolicyBundle::compatibility_package(&CompatibilityBundleConfig::production_compatible("reference_perplexity.v1".into(), MINIMAL_PROJECTION_ID.into(), MINIMAL_INDEX_ID.into(), &gate)?, &scorer, &embedder)`; the privacy boundary is `Arc::new(DeterministicPipelinePrivacyBoundary)` (PC-D6); one recording `trace_credit` adapter on the rail `none`; the last argument of `assemble_ingest_pipeline_runtime` is `gate`. Call `assemble_ingest_pipeline_runtime` with the argument values of `assemble_compatibility_pipeline_service_with`: `production_required: false`, `tenants_processed: true`, `allow_test_dependencies: true`, `unqualified_routing_allowed: true` (PC-D15). The `CompareAssembler` builds the service with `.with_unqualified_routing(context.unqualified_routing_allowed)`, as `CompatibilityTestAssembler` does. If it does not, the assembly refuses with `pipeline_runtime_unqualified_routing_mismatch`.
  - `state.tenant_rollout_gates = TraceTenantRolloutGates::for_feature(TraceTenantRolloutFeature::PipelineReceipts, &["tenant-compare-candidate"])`.
  - `state.pipeline_activation = routing_store(&runtime)` (PC-D15). The harness writes no routing row. The baseline tenant is not in the rollout gate, so its receipts stay on the old path; assertion 1 of the spike test proves it.
  - Do not start the in-process perplexity score driver (the test state does not start it; assert in a comment where that is true).
  - The baseline gets no value from an environment variable ("How the baseline is configured and identified"). Search the handlers that the harness calls (the receipt, the two review routes of the old path, the gate route, and the pipeline review routes) and the functions that they call for a read of an environment variable at request time (`std::env::var`, `env_truthy`). Write each one in the ledger with its variable name. If one exists, it is a finding for the owner: the child allowlist of `pipeline.py` is then the only guard, and a direct `cargo test` run has none.
  - Set `CompareApp.root` to a copy of `state.root` before the state goes to the app (PC-D18). Do not derive the path a second way: `clear_baseline_derived` must remove files from the directory that the receipt handler reads.

- [ ] **Step 3: Run the spike test.** The Step 1 command. Expected: PASS. Record in the ledger: the receipt status of the baseline, the two measured perplexity values (they must be equal; if they are not, that is the first finding, not a harness defect: record both values and continue), and the time from the candidate receipt to `complete`.

- [ ] **Step 4: Write the failing driver tests** (PostgreSQL; they self-skip without the URL). Each test starts one app with `CompareTenants::random()` and uses fixtures that the test builds in memory. A fixture label is used by one test only: two tests with one label send one `submission_id`, and the two paths answer a repeated id in different ways. Unless a test says another value, `accept_medium_risk` is `true`:
  1. `an_admitted_trace_gives_two_full_records`: one prose fixture. Both records have `receipt_code == 200`, `terminal == true`, `admission == Admit`, `review == None`, `scored == true`, `gate.is_some()`. `compare_records` of the two is `Equal`. If it is not, do not change the drivers to hide it: record the fields in the ledger and mark the test with the exact difference it sees (a difference here is a finding for the owner).
  2. `privacy_fields_come_from_the_submission_row`: both records have `privacy_risk == Some("medium")` and `privacy_basis == ["consent_content_flag"]` for a prose fixture (the server rescrub classifies message text this way; if the stored labels have another spelling, use the stored spelling in the assertion and say so in the ledger). If the two sides store different values for the same envelope, that is a finding: record the two values and make the test state the difference.
  3. `a_declared_medium_trace_is_aligned`: a fixture with `privacy_risk: "medium"`. The baseline has `admission == Admit`. The candidate has `admission == Quarantine`, `review == Approve`, `review_source == Alignment`, and is scored. `compare_records` is `Unexplained { fields }` and `fields` contains `"admission"`.
  4. `a_declared_high_trace_is_aligned`: `privacy_risk: "high"`. The baseline has `admission == Quarantine`, `review == Reject`, `review_source == Alignment`, `scored == false`. The candidate has `admission == Reject`, `scored == false`. `fields` contains `"admission"` and does not contain `"scored"`.
  5. `a_failed_gate_gives_no_membership_and_no_credit`: start the app with a perplexity floor of `u64::MAX` on the two sides (through `compare_main_gate`, not through the skew). Both records have `gate.quality_passed == false`, `member == false`, `member_chunks.is_empty()`, `credit_events.is_empty()`.
  6. `a_passed_gate_gives_membership_and_one_credit_event`: floors of zero except `perplexity_floor_micros: 1`. Both records have `member == true`, `member_chunks` not empty and equal on the two sides, and `credit_events == [CreditEvent { event_type: "novelty_utility", microcredits: 2_500_000 }]`.
  7. `the_second_trace_sees_the_first_in_both_indexes`: two different prose fixtures in order. For the second trace, both records have `gate.index_cardinality == Some(n)` with the same `n > 0`.
  8. `no_probe_reaches_a_body_or_a_record`: after the tests above, `Probes::check` ran on each received body and on the canonical bytes of each record, with the tokens, the `secret_probe`, and the content probe. Add a unit test of `Probes::check` that it panics with its label when the bytes contain a token, and one when the bytes contain the content probe. This is a privacy guard, so give it a mutation check (common.md): remove the content probe from the check and see the second unit test fail.
  9. `thirty_five_receipts_pass_the_rate_limit` (PC-D16, plan review G1): 35 different small prose fixtures in one app; each receipt answers 200 on the two sides. Mutation check: remove the `configure_unbounded_submit_limits_for_test` call and see receipt 31 answer 429.
  10. `twenty_quarantined_traces_all_complete` (plan review G3): 20 fixtures with `privacy_risk: "medium"` and 20 labels in one app; each candidate record has `review == Approve` and `terminal == true`. Without the wait for `awaiting_review` and the second attempt, this test fails at random.
  11. `approve_baseline_is_driven` (plan review G16): `accept_medium_risk: false`. One prose fixture gives the pair (Quarantine, Admit) and the action `ApproveBaseline`. The baseline has `review == Approve`, `review_source == Alignment`. The two records are scored, and `compare_records` reports `admission` only.
  12. `the_hash_rule_is_driven_on_both_sides` (plan review G16): `accept_medium_risk: false`. Two fixtures with `privacy_risk: "medium"` whose `trace_hash` has an even and an odd first byte (try labels until you have one of each) give (Quarantine, Quarantine). The even one has `review == Approve`, `review_source == HashRule` on the two sides; the odd one has `review == Reject` on the two sides and `scored == false`.
  13. `an_aligned_member_keeps_the_indexes_equal` (plan review G16 and G17; Review Focus 2): `accept_medium_risk: false`. One prose fixture (aligned by `ApproveBaseline`, a member on the two sides), then a second, different prose fixture. For the second trace, `gate.index_cardinality` is `Some(n)` with the same `n > 0` on the two sides. This is the test in which a trace that needed an alignment action is a member of the two indexes.
  14. `skip_baseline_gate_and_stop_are_driven` (plan review G16): call `baseline_finish` and `candidate_finish` directly with `AlignmentAction::SkipBaselineGate` for an accepted prose trace: the baseline record has `scored == false`, `gate_skipped_by_alignment == true`, and no `trace_gate_decisions` row exists. Call them with `AlignmentAction::Stop` for a baseline and a candidate `AdmissionObservation` with `admission: Refused`: no HTTP call is sent for either side (count the calls through the wrapper), each record has `terminal == false`, the two calls return in less than 1 s together (no poll and no 60 s wait, PC-D19), and `compare_records` of the two is `Unexplained { fields: ["terminal"] }`.
  15. `removing_the_derived_files_changes_no_record` (PC-D18): three prose fixtures in order. The third has the text of the second and another label, so it has another `submission_id` and the same canonical summary. Send them to two apps, each with `CompareTenants::random()`. App A calls `clear_baseline_derived` after the two finish steps of each trace, as the run does. App B does not call it. Assert:
      - (a) In app B, `read_all_derived_records(&app.root, baseline)` gives three records, and the third has `duplicate_score > 0`. The scan found a neighbor there, so the test exercises the scan.
      - (b) In app A, each call answers 1, and the baseline `derived/` directory has no `.json` file after each call.
      - (c) The canonical bytes of each of the six records of app A equal those of app B.
      - (d) In the two apps, the candidate tenant's `derived/` directory does not exist or has no `.json` file. PC-D18 says that a candidate receipt sees an empty directory; this assertion proves it.

      If (c) or (d) fails, the scan result reaches a compared field or the two sides do not get equal scan values. That is a finding for the owner, PC-D18 is then not valid, and the task stops. Do not change the test to pass. Mutation check: in app A, remove the `clear_baseline_derived` call and see (b) fail.

- [ ] **Step 5: Run them to see them fail.** `TRACE_COMMONS_PG_TEST_DATABASE_URL=... cargo test -p trace-commons-server --bin trace-commons-ingest pipeline_compare_pg_tests -- --test-threads=1` on a fresh database. Expected: a compile failure.

- [ ] **Step 6: Implement the admission step.** For each side: `Probes::check_sent` the body; send it through the wrapper of this task (not `post_trace`, which panics on a transport error); `Probes::check` the response. Then read `SELECT status, privacy_risk, residual_risk_basis FROM trace_submissions WHERE tenant_id = $1 AND submission_id = $2` in a tenant transaction on the owner pool (`tenant_tx`, as `ledger_rows` does).
  - `privacy_basis`: the labels of the JSON array, sorted. A NULL column gives an empty list.
  - Baseline `admission`: `accepted -> Admit`, `quarantined -> Quarantine`, a receipt that is not 200 `-> Refused`, any other status `-> Other`.
  - Candidate `admission`: after the receipt, read the run with the harness's own `query_opt` in a tenant transaction (the same statement as `run_of_submission`, which panics when no run exists and thus cannot be used here); `admission_decision` is `admit`, `quarantine`, or `reject` (use the stored spelling; map it to the label). No run after a 200 receipt `-> Other`.
  - Each driver function returns `Result<_, &'static str>`. A transport error, a pool error, or a missing row that the step needs is an `Err` with a label (`compare_http_failed`, `compare_database_failed`, `compare_candidate_evidence_incomplete`), not a panic. Task 6 writes a partial report before it fails (plan review G7).

- [ ] **Step 7: Implement the finish step.**
  - Baseline, by `AlignmentAction`:
    - `ApproveBaseline`, `RejectBaseline`, `HashRuleBoth(label)`: claim the lease (`POST /v1/review/{submission_id}/lease`, body `{}`) and send the decision (`POST /v1/review/{submission_id}/decision`, body `{"decision": "approve" | "reject", "reason": "comparison_review"}`) with the reviewer token. Find one existing test of `review_decision_handler` in `tests.rs` and copy its request shape; name it in a comment.
    - Then, if the trace is now accepted and the action is not `SkipBaselineGate`: `POST /v1/workers/gate/evaluate`. `scored` is true when it answers 200.
    - `Stop`: no call.
    - Read all `trace_gate_decisions` rows of the submission. A scored trace has exactly one row; more than one is `Err("compare_baseline_scored_twice")` (each gate call makes a new decision and new index entries, so a second call would change the baseline index). The columns: `perplexity_passed`, `novelty_passed`, `perplexity_micros`, `tail_fraction_micros`, `peak_perplexity_micros`, `novelty_score_micros`, `peak_novelty_micros`, `chunk_count`, `total_chunk_count`, `chunks_capped`, `index_cardinality_at_scoring`, `credit_quality_micros`, `credit_quality_calibration_version`. `quality_passed` is `perplexity_passed`. Read `credit_quality_calibration_version`, `chunk_count`, and `total_chunk_count` as `i32` and widen them (the columns are `INT4`; a read as `i64` is an error).
    - `member_chunks`: `SELECT chunk_index FROM trace_gate_chunk_vector_entries WHERE tenant_id = $1 AND decision_id = $2 ORDER BY chunk_index`. `member` is that the list is not empty.
    - `terminal`: true when no call that the harness sent for this side failed. A review call or a gate call that does not answer 200 gives `terminal: false`, also for the action `None`: the gate route adds to the index before it writes the decision row, so a 500 can leave the baseline index changed with no row, and `alignment_lost` must then stop the run (PC-D20). Exception (PC-D19): a side whose admission is `Refused` or `Other` has `terminal: false`.
  - Candidate, by `AlignmentAction`:
    - The values of `pipeline_runs.state` are `pending`, `leased`, `retry`, `awaiting_review`, `complete`, and `failed` (migration V93). `rejected` and `withdrawn` are values of the status route, not of this column. A run that Admission or Review rejects ends as `complete`.
    - `ApproveCandidate`, `HashRuleBoth(label)`: first poll `pipeline_runs.state` (every 25 ms, PC-D14, inside one bound of 60 s for the whole candidate finish) until it is `awaiting_review`. The worker claims a quarantined run (`leased`), runs Review, finds no assessment, and parks it as `awaiting_review`; the review queue, the claim, and the assessment refuse a run that is `leased` (plan review G3). Then send the three pipeline review calls that `review_quarantine` sends (`pipeline_corpus_pg_tests.rs`, about line 829), with the reviewer token of the candidate tenant. The harness cannot call `review_quarantine` itself (it takes the corpus harness's own types), so send the calls here:
      1. `GET /v1/review/pipeline/quarantine`: 200, and the list holds the run. Take `admission_reason` from that item.
      2. `POST /v1/review/pipeline/runs/{run_id}/claim`: 200 with `lease_token`.
      3. `POST /v1/review/pipeline/runs/{run_id}/assessment` with `lease_token`, `recommendation` (`approve` or `reject`), `reason` (the admission reason), and `resolved_quarantine_reasons` (the admission reason for an approval, an empty list for a rejection): 200 with `assessment_id`.
      If one call does not answer 200, wait 25 ms and send the three calls again, inside the bound (a second claim by the same reviewer is accepted). If the review does not succeed inside the bound, answer `Err("compare_candidate_review_failed")`.
    - Order for `ApproveCandidate` and `HashRuleBoth(Approve)`: Task 6 calls `candidate_finish` before `baseline_finish` for these two actions, so the baseline index does not get a trace that the candidate review then fails to add. For every other action the baseline is first.
    - Then poll `pipeline_runs.state` every 25 ms, inside the same bound, until it is `complete` or `failed`. Stop at once for `awaiting_review` when the action has no candidate review (the state does not change without an assessment). `terminal` is true only for `complete`. A candidate whose admission is `Refused` or `Other` has `terminal: false`, and the harness makes no poll and no wait for it (PC-D19): a refused receipt has no run, and a poll for it waits the full 60 s.
    - Read the run again with the harness's own statement. If a Score outcome exists, parse `evidence` as `ScoreEvidence` and fill `GateValues` from its fields (`quality_passed`, `novelty_passed`, the five measured values, `chunk_count`, `total_chunk_count`, `chunks_capped`, `index_cardinality`, `credit_quality_micros` (a `u64`; convert with `i64::try_from`), `credit_quality_version` (an `i32`; widen)). A missing field in a Score outcome is `Err("compare_candidate_evidence_incomplete")`.
    - `member` is `run.index_membership == "included" && run.index_write_state == "complete"`, as `main` counts an include (`versioned_pipeline.rs`, about line 11926). `member_chunks`: if `member`, `service.load_index_command(&run, &evidence)` and the sorted `chunk` of each entry; else empty.
  - Both: `credit_events` from `SELECT event_type, points_delta FROM trace_credit_ledger WHERE tenant_id = $1 AND submission_id = $2 ORDER BY occurred_at`, with `Microcredits::from_credit_decimal`, as `ledger_rows` does.
  - Both: `Probes::check` the canonical bytes of the record.

- [ ] **Step 8: Run the tests.** The Step 5 command on a fresh database. Expected: PASS, or a recorded finding as Step 4 says. A finding does not block the task: the task is complete when each test either passes or states the exact difference, and the ledger lists each difference for the owner.

- [ ] **Step 9: The efficiency lens (common.md, review rule 3).** For one trace, count the statements and HTTP calls of each side and write them in the ledger. Each finish step uses one transaction for its reads. No statement runs inside the poll loop except the state read.

  Also record the number of driver tests that passed (15 and the spike).

- [ ] **Step 10: Commit**

```bash
git add crates/trace-commons-server/src/bin/trace_commons_ingest_internal
git commit -m "Drive one trace through the old gate path and the pipeline"
```

---

### Task 6: The run, the records file, and the report

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_compare_pg_tests.rs`

**Interfaces:**
- Consumes: Tasks 1 to 5. `PipelineCheckEmitter::emit_pass_from_env`, `write_atomically` (`pipeline_corpus_pg_tests.rs`), `ARTIFACT_ROOT_VAR`, `TEST_MASTER_KEY_VAR`.
- Produces: the ignored test `tests::pipeline_compare_pg_tests::pipeline_compare_run`, `CompareRunConfig::from_vars`, and the files that Task 7 reads.

- [ ] **Step 1: Write the failing configuration tests** (no database):
  1. `no_compare_variable_means_nothing_to_run`: `CompareRunConfig::from_vars(|_| None)` is `Ok(None)`.
  2. `every_required_variable_is_checked`: with one of bootstrap path, holdout path, manifest path, check id, report path, or records path missing, the result is `Err` with a label that names the missing one (`compare_bootstrap_path_missing`, `compare_holdout_path_missing`, `compare_manifest_path_missing`, `compare_check_id_missing`, `compare_report_path_missing`, `compare_records_path_missing`).
  3. `the_check_id_and_the_limit_are_validated`: a check id that is not `pipeline_comparison_local` or `pipeline_comparison_hf` is `Err("compare_check_id_invalid")`. A limit of `0`, a negative number, or text is `Err("compare_limit_invalid")`. A skew other than `baseline_quality_floor` is `Err("compare_skew_invalid")`.
  4. `the_manifest_gives_the_pin_digests`: a manifest file with the five digests, `sample_count`, and `source.with_events == true` gives the map of `ComparisonReportInput::pin_digests` and the `trace_count` (from `sample_count`). A manifest without `with_events: true` is `Err("compare_manifest_without_events")`.
  5. `an_empty_variable_is_a_missing_variable`: each required variable set to an empty text gives the same `..._missing` label as an unset variable.
  6. `an_unsafe_record_value_is_refused` (Review Focus 3): the function that checks a record before it is written (`check_record_values`) answers `Err("compare_record_value_unsafe")` for a `trace_hash` that is not a `sha256:` value of 64 hex characters, for a `privacy_risk` that is not `low`, `medium`, or `high`, and for a `privacy_basis` entry or a credit `event_type` that fails `is_safe_label`. It answers `Ok` for the Task 1 helper record. Privacy guard: mutation check (remove one of the four checks, see the test fail).

- [ ] **Step 2: Run them to see them fail.** `cargo test -p trace-commons-server --bin trace-commons-ingest pipeline_compare_pg_tests`. Expected: a compile failure.

- [ ] **Step 3: Implement `pipeline_compare_run`** (`#[tokio::test(flavor = "multi_thread", worker_threads = 4)]`, `#[ignore = "the implementation of \`pipeline.py compare\`: run it through that command"]`):
  1. Read the configuration. `Ok(None)` returns. When the configuration is `Some`, the database URL is required: `expect("compare_database_url_missing")`, as the corpus harness does (never a skip).
  1a. Before the first trace, compute the SHA-256 of the two corpus files (a streamed read, no parse) and require that they equal the manifest's `bootstrap_corpus_digest` and `holdout_corpus_digest` (`compare_corpus_digest_mismatch`). A run of many hours must not find a wrong corpus at its end.
  2. `calibrate_floors(bootstrap)`; `gate = compare_main_gate(floors)`.
  3. `start_compare_app(gate, artifact_root, master_key, CompareAppOptions { tenants: CompareTenants::run(), skew, accept_medium_risk: true })`; `wait_for_pipeline_ready`.
  4. Open the records file for writing (a `BufWriter`) and a SHA-256 of what is written.
  5. For each partition in order (`bootstrap`, then `holdout`), for each fixture of its reader, until the limit: set the probes; build the envelope and its body; `baseline_admission`, then `candidate_admission` (the baseline first, as the parity test requires); `alignment_action`; `baseline_finish`, then `candidate_finish` (the candidate first for `ApproveCandidate` and `HashRuleBoth(Approve)`, Task 5 Step 7); `check_record_values` and `Probes::check` for each record; `compare_records`; `summary.observe`; write the two records as two canonical JSON lines and flush the writer; `clear_baseline_derived` (PC-D18: after all calls and reads of this trace, before the next receipt); drop the fixture, the envelope, and the body. Then, if `alignment_lost(action, &baseline_record, &candidate_record)`: keep the pair's position and the label `COMPARISON_ALIGNMENT_LABEL`, and leave the loop (PC-D20). The pair is in the summary and in the records file before the loop ends.
     The loop body returns `Result<(), &'static str>`. On the first `Err`, leave the loop, keep the label, and go to step 7: the run then writes a report with `partial: true` for the pairs that it compared, and fails with the label at step 9 (plan review G7). A run cannot continue from a position, because the two indexes are in memory: a second start begins at trace 1.
     If the variable `TRACE_COMMONS_PIPELINE_COMPARE_TIMING_PATH` is set, also write one line for each trace to that file: the position, the chunk count, and the milliseconds of the baseline receipt, the baseline gate call, the candidate receipt, the candidate wait, and the reads. Write the calibration seconds as its first line. The file holds labels and numbers only. It is not part of the report or of a digest (plan review G12).
  6. After the loop: `finish()` each reader and assert that the two reader digests equal the manifest's also (`compare_corpus_digest_mismatch`): the reader then read the same bytes that step 1a checked. With a limit, read the rest of the files for the digest without building envelopes.
  7. Stop the app (`stop.send(())`, `join_within(server, 20, "compare app")`).
  8. Build the report (`partial` is true when the run compared fewer pairs than the trace count: a limit, or an early stop of step 5; `skew` is the skew value or `null`; `alignment_lost_position` is the position that step 5 kept, or `null`). `Probes::check` its bytes. Write it with `write_atomically`.
  9. If step 5 kept an error label (a driver error, or `COMPARISON_ALIGNMENT_LABEL`), panic with it now. Then assert `summary.refused_total() == 0` with the message `COMPARISON_REFUSED_LABEL` (PC-D19; for each run, also a partial one). Then assert `summary.unexplained_total() == 0` with the message `COMPARISON_UNEXPLAINED_LABEL`. The order is fixed: a pair that loses the alignment and a refused pair are also `unexplained`, and the more exact label must win. Then, for a run that is not partial, assert `summary.branch_gaps().is_empty()` with `COMPARISON_BRANCH_LABEL`.
  10. For a run that is not partial and has no skew: `PipelineCheckEmitter::emit_pass_from_env(&check_id, Some(&app.package), json!({"traces": compared_count, "equal": equal_count, "permitted": permitted_total, "unexplained": 0, "records_hash": records_digest, "report_hash": sha256_bytes(&report_bytes)}))`.

  The report is written before the checks of step 9, so a failed run leaves a report that names its differences.

- [ ] **Step 4: Run the four scenarios by hand.** Export the two local pins (the Task 3 command with each pin's values; the output directories are `.local/pipeline/compare-local` and `.local/pipeline/compare-risk`). For each run: a fresh database `admission_test_cmp_run`, a fresh artifact directory, and the variables of the Interfaces section. The check result variables are not set (the emitter is then a no-op). Use absolute paths under the worktree's `.local/`.

```bash
TRACE_COMMONS_PG_TEST_DATABASE_URL=postgres://trace@127.0.0.1:55434/admission_test_cmp_run \
TRACE_COMMONS_PIPELINE_COMPARE_BOOTSTRAP_PATH=$PWD/.local/pipeline/compare-local/bootstrap-compare.jsonl \
TRACE_COMMONS_PIPELINE_COMPARE_HOLDOUT_PATH=$PWD/.local/pipeline/compare-local/holdout-compare.jsonl \
TRACE_COMMONS_PIPELINE_COMPARE_MANIFEST_PATH=$PWD/.local/pipeline/compare-local/source-manifest.json \
TRACE_COMMONS_PIPELINE_COMPARE_CHECK_ID=pipeline_comparison_local \
TRACE_COMMONS_PIPELINE_COMPARE_REPORT_PATH=$PWD/.local/pipeline/compare-local/report.json \
TRACE_COMMONS_PIPELINE_COMPARE_RECORDS_PATH=$PWD/.local/pipeline/compare-local/comparison-records.jsonl \
cargo test -p trace-commons-server --bin trace-commons-ingest -- --ignored --exact tests::pipeline_compare_pg_tests::pipeline_compare_run
```

  Expected results (write each report's counts in the ledger):
  1. **The local pin:** PASS. `trace_count == 10`, `unexplained_total == 0`, `branch_gaps == []`. The baseline distribution has `quality_failed >= 1`, `novelty_failed >= 1`, `quality_passed >= 1`, `chunks_capped == 0`, and one trace with more than one chunk in the records.
  2. **The risk pin:** PASS. The report has `unexplained_total == 0`, `unexplained_counts == {}`, `permitted_counts == {"medium_risk_privacy_review": 1, "high_risk_admission_reject": 1}`, and `permitted_total == 2` (the declared medium trace by PC-D22 and the declared high trace by PC-D27; Review Focus 2: no novelty field is in the counts). Before PC-D27 the scenario failed with one unexplained `admission` difference, and before PC-D22 with two. The two risk traces are the first two of the holdout partition (`s04h`, `s04m`), and the two traces after them (`s05`, `s06`) are `Equal` with `index_cardinality > 0` in the records file. A declared-risk trace is metadata only, so it is improbable that it becomes a member of an index; the test in which an aligned trace is a member is Task 5 Step 4 test 13.
  3. **The local pin with `TRACE_COMMONS_PIPELINE_COMPARE_SKEW=baseline_quality_floor`:** FAIL with `comparison_alignment_lost`. `unexplained_counts` contains `quality_passed` (Review Focus 1). With this skew, no baseline trace passes the quality gate, so no baseline trace is a member. The run stops at the first trace that is a member on the candidate (PC-D20): `alignment_lost_position` is that position, and `partial` is true if traces remain. If the run fails with `comparison_has_unexplained_differences` and no `alignment_lost_position`, the stop does not work: that is a defect of the harness, not a finding.
  4. **The local pin again:** the `report_digest` and the `records_digest` equal those of run 1 (Review Focus 5).

  If run 1 has a branch gap, change the fixture text of Task 3 (and the pin digests) until each branch has evidence. Do not change the percentile.
  If run 1 has an unexplained difference, it is a finding: record the fields and the counts, tell the owner, and stop this task. The CI step of Task 7 cannot pass until the owner decides.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_compare_pg_tests.rs crates/trace-commons-server/tests/fixtures/pipeline-compare-jsonl
git commit -m "Run a pinned sample through both paths and report the differences"
```

---

### Task 7: `pipeline.py compare`

**Files:**
- Create: `scripts/operator/pipeline_tooling/comparison.py`
- Modify: `scripts/operator/pipeline.py`
- Modify: `scripts/operator/test_pipeline_tooling.py`

**Interfaces:**
- Consumes: Task 3's export flags and pins; Task 6's harness variables, report, and check ids; `load_pin`, `safe_report_value`, `HF_CACHE_DIR` (`corpus.py`); `Environment`, `Run`, `child_environment`, `run_child`; `cargo_test`; `load_results`, `require_current_pass_results`, `validate_evidence`, `canonical`.
- Produces:

```python
# pipeline_tooling/comparison.py
REPORT_SCHEMA = "trace_commons.pipeline_comparison_report.v1"
CHECK_IDS = frozenset({"pipeline_comparison_local", "pipeline_comparison_hf"})
BLOCKERS = (*LOCAL_BLOCKERS, "deterministic_privacy_only", "baseline_derived_scan_removed",  # PC-D18
            "duplicate_short_circuits_not_compared", "review_start_privacy_pass_not_compared")  # PC-D24, PC-D25
UNEXPLAINED_LIST_LIMIT = 1000
UNEXPLAINED_LABEL = "comparison_has_unexplained_differences"
BRANCH_LABEL = "comparison_gate_branch_not_exercised"
REFUSED_LABEL = "comparison_receipt_refused"      # PC-D19
ALIGNMENT_LABEL = "comparison_alignment_lost"     # PC-D20

def export_compare_corpus(run, pin_path, env, *, name, local_dir=None, release=False):
    """Runs the export with --with-events into run.run_dir / "compare" / name,
    with the step label compare_export_<name>. With release=True, the cargo
    command has --release (PC-D21). Returns
    (bootstrap-compare.jsonl, holdout-compare.jsonl, source-manifest.json)."""

def validate_comparison_report(report):
    """Raises ToolingError("comparison_report_malformed") or a more exact label."""

def markdown(report): ...

# pipeline.py
COMPARE_HARNESS = "tests::pipeline_compare_pg_tests::pipeline_compare_run"
COMPARE_LOCAL_PIN = ROOT / "crates/trace-commons-server/tests/fixtures/pipeline-compare-jsonl/pin-local.json"
COMPARE_RISK_PIN = ROOT / "crates/trace-commons-server/tests/fixtures/pipeline-compare-jsonl/pin-local-risk.json"
def run_compare(args, run): ...
```

  Command line: `pipeline.py compare (--corpus PIN | --self-test) [--limit N] [--postgres-admin-url URL]`. `--corpus` and `--self-test` exclude each other, and one is required (`compare_corpus_or_self_test_required`, `compare_corpus_and_self_test_conflict`). `--limit` is refused with `--self-test` (`compare_self_test_takes_no_option`). `--limit` must be a positive integer (`compare_limit_invalid`).

  `compare` has no `--archive` option (owner, question 8): the lab catalog accepts only the corpus report and the qualification report, so an archive step fails with `unsupported_report_schema` after the full run (plan review G8). The committed report and the check result are the durable record.

  `compare` has no build option (PC-D21). `--corpus` runs the harness and the export with Cargo's optimized build: the cargo arguments of the harness are `(*INGEST_TEST_ARGS, "--release")`, and the export gets `release=True`. `--self-test` uses the debug build for the two.

- [ ] **Step 1: Write the failing self-tests** in `test_pipeline_tooling.py`. Add a helper `_comparison_report(check_id, **overrides)` that builds a valid report (the object of the Interfaces section, signed with `results.canonical` as `_corpus_report` does) and a helper `_write_compare_outputs(env, *, unexplained=None, branch_gaps=(), emit=True)` that writes the report, a records file, and (for a pass) the check result and evidence, as `_write_harness_outputs` does for the corpus harness. New class `ComparisonReportValidationTests`:
  1. `test_a_valid_report_passes`.
  2. `test_comparison_report_refuses_private_content` (Review Focus 3): a report with a key `input`, with a raw UUID value, with a value that starts with `token-`, and with a float each raise `ToolingError`.
  3. `test_counts_must_agree`: `equal_count + permitted_total + unexplained_total != compared_count` raises `comparison_count_mismatch`; `len(unexplained) != min(unexplained_total, 1000)` raises it; `compared_count > trace_count` raises it; `partial` that does not equal `compared_count < trace_count` raises `comparison_partial_mismatch`. An `alignment_lost_position` that is not `null` and not a non-negative integer raises `comparison_report_malformed`.
  4. `test_the_digest_is_checked`: one changed count without a new digest raises `report_digest_mismatch`.
  5. `test_the_blockers_and_the_scope_are_required`: a missing `deterministic_privacy_only`, a missing `baseline_derived_scan_removed` (PC-D18), a missing `duplicate_short_circuits_not_compared` (PC-D24), a missing `review_start_privacy_pass_not_compared` (PC-D25), `production_ready: true`, or a check id outside `CHECK_IDS` each raise.
  6. `test_a_malformed_report_gives_a_label`: a missing field raises `comparison_report_malformed`, never a `KeyError`.
     Added on 2026-10-10 (PC-D26): `test_each_side_counts_each_compared_pair` (a side whose admission counts do not add up to `compared_count`, or whose gate counts do not add up to `scored`, raises `comparison_count_mismatch`; a quarantined and approved trace may be scored), `test_the_excluded_rules_are_the_closed_list` (a list that is not exactly the three rules raises `comparison_report_malformed`), and `test_the_comparison_blockers_agree_with_the_harness` (the Rust `COMPARISON_BLOCKERS` equals `comparison.BLOCKERS`).

  New class `CompareCommandTests(_CorpusRunCase)` (patch `pipeline.cargo_test` and `pipeline.export_compare_corpus`):
  7. `test_compare_passes_only_the_expected_variables`: `compare --corpus <local pin>`; one cargo call with `COMPARE_HARNESS`, `exact`, `ignored`; the `TRACE_COMMONS_` variables are exactly the set of the Interfaces section without `LIMIT` and `SKEW`; the check id is `pipeline_comparison_local`; exit code 0; the output has `PipelineCompareOK:` with `traces=`, `equal=`, `permitted=`, `unexplained=0`, `seconds=`, and `run=`.
  8. `test_a_pin_without_events_is_refused`: a pin without `with_events: true` exits 1 with `comparison_pin_without_events` before any cargo or Docker call. A run that is not partial refuses a pin that lacks one of the five digests (`comparison_pin_digest_missing`): the declared risks and `trace_file_for` change only the corpus files, so only the two corpus digests cover them.
  9. `test_a_network_pin_uses_the_hf_check_id`: a pin without `local_jsonl_dir` gives `pipeline_comparison_hf`.
  10. `test_limit_makes_a_partial_run`: `--limit 3`; the variable `TRACE_COMMONS_PIPELINE_COMPARE_LIMIT` is `3`; a partial report passes without a check result; the output has `partial=true`.
  11. `test_unexplained_differences_fail_with_their_label`: the fake harness writes a report with `unexplained_total: 2` and raises `StepFailed`; the exit code is not 0; stderr has `PipelineFailure: comparison_has_unexplained_differences`; stdout has `PipelineCompareReport: unexplained=2 alignment_lost=none report=...`; the report is under `LOCAL_DIR`.
  12. `test_a_branch_gap_fails_with_its_label`: a report with `branch_gaps: ["novelty_passed_false"]` and a failed step gives `comparison_gate_branch_not_exercised`.
  13. `test_a_failed_step_without_a_report_keeps_the_step_failure`: the fake harness raises `StepFailed` and writes nothing; stderr has `step_failed` with `exit=` and `log=`. `test_a_pass_without_a_report_fails`: the fake harness passes and writes no report; the failure is `comparison_report_missing` (spec section 13). `test_a_report_that_is_not_json_fails`: the failure is `comparison_report_malformed`. `test_a_report_with_a_skew_gets_no_check_result`: a passing report with `skew: "baseline_quality_floor"` and a check result is refused with `comparison_skew_with_check_result`.
  14. `test_a_pass_needs_a_current_check_result`: the fake harness writes a passing report and no result; the failure is the label that `require_current_pass_results` gives for a missing result.
  15. `test_the_database_guard_applies`: the fake `_invoke` answers `3` for the committed transactions; the failure is `database_check_executed_nothing:compare_run`.
  16. `test_self_test_runs_four_scenarios`: `compare --self-test`; four cargo calls in one environment with the steps `compare_self_pass`, `compare_self_risk`, `compare_self_skew`, `compare_self_repeat`; the third has `TRACE_COMMONS_PIPELINE_COMPARE_SKEW=baseline_quality_floor`; the fake harness gives a pass, a pass with `unexplained_counts == {}` and `permitted_counts == {"medium_risk_privacy_review": 1, "high_risk_admission_reject": 1}` (before PC-D27: a failure with `unexplained_counts == {"admission": 1}`), a report whose `unexplained_counts` contains `quality_passed` and whose `alignment_lost_position` is a number (PC-D20), and a pass with the digests of the first; no cargo call has `--release`, and each `export_compare_corpus` call has `release=False` (PC-D21); `export_compare_corpus` is called two times, with `name="local"` and `name="risk"`, and scenario 2 gets corpus paths that differ from the paths of scenarios 1, 3, and 4; no call has a `TRACE_COMMONS_PIPELINE_CHECK_` variable; exit code 0; the output has `PipelineCompareSelfTestOK: scenarios=4`.
  17. `test_self_test_fails_when_an_expected_failure_passes`: the second scenario gives another result than the two permitted pairs (no pair, one of the two, or a rule counted twice): `compare_self_test_risk_fields`; a report with an unexplained difference, an alignment loss, or a refused receipt gives the label of that failure. The third passes: `compare_self_test_skew_passed`. The third fails with `alignment_lost_position: null`, or without `quality_passed` in `unexplained_counts`: `compare_self_test_skew_fields` (PC-D20: the CI step then proves the stop). The fourth has another `report_digest`: `compare_self_test_not_deterministic`.
  18. `test_compare_options_are_validated`: the three option labels of the Interfaces section, and `compare_limit_invalid` for `--limit 0`. The parser refuses `compare --archive` and `compare --release` (exit code 2): the two options do not exist.
  19. `test_an_alignment_loss_fails_with_its_label` (PC-D20): the fake harness writes a report with `alignment_lost_position: 4`, `partial: true`, and `unexplained_total: 1`, and raises `StepFailed`. stderr has `PipelineFailure: comparison_alignment_lost`, and the `PipelineCompareReport:` line has `alignment_lost=4`. A harness that passes with such a report is refused with the same label.
  20. `test_a_refused_receipt_fails_with_its_label` (PC-D19): the fake harness writes a report with `distribution.candidate.refused: 1` and `unexplained_total: 1`, and raises `StepFailed`. stderr has `PipelineFailure: comparison_receipt_refused`. A harness that passes with such a report and a check result is refused with the same label. A report with an alignment loss and a refused count gives `comparison_alignment_lost`: the order is alignment, refusal, unexplained, branch.
  21. `test_a_corpus_run_uses_the_optimized_build` (PC-D21): `compare --corpus <local pin>`; the cargo arguments of the harness call hold `--release`, and `export_compare_corpus` gets `release=True`. A unit test of `export_compare_corpus` with a patched `run_child`: with `release=True` the cargo command holds `--release` before its `--` separator, and with the default it has no `--release`.

- [ ] **Step 2: Run them to see them fail.** `python3 scripts/operator/test_pipeline_tooling.py`. Expected: errors (`comparison` does not exist).

- [ ] **Step 3: Implement `comparison.py`.**
  - `export_compare_corpus`: the function builds its own cargo command (`export_hf_corpus` has a fixed step label and a fixed directory, so do not call it). With `release=True`, `--release` goes before the `--` separator of that command (PC-D21). The command is the command of `export_hf_corpus` plus `--with-events`, plus one `--session-name` for each name in the pin's `session_names`, plus one `--declared-privacy-risk NAME=RISK` for each entry of `declared_privacy_risk`. The two list fields are accepted only when the pin has `local_jsonl_dir` (`comparison_pin_field_needs_local_dir`). The output directory is `run.run_dir / "compare" / name` and the step label is `compare_export_<name>` (the export writes fixed file names, and one step label has one log file, so two exports of one run need two names; plan review G9). After the run: each of the pin's five digest fields that is present must equal the manifest's (`hf_<field>_mismatch`; `export_hf_corpus` compares four today, and `configuration_digest` is the new fifth); `manifest["source"]["with_events"] is True`; `contains_raw_trace_text is False`. Do not call `validate_hf_manifest`: it requires an exact key set for `source`, and a manifest with `with_events` fails it.
  - `validate_comparison_report`: the checks that the Step 1 tests name, in one function wrapped as `validate_report` is (`KeyError`, `TypeError`, `AttributeError`, `IndexError` become `comparison_report_malformed`). Use `safe_report_value(report) == report`.
  - `markdown`: the title `# Pipeline comparison report`, the check id, the bundle, the pin digests, the floors, one table of the counts, one table of the distribution for the two sides, the list of `unexplained_counts`, the first 20 entries of `unexplained`, the blockers, and one line with `alignment_lost_position` when it is not `null`.

- [ ] **Step 4: Implement the command in `pipeline.py`.**

```python
def _compare_once(run, environment, exported, check_id, step, *, limit=None, skew=None, release=False):
    """One harness run in its own scenario. Returns (report or None, StepFailed or None, report_bytes)."""

def run_compare(args, run): ...
```

  - `_compare_once`: `scenario = environment.scenario(step)`; the variables of the Interfaces section (the report at `run.run_dir / f"{step}-report.json"`, the records at `run.run_dir / f"{step}-records.jsonl"`, a new `secrets.token_hex(32)` key); `cargo_test(run, step, cargo_args, COMPARE_HARNESS, child_environment(extra), exact=True, ignored=True)`, where `cargo_args` is `(*INGEST_TEST_ARGS, "--release")` with `release=True` and `INGEST_TEST_ARGS` without it (PC-D21; `cargo_test` puts the arguments in its list command and in its run command, so the two use one profile). Catch `StepFailed` and keep it. Then `scenario.committed_transactions(scenario.pilot_database) >= 5` or `database_check_executed_nothing:<step>` (only when the step did not fail). Read and validate the report if the file exists. A file that is not JSON is `comparison_report_malformed`.
  - A normal run (`--corpus`): load the pin; require `with_events`; export with `release=True`; one `Environment`; `_compare_once(..., step="compare_run", limit=args.limit, release=True)`.
    - A failed step with a valid report: write the report and its Markdown to the local report path (and `.md`), print `PipelineCompareReport: unexplained=<n> alignment_lost=<position or none> report=<path>`, and raise the first of these that applies: `ToolingError(ALIGNMENT_LABEL)` if `alignment_lost_position` is not `null`; `ToolingError(REFUSED_LABEL)` if a `refused` count of the distribution is not zero; `ToolingError(UNEXPLAINED_LABEL)` if `unexplained_total > 0`; `ToolingError(BRANCH_LABEL)` if `branch_gaps` is not empty; else the step failure again.
    - A failed step without a valid report: raise the step failure.
    - A pass with no report file: raise `ToolingError("comparison_report_missing")`.
    - A pass: require `report["alignment_lost_position"] is None` (`ALIGNMENT_LABEL`), then that the two `refused` counts are zero (`REFUSED_LABEL`), then `report["unexplained_total"] == 0`; a report with a skew and a check result is refused (`comparison_skew_with_check_result`); for a report that is not partial, require the current pass result for `check_id` (`CheckSpec(check_id, digests_required=True)`), require that its three digests equal the report's, and require the evidence to equal `{"traces", "equal", "permitted", "unexplained", "records_hash", "report_hash"}` with the report's values and the SHA-256 of the two files. Write the local report: copy the bytes of the harness's file (the evidence holds `report_hash` of those bytes; do not serialize the report again). `run.require_code_revision_unchanged()`. The command writes nothing to the lab catalog (question 8).
    - Print `PipelineCompareOK: traces=<compared_count> equal=<n> permitted=<n> unexplained=0 partial=<true|false> seconds=<whole seconds> run=<run_id> report=<path>`. The run id is a label (`q` and 8 hex characters); Task 11 needs it to find the check result. The seconds come from `time.monotonic()` around `_compare_once`. They are printed only; they are not in the report.
  - The local report path is `LOCAL_DIR / f"pipeline-comparison-{check_id}{'-partial' if partial else ''}.json"` (and `.md`). One name for each check id and for a partial run, so a `--limit` run or a local-pin run does not replace the report of the full run (plan review G28).
  - `--self-test`: export the local pin (`name="local"`) and the risk pin (`name="risk"`); one `Environment`; four `_compare_once` calls (`compare_self_pass`, `compare_self_risk`, `compare_self_skew` with `skew="baseline_quality_floor"`, `compare_self_repeat`), with the requirements of Step 1 tests 16 and 17. The check id is `pipeline_comparison_local` for each. Do not set the three check result variables in a self-test scenario: the emitter is then a no-op (it refuses a second result for one check id in one directory), and each scenario is judged by its report alone. Give `_compare_once` a parameter `emit=True` for this.
  - Register the subparser in `build_parser` and add the command to the module's doc comment.

- [ ] **Step 5: Run the self-tests.** `python3 scripts/operator/test_pipeline_tooling.py`. Expected: PASS (the tests that exist at `BASE_CMP`, 100 on `main` at `0e5dffe6b`, and the new tests of Step 1).

- [ ] **Step 6: Run the real command.** Give the time first: about 3 minutes for the self-test with a warm debug build. The second command uses the optimized build (PC-D21), and its first run makes that build: one more full build of the ingest test binary and of the export binary. Tell the owner before it starts, and run it in the background.

```bash
python3 scripts/operator/pipeline.py compare --self-test
python3 scripts/operator/pipeline.py compare --corpus crates/trace-commons-server/tests/fixtures/pipeline-compare-jsonl/pin-local.json
```

  Expected: `PipelineCompareSelfTestOK: scenarios=4`; `PipelineCompareOK: traces=10 ... unexplained=0 partial=false`. `pipeline.py qualify` is a gate check (Task 10 Step 2), not a check of this task (common.md's three levels). Then run the checks for the package of Tasks 4 to 7 (clippy and `license_boundary`, Global Constraints).

- [ ] **Step 7: Commit**

```bash
git add scripts/operator
git commit -m "Add pipeline.py compare"
```

---

### Task 8: The network pin, CI, and the runbook

**Files:**
- Create: `docs/superpowers/specs/fixtures/versioned-pipeline-comparison-hf-pin-v1.json`
- Modify: `.github/workflows/ci.yml`
- Create: `docs/operator/pipeline-comparison.md`
- Modify: `docs/operator/README.md`, `docs/operator/pipeline-lab.md`, `docs/operator/pipeline-qualification.md`, `CLAUDE.md`

**Interfaces:**
- Consumes: Task 3's export; Task 7's command.
- Produces: the pin that Task 9 and Task 11 run.

- [ ] **Step 1: Make the pin.** Tell the owner the time first: about 9,500 files are not in the cache, and the estimate at 16 parallel downloads is 35 to 60 minutes. Run it in the background.

```bash
cargo run -q -p trace-commons-server --bin trace-commons-pipeline-corpus-export -- \
  --repository jedisct1/security-audits --revision 6d527ff0081eec6704c2a4f00e1ef8d308ae7366 \
  --split train --translator swival --with-events \
  --cache-dir "$PWD/.local/pipeline/hf-cache" --output-dir "$PWD/.local/pipeline/compare-pin-draft-hf" \
  --bootstrap-count 1000 --holdout-count 9000 --min-words 200 --max-words 20000
```

  Write the pin from the manifest: the nine source values (`expected_instrument_count: 1`), `with_events: true`, and the five digests. It has no `local_jsonl_dir`. Run the export a second time with `--expected-source-digest` and `--expected-order-digest`: it must pass from the cache in a few minutes, and the two corpus digests must be equal to the first run's (a difference means that the export is not deterministic: stop and find the cause). Record in the ledger: the number of files that the export read, the size of the two JSONL files, and the peak memory of the export (`/usr/bin/time -v`, on a second run that does not build). Run with `HF_ENDPOINT` and `HF_HOME` not set: they reach this direct `cargo run`, but they do not reach a child of `pipeline.py`. If the export fails on a download, run the same command again; it continues from the cache.

- [ ] **Step 2: The CI step.** In the job `pipeline qualification and restore`, immediately after the step `pipeline qualify` and before the upload step (which has `if: failure()`):

```yaml
      - name: pipeline compare self-test
        run: python3 scripts/operator/pipeline.py compare --self-test
```

  Do not change the job's `timeout-minutes`, its cache key, the upload step, or the trim step. The job stays not required.

- [ ] **Step 3: The runbook** `docs/operator/pipeline-comparison.md`, in the form of `pipeline-lab.md`. Sections: a status line (what the tool is, and that it is local evidence with the reference scorer); "Run a comparison" (the command, the two pins, `--limit`, `--self-test`); "What is compared" (the field table of spec section 10.1 and the three exclusions); "Alignment" (the table of spec section 8.2); "The floors" (the calibration); "The report" (each field of the report); "Limits" (reference scorers, no classifier and no backstop, the gate route and not the score driver, serial order, unqualified test routing (PC-D15), the harness removes the submit rate limit for its own principals (PC-D16), the harness removes the baseline tenant's derived files after each trace, so the old path's duplicate scan always sees an empty tenant (PC-D18, the blocker `baseline_derived_scan_removed`), the baseline has the gate values of `CompatibilityBundleConfig::local_reference()` with the derived floors (the chunk values, the insert threshold, and `top_k` equal a deployed server's defaults (PC-D23)) and not the configuration of a deployed server ("How the baseline is configured and identified" in the plan: write its points in the runbook's words), `compare --corpus` uses Cargo's optimized build and `--self-test` the debug build (PC-D21), a run stops at the first pair after which the two indexes can differ (PC-D20), and a run cannot continue from a position); "Failure labels" (each new label of this plan with its cause); "The full run" (the download time, the disk space, where the committed report is, and which build profile made it; the size of one run directory, and the sentence "after a run, remove the corpus files under `runs/<id>/compare/` and the directory `runs/<id>/artifacts/`": each run leaves a full copy of the trace text and the encrypted artifacts under `.local/pipeline/runs/`). The run time is not known at this task: Task 9 adds that sentence in its own commit, before the full run starts. In "The report", add these sentences (owner, question 12): "`trace_hash` and `position` identify the dataset file to a person who has the pin and the public dataset. This is intended, so that a person can reproduce a difference. Do not use the comparison on a corpus that is not public." In "The report", also add three sentences on results that a second run can fail to repeat (plan review G34): the two credit-quality values depend on the run date, because each side selects the calibration era from its own clock; a difference in `privacy_basis` that a second run does not show points to the entropy sum, which adds `f64` terms in `HashMap` order; compare digests only between runs on one platform, because the reference scorer uses `f64::ln` and `f64::exp`. Every sentence must be true of the code at the tip: check each label and each flag against the code.

- [ ] **Step 4: The index.** In `docs/operator/README.md`, add the table row `| Comparing the old gate path with the versioned pipeline | [\`./pipeline-comparison.md\`](./pipeline-comparison.md) |` after the `pipeline-qualification` row, and the list entry in the same form as its neighbors. In `pipeline-lab.md`, add one sentence under "The HF pin" that names the network pin and the new runbook. In `docs/operator/pipeline-qualification.md` and in `CLAUDE.md`, add one sentence where each describes the CI job `pipeline qualification and restore`: the job also runs `pipeline.py compare --self-test`.

- [ ] **Step 5: Check.** `python3 scripts/operator/test_pipeline_tooling.py`; `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/ci.yml'))"` if PyYAML is installed, else read the diff of the workflow. Every relative link in the new runbook opens a file that exists.

- [ ] **Step 6: Commit**

```bash
git add docs/superpowers/specs/fixtures/versioned-pipeline-comparison-hf-pin-v1.json .github/workflows/ci.yml docs/operator CLAUDE.md
git commit -m "Pin the comparison sample and document the command"
```

---

### Task 9: The 100-trace run (no commit; ends with two owner decisions)

- [ ] **Step 1: Run it.**

```bash
/usr/bin/time -v python3 scripts/operator/pipeline.py compare \
  --corpus docs/superpowers/specs/fixtures/versioned-pipeline-comparison-hf-pin-v1.json --limit 100
```

  The calibration reads all 1,000 bootstrap traces. Run it in the background. The command uses the optimized build (PC-D21), which is the build of the full run, so the measured times apply to Task 11. If Task 7 Step 6 did not make that build in this worktree, the first run makes it: run the command a second time for the time and memory values, because `/usr/bin/time -v` includes `rustc` when the command builds.

  Before the start (plan review G13): commit all work. `pipeline.py` hashes the content of each tracked and each untracked, not ignored file at the start and at the end, and a difference fails the command with `code_revision_changed` after the harness passed. Until the command ends, do not create, change, or delete a file in the worktree outside `.local/` and `target/`. A commit of unchanged files is safe; an edit is not. The ledger is in the main checkout and is safe. Record `git rev-parse HEAD` and the output of `pipeline.py revision` in the ledger before the start.

- [ ] **Step 2: Record in the ledger:**
  - the seconds for the calibration (from the step log's test duration minus the loop; add two `eprintln!` lines with label-only text and a duration to the harness if the log does not show it) and for the 100 traces;
  - the peak memory of the test process;
  - the derived floors;
  - the counts: `equal_count`, `unexplained_counts`, the distribution of the two sides, and the number of traces with more than one chunk;
  - the estimate for 10,000 traces: `seconds for 100 traces * 100`, plus the growth of the index scan (the index holds about 51,000 vectors at the end; measure the time of the last 10 traces against the first 10 to see the trend). PC-D18 removes the derived-record scan, which was the largest growth term. These growth terms stay: the audit log read, the credit event file, and the two index scans. The read of the dedup signal rows is in a deployment and not in a comparison run (no gate-driver pool). The plan review estimates them at 25 to 70 minutes in total for 10,000 traces in an optimized build, not measured.

- [ ] **Step 3: Report to the owner and stop.** Give:
  1. Each unexplained field with its count and one trace hash. For the two rows of spec section 8.3: how many of the 100 traces are in each row. The owner decides for each: a named rule (then a small PR to `main` for the compatibility mapping, and a rule in `ComparisonRule` with its test) or a defect (then an issue for the PR that owns the code).
  2. The time estimate, from the optimized build (PC-D21). The owner decides: run as it is, or divide the traces between tenant pairs (a design change: it needs its own short plan).
  3. The finding about the old path's receipt cost ("Items for other PRs"). The run does not measure this cost, because PC-D18 removes the scan's input. Give the plan review's estimate (5 to 19 hours for 10,000 traces in an optimized build) and say that it is not measured. This item needs no decision here: the owner ruled on 2026-10-08 that the code must change. Put the estimate and the code references in the issue for that correction.

  Do not start Task 11 before both decisions are in the ledger.

---

### Task 10: Gate (no commit)

Owner rule (project memory "Publish, then let CI verify", common.md "Publish"): no multi-hour local gates.

- [ ] **Step 1:** Every check in Global Constraints, from the worktree root, at the tip only.
- [ ] **Step 2:** The focused suites, each one time on a fresh database: `cargo test -p trace-commons-server --lib versioned_pipeline_comparison`; `cargo test -p trace-commons-server --bin trace-commons-pipeline-corpus-export`; `cargo test -p trace-commons-server --bin trace-commons-pilot-bootstrap`; the `pipeline_compare_pg_tests` module with the database URL; `python3 scripts/operator/test_pipeline_tooling.py`; `python3 scripts/operator/pipeline.py compare --self-test`; `python3 scripts/operator/pipeline.py qualify` one time.
- [ ] **Step 3:** The final review, with separate lenses (common.md, review rule 5): correctness (the comparison function and the alignment against the spec, row by row); `main` integration (what the harness changes in shared test helpers, and that `run` and `qualify` output is unchanged); efficiency (memory and statements for each trace; the Task 5 Step 9 list); determinism (each source of a random or time value in the envelope, the records, and the report); privacy (each value that reaches a record, the report, a log line, or stdout). Each lens gives candidate findings; a separate step verifies each one. Triage by review rule 6.
- [ ] **Step 4:** Leave to the upstream CI when the branch is published: `cargo test --workspace`, the feature checks, the `cargo deny` runs, the MSRV floor, and the whole ingest bin against PostgreSQL.
- [ ] **Step 5:** Record the candidate revision, every executed check, the merges of `main` that the branch took, and the open findings in the ledger. Write the PR description draft in `replies.md` (scope, tests, a link to the spec, the limits, the licensing attestation from the PR template). Do not push. Ask the owner.

---

### Task 11: The full run and the committed report (after the two decisions of Task 9)

**Files:**
- Create: `docs/superpowers/reports/<date>-pipeline-comparison-10k.json` and `.md`
- Create: `docs/superpowers/reports/<date>-pipeline-comparison-10k.result.json` and `.evidence.json` (owner, question 11)

- [ ] **Step 1:** Apply the owner's decisions of Task 9 (a new rule with its unit test and its mapping PR, or nothing). Run Task 10 Step 1 and Step 2 again if code changed.
- [ ] **Step 2:** Tell the owner the time estimate from Task 9. Apply the rule of Task 9 Step 1 first: commit all work, change no file in the worktree outside `.local/` and `target/` until the command ends, and record `git rev-parse HEAD` and `pipeline.py revision` in the ledger. Run in the background:

```bash
python3 scripts/operator/pipeline.py compare \
  --corpus docs/superpowers/specs/fixtures/versioned-pipeline-comparison-hf-pin-v1.json
```

- [ ] **Step 3:** If it passes: copy `.local/pipeline-comparison-pipeline_comparison_hf.json` and its `.md` (the local report path of Task 7 Step 4) to `docs/superpowers/reports/` with the run's date. First check the file that you copy (plan review G28): `check_id == "pipeline_comparison_hf"`, `partial == false`, `skew == null`, `alignment_lost_position == null`, the two `refused` counts are zero, `trace_count == 10000`, `unexplained_total == 0`, the five pin digests equal those of the committed pin, and the SHA-256 of the file equals `report_hash` in the run's evidence file.

  Also copy the run's check result and its evidence file (`pipeline_comparison_hf.result.json` and `pipeline_comparison_hf.evidence.json`, from `.local/pipeline/runs/<run_id>/results/`; the `PipelineCompareOK` line prints `run=<run_id>`) to the two names of the Files list (owner, question 11). The report has no code revision; these two files connect it to one. Before the copy, require: `code_revision_hash` of the result equals the output of `pipeline.py revision` that Step 2 wrote in the ledger; the three digests of the result equal the report's; and each value in the two files is a label, a number, a `sha256:` value, or the ISO time `observed_at` of the result (read the two files; they must hold no path, no UUID, and no tenant name). The result has `safe_blockers: []`; the eight blockers are in the report, and that is correct. Do not edit the two copies.

  Add six lines at the top of the `.md` copy: the branch revision, the pin path, the run time, the target triple (`rustc -vV`, the line `host`), the build profile (`release`, PC-D21), and the sentence "Local evidence with the reference scorer and embedder; not a production promotion." Below these lines, add a section "Findings outside the compared fields" to the `.md` copy. It states the finding about the old path's receipt cost ("Items for other PRs") in four sentences or fewer: what the old receipt does, that the harness removes the baseline tenant's derived files after each trace (PC-D18, the blocker `baseline_derived_scan_removed`), that this run thus does not measure the cost, and the plan review's estimate, marked as not measured. Do not edit the `.json` copy: its digest must verify.
- [ ] **Step 4:** If it fails: keep the report under `.local/`, record the counts and the first unexplained position in the ledger, and report to the owner. A failed full run is a result. Do not commit a failed report as evidence.
- [ ] **Step 5: Commit** (only for a pass)

```bash
git add docs/superpowers/reports
git commit -m "Record the 10,000-trace pipeline comparison"
```

---

## Self-review (done while writing)

**Spec coverage:**

| Spec section | Task |
|---|---|
| 5 Dataset and sample; the pin | 3 (export), 8 (network pin) |
| 6.1 Calibration | 2 (`derive_floors`), 4 (`calibrate_floors`) |
| 6.2 Shared configuration | 4 (`compare_main_gate`), 5 (`start_compare_app`) |
| 7.1 Structure, 7.2 Side drivers | 5 |
| 7.3 Envelope | 3 (`trace_file_for`), 4 (`compare_envelope`) |
| 7.4 Sequence, 7.5 Volume | 6 (the loop), 4 (the lazy reader), 5 (`Probes`) |
| 8.1 Privacy configuration | 5 (Step 2), PC-D6 |
| 8.2 Alignment | 2 (the table), 5 (the two finish steps) |
| 8.3 Known differences | 5 (tests 3 and 4), 6 (scenario 2), 9 (the owner ruling) |
| 9 Records | 1 (the type), 6 (the file) |
| 10 Comparison | 1 |
| 11 Command, export changes | 7, 3 |
| 12 Report | 2 (builder), 7 (validator, Markdown) |
| 13 Failure labels | 6 (the checks of step 9), 7 (the mapping) |
| 14 CI and fixtures | 3 (fixtures), 7 (`--self-test`), 8 (the step) |
| 15 Tests | 1, 2, 4 (index similarity), 6 and 7 (negative test) |
| 16 Workstream, sequence | Preconditions, 8 (runbook), 9, 11 |
| 17 Points to verify | PC-D5 (3), PC-D7 (5), Task 5 Step 3 (4), Task 9 (7), PC-D10 (8), Task 4 test 8 (6) |
| 18 Two bundles | Global Constraints (side names, the marker), Task 5 (the drivers) |

**Differences from the spec (the owner accepted the first six on 2026-10-08; the others follow from the owner's answers to questions 6 to 10 on the same day):**
- Spec section 9 lists flat gate fields. The record nests them in `gate` (absent when no score exists). The names are the same.
- Spec section 11.2 says that the fixture keeps the session events. The plan writes a JSONL corpus with `trace_file` (PC-D3).
- Spec section 14 has one fixture set with a medium-risk and a high-risk trace. The plan uses a second pin and `--self-test` (PC-D9).
- Spec section 15 changes the candidate's floor in the negative test. The plan changes the baseline's (PC-D8).
- Spec sections 1 and 3 say that the tool does not depend on PR 5. PR 5 is merged, and `BASE_CMP` includes it. The harness uses PR 5's routing store and its unqualified test routing (PC-D15, accepted by the owner on 2026-10-08). The compared surface does not change.
- The record and the report have three fields that the spec does not name: `terminal`, `gate_skipped_by_alignment`, and `first_unexplained_position`. Each serves Review Focus 1 or 2.
- A seventh difference, which the owner selected on 2026-10-08 with question 9: spec section 12 lists the blockers of a corpus report plus `deterministic_privacy_only`. The report has one more blocker, `baseline_derived_scan_removed`, because the harness removes the baseline tenant's derived files after each trace (PC-D18). The spec has no such step.
- Spec section 8.2 gives a rule only for a refusal on one side. The plan adds a rule for each refusal (PC-D19, question 6) and the failure label `comparison_receipt_refused`, which spec section 13 does not list.
- Spec section 7.4 has no early stop. The plan stops the run at the first pair after which the two indexes can differ (PC-D20, question 7), with the report field `alignment_lost_position` and the failure label `comparison_alignment_lost`. The negative test of spec section 15 thus fails with that label and not with the label for unexplained differences.
- Spec section 11.1 gives `compare` the option `--archive`. The plan has no such option (question 8).
- The spec does not name a build profile. `compare --corpus` uses the optimized build, and `compare --self-test` uses the debug build (PC-D21, question 10).

**Placeholders:** none. The fixture text of Task 3 Step 2 is given as rules, not as text, because the text has no required wording; Task 6 Step 4 states the property that the text must give and what to do if it does not.

**Type consistency:** `ComparisonRecord`, `GateValues`, `AdmissionLabel`, `AlignmentAction`, `DerivedFloors`, `ComparisonSummary`, `ComparisonReportInput`, `CompareFixture`, `CompareCorpusReader`, `CompareApp`, `CompareTenants`, `CompareAppOptions`, `AdmissionObservation`, `RecordBase`, and `Probes` have one definition each, in the Interfaces section or in the task that produces them, and later tasks use those names.

## Plan review of 2026-10-08 (common.md, review rule 7)

Five reviewers read this plan against `main` at `0e5dffe6b`, each with one lens, and one verifier checked each candidate against the code. The files are in the ledger `.superpowers/sdd/2026-10-02-pipeline-comparison-plan/`: `plan-review-context.md`, `plan-review-design.md`, `plan-review-integration.md`, `plan-review-efficiency.md`, `plan-review-concurrency.md`, `plan-review-operations.md`, and `plan-review-verified.md` (37 groups, G1 to G37, with the evidence for each).

Result: 35 groups confirmed, 1 plausible, 1 owner question, 0 refuted. Severity: 2 Critical, 14 Important, 20 Minor. Nothing was built or run; each time value in the review is an estimate from the code.

The plan has the change for each of these groups (the group id is in the changed text where it helps):

| Group | Severity | Defect | Where the plan changed |
|---|---|---|---|
| G1 | Critical | The submit rate limit (30 receipts in 60 s for each principal) refuses the harness after trace 30. | PC-D16; Task 5 Step 2, Step 4 test 9; Task 8 Step 3 |
| G2 | Important | The plan did not define a refused receipt: two equal refusals compared `Equal`, or each one waited 60 s. | PC-D19 (owner, question 6); Interfaces; Task 1 Step 1 tests 11 and 13; Task 5 Step 4 test 14, Step 7; Task 6 Step 3; Task 7 Interfaces, Step 1 test 20, Step 4 |
| G6 | Important | The run continued after the two indexes could differ. | PC-D20 (owner, question 7); Interfaces; Task 2 Step 1 tests 10 to 13, Step 3; Task 6 Step 3, Step 4 scenario 3; Task 7 Interfaces, Step 1 tests 3, 16, and 19, Step 3, Step 4 |
| G8 | Important | `compare --archive` failed with `unsupported_report_schema` after the full run. | Owner, question 8; Task 7 Interfaces, Step 1 test 18, Step 4; Task 11 Step 2; "Differences from the spec" |
| G11 | Important | The plan offered a release build as a choice, and `compare` could not start one. | PC-D21 (owner, question 10); Task 7 Interfaces, Step 1 tests 16, 18, and 21, Step 4, Step 6; Task 8 Step 3; Task 9 Step 1, Step 3; Task 11 Step 3 |
| G28 (one part), G29 | Minor, owner question | The committed evidence had no code revision; `trace_hash` and `position` identify the dataset file. | Owner, questions 11 and 12; Task 11 Files, Step 3; Global Constraints; Task 8 Step 3 |
| G3, G4 | Critical, Minor | The candidate review calls were sent with no wait for `awaiting_review`; the poll named two states that the column does not have. | Task 5 Step 7, Step 4 test 10 |
| G5 | Important | The Task 5 tests shared one database and constant tenants, and a tenant keeps its first bundle. | PC-D17; Task 5 Interfaces, Step 1, Step 2, Step 4 |
| G7 | Important | A panic at trace N left no report; two helpers panic where a driver needs an answer. | Task 5 Files, Step 6, Step 7; Task 6 Step 3 |
| G9 | Important | `--self-test` exported two pins into one directory. | Task 7 Interfaces, Step 1 test 16, Step 3, Step 4 |
| G10 | Important | Each baseline receipt reads and compares every earlier derived record of the tenant. | PC-D18 (owner, question 9); Global Constraints; Interfaces (the blocker); Task 5 Interfaces, Step 2, Step 4 test 15; Task 6 Step 3; Task 7 Interfaces, Step 1 test 5; Task 8 Step 3; Task 9 Step 2, Step 3; Task 11 Step 3 |
| G12 | Important | The Task 9 measurement could not give its numbers. | Task 6 Step 3 (the timing file); PC-D14; Task 9 after the owner's answers |
| G13 | Important | A file change during a long run fails the command at its end. | Task 9 Step 1; Task 11 Step 2; Task 8 Step 3 |
| G14 | Important | The Phase B merge stops on 29 conflicted files. | Preconditions, item 2 |
| G15 | Important | The check block contradicted common.md's three levels. | Global Constraints; Task 3 Step 8; Task 7 Step 6 |
| G16 | Important | Four alignment actions had no driver test. | Task 5 Interfaces, Step 4 tests 11 to 14 |
| G17 | Important | Scenario 2 could not show a chain of novelty differences. | Task 3 Step 2 and Step 7 (`s04h`, `s04m`); Task 6 Step 4 |
| G18 to G27, G30 to G37 | Minor | See `plan-review-verified.md`. | Tasks 1 to 8 and Task 11, Global Constraints, "If the work stops" |

Seven groups need an owner decision, because each one changes what a pass means, what the committed evidence is, or what the full run costs: G2, G6, G8, G10, G11, G28 (one part), and G29. They are questions 6 to 12 below. The owner answered all seven on 2026-10-08, and the plan text has the changes (the rows G2, G6, G8, G10, G11, G28, and G29 of the table above). These changes were made after the five-lens review, so one separate check reads them before the owner's approval.

## Questions for the owner

1. **PC-D5. Answered on 2026-10-08: accepted.** The baseline index is `MockVectorIndex`, in test code only. The result scope is `local_test`, and the report keeps the blocker `synthetic_index`.
2. **PC-D8. Answered on 2026-10-08: accepted.** The negative test changes the baseline floor, because ingest refuses a candidate bundle whose floors differ from `main`'s.
3. **PC-D9. Answered on 2026-10-08: accepted.** The medium-risk and high-risk traces go into a second local pin, and the CI step is `pipeline.py compare --self-test` with four scenarios.
4. **PC-D15. Answered on 2026-10-08: accepted.** The candidate tenant gets the pipeline through unqualified test routing, not through an activation. The comparison does not go through the activation gate; `pipeline_activation_pg_tests.rs` covers that gate.
5. **Execution. Answered on 2026-10-08: subagent-driven** (superpowers:subagent-driven-development).
6. **Plan review G2. Answered on 2026-10-08: agreed (PC-D19).** The question: a receipt that a side refuses in a full run. Spec section 8.2 gives a rule only for a refusal on one side. As the plan is written, two equal refusals compare `Equal` (or each one waits 60 s). I recommend: the pair is `unexplained`, the harness makes no 60 s wait for it, and a full run with one or more refused receipts fails with its own label (`comparison_receipt_refused`) and emits no check result. Nothing content-dependent was compared for such a trace. With PC-D16, a refusal is rare, so this costs nothing in a good run. The alternative: two equal refusals are `Equal`, and the run can pass.
7. **Plan review G6. Answered on 2026-10-08: agreed, stop when the indexes deviate (PC-D20).** The question: the harness can no longer keep the two indexes equal (for example: the baseline refuses a trace that the candidate admits, a side does not reach a terminal state, or a review call fails). I recommend: stop the run at that trace, write a partial report with a new field `alignment_lost_position`, and fail with the label `comparison_alignment_lost`. Later results are not independent evidence, and a permanent cause makes the run wait 60 s for each remaining trace. The alternative: continue and report all later differences.
8. **Plan review G8. Answered on 2026-10-08: remove `--archive` from `compare`.** The question: `compare --archive`. `update_catalog` accepts only the qualification report and the corpus report, so `--archive` fails with `unsupported_report_schema` at the last step of the full run. I recommend: remove `--archive` from `compare` in this PR. This is one more difference from spec section 11.1. The committed report and the check result are the durable record, and the catalog is a local file. The alternative: add a third entry type to the catalog, with its validator and tests.
9. **Plan review G10. Answered on 2026-10-08: the harness removes the baseline tenant's `derived/` files after each trace (PC-D18).** The question: the old path's receipt compares each new trace with every earlier trace of the tenant (about 5 x 10^7 comparisons and file reads for 10,000 traces; no state setting turns it off; no compared decision uses the result). The time is not known: the review estimates 5 to 19 hours in a release build, with no measurement. The first recommendation was to accept the cost and to measure it with a 1,000-trace run. The owner selected a different choice: remove the baseline tenant's `derived/` files after each trace. That changes what the baseline stores, and this answer is the written ruling for it. The owner also ruled that the old receipt code must change in its own PR ("Items for other PRs"). The choice not taken: divide the traces between K tenant pairs, which gives K smaller indexes and changes what the novelty evidence means.
10. **Plan review G11. Answered on 2026-10-08: no option; `compare --corpus` uses the optimized build and `compare --self-test` the debug build (PC-D21).** The question: the build profile of the evidence run. `pipeline.py compare` has no way to start a release build, and Task 9 offers one as a choice. "Release build" is Cargo's name for an optimized build (`cargo test --release`); nothing is released. Recommendation, revised on 2026-10-08 after the owner's question, and accepted: add no option. `compare --corpus` always uses the optimized build, for the harness and for the export. `compare --self-test` keeps the debug build: the CI job has that build already for `qualify`, the two local pins have 10 and 8 traces, and an optimized build there adds one more full build and one more set of cached files to a cache that is near GitHub's limit. Name the build profile in the runbook and in the committed report's header lines. The two sides run in one binary in each profile, and the production binary is an optimized build. The first recommendation was an option `--release`. The alternative: keep the debug profile for all runs.
11. **Plan review G28. Answered on 2026-10-08: agreed; Task 11 commits the check result and its evidence file.** The question: the committed evidence. The report has no code revision; only the check result has `code_revision_hash`. I recommend: Task 11 also commits `pipeline_comparison_hf.result.json` and its `.evidence.json` beside the report. They are hash-only. Without them, only a line that a person types connects the report to a code revision.
12. **Plan review G29 (privacy). Answered on 2026-10-08: accepted, with the sentence in Global Constraints and in the runbook.** The question: `trace_hash` and `position` identify the dataset file to a person who has the pin and the public dataset. The record, a failed report, the ledger, and an issue that names a difference hold them. I recommend: accept it, with one sentence in Global Constraints and in the runbook ("this is intended, so a person can reproduce a difference; do not use the comparison on a corpus that is not public"). A salt would make two runs of one pin give two digests.
