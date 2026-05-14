# Bake-off report (<TBD: generated_at from JSON>)

- run identifier: A2.6c (agent-traces novel slice retrofit)
- corpus: sha256:46e0eef8a52e309ce695ad20d1e242ce43eb210c11e02764beeaf7fa3d341bb5
- manifest: sha256:2e360df9449d81d664caeb0e17ed893ccb28e5998604c4caafd1aa46a13fd0f0
- decision-rule version: 1
- ctx_max_tokens: 4096
- determinism gate: 0.00001
- novel slice: swival agent-traces (security-audit proof + fix_outline + source_code prefix, 300 rows, length-filtered 200-2000 words)
- duplicate slice: Wikipedia article intros (reused from A2.4 corpus build, 300 rows)
- paraphrase slice: 300 rows (Qwen3-4B-Base back-translation, batched)
- hardware: Lambda H100 SXM5 80GB, region <TBD>

Winner: <TBD>

| candidate | auc | paraphrase_delta | tail_range | throughput_tps | determinism_stddev | license | params_b | passed_gate |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| llama-3.1-8b-instruct | <TBD> | <TBD> | <TBD> | <TBD> | <TBD> | LlamaCommunity | 8 | <TBD> |
| qwen3-8b-base | <TBD> | <TBD> | <TBD> | <TBD> | <TBD> | Apache2 | 8 | <TBD> |
| qwen3.6-27b-dense | <TBD> | <TBD> | <TBD> | <TBD> | <TBD> | Apache2 | 27 | <TBD> |
| gemma-4-31b | <TBD> | <TBD> | <TBD> | <TBD> | <TBD> | Apache2 | 31 | <TBD> |

## Hypothesis test outcome

Spec hypothesis: swapping the novel slice from OASST2 chat to swival
agent-traces (security audits) while keeping Wikipedia intros as the
duplicate slice may push AUC across 0.5 for at least one candidate. If
true, A2.5's "perplexity disabled at launch" recommendation can be
partially walked back and Phase A.5 can be deferred or scrapped.

Outcome (circle the branch that fired when results land):

- [ ] **Branch 1 — at least one candidate AUC > 0.5.** Hypothesis holds.
      File A2.7 PR updating A2.5's floor recommendations: re-enable the
      perplexity floor calibrated against this run's distribution. Close
      Phase A.5 (perplexity-replacement metric) as no longer needed.
- [ ] **Branch 2 — all candidates 0.5 > AUC > 0.4.** Hypothesis partially
      supported. Document the partial improvement. Phase A.5 stays
      parked with reduced urgency. Floor recommendation stays at A2.5's
      (0), documented as conservative-by-default; operators may
      experiment with positive floors against their own pilot data.
- [ ] **Branch 3 — all candidates AUC < 0.4.** Hypothesis fails. A2.5's
      conclusion is reinforced: perplexity-based novelty doesn't work
      across multiple corpus shapes for modern aligned LLMs. Phase A.5
      stays on the roadmap; agent-traces corpus retired as a hedge.

## Comparison to A2.3c and A2.4

Across the three corpus variants we have now measured each candidate
against. Novel-slice and duplicate-slice composition is the only
variable changing across rows; candidate code, paraphrase pipeline, and
corpus size (300/300/300) are held constant.

| candidate | A2.3c AUC (OASST2 / boilerplate) | A2.4 AUC (OASST2 / wiki intros) | A2.6 AUC (swival / wiki intros) |
| --- | --- | --- | --- |
| llama-3.1-8b-instruct | 0.119744 | 0.240022 | <TBD> |
| qwen3-8b-base | 0.235000 | 0.206522 | <TBD> |
| qwen3.6-27b-dense | 0.275922 | 0.264117 | <TBD> |
| gemma-4-31b | 0.054500 | 0.184867 | <TBD> |

## Next-step recommendation

Selected branch: <TBD — circle one of the three above when results land>.

Concrete actions, conditional on branch:

- **If Branch 1 fires:** open A2.7 PR retitled "Re-enable perplexity
  floor against swival-calibrated distribution"; recalibrate floor via
  `scripts/operator/calibrate-from-hf.sh` against the winning candidate;
  update `docs/operator/env-reference.md` defaults; mark Phase A.5
  closed in the roadmap.
- **If Branch 2 fires:** annotate A2.5's recommendation as
  "conservative-by-default, operator-overridable"; leave Phase A.5 on
  the roadmap with reduced priority; no production-default changes.
- **If Branch 3 fires:** reinforce A2.5's recommendation; keep
  perplexity floor at 0 for pilot launch; promote Phase A.5
  (perplexity-replacement metric) to the next active slice.
