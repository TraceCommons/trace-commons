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
compared value must be exactly equal. One named rule permits one known
difference (see [The permitted differences](#the-permitted-differences)).
Any other difference fails the run. The design is in
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
| [`pin-local-risk.json`](../../crates/trace-commons-server/tests/fixtures/pipeline-compare-jsonl/pin-local-risk.json) | 8 (4 bootstrap, 4 holdout) | A local pin with one declared `medium` trace and one declared `high` trace. It must pass with one permitted pair, the `medium` trace by `medium_risk_privacy_review`. The `high` trace is quarantined on the two sides and compares equal. `--self-test` uses it. |
| [`versioned-pipeline-comparison-hf-pin-v1.json`](../superpowers/specs/fixtures/versioned-pipeline-comparison-hf-pin-v1.json) | 10,000 (1,000 bootstrap, 9,000 holdout) | The network pin: it has no `local_jsonl_dir`, and its export reads the public Hugging Face dataset `jedisct1/security-audits`. Word filter 200 to 20,000. Check id `pipeline_comparison_hf`. See [The full run](#the-full-run). |

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

The harness builds one envelope for each trace. The contributor-side
redaction runs first, as it does in pilot-bootstrap, and the envelope keeps
the privacy risk that this redaction gives. Only a local pin can declare a
risk (`declared_privacy_risk`). For such a trace, the harness sends a
metadata-only envelope with the declared risk.

A pass prints one line to stdout and exits with 0:

```text
PipelineCompareOK: traces=10 equal=10 permitted=0 unexplained=0 partial=false seconds=12 run=q0af28b7a report=.local/pipeline-comparison-pipeline_comparison_local.json
```

`seconds=` is the time of the harness step with its build and its database
setup. It is not the time of the comparison. The timing file of the run
(`compare_run-timing.jsonl`) has the time of each trace.

A run that fails after the harness wrote a valid report prints the run and
the report, and then the failure label:

```text
PipelineCompareReport: unexplained=1 alignment_lost=none partial=false run=q0af28b7a report=.local/pipeline-comparison-pipeline_comparison_local-failed.json
PipelineFailure: comparison_has_unexplained_differences
```

`partial=true` without `--limit` and with `alignment_lost=none` means that
an error ended the run early. The label of the error is in
`logs/compare_run.log` of that run.

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

`--self-test` proves that the comparison permits the one ruled difference
and fails on any other. Scenario 3 is the run that shows the failure. It
runs the harness four times in one environment, with the two local pins, and
it passes only when each run gives its expected result:

1. `pin-local.json` passes and compares each of its traces.
2. `pin-local-risk.json` passes and compares each of its traces, with
   exactly one permitted pair and nothing else. The baseline accepts
   the declared `medium` trace and the candidate quarantines it. The rule
   `medium_risk_privacy_review` permits this pair. The two sides quarantine
   the declared `high` trace, and this pair is equal. The `quarantine` count
   is 1 on the baseline and 2 on the candidate, and no side rejects a trace.
   The report has no unexplained difference. The alignment keeps the two
   indexes equal, so no later trace differs and the run does not stop early.
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

After the build, `compare --corpus` on `pin-local.json` takes about 12 s
and `compare --self-test` takes about 45 s, on one 12-core server. The time
of the build and the times of the network pin are in
[Time and memory](#time-and-memory).

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
`unexplained` with the field names. The run passes only when no trace is
`unexplained`.

### The permitted differences

One rule permits a difference in a compared field. The rule permits the
difference in `admission` only.

| Rule | Field | Source |
|---|---|---|
| `medium_risk_privacy_review` | `admission` | ruling PC-D22 (2026-10-09) |

**`medium_risk_privacy_review`.** The old path accepts a trace with a
`medium` risk, because the harness
sets the state field that has the same effect as
`TRACE_COMMONS_ACCEPT_MEDIUM_RISK_SUBMISSIONS=true` (it sets the field in
code and reads no environment variable). The pipeline
quarantines the same trace for review. The owner ruled that this is
intended.

The rule permits a pair only when all of these are true:

- The field is `admission`.
- `privacy_risk` is `medium` on the two records.
- `privacy_basis` is equal on the two records, and it is not exactly
  `["consent_content_flag"]`. An empty basis meets this too: a declared
  risk has no basis label, and the permitted pair of `--self-test` (the
  declared medium trace) is such a pair.
- The baseline admission is `admit` and the candidate admission is
  `quarantine`.

A trace whose risk is High at the receipt has no rule. The old path
quarantines it, and the pipeline quarantines it too (ruling PC-D27,
2026-10-11). The two sides compare equal. A pipeline that rejects such a
trace gives an `unexplained` difference in `admission`.

A pair that meets the condition of the rule and has no other difference is
`permitted`. A pair with a difference in another field is `unexplained`
with that other field only. A pair that does not meet the condition of the
rule stays `unexplained`: the reverse direction, another risk, a basis that
differs between the sides, or another pair of decisions.

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
| `safe_blockers` | always `local_test_only`, `local_reference_scorer`, `local_reference_embedder`, `synthetic_index`, `synthetic_settlement`, `static_bearer_authentication`, `deterministic_privacy_only`, `baseline_derived_scan_removed`, `duplicate_short_circuits_not_compared`, `review_start_privacy_pass_not_compared` (see [Limits](#limits)). A report without one of them is refused (`missing_local_blockers`). |
| `pin` | the five digests of the export manifest: `source_digest`, `order_digest`, `configuration_digest`, `bootstrap_corpus_digest`, `holdout_corpus_digest` |
| `bundle_id`, `package_hash`, `configuration_digest`, `dependency_digest` | the candidate's compatibility package. The bundle holds the floors, so the bundle id changes with the floors. |
| `floors` | the three derived floors, in micros |
| `trace_count` | the number of traces in the pin (`sample_count` of the manifest) |
| `compared_count` | the number of traces that the run compared |
| `equal_count` | the traces with the result `equal` |
| `permitted_counts` | for each rule, the traces that it permitted. A key is the id of a rule in `permitted_rules`. A trace that two rules permit counts for each of them. |
| `permitted_total` | the traces with the result `permitted`. A trace counts one time, also when several rules permit it. |
| `unexplained_counts` | for each compared field, the traces in which it differs |
| `unexplained_total` | the traces with the result `unexplained` |
| `unexplained` | the first 1,000 unexplained traces in sample order, each with `position`, `trace_hash`, and `fields` |
| `first_unexplained_position` | the position of the first unexplained trace, or `null` |
| `alignment_lost_position` | the position of the pair at which the run stopped, or `null` |
| `excluded_rules` | the three rules of [What is compared](#what-is-compared), each with `rule`, `source`, and `fields`. A report with another list is refused (`comparison_report_malformed`). |
| `permitted_rules` | the closed list of the rules that permit a difference (see [The permitted differences](#the-permitted-differences)), each with `rule`, `source`, and `fields`. A report with another list is refused (`comparison_report_malformed`). |
| `distribution` | for `baseline` and for `candidate`: the counts `admit`, `quarantine`, `reject`, `refused`, `other`, `scored`, `quality_passed`, `quality_failed`, `novelty_passed`, `novelty_failed`, `member`, `not_member`, `chunks_capped`. `member` and `not_member` count scored traces only. On each side the five admission counts add up to `compared_count`, each pass and fail pair and `member` with `not_member` add up to `scored`, and `chunks_capped` is at most `scored`; a report in which they do not is refused (`comparison_count_mismatch`). |
| `branch_gaps` | the gate branches with no evidence on the baseline side: any of `quality_passed_true`, `quality_passed_false`, `novelty_passed_true`, `novelty_passed_false`, `member_true`, `member_false`. Always empty for a partial run. |
| `records_digest` | the SHA-256 of the records file |
| `report_digest` | the SHA-256 of the canonical report without this field |

`equal_count`, `permitted_total`, and `unexplained_total` add up to
`compared_count`. A position counts through the two partitions, from
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
2. Run this command. A position counts through the two files, bootstrap
   first, from 0. The command prints the file and the line number of the
   position, and the hash of the `trace_id` of that line, which must equal
   `trace_hash`. It prints no trace text.

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

The two records of the trace are the lines `2 * position + 1` (baseline)
and `2 * position + 2` (candidate) of the records file.

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

The report under `.local/` is the evidence of a run only when its SHA-256
is the `report_hash` in `results/<check id>.evidence.json` of the run
directory that the `PipelineCompareOK` line names. The result file in that
directory has the code revision hash.

A full run also needs a pin with all five digests. A pin that lacks one of
`configuration_digest`, `bootstrap_corpus_digest`, and
`holdout_corpus_digest` can only make a partial run
(`comparison_pin_digest_missing`).

### Where the files are

The latest report of `compare --corpus` goes under `.local/`, with a
Markdown view beside it:

| File | Content |
|---|---|
| `.local/pipeline-comparison-<check id>.json` and `.md` | The latest report of a full run that passed. |
| `.local/pipeline-comparison-<check id>-partial.json` and `.md` | The latest report of a partial run: a run with `--limit`, or a run that stopped early. |
| `.local/pipeline-comparison-<check id>-failed.json` and `.md` | The latest full report of a run that failed. |

The line `PipelineCompareOK` is the proof that the run passed. It is
evidence only when it says `partial=false`.

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

The machine and the checkout path also matter. The redactor replaces the
home path and the path of the current directory in trace text, and the
current directory of the harness is the crate directory of the checkout. A
run on another machine, or from a checkout at another path, can thus give
other floors and other digests for a sample that contains such a path. The
two sides change together.

## Limits

What a passing report does not show:

- **Reference scorers.** The two sides use `ReferencePerplexityScorer` and
  `ReferenceEmbedder`. The result says nothing about gate quality or about
  a real scorer.
- **No classifier and no backstop.** The two sides use the deterministic
  rescrub only. The prose classifier and the PII backstop are off (blocker
  `deterministic_privacy_only`).
- **No Review-start privacy pass.** The pipeline runs the prose classifier
  again in a privacy pass at the start of Review (#1324). The harness reads
  the candidate's `privacy_risk`, `privacy_basis`, and `admission` at the
  receipt, before the pass, so the pass is not compared (blocker
  `review_start_privacy_pass_not_compared`, PC-D25). With the deterministic
  boundary of this tool the pass changes nothing. With a real classifier, a
  pass that escalates parks the candidate run for a human, and the run
  stops with `comparison_alignment_lost`: it fails, but the label names
  the alignment and not the privacy pass.
- **No duplicate short-circuits.** `main` records a duplicate as
  `skipped_duplicate` or `cached`, with no credit quality: the old path in
  its in-process perplexity score driver, and the pipeline at Settle since
  #1325. The run applies them on neither side (blocker
  `duplicate_short_circuits_not_compared`, PC-D24). The harness calls the
  old gate through `POST /v1/workers/gate/evaluate`, so the score driver
  does not run, and it assembles the candidate runtime with no
  `duplicate_controls`. The two sides also read `credit_quality_micros`
  from different places: the candidate from its Score evidence, the
  baseline from its gate decision row. For a duplicate these differ since
  #1325, so a passing report says nothing about duplicates.
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
- **No dedup signals on the baseline.** The harness database has no
  gate-driver pool, so the baseline's gate call reads no dedup signal row.
  Each baseline decision is thus its own dedup cluster. No compared field
  depends on it (rule `shadow_values_not_in_contract`).
- **An exact baseline index.** The baseline index is `MockVectorIndex`,
  which compares a query with each stored vector, in memory. The two gate
  services of `main` with a real scorer, `enclave_local_gpu` and
  `enclave_near_ai`, use `UsearchVectorIndex`
  (`crates/trace-commons-server/src/bin/trace-commons-ingest.rs`). The
  report says nothing about that index.
- **Not the configuration of a deployed server.** The harness builds the
  baseline state in code, and it does not start the server through the
  production start function, which reads the configuration from the
  environment. One gate configuration gives the two sides: the derived
  floors, `top_k` 5 (the default of `main`, `TRACE_COMMONS_GATE_DEFAULT_TOP_K`),
  and the other values of `CompatibilityBundleConfig::local_reference()`
  (insert threshold 50,000 micros, chunk target 2,048 tokens, chunk maximum
  3,072 tokens, chunk cap 16, chunk minimum 64 tokens). Two values are not
  the default values of `main`
  (`crates/trace-commons-server/src/bin/trace-commons-ingest.rs`):
  - The credit for novelty is 2.5 credits on the two sides, so that the
    credit branch gets evidence. The default of `main` is 0
    (`DEFAULT_NOVELTY_UTILITY_CREDIT_POINTS_DELTA`).
  - The baseline accepts medium-risk submissions, as
    `deploy/pilot-gcp/ingest.env.template` sets it. The default of `main`
    is to quarantine them (`TRACE_COMMONS_ACCEPT_MEDIUM_RISK_SUBMISSIONS`
    not set). The rule `medium_risk_privacy_review` permits the resulting
    `admission` difference.

  A deployment that sets other values has a different old path, and the
  report says nothing about it.
- **An early stop.** A run stops at the first pair after which the two
  indexes can differ (see [Alignment](#alignment)). The traces after that
  pair are not compared, and the report is partial unless that pair is the
  last trace of the pin.
- **No continuation.** A run cannot continue from a position. Each run
  starts at position 0 with empty indexes.
- **A trace that the redaction refuses.** The contributor-side redaction
  can refuse a session, for example for a credential in a metadata field.
  A refused bootstrap trace stops each run of the pin in the calibration,
  with no report. A refused holdout trace stops the run at that trace, with
  a partial report. The label is `compare_envelope_failed`, in the log of
  the harness step. Such a pin cannot make a full run.
- **Time limits.** Each HTTP call of the harness has a limit of 60 s, and
  the candidate side of one trace has a limit of 60 s.
- **The product surfaces.** The receipt body, the status, the credit
  summary, dedup, and withdrawal are not compared.

## When a run fails

`pipeline.py` never prints a child command's own output. A failure is one
safe label on stderr, as `PipelineFailure: <label>`. See
[pipeline-lab.md](pipeline-lab.md#failure-labels) for the form of the line.
Each label of `compare` is in
[pipeline-comparison-labels.md](pipeline-comparison-labels.md), with its
cause.

One refusal does not have this form. `pipeline.py` refuses an argument that
looks like a URL before it starts a run; only the value of
`--postgres-admin-url` can be a URL. Today this refusal ends with a Python
traceback that names `argument_looks_like_url`, not with a
`PipelineFailure` line.

The harness is a child. `pipeline.py` shows a harness failure in one of two
ways:

- The harness left a valid report that names a failure. `pipeline.py`
  prints that label (one of the four labels of
  [When a run is evidence](#when-a-run-is-evidence)).
- Each other harness failure is
  `PipelineFailure: step_failed:compare_run exit=<n> log=<run dir>/logs/compare_run.log`.
  The label of the cause is in that log, in the panic message.

## The full run

The full run is `compare --corpus` with the network pin and no `--limit`:

```bash
python3 scripts/operator/pipeline.py compare \
  --corpus docs/superpowers/specs/fixtures/versioned-pipeline-comparison-hf-pin-v1.json
```

The committed report of a full run is in `docs/superpowers/reports/`, with
the build profile that made it: `compare --corpus` always uses Cargo's
optimized build. The first one is the run of 2026-10-11:
[`2026-10-11-pipeline-comparison-10k.md`](../superpowers/reports/2026-10-11-pipeline-comparison-10k.md),
with its `.json`, `.result.json`, and `.evidence.json` files. Of 10,000
traces, 9,853 pairs are equal, 147 pairs are permitted by
`medium_risk_privacy_review`, and no pair is unexplained.

The full run is long (see [Time and memory](#time-and-memory)):

- Commit your work first, and change no file of the checkout during the run.
  The code revision hash covers each file that can change what the server
  does or what a qualification run finds. The list is in the entry
  `pipeline.py revision` of
  [Signed check results](pipeline-qualification.md#signed-check-results).
  A run that passed fails with `code_revision_changed` if the hash at its
  end differs from the hash at its start.
- The command prints nothing until its end. For the progress, count the
  lines of `compare_run-timing.jsonl` in the newest run directory: one line
  for the calibration, then one line for each trace.
- Ctrl-C ends the command with exit code 130 and no line. The command still
  removes its databases and its container. A hang-up or a `kill` runs no
  cleanup, so use a terminal multiplexer. Remove a container that stayed
  with `docker rm -f tc-pipeline-<run id>`. With `--postgres-admin-url`
  there is no container, and two kinds of database stay on that server.
  First, drop the database `pipeline_tooling_lock`, or the next command
  fails with `pipeline_tooling_server_busy`. Second, drop the databases of
  the scenarios of the run. A scenario has the databases
  `admission_test_<run8>_<NN>` and `pipeline_test_<run8>_<NN>`. A scenario
  can also have `admission_test_<run8>_<NN>_pilot` and
  `admission_test_<run8>_<NN>_restored`, and the cleanup drops these too if
  they exist. `<run8>` is the run id without its first letter `q`. The run id
  is the name of the directory of the run that stopped under
  `.local/pipeline/runs/`. Several runs can have a directory there. Find the
  run that stopped, for example from the time of its directory.
  `<NN>` is the number of the scenario with two digits: a `compare --corpus`
  run has `01`, and a `compare --self-test` run has `01` to `04`.
  The list command below shows each database of the server whose name starts
  with `admission_test_` or `pipeline_test_`, also those of other runs and
  of local tests. Drop only the names that contain the 8 characters
  `<run8>` of the run that stopped. To list the databases, run `psql postgres://trace@127.0.0.1:<port>/postgres -qtA -c "SELECT datname FROM pg_database WHERE datname LIKE 'admission\_test\_%' OR datname LIKE 'pipeline\_test\_%'"`.
  To remove one, run `DROP DATABASE "<name>" WITH (FORCE);` on that server.
- The optimized build had a peak memory of about 7.7 GB on the 12-core
  server of the measurements.

### The download

The export reads the session files of the dataset in name order. It keeps
the first 10,000 that the translator accepts and that pass the word filter.
For the network pin, it reads 10,128 files. It downloads each file that is
not in the cache `.local/pipeline/hf-cache/`, 4 files at a time. The
download window is small so that the export sends fewer concurrent requests to
Hugging Face while the export does not wait after an HTTP 429 (issue #1308).
The measured times of this section were taken with a download window of 16.
They are not repeated with a download window of 4.

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
and `HF_HOME` not set. It uses the optimized build, as the export of
`compare --corpus` does:

```bash
cargo run -q --release -p trace-commons-server --bin trace-commons-pipeline-corpus-export -- \
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
build and a full cache.

| Run | Wall time |
|---|---|
| `--limit 100`, no build | 59 s |
| `--limit 1000`, no build | 6 min 14 s |
| The full run (10,000 traces), no build | 1 h 4 min 25 s |

- A run that must build the optimized test binary first takes 3.5 to 5
  minutes more.
- The calibration of the 1,000 bootstrap traces takes about 7 s. Each run
  does it, also with `--limit`.
- The peak memory of the largest child process of a run with no build is
  139 MB, for 100 traces and for 1,000 traces, and 140 MB for the full run.

The full run of 2026-10-11 took 1 h 4 min 25 s. The two index scans grow
with the number of traces, and so do two reads of the old path: the audit
log file and the credit event file of the tenant. In that run the growth
was small: the measured steps of the last 1,000 traces took 396 s, and
those of the first 1,000 traces took 383 s.

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
each 1,000 traces. The run directory of the full run of 2026-10-11 is
3.0 GB: 2.7 GB under `artifacts/` and 329 MB under `compare/`.

Nothing under `.local/pipeline/runs/` is deleted automatically. After a
run, remove the corpus files under `runs/<id>/compare/` and the directory
`runs/<id>/artifacts/`:

```bash
RUN=q0af28b7a  # the run id: `run=` in the PipelineCompareOK line
rm -r ".local/pipeline/runs/$RUN/compare" ".local/pipeline/runs/$RUN/artifacts"
```

Keep the report, the records file, the timing file, the logs, and
`results/`: they hold no trace text.
