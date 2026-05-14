# A2.7 Perplexity Floor Update — Design (Pilot Recalibration After A2.6)

Date: 2026-05-14
Status: **DRAFT — pending A2.6 outcome confirmation.**
Owner: Trace Commons / Datasets lane
Predecessors:
- `2026-05-14-gate-floor-recalibration-design.md` (A2.5 — recommendation this spec partially walks back)
- `2026-05-14-agent-traces-bakeoff-design.md` (A2.6 — the bake-off whose outcome triggers this spec)
- `2026-05-14-gate-floor-recalibration-findings.md` (A2.5 findings report)
- `2026-05-14-a5-perplexity-replacement-design.md` (Phase A.5 — the alternative path if A2.6 confirms A2.5)

## Activation condition

> A2.6 is in flight on a Lambda H100 at spec time. This spec is pre-drafted
> so we can file it within minutes of the bake-off completing. Whether it
> activates, partially activates, or is superseded depends on A2.6's
> measured AUC. The branching is fully resolved by the `Trigger criteria`
> section below; once A2.6's report lands in
> `docs/superpowers/reports/`, follow that table — no fresh judgement call
> required.
>
> Until A2.6 results are recorded, this spec is not operative. Do not
> implement against it; do not link from the roadmap as active.

## Trigger criteria

A2.6's success-criteria section names three outcome branches. Each maps
to exactly one A2.7 path. The decision is mechanical:

| A2.6 outcome (per `What success looks like`) [^a26-corpus] | A2.7 path | Spec status header becomes |
|----------------------------------------------|-----------|----------------------------|
| **Outcome 1.** At least one candidate AUC > 0.5. | **Fires (full).** Recalibrate `TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS` to a non-zero value via the recipe in `Outcome 1 procedure`. Update operator docs. Close Phase A.5. | `ACTIVE — calibration recipe in §Outcome 1.` |
| **Outcome 2.** All candidates 0.5 > AUC > 0.4. | **Fires (documentation-only).** Floor stays at `0`. A2.5's runbook prose updated to record that A2.6 partially supported the hypothesis. Phase A.5 stays parked with reduced urgency (matches A2.6's own success-criteria language). | `ACTIVE — docs-only; see §Outcome 2.` |
| **Outcome 3.** All candidates AUC < 0.4. | **Does NOT fire.** A2.5 stands as the operative pilot-launch recommendation. Phase A.5 activates per its own spec (`2026-05-14-a5-perplexity-replacement-design.md`). | `SUPERSEDED — Phase A.5 activated instead; see §Outcome 3.` |

[^a26-corpus]: A2.6's corpus is fixed as agent-traces (swival) novel + Wikipedia duplicate. The AUC thresholds above are measured against that corpus shape.

The 0.4 / 0.5 boundaries are inherited verbatim from A2.6's spec. A2.7
does not move them.

If A2.6's report flags the **dataset-recency risk** from A2.6 Open Question 1
(swival predates a winning candidate's training cutoff), the outcome is
demoted one tier (Outcome 1 → 2, Outcome 2 → 3) before applying the table
above. A flagged dataset-recency failure on the *only* candidate above
0.5 means Outcome 1 cannot be claimed.

## Motivation

A2.6 tests the hypothesis that A2.3c + A2.4's "perplexity AUC < 0.5 on
every candidate" finding was a corpus-design artifact: OASST2
conversational reasoning is heavily in-training-distribution for every
modern aligned LLM, so the "novel" slice was less surprising than the
"duplicate" slice. A2.6 swaps the novel slice to
`jedisct1/agent-traces-swival` (OSS security-audit traces) — a much-less-
in-distribution shape.

**Under Outcome 1**, the perplexity gate is not structurally broken.
A2.5's recommendation of "perplexity floor at 0 (disabled) at pilot
launch" was a correct response to A2.3c + A2.4's data but is no longer
the right pilot-launch posture. It should be partially walked back:

- The perplexity gate works with the right corpus shape.
- Pilot traffic is expected to resemble agent-traces (multi-turn
  tool-using sessions, structured findings, code-adjacent reasoning) far
  more than it resembles OASST2 chat.
- The pilot-launch perplexity floor should be re-enabled at a value
  calibrated against A2.6's measured distribution rather than left at 0.

**Under Outcome 2**, the gate is weakly discriminating — better than
A2.5 measured, but not enough to justify a positive floor at pilot
launch. The walkback is doc-only: A2.5's "conservative-by-default"
language is hardened, and operators are told they may experiment with
positive floors against their own data.

**Under Outcome 3**, A2.5's finding generalizes across two distinct
corpus shapes (OASST2 + Wikipedia, agent-traces + Wikipedia). The
perplexity-disabled posture is correct and Phase A.5 takes the next-step
slot.

## Goal

1. Provide a single, executable specification covering all three A2.6
   outcomes so the post-result work is mechanical.
2. Under Outcome 1, recalibrate `TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS`
   for pilot launch using the procedure in §Outcome 1 procedure.
3. Under Outcome 2, update A2.5's operator-facing prose without changing
   any floor value.
4. Under Outcome 3, mark A2.7 superseded and hand off to Phase A.5.
5. Preserve A2.5's findings record as history in every case. A2.5 is
   never retracted.

## Non-goals

- **No code changes** under any outcome. The gate-service binary already
  reads the three floors; A2.7 only changes *recommended values* and
  *operator-facing documentation*.
- **No new bake-off.** A2.6's data is the calibration input.
- **No re-litigation of model choice.** Under Outcome 1, the winning
  candidate from A2.6 dictates the calibration; A2.7 inherits the
  answer. The "Model choice implications" section records both base-
  model and instruct-model implications so a future swap doesn't lose
  context.
- **No invalidation of A2.5's tail-fraction or novelty-floor decisions.**
  Those remain unchanged under every outcome. Only the perplexity floor
  is in scope, and only under Outcome 1 does its value move.

## Decisions baked in (conditional on A2.6 outcome)

| Decision | Outcome 1 | Outcome 2 | Outcome 3 |
|----------|-----------|-----------|-----------|
| `PERPLEXITY_FLOOR_MICROS` for pilot launch | Non-zero; see §Outcome 1 procedure. | `0` (unchanged). | `0` (unchanged). |
| `TAIL_FRACTION_FLOOR_MICROS` for pilot launch | `0` at launch, calibrate post-first-1000-pilot-traces — unchanged from A2.5. | Same. | Same. |
| `NOVELTY_FLOOR_MICROS` for pilot launch | `500000` (cosine novelty 0.5) — unchanged from A2.5. | Same. | Same. |
| Winning candidate model | Inherit from A2.6 report (used only for calibration math). | n/a. | n/a. |
| Phase A.5 status | **Closed** — perplexity-replacement work no longer needed. | **Parked with reduced urgency** (matches A2.6's success-criteria language verbatim). | **Active** — Phase A.5 spec fires per its own activation condition. |
| A2.7 spec status header | `ACTIVE — calibration recipe in §Outcome 1.` | `ACTIVE — docs-only; see §Outcome 2.` | `SUPERSEDED — Phase A.5 activated instead.` |

## Outcome 1 procedure (AUC > 0.5)

This is the only branch that produces a non-zero floor value.

### Inputs

From A2.6's committed report
(`docs/superpowers/reports/2026-05-14-model-bakeoff-result-a26.{json,md}`):

- Per-candidate AUC on the agent-traces novel slice vs. Wikipedia
  duplicate slice.
- For each candidate, the per-row `aggregate_perplexity_micros` values
  for the novel slice and the duplicate slice (the bake-off binary
  already writes these per A2.3c/A2.4 conventions).
- The dataset-recency check from A2.6 Open Question 1.

### Step 1: pick the calibration candidate

A2.6's report names "winning candidate" by AUC. A2.7 uses a deliberately
*more conservative* pick for floor calibration:

> **The calibration candidate is the lowest-AUC model among those that
> cleared AUC > 0.5.** Not the best-AUC model.

Rationale: the gate runs in production against a single deployed model.
Picking the worst-of-passing makes the floor robust to model swap — if
the operator later promotes a smaller / cheaper candidate that also
cleared 0.5, the floor still discriminates. If only one candidate
cleared 0.5, that candidate is the calibration candidate by default.

This contradicts the placeholder language ("winning candidate") in
earlier drafts of this spec; the placeholder is superseded by this step.

### Step 2: derive the candidate floor value

Two methods. Run both; if they disagree by more than 2x, fall through to
Open Question 1.

**Method A — Youden's J on the labeled split.** A2.6's bake-off produces
labeled novel/duplicate rows. For the calibration candidate, sweep
candidate floor values across the observed `aggregate_perplexity_micros`
range. At each candidate floor, compute:

```
J = TPR(novel admitted) - FPR(duplicate admitted)
  = P(perplexity >= floor | novel) - P(perplexity >= floor | duplicate)
```

The Youden-optimal floor is the value of `floor` that maximizes J. This
is the standard ROC-derived operating point and aligns with the AUC
measurement A2.6 already reports.

**Method B — novel-slice low-tail percentile.** Compute the 10th
percentile of `aggregate_perplexity_micros` over the agent-traces novel
slice for the calibration candidate. The 10th percentile is the
contributor-friendly choice: it admits the bottom-decile novel scorers
while still rejecting duplicate content that lies below the novel
distribution. (10th, not 30th as earlier drafts suggested — see
§Open questions Q2.)

**Reconcile.** If Method A and Method B agree within 2x, take their
geometric mean. If they diverge by more than 2x, the labeled-AUC signal
and the percentile signal are telling different stories — file under
§Open questions Q1 rather than picking arbitrarily.

### Step 3: apply a headroom margin

Pilot traffic is not bake-off traffic. To avoid pilot-day false-rejects
from distribution drift, apply a conservative downward margin:

> **Final floor = floor-from-Step-2 × 0.5.**

This 2x headroom is the same order-of-magnitude conservatism A2.5
applied when setting `NOVELTY_FLOOR_MICROS=500000` against an idealized
0.7 cosine novelty target. The margin is reviewed (not necessarily
re-applied) after the first 1000 pilot traces.

### Step 4: encode as micros

`aggregate_perplexity_micros` in the bake-off CSVs is already in the
unit the gate-service binary consumes (per `micros_to_f64` in
`crates/trace-commons-server/src/bin/gate_calibrate/run_candidate_eval.rs`:142
— micros = perplexity × 1e6). No further unit conversion is required.
The Step-3 value is the env var value, rounded down to the nearest
integer:

```
TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS=<floor(final_floor)>
```

### Step 5: validate against A2.5's invariants

- Confirm `>= 0`. (Negative perplexity micros is malformed.)
- Confirm the deployment-runbook invariant "at least one of the three
  floors must be positive" is still satisfied (it is, by
  `NOVELTY_FLOOR_MICROS=500000` regardless).
- Confirm the value does not exceed the calibration candidate's *median*
  novel-slice perplexity. A floor above the median rejects > 50% of
  contributor-grade novel content under bake-off distribution and is
  prima facie too tight; if Step 3 produces such a value, treat as
  Outcome-2 instead of Outcome-1 and stop.

### Step 6: record

The Outcome-1 deliverable set (next section) carries the actual numbers
forward into the operator docs and findings report.

## Outcome 1 deliverables

A2.7 is documentation-only. No code change ships. Under Outcome 1:

1. **A2.7a — findings + decision note.** A
   `docs/superpowers/reports/2026-05-14-a27-perplexity-floor-update.md`
   that records: A2.6's measured per-candidate distribution, the
   §Outcome 1 procedure as actually executed (which candidate, Method
   A/B values, margin applied), the resulting floor, and the operator-
   facing impact of the change. Companion to A2.5's findings report.
2. **A2.7b — calibration runbook update.** Rewrite the relevant section
   of `docs/operator/calibration.md` (the "A2.5 pilot-launch floor
   recommendations" table) to use A2.7's floor value rather than `0`.
   Keep A2.5's reasoning trail (one paragraph) so an operator reading
   forward in time understands why the value changed.
3. **A2.7c — env-reference update.** Note in
   `docs/operator/env-reference.md` that
   `TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS` now defaults to A2.7's
   value with a pointer to this spec and its findings report.
4. **A2.7d — deployment.md update.** Reflect the non-zero floor in
   `docs/operator/deployment.md`'s recommended deployment configuration;
   remove the "disabled at launch" language A2.5 added.
5. **A2.7e — roadmap entry.** Add an A2.7 status line under Phase A in
   `docs/trace-commons-roadmap.md`. Update Phase A.5 from "parked,
   design pending" to "closed; superseded by A2.6 + A2.7."

## Outcome 2 deliverables (partial improvement; floor stays at 0)

A2.7 is documentation-only, no floor value changes. Under Outcome 2:

1. **A2.7a-partial — findings note.** Short report at
   `docs/superpowers/reports/2026-05-14-a27-perplexity-floor-update.md`
   recording A2.6's per-candidate AUCs and the decision to hold the
   floor at `0`. One paragraph each: what A2.6 changed, why it's not
   enough to justify a positive floor at pilot launch, what would
   change the answer.
2. **A2.7b-partial — calibration.md commentary update.** In the "A2.5
   pilot-launch floor recommendations" table, harden the
   `PERPLEXITY_FLOOR_MICROS=0` row's "Notes" column to read
   "conservative-by-default; A2.6 measured 0.4 < AUC < 0.5 on
   agent-traces — better than A2.3c/A2.4 but not enough to justify a
   positive floor without pilot data." Add a one-sentence operator note
   that operators may experiment with positive floors against their own
   data once they have it.
3. **A2.7c-partial — env-reference.md commentary update.** Update the
   `TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS` row's free-text column
   to point at both A2.5 and A2.6 findings, not just A2.5.
4. **A2.7d-partial — roadmap entry.** Add an A2.7 status line. Phase
   A.5's roadmap entry stays "parked" but its urgency note is updated to
   reflect A2.6's partial improvement.

No change to `deployment.md`'s active configuration under Outcome 2.

## Outcome 3 closure (AUC < 0.4 across all candidates)

A2.7 does not fire. The mechanical handoff:

1. This spec's status header is updated to
   `SUPERSEDED — Phase A.5 activated instead.` No deliverables ship
   under the A2.7 name.
2. The Phase A.5 spec
   (`2026-05-14-a5-perplexity-replacement-design.md`) is moved from
   `DRAFT — pending A2.6 outcome confirmation` to `ACTIVE` per its own
   activation condition.
3. A2.5 remains the operative pilot-launch recommendation unchanged.
4. The roadmap entry under Phase A reflects A2.7 closure and Phase A.5
   activation in a single bullet.

No findings report is filed under the A2.7 name; A2.6's report is the
record. Phase A.5's first deliverable will reference A2.6 + A2.7-as-
superseded for its own motivation.

## Model choice implications

The winning candidate from A2.6 changes what the perplexity gate
*measures* in subtle but important ways. Both plausible Outcome-1
winners need an operator note. Recorded here regardless of which fires
so a future model swap doesn't lose context.

### Scenario A: an instruct/aligned model wins

(Llama-3.1-8B-Instruct, Qwen 3.6 27B Dense if instruction-tuned, etc.)

- The gate is measuring "surprise under an aligned-model prior."
  Aligned models have stronger priors over chat-shaped and tool-use-
  shaped output; novelty in those domains shows up as elevated
  perplexity.
- Operator implication: agent-traces that look very RLHF-conformant
  (polite hedging, "I'll help you with that" preambles) will score
  *low* perplexity even if their content is novel. Calibration should
  expect contributors who format their reasoning as conversational chat
  to fall closer to the floor than contributors who post raw structured
  findings.
- Drift-detection: if pilot traffic shifts toward chat-shaped traces,
  the floor will reject contributor-grade work. Add a smoke check that
  compares monthly mean perplexity against A2.6's calibration baseline.

### Scenario B: a base (non-instruct) model wins

(Qwen3-8B-Base, Gemma 4 31B Base)

- The gate is measuring "surprise under a pretraining prior." Base
  models have weaker conversational priors; novelty is dominated by
  lexical and topical rarity rather than format conformance.
- Operator implication: structured-finding traces and chat-shaped
  traces are evaluated more uniformly. Contributors who paraphrase
  well-known content in unusual formats will not get an unfair pass.
- Drift-detection: less sensitive to format shift, more sensitive to
  topical shift. If pilot traffic concentrates on a single technical
  domain, the floor's effective discrimination drops. Smoke check
  should sample perplexity by trace-source-tag and watch for per-source
  mean collapse.

A2.7's operator runbook (Outcome 1) must include both notes; only the
one matching A2.6's calibration candidate becomes the operative
guidance, but both are recorded.

## Open questions

1. **Method A and Method B disagree by more than 2x.** §Outcome 1 Step 2
   falls through here. A2.5 did not commit to a tiebreaker — A2.5's
   calibration runbook uses a *single* percentile method against an
   unlabeled bootstrap distribution, not a labeled novel/duplicate
   split, so the question doesn't exist at the A2.5 level. **Resolution
   path:** if this fires, defer the floor decision to the human
   operator in the A2.7a findings report and document both candidate
   values. Do not auto-pick. The pilot can launch with `0` while the
   investigation runs (matches Outcome 2).

2. **Is 10th-percentile the right Method-B percentile?** Earlier drafts
   of this spec proposed 30th percentile. A2.5's runbook uses 70th
   percentile *of bootstrap (mixed) distribution targeting 30% pass
   rate*; that's a different math against a different distribution and
   is not directly transferable. **Resolution path:** 10th percentile
   for pilot launch (contributor-friendly + Step-3 margin already
   doubles the conservatism). Revisit after first 1000 pilot traces let
   us measure false-rejection rate against operator-labeled ground
   truth. A2.5 did not commit to a value here, so this remains an open
   question rather than an inherited decision.

3. **Should the Step-3 headroom margin (0.5×) be revisited?** A2.5
   applied no explicit headroom margin to `NOVELTY_FLOOR_MICROS=500000`;
   that value targeted a 0.5 cosine novelty directly. **Resolution
   path:** 0.5× for pilot launch, revisit at the same checkpoint as
   percentile choice. Flagged so a future operator doesn't read 0.5×
   as an inherited A2.5 invariant.

4. **Should we re-run A2.6 against a fresher dataset before committing?**
   `jedisct1/agent-traces-swival` may have been scraped into training
   corpora since publication. A2.6 Open Question 1 already commits to a
   dataset-recency pre-flight check. **Resolution path:** if A2.6's
   report flags a recency failure on the calibration candidate, demote
   the outcome one tier per the `Trigger criteria` note (Outcome 1 → 2,
   Outcome 2 → 3).

5. **What replaces Phase A.5 on the roadmap once closed?** Pilot-
   bootstrap harness work (A.6) and the embedder-path validation are
   the obvious next slots. **Resolution path:** Phase A.5's old slot
   becomes "pilot-data calibration of tail-fraction floor" — the work
   A2.5 already queued, now elevated because it's the only remaining
   floor uncalibrated. Applies only to Outcome 1.

## Trade-offs explicitly accepted

- **A2.5 stands as history; A2.7 (under Outcome 1 or 2) is the operative
  recommendation going forward.** An operator reading from oldest spec
  forward sees the full arc: A2 designed the floor, A2.3c/A2.4 showed
  it inverted on the test corpus, A2.5 disabled it for pilot, A2.6
  retested with a better-shaped corpus, A2.7 re-enables (Outcome 1) or
  hardens the disabled-by-default posture (Outcome 2). We do not
  retract A2.5; the operator runbook says "see A2.5 for context, see
  A2.7 for current values."
- **Calibration candidate is worst-of-passing, not best.** Under
  Outcome 1 the floor is conservative against future model swap rather
  than tuned to the strongest measured AUC. Accepts a tighter pilot-day
  floor in exchange for robustness.
- **Phase A.5 closes on a single bake-off result under Outcome 1.**
  A2.6 is one experiment. If pilot data later contradicts A2.6
  (perplexity floor rejects contributor-grade traces at unacceptable
  rates), Phase A.5 reopens. This is acceptable risk; the alternative
  is keeping perplexity disabled in the face of evidence it works.
- **Floor value is conservative by design.** Method B's 10th percentile
  plus Step-3's 0.5× margin biases toward contributor acceptance over
  duplicate rejection. We'd rather let some duplicates through and catch
  them at human review than reject novel contributor work at the gate.
- **Outcome 2 changes no runtime behavior.** All risk is documentation
  drift; the runtime stays on A2.5's settings.

## Out of scope (recorded so we don't accidentally re-open it)

- New perplexity metric design (contrastive, per-token rarity, learned).
  Closed under Outcome 1 with Phase A.5. Activated under Outcome 3 via
  Phase A.5's own spec, not here.
- Gate-service trait changes.
- Bake-off binary updates.
- Model retraining or fine-tuning the candidate models.
- Re-running A2.4 (Wikipedia duplicate slice with OASST2 novel slice).
  A2.6 superseded that comparison's relevance.
- Any change to `TAIL_FRACTION_FLOOR_MICROS` or `NOVELTY_FLOOR_MICROS`
  at pilot launch under any outcome.
