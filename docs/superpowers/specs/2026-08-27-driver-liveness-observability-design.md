# Driver liveness and failure-class observability

Design for #438. Two subsystems were completely dead in production and
neither raised anything louder than a `WARN`. One had been dead for five
days, emitting the same hash-only warning every 45 seconds -- 6,999 times --
and nothing noticed.

This is an observability gap, not a bug in either subsystem. The transient
failure work in #437, #441 and #448 fixed what the outage *did* to traces;
this fixes the fact that nobody could tell it was happening.

## Why the existing signal failed

Three separate properties, each individually reasonable, combined into
silence.

**A repeating identical warning reads as steady state.** The same
`error_hash` 6,999 times conveys "this is normal here", which is exactly
backwards. A subsystem failing *every single time* is more serious than one
failing intermittently, not less.

**Hash-only logging is correct policy and hostile to triage.** Decoding
`sha256:37769fdd...` required reading the code that built the string,
enumerating candidate error texts, probing the live endpoint to recover a
body hash and length, and reproducing the sha256 byte for byte. That is a
fine forensic tool and a terrible alerting mechanism.

**Nothing distinguishes one failure from five days of them.** Both are a
`WARN`. There is no state anywhere that answers "when did this last work?".

## Scope: all twelve loops, not the two that died

The two drivers named in the incident are not special. Twelve spawned tasks
in `trace-commons-ingest.rs` share a byte-identical shape:

```rust
tokio::spawn(async move {
    loop {
        tokio::time::sleep(config.interval).await;
        match run_..._tick(...).await {
            Ok(summary) => tracing::info!(..., "... tick completed"),
            Err(error) => tracing::warn!(
                error_hash = %safe_display_error_hash(&error),
                "... tick failed"
            ),
        }
    }
});
```

Export jobs, NEAR credit outbox, retention maintenance, vector index,
perplexity scoring, PII backstop, benchmark registry, benchmark pipeline,
credit cycle, credit settlement, process evaluation, and revocation
propagation. Every one has the blind spot; two of them happened to be the
ones pointed at a vendor that ran out of credit.

Fixing two would leave ten loops able to reproduce this issue exactly. The
work goes through a shared wrapper so all of them get it at once.

## What this is NOT

The incident report suggests `/health` as the home for liveness, on the
basis that it "already reports build and privacy-filter status". It does
not: `health_handler` returns `status`, `schema_version`, `build_commit`,
`build_time` and `build_version`, takes no `State`, and reads nothing.

More importantly `/health` is unauthenticated -- registered on the router
before any auth layer. "Which subsystem is currently dead, and for how long"
is operational intelligence: it tells an unauthenticated caller when the PII
backstop is not running. Liveness therefore goes on an admin-gated route and
`/health` is left alone.

## Design

### 1. Failure class

```rust
enum DriverFailureClass {
    UpstreamUnavailable,
    ConfigMissing,
    DependencyUnavailable,
    ContentRejected,
    Unclassified,
}
```

`as_label()` returns a stable snake_case string. The label sits **alongside**
`error_hash`, never instead of it: the hash stays for forensics, the label
makes the log greppable and the class comparable across ticks. Neither
carries error text, a URL, a vendor response body, or anything about a trace,
so the hash-only convention is preserved intact.

Classification reads the error's **type**, never its message, matching the
discipline the existing per-driver classifiers already use:

```rust
fn classify_driver_failure(error: &anyhow::Error) -> DriverFailureClass
```

downcasting to the typed markers that exist today -- `ScorerFailure` and
`TraceContributionError`, both of which already expose `is_transient()`. An
unrecognised error is `Unclassified` rather than guessed at. No per-driver
override until a driver demonstrably needs one.

### 2. Liveness state

```rust
struct DriverLiveness {
    driver: &'static str,
    interval: Duration,
    started_at: DateTime<Utc>,
    last_success_at: Option<DateTime<Utc>>,
    last_failure_at: Option<DateTime<Utc>>,
    consecutive_failures: u64,
    last_failure_class: Option<DriverFailureClass>,
    last_error_hash: Option<String>,
    escalated: bool,
    suppressed_since: Option<DateTime<Utc>>,
    suppressed_count: u64,
}
```

Held in a `DriverLivenessRegistry` on `AppState`, keyed by a `&'static str`
driver name.

**In memory, deliberately.** A Postgres-backed table would survive a restart
and be readable with `psql` while the server is down, which is the one real
advantage. Against that: a migration, an RLS policy for a cross-tenant
operational table, and a write per driver per tick -- the PII backstop ticks
every 45 seconds. And a `last_success_at` that survives a restart is worse
than no value at all, because after a restart the process genuinely does not
know whether the driver works; an in-memory `None` says that honestly.

### 3. The decision is a pure function

```rust
fn observe_driver_tick(
    prev: &DriverLiveness,
    outcome: DriverTickOutcome,
    now: DateTime<Utc>,
) -> (DriverLiveness, LogAction)
```

`LogAction` is an enum the loop executes: emit a warning, emit an escalated
error, suppress, emit a recovery line.

This is the load-bearing structural decision. Every rule below -- escalation
threshold, suppression, recovery -- becomes testable as a table over
`(previous state, outcome, clock)` with no spawned task, no sleeping, and no
injected clock trait. The spawn wrapper reduces to calling this and matching
on the result.

### 4. Escalation by elapsed time

A driver is **stale** when

```
now - last_success_at > max(3 x interval, 5 minutes)
```

Before any success has been recorded, the window is measured from
`started_at` instead.

Elapsed time rather than a consecutive-failure count, because the intervals
differ by an order of magnitude and are operator-configurable well beyond
that. Defaults in the tree are 45s, 60s and 300s; the scheduler env parsers
accept anything from 5 seconds to 86,400. "Three consecutive failures"
therefore means two and a quarter minutes for one driver and three days for
another. A multiple of the driver's own interval scales across that range,
and the five-minute floor stops a fast driver escalating on a brief blip.

Against the current defaults the threshold resolves to 5 minutes for the 45s
and 60s drivers and 15 minutes for the 300s ones.

### 5. Log volume

| Transition | Level | Carries |
|---|---|---|
| First failure after a success | `WARN` | class, hash |
| Crossing the stale threshold | `ERROR` (once) | class, hash, consecutive count, duration dead |
| Still failing, already escalated | suppressed | -- |
| Every 15 min while escalated | `ERROR` | class, hash, count, duration dead, suppressed count |
| First success after failures | `INFO` | failures survived, duration dead |

Two things this fixes beyond the level change. The 6,999 identical lines
become roughly 480 over the same five days, each one carrying a duration that
makes the age obvious without arithmetic across timestamps. And there is a
**recovery** line, which the incident had no equivalent of at all -- the
outage's end was as invisible as its start.

### 6. Admin surface

`GET /v1/admin/driver-liveness`, admin-gated like its neighbours, returning
per driver: name, interval, `last_success_at`, `consecutive_failures`,
`last_failure_class`, `last_error_hash`, and a derived `stale` boolean.

A dedicated route rather than a section on `/v1/admin/operational-summary`,
because that handler is tenant-scoped, performs DB reads, and writes an audit
event per call. Driver liveness is process-global and tenant-independent;
attaching it there would be a category error and would put a DB round trip
and an audit write on what should be a cheap poll.

## Testing

Table-driven over `observe_driver_tick`:

- **The incident.** A driver failing every tick for five simulated days emits
  one `WARN`, then escalated `ERROR`s at the repeat interval -- not one line
  per tick.
- **Flapping does not escalate.** Alternating success and failure never
  crosses the threshold, because each success resets `last_success_at`.
  Escalation must mean "dead", not "unreliable".
- **Recovery emits exactly one `INFO`**, carrying the true duration and count.
- **Threshold scales.** A 45-second driver and an hourly driver escalate at
  their own multiples, not a shared constant.
- **Classification** maps each typed marker to its stable label, and an
  unrecognised error to `Unclassified` rather than to a guess.
- **Registry keys are distinct** across all twelve drivers, so the shared
  wrapper cannot silently collide two and report one driver's health for
  another.

No test sleeps or spawns; `now` is a parameter.

## What this does not do

It does not alert. Nothing currently watches the pilot's logs -- they are
written to `/var/log/tracecommons/ingest.log`, outside the journal -- so an
escalated `ERROR` still requires a human to look. This design makes the
outage *legible in one query* and greppable by a stable label; wiring that to
a pager is separate work, and the admin route is the hook for it.
