# Transient upstream failures must not condemn a trace

Design for the PII backstop driver's attempt accounting.

## The incident that produced this

On 2026-08-26 the NEAR AI account behind the privacy classifier ran out of
credit. Two things followed.

Ticks aborted at the canary for five days with a constant `error_hash`. Once
credit was restored, ticks began completing -- and every item failed instead,
with a different constant hash. Both hashes were decoded by reproducing the
logged string byte-for-byte; the second is:

```
trace contribution redaction failed: near-ai privacy classifier returned
non-2xx: status=502 body_hash=sha256:5e6c4f58... body_len=127
```

whose body is NEAR AI's own text: `"Privacy classify request failed. Please
try again later."` -- an upstream server error, not our bug.

Within roughly twenty minutes, 37 held traces had accumulated failures and
**16 had reached `attempts=5`**, the point at which the enumeration filter
excludes them permanently. Nothing was wrong with any of those traces.

## The two defects

### 1. The driver discards a distinction the adapter already makes

`privacy_filter_near_ai` retries transport errors and 5xx up to
`MAX_CLASSIFY_ATTEMPTS` with exponential backoff, and deliberately does not
retry 4xx or body-shape failures. That is the correct transient/permanent
split, and it is made per request.

It is then thrown away: the adapter returns an opaque error, and
`process_one_pii_backstop`'s caller bumps the permanent counter for any `Err`
at all. A vendor's bad afternoon is charged to the trace.

### 2. The canary is not representative of real load

The canary gates the whole tick, and it passed throughout -- because its
synthetic text is small enough to succeed while real envelopes were 502ing.
A gate that cannot fail the way production fails is not a gate.

## The design

### Transient failures do not bump

Classify the failure at the driver:

- **Transient** -- transport error, timeout, 5xx: log hash-only, **leave the
  attempt counter untouched**, retry on a later tick.
- **Permanent** -- 4xx, malformed body, oversized input, or anything about
  the trace itself: bump exactly as today.

A trace is then only ever excluded for something wrong with *the trace*.

**The classification must be typed, not parsed.** Do not string-match
`status=502` out of a reason field. The adapter knows which case it is at the
point it decides whether to retry; that knowledge must reach the driver as
structured data -- a variant, a flag on the error, or an `is_transient()`
accessor. A future error string edit must not silently change retry policy.

### A consecutive-failure circuit breaker

Transient failures no longer costing anything means a dead upstream would be
hammered indefinitely. So the tick aborts after N consecutive per-item
failures, leaving the rest of the batch untouched.

This is what catches an unrepresentative canary: the canary predicts, and the
breaker stops the tick when the prediction was wrong. The canary itself is
NOT changed here -- growing it to "realistic" size is a guess that drifts as
payloads change, and it costs a real classify call every tick.

### Not in scope

- No change to the canary.
- No change to `MAX_CLASSIFY_ATTEMPTS` or the adapter's backoff.
- No change to the scoring driver. It has the same shape and probably the
  same defect, but it is a separate path and should be a separate change
  once this one is proven.

## Operational note

**Resetting the exhausted traces must come after this ships**, not before.
Resetting first re-arms the same trap: the 16 already-excluded traces would
re-exhaust within the hour on the same upstream flakiness.
