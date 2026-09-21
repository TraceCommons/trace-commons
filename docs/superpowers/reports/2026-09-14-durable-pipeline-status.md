# Durable pipeline implementation status

The durable pipeline keeps the minimal policies from the minimal milestone. It adds durable execution and
bundle recovery for local and test deployments.

The pipeline now has these properties:

- A receipt transaction stores the submission, object reference, run,
  Admission outcome, and transition.
- A receipt key lock serializes simultaneous first submissions.
- A staged artifact record identifies an encrypted object after an interrupted
  receipt.
- Each worker claim has a token and an expiry time.
- Only the current lease token can commit an outcome or an error state.
- A retry uses a bounded delay and a maximum attempt count.
- Each retry loads the package that the run identifies.
- Bundle activation and rollback apply only to later runs.
- Bundle packages and phase outcomes are immutable.
- Outcome readers reject an unknown required schema version.

## Run the corpus

Run this command:

```bash
scripts/operator/run-minimal-pipeline-corpus.sh
```

The command starts PostgreSQL 17 and applies all migrations. Then it starts
the local server with a non-owner role.

The command writes these reports:

- `.local/pipeline-report-v2.json`
- `.local/pipeline-report-v2.md`

The JSON report includes the attempt count, retry limit, next attempt time,
and time in the current phase.

## Recovery tests

Set `TRACE_COMMONS_PG_TEST_DATABASE_URL` to a PostgreSQL test database. Then
run this command:

```bash
cargo test -p trace-commons-server --test versioned_pipeline_pg
```

The test covers receipt races, concurrent workers, stale leases, retry
exhaustion, bundle activation, rollback, and crash boundaries 1 through 5.

The test also examines the privileges of `pipeline_claimer`. This role can
call `claim_pipeline_run(uuid, integer)`. It has no direct read access to the
run or outcome tables.

## Current limits

This path remains local and test only. It uses the fixed zero-credit bundle.

The settlement milestone adds index commands and positive credit operations. See
[pipeline settlement status](./2026-09-14-pipeline-settlement-status.md).
