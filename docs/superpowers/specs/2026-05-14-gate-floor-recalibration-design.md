# Gate Floor Recalibration — Design (Phase A2.5 Retrofit)

Date: 2026-05-14
Status: Draft (pre-implementation)
Owner: Trace Commons / Datasets lane
Predecessors:
- `2026-05-12-perplexity-scorer-design.md` (A2 — original gate floor design)
- `2026-05-13-model-bakeoff-retrofit-design.md` (A2.1)
- `2026-05-13-bakeoff-arch-dispatch-design.md` (A2.2)
- `2026-05-13-mistralrs-migration-design.md` (A2.3)
Driver: A2.3c (`2026-05-13-model-bakeoff-result-notes.md`) + A2.4 (Wikipedia-corpus re-run, in flight at spec time)

## Motivation

A2 designed the production novelty gate with three floor controls:

| Env var | Intended semantic |
|---------|-------------------|
| `TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS` | Minimum aggregate perplexity ("model surprise") for a trace to pass |
| `TRACE_COMMONS_GATE_TAIL_FRACTION_FLOOR_MICROS` | Minimum tail-fraction (fraction of below-cutoff tokens) |
| `TRACE_COMMONS_GATE_NOVELTY_FLOOR_MICROS` | Minimum cosine novelty against the vector index |

The A2.3c and A2.4 bake-offs measured what these floors actually
discriminate when run against four modern LLM candidates (Llama-3.1-8B-
Instruct, Qwen3-8B-Base, Qwen 3.6 27B Dense, Gemma 4 31B Base) across
two distinct duplicate-slice constructions. The headline finding is
uncomfortable:

**Across every candidate and every duplicate-slice variant, the
perplexity-based novelty AUC is well below 0.5** (range: 0.054 to
0.276). The metric isn't noisy or weakly-discriminating — it's
*inverted*. Modern instruct-aligned LLMs find OASST2-style reasoning
*less* surprising than well-trodden duplicate content (Wikipedia
intros, license boilerplate, stock prose). Setting a positive
perplexity floor would systematically *reject* contributor-grade
reasoning traces and *accept* duplicates.

This means the A2 gate's first floor — `PERPLEXITY_FLOOR_MICROS` —
does not measure what we want it to measure, on any of the candidate
models we can run. The framing of "novel reasoning = surprising
tokens" is broken for the modern aligned-LLM ecosystem.

## Goal

Reconfigure the gate floors for first pilot launch to reflect the
empirical reality:

1. Ship `TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS=0` (effectively
   disabled) for pilot launch, with documented justification in the
   operator runbook.
2. Promote `TRACE_COMMONS_GATE_TAIL_FRACTION_FLOOR_MICROS` to the
   active perplexity-side gate, calibrated against actual pilot
   traces once they exist. A2.3c's Gemma 4 31B run showed
   `tail_fraction_range` ≈ 0.20 — the *tail* of the perplexity
   distribution does separate slices even when the *aggregate* does
   not.
3. Keep `TRACE_COMMONS_GATE_NOVELTY_FLOOR_MICROS` as the primary gate.
   The novelty-embedder + vector-index path is unaffected by the
   perplexity finding; it's a similarity-based discriminator, not a
   generation-probability one.
4. Document the gate-design tradeoffs and future-work options so the
   operator has full context for the pilot launch and for the eventual
   Phase A.5 gate redesign.

## Non-goals

- **No code changes.** All three env vars already exist. Their
  semantics don't change; only the *recommended values* for pilot
  launch change.
- **No new metric design.** This is a reconfiguration, not a gate
  redesign. The perplexity-replacement work (contrastive perplexity,
  per-token rarity, learned discriminator) is parked for Phase A.5.
- **No re-run of the bake-off.** The A2.3c + A2.4 data is sufficient
  to make the recalibration decision; further bake-off runs against
  the current corpus shape would just re-confirm the same finding.
- **No invalidation of A2.3 or A2.2.** The mistralrs migration and
  arch-dispatch work are still load-bearing; this retrofit operates
  one layer up, on the operator-facing parameters.

## Decisions baked in

| Decision | Value | Rationale |
|----------|-------|-----------|
| `PERPLEXITY_FLOOR_MICROS` for pilot launch | **0** | All measured AUCs < 0.5; any positive floor rejects in the wrong direction. |
| `TAIL_FRACTION_FLOOR_MICROS` for pilot launch | **0** at code-launch, calibrate post-first-1000-traces | A2.3c data suggests real signal exists but it's not yet calibrated against pilot-distribution traces. |
| `NOVELTY_FLOOR_MICROS` for pilot launch | **500000** (cosine novelty 0.5) — unchanged from existing deployment.md recommendation | Embedder + vector-index path is unaffected by the perplexity finding. |
| Model pick stays `Qwen3-8B-Base` | Operationally cleanest of the candidates (Apache-2.0, base, smallest VRAM). The "winner by license tiebreaker" framing of A2.3c stands but isn't load-bearing for the gate-floor decision. | Picking a different candidate doesn't change the perplexity-floor decision; all four had AUC < 0.5. |
| Per-token-rarity / contrastive-perplexity work | Deferred to Phase A.5 | Phase A pilot can ship with novelty-only as the active perplexity-side gate. |

## What we shipped vs what we ship now

| Floor | A2 deployment.md target | A2.5 pilot recommendation | Why |
|-------|--------------------------|----------------------------|-----|
| Perplexity | "calibrate to X micros from Phase 1" | **0** (disabled at launch) | Inverted signal across all candidates |
| Tail-fraction | "calibrate to X micros from Phase 1" | **0** at launch, calibrate after ~1000 pilot traces | Tail signal exists but not yet pilot-calibrated |
| Novelty (cosine) | 500000 | 500000 (unchanged) | Embedder path works fine |

## Open questions

1. **Should the perplexity floor be inverted to `<= floor` semantics
   instead of disabled?** Setting a low-perplexity *ceiling* would
   reject traces that are too easily-predictable (boilerplate-like).
   That's a different gate — not novelty — but might still have
   value. **Recommendation:** Park for Phase A.5. Pilot-launch
   simplicity wins; one less floor to calibrate.

2. **Should we revisit the candidate selection?** A2.3c picked
   Qwen3-8B-Base by license tiebreaker inside a marginal-AUC band.
   With perplexity disabled at launch, the model choice matters
   mainly for tail-fraction signal quality. Gemma 4 31B had the
   strongest tail-fraction range (0.20 in A2.3c). **Recommendation:**
   stay with Qwen3-8B-Base for pilot. Smaller model = faster scoring
   = more contributor traces per unit cost. Gemma 4 is a future
   upgrade candidate if pilot data shows tail-fraction is doing real
   work.

3. **What metric replaces perplexity in Phase A.5?** Candidates:
   - **Contrastive perplexity:** difference in logprobs between two
     model checkpoints (one well-trained, one less so). The
     *difference* might be more novelty-indicative than absolute.
   - **Per-token rarity:** explicitly gather lowest-N logprobs across
     the trace. If *any* genuinely surprising tokens exist, treat as
     novel. (Basically what tail-fraction does, just tighter.)
   - **Learned discriminator:** train a small classifier on labeled
     novel/duplicate examples. Requires labeled pilot data we don't
     have yet.
   **Recommendation:** All three are research-shaped. Pick after the
   first ~1000 pilot traces let us label novel/duplicate exemplars
   from our actual distribution.

4. **Does the embedder + vector-index path actually work?** A2/A3/A4
   shipped the path but A2.3c + A2.4 didn't test it (those were
   perplexity-only bake-offs against the gate's first floor). The
   pilot launch is the first real test. **Recommendation:** Add
   embedder + vector-index validation to the operator smoke
   checklist; if novelty AUC also collapses on real traces, A2.5's
   "perplexity-disabled, novelty-primary" recommendation needs
   revisiting *fast*.

## Deliverables

1. **A2.5a — findings report.** A `docs/superpowers/reports/
   2026-05-14-gate-floor-recalibration-findings.md` documenting the
   A2.3c + A2.4 data and the decision logic. Companion to the
   existing A2.1 / A2.3c result reports.
2. **A2.5b — calibration runbook update.** Rewrite the
   `docs/operator/calibration.md` Phase 0 + Phase 1 sections to
   reflect the new floor recommendations.
3. **A2.5c — env-reference update.** Note in `docs/operator/
   env-reference.md` that the perplexity floor defaults to 0 for
   pilot launch with a pointer to A2.5 for context.
4. **A2.5d — roadmap entry.** Add an A2.5 status line under Phase A
   in `docs/trace-commons-roadmap.md`. Phase A.5 (gate-design
   reconsideration) joins the deferred list.

A2.5 is documentation-only. No code change ships. The existing env
vars + binary continue to work; just the *values* the operator should
set at pilot launch differ from the A2 spec.

## Out of scope (recorded so we don't accidentally re-open it)

- New perplexity metric design (contrastive, per-token rarity, learned).
- Gate-service trait changes.
- Bake-off binary updates.
- Phase B / dstack work.
- Model retraining or fine-tuning the candidate models.

## Trade-offs explicitly accepted

- **The gate as designed in A2 doesn't measure novelty.** This was
  the original A2 hypothesis; the data invalidates it for modern
  aligned LLMs. We're not pretending otherwise.
- **The pilot launches with one of three intended floors active.**
  Novelty (embedder-based) is the only floor doing real work at
  launch. Tail-fraction joins after first-1000-trace calibration.
- **The model choice (Qwen3-8B-Base) is no longer decisive.** Picking
  a different candidate from the A2.3c set wouldn't materially change
  the gate behavior given perplexity is disabled. We keep the smallest
  candidate for cost reasons.
- **Phase A.5 work is real but deferred.** Until pilot data shows
  what kind of "novelty" we actually need to discriminate, designing
  a replacement metric is premature.

## What's worth being explicit about

If a stakeholder asks "does the perplexity gate work?" — the honest
answer is **"the perplexity gate as we designed it in A2 does not
discriminate novel reasoning traces from common content for any of
the four candidate models we can run today. We are shipping the
pilot with the perplexity floor at 0 (disabled). The novelty-embedder
floor — a different mechanism that does similarity matching against
the vector index — is the primary gate. The tail-fraction floor will
be calibrated post-first-1000-pilot-traces. A future retrofit (Phase
A.5) may add a contrastive-perplexity or learned-discriminator
metric that does work."**

That's the spec. No code change ships. Just the operator runbook +
findings record + roadmap entry.
