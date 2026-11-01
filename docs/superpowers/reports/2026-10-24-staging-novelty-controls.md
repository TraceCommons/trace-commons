# Staging novelty controls — October 24, 2026

## Decision and scope

Restore the previously adopted **100,000 micros (0.10) holding value**, with
explicit user approval. Staging was configured at 500,000, the old uncalibrated
launch recommendation. The September 21 report already records the historical
basis and limitations of 0.10. This experiment checks basic duplicate rejection
and distinct-content acceptance; it does **not** fit a new optimal threshold.
No historical consent, decisions, ledger entries, or payout checks are rewritten.

## Reproduction

The checked-in `crates/trace-commons-gate-enclave/examples/novelty_controls.rs`
and adjacent JSON fixtures run the production FastEmbed adapter and standard
orchestrator, chunker, mean-pooled embedding, and novelty aggregation. An isolated
in-memory exact cosine index replaces approximate Usearch. Each case seeds one
reference; query insertions are removed before the next query. It never opens a
production index or database and never calls the hosted scorer.

Perplexity is a mock with its floors disabled. Every input is asserted to have
one chunk, making the scorer's token weights irrelevant to novelty aggregation.
This cannot validate multi-chunk weighting or hosted perplexity. The orchestrator's
novelty floor is zero during measurement so each reference can seed its isolated
index. Reported classifications compare measured scores with the candidate floors
using the same inclusive comparison as production; they are not the orchestrator's
zero-floor `novelty_passed` flag. Rust assertions enforce exact/near rejection and
distinct acceptance at 100000, and all controls remaining below 500000.

```sh
RUSTFLAGS='-D warnings' CARGO_INCREMENTAL=0 cargo build \
  -p trace-commons-gate-enclave --example novelty_controls --features near-ai-scorer
TRACE_COMMONS_EMBEDDER_CACHE_DIR=/path/to/copied/cache \
  target/debug/examples/novelty_controls
```

The staging model cache was copied read-only to an isolated local temporary
folder. Model: `BAAI/bge-large-en-v1.5`, FastEmbed repository
`Xenova/bge-large-en-v1.5`, snapshot
`dfeef6070b90658e1b391a6940efdb0925c1de6f6`, full 1024 dimensions,
512-token model context. The ONNX SHA-256 matched both machines:
`69ed3f810d3b6d13f70dff9ca89966f39c0a0e877fb88211be7bcc070df2a2ce`.

## Results

Build and execution completed successfully. Twelve controls passed the expected
classification checks. The [JSONL artifact](2026-10-24-staging-novelty-controls.jsonl)
contains exact measured scores.

| Control class | n | Novelty micros range | Pass at 100000 | Pass at 500000 |
| --- | --- | --- | --- | --- |
| Exact copy | 3 | 0 | 0 | 0 |
| Light edit | 3 | 138–1369 | 0 | 0 |
| Distinct, same technical domain | 3 | 187455–286357 | 3 | 0 |
| Distinct, different domain | 3 | 372868–431653 | 3 | 0 |

The same-domain controls cover different PostgreSQL, DNS, and Rust archive
problems. At 0.50 every distinct control was rejected; at 0.10 every distinct
control passed and every exact/lightly edited duplicate was rejected.

## Limits and next validation

The fixtures are small and hand-authored. A clear gap in these controls neither
establishes a unique optimum nor guarantees separation on real contributor
traces. Shared prompts, semantically related distinct work, larger tenant
corpora, approximate nearest-neighbor search, and long multi-chunk sessions
require further measurement. The [official model card](https://huggingface.co/BAAI/bge-large-en-v1.5#frequently-asked-questions)'s threshold examples are guidance,
not proof of a universal score range; v1.5 alleviates earlier BGE distribution
concentration.

Novelty rescores against the trace's own indexed vectors can self-match. The
existing perplexity-only rescore endpoint intentionally does not change novelty.
Validate prospective uploads after rollout with unchanged consent and quarantine
checks, and retain exact/near-duplicate controls in future calibration work.
