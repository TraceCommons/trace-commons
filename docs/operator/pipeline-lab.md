# Local pipeline lab

> **Status: design stage.** This runbook describes tooling that is not in this
> repository yet: the lab scripts and `trace-commons-pipeline-local`. It
> arrives with the pipeline runtime and qualification changes, and its
> commands can change before then. Do not follow it on a deployment.

## Purpose

Use the lab to run a fixed corpus, inspect a graded report, build a signed
package, and keep development evidence in a catalog. The lab is a local
command. It starts the existing pipeline test runner for the duration of a
corpus run. It does not add an HTTP service, database, table, or run identifier
to ingest.

The five commands are `corpus-run`, `report`, `catalog`, `qualify`, and
`package`. Calibration is not implemented.

## Run the corpus

Prerequisites: Rust, Cargo, Python 3.10 or later, Docker, and an available
Docker daemon. Docker must be able to load `postgres:17-alpine`.

From the repository root, run:

```bash
bash scripts/operator/lab/run.sh corpus-run
```

The command does these operations:

1. Read and validate the git corpus. Snapshot its bytes and compute SHA-256.
2. Build and sign the minimal Rust policy package with a disposable local key.
3. Start a new PostgreSQL container on a loopback port.
4. Apply the same migrations as production, including the versioned pipeline migrations and existing
   tables. Run the corpus with a separate `NOBYPASSRLS` runtime role.
5. Run each fixture in array order. Check replay, changed-content refusal,
   phase order, typed decisions, evidence and evaluation, expected Admission
   decisions, outcome counts, and contributor processing state.
6. Check that another tenant cannot read the first pipeline run.
7. Write the report and update the catalog. Stop the processes and remove the
   container, encrypted objects, and temporary inputs.

The command writes:

- `.local/lab/report.json`: the latest graded report.
- `.local/lab/report.md`: the readable report.
- `.local/pipeline-lab-catalog-v1.json`: the catalog.
- `.local/lab-records/`: immutable copies of reports, signed packages, public
  keys, and qualification evidence, named by their file digests.

The catalog preserves earlier reports, including runs of another corpus or
bundle. Repeating a catalog update for the same records does not add duplicates.
The latest report files can be replaced. Use the archived files for evidence.
A graded failure is recorded and returns a nonzero exit status. A startup or
submission failure returns a safe error label and does not claim a graded run.

## What traces run

The default input is
[`versioned-pipeline-minimal-corpus-v1.json`](../superpowers/specs/fixtures/versioned-pipeline-minimal-corpus-v1.json).
It contains five synthetic traces in this order:

| Label | Admission | Final result | Outcomes |
| --- | --- | --- | --- |
| `clean_tool_plan` | Admit | Complete | 4 |
| `locally_redacted_secret` | Admit | Complete after local redaction | 4 |
| `privacy_quarantine_approved` | Quarantine | Approve in Review, then complete | 4 |
| `privacy_quarantine_rejected` | Quarantine | Reject in Review | 2 |
| `privacy_risk_rejected` | Reject | Reject in Admission | 1 |

The corpus digest covers the exact file bytes, including fixture order. To
select another version, use `--corpus PATH`. To require specific bytes, also
use `--corpus-digest sha256:HEX`. Keep the fixture array order fixed. Labels,
trace IDs, and submission IDs must be unique. Reports are rejected as corpus
inputs. In particular, `.local/pipeline-restore-corpus.json` is an output
report from the restore drill.

The git corpus stays the fast CI corpus. The lab does not download Hugging
Face data. A later HF JSONL adapter can emit this same input schema.
`trace-commons-pilot-bootstrap` downloads JSONL and submits to ingest; it is
not the pipeline lab. `trace-commons-gate-calibrate` calibrates the old gate;
it does not calibrate these four policies.

## Isolation and privacy

The schema is production-shaped. The PostgreSQL instance is throwaway.
`corpus-run` always creates its own Docker container. It never uses or migrates
`DATABASE_URL`, `TRACE_COMMONS_DATABASE_URL`, or a shared development instance.
There is no external database option on this command. The integration tests
retain their explicit `TRACE_COMMONS_PG_TEST_DATABASE_URL` option.
The direct local runner also accepts that test variable or `--database-url`.
It no longer reads the ingest database variable. An explicitly configured test
database that cannot connect or migrate fails the integration tests.

Each corpus run uses a new encrypted artifact directory and a new in-memory
index. The index starts empty. The default ports are selected for the run.
`TRACE_COMMONS_PIPELINE_PORT` and `TRACE_COMMONS_PIPELINE_PG_PORT` can select
fixed loopback ports when needed. An occupied port causes failure.

The runner receives only the local test configuration. Inherited ingest,
telemetry, authentication, and payout settings do not reach that process.
External payout stays disabled. The settlement adapter records test operations.
The runner produces no stored server log. Private temporary files are removed
on normal exit, failure, and handled interruption. A forced process kill or
host failure can require removal of a `trace-commons-lab-*` container.

Stored corpus reports contain structured decisions, numbers, labels, and
hashes. Pipeline UUIDs are hashed. Trace text, fixture secret probes, raw
account IDs, credentials, and artifact bytes do not enter these reports.
Packages contain the policy configuration and referenced artifact bytes;
they contain no corpus or report.

## Select policies and build a package

The default profile is `minimal`. Use `--policies compatibility` for the
existing compatibility policies with local reference dependencies.

| Phase | Minimal implementation | Compatibility implementation |
| --- | --- | --- |
| Admission | `trace_commons.admission.authority_privacy.v1` | Same |
| Review | `trace_commons.review.authority_privacy.v1` | Same |
| Score | `trace_commons.score.minimal.v1` | `trace_commons.score.compatibility.v1` |
| Settle | `trace_commons.settle.minimal.v1` | `trace_commons.settle.compatibility.v1` |

Every report includes the complete four-policy manifest, `bundle_id`,
`package_hash`, per-policy configuration hashes, and `configuration_digest`.
The configuration digest is SHA-256 of the compact, sorted JSON map from each
phase name to its configuration hash. Storage settings remain separate in
`configuration_identities`. `runner_artifact_hash` identifies the actual
compiled local executable used for the run.

Build a reusable signed package first:

```bash
bash scripts/operator/lab/run.sh package --policies compatibility
bash scripts/operator/lab/run.sh corpus-run \
  --package .local/lab/package.json \
  --trusted-key .local/lab/trusted-key.json
```

The runner verifies the signature and artifact hashes before using the
package. Every fixture must bind to that exact bundle. The catalog archives
that package and its public key with the report. Do not also pass `--policies`
when selecting a package.

`package` uses the existing `BundlePackage`, canonical hashing, Ed25519
signature format, and `BundlePackageTrustStore` verifier. By default it creates
a disposable key and writes only the public key. For a controlled signing key,
pass `--signing-key PATH --key-id LABEL`. The key file must contain Ed25519
PKCS#8 DER. Use `--output` and `--public-key-output` to select output paths.
Protect signing keys outside the catalog. A public key emitted by this command
is a local verification input; it is not automatically a trusted release key.

The package names each policy's `implementation_id`. Policies remain Rust code
in the server. The package does not hash or contain that code. The report's
executable hash is separate build evidence, and qualification binds a package
to the code revision that it tested. A new algorithm requires a server
implementation, a new `implementation_id`, and tests. Editing JSON cannot
install a new algorithm.

Both bundled profiles are local/test candidates. Signing does not make them
production-selectable. Production qualification rejects minimal implementations
and reference or synthetic dependencies. A production candidate must use the
approved implementations and dependency profile, pass the existing qualification
checks, and be signed by a key already approved in the release trust store.
Ingest receives the signed package and the existing qualification metadata;
it does not load this catalog, corpus, or report files. Activation remains the
qualification and tenant activation process.

## Read a report and update the catalog

```bash
bash scripts/operator/lab/run.sh report .local/lab/report.json
bash scripts/operator/lab/run.sh catalog --report .local/lab/report.json
```

The report schema is `trace_commons.pipeline_corpus_report.v5`. Its
`report_digest` covers canonical report content, excluding that field itself.
Timing fields are omitted. Actual outcome and command identities can change
between runs, so archived reports can have different digests.

`result_digest` compares graded behavior for a bundle and ordered corpus. It
covers fixture labels, expected and observed processing results, phases,
decisions, evidence, evaluation, and replay checks. In this comparison only,
provenance hashes become the label `hash` and outcome IDs are omitted. Use the
full report to inspect the actual hashes. A matching result digest does not
establish production readiness.

The catalog schema is `trace_commons.pipeline_lab_catalog.v1`. Each `bundles`
entry is keyed by `bundle_id`. It contains:

- The most recently indexed corpus and configuration digests.
- `development_records`: paths to archived records, relative to the catalog.
- `reports`: each report's digests, pass/fail status, report path, and package
  and public-key paths when supplied.
- `production_ready: false` and explicit local dependency blockers.

Each report record retains its own corpus and configuration digests. Updating
one entry does not remove other bundles or earlier records. Catalog writes use
an exclusive file lock and atomic replacement. Existing qualification entries are
preserved; their older development-record paths retain their original meaning.

For a manual catalog import, use `--package PATH --trusted-key PATH` to attach
a signed package. Use repeated `--record PATH` options to attach matching
qualification, inventory, or restore evidence. The command checks supported
schemas and rejects qualification evidence for another bundle, corpus, or
configuration. Use `--catalog PATH` to keep a separate catalog.

## Qualification and test levels

```bash
bash scripts/operator/lab/run.sh qualify
```

This command wraps the existing pipeline qualification script. It runs package
and lab checks, the full `versioned_pipeline_pg` integration suite, the
compatibility corpus, and the backup/restore drill. That integration suite
includes activation coverage. It writes the existing qualification reports and updates the same
catalog through the standalone `catalog` command.

The old minimal, compatibility, and product corpus scripts remain supported.
They call the shared lab workflow and retain their report filenames. Their
filename version numbers do not select the report schema.

`versioned_pipeline_pg` remains the integration suite for schema contracts,
transactions, crashes, fenced leases, RLS, and activation. The lab supplies
corpus, qualification, and package evidence. It does not replace those tests.
For lab file-handling tests only, run:

```bash
python3 -m unittest discover -s scripts/operator/lab -p 'test_*.py'
RUSTFLAGS='-D warnings' cargo test -p trace-commons-server --bin trace-commons-pipeline-local
```

## Deferred work

There is no calibration command or automatic bootstrap/holdout split. A later
calibration command must keep bootstrap and holdout data separate, record both
input digests and fixed orders, evaluate the holdout, and emit changed Score
or Admission configuration. Changed configuration must produce a new
`bundle_id`, package, and report. Moving the old scripts does not implement
this behavior. Multi-party valuation (`SCR-005`) remains out of scope.
