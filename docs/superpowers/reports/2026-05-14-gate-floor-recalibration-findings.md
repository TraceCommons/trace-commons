# Gate Floor Recalibration Findings (A2.5)

Date: 2026-05-14
Owner: Trace Commons / Datasets lane
Companion to:

- `2026-05-13-model-bakeoff-result.{json,md}` (A2.3c, complete)
- `2026-05-13-model-bakeoff-result-notes.md` (A2.3c interpretive notes)
- `2026-05-14-gate-floor-recalibration-design.md` (A2.5 spec; this report
  is the empirical basis)

Driver runs:

- A2.3c — 4-candidate bake-off against the boilerplate-shaped duplicate
  slice (license boilerplate, FAQ-prefix, stock prose). Complete.
- A2.4 — same 4 candidates against a Wikipedia-introductions duplicate
  slice. Complete (2026-05-14, 4 of 4 candidates). A2.4 was
  constructed specifically to test the hypothesis that A2.3c's
  headline finding was a corpus artifact.

## TL;DR

Across every candidate and every duplicate-slice variant we measured,
the perplexity-based novelty AUC is **well below 0.5**. The metric
isn't noisy or weakly-discriminating — it's *inverted*. Modern
instruct-aligned and base LLMs find OASST2-style reasoning *less*
surprising than well-trodden duplicate content. Setting a positive
perplexity floor would systematically *reject* contributor-grade
reasoning and *accept* duplicates.

Pilot launch ships with:

- `TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS=0` (disabled)
- `TRACE_COMMONS_GATE_TAIL_FRACTION_FLOOR_MICROS=0` at launch;
  calibrate post-first-1000-pilot-traces against the tail-fraction
  column only
- `TRACE_COMMONS_GATE_NOVELTY_FLOOR_MICROS=500000` (unchanged; the
  embedder + vector-index path is unaffected by this finding)

## A2.3c — Boilerplate-corpus run (complete)

Duplicate slice: license boilerplate, FAQ prefixes, and stock prose.
Novel slice: OASST2 reasoning. Paraphrase slice: Qwen3-4B-Base
back-translation.

| Candidate              | AUC    | Paraphrase delta | Tail-fraction range | Throughput tps |
|------------------------|--------|------------------|---------------------|----------------|
| Llama-3.1-8B-Instruct  | 0.1197 | 0.1249           | 0.1273              | 288.5          |
| Qwen3-8B-Base          | 0.2350 | 0.1416           | 0.0099              | 245.5          |
| Qwen 3.6 27B Dense     | 0.2759 | 0.1398           | 0.0125              | 118.2          |
| Gemma 4 31B Base       | 0.0545 | 0.1734           | 0.2019              | 200.4          |

AUC range: 0.054 to 0.276. None of the four candidates exceeds 0.5.
The base/instruct/dense/large axis doesn't change the sign of the
result — every model under measurement finds duplicate boilerplate
*more* surprising than novel reasoning.

## A2.4 — Wikipedia-corpus run (complete)

Same four candidates, same paraphrase pipeline. Duplicate slice
replaced with Wikipedia article introductions on the hypothesis that
"boilerplate is too far from the model's training distribution; using
something the model has seen will reverse the AUC."

Source: `docs/superpowers/reports/2026-05-14-model-bakeoff-result-a24.json`.

| Candidate              | AUC    | Paraphrase delta | Tail-fraction range | Throughput tps |
|------------------------|--------|------------------|---------------------|----------------|
| Llama-3.1-8B-Instruct  | 0.2400 | 0.1249           | 0.0212              | 316.2          |
| Qwen3-8B-Base          | 0.2065 | 0.1416           | 0.0251              | 270.7          |
| Qwen 3.6 27B Dense     | 0.2641 | 0.1398           | 0.0223              | 126.1          |
| Gemma 4 31B Base       | 0.1849 | 0.1734           | 0.0328              | 213.9          |

Gemma 4 31B Base improved from 0.0545 (A2.3c) to 0.1849 (A2.4) — a
+0.130 jump, the largest absolute swing in the dataset besides
Llama-Instruct's +0.120. Wikipedia helps the dense/large base model
substantially even though it slightly hurt the smaller Qwen3-8B-Base.
The corpus change moves every candidate but does not move any of
them across 0.5.

Headline-winner note: A2.4's `pick_winner` selected
`llama-3.1-8b-instruct` (AUC 0.2400, in-throughput-budget) over
A2.3c's Qwen3-8B-Base. Qwen 3.6 27B Dense had the highest A2.4 AUC
(0.2641) but failed the throughput floor (126 tps vs ~158 cutoff).
**Neither winner pick is load-bearing for the gate-floor decision**
— per A2.5, the perplexity floor ships at 0 either way.

## Side-by-side delta (the most important data view)

| Candidate              | A2.3c AUC | A2.4 AUC | Δ        |
|------------------------|-----------|----------|----------|
| Llama-3.1-8B-Instruct  | 0.1197    | 0.2400   | +0.120   |
| Qwen3-8B-Base          | 0.2350    | 0.2065   | −0.029   |
| Qwen 3.6 27B Dense     | 0.2759    | 0.2641   | −0.012   |
| Gemma 4 31B Base       | 0.0545    | 0.1849   | +0.130   |

## Interpretation

Four interpretive points, in order of how load-bearing they are for
the pilot-launch decision:

1. **Every measured AUC is below 0.5.** The metric isn't weakly
   discriminating — it's *inverted*. Models find OASST2 reasoning
   *less* surprising than the duplicate slice. A perplexity floor in
   the natural direction (`>= floor`) would reject novel reasoning
   and accept duplicates. This is the load-bearing fact for A2.5: the
   gate as designed in A2 does not measure what it was meant to
   measure.

2. **The corpus change (boilerplate → Wikipedia) helped Llama-Instruct
   decisively (+0.12) and Gemma 4 31B substantially (+0.13) but
   slightly hurt the smaller Qwen base models (−0.029 to −0.012).**
   Hypothesis: smaller Qwen base models were already less RLHF-
   distorted, so the boilerplate corpus wasn't penalizing them as
   much; swapping to Wikipedia (which they're trained on heavily)
   makes the duplicate slice even *less* surprising for them →
   reduces the gap. Llama-Instruct and Gemma 4 31B, by contrast, both
   gain from the corpus swap — Llama because its RLHF-shaped
   confidence aligns with Wikipedia prose, Gemma 4 because it's
   trained on Wikipedia at extreme density. None of this changes the
   sign — every number is still below 0.5 — but it does say the
   corpus-design intuition that worked for A2's pseudocode
   (boilerplate as "obviously duplicate") doesn't survive contact
   with the models we can actually run.

3. **Gemma 4 31B's tail_fraction_range is the strongest single signal
   in the dataset** (0.2019 in A2.3c, dropping to 0.0328 in A2.4 — the
   tail signal is corpus-dependent too, not invariant). The *tail* of
   the perplexity distribution does separate slices even when the
   aggregate does not. This is what justifies leaving the
   tail-fraction floor in the codebase (at 0 for pilot launch) rather
   than removing it; the floor needs pilot-distribution calibration
   before it can usefully discriminate, but the residual signal is
   real.

4. **The pattern is consistent across model families and corpus
   designs.** This isn't a single-candidate quirk that another model
   choice would fix. It's a property of modern instruct-aligned and
   base LLMs vs the OASST2-reasoning shape we used for the novel
   slice. Picking Gemma 4 over Qwen3-8B-Base, or running A2.5 against
   a fifth candidate, does not change the recommendation. (We keep
   Qwen3-8B-Base by license + VRAM cost; see A2.3c notes.)

## Pilot-launch floor recommendations

| Floor                                 | Pilot value          | Why                                                                                                                     |
|---------------------------------------|----------------------|-------------------------------------------------------------------------------------------------------------------------|
| `PERPLEXITY_FLOOR_MICROS`             | **0** (disabled)     | All measured AUCs < 0.5 across both corpora and four candidates. Any positive floor rejects in the wrong direction.     |
| `TAIL_FRACTION_FLOOR_MICROS`          | **0** at launch      | Tail-fraction range showed real signal on Gemma 4 31B (0.20) but is uncalibrated against the pilot distribution. Calibrate post-first-1000-pilot-traces using only the tail-fraction column from `trace-commons-gate-calibrate` output. |
| `NOVELTY_FLOOR_MICROS`                | **500000** (cosine 0.5; unchanged) | Embedder + vector-index path is similarity-based and is unaffected by the perplexity finding. Active primary gate at launch. |

The deployment runbook's "at least one of the three floors must be
positive" invariant is satisfied by the novelty floor alone.

## Why this isn't easy to fix in the current shape

Three approaches were considered and rejected for pilot launch:

- **Invert the perplexity gate to `<= floor`** (treat low-perplexity
  traces as suspect). Plausible — boilerplate-like content would be
  filtered. But this is a *different* gate (anti-boilerplate, not
  novelty) and conflating the two would mislead operators about what
  the floor means. Parked for Phase A.5 if it remains interesting
  after pilot data lands.
- **Re-run the bake-off against a different corpus.** A2.4 already
  did this. The signal direction didn't change; only the magnitudes
  shifted. A third corpus design is unlikely to find the missing
  factor-of-2.5 in AUC.
- **Pick a different candidate model.** Wouldn't change the sign.
  All four candidates were below 0.5 on A2.3c; three of four are
  below 0.5 on A2.4. The Gemma 4 31B A2.4 result is outstanding, but
  given its A2.3c AUC of 0.054 the prior on "Gemma 4 fixes this" is
  weak.

## Future work (Phase A.5)

Three candidate metrics for the perplexity-replacement gate, all
deferred until the pilot produces ~1000 labeled novel/duplicate
traces from the real contributor distribution:

- **Contrastive perplexity.** Delta in logprobs between two model
  checkpoints (one well-trained, one less so). The *difference*
  between models may be more novelty-indicative than either
  absolute. Adds runtime cost (two models loaded) but no schema
  changes.
- **Per-token rarity.** Explicitly gather the lowest-N logprobs
  across the trace; if any genuinely surprising tokens exist, treat
  as novel. This is a tighter version of tail-fraction and might
  collapse into it once tail-fraction is calibrated against pilot
  data.
- **Learned discriminator.** Small classifier trained on labeled
  novel/duplicate exemplars from the pilot. Requires labeled data we
  don't have yet; the first ~1000 pilot traces are the prerequisite.

Spec link: `2026-05-14-gate-floor-recalibration-design.md` §3.

## What this report does not claim

- That the embedder + vector-index path works as designed. A2.3c and
  A2.4 only measured the perplexity-side floors. The novelty-floor
  path is unmeasured at pilot launch and should be validated against
  the first real traces.
- That Gemma 4 31B's A2.4 result will or will not match its A2.3c
  result. Report will be appended when A2.4 completes; the
  recommendation does not depend on that data point.
- That the gate is unfixable. Phase A.5 candidates exist; this
  report is the basis for parking that work until pilot data
  unblocks it.
