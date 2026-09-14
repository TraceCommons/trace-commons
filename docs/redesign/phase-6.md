# Phase 6 product integration

Phase 6 connects the versioned pipeline to contributor, customer, worker, and
operator interfaces. The legacy product interfaces remain available for
retained records.

The pipeline now has these properties:

- Contributor status comes from runs, outcomes, credit batches, and payout
  state.
- Processing, credit, and payout use separate response fields.
- Status batches accept no more than 500 submission identifiers.
- An unrelated principal cannot detect a submission through the status
  interface.
- Signed score attestations identify the run, Score outcome, schema, and
  bundle.
- Customer exports select approved and operable registry revisions.
- Each export uses an immutable source snapshot and a stable source-list hash.
- Export items retain the view schema, consent, source revision, and bundle.
- A retry with the same request key returns the same export snapshot.
- Withdrawal writes a hash-only tombstone and invalidates managed export
  membership.
- Withdrawal keeps committed credit and reports known managed distribution.
- Object removal uses the existing bounded revocation queue.
- Index invalidation stops after five unsuccessful attempts.
- The lifecycle summary reports terminal index invalidation errors.
- Disabled community interfaces continue to return not-found responses.

## Public staging interfaces

`trace-commons-pipeline-local` has compatibility routes for the Phase 6
staging demonstration:

- `POST /v1/traces`
- `DELETE /v1/traces/{submission_id}`
- `POST /v1/contributors/me/submission-status`
- `GET /v1/contributors/me/credit`
- `GET /v1/contributors/me/credit-events`
- `GET /v1/contributors/me/score-attestation`
- `GET /.well-known/trace-commons-attestation-keyset.json`

The staging runner also has scoped export, lifecycle, review, and worker
routes. Local static tokens cannot satisfy production readiness.

## Run the corpus

Run this command:

```bash
bash scripts/operator/run-product-pipeline-corpus.sh
```

The report includes the public processing, credit, and payout states. The
command uses PostgreSQL and encrypted local artifacts.

## Run the PostgreSQL tests

Set `TRACE_COMMONS_PG_TEST_DATABASE_URL` to a PostgreSQL test database. Then
run this command:

```bash
cargo test -p trace-commons-server --test versioned_pipeline_pg
```

The Phase 6 tests cover status authority, signed provenance, export snapshots,
withdrawal, and terminal invalidation errors.

## Current limits

This path is a staging candidate. Phase 7 adds package trust, the deployment
inventory, restore tests, and promotion evidence.

See [phase-7.md](./phase-7.md).
