# Pilot Bootstrap Replay Harness — Operator Runbook

Phase: A.6. Spec:
`docs/superpowers/specs/2026-05-14-pilot-bootstrap-harness-design.md`.
Plan: `docs/superpowers/plans/2026-05-14-pilot-bootstrap-harness.md`.

The `tracedao-pilot-bootstrap` binary replays HuggingFace agent-traces
datasets into a running `tracedao-ingest` so the pilot has real-shaped
traffic for tail-fraction calibration (A2.5), Phase A.5 metric design,
and end-to-end pipeline validation.

This is a load-generation tool — not a substitute for real contributor
clients. When Ironclaw integration lands the harness is retired or kept
as an ops calibration utility.

## What it is (and is not)

- Single binary, single process, single tenant.
- Reads `.jsonl` session files from HuggingFace via `hf-hub` (one file =
  one session = one trace), flattens each session into a body via the
  per-dataset translator, runs the body through the deterministic
  redactor, and POSTs the resulting `tracedao-protocol` envelope to
  `/v1/traces` at a configurable rate.
- Idempotent: re-running against the same dataset is safe — the
  ingest server collapses duplicate submission ids to no-ops.
- **Not** a multi-tenant load tester. **Not** adversarial input. **Not**
  a daemon. **Not** a credit-issuing path (run with the existing
  zero-credit calibration semantics).

## Prerequisites

1. A running `tracedao-ingest` (locally or on a staging deployment).
2. A configured bootstrap-tenant token, e.g.

   ```
   TRACE_COMMONS_TENANT_TOKENS=bootstrap-tenant:contributor:<token>
   ```

   on the ingest side. The harness reads the same token from
   `TRACE_COMMONS_PILOT_TENANT_TOKEN` (or `--tenant-token`).

3. Free disk space for the HF session cache (`~/.cache/huggingface`
   by default; override with `--cache-dir`). The swival JSONL session
   files total a few GB; multi-dataset runs add proportionally.
4. Zero-credit calibration mode on the server side:

   ```
   TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA=0
   ```

## Session schema

All three target datasets ship one `.jsonl` file per session at the
repo root. Each line is one event row; the translator concatenates the
textual content of every event into one trace body. Recognized text
fields (per event, in this order): `message.content` (string OR list of
`{type:"text", text:"..."}` or `{type:"thinking", thinking:"..."}`
chunks) and the top-level `content`. Other event fields (`type`,
`model_change`, `parentId`, `timestamp`, ...) are ignored. Sessions
whose flattened body falls outside the word-count bounds
(`--min-words`/`--max-words`, default 200..=2000) are skipped.

See `scripts/operator/build-agent-traces-corpus.py` for the
authoritative reference implementation and
[`./pilot-bootstrap-dryrun-notes.md`](./pilot-bootstrap-dryrun-notes.md)
for the post-mortem of the earlier parquet-shaped loader.

## Local smoke validation

Before pointing the harness at a real ingest deployment, run the
loopback smoke. It builds the binary, spins up a stdlib-only Python
mock that plays both `huggingface.co` and `/v1/traces` on
`127.0.0.1:3907`, primes the hf-hub cache from a directory of
checked-in JSONL session fixtures
(`scripts/operator/fixtures/swival-smoke/`), and asserts both the
happy-path POST count and submission-id idempotency across two
consecutive runs.

The smoke is fully offline — no `huggingface.co` reachability or
`HF_TOKEN` is needed. Typical wall-clock cost is well under a minute
on a developer laptop.

```
# Build once (release profile keeps the smoke fast).
cargo build --release --bin tracedao-pilot-bootstrap

# Run the loopback smoke. Exits 0 on success, 1 with a hash-only
# diagnostic label on failure (e.g. `run2_distinct_3_expected_10_idempotency_broken`).
./scripts/operator/pilot-bootstrap-smoke.sh
```

Expected last line:

```
SmokePilotBootstrapOK: 10 submissions, idempotency confirmed (distinct stayed at 10 across 2 runs)
```

Tunables (override via env):

| Var | Default | Meaning |
|-----|---------|---------|
| `SMOKE_COUNT` | `10` | submissions per run |
| `SMOKE_PORT` | `3907` | loopback port for the mock server |
| `SMOKE_BINARY` | `./target/release/tracedao-pilot-bootstrap` | binary under test |
| `SMOKE_SIDECAR` | `/tmp/pilot-bootstrap-smoke-sidecar.jsonl` | sidecar output |
| `SMOKE_HF_CACHE` | `/tmp/pilot-bootstrap-smoke-hf-cache` | scratch hf-hub cache |
| `SMOKE_MOCK_LOG` | `/tmp/pilot-bootstrap-smoke-mock.log` | mock server stderr |
| `SMOKE_BINARY_LOG` | `/tmp/pilot-bootstrap-smoke-binary.log` | binary stdout+stderr |

The mock server only binds to `127.0.0.1` and refuses any non-loopback
host. The smoke cleans up the sidecar before each run and terminates
the mock on `EXIT`/`INT`/`TERM`. Run the smoke whenever the binary,
the protocol envelope, or the translators change before promoting to
a staging deployment.

## Quick start

### 100-submission smoke

Verifies end-to-end wiring before the long run. Roughly two minutes at
the default 1 req/sec.

```
TRACE_COMMONS_PILOT_TENANT_TOKEN=<token> \
  cargo run -p tracedao-server --bin tracedao-pilot-bootstrap -- \
    --source jedisct1/agent-traces-swival \
    --count 100 \
    --target http://localhost:3907 \
    --rate 1 \
    --sidecar ./pilot-smoke.jsonl
```

Confirm:

- The sidecar JSONL has 100 lines.
- The `gate_decision` column shows a non-trivial mix
  (`accepted` / `quarantined` / `rejected`).
- The `trace_gate_decisions` table on the server has 100 rows whose
  `submission_id` matches the sidecar's.

### 30k-submission pilot run

The actual A.6 deliverable. ~8 hours at 1 req/sec, ~50 minutes at 10
req/sec. Start at 1 req/sec; once you confirm server CPU and DB load
stay under ~50%, bump `--rate` up.

```
TRACE_COMMONS_PILOT_TENANT_TOKEN=<token> \
  cargo run --release -p tracedao-server --bin tracedao-pilot-bootstrap -- \
    --source jedisct1/agent-traces-swival \
    --count 30000 \
    --target http://localhost:3907 \
    --rate 1 \
    --sidecar ./pilot-30k.jsonl
```

Run in a screen / tmux session — the binary runs to completion and
exits when `--count` is reached. A restart picks up where it left off
because the deterministic submission id collapses already-sent rows
to no-ops at the server.

### Multi-dataset run (A.6 v1.5)

Mix pi-mono and DeepSeek-v4-Pro-Agent so the vector index gets
distributional diversity. The translator is auto-detected from
`--source`; pass `--translator` to override.

```
# pi-mono coding-session traces
cargo run --release ... -- --source badlogicgames/pi-mono --count 10000 ...

# DeepSeek agent traces
cargo run --release ... -- --source TeichAI/DeepSeek-v4-Pro-Agent --count 10000 ...
```

## CLI reference

| Flag | Default | Notes |
|------|---------|-------|
| `--source` | `jedisct1/agent-traces-swival` | HF dataset id |
| `--translator` | auto-detected | `swival` / `pi-mono` / `deepseek-agent` |
| `--count` | `1000` | total submissions |
| `--target` | `http://localhost:3907` | ingest base URL |
| `--tenant-token` | `$TRACE_COMMONS_PILOT_TENANT_TOKEN` | bearer token |
| `--rate` | `1.0` | requests per second |
| `--sidecar` | `./pilot-bootstrap-sidecar.jsonl` | append-only JSONL |
| `--seed` | `42` | deterministic row sampling seed |
| `--cache-dir` | hf-hub default | JSONL session cache directory |
| `--min-words` | `200` | drop sessions with fewer words after flatten |
| `--max-words` | `2000` | drop sessions with more words after flatten |
| `--dry-run` | off | print resolved config and exit |

## Sidecar interpretation

The sidecar is append-only JSONL, one row per submission attempt.
Fields are hash-only / label-only — no raw trace body, no bearer
token, no contributor identity:

```
{"submission_id":"...",
 "source_dataset":"jedisct1/agent-traces-swival",
 "source_row_id":"<title-or-row-key>",
 "source_domain_tag":"security-audit/reentrancy",
 "http_status":200,
 "gate_decision":"accepted",
 "elapsed_ms":42,
 "timestamp":"2026-05-14T20:30:00Z"}
```

Join the sidecar against `trace_gate_decisions` by `submission_id` (the
sidecar id is the first 32 hex chars of SHA-256(trace_body) and matches
the server's `submission_id` column) to compute per-domain gate-pass
rates and per-domain tail-fraction distributions.

## Teardown

When the run finishes (or is interrupted):

1. Stop the harness with Ctrl+C. There is no daemon to disable.
2. Keep the sidecar — it's the calibration input for A2.5's
   tail-fraction floor and Phase A.5's metric design.
3. The submitted rows stay in the ingest server's audit chain and
   vector index. They are real submissions for storage purposes; only
   credit issuance is zero-valued.
4. If a run needs to be rolled back (e.g. wrong dataset), use the
   `/v1/admin/trace-revoke-drill` path or the storage-level revocation
   worker — the harness does not expose its own delete path.

## Failure modes

- **403 / 401** — `TRACE_COMMONS_PILOT_TENANT_TOKEN` doesn't match the
  server's `TRACE_COMMONS_TENANT_TOKENS`, or the bootstrap tenant lacks
  contributor scope. Sidecar records the failure with
  `gate_decision = "rejected"`.
- **5xx** — retried with exponential backoff (max 3 retries). After
  retries are exhausted the row is sidecar-recorded with
  `gate_decision = "error"`; the loop continues.
- **Parquet decode error** — row is skipped, error counter incremented,
  loop continues.
- **Translator error** (e.g. pi-mono row with no `messages`) — row is
  skipped, error counter incremented, loop continues.

The summary printed on exit reports the totals so a partial sidecar
can be interpreted alongside the run.
