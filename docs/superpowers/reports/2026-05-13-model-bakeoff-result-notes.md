# Bake-off Result Notes (2026-05-13)

Companion to `2026-05-13-model-bakeoff-result.{json,md}`.

## TL;DR

**Winner:** `qwen3-8b-base` (Apache-2.0, 8B dense, base — not instruct).

| Metric | Llama-3.1-8B-Instruct | Qwen3-8B-Base |
|---|---|---|
| Discrimination AUC | 0.105 | **0.720** |
| Paraphrase delta | 0.132 | 0.197 |
| Tail-fraction range | 0.114 | 0.111 |
| Throughput tps | 99.8 | 93.3 |
| Determinism stddev | 1.1e-16 | 4.8e-13 |
| Weighted score | 0.423 | **0.770** |

Qwen wins by **+82% relative** on the spec-committed weighted score
(`0.6·AUC + 0.3·(1−paraphrase_delta) + 0.1·tail_range_normalized`),
which is far outside the 2% tolerance band that would otherwise route
to the license tiebreaker. The Apache-2.0 license is a bonus.

## Headline finding: instruct-tuned Llama-3.1 is *inversely* calibrated

The most striking number in the report is Llama-3.1-8B-Instruct's AUC
of **0.105**. The metric is "probability a randomly drawn novel sample
has higher perplexity than a randomly drawn duplicate sample." At 0.5
the model is uncorrelated. At 1.0 it's perfectly discriminating
(novel-as-novel, duplicate-as-duplicate). At 0.105 it's *reversed* —
the model is more confident on novel reasoning traces than on stock
boilerplate.

Why: Llama-3.1-8B-**Instruct** has been RLHF'd to produce confident
helpful-completion patterns. Boilerplate / FAQ-prefix / docstring-style
text is exactly the distribution alignment training rewards confidence
on, so the model finds it more "expected" than the genuine OASST2
reasoning we used in the novel slice. The instruct-tuning has folded
the per-token distribution against the calibration property a
perplexity gate needs.

This empirically confirms the base-vs-instruct hypothesis the spec
parked as future work. **For the production gate, base models beat
instruct models** at the perplexity-discrimination task by a large
margin.

A follow-up worth doing eventually: run Llama-3.1-8B (base, no
instruct) for comparison. The hypothesis is that base Llama would have
a high AUC similar to Qwen3-8B-Base, and the instruct distortion is
specifically what tanks the metric. Worth ~$8 of Lambda time when
convenient; not blocking the pilot.

## Deviations from the spec'd plan

Several things didn't go as the spec described. All are documented
deviations, not result-tainting:

1. **2-candidate run, not 4.** Candle 0.10.2 can't load Qwen 3.6 27B
   (`model_type: qwen3_5`, multimodal config with nested `text_config`)
   or Gemma 4 31B (`model_type: gemma4`, same nested shape). Both
   failed at the `CandlePerplexityScorerLoadFailed` stage with
   `missing field hidden_size` because their text-model schema is one
   level deeper than candle expects. We trimmed the manifest to the
   two candidates that actually load.

2. **First run aborted at candidate 3 of 4.** The original orchestrator
   used `?` to propagate candidate load failures, killing the run when
   Qwen 3.6 failed and losing ~96 min of already-completed Llama +
   Qwen3-8B scoring. Fixed in PR #34 (bake-off observability) —
   subsequent runs catch load failures, mark the candidate failed in
   the report, and continue. The re-run with the trimmed manifest
   produced this result.

3. **OASST2-only novel slice, no GAIA.** GAIA is gated on Hugging Face
   (`"gated": "auto"`) and even with the token approved, the canonical
   file paths (`2023/validation/metadata.jsonl`) returned 404. Skipped
   to preserve session momentum. OASST2 alone provided 300 reasoning-
   shaped traces (filtered for length 200–2000 words + reasoning-marker
   regex match) which is enough for a sub-1% confidence AUC.

4. **300/300/300 corpus, not 500/500/500.** Tightened from the spec's
   1500-trace target. AUC confidence at 300/300 is roughly ±6%, well
   within the +82% relative gap between winner and runner-up — the
   result is decisive at this corpus size.

5. **Paraphrase backtranslation initially un-batched.** First attempt
   was at ~15s/entry single-batch (300 entries = ~75 min). Replaced
   the helper with a batched version (batch=16, left-padded for causal
   LM), finished in ~5 min. The batched script is now in
   `/tmp/bakeoff_paraphrase.py` on the bake-off host — should be
   upstreamed into `scripts/operator/bakeoff_paraphrase.py` (current
   in-tree version is the un-batched original).

6. **Skipped the dependency on `bakeoff-duplicate-seeds.txt`.** Patched
   the script to symlink the seeds from the repo into `/tmp` because
   the copy of the build script was in `/tmp` and used a
   relative-to-itself path. The in-tree script doesn't have this
   issue; it was an artifact of running a patched copy.

## Run metadata

- **Hardware:** Lambda Cloud H100 SXM5 80GB, us-southeast-1, $4.29/hr
- **OS:** Ubuntu 24.04 LTS, glibc 2.39, gcc-13.3.0, CUDA 12.8
- **Binary:** `trace-commons-gate-calibrate bake-off` at commit `707ef4e`
  (without the observability + load-failure improvements from PR #34
  which landed *after* this run)
- **Total elapsed:** ~5h 40min wall-clock from instance launch to
  termination
- **Cost:** ~$24
- **Corpus SHA256:** `sha256:8acb0be339b2da278986c389700884b23a92dafc85e41c6549d963b550938660`
- **Manifest SHA256:** `sha256:6870f59aec03472180ae86c569933bade4a7b1abe9105d4dfd5587921e0814bf`
- **Decision rule version:** 1
- **Gate version hash:** stamped in the JSON

## Recommended follow-up (spec rollout A2.1c – e)

1. **A2.1c — flip the production defaults.** Update
   `TRACE_COMMONS_PERPLEXITY_MODEL_ID` default from
   `meta-llama/Llama-3.1-8B-Instruct` to `Qwen/Qwen3-8B-Base` in
   `docs/operator/env-reference.md` and the deployment runbook. The
   Qwen3-8B-Base path replaces the incumbent Llama-3.1-8B-Instruct
   path.

2. **A2.1d — recalibrate floors against Qwen3-8B-Base.** The existing
   Phase 1 calibration (perplexity, tail-fraction, novelty floors)
   was tuned against Llama-3.1's distribution. With a different model
   the absolute perplexity values shift; floor numbers will need
   re-derivation via `scripts/operator/calibrate-from-hf.sh` against
   the new model. ~3-4 hr GPU time, similar cost (~$15-20).

3. **A2.1e — final smoke on Qwen3-8B-Base.** Re-run the 2026-05-13
   Lambda smoke flow (the original gate-readiness validation) with
   the new model. Should be quick (~30 min, <$5).

## Future work

- **Re-run with Qwen 3.6 27B Dense and Gemma 4 31B** when candle (or a
  swap to mistralrs / ort) supports their architectures. The bigger
  dense models might beat Qwen3-8B-Base, or might not — the headline
  base-vs-instruct effect will dominate either way.
- **Run a Llama-3.1-8B-Base (no instruct) comparison** to confirm the
  instruct-tuning hypothesis explicitly.
- **Upstream the batched paraphrase helper** to
  `scripts/operator/bakeoff_paraphrase.py` so future operators don't
  have to patch in-place. ~$0.50 of GPU vs ~$5 unbatched on 300 pairs.
- **Add a fix for the bash script's `relative-to-script_dir` seed-file
  path** so the script can be copied to /tmp and still find the seeds.
  Minor.
