# A2.7 Perplexity Floor Update Implementation Plan (STUB)

> **Status:** STUB — activated only if A2.6 final results show at least one candidate AUC > 0.5. The spec at `docs/superpowers/specs/2026-05-14-a27-perplexity-floor-update-design.md` is authoritative for the calibration math.
>
> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Re-enable the perplexity floor with a calibrated micros value derived from the A2.6 bake-off's per-candidate scoring distribution.

**Architecture:** Pure operator + config change. No new code. Updates A2.5's `TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS` from 0 to a specific value, with rationale documented and the spec marked "fired."

**Tech Stack:** Operator runbook + env-var config + docs.

## 1. What needs to happen if AUC > 0.5 (full fire)

This is the primary path. The A2.6 bake-off produced at least one candidate
that cleanly separates novel-slice from duplicate-slice traces, so the
perplexity scorer is fit for purpose and the floor can be turned back on.

### Slice A2.7a — Extract per-trace scores for the calibration candidate

- [ ] Identify the calibration candidate per spec § Outcome 1: the
  **worst-of-passing** candidate among the candidates with AUC > 0.5.
  Worst-of-passing = lowest AUC among the passing set, so the floor is
  set against the weakest model that still cleared the bar. This protects
  against operator-future-swap to a degraded candidate.
- [ ] Pull the per-trace perplexity scores from the A2.6 report's
  `metrics.perplexity` column for that candidate. Report path:
  `docs/superpowers/reports/2026-05-14-model-bakeoff-result-a26.json`.
- [ ] Confirm both the novel-slice and duplicate-slice rows are present
  and labeled before computing any statistics.

### Slice A2.7b — Compute the floor micros value

- [ ] Compute the Youden's-J optimum cutoff on the candidate's
  novel-vs-duplicate ROC curve. This is the spec's "high-precision"
  anchor.
- [ ] Compute the 10th-percentile perplexity score on the **novel slice
  only**. This is the spec's "low-recall-loss" anchor.
- [ ] Take the **geometric mean** of the two anchors. Per spec §
  Outcome 1 the geometric mean is the chosen central tendency because
  the two anchors live on log-scale-comparable axes.
- [ ] Apply the **0.5× headroom margin** — divide the geometric mean by
  two. Per spec, the half-margin reflects post-A2.5 conservatism: we
  prefer to under-floor and re-tune in A2.8 over over-floor and shed
  legitimate pilot traffic in the first week.
- [ ] Convert the result to micros using the same `micros_to_f64`
  helper the perplexity scorer uses internally (so the floor unit is
  byte-identical to the comparison the gate makes at runtime).
- [ ] Record the intermediate values (Youden's-J optimum, p10,
  geometric mean, post-margin, micros) in the operator runbook for
  audit. Hash-only redaction does not apply here — these are
  calibration constants, not operator secrets.

### Slice A2.7c — Update operator docs

- [ ] Update `docs/operator/calibration.md` perplexity-floor section
  with the new value, the candidate it was calibrated against, and the
  AUC at which it was set. Strike the "set to 0 pending A2.6 outcome"
  note from A2.5.
- [ ] Update any deployment template that pins
  `TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS` (search the repo for
  the env-var name; A2.5 left it at `0` in at least one template).
- [ ] Mark the A2.7 spec status as **FIRED** with a one-line note
  pointing at the report and the PR.

### Slice A2.7d — Land the production deployment env change

- [ ] File a one-commit PR with subject matching the existing
  short-imperative convention (no `feat:` prefix). The PR baked
  value lives in the production deployment env and the
  `calibration.md` runbook, and nothing else.
- [ ] Wait for CI green + reviewer sign-off. Roll out behind the
  standard rollout-smoke required check — the perplexity-floor
  drill (if one exists) should be re-run with the new value to
  confirm the gate decision still matches expected behavior on
  fixture traces.

## 2. What needs to happen if 0.4 < AUC < 0.5 (docs-only fire)

This is the partial-credit path. No candidate cleanly clears the bar
but the best candidate is close enough that the operator chooses to
document the conservative-by-default posture rather than escalate to
Phase A.5 immediately.

### Slice A2.7-partial-a — Document the conservative-by-default reasoning

- [ ] Update `docs/operator/calibration.md` to keep
  `TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS=0` and add a one-paragraph
  explanation: "A2.6 produced AUC in the 0.4-0.5 band; the perplexity
  scorer is directionally useful but not separating-quality. The floor
  stays disabled pending either (a) a better metric from Phase A.5 or
  (b) a re-run of the bake-off on a corpus the operator has reason to
  expect will produce a higher AUC."
- [ ] Cross-link the A.5 plan stub so the operator knows Phase A.5 is
  parked-but-available.

### Slice A2.7-partial-b — Land the docs-only PR

- [ ] File a one-commit PR with only the calibration.md change and the
  spec status update ("FIRED — docs-only branch"). No env-var change,
  no code change.
- [ ] Phase A.5 stays parked but with reduced urgency. The operator
  may revisit A.5 if pilot traffic suggests the perplexity scorer is
  letting in too much duplicate-shaped content, but A.5 is not the
  default next move under this branch.

## 3. What is pre-decided

The A2.7 spec PR #58 already settled the following — do **not**
re-litigate during implementation:

- **Calibration candidate selection:** worst-of-passing among
  AUC > 0.5 candidates. Not best-of-passing, not median.
- **Floor formula:** geometric mean of Youden's-J optimum and
  10th-percentile novel-slice perplexity.
- **Headroom margin:** 0.5×. Not 0.25×, not 0.75×.
- **Unit conversion:** micros via the scorer's internal
  `micros_to_f64` helper. Not a hand-converted decimal.
- **Report JSON format:** fixed by the A2.6 report skeleton and the
  bake-off binary's `--report-out` flag. The A2.7 implementer reads
  the report; they do not change it.
- **Channel separation:** A2.5's tail-fraction floor stays where it
  is. A2.7 only adjusts the **perplexity** floor.

## 4. What is pending

Open questions the implementer must resolve at execution time:

- **Tiebreaker if multiple candidates pass:** the spec parks this.
  Worst-of-passing is well-defined for distinct AUC values; if two
  candidates tie on AUC to the precision the report emits, the
  implementer picks the one with the lower 10th-percentile
  novel-slice perplexity (more-conservative anchor) and records the
  tiebreaker in the calibration.md note.
- **Per-environment headroom adjustment** (staging vs prod): out of
  scope. A2.7 ships a single value across all environments.
  Environment-specific tuning is a hypothetical A2.8.
- **Whether the floor should also gate tail-fraction simultaneously:**
  **NO.** Tail-fraction is A2.5's separate channel and stays on its
  own env var (`TRACE_COMMONS_GATE_TAIL_FRACTION_FLOOR_MICROS`).
  Conflating the two floors is explicitly out of scope.

## 5. Decision gate

A2.7 should not start until **both** of the following hold:

- [ ] A2.6 report is in `docs/superpowers/reports/`
  (`2026-05-14-model-bakeoff-result-a26.{json,md}`).
- [ ] The "Trigger criteria" table in the A2.7 spec maps cleanly to
  outcome branch 1 (full fire, AUC > 0.5) or outcome branch 2
  (docs-only, 0.4 < AUC < 0.5). If the report shows AUC < 0.4 across
  every candidate, this plan never executes — A.5 activates instead
  (see `docs/superpowers/plans/2026-05-14-a5-perplexity-replacement.md`).

## 6. Out of scope

Anything beyond changing the floor value:

- **No new spec.** A2.7's spec is authoritative; the implementer
  follows it.
- **No new metric.** That is Phase A.5's job.
- **No new bake-off run.** A2.7 calibrates against A2.6's report; if
  the report is missing data the implementer needs, that's an A2.6
  gap, not an A2.7 gap.
- **No mid-flight re-tune.** If the operator finds the post-A2.7
  floor is wrong (too tight, too loose, or wrong-shaped against
  pilot traffic), that is a **new** recalibration plan — call it
  A2.8 — not a fix-this-plan-mid-flight amendment.
- **No code changes.** A2.7 is operator + config + docs. If the
  implementer feels the urge to refactor the scorer, the gate, or
  the bake-off binary while in the area, that urge belongs in a
  different PR.
