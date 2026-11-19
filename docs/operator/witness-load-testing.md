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
| `near_ai_probe.py` | For the key holder: measures the live endpoint's body limit, array batching and latency with synthetic text (below). |
| `witness_load.py` | `run` submits synthetic contributions to `POST /v1/witness/jobs` from N workers, polls each to its answer, and prints status counts, latency (submit to answer) p50/p90/p99/max, requests/s and KB/s as JSON. `body` writes one body to stdout. |

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

## Measuring the live endpoint (key holder only)

`scripts/witness-load/near_ai_probe.py` measures NEAR AI's `privacy/classify`
directly, for the two settings the witness depends on:
`TRACE_NEAR_AI_PRIVACY_MAX_REQUEST_BODY_BYTES` and
`TRACE_NEAR_AI_PRIVACY_MAX_BATCH_INPUTS`. It sends synthetic text one request
at a time, five seconds apart, at most 12 requests a run. It prints sizes,
statuses, entry counts and seconds; it never prints the key, any text, or a
response body. Nothing is sent without `--yes`. Use a key that no driver or
witness is using while it runs.

```bash
export TRACE_NEAR_AI_PRIVACY_API_KEY=...   # from your own secret store
python3 -I scripts/witness-load/near_ai_probe.py limit --yes      # body limit, <= 8 requests
python3 -I scripts/witness-load/near_ai_probe.py batch --yes      # arrays of 4, 16, 64 short texts
python3 -I scripts/witness-load/near_ai_probe.py batch --yes --batch-sizes 64 --batch-item-kib 3
python3 -I scripts/witness-load/near_ai_probe.py latency --yes    # 1 to 200 KiB bodies, fitted
# Last, on its own key: bursts of 2 then 4 overlapping requests, a minute apart, 8 in all
python3 -I scripts/witness-load/near_ai_probe.py overlap --yes --i-understand-overlap
```

Read `limit` as: the largest 200 and the smallest 413, in body bytes. The
search stops on any other status rather than reading it as a size answer.
Read `batch` as: `entries_match_inputs=True` and `same_spans=True` on every
compared input. Anything else means batching stays off.

Read `latency` from its last line: `base_s` and `per_kib_s` fit the shape
of the adapter's timeout (`TRACE_NEAR_AI_PRIVACY_TIMEOUT_MS` +
`_PER_KIB_MS` x body KiB), and `worst_ratio_to_default_timeout` is the
slowest measured request as a fraction of the default timeout for its size.
Above about 0.5, there is less than 2x headroom: raise the timeout or lower
the window cap.

Read `overlap` as the largest `in_flight` whose burst was all `200`, with
`after_bursts` also `200`. That is the most `TRACE_COMMONS_WITNESS_MAX_CLASSIFY_IN_FLIGHT`
should be, summed across every witness replica sharing the key. The mode
sends a serial baseline first and stops at the first burst with any
failure. A failed burst has poisoned this key for minutes before (2026-08-28),
so run it last and on a key nothing else is using.

## Reading the numbers

- **Vary `--seed` between runs, and restart the witness for cold
  numbers.** The NEAR AI adapter caches classified windows for the life of
  the process. A repeated seed is answered from that cache and makes zero
  classifier calls, which looks like a fast witness and is not one.
- `--events` matters more than `--size-kb`. The witness classifies each
  text leaf separately, so wall time follows the number of leaves.
- A `503` is `witness_saturated` (all request slots busy; the witness
  answers at once with `Retry-After`), `witness_classifier_unavailable`
  (the classifier stayed down, slow or rate limited through the
  adapter's retries, or the classify budget ran out first, possibly in the
  queue; it arrives after them, about two minutes against a dead endpoint,
  with `Retry-After: 60`), or `witness_redaction_failed`
  (the pass refused: a permanent classifier refusal, a residual secret, a
  result too large to keep). Read the response body to tell them apart.
- The stand-in scales perfectly with concurrency because it does not model
  the endpoint's overlap trip unless you pass `--trip-above`. Treat
  concurrency gains measured here as an upper bound.
- Against a deployed witness over HTTPS, a python.org build of Python may
  not find system CA certificates; set `SSL_CERT_FILE=/etc/ssl/cert.pem`
  (macOS), or write a body with `witness_load.py body` and send it with
  `curl`.
