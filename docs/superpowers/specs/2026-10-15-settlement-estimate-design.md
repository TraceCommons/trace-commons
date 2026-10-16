# Settlement estimate: what the traces on this machine are worth

Status: draft, owner decisions recorded 2026-10-15.

## Goal

Tell a contributor, before anything is sent, roughly how much NEAR AI
inference the traces on their machine would settle for. Show it in the
menu bar and in notifications. Refresh it when significant new traces
appear, and when the pricing behind it changes.

This builds on the local credit estimate (nudge value addendum, 4.6), which
estimates per trace from content-free `lef1` features looked up in a
published table. That table's bands are in displayed credit points
(`10 * q`). They cannot be summed, and nothing draws them under the one-tier
built-in table. This design adds a second table, priced in settlement
units, and a machine-wide total built from it.

## Owner decisions (2026-10-15)

| # | Decision |
|---|---|
| S1 | The figure is shown in inference dollars. |
| S2 | The credit-to-dollar rate is set by the operator in the installed table (`usd_micros_per_credit`). With no rate, nothing is drawn. The repository carries only synthetic test values. |
| S3 | The total covers every trace found on the machine, in declared and undeclared folders alike. The one exception: a folder whose project resolves to Never (`ProjectMode::Ignore`, including the global Never override) is not read at all. Any other folder is fair game. |
| S4 | S3 reverses the 2026-08-19 fail-closed-roots rule **for local estimation only**. Queueing, preview and upload stay fail-closed on undeclared roots, exactly as today. |
| S5 | The total is a sum of per-tier expected awards, with traces withheld at zero counted as zero. Its band is the calibrated ratio of actual to predicted batch totals. It is not a sum of per-trace bands. |
| S6 | Notify when the lower end has risen by at least 25% and at least $1 since the last notification, or when a pricing change moves the total by at least 25% either way. Otherwise the menu bar updates silently. |
| S7 | Menu bar: the re-engagement icon state, plus one line in the menu. No figure in the bar itself. |
| S8 | Per-trace estimates on queue entries move to dollars under the same table. The points table is no longer drawn. |
| S9 | Positive consent is required before any trace data goes to the server or to a TEE (the privacy filter, the scoring model, the witness). Reading a folder to estimate it is not consent, and an estimate never moves a trace closer to sending: no auto-queueing, no auto-preview, no pre-redaction call. Only the contributor's explicit action in an existing consent flow sends anything. |

## Non-goals

- Computing the Score policy on the device. Perplexity and novelty do not
  exist there, and E14 still holds. The device looks up a fitted table and
  nothing more.
- Any server-side estimate of a trace. Nothing about a trace leaves before
  consent.
- Windows, Linux GTK and Tauri surfaces. Those shells are frozen until
  macOS ships. The daemon and core copy are shared, so they can adopt it
  later without design work.

## 1. The settlement table

### Wire shape

The table is served at `GET /v1/credit-estimate/settlement-table`. It is
unauthenticated and outside tenant context, like `/v1/credit-estimate/table`
and `/v1/source`. The v1 path and its parser are unchanged. A client that
does not know the new path never asks for it.

```json
{
  "schema_version": 2,
  "features_version": "lef1",
  "version": "s3",
  "pricing_version": "p2",
  "instrument_id": "trace_credit",
  "atomic_units_per_credit": 1000000,
  "usd_micros_per_credit": 120000,
  "published_at": "2026-10-20T00:00:00Z",
  "bytes_per_token": 4,
  "chunk_target_tokens": 2048,
  "chunk_cap": 16,
  "weights": [],
  "cut_offs": [0.4],
  "tiers": [
    {"mean_atomic_units": 150000, "withheld_share_micros": 300000},
    {"mean_atomic_units": 600000, "withheld_share_micros": 100000}
  ],
  "aggregate_band": {"low_ratio_micros": 600000, "high_ratio_micros": 1500000},
  "fit_rows": 412
}
```

The values above are synthetic.

- **Tiers.** `weights` and `cut_offs` mean exactly what they mean in v1,
  so the same `lef1` scoring assigns a trace to a tier.
- **`mean_atomic_units`.** The mean award per trace in the tier, counting a
  trace withheld at zero as 0.
- **`withheld_share_micros`.** Explanatory only. The mean already includes
  it.
- **`aggregate_band`.** Scales a summed mean into the displayed band (§3).
- **`pricing_version`.** An opaque label the operator sets for the Score
  policy configuration the table was fitted against. Changing it is how a
  pricing update reaches clients.
- **No server-side meaning.** The server attaches none of these figures to
  any account. They are aggregates over many decisions across tenants.

### Refusal rules (client and server parser alike)

The table is refused when any of these holds:

- `schema_version` is not 2, or `features_version` is not `lef1`;
- there are fewer than 2 tiers, or more than `ESTIMATE_MAX_TIERS`;
- a tier mean is not positive, or not finite;
- the cut-offs are not strictly increasing;
- `aggregate_band` is out of order, or has `low_ratio_micros == 0`;
- `instrument_id` is not `trace_credit`;
- `atomic_units_per_credit == 0`;
- the table is larger than `ESTIMATE_TABLE_MAX_BYTES`.

`usd_micros_per_credit` is optional. A table without it parses, but draws
nothing (S2). A refused fetch falls back to *no table*, never to a stale
fetched one. There is no built-in settlement table. A table older than
`ESTIMATE_TABLE_MAX_AGE` (`published_at`) draws nothing.

### Serving

The operator installs the file and points a new env var at it:
`TRACE_COMMONS_CREDIT_SETTLEMENT_TABLE_PATH`. It is validated at boot with
the client's parser, and an invalid file fails boot with
`credit_settlement_table_invalid`. When the var is unset, the route returns
404, and clients treat a 404 as "no table": nothing is drawn.

## 2. Fitting the table

`POST /v1/admin/credit-estimate-eval` gains `target`, which is `display`
(the default, today's behaviour) or `settlement`. Under `settlement`, each
row's label is the run's settled `trace_credit` award:

| Run state | Label |
|---|---|
| A committed Score outcome with a `pipeline_run_settlements` row for `trace_credit` | `atomic_units` |
| A committed Score outcome with no such row (withheld at zero) | 0 |
| Held for review, or no committed Score outcome | Excluded |

Rows otherwise follow today's eval rules:

- features are derived inside the gate service, so the handler holds no
  plaintext;
- rows are label-only and shuffled;
- they carry a per-run tenant tag and no submission id;
- the run is bounded by `CREDIT_ESTIMATE_EVAL_MAX_LIMIT`;
- it writes nothing.

With `fit=true`, `fit_settlement_table` does the following:

1. Split by decision time, as E9 does: fit on the earlier part, evaluate on
   the later part.
2. Choose cut-offs as `fit_estimate_table` does, then set each tier's
   `mean_atomic_units` and `withheld_share_micros` from the fit split.
3. On the evaluation split, draw `B = 200` random batches of
   `n = min(20, rows)` traces. For each batch, record
   `actual_total / predicted_total`, where predicted is the sum of tier
   means. `aggregate_band` is the p10 and p90 of those ratios. Batches with
   `predicted_total == 0` cannot occur, because the means are positive.
   The batch draw is seeded from the request, so a run is reproducible.
4. Return the metrics and a candidate table with no `usd_micros_per_credit`
   and no `pricing_version`. The operator adds both.

The report also returns the evaluation split's
`sum(actual) / sum(predicted)` over all its rows, as a bias check.

## 3. The machine-wide total

### Per trace

- `tier = assign(features, table)`.
- `expected_i = tier.mean_atomic_units`.
- When a trace has no features (unreadable, or empty content), it is
  *unknown*. It is counted separately and contributes nothing.

### Total

- `sum = Σ expected_i` over known traces.
- The low end is `sum * low_ratio`.
- The high end is `sum * high_ratio`.
- Each end is converted to dollars with
  `usd = atomic_units / atomic_units_per_credit * usd_micros_per_credit / 1e6`.

Display rounding:

| Ends | Rounding |
|---|---|
| Both ends under $10 | Down/up to $0.50 steps |
| Otherwise | To whole dollars |

The low end is rounded down and the high end up. When the rounded low end
is $0, the line reads "up to $Y", never "$0 to $Y". When no trace is known,
or no table is drawable, nothing is shown at all: not $0, not a
placeholder.

A per-trace entry estimate (S8) is
`expected_i * [low_ratio, high_ratio]`, drawn with the same rounding.
Applying a ratio band built for batches to a single trace is labelled an
estimate everywhere, and it understates a single trace's spread. The
explainer says so.

### What counts

| Trace | Counts |
|---|---|
| In a folder whose project resolves to `ProjectMode::Ignore` | No. Not read. The check runs before any file open. |
| Uploaded (history cache has a submission), kept, or withdrawn | No |
| Queued, waiting, watch-only, or in a declared or undeclared folder | Yes |
| Still being written (the watcher's settle rule) | Not yet |

Undeclared folders are enumerated through each `TraceSource::discover()`,
as `discover_across_sources` does. A source that fails is skipped and named
by a label, as the watcher skips it.

### Cost and caching

- Features use the existing bounded pass:
  `ESTIMATE_SAMPLE_BYTES` per trace, whatever its size.
- They are cached in the contributor store, keyed by
  `(source, path hash, size, mtime)`. A trace is re-read only when one of
  those changes. The cache holds features and the key only, never content
  and never a raw path.
- A full scan runs on a background worker at low priority and yields
  between traces.

### When to re-estimate

| Trigger | Action |
|---|---|
| The watcher reports new or changed sessions | Re-estimate after a 10-minute debounce, at most hourly |
| Daily scan of undeclared folders | Rediscover and re-estimate |
| A fetched settlement table whose `version` or `pricing_version` differs from the one in force | Re-estimate immediately |
| A project's mode changes to or from Never | Re-estimate (drop or add that folder) |
| The estimate switch is turned on | Full scan |

The settlement table is fetched on the existing `EstimateTableSchedule`:
24 h, with jitter, and no machine-dependent inputs.

### Off switch

Settings, under the suggestions switch: "Estimate what traces on this Mac
are worth". It is on by default. Off stops the scan, deletes the feature
cache, and hides every surface.

## 4. Surfaces (macOS)

### IPC

`status` gains `settlement_estimate`, present only when it is drawable:

```json
{"low_usd_micros": 4000000, "high_usd_micros": 9000000,
 "known": 212, "unknown": 7, "table_version": "s3",
 "pricing_version": "p2", "estimated_at": "..."}
```

A new event, `EVENT_SETTLEMENT_ESTIMATE_CHANGED`, fires when the rounded
display changes.

### Menu bar

- **Icon state.** The existing re-engagement icon state lights up when the
  estimate is drawable and its low end is at least $1, and it is not muted.
  Every existing gate applies: paused, consent hold, signed out, unhealthy,
  and the suggestions switch.
- **Menu line.** The menu bar panel shows one line, from
  `MenuBarGlassPanel`'s existing glass components:
  `ESTIMATE_TOTAL_LINE` ("About {low} to {high} of inference in traces on
  this Mac"), with "Estimate" and a Review traces action that opens the
  Traces view.

### Notification

A new attention kind, `SettlementEstimate`, posts standalone through
`attention::arbitrate`. It inherits quiet hours, the daily, weekly and
per-kind caps, the minimum gap, mute, and the gating above.

It is a candidate when either of these holds:

- `low >= 1.25 * last_notified_low` and `low - last_notified_low >= $1`;
- or the table's `pricing_version` changed and
  `|mid - last_notified_mid| >= 0.25 * last_notified_mid`.

It is never a candidate for its first estimate while the user is inside
first run. "Not now" only dismisses the notification.
`last_notified_low` and `last_notified_mid` are kept in the attention log.

### Copy

All copy lives in core (`nudge_copy.rs`) and is mirrored through the C ABI,
marked DRAFT, NEEDS APPROVAL. The strings are the menu line, the
notification title and body, the per-entry band, the explainer, and the
Settings switch with its footnote. Every string carrying a figure says
"estimate" or "about". The explainer says:

- the figure is made on this Mac from the traces' size and makeup;
- nothing was sent;
- the rate comes from Trace Commons' published pricing;
- what a trace actually settles for is decided when it is scored.

The UI goes to design review after it is built.

## 5. Privacy and logging

- Nothing about a trace leaves the device, neither to the Trace Commons
  server nor to any TEE-hosted service, without positive consent (S9). The
  only network request this feature adds is the table fetch, which carries
  no credential and no trace data, and whose schedule has no machine input.
  The Review traces action opens the existing consent flow. It sends
  nothing by itself.
- The feature cache and logs are label-only and counts-only: no paths, no
  project names, no per-trace figures. The cache keys on a path hash.
- **Never is absolute.** An Ignore folder is skipped before any file is
  opened. A test proves no open call reaches it.
- S4 is scoped in code. The estimation scanner is a separate entry point
  that cannot enqueue, preview or upload. The queue and the watcher keep
  their declared-roots check.

## 6. Rollout

1. Server: the v2 parser in the protocol crate, the route, the env var,
   and `target=settlement` on the eval route.
2. Client: the scanner, the cache, the aggregate, IPC, the macOS
   surfaces, and copy.
3. Deploy the server. Run the eval with `fit=true` read-only on the pilot,
   and bring the candidate table to the owner. The owner sets
   `usd_micros_per_credit` and `pricing_version`, then the table is
   installed. **Ask before touching the pilot.**
4. Release the client. Until step 3 is done, clients draw nothing.

## 7. Testing

- **Protocol:**
  - each refusal rule;
  - parses without a rate but is not drawable;
  - stale `published_at` is not drawable;
  - tier assignment matches v1 for the same weights and cut-offs.
- **Aggregate:** a hand-computed fixture for the sum, the band, the dollar
  conversion, rounding at the $10 boundary, "up to" at a $0 low end, and
  unknown traces excluded.
- **Fit:**
  - synthetic runs with settled, withheld and held outcomes: held ones are
    excluded and withheld ones count 0;
  - the ratio band is reproducible from a seed;
  - the bias check is reported.
- **Daemon:**
  - the Ignore folder is never opened (a source double that fails the
    test on open);
  - uploaded, kept and withdrawn traces are excluded;
  - the cache hits on an unchanged key and misses on an mtime change;
  - the debounce;
  - a version or pricing change triggers a re-estimate;
  - the off switch deletes the cache;
  - the estimation scanner cannot enqueue;
  - a full scan and estimate makes no network request other than the
    table fetch (a transport double that fails the test on any other
    request, covering the ingest, the privacy filter and inference
    endpoints).
- **Attention:** the 25% / $1 rule at its edges, a pricing-change move in
  both directions, inheritance of each existing gate, and "Not now" only
  dismisses.
- **macOS:**
  - the menu line renders from a status fixture;
  - absent when not drawable;
  - the icon state follows the gating.
- **Copy:** the width and "estimate" word checks in `nudge_copy` cover the
  new strings.
