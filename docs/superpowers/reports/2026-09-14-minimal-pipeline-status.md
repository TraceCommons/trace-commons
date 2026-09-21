# Minimal pipeline implementation status

The minimal pipeline is an isolated local and test path. It does not replace the current
ingest path. It uses one fixed bundle:

- Admission checks authentication, authority, and the envelope schema.
- Review passes through the encrypted submitted bytes and creates one approved
  registry revision.
- Score records zero completed microcredits.
- Settle records index exclusion, zero finalized microcredits, and no batch.

The binary refuses to start unless you supply `--allow-minimal-policies`. It
also refuses a non-loopback bind address. Do not use this bundle in production.

## Run the corpus

The following command starts an isolated PostgreSQL 17 container, applies the
migrations, starts the server with a non-owner runtime role, submits the
versioned local fixtures, runs the worker, and writes JSON and Markdown
reports:

```bash
scripts/operator/run-minimal-pipeline-corpus.sh
```

The command requires Docker, Cargo, curl, Python 3, and grep. It does not fetch
a corpus or model and does not enable external payout.

Reports are written to:

- `.local/pipeline-report-v1.json`
- `.local/pipeline-report-v1.md`

The runner replays each exact request and checks that it gets the same run. It
also changes the request bytes while keeping the idempotency key and checks for
an HTTP conflict. It queries one run with another tenant and expects a
not-found response. The second fixture contains a known secret before local
redaction. The runner checks that the secret is absent from the submitted
envelope, reports, and server log.

## Server options

`trace-commons-pipeline-local serve` accepts:

- `--database-url` or `TRACE_COMMONS_PG_TEST_DATABASE_URL` for a dedicated test database
- `--bind` (default `127.0.0.1:3917`)
- `--artifact-root`
- `--allow-minimal-policies` (required)
- `--skip-migrations` for a restricted runtime role after an owner applies
  migrations
- `--fail-phase review|score|settle` for asynchronous policy failure tests

The server requires `TRACE_COMMONS_PIPELINE_MASTER_KEY` and
`TRACE_COMMONS_PIPELINE_TOKENS`. Token entries use this format:

```text
token,tenant,principal,contributor|worker|operator
```

Separate entries with semicolons. The server never writes token values to
logs. The principal must use a role-specific `*_sha256:` prefix followed by
64 lowercase hexadecimal characters.

`trace-commons-pipeline-local corpus` accepts separate submit, worker, and
inspection tokens. It also accepts fixture and report paths and a completion
time limit.

## Current limits

This milestone has one worker claim at a time per test tenant. It has no
fenced lease, retry delay, positive credit, index write, production privacy
policy, or production bundle distribution. A claimed policy failure records a
safe operational label and no outcome for the failed phase. The durable pipeline adds
concurrent workers, recovery, retained bundle loading, and lease fencing.

See [pipeline settlement status](./2026-09-14-pipeline-settlement-status.md) for the current local pipeline.
