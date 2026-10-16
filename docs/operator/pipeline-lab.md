# Local pipeline corpus runs

> **Status: the tooling exists.** `pipeline.py run` and `pipeline.py package`
> run today against a real, disposable PostgreSQL server and the shared
> `trace-commons-ingest` app. There is no separate lab binary or service:
> `run` hosts the app in-process for the run's duration, through an ignored
> Rust test, and tears everything down when it finishes. Production
> routing of live tenants through the pipeline is still off; see
> [pipeline-activation.md](pipeline-activation.md).

Use `pipeline.py run` to send a fixed corpus through the shared
`trace-commons-ingest` app over real HTTP and check the graded result.
Use `pipeline.py package` to build and sign a reusable bundle package first.
`pipeline.py qualify` (see [pipeline-qualification.md](pipeline-qualification.md))
runs these same corpus checks as part of the full required-check set.

## Run a corpus

```bash
python3 scripts/operator/pipeline.py run --bundle minimal
python3 scripts/operator/pipeline.py run --bundle compatibility
```

`--bundle` selects one of the two built-in bundles (`minimal` or
`compatibility`); pass a signed package instead with `--package PATH
--trusted-key PATH` (the outputs of `pipeline.py package` below) -- exactly
one of `--bundle` or the package pair is required. `--corpus PATH` selects
another corpus file (default:
[`versioned-pipeline-minimal-corpus-v1.json`](../superpowers/specs/fixtures/versioned-pipeline-minimal-corpus-v1.json),
five fixtures: `clean_tool_plan`, `locally_redacted_secret`,
`privacy_quarantine_approved`, `privacy_quarantine_rejected`,
`privacy_high_risk_review_rejected`; the last is a High-risk receipt
Admission holds for review, which the harness's reviewer then rejects). `--corpus-digest sha256:HEX` refuses a corpus file
whose bytes do not hash to that digest -- checked before any database
starts. `--postgres-admin-url URL` and `--archive` work as in
[pipeline-qualification.md](pipeline-qualification.md).

The command checks the corpus file, starts one `Environment` (its own
scenario database and encrypted artifact root), runs every fixture through
the shared ingest app in array order over real HTTP, requires the check to
show at least 5 committed transactions and a current pass result, and
writes the latest report under `.local/`. Corpus fixture order, labels,
trace ids, and submission ids must be unique; a report file is refused as a
corpus input.

## The HF pin

`--corpus` also accepts an HF pin descriptor
(`trace_commons.pipeline_hf_corpus_pin.v1`) in place of a direct corpus
file. `qualify`'s third corpus run always uses the local fixture pin:

```bash
python3 scripts/operator/pipeline.py run --bundle compatibility \
  --corpus crates/trace-commons-server/tests/fixtures/pipeline-hf-jsonl/pin-local.json
```

This pin names a `local_jsonl_dir`
(`crates/trace-commons-server/tests/fixtures/pipeline-hf-jsonl/`, two
committed `.jsonl` files) instead of a live Hugging Face download: the
export binary (`trace-commons-pipeline-corpus-export`) reads the fixture
files from that directory and checks the result against the pin's own
`source_digest` and `order_digest` fields, the same way it would check a
real remote export. No PR 4 check makes a network call.

Every PR 4 pin, including this one, names the dataset
`jedisct1/security-audits` as its `repository` field for consistency with
the network pin. This runbook does not print `pin-local.json`'s digest
fields; read the file itself if you need them.

The network pin -- the one whose export downloads from Hugging Face -- is
[`versioned-pipeline-comparison-hf-pin-v1.json`](../superpowers/specs/fixtures/versioned-pipeline-comparison-hf-pin-v1.json),
the 10,000-trace sample of `pipeline.py compare`; see
[pipeline-comparison.md](pipeline-comparison.md#the-full-run). It is for
`compare` only. `pipeline.py run --corpus` cannot use it: the run fails at
the corpus digest check (`hf_bootstrap_corpus_digest_mismatch`), because
the corpus digests of that pin are those of the `--with-events` export.

## Build a package

```bash
python3 scripts/operator/pipeline.py package --bundle minimal \
  --output .local/lab/package.json --public-key-output .local/lab/trusted-key.json
python3 scripts/operator/pipeline.py run --bundle minimal \
  --package .local/lab/package.json --trusted-key .local/lab/trusted-key.json
```

By default this builds a disposable Ed25519 key and writes only its public
half. Pass `--signing-key PATH --key-id LABEL` for a controlled key
(`PATH` must be Ed25519 PKCS#8 DER). `--output` and `--public-key-output`
must differ. Building and signing a package does not qualify it for
production -- see
[pipeline-qualification.md](pipeline-qualification.md#package-trust-and-qualify_bundle).

## Report fields

Each corpus run writes `trace_commons.pipeline_corpus_report.v1` to the run
directory and the latest copy to `.local/pipeline-<label>-corpus-report.json`
(plus a `.md` rendering), where `<label>` is the bundle name, `package`, or
`hf_local`. The report's `report_digest` is the SHA-256 of its own canonical
bytes with that field excluded (timing fields are never in the report at
all, so digests are stable across runs of the same bundle and corpus).

Top-level fields: `schema`, `check_id` (`pipeline_http_corpus_<label>`),
`scope: "local_test"`, `production_ready: false`,
`external_payout_enabled: false`, `safe_blockers` (exactly
`local_test_only`, `local_reference_scorer`, `local_reference_embedder`,
`synthetic_index`, `synthetic_settlement`, `static_bearer_authentication`),
`bundle_id`, `package_hash`, `configuration_digest`, `dependency_digest`,
`policy_manifest` (one entry per phase -- `admission`, `review`, `score`,
`settle` -- each with its `implementation_id` and `configuration_hash`),
`fixture_count`, `completed_fixture_count`, `failure_count`,
`replay_same_run_count`, `changed_content_refused_count`,
`tenant_isolation`, and `partitions`: one section per corpus partition
(`corpus`, or `bootstrap` then `holdout` for an HF pin), each with its own
`corpus_digest` and ordered `fixtures`. Each fixture carries its admission
decision against the expectation, its final state, outcome and instrument
counts against expectations, the replay and changed-content-refusal checks,
tenant isolation, and a `mismatches` list -- empty on a passing fixture,
naming exactly which expectation failed otherwise. A report with recorded
fixture failures is still a *valid* report; `pipeline.py` keeps a failed
harness run's report (when it validates) and prints
`PipelineRunReport: failures=<n> report=<path>` before re-raising the step
failure.

## Isolation and privacy

Every corpus run gets its own scenario database and its own encrypted
artifact root, created fresh and dropped when the environment closes.
Nothing here ever points at `DATABASE_URL`, a shared development database,
or any variable but `TRACE_COMMONS_PG_TEST_DATABASE_URL` /
`--postgres-admin-url`. The artifact master key is random per corpus run
and is only ever set in the child process's own environment
(`TRACE_COMMONS_PIPELINE_TEST_MASTER_KEY_HEX`); `pipeline.py` never logs it
and never prints a child's stdout or stderr, only the step label, exit
code, and protected log path.

Fixture secret probes never leave the run. Every corpus fixture is required
to carry a non-empty `secret_probe`, and the report validator refuses any
report containing a raw UUID (hashed instead), any of the private field
names (`input`, `text`, `trace_text`, `secret`, `secret_probe`, `token`,
`account_id`, `email`), or a secret-shaped string value -- the same rule
`validate_evidence` applies to check evidence. This is exercised, not just
asserted: injecting one fixture's `secret_probe` into a corpus report makes
the harness itself panic with `corpus_probe_in_report` before any report is
written, rather than relying on the tooling to catch a leak after the fact.

Over HTTP, the harness checks the probes (each fixture's `secret_probe` and
`server_privacy_probe`, and its own bearer tokens) in every request body it
sends and every response body it receives, success bodies included. It does
not check headers: the bearer tokens travel in the `Authorization` header by
design.

## Failure labels

`pipeline.py` never prints a child command's own output. A failure is
always one safe label to stderr, printed once, as
`PipelineFailure: <label>`; a step that ran a test or a subprocess and
exited nonzero (`StepFailed`) adds ` exit=<n> log=<run dir>/logs/<step>.log`
after the label, naming its own protected log file -- never the log's
contents.

Common labels from a corpus run: `corpus_digest_mismatch` (the file's bytes
do not match `--corpus-digest`), `corpus_not_json` /
`unsupported_corpus_schema` / `empty_corpus` / `unsafe_fixture_label` /
`duplicate_corpus_identity` / `empty_secret_probe` (a malformed corpus
file), `corpus_package_and_bundle_conflict` /
`corpus_package_and_key_required` / `corpus_bundle_or_package_required` (an
invalid `--bundle`/`--package`/`--trusted-key` combination),
`hf_manifest_contains_raw_trace_text` (the export produced a manifest that
admits raw trace text), `database_check_executed_nothing:<step>` (the
scenario database shows fewer than 5 committed transactions),
`corpus_report_missing` / `corpus_report_malformed` /
`corpus_report_has_failures` / `corpus_evidence_mismatch` (the harness's own
report or evidence did not check out), `step_failed:<step>` for any test
or export step that exited nonzero -- its log is at the path the failure
line names -- and `cargo_test_list_failed:<step>` when the step's
`cargo test -- --list` exited nonzero (usually a compile error: the list
builds the test binary); its log, at the path the line names, holds the
list's own output. A pin whose source or sample order no longer matches the
recorded digest fails this way too: the export binary itself refuses with
"source digest changed" or "sample order digest changed", which surfaces as
`step_failed:hf_corpus_export`; the step's protected log holds the export's
own message. See
[pipeline-qualification.md](pipeline-qualification.md#the-environment-container-digest---postgres-admin-url-one-server-at-a-time)
for the environment-level labels (`pipeline_tooling_container_*`,
`pipeline_tooling_admin_url_invalid`, `pipeline_tooling_server_busy`).
