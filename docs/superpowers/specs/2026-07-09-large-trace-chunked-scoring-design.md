# Large-Trace Chunked Scoring — Design

Date: 2026-07-09
Status: Draft (design approved; spec under review)

## Problem

The perplexity gate scores a decrypted trace envelope by sending its full text
to NEAR AI (`/completions`, `echo: true` + `logprobs`), which returns per-token
logprobs. Two failures result on large traces:

1. **The endpoint crashes.** `echo: true` forces the TEE vLLM backend to compute
   `prompt_logprobs` for every prompt token. A large trace (e.g. a 169 KB Claude
   Code session ≈ 30–40K tokens) with `logprobs: 5` produces a huge memory spike
   and response, OOM-crashing the model worker. NEAR's fleet pins HTTP/2
   connections to specific TEE backends; when one crashes, its pinned connection
   resets and *subsequent requests to that backend fail even when tiny* until the
   trust domain restarts. Observed directly: 15 traces ≤12.5 KB scored cleanly,
   then a 169 KB trace + a burst knocked the backend over and even a 175-byte
   probe got TLS resets during recovery.
2. **Naive truncation loses signal.** Simply sending a prefix would keep requests
   small but discard high-value novel content buried deep in a large trace.

A parallel latent bug: the local embedding-novelty path has **no token-aware
truncation** — it relies on the fastembed model's ~512-token context window, so a
40K-token trace is embedded from only its **first ~512 tokens**. Large traces are
silently embedded from their opening alone.

## Goals

- Score the **whole** trace — no buried-signal loss for either the perplexity or
  the embedding-novelty signal.
- Keep **every** backend request bounded so `echo + prompt_logprobs` can never OOM
  the TEE worker.
- Record both a **representative** (whole-trace) and a **peak** (most-novel-region)
  value for each signal, so an evolving credit model can use either.
- Bounded, predictable cost with a hard per-trace cap and no silent truncation.

## Settled decisions (from brainstorming)

| Question | Decision |
|---|---|
| Score semantics | Record **both** a representative (whole-trace) value and a **peak** (most-novel-region) value, separately, for each signal. |
| Chunking | **Hybrid**: pack consecutive events into bounded windows on turn/message boundaries; fixed character/token-window fallback for a single oversized event or an unrecognized format. |
| Coverage vs cost | **Full coverage** with a **hard per-trace chunk cap** (default 16); if capped, record `chunks_capped` + drop count (hash-only) — never silently drop. |
| Scope | **Both** signals: perplexity (NEAR) and embedding novelty (local). |
| Vector index | **Per-chunk entries**, capped per trace, with near-duplicate dedup on insert. |
| Per-chunk error | **Fail-closed** for v1 — any chunk scoring/embedding error fails the whole evaluation; the driver's idempotent re-scoring + backoff retries. |

## Architecture

A new shared component, `TraceChunker`, is the single place that guarantees no
oversized backend request. The orchestrator drives a per-chunk scoring loop and
aggregates. The `PerplexityScorer` / `TextEmbedder` traits are unchanged — they
still operate on one chunk of text.

```
evaluate(envelope_plaintext, tenant_storage_ref):
  events   = parse TraceContributionEnvelope(envelope_plaintext).events
  chunks   = TraceChunker::chunk(events)         # ≤ cap, each ≤ budget
  per_chunk = []
  for chunk in chunks (sequential):              # never a burst
     perp  = perplexity.score(chunk.text)        # one NEAR call, echo+logprobs
     emb   = mean_pool(embedder.embed(sub_windows_of(chunk)))  # no 512 truncation
     nov   = 1 - max_cos_sim(emb, tenant per-chunk index)
     per_chunk.push({sum_nll, n, logprobs, emb, nov, tokens})
  decision = aggregate(per_chunk)                # representative + peak
  insert_index_entries(per_chunk)                # per-chunk, deduped, capped
  return decision
```

### `TraceChunker`

- **Input:** the structured `TraceContributionEnvelope { events: Vec<TraceContributionEvent> }` — parsed, so chunking is semantic, not byte-blind.
- **Canonical text rendering:** each event is rendered to a canonical *text* form
  (role/kind + content), **not** raw JSON — JSON braces/keys would dilute the
  perplexity signal. This rendering is shared by both signals so they see
  identical text.
- **Semantic packing:** walk events in order; greedily append each rendered event
  to the current chunk until adding the next would exceed the **target budget**;
  then start a new chunk. Chunks respect event boundaries.
- **Fixed fallback:** a single rendered event larger than the hard max is split
  into fixed character-window sub-chunks. Same fallback path is used if the
  envelope carries no usable event structure.
- **Budget (evidence-based):** target ~**2048 tokens (~8 KB text)** per chunk,
  hard max ~**3072 tokens (~12 KB)**. The 15 traces that scored cleanly were all
  ≤12.5 KB; the 169 KB trace crashed. Configurable.
- **Hard cap:** at most **N chunks per trace** (default 16). Beyond it, score the
  first N and set `chunks_capped = true` with the dropped-chunk count logged
  hash-only. Never silent.

Note: token budgets are enforced by a cheap char-length proxy (no local
tokenizer dependency); the constants are chosen with margin so the proxy error
cannot push a chunk past the backend's safe size.

## Aggregation

### Perplexity (per chunk → trace)

Each chunk call returns per-token logprobs. Keep per chunk `c`:
`sum_nll_c = −Σ logprob` over usable tokens (token 0 dropped, as today), and
`n_c` = usable token count.

- **Representative** = `exp( Σ_c sum_nll_c / Σ_c n_c )` — the token-weighted
  whole-trace mean-NLL perplexity. Equals a single whole-trace call, modulo one
  dropped context-token per chunk boundary (with ~2K-token chunks, <0.05% of
  tokens — an accepted, bounded approximation). Stored in the existing
  `perplexity_micros`.
- **Peak** = `max_c exp(sum_nll_c / n_c)` over chunks passing the **min-content
  guard** (≥ ~64 tokens), so a tiny surprising fragment can't spike the peak.
  Stored in new `peak_perplexity_micros`.
- **`tail_fraction`** (representative) = `Σ_c surprising_tokens_c / Σ_c n_c` —
  exact whole-trace fraction of tokens below the surprise cutoff. Existing
  `tail_fraction_micros`.
- **`rarity`** (global top-K): concatenate per-token logprobs across *all* chunks,
  take the K globally-rarest, `exp(−mean(K rarest))`. Computed over the whole
  trace — no loss. Retained per-token logprobs are bounded by cap × budget.

### Embedding novelty (per chunk → trace)

The chunk is ~2K tokens but fastembed truncates at ~512, which would re-truncate
*inside* each chunk. Fix: a **chunk's embedding = mean-pool of its ≤512-token
embedding sub-windows**, so the vector reflects the whole chunk with no
truncation.

- Per-chunk novelty = `1 − max cosine-sim(chunk_emb, existing per-chunk index
  entries in the tenant)`.
- **Representative** = token-weighted mean of chunk novelties. Existing
  `novelty_score_micros`.
- **Peak** = `max_c` chunk novelty over min-content-guarded chunks. New
  `peak_novelty_micros`.
- **Index writes:** insert each chunk's embedding as its own entry tagged
  `(submission_id, chunk_index)`; **skip near-duplicates** whose novelty is below
  an insert threshold; honor the per-trace chunk cap. This bounds index growth.

### Peak granularity

The **peak operates at chunk (~2K-token) granularity for both signals** —
perplexity uses the full chunk; embedding mean-pools the chunk's sub-windows.
Consistent peak semantics across signals.

## Schema

New nullable columns on `trace_gate_decisions` (migration V37):

- `peak_perplexity_micros BIGINT`
- `peak_novelty_micros BIGINT`
- `chunk_count INT`
- `chunks_capped BOOLEAN`

The existing `perplexity_micros` / `novelty_score_micros` / `tail_fraction_micros`
continue to hold the **representative** values — backward-compatible: the 15
already-scored rows read as single-chunk traces (`chunk_count = 1`,
`peak = representative`, `chunks_capped = false`) via nullable/`COALESCE`
semantics. No re-scoring of existing rows.

The vector store gains a per-chunk entry shape `(submission_id, chunk_index)`
instead of one-per-submission; existing entries are treated as `chunk_index = 0`.
RLS-forced like every Trace Commons table, tenant-scoped as today.

## Config (env, safe defaults; chunking is always on)

- `TRACE_COMMONS_GATE_CHUNK_TARGET_TOKENS` — default `2048`.
- `TRACE_COMMONS_GATE_CHUNK_MAX_TOKENS` — hard per-chunk max, default `3072`.
- `TRACE_COMMONS_GATE_CHUNK_CAP` — max chunks per trace, default `16`.
- `TRACE_COMMONS_GATE_CHUNK_MIN_TOKENS` — peak-eligibility guard, default `64`.
- `TRACE_COMMONS_GATE_EMBED_INSERT_NOVELTY_MICROS` — dedup insert threshold,
  default `50000` (0.05): a chunk whose novelty is below this is a near-duplicate
  of an existing entry and is not inserted.
- `logprobs_top_k` default lowered **5 → 1** (perplexity needs only the realized
  token's logprob; cuts backend memory + response ~5×).

There is no feature flag — this is the scoring path. A small trace is one chunk
and behaves exactly as before.

## Error handling & safety

- **Fail-closed (v1):** any chunk's scorer/embedder error fails the whole
  evaluation, records nothing, and the driver re-scores later with backoff.
  (Partial-tolerance — record successes, retry only gaps — is a possible future
  optimization; out of scope for v1.)
- **No OOM:** every chunk is bounded, so `echo + prompt_logprobs` can't overrun
  the backend. This is the root-cause fix for the endpoint crashes.
- **Sequential per trace:** chunks scored one at a time, so a single trace can't
  pile concurrent heavy requests onto one pinned backend.
- **Hash-only logging:** `chunk_count`, `chunks_capped`, drop count, and existing
  hash-safe error labels only. Never chunk content, trace bodies, or the NEAR AI
  response body.

## Testing

- **Unit (CI):**
  - Chunker: semantic packing on event boundaries; oversized-event fixed
    fallback; cap enforcement + drop count; min-content guard.
  - Perplexity aggregation: a single-chunk trace's representative == the
    whole-trace value within float tolerance (**proves no mean signal loss**);
    token-weighted-mean correctness across chunks; peak = max eligible chunk;
    `tail_fraction` exact; global top-K rarity over concatenated logprobs.
  - Embedding: per-chunk mean-pool covers the whole chunk (no 512 truncation);
    representative mean + peak max; near-duplicate insert is skipped at the
    threshold.
- **Integration (CI):** a multi-chunk trace with a mock scorer + mock embedder
  drains, aggregates, enforces the cap, and **fails closed** when one chunk
  errors. No live NEAR AI in tests.
- **PG-gated:** the per-chunk vector-entry shape + new decision columns under
  tenant RLS.
- **Manual (pilot):** the 169 KB trace (`bfd6d37d`) and the other 4 previously-
  failing traces score without crashing the endpoint; verify representative +
  peak recorded and `chunk_count > 1` for the large one.

## Non-goals (explicitly out of scope)

- Changing the gate floor / enabling gating. Floor stays 0 (record-only).
- Cross-chunk contextual scoring — each chunk is scored independently.
- Re-scoring already-scored submissions (idempotent unless their attempt rows are
  reset).
- Parsing below the event level (no per-tool-argument structure).
- Vector-index compaction / GC at scale beyond the per-trace cap + dedup — index
  growth management for high volume is a tracked follow-up.
- Partial-tolerant per-chunk retry (v1 is fail-closed).
