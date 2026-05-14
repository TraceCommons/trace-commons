# A2.7 Perplexity Floor Update — Design (Pilot Recalibration After A2.6)

Date: 2026-05-14
Status: **DRAFT — pending A2.6 outcome confirmation.**
Owner: Trace Commons / Datasets lane
Predecessors:
- `2026-05-14-gate-floor-recalibration-design.md` (A2.5 — recommendation this spec partially walks back)
- `2026-05-14-agent-traces-bakeoff-design.md` (A2.6 — the bake-off whose outcome triggers this spec)
- `2026-05-14-gate-floor-recalibration-findings.md` (A2.5 findings report)

## Activation condition

> **TODO: activate only if A2.6 reports AUC > 0.5 for at least one candidate model.**
>
> A2.6 is in flight on a Lambda H100 at spec time. This spec is pre-drafted so
> we can file it within minutes of the bake-off completing if (and only if) the
> hypothesis holds. If A2.6 keeps all candidates below AUC 0.5, file the
> companion Phase A.5 spec (`2026-05-14-a5-perplexity-replacement-design.md`)
> instead and leave this one as `DRAFT — superseded`.
>
> Until A2.6 results are recorded in `docs/superpowers/reports/`, this spec is
> not operative. Do not implement against it; do not link from the roadmap as
> active.

## Motivation

A2.6 tested the hypothesis that A2.3c + A2.4's "perplexity AUC < 0.5 on every
candidate" finding was a corpus-design artifact: OASST2 conversational
reasoning is heavily in-training-distribution for every modern aligned LLM,
so the "novel" slice was less surprising than the "duplicate" slice. A2.6
swapped the novel slice to `jedisct1/agent-traces-swival` (OSS security-audit
traces) — a much-less-in-distribution shape.

**Assuming A2.6 confirms the hypothesis** (AUC > 0.5 on at least one
candidate), the perplexity gate is not structurally broken. A2.5's
recommendation of "perplexity floor at 0 (disabled) at pilot launch" was a
correct response to A2.3c + A2.4's data but is no longer the right pilot-
launch posture. It should be partially walked back:

- The perplexity gate works with the right corpus shape.
- Pilot traffic is expected to resemble agent-traces (multi-turn tool-using
  sessions, structured findings, code-adjacent reasoning) far more than it
  resembles OASST2 chat.
- Therefore the pilot-launch perplexity floor should be re-enabled at a value
  calibrated against A2.6's measured distribution rather than left at 0.

Phase A.5 (perplexity-replacement metric design) was parked in A2.5 against
the possibility that no corpus shape would make perplexity discriminate. A2.6
removes that possibility. **Phase A.5 closes** unless pilot data later
contradicts A2.6.

## Goal

1. Recalibrate `TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS` for pilot launch
   to a non-zero value derived from A2.6's measured perplexity distribution
   on the winning candidate.
2. Document the operator procedure for the partial re-enable — calibration
   reference values, smoke-check expectations, drift-detection guidance.
3. Update Phase A.5's roadmap entry from "parked, design pending" to
   "closed; superseded by A2.6 + A2.7."
4. Preserve A2.5's findings record as history. A2.5 is not retracted; A2.7
   is the operative recommendation going forward.

## Non-goals

- **No code changes.** The gate-service binary already reads the three
  floors; A2.7 changes only the *recommended values* and the operator-facing
  documentation.
- **No new bake-off.** A2.6's data is the calibration input.
- **No re-litigation of model choice.** The winning candidate from A2.6 is
  the operative pick. Whether that's Qwen3-8B-Base, Llama-3.1-8B-Instruct,
  Qwen 3.6 27B, or Gemma 4 31B falls out of A2.6's report; A2.7 inherits
  the answer.
- **No invalidation of A2.5's tail-fraction or novelty-floor decisions.**
  Those remain unchanged. Only the perplexity floor moves.

## Decisions baked in (conditional on A2.6 outcome)

| Decision | Value | Rationale |
|----------|-------|-----------|
| `PERPLEXITY_FLOOR_MICROS` for pilot launch | **Non-zero, TBD from A2.6 distribution.** Placeholder formula: *the winning candidate's mean perplexity at the 30th percentile of A2.6's novel slice*, expressed in micros (perplexity × 1e6 if perplexity is stored as a fractional logprob ratio, or the equivalent unit the binary already consumes). The 30th percentile is conservative — it admits the bottom-third novel-trace scorers while still rejecting boilerplate that A2.6 demonstrated lies below the novel distribution. | A2.6's data lets us read a calibrated value rather than guess. |
| `TAIL_FRACTION_FLOOR_MICROS` for pilot launch | **0 at launch, calibrate post-first-1000-pilot-traces** — unchanged from A2.5 | A2.6 doesn't change the tail-fraction story; that signal still wants real pilot data to calibrate. |
| `NOVELTY_FLOOR_MICROS` for pilot launch | **500000** (cosine novelty 0.5) — unchanged from A2.5 | Embedder + vector-index path is independent of the perplexity finding. |
| Winning candidate model | Inherit from A2.6 report | A2.7 doesn't re-pick; it consumes A2.6's answer. |
| Phase A.5 status | **Closed** (perplexity-replacement work no longer needed) | A2.6 removed the trigger that put Phase A.5 on the roadmap. |

## Model choice implications

The winning candidate from A2.6 changes what the perplexity gate *measures*
in subtle but important ways. Both plausible outcomes need an operator note:

### Scenario A: an instruct/aligned model wins (Llama-3.1-8B-Instruct, Qwen 3.6 27B Dense if instruction-tuned, etc.)

- The gate is measuring "surprise under an aligned-model prior." Aligned
  models have stronger priors over chat-shaped and tool-use-shaped output;
  novelty in those domains shows up as elevated perplexity.
- Operator implication: agent-traces that look very RLHF-conformant (polite
  hedging, "I'll help you with that" preambles) will score *low* perplexity
  even if their content is novel. Calibration should expect contributors who
  format their reasoning as conversational chat to fall closer to the floor
  than contributors who post raw structured findings.
- Drift-detection: if pilot traffic shifts toward chat-shaped traces, the
  floor will reject contributor-grade work. Add a smoke check that compares
  monthly mean perplexity against A2.6's calibration baseline.

### Scenario B: a base (non-instruct) model wins (Qwen3-8B-Base, Gemma 4 31B Base)

- The gate is measuring "surprise under a pretraining prior." Base models
  have weaker conversational priors; novelty is dominated by lexical and
  topical rarity rather than format conformance.
- Operator implication: structured-finding traces and chat-shaped traces are
  evaluated more uniformly. Contributors who paraphrase well-known content
  in unusual formats will not get an unfair pass.
- Drift-detection: less sensitive to format shift, more sensitive to topical
  shift. If pilot traffic concentrates on a single technical domain, the
  floor's effective discrimination drops. Smoke check should sample
  perplexity by trace-source-tag and watch for per-source mean collapse.

A2.7's operator runbook must include both notes; only the one matching A2.6's
winner becomes the operative guidance, but both are recorded so a future
model swap doesn't lose context.

## Deliverables

A2.7 is documentation-only. No code change ships.

1. **A2.7a — findings + decision note.** A
   `docs/superpowers/reports/2026-05-14-a27-perplexity-floor-update.md` that
   records: A2.6's measured per-candidate distribution, the calibration math
   for the chosen floor value, the operator-facing impact of the change.
   Companion to A2.5's findings report; the two together tell the full
   "we thought it was broken, then we found it wasn't" arc.
2. **A2.7b — calibration runbook update.** Rewrite the relevant section of
   `docs/operator/calibration.md` to use A2.7's floor value rather than 0.
   Keep A2.5's reasoning trail (one paragraph) so an operator reading
   forward in time understands why the value changed.
3. **A2.7c — env-reference update.** Note in `docs/operator/env-reference.md`
   that `TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS` defaults to A2.7's
   value with a pointer to this spec.
4. **A2.7d — deployment.md update.** Reflect the non-zero floor in the
   recommended deployment configuration; remove the "disabled at launch"
   language A2.5 added.
5. **A2.7e — roadmap entry.** Add an A2.7 status line under Phase A in
   `docs/trace-commons-roadmap.md`. Update Phase A.5 from "parked, design
   pending" to "closed; superseded by A2.6 + A2.7."

## Open questions

1. **What is the exact floor value?** Depends on A2.6's per-candidate
   distribution and on which candidate wins. The placeholder is "30th
   percentile of novel-slice perplexity for the winning candidate."
   Resolved when A2.6's report is committed.

2. **Should the 30th-percentile choice be revisited?** A lower percentile
   (e.g., 10th) admits more contributor diversity at the cost of letting
   more duplicates through. A higher percentile (e.g., 50th) is stricter.
   **Recommendation:** 30th for pilot launch — conservative on the "don't
   reject contributors" side. Revisit after the first 1000 pilot traces let
   us measure false-rejection rate against operator-labeled ground truth.

3. **Should we re-run A2.6 against a fresher dataset before committing?**
   `jedisct1/agent-traces-swival` may have been scraped into training
   corpora since publication. If A2.6's winner has its training cutoff
   after swival's upload date, the AUC > 0.5 result is suspect. **Recommendation:**
   include a dataset-recency check in the A2.7 findings; if it fails,
   A2.7 itself is suspect and A2.5 stands.

4. **What replaces Phase A.5 on the roadmap once closed?** Pilot-bootstrap
   harness work (A.6) and the embedder-path validation are the obvious
   next slots. **Recommendation:** Phase A.5's old slot becomes
   "pilot-data calibration of tail-fraction floor" — the work A2.5 already
   queued, now elevated because it's the only remaining floor uncalibrated.

## Trade-offs explicitly accepted

- **A2.5 stands as history; A2.7 is the operative recommendation going
  forward.** An operator reading from oldest spec forward sees the full
  arc: A2 designed the floor, A2.3c/A2.4 showed it inverted on the test
  corpus, A2.5 disabled it for pilot, A2.6 showed the inversion was a
  corpus artifact, A2.7 re-enables with a calibrated value. We do not
  retract A2.5; the operator runbook says "see A2.5 for context, see A2.7
  for current values."
- **Phase A.5 closes on a single bake-off result.** A2.6 is one experiment.
  If pilot data later contradicts A2.6 (perplexity floor rejects
  contributor-grade traces at unacceptable rates), Phase A.5 reopens.
  This is acceptable risk; the alternative is keeping perplexity disabled
  in the face of evidence it works.
- **Floor value is conservative by design.** 30th-percentile is biased
  toward contributor acceptance over duplicate rejection. We'd rather
  let some duplicates through and catch them at human review than reject
  novel contributor work at the gate.

## Out of scope (recorded so we don't accidentally re-open it)

- New perplexity metric design (contrastive, per-token rarity, learned).
  Closed with Phase A.5.
- Gate-service trait changes.
- Bake-off binary updates.
- Model retraining or fine-tuning the candidate models.
- Re-running A2.4 (Wikipedia duplicate slice with OASST2 novel slice). A2.6
  superseded that comparison's relevance.
