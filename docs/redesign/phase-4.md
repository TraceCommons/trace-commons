# Phase 4 authority and privacy policies

Phase 4 adds authority and privacy controls to the local versioned pipeline.
The Score policy stays fixed.

The pipeline now has these properties:

- Admission validates the schema, contribution path, consent, allowed uses,
  authority, privacy risk, tombstones, and limits.
- Admission returns `Admit`, `Quarantine`, or `Reject`.
- PostgreSQL applies tenant and principal limits across service instances.
- An exact receipt replay does not consume another limit unit.
- Review transforms content and stores the encrypted result before commit.
- A quarantined run requires a leased human assessment.
- The bound Review policy uses the assessment as evidence.
- Each phase checks policy and lifecycle state before work and during commit.
- An operator can suspend, resume, or terminate a bound policy.
- A suspension does not consume retry attempts or change the bundle.
- A withdrawal before Settle forces index exclusion.
- A withdrawal does not remove credit from a committed Score outcome.
- A withdrawal after an index write queues an index invalidation.

## Run the corpus

Run this command:

```bash
scripts/operator/run-minimal-pipeline-corpus.sh
```

The command starts PostgreSQL 17. It operates the server with a non-owner
runtime role.

The command writes these reports:

- `.local/pipeline-report-v4.json`
- `.local/pipeline-report-v4.md`

The corpus includes Admission approval, quarantine, and rejection. It also
includes Review approval, rejection, and content transformation.

## Run the PostgreSQL tests

Set `TRACE_COMMONS_PG_TEST_DATABASE_URL` to a PostgreSQL test database. Then
run this command:

```bash
cargo test -p trace-commons-server --test versioned_pipeline_pg
```

The Phase 4 tests cover concurrent limits, review leases, policy suspension,
withdrawal ordering, credit retention, and index invalidation.

## Current limits

This path remains local and test only. The Score amount stays fixed.

The production quarantine age and remediation policy remain open. This local
path requires a leased assessment and has no production activation.

Phase 5 adds the production compatibility Score policy. See
[phase-5.md](./phase-5.md).
