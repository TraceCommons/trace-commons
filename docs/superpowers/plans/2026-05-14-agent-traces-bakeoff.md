# Agent-Traces Bake-off Implementation Plan (A2.6 Retrofit)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a small corpus-builder addendum that extracts agent traces from `jedisct1/agent-traces-swival` and emits a slice tarball compatible with the existing bake-off binary. Run one bake-off and commit the report.

**Architecture:** Pure operator script (Python). Reuses A2.3c/A2.4's bake-off binary as-is. Reuses A2.4's Wikipedia duplicate slice. Reuses A2.3c/A2.4's paraphrase pipeline.

**Tech Stack:** Python (pyarrow + datasets-hub access). No Rust changes.

**Spec:** `docs/superpowers/specs/2026-05-14-agent-traces-bakeoff-design.md`

---

## File Map

**New files**

| Path | Responsibility |
|------|----------------|
| `scripts/operator/build-agent-traces-corpus.py` | Pulls swival dataset, extracts/filters/joins per-row text, splits into novel slice, packs corpus tarball (or merges into an existing tarball) |
| `docs/superpowers/reports/2026-05-14-model-bakeoff-result-a26.{json,md,notes.md}` | Post-run output (committed in Slice 3 after the operator runs A2.6b) |

**Modified files**

| Path | What changes |
|------|--------------|
| `docs/operator/calibration.md` | Add a Phase 0.1 subsection describing the A2.6 corpus variant + when to use it |
| `docs/trace-commons-roadmap.md` | A2.6 entry under Phase A status (initially "pending run"; flipped to "done" after A2.6c report lands) |

**Out of scope (do not touch)**

- The bake-off Rust binary (`trace-commons-gate-calibrate`)
- The mistralrs backend
- Any candidate-arch logic
- A2.5's floor recommendations (those are pilot-launch defaults; A2.6 doesn't change them)
- The existing `build-bakeoff-corpus.sh` script (new script lives alongside; no edits to the old one)

---

## Pre-flight

- [ ] **Confirm green baseline.**

```bash
cargo check -p trace-commons-server --bins
```

Expected: clean. No code changes will happen in this retrofit.

- [ ] **Read the spec + the A2.3c/A2.4 result notes** so you have the comparison baselines fresh.

- [ ] **Spec Open Question 1 — check swival dataset date vs candidate training cutoffs.**

```bash
# Get swival's first commit / upload date:
curl -sS https://huggingface.co/api/datasets/jedisct1/agent-traces-swival | python3 -c "
import json, sys
d = json.load(sys.stdin)
print('createdAt:', d.get('createdAt'))
print('lastModified:', d.get('lastModified'))
"
```

Candidate cutoffs (record what you find):
- Llama-3.1-8B-Instruct — Dec 2023 cutoff (publicly known)
- Qwen3-8B-Base — Apr 2025 release; cutoff likely late 2024
- Qwen 3.6 27B Dense — Apr 2026 release; cutoff late 2025 or early 2026
- Gemma 4 31B Base — Apr 2026 release; cutoff late 2025

If swival's first upload date is *after* any candidate's cutoff, that candidate is genuinely out-of-distribution for swival content. Llama and Qwen3 are likely safe; Qwen 3.6 and Gemma 4 are borderline.

If swival predates *all* candidate cutoffs, the experiment is less meaningful — surface to operator before proceeding. Switch to a fresher dataset like `lewtun/ml-intern-sessions` (updated 1 day ago).

---

## Slice 1 — Corpus builder for agent-traces

### Task 1: Write `build-agent-traces-corpus.py`

**Files:**
- Create: `scripts/operator/build-agent-traces-corpus.py`

**Inputs:**
- `--source` — HF dataset id (default `jedisct1/agent-traces-swival`)
- `--duplicate-corpus` — path to existing `corpus-wiki.tar.zst` (the A2.4 corpus to reuse the duplicate + paraphrase slices)
- `--count` — number of novel entries (default 300)
- `--seed` — RNG seed (default 42)
- `--out` — output `.tar.zst` path

**Behavior:**
1. Download the source dataset (caches under `~/.cache/huggingface/datasets`).
2. Parse each row. For swival format:
   ```python
   text = "\n\n".join([
       row.get("title", ""),
       row.get("severity", "") + " " + row.get("finding_type", ""),
       "\n".join(row.get("preconditions", [])),
       "\n".join(row.get("proof", [])),
       row.get("fix_outline", ""),
       (row.get("source_code", "") or "")[:1000],  # first ~1000 chars
   ]).strip()
   ```
   For other dataset formats, accept a `--format` flag to switch the row-to-text mapping; default to "swival" for v1.
3. Length-filter to 200-2000 words (`wc -w` equivalent in Python).
4. Sample `--count` traces with deterministic seed.
5. Read the duplicate slice + paraphrase slice from `--duplicate-corpus` (use the same `read_corpus` logic the bake-off binary uses — see `crates/trace-commons-server/src/bin/gate_calibrate/bakeoff_corpus.rs`).
6. Recompute slice SHA256s for novel + duplicate + paraphrase per the corpus format.
7. Pack into a new `.tar.zst` and print the output SHA256.

**Validation:** the new tarball must load through the existing `bakeoff_corpus::load_corpus` without error. The bake-off binary should not need any modifications.

- [ ] **Step 1: Read `crates/trace-commons-server/src/bin/gate_calibrate/bakeoff_corpus.rs`** so the Python emits a format the Rust loader accepts (manifest.json shape, slice directory layout, SHA256 calculation order).

- [ ] **Step 2: Write the script.** ~150-200 lines of Python.

- [ ] **Step 3: Run locally with `--count=4` against a downloaded swival shard** to verify the output tarball loads in the binary's existing dry-run smoke (`cargo test -p trace-commons-server --test bakeoff_corpus_script` doesn't test this directly; the closest existing test is `bakeoff_corpus::tests::loads_synthetic_corpus_three_slices`).

- [ ] **Step 4: Commit.**

```bash
git add scripts/operator/build-agent-traces-corpus.py
git commit -m "Add agent-traces corpus builder for A2.6 bake-off"
```

---

## Slice 2 — Operator runbook entry

### Task 2: Document A2.6 corpus variant + when to use it

**Files:**
- Modify: `docs/operator/calibration.md`

Add a "Phase 0.1 — Alternative corpus shapes" subsection between the existing Phase 0 (model bake-off) and Phase 1 (offline HF bootstrap) sections. Cover:

- Why an alternative corpus exists (A2.5 finding that OASST2 corpus inverted AUC across all candidates)
- Three corpus variants we've measured:
  - boilerplate-duplicate (A2.3c) — AUC range 0.054-0.276
  - Wikipedia-duplicate (A2.4) — AUC range 0.185-0.264
  - agent-traces-novel + Wikipedia-duplicate (A2.6, pending) — hypothesis: AUC may cross 0.5
- How to run the A2.6 corpus variant (`build-agent-traces-corpus.py` command line)
- When to use which corpus: A2.6 is the experiment; A2.4 is the baseline if A2.6 invalidates the hypothesis

- [ ] **Step 1: Write the section.**
- [ ] **Step 2: Commit.**

```bash
git commit -m "Document A2.6 alternative-corpus variant in calibration runbook"
```

---

## Slice 3 — Operator activity: run + report (deferred to operator)

This slice is **not implementer work.** It's an operator activity on Lambda H100.

The plan documents what the operator does so the resulting report is comparable to A2.3c/A2.4:

1. Provision Lambda H100 (us-southeast-1 preferred; A100-80GB acceptable fallback).
2. Stage 4 models + Qwen3-4B-Base for paraphrase (already exercised in A2.3c/A2.4).
3. Run `build-agent-traces-corpus.py` to produce `corpus-a26.tar.zst`.
4. Run the bake-off: same `candidates-4way.toml`, new corpus, new report-out path:
   ```bash
   ./target/release/trace-commons-gate-calibrate bake-off \
     --candidates=$HOME/bakeoff/candidates-4way.toml \
     --corpus=$HOME/bakeoff/corpus-a26.tar.zst \
     --hardware=h100 \
     --report-out=$HOME/bakeoff/report-a26.json
   ```
5. Pull the report locally, scp to `docs/superpowers/reports/2026-05-14-model-bakeoff-result-a26.{json,md}`.
6. Write the notes companion at `docs/superpowers/reports/2026-05-14-model-bakeoff-result-a26-notes.md` documenting:
   - Per-candidate AUC comparison to A2.3c + A2.4
   - Whether the hypothesis held (AUC > 0.5 for any candidate)
   - Recommended next step (A2.7 floor update, or A2.5 stands)
7. Commit the three new files.
8. Teardown the H100 (Lambda capacity is scarce; release it).

Cost: ~$25 (~5 hr GPU).

- [ ] **Step 1: Document the operator procedure in a small `docs/operator/agent-traces-bakeoff-run.md` runbook.**

- [ ] **Step 2: Commit.**

```bash
git commit -m "Add agent-traces bake-off operator runbook"
```

---

## Slice 4 — Roadmap

### Task 3: Add A2.6 entry under Phase A

**Files:**
- Modify: `docs/trace-commons-roadmap.md`

Add a single-line entry under the Phase A status block:

```
- A2.6: agent-traces novel-slice bake-off — pending run (corpus builder + runbook merged; awaiting operator bake-off run + report per spec rollout A2.6b)
```

Flip to "done" when the A2.6c report PR lands.

- [ ] **Step 1: Make the edit.**
- [ ] **Step 2: Commit.**

```bash
git commit -m "Add A2.6 pending entry to roadmap"
```

---

## Done criteria

- [ ] `cargo check -p trace-commons-server --bins` clean (unchanged from baseline)
- [ ] Four commits on `feat/a26-agent-traces-bakeoff`, in order with these subjects:
  1. `Add agent-traces corpus builder for A2.6 bake-off`
  2. `Document A2.6 alternative-corpus variant in calibration runbook`
  3. `Add agent-traces bake-off operator runbook`
  4. `Add A2.6 pending entry to roadmap`
- [ ] All commits carry the Co-Authored-By trailer
- [ ] No `--no-verify`, no `--amend`
- [ ] No emojis
- [ ] PR opened against `main`

---

## What this plan does NOT do

- **Does not run the bake-off.** That's A2.6b, operator activity.
- **Does not commit the report.** That's A2.6c, post-run PR.
- **Does not change A2.5's floor recommendations.** If A2.6 results justify it, A2.7 does that as a separate PR.
- **Does not write the corpus to the existing `build-bakeoff-corpus.sh` script.** New script alongside; cleaner diff and easier to back out if the experiment fails.
- **Does not introduce new Cargo deps.** Pure Python script.

## Spec open questions parked here

1. **Swival dataset date vs candidate training cutoffs.** Pre-flight step checks this; if all candidates have swival in training, switch to a fresher dataset.
2. **Should the duplicate slice also change?** Stay with Wikipedia for direct A2.4-comparability. Future retrofit can vary the duplicate slice.
3. **Multi-source agent-traces corpus.** v1 is swival-only. If A2.6 hypothesis holds, A2.7 can blend swival + pi-mono + DeepSeek-v4-Pro-Agent.
4. **Larger corpus.** 300 entries; ±6% AUC confidence is fine for the directional question.
