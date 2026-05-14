# scripts/research

Research prototypes that validate metric or scoring designs *before* they land
as Rust code in `crates/trace-commons-server` or `crates/trace-commons-gate-enclave`.
Treat these as throwaway experiments: they are not wired into CI gates, they
are not signed by the production toolchain, and their outputs should not be
quoted in operator-facing decisions without a paired commit-to-Rust step.

## per-token-rarity-prototype.py

Implements the **per-token rarity** metric proposed in Phase A.5 as the
cheapest first experiment after A2.6. See the design context in
`docs/superpowers/specs/2026-05-14-a5-perplexity-replacement-design.md`
("Approach 2: Per-token rarity").

What it does:

1. Loads a bake-off corpus `.tar.zst` produced by the bake-off binary
   (`crates/trace-commons-server/src/bin/gate_calibrate/bakeoff_corpus.rs`) and
   verifies the manifest slice sha256s the same way the Rust loader does.
2. Loads a causal language model via `transformers.AutoModelForCausalLM`
   (default `gpt2` for CPU smoke; pass `--model=Qwen/Qwen3-8B-Base` or a
   local checkpoint for real research runs).
3. For each trace, computes the per-token logprobs of the actual next-token
   continuation and reduces them two ways:
   - **Aggregate perplexity:** `exp(-mean(logprobs))` — the existing metric.
   - **Per-token rarity:** `exp(-mean(K-lowest logprobs))` — the proposed
     metric, with `K` configurable via `--k` (spec default: 10).
4. Computes Mann–Whitney U discrimination AUC for novel-vs-duplicate against
   each metric. The AUC formula is ported directly from
   `crates/trace-commons-server/src/bin/gate_calibrate/bakeoff_metrics.rs::discrimination_auc`
   so the prototype's numbers are directly comparable to the bake-off binary's
   output.
5. Prints the two AUCs and the delta, plus a one-line recommendation
   (`investigate further` if delta >= 0.05, `marginal` if smaller positive,
   `does not add signal` otherwise — matching the spec's open-question
   threshold).

### Running against a real corpus

```bash
python3 scripts/research/per-token-rarity-prototype.py \
    --corpus=/path/to/bake-off-corpus.tar.zst \
    --model=Qwen/Qwen3-8B-Base \
    --k=10 \
    --out=/tmp/rarity-report.json
```

Requires the operator-managed Python toolchain: `transformers`, `torch`,
either `zstandard` or the `zstd` CLI on `$PATH`.

### Running against the synthetic fixture (CI / plumbing smoke)

```bash
python3 scripts/research/per-token-rarity-prototype.py \
    --corpus=scripts/research/fixtures/synthetic-corpus.tar.zst \
    --model=gpt2 \
    --k=4
```

The synthetic fixture in `fixtures/synthetic-corpus.tar.zst` matches the
format the Rust loader expects (manifest + sorted-filename slices with
verified sha256s) and is small enough to run with a CPU-only `gpt2`.
**Important:** the synthetic fixture's AUC numbers are meaningless — its
"novel" slice is hand-picked rare-vocabulary one-liners and its "duplicate"
slice is generic fluent text, but the corpus is too small and too contrived
to draw any conclusion. Use the synthetic fixture only to validate that the
pipeline runs end-to-end.

### What success looks like

Per the A.5 spec's open question 1: per-token rarity is worth promoting to a
real implementation spec (A.5a) only if it improves AUC over aggregate
perplexity by at least `0.05` on a real bake-off corpus. A smaller delta is
not worth a new floor.

If a real-corpus run shows `delta >= 0.05`, file the A.5a implementation
spec; the production landing site is the `trace-commons-gate-enclave::perplexity_local`
module, where per-token logprobs are already plumbed through from the A2.3
work. The aggregator (currently mean-log → exp) is the only change.

### Why this is a script, not a Rust binary

Phase A.5 is research scoping. We don't want to grow the Rust scorer surface
until we know the metric earns its place. The Python prototype lets us iterate
on `K`, on the aggregation (mean of K-lowest vs other reductions), and on
tokenizer-aware filtering without dragging the whole bake-off harness along
for the ride. Once a metric earns its keep, the production implementation
goes into the gate-enclave's scorer module under a proper spec.
