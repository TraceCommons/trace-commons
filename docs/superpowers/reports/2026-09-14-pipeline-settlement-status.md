# Pipeline settlement implementation status

> **Status: historical.** This report describes an unmerged development
> branch. None of the code it describes is in this repository yet.

The settlement milestone keeps the isolated local and test path. It adds fixed positive credit,
sealed index commands, and internal settlement records.

The pipeline now has these properties:

- A fixed Score policy can award zero or a positive microcredit amount.
- A positive Score commits one eligible credit event with the Score outcome.
- A zero Score does not create a credit event.
- Settle decides index membership from the bound bundle and committed outcomes.
- An include decision stores an encrypted command and its hash before the write.
- A retry uses the sealed command. It does not evaluate membership again.
- Index application and credit settlement record separate progress.
- The Settle outcome is written only after every required internal operation.
- External NEAR payout stays in the outbox. It does not change a Settle outcome.

## Run the corpus

Run this command:

```bash
scripts/operator/run-minimal-pipeline-corpus.sh
```

The command starts PostgreSQL 17 and applies all migrations. Then it starts the
local server with a non-owner role.

The command writes these reports:

- `.local/pipeline-report-v3.json`
- `.local/pipeline-report-v3.md`

The JSON report includes index command hash, index progress, credit progress,
and payout state. It does not include account references or transaction hashes.

The default corpus still uses the zero-credit exclude bundle. PostgreSQL tests
cover the four combinations of index inclusion or exclusion and zero or
positive Score.

## Recovery tests

Set `TRACE_COMMONS_PG_TEST_DATABASE_URL` to a PostgreSQL test database. Then
run this command:

```bash
cargo test -p trace-commons-server --test versioned_pipeline_pg
```

The test covers the durable recovery cases. It also covers crash boundaries 6
through 11, sealed command reuse, independent index and credit recovery, held
credit, shared settlement batches, and NEAR outbox states.

## Current limits

This path remains local and test only. The Score amount is fixed. The index
adapter is an isolated test writer.

The authority and privacy milestone adds production authority, privacy, and policy-status controls.
