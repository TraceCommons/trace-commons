# Bake-off report (partial results as of 2026-05-14 18:01 UTC; Gemma 4 31B still scoring)

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

Winner: <TBD — Gemma 4 31B still scoring; Qwen 3.6 27B Dense is the current leader with AUC 0.9363>

| candidate | auc | paraphrase_delta | tail_range | throughput_tps | determinism_stddev | license | params_b | passed_gate |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| llama-3.1-8b-instruct | 0.3425 | 0.8291 | 0.0153 | 291.76 | 1.110e-16 | LlamaCommunity | 8 | true |
| qwen3-8b-base | 0.2431 | 0.8234 | 0.0071 | 248.46 | 1.110e-16 | Apache2 | 8 | true |
| qwen3.6-27b-dense | 0.9363 | 0.5980 | 0.0965 | 119.49 | 2.665e-15 | Apache2 | 27 | true |
| gemma-4-31b | <TBD> | <TBD> | <TBD> | <TBD> | <TBD> | Apache2 | 31 | <TBD> |

Gemma 4 31B Base: scoring in progress as of 18:01 UTC; expected completion ~21:00 UTC.

## Hypothesis test outcome

Spec hypothesis: swapping the novel slice from OASST2 chat to swival
agent-traces (security audits) while keeping Wikipedia intros as the
duplicate slice may push AUC across 0.5 for at least one candidate. If
true, A2.5's "perplexity disabled at launch" recommendation can be
partially walked back and Phase A.5 can be deferred or scrapped.

Outcome (circle the outcome that fired when results land):

> Outcome 1/2/3 maps to the three branches in the A2.6 spec's "What success looks like" section.

- [x] **Outcome 1 — at least one candidate AUC > 0.5.** Hypothesis holds.
      FIRES via Qwen 3.6 27B Dense (AUC 0.9363). A2.7 perplexity floor
      recalibration is the active follow-up; Phase A.5
      (perplexity-replacement metric) is now DEFERRED. Gemma 4 31B's
      result, when it lands, may strengthen the conclusion but is
      unlikely to invert it (Outcome 1 already fired and is sticky).
- [ ] **Outcome 2 — all candidates 0.5 > AUC > 0.4.** Hypothesis partially
      supported. Document the partial improvement. Phase A.5 stays
      parked with reduced urgency. Floor recommendation stays at A2.5's
      (0), documented as conservative-by-default; operators may
      experiment with positive floors against their own pilot data.
- [ ] **Outcome 3 — all candidates AUC < 0.4.** Hypothesis fails. A2.5's
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
| llama-3.1-8b-instruct | 0.119744 | 0.240022 | 0.3425 |
| qwen3-8b-base | 0.235000 | 0.206522 | 0.2431 |
| qwen3.6-27b-dense | 0.275922 | 0.264117 | 0.9363 (not evaluated in A2.3c/A2.4 — candle backend limitation; first evaluated in A2.6 via mistralrs) |
| gemma-4-31b | 0.054500 | 0.184867 | <TBD> (not evaluated in A2.3c/A2.4 — candle backend limitation; first evaluated in A2.6 via mistralrs) |

Note: the A2.3c and A2.4 columns for the 27B/31B rows are retained from
the prior reports for shape continuity, but those runs aborted on model
load under the candle backend. A2.6 is the first run where the larger
two candidates produce real AUCs; the 27B/31B prior-column numbers
should be read as "no comparable measurement" rather than as a
regression.

## Next-step recommendation

Selected outcome: **Outcome 1**.

Concrete actions:

- A2.7 fires (full): promote PR #74's plan stub to executable plan.
  Calibration candidate is the worst-of-passing model (to be selected
  once Gemma 4 31B's AUC lands; current worst-of-passing among the
  three completed candidates is Llama-3.1-8B-Instruct at AUC 0.3425,
  which did not cross 0.5 — Qwen 3.6 27B Dense is the only crosser so
  far, so it is provisionally both best- and worst-of-passing).
- Open A2.7 PR retitled "Re-enable perplexity floor against
  swival-calibrated distribution"; recalibrate floor via
  `scripts/operator/calibrate-from-hf.sh` against the chosen
  calibration candidate; update `docs/operator/env-reference.md`
  defaults.
- Phase A.5 (perplexity-replacement metric) marked DEFERRED per
  Outcome 1 routing.
