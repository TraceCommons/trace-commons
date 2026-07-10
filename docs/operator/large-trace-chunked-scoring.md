# Large-Trace Chunked Scoring

The gate scores every trace in bounded chunks so no NEAR AI
`echo + prompt_logprobs` request can OOM the TEE backend, and so large
traces contribute their full content to both signals (no truncation).
Chunking is always on; a small trace is one chunk and behaves as before.

## Knobs (env, safe defaults)

| Env var | Default | Meaning |
|---|---|---|
| `TRACE_COMMONS_GATE_CHUNK_TARGET_TOKENS` | `2048` | Greedy packing target per chunk (~8 KB text; char-proxy, no tokenizer). |
| `TRACE_COMMONS_GATE_CHUNK_MAX_TOKENS` | `3072` | Hard per-chunk max (~12 KB). A single larger event splits into fixed char windows. |
| `TRACE_COMMONS_GATE_CHUNK_CAP` | `16` | Max chunks per trace. Beyond it, trailing chunks are dropped and `chunks_capped` is recorded with a hash-only drop count in logs. Never silent. |
| `TRACE_COMMONS_GATE_CHUNK_MIN_TOKENS` | `64` | Min scored tokens for a chunk to be peak-eligible (blocks tiny-fragment peak spikes). |
| `TRACE_COMMONS_GATE_EMBED_INSERT_NOVELTY_MICROS` | `50000` | Per-chunk index-insert dedup threshold: chunks below it are near-duplicates and are not inserted. |

NEAR AI requests send `logprobs: 1` by default (was 5) — perplexity needs
only the realized token's logprob.

## Decision-row semantics (migration V37)

- `perplexity_micros` / `novelty_score_micros` / `tail_fraction_micros`:
  representative (token-weighted whole-trace) values, as before.
- `peak_perplexity_micros` / `peak_novelty_micros`: most-surprising /
  most-novel min-content-guarded chunk. NULL on pre-V37 rows (read as
  "peak = representative").
- `chunk_count` (NULL reads as 1), `chunks_capped` (NULL reads as false).
- Per-chunk vector-index entries live in `trace_gate_chunk_vector_entries`
  keyed `(tenant_id, decision_id, chunk_index)`; the decision row's
  `vector_entry_id` holds the first inserted entry for back-compat.

## Revocation

Revocation drives two distinct vector-invalidation mechanisms, not one:

- **Per-chunk entries (V37+)**: `enqueue_vector_entry_invalidation_items_for_revocation`
  reads `trace_gate_chunk_vector_entries` for the submission and enqueues one
  `InvalidateVector` propagation-queue item per recorded chunk entry
  (idempotent per entry id). These items are retry/attempt-tracked like the
  rest of the revocation propagation queue; the propagation worker's
  `InvalidateVector` branch consumes them unchanged, one vector-entry id per
  item.
- **Legacy pre-V37 rows**: `invalidate_trace_vector_entries_for_submission`
  is a synchronous, direct `UPDATE trace_vector_entries ... SET status =
  invalidated` executed inline during revocation. It does **not** create a
  propagation-queue row and carries no retry/attempt tracking — it either
  commits as part of the revocation request or the request fails. Pre-V37
  decisions have no chunk rows in `trace_gate_chunk_vector_entries`, so this
  is the only invalidation path that reaches them; for V37+ submissions this
  call is a no-op because their entries live in the per-chunk table instead.

Operators diagnosing a stuck revocation should check chunk-entry propagation
items (queued, retried, may lag) separately from the legacy single-entry
update (synchronous, either already done or the revocation call itself
failed).

## Failure semantics

Fail-closed v1: any chunk's scorer/embedder error fails the whole
evaluation; the scoring driver retries with backoff via
`trace_gate_evaluation_attempts`. Chunks are scored sequentially per trace —
never a concurrent burst against one pinned backend.
