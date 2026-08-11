# Community analytics: what a noise mechanism would have to be

**Date:** 2026-08-07
**Status:** Proposed — decision required before implementation
**Scope:** `COMMUNITY_APPROVED_NOISE_SEED_PREFIXES` and the community analytics
release path.

## Summary

The honest conclusion first, because it decides everything else: **at the
pilot's current corpus size, differential privacy with a meaningful epsilon
destroys these aggregates entirely.** Not degrades — destroys. The noise
needed for even a weak guarantee is several times larger than the values
being published.

So the real decision is not "which mechanism", it is **whether to publish
corpus analytics at all yet**, and if so, under what protection and with what
claim attached. Implementing a mechanism is the easy part and the wrong place
to start.

## What is released

`compute_corpus_analytics_summary` produces, over a rolling 7-day window:

| Field | Shape |
|---|---|
| `total_submissions` | one count |
| `total_accepted`, `total_rejected` | two counts |
| `accept_rate` | derived from the above |
| `novelty_histogram` | 11 bucket counts |
| `gate_outcomes` | 4 labelled counts |

`accept_rate` is post-processing of noised counts and costs no additional
budget (post-processing invariance). Everything else is a raw `COUNT(*)`.

## Sensitivity

**Event level** (add or remove one submission) is bounded. One submission
produces one gate-decision row, so it moves:

- `total_submissions` by 1
- exactly one of `total_accepted` / `total_rejected` by 1
- exactly one histogram bucket by 1
- exactly one gate outcome by 1

giving **L1 sensitivity Δ₁ = 4**.

**User level** (add or remove a contributor and everything they submitted) is
**unbounded**. The queries are uncapped `COUNT(*)` with no per-contributor
grouping, so one prolific contributor can be most of the corpus.

This distinction is the whole privacy claim. Event-level DP supports "you
cannot tell whether *this trace* is in the corpus". It does **not** support
"you cannot tell whether Alice contributed" — which is what a reader of
`/about/privacy` would assume. Getting user-level requires a per-contributor
contribution cap C, giving Δ₁ = 4C, and capping changes the published totals:
they stop being counts of what happened and become counts of what was
counted.

**Decision 1: event-level or user-level?** If user-level, **Decision 2: what
is C**, and is a total that under-reports prolific contributors acceptable?

## Mechanism

Assuming the above is settled, the mechanism itself is not controversial:

- **Discrete Laplace** (two-sided geometric) over integer counts, parameter
  `exp(-ε/Δ₁)`. Preferred over continuous Laplace plus rounding, which has
  known floating-point attacks.
- **Cryptographic randomness, freshly sampled per release.** Explicitly not a
  keyed hash of the data. The current `noisy_analytics_count` derives its
  noise from a hash whose input includes the true count, so each count maps
  to exactly one output — the output distributions for neighbouring datasets
  are disjoint point masses, which is the opposite of what DP requires. It
  cannot be repaired by recalibration and must not be promoted.
- Noise applied to each of the 18 counts, then `accept_rate` recomputed from
  the noised values, then negative results clamped to zero (clamping is
  post-processing and safe).

## Composition, and why the cadence is the problem

A record stays in the 7-day window for 7 days, so it is included in every
release made during that time. Composition is over a record's lifetime in the
window, not per release.

| Release cadence | Releases per record lifetime | ε per release for ε_total = 1 | Noise scale Δ₁/ε |
|---|---|---|---|
| 15 min (current worker) | 672 | 0.0015 | ~2,700 |
| Daily | 7 | 0.14 | ~28 |
| Weekly | 1 | 1.0 | 4 |

The published snapshot is already computed once and served until the next
recompute, so the *release* rate is the recompute cadence rather than the
query rate. That architecture is right and is what makes any of this
tractable. But it means the recompute interval **is** the privacy budget, and
the 15-minute cadence we just enabled is incompatible with a meaningful
epsilon. Analytics would need its own, much slower schedule than the roster.

## Utility, at real numbers

From the pilot corpus today:

```
submissions all-time:            352
submissions in the 7-day window:  13
gate decisions in the window:     13
```

Thirteen records spread across 11 histogram buckets and 4 gate outcomes.
Individual cells are between 0 and 13.

Against that, the noise scales above:

- weekly release, ε = 1: noise scale 4 — the totals (13 ± 4) are marginal, the
  per-cell values are pure noise
- daily release, ε = 1 over the window: scale 28 — nothing survives
- at the current 15-minute cadence: scale ~2,700 — absurd

Even the most generous configuration publishes a histogram that is entirely
noise. **There is no epsilon at this corpus size that yields both a defensible
guarantee and a meaningful chart.** That is a property of the data volume, not
of the mechanism, and no implementation choice fixes it.

## Options

**A. Keep analytics withheld until the corpus is large enough.** No code, no
claim, no risk. The gate already does this and the site already renders an
honest "not live yet". Revisit when in-window counts are in the thousands,
where a scale-4 noise term is a rounding error rather than the signal.
*Recommended for now.*

**B. Publish under suppression and cohort size only, and say so.** Drop the DP
claim rather than fake it. The protections would be: the min-cell floor now
applied to cell contents (#237), the tenant-cohort floor, and consent scoping.
Those are real and defensible — they are just not differential privacy, and
`/about/data-policy` would have to say precisely that. This needs the control
renamed: `community_noise_mechanism` should become an explicit publication
basis with values like `suppression_only` or a named approved mechanism, so
the code records which protection is actually in force rather than being
"satisfied" by something that is not a mechanism.

**C. Implement DP anyway**, with analytics on a weekly release schedule
separate from the roster's. Honest, expensive, and produces a chart that is
mostly noise until the corpus grows. Hard to justify today.

**D. Publish only what tolerates noise.** `accept_rate` as a coarse band, no
histogram, no per-outcome breakdown. Smallest useful surface, but it is a
product decision about whether that page is worth having at all.

## What the approval gate should check

Whatever is chosen, `community_noise_mechanism_approved` should verify more
than a string prefix. A snapshot's privacy block already carries
`epsilon_charged` and `sensitivity` fields that are currently always `None`.
An approved release should populate both, and the gate should refuse a
snapshot whose recorded epsilon exceeds the configured maximum — so the
published artifact carries its own accounting rather than relying on the
pipeline having done the right thing.

## Recommendation

Take **A** now and **B** when analytics are wanted live, with the control
renamed so the code cannot claim a guarantee it does not provide. Revisit **C**
only when in-window volume makes it meaningful.

The work that is worth doing today is none of the above: it is deciding
whether these particular aggregates are worth publishing, given that the
useful version of them is the un-noised version and the private version is
empty.
