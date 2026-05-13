# Perplexity Scorer Model Bake-off — Design (Phase A2.1 Retrofit)

Date: 2026-05-13
Status: Draft (pre-implementation)
Owner: Trace Commons / Datasets lane
Predecessor: `2026-05-12-perplexity-scorer-design.md` (A2 picked Llama-3.1-8B-Instruct on tooling grounds, deferred empirical validation to a retrofit)

## Motivation

A2 picked Llama-3.1-8B-Instruct as the default perplexity scorer because
candle's Llama support was the most mature path to a working binary, not
because Llama-3.1-8B was empirically the best choice for our gate. The
2026-05-13 Lambda smoke confirmed the candle stack works end-to-end on
real GPU hardware, which removes the tooling argument. At the same time
the open-weight landscape has moved meaningfully since A2 landed:

- **Qwen 3.6 27B Dense** (April 2026) — Apache 2.0, 256K context, claimed to beat
  the 397B MoE Qwen 3.5 on coding benchmarks.
- **Gemma 4 31B Dense** (April 2026) — first Gemma release under Apache 2.0,
  Google's "byte for byte the most capable open model" claim.
- **Qwen3-8B Dense** (April 2025) — Apache 2.0, more recent than
  Llama-3.1-8B and on a license clean enough for an open audit-anchored
  protocol.

Once we issue credit against a particular `gate_version_hash`, swapping
the model means a vector replay + audit-grandfathering exercise. So
picking well *before* the first pilot run is materially cheaper than
picking well after. This retrofit is the work of picking well, on
evidence.

## Goal

Run a held-out bake-off across a small candidate set of dense
open-weight models, pick the winner on measured perplexity-based
novelty discrimination (not benchmark accuracy), and update the
production default for `TRACE_COMMONS_PERPLEXITY_MODEL_ID` /
`TRACE_COMMONS_PERPLEXITY_MODEL_PATH`. Document the methodology and
the result so the choice is reproducible and reviewable.

## Non-goals

- **Replacing the perplexity gate itself.** Single-floor + tail-fraction
  remains; this retrofit only swaps the model that produces the
  logprobs.
- **MoE candidates.** Expert routing introduces per-batch non-
  determinism that we do not want in a reproducibility-critical gate.
  Documented exclusion; revisit only if a future MoE has provably
  deterministic routing.
- **Real-time model selection.** One configured model per deployment,
  unchanged from A2.
- **Re-running calibration on the live pilot floors.** The floors are a
  separate concern; this retrofit produces the model id, not the
  floor numbers. Floors get re-calibrated against the chosen model in
  a separate pass (the existing `calibrate-from-hf.sh` flow).
- **Embedder bake-off.** A3 (`fastembed` + BGE-large-en-v1.5) stays.
  The embedding signal is orthogonal to the perplexity signal and the
  bake-off would dilute focus.

## Decisions baked in

| Decision | Value |
|----------|-------|
| Architecture filter | Dense only (no MoE) |
| License filter | Apache 2.0 or MIT (no Llama Community license for new picks) |
| Size band | 8B – 32B params (fits H100 80GB with KV-cache headroom) |
| Inference library | `candle` (unchanged from A2) |
| Numeric format | bf16, full precision (unchanged from A2) |
| Eval host | Single A10 24GB for 8B candidates, single H100 80GB for ≥14B |
| Eval framework | Extend existing `tracedao-gate-calibrate` binary |
| Decision artifact | A report doc committed alongside this spec + a one-line PR flipping the default |

## Candidate set

Four candidates, in increasing size order:

| Candidate | Size | License | Why on the list |
|-----------|------|---------|-----------------|
| Llama-3.1-8B-Instruct | 8B dense | Llama Community | **Incumbent.** Must be on the list as the baseline. |
| Qwen3-8B (base) | 8B dense | Apache 2.0 | Same size class, cleaner license, post-Llama-3.1 training. **Base, not instruct** — instruct flattens the distribution and degrades perplexity calibration. |
| Qwen 3.6 27B Dense | 27B dense | Apache 2.0 | Frontier-claim dense model from April 2026. Hybrid Gated-DeltaNet attention is novel; we measure whether it helps or hurts perplexity-on-reasoning-traces. |
| Gemma 4 31B Dense | 31B dense | Apache 2.0 | Google's claim of "most capable open." Apache 2.0 is the new-news; prior Gemma license made it a non-starter. |

Excluded explicitly: Llama 4 Scout (MoE), Qwen3-30B-A3B (MoE), Qwen 3.6
35B-A3B (MoE), DeepSeek V4 Pro/Flash (MoE + too large), Gemma 4 26B MoE
(MoE), Llama 3.2 / 3.3 variants (no clear improvement over 3.1 for our
slot).

Door explicitly left open: **base vs instruct** is its own axis. The
candidate set above is intentionally mixed (Llama-3.1-8B-**Instruct**,
Qwen3-8B-**Base**, Qwen 3.6-27B-Base, Gemma-4-31B-Base) so the bake-off
also produces a base-vs-instruct datapoint. The hypothesis is that base
wins on perplexity calibration; the bake-off will confirm or falsify.

## Evaluation methodology

### Eval corpus

Three slices, stratified, ~1500 traces total:

1. **Known-novel slice (~500 traces)** — held-out subset of OASST2
   conversations and GAIA reasoning traces never used for any model's
   training cutoff. Filter to entries where humans clearly compose
   non-stock reasoning. These should score *high perplexity* (the model
   is surprised → novel signal is real).

2. **Known-duplicate slice (~500 traces)** — boilerplate completions,
   FAQ-style answers, common library docstring text, and curated
   high-frequency code patterns. Sourced from public corpora the
   candidate models almost certainly memorized. These should score
   *low perplexity*.

3. **Adversarial paraphrase slice (~500 pairs)** — pairs of traces where
   one is the original and one is a light paraphrase (back-translation
   or single-token substitutions). A well-calibrated scorer should give
   the paraphrase a *similar* perplexity to the original. Sensitivity
   to trivial rewording is bad for gate stability.

Corpus generation script lands as
`scripts/operator/build-bakeoff-corpus.sh`. Outputs a versioned tarball
with a SHA256 — pinned, so the bake-off is reproducible.

### Metrics

For each (candidate, slice) pair we record:

| Metric | What it measures | Why we care |
|--------|------------------|-------------|
| **Discrimination AUC** | Separation between known-novel and known-duplicate perplexity distributions | Headline metric. Higher is better. |
| **Paraphrase stability** | Median absolute relative perplexity delta between original and paraphrase | Lower is better. Gate-stability proxy. |
| **Tail-fraction range** | Spread of `fraction_below_cutoff` between novel and duplicate slices | Drives the *second* gate floor; we want spread, not collapse. |
| **Throughput** | Tokens/sec on the target hardware (A10 or H100) | Operational reality. Used to break ties only. |
| **Memory footprint** | Peak VRAM, end-of-eval | Confirms it fits the deployment hardware. |
| **Determinism** | Std-dev of perplexity over 3 repeat runs of the same input (seed pinned) | Sanity check that the scorer is reproducible — table stakes. |

### Decision rule

The winning model is the candidate that:

1. Hits the determinism gate (std-dev < 1e-5 on repeat) — table stakes.
2. Maximizes a weighted score: `0.6 * discrimination_auc + 0.3 * (1 -
   paraphrase_delta_normalized) + 0.1 * tail_fraction_range_normalized`.
3. Falls within ≤ 50% throughput penalty vs the fastest candidate (we
   don't want to win by being 10x slower in absolute terms).

Ties (within 2% on the weighted score) are broken by:
1. License permissiveness (Apache > MIT > Llama Community).
2. Smaller params (less VRAM = more KV cache headroom = bigger context).
3. Recency of release (newer training cutoff → less stale).

This rule is committed *before* the bake-off runs. Picking after
inspecting results is how you fool yourself.

## Implementation

### Surface change in `tracedao-gate-calibrate`

The existing offline binary at `crates/tracedao-server/src/bin/
tracedao-gate-calibrate.rs` runs perplexity calibration against a
single configured model. Extend it with a `--bake-off` mode that
accepts a candidate manifest and an eval-corpus tarball, then runs
the metric set above against each candidate and emits a JSON report.

```rust
// New CLI surface:
//   tracedao-gate-calibrate bake-off \
//     --candidates=/path/to/candidates.toml \
//     --corpus=/path/to/bakeoff-corpus.tar.zst \
//     --hardware=h100|a10 \
//     --report-out=/path/to/report.json
```

`candidates.toml`:

```toml
[[candidate]]
id = "llama-3.1-8b-instruct"
path = "/srv/models/llama-3.1-8b-instruct"
arch = "llama"
license = "llama-community"

[[candidate]]
id = "qwen3-8b-base"
path = "/srv/models/qwen3-8b-base"
arch = "qwen2"
license = "apache-2.0"

[[candidate]]
id = "qwen3.6-27b-dense"
path = "/srv/models/qwen3.6-27b"
arch = "qwen2"
license = "apache-2.0"

[[candidate]]
id = "gemma-4-31b-base"
path = "/srv/models/gemma-4-31b"
arch = "gemma3"
license = "apache-2.0"
```

Each candidate gets loaded via `CandlePerplexityScorer::try_new` (same
constructor A2 ships), scored against the corpus, then torn down before
the next candidate loads. Loading all four simultaneously is not an
option — they don't fit. Sequencing means the full bake-off is bounded
by `sum(load_time + corpus_eval_time)` across candidates.

### Reproducibility

The bake-off emits:

- `report.json` — full per-candidate per-slice metrics
- `report.md` — human-readable summary, decision rule output, winner
- `report.sha256` — hash of the report
- Corpus tarball SHA256, candidate manifest SHA256, and a
  `gate_version_hash` for each candidate

Anyone running the same corpus + candidates on the same hardware
generation should reproduce the report bit-for-bit (modulo
floating-point non-determinism in candle, which we cap at the
determinism gate above).

### Hardware budget

| Phase | Hardware | Est. time | Cost |
|-------|----------|-----------|------|
| Corpus build (one-time) | CPU host | ~30 min | free (existing infra) |
| 8B candidates (Llama-3.1, Qwen3) | A10 24GB | ~1.5 hr each | ~$4 |
| ≥14B candidates (Qwen 3.6 27B, Gemma 4 31B) | H100 80GB | ~3 hr each | ~$15 |
| Report generation + decision | local | ~30 min | free |
| **Total bake-off run** | | **~9 hr GPU** | **~$35** |

Budget cap: $50. If a candidate fails to load or the eval blows up,
abort that candidate, document why in the report, and continue with
the remaining set.

## Open questions

1. **Should the bake-off corpus be checked into the repo or hosted
   externally?** The corpus is ~50-200 MB compressed; bigger than is
   polite for git but small enough that LFS works. Recommend: LFS, so
   the bake-off is reproducible from a single `git clone`. Operator
   can opt out via env var if LFS bandwidth is a concern.

2. **How adversarial should the paraphrase slice be?** Back-translation
   via a small open model produces moderate paraphrases; trained
   paraphrase models produce stronger ones. Stronger is more honest
   but introduces a second model dependency. Recommend: back-
   translation with Qwen3-4B (cheap, Apache 2.0, no new dep).

3. **What if the winner is barely better than the incumbent?** Define
   "barely" as a < 3% improvement on the weighted score. In that case
   the recommendation is to *keep* Llama-3.1-8B-Instruct, on the
   reasoning that we already have the operational muscle memory and
   the model swap costs (vector replay, audit grandfathering) aren't
   justified by a marginal win. The bake-off explicitly accepts "no
   change" as a valid outcome.

4. **What if Qwen 3.6 27B Dense or Gemma 4 31B Dense wins, but we
   haven't acquired H100 budget yet?** The smoke ran on an A10; the
   pilot is targeted at H100 80GB but that's not yet provisioned. If
   the winner is the 8B candidate, this is moot. If it's a ≥27B
   candidate, the pilot deployment plan needs an H100 line item before
   the model swap lands. Document the dependency in the resulting PR;
   don't flip the default until the production hardware is real.

5. **Recalibration cost after the swap.** Floors are scaled to the
   winner's perplexity distribution, which is different from the
   incumbent's. The `calibrate-from-hf.sh` pass needs to re-run after
   the model swap. Account for this in the rollout (1 day, ~$20 of
   H100 time, dry-run gate emission throughout).

## Deliverables

1. `crates/tracedao-server/src/bin/tracedao-gate-calibrate.rs` extended
   with `--bake-off` mode.
2. `scripts/operator/build-bakeoff-corpus.sh` — corpus assembler from
   OASST2 + GAIA + curated duplicate slice.
3. `docs/operator/calibration.md` — new "Model bake-off (A2.1)"
   subsection pointing to this spec and the resulting report.
4. `docs/superpowers/reports/2026-MM-DD-model-bakeoff-result.md` —
   the actual bake-off report, committed after the run.
5. A one-line PR flipping `TRACE_COMMONS_PERPLEXITY_MODEL_ID` /
   `TRACE_COMMONS_PERPLEXITY_MODEL_PATH` defaults to the winner.
   Separate PR from the bake-off binary, so the decision is reviewable
   on its own.

## Rollout

This retrofit lands *before* the first pilot, not after. Sequencing:

1. **A2.1a — binary + corpus** — extend the calibrate binary, build
   the corpus, exercise on a single candidate locally (cheap).
2. **A2.1b — bake-off run** — provision the GPU host, run all four
   candidates, generate the report. Single $35-ish GPU session.
3. **A2.1c — decision** — review the report, flip the default (or
   document no-change). One PR.
4. **A2.1d — recalibration against the winner** — run
   `calibrate-from-hf.sh` against the winner on the same H100 session
   that finishes the bake-off; reuse the GPU box. One PR with the new
   floor numbers.
5. **A2.1e — final smoke on the winner** — re-run the Lambda smoke
   from 2026-05-13 with the new model + floors, confirm
   `gate_service.ready=true` and audit-chain drill still passes. One
   PR if any operator-doc changes are needed.

Total elapsed: ~1 week, ~$60 GPU spend.

## Trade-offs explicitly accepted

- We are picking a model on metrics we designed, not on user behavior.
  The eval corpus is our best honest proxy for real pilot traces but
  it is a proxy. The first ~1000 real pilot traces should be used to
  cross-validate; if the bake-off winner underperforms in the real
  distribution, that's a Phase 2 re-cal trigger, not a retrofit
  failure.
- We are excluding MoE on a-priori grounds. If MoE inference becomes
  provably deterministic (e.g. fixed expert routing per token, or a
  candle-level reproducibility primitive), we re-open the candidate
  set. Today, MoE perplexity scores aren't reproducible enough for a
  credit-gating role.
- We are accepting some over-fit risk to OASST2 + GAIA distributions.
  Both corpora are widely trained-on; "known-novel" is a strong word.
  Mitigation: the adversarial paraphrase slice catches the
  pathological case where a model simply memorized the test set.

## Out of scope (recorded so we don't accidentally re-open it)

- Dynamic per-trace model selection.
- A learned classifier replacing the perplexity floor.
- Quantized variants (Q4/Q5/Q8) in the bake-off — A2 ruled out quantized
  for the gate path; this retrofit doesn't re-litigate that.
- Embedder swap or vector-dim change — A3 stays.
- Phase B / dstack work — orthogonal; the chosen model carries over to
  Phase B unchanged.
