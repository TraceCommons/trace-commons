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
input (ids, argument strings, keys). Each still costs the fixed latency, so
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
spec. It does mean the live latency is not the model above, and the model
needs re-measuring against the endpoint when it is healthy.

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
classification stops at the first error.

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

### 2.3 Not done: batching small leaves

The endpoint accepts an array of inputs. Sending many small leaves in one
request would cut the 68% of calls under 200 bytes to a handful. Batching
was reverted in #468 because the endpoint then limited tokens per request;
since 2026-09-03 the limit is the body size, and nobody has measured batching
under it. That needs a low-volume synthetic measurement by someone holding a
key, with no driver live on it, before any code depends on it. It would sit
behind the same answer cache as 2.1, so output identity is tested the same
way.

### 2.4 Known gap: the window cache stops filling

The NEAR AI adapter's window cache stops inserting after 20,000 entries
(`MAX_CACHED_CLASSIFY_WINDOWS`). It was sized for a driver tick, but the
witness holds one adapter for its whole life, so a long-running witness's
cache goes cold for good. Correctness is unaffected, and 2.1 does not depend
on that cache (it keeps its own answers per request). An eviction policy
would restore the cross-request hit rate.

### 2.5 Skip the classifier for keys and structural tokens (owner decisions 2026-11-05)

On by default, in one function the client's redaction and the witness's
both call (`classifier_skips_structural_text`, `trace_contribution.rs`).
The owner accepted the first rule, then took the recommended tightening
the same day ("go with recs"). The rule as built:

1. **An identifier-shaped key does not go to the classifier.** A key that
   is not identifier-shaped (by rule 2) is classified like a value:
   `Jane Doe`, `Owner`.
2. **A value is skipped** when all of these hold:
   - it is one to forty characters, each an ASCII letter, digit or one of
     `_ . - / :`;
   - it has no uppercase letter followed by a lowercase one;
   - it has no run of seven or more digits;
   - it is not a dotted name without a slash (which covers IPv4 addresses
     and hostnames);
   - it is not IPv6-shaped (two or more `:` and otherwise hex digits).
3. **A value under an identity key** -- `author`, `user`, `username`,
   `name`, `owner`, `email`, `login`, `committer` or `assignee`, matched
   case-insensitively against the whole key, at any depth below it -- **is
   always classified**, whatever its shape (`IDENTITY_KEYS`).
4. **Everything else is classified**, however short: anything with
   whitespace, any non-ASCII character, any capitalized word (`Jane`,
   `Sam`, `McDonald`), any token over forty characters.

Examples:

- Skipped: `bash`, `edit_file`, `src/main.rs`, `200`, a hex digest,
  `true`, `call_1`, `ERROR`, an ISO timestamp.
- Classified: `Jane Doe`, `Jane`, `Dear Sam`, `fix the login bug`, `José`,
  a 41-character token, `203.0.113.7`, `build.corp.internal`, `main.rs`,
  `2001:db8::1`, `4155550123`, and `jdoe` under `author`.

This is the classifier stage only. The deterministic detectors run on
every field, skipped or not, before the rule is asked. They still remove
`a@b.co`, `+1-415-555-0123`, an AWS access key, `10.0.0.12`,
`https://intranet.example.corp/wiki` and `/home/jdoe/notes.txt`.

**What it still gives up.** A bare lowercase username (`jdoe`) under a key
that is not an identity key is skipped, and the deterministic pass does not
catch it. A test pins this, so the spec cannot drift from the code. Before
the tightening, a public IPv4 address, a bare hostname, an IPv6 address and
an unformatted phone number were skipped as well; they are classified now.

**Savings.** Measured on the load harness's synthetic contributions, a
fresh witness each way, zero-latency stand-in classifier, the tightened
rule, 64 KiB windows:

| contribution | calls, skip off | calls, skip on |
|---|---|---|
| 241 KB in one tool result | 16 | 9 (-44%) |
| 241 KB over 120 tool results | 358 | 239 (-33%) |

Most of the remaining calls are synthetic tool output with whitespace in
it. Real sessions carry more ids, paths and flags, so they should save
more; that is not measured.

**Not covered.** The ingest server's prose-PII backstop
(`rescrub_envelope_prose_pii_with`) is unchanged and still classifies
every string and key it examines. The redaction pipeline alias is
unchanged as well. Whether the skip should move the alias is an owner
question: it changes classifier coverage without changing which
classifier runs.

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

## 5. Order of work

1. Find why the live witness's classifier calls failed on 2026-11-05 (CVM
   logs, `near-ai privacy classify failed` labels, key status).
2. Re-measure the endpoint with synthetic text, at low volume, on a key no
   driver is using: the body limit (binary search with escape-heavy text),
   per-call latency, overlap tolerance at 2/4/8, and array batching.
3. Set `TRACE_COMMONS_WITNESS_MAX_CLASSIFY_IN_FLIGHT` and the body cap from
   step 2. Ship with the redeploy the companion spec already sequences.
4. Batching (2.3), if step 2 shows it holds.
5. Replicas (section 3), when availability matters or when per-replica
   keys are available.
