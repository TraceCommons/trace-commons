# Pipeline compatibility implementation status

The compatibility milestone adds a compatibility Score policy to the local versioned pipeline.
Admission and Review stay on the authority and privacy policies.

The pipeline now has these properties:

- Score uses the reference scorer and embedder behind `ScorePolicy`.
- Score queries a read-only index. It does not write.
- Score records model, projection, snapshot, neighbor hash, cardinality,
  coverage, and quality measurements.
- Sensitive neighbor lists and embeddings stay in encrypted artifacts.
- A scorer or embedder failure is an operational error. It does not create a
  Score outcome or a credit event.
- Settle decides index membership from committed Score evidence.
- Settle does not query the live index for that decision.
- A shadow comparison uses a separate index namespace and creates no credit.

The mapping from current gate results is
[compatibility mapping](../specs/2026-09-14-versioned-pipeline-compatibility-mapping.md). The approved local
baseline is [compatibility baseline](../specs/versioned-pipeline-compatibility-baseline-v1.json).

## Run the corpus

Run this command:

```bash
scripts/operator/run-compatibility-pipeline-corpus.sh
```

The command starts PostgreSQL 17. It operates the server with a non-owner
runtime role and the compatibility bundle.

The command writes these reports:

- `.local/pipeline-report-v5.json`
- `.local/pipeline-report-v5.md`

The minimal-policy command `scripts/operator/run-minimal-pipeline-corpus.sh` still
runs the fixed-score bundle.

## Run the PostgreSQL tests

Set `TRACE_COMMONS_PG_TEST_DATABASE_URL` to a PostgreSQL test database. Then
run this command:

```bash
cargo test -p trace-commons-server --test versioned_pipeline_pg
```

The compatibility tests cover Score write isolation, Settle membership after a live
index change, and a Score dependency failure that retries on the same bundle.

## Current limits

This path remains local and test only. The local bundle uses the reference
scorer with zero gate floors. A staging bundle must include deployed floors
in the bundle configuration.

The product integration milestone connects contributor, customer, and operator product interfaces. See
[pipeline product integration status](./2026-09-14-pipeline-product-integration-status.md).
