# Phase A.5 Perplexity-Replacement Metric Implementation Plan (STUB)

> **Status:** STUB — activated only on A2.6 AUC < 0.4. Until then, do not execute. The spec at `docs/superpowers/specs/2026-05-14-a5-perplexity-replacement-design.md` is authoritative for design intent.
>
> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the perplexity-based novelty scorer with a metric that actually crosses AUC > 0.5 on realistic Trace Commons corpora.

**Architecture:** TBD pending spec resolution of metric choice. Two candidates from the spec: (a) per-token rarity (top-K rarest log-probs aggregate), (b) contrastive perplexity (base-model vs aligned-model log-prob delta). The spec also parks a learned-discriminator option for follow-up.

**Tech Stack:** Same as current — mistralrs raw-logits channel for token-level log-probs.

## 1. What already exists

The per-token-rarity scaffolding landed in PR #63 ahead of A.5 activation. If A.5
fires, the following components are already in tree and do **not** need to be
rebuilt:

- `TokenRarityScorer` trait — per-token rarity scorer interface (PR #63).
- `MockTokenRarityScorer` — deterministic fixture scorer used by the bake-off
  binary's mock-scorer path (PR #63).
- Bake-off binary `--scorer perplexity|token-rarity|both` CLI flag —
  selects which metric the bake-off computes for each row (PR #63).
- Bake-off report `metrics: {perplexity, token_rarity}` JSON schema — both
  metric columns are emitted side-by-side so AUC comparisons against the
  existing perplexity column are mechanical (PR #63).
- Synthetic-fixture parity test against the Python prototype at
  `scripts/research/per-token-rarity-prototype.py` — confirms the Rust
  aggregator matches the prototype's numbers on the fixture under
  `scripts/research/fixtures/` (PR #63).

**Open puzzle piece (intentionally deferred in PR #63):**

- Real-scorer (`LocalPerplexityScorer`) rarity wiring is **not** implemented.
  The bake-off currently returns a `BakeoffRealRarityNotImplemented` error
  class when `--scorer token-rarity|both` is combined with the real
  (mistralrs-backed) scorer path. This is the load-bearing deferred work A.5a
  has to land before any real-corpus bake-off can run.

## 2. What needs to land

TBD list pending metric choice. Sketch per candidate from the spec
(`§Comparison and recommendation`):

### Candidate A — Per-token rarity (spec recommendation, lead)

- [ ] Lift the per-token logprob vector out of mistralrs differently. The
  current `LocalPerplexityScorer` aggregates to a single perplexity scalar;
  rarity needs the full per-token vector. Investigate whether mistralrs
  exposes the raw logprob stream or whether we need to keep the
  intermediate vector before aggregation.
- [ ] Implement `TokenRarityScorer` for `LocalPerplexityScorer` (the type
  that currently returns `BakeoffRealRarityNotImplemented`).
- [ ] Add tokenizer-aware filter (skip pure-punctuation, single-Unicode-
  codepoint, and < 2-char tokens) per spec `§Approach 2 / Risk`.
- [ ] Wire `TRACE_COMMONS_SCORER_RARITY_K` env var (default ~16 or 32).
- [ ] Add minimum-spread constraint on rare-token positions per spec
  cherry-picking mitigation.

**Estimate:** ~1 day of Rust. The bake-off binary, CLI flag, JSON schema,
and trait are all already in place from PR #63.

### Candidate B — Contrastive perplexity

- [ ] Add `TRACE_COMMONS_SCORER_CONTRASTIVE_MODEL_PATH` env var.
- [ ] Load both well-trained and less-trained models at scorer startup.
- [ ] Modify the score path to run two forward passes per scoring call and
  return the difference.
- [ ] Doubles VRAM and latency — spec needs to confirm the cost is
  acceptable at pilot scale before this candidate fires.

**Estimate:** 2-3 days of Rust + 1 day of bake-off integration, plus
operator GPU time for a fresh bake-off run (~$50-100 on Lambda H100).

### Candidate C — Learned discriminator

Cannot start without A.6 (pilot-bootstrap) pilot data. Out of scope for the
initial A.5 activation; revisited only if A.5a + A.5b both fail. Spec
parks this until pilot-bootstrap produces ~1000 labeled traces.

## 3. Decision gate

A.5 should not start until **all** of the following hold:

- [ ] A2.6 report is in (current state at stub write: bake-off running on
  Lambda H100). Report path:
  `docs/superpowers/reports/2026-05-14-model-bakeoff-result-a26.{json,md}`.
- [ ] A2.6 AUC < 0.4 across all candidates **OR** partial-pass case
  (0.4 < AUC < 0.5) where the operator explicitly decides to activate
  A.5 anyway. Per the A2.7 spec, the 0.4 < AUC < 0.5 branch parks A.5
  with reduced urgency by default; activating in that branch is a
  judgement call, not mechanical.
- [ ] Metric-choice ambiguity in the spec is resolved. The spec lists
  three candidates and recommends per-token rarity as the lead but the
  recommendation is conditional on a one-day literature-review check
  (spec `§Open questions Q3`). Resolve via Plan Reviewer agent before
  starting A.5a.

If A2.6 AUC > 0.5 fires for any candidate, A2.7 activates instead and
this plan is closed (not executed). See
`docs/superpowers/specs/2026-05-14-a27-perplexity-floor-update-design.md`
for the alternative path.

## 4. Slice sketch

Per metric choice, but the common skeleton:

### Slice A.5a — Wire real scorer for chosen metric

- [ ] Replace `BakeoffRealRarityNotImplemented` with a working real-
  scorer rarity path (Candidate A) OR add a two-model contrastive path
  (Candidate B).
- [ ] Unit tests against the synthetic fixture used in PR #63's parity
  test.
- [ ] Integration test that runs the bake-off binary end-to-end against
  a small fixture corpus with `--scorer <chosen>`.

### Slice A.5b — Re-run bake-off

- [ ] Re-run the bake-off on the **same** corpus as A2.6 (agent-traces
  novel slice + Wikipedia duplicate slice) for AUC-comparability with
  A2.6's numbers.
- [ ] Operator GPU time on Lambda H100; budget per spec.
- [ ] Land the report at
  `docs/superpowers/reports/2026-05-14-a5a-bakeoff-result.{json,md}`.

### Slice A.5c — Report and recommend floor

- [ ] If A.5b shows AUC > 0.5 on the chosen metric, derive a new floor
  value using a recipe analogous to A2.7 `§Outcome 1 procedure`
  (Youden's J + low-tail percentile + 0.5× headroom margin).
- [ ] If A.5b still shows AUC < 0.5, escalate to Candidate B (or C,
  pending A.6) per spec sequencing.
- [ ] Operator runbook update analogous to A2.7's documentation set
  (`calibration.md`, `env-reference.md`, `deployment.md`).

### Slice A.5d — Production wiring

- [ ] Replace **or** augment the perplexity scorer in the gate-service
  with the new metric. Per spec `§Comparison and recommendation`, if
  per-token rarity works it should ship **alongside** the existing
  (disabled) perplexity floor as a fourth floor
  (`TRACE_COMMONS_GATE_RARITY_FLOOR_MICROS`) rather than replacing it.
- [ ] Migrate operator-facing documentation: env-reference,
  deployment, calibration.md, runbook.
- [ ] Smoke check that the new floor is wired into the rollout-smoke
  evidence path the same way the existing floors are.

## 5. Out of scope

Per the spec `§Out of scope`:

- Do not add new corpora variants. Use the same novel + duplicate slices
  A2.6 used so AUC numbers are directly comparable.
- Do not change the bake-off binary's structural shape (CLI flag,
  report JSON schema, mock-vs-real scorer split). PR #63's contract
  stands.
- Do not introduce a new dependency without explicit approval (per
  the repo dependency policy).
- Do not wire production scorer changes (slice A.5d) until the
  bake-off proves the metric (slices A.5a / A.5b / A.5c).
- Do not re-litigate model selection. The bake-off candidate set
  inherits from A2.6.
- Do not touch `TAIL_FRACTION_FLOOR_MICROS` or `NOVELTY_FLOOR_MICROS`.
  Those remain on A2.5's settings under every A.5 outcome; A.5 only
  adds a new floor or replaces the perplexity floor.
- Do not start the learned-discriminator candidate (Candidate C) before
  A.6 has produced labeled pilot data.
