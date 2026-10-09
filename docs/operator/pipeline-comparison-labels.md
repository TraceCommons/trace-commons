# Failure labels of `pipeline.py compare`

This page gives each failure label of `pipeline.py compare` with its cause.
The runbook is [pipeline-comparison.md](pipeline-comparison.md). Its section
[When a run fails](pipeline-comparison.md#when-a-run-fails) says how a
failure shows: as a label that `pipeline.py` prints, or as
`step_failed:compare_run` with the label in the log of the step.

A compile error is `cargo_test_list_failed:compare_run`. A failed export is
`step_failed:compare_export_corpus`, and its log holds the export's own
message. `--self-test` has the step names of
[Where the files are](pipeline-comparison.md#where-the-files-are) in place
of `compare_run`, and `compare_export_local` and `compare_export_risk` for
its two exports.

A report can name one of four failures: `comparison_alignment_lost`,
`comparison_receipt_refused`, `comparison_has_unexplained_differences`, and
`comparison_gate_branch_not_exercised`. Their conditions and their order
are in
[When a run is evidence](pipeline-comparison.md#when-a-run-is-evidence).

## The command line and the pin

Printed by `pipeline.py` before the export:

| Label | Cause |
|---|---|
| `compare_corpus_or_self_test_required` | Neither `--corpus` nor `--self-test` was given. |
| `compare_corpus_and_self_test_conflict` | `--corpus` and `--self-test` were given together. |
| `compare_self_test_takes_no_option` | `--limit` was given with `--self-test`. |
| `compare_limit_invalid` | `--limit` is below 1. The harness also gives this label for a limit variable that is not a positive integer. |
| `comparison_pin_unreadable` | The pin file cannot be read, or it is not a JSON object, or one of its fields `local_jsonl_dir`, `session_names`, and `declared_privacy_risk` is not a string, a list of strings, and a map of strings. |
| `comparison_pin_without_events` | The pin does not have `with_events: true`. |
| `comparison_pin_digest_missing` | The pin lacks one of `configuration_digest`, `bootstrap_corpus_digest`, and `holdout_corpus_digest`, and the run is not a partial run. |
| `comparison_pin_field_needs_local_dir` | The pin has `session_names` or `declared_privacy_risk` and no `local_jsonl_dir`. |

## The export

Printed by `pipeline.py` after the export:

| Label | Cause |
|---|---|
| `comparison_manifest_malformed` | The export left no `source-manifest.json`, or the manifest is not a JSON object, or it lacks one of the five digests or a whole `sample_count`. |
| `comparison_manifest_without_events` | The manifest does not say that the export had `--with-events`. |

## The harness: before the first trace

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
| `compare_envelope_failed` | The contributor-side redaction refused a trace or gave an error, in the calibration or in the run, or the envelope of a trace of the run could not be serialized. |
| `comparison_calibration_failed`, `comparison_calibration_invalid`, `comparison_calibration_empty` | The calibration could not measure the bootstrap partition: an envelope could not be serialized or a gate evaluation failed, the three lists of values do not have one length, or the partition has no trace. |
| `comparison_floors_all_zero` | Each of the three derived floors is zero. |
| `compare_pipeline_assembly_failed`, `compare_submission_quota_enabled`, `compare_driver_configured`, `compare_http_client_failed` | The app could not start as the harness requires: the pipeline runtime could not be assembled, the test state has a submission quota, a perplexity score driver, or a PII backstop driver, or the HTTP client could not be built. |
| `compare_candidate_bundle_mismatch` | The candidate tenant's active bundle is not the bundle of this run. A tenant keeps its first bundle, so the database held that tenant already, with other floors. |

## The harness: the run

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

## The report and the check result

Printed by `pipeline.py` after the harness. Each label of this table is a
defect of the harness or of the tooling, not a difference between the two
sides. Keep the run directory.

| Label | Cause |
|---|---|
| `comparison_report_missing`, `comparison_report_malformed`, `comparison_count_mismatch`, `comparison_partial_mismatch` | The harness passed, but its report is absent or not valid: it is not JSON, it lacks a field or has a field that the harness does not write, a value has the wrong type, its list `permitted_rules` is not the closed list (rule `medium_risk_privacy_review`, source `ruling.PC-D22`, field `admission`), `permitted_counts` names another rule, or its counts do not agree with each other. |
| `comparison_report_check_mismatch`, `comparison_report_pin_mismatch`, `comparison_report_trace_count_mismatch`, `comparison_report_skew_mismatch`, `comparison_compared_count_mismatch` | The report is not the report of this run: it names another check id, other pin digests than the export manifest, another trace count, or another skew, or a pass compared a number of traces that is not the limit or the trace count. |
| `comparison_check_results_unexpected`, `comparison_skew_with_check_result` | A check result exists where none can: for a partial run, for a report with a skew, or a result that is not the result of this check. |
| `comparison_report_package_mismatch`, `comparison_evidence_malformed`, `comparison_evidence_mismatch`, `comparison_records_missing` | For a full run that passed, the check result does not agree with the report: the package digests differ, the evidence file cannot be read, the evidence does not equal the counts and the two file hashes, or the records file cannot be read. |

An invalid report of a harness that failed is left out, and the step
failure is shown.

## The self-test

| Label | Cause |
|---|---|
| `compare_self_test_pass_incomplete` | Scenario 1 or 4 passed, but it was partial or compared no trace. |
| `compare_self_test_risk_passed` | Scenario 2 passed. The comparison did not find the unexplained `admission` difference of the declared `high` trace. |
| `compare_self_test_risk_fields` | Scenario 2 failed with a result that is not exactly one unexplained `admission` difference and one pair permitted by `medium_risk_privacy_review`, in a full run with no refused receipt and no alignment loss. |
| `compare_self_test_skew_passed` | Scenario 3 passed. The comparison did not find the changed floor. |
| `compare_self_test_skew_fields` | Scenario 3 failed, but its report lacks the alignment position or `quality_passed`. |
| `compare_self_test_not_deterministic` | Scenario 4 gave a `report_digest` that is not the digest of scenario 1. |

## Labels of the shared tooling

`compare` can also give labels that it shares with `pipeline.py run` and
`qualify`. [pipeline-lab.md](pipeline-lab.md#failure-labels) has the form
of the failure line and the labels of a step.
[pipeline-qualification.md](pipeline-qualification.md) has the
[environment](pipeline-qualification.md#the-environment-container-digest---postgres-admin-url-one-server-at-a-time)
and the
[result contract](pipeline-qualification.md#the-result-contract-and-what-makes-a-result-invalid).
The shared labels, by the part that gives them:

| Part | Labels |
|---|---|
| The pin | `unsupported_pin_schema`, `pin_missing_field` |
| A child step | `step_failed:<step>`, `cargo_test_list_failed:<step>`, `cargo_filter_matched_zero_tests` |
| The export manifest | `hf_manifest_contains_raw_trace_text` |
| The environment | `pipeline_tooling_container_start_failed`, `pipeline_tooling_container_not_ready`, `pipeline_tooling_container_port_unavailable`, `pipeline_tooling_admin_url_invalid`, `pipeline_tooling_server_busy`, `pipeline_tooling_sql_failed`, `cleanup_failed` |
| The transaction count after a harness that passed | `database_check_executed_nothing:<step>` |
| The report, with the checks of a corpus report | `unsupported_report_schema`, `invalid_report_scope`, `payout_enabled`, `missing_local_blockers`, `unsafe_report`, `unsafe_report_field`, `unsafe_report_value`, `private_report_field`, `invalid_report_check_id`, `invalid_report_hash`, `report_digest_mismatch` |
| The check result of a full run that passed | `check_result_missing:<check id>`, each other `check_result_*` or `check_evidence_*` label, `unsafe_evidence_field`, `unsafe_evidence_value` |
| The log of the harness step | `pipeline_check_environment_invalid`, `pipeline_check_environment_incomplete`, `pipeline_check_already_emitted`, `pipeline_output_directory_unwritable`, `pipeline_output_unwritable` |

Two shared labels have a cause that is true only for `compare`:

| Label | Cause |
|---|---|
| `hf_<field>_mismatch` | A digest of the export manifest is not the digest of the pin (for example `hf_bootstrap_corpus_digest_mismatch`). `compare` examines all five digests of the pin. A source or an order that changed fails earlier, in the export itself (`step_failed:compare_export_corpus`). |
| `code_revision_changed` | At the end of a `compare --corpus` run that passed each other check, the hash of the tree is not the hash at the start of the command: a file changed while the command ran. The report is not written under `.local/`. A run that failed earlier does not make this check. |
