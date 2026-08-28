# Gate-scoring throughput: implementation plan

Date: 2026-08-27
Status: Proposed
Companion: docs/superpowers/specs/2026-08-27-hackathon-onboarding-friction-design.md
(Slice E makes the backlog visible and signed; this plan drains it.)

All paths are relative to the repo root on branch `devfolio-feedback-3`.

## 1. Where the time goes

### Verified in code

The pipeline has four nested serial layers, and every one of them is a
throughput cap:

1. **The tick cadence is `sleep(interval)` THEN tick, sequentially.**
   `spawn_perplexity_score_driver_task`
   (`crates/trace-commons-server/src/bin/trace-commons-ingest.rs:9194`) loops
   `sleep(config.interval).await` followed by
   `run_perplexity_score_driver_tick`. The sleep does not overlap the tick, so
   the cycle time is `interval + tick_duration`, not `max(interval,
   tick_duration)`. The oft-quoted 6.7/min (batch 5 / 45 s) is therefore an
   UPPER bound that only holds if scoring were instant. Real throughput is
   `batch / (45 s + batch x per_trace_latency)`.

2. **Submissions within a tick are scored strictly sequentially.**
   `run_perplexity_score_driver_tick` (ingest.rs:39209) — `for item in &items
   { score_one_submission(...).await }`. Its own doc comment says "scored
   each sequentially (no concurrency)".

3. **Chunks within a trace are scored strictly sequentially.**
   `Orchestrator::chunk_and_score_perplexity`
   (`crates/trace-commons-gate-enclave/src/orchestrator.rs:104`) loops
   `self.perplexity.score_chunk(...)` per chunk, with the deliberate comment
   "Sequential — never a concurrent burst against one pinned backend". Each
   chunk is one blocking HTTPS round trip to NEAR AI (`perplexity_near_ai.rs`),
   retried up to `MAX_SCORE_ATTEMPTS = 3` with 250 ms-doubling backoff; a
   timeout is non-retryable because the completion may already have billed.
   The whole evaluation runs inside a single `spawn_blocking`
   (ingest.rs:49160-49170).

4. **Embedding is also sequential and local-CPU.** After perplexity, the
   orchestrator embeds every chunk via fastembed (bge-large ONNX) one chunk at
   a time (orchestrator.rs:171-189). The pilot host has 2 vCPUs and is known
   to be CPU-saturated by this embedder (separately established on the pilot;
   re-verify at measurement time).

5. **Enumeration is FIFO cross-tenant with no priority.**
   `list_submissions_needing_gate_decision`
   (`crates/trace-commons-server/src/db/postgres.rs:4522`): `ORDER BY
   s.received_at ASC LIMIT $4`, cross-tenant via the gate-driver role. A
   hackathon tenant's final-hour burst queues behind whatever background load
   arrived first.

6. **Transient failures do not charge the attempt budget; permanent ones do.**
   `score_one_submission` (ingest.rs:50194) bumps
   `trace_gate_evaluation_attempts` on every hard failure but returns
   `GateOutcome::TransientFailed` WITHOUT bumping when
   `is_transient_gate_scoring_failure` finds the typed `ScorerFailure` marker
   (ingest.rs:39296-39306). A submission reaching `max_attempts` (default 5)
   leaves the enumeration permanently (`gate_scoring_exhausted`,
   ingest.rs:50152). Any concurrency change must preserve this typed
   classification exactly.

7. **The circuit breaker is a consecutive-failure counter.**
   `MAX_CONSECUTIVE_SCORE_DRIVER_FAILURES = 3` (ingest.rs:39292): three
   consecutive per-item failures (transient or permanent) abort the rest of
   the tick, resetting on any success. "Consecutive" is only meaningful under
   sequential execution — a concurrent tick must redefine it.

8. **The knobs already exist and are clamped generously.** Interval clamps to
   `[5, 86400]` s, batch to `[1, 1000]`
   (`docs/operator/env-reference.md:346-347`; constants at ingest.rs:997-999,
   env parsing at ingest.rs:6283-6310). A pure-config burst is available
   today — but layers 2-4 mean it saturates at `1 / per_trace_latency`
   regardless of batch size.

9. **No timing is observable.** The tick-completed log (ingest.rs:9214-9223)
   carries counts only — no tick duration, no per-item latency, no backlog
   depth. The 12-window PII precedent (`git show b6091f51`) found its 9-min/
   trace figure only by measuring; we have not measured this path.

10. **The on-demand route always-scores and shares no bookkeeping.**
    `POST /v1/workers/gate/evaluate` (route at ingest.rs:7661, handler at
    ingest.rs:50418) is behind `require_vector_operator`, calls
    `evaluate_and_record_gate` directly — no skip-duplicate/cache cost
    controls, no attempt bump on failure, and nothing prevents it racing the
    driver into a duplicate decision row for the same submission (enumeration
    only filters on `decision_id IS NULL` at read time; there is no
    per-submission claim).

### Inferred / unknown — and how to measure

- **Per-trace latency is unknown.** Plausibly dominated by chunk count x NEAR
  round trip (a 169 KB pilot trace scored as 15 chunks), plus embedder CPU on
  a 2-vCPU host. Which of the two dominates is NOT established. Measure
  before tuning: Task 0 below.
- **The 12% 502 rate** was measured on the privacy-classify endpoint, below
  its size cliff (commit b6091f51). The completions endpoint is the same
  vendor but a different, billed call; assume similar flakiness, verify from
  retry counts once instrumented.
- **Whether the queue actually starves at hackathon scale** follows
  arithmetically from any per-trace latency over ~1 s at default pacing, so
  the qualitative conclusion is safe; the quantitative gap (how much
  concurrency is enough) needs the baseline.

Bottom line, honestly stated: the 45 s interval and batch 5 are a hard
config-level cap (verified), and beneath it the serial structure at all three
levels (tick, submission, chunk) caps throughput at roughly one trace per
per-trace latency even with maximal config (verified structure, unmeasured
latency). Host CPU via the embedder is a plausible secondary constraint
(inferred). The flaky endpoint costs latency via retries but is bounded by
MAX_SCORE_ATTEMPTS (verified mechanism, unmeasured contribution).

## 2. Approaches

### A. Config-only burst (interval down, batch up)

Set `TRACE_COMMONS_PERPLEXITY_DRIVER_INTERVAL_SECONDS=5`,
`TRACE_COMMONS_PERPLEXITY_DRIVER_BATCH_SIZE=50` for the event window. Zero
code, reversible by unsetting env, available for an event next week.

- Ceiling: with interval 5 and a large batch, cycle time is dominated by the
  serial tick, so throughput approaches `1 / per_trace_latency` — better than
  today (which wastes 45 s per 5 traces) but structurally capped.
- Failure mode: none new. The breaker still aborts a tick on 3 consecutive
  failures; transients still don't charge budget. Spend rises linearly with
  scored traces, which at a hackathon is the point.
- Verdict: do it regardless (it is the rollback position for everything
  below), but it does not by itself survive a final-hour field.

### B. Concurrency across submissions within a tick (recommended)

Score up to `N` submissions concurrently inside
`run_perplexity_score_driver_tick`, `N` from a new env
`TRACE_COMMONS_PERPLEXITY_DRIVER_CONCURRENCY`, default `1` (today's behavior,
byte-for-byte). This is the same move as commit b6091f51 (PII windows,
bounded at 8), one level up: the chunk loop inside the orchestrator stays
deliberately serial, so total in-flight NEAR requests are bounded by `N` (one
per trace at a time), not `N x chunks`.

- Why not chunk-level concurrency instead: the orchestrator's serial chunk
  loop is synchronous (blocking reqwest inside `spawn_blocking`) and
  deliberately so; making it concurrent means async-ifying the
  `PerplexityScorer` trait across `trace-commons-gate-api` and both backends,
  a seam change with proprietary-backend implications. Driver-level
  concurrency gets the same wall-clock overlap for a fraction of the surgery,
  and each blocking evaluation just occupies one thread of tokio's blocking
  pool (default cap 512; N will be single digits).
- Failure mode 1 — offered rate against a ~12%-502 endpoint. Concurrency
  multiplies the offered rate; retries multiply it again. Mitigation: default
  `N=1`; recommend 4 for the pilot; document that the PII driver already
  offers 8-concurrent to the same vendor and that the two drivers share the
  endpoint, so the operator should consider both when raising either.
- Failure mode 2 — attempt-budget integrity. `score_one_submission` already
  classifies transient-vs-permanent per item and bumps (or not) internally;
  concurrent execution does not change per-item classification. No change to
  budget semantics is needed, only to the breaker (next point). This is the
  property that must have a regression test: a transient failure under
  concurrency must not bump attempts.
- Failure mode 3 — the breaker. "Consecutive" is undefined across concurrent
  completions. Redefine per tick: stop dispatching NEW items once total
  failures (transient + permanent) in this tick reach
  `MAX_CONSECUTIVE_SCORE_DRIVER_FAILURES`, let in-flight items finish, mark
  `breaker_tripped`. With N=1 this degrades exactly to today's behavior
  (failures with no intervening success are consecutive). Slightly more
  trigger-happy under concurrency for interleaved failures — acceptable: the
  breaker exists to stop paying a dead backend, and the un-dispatched
  remainder is re-enumerated next tick with no budget charged for transients.
- Failure mode 4 — novelty ordering race. Sequentially, trace B's novelty
  sees trace A's just-inserted chunks. Concurrently, up to N near-identical
  traces submitted together can each score high novelty against the
  pre-batch index and all pass. Bounded by N; partially covered by the
  submit-time duplicate precheck (`skip_duplicates`, threshold 900000 micros)
  and the cross-submission cache keyed on `canonical_summary_hash` — but the
  cache is read before scoring starts, so two concurrent same-hash items dodge
  it. Cheap hard mitigation: within a tick, never dispatch two items
  concurrently whose `canonical_summary_hash` matches an item already in
  flight (hold the second back to the next wave). Note that serializing by
  TENANT instead would be useless here: a Devfolio event is one tenant, so
  intra-tenant concurrency is precisely what the hackathon case needs.
- Vector-index safety: per-tenant `Mutex<usearch::Index>`
  (`vector_index_usearch.rs:9,81`), so concurrent nearest/insert serialize
  briefly on the index and are safe.

### C. Backlog-aware pacing (drain mode)

When a tick enumerated a full batch and the breaker did not trip, sleep a
short drain interval (default 2 s) instead of the full 45 s; otherwise sleep
the normal interval. Env `TRACE_COMMONS_PERPLEXITY_DRIVER_DRAIN_INTERVAL_SECONDS`,
default equal to the normal interval (i.e. feature off). Removes the guess-a-
number problem of approach A: quiet periods keep today's cost pacing, a
backlog drains at the speed B allows. The 45 s pacing was a cost control on
paid inference; drain mode spends exactly what the backlog requires and
nothing more, and the breaker still halts spend when the backend is dead.

### D. Tenant priority in enumeration

`ORDER BY (s.tenant_id = ANY($5)) DESC, s.received_at ASC` in
`list_submissions_needing_gate_decision`, with the priority tenant-id list
from env `TRACE_COMMONS_PERPLEXITY_DRIVER_PRIORITY_TENANTS`
(comma-separated tenant ids; operator config, not logged — logs stay
hash-only). FIFO within each class, so background load still drains, just
behind the event tenant. Complements B and C; on its own it reorders
starvation rather than fixing it.

### Rejected: wiring `POST /v1/workers/gate/evaluate` into a drain path

The handler bypasses skip-duplicate and cache cost controls, does not bump
attempts on failure, and can race the in-process driver into duplicate
decision rows for one submission (finding 10). Making it safe means giving it
the cost-control wrapper plus a per-submission claim — real work that buys a
second scoring path to keep consistent, when B+C reach the same throughput
inside the path that already has correct bookkeeping. Leave the route as the
manual one-off tool it is; do not wire it.

### Recommendation

Task 0 (measure) now; A as the event-week stopgap; B + C as the fix, in that
order; D if event traffic will share the pilot with background load (it
will). All defaults preserve current behavior exactly; the pilot opts in by
env.

## 3. The plan

### Baseline, measured 2026-08-27 (Task 0 step 1, done)

Taken on the pilot before any tuning. **The headline number changes the
diagnosis.**

From `/var/log/tracecommons/ingest.log`, over 129 ticks that scored at least
one trace, taking `(gap between consecutive tick lines - 45 s interval) /
scored`:

| per-trace scoring time | |
| --- | --- |
| p50 | 287 s (~4.8 min) |
| p90 | 385 s |
| max | 918 s |

From the database, via the gate-driver role (the `app` role cannot see these
rows: it is `NOBYPASSRLS` against forced RLS with no tenant GUC, and returns
zero for everything, which is not the same as there being nothing):

| | |
| --- | --- |
| submissions | 1055 |
| scored | 1035 |
| unscored | 20 -- of which 13 at 0 attempts, 3 at 1, 1 at 4, **3 at max_attempts and therefore permanently out of the queue** |
| queue latency `decided_at - received_at` | p50 39 min, p90 32.5 h, p99 22 days |

The queue-latency tail spans the pilot's whole history including outages and
the PII-backstop wedge, so it overstates steady state. There has been no
traffic for 30 days, so a clean steady-state figure would need a load run.

**The queue-depth rows above are a snapshot and have since moved**: the
unscored traces were requeued shortly after this was taken, so the 20 and the
three exhausted no longer describe the pilot. The per-trace timing does not
move -- it is derived from historical tick logs -- and it is the number this
plan turns on.

**What this says.** At ~287 s per trace, scored one at a time, throughput is
about **12.5 traces/hour** -- not the ~6.7 per *minute* the config-level
arithmetic implies. The 45 s interval and batch of 5 are not the binding
constraint and tuning them alone buys almost nothing: a field of 200
submissions in a final hour would take roughly 16 hours to clear.

That makes Task 1 (concurrency across submissions) the load-bearing change
rather than an optimisation, and it sets the sizing: N=4 gives ~50 traces/hour,
N=8 about 100. It also means the per-chunk serial loop inside a trace, not the
driver's pacing, is where the 287 s lives -- so the next measurement worth
having is the split between NEAR round trips and local embedding inside one
trace.

### Task 0 — baseline and instrumentation (no behavior change)

1. Capture the current baseline on the pilot before any change:
   - Tick timing from existing logs (`/var/log/tracecommons/ingest.log`, not
     journalctl): timestamp deltas between consecutive
     "perplexity score driver tick completed" lines, minus the 45 s interval,
     give tick duration; `scored` per line gives per-trace latency
     (`tick_duration / scored`).
   - Queue latency from the DB (gate-driver or admin credentials):
     percentiles of `d.decided_at - s.received_at` over the last 30 days, and
     the current count of submissions matching the enumeration predicate
     (backlog depth). Record both in the plan's PR description as the
     baseline.
2. Add to `PerplexityDriverTickSummary` and the tick-completed log line:
   `tick_duration_ms`, and `backlog` (a `COUNT(*)` over the same predicate as
   the enumeration query, new `Database` method
   `count_submissions_needing_gate_decision` beside the list method in
   `crates/trace-commons-server/src/db/postgres.rs`). Counts and durations
   only — hash-only policy untouched.
- Files: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs`
  (summary struct, tick fn, spawn log line), `crates/trace-commons-server/src/db/mod.rs`
  + `db/postgres.rs` (count method).
- Tests: unit test for the count query beside the existing pg-store tests
  (`trace_corpus_pg_store`, requires PostgreSQL; note CI never runs pg tests
  — run locally); summary-serialization/log-shape assertions in
  `trace_commons_ingest_internal/tests.rs`.

### Task 1 — concurrency within a tick (approach B)

Depends on Task 0 (so the improvement is measurable against a baseline).

1. New env `TRACE_COMMONS_PERPLEXITY_DRIVER_CONCURRENCY`, default 1, clamped
   `[1, 16]` (16 = 2x the PII bound; anything higher needs new evidence about
   the vendor). Parse beside the existing driver envs (ingest.rs:6283-6310),
   carry on `PerplexityScoreDriverConfig`.
2. Rework `run_perplexity_score_driver_tick`:
   - Dispatch items through a bounded concurrent scheduler (`JoinSet` capped
     at N, or `futures::stream::iter(...).buffer_unordered(N)` — `futures` is
     already a workspace dep via the protocol crate; for the server crate
     confirm it is an existing direct dep before using, else `JoinSet`, which
     is tokio-only and needs nothing new. Do not add a dependency without
     surfacing it).
   - Same-`canonical_summary_hash` hold-back: before dispatching an item,
     if an in-flight item shares its hash, defer it behind that item's
     completion. Requires fetching the hash per item up front — one
     `get_trace_submission` per item, which `score_one_submission` already
     does; hoist or duplicate the lookup, keep it cheap.
   - Breaker: count this tick's failures (transient + permanent, interleaved
     successes reset nothing under concurrency — use a per-tick total of
     failures SINCE the last success completion); once it reaches
     `MAX_CONSECUTIVE_SCORE_DRIVER_FAILURES`, dispatch no new items, drain
     in-flight, set `breaker_tripped`. Document in the constant's comment
     that under N=1 this is exactly the old consecutive semantics.
3. `score_one_submission` and everything below it is untouched — attempt
   budget, transient classification, cost controls all stay where they are.
- Files: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` only
  (constants block ~:995, env constants ~:771, config parse ~:6283, tick
  ~:39209).
- Tests (in `trace_commons_ingest_internal/tests.rs`, following the
  b6091f51 pattern of timing a delayed mock):
  - Overlap regression: N=4 against a scorer stub that delays; wall clock
    must beat sequential (loose bound, property is overlap not speedup).
  - N=1 byte-compatibility: summary identical to the pre-change tick for the
    same script of outcomes.
  - Transient failure under concurrency bumps NO attempt row.
  - Breaker under concurrency: after 3 failures, remaining items are never
    dispatched, in-flight complete, `breaker_tripped` set.
  - Same-hash hold-back: two same-hash items never in flight together, and
    the second hits the cross-submission cache.

### Task 2 — drain-mode pacing (approach C)

Depends on Task 1 (drain mode without concurrency just spins the serial
bottleneck faster; it works, but ship them together so the measurement is of
the real configuration).

1. New env `TRACE_COMMONS_PERPLEXITY_DRIVER_DRAIN_INTERVAL_SECONDS`, default
   = interval (off), clamp `[1, interval]`.
2. In `spawn_perplexity_score_driver_task`'s loop: choose the next sleep from
   the tick result — full batch enumerated AND no breaker trip → drain
   interval, else normal interval. (Tick must report items-enumerated;
   extend the summary.)
- Tests: sleep selection per (enumerated, breaker) combination via the
  summary; no timing flakiness — test the chooser fn, not the loop.

### Task 3 — tenant priority (approach D)

Independent of Tasks 1-2; can land in parallel after Task 0.

1. New env `TRACE_COMMONS_PERPLEXITY_DRIVER_PRIORITY_TENANTS`
   (comma-separated tenant ids, empty default = exact current SQL behavior).
2. Thread through config into `list_submissions_needing_gate_decision` as a
   `&[String]` parameter; `ORDER BY (s.tenant_id = ANY($5)) DESC,
   s.received_at ASC`. Empty slice keeps the plan identical.
- Files: ingest.rs (env + config), `db/mod.rs` trait signature,
  `db/postgres.rs` query. Trait-signature change touches any other impl/mock
  of `Database` — sweep with `cargo check`.
- Tests: pg-store test that a priority tenant's newer submission enumerates
  ahead of an older background one, and that FIFO holds within each class.
  Run locally (CI has no pg).

### Task 4 — docs and operator surface

1. `docs/operator/perplexity-scoring-driver.md`: new knobs; a "hackathon /
   burst mode" section giving a concrete recommended event profile
   (concurrency 4, batch 25, drain interval 2 s, priority = event tenant id)
   and the shared-vendor caveat (PII driver's 8-concurrent windows hit the
   same endpoint; do not max both).
2. `docs/operator/env-reference.md`: the three new rows with defaults and
   clamps.
3. `deploy/pilot-gcp/ingest.env.template`: new vars, commented out, with the
   event profile as the comment.
4. Note in the GPU/inference cost ledger runbook that scored-traces/hour is
   the spend proxy and drain mode raises it during backlogs only.

### Verification gate (before claiming green, per repo policy)

```
RUSTFLAGS='-D warnings' cargo check -p trace-commons-server --bins
RUSTFLAGS='-D warnings' cargo check -p trace-commons-server --bins --features near-ai-scorer
RUSTFLAGS='-D warnings' cargo test -p trace-commons-server --no-run
cargo clippy -p trace-commons-server --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
cargo fmt --all
```

`near-ai-scorer` is the pilot's build configuration and a CI job; a change
that only compiles under default features fails CI.

### Rollout

1. Land Task 0; deploy; capture one week (or at minimum one day) of
   `tick_duration_ms` / `backlog` / per-trace latency. This IS the baseline.
2. Land Tasks 1-3 with all defaults off/1 — deploy is a no-op behavior-wise.
   Verify the deployed binary by string marker (`--version` or a new env
   name in `strings`), not host git state — the host checkout is known-stale.
3. On the pilot, set concurrency 4 only. Watch one day: per-trace latency
   unchanged, throughput up, transient rate not exploding (a transient rate
   markedly above the ~12% baseline scaled by chunk count means the vendor is
   choking on the offered rate — halve N).
4. Add drain interval 2 s. Backlog gauge should now trend to zero after any
   burst.
5. Before the next event: set the event tenant in
   `PRIORITY_TENANTS`, raise batch to 25. After the event: unset priority.

### Rollback

Every step is env-only at runtime: unset the new vars and the driver is
byte-identical to today's (defaults are the current behavior; N=1 preserves
even the breaker semantics). Code rollback is a single revert of each task's
PR; no migrations, no schema, no data to unwind. Nothing in this plan touches
attempt rows retroactively, so no cleanup is ever required.

### How we know it worked

- Primary: p90 of `decided_at - received_at` for the event tenant during a
  synthetic final-hour burst (submit ~100 traces in 10 minutes against
  staging or the pilot off-hours) drops from the Task-0 baseline to under the
  event's attestation-wait window.
- Secondary: `backlog` gauge returns to zero within X minutes of the burst
  (X from the baseline arithmetic: 100 traces / (N / per_trace_latency)).
- Guardrails that must NOT move: `gate_scoring_exhausted` warnings per week
  (attempt budget integrity), `failed` (permanent) rate, and duplicate-pair
  pass-throughs (spot-check same-`canonical_summary_hash` decisions for
  double high-novelty passes).

### Open questions (flagged, not blocking)

- Whether embedder CPU or NEAR round trips dominate per-trace latency decides
  whether N=4 helps 4x or stalls on 2 vCPUs. Task 0 answers it; if CPU-bound,
  the next lever is the host size or offloading the embedder, not more N.
- The PII driver and this driver share the vendor. A shared cross-driver
  in-flight budget is the principled fix if both run hot simultaneously;
  out of scope here, worth a line in the roadmap's production-gap queue.
