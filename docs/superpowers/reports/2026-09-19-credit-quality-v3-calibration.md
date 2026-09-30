# Credit-quality calibration V3, for Qwen/Qwen3.8-27B

Date: 2026-09-19
Status: constants and schedule in code. **Not in effect on any deployment
until that build is deployed and `/v1/admin/score-credit-quality` is run.**

Companion to `2026-09-19-perplexity-floor-calibration-qwen3-8.md`, which
moved the gate's perplexity floor from 6.0 to 1.5 for the same reason.

## Why

`credit_quality` computes `q = f(perplexity) * g(novelty) * a(peak / whole)`
from constants compiled into the binary. `contributor_cap::increment_micros`
takes `q`, and the score attestation reports it. V1 and V2 were calibrated on
`Qwen3.6-27B-FP8`: perplexity floor 6.0 (the gate floor then), ceiling 38.5
(that model's p90). The pilot's scorer moved to `Qwen/Qwen3.8-27B` on
2026-09-09 05:49:19Z and the constants did not.

Mean `q` over scored rows, from `trace_gate_decisions`:

| era | perplexity | n | q median | q mean | q = 0 |
|---|---|---|---|---|---|
| 3.6 (07-06 to 09-02) | passed | 636 | 0.048 | 0.110 | 293 (46%) |
| 3.6 | failed | 132 | 0.075 | 0.071 | 15 |
| 3.8 (09-09 on) | passed | 123 | 0.000 | 0.055 | 69 (56%) |
| 3.8 | failed | 184 | 0.053 | 0.046 | 42 |

Under 3.8, mean credit quality is about half what passing traces earned
under 3.6.

The gate's pass/fail credit emitter (`attempt_emit_novelty_utility_credit`)
has one caller, the HTTP gate-worker handler. A deployment that scores
through the perplexity driver never reaches it, so on the pilot a failed
perplexity floor withheld nothing through that path. What moved was `q`.

## What the production distribution says

307 scored rows, `decided_at >= 2026-09-09 05:49:19Z`, `perplexity_micros > 0`,
read through the column-scoped `trace_gate_driver` role, aggregates only.

**Perplexity.** min 1.65, p50 5.14, p90 13.73, p95 23.99. By the design
spec's method the floor is the live gate floor (1.5) and the ceiling is the
production p90 (13.73).

**Anomaly ratio `r = peak chunk / whole trace`.** The spec says this term
"defaults to 1.0 and only bites on a suspiciously spiky profile", with soft
and hard thresholds of 3 and 10.

| era | n | r p25 | r p50 | r p75 | r p90 | r p95 | r p99 | penalised (r > 3) | zeroed (r >= 10) |
|---|---|---|---|---|---|---|---|---|---|
| 3.6 (08-17 to 09-02) | 396 | 8.16 | 21.41 | 91.31 | 249.28 | 449.03 | | 89.1% | 67.7% |
| 3.8 (09-09 on) | 307 | 3.51 | 7.11 | 15.85 | 39.54 | 70.73 | 310.4 | 79.2% | 36.2% |

Under 3.8, by chunks scored:

| chunks | n | r p50 | r p90 | zeroed |
|---|---|---|---|---|
| 1 | 19 | 1.00 | 1.00 | 0.0% |
| 2-8 | 26 | 3.85 | 13.36 | 19.2% |
| 9-15 | 24 | 7.80 | 41.06 | 37.5% |
| 16 (the cap) | 238 | 8.30 | 43.94 | 40.8% |

The ratio is exactly 1 for a single-chunk trace and grows with chunk count:
a long trace always has one chunk far more surprising than its
token-weighted mean. Thresholds of 3 and 10 therefore read an ordinary long
trace as an anomaly. They penalised most real traces under both models; this
is not a Qwen3.8 effect, though the model switch is what surfaced it.

**Novelty.** p50 0.165, p90 0.268, p95 0.292, p99 1.000. The spec pins the
credit-quality novelty floor to the gate's floor, 0.5, which sits above the
p95. Novelty pass rates have been 1 to 10% since the week of 2026-08-17, so
`g` is the 0.30 graded multiplier for nearly every trace. **V3 does not
change novelty**; see Open questions.

## V3

| constant | V2 | V3 | source |
|---|---|---|---|
| `ppl_floor_micros` | 6,000,000 | 1,500,000 | the live gate floor |
| `ppl_ceil_micros` | 38,500,000 | 13,730,000 | production p90 under 3.8 |
| `anomaly_soft_ratio_micros` | 3,000,000 | 40,000,000 | observed r p90 (39.54) |
| `anomaly_hard_ratio_micros` | 10,000,000 | 310,000,000 | observed r p99 (310.4) |
| `nov_floor_micros`, `nov_ceil_micros` | 0.5, 1.0 | unchanged | pinned to the gate |
| `ppl_floor_mult_micros`, `nov_floor_mult_micros` | 0.25, 0.30 | unchanged | policy, not a model property |

## Simulated effect

The function was re-implemented in SQL and run over the same 307 rows. Under
V2 it reproduces the stored production mean exactly (0.049 computed, 0.049
stored), which is what makes the V3 row believable.

| constants | zeroed | anomaly-penalised | q p10 | q p25 | q p50 | q p75 | q p90 | q mean | mean, 1 chunk | mean, capped |
|---|---|---|---|---|---|---|---|---|---|---|
| V2 | 36.2% | 79.2% | 0.000 | 0.000 | 0.036 | 0.075 | 0.094 | 0.049 | 0.163 | 0.035 |
| V3 | 1.6% | 9.8% | 0.127 | 0.156 | 0.203 | 0.245 | 0.290 | 0.208 | 0.270 | 0.199 |

For comparison, recalibrating perplexity alone (floor 1.5, ceiling 13.73,
anomaly thresholds left at 3 and 10) gives a mean of 0.092 and still zeroes
36.2%. The anomaly term is the larger lever.

## Scoping: a schedule, not a bumped constant

The batch pass recomputes every row. A single active constant set would
re-score Qwen3.6-era rows against the Qwen3.8 ceiling, where their
perplexities (median about 19) saturate, inflating old credit. So constants
are selected by when a decision was made:
`credit_quality::constants_at(decided_at)` returns V2 before
2026-09-09T05:49:19Z and V3 from then on, and both the inline score and the
batch pass use it. The batch pass is therefore idempotent across a model
switch, and needs no operator-supplied cutoff that could be mistyped. A
test scores two rows with identical signals on either side of the switch
and requires versions 2 and 3.

The switch instant is the pilot's. A second deployment that changed models
at a different time would need its own era; the schedule is the place for
it.

## Limits

- 307 traces over ten days from one pilot population; 78% at the 16-chunk
  cap.
- Raising the anomaly thresholds trades away some of the term's purpose. It
  exists to catch one inserted garbage chunk; at 40 and 310 it still zeroes
  a peak 400 times the trace, and no longer penalises ordinary long traces,
  but a padded chunk at 30 times the trace now passes unpenalised.
- p99 of 307 rows rests on about three observations.
- The simulation is arithmetic over stored signals. It says what `q` would
  be, not whether `q` is right; no labeled quality data exists for these
  traces.

## Open questions

- **Novelty.** The gate's 0.5 floor has refused about 98% of scored traces
  since mid-August, under both models. It bounds `g` at 0.30 for nearly
  everyone, which is a larger effect on `q` than anything perplexity does,
  and `novelty_passed` also decides what is inserted into the vector index.
  Whether that reflects agent traces genuinely sitting close together in
  embedding space, a change to the embedder or index around 2026-08-17, or a
  floor set for a different distribution, has not been established. It is a
  gate decision and needs its own investigation.
- **The anomaly ratio should be normalised by chunk count.** That is a
  change to the function. V3 only stops the constants from punishing length.
- **What the depressed `q` cost contributors** between 2026-09-09 and the
  day V3 takes effect: accrued cap units per contributor, and whether any
  score attestation was issued to a collector with those values. Not
  established; the narrow role cannot read the ledger.
