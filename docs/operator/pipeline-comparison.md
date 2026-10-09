# Comparing the old gate path with the versioned pipeline

> **Status: the tooling exists.** `pipeline.py compare` runs today against a
> real, disposable PostgreSQL server and the shared `trace-commons-ingest`
> app. A result is local evidence only: the two sides use the reference
> scorer and the reference embedder, and each report carries
> `production_ready: false` and eight blockers. The result says nothing
> about gate quality. Production routing of live tenants through the
> pipeline is a separate operator decision; see
> [pipeline-activation.md](pipeline-activation.md).

`pipeline.py compare` sends the same traces through two paths of one app:

- **`baseline`**: the old gate path of `main`.
- **`candidate`**: the versioned pipeline, with the compatibility bundle.

For each trace, the command compares the content-dependent decisions of the
two sides: the privacy risk, the admission, the gate results, the measured
values, the chunk counts, the index membership, and the credit. Each
compared value must be exactly equal. No rule permits a difference today,
so one difference fails the run. The design is in
[the comparison spec](../superpowers/specs/2026-10-02-pipeline-comparison-design.md).

## Run a comparison

```bash
python3 scripts/operator/pipeline.py compare \
  --corpus crates/trace-commons-server/tests/fixtures/pipeline-compare-jsonl/pin-local.json
python3 scripts/operator/pipeline.py compare --self-test
```

Exactly one of `--corpus PIN` and `--self-test` is required. The command
has no `--bundle` option and no `--archive` option, and it writes nothing
to the lab catalog. It is independent of `pipeline.py qualify` and adds no
check to the required check set.

The command needs Docker for its PostgreSQL container, or
`--postgres-admin-url URL` for a server that exists. The two work as they
do for `pipeline.py run`; see
[pipeline-qualification.md](pipeline-qualification.md#the-environment-container-digest---postgres-admin-url-one-server-at-a-time).

### The pins

`--corpus` takes a comparison pin. A comparison pin is an HF pin descriptor
(`trace_commons.pipeline_hf_corpus_pin.v1`, see
[pipeline-lab.md](pipeline-lab.md#the-hf-pin)) with these fields added:

| Field | Content |
|---|---|
| `with_events` | `true`. The export then keeps the session events of each trace. `compare` refuses a pin without it. |
| `session_names` | Optional. A list of file names. Only these files are candidates. For a local pin only. |
| `declared_privacy_risk` | Optional. A map from a file name to `medium` or `high`. For a local pin only. |

Three pins are committed:

| Pin | Traces | Use |
|---|---|---|
| [`pin-local.json`](../../crates/trace-commons-server/tests/fixtures/pipeline-compare-jsonl/pin-local.json) | 10 (4 bootstrap, 6 holdout) | A local pin: it names a `local_jsonl_dir` with committed session files, and it needs no network. It must pass. Check id `pipeline_comparison_local`. |
| [`pin-local-risk.json`](../../crates/trace-commons-server/tests/fixtures/pipeline-compare-jsonl/pin-local-risk.json) | 8 (4 bootstrap, 4 holdout) | A local pin with one declared `medium` trace and one declared `high` trace. It must fail with two `admission` differences. `--self-test` uses it. |
| [`versioned-pipeline-comparison-hf-pin-v1.json`](../superpowers/specs/fixtures/versioned-pipeline-comparison-hf-pin-v1.json) | 10,000 (1,000 bootstrap, 9,000 holdout) | The network pin: it has no `local_jsonl_dir`, and its export reads the public Hugging Face dataset `jedisct1/security-audits`. Word filter 200 to 20,000. Check id `pipeline_comparison_hf`. See [The full run](#the-full-run). |

The check id comes from the pin and from nothing else: a pin with a
`local_jsonl_dir` gives `pipeline_comparison_local`, and a pin without one
gives `pipeline_comparison_hf`.

### What one run does

1. It reads the pin.
2. It exports the pin with the binary `trace-commons-pipeline-corpus-export`
   and `--with-events`. The output is `bootstrap-compare.jsonl` and
   `holdout-compare.jsonl` (one trace on each line) and
   `source-manifest.json`. Each digest that the pin has must equal the
   digest of the manifest. This step comes before any database starts.
3. It runs the harness, the ignored Rust test
   `pipeline_compare_pg_tests::pipeline_compare_run`. The harness checks the
   two corpus files against the manifest, derives the floors, starts one app
   with two tenants, and sends each trace through the two sides in the order
   of the files. It writes two records for each trace and one report.
4. It checks the report against this run, writes the latest report under
   `.local/`, and prints one line.

A pass prints one line to stdout and exits with 0:

```text
PipelineCompareOK: traces=10 equal=10 permitted=0 unexplained=0 partial=false seconds=12 run=q0af28b7a report=.local/pipeline-comparison-pipeline_comparison_local.json
```

`seconds=` is the time of step 3. It includes the build of the test binary
when the command builds, so it is not the time of the comparison.
The timing file of the run (`compare_run-timing.jsonl` in the run
directory) has the time of each trace; see
[Where the files are](#where-the-files-are).

A run that fails after the harness wrote a valid report prints where the
report is, and then the failure label:

```text
PipelineCompareReport: unexplained=2 alignment_lost=none partial=false report=.local/pipeline-comparison-pipeline_comparison_local.json
PipelineFailure: comparison_has_unexplained_differences
```

### `--limit N`

`--limit N` compares only the first `N` traces. `N` must be 1 or more. The
calibration still uses the full bootstrap partition, so the floors are the
floors of the full run. A limit at or above the number of traces of the pin
is a full run.

A run with a limit below that number is a partial run. Its report has
`partial: true`, its line says `partial=true`, and the gate branch check
does not apply to it. A run with `partial=true` is not evidence for an
activation: it emits no check result.

### `--self-test`

`--self-test` proves that the comparison can fail. It runs the harness four
times in one environment, with the two local pins, and it passes only when
each run gives its expected result:

1. `pin-local.json` passes and compares each of its traces.
2. `pin-local-risk.json` fails with exactly two `admission` differences and
   nothing else. The baseline accepts the declared `medium` trace and the
   candidate quarantines it. The baseline quarantines the declared `high`
   trace and the candidate rejects it. The alignment keeps the two indexes
   equal, so no later trace differs and the run does not stop early.
3. `pin-local.json` with the baseline's quality floor changed fails. The
   report names the skew (`skew: "baseline_quality_floor"`), the pair at
   which the run stopped, and `quality_passed`.
4. `pin-local.json` again gives the `report_digest` of run 1.

A pass prints `PipelineCompareSelfTestOK: scenarios=4`. `--self-test` takes
no `--corpus` and no `--limit`. Each run of `--self-test` is not evidence
for an activation: it emits no check result, and it writes no report under
`.local/`. The CI job `pipeline qualification and restore` runs
`--self-test` after `pipeline.py qualify`.

### Build and time

`compare --corpus` uses Cargo's optimized build (`--release`) for the export
and for the harness. `compare --self-test` uses the debug build. There is
no option for this.

Measured on one machine, a 12-core server:

- The first optimized build in a checkout takes about 5 minutes.
- `compare --corpus` on `pin-local.json` takes about 12 s after the build.
- `compare --self-test` takes about 45 s after the build.

The times of the network pin are in [Time and memory](#time-and-memory).

## What is compared

The harness writes one record for each trace and each side. The comparison
reads only the two records. The compared fields, in the order of the
comparison:

| Compared field | Baseline source | Candidate source |
|---|---|---|
| `receipt_code` | the HTTP status of `POST /v1/traces` | the same |
| `terminal` | each call of the side after the receipt answered 200 | the run state is `complete` |
| `privacy_risk`, `privacy_basis` | `trace_submissions.privacy_risk` and `residual_risk_basis` | the same columns |
| `admission` | `trace_submissions.status` after the receipt (`accepted` is `admit`, `quarantined` is `quarantine`) | `pipeline_runs.admission_decision` |
| `scored` | the gate route answered 200 | the run has a Score outcome |
| `quality_passed` | `trace_gate_decisions.perplexity_passed` | `quality_passed` in the Score evidence |
| `novelty_passed` | `novelty_passed` | the same name in the Score evidence |
| `perplexity_micros`, `tail_fraction_micros`, `peak_perplexity_micros` | the same names in `trace_gate_decisions` | the same names in the Score evidence |
| `novelty_score_micros`, `peak_novelty_micros` | the same names | the same names |
| `chunk_count`, `total_chunk_count`, `chunks_capped` | the same names | the same names |
| `index_cardinality` | `index_cardinality_at_scoring` | `index_cardinality` |
| `credit_quality_micros`, `credit_quality_version` | `credit_quality_micros`, `credit_quality_calibration_version` | the same names in the Score evidence |
| `member` | the decision has one or more rows in `trace_gate_chunk_vector_entries` | `index_membership` is `included` and `index_write_state` is `complete` |
| `member_chunks` | the `chunk_index` values of those rows | the chunk numbers of the sealed index command |
| `credit_events` | the `trace_credit_ledger` rows of the submission: the event type and the amount in microcredits | the same table |

Rules of the comparison:

- Each field is compared for exact equality. A measured value is an integer
  in micros.
- A difference in `receipt_code` is the only field that the comparison
  names for that trace.
- A pair in which a side is not terminal is never equal: the comparison
  names `terminal`, also when the two values are equal. A side whose
  admission is `refused` or `other` is never terminal.
- The gate values (from `quality_passed` to `credit_quality_version`) are
  compared only when the two sides are scored. When only one side is
  scored, the comparison names `scored`.
- `admission` has five values: `admit`, `quarantine`, `reject`, `refused`
  (the receipt did not answer 200), and `other` (a state that is none of
  the three decisions).

Each trace gets one result: `equal`, `permitted` with the rule names, or
`unexplained` with the field names. No rule permits a difference in a
compared field today, so the tool never gives `permitted`.

Three fields of a record are not compared: `review`, `review_source`, and
`gate_skipped_by_alignment`. The fields `position`, `partition`,
`trace_hash`, and `side` identify the pair. If they do not agree, or if
`scored` does not agree with the gate values of a record, the result names
`record_pair` only.

Three groups of values are not in a record and are not compared. Each has a
named rule, and each report lists the three rules in `excluded_rules`:

| Rule | Values that are not compared | Source |
|---|---|---|
| `deterministic_index_keys` | the index entry identifiers and `nearest_neighbor_hash` (the hash includes the keys) | [compatibility mapping](../superpowers/specs/2026-09-14-versioned-pipeline-compatibility-mapping.md), required behavior change 2 |
| `ledger_reason_text` | the text of the ledger reason | ruling T15-2 |
| `shadow_values_not_in_contract` | the dedup penalty, the contributor cap, and `anomaly_withheld` | compatibility mapping, "Membership and credit rules" |

## Alignment

One admission difference can change the index membership. Then
`index_cardinality` and the novelty values of each later trace differ, and
one cause gives thousands of results. The harness thus keeps the two
indexes equal after an admission difference. It still records the
difference itself, in the field `admission`.

After the two receipts, the harness selects one action for the pair:

| Baseline | Candidate | Harness action |
|---|---|---|
| `admit` | `admit` | none |
| `reject` | `reject` | none |
| `admit` | `quarantine` | approve the candidate |
| `admit` | `reject` | do not call the baseline gate (`gate_skipped_by_alignment: true`) |
| `quarantine` | `admit` | approve the baseline |
| `quarantine` | `quarantine` | apply the hash rule to the two sides |
| `quarantine` | `reject` | reject the baseline |
| each other pair | | stop: no review and no gate call |

The old path has no direct rejection: it accepts or it quarantines. A
baseline `reject` is thus not a result that the old path gives today.

The hash rule: approve if the first byte of `trace_hash` is even, and
reject if it is odd.

Each record names the review decision (`review`: `none`, `approve`,
`reject`) and its source (`review_source`: `none`, `hash_rule`,
`alignment`).

The run stops at the first pair after which the two indexes can differ:

1. The action is "stop" and the candidate admitted the trace. The pipeline
   worker then adds the trace with no call from the harness, and the
   baseline adds nothing.
2. The action is not "stop" and a side is not terminal.
3. The two sides are terminal, and `member` or `member_chunks` differ.

That pair is the last compared pair. It is in the counts and in the records
file. The report names its position in `alignment_lost_position`, and the
run fails with `comparison_alignment_lost`. A difference that leaves the
two indexes equal, for example in `perplexity_micros` only, does not stop
the run.

## The floors

The gate has three floors: `perplexity_floor_micros`,
`tail_fraction_floor_micros`, and `novelty_floor_micros`. No floor file
exists. The harness derives the floors again in each run, before the app
starts:

1. It builds the envelope of each bootstrap trace, in order.
2. It evaluates each envelope with the old gate orchestrator, the reference
   scorer, the reference embedder, a new empty in-memory index, and floors
   of zero.
3. Each floor is the lower median of its measured values: the value at
   index `(n - 1) / 2` of the `n` sorted values.

The calibration always uses the full bootstrap partition, also with
`--limit`. The two sides get the same three floors, and the report has
them in `floors`. A sample whose three floors are all zero is refused
(`comparison_floors_all_zero`): with floors of zero, each trace passes the
two gates and the run compares nothing.

## The report

The harness writes one report with the schema
`trace_commons.pipeline_comparison_report.v1`. `pipeline.py` validates it
with the privacy checks of a corpus report: no raw UUID, no private field
name, no secret-shaped value. The report has no time field.

| Field | Content |
|---|---|
| `schema` | `trace_commons.pipeline_comparison_report.v1` |
| `check_id` | `pipeline_comparison_local` or `pipeline_comparison_hf` |
| `scope`, `production_ready`, `external_payout_enabled` | always `local_test`, `false`, `false` |
| `partial` | `true` when the run compared fewer traces than the pin has |
| `skew` | `null`, or `baseline_quality_floor` in scenario 3 of `--self-test` |
| `safe_blockers` | always `local_test_only`, `local_reference_scorer`, `local_reference_embedder`, `synthetic_index`, `synthetic_settlement`, `static_bearer_authentication`, `deterministic_privacy_only`, `baseline_derived_scan_removed` |
| `pin` | the five digests of the export manifest: `source_digest`, `order_digest`, `configuration_digest`, `bootstrap_corpus_digest`, `holdout_corpus_digest` |
| `bundle_id`, `package_hash`, `configuration_digest`, `dependency_digest` | the candidate's compatibility package. The bundle holds the floors, so the bundle id changes with the floors. |
| `floors` | the three derived floors, in micros |
| `trace_count` | the number of traces in the pin (`sample_count` of the manifest) |
| `compared_count` | the number of traces that the run compared |
| `equal_count` | the traces with the result `equal` |
| `permitted_counts` | for each rule, the traces that it permitted. Always `{}` today. |
| `unexplained_counts` | for each compared field, the traces in which it differs |
| `unexplained_total` | the traces with the result `unexplained` |
| `unexplained` | the first 1,000 unexplained traces in sample order, each with `position`, `trace_hash`, and `fields` |
| `first_unexplained_position` | the position of the first unexplained trace, or `null` |
| `alignment_lost_position` | the position of the pair at which the run stopped, or `null` |
| `excluded_rules` | the three rules of [What is compared](#what-is-compared), each with `rule`, `source`, and `fields` |
| `distribution` | for `baseline` and for `candidate`: the counts `admit`, `quarantine`, `reject`, `refused`, `other`, `scored`, `quality_passed`, `quality_failed`, `novelty_passed`, `novelty_failed`, `member`, `not_member`, `chunks_capped`. `member` and `not_member` count scored traces only. |
| `branch_gaps` | the gate branches with no evidence on the baseline side: any of `quality_passed_true`, `quality_passed_false`, `novelty_passed_true`, `novelty_passed_false`, `member_true`, `member_false`. Always empty for a partial run. |
| `records_digest` | the SHA-256 of the records file |
| `report_digest` | the SHA-256 of the canonical report without this field |

`equal_count`, the sum of `permitted_counts`, and `unexplained_total` add
up to `compared_count`. A position counts through the two partitions, from
0: the first holdout trace of the network pin has the position 1,000.

`trace_hash` is the SHA-256 of the trace identifier. `trace_hash` and
`position` identify the dataset file to a person who has the pin and the
public dataset. This is intended, so that a person can reproduce a
difference. Do not use the comparison on a corpus that is not public.

### Find the trace of a difference

An entry of `unexplained` names a trace by `position` and `trace_hash`. To
find the input of that trace:

1. Take the two corpus files of the run, in `compare/corpus/` of the run
   directory. If you removed them, make them again with the export command
   of [The download](#the-download), which writes them to
   `.local/pipeline/compare-pin-check-hf/`. The export of one pin gives the
   same bytes each time.
2. Count through the two files, bootstrap first, from 0. The positions 0 to
   `bootstrap_count - 1` are the lines of `bootstrap-compare.jsonl`, in
   order. Each later position is a line of `holdout-compare.jsonl`. For the
   network pin, position 1,003 is line 4 of `holdout-compare.jsonl`.
3. Check the line: `trace_hash` is `sha256:` and the SHA-256 of the
   `trace_id` text of that line.

This command does steps 2 and 3. It prints the file, the line number, and
the hash, and no trace text:

```bash
CORPUS=.local/pipeline/runs/q0af28b7a/compare/corpus  # the directory of step 1
POSITION=4                                            # `position` in the report
python3 - "$CORPUS" "$POSITION" <<'EOF'
import hashlib, json, sys
corpus = sys.argv[1]
position = int(sys.argv[2])
bootstrap = json.load(open(f"{corpus}/source-manifest.json"))["bootstrap_count"]
name, index = ("bootstrap", position) if position < bootstrap else ("holdout", position - bootstrap)
with open(f"{corpus}/{name}-compare.jsonl") as lines:
    for number, line in enumerate(lines):
        if number == index:
            trace_id = json.loads(line)["trace_id"]
            print(f"{name}-compare.jsonl line {index + 1}")
            print("sha256:" + hashlib.sha256(trace_id.encode()).hexdigest())
EOF
```

The line holds the recorded trace (`trace_file`) that the run sent to the
two sides. It does not hold the name of the dataset file: line `n` of the
sample is the `n`-th file, in name order, that the export accepted. The two
records of the trace are the lines `2 * position + 1` (baseline) and
`2 * position + 2` (candidate) of the records file.

### When a run is evidence

A run fails when its report names one of these, in this order:

1. `alignment_lost_position` is not `null`
   (`comparison_alignment_lost`).
2. A `refused` count is not zero (`comparison_receipt_refused`). This also
   fails a partial run.
3. `unexplained_total` is not zero
   (`comparison_has_unexplained_differences`).
4. For a full run: `branch_gaps` is not empty
   (`comparison_gate_branch_not_exercised`). `quality_passed`,
   `novelty_passed`, and `member` must each be true for one trace or more
   and false for one trace or more.

Only a full run that passes, with no skew, emits a check result:
`<check id>.result.json` and `<check id>.evidence.json` in the `results/`
directory of the run. The result holds the code revision hash and the
digests of the package. The evidence holds the counts and the hashes of the
records file and of the report file. A partial run, a failed run, and each
run of `--self-test` emit no check result.

A full run also needs a pin with all five digests. A pin that lacks one of
`configuration_digest`, `bootstrap_corpus_digest`, and
`holdout_corpus_digest` can only make a partial run
(`comparison_pin_digest_missing`).

### Where the files are

The latest report of `compare --corpus` goes under `.local/`, with a
Markdown view beside it:

| File | Content |
|---|---|
| `.local/pipeline-comparison-<check id>.json` and `.md` | The latest report of a full run: a pass, or a failure that the report itself names. |
| `.local/pipeline-comparison-<check id>-partial.json` and `.md` | The latest report of a partial run: a run with `--limit`, or a run that stopped early. |
| `.local/pipeline-comparison-<check id>-failed.json` and `.md` | A full report that names no failure, from a harness step that failed all the same. |

The last row matters. A failed harness step that leaves a full report with
no named failure is kept under the name with the suffix `-failed`. Such a
report reads as the report of a pass, and no check result exists for it.
The name without a suffix is written only for a full run that passed, or
for a full run whose report names its own failure. The line
`PipelineCompareOK` is the proof that the run passed. It is evidence only
when it says `partial=false`.

The run directory `.local/pipeline/runs/<run id>/` holds:

- `compare/corpus/`: the two corpus files and `source-manifest.json`. The
  corpus files hold the trace text.
- `compare_run-report.json`: the report, as the harness wrote it.
- `compare_run-records.jsonl`: two lines of canonical JSON for each trace,
  the baseline record and then the candidate record. A record holds only
  numbers, enumerations, hashes, and labels.
- `compare_run-timing.jsonl`: the timing file. It has the time of the
  calibration and the time of each trace. It is not part of the report or
  of a digest.
- `logs/compare_export_corpus.log` and `logs/compare_run.log`: the
  protected logs of the two steps.
- `results/`: the check result and its evidence, for a full run that
  passed.
- `artifacts/`: the encrypted artifacts of the two tenants.

`--self-test` uses the names `compare/local/`, `compare/risk/`, and
`compare_self_pass`, `compare_self_risk`, `compare_self_skew`, and
`compare_self_repeat` in place of `compare_run`.

### Results that a second run can fail to repeat

Two runs of one pin on one machine, one checkout path, and one date
normally give the same `records_digest` and the same `report_digest`. Three
conditions can change a result:

- The two credit-quality values (`credit_quality_micros` and
  `credit_quality_version`) depend on the run date, because each side
  selects the calibration era from its own clock. The schedule has one
  switch, on 2026-09-09. A run after a later switch gives other values and
  other digests.
- A difference in `privacy_basis` that a second run does not show points
  to the entropy sum of the secret scan. That sum adds `f64` terms in
  `HashMap` order, and the order changes from one map to the next. The sum
  can thus differ in its last bit, and a token at the threshold can get
  two results.
- Compare digests only between runs on one platform. The reference scorer
  uses `f64::ln` and `f64::exp`, and their last bit can differ between
  platforms.

The machine and the checkout path also matter. The receipt handler of
`main` builds a redactor for each receipt, and the redactor reads `HOME`,
the current directory, and `TRACE_PRIVACY_FILTER_BACKEND`. The harness
builds its envelopes with the same redactor. The rescrub replaces the home
path and the current directory path in trace text with a placeholder. The
current directory of the harness is the crate directory of the checkout.
The two sides change together, so the comparison holds. But a run on
another machine, or from a checkout at another path, can give other floors
and other digests for a sample that contains such a path.

`pipeline.py` gives a child process only the variables of
`CHILD_ENV_ALLOWLIST` (`scripts/operator/pipeline_tooling/environment.py`)
and the harness variables. No `TRACE_COMMONS_*` variable of the shell is in
that list, and `TRACE_PRIVACY_FILTER_BACKEND` is not in it. That variable
thus does not reach a run that the command starts. `HOME` is in the list.

## Limits

What a passing report does not show:

- **Reference scorers.** The two sides use `ReferencePerplexityScorer` and
  `ReferenceEmbedder`. The result says nothing about gate quality or about
  a real scorer.
- **No classifier and no backstop.** The two sides use the deterministic
  rescrub only. The prose classifier and the PII backstop are off (blocker
  `deterministic_privacy_only`).
- **The gate route, not the score driver.** The harness calls the old gate
  through `POST /v1/workers/gate/evaluate`. The in-process perplexity score
  driver of `main` does not run, so its duplicate skip and its cache are
  not compared.
- **Serial order.** One trace reaches its end on the two sides before the
  next trace starts. The run shows nothing about concurrent submission,
  load, or throughput.
- **Unqualified test routing.** The candidate tenant gets the pipeline
  through its rollout gate (`PipelineReceipts`). The harness assembles the
  runtime with test dependencies allowed and with unqualified routing
  allowed. It writes no routing row and does not activate the tenant.
- **No submit rate limit.** The harness removes the submit rate limit for
  its own five tokens. `main` permits 30 receipts in 60 s for each
  principal, and a run sends more.
- **No baseline duplicate scan.** After each trace, the harness removes
  each `.json` file from the baseline tenant's `derived/` directory. The
  old path's receipt reads and compares each earlier derived record of the
  tenant, so in the run that scan always sees an empty tenant (blocker
  `baseline_derived_scan_removed`). The harness removes no database row and
  no other file. The result of that scan is not a compared field.
- **Not the configuration of a deployed server.** The harness builds the
  baseline state in code, and it does not start the server through the
  production start function, which reads the configuration from the
  environment. One gate configuration gives the two sides: the derived
  floors, and the values of `CompatibilityBundleConfig::local_reference()`
  (insert threshold 50,000 micros, `top_k` 8, chunk target 2,048 tokens,
  chunk maximum 3,072 tokens, chunk cap 16, chunk minimum 64 tokens). Three
  values are not the default values of `main`
  (`crates/trace-commons-server/src/bin/trace-commons-ingest.rs`):
  - `top_k` is 8. The default of `main` is 5
    (`TRACE_COMMONS_GATE_DEFAULT_TOP_K`).
  - The credit for novelty is 2.5 credits on the two sides, so that the
    credit branch gets evidence. The default of `main` is 0
    (`DEFAULT_NOVELTY_UTILITY_CREDIT_POINTS_DELTA`).
  - The baseline accepts medium-risk submissions, as
    `deploy/pilot-gcp/ingest.env.template` sets it. The default of `main`
    is to quarantine them (`TRACE_COMMONS_ACCEPT_MEDIUM_RISK_SUBMISSIONS`
    not set).

  A deployment that sets other values has a different old path, and the
  report says nothing about it.
- **An early stop.** A run stops at the first pair after which the two
  indexes can differ (see [Alignment](#alignment)). The traces after that
  pair are not compared, and the report is partial unless that pair is the
  last trace of the pin.
- **No continuation.** A run cannot continue from a position. Each run
  starts at position 0 with empty indexes.
- **Time limits.** Each HTTP call of the harness has a limit of 60 s, and
  the candidate side of one trace has a limit of 60 s.
- **The product surfaces.** The receipt body, the status, the credit
  summary, dedup, and withdrawal are not compared.

## Failure labels

`pipeline.py` never prints a child command's own output. A failure is one
safe label on stderr, as `PipelineFailure: <label>`. See
[pipeline-lab.md](pipeline-lab.md#failure-labels) for the form of the line.
The labels that `compare` shares with the other commands are in
[Labels of the shared tooling](#labels-of-the-shared-tooling).

The harness is a child. `pipeline.py` shows a harness failure in one of two
ways:

- The harness left a valid report that names a failure. `pipeline.py`
  prints that label (one of the four labels of
  [When a run is evidence](#when-a-run-is-evidence)).
- Each other harness failure is
  `PipelineFailure: step_failed:compare_run exit=<n> log=<run dir>/logs/compare_run.log`.
  The label of the cause is in that log, in the panic message.

A compile error is `cargo_test_list_failed:compare_run`. A failed export is
`step_failed:compare_export_corpus`, and its log holds the export's own
message. `--self-test` has the step names of
[Where the files are](#where-the-files-are) in place of `compare_run`, and
`compare_export_local` and `compare_export_risk` for its two exports.

### The command line and the pin

Printed by `pipeline.py` before the export:

| Label | Cause |
|---|---|
| `compare_corpus_or_self_test_required` | Neither `--corpus` nor `--self-test` was given. |
| `compare_corpus_and_self_test_conflict` | `--corpus` and `--self-test` were given together. |
| `compare_self_test_takes_no_option` | `--limit` was given with `--self-test`. |
| `compare_limit_invalid` | `--limit` is below 1. The harness also gives this label for a limit variable that is not a positive integer. |
| `comparison_pin_unreadable` | The pin file cannot be read, or it is not a JSON object. |
| `comparison_pin_without_events` | The pin does not have `with_events: true`. |
| `comparison_pin_digest_missing` | The pin lacks one of `configuration_digest`, `bootstrap_corpus_digest`, and `holdout_corpus_digest`, and the run is not a partial run. |
| `comparison_pin_field_needs_local_dir` | The pin has `session_names` or `declared_privacy_risk` and no `local_jsonl_dir`. |

### The export

Printed by `pipeline.py` after the export:

| Label | Cause |
|---|---|
| `comparison_manifest_malformed` | The export left no `source-manifest.json`, or the manifest is not a JSON object, or it lacks one of the five digests or a whole `sample_count`. |
| `comparison_manifest_without_events` | The manifest does not say that the export had `--with-events`. |

### The harness: before the first trace

In the log of the step:

| Label | Cause |
|---|---|
| `compare_bootstrap_path_missing`, `compare_holdout_path_missing`, `compare_manifest_path_missing`, `compare_check_id_missing`, `compare_report_path_missing`, `compare_records_path_missing`, `compare_check_id_invalid`, `compare_skew_invalid`, `compare_database_url_missing` | A harness variable is not set, is empty, or has a value that is not valid. `pipeline.py` sets each one. |
| `compare_manifest_invalid`, `compare_manifest_without_events` | The manifest cannot be read or parsed, it lacks a digest or `sample_count`, or it is of an export without `--with-events`. |
| `compare_corpus_read_failed` | A corpus file cannot be opened or read. |
| `compare_corpus_digest_mismatch` | A corpus file does not have the digest of the manifest. The harness checks this before the first trace and again after the last. |
| `compare_trace_count_mismatch` | The two corpus files do not have `sample_count` lines together, or the run compared a number of traces that is not the limit or the trace count. |
| `compare_corpus_line_invalid` | A line of a corpus file is not a valid fixture. The log has `partition=` and `line=` (the line number in that file, from 1). |
| `compare_output_write_failed` | The records file or the report directory cannot be created or written. |
| `compare_envelope_failed` | The redactor gave an error, or an envelope could not be serialized, in the calibration or for one trace. |
| `comparison_calibration_failed`, `comparison_calibration_invalid`, `comparison_calibration_empty` | The calibration could not measure the bootstrap partition: a gate evaluation failed, the three lists of values do not have one length, or the partition has no trace. |
| `comparison_floors_all_zero` | Each of the three derived floors is zero. |
| `compare_pipeline_assembly_failed`, `compare_submission_quota_enabled`, `compare_driver_configured`, `compare_http_client_failed` | The app could not start as the harness requires: the pipeline runtime could not be assembled, the test state has a submission quota, a perplexity score driver, or a PII backstop driver, or the HTTP client could not be built. |
| `compare_candidate_bundle_mismatch` | The candidate tenant's active bundle is not the bundle of this run. A tenant keeps its first bundle, so the database held that tenant already, with other floors. |

### The harness: the run

The first error of a trace ends the loop. The harness prints the label,
writes the report of the pairs that it compared, stops the app, and then
fails. A probe label is different: the harness panics at once.

| Label | Cause |
|---|---|
| `compare_http_failed` | An HTTP call of a driver had a transport error or no answer in 60 s. |
| `compare_database_failed` | A database read of a driver failed, or a row lacks a value that `main` always writes. |
| `compare_candidate_review_failed` | The candidate run was not in `awaiting_review` when a review was due, or no review succeeded in 60 s. |
| `compare_candidate_evidence_incomplete` | The Score evidence lacks a gate field, or a member run has no sealed index command. |
| `compare_baseline_scored_twice` | The baseline has more than one gate decision for the trace, so its index holds the trace twice. |
| `compare_baseline_derived_clear_failed` | An I/O error while the harness removed the baseline tenant's derived files. |
| `compare_record_value_unsafe` | A record holds a text that is not a hash, a risk label, or a safe label. The record is not written. |
| `compare_probe_in_request`, `compare_probe_in_response`, `compare_probe_in_record`, `compare_probe_in_report` | A request body holds a harness token; or a response body, a record, or the report holds a token, the `secret_probe` of the trace, or the start of its text. That record or report is not written. |
| `comparison_report_invalid` | The report could not be built as canonical JSON. |
| `compare_app_stop_failed` | The app stopped before the harness stopped it. |
| `compare_timing_write_failed` | The timing file cannot be created or written. This is one line in the log, and the run continues. |

### The verdict of the harness

| Label | Cause |
|---|---|
| `comparison_alignment_lost` | The run stopped at a pair after which the two indexes can differ. See [Alignment](#alignment). |
| `comparison_receipt_refused` | One side or the two sides refused a receipt. Nothing content-dependent was compared for that trace. |
| `comparison_has_unexplained_differences` | One trace or more has a difference in a compared field. |
| `comparison_gate_branch_not_exercised` | A full run in which a gate branch has no evidence. The report names the branches in `branch_gaps`. |

### The report and the check result

Printed by `pipeline.py` after the harness. Each label of this table is a
defect of the harness or of the tooling, not a difference between the two
sides. Keep the run directory.

| Label | Cause |
|---|---|
| `comparison_report_missing`, `comparison_report_malformed`, `comparison_count_mismatch`, `comparison_partial_mismatch` | The harness passed, but its report is absent or not valid: it is not JSON, it lacks a field or has a field that the harness does not write, a value has the wrong type, or its counts do not agree with each other. |
| `comparison_report_check_mismatch`, `comparison_report_pin_mismatch`, `comparison_report_trace_count_mismatch`, `comparison_report_skew_mismatch`, `comparison_compared_count_mismatch` | The report is not the report of this run: it names another check id, other pin digests than the export manifest, another trace count, or another skew, or a pass compared a number of traces that is not the limit or the trace count. |
| `comparison_check_results_unexpected`, `comparison_skew_with_check_result` | A check result exists where none can: for a partial run, for a report with a skew, or a result that is not the result of this check. |
| `comparison_report_package_mismatch`, `comparison_evidence_malformed`, `comparison_evidence_mismatch`, `comparison_records_missing` | For a full run that passed, the check result does not agree with the report: the package digests differ, the evidence file cannot be read, the evidence does not equal the counts and the two file hashes, or the records file cannot be read. |

An invalid report of a harness that failed is left out, and the step
failure is shown.

### The self-test

| Label | Cause |
|---|---|
| `compare_self_test_pass_incomplete` | Scenario 1 or 4 passed, but it was partial or compared no trace. |
| `compare_self_test_risk_passed` | Scenario 2 passed. The comparison did not find the two admission differences. |
| `compare_self_test_risk_fields` | Scenario 2 failed with a result that is not exactly two `admission` differences in a full run with no refused receipt and no alignment loss. |
| `compare_self_test_skew_passed` | Scenario 3 passed. The comparison did not find the changed floor. |
| `compare_self_test_skew_fields` | Scenario 3 failed, but its report lacks the skew, the alignment position, or `quality_passed`. |
| `compare_self_test_not_deterministic` | Scenario 4 gave a `report_digest` that is not the digest of scenario 1. |

### Labels of the shared tooling

`compare` can also give these labels, which it shares with `pipeline.py run`
and `qualify`:

| Label | Cause |
|---|---|
| `unsupported_pin_schema`, `pin_missing_field` | The pin does not have the schema `trace_commons.pipeline_hf_corpus_pin.v1`, or it lacks a required field, for example `source_digest` or `order_digest`. |
| `step_failed:<step>` | A child step, an export or the harness, exited with a code that is not 0. The line also has `exit=` and `log=`. |
| `cargo_test_list_failed:<step>`, `cargo_filter_matched_zero_tests` | The list step of `cargo test` failed, usually with a compile error, or the name of the harness test matched no test. |
| `hf_<field>_mismatch`, `hf_manifest_contains_raw_trace_text` | A digest of the export manifest is not the digest of the pin (for example `hf_bootstrap_corpus_digest_mismatch`), or the manifest does not say `contains_raw_trace_text: false`. A source or an order that changed fails earlier, in the export itself (`step_failed:compare_export_corpus`). |
| `pipeline_tooling_container_start_failed`, `pipeline_tooling_container_not_ready`, `pipeline_tooling_container_port_unavailable`, `pipeline_tooling_admin_url_invalid`, `pipeline_tooling_server_busy` | The environment could not start. Docker could not start the container, its server did not get ready, or its port could not be read; or the admin URL does not have the host `127.0.0.1` and the user `trace`; or another `pipeline.py` command holds that server. |
| `database_check_executed_nothing:<step>` | The harness passed, but the scenario database shows fewer than 5 committed transactions. |
| `unsupported_report_schema`, `invalid_report_scope`, `payout_enabled`, `missing_local_blockers`, `unsafe_report`, `unsafe_report_field`, `unsafe_report_value`, `private_report_field`, `invalid_report_check_id`, `invalid_report_hash`, `report_digest_mismatch` | The report of a harness that passed fails a check that it shares with a corpus report: the schema, the scope, the payout flag, the blockers, a field name or a value that is not safe, the check id, the form of a hash, or the digest of the report. |
| `check_result_missing:<check id>`, and each other `check_result_*` or `check_evidence_*` label | A full run that passed did not leave a valid, current pass result of its own. See [pipeline-qualification.md](pipeline-qualification.md#the-result-contract-and-what-makes-a-result-invalid). |
| `code_revision_changed` | A file of the tree changed while the command ran. The report is not written under `.local/`. |
| `cleanup_failed` | The environment could not remove its container or its databases. |
| `pipeline_check_environment_invalid`, `pipeline_check_environment_incomplete`, `pipeline_check_already_emitted` | In the log of the harness step. The check result variables are not valid or not complete, or a result of this check id exists already in the results directory. |
| `pipeline_output_directory_unwritable`, `pipeline_output_unwritable` | In the log of the harness step. The harness could not write its report file. |

## The full run

The full run is `compare --corpus` with the network pin and no `--limit`:

```bash
python3 scripts/operator/pipeline.py compare \
  --corpus docs/superpowers/specs/fixtures/versioned-pipeline-comparison-hf-pin-v1.json
```

No report of a full run is committed yet. The committed report of a full
run goes to `docs/superpowers/reports/`, with the build profile that made
it: `compare --corpus` always uses Cargo's optimized build.

### The download

The export reads the session files of the dataset in name order. It keeps
the first 10,000 that the translator accepts and that pass the word filter.
For the network pin, it reads 10,128 files. It downloads each file that is
not in the cache `.local/pipeline/hf-cache/`, 16 files at a time.

The first export of the network pin cannot complete in one run. Hugging
Face limits a client with no token to 3,000 file requests in each window
of 300 s. The server states this policy in a response header:
`ratelimit-policy: "fixed window";"resolvers";q=3000;w=300` (read on
2026-10-08; the limit is theirs and can change). In one window of 3,000
requests, the export got about 1,000 to 1,500 new files, and then a
download failed with `429 Too Many Requests`. The export does not wait for
the next window: it makes one more attempt at once, and then it stops with
exit code 1. The files that it got stay in the cache, and the next run
continues from the cache.

Thus the first export needs several runs of the export command, one for
each window. Run this command from the repository root, with `HF_ENDPOINT`
and `HF_HOME` not set:

```bash
cargo run -q -p trace-commons-server --bin trace-commons-pipeline-corpus-export -- \
  --repository jedisct1/security-audits --revision 6d527ff0081eec6704c2a4f00e1ef8d308ae7366 \
  --split train --translator swival --with-events \
  --cache-dir "$PWD/.local/pipeline/hf-cache" --output-dir "$PWD/.local/pipeline/compare-pin-check-hf" \
  --bootstrap-count 1000 --holdout-count 9000 --min-words 200 --max-words 20000 \
  --expected-source-digest sha256:ebd5a801387ff010376c97b9aadccb2a4802a4e31de4a17fc4d3400d21d859ca \
  --expected-order-digest sha256:a9d891cee558d7278252fe34fc6016fecbfeeced62f5d27d387617139b8d0476
```

A run that fails prints the error of the download. A run that passes
prints the manifest as one JSON line. The two digests in the command are
the digests of the pin, so a pass also proves that the cache holds the
sample of the pin. Before each new run, wait until the window has reset.
This command shows the state of the window:

```bash
curl -sI https://huggingface.co/datasets/jedisct1/security-audits/resolve/6d527ff0081eec6704c2a4f00e1ef8d308ae7366/README.md \
  | grep -i '^ratelimit:'
```

`r=` is the number of requests that are left, and `t=` is the number of
seconds to the reset. Do not start a run when `r` is 0. A run can also stop
with a transport error on a download: run the command again in the next
window. With an empty cache, the download took 11 runs and 57 minutes (the
night of 2026-10-08 to 2026-10-09, UTC). Two of those runs stopped with a
transport error on one file, which the server did not send for about 10
minutes.

After the pass, remove the directory
`.local/pipeline/compare-pin-check-hf/`: its two corpus files hold the
trace text.

`pipeline.py compare --corpus` then reads the same cache, and its export
downloads no session file. With an empty cache, or one that is not full,
the export of `compare` fails in the same way:
`PipelineFailure: step_failed:compare_export_corpus`, with
`429 Too Many Requests` in the log of the step. `pipeline.py` gives a child
neither `HF_ENDPOINT` nor `HF_HOME`.

Also with a full cache, the export asks Hugging Face for the list of the
files, so it needs the network.

### Time and memory

These values are from one machine, a 12-core server, with the optimized
build and a full cache. They are measurements of two partial runs, not of
the full run.

| Run | Wall time |
|---|---|
| `--limit 100`, no build | 59 s |
| `--limit 1000`, no build | 6 min 14 s |

- A run that must build the optimized test binary first takes about 3.5
  minutes more.
- The calibration of the 1,000 bootstrap traces takes about 7 s. Each run
  does it, also with `--limit`.
- One trace takes about 350 ms: about 75 ms for the baseline receipt, 27 ms
  for the baseline gate call, 52 ms for the candidate receipt, 186 ms for
  the candidate wait, and 7 ms for the reads.
- The time of one trace shows no clear growth up to 1,000 traces: the mean
  of each block of 100 traces stays between 338 and 360 ms.
- The peak memory of the largest child process is 139 MB, for 100 traces
  and for 1,000 traces.
- Two runs of `--limit 100` gave the same `report_digest` and the same
  `records_digest`.

The full run of 10,000 traces is estimated at 1 to 1.5 hours. This is an
estimate from the 1,000-trace run, not a measurement. Some costs grow with
the number of traces: the two index scans, and the file reads of the old
path. They are small at 1,000 traces and are not measured beyond it.

### Disk space

| What | Size |
|---|---|
| The cache `.local/pipeline/hf-cache/` with the 10,128 session files | 478 MB |
| The two corpus files of one export | 56 MB (bootstrap) and 289 MB (holdout) |
| The optimized build under `target/release/` | about 2.2 GB |
| One run directory, `--limit 100` | 358 MB |
| One run directory, `--limit 1000` | 638 MB |

Each run makes its own export, so each run directory holds a full copy of
the two corpus files under `compare/corpus/` (344 MB). It also holds the
encrypted artifacts of the two tenants under `artifacts/`: about 290 MB for
each 1,000 traces. A run directory of the full run is thus about 3.3 GB;
this is an estimate from the two measured runs.

Nothing under `.local/pipeline/runs/` is deleted automatically. After a
run, remove the corpus files under `runs/<id>/compare/` and the directory
`runs/<id>/artifacts/`:

```bash
RUN=q0af28b7a  # the run id: `run=` in the PipelineCompareOK line
rm -r ".local/pipeline/runs/$RUN/compare" ".local/pipeline/runs/$RUN/artifacts"
```

Keep the report, the records file, the timing file, the logs, and
`results/`: they hold no trace text.
