# PR #95 Multimodal Pipeline Hang Regression — 2026-05-14

## Summary

The bake-off binary post PR #95 (`Persist per-trace perplexity scores
in bake-off report`) hangs indefinitely when scoring a candidate whose
mistralrs pipeline reports input modalities `[Text, Vision]`. The
hang is silent: no GPU usage, no error log, no progress events past
the initial `candidate_eval_start`. The hung process accumulates CPU
time consistent with a polling/wait loop but emits no further log
output.

This blocks the A2.7 perplexity floor calibration rerun against
Qwen 3.6 27B Dense — the candidate that fired A2.6 Outcome 1 (AUC
0.936) — because the only HF-published 27B Qwen 3.6 variant
(`Qwen/Qwen3.6-27B`) is the multimodal `Qwen3_5ForConditionalGeneration`
architecture.

## Reproduction

On a clean Lambda H100 SXM5 80GB instance, with main at commit
`6dbbe3d` (post PR #95):

1. Build: `cargo build --release -p trace-commons-server --bin trace-commons-gate-calibrate --features local-gpu-models` — succeeds.
2. Download model: `huggingface_hub.snapshot_download("Qwen/Qwen3.6-27B", local_dir="~/models/qwen3.6-27b-dense")` — ~52 GB across 15 safetensors files.
3. Manifest: single-candidate `scripts/operator/candidates-qwen-only.toml` pointing at the downloaded path, `arch = "qwen3"`.
4. Run: `trace-commons-gate-calibrate bake-off --candidates=... --corpus=corpus-a26.tar.zst --hardware=h100 --report-out=report-a27.json`
5. Binary loads the model (`bakeoff_candidate_load_done load_elapsed_seconds=93.8`), emits `candidate_eval_start`, then hangs.

## Observed behavior

- **Pipeline detection:** `Pipeline input modalities are [Text, Vision]`. Same detection in A2.6's log for the same candidate — that run scored successfully.
- **Last log line:** `Prefix caching enabled (sequence-level, non-paged attention). Expect higher multi-turn throughput for both text and multimodal.` — emitted within 2 seconds of `candidate_eval_start`. After that: nothing.
- **GPU utilization:** `0%` throughout. `nvidia-smi` reports `0 MiB` allocated. The model weights never make it into VRAM.
- **CPU time:** the process accumulates ~3 threads worth of CPU time (170+ minutes CPU time across 57 minutes wall clock at peak) without emitting log output.
- **No errors:** no panic, no stderr, no `score_failed` events. Just silent waiting.
- **No truncation WARNs:** Last A2.6 run emitted near-constant `PerplexityScorerInputTruncated` WARNs once scoring began. Today's hung run emits ZERO.

## Why we believe this is PR #95's regression

1. **A2.6 ran the same model successfully** (per archived log `docs/superpowers/reports/2026-05-14-model-bakeoff-result-a26.log`, lines around `candidate_id=qwen3.6-27b-dense` at 14:14:18 UTC).
2. **mistralrs git rev is identical** (`2d4ba4f16f61e5e18be085d0dd137bc95cba038a` in both builds — pinned in Cargo).
3. **PR #95 changed `run_candidate_eval.rs`** to add per-slice `Vec<Option<f64>>` accumulators populated as the scoring channel drains. The new code path is the likely deadlock site.
4. **Mock-scorer tests passed in PR #95** but those tests synthesize per-trace scores directly without exercising mistralrs's Raw response channel for multimodal pipelines.

The failure mode is consistent with the per-trace accumulator
waiting on a channel-receive that never completes when the response
metadata signals a different shape than the text-only pipeline
(possibly a Vision token in the response that the new path doesn't
drain).

## Cost of this run

- Lambda H100 SXM5 80GB, us-southeast-1, ~63 minutes session time (build + download + hung scoring + teardown)
- ~$4.51 sunk
- No usable report data produced
- Logged separately in `docs/operator/gpu-cost-ledger.md`

## Recommended fix path

1. **Reproduce without GPU.** Add a multimodal-pipeline integration test for the per-trace-score accumulator path. The test does NOT need real GPU inference — mock the mistralrs Raw channel to emit responses with the multimodal metadata shape, and assert the accumulator drains them without hanging.

2. **Fix the accumulator drain logic.** Likely a missing branch where multimodal responses include extra envelopes (Vision-token markers, image-grid metadata) that the per-trace path doesn't recognize.

3. **Re-run A2.7 calibration.** After the fix, re-provision an H100 and re-run Qwen 3.6 27B against `corpus-a26.tar.zst` with `candidates-qwen-only.toml`. Same procedure as today, with the fixed binary. Expected ~3h17m + ~$15.

4. **Smoke against text-only model first.** Llama-3.1-8B-Instruct is text-only (`Pipeline input modalities are [Text]`). After the fix, run it as a smoke verification (~85 min, ~$6) before committing to the 27B re-bake.

## What stays in place until then

- Pilot launches with `TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS=0` per A2.5 (conservative-by-default).
- The `tail-floor` subcommand (PR #66) drives a separate calibration path off real pilot traffic via `trace-commons-gate-decisions` rows.
- A2.7 calibration runbook (`docs/operator/a27-perplexity-floor-calibration.md`) already carries the "BLOCKED ON BAKE-OFF BINARY GAP" callout — extend it with a reference to this regression report.

## Hash-only audit

This report contains no contributor identity, raw trace bodies, raw
URLs, tokens, ARNs, transaction hashes, KEK material, or operator
secrets. Lambda instance ID is operator-secret and is not recorded
here (per the GPU cost ledger's convention).
