# Load-testing the redaction witness

How to measure the witness's latency and throughput on a laptop, with the
real release binary and no enclave, no classifier key and no real data.
The design this informs is
[`../superpowers/specs/2026-11-05-witness-scaling-design.md`](../superpowers/specs/2026-11-05-witness-scaling-design.md).

## Rules

- **Synthetic text only.** `witness_load.py` generates every body from a
  fixed vocabulary. Do not point it at a file, and do not add an option
  that reads one.
- **Loopback by default.** The stand-ins bind `127.0.0.1` or a unix
  socket. A deployed witness may be sent a handful of requests, one at a
  time, and never while a driver is using the same NEAR AI key: concurrent
  requests there spend that key, and an overlap burst has poisoned it
  before (memory note on NEAR AI classify limits, 2026-08-28).
- **Local tooling, not a deployment.** A witness booted against the
  stand-in agent signs with a published test key (scalar 2) and reports a
  measurement spliced into a borrowed fixture quote. Nothing trusts that
  measurement, and nothing should.
- Kill what you started when you finish (the three pids below).

## The pieces (`scripts/witness-load/`)

| file | what it is |
|---|---|
| `fake_dstack_agent.py` | Serves `GET /v0/GetKey` and `/v0/GetQuote` on a unix socket in the shape `DstackSocketAgent` reads. The quote is the attestation crate's real fixture TDX quote, with `report_data` spliced in at the offset the witness parses. |
| `stub_classifier.py` | A NEAR AI `privacy/classify` stand-in. Latency is `--base-ms + --per-kib-ms x body KiB`, default 1300 + 32, fitted to the 2026-09-03 live measurements. A body over `--max-body-bytes` gets a 413. `--curve measured-2026-11-05` uses the owner's measured latency for that day instead. `--trip-above N` fails with a 502 any request that arrives while N are already in flight. Spans every `Alice Example` at codepoint offsets. Logs body size, in-flight count, status and seconds per request, never content. `GET /stats` and `GET /stats/reset` serve the totals. |
| `witness_load.py` | `run` submits synthetic contributions to `POST /v1/witness/jobs` from N workers and polls each to its answer and prints status counts, latency p50/p90/p99/max, requests/s and KB/s as JSON. `body` writes one body to stdout. |

## Run it

Build the binary (on a shared machine, through the build lock):

```bash
RUSTFLAGS="-D warnings" CARGO_INCREMENTAL=0 \
  cargo build --release -p trace-commons-server --bin trace-commons-witness
```

Start the three processes. The socket path is relative because macOS caps
unix socket paths at 104 bytes; run from a short working directory.

```bash
W=$PWD   # the repository root
mkdir -p /tmp/wl && cd /tmp/wl
python3 -I $W/scripts/witness-load/fake_dstack_agent.py --socket agent.sock & echo $! > agent.pid
python3 -I $W/scripts/witness-load/stub_classifier.py --port 18471 & echo $! > stub.pid
TRACE_COMMONS_WITNESS_DSTACK_SOCKET=agent.sock \
TRACE_COMMONS_WITNESS_REDACTION=full-pipeline \
TRACE_COMMONS_WITNESS_BIND=127.0.0.1:18088 \
TRACE_COMMONS_WITNESS_MAX_CLASSIFY_IN_FLIGHT=2 \
TRACE_PRIVACY_FILTER_BACKEND=near-ai \
TRACE_NEAR_AI_PRIVACY_BASE_URL=http://127.0.0.1:18471/v1 \
TRACE_NEAR_AI_PRIVACY_API_KEY=stub-not-a-key \
  $W/target/release/trace-commons-witness > witness.log 2>&1 & echo $! > witness.pid
```

Then drive it:

```bash
# one request: 241 KB of tool output across 60 tool results
python3 -I $W/scripts/witness-load/witness_load.py run --url http://127.0.0.1:18088 \
  --size-kb 241 --events 60 --seed $RANDOM

# 8 workers, 32 requests, three sizes
python3 -I $W/scripts/witness-load/witness_load.py run --url http://127.0.0.1:18088 \
  --concurrency 8 --requests 32 --size-kb 50,250,900 --events 40 --seed $RANDOM

curl -s 127.0.0.1:18471/stats    # classify calls, bytes, peak in-flight
kill $(cat witness.pid stub.pid agent.pid)
```

## Reading the numbers

- **Vary `--seed` between runs, and restart the witness for cold
  numbers.** The NEAR AI adapter caches classified windows for the life of
  the process. A repeated seed is answered from that cache and makes zero
  classifier calls, which looks like a fast witness and is not one.
- `--events` matters more than `--size-kb`. The witness classifies each
  text leaf separately, so wall time follows the number of leaves.
- A `503` is either `witness_saturated` (all request slots busy; the
  witness answers at once with `Retry-After`) or `witness_redaction_failed`
  (the classifier failed; it arrives after the adapter's retries, about
  two minutes against a dead endpoint). Read the response body to tell
  them apart.
- The stand-in scales perfectly with concurrency because it does not model
  the endpoint's overlap trip unless you pass `--trip-above`. Treat
  concurrency gains measured here as an upper bound.
- Against a deployed witness over HTTPS, a python.org build of Python may
  not find system CA certificates; set `SSL_CERT_FILE=/etc/ssl/cert.pem`
  (macOS), or write a body with `witness_load.py body` and send it with
  `curl`.
