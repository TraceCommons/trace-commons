# Phase 7 qualification

Phase 7 adds package trust, operational evidence, and restore qualification.
It does not activate the new pipeline for production submissions.

The pipeline now has these properties:

- Ed25519 signatures bind canonical package bytes.
- A trust store rejects unknown package signing keys.
- Production qualification rejects unknown policy implementations.
- Production qualification rejects development and synthetic dependencies.
- Only a qualified package can use the production activation operation.
- Package and qualification records remain immutable.
- Readiness separates liveness from dependency and control state.
- Operational summaries show work age, errors, holds, outbox state, and
  blocked lifecycle work.
- Forensic traces link a run to outcomes, commands, credit, batches, and
  interventions.
- Promotion evidence has a maximum age.
- Missing, failed, or stale drill evidence blocks promotion.
- Index rebuild uses sealed commands and does not create outcomes or credit.
- PostgreSQL and encrypted object restore preserve operation identities.
- A machine-generated inventory classifies routes, operations, adapters,
  tables, object namespaces, telemetry destinations, and roles.

## Local qualification

Run this command:

```bash
bash scripts/operator/run-phase7-qualification.sh
```

The command does these tests:

1. It validates the deployment inventory.
2. It tests package identity and package trust.
3. It runs the full PostgreSQL pipeline suite.
4. It runs the compatibility corpus through the HTTP and worker paths.
5. It restores PostgreSQL and encrypted objects.
6. It compares immutable identities before and after the restore.
7. It writes a qualification report and a lab catalog.

The command writes these files:

- `.local/pipeline-qualification-report-v7.json`
- `.local/pipeline-lab-catalog-v1.json`
- `.local/phase7-interface-inventory.json`
- `.local/pipeline-restore-report-v7.json`
- `.local/pipeline-report-v7.json`
- `.local/pipeline-report-v7.md`

## PostgreSQL qualification

Set `TRACE_COMMONS_PG_TEST_DATABASE_URL` to a PostgreSQL test database.
Then run this command:

```bash
cargo test -p trace-commons-server --test versioned_pipeline_pg
```

CI supplies the database. Therefore, a skipped database test cannot qualify
the candidate in CI.

## Production package registration

Build the package in the policy lab. Sign its canonical package hash with an
approved Ed25519 release key.

Call `PipelineQualificationStore::qualify_bundle` with the signed package,
the trust store, the evidence digests, and the production dependency profile.

Pass the current promotion decision, deployed code revision, and dependency
profile to `activate_qualified_bundle`. The operation rejects stale evidence,
a revision mismatch, or a non-production dependency.

Phase 8 owns tenant activation and production rollback observation. See
[phase-8.md](./phase-8.md).

## Local runner limit

`trace-commons-pipeline-local` remains a local test process. Its readiness
response lists static authentication and synthetic adapters as blockers.

The local lab catalog also records these blockers. A local corpus report
cannot serve as production evidence.
