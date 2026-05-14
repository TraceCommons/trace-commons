# Gate Floor Recalibration Implementation Plan (A2.5 Retrofit)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Update operator-facing documentation to reflect the new gate-floor recommendations driven by the A2.3c + A2.4 bake-off findings. Perplexity floor ships at 0 (disabled) for pilot launch; novelty-embedder floor stays primary; tail-fraction floor is post-calibrated against pilot data.

**Architecture:** No code changes. Documentation-only retrofit. The existing env vars (`TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS`, `TRACE_COMMONS_GATE_TAIL_FRACTION_FLOOR_MICROS`, `TRACE_COMMONS_GATE_NOVELTY_FLOOR_MICROS`) continue to work; only the *recommended values* for pilot launch differ from the A2 deployment runbook.

**Tech Stack:** Markdown.

**Spec:** `docs/superpowers/specs/2026-05-14-gate-floor-recalibration-design.md`

---

## File Map

**New files**

| Path | Responsibility |
|------|----------------|
| `docs/superpowers/reports/2026-05-14-gate-floor-recalibration-findings.md` | Findings report: A2.3c + A2.4 data summary, the AUC-below-0.5 inversion, decision logic for the new floor recommendations. Companion to the existing A2.1 + A2.3c reports. |

**Modified files**

| Path | What changes |
|------|--------------|
| `docs/operator/calibration.md` | Rewrite Phase 0 + Phase 1 sections. Phase 0 keeps the bake-off binary description but notes the empirical finding that perplexity AUC < 0.5 across candidates; Phase 1 sets the new floor recommendations (perplexity=0, tail-fraction=0 at launch with post-pilot calibration, novelty=500000). |
| `docs/operator/env-reference.md` | Note that `PERPLEXITY_FLOOR_MICROS` defaults to 0 for pilot launch with a pointer to the A2.5 spec for context. Same for `TAIL_FRACTION_FLOOR_MICROS`. |
| `docs/operator/deployment.md` | Update the deployment-walkthrough env-var block to show the new floor values + a one-line "see calibration.md for why" pointer. |
| `docs/trace-commons-roadmap.md` | A2.5 status line under Phase A. Add Phase A.5 (gate-design reconsideration) to the deferred list. |

**Out of scope (do not touch)**

- The bake-off binary (`tracedao-gate-calibrate`) and its tests.
- The gate service code (`tracedao-ingest`, `tracedao-gate-enclave`).
- The candidate manifest schema.
- The A2.3c + A2.4 result reports (those stand as history).
- Phase A.5 design work (deferred to future retrofit).

---

## Pre-flight

- [ ] **Confirm green baseline.**

```bash
cargo check -p tracedao-server --bins
```

Expected: clean. (No code changes will happen, but good to confirm we're starting from a known state.)

- [ ] **Read the spec + the driver reports.**

```bash
$EDITOR docs/superpowers/specs/2026-05-14-gate-floor-recalibration-design.md
$EDITOR docs/superpowers/reports/2026-05-13-model-bakeoff-result-notes.md
```

(A2.4 result notes will land in this retrofit's Slice 1 — they don't exist yet at plan-write time.)

You need to internalize the AUC-below-0.5 finding before writing the operator-facing updates. The whole point of the recalibration is that the data invalidates one of A2's load-bearing assumptions; the operator needs to know *why* the floor changed, not just *what* changed.

---

## Slice 1 — Findings report

### Task 1: Write the gate-floor recalibration findings report

**Files:**
- Create: `docs/superpowers/reports/2026-05-14-gate-floor-recalibration-findings.md`

The report should cover:

- TL;DR: AUC < 0.5 across all 4 candidates × 2 corpora; perplexity gate doesn't discriminate as designed; new floor recommendations.
- A2.3c numbers (4 candidates × boilerplate-corpus): the headline AUC inversion.
- A2.4 numbers (4 candidates × Wikipedia-corpus): the corpus-design hypothesis test result.
- Side-by-side delta table per candidate (showing Llama's +0.12 improvement vs Qwen's -0.03 slide).
- The interpretive analysis: instruct-aligned models find OASST2 reasoning low-perplexity because it's their native training distribution.
- The tail-fraction signal that *does* persist (Gemma 4 31B at 0.20).
- Concrete pilot-launch floor recommendations.
- Future work: Phase A.5 metric design candidates.

The report companions the existing A2.1 + A2.3c result notes. Don't duplicate them; reference them. ~3-5 pages of markdown.

- [ ] **Step 1: Read the spec's "What's worth being explicit about" section** for the framing the report should land. The honest-answer paragraph in the spec is the answer to a stakeholder; the report is the data + reasoning behind that answer.

- [ ] **Step 2: Pull the A2.3c result JSON + A2.4 result JSON** from `docs/superpowers/reports/2026-05-13-model-bakeoff-result.json` and (when A2.4 lands) from wherever it gets committed. Include the per-candidate AUC table in the report.

- [ ] **Step 3: Write the report.**

- [ ] **Step 4: Commit.**

```bash
git add docs/superpowers/reports/2026-05-14-gate-floor-recalibration-findings.md
git commit -m "Record gate-floor recalibration findings from A2.3c + A2.4"
```

---

## Slice 2 — Calibration runbook

### Task 2: Rewrite Phase 0 + Phase 1 of `docs/operator/calibration.md`

**Files:**
- Modify: `docs/operator/calibration.md`

Changes:

- **Phase 0 (Model bake-off):** Keep the existing description of how to run the bake-off binary. Add a callout block at the top noting that A2.3c + A2.4 measured AUC < 0.5 across all candidates, so the bake-off's headline winner-pick is *not* a "this model has good perplexity discrimination" signal; it's a "this model has the best operationally-acceptable score under our decision rule." Point to the findings report.

- **Phase 1 (Offline HF bootstrap):** Update the floor calibration guidance:
  - `PERPLEXITY_FLOOR_MICROS`: **ship at 0 for pilot launch**. Document why (per A2.5 findings). Note that bumping above 0 in production today would systematically reject contributor reasoning.
  - `TAIL_FRACTION_FLOOR_MICROS`: ship at 0 for pilot launch. After the first ~1000 real pilot traces accumulate, calibrate per the existing Phase 1 procedure but using only the tail-fraction column from `tracedao-gate-calibrate` output. The aggregate-perplexity column is misleading.
  - `NOVELTY_FLOOR_MICROS`: 500000 (cosine novelty 0.5) — unchanged from A2 deployment.md. The embedder + vector-index path was not part of the A2.3c/A2.4 invalidation; it's the active gate at launch.

- **Phase 2 (live cutover):** Note that flipping `NOVELTY_UTILITY_CREDIT_POINTS_DELTA` from 0 to a positive value at the end of Phase 2 still applies. The phase semantics don't change.

- [ ] **Step 1: Read the current calibration.md** end-to-end so the new content matches voice + structure.

- [ ] **Step 2: Insert the Phase 0 callout block** about the AUC inversion. Keep it tight — one paragraph + a pointer to the findings report.

- [ ] **Step 3: Rewrite Phase 1's floor-recommendation table** to show the new pilot-launch values.

- [ ] **Step 4: Add a "When the perplexity gate becomes useful" subsection** at the bottom of Phase 1, summarizing the Phase A.5 future-work options (contrastive perplexity, per-token rarity, learned discriminator). One paragraph each, links to the spec.

- [ ] **Step 5: Commit.**

```bash
git commit -m "Recalibrate gate-floor guidance in operator runbook after A2.5 findings"
```

---

## Slice 3 — Env reference + deployment doc

### Task 3: Update env-reference.md + deployment.md

**Files:**
- Modify: `docs/operator/env-reference.md`
- Modify: `docs/operator/deployment.md`

Two small changes:

- In `env-reference.md`, find the rows for `TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS` and `TRACE_COMMONS_GATE_TAIL_FRACTION_FLOOR_MICROS`. Update each to note "pilot-launch default: 0 (see A2.5 spec / calibration.md Phase 1)."

- In `deployment.md`, find the env-var block in the "Configure environment" section. Update the three lines that set the floors:
  ```sh
  export TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS=0      # A2.5: perplexity AUC < 0.5; disabled at pilot launch
  export TRACE_COMMONS_GATE_TAIL_FRACTION_FLOOR_MICROS=0   # A2.5: calibrate post-first-1000-traces
  export TRACE_COMMONS_GATE_NOVELTY_FLOOR_MICROS=500000    # 0.5 cosine novelty; unchanged
  ```
  And update the "At least one of the three gate floors must be positive" sentence — that's still true (novelty floor is positive), but the comment about which one needs the update.

- [ ] **Step 1: Update env-reference.md.**
- [ ] **Step 2: Update deployment.md.**
- [ ] **Step 3: Commit.**

```bash
git commit -m "Note A2.5 gate-floor defaults in env-reference and deployment runbook"
```

---

## Slice 4 — Roadmap

### Task 4: Add A2.5 + Phase A.5 entries to the roadmap

**Files:**
- Modify: `docs/trace-commons-roadmap.md`

Two additions:

- Under Phase A status block, add `- A2.5: gate-floor recalibration after bake-off findings — done`.

- Add a new Phase A.5 (deferred list) for the perplexity-replacement metric work. One paragraph noting the three candidate approaches (contrastive perplexity, per-token rarity, learned discriminator) and the dependency on first ~1000 pilot traces.

- [ ] **Step 1: Make both edits.**

- [ ] **Step 2: Commit.**

```bash
git commit -m "Add A2.5 + Phase A.5 entries to roadmap"
```

---

## Done criteria

- [ ] `cargo check -p tracedao-server --bins` clean (unchanged from baseline — no code changes).
- [ ] Four commits on `feat/a25-gate-floor-recalibration`, in this order with these subjects:
  1. `Record gate-floor recalibration findings from A2.3c + A2.4`
  2. `Recalibrate gate-floor guidance in operator runbook after A2.5 findings`
  3. `Note A2.5 gate-floor defaults in env-reference and deployment runbook`
  4. `Add A2.5 + Phase A.5 entries to roadmap`
- [ ] All commits carry the Co-Authored-By trailer for `Claude Opus 4.7 (1M context) <noreply@anthropic.com>`.
- [ ] No `--no-verify`, no `--amend`.
- [ ] No emojis.
- [ ] PR opened against `main` with a description summarizing the four documentation changes.

---

## What this plan does NOT do

- **Does not change any code.** All env vars, structs, traits, and binaries stay as-is. This is a recommended-values update + documentation refresh.
- **Does not delete A2 / A2.1 / A2.3c / A2.4 history.** Those reports stand as the record. A2.5 is the *interpretation* of that history; it doesn't replace it.
- **Does not start Phase A.5 (perplexity-replacement metric).** Phase A.5 is parked in the deferred-work list of the roadmap; design work begins after the first ~1000 pilot traces give us labeled novel/duplicate exemplars.
- **Does not flip env-var defaults in code.** The A2 spec said operators set these per their deployment. We're updating the *recommended values* in the runbook, not the compile-time defaults.

## Spec open questions parked here

1. **Inverted perplexity floor (`<= floor` semantics).** Parked for Phase A.5; pilot-launch simplicity wins.
2. **Candidate revisit.** Stay with Qwen3-8B-Base for cost; revisit if pilot tail-fraction data favors larger models.
3. **Phase A.5 metric design.** Three candidates documented in the findings report; pick after first ~1000 pilot traces.
4. **Embedder + vector-index path validation.** Add to operator smoke checklist (existing `smoke-gate.sh`) in a follow-up — that's a different scope. This retrofit assumes the embedder path works as designed in A3.
