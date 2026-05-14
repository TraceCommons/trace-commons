# Agent-Traces Bake-off — Design (Phase A2.6 Retrofit)

Date: 2026-05-14
Status: Draft (pre-implementation)
Owner: Trace Commons / Datasets lane
Predecessors:
- `2026-05-14-gate-floor-recalibration-design.md` (A2.5 — the finding this retrofit may invalidate)
- `2026-05-13-mistralrs-migration-design.md` (A2.3 — backend in use)
- `2026-05-13-model-bakeoff-retrofit-design.md` (A2.1 — original corpus design)
Driver: HF survey of `format:agent-traces` datasets (2026-05-14) revealed 33.7k+ rows of OSS security-audit and coding-agent traces that match Trace Commons's intended input shape far better than the OASST2 + boilerplate corpus we ran in A2.3c / A2.4.

## Motivation

A2.5 concluded that the perplexity floor must ship at 0 for pilot launch because every candidate × corpus combination measured AUC < 0.5. That conclusion was based on:

- **Novel slice:** OASST2 conversational reasoning (heavily in-training-distribution for every candidate)
- **Duplicate slice:** boilerplate snippets (A2.3c) OR Wikipedia article intros (A2.4)

Both corpus variants suffered the same structural problem: OASST2 chat is *exactly* what these models were RLHF'd on, so they find it predictable. Every duplicate-slice variant we tried (snippets and prose) gave models less-trained-on material as the "duplicate" — backwards from intent.

A 2026-05-14 survey of HuggingFace's `format:agent-traces` dataset filter surfaced a different kind of data: **multi-turn tool-using sessions** captured from real OSS work. The standout is `jedisct1/agent-traces-swival` — 33,667 security-audit traces (MIT-licensed, ungated, ~1.58 GB) from the Swival open-source agent. Each trace is structured (`finding_type`, `severity`, `locations`, `proof`, `fix_outline`, source code, multi-step tool calls) and concerns specific OSS projects (Keccak crypto, malloc implementations, web frameworks).

This data is closer to Trace Commons's intended input shape *and* less in-distribution for the candidate models. The hypothesis worth testing:

> **If we swap the bake-off's novel slice from OASST2 chat to agent-traces (security audits / coding sessions), and keep Wikipedia intros as the duplicate slice, the AUC may cross 0.5.** That would mean the perplexity gate works fine — A2's *novel-slice choice* was the bug.

If true, A2.5's "perplexity disabled at launch" recommendation can be partially walked back and Phase A.5 (perplexity-replacement metric) deferred further or scrapped. If false, A2.5 stands and Phase A.5 remains the path.

This is a one-day code + one-bake-off-run experiment. ~$25 of Lambda time to resolve a question worth resolving before pilot launch.

## Goal

Run one new bake-off (A2.6c) against an agent-traces novel slice and the existing Wikipedia duplicate slice. Same 4 candidates, same code, same paraphrase pipeline. Generate a report comparable to A2.3c + A2.4. If AUC crosses 0.5 for any candidate, file an A2.7 PR to update A2.5's floor recommendations.

## Non-goals

- **No new code in the gate-service or bake-off binary.** The binary handles whatever corpus we feed it. This retrofit only changes the corpus contents.
- **No new candidate models.** The Llama-3.1-8B-Instruct + Qwen3-8B-Base + Qwen 3.6 27B Dense + Gemma 4 31B Base set is what we have GPU experience with.
- **No new architecture, no new dependencies, no mistralrs swap.** A2.3's mistralrs backend handles agent-traces input the same way it handles any other text.
- **No production deployment changes.** A2.5's pilot-launch floors (perplexity = 0, tail-fraction = 0, novelty = 500000) stand until A2.6's data justifies revisiting them.

## Decisions baked in

| Decision | Value |
|----------|-------|
| Novel-slice source | `jedisct1/agent-traces-swival` — 33.7k OSS security-audit traces, MIT, ungated |
| Novel-slice extraction | The `proof` + `fix_outline` + first ~1000 chars of `source_code` per row, joined as prose. Length-filter to 200-2000 words like the OASST2 filter. Sample 300 with a deterministic seed. |
| Duplicate slice | Wikipedia intros (reuse from A2.4's `corpus-wiki.tar.zst`) |
| Paraphrase pipeline | Reuse from A2.3c/A2.4 — Qwen3-4B-Base back-translation, batched (BAKEOFF_BATCH_SIZE=16) |
| Candidate set | Identical to A2.3c/A2.4 (Llama-3.1-8B + Qwen3-8B + Qwen 3.6 27B + Gemma 4 31B) |
| Corpus size | 300/300/300, matching A2.3c/A2.4 |
| Hardware | Lambda H100 SXM5 80GB (us-southeast-1 or wherever capacity is) |

## What success looks like

A2.6c report committed to `docs/superpowers/reports/`. Three possible outcomes, each with a clear next step:

1. **At least one candidate AUC > 0.5.** The hypothesis holds — perplexity gate works with the right corpus.
   - File A2.7 PR updating A2.5's floor recommendations: re-enable perplexity floor with a value calibrated against this run's distribution.
   - Phase A.5 (perplexity-replacement metric) becomes "no longer needed" — close the parked item.

2. **All candidates 0.5 > AUC > 0.4.** The hypothesis is partially supported — agent-traces help substantially but don't fully invert.
   - Document the partial improvement.
   - Phase A.5 remains parked but with reduced urgency — the gate-as-designed isn't broken, just weak.
   - Floor recommendation stays at A2.5's (0) but documented as conservative-by-default; operators can experiment with positive floors against their own pilot data.

3. **All candidates AUC < 0.4.** Agent-traces don't fix the issue either.
   - A2.5's conclusion is reinforced: perplexity-based novelty doesn't work for modern aligned LLMs across multiple corpus shapes.
   - Phase A.5 stays on the roadmap; agent-traces corpus is no longer a "maybe this would have worked" hedge.

Any outcome is informative. The cost ($25, ~5 hr GPU) is small relative to the resolved-uncertainty value.

## Open questions

1. **Are security audit traces from the swival dataset already in pretraining data?** `jedisct1/agent-traces-swival` was created via the Swival agent + uploaded to HF; it might have been scraped into training corpora since its publication date. **Recommendation:** check the dataset's first-upload date vs each candidate's training cutoff. If swival predates the candidates' cutoffs, switch to a fresher dataset (e.g., `lewtun/ml-intern-sessions` updated 1 day ago).

2. **Should the duplicate slice also change?** Wikipedia intros worked partially in A2.4 (improved 2 of 4 candidates). Keep them for direct comparability across A2.4 and A2.6, or switch to "agent-traces from a different domain" for tighter format-match? **Recommendation:** keep Wikipedia for now to isolate the variable change to the novel slice. If A2.6 results are inconclusive, A2.7 can try same-format-different-domain duplicates.

3. **Multi-candidate corpus sources?** Three agent-trace datasets are good candidates: swival (security audits), pi-mono (OSS coding sessions), DeepSeek-v4-Pro-Agent (tool-using software engineering). **Recommendation:** start with swival alone for the first run — single-source means single hypothesis to test. If results are promising, A2.7 can mix sources.

4. **Larger corpus size to chase a clearer signal?** 300 traces gives ±6% AUC confidence; a 1000-trace run would give ±3%. **Recommendation:** 300 is fine for the directional question this experiment asks. Larger samples are for the post-pilot real-data calibration, not for A2.6.

## Deliverables

1. **A2.6a — corpus builder addendum.** A small Python script under `scripts/operator/build-agent-traces-corpus.py` that reads the swival dataset's parquet shards, extracts/filters/joins per the spec, and emits a slice tarball compatible with the existing `bakeoff_corpus::load_corpus` format. No changes to `build-bakeoff-corpus.sh`; new script alongside.

2. **A2.6b — bake-off run.** Operator activity on Lambda H100. Same shape as A2.3c/A2.4. ~5 hr GPU, ~$25.

3. **A2.6c — report.** `docs/superpowers/reports/2026-05-14-model-bakeoff-result-a26.{json,md,notes.md}` documenting the data and the next-step decision.

4. **(Conditional) A2.7 — floor recommendation update.** Only fires if A2.6 results justify a recommendation change. Otherwise A2.5 stands.

## Trade-offs explicitly accepted

- **One more bake-off run before pilot.** The pilot was technically launchable as of A2.5; this is one more experiment first.
- **Risk that agent-traces are also in training data.** Mitigated by Open Question 1's pre-flight check.
- **Doesn't unblock pilot client work.** A.6 (pilot-bootstrap harness) is the parallel track that does. Files together.
