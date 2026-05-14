# A.5a — Per-Token Rarity Pre-Flight Operator Runbook

Focused one-candidate rerun against the committed A2.6 corpus that emits
per-trace rarity scores plus a per-K AUC sweep over `K ∈ {4, 8, 16, 32}`.
The script lives at `scripts/research/a5a_rarity_preflight.py`. This is
the cheapest de-risking step the architect requires (verdict
2026-05-14) **before** any production wiring of `LocalPerplexityScorer`
to the `TokenRarityScorer` trait.

Authoritative design: `docs/superpowers/specs/2026-05-14-a5a-rarity-real-scorer-design.md`
("Pre-flight experiment (mandatory before production wiring)").

**Expected cost:** ~$8 (~1.5 hr on a single Lambda H100 SXM5 80GB).

---

## When to fire this runbook

Only fire after **both** of the following are true:

1. A2.6 final results are recorded in `docs/superpowers/reports/` and the
   decision rule activated A.5 (all candidates AUC < 0.4, or operator
   decision in the 0.4–0.5 partial range).
2. The A.5 design spec selected per-token rarity (A.5a) as the first
   replacement candidate.

If A2.6 cleared the perplexity floor or the A.5 spec routed to a
different candidate first (e.g. A.5b contrastive), **do not run this
pre-flight** — the result is not actionable.

---

## Hardware

- **Recommended:** Lambda H100 SXM5 80GB (any region with capacity).
- **Acceptable fallback:** A100 80GB. Adds roughly 25% wall-clock.
- **Not acceptable:** any GPU below 40GB. The recommended candidate
  (Llama-3.1-8B-Instruct, for continuity with A2.6's partial result of
  AUC 0.342) does not fit comfortably in less.

A single instance is enough. No multi-GPU split; the pre-flight is a
one-candidate run.

---

## Step 1 — Provision the host

Provision a single Lambda H100 SXM5 80GB. Boot a recent Ubuntu LTS image
with the same CUDA + Python toolchain you used for A2.6. No new system
packages are required.

## Step 2 — Stage the candidate + corpus

Reuse the A2.6 staging:

- Candidate weights under `~/models/<candidate>/` (recommend
  `llama-3.1-8b-instruct` for continuity).
- A2.6 corpus tarball at `~/bakeoff/corpus-a26.tar.zst`. The committed
  fixture path is `scripts/operator/fixtures/corpus-a26.tar.zst`
  (sha256 `46e0eef8a52e309ce695ad20d1e242ce43eb210c11e02764beeaf7fa3d341bb5`);
  copy or symlink as convenient.

Install Python deps if the host is not already set up:

```bash
pip install transformers torch zstandard
```

`zstandard` is optional — if it's missing, the script falls back to the
`zstd` CLI on `$PATH`.

## Step 3 — Run the pre-flight

```bash
python3 scripts/research/a5a_rarity_preflight.py \
    --candidate llama-3.1-8b-instruct \
    --corpus ~/bakeoff/corpus-a26.tar.zst \
    --k-values "4,8,16,32" \
    --report-out ~/preflight-llama.json
```

The script writes the JSON report to `--report-out` and emits a
hash-only summary on stderr:

```
Perplexity baseline AUC (novel vs duplicate): 0.342
K=4: rarity AUC = 0.395
K=8: rarity AUC = 0.418
K=16: rarity AUC = 0.402
K=32: rarity AUC = 0.371
Elapsed: 4942.18s (245.1 tok/s)
Report: /home/ubuntu/preflight-llama.json
```

The figures above are illustrative; the actual values come from the
script's output.

## Step 4 — Verify provenance

The report header carries the corpus sha256 (computed by the script
over the input tarball) and a `model_sha_or_revision` label. Confirm
both match the A2.6 manifest entry for the chosen candidate before
quoting the results in any decision artifact.

```bash
jq '.corpus_sha256, .model_sha_or_revision, .slice_counts' \
   ~/preflight-llama.json
```

If `corpus_sha256` does not equal
`sha256:46e0eef8a52e309ce695ad20d1e242ce43eb210c11e02764beeaf7fa3d341bb5`,
the wrong corpus was scored — re-stage and re-run.

## Step 5 — Apply the decision rule

The spec's decision rule (verbatim):

> **Proceed with A.5a** if rarity AUC > 0.5 **AND**
> `(rarity AUC − A2.6 perplexity AUC) ≥ 0.05` for at least one K in
> `{4, 8, 16, 32}`. **Stop and re-route to A.5b (contrastive)** otherwise.

Procedure:

1. Look up the A2.6 perplexity AUC for the same candidate in
   `docs/superpowers/reports/2026-05-14-model-bakeoff-result-a26.md`.
2. For each K in `{4, 8, 16, 32}`, compute
   `rarity_auc[K] - a26_perplexity_auc`. The script's
   `perplexity_baseline.auc_novel_vs_duplicate` is computed on the
   same input but with a different model loader path; the A2.6
   number is the authoritative comparator for the decision rule
   (the pre-flight's baseline is a sanity check, not a replacement).
3. **Proceed with A.5a** if any K satisfies both conditions.
4. **Otherwise: stop.** Mark the A.5a spec
   `DRAFT — superseded by A.5b` and file an A.5b spec under
   `docs/superpowers/specs/`.

Do not partially proceed: a marginal delta (`0 < delta < 0.05`) is the
"stop" branch, not a green light. The spec's threshold is binary by
design.

---

## What this pre-flight does NOT do

- It does **not** modify any Rust crate. The actual A.5a production
  wiring — implementing `TokenRarityScorer` on `LocalPerplexityScorer`
  and surfacing un-aggregated logprobs out of mistralrs — is the slice
  this pre-flight gates. Do not start that slice until the decision
  rule passes.
- It does **not** evaluate the bake-off binary's `--scorer
  token-rarity` path. That path still raises
  `BakeoffRealRarityNotImplemented` for real-scorer candidates; the
  pre-flight bypasses it by computing rarity offline in Python.
- It does **not** assess whether tokenizer artifacts (URL fragments,
  UUIDs) inflate rarity scores. That open question is deferred to the
  real-scorer slice; the architect flagged it as a known risk but did
  not require a pre-flight signal on it.

## Failure modes

| Symptom | Likely cause | Action |
|---|---|---|
| `corpus tarball missing manifest.json` | Wrong tarball; A2.6 fixture not staged | Re-stage `scripts/operator/fixtures/corpus-a26.tar.zst` |
| `novel/duplicate/paraphrase slice sha256 mismatch` | Corpus was modified after the manifest was sealed | Re-pull from the committed fixture; do not edit the tarball |
| `transformers/torch import failed` | Python env not provisioned | `pip install transformers torch` on the Lambda host |
| Out-of-memory loading the candidate | GPU too small for the candidate | Drop to an 8B candidate or move to H100 80GB |
| AUC = 0.5 across all K | All logprob lists empty (model emitted no tokens) | Inspect `slice_counts` and `tokens_per_second` in the report; if `tokens_per_second` is near zero, the model load is broken |

## Hash-only audit

The script does **not** log raw token strings, token ids, or trace
bodies. Operational output is limited to slice counts, AUC values, the
corpus sha256, the candidate id (operator-set label), and the model
revision label. The JSON report contains per-trace rarity scores in
fixed-point micros — these are scalars, not text, and are safe to
attach to A.5 decision artifacts in operator-only channels.
