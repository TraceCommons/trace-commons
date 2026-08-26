# Large-Trace Chunked Scoring

The gate scores every trace in bounded chunks so no NEAR AI
`echo + prompt_logprobs` request can OOM the TEE backend. Chunking is
always on; a small trace is one chunk and behaves as before.

**Scoring a large trace is lossy.** A trace that packs into more chunks
than `TRACE_COMMONS_GATE_CHUNK_CAP` does NOT contribute its full content
to either signal — the cap is a hard bound on how many scorer requests one
trace may cost, and the scorer has no retry, so raising it multiplies both
latency and fail-closed failure exposure. What the cap does guarantee is
that the scored sample is *unbiased in position*: the gate keeps an evenly
strided subset spanning the whole trace, endpoint-inclusive, rather than
its first N chunks.

Concretely, when a trace packs into `total > cap` chunks, the gate scores
exactly `cap` of them at positions `round(j * (total - 1) / (cap - 1))`
for `j` in `0..cap`. The trace's first and last chunks are always among
them. The selection is pure integer arithmetic — no RNG, no clock — so the
same trace always yields the same chunks and the same score.

This replaced prefix truncation, which kept chunks `0..cap` and so judged
every long trace on its opening: the system prompt, the environment
banner, and the first file reads, which are the most boilerplate and the
most repeated part across one contributor's sessions. On the pilot,
capped traces passed the gate at roughly 11% against roughly 65% for
uncapped ones, consistently across months.

`chunk_index` on a per-chunk vector entry is the chunk's **original**
position in the trace, not its position within the surviving set. Under a
cap those indices are sparse (e.g. `0, 7, 13, ...`) and do not run
contiguously; nothing keys off contiguity. Note they were already sparse
before this change, because only chunks clearing
`TRACE_COMMONS_GATE_EMBED_INSERT_NOVELTY_MICROS` get an entry at all.

Changing the selection algorithm changes the `chunk_selection=` field of
the gate version canonical string and therefore `gate_version_hash`, so
decisions taken under different selection arithmetic are never
indistinguishable under one stamp.

## Knobs (env, safe defaults)

| Env var | Default | Meaning |
|---|---|---|
| `TRACE_COMMONS_GATE_CHUNK_TARGET_TOKENS` | `2048` | Greedy packing target per chunk (~8 KB text; char-proxy, no tokenizer). |
| `TRACE_COMMONS_GATE_CHUNK_MAX_TOKENS` | `3072` | Hard per-chunk max (~12 KB). A single larger event splits into fixed char windows. |
| `TRACE_COMMONS_GATE_CHUNK_CAP` | `16` | Max chunks per trace, i.e. max scorer requests per trace. Beyond it the gate scores an evenly strided, endpoint-inclusive subset of exactly `cap` chunks spanning the whole trace; the rest are dropped, `chunks_capped` is set, and the drop count is logged hash-only. Never silent. |
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
