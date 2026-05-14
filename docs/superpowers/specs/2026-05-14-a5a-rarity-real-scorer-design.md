# Phase A.5a — Per-Token Rarity Real-Scorer Wiring and Pre-Flight (Design)

Date: 2026-05-14
Status: **DRAFT — activated only if A2.6 final results trigger A.5 activation
(all candidates AUC < 0.4, or operator decision in the 0.4-0.5 partial
range).** Do not implement against this spec until A2.6 results are recorded
in `docs/superpowers/reports/` and an operator has signed off on A.5
activation.
Owner: Trace Commons / Datasets lane
Predecessors:
- `2026-05-14-a5-perplexity-replacement-design.md` (A.5 — replacement-metric
  design that scopes per-token rarity as candidate #1)
- `2026-05-14-agent-traces-bakeoff-design.md` (A2.6 — bake-off whose
  outcome activates A.5 / A.5a)
- `2026-05-14-gate-floor-recalibration-design.md` (A2.5 — perplexity
  inversion finding)
- `2026-05-12-perplexity-scorer-design.md` (A2 — original perplexity scorer
  whose `LocalPerplexityScorer` we extend)
- PR #63 (`Add per-token rarity scorer to bake-off binary`) — shipped the
  `TokenRarityScorer` trait, mock scorer, `per_token_rarity_micros` helper,
  and `--scorer perplexity|token-rarity|both` CLI flags; explicitly deferred
  the real-scorer rarity path with error class
  `BakeoffRealRarityNotImplemented`.

## Goal

Wire `LocalPerplexityScorer` (in `crates/tracedao-gate-enclave/src/perplexity_local.rs`)
to implement the `TokenRarityScorer` trait so the existing bake-off
binary's `--scorer token-rarity` and `--scorer both` paths run against real
mistralrs-backed models — not just `MockTokenRarityScorer` — and so that
final-results activation of A.5 has a build-ready slice to execute.

## Pre-flight experiment (mandatory before production wiring)

Architect verdict (2026-05-14) requires an offline AUC pre-flight against the
exact A2.6 corpus **before** we invest in the real-scorer wiring. The goal is
to cheaply confirm that per-token rarity does not share perplexity's failure
mode on this corpus shape; if it does, A.5a is the wrong slice and we should
re-route to A.5b (contrastive).

Procedure:

1. Pick one A2.6 candidate (recommend `meta-llama/Llama-3.1-8B-Instruct` for
   continuity with the A2.6 partial result, AUC 0.342).
2. Re-run that single candidate on a Lambda H100 in **rarity-only** mode
   against the same A2.6 corpus
   (`sha256:46e0eef8a52e309ce695ad20d1e242ce43eb210c11e02764beeaf7fa3d341bb5`).
3. Emit per-trace rarity scores and a per-K AUC sweep for
   `K ∈ {4, 8, 16, 32}`.
4. Do **not** log token strings, token IDs, or any trace text — hash-only
   audit per the repo convention. Pre-flight emits only AUCs, per-trace
   scalar scores, and corpus-hash provenance.

Decision rule:

- **Proceed with A.5a** if rarity AUC > 0.5 **AND**
  `(rarity AUC − A2.6 perplexity AUC) ≥ 0.05` for at least one K in
  `{4, 8, 16, 32}`.
- **Stop and re-route to A.5b (contrastive)** otherwise. Mark this spec
  `DRAFT — superseded by A.5b` rather than building dead code against
  rarity.

Expected cost: ~1.5 hours GPU on Lambda H100, ~$8 — a fraction of A2.6.

Implementation note: this pre-flight is an **investigation, not a build**.
It does not require `LocalPerplexityScorer` to implement `TokenRarityScorer`
yet. A one-off Python script consuming the mistralrs `Raw` channel directly
and aggregating rarity offline is sufficient. Keep the script in
`scripts/research/` and treat its output as evidence, not production code.

## Real-scorer wiring (the actual A.5a build)

If and only if the pre-flight decision rule passes:

- **Where to extend.** Implement the `TokenRarityScorer` trait on
  `LocalPerplexityScorer` in
  `crates/tracedao-gate-enclave/src/perplexity_local.rs`. Keep the existing
  `PerplexityScorer` impl untouched; rarity is an additional trait impl on
  the same type, sharing the underlying mistralrs session.
- **Sourcing per-token logprobs.** The current perplexity path already
  consumes per-token logprobs from the mistralrs (git-pinned `2d4ba4f`)
  `Request::Normal` + `ResponseOk::Raw` channel and aggregates them into a
  sum/mean. The change for rarity is to expose those logprobs
  **un-aggregated** to the rarity formula. The aggregation lives in
  `per_token_rarity_micros` — that helper is the proven path (it has a
  Python-prototype parity test from PR #63) and must be reused as-is.
- **K parameter.** `--token-rarity-k` already exists at the CLI layer
  (default 10). Plumb it through the bake-off harness into the real-scorer
  call site; do not re-introduce a hard-coded K.
- **Error class cleanup.** Once the real path is wired and tested, remove
  `BakeoffRealRarityNotImplemented` from the bake-off binary's error enum.
  Leaving the class behind invites silent regression.

## Test plan

- **Unit anchor.** Re-use PR #63's synthetic-fixture parity test for
  `per_token_rarity_micros`. It already pins the Python-prototype numerics;
  no new unit test is required for the aggregation.
- **Integration test.** Add a bake-off-binary integration test that runs
  `--scorer both` against a tiny in-tree corpus with the real
  `LocalPerplexityScorer`. Keep the existing mock-scorer integration test
  as a separate target — the mock path must not regress when the real path
  lands.
- **Build hygiene.** Run `cargo test -p tracedao-gate-enclave` and
  `RUSTFLAGS="-D warnings" cargo test --no-run` on the affected crates to
  ensure removing the deferred error class does not leave dead code.
- **Determinism.** Pre-flight and integration runs must record the corpus
  sha256 and the mistralrs commit pin in the report header so re-runs are
  reproducible.

## Production wiring (deferred — out of scope for A.5a)

A.5a's deliverable is **"real-scorer rarity works in the bake-off
binary."** That is all. The production gate-service still consumes the
perplexity floor; rolling rarity into the gate-service is a separate slice
contingent on A.5b confirmation that rarity holds up on a second corpus
shape. Do not extend the gate-service in this slice; do not change floor
calibration in this slice; do not touch operator surfaces in this slice.

## Open questions

- **mistralrs Raw-channel granularity.** Will the `ResponseOk::Raw`
  channel surface logprobs at the exact granularity rarity needs (per-token,
  post-softmax), or will an intermediate transformation be required to
  recover the post-softmax distribution from raw logits? Architect flagged
  this as the main wiring risk; the pre-flight script will tell us.
- **K choice.** Spec inherits the PR-#63 default of `K = 10`, but the
  pre-flight sweep over `{4, 8, 16, 32}` will determine whether that
  default is appropriate or should move.
- **Tokenizer-artifact mitigation.** Architect flagged URL fragments and
  UUIDs as a likely rarity failure mode (synthetic high rarity on tokens
  that carry no novelty signal). Open whether filtering belongs at the
  scorer layer, upstream at ingestion, or in a corpus pre-pass. Decision
  deferred until pre-flight evidence is in hand.

## Non-goals

- No dependency on any closed-API model. mistralrs + open-weights only.
- No collection of labeled / supervised training data. Rarity is unsupervised
  by design.
- No production gate-service rewiring. That is A.5b territory at the
  earliest.
- No abandoning perplexity as a co-floor. Per the A.5 design spec, rarity
  ships **alongside** perplexity as an additional floor, not as a
  replacement; co-floor composition stays as designed.

## Trade-offs explicitly accepted

- **One additional bake-off run before any production work.** The
  pre-flight burns ~$8 of GPU time and ~1.5 hours of wall clock before we
  even start the wiring. Accepted in exchange for not building a Medium-
  effort real-scorer slice on top of a metric that quietly inherits
  perplexity's failure mode.
- **Risk that rarity shares perplexity's failure mode.** Mitigated by the
  pre-flight decision rule — if AUC does not clear `> 0.5` and a +0.05
  margin over A2.6 perplexity for some K, we stop and re-route to A.5b
  rather than push through.
- **Real-scorer wiring is non-trivial.** Architect estimate is Medium
  effort (1-2 days) once pre-flight clears. The cost is in surfacing
  un-aggregated logprobs from the mistralrs Raw channel and threading K
  end-to-end; the aggregation math itself is already shipped and tested.
