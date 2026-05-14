# PR #95 Multimodal Pipeline Hang Regression — 2026-05-14

> **2026-05-14 update — hypothesis retracted.** Follow-up investigation
> (no GPU required) demonstrates that PR #95 cannot have introduced a
> deadlock. The reproduction was run against a **CPU-only mistralrs
> build** (`--features local-gpu-models`, missing the `-cuda` suffix),
> not the CUDA build A2.6 used. The "hang" is CPU inference of a 27B
> multimodal model — functionally unbounded, not deadlocked. The
> original hypothesis section is preserved below for the audit trail;
> see the "Actual root cause" section at the bottom for the correction.

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

1. Build: `cargo build --release -p tracedao-server --bin tracedao-gate-calibrate --features local-gpu-models` — succeeds.
2. Download model: `huggingface_hub.snapshot_download("Qwen/Qwen3.6-27B", local_dir="~/models/qwen3.6-27b-dense")` — ~52 GB across 15 safetensors files.
3. Manifest: single-candidate `scripts/operator/candidates-qwen-only.toml` pointing at the downloaded path, `arch = "qwen3"`.
4. Run: `tracedao-gate-calibrate bake-off --candidates=... --corpus=corpus-a26.tar.zst --hardware=h100 --report-out=report-a27.json`
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
- The `tail-floor` subcommand (PR #66) drives a separate calibration path off real pilot traffic via `tracedao-gate-decisions` rows.
- A2.7 calibration runbook (`docs/operator/a27-perplexity-floor-calibration.md`) already carries the "BLOCKED ON BAKE-OFF BINARY GAP" callout — extend it with a reference to this regression report.

## Actual root cause (2026-05-14 follow-up)

Source review against `main` at commit `02fed39` shows the original
"PR #95 introduced a multimodal channel-drain deadlock" hypothesis
cannot be correct:

1. **PR #95's diff to `run_candidate_eval.rs` adds only synchronous
   `Vec::push(Some(_))` / `Vec::push(None)` lines** alongside the
   existing scoring code. No new `await`, no channel-receive, no
   `select!`, no async branching. The function `run_candidate_eval`
   is `async fn` but contains zero `.await` calls before or after
   PR #95 — every scorer call goes through the synchronous
   `PerplexityScorer::score` trait method.
2. **The mistralrs channel-receive in
   `crates/tracedao-gate-enclave/src/perplexity_local.rs` was not
   touched by PR #95.** That code (single-shot `rx.recv().await` on
   a `tokio::sync::mpsc::channel(1)` plus a `ResponseOk::Raw`
   destructure) is unchanged since PR #40 (A2.3 mistralrs migration).
3. **`Cargo.lock` has not been modified since PR #85** (before A2.6).
   mistralrs is pinned to git rev
   `2d4ba4f16f61e5e18be085d0dd137bc95cba038a` and is byte-identical
   between A2.6 and the today's binary. No transitive dependency
   bumps.
4. **A2.6 successfully scored the same model in this same code path.**
   The only Rust difference between A2.6's binary and today's binary
   is the synchronous accumulator added by PR #95 — which cannot
   deadlock anything.

Re-reading the reproduction command in this report's "Reproduction"
section reveals the actual cause:

```
cargo build --release -p tracedao-server --bin tracedao-gate-calibrate \
    --features local-gpu-models
```

This is the **CPU-only mistralrs build**. The CUDA build requires
`--features local-gpu-models-cuda` (see `docs/operator/deployment.md`,
`docs/operator/env-reference.md`, and `docs/operator/calibration.md`,
all of which document the `-cuda` suffix). On a Lambda H100 the
operator runbook calls for `local-gpu-models-cuda`; A2.6's
log-confirmed 10.3s load + 57 GB peak VRAM is consistent with the
CUDA build. Today's 93.8s load + 0 MiB VRAM is consistent with
mistralrs running on CPU.

The "hang" is not a deadlock. It is CPU inference of a 27B
multimodal model — well into the days-per-trace regime, with no
truncation WARN because the first trace's tokenization succeeds
but the prefill never finishes. nvidia-smi reports 0% / 0 MiB
because nothing was ever dispatched to the GPU. No `score_failed`
events fire because the scorer is blocked in mistralrs's CPU
forward pass, not erroring.

### Why no Rust fix is needed

The bug is in the operator command, not the code. The fix is:

- Re-issue the A2.7 build with `--features local-gpu-models-cuda`.
- Add a runtime guard in the bake-off binary (or a startup check in
  `LocalPerplexityScorer::try_new`) that refuses to run on CUDA
  hosts when mistralrs was compiled without the CUDA backend, so
  this footgun fails closed with a useful error instead of silently
  burning CPU hours. Tracked as a follow-up — out of scope for the
  A2.7 calibration rerun itself.

### What I did NOT change

- No Rust source edits. PR #95's accumulator additions are correct
  and uninvolved.
- No mistralrs version bump.
- No new integration test. A test that mocks a "multimodal Raw
  response shape" would not reproduce the actual issue (CPU
  inference is too slow) and would mislead future maintainers.

### Action items

1. **Re-run A2.7 with the correct feature flag.** Single-line fix
   in whatever provisioning script staged the broken build.
2. **Optional follow-up:** add the CUDA-feature/device-mismatch
   startup guard so this can't recur silently.
3. Roadmap entry for A2.7 stays "blocked on H100 capacity"; the
   bake-off binary itself is not blocked.

## Hash-only audit

This report contains no contributor identity, raw trace bodies, raw
URLs, tokens, ARNs, transaction hashes, KEK material, or operator
secrets. Lambda instance ID is operator-secret and is not recorded
here (per the GPU cost ledger's convention).
