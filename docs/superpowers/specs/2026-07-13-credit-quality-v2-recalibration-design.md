# Credit-quality V2 recalibration — design

## Goal

Fix the degeneracy in the shadow credit-quality score `q ∈ [0,1]` (sub-project
#1, PR #168). On the 349-decision pilot backfill, 347/349 scored `q = 0`. Replace
the hard-zeroing floors on the perplexity and novelty terms with a graded
(affine) floor so substantive-but-unremarkable work earns a nonzero quality
signal, while genuine high-quality work still saturates to 1.0. Shadow-only —
nothing settles, pays, or gates on `q`.

## Background: why it degenerated, and why softening is now safe

`q = f · g · a` is a pure product where each term hard-zeroes below its floor, so
any single failing signal collapses `q`. On the homogeneous single-contributor
pilot corpus: novelty `g` fails its floor for ~87% of traces (the dominant
killer), perplexity `f` fails for ~31%, anomaly withholds ~8%. Multiplied, ~99%
zero out.

When `q` was first designed, its novelty term was load-bearing for
anti-duplication: a low-novelty trace *had* to be punished because nothing else
caught resubmission / paraphrase farms. **That is no longer true.** Cross-trace
dedup (sub-project #2, merged as PR #169) now carries that load orthogonally via
`dup_pen = 1 / dedup_cluster_size`. A farm that resubmits N rewordings collapses
on `dup_pen`, independent of `q`. So `q`'s novelty term can be relaxed from a
necessary multiplier to a graded quality signal without reopening the farming
hole. This is the economic justification for V2.

The anti-fabrication posture remains economics-first: `dup_pen` (dedup), the
anomaly fraud gate, and the deferred per-contributor caps + delayed settlement do
the anti-gaming work; `q` grades quality on a continuum.

## Approach: affine graded floor

Each saturating term is wrapped with a per-term floor multiplier:

```
x' = floor_mult + (1 - floor_mult) · saturating_term(value, floor, ceil)
```

`saturating_term` is unchanged (0 below floor, concave rise, 1 at/above ceil).
The wrapper maps its `[0,1]` output onto `[floor_mult, 1]`:

- A trace that entirely fails a signal gets `floor_mult` for that term, not 0.
- A trace at/above the ceiling still gets 1.0.
- Monotonicity and concavity are preserved (affine transform of a monotone
  concave map).

**Both `f` (perplexity) and `g` (novelty) get a graded floor.** The anomaly term
`a` is unchanged — it stays a hard fraud gate that can zero `q` at the hard
spikiness ratio.

## Constants and versioning

Add two per-term floor-mult fields to `CreditQualityConstants`:
`ppl_floor_mult_micros`, `nov_floor_mult_micros`.

**V1 sets both to 0.** With `floor_mult = 0` the affine formula is byte-identical
to today's hard-zero (`x' = 0 + 1·sat = sat`), so every existing V1 test passes
unchanged and V1's already-stored scores stay reproducible.

`CREDIT_QUALITY_CONSTANTS_V2` (`version: 2`) keeps V1's floors, ceilings, and
anomaly thresholds and adds starting floor-mults, to be tuned on the backfill:

- `nov_floor_mult_micros: 300_000` (0.30) — softened most; dedup carries the
  duplication defense.
- `ppl_floor_mult_micros: 250_000` (0.25) — effort still matters, but
  trivial-but-real work isn't zeroed.

Worst-case non-anomalous `q` floor = 0.25 · 0.30 = 0.075; genuine high-quality
still → 1.0.

A single active alias `CREDIT_QUALITY_ACTIVE = CREDIT_QUALITY_CONSTANTS_V2` is
introduced; both production call sites (inline gate-time score, batch admin
re-score route) and the two active-decision tests reference it, so a future
recalibration touches one line.

## Persistence

No migration. The V39 columns (`credit_quality_micros`,
`credit_quality_anomaly_ratio_micros`, `credit_quality_calibration_version`)
already exist. Only the stamped `version` value changes from 1 to 2 (data, not
schema). The batch route `POST /v1/admin/score-credit-quality` re-scores and
re-stamps existing decisions to version 2 on demand.

## Testing

- **V1 invariance:** all existing V1 tests pass unchanged (affine with
  `floor_mult=0` reproduces the hard-zero exactly). This is the regression guard.
- **V2 floor lifts the zero:** a trace below both floors (and non-anomalous)
  scores ≈ 0.075, not 0.
- **Shape preserved under V2:** monotonic non-decreasing in perplexity and
  novelty, concave (diminishing returns), bounded `[0, 1e6]`.
- **Anomaly still hard:** at/above the hard ratio, `q = 0` and `anomaly_withheld`
  set, even with softened floors.
- **Anti-gaming sanity:** `genuine_beats_every_gamed_variant` still holds under
  V2 floor-mults (genuine mid-high beats the rare-token pump, distinctive-token
  shim, and peak parasite). This is a sanity check, not the primary defense —
  `dup_pen` + anomaly + caps are.
- **Wiring:** the two active-decision integration tests assert the stored
  `version` equals the active constant's version (2) and the stored `q` matches
  `credit_quality(..., CREDIT_QUALITY_ACTIVE)`.

## Rollout

Land the formula + constants + tests (no migration). Deploy to the pilot. Run the
batch re-score over the 349 now-27B-consistent decisions, inspect the new `q`
distribution (confirm non-degeneracy and a sane spread), and tune the two
floor-mults if the spread is too flat or too generous before any future
settlement reads `q`. No payout, no gating.

## Non-goals

- No change to `f`/`g` floors/ceilings, the anomaly thresholds, perplexity,
  novelty, tail-fraction, dedup, gate status, or credit.
- No new migration or column.
- No settlement, payout, or gating on `q`.
- No change to the anomaly fraud gate's hard-zero behavior.
