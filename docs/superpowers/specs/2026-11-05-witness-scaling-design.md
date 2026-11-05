# Witness scaling

Status: draft, 2026-11-05. Owner direction: "we should also make sure the
witness is scalable." Companion to
[2026-11-05-witness-at-submission-design.md](2026-11-05-witness-at-submission-design.md),
whose section 6 names witness throughput as the binding constraint on
submission and leaves raising it to witness work. This is that work: what
the cost is, what changed in-process, and how to run more than one witness.

Nothing here has been deployed. The witness redeploy is held until clients
refresh accepted measurements (companion spec, section 7). Every number
below is labelled measured or modelled.

## 1. Where a request's time goes

Measured locally on 2026-11-05: the release `trace-commons-witness` binary
in `full-pipeline` / `near-ai`, booted against a stand-in dstack agent and
a loopback stand-in for NEAR AI's `privacy/classify`
(`scripts/witness-load/`, see `docs/operator/witness-load-testing.md`). The
stand-in models the endpoint's latency as 1.3 s + 32 ms per KiB of request
body, fitted to the 2026-09-03 live measurements (6.4 KB in 1.5 s, 203 KB in
7.1 s, 243 KiB in 9.3 s). All text is synthetic.

**The witness's own work is negligible.** With a zero-latency classifier,
a 241 KB contribution split over 120 tool results takes 0.17 s end to end.
900 KB over 400 tool results takes 0.41 s, and 2 MB in one field 0.38 s.
Parsing, the deterministic detectors, serialisation and signing are all
under 1% of a request with a real classifier behind it.

**The time is classifier round trips, one after another.** `redact_trace`
asks the classifier about every text leaf in turn: event content, metadata
strings, structured-payload strings and keys. On main the NEAR AI adapter
also classifies a field's windows one at a time
(`MAX_CONCURRENT_CLASSIFY_WINDOWS = 1`). A cold witness (fresh process, so
no window-cache hits), measured:

| synthetic contribution | classify calls | sequential (today) |
|---|---|---|
| 241 KB in 1 tool result | 13 | 24.0 s |
| 50 KB over 20 tool results | 58 | 77.3 s |
| 241 KB over 60 tool results | 140 | 190.2 s |

Of the calls a 120-result contribution makes, 68% carry under 200 bytes of
input (ids, argument strings, keys); these are what 2.3 batches. Each still costs the fixed latency, so
call count, not bytes, sets the wall time. A real session with hundreds of
events passes the witness's 300 s limit on call count alone.

**The live witness, 2026-11-05 ~04:10Z** (a handful of synthetic requests,
one at a time): every request with text the witness had not seen before was
refused with `503 witness_redaction_failed` after about 122 s, whether
20 KB or 241 KB. A cached "user: hello" answered in 0.08 s. 122 s is the
NEAR AI adapter's four attempts against its 30 s per-call timeout, so the
classifier call itself was failing from inside the CVM at that time. The
earlier 241 KB success in 114.7 s fits the same pattern: three timeouts
and then a success. Diagnosing that is an operations task, outside this
spec. It does mean the live latency is not the model above.

The owner then measured the endpoint directly from a Mac with a valid key
(2026-11-05, synthetic text): 20 KiB in 11.3 s, 103 KiB in 14.8 s, 205 KiB
in 36.5 s. That is several times slower than the September model, and the
fixed cost per call (~11 s) is what grew most. At that rate each text leaf
costs over 10 s on its own, which makes call count, the subject of 2.1 and
2.3, matter even more than the stand-in tables below show. It also put a
full window past the adapter's 30 s per-call timeout; #1973 sizes the
timeout by body and lowers the window cap for that.

## 2. In-process changes (this branch)

### 2.1 Classify a contribution's leaves concurrently

`witness_service::prefetch::PrefetchingContributionRedaction` replaces
`PipelineContributionRedaction` for `full-pipeline`. It runs three passes:

1. **Record.** The unmodified pipeline runs with a classifier that records
   each text it is asked about and finds nothing.
2. **Classify.** The distinct texts go to the real adapter, a bounded number
   at once.
3. **Replay.** The unmodified pipeline runs again, answered from those
   results.

The classifier stage never feeds the deterministic stage. Placeholder
counters, the scan budget and every later classifier input come from the
raw contribution alone, so the recorded texts are exactly the ones the
replay asks about. The replay is the same call the sequential witness
makes. Tests assert the same envelope bytes and the same private edit maps
against the sequential pass, for the same classifier answers. A classifier
failure is still a refusal: answers are taken in pipeline order and
classification stops at the first error. A text the replay asks about that
has no answer is a refusal too (`witness_classify_answer_missing`), never a
fresh call.

The bound is per process (`TRACE_COMMONS_WITNESS_MAX_CLASSIFY_IN_FLIGHT`,
default 2), shared by every request, because every request spends the same
classifier key. Two comes from the one controlled measurement there is
(2026-08-28: two concurrent requests passed, four failed together and
briefly poisoned the key). Today's witness already allows four overlapping
calls, one per request slot, so two tightens the worst case. Measured with
the stand-in:

| contribution | in flight 1 | 2 | 4 |
|---|---|---|---|
| 241 KB / 1 result | 24.0 s | 14.8 s | 10.8 s |
| 50 KB / 20 results | 77.3 s | 38.7 s | 20.0 s |
| 241 KB / 60 results | 190.2 s | 95.2 s | 47.7 s |

Under load the default costs something. Four 50 KB / 20-result requests
sent at once, cold, measured:

| binary | each request | peak classify calls in flight |
|---|---|---|
| sequential (main's wiring) | 93.0 s | 4 |
| this branch, in flight 2 | 119.1 s | 2 |
| this branch, in flight 4 | 86.4 s | 4 |

Main already puts up to four calls in flight when four requests coincide.
At 2, a lone request runs twice as fast as on main, but a saturated witness
runs slower than main, because four requests share two permits. At 4,
neither case regresses, and key-wide overlap is no higher than main's worst
case. The catch is that one large request now reaches that worst case on
its own, where on main it took four users at once. **Owner decision: 2 or
4.** The binary ships 2 until the endpoint is re-measured.

The stand-in scales linearly because it does not model the endpoint's
overlap trip. Raise the default only after re-measuring the real endpoint
with synthetic text, never with a key a live driver is using.

### 2.2 Size NEAR AI windows by the encoded request body

Owner decision, 2026-11-05. NEAR AI windows are now cut so that each request
body, as serialised, fits a cap under the ~256 KiB limit measured on
2026-09-03 (240 KiB at first, 128 KiB since; see below). The 32,000-token budget is removed and
`TRACE_NEAR_AI_PRIVACY_MAX_INPUT_TOKENS` is refused at startup; the
replacement is `TRACE_NEAR_AI_PRIVACY_MAX_REQUEST_BODY_BYTES`. When a body
is over the cap or draws a 413, the window is halved, at most four levels
deep, and each half is classified. Spans are mapped back in codepoints, and
no cut falls inside a codepoint. This change applies to every NEAR AI
caller (contributor, ingest backstop, witness). The tokenizer windowing now
compiles only for the self-hosted backend.

This saves calls only on fields over about 120 KB. For the traces in
section 1, 2.1 and 2.3 are the levers.

**Revised the same day: a 128 KiB cap and a timeout that grows with the
body.** The owner's live measurement (20 KiB 11.3 s, 103 KiB 14.8 s,
205 KiB 36.5 s) put a 240 KiB window past the adapter's flat 30 s per-call
timeout. Every full window would then have run four timed-out attempts and
been refused. That is the 122 s `witness_redaction_failed` the live witness
returns, reproduced locally with the stand-in at the measured curve.

Two settings changed together:

- **The window cap is now 128 KiB.** Past ~100 KiB the endpoint gets
  slower per byte (two 103 KiB requests take 29.6 s, one 205 KiB request
  takes 36.5 s), so smaller windows cost no time.
- **Each request's timeout is 15 s + 250 ms per KiB of body**
  (`TRACE_NEAR_AI_PRIVACY_TIMEOUT_MS`, `TRACE_NEAR_AI_PRIVACY_TIMEOUT_PER_KIB_MS`).
  That is 47 s at the cap, about 2x the expected time, and 20 s at 20 KiB.

A full window's worst case, four timed-out attempts plus backoff, is
189.75 s. A compile-time assert keeps it at or under 240 s, and a witness
test keeps it under the 300 s request limit, which stays under the client's
330 s deadline.

Measured with the stand-in at the 2026-11-05 curve, one 241 KB field:

| settings | result |
|---|---|
| old: 240 KiB windows, flat 30 s | 503 `witness_redaction_failed` at 127 s |
| new | 200 in 39.5 s |

A 900 KB field: 200 in 133 s. The feature still lists
`dep:tiktoken-rs`, now unused on this path, because removing it would change
the separate GTK and Tauri lockfiles; drop it together with a refresh of
those lockfiles.

### 2.3 Batching small leaves, off by default

The endpoint accepts an array of inputs. Batching was reverted in #468
because the endpoint then limited tokens per request, and no one has
measured it under today's body limit. So it is built and off:
`TRACE_NEAR_AI_PRIVACY_MAX_BATCH_INPUTS`, default 1.

When it is on, `PrivacyFilterAdapter::redact_texts` packs texts that fit
one window into input arrays, in order. Packing stops at the item limit and
at the encoded-body cap, and each answer is mapped back by its index.

- A text that is blank, too large for one window, or already cached goes
  through `redact_text` exactly as it would alone.
- A failed request, or a response whose entry count differs from its
  input count, fails every text in that request.
- A span that fails verification in any entry fails the whole batch.
- A 413 halves the batch.

The witness hands the adapter `preferred_batch_inputs()` texts per permit,
so the in-flight bound counts requests, not texts. Tests show each batched
answer equals the unbatched one, and that the witness's envelope and edit
maps are unchanged with batching on.

**Measured 2026-11-05, owner's key, synthetic text: correct, not faster.**
An array returns one entry per input, and each entry's spans match the same
input sent alone. But the endpoint appears to work through the array one
item at a time:

| inputs per request | time | per input |
|---|---|---|
| 1 (small, alone) | ~2.0 s | 2.0 s |
| 4 | 6.41 s | 1.6 s |
| 16 | 25.50 s | 1.6 s |
| 64 | 87.00 s | 1.4 s |

Batching saves about 0.5 s per input, and it lengthens each call, which
works against the per-call timeout and the request budget. **It stays off**
(owner decision). The levers left are parallel calls (the `overlap`
probe), more keys or replicas, and not classifying leaves that cannot
carry prose (#1977).

### 2.4 Known gap: the window cache stops filling

The NEAR AI adapter's window cache stops inserting after 20,000 entries
(`MAX_CACHED_CLASSIFY_WINDOWS`). It was sized for a driver tick, but the
witness holds one adapter for its whole life, so a long-running witness's
cache goes cold for good. Correctness is unaffected, and 2.1 does not depend
on that cache (it keeps its own answers per request). An eviction policy
would restore the cross-request hit rate.

## 3. More than one witness

### 3.1 Identity: replicas share key and measurement

Read from dstack source (`Dstack-TEE/dstack`, 2026-11-05):

- `GetKey` derives from the app's k256 key: `derive_key(k256_app_key,
  [path])` in `guest-agent/src/rpc_service.rs`. The KMS derives that key
  from its root and **the app id only**:
  `derive_k256_key(&root, &app_id)` in `kms/src/main_service.rs`. The
  instance id enters only the disk-encryption key. So every instance of
  one app id derives the same witness signing key and the same signing
  address.
- MRTD is the image, and MRCONFIGID carries the compose hash. Replicas of
  one app id at one compose hash therefore report the same
  `mrtd:..+mrconfigid:..` label the witness pins. The instance id is
  measured into RTMR3, which the witness does not pin.
- Phala exposes this as `phala cvms replicate <app_id>` ("Create a replica
  of an existing CVM"; `--compose-hash` is required when the app has
  several live instances; `-e` takes a per-replica env file). Not run:
  this spec changes no live state.

Consequence for the accepted-measurements list (companion spec 7.2):
**replicas need no extra entries.** One entry, `{measurements,
signing_address}`, covers every replica of that app id at that compose
hash. A rollout that replicates *before* moving the compose must keep all
replicas on one compose hash, or each hash needs its own entry. Run the
rollover in 7.2 as written (publish old + new, move every replica, publish
new only), with every replica in the signed list before it serves.

A fresh `phala deploy` creates a new app id and so a **new signing
address**. Grow by replicating the existing app, never by deploying a
second one.

### 3.2 Routing

The witness URL names the app id (`<app_id>-8088.<gateway domain>`). The
dstack gateway resolves an app id to its instances and connects to one of
the top `connect_top_n` by recent WireGuard handshake, or to a random one
(`select_top_n_hosts`, `gateway/src/main_service.rs`). It routes by
connection, not by how busy a replica is. Two things follow:

- **Admission control stays in each replica.** Each witness keeps its
  request slots and answers `503 witness_saturated` with `Retry-After`
  when full. The client already treats that as transient and retries with
  backoff (companion spec section 6). A retry is a new connection, so it
  can land on another replica. This needs no front end of our own, and
  none should be added inside the trust boundary.
- **Per-instance URLs** (`<instance_id>-8088...`) bypass the balancer.
  Clients must not pin them, or a replica's removal breaks them.

### 3.3 The shared classifier key is the real limit

All replicas spend one NEAR AI key unless each replica gets its own in its
env file. The key's tolerance for overlap is unknown since 2026-09-03 (see
2.1). Key-wide overlap is `replicas x TRACE_COMMONS_WITNESS_MAX_CLASSIFY_IN_FLIGHT`,
so adding replicas without lowering the per-replica bound raises pressure
on the key. Choose the key-wide budget first, then divide it across the
replicas. Replicas buy availability and CPU headroom, which section 1 shows
is not the constraint. More classifier throughput means more keys, or a
higher limit agreed with NEAR AI.

### 3.4 Fairness

Within a replica, the classify permits are a FIFO semaphore. Each request
holds at most `max_in_flight` queued acquisitions at a time, so concurrent
requests interleave and a large trace cannot starve a small one. Across
replicas, the gateway's connection spread and the busy answer are what
balance load. The existing reservation of interactive slots
(`TRACE_COMMONS_WITNESS_RESERVED_INTERACTIVE_SLOTS`) still keeps background
submission work from taking the last slot.

### 3.5 Capacity metrics (label-only)

Each replica already logs label-only lines. For capacity, add counters,
never content:

- classify calls per request, and calls per second;
- classify in-flight and permit wait time;
- request slots in use, and `witness_saturated` answers;
- request duration by outcome (`ok`, `witness_redaction_failed`, timeout);
- 413 halvings.

These are counts and durations. Where the operator can read them is the
open question: CVM logs are public as Phala deploys them today (see memory
`phala deploy ignores app-compose.json`). Label-only content makes that
acceptable. A metrics endpoint would be another unauthenticated surface,
and has to stay counts-only to the same standard.

## 4. Throughput and cost, from the measurements

Per replica, with the classifier at the modelled latency:

- calls per contribution ≈ distinct text leaves + extra windows for fields
  over ~120 KB. In the synthetic shape that is about 2.3 calls per tool
  call and result pair, cold.
- wall time ≈ calls x ~1.35 s / in-flight.
- sustained contributions per hour per replica ≈ 3600 x in-flight /
  (calls per contribution x ~1.35 s). At in-flight 2, a 140-call trace
  takes about 95 s, so one replica clears about 38 such traces an hour
  while using none of its CPU.

So **classifier calls per hour is the capacity unit**, and the key sets
it. At in-flight 2 that is about 5,300 calls an hour per key. Witness CPU
and memory set no practical limit at these rates: about 0.4 s of CPU per
900 KB, plus the existing 64 MiB request cap. The cheapest CVM size that
holds four 64 MiB requests is enough. Adding replicas raises availability,
not throughput, unless each brings its own classifier capacity. This spec
gives no dollar figures: Phala's CVM pricing and NEAR AI's per-call pricing
are not in this repository, and nobody has measured them.

## 5. Submit-then-poll witness API (design sketch, owner request 2026-11-05)

Today a review is one long HTTP request: the client sends the raw session
and waits, with nothing on the wire, for up to 540 s. That couples the
review's length to every timeout between client and witness (the dstack
gateway's 10-minute idle close is the binding one), and a dropped
connection loses the work. Submit-then-poll decouples them.

**Routes.**

- `POST /v1/witness/jobs` takes the same body as `POST /v1/witness`, plus
  the contributor nonce and a fetch-secret hash (below). It answers at
  once with `202 {job_id, poll_after_secs, queue_position}`, or the
  existing immediate busy when the queue is full or the queued wait would
  pass the job's lifetime.
- `GET /v1/witness/jobs/{job_id}` answers `pending` with the queue
  position, `done` with the same response body and certificate headers
  `POST /v1/witness` returns today, or a refusal label.
- `DELETE /v1/witness/jobs/{job_id}` cancels.

**The nonce still binds the result to this client's request.** The client
sends its nonce in the POST, exactly as it sends it now. The witness binds
it into the certificate it signs when the job finishes; nothing about the
certificate format changes. Two requests do not weaken the binding: the
certificate covers the nonce and the redacted bytes whichever response
carries it. The attestation quote is still fetched separately by nonce
(`/v1/attestation`), unchanged.

**Who may fetch.** `job_id` is 128 random bits, but an id travels in URLs
and logs, so it is not the only capability. The POST carries
`sha256(fetch_secret)`, where the secret is 32 random bytes the client
keeps. The GET and DELETE carry the secret itself, in a header. The witness
compares hashes. A leaked job id alone fetches nothing.

**Retention and one-time fetch.**

- The raw session is held only until its job is processed, then dropped.
- The redacted result is held until its first successful fetch, then
  deleted.
- An unfetched result expires after a short time (proposal: 15 minutes),
  as does a job that never ran.
- A fetch after deletion answers `gone`. The client then resubmits; the
  approval covers the same send to the same witness.

**Storage: memory only.** A raw session never touches disk, sealed or not.
Sealed disk would let jobs survive a restart, at the cost of raw transcripts
at rest inside the CVM and a deletion story for them. A restart losing
in-flight jobs costs a resubmission, and the client already retries. Memory
is bounded by the job limit times the 1 MiB body cap.

**Limits and backpressure.**

- A bounded job table (proposal: 64 jobs, about 400 MiB worst case at six
  copies per job; or count result bytes and drop raw bytes as each job
  runs).
- Jobs run in arrival order through the same classify queue and permits as
  today (#1973).
- A POST is refused busy when the table is full or the estimated wait
  would pass the job lifetime, so a job is never accepted only to expire.
- `poll_after_secs` comes from queue position times the recent per-job
  time, so clients do not poll hot.
- `GET` is cheap and outside the job bound, like `/v1/attestation`.

**Cancellation.** `DELETE` with the secret drops the raw session at once,
if the job has not started. A running job is cancelled at its next
classifier call: no further calls start, and in-flight ones are dropped.
Its result is never produced.

**Replicas.** The gateway routes the app-id URL to any instance (3.2), so a
GET can land on a replica that does not hold the job. Two options:

- **Instance-addressed polling (recommended).** The 202 names the
  instance; the client polls `<instance_id>-8088.<gateway>`, which the
  gateway routes to that instance directly. Every replica shares the
  measurement and signing key (3.1), so the client's checks are unchanged.
  If the instance is gone, the poll fails, and the client resubmits through
  the app-id URL.
- **Shared job state across replicas.** This would move raw sessions
  between CVMs or into a store outside them. Not proposed.

**Privacy.**

- Raw text is held only in the CVM's memory, only until processed.
- The redacted result is held only until fetched or expired.
- Job ids and fetch secrets are never logged.
- `/v1/diagnostics` gains label-only queue counts (jobs pending, running,
  done-unfetched, expired), never ids.

**Client changes.** The witness worker (#1967) submits, keeps the job id
and secret in memory, and polls on `poll_after_secs` with jitter. Its
existing transient-retry path covers `busy`, `gone` and an unreachable
instance. The interactive preview can use the same path, so a person
waiting sees "Running privacy check..." with no 540 s connection
underneath. `POST /v1/witness` stays for older clients until they age out.

**Open questions for the owner.**

1. Result lifetime: is 15 minutes right? Longer helps a sleeping laptop
   and costs memory.
2. Memory only (lost on restart), or sealed disk for results only (never
   raw)?
3. Should the queue position be shown to the person, or kept internal?
4. Instance-addressed polling ties a job to one CVM. Is losing jobs when
   that CVM restarts acceptable?
5. Keep `POST /v1/witness` indefinitely, or set a date to retire it?

## 6. Order of work

1. Find why the live witness's classifier calls failed on 2026-11-05 (CVM
   logs, `near-ai privacy classify failed` labels, key status).
2. Re-measure the endpoint with synthetic text, at low volume, on a key no
   driver is using (`scripts/witness-load/near_ai_probe.py`). Done
   2026-11-05: `latency` (dense text ~3 s + 0.4 s/KiB; it set #1973's
   64 KiB cap and 15 s + 600 ms/KiB timeout) and `batch` (correct, not
   faster; see 2.3). Still to run: `limit`, and `overlap` (2 then 4
   requests at once, 8 requests in all). Overlap is the measurement that
   poisoned the key last time, so it runs last, deliberately, on its own key.
3. Raise `TRACE_COMMONS_WITNESS_MAX_CLASSIFY_IN_FLIGHT` above 2 only if
   `overlap` shows 4 holds. Ship with the redeploy the companion spec
   already sequences.
4. Batching (2.3) stays off; the numbers say it does not pay.
5. Replicas (section 3), when availability matters or when per-replica
   keys are available.
