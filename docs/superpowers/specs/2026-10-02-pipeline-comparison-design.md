# Pipeline comparison design

Date: 2026-10-02. Status: design, approved by the owner in sections. The
implementation plan comes after the owner reviews this file. The plan
changes this design in eleven points
(`docs/superpowers/plans/2026-10-02-pipeline-comparison-plan.md`,
"Differences from the spec"), and the runbook
`docs/operator/pipeline-comparison.md` describes the tool as built.

This document specifies a tool that sends the same real traces through the
old gate path of `main` and through the versioned pipeline, and compares the
content-dependent decisions for each trace.

## 1. Purpose

The purpose is activation confidence. The tool gives evidence that the
versioned pipeline, with the compatibility bundle, makes the same decisions
as the old path on real traces, apart from named, permitted differences.

The evidence supports the operator decision to activate the pipeline in
production. That decision comes after PR 5 (activation mechanics) is merged.
The tool does not depend on PR 5. It depends on PR 4
(TraceCommons/trace-commons#1166, branch `vp/pipeline-qualification`).

## 2. Owner decisions (2026-10-02)

| # | Decision |
|---|---|
| D1 | Purpose: activation confidence. Not gate quality, not operational load. |
| D2 | Compared surface: content-dependent decisions only. The current parity test continues to cover the product surfaces. |
| D3 | Gate floors: derived from the bootstrap partition. The bundle uses `CompatibilityBundleConfig::production_compatible`. |
| D4 | Criterion: zero unexplained differences. Measured values are compared as exact integers. |
| D5 | Order: serial on the two sides. One trace reaches a terminal state on the two sides before the next trace starts. |
| D6 | Deliverable: a repeatable command, a CI check on local fixtures, and one committed report of the full run. |
| D7 | Approach: one app, two tenants, full HTTP. |
| D8 | Envelope: multi-event, built from the translator's session events. |
| D9 | Sample: 10,000 traces, word filter 200 to 20,000. |
| D10 | The two admission differences that the code already shows (section 8.3) get no rule now. The owner makes a ruling after the 100-trace run. Update of 2026-10-09: the owner ruled for row 3 (PC-D22, section 10.3). Update of 2026-10-11: the owner ruled for row 4 (PC-D27, section 10.3). |
| D11 | The command has no `--bundle` option now. A later version compares two bundles (section 18). |

## 3. Current state

Facts at PR 4 head `cbe165ff`:

- `pipeline.py run --corpus <pin>` sends a Hugging Face sample through the
  pipeline only. It checks each trace against a fixed expectation. It does
  not send traces through the old path.
- The test `legacy_and_pipeline_tenants_match_under_equivalent_configuration`
  (`pipeline_http_pg_tests.rs`) compares the two paths on two synthetic
  cases. Its old path uses `InMemoryGateService`, whose floors always pass.
- The old gate path is `EnclaveGateOrchestrator::evaluate`
  (`crates/trace-commons-gate-enclave/src/orchestrator.rs`), reached through
  `EnclaveGateService<P, E, V>` and `evaluate_and_record_gate`. It writes a
  `trace_gate_decisions` row.
- The new path is `CompatibilityScorePolicy` and `CompatibilitySettlePolicy`
  (`crates/trace-commons-server/src/versioned_pipeline_compat.rs`). It writes
  Score evidence and a sealed index command.
- The two paths call the same functions for chunks and aggregation
  (`chunk_envelope_plaintext`, `aggregate_chunked_perplexity`,
  `aggregate_chunked_novelty`, `embed_chunk_mean_pooled`).
- The two paths call the same deterministic privacy function,
  `rescrub_trace_envelope`.
- `CompatibilityBundleConfig::local_reference` sets the three gate floors to
  zero. With zero floors, each trace passes the two gates.
- `main` has two entries to the old gate: the worker route
  `POST /v1/workers/gate/evaluate`, and the in-process perplexity score
  driver (`score_one_submission`). The driver adds cost controls (a
  duplicate skip and a cache) and then calls the same function,
  `evaluate_and_record_gate`.
- Since #1325 (2026-10-09) the pipeline applies the same two
  short-circuits, in the same order and with the same knobs
  (`PipelineNoveltyUtilityChecks::duplicate_controls`), when it writes a
  run's gate decision row at Settle: `skipped_duplicate` or `cached`, with
  no credit quality, and the NoveltyUtility leg withheld. The Score
  evidence is unchanged.
- Since #1324 (2026-10-09) the pipeline runs the prose classifier in a
  privacy pass at the start of Review, after the receipt. A pass that
  raises the risk parks the run for a human (`escalated`), and since #1332
  it also moves an admitted submission to `quarantined`.

## 4. Scope

In scope:

- A comparison of the content-dependent decisions for each trace: privacy
  risk, admission, the gate results, the measured values, the chunk counts,
  the index membership, and the credit event.
- The reference scorer and the reference embedder on the two sides.
- A command, a report, a CI check, a runbook, and one full run.

Out of scope:

- Real scorers. The result says nothing about gate quality.
- The prose classifier and the PII backstop. They need a model.
- The Review-start privacy pass of the pipeline (#1324). The harness reads
  the privacy fields at the receipt, before the pass (section 8.3, PC-D25).
- Concurrent submission, load, and throughput.
- The product surfaces (receipt, status, credit summary, dedup, withdrawal).
- The duplicate short-circuits (`skipped_duplicate`, `cached`). The two
  production paths apply them, the old path in the score driver and the
  pipeline at Settle since #1325, and the run applies them on neither side
  (section 17, point 2, PC-D24). The report has the blocker
  `duplicate_short_circuits_not_compared`.
- A promotion requirement in `evaluate_promotion`.
- A comparison of two bundles (section 18).

## 5. Dataset and sample

### 5.1 Source

- Repository `jedisct1/security-audits`, revision
  `6d527ff0081eec6704c2a4f00e1ef8d308ae7366`, split `train`, translator
  `swival`. These are the values of the PR 4 pin.
- The revision has 35,219 session files.
- The export binary (`trace-commons-pipeline-corpus-export`) selects
  sessions in name order and keeps those that pass the word filter. This
  behavior does not change.

### 5.2 Sample

- 10,000 traces in one fixed order: 1,000 bootstrap traces, then 9,000
  holdout traces.
- Word filter: 200 to 20,000 words.
- A new network pin (`trace_commons.pipeline_hf_corpus_pin.v1`) records the
  counts, the filter, and the digests. It is committed beside
  `docs/superpowers/specs/fixtures/versioned-pipeline-minimal-corpus-v1.json`.
  This is the PR that first needs the network dataset, so the pin goes into
  this PR (owner ruling of 2026-09-30).

### 5.3 Measurement (2026-10-02)

The first 500 sessions in name order, with the word filter off:

| Value | Result |
|---|---|
| Sessions that the translator accepts | 500 of 500 |
| Words for each trace | minimum 486, median 2,910, 90th percentile 8,438, maximum 19,884 |
| Mean size | 3,917 words, 38,057 characters |
| Pass 200 to 2,000 words (the PR 4 filter) | 169 of 500 (33.8%) |
| Pass 200 to 10,000 words | 474 of 500 (94.8%) |
| Pass 200 to 20,000 words | 500 of 500 (100%) |
| More than 16 chunks (estimate at 8,192 characters for each chunk) | 11 of 500 (2.2%) |

Conclusions:

- With the filter 200 to 20,000, the first 10,000 session files give the
  sample. With the PR 4 filter, the export must read approximately 29,600
  files, and no trace has more than two chunks.
- Approximately 2% of the traces exceed the chunk cap of 16, so
  `chunks_capped` gets evidence.
- 10,000 traces are approximately 380 MB of text and approximately 51,000
  index vectors.
- The export downloads one file at a time, at approximately 18 files in
  each minute. 10,000 files take more than 9 hours. Section 11.2 changes
  this.

## 6. Gate configuration

### 6.1 Calibration

The calibration step derives the three floors from the bootstrap partition.

1. Build the envelope for each bootstrap trace, in order (section 7.3).
2. Evaluate each envelope with the old orchestrator, the reference scorer,
   the reference embedder, a new empty in-memory index, and zero floors.
3. Set `perplexity_floor_micros`, `tail_fraction_floor_micros`, and
   `novelty_floor_micros` to the lower median of their measured values.

The step has no database and no HTTP. It is deterministic, so the command
derives the floors again in each run and writes them to the report. There is
no stored floor file.

The percentile is a constant in the code. A change to it is a reviewed
change.

### 6.2 Shared configuration

One `MainGateConfig` value holds the derived floors and the other gate
settings. `top_k` is 5, the default of `main`
(`TRACE_COMMONS_GATE_DEFAULT_TOP_K`, owner decision PC-D23). The other
settings are the values of `CompatibilityBundleConfig::local_reference`:
insert threshold 50,000, chunk target 2,048 tokens, chunk maximum 3,072
tokens, chunk cap 16, chunk minimum 64 tokens.

The `NoveltyUtility` delta is not zero, so the credit branch gets evidence.
It has the value of the current parity test
(`LEGACY_NOVELTY_UTILITY_CREDIT_POINTS_DELTA`) on the two sides.

- Baseline side: the orchestrator configuration comes from this
  `MainGateConfig`.
- Candidate side: `CompatibilityBundleConfig::production_compatible` builds
  the bundle from the same `MainGateConfig`. `matches_main_gate` must be
  true.

## 7. Harness

### 7.1 Structure

- A new ignored Rust test, `pipeline_compare_run`, in a new file
  `pipeline_compare_pg_tests.rs` beside `pipeline_corpus_pg_tests.rs`.
  `pipeline.py compare` starts it.
- One scenario database, one encrypted artifact root, and one `AppState`,
  as in the current parity test.
- Two tenants. The baseline tenant uses the old path. The candidate tenant
  has the rollout gate `PipelineReceipts` and uses the pipeline.
- The in-process score driver does not run. The harness is the only caller
  of the old gate.

### 7.2 Side drivers

Each side has one driver. A driver sends a trace, moves it to a terminal
state, and returns a record (section 9).

- **Baseline driver (old path).** `EnclaveGateService` with
  `ReferencePerplexityScorer`, `ReferenceEmbedder`, and an in-memory index.
  It uses `POST /v1/traces`, the old review routes
  (`/v1/review/{submission_id}/lease`, `/v1/review/{submission_id}/decision`),
  and `POST /v1/workers/gate/evaluate`.
- **Candidate driver (pipeline).** The compatibility bundle of section 6.2
  and `IsolatedPipelineIndex`. It uses `POST /v1/traces`, the pipeline
  status, and the pipeline review routes (`/v1/review/pipeline/...`).

The baseline driver and the rules that apply only to the old path are
marked in the code. A later change can remove them without a change to the
record format or the report format (section 18).

The two indexes must give the same similarity for the same vectors. A unit
test proves this on a fixed set of vectors.

### 7.3 Envelope

- The export keeps the translator's session events (section 11.2).
- The harness builds one multi-event envelope for each trace, as
  pilot-bootstrap does (`build_envelope_from_draft`). The contributor-side
  local redaction runs first.
- Each envelope gives consent to model training, so a credit is awarded and
  not withheld (ruling T9-6).
- The two tenants get the same bytes. The submission key is
  `(tenant_id, submission_id)`, so the two tenants can use the same
  identifiers.

### 7.4 Sequence for each trace

1. Send the envelope to the baseline tenant, then to the candidate tenant.
2. Read the privacy risk, its basis, and the admission decision of each
   side.
3. Apply the alignment table (section 8.2).
4. Baseline: if the trace is accepted and alignment permits, call the gate
   route.
5. Candidate: wait for a terminal state.
6. Read the results of the two sides and write the two records.
7. Compare the two records (section 10).

Trace `n + 1` starts only after step 7 of trace `n`.

### 7.5 Changes for volume

- The harness examines each HTTP body for probes when the body passes. It
  does not keep all bodies in memory.
- The harness does not do the replay check or the changed-content check.
  `pipeline.py run` covers them.

## 8. Privacy and admission

### 8.1 Configuration

- The two sides use the deterministic rescrub only. The prose classifier
  and the PII backstop are off. The report has the blocker
  `deterministic_privacy_only`.
- The candidate's privacy fields are read at the receipt. The
  Review-start privacy pass (#1324) runs later, and no compared field
  reads its result (PC-D25). The report has the blocker
  `review_start_privacy_pass_not_compared`.
- `TRACE_COMMONS_ACCEPT_MEDIUM_RISK_SUBMISSIONS` is true on the baseline
  side. This is the value of `deploy/pilot-gcp/ingest.env.template`.

### 8.2 Alignment

One admission difference can change the index membership. Then all later
novelty values differ, and the true cause is hidden. The harness thus keeps
the two indexes equal. It still records the admission difference itself.

| Baseline | Candidate | Harness action |
|---|---|---|
| accepted | admit | none |
| accepted | quarantine | approve the candidate |
| accepted | reject | do not call the baseline gate |
| quarantined | admit | approve the baseline |
| quarantined | quarantine | apply the hash rule to the two sides |
| quarantined | reject | reject the baseline |

The hash rule: approve if the first byte of `trace_hash` (section 9) is
even, reject if it is odd.

Each record names the review decision and its source (`none`, `hash_rule`,
or `alignment`).

A receipt that one side refuses is a difference in the field
`receipt_code`. Such a trace has no other compared fields.

### 8.3 Differences that the code already shows

| Rescrub result | Baseline | Candidate |
|---|---|---|
| Low | accepted | admit |
| Medium, the basis is the consent flag only | accepted | admit |
| Medium, a different basis | accepted | quarantine |
| High | quarantined | reject |

No written ruling existed for the last two rows when the tool was built.
The 100-trace run gave their counts on real traces. The owner then decides
for each row: a named rule or a defect (D10).

Ruling of 2026-10-09 (PC-D22) for row 3: the quarantine of a trace whose
medium risk has a cause other than the consent flag alone is intended. The
tool permits this difference under the rule `medium_risk_privacy_review`
(section 10.3).

Ruling of 2026-10-11 (PC-D27) for row 4: the reject of a trace whose risk is
High at the receipt is intended. The old path quarantines such a trace
(`status_for_risk` gives `Quarantined`) and a reviewer then decides. The
pipeline rejects it at Admission (reason `privacy_risk_rejected`) and runs no
Review. The source of the intent is the merged design
`docs/superpowers/specs/2026-10-09-pipeline-async-privacy-rescrub-design.md`,
which says twice: "A receipt-time High is still rejected by Admission." The
full run of 2026-10-09 (10,000 traces) had one pair of row 4, at position
8298. The tool permits this difference under the rule
`high_risk_admission_reject` (section 10.3). With this rule, a trace whose
risk is High at the receipt gets no reviewer in the pipeline, and on `main` a
reviewer can approve it. Any other pair that does not meet the exact
condition of a rule stays `unexplained`.

The condition on the basis is "not exactly `["consent_content_flag"]`", so
an empty basis also meets it. A declared risk has no basis label, and the
permitted pair of `--self-test` (the declared medium trace) meets the rule
through its empty basis. The rule is not narrowed to a non-empty basis.

The table is the receipt. Since #1324 the pipeline runs the prose
classifier again in a privacy pass at the start of Review. The harness
reads `privacy_risk`, `privacy_basis`, and `admission` at the receipt, so
the pass is not compared (PC-D25, blocker
`review_start_privacy_pass_not_compared`). With the deterministic boundary
of this tool the classifier half of the pass changes nothing, and the pass
is `cleared` for each trace. With a real classifier, a pass that escalates
parks the candidate run for a human. The candidate side is then not
terminal, and the run stops with `comparison_alignment_lost` (PC-D20). That
fails closed, but the label names the alignment and not the privacy pass.

## 9. Records

The harness writes one record for each trace and each side to
`comparison-records.jsonl` in the run directory. Each line is canonical
JSON (`trace_commons_protocol::canonical_json::to_canonical_vec`).

A record contains labels, numbers, and hashes only:

| Field | Content |
|---|---|
| `position`, `partition` | the place of the trace in the sample |
| `trace_hash` | SHA-256 of the trace identifier |
| `side` | `baseline` or `candidate` |
| `receipt_code` | the HTTP status of the receipt |
| `privacy_risk`, `privacy_basis` | the risk label and the basis labels |
| `admission` | `admit`, `quarantine`, or `reject` |
| `review`, `review_source` | the review decision and its source |
| `scored` | whether a gate evaluation ran |
| `quality_passed`, `novelty_passed` | the gate results |
| `perplexity_micros`, `tail_fraction_micros`, `peak_perplexity_micros` | measured values |
| `novelty_score_micros`, `peak_novelty_micros` | measured values |
| `chunk_count`, `total_chunk_count`, `chunks_capped` | coverage |
| `index_cardinality` | the index size at the time of the score |
| `member`, `member_chunks` | the membership and the chunk numbers in the index |
| `credit_quality_micros`, `credit_quality_version` | the shadow credit quality |
| `credit_event`, `credit_microcredits` | the ledger event type and amount |

The side names are `baseline` and `candidate`, not `legacy` and `pipeline`
(section 18).

## 10. Comparison

### 10.1 Compared fields

| Baseline source | Candidate source |
|---|---|
| privacy risk and basis | privacy risk and basis |
| status after receipt | admission decision |
| `perplexity_passed` | `quality_passed` |
| `novelty_passed` | `novelty_passed` |
| `perplexity_micros`, `tail_fraction_micros`, `peak_perplexity_micros` | the same names in the Score evidence |
| `novelty_score_micros`, `peak_novelty_micros` | the same names |
| `chunk_count`, `total_chunk_count`, `chunks_capped` | the same names |
| `index_cardinality_at_scoring` | `index_cardinality` |
| chunk numbers in `inserted_chunk_entries` | chunk numbers in the sealed index command |
| `q_micros` and its calibration version | `credit_quality_micros`, `credit_quality_version` |
| `NoveltyUtility` event amount | Score award for `trace_credit` |

Each field is compared for exact equality.

### 10.2 Fields that are not compared

Each exclusion has a named rule:

| Rule | Field | Source |
|---|---|---|
| `deterministic_index_keys` | index entry identifiers, `nearest_neighbor_hash` | compatibility mapping, required behavior change 2. The hash includes the keys. |
| `ledger_reason_text` | the text of the ledger reason | ruling T15-2 |
| `shadow_values_not_in_contract` | dedup penalty, contributor cap, `anomaly_withheld` | compatibility mapping, "Membership and credit rules" |

### 10.3 Rules

- The rules are one enumeration in a pure comparison function. The function
  has no database, no HTTP, and no clock.
- Each rule has an identifier that matches `^[a-z0-9_]{1,64}$` and cites its
  source.
- A rule of this section either excludes a field from the comparison (section
  10.2) or permits a difference in a compared field (a permitted difference).
  A permitted difference applies only when its exact condition holds.
- A new rule is a reviewed change to the code. If the rule also changes the
  compatibility mapping, that text goes to `main` as its own small upstream
  PR (flow rule 1 of the workstream rules).

Permitted differences:

| Rule | Field | Condition | Source |
|---|---|---|---|
| `medium_risk_privacy_review` | `admission` | The privacy risk is `medium` on the two records. The privacy basis is equal on the two records and is not exactly `consent_content_flag` (an empty basis meets this, section 8.3). The baseline admission is `admit` and the candidate admission is `quarantine`. | ruling PC-D22 (2026-10-09) |
| `high_risk_admission_reject` | `admission` | The privacy risk is `high` on the two records. The privacy basis is equal on the two records (its content does not matter). The baseline admission is `quarantine` and the candidate admission is `reject`. | ruling PC-D27 (2026-10-11) |

A rule permits no other field. If another compared field also differs,
the pair is `unexplained` with that field only. `permitted_rules` lists the
rules in the order of the table.

### 10.4 Result

- Each trace gets one result: `equal`, `permitted` with the rule
  identifiers, or `unexplained` with the field names.
- The run passes only if no trace is `unexplained`.
- A full run also fails if a branch has no evidence: `quality_passed`,
  `novelty_passed`, and `member` must each be true for one trace or more and
  false for one trace or more.

## 11. Command

### 11.1 Interface

```bash
python3 scripts/operator/pipeline.py compare --corpus <pin> [--limit N] \
  [--postgres-admin-url URL] [--archive]
```

- No `--bundle` option (D11).
- `--limit N` compares only the first N traces. The calibration still uses
  the full bootstrap partition. The report has `partial: true`. A partial
  report is not activation evidence, and the branch check of section 10.4
  does not apply.
- `--postgres-admin-url` and `--archive` work as they do for
  `pipeline.py run`.
- The command is independent of `pipeline.py qualify`. It adds no check to
  the required check set.

Steps of one run:

1. Export the pin and check its digests, before any database starts.
2. Calibrate the floors.
3. Start one environment and run the harness.
4. Require committed transactions in the scenario database.
5. Validate the report and write the latest copy under `.local/`.
6. Print one line with the counts and the duration.

### 11.2 Export changes

- A new flag, `--with-events`, makes the export keep the session events in
  each fixture. `pipeline.py run` does not use the flag, so its corpus
  digests do not change.
- The export downloads files with bounded concurrency and keeps the name
  order for the selection. No new third-party dependency.

## 12. Report

Schema: `trace_commons.pipeline_comparison_report.v1`. The Rust harness
writes the report. The Python tooling validates it with the same privacy
checks as a corpus report (no raw UUID, no private field name, no
secret-shaped value) and writes a `.md` copy.

Fields:

- `schema`, `check_id`, `scope: "local_test"`, `production_ready: false`,
  `partial`.
- `safe_blockers`: the blockers of a corpus report, plus
  `deterministic_privacy_only`, `baseline_derived_scan_removed` (PC-D18),
  `duplicate_short_circuits_not_compared` (PC-D24), and
  `review_start_privacy_pass_not_compared` (PC-D25).
- Identity: the pin digests, `bundle_id`, `package_hash`,
  `configuration_digest`, `dependency_digest`, and the derived floors.
- Counts: `trace_count`, `equal_count`, `permitted_counts` for each rule,
  `permitted_total` (the number of permitted traces; a trace that several
  rules permit counts one time here and one time for each rule),
  `unexplained_counts` for each field. `equal_count`, `permitted_total`,
  and `unexplained_total` add up to `compared_count`.
- `excluded_rules` and `permitted_rules`: each rule with its `rule`,
  `source`, and `fields`. A report holds exactly the closed list of
  excluded rules (PC-D26) and exactly the closed list of permitted rules
  (`medium_risk_privacy_review`, then `high_risk_admission_reject`).
- `distribution` for each side: admission decisions, gate passes and
  failures, membership, and capped traces. The admission counts of a side
  add up to `compared_count`, each pair of gate counts adds up to `scored`,
  and `chunks_capped` is at most `scored` (PC-D26).
- `unexplained`: the trace hash and the field names of each unexplained
  trace. The list holds the first 1,000 entries in sample order, and
  `unexplained_total` holds the full count.
- `records_digest`: the SHA-256 of `comparison-records.jsonl`.
- `report_digest`: the SHA-256 of the canonical report without this field.

The report has no time fields. The same pin and the same code give the same
`report_digest`.

## 13. Failure labels

- `comparison_has_unexplained_differences`
- `comparison_gate_branch_not_exercised`
- `comparison_report_missing`
- `comparison_report_malformed`

The current labels for the export, the database, and the environment do not
change. `pipeline.py` prints one safe label for a failure and never prints
a child command's output.

## 14. CI and fixtures

- One new step in the job `pipeline qualification and restore`, after
  `pipeline qualify`. The job stays not required.
- The step runs `pipeline.py compare` on a new local fixture directory,
  `crates/trace-commons-server/tests/fixtures/pipeline-compare-jsonl/`, with
  approximately 12 synthetic multi-event sessions and its own local pin. It
  makes no network call.
- The fixtures give a gate pass, a quality failure, and a novelty failure
  (a repeated session).
- The fixtures also give one medium-risk trace and one high-risk trace. If
  session text cannot give these risks after the local redaction, the local
  pin declares the risk for those two sessions. Only a local pin accepts
  that field. With PC-D22 and PC-D27 the two risk traces are two permitted
  pairs: the risk pin passes with `permitted_total` 2, and `--self-test`
  expects that result. Its negative proof is the scenario with the skewed
  floor.
- A trace above the chunk cap is too large for a fixture. A unit test
  covers it.

## 15. Tests

- Unit tests for each rule, each compared field, the alignment table, and a
  capped trace. They are in the default test run.
- A unit test that the two index implementations give the same similarity.
- Python self-tests in `scripts/operator/test_pipeline_tooling.py` for the
  report validation, the failure labels, and the refusal of private
  content.
- The CI run on the local fixtures.
- A negative test: a test-only switch gives the candidate side a different
  floor. The run must fail with `comparison_has_unexplained_differences`. A
  comparison that cannot fail is not evidence.

## 16. Workstream

- Branch `vp/pipeline-comparison`, worktree
  `.claude/worktrees/pipeline-comparison`, stacked on PR 4
  (`upstream/vp/pipeline-qualification`, head `cbe165ff`).
- It takes PR 4 changes by a merge. When PR 4 is squash-merged, it merges
  `upstream/main` one time.
- A new PostgreSQL container, `tc-pipeline-pg-cmp`, on 127.0.0.1:55434.
- No migration. No new third-party dependency.
- A defect that the comparison finds is not fixed on this branch. A defect
  in merged PR 3 code goes to the PR 3 follow-up PR. A defect in PR 4 code
  goes to PR 4.
- New runbook `docs/operator/pipeline-comparison.md`, listed in
  `docs/operator/README.md`.
- The report of the full run (`.json` and `.md`) is committed under
  `docs/superpowers/reports/`.

Sequence:

1. The record, the comparison function, and the rules.
2. The export changes, the local fixture sessions, and the local pin.
3. The calibration step.
4. The harness with the two side drivers, and the report.
5. `pipeline.py compare`, the report validation, and the self-tests.
6. The CI step and the runbook.
7. A run with `--limit 100` on the network pin. It gives the speed and the
   counts for the two rows of section 8.3. The owner makes a ruling on those
   rows.
8. An owner decision on the speed: run as it is, use a release build, or
   divide the traces between tenant pairs.
9. The full run with 10,000 traces, and the committed report.

## 17. Risks and points that the plan must verify

1. **PR 4 is not merged.** A review round on PR 4 can change the harness
   code that this branch uses.
2. **The duplicate short-circuits are not compared.** The baseline calls
   the gate route, which is `evaluate_and_record_gate`. The pilot's
   in-process score driver adds a duplicate skip and a cache before that
   call (`skipped_duplicate`, `cached`). Correction of 2026-10-10: the text
   until then said "The pipeline has no such step". That is false since
   #1325, which applies the same two short-circuits to the pipeline's gate
   decision row at Settle. The run applies them on neither side:
   - The candidate runtime is assembled with
     `PipelineNoveltyUtilityChecks::default()` apart from the issuer, so
     `duplicate_controls` is `None`. Ingest always passes `Some` for an
     assembled runtime.
   - The baseline goes through `POST /v1/workers/gate/evaluate`, and the
     short-circuits are only in the score driver. `clear_baseline_derived`
     (PC-D18) also removes the derived record whose duplicate score the
     driver reads.
   - The two sides read `credit_quality_micros` from different places: the
     candidate from the Score evidence (`candidate_gate_values`), the
     baseline from its gate decision row (`baseline_finish`). Without the
     short-circuits the two are equal. Since #1325 they differ for a
     duplicate: the row has no credit quality, and the Score evidence
     keeps it.

   With `duplicate_controls: Some(PipelineDuplicateControls::MAIN_DEFAULT)` on the candidate and two
   equal traces, the review of 2026-10-10 found the second pair
   `unexplained` in `credit_events` only, while `credit_quality_micros`
   compared equal. The owner chose to name the gap and not to compare the
   path in this PR: the report has the blocker
   `duplicate_short_circuits_not_compared` (PC-D24), and a follow-up issue
   holds the comparison of the path (`MAIN_DEFAULT` on the candidate, the
   baseline through the score driver, and the candidate's credit quality
   read from its gate decision row).
3. **The baseline index.** The in-memory index of the gate-enclave crate is
   `MockVectorIndex`. A `Mock*` type must never gate anything in production.
   The plan must confirm that its use in a test harness is acceptable, or
   give the baseline side a different in-memory index.
4. **The Score input.** The old path scores the decrypted stored envelope.
   The new path scores the reviewed artifact. If the two inputs are not the
   same bytes, the measured values differ. This is a result that the tool
   must find, not a defect of the tool.
5. **The aggregation argument.** The old path gives
   `qualifying_chunk_floor_micros` to `aggregate_chunked_perplexity`. The new
   path gives `perplexity_floor_micros`. The plan must confirm which compared
   fields this argument changes.
6. **A constant measured value.** If the reference scorer gives the same
   tail fraction for each trace, its floor is that value and the tail check
   always passes. The perplexity floor then gives the quality failures.
7. **Speed.** The harness waits a minimum of one 100 ms poll interval for
   each trace, and each index search is a full scan in a development build.
   Step 7 of the sequence measures the speed.
8. **Read access to the candidate results.** The plan must select how the
   harness reads the Score evidence values: the forensic route or the
   runtime store, as the parity test does.

## 18. Future work: two bundles

When the old path and the compatibility layer are removed, the command
compares two bundles of the new architecture. It then needs two options,
for example `--baseline-bundle` and `--candidate-bundle`.

This design prepares for that change and does not build it:

- The records, the comparison function, and the report use the side names
  `baseline` and `candidate`.
- Each side has its own driver. A later change replaces the baseline driver
  with a second pipeline driver.
- The rules that apply only to the old path are marked. A later change
  removes them.
