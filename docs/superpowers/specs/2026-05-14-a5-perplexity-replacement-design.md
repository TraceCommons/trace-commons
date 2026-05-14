# Phase A.5 Perplexity Replacement — Design (Three Candidate Approaches)

Date: 2026-05-14
Status: **DRAFT — pending A2.6 outcome confirmation.**
Owner: Trace Commons / Datasets lane
Predecessors:
- `2026-05-14-gate-floor-recalibration-design.md` (A2.5 — the finding that put Phase A.5 on the roadmap)
- `2026-05-14-agent-traces-bakeoff-design.md` (A2.6 — the bake-off whose outcome triggers this spec)
- `2026-05-14-gate-floor-recalibration-findings.md` (A2.5 findings report)
- `2026-05-12-perplexity-scorer-design.md` (A2 — original perplexity scorer)

## Activation condition

> **TODO: activate only if A2.6 keeps all candidate AUCs below 0.5.**
>
> A2.6 is in flight on a Lambda H100 at spec time. This spec is pre-drafted
> so we can file it within minutes of the bake-off completing if (and only
> if) A2.6 confirms A2.5's pessimism with a second corpus shape. If A2.6
> reports AUC > 0.5 for any candidate, file the companion A2.7 spec
> (`2026-05-14-a27-perplexity-floor-update-design.md`) instead and leave
> this one as `DRAFT — superseded`.
>
> Until A2.6 results are recorded in `docs/superpowers/reports/`, this spec
> is not operative. Do not implement against it; do not link from the
> roadmap as active.

## Motivation

A2.5 documented that the perplexity gate's "novelty = surprising tokens"
framing is inverted on every modern aligned LLM candidate against an OASST2-
novel-slice corpus. A2.6 tested whether that inversion was a corpus-design
artifact by swapping the novel slice to OSS security-audit agent-traces
(`jedisct1/agent-traces-swival`). **Assuming A2.6 also reports AUC < 0.5
across all candidates**, the inversion is corpus-shape-robust: it is a
property of the metric, not of any single dataset choice.

That conclusion reinforces A2.5's "perplexity disabled at launch"
recommendation and elevates Phase A.5 (perplexity-replacement metric design)
from "parked" to "active design work." This spec lays out three candidate
replacement approaches, evaluates them on cost / risk / earliest-deliverable
axes, and recommends a sequencing.

## Goal

1. Identify a small set of credible replacement metrics for the perplexity
   floor — each one capable of discriminating novel reasoning from common
   content where absolute perplexity has failed.
2. Compare them on implementation cost, risk profile, and time-to-first-
   experiment.
3. Recommend a sequencing so the cheapest / lowest-risk experiment runs
   first and informs the others.
4. Defer the actual implementation to a downstream A.5a / A.5b / A.5c spec
   per chosen approach. This spec is the design-space sketch, not the
   build instructions.

## Non-goals

- **No code in this spec.** Each approach gets enough technical detail to
  evaluate it; full implementation specs come later.
- **No commitment to ship all three.** The recommendation is to start with
  one and only escalate if it's insufficient.
- **No re-litigation of A2.5 or A2.6.** Their findings are inputs.
- **No new candidate-model bake-off in this spec.** The replacement-metric
  experiments may run against the existing A2.6 corpus; new corpus design
  is out of scope.

## Approach 1: Contrastive perplexity

### Idea

Score each input under *two* model checkpoints — one well-trained
(`Qwen3-8B-Base`, our current candidate winner from A2.3c by license
tiebreaker) and one less-trained (e.g., a stale `Qwen2-7B`, a stripped-down
random-init model, or an earlier-step checkpoint of the same architecture).
Report the *difference* in average logprob per token as the novelty score.

Score = `mean_logprob(input | well_trained) - mean_logprob(input | less_trained)`

Larger positive differences mean the well-trained model handles this input
much better than the less-trained one — i.e., training learned this content,
i.e., it is *in* training distribution, i.e., *not novel*. Smaller or
negative differences mean both models handle it equally well (or equally
poorly), suggesting the content is generic enough that training didn't
specifically help (novel/atypical), or rare enough that neither model has
strong priors on it.

### Why it might work

The absolute-perplexity finding from A2.3c/A2.4/A2.6 is that aligned LLMs
find chat-shaped and audit-shaped content unsurprising regardless of
semantic novelty. Both well-trained and less-trained models share this
property to some degree — but the well-trained model has *additionally*
absorbed specific datasets that the less-trained one hasn't. The difference
between the two isolates "what training added," which is closer to what
"in-distribution" actually means than absolute logprob.

For genuinely surprising content (a novel reasoning move, a domain-specific
technical detail the well-trained model didn't get extra training on), both
models are roughly equally bad. For duplicate content (Wikipedia intros,
license text, scraped boilerplate), the well-trained model is much better
because that content was in its training set specifically. The DIFFERENCE
captures "how much did training help on this input?" — which is the novelty
signal we wanted absolute perplexity to provide.

### Implementation cost

- Add a `TRACE_COMMONS_SCORER_CONTRASTIVE_MODEL_PATH` env var alongside the
  existing primary-model path.
- Load both models at scorer startup; keep both resident in GPU memory.
- Modify the score path to run two forward passes per scoring call and
  return the difference.
- Roughly doubles per-trace inference cost (VRAM and latency).

Estimated effort: ~3-5 days of Rust + 1 day of bake-off integration. The
mistralrs backend already supports the per-token logprob path A2.3 set up;
loading two models is mechanical.

### Risk

- **Picking the "less-trained" model is non-obvious.** Stale `Qwen2-7B` is
  the obvious candidate — same architecture, earlier training, available.
  A stripped-down random-init has no training-specific priors but has poor
  baseline fluency, which may dominate the signal. An early-step checkpoint
  of `Qwen3-8B-Base` (if recoverable from HF) is ideal but may not be
  publicly available. Empirical work needed; the choice itself is part of
  the experiment.
- **2x inference cost is real.** At pilot scale (~1000 traces/day) it's
  manageable. At post-pilot scale it may not be.
- **The "difference" may inherit the same in-distribution bias.** If both
  models were trained on the same web-scrape base corpus, their difference
  on web-scrape duplicate content may be small in *both* directions — and
  the signal collapses to noise on the exact content type we want to gate
  against.

### Detect with

Another bake-off, comparable to A2.6 in shape: same novel + duplicate
slices, same 4 candidates (each candidate playing the "well-trained" role,
paired with a fixed stale partner like Qwen2-7B), measure contrastive AUC
and compare to absolute-perplexity AUC. If contrastive AUC > 0.5 for at
least one well-trained candidate, the approach works.

## Approach 2: Per-token rarity (tighter version of tail-fraction)

### Idea

Rather than aggregating perplexity over all tokens (the current scorer
path) or counting the fraction-below-some-cutoff (the current tail-fraction
floor), explicitly identify the *K rarest tokens* in each trace and score
by their joint surprise. Concretely:

- Run the trace through the scorer model and collect per-token logprobs.
- Sort tokens by logprob (ascending); pick the K lowest (most surprising).
- Score = mean (or sum) of those K logprobs.
- Floor compares this aggregated rare-token score against a calibrated
  threshold.

### Why it might work

A2.3c's Gemma 4 31B run already showed `tail_fraction_range ≈ 0.20` — the
*tail* of the perplexity distribution separates novel from duplicate slices
even when the *aggregate* does not. Tail-fraction (the current floor)
partially captures this by counting how many below-cutoff tokens exist, but
it throws away magnitude information: a trace with 5 mildly-rare tokens
and a trace with 5 extremely-rare tokens score identically.

Per-token rarity preserves magnitude. Novel reasoning often has a small
number of genuinely surprising tokens (a domain-specific term, a non-
canonical reasoning move, an unusual variable name in code) sitting inside
otherwise-fluent surrounding text. The aggregate perplexity averages those
rare tokens away; the tail-fraction counts them flatly. Per-token rarity
weights them.

### Implementation cost

- Modify the score-aggregator in the candle/mistralrs scorer (where
  per-token logprobs are already available) to track and return the top-K
  rarest tokens explicitly.
- Add `TRACE_COMMONS_SCORER_RARITY_K` env var (default ~16 or 32).
- Existing tail-fraction floor stays in place; per-token rarity adds a new
  floor (`TRACE_COMMONS_GATE_RARITY_FLOOR_MICROS`) rather than replacing.

Estimated effort: ~1 day of Rust. The per-token logprob path already
exists; this is an aggregation change at the scorer's output layer.

### Risk

- **The K-rarest tokens might be tokenizer artifacts.** BPE tokenizers
  produce a long tail of rare subword fragments — Unicode codepoints, URL
  fragments, encoded-binary chunks. Those tokens are "rare" in the trained
  model's distribution but say nothing about semantic novelty. A
  tokenizer-aware filter (skip tokens that are pure-punctuation, single
  Unicode codepoints, or shorter than 2 characters) is needed.
- **K is a hyperparameter we'll need to calibrate.** Too low and we miss
  signal; too high and we re-average back to aggregate perplexity. A
  small bake-off variant can sweep K and pick the value with the highest
  AUC.
- **Cherry-picking is real.** With K = 16 across a 1000-token trace, an
  adversarial contributor could pad their trace with one rare token and
  995 boilerplate tokens. Tail-fraction is somewhat robust to this because
  it's a ratio; per-token rarity is more vulnerable. Mitigation: require
  rare tokens to be spread across the trace (e.g., minimum distance
  between rare-token positions) rather than clustered.

### Detect with

Add a per-token-rarity column to the existing bake-off CSV output and
compare AUC to the existing absolute-perplexity and tail-fraction columns
across the A2.6 corpus. Cheap to add; runs as a side computation in the
same bake-off pass. No new GPU time required.

### Prototype

A standalone Python prototype lives at
`scripts/research/per-token-rarity-prototype.py` (with a synthetic fixture
under `scripts/research/fixtures/`). It loads a bake-off corpus `.tar.zst`,
runs a configurable HF causal LM over the novel and duplicate slices, and
reports aggregate-perplexity AUC alongside per-token-rarity AUC using the
same Mann–Whitney U formula as `bakeoff_metrics::discrimination_auc`. Use
it to validate the metric on a real corpus before committing to the A.5a
Rust implementation; the prototype is the canonical "did rarity earn its
keep?" probe referenced in open question 1 below.

## Approach 3: Learned discriminator

### Idea

Train a small classifier — either a LoRA-tuned 3B-param model or a simple
MLP on top of the existing novelty-embedder output — on labeled novel /
duplicate pairs. The discriminator's logit IS the gate score. No
hand-designed feature (perplexity, rarity, novelty cosine) is in the path;
the classifier learns whatever discrimination function the labeled data
supports.

### Why it might work

If human reviewers can reliably label "this trace is novel reasoning" vs
"this trace is paraphrased boilerplate," a trained model can learn the
same distinction. The perplexity approach is essentially asking "what
features of novelty can we encode by hand?" and getting an answer of
"none that work on aligned LLMs." A learned approach sidesteps the
feature-engineering question entirely.

It also avoids the perplexity-calibration problem: the discriminator's
output is a probability, not a model-specific perplexity in unknown units.
Operators can set the floor at 0.5 and it always means "more likely novel
than duplicate" regardless of the underlying model.

### Implementation cost

Highest of the three.

- Need labeled training data. Not available until first ~1000 pilot traces
  accumulate via A.6 (pilot-bootstrap harness).
- Need a labeling pipeline. Either operator-driven (slow, low-volume) or
  reviewer-driven (already in the review flow, but needs schema additions
  to capture "novel/duplicate" alongside the existing review labels).
- Need training infrastructure. Lambda H100 jobs are fine for LoRA tuning;
  serving infrastructure for the resulting model is non-trivial (mistralrs
  serves it but we need a deployment story).
- Need a retraining cadence. The training distribution drifts as
  contributor traffic evolves; a stale discriminator is potentially worse
  than no discriminator. Plan for monthly retraining at minimum.

Estimated effort: probably 2-3 weeks of work spread across labeling
pipeline, training, evaluation, and serving. Cannot start until pilot data
exists.

### Risk

- **Label quality.** Operator/reviewer disagreement on novel vs duplicate
  is real; inter-rater reliability needs measurement before training.
- **Distribution shift.** A discriminator trained on early pilot traces
  may not generalize to later traffic; needs ongoing evaluation.
- **Training/serving infra overhead.** We don't currently have a model-
  retraining-and-deployment pipeline. Building one for a single gate
  metric is a lot.
- **Adversarial vulnerability.** Trained classifiers are notoriously
  attackable by adversaries with query access. Once a contributor sees the
  gate's outputs, they can iterate against it; a hand-designed metric is
  somewhat more robust because there's no gradient to follow.
- **Retraining cadence vs operational simplicity.** Each retraining is a
  potential regression risk; rollback paths need to exist.

### Detect with

Cannot be tested before A.6 (pilot-bootstrap harness) generates labeled
training data. Earliest possible bake-off: after ~1000 pilot traces have
been reviewed and labeled, train a small LoRA against that label set,
hold out 20% for AUC evaluation. If AUC > 0.7 on held-out pilot data, the
approach is viable.

### Dependency

Blocks on A.6 (pilot-bootstrap harness, already in flight per recent
commit). Cannot start design-for-implementation until A.6 lands and pilot
data accumulates.

## Comparison and recommendation

| Approach | Cost | Risk | Earliest deliverable |
|----------|------|------|----------------------|
| Contrastive perplexity | Medium (2x inference, model selection) | Medium (less-trained model choice non-obvious; signal may inherit in-distribution bias) | ~1 week |
| Per-token rarity | Low (modify aggregator) | Low (well-understood tokenizer concerns, gameable but mitigable) | ~3 days |
| Learned discriminator | High (labeling pipeline, training infra, serving, retraining cadence) | Medium-high (label quality, distribution shift, adversarial) | After A.6 pilot data accumulates (~weeks) |

**Recommendation:** sequence as `Per-token rarity → Contrastive perplexity → Learned discriminator`.

1. **Start with per-token rarity** as the first experiment. It's the
   cheapest by an order of magnitude, the lowest risk, and the fastest
   to a yes/no answer. The signal it explores (tail of the per-token
   distribution) is one A2.3c already hinted at. Add a column to the next
   bake-off and we have an answer in days.

2. **If per-token rarity is insufficient, move to contrastive perplexity.**
   It's the natural medium-term follow-up because it tests a structurally
   different hypothesis (training-delta as novelty) rather than just a
   tighter version of what we already have. ~1 week of work plus a
   bake-off run.

3. **Park learned discriminator until A.6 pilot data exists.** Even if
   per-token rarity and contrastive perplexity both fail, a learned
   approach cannot start without labels. Pilot-bootstrap (A.6) is already
   the gating dependency; revisit this approach after A.6 has produced
   the first 1000 labeled traces.

If per-token rarity works, the gate ships with `perplexity-floor = 0`,
`tail-fraction-floor = 0`, `novelty-floor = 500000`, and a new
`rarity-floor = <calibrated>`. A2.5's "perplexity disabled at launch"
recommendation stands; we just add a fourth, working floor next to it.

## Deliverables (this spec)

This spec is the design-space sketch. It produces no code and no
operator-facing documentation directly. Its outputs are:

1. **A.5a — per-token rarity implementation spec.** Filed if the
   recommendation is accepted. Builds out the scorer aggregator change,
   tokenizer-aware filter design, K-hyperparameter sweep design, and
   bake-off integration.
2. **A.5b — contrastive perplexity implementation spec.** Filed if A.5a
   ships and per-token rarity is insufficient.
3. **A.5c — learned discriminator implementation spec.** Filed if A.5a
   and A.5b both ship and remain insufficient, AND A.6 has produced
   sufficient labeled pilot data.
4. **Roadmap update.** Phase A.5's roadmap entry moves from "parked,
   design pending" to "active; sequenced A.5a → A.5b → A.5c."

## Open questions

1. **Does per-token rarity meaningfully improve AUC vs the existing
   tail-fraction floor?** The two are conceptually adjacent. Tail-fraction
   already counts how many below-cutoff tokens exist. If per-token rarity
   is just tail-fraction with magnitude weighting, the AUC delta might
   be marginal. **Recommendation:** A.5a's bake-off must measure both
   columns side-by-side; if the delta is < 0.05 AUC, the cost of a new
   floor isn't worth it.

2. **For contrastive perplexity, what is the canonical less-trained
   partner?** Open candidates: `Qwen2-7B`, `Qwen1.5-7B`, an
   `nano`-scale Qwen3 if one exists, a random-init transformer of the
   same arch. **Recommendation:** A.5b's first experiment should sweep
   2-3 partner choices and report all AUCs.

3. **Are we sure no off-the-shelf novelty/anomaly detector solves
   this?** There's existing literature on novelty detection in text
   (e.g., Mahalanobis-distance-on-embedding-space, OpenAI's GPT-2-era
   surprise metrics). **Recommendation:** spend one day of literature
   review before committing to A.5a; if a published approach beats
   our three candidates on a held-out test set, adopt it instead.

4. **Should we revisit candidate-model selection?** A2.3c picked
   Qwen3-8B-Base by license tiebreaker. If contrastive perplexity is
   sensitive to the well-trained model's training corpus, the model
   choice becomes load-bearing again. **Recommendation:** treat the
   model choice as a sweep variable in A.5b's first experiment rather
   than inheriting from A2.3c.

## Trade-offs explicitly accepted

- **We're committing to a multi-week research arc on the basis of two
  bake-offs (A2.3c/A2.4 and A2.6).** That's a small evidence base for
  a metric redesign. Mitigation: each approach has its own bake-off
  detection criterion; we exit the arc as soon as one approach hits
  AUC > 0.5 rather than running all three to completion.
- **Per-token rarity as the lead candidate is a conservative choice.**
  It's the most-like-what-we-have approach; if perplexity is structurally
  broken on aligned LLMs (A2.6's finding under this branch), per-token
  rarity may inherit the same break. A more aggressive lead choice would
  be contrastive perplexity. We accept the conservative path because the
  cost delta is large (~3 days vs ~1 week) and the information value is
  comparable.
- **Learned discriminator is gated on A.6.** This means if both lighter
  approaches fail, the gate ships pilot with novelty-only (A2.5's posture)
  for weeks or months until pilot data accumulates. That's the realistic
  outcome and we should plan operator communications around it.

## Out of scope (recorded so we don't accidentally re-open it)

- New candidate-model bake-offs against the existing corpus shape (A2.6
  is the last; further runs against OASST2 or Wikipedia novel slices are
  not informative).
- Gate-service architecture changes (the floor / score / threshold
  pattern stays; only the score function changes).
- Replacing the novelty-embedder + vector-index path. That's a different
  floor and a different mechanism; this spec doesn't touch it.
- Phase B / dstack work.
- Model retraining or fine-tuning the existing candidate base models.
  Learned discriminator (A.5c) trains a small classifier head, not the
  base model.
