# Large-Trace Chunked Scoring Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Score whole traces of any size through the perplexity + embedding-novelty gate by chunking the parsed envelope into bounded windows, so no NEAR AI request can OOM the TEE backend and no buried signal is lost, recording both representative and peak values per signal.

**Architecture:** A new pure `TraceChunker` in `trace-commons-gate-enclave` parses the contribution-envelope JSON, renders each event to canonical text, and greedily packs events into ≤16 bounded chunks (char-length token proxy, no tokenizer dep). `EnclaveGateOrchestrator::evaluate` loops chunks sequentially (fail-closed), scoring perplexity per chunk via a new `PerplexityScorer::score_chunk` method that exposes `(sum_nll, n, tail_tokens, logprobs)`, and embedding each chunk via mean-pooled ≤512-token sub-windows. Pure aggregation helpers produce representative (token-weighted whole-trace) and peak (min-content-guarded max) values. The host maps four new `OrchestrationDecision` fields into four new nullable `trace_gate_decisions` columns (migration V37) and persists per-chunk vector-index entries in a new RLS-forced `trace_gate_chunk_vector_entries` table keyed `(tenant_id, decision_id, chunk_index)` for revocation tracking.

**Tech Stack:** Rust; `trace-commons-gate-enclave` crate (anyhow, sha2, uuid, serde_json); `trace-commons-server` crate (tokio-postgres via existing `Database` trait); PostgreSQL migrations with forced RLS.

## Global Constraints

- Postgres-only repo; single backend. Do NOT add libsql/dual-feature builds. `cargo check -p trace-commons-server` suffices.
- CI enforces warnings-as-errors: verify with `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins` and `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run`. For the gate-enclave crate use `--features near-ai-scorer` where the NEAR path is touched.
- Clippy allow-list (run locally): `cargo clippy -p <crate> --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`. Do not widen it.
- Hash-only/label-only audit + logging: never log chunk content, trace bodies, contributor identity, raw URLs/tokens, or NEAR response bodies. Only counts, hashes, and fixed labels.
- No emojis in commits/PRs/code. Short imperative commit subjects, no `feat:`/`fix:` prefixes (match repo style).
- Migrations live in `migrations/`; new one is `V37__...`. RLS forced on every table via `trace_current_tenant_id()`.
- No new dependencies without flagging; prefer stdlib + existing deps + inline utilities. If a task appears to need a new crate, STOP and note it as an open question in the plan rather than adding it.

**Dependency note (flagged up front, not an open question):** Task 1 flips `serde_json` in `crates/trace-commons-gate-enclave/Cargo.toml` from `optional = true` to a required dependency. No new crate enters the tree — `serde_json` is already a workspace dependency and already compiled into every feature build of this crate; only the default (no-features) build of `trace-commons-gate-enclave` newly links it. This is required because the always-compiled orchestrator must parse envelope JSON to chunk on event boundaries. Surface this Cargo.toml change explicitly in the PR description per the dependency policy. The alternative — a path dependency on `trace-commons-protocol` — was rejected: that crate drags tokio/regex/rust_decimal into the enclave crate.

---

## File Structure

| File | Action | Responsibility |
|---|---|---|
| `crates/trace-commons-gate-enclave/src/chunker.rs` | Create | Pure `TraceChunker`: envelope-JSON event parsing (lenient, `serde_json::Value`), canonical event text rendering, greedy semantic packing, oversized-event fixed char-window fallback, chunk cap + drop count. No I/O, no decryption. |
| `crates/trace-commons-gate-enclave/src/chunk_aggregate.rs` | Create | Pure aggregation: `ChunkedPerplexityAggregate` (representative/peak/tail), `global_rarity_micros_across_chunks`, token-weighted novelty representative/peak helpers. |
| `crates/trace-commons-gate-enclave/src/perplexity.rs` | Modify | Add `ChunkPerplexity` result type + `PerplexityScorer::score_chunk` provided method (default derives from `score()`). |
| `crates/trace-commons-gate-enclave/src/perplexity_near_ai.rs` | Modify | Native `score_chunk` from `fetch_logprobs`; `logprobs_top_k` doc + test updates (5 -> 1 default lives in ingest.rs const). |
| `crates/trace-commons-gate-enclave/src/embedder.rs` | Modify | Add `embed_chunk_mean_pooled` (≤512-token sub-window mean-pool, L2-renormalized). |
| `crates/trace-commons-gate-enclave/src/orchestrator.rs` | Modify | Chunk loop in `evaluate` (perplexity first, then embedding), new `OrchestrationDecision` fields (`peak_perplexity_micros`, `peak_novelty_micros`, `chunk_count`, `chunks_capped`, `inserted_chunk_entries`), new `EnclaveGateOrchestratorConfig` chunk knobs, per-chunk index insert with dedup threshold. |
| `crates/trace-commons-gate-enclave/src/lib.rs` | Modify | Export new modules/types. |
| `crates/trace-commons-gate-enclave/Cargo.toml` | Modify | `serde_json` optional -> required (flagged above). |
| `migrations/V37__large_trace_chunked_scoring.sql` | Create | 4 nullable columns on `trace_gate_decisions`; new `trace_gate_chunk_vector_entries` table, RLS-forced. |
| `crates/trace-commons-server/src/db/postgres.rs` | Modify | Register V37 in the idempotent migration runner (pattern at the V36 block near line 1126). |
| `crates/trace-commons-server/src/trace_gate_service.rs` | Modify | `GateDecision` gains peak/chunk fields + `chunk_vector_entries`; all four `TraceGateService` impls updated. |
| `crates/trace-commons-server/src/trace_corpus_storage.rs` | Modify | `TraceGateDecisionRow` gains 4 nullable fields; new `TraceGateChunkVectorEntryRow`; `Database` trait gains `insert_trace_gate_decision_with_chunk_entries` + `list_trace_gate_chunk_vector_entries` (both with defaults). |
| `crates/trace-commons-server/src/db/trace_corpus_pg.rs` | Modify | Extend insert/select column lists (~5258-5290, ~5344-5384, ~5440-5476); implement the two new trait methods atomically. |
| `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` | Modify | Chunk-knob env parsing (near line 226 consts / lines 4587+ and 4730+ builders); `TRACE_COMMONS_NEAR_AI_DEFAULT_LOGPROBS_TOP_K` 5 -> 1 (line 279); `evaluate_and_record_gate` mapping (~44861-44900); synthetic-row sites (~44934, ~45036, ~45266); revocation enqueue planner beside `enqueue_worker_queue_invalidation_items_for_revocation` (~50035). |
| `docs/operator/large-trace-chunked-scoring.md` | Create | Operator runbook: knobs, defaults, chunks_capped meaning, revocation notes. |
| `docs/operator/README.md` | Modify | Index line for the new runbook. |

Interface names used across tasks (single source of truth):

- `chunker::ChunkerConfig { target_tokens: usize, max_tokens: usize, chunk_cap: usize }`
- `chunker::TraceChunk { chunk_index: u32, text: String }`
- `chunker::ChunkPlan { chunks: Vec<TraceChunk>, chunks_capped: bool, dropped_chunk_count: u32 }`
- `chunker::chunk_envelope_plaintext(plaintext: &[u8], cfg: &ChunkerConfig) -> ChunkPlan`
- `perplexity::ChunkPerplexity { sum_nll: f64, tokens: u64, tail_tokens: u64, logprobs: Vec<f32> }`
- `PerplexityScorer::score_chunk(&self, chunk: &[u8]) -> anyhow::Result<ChunkPerplexity>`
- `chunk_aggregate::ChunkedPerplexityAggregate { representative_perplexity_micros: u64, peak_perplexity_micros: u64, tail_fraction_micros: u64, tokens_scored: u64 }`
- `chunk_aggregate::aggregate_chunked_perplexity(chunks: &[ChunkPerplexity], min_chunk_tokens: u64) -> ChunkedPerplexityAggregate`
- `chunk_aggregate::global_rarity_micros_across_chunks(chunks: &[ChunkPerplexity], k: usize) -> u64`
- `chunk_aggregate::aggregate_chunked_novelty(novelty_micros: &[u64], chunk_tokens: &[u64], min_chunk_tokens: u64) -> (u64, u64)` (representative, peak)
- `embedder::embed_chunk_mean_pooled<E: Embedder + ?Sized>(embedder: &E, chunk_text: &str) -> anyhow::Result<Vec<f32>>`
- `orchestrator::InsertedChunkEntry { chunk_index: u32, entry_id: Uuid }`
- `trace_gate_service::GateChunkVectorEntry { chunk_index: u32, vector_entry_id: Uuid }`
- `trace_corpus_storage::TraceGateChunkVectorEntryRow { decision_id: Uuid, submission_id: Uuid, chunk_index: i32, vector_entry_id: Uuid }`

---

### Task 1: TraceChunker with canonical event rendering

**Files:**
- Create: `crates/trace-commons-gate-enclave/src/chunker.rs`
- Modify: `crates/trace-commons-gate-enclave/Cargo.toml` (serde_json optional -> required; remove `"dep:serde_json"` from both feature lists)
- Modify: `crates/trace-commons-gate-enclave/src/lib.rs` (add `pub mod chunker;`)

**Interfaces:**
- Consumes: nothing from other tasks. Parses envelope JSON leniently via `serde_json::Value` (fields `events[].event_type`, `events[].tool_name`, `events[].redacted_content` — mirrors `TraceContributionEvent` in `crates/trace-commons-protocol/src/trace_contribution.rs:173` without importing that crate).
- Produces: `ChunkerConfig`, `TraceChunk`, `ChunkPlan`, `chunk_envelope_plaintext`, `chunk_rendered_events`, `render_event_text`, `parse_envelope_rendered_events`, `APPROX_CHARS_PER_TOKEN` — consumed by Tasks 4, 7, 9.

- [ ] **Step 1: Flip serde_json to a required dependency**

In `crates/trace-commons-gate-enclave/Cargo.toml`, change:

```toml
serde_json = { version = "1", optional = true }
```

to:

```toml
# Required (not optional): the always-compiled TraceChunker parses the
# contribution-envelope JSON to chunk on event boundaries. Already a
# workspace dep; no new crate. Flagged in the PR description.
serde_json = "1"
```

and delete the `"dep:serde_json",` line from BOTH the `local-gpu-models` and `near-ai-scorer` feature arrays (a non-optional dep cannot appear in a feature list; the build fails otherwise).

- [ ] **Step 2: Write the failing tests**

Create `crates/trace-commons-gate-enclave/src/chunker.rs` with the module doc, the test module below, and NO implementation yet (types + `todo!()` bodies are fine, or write tests first in the same file and let them fail to compile — either way the test run must fail before the implementation lands):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(target_tokens: usize, max_tokens: usize, chunk_cap: usize) -> ChunkerConfig {
        ChunkerConfig {
            target_tokens,
            max_tokens,
            chunk_cap,
        }
    }

    fn envelope_json(contents: &[&str]) -> Vec<u8> {
        // Minimal envelope shape: only the fields the lenient parser reads.
        let events: Vec<serde_json::Value> = contents
            .iter()
            .enumerate()
            .map(|(i, c)| {
                serde_json::json!({
                    "event_type": if i % 2 == 0 { "user_message" } else { "assistant_message" },
                    "redacted_content": c,
                })
            })
            .collect();
        serde_json::to_vec(&serde_json::json!({ "events": events })).unwrap()
    }

    #[test]
    fn render_event_text_is_role_plus_content_not_json() {
        let rendered = render_event_text("tool_call", Some("Bash"), "ls -la");
        assert_eq!(rendered, "tool_call (Bash): ls -la\n");
        let rendered = render_event_text("user_message", None, "hello");
        assert_eq!(rendered, "user_message: hello\n");
        assert!(!rendered.contains('{'), "rendering must not be raw JSON");
    }

    #[test]
    fn small_envelope_is_a_single_chunk() {
        let plaintext = envelope_json(&["hello", "world"]);
        let plan = chunk_envelope_plaintext(&plaintext, &cfg(2048, 3072, 16));
        assert_eq!(plan.chunks.len(), 1);
        assert!(!plan.chunks_capped);
        assert_eq!(plan.dropped_chunk_count, 0);
        assert_eq!(plan.chunks[0].chunk_index, 0);
        assert!(plan.chunks[0].text.contains("user_message: hello"));
        assert!(plan.chunks[0].text.contains("assistant_message: world"));
    }

    #[test]
    fn packing_respects_event_boundaries() {
        // target 8 tokens = 32 chars. Each rendered event is
        // "user_message: aaaaaaaaaa\n" = 25 chars, so exactly one event fits
        // per chunk (a second would exceed 32 chars).
        let e = "aaaaaaaaaa";
        let plaintext = envelope_json(&[e, e, e]);
        let plan = chunk_envelope_plaintext(&plaintext, &cfg(8, 16, 16));
        assert_eq!(plan.chunks.len(), 3);
        for (i, chunk) in plan.chunks.iter().enumerate() {
            assert_eq!(chunk.chunk_index, i as u32);
            // Every chunk starts at an event boundary (a rendered label).
            assert!(
                chunk.text.starts_with("user_message: ")
                    || chunk.text.starts_with("assistant_message: "),
                "chunk must start on an event boundary, got {:?}",
                &chunk.text[..20.min(chunk.text.len())]
            );
        }
    }

    #[test]
    fn oversized_single_event_splits_by_fixed_char_windows() {
        // One event whose rendered form far exceeds max (16 tokens = 64
        // chars). Must split into target-sized (8 tokens = 32 chars) windows.
        let big = "x".repeat(300);
        let plaintext = envelope_json(&[&big]);
        let plan = chunk_envelope_plaintext(&plaintext, &cfg(8, 16, 100));
        assert!(plan.chunks.len() > 1, "oversized event must split");
        for chunk in &plan.chunks {
            assert!(
                chunk.text.chars().count() <= 16 * APPROX_CHARS_PER_TOKEN,
                "no chunk may exceed the hard max"
            );
        }
        // No content lost: total non-boundary chars preserved.
        let total: usize = plan.chunks.iter().map(|c| c.text.chars().count()).sum();
        assert!(total >= 300, "split must not drop content");
    }

    #[test]
    fn oversized_split_respects_utf8_char_boundaries() {
        // Multi-byte chars: a byte-index split would panic or shear a char.
        let big = "\u{00e9}".repeat(300); // 'e-acute', 2 bytes each
        let plaintext = envelope_json(&[&big]);
        let plan = chunk_envelope_plaintext(&plaintext, &cfg(8, 16, 100));
        assert!(plan.chunks.len() > 1);
        for chunk in &plan.chunks {
            // If a char were sheared, String construction would have panicked
            // already; assert the content is intact e-acute runs.
            assert!(chunk.text.chars().all(|c| c == '\u{00e9}'
                || c.is_ascii_alphanumeric()
                || c == ':'
                || c == ' '
                || c == '_'
                || c == '\n'));
        }
    }

    #[test]
    fn cap_enforced_with_drop_count() {
        let e = "b".repeat(100);
        let contents: Vec<String> = (0..10).map(|_| e.clone()).collect();
        let refs: Vec<&str> = contents.iter().map(|s| s.as_str()).collect();
        // target 25 tokens = 100 chars: one event per chunk -> 10 chunks; cap 4.
        let plaintext = envelope_json(&refs);
        let plan = chunk_envelope_plaintext(&plaintext, &cfg(25, 50, 4));
        assert_eq!(plan.chunks.len(), 4);
        assert!(plan.chunks_capped);
        assert_eq!(plan.dropped_chunk_count, 6);
    }

    #[test]
    fn non_json_plaintext_falls_back_to_fixed_windows() {
        let raw = "z".repeat(200);
        let plan = chunk_envelope_plaintext(raw.as_bytes(), &cfg(8, 16, 100));
        assert!(plan.chunks.len() > 1, "fallback must window raw text");
        let total: usize = plan.chunks.iter().map(|c| c.text.chars().count()).sum();
        assert_eq!(total, 200);
    }

    #[test]
    fn json_without_events_falls_back() {
        let plan =
            chunk_envelope_plaintext(br#"{"schema_version":"x"}"#, &cfg(2048, 3072, 16));
        assert_eq!(plan.chunks.len(), 1, "no-events JSON falls back to raw text");
    }

    #[test]
    fn empty_plaintext_yields_single_empty_chunk() {
        let plan = chunk_envelope_plaintext(b"", &cfg(2048, 3072, 16));
        assert_eq!(plan.chunks.len(), 1);
        assert_eq!(plan.chunks[0].text, "");
        assert!(!plan.chunks_capped);
    }

    #[test]
    fn chunking_is_deterministic() {
        let plaintext = envelope_json(&["alpha", "beta", "gamma"]);
        let a = chunk_envelope_plaintext(&plaintext, &cfg(8, 16, 16));
        let b = chunk_envelope_plaintext(&plaintext, &cfg(8, 16, 16));
        assert_eq!(a, b);
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p trace-commons-gate-enclave chunker -- --nocapture`
Expected: FAIL (compile error: types/functions not defined, or `todo!()` panics).

- [ ] **Step 4: Write the implementation**

Fill in `crates/trace-commons-gate-enclave/src/chunker.rs` above the test module:

```rust
//! TraceChunker: split a contribution-envelope plaintext into bounded text
//! chunks for per-chunk gate scoring.
//!
//! Pure and deterministic — no I/O, no decryption, no tokenizer dependency.
//! Token budgets are enforced by a char-length proxy
//! (`APPROX_CHARS_PER_TOKEN`); the production constants carry enough margin
//! that proxy error cannot push a chunk past the backend's safe size.
//!
//! Hash-only logging convention: this module never logs. Callers may log
//! chunk COUNTS only, never chunk text.

/// Char-per-token proxy (~4 chars/token for English/code text). Shared by
/// the chunker and the embedding sub-window helper so both budgets scale
/// identically.
pub const APPROX_CHARS_PER_TOKEN: usize = 4;

/// Chunking budgets, expressed in tokens (converted internally via
/// [`APPROX_CHARS_PER_TOKEN`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkerConfig {
    /// Greedy packing target per chunk (default 2048 ≈ 8 KB).
    pub target_tokens: usize,
    /// Hard per-chunk maximum (default 3072 ≈ 12 KB). A single rendered
    /// event larger than this splits into fixed `target_tokens`-sized
    /// char windows.
    pub max_tokens: usize,
    /// Hard cap on chunks per trace (default 16). Beyond it, later chunks
    /// are dropped and counted — never silently.
    pub chunk_cap: usize,
}

impl ChunkerConfig {
    fn target_chars(&self) -> usize {
        self.target_tokens.saturating_mul(APPROX_CHARS_PER_TOKEN).max(1)
    }
    fn max_chars(&self) -> usize {
        self.max_tokens.saturating_mul(APPROX_CHARS_PER_TOKEN).max(1)
    }
}

/// One bounded scoring window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceChunk {
    pub chunk_index: u32,
    pub text: String,
}

/// The full chunking outcome for one trace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkPlan {
    pub chunks: Vec<TraceChunk>,
    pub chunks_capped: bool,
    pub dropped_chunk_count: u32,
}

/// Render one event to its canonical text form: `kind (tool): content\n` or
/// `kind: content\n`. Shared by both signals so they score identical text.
/// Intentionally NOT raw JSON — braces/keys would dilute the perplexity
/// signal.
pub fn render_event_text(event_type: &str, tool_name: Option<&str>, content: &str) -> String {
    match tool_name {
        Some(t) if !t.is_empty() => format!("{event_type} ({t}): {content}\n"),
        _ => format!("{event_type}: {content}\n"),
    }
}

/// Leniently parse the envelope JSON and render its events. Returns `None`
/// when the plaintext is not JSON, has no `events` array, or the array is
/// empty — callers fall back to fixed-window chunking of the raw text.
pub fn parse_envelope_rendered_events(plaintext: &[u8]) -> Option<Vec<String>> {
    let v: serde_json::Value = serde_json::from_slice(plaintext).ok()?;
    let events = v.get("events")?.as_array()?;
    if events.is_empty() {
        return None;
    }
    Some(
        events
            .iter()
            .map(|e| {
                let event_type = e
                    .get("event_type")
                    .and_then(|x| x.as_str())
                    .unwrap_or("event");
                let tool_name = e.get("tool_name").and_then(|x| x.as_str());
                let content = e
                    .get("redacted_content")
                    .and_then(|x| x.as_str())
                    .unwrap_or("");
                render_event_text(event_type, tool_name, content)
            })
            .collect(),
    )
}

/// Split `text` into fixed windows of at most `window_chars` CHARS (not
/// bytes) — UTF-8-boundary safe.
fn split_fixed_char_windows(text: &str, window_chars: usize) -> Vec<String> {
    let window_chars = window_chars.max(1);
    let mut out = Vec::new();
    let mut current = String::new();
    let mut count = 0usize;
    for ch in text.chars() {
        current.push(ch);
        count += 1;
        if count == window_chars {
            out.push(std::mem::take(&mut current));
            count = 0;
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// Greedily pack consecutive rendered events into chunks of at most
/// `target_chars`, respecting event boundaries. A single event larger than
/// `max_chars` splits into `target_chars` fixed windows. Applies the cap.
pub fn chunk_rendered_events(events: &[String], cfg: &ChunkerConfig) -> ChunkPlan {
    let target = cfg.target_chars();
    let max = cfg.max_chars();
    let mut texts: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_chars = 0usize;
    for event in events {
        let event_chars = event.chars().count();
        if event_chars > max {
            // Oversized event: flush the open chunk, then fixed windows.
            if !current.is_empty() {
                texts.push(std::mem::take(&mut current));
                current_chars = 0;
            }
            texts.extend(split_fixed_char_windows(event, target));
            continue;
        }
        if !current.is_empty() && current_chars + event_chars > target {
            texts.push(std::mem::take(&mut current));
            current_chars = 0;
        }
        current.push_str(event);
        current_chars += event_chars;
    }
    if !current.is_empty() {
        texts.push(current);
    }
    if texts.is_empty() {
        texts.push(String::new());
    }
    finalize_plan(texts, cfg)
}

fn finalize_plan(mut texts: Vec<String>, cfg: &ChunkerConfig) -> ChunkPlan {
    let cap = cfg.chunk_cap.max(1);
    let total = texts.len();
    let (chunks_capped, dropped_chunk_count) = if total > cap {
        texts.truncate(cap);
        (true, (total - cap) as u32)
    } else {
        (false, 0)
    };
    ChunkPlan {
        chunks: texts
            .into_iter()
            .enumerate()
            .map(|(i, text)| TraceChunk {
                chunk_index: i as u32,
                text,
            })
            .collect(),
        chunks_capped,
        dropped_chunk_count,
    }
}

/// Top-level entry: parse the envelope's events and pack semantically; fall
/// back to fixed char windows over the (lossy-UTF-8) raw text when the
/// plaintext carries no usable event structure. Always returns at least one
/// chunk. All chunk text is valid UTF-8 by construction, which also
/// guarantees the NEAR AI scorer's UTF-8 prompt requirement downstream.
pub fn chunk_envelope_plaintext(plaintext: &[u8], cfg: &ChunkerConfig) -> ChunkPlan {
    if let Some(events) = parse_envelope_rendered_events(plaintext) {
        return chunk_rendered_events(&events, cfg);
    }
    let text = String::from_utf8_lossy(plaintext);
    finalize_plan(split_fixed_char_windows(&text, cfg.target_chars()), cfg)
}
```

Add to `crates/trace-commons-gate-enclave/src/lib.rs` beside the existing `pub mod embedder;` line:

```rust
pub mod chunker;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p trace-commons-gate-enclave chunker`
Expected: PASS (all 10 tests).

Also run the default build to prove the serde_json flip is clean:
Run: `RUSTFLAGS="-D warnings" cargo check -p trace-commons-gate-enclave && RUSTFLAGS="-D warnings" cargo check -p trace-commons-gate-enclave --features near-ai-scorer`
Expected: both succeed with no warnings.

- [ ] **Step 6: Clippy**

Run: `cargo clippy -p trace-commons-gate-enclave --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/trace-commons-gate-enclave/Cargo.toml crates/trace-commons-gate-enclave/src/chunker.rs crates/trace-commons-gate-enclave/src/lib.rs
git commit -m "add TraceChunker with canonical event rendering and bounded packing"
```

---

### Task 2: Lower logprobs_top_k default from 5 to 1

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs:279` (const `TRACE_COMMONS_NEAR_AI_DEFAULT_LOGPROBS_TOP_K`)
- Modify: `crates/trace-commons-gate-enclave/src/perplexity_near_ai.rs:67-72` (field doc), `:296-305` (`ok_cfg`), `:336-344` (`build_request_sets_echo_and_logprobs` test)

**Interfaces:**
- Consumes: `NearAiScorerConfig { logprobs_top_k: u32, .. }` (existing; validation range `1..=5` at `perplexity_near_ai.rs:95-99` is unchanged and already admits 1).
- Produces: no new symbols. Every NEAR AI scoring request now sends `logprobs: 1`.

- [ ] **Step 1: Write the failing test**

In `crates/trace-commons-gate-enclave/src/perplexity_near_ai.rs`, update the existing test (and `ok_cfg`) to lock the new default shape:

```rust
    fn ok_cfg() -> NearAiScorerConfig {
        NearAiScorerConfig {
            base_url: "https://qwen3-30b.completions.near.ai/v1".to_string(),
            model: "Qwen/Qwen3-30B-A3B-Instruct-2507".to_string(),
            api_key: "sk-test".to_string(),
            tail_logprob_cutoff: -8.0,
            // Production default (mirrors
            // TRACE_COMMONS_NEAR_AI_DEFAULT_LOGPROBS_TOP_K = 1): perplexity
            // needs only the realized token's logprob; k=1 cuts backend
            // memory + response size ~5x vs the old 5.
            logprobs_top_k: 1,
            timeout: Duration::from_secs(30),
        }
    }
```

```rust
    #[test]
    fn build_request_sets_echo_and_logprobs() {
        let s = NearAiPerplexityScorer::try_new(ok_cfg()).unwrap();
        let req = s.build_request(b"The capital of France is Paris.").unwrap();
        assert!(req.echo);
        assert_eq!(req.logprobs, 1);
        assert!(!req.stream);
        assert_eq!(req.temperature, 0.0);
        assert_eq!(req.model, "Qwen/Qwen3-30B-A3B-Instruct-2507");
    }
```

- [ ] **Step 2: Run test to verify current state**

Run: `cargo test -p trace-commons-gate-enclave --features near-ai-scorer build_request_sets_echo_and_logprobs`
Expected: PASS already (the test only exercises the cfg it constructs). The failing check is the ingest const — verify it is still 5:

Run: `grep -n "TRACE_COMMONS_NEAR_AI_DEFAULT_LOGPROBS_TOP_K: u32 = " crates/trace-commons-server/src/bin/trace-commons-ingest.rs`
Expected: `279:const TRACE_COMMONS_NEAR_AI_DEFAULT_LOGPROBS_TOP_K: u32 = 5;`

- [ ] **Step 3: Change the production default**

In `crates/trace-commons-server/src/bin/trace-commons-ingest.rs:279`:

```rust
// Chunked scoring sends one bounded request per chunk; perplexity needs
// only the realized token's logprob, so k=1 cuts TEE backend memory and
// response size ~5x vs the OpenAI-canonical 5 (large-trace OOM root cause).
const TRACE_COMMONS_NEAR_AI_DEFAULT_LOGPROBS_TOP_K: u32 = 1;
```

Also update the stale doc comment on `NearAiScorerConfig::logprobs_top_k` (`perplexity_near_ai.rs:67-72`) to:

```rust
    /// Per-call `logprobs` value sent to the API. Production default is 1:
    /// both metrics here consume only the realized-token NLL, and
    /// `echo + prompt_logprobs` memory on the TEE backend scales with this
    /// value. NEAR AI's hosted vLLM accepts 1..=5.
    pub logprobs_top_k: u32,
```

- [ ] **Step 4: Verify**

Run: `cargo test -p trace-commons-gate-enclave --features near-ai-scorer && RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins`
Expected: PASS / clean.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-gate-enclave/src/perplexity_near_ai.rs crates/trace-commons-server/src/bin/trace-commons-ingest.rs
git commit -m "lower NEAR AI logprobs default from 5 to 1"
```

---

### Task 3: Per-chunk perplexity result type + pure aggregation helpers

**Files:**
- Modify: `crates/trace-commons-gate-enclave/src/perplexity.rs` (add `ChunkPerplexity`, `score_chunk` provided method)
- Modify: `crates/trace-commons-gate-enclave/src/perplexity_near_ai.rs` (native `score_chunk`)
- Create: `crates/trace-commons-gate-enclave/src/chunk_aggregate.rs`
- Modify: `crates/trace-commons-gate-enclave/src/lib.rs` (add `pub mod chunk_aggregate;`)

**Interfaces:**
- Consumes: `aggregate_perplexity_metrics(logprobs: &[f32], tail_logprob_cutoff: f32) -> PerplexityResult` and `per_token_rarity_micros(logprobs: &[f32], k: usize) -> u64` from `perplexity_local.rs` (both drop element 0 as the BOS placeholder — reuse, do not reinvent).
- Produces:
  - `pub struct ChunkPerplexity { pub sum_nll: f64, pub tokens: u64, pub tail_tokens: u64, pub logprobs: Vec<f32> }` (in `perplexity.rs`; `logprobs` holds the USABLE post-BOS-drop logprobs, may be empty when a scorer cannot expose raw logprobs)
  - `PerplexityScorer::score_chunk(&self, chunk: &[u8]) -> anyhow::Result<ChunkPerplexity>` (provided method, default derives from `score()`)
  - `pub struct ChunkedPerplexityAggregate { pub representative_perplexity_micros: u64, pub peak_perplexity_micros: u64, pub tail_fraction_micros: u64, pub tokens_scored: u64 }`
  - `pub fn aggregate_chunked_perplexity(chunks: &[ChunkPerplexity], min_chunk_tokens: u64) -> ChunkedPerplexityAggregate`
  - `pub fn global_rarity_micros_across_chunks(chunks: &[ChunkPerplexity], k: usize) -> u64`
  - `pub fn aggregate_chunked_novelty(novelty_micros: &[u64], chunk_tokens: &[u64], min_chunk_tokens: u64) -> (u64, u64)`

- [ ] **Step 1: Write the failing tests for `ChunkPerplexity` + `score_chunk`**

Append to the test module of `crates/trace-commons-gate-enclave/src/perplexity.rs`:

```rust
    #[test]
    fn default_score_chunk_derives_from_score_within_tolerance() {
        // exp(sum_nll / tokens) must reproduce score()'s aggregate within
        // f64 ln/exp round-trip tolerance.
        let s = MockPerplexityScorer::new();
        let whole = s.score(b"hello world").unwrap();
        let chunk = s.score_chunk(b"hello world").unwrap();
        assert_eq!(chunk.tokens, whole.tokens_scored);
        let rebuilt = ((chunk.sum_nll / chunk.tokens as f64).exp() * 1_000_000.0) as u64;
        let diff = rebuilt.abs_diff(whole.aggregate_perplexity_micros);
        assert!(diff <= 2, "ln/exp round trip drifted by {diff} micros");
        // Mock cannot expose raw logprobs.
        assert!(chunk.logprobs.is_empty());
        // tail_tokens is clamped to tokens even though the mock's
        // tail_fraction_micros band can exceed 1.0.
        assert!(chunk.tail_tokens <= chunk.tokens);
    }

    #[test]
    fn default_score_chunk_zero_tokens_is_all_zero() {
        struct ZeroScorer;
        impl PerplexityScorer for ZeroScorer {
            fn score(&self, _plaintext: &[u8]) -> anyhow::Result<PerplexityResult> {
                Ok(PerplexityResult {
                    aggregate_perplexity_micros: 0,
                    tail_fraction_micros: 0,
                    tokens_scored: 0,
                })
            }
        }
        let c = ZeroScorer.score_chunk(b"").unwrap();
        assert_eq!(c.tokens, 0);
        assert_eq!(c.sum_nll, 0.0);
        assert_eq!(c.tail_tokens, 0);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p trace-commons-gate-enclave score_chunk`
Expected: FAIL — `ChunkPerplexity` / `score_chunk` not defined.

- [ ] **Step 3: Implement `ChunkPerplexity` + provided `score_chunk`**

In `crates/trace-commons-gate-enclave/src/perplexity.rs`, add after `PerplexityResult`:

```rust
/// Raw per-chunk scoring material for whole-trace aggregation. Unlike
/// [`PerplexityResult`] (already collapsed to micros), this keeps the sums
/// so the orchestrator can compute the token-weighted whole-trace mean
/// `exp(sum over chunks of sum_nll / sum over chunks of n)` exactly.
#[derive(Debug, Clone, PartialEq)]
pub struct ChunkPerplexity {
    /// `-sum(logprob)` over the chunk's usable tokens (token 0 dropped).
    pub sum_nll: f64,
    /// Usable token count for the chunk (`n_c`).
    pub tokens: u64,
    /// Count of usable tokens with `logprob < tail_logprob_cutoff`.
    pub tail_tokens: u64,
    /// The usable (post-BOS-drop) per-token logprobs, for global top-K
    /// rarity across chunks. Empty when the scorer cannot expose raw
    /// logprobs (e.g. the mock) — rarity then simply has no contribution
    /// from that chunk.
    pub logprobs: Vec<f32>,
}
```

and extend the trait with a provided method:

```rust
pub trait PerplexityScorer: Send + Sync {
    fn score(&self, plaintext: &[u8]) -> anyhow::Result<PerplexityResult>;

    /// Score one bounded chunk and return raw aggregation material. The
    /// default derives `(sum_nll, n, tail_tokens)` from [`score`]'s
    /// collapsed micros (exact within f64 ln/exp round-trip tolerance) and
    /// exposes no raw logprobs. Real scorers that hold per-token logprobs
    /// SHOULD override this to return them losslessly.
    fn score_chunk(&self, chunk: &[u8]) -> anyhow::Result<ChunkPerplexity> {
        let r = self.score(chunk)?;
        let tokens = r.tokens_scored;
        if tokens == 0 {
            return Ok(ChunkPerplexity {
                sum_nll: 0.0,
                tokens: 0,
                tail_tokens: 0,
                logprobs: Vec::new(),
            });
        }
        let perp = r.aggregate_perplexity_micros as f64 / 1_000_000.0;
        let mean_nll = if perp > 0.0 { perp.ln() } else { 0.0 };
        let tail_fraction = r.tail_fraction_micros as f64 / 1_000_000.0;
        let tail_tokens = ((tail_fraction * tokens as f64).round() as u64).min(tokens);
        Ok(ChunkPerplexity {
            sum_nll: mean_nll * tokens as f64,
            tokens,
            tail_tokens,
            logprobs: Vec::new(),
        })
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p trace-commons-gate-enclave score_chunk`
Expected: PASS (2 tests).

- [ ] **Step 5: Write the failing test for the NEAR native `score_chunk`**

Append to the test module of `crates/trace-commons-gate-enclave/src/perplexity_near_ai.rs`:

```rust
    #[test]
    fn chunk_perplexity_from_logprobs_matches_helpers() {
        // Unit-test the pure conversion (no HTTP): the same fixture logprob
        // slice must produce sum_nll / tokens / tail_tokens consistent with
        // aggregate_perplexity_metrics.
        let logprobs = vec![0.0_f32, -1.0, -2.0, -20.0, -1.0];
        let chunk = chunk_perplexity_from_logprobs(&logprobs, -8.0);
        assert_eq!(chunk.tokens, 4);
        assert!((chunk.sum_nll - 24.0).abs() < 1e-6);
        assert_eq!(chunk.tail_tokens, 1); // only -20.0 < -8.0
        assert_eq!(chunk.logprobs, vec![-1.0, -2.0, -20.0, -1.0]);
        let rebuilt = ((chunk.sum_nll / chunk.tokens as f64).exp() * 1_000_000.0) as u64;
        let reference = aggregate_perplexity_metrics(&logprobs, -8.0);
        let diff = rebuilt.abs_diff(reference.aggregate_perplexity_micros);
        assert!(diff <= 2, "chunk-form perplexity drifted by {diff} micros");
    }
```

- [ ] **Step 6: Run test to verify it fails**

Run: `cargo test -p trace-commons-gate-enclave --features near-ai-scorer chunk_perplexity_from_logprobs`
Expected: FAIL — `chunk_perplexity_from_logprobs` not defined.

- [ ] **Step 7: Implement the NEAR native `score_chunk`**

In `crates/trace-commons-gate-enclave/src/perplexity_near_ai.rs`, add `ChunkPerplexity` to the existing `use crate::perplexity::{...}` import, then add a free function and the trait override:

```rust
/// Convert a raw logprob slice (element 0 = BOS placeholder, dropped) into
/// [`ChunkPerplexity`]. Factored out of `score_chunk` so it unit-tests
/// without HTTP. Fail-closed parallel to `aggregate_perplexity_metrics`:
/// short or non-finite input collapses to a zero-token chunk.
fn chunk_perplexity_from_logprobs(logprobs: &[f32], tail_logprob_cutoff: f32) -> ChunkPerplexity {
    if logprobs.len() < 2 || logprobs[1..].iter().any(|lp| !lp.is_finite()) {
        return ChunkPerplexity {
            sum_nll: 0.0,
            tokens: 0,
            tail_tokens: 0,
            logprobs: Vec::new(),
        };
    }
    let usable = &logprobs[1..];
    let sum_nll: f64 = usable.iter().map(|lp| -(*lp as f64)).sum();
    let tail_tokens = usable.iter().filter(|&&lp| lp < tail_logprob_cutoff).count() as u64;
    ChunkPerplexity {
        sum_nll,
        tokens: usable.len() as u64,
        tail_tokens,
        logprobs: usable.to_vec(),
    }
}
```

and inside `impl PerplexityScorer for NearAiPerplexityScorer` add:

```rust
    fn score_chunk(&self, chunk: &[u8]) -> anyhow::Result<ChunkPerplexity> {
        let logprobs = self.fetch_logprobs(chunk)?;
        Ok(chunk_perplexity_from_logprobs(
            &logprobs,
            self.cfg.tail_logprob_cutoff,
        ))
    }
```

- [ ] **Step 8: Run test to verify it passes**

Run: `cargo test -p trace-commons-gate-enclave --features near-ai-scorer chunk_perplexity_from_logprobs`
Expected: PASS.

- [ ] **Step 9: Write the failing tests for the aggregation helpers**

Create `crates/trace-commons-gate-enclave/src/chunk_aggregate.rs` with tests first (implementation `todo!()` or absent):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::perplexity::ChunkPerplexity;

    fn chunk(sum_nll: f64, tokens: u64, tail_tokens: u64, logprobs: Vec<f32>) -> ChunkPerplexity {
        ChunkPerplexity {
            sum_nll,
            tokens,
            tail_tokens,
            logprobs,
        }
    }

    #[test]
    fn single_chunk_representative_equals_whole_call() {
        // One chunk of uniform logprob -1.0 over 4 tokens: perplexity = e.
        let c = chunk(4.0, 4, 0, vec![-1.0; 4]);
        let agg = aggregate_chunked_perplexity(&[c], 1);
        let want = (1.0_f64.exp() * 1_000_000.0) as u64;
        assert!(agg.representative_perplexity_micros.abs_diff(want) <= 2);
        assert_eq!(agg.peak_perplexity_micros, agg.representative_perplexity_micros);
        assert_eq!(agg.tail_fraction_micros, 0);
        assert_eq!(agg.tokens_scored, 4);
    }

    #[test]
    fn representative_is_token_weighted_across_chunks() {
        // Chunk A: 100 tokens at mean_nll 1.0. Chunk B: 300 tokens at
        // mean_nll 3.0. Weighted mean_nll = (100 + 900)/400 = 2.5.
        let a = chunk(100.0, 100, 0, vec![]);
        let b = chunk(900.0, 300, 0, vec![]);
        let agg = aggregate_chunked_perplexity(&[a, b], 1);
        let want = (2.5_f64.exp() * 1_000_000.0) as u64;
        assert!(agg.representative_perplexity_micros.abs_diff(want) <= 2);
        assert_eq!(agg.tokens_scored, 400);
    }

    #[test]
    fn peak_is_max_over_min_content_guarded_chunks() {
        // Chunk A: 100 tokens, mean_nll 1.0. Chunk B: 100 tokens, mean_nll
        // 3.0 (the peak). Chunk C: 4 tokens, mean_nll 10.0 — a tiny
        // surprising fragment that the 64-token guard must exclude.
        let a = chunk(100.0, 100, 0, vec![]);
        let b = chunk(300.0, 100, 0, vec![]);
        let c = chunk(40.0, 4, 0, vec![]);
        let agg = aggregate_chunked_perplexity(&[a, b, c], 64);
        let want = (3.0_f64.exp() * 1_000_000.0) as u64;
        assert!(agg.peak_perplexity_micros.abs_diff(want) <= 2);
    }

    #[test]
    fn peak_falls_back_to_representative_when_no_chunk_is_eligible() {
        let a = chunk(10.0, 10, 0, vec![]);
        let agg = aggregate_chunked_perplexity(&[a], 64);
        assert_eq!(agg.peak_perplexity_micros, agg.representative_perplexity_micros);
    }

    #[test]
    fn tail_fraction_is_exact_over_all_tokens() {
        // 1 tail token of 4 + 3 tail tokens of 6 = 4/10 = 0.4.
        let a = chunk(4.0, 4, 1, vec![]);
        let b = chunk(6.0, 6, 3, vec![]);
        let agg = aggregate_chunked_perplexity(&[a, b], 1);
        assert_eq!(agg.tail_fraction_micros, 400_000);
    }

    #[test]
    fn zero_total_tokens_is_all_zero() {
        let agg = aggregate_chunked_perplexity(&[chunk(0.0, 0, 0, vec![])], 64);
        assert_eq!(agg.representative_perplexity_micros, 0);
        assert_eq!(agg.peak_perplexity_micros, 0);
        assert_eq!(agg.tail_fraction_micros, 0);
        assert_eq!(agg.tokens_scored, 0);
    }

    #[test]
    fn global_rarity_selects_across_chunk_boundaries() {
        // Rarest two tokens live in DIFFERENT chunks: -5.0 (chunk A) and
        // -4.0 (chunk B). K=2 -> mean_nll 4.5 -> exp(4.5).
        let a = chunk(5.5, 2, 0, vec![-0.5, -5.0]);
        let b = chunk(4.1, 2, 0, vec![-4.0, -0.1]);
        let got = global_rarity_micros_across_chunks(&[a, b], 2);
        let want = (4.5_f64.exp() * 1_000_000.0) as u64;
        assert!(got.abs_diff(want) <= 40, "got {got}, want ~{want}");
    }

    #[test]
    fn global_rarity_ignores_chunks_without_logprobs() {
        let a = chunk(5.0, 5, 0, vec![]);
        let b = chunk(2.0, 2, 0, vec![-1.0, -1.0]);
        let got = global_rarity_micros_across_chunks(&[a, b], 2);
        let want = (1.0_f64.exp() * 1_000_000.0) as u64;
        assert!(got.abs_diff(want) <= 40);
    }

    #[test]
    fn novelty_representative_is_token_weighted_and_peak_guarded() {
        // Chunk novelties 0.2 (100 tok), 0.8 (100 tok), 1.0 (4 tok, guarded
        // out of peak). Representative = (0.2*100 + 0.8*100 + 1.0*4)/204.
        let (rep, peak) = aggregate_chunked_novelty(
            &[200_000, 800_000, 1_000_000],
            &[100, 100, 4],
            64,
        );
        let want_rep = ((0.2 * 100.0 + 0.8 * 100.0 + 1.0 * 4.0) / 204.0 * 1_000_000.0) as u64;
        assert!(rep.abs_diff(want_rep) <= 1);
        assert_eq!(peak, 800_000);
    }

    #[test]
    fn novelty_zero_weights_fall_back_to_uniform() {
        // All-zero token counts (degenerate scorer) must not divide by zero;
        // fall back to an unweighted mean and unguarded peak.
        let (rep, peak) = aggregate_chunked_novelty(&[200_000, 400_000], &[0, 0], 64);
        assert_eq!(rep, 300_000);
        assert_eq!(peak, 400_000);
    }
}
```

- [ ] **Step 10: Run tests to verify they fail**

Run: `cargo test -p trace-commons-gate-enclave chunk_aggregate`
Expected: FAIL — module/functions not defined.

- [ ] **Step 11: Implement the aggregation helpers**

Fill in `crates/trace-commons-gate-enclave/src/chunk_aggregate.rs` above the tests:

```rust
//! Pure whole-trace aggregation over per-chunk scoring results.
//!
//! Representative = token-weighted whole-trace values (equal to a single
//! whole-trace call within float tolerance for one chunk). Peak = the
//! most-surprising / most-novel min-content-guarded chunk. All math is f64;
//! degenerate inputs collapse to zero (fail-closed — the configured floors
//! then refuse the gate naturally).

use crate::perplexity::ChunkPerplexity;
use crate::perplexity_local::per_token_rarity_micros;

/// Whole-trace perplexity aggregate. Micros fields saturate; non-finite
/// intermediates collapse to zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkedPerplexityAggregate {
    /// `exp( sum_c sum_nll_c / sum_c n_c )` — stored in the existing
    /// `perplexity_micros` column.
    pub representative_perplexity_micros: u64,
    /// `max_c exp(sum_nll_c / n_c)` over chunks with `n_c >=
    /// min_chunk_tokens`; falls back to the representative when no chunk is
    /// eligible. Stored in the new `peak_perplexity_micros` column.
    pub peak_perplexity_micros: u64,
    /// `sum_c tail_tokens_c / sum_c n_c` — the exact whole-trace fraction.
    pub tail_fraction_micros: u64,
    /// `sum_c n_c`.
    pub tokens_scored: u64,
}

fn saturating_micros_f64(v: f64) -> u64 {
    if !v.is_finite() || v < 0.0 {
        return 0;
    }
    let scaled = v * 1_000_000.0;
    if scaled >= u64::MAX as f64 {
        u64::MAX
    } else {
        scaled as u64
    }
}

pub fn aggregate_chunked_perplexity(
    chunks: &[ChunkPerplexity],
    min_chunk_tokens: u64,
) -> ChunkedPerplexityAggregate {
    let total_tokens: u64 = chunks.iter().map(|c| c.tokens).sum();
    if total_tokens == 0 {
        return ChunkedPerplexityAggregate {
            representative_perplexity_micros: 0,
            peak_perplexity_micros: 0,
            tail_fraction_micros: 0,
            tokens_scored: 0,
        };
    }
    let total_nll: f64 = chunks.iter().map(|c| c.sum_nll).sum();
    let total_tail: u64 = chunks.iter().map(|c| c.tail_tokens).sum();
    let representative = (total_nll / total_tokens as f64).exp();
    let representative_perplexity_micros = saturating_micros_f64(representative);

    let peak = chunks
        .iter()
        .filter(|c| c.tokens >= min_chunk_tokens && c.tokens > 0)
        .map(|c| (c.sum_nll / c.tokens as f64).exp())
        .fold(f64::NEG_INFINITY, f64::max);
    let peak_perplexity_micros = if peak.is_finite() {
        saturating_micros_f64(peak)
    } else {
        representative_perplexity_micros
    };

    ChunkedPerplexityAggregate {
        representative_perplexity_micros,
        peak_perplexity_micros,
        tail_fraction_micros: saturating_micros_f64(total_tail as f64 / total_tokens as f64),
        tokens_scored: total_tokens,
    }
}

/// Global top-K rarity over the concatenation of all chunks' usable
/// logprobs: `exp(-mean(K globally-rarest))`. Reuses
/// [`per_token_rarity_micros`] by prepending its expected BOS placeholder.
/// Chunks whose scorer exposed no raw logprobs contribute nothing.
pub fn global_rarity_micros_across_chunks(chunks: &[ChunkPerplexity], k: usize) -> u64 {
    let mut all: Vec<f32> = Vec::with_capacity(1 + chunks.iter().map(|c| c.logprobs.len()).sum::<usize>());
    all.push(0.0); // BOS placeholder dropped by the helper.
    for c in chunks {
        all.extend_from_slice(&c.logprobs);
    }
    per_token_rarity_micros(&all, k)
}

/// Token-weighted representative + min-content-guarded peak over per-chunk
/// novelty scores (micros). Weights are the chunks' scored-token counts;
/// all-zero weights fall back to an unweighted mean and an unguarded peak
/// so a degenerate scorer cannot zero out the novelty signal.
pub fn aggregate_chunked_novelty(
    novelty_micros: &[u64],
    chunk_tokens: &[u64],
    min_chunk_tokens: u64,
) -> (u64, u64) {
    assert_eq!(novelty_micros.len(), chunk_tokens.len());
    if novelty_micros.is_empty() {
        return (0, 0);
    }
    let total_tokens: u64 = chunk_tokens.iter().sum();
    let representative = if total_tokens == 0 {
        novelty_micros.iter().map(|&n| n as f64).sum::<f64>() / novelty_micros.len() as f64
    } else {
        novelty_micros
            .iter()
            .zip(chunk_tokens.iter())
            .map(|(&n, &t)| n as f64 * t as f64)
            .sum::<f64>()
            / total_tokens as f64
    };
    let peak = novelty_micros
        .iter()
        .zip(chunk_tokens.iter())
        .filter(|(_, &t)| total_tokens == 0 || t >= min_chunk_tokens)
        .map(|(&n, _)| n)
        .max()
        .unwrap_or_else(|| representative as u64);
    (representative as u64, peak)
}
```

Add to `crates/trace-commons-gate-enclave/src/lib.rs`:

```rust
pub mod chunk_aggregate;
```

- [ ] **Step 12: Run tests to verify they pass**

Run: `cargo test -p trace-commons-gate-enclave chunk_aggregate && cargo test -p trace-commons-gate-enclave --features near-ai-scorer`
Expected: PASS (all).

- [ ] **Step 13: Clippy + commit**

Run: `cargo clippy -p trace-commons-gate-enclave --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`
Expected: clean.

```bash
git add crates/trace-commons-gate-enclave/src/perplexity.rs crates/trace-commons-gate-enclave/src/perplexity_near_ai.rs crates/trace-commons-gate-enclave/src/chunk_aggregate.rs crates/trace-commons-gate-enclave/src/lib.rs
git commit -m "add per-chunk perplexity result type and whole-trace aggregation helpers"
```

---

### Task 4: Orchestrator perplexity chunk loop (embedding path unchanged)

**Files:**
- Modify: `crates/trace-commons-gate-enclave/src/orchestrator.rs` (config lines 14-37, decision struct lines 42-57, `evaluate` lines 111-181, tests 220-313)

**Interfaces:**
- Consumes: `chunk_envelope_plaintext`, `ChunkerConfig` (Task 1); `PerplexityScorer::score_chunk`, `ChunkPerplexity` (Task 3); `aggregate_chunked_perplexity` (Task 3).
- Produces: `EnclaveGateOrchestratorConfig` gains `pub chunk_target_tokens: usize, pub chunk_max_tokens: usize, pub chunk_cap: usize, pub chunk_min_tokens: u64, pub embed_insert_novelty_micros: u64`. `OrchestrationDecision` gains `pub peak_perplexity_micros: u64, pub peak_novelty_micros: u64, pub chunk_count: u32, pub chunks_capped: bool`. Consumed by Tasks 5, 7, 8, 9. In this task `peak_novelty_micros` is set equal to `novelty_score_micros` (real peak lands in Task 7) and embedding still runs on the whole plaintext.

- [ ] **Step 1: Extend the config with chunk knobs**

In `crates/trace-commons-gate-enclave/src/orchestrator.rs`, extend `EnclaveGateOrchestratorConfig`:

```rust
#[derive(Debug, Clone)]
pub struct EnclaveGateOrchestratorConfig {
    pub gate_policy_version: String,
    pub gate_version_hash: String,
    pub perplexity_floor_micros: u64,
    pub tail_fraction_floor_micros: u64,
    pub novelty_floor_micros: u64,
    pub top_k: usize,
    /// Greedy chunk-packing target, in tokens (char proxy). Default 2048.
    pub chunk_target_tokens: usize,
    /// Hard per-chunk max, in tokens. Default 3072.
    pub chunk_max_tokens: usize,
    /// Hard cap on chunks per trace. Default 16.
    pub chunk_cap: usize,
    /// Min scored tokens for a chunk to be peak-eligible. Default 64.
    pub chunk_min_tokens: u64,
    /// Per-chunk index-insert dedup threshold: a chunk whose novelty is
    /// below this is a near-duplicate and is not inserted. Default 50000.
    pub embed_insert_novelty_micros: u64,
}
```

and in `mock_default()` add:

```rust
            chunk_target_tokens: 2048,
            chunk_max_tokens: 3072,
            chunk_cap: 16,
            chunk_min_tokens: 64,
            embed_insert_novelty_micros: 50_000,
```

- [ ] **Step 2: Extend `OrchestrationDecision`**

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct OrchestrationDecision {
    pub gate_policy_version: String,
    pub gate_version_hash: String,
    pub perplexity_micros: u64,
    pub tail_fraction_micros: u64,
    pub perplexity_passed: bool,
    pub novelty_score_micros: u64,
    pub nearest_neighbor_hash: String,
    pub novelty_passed: bool,
    pub embedding_evidence_hash: String,
    pub attestation_chain_hash: String,
    /// `Some(id)` when both gates passed and the orchestrator inserted the
    /// embedding into the vector index; `None` otherwise.
    pub inserted_entry_id: Option<Uuid>,
    /// Peak (most-surprising min-content-guarded chunk) perplexity.
    /// Equals `perplexity_micros` for single-chunk traces.
    pub peak_perplexity_micros: u64,
    /// Peak (most-novel min-content-guarded chunk) novelty. Equals
    /// `novelty_score_micros` for single-chunk traces.
    pub peak_novelty_micros: u64,
    /// Number of chunks scored (>= 1).
    pub chunk_count: u32,
    /// True when the per-trace chunk cap dropped trailing chunks.
    pub chunks_capped: bool,
}
```

- [ ] **Step 3: Write the failing tests**

Append to the orchestrator test module:

```rust
    use crate::perplexity::{ChunkPerplexity, PerplexityResult};

    /// Fixed-value scorer for exact-identity assertions.
    struct StubScorer;
    impl crate::perplexity::PerplexityScorer for StubScorer {
        fn score(&self, _plaintext: &[u8]) -> anyhow::Result<PerplexityResult> {
            Ok(PerplexityResult {
                aggregate_perplexity_micros: 2_718_281,
                tail_fraction_micros: 250_000,
                tokens_scored: 100,
            })
        }
    }

    /// Scorer that fails on its Nth call — proves fail-closed mid-loop.
    struct FailOnNthScorer {
        n: std::sync::atomic::AtomicUsize,
        fail_at: usize,
    }
    impl crate::perplexity::PerplexityScorer for FailOnNthScorer {
        fn score(&self, plaintext: &[u8]) -> anyhow::Result<PerplexityResult> {
            let call = self
                .n
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if call == self.fail_at {
                anyhow::bail!("ChunkScorerInjectedFailure");
            }
            MockPerplexityScorer::new().score(plaintext)
        }
    }

    fn multi_chunk_envelope(event_count: usize, event_chars: usize) -> Vec<u8> {
        let content = "a".repeat(event_chars);
        let events: Vec<serde_json::Value> = (0..event_count)
            .map(|_| {
                serde_json::json!({
                    "event_type": "assistant_message",
                    "redacted_content": content,
                })
            })
            .collect();
        serde_json::to_vec(&serde_json::json!({ "events": events })).unwrap()
    }

    #[test]
    fn single_chunk_trace_matches_whole_call_on_rendered_text() {
        // A small trace is one chunk; its representative and peak must equal
        // the scorer's single-call values on that same chunk text (identity
        // within ln/exp round-trip tolerance for the derived default
        // score_chunk path).
        let mut cfg = EnclaveGateOrchestratorConfig::mock_default();
        cfg.chunk_min_tokens = 1;
        let orch = EnclaveGateOrchestrator::new(
            StubScorer,
            MockEmbedder::new(),
            MockVectorIndex::new(),
            cfg,
        );
        let d = orch
            .evaluate(&multi_chunk_envelope(1, 40), "tenant_a")
            .unwrap();
        assert_eq!(d.chunk_count, 1);
        assert!(!d.chunks_capped);
        assert!(d.perplexity_micros.abs_diff(2_718_281) <= 2);
        assert!(d.peak_perplexity_micros.abs_diff(2_718_281) <= 2);
        assert!(d.tail_fraction_micros.abs_diff(250_000) <= 2_500);
        assert_eq!(d.peak_novelty_micros, d.novelty_score_micros);
    }

    #[test]
    fn large_trace_produces_multiple_chunks_and_cap() {
        // 20 events of 8000 chars each: every event fills a ~2048-token
        // (8192-char) target chunk alone -> 20 chunks -> capped at 16.
        let orch = orch_with_floors(0, 0, 0);
        let d = orch
            .evaluate(&multi_chunk_envelope(20, 8_000), "tenant_a")
            .unwrap();
        assert_eq!(d.chunk_count, 16);
        assert!(d.chunks_capped);
    }

    #[test]
    fn chunk_scorer_error_fails_the_whole_evaluation() {
        let mut cfg = EnclaveGateOrchestratorConfig::mock_default();
        cfg.chunk_min_tokens = 1;
        let orch = EnclaveGateOrchestrator::new(
            FailOnNthScorer {
                n: std::sync::atomic::AtomicUsize::new(0),
                fail_at: 2,
            },
            MockEmbedder::new(),
            MockVectorIndex::new(),
            cfg,
        );
        let err = orch
            .evaluate(&multi_chunk_envelope(20, 8_000), "tenant_a")
            .unwrap_err();
        assert!(
            format!("{err:#}").contains("PerplexityScorerInferenceFailed"),
            "fail-closed error context missing: {err:#}"
        );
    }
```

Note: the existing tests (`deterministic_for_same_input`, etc.) feed `b"hello world"` — non-JSON, so the chunker falls back to a single raw-text chunk whose bytes equal the input; mock scores are unchanged and those tests keep passing.

- [ ] **Step 4: Run tests to verify they fail**

Run: `cargo test -p trace-commons-gate-enclave orchestrator`
Expected: FAIL — new config/decision fields and behavior missing (compile errors first; fix struct literals in existing tests only by compiling, not by changing assertions).

- [ ] **Step 5: Implement the chunk loop in `evaluate`**

Replace the body of `evaluate` (lines 111-181) with:

```rust
    pub fn evaluate(
        &self,
        plaintext: &[u8],
        tenant_storage_ref: &str,
    ) -> anyhow::Result<OrchestrationDecision> {
        // Chunk the parsed envelope. Every backend request downstream is now
        // bounded — the root-cause fix for the TEE OOM crashes.
        let chunker_cfg = crate::chunker::ChunkerConfig {
            target_tokens: self.cfg.chunk_target_tokens,
            max_tokens: self.cfg.chunk_max_tokens,
            chunk_cap: self.cfg.chunk_cap,
        };
        let plan = crate::chunker::chunk_envelope_plaintext(plaintext, &chunker_cfg);
        if plan.chunks_capped {
            // Hash-only: counts and a fixed label only. Never chunk content.
            tracing::warn!(
                error_class = "TraceChunkCapExceeded",
                chunk_count = plan.chunks.len(),
                dropped_chunk_count = plan.dropped_chunk_count,
                "trace chunk cap enforced"
            );
        }

        // Fail-closed inference: any chunk's scorer error refuses the whole
        // evaluation (v1 semantics). Sequential — never a concurrent burst
        // against one pinned backend.
        let mut chunk_scores: Vec<crate::perplexity::ChunkPerplexity> =
            Vec::with_capacity(plan.chunks.len());
        for chunk in &plan.chunks {
            let cs = self
                .perplexity
                .score_chunk(chunk.text.as_bytes())
                .context("PerplexityScorerInferenceFailed")?;
            chunk_scores.push(cs);
        }
        let perp_agg = crate::chunk_aggregate::aggregate_chunked_perplexity(
            &chunk_scores,
            self.cfg.chunk_min_tokens,
        );

        // Embedding path: whole-plaintext for now (per-chunk embedding lands
        // with the embedding half of the chunked-scoring slice).
        let embedding = self
            .embedder
            .embed(plaintext)
            .context("EmbedderInferenceFailed")?;
        let neighbors = self
            .index
            .nearest(tenant_storage_ref, &embedding, self.cfg.top_k)?;

        let max_sim = neighbors
            .iter()
            .map(|n| n.similarity)
            .fold(f32::NEG_INFINITY, f32::max);
        let novelty_score_f = if max_sim.is_finite() {
            (1.0 - max_sim).max(0.0)
        } else {
            1.0
        };
        let novelty_score_micros = (novelty_score_f.clamp(0.0, 2.0) * 1_000_000.0) as u64;

        let perplexity_passed = perp_agg.representative_perplexity_micros
            >= self.cfg.perplexity_floor_micros
            && perp_agg.tail_fraction_micros >= self.cfg.tail_fraction_floor_micros;
        let novelty_passed = novelty_score_micros >= self.cfg.novelty_floor_micros;

        let nearest_neighbor_hash = hash_neighbors(&neighbors);
        let embedding_evidence_hash = hash_embedding_evidence(
            &self.cfg.gate_policy_version,
            tenant_storage_ref,
            &embedding,
        );
        let attestation_chain_hash =
            hash_attestation_chain(&self.cfg.gate_policy_version, &self.cfg.gate_version_hash);

        let mut inserted_entry_id = None;
        if perplexity_passed && novelty_passed {
            let entry_id = Uuid::new_v4();
            self.index
                .insert(entry_id, tenant_storage_ref, &embedding)?;
            inserted_entry_id = Some(entry_id);
        }

        Ok(OrchestrationDecision {
            gate_policy_version: self.cfg.gate_policy_version.clone(),
            gate_version_hash: self.cfg.gate_version_hash.clone(),
            perplexity_micros: perp_agg.representative_perplexity_micros,
            tail_fraction_micros: perp_agg.tail_fraction_micros,
            perplexity_passed,
            novelty_score_micros,
            nearest_neighbor_hash,
            novelty_passed,
            embedding_evidence_hash,
            attestation_chain_hash,
            inserted_entry_id,
            peak_perplexity_micros: perp_agg.peak_perplexity_micros,
            // Real per-chunk novelty peak lands with the embedding half;
            // until then peak == representative (exact for 1 chunk).
            peak_novelty_micros: novelty_score_micros,
            chunk_count: plan.chunks.len() as u32,
            chunks_capped: plan.chunks_capped,
        })
    }
```

Note: `tracing` is already a dependency of this crate. The tail-fraction identity in `single_chunk_trace_matches_whole_call_on_rendered_text` uses a 2,500-micros tolerance because the default `score_chunk` reconstructs `tail_tokens` by rounding `tail_fraction * tokens`.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p trace-commons-gate-enclave`
Expected: PASS — all orchestrator tests (old and new), chunker, chunk_aggregate, perplexity.

- [ ] **Step 7: Fix downstream compile (server crate)**

`trace_gate_service.rs:485+` constructs `GateDecision` from `OrchestrationDecision` by field — new fields are not yet consumed, so the server still compiles unchanged. Verify:

Run: `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins && RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run`
Expected: clean. If `OrchestrationDecision` struct literals exist in server tests, add the four new fields with `peak_perplexity_micros: 0, peak_novelty_micros: 0, chunk_count: 1, chunks_capped: false`.

- [ ] **Step 8: Commit**

```bash
git add crates/trace-commons-gate-enclave/src/orchestrator.rs
git commit -m "score perplexity per chunk in the gate orchestrator with fail-closed loop"
```

---

### Task 5: Config env vars, migration V37 columns, and host mapping of the new decision fields

**Files:**
- Create: `migrations/V37__large_trace_chunked_scoring.sql`
- Modify: `crates/trace-commons-server/src/db/postgres.rs` (V37 registration after the V36 block near line 1126)
- Modify: `crates/trace-commons-server/src/trace_corpus_storage.rs:1771-1797` (`TraceGateDecisionRow`)
- Modify: `crates/trace-commons-server/src/trace_gate_service.rs:74-91` (`GateDecision`) and every `GateDecision` construction (`InMemoryGateService` ~262, `LegacyDeterministicGateService` ~326, `DstackGateService` ~394, `EnclaveGateService` ~485, tests, and the synthetic decision in `trace-commons-ingest.rs` ~45266)
- Modify: `crates/trace-commons-server/src/db/trace_corpus_pg.rs` (insert ~5344-5384; SELECT column lists + row mappings at ~5258-5312 and ~5440-5476)
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` (env consts near line 226; both gate-service builders ~4587-4640 and ~4730-4745; `compute_gate_version_hash` call sites; row constructions ~44869, ~45036)

**Interfaces:**
- Consumes: `OrchestrationDecision.{peak_perplexity_micros, peak_novelty_micros, chunk_count, chunks_capped}` (Task 4); `EnclaveGateOrchestratorConfig` chunk knobs (Task 4).
- Produces:
  - `GateDecision` gains `pub peak_perplexity_micros: u64, pub peak_novelty_micros: u64, pub chunk_count: u32, pub chunks_capped: bool`
  - `TraceGateDecisionRow` gains `pub peak_perplexity_micros: Option<i64>, pub peak_novelty_micros: Option<i64>, pub chunk_count: Option<i32>, pub chunks_capped: Option<bool>`
  - Env names: `TRACE_COMMONS_GATE_CHUNK_TARGET_TOKENS` (2048), `TRACE_COMMONS_GATE_CHUNK_MAX_TOKENS` (3072), `TRACE_COMMONS_GATE_CHUNK_CAP` (16), `TRACE_COMMONS_GATE_CHUNK_MIN_TOKENS` (64), `TRACE_COMMONS_GATE_EMBED_INSERT_NOVELTY_MICROS` (50000)
  - Migration `V37__large_trace_chunked_scoring.sql` also creates `trace_gate_chunk_vector_entries` (consumed by Task 8).

- [ ] **Step 1: Write the migration**

Create `migrations/V37__large_trace_chunked_scoring.sql`:

```sql
-- Large-trace chunked scoring (strictly additive).
--
-- New nullable columns on trace_gate_decisions: peak (most-novel-region)
-- values plus chunk bookkeeping. Existing rows read as single-chunk traces
-- via NULL semantics (chunk_count NULL => 1, peak NULL => representative,
-- chunks_capped NULL => false). No existing rows are migrated or re-scored.
ALTER TABLE trace_gate_decisions
    ADD COLUMN IF NOT EXISTS peak_perplexity_micros BIGINT,
    ADD COLUMN IF NOT EXISTS peak_novelty_micros BIGINT,
    ADD COLUMN IF NOT EXISTS chunk_count INT,
    ADD COLUMN IF NOT EXISTS chunks_capped BOOLEAN;

-- Per-chunk vector-index entries. One row per inserted chunk embedding,
-- keyed (submission_id, chunk_index) per the design; decision_id ties the
-- set to the audit row that produced it. The decision row's legacy
-- vector_entry_id column (V24) keeps holding the FIRST inserted entry so
-- existing single-entry consumers (vector replay, operator flows) continue
-- to work; this table is the complete authoritative set for revocation.
-- Existing pre-V37 entries are treated as chunk_index = 0 and remain
-- reachable via the decision row's vector_entry_id.
CREATE TABLE IF NOT EXISTS trace_gate_chunk_vector_entries (
    tenant_id       TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    decision_id     UUID NOT NULL,
    submission_id   UUID NOT NULL,
    chunk_index     INT NOT NULL CHECK (chunk_index >= 0),
    vector_entry_id UUID NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, decision_id, chunk_index)
);

CREATE INDEX IF NOT EXISTS idx_trace_gate_chunk_vector_entries_submission
    ON trace_gate_chunk_vector_entries (tenant_id, submission_id);

ALTER TABLE trace_gate_chunk_vector_entries ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_gate_chunk_vector_entries FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_gate_chunk_vector_entries;
CREATE POLICY trace_corpus_tenant_isolation ON trace_gate_chunk_vector_entries
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());
```

- [ ] **Step 2: Register V37 in the migration runner**

In `crates/trace-commons-server/src/db/postgres.rs`, locate the V36 registration block (`grep -n "V36" crates/trace-commons-server/src/db/postgres.rs` -> line ~1126) and add immediately after it, following the exact idempotent pattern used for V23/V24 (seen at lines 860-905):

```rust
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&37_i32],
            )
            .await?
            .is_some();
        if !already_applied {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V37__large_trace_chunked_scoring.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&37_i32, &"large_trace_chunked_scoring"],
                )
                .await?;
        }
```

Note (from project memory): the shared `trace_commons_test` DB already has migrations 30-34 applied and CI never runs PG tests; V37 applies cleanly on top. Do not renumber.

- [ ] **Step 3: Extend the storage row and gate-decision types (compile-driven)**

`crates/trace-commons-server/src/trace_corpus_storage.rs` — append to `TraceGateDecisionRow` (after `credit_withheld_reason`):

```rust
    /// Peak (most-surprising min-content-guarded chunk) perplexity in
    /// micros (migration V37). `None` on pre-chunking rows — readers treat
    /// `None` as "peak == representative" (single-chunk semantics).
    pub peak_perplexity_micros: Option<i64>,
    /// Peak per-chunk novelty in micros (migration V37). Same `None`
    /// semantics as `peak_perplexity_micros`.
    pub peak_novelty_micros: Option<i64>,
    /// Number of chunks scored (migration V37). `None` reads as 1.
    pub chunk_count: Option<i32>,
    /// True when the per-trace chunk cap dropped trailing chunks
    /// (migration V37). `None` reads as false.
    pub chunks_capped: Option<bool>,
```

`crates/trace-commons-server/src/trace_gate_service.rs` — append to `GateDecision` (after `vector_entry_id`):

```rust
    /// Peak (most-surprising min-content-guarded chunk) perplexity.
    pub peak_perplexity_micros: u64,
    /// Peak per-chunk novelty.
    pub peak_novelty_micros: u64,
    /// Number of chunks scored (>= 1; deterministic services report 1).
    pub chunk_count: u32,
    /// True when the per-trace chunk cap dropped trailing chunks.
    pub chunks_capped: bool,
```

Then let the compiler enumerate every construction site and fill each:

- `EnclaveGateService::evaluate_trace` (~485): map from the orchestrator decision:

```rust
            peak_perplexity_micros: decision.peak_perplexity_micros,
            peak_novelty_micros: decision.peak_novelty_micros,
            chunk_count: decision.chunk_count,
            chunks_capped: decision.chunks_capped,
```

- `InMemoryGateService::evaluate_trace` (~262) and `LegacyDeterministicGateService::evaluate_trace` (~326): single-chunk semantics — peak equals the representative value each impl already computed:

```rust
            peak_perplexity_micros: perplexity_micros,
            peak_novelty_micros: novelty_score_micros,
            chunk_count: 1,
            chunks_capped: false,
```

(use each impl's local variable names for the representative values; if the impl builds the struct inline from expressions, bind those expressions to locals first so peak reuses them.)

- `DstackGateService::evaluate_trace` (~394) returns an error today; if it constructs no decision, nothing to do.
- The synthetic `GateDecision` in `gate_evaluate_worker_handler` (`trace-commons-ingest.rs` ~45266, the credit-emission stub with zeroed fields): add `peak_perplexity_micros: 0, peak_novelty_micros: 0, chunk_count: 1, chunks_capped: false,`.
- Any `GateDecision`/`TraceGateDecisionRow` literals in `trace_gate_service.rs` tests and `trace_upload_claim_issuer.rs`/`tests.rs` mocks: same single-chunk defaults; for `TraceGateDecisionRow` literals use `peak_perplexity_micros: None, peak_novelty_micros: None, chunk_count: None, chunks_capped: None`.

Enumerate with:

Run: `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins 2>&1 | grep -E "^error" | head -50`
and fix each missing-field error as above. Also run `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run` to catch test-module literals (the extracted test file `trace_commons_ingest_internal/tests.rs` has three mock `Database` impls; their `insert_trace_gate_decision` signatures are unchanged).

- [ ] **Step 4: Map the fields in `evaluate_and_record_gate` and the PG queries**

`trace-commons-ingest.rs` ~44869, extend the row construction:

```rust
        peak_perplexity_micros: Some(
            i64::try_from(decision.peak_perplexity_micros).unwrap_or(i64::MAX),
        ),
        peak_novelty_micros: Some(
            i64::try_from(decision.peak_novelty_micros).unwrap_or(i64::MAX),
        ),
        chunk_count: Some(i32::try_from(decision.chunk_count).unwrap_or(i32::MAX)),
        chunks_capped: Some(decision.chunks_capped),
```

The skip-duplicate synthetic row at ~45036 gets `peak_perplexity_micros: None, peak_novelty_micros: None, chunk_count: None, chunks_capped: None`. `build_cost_control_decision_row` (~44934) uses `..template` and needs no change (cached rows copy the source row's peaks, consistent with copying its representative values).

`crates/trace-commons-server/src/db/trace_corpus_pg.rs`:

1. `insert_trace_gate_decision` (~5350): extend the column list and placeholders:

```rust
            "INSERT INTO trace_gate_decisions (
                 tenant_id, decision_id, submission_id, gate_policy_version,
                 gate_version_hash, perplexity_micros, tail_fraction_micros,
                 perplexity_passed, novelty_score_micros, nearest_neighbor_hash,
                 novelty_passed, embedding_evidence_hash, attestation_chain_hash,
                 decided_at, vector_entry_id, credit_withheld_reason,
                 peak_perplexity_micros, peak_novelty_micros, chunk_count, chunks_capped
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20)",
```

with the four new params appended to the `&[...]` array:

```rust
                &decision.peak_perplexity_micros,
                &decision.peak_novelty_micros,
                &decision.chunk_count,
                &decision.chunks_capped,
```

2. Both SELECTs in `stream_trace_gate_decisions_for_replay` (~5258-5289): append `, peak_perplexity_micros, peak_novelty_micros, chunk_count, chunks_capped` to the column list, and extend the row mapping (~5292):

```rust
                peak_perplexity_micros: row.get("peak_perplexity_micros"),
                peak_novelty_micros: row.get("peak_novelty_micros"),
                chunk_count: row.get("chunk_count"),
                chunks_capped: row.get("chunks_capped"),
```

3. `find_gate_decision_by_canonical_hash` (~5440): append the same four columns (`d.peak_perplexity_micros, d.peak_novelty_micros, d.chunk_count, d.chunks_capped`) and extend the indexed row mapping with `peak_perplexity_micros: row.get(15), peak_novelty_micros: row.get(16), chunk_count: row.get(17), chunks_capped: row.get(18),`.

Confirm no other full-column reads exist:
Run: `grep -n "credit_withheld_reason" crates/trace-commons-server/src/db/trace_corpus_pg.rs crates/trace-commons-server/src/db/postgres.rs`
Every hit that maps a `TraceGateDecisionRow` must be extended the same way (the aggregate/histogram queries in `postgres.rs:1495-1545` and the work-item enumeration at `postgres.rs:3585` do not select full rows and need no change).

- [ ] **Step 5: Parse the chunk-knob env vars and thread them into both builders**

In `trace-commons-ingest.rs`, add beside the existing gate consts (~line 226):

```rust
const TRACE_COMMONS_GATE_CHUNK_TARGET_TOKENS: &str = "TRACE_COMMONS_GATE_CHUNK_TARGET_TOKENS";
const TRACE_COMMONS_GATE_CHUNK_MAX_TOKENS: &str = "TRACE_COMMONS_GATE_CHUNK_MAX_TOKENS";
const TRACE_COMMONS_GATE_CHUNK_CAP: &str = "TRACE_COMMONS_GATE_CHUNK_CAP";
const TRACE_COMMONS_GATE_CHUNK_MIN_TOKENS: &str = "TRACE_COMMONS_GATE_CHUNK_MIN_TOKENS";
const TRACE_COMMONS_GATE_EMBED_INSERT_NOVELTY_MICROS: &str =
    "TRACE_COMMONS_GATE_EMBED_INSERT_NOVELTY_MICROS";
const TRACE_COMMONS_GATE_DEFAULT_CHUNK_TARGET_TOKENS: usize = 2048;
const TRACE_COMMONS_GATE_DEFAULT_CHUNK_MAX_TOKENS: usize = 3072;
const TRACE_COMMONS_GATE_DEFAULT_CHUNK_CAP: usize = 16;
const TRACE_COMMONS_GATE_DEFAULT_CHUNK_MIN_TOKENS: usize = 64;
const TRACE_COMMONS_GATE_DEFAULT_EMBED_INSERT_NOVELTY_MICROS: usize = 50_000;
```

and a parse-and-validate helper (place it near `parse_usize_env`, which it reuses):

```rust
/// Chunking knobs shared by every enclave gate-service flavor. Values are
/// operator-set and safe to surface in error strings (no secrets).
#[derive(Debug, Clone, Copy)]
struct GateChunkingEnvConfig {
    chunk_target_tokens: usize,
    chunk_max_tokens: usize,
    chunk_cap: usize,
    chunk_min_tokens: u64,
    embed_insert_novelty_micros: u64,
}

fn parse_gate_chunking_config_from_env() -> anyhow::Result<GateChunkingEnvConfig> {
    let chunk_target_tokens = parse_usize_env(
        TRACE_COMMONS_GATE_CHUNK_TARGET_TOKENS,
        TRACE_COMMONS_GATE_DEFAULT_CHUNK_TARGET_TOKENS,
    )?;
    let chunk_max_tokens = parse_usize_env(
        TRACE_COMMONS_GATE_CHUNK_MAX_TOKENS,
        TRACE_COMMONS_GATE_DEFAULT_CHUNK_MAX_TOKENS,
    )?;
    let chunk_cap = parse_usize_env(
        TRACE_COMMONS_GATE_CHUNK_CAP,
        TRACE_COMMONS_GATE_DEFAULT_CHUNK_CAP,
    )?;
    let chunk_min_tokens = parse_usize_env(
        TRACE_COMMONS_GATE_CHUNK_MIN_TOKENS,
        TRACE_COMMONS_GATE_DEFAULT_CHUNK_MIN_TOKENS,
    )?;
    let embed_insert_novelty_micros = parse_usize_env(
        TRACE_COMMONS_GATE_EMBED_INSERT_NOVELTY_MICROS,
        TRACE_COMMONS_GATE_DEFAULT_EMBED_INSERT_NOVELTY_MICROS,
    )?;
    anyhow::ensure!(
        chunk_target_tokens > 0,
        "{TRACE_COMMONS_GATE_CHUNK_TARGET_TOKENS} must be greater than zero"
    );
    anyhow::ensure!(
        chunk_max_tokens >= chunk_target_tokens,
        "{TRACE_COMMONS_GATE_CHUNK_MAX_TOKENS} must be >= {TRACE_COMMONS_GATE_CHUNK_TARGET_TOKENS}"
    );
    anyhow::ensure!(
        chunk_cap >= 1,
        "{TRACE_COMMONS_GATE_CHUNK_CAP} must be at least 1"
    );
    Ok(GateChunkingEnvConfig {
        chunk_target_tokens,
        chunk_max_tokens,
        chunk_cap,
        chunk_min_tokens: chunk_min_tokens as u64,
        embed_insert_novelty_micros: embed_insert_novelty_micros as u64,
    })
}
```

In BOTH `build_enclave_local_gpu_gate_service_from_env` (the `EnclaveGateOrchestratorConfig` construction at ~4630) and `build_enclave_near_ai_gate_service_from_env` (its parallel construction after ~4800), call the helper and extend the cfg:

```rust
    let chunking = parse_gate_chunking_config_from_env()?;
```

```rust
    let cfg = EnclaveGateOrchestratorConfig {
        gate_policy_version,
        gate_version_hash,
        perplexity_floor_micros,
        tail_fraction_floor_micros,
        novelty_floor_micros,
        top_k,
        chunk_target_tokens: chunking.chunk_target_tokens,
        chunk_max_tokens: chunking.chunk_max_tokens,
        chunk_cap: chunking.chunk_cap,
        chunk_min_tokens: chunking.chunk_min_tokens,
        embed_insert_novelty_micros: chunking.embed_insert_novelty_micros,
    };
```

Also fold the chunk knobs into `compute_gate_version_hash` so a knob change re-stamps the gate version: add five trailing parameters `chunk_target_tokens: usize, chunk_max_tokens: usize, chunk_cap: usize, chunk_min_tokens: u64, embed_insert_novelty_micros: u64` to its signature, hash them into the digest in order after the existing inputs (`h.update(chunk_target_tokens.to_be_bytes()); ...`), and pass `chunking.*` at both call sites.

- [ ] **Step 6: Write the env-parse test**

The ingest binary's test module is the extracted `trace_commons_ingest_internal/tests.rs`. Add:

```rust
    #[test]
    fn gate_chunking_env_defaults_and_validation() {
        // Defaults with no env set (tests must not set global env — the
        // parser reads process env, so only assert the default path, which
        // is what CI exercises).
        let cfg = parse_gate_chunking_config_from_env().expect("defaults parse");
        assert_eq!(cfg.chunk_target_tokens, 2048);
        assert_eq!(cfg.chunk_max_tokens, 3072);
        assert_eq!(cfg.chunk_cap, 16);
        assert_eq!(cfg.chunk_min_tokens, 64);
        assert_eq!(cfg.embed_insert_novelty_micros, 50_000);
    }
```

- [ ] **Step 7: Run tests + full verification**

Run: `cargo test -p trace-commons-server gate_chunking_env_defaults`
Expected: PASS.

Run: `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins && RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run && cargo clippy -p trace-commons-server --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`
Expected: clean.

(Optional, local-only, requires PostgreSQL: `cargo test -p trace-commons-server --test trace_corpus_pg_store` to confirm V37 applies to the shared test DB. CI never runs PG tests.)

- [ ] **Step 8: Commit**

```bash
git add migrations/V37__large_trace_chunked_scoring.sql crates/trace-commons-server/src/db/postgres.rs crates/trace-commons-server/src/db/trace_corpus_pg.rs crates/trace-commons-server/src/trace_corpus_storage.rs crates/trace-commons-server/src/trace_gate_service.rs crates/trace-commons-server/src/bin/trace-commons-ingest.rs crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs
git commit -m "persist peak perplexity and chunk bookkeeping via migration V37 and chunk-knob env config"
```

This closes the perplexity half: the endpoint-OOM fix (bounded chunk requests, `logprobs: 1`, sequential loop) is fully shippable at this commit.

---

### Task 6: Chunk embedding mean-pool helper

**Files:**
- Modify: `crates/trace-commons-gate-enclave/src/embedder.rs` (add constant + helper + tests)

**Interfaces:**
- Consumes: `Embedder::embed(&self, plaintext: &[u8]) -> Result<Vec<f32>>` (existing), `APPROX_CHARS_PER_TOKEN` (Task 1).
- Produces: `pub const EMBED_SUB_WINDOW_TOKENS: usize = 512;` and `pub fn embed_chunk_mean_pooled<E: Embedder + ?Sized>(embedder: &E, chunk_text: &str) -> anyhow::Result<Vec<f32>>` — consumed by Task 7.

- [ ] **Step 1: Write the failing tests**

Append to the test module of `crates/trace-commons-gate-enclave/src/embedder.rs`:

```rust
    #[test]
    fn short_chunk_mean_pool_equals_direct_embed() {
        // A chunk within one sub-window must embed identically to a direct
        // call — proves the helper is a no-op for small inputs.
        let e = MockEmbedder::new();
        let direct = e.embed(b"hello world").unwrap();
        let pooled = embed_chunk_mean_pooled(&e, "hello world").unwrap();
        assert_eq!(direct, pooled);
    }

    #[test]
    fn long_chunk_covers_all_sub_windows_not_just_the_first() {
        // Two chunks that share the same first sub-window but differ in the
        // second MUST produce different embeddings — the old truncation bug
        // would make them identical.
        let e = MockEmbedder::new();
        let window_chars = EMBED_SUB_WINDOW_TOKENS * crate::chunker::APPROX_CHARS_PER_TOKEN;
        let shared_head = "a".repeat(window_chars);
        let text_1 = format!("{shared_head}{}", "b".repeat(window_chars));
        let text_2 = format!("{shared_head}{}", "c".repeat(window_chars));
        let v1 = embed_chunk_mean_pooled(&e, &text_1).unwrap();
        let v2 = embed_chunk_mean_pooled(&e, &text_2).unwrap();
        assert_ne!(v1, v2, "tail content must influence the chunk embedding");
    }

    #[test]
    fn mean_pooled_embedding_is_unit_norm() {
        let e = MockEmbedder::new();
        let window_chars = EMBED_SUB_WINDOW_TOKENS * crate::chunker::APPROX_CHARS_PER_TOKEN;
        let text = "x".repeat(window_chars * 3 + 17);
        let v = embed_chunk_mean_pooled(&e, &text).unwrap();
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-3, "expected unit norm, got {norm}");
    }

    #[test]
    fn empty_chunk_embeds_the_empty_string() {
        let e = MockEmbedder::new();
        let direct = e.embed(b"").unwrap();
        let pooled = embed_chunk_mean_pooled(&e, "").unwrap();
        assert_eq!(direct, pooled);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p trace-commons-gate-enclave embedder`
Expected: FAIL — `embed_chunk_mean_pooled` / `EMBED_SUB_WINDOW_TOKENS` not defined.

- [ ] **Step 3: Implement the helper**

Add to `crates/trace-commons-gate-enclave/src/embedder.rs` after the `Embedder` trait:

```rust
/// Sub-window size for chunk embeddings, in tokens. Matches the fastembed
/// model family's ~512-token context; anything longer is silently truncated
/// by the model, which is exactly the bug this helper fixes.
pub const EMBED_SUB_WINDOW_TOKENS: usize = 512;

/// Embed a chunk (~2K tokens) with no truncation: split it into ≤512-token
/// (char-proxy) sub-windows, embed each, mean-pool, and L2-renormalize so
/// cosine similarity stays a dot product downstream. A chunk that fits in
/// one sub-window embeds identically to a direct `embed` call.
pub fn embed_chunk_mean_pooled<E: Embedder + ?Sized>(
    embedder: &E,
    chunk_text: &str,
) -> anyhow::Result<Vec<f32>> {
    let window_chars = EMBED_SUB_WINDOW_TOKENS * crate::chunker::APPROX_CHARS_PER_TOKEN;
    let char_count = chunk_text.chars().count();
    if char_count <= window_chars {
        return embedder.embed(chunk_text.as_bytes());
    }

    // Char-boundary-safe fixed windows.
    let mut windows: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut count = 0usize;
    for ch in chunk_text.chars() {
        current.push(ch);
        count += 1;
        if count == window_chars {
            windows.push(std::mem::take(&mut current));
            count = 0;
        }
    }
    if !current.is_empty() {
        windows.push(current);
    }

    let mut sum: Vec<f32> = Vec::new();
    for w in &windows {
        let v = embedder.embed(w.as_bytes())?;
        if sum.is_empty() {
            sum = vec![0.0; v.len()];
        }
        anyhow::ensure!(
            v.len() == sum.len(),
            "EmbedderSubWindowDimMismatch"
        );
        for (s, x) in sum.iter_mut().zip(v.iter()) {
            *s += x;
        }
    }
    let n = windows.len() as f32;
    for s in &mut sum {
        *s /= n;
    }
    let norm: f32 = sum.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for s in &mut sum {
            *s /= norm;
        }
    }
    Ok(sum)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p trace-commons-gate-enclave embedder`
Expected: PASS (existing 3 + new 4 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-gate-enclave/src/embedder.rs
git commit -m "add mean-pooled sub-window chunk embedding helper"
```

---

### Task 7: Per-chunk novelty in the orchestrator with dedup insert and peak wiring

**Files:**
- Modify: `crates/trace-commons-gate-enclave/src/orchestrator.rs` (embedding section of `evaluate`, `OrchestrationDecision`, evidence hashing, tests)

**Interfaces:**
- Consumes: `embed_chunk_mean_pooled` (Task 6); `aggregate_chunked_novelty` (Task 3); `ChunkPlan`/`TraceChunk` (Task 1); `cfg.embed_insert_novelty_micros`, `cfg.chunk_min_tokens` (Task 4).
- Produces: `pub struct InsertedChunkEntry { pub chunk_index: u32, pub entry_id: Uuid }`; `OrchestrationDecision` gains `pub inserted_chunk_entries: Vec<InsertedChunkEntry>`; `peak_novelty_micros` becomes the real guarded per-chunk max; `inserted_entry_id` remains `Some(first inserted entry)` for back-compat. Consumed by Task 8.

- [ ] **Step 1: Write the failing tests**

Append to the orchestrator test module:

```rust
    #[test]
    fn duplicate_chunk_is_deduped_on_insert_but_novel_chunks_insert_per_chunk() {
        // First trace: 2 distinct large chunks -> both insert.
        let orch = orch_with_floors(0, 0, 0);
        let first = orch
            .evaluate(&two_distinct_chunk_envelope("alpha", "beta"), "tenant_a")
            .unwrap();
        assert_eq!(first.chunk_count, 2);
        assert_eq!(first.inserted_chunk_entries.len(), 2);
        assert_eq!(first.inserted_chunk_entries[0].chunk_index, 0);
        assert_eq!(first.inserted_chunk_entries[1].chunk_index, 1);
        assert_eq!(
            first.inserted_entry_id,
            Some(first.inserted_chunk_entries[0].entry_id)
        );

        // Second trace: chunk 0 duplicates the first trace's chunk 0
        // (novelty ~0 < insert threshold -> skipped); chunk 1 is new.
        let second = orch
            .evaluate(&two_distinct_chunk_envelope("alpha", "gamma"), "tenant_a")
            .unwrap();
        assert_eq!(second.chunk_count, 2);
        assert_eq!(
            second.inserted_chunk_entries.len(),
            1,
            "near-duplicate chunk must be skipped at the insert threshold"
        );
        assert_eq!(second.inserted_chunk_entries[0].chunk_index, 1);
    }

    #[test]
    fn peak_novelty_is_max_over_chunks_and_representative_is_weighted() {
        let orch = orch_with_floors(0, 0, 0);
        // Seed the index with one trace.
        orch.evaluate(&two_distinct_chunk_envelope("alpha", "beta"), "tenant_a")
            .unwrap();
        // Re-submit: chunk "alpha" is a duplicate (novelty ~0), chunk
        // "delta" is fresh (novelty ~1.0). Peak must reflect the fresh
        // chunk; representative must sit strictly between.
        let d = orch
            .evaluate(&two_distinct_chunk_envelope("alpha", "delta"), "tenant_a")
            .unwrap();
        assert!(d.peak_novelty_micros > 900_000, "fresh chunk drives the peak");
        assert!(
            d.novelty_score_micros < d.peak_novelty_micros,
            "representative must be dragged down by the duplicate chunk"
        );
    }

    #[test]
    fn failed_gate_inserts_no_chunk_entries() {
        let orch = orch_with_floors(u64::MAX, 0, 0);
        let d = orch
            .evaluate(&two_distinct_chunk_envelope("alpha", "beta"), "tenant_a")
            .unwrap();
        assert!(!d.perplexity_passed);
        assert!(d.inserted_chunk_entries.is_empty());
        assert!(d.inserted_entry_id.is_none());
    }

    /// Envelope with exactly two chunks: each event's rendered form fills a
    /// whole target chunk (8192 chars at the default 2048-token target).
    fn two_distinct_chunk_envelope(seed_a: &str, seed_b: &str) -> Vec<u8> {
        let pad = |seed: &str| {
            let mut s = String::new();
            while s.len() < 8_000 {
                s.push_str(seed);
                s.push(' ');
            }
            s
        };
        serde_json::to_vec(&serde_json::json!({
            "events": [
                { "event_type": "assistant_message", "redacted_content": pad(seed_a) },
                { "event_type": "assistant_message", "redacted_content": pad(seed_b) },
            ]
        }))
        .unwrap()
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p trace-commons-gate-enclave orchestrator`
Expected: FAIL — `inserted_chunk_entries` not defined; dedup/peak behavior missing.

- [ ] **Step 3: Implement per-chunk embedding + novelty + insert**

In `orchestrator.rs`, add above `OrchestrationDecision`:

```rust
/// A per-chunk vector-index entry the orchestrator inserted. The host maps
/// these to `(submission_id, chunk_index)` rows in
/// `trace_gate_chunk_vector_entries` for revocation tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InsertedChunkEntry {
    pub chunk_index: u32,
    pub entry_id: Uuid,
}
```

add to `OrchestrationDecision`:

```rust
    /// Every chunk entry inserted into the vector index (both gates passed,
    /// per-chunk novelty at or above the insert threshold). Empty on fail.
    pub inserted_chunk_entries: Vec<InsertedChunkEntry>,
```

then replace the embedding section of `evaluate` (everything from the whole-plaintext `let embedding = ...` through the insert block) with:

```rust
        // Per-chunk embedding: mean-pooled sub-windows so the vector covers
        // the whole chunk (no 512-token truncation), then per-chunk novelty
        // against the tenant's existing per-chunk entries. Fail-closed: any
        // chunk's embedder/index error refuses the evaluation.
        let mut chunk_embeddings: Vec<Vec<f32>> = Vec::with_capacity(plan.chunks.len());
        let mut chunk_novelty_micros: Vec<u64> = Vec::with_capacity(plan.chunks.len());
        let mut all_neighbors: Vec<crate::vector_index::NearestNeighbor> = Vec::new();
        for chunk in &plan.chunks {
            let emb = crate::embedder::embed_chunk_mean_pooled(&self.embedder, &chunk.text)
                .context("EmbedderInferenceFailed")?;
            let neighbors = self
                .index
                .nearest(tenant_storage_ref, &emb, self.cfg.top_k)?;
            let max_sim = neighbors
                .iter()
                .map(|n| n.similarity)
                .fold(f32::NEG_INFINITY, f32::max);
            let novelty_f = if max_sim.is_finite() {
                (1.0 - max_sim).max(0.0)
            } else {
                1.0
            };
            chunk_novelty_micros.push((novelty_f.clamp(0.0, 2.0) * 1_000_000.0) as u64);
            all_neighbors.extend(neighbors);
            chunk_embeddings.push(emb);
        }
        let chunk_token_counts: Vec<u64> = chunk_scores.iter().map(|c| c.tokens).collect();
        let (novelty_score_micros, peak_novelty_micros) =
            crate::chunk_aggregate::aggregate_chunked_novelty(
                &chunk_novelty_micros,
                &chunk_token_counts,
                self.cfg.chunk_min_tokens,
            );

        let perplexity_passed = perp_agg.representative_perplexity_micros
            >= self.cfg.perplexity_floor_micros
            && perp_agg.tail_fraction_micros >= self.cfg.tail_fraction_floor_micros;
        let novelty_passed = novelty_score_micros >= self.cfg.novelty_floor_micros;

        let nearest_neighbor_hash = hash_neighbors(&all_neighbors);
        let embedding_evidence_hash = hash_chunk_embedding_evidence(
            &self.cfg.gate_policy_version,
            tenant_storage_ref,
            &chunk_embeddings,
        );
        let attestation_chain_hash =
            hash_attestation_chain(&self.cfg.gate_policy_version, &self.cfg.gate_version_hash);

        // Insert per-chunk entries: both gates passed AND the chunk clears
        // the near-duplicate threshold. Novelties were all computed against
        // the pre-trace index (matches the design pseudocode), so intra-
        // trace duplicate chunks each measure against prior traces only.
        let mut inserted_chunk_entries: Vec<InsertedChunkEntry> = Vec::new();
        if perplexity_passed && novelty_passed {
            for (i, emb) in chunk_embeddings.iter().enumerate() {
                if chunk_novelty_micros[i] < self.cfg.embed_insert_novelty_micros {
                    continue;
                }
                let entry_id = Uuid::new_v4();
                self.index.insert(entry_id, tenant_storage_ref, emb)?;
                inserted_chunk_entries.push(InsertedChunkEntry {
                    chunk_index: plan.chunks[i].chunk_index,
                    entry_id,
                });
            }
        }
        let inserted_entry_id = inserted_chunk_entries.first().map(|e| e.entry_id);
```

and update the final struct literal:

```rust
        Ok(OrchestrationDecision {
            gate_policy_version: self.cfg.gate_policy_version.clone(),
            gate_version_hash: self.cfg.gate_version_hash.clone(),
            perplexity_micros: perp_agg.representative_perplexity_micros,
            tail_fraction_micros: perp_agg.tail_fraction_micros,
            perplexity_passed,
            novelty_score_micros,
            nearest_neighbor_hash,
            novelty_passed,
            embedding_evidence_hash,
            attestation_chain_hash,
            inserted_entry_id,
            peak_perplexity_micros: perp_agg.peak_perplexity_micros,
            peak_novelty_micros,
            chunk_count: plan.chunks.len() as u32,
            chunks_capped: plan.chunks_capped,
            inserted_chunk_entries,
        })
```

Add the multi-chunk evidence hash beside `hash_embedding_evidence` (keep the old function only if still referenced; otherwise replace it):

```rust
fn hash_chunk_embedding_evidence(
    gate_policy_version: &str,
    tenant_storage_ref: &str,
    chunk_embeddings: &[Vec<f32>],
) -> String {
    let mut h = Sha256::new();
    h.update(b"trace_gate_enclave.embedding_evidence.v2\n");
    h.update(gate_policy_version.as_bytes());
    h.update(b"\n");
    h.update(tenant_storage_ref.as_bytes());
    h.update(b"\n");
    for (i, embedding) in chunk_embeddings.iter().enumerate() {
        h.update((i as u32).to_be_bytes());
        h.update(b"\n");
        for x in embedding {
            h.update(x.to_be_bytes());
        }
        h.update(b"\n");
    }
    format!("sha256:{:x}", h.finalize())
}
```

Existing tests to reconcile (behavior, not weakenings): `inserted_trace_is_its_own_nearest_neighbor_next_time` and `high_novelty_floor_fails_a_duplicate_trace` still pass — a single-chunk `b"hello world"` trace inserts one chunk entry whose embedding equals the direct embed, so the second evaluation still sees cosine ~1.0. Task 4's test `single_chunk_trace_matches_whole_call_on_rendered_text` gets `assert_eq!(d.inserted_chunk_entries.len(), 1);` appended.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p trace-commons-gate-enclave`
Expected: PASS (all).

- [ ] **Step 5: Verify the server still compiles**

Run: `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins`
Expected: clean (`inserted_chunk_entries` is not yet consumed host-side; `EnclaveGateService` maps named fields, so an added field does not break it).

- [ ] **Step 6: Commit**

```bash
git add crates/trace-commons-gate-enclave/src/orchestrator.rs
git commit -m "embed and score novelty per chunk with dedup insert and guarded peak"
```

---

### Task 8: Per-chunk vector-entry persistence and revocation tracking

**Files:**
- Modify: `crates/trace-commons-server/src/trace_corpus_storage.rs` (new `TraceGateChunkVectorEntryRow`, two new `Database` trait methods with defaults)
- Modify: `crates/trace-commons-server/src/db/trace_corpus_pg.rs` (PG implementations)
- Modify: `crates/trace-commons-server/src/trace_gate_service.rs` (`GateDecision.chunk_vector_entries`, `GateChunkVectorEntry`; `EnclaveGateService::evaluate_trace` mapping; other impls set `vec![]`)
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` (`evaluate_and_record_gate` ~44861-44900; new `enqueue_vector_entry_invalidation_items_for_revocation` beside `enqueue_worker_queue_invalidation_items_for_revocation` ~50035, wired at its call sites)

**Design rationale (settled here, not open):** a child table (`trace_gate_chunk_vector_entries`, created by Task 5's V37) is used instead of a UUID-array/JSON column on the decision row because (a) the revocation machinery is per-entry: `TraceRevocationPropagationTarget::VectorEntry { vector_entry_id }` items and the `is_vector_entry_revoked` check (`trace_corpus_pg.rs:5314-5343`) match a single `target_json ->> 'vector_entry_id'`; (b) the existing `trace_vector_entries` table already models one-row-per-entry with `(tenant_id, submission_id, vector_entry_id)` conflict keys; (c) `stream_trace_gate_decisions_for_replay` filters `vector_entry_id IS NOT NULL` and continues to work unchanged with the decision row's first-entry column. Legacy pre-V37 rows read as `chunk_index = 0` via the decision row's `vector_entry_id`.

**Interfaces:**
- Consumes: `OrchestrationDecision.inserted_chunk_entries` (Task 7); V37 table (Task 5); existing planner pattern `enqueue_worker_queue_invalidation_items_for_revocation` (`trace-commons-ingest.rs:50035`); existing worker branch `StorageTraceRevocationPropagationAction::InvalidateVector` (`trace-commons-ingest.rs:50525-50556`) which needs NO change — it already invalidates one entry id per item.
- Produces:
  - `trace_corpus_storage::TraceGateChunkVectorEntryRow { decision_id: Uuid, submission_id: Uuid, chunk_index: i32, vector_entry_id: Uuid }`
  - `Database::insert_trace_gate_decision_with_chunk_entries(&self, tenant_id: &str, decision: TraceGateDecisionRow, chunk_entries: Vec<TraceGateChunkVectorEntryRow>) -> Result<(), DatabaseError>`
  - `Database::list_trace_gate_chunk_vector_entries(&self, tenant_id: &str, submission_id: Uuid) -> Result<Vec<TraceGateChunkVectorEntryRow>, DatabaseError>`
  - `trace_gate_service::GateChunkVectorEntry { chunk_index: u32, vector_entry_id: Uuid }`; `GateDecision.chunk_vector_entries: Vec<GateChunkVectorEntry>`

- [ ] **Step 1: Add the storage row type and trait methods (defaults keep test doubles compiling)**

In `crates/trace-commons-server/src/trace_corpus_storage.rs`, after `TraceGateDecisionRow`:

```rust
/// One per-chunk vector-index entry row (`trace_gate_chunk_vector_entries`,
/// migration V37). Keyed `(tenant_id, decision_id, chunk_index)`; the
/// complete authoritative entry set for a decision. The decision row's
/// legacy `vector_entry_id` column keeps holding the FIRST entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceGateChunkVectorEntryRow {
    pub decision_id: Uuid,
    pub submission_id: Uuid,
    pub chunk_index: i32,
    pub vector_entry_id: Uuid,
}
```

and in the `Database` trait (beside `insert_trace_gate_decision`, ~2317):

```rust
    /// Insert a gate-decision row together with its per-chunk vector-entry
    /// rows, atomically (one transaction). The default delegates to
    /// `insert_trace_gate_decision` and DROPS the chunk entries — acceptable
    /// only for non-PG test doubles; the PG impl overrides this.
    async fn insert_trace_gate_decision_with_chunk_entries(
        &self,
        tenant_id: &str,
        decision: TraceGateDecisionRow,
        _chunk_entries: Vec<TraceGateChunkVectorEntryRow>,
    ) -> Result<(), DatabaseError> {
        self.insert_trace_gate_decision(tenant_id, decision).await
    }

    /// List all per-chunk vector entries recorded for a submission (all of
    /// its decisions). Default returns empty for non-PG test doubles.
    async fn list_trace_gate_chunk_vector_entries(
        &self,
        _tenant_id: &str,
        _submission_id: Uuid,
    ) -> Result<Vec<TraceGateChunkVectorEntryRow>, DatabaseError> {
        Ok(Vec::new())
    }
```

- [ ] **Step 2: Write the failing PG test (PG-gated; CI never runs it, run locally if PostgreSQL is available)**

Find the existing PG store test harness: `crates/trace-commons-server/tests/trace_corpus_pg_store.rs` (run via `cargo test -p trace-commons-server --test trace_corpus_pg_store`). Add, following the file's existing setup helpers for constructing the store and a tenant (reuse its existing fixture functions for pool/tenant creation — copy the setup lines of the nearest gate-decision test in that file verbatim):

```rust
#[tokio::test]
async fn chunk_vector_entries_insert_atomically_and_list_by_submission() {
    // Reuse this file's standard setup: build the PG store and a tenant the
    // same way the neighboring trace_gate_decisions test does.
    let (db, tenant_id) = setup_store_and_tenant().await;

    let submission_id = Uuid::new_v4();
    let decision_id = Uuid::new_v4();
    let row = sample_gate_decision_row(decision_id, submission_id); // helper mirroring existing tests
    let entries = vec![
        TraceGateChunkVectorEntryRow {
            decision_id,
            submission_id,
            chunk_index: 0,
            vector_entry_id: Uuid::new_v4(),
        },
        TraceGateChunkVectorEntryRow {
            decision_id,
            submission_id,
            chunk_index: 1,
            vector_entry_id: Uuid::new_v4(),
        },
    ];
    db.insert_trace_gate_decision_with_chunk_entries(&tenant_id, row, entries.clone())
        .await
        .expect("atomic insert");

    let listed = db
        .list_trace_gate_chunk_vector_entries(&tenant_id, submission_id)
        .await
        .expect("list");
    assert_eq!(listed, entries);

    // Tenant isolation: a different tenant must see nothing (RLS).
    let (_, other_tenant) = setup_store_and_tenant().await;
    let cross = db
        .list_trace_gate_chunk_vector_entries(&other_tenant, submission_id)
        .await
        .expect("cross-tenant list");
    assert!(cross.is_empty());
}
```

If `setup_store_and_tenant` / `sample_gate_decision_row` do not exist under those names, use the file's actual fixture functions (the test body's assertions are the contract; the setup lines mirror the neighboring gate-decision test). If the submission-FK pattern in this repo requires a real `trace_submissions` row first, insert one the same way the neighboring test does.

- [ ] **Step 3: Run the test to verify it fails** (local PG only)

Run: `cargo test -p trace-commons-server --test trace_corpus_pg_store chunk_vector_entries -- --nocapture`
Expected: FAIL — PG impl missing (default drops entries, list returns empty -> assertion fails). Without local PG, proceed on compile-checks and rely on Step 6's non-PG verification.

- [ ] **Step 4: Implement the PG methods**

In `crates/trace-commons-server/src/db/trace_corpus_pg.rs`, next to `insert_trace_gate_decision`:

```rust
    async fn insert_trace_gate_decision_with_chunk_entries(
        &self,
        tenant_id: &str,
        decision: TraceGateDecisionRow,
        chunk_entries: Vec<TraceGateChunkVectorEntryRow>,
    ) -> Result<(), DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        tx.execute(
            "INSERT INTO trace_gate_decisions (
                 tenant_id, decision_id, submission_id, gate_policy_version,
                 gate_version_hash, perplexity_micros, tail_fraction_micros,
                 perplexity_passed, novelty_score_micros, nearest_neighbor_hash,
                 novelty_passed, embedding_evidence_hash, attestation_chain_hash,
                 decided_at, vector_entry_id, credit_withheld_reason,
                 peak_perplexity_micros, peak_novelty_micros, chunk_count, chunks_capped
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20)",
            &[
                &tenant_id,
                &decision.decision_id,
                &decision.submission_id,
                &decision.gate_policy_version,
                &decision.gate_version_hash,
                &decision.perplexity_micros,
                &decision.tail_fraction_micros,
                &decision.perplexity_passed,
                &decision.novelty_score_micros,
                &decision.nearest_neighbor_hash,
                &decision.novelty_passed,
                &decision.embedding_evidence_hash,
                &decision.attestation_chain_hash,
                &decision.decided_at,
                &decision.vector_entry_id,
                &decision.credit_withheld_reason,
                &decision.peak_perplexity_micros,
                &decision.peak_novelty_micros,
                &decision.chunk_count,
                &decision.chunks_capped,
            ],
        )
        .await
        .map_err(DatabaseError::Postgres)?;
        for entry in &chunk_entries {
            tx.execute(
                "INSERT INTO trace_gate_chunk_vector_entries (
                     tenant_id, decision_id, submission_id, chunk_index, vector_entry_id
                 ) VALUES ($1,$2,$3,$4,$5)",
                &[
                    &tenant_id,
                    &entry.decision_id,
                    &entry.submission_id,
                    &entry.chunk_index,
                    &entry.vector_entry_id,
                ],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        }
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(())
    }

    async fn list_trace_gate_chunk_vector_entries(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
    ) -> Result<Vec<TraceGateChunkVectorEntryRow>, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT decision_id, submission_id, chunk_index, vector_entry_id
                 FROM trace_gate_chunk_vector_entries
                 WHERE tenant_id = $1 AND submission_id = $2
                 ORDER BY decision_id, chunk_index",
                &[&tenant_id, &submission_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(rows
            .into_iter()
            .map(|row| TraceGateChunkVectorEntryRow {
                decision_id: row.get("decision_id"),
                submission_id: row.get("submission_id"),
                chunk_index: row.get("chunk_index"),
                vector_entry_id: row.get("vector_entry_id"),
            })
            .collect())
    }
```

Add `TraceGateChunkVectorEntryRow` to the `use crate::trace_corpus_storage::{...}` import list at the top of the file.

- [ ] **Step 5: Thread chunk entries through the gate service and host**

`trace_gate_service.rs` — add beside `GateDecision`:

```rust
/// One inserted per-chunk vector-index entry, host-facing form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateChunkVectorEntry {
    pub chunk_index: u32,
    pub vector_entry_id: Uuid,
}
```

add to `GateDecision`:

```rust
    /// Every per-chunk vector-index entry the gate inserted. Empty for
    /// deterministic/legacy services and failed gates. The host persists
    /// these as (submission_id, chunk_index)-tagged rows for revocation.
    pub chunk_vector_entries: Vec<GateChunkVectorEntry>,
```

`EnclaveGateService::evaluate_trace` maps:

```rust
            chunk_vector_entries: decision
                .inserted_chunk_entries
                .iter()
                .map(|e| GateChunkVectorEntry {
                    chunk_index: e.chunk_index,
                    vector_entry_id: e.entry_id,
                })
                .collect(),
```

All other `GateDecision` constructions (the two deterministic services, the ingest synthetic stub at ~45266, tests): `chunk_vector_entries: Vec::new(),`.

`evaluate_and_record_gate` (`trace-commons-ingest.rs` ~44895) — replace the insert call:

```rust
    let chunk_entries: Vec<StorageTraceGateChunkVectorEntryRow> = decision
        .chunk_vector_entries
        .iter()
        .map(|e| StorageTraceGateChunkVectorEntryRow {
            decision_id,
            submission_id,
            chunk_index: i32::try_from(e.chunk_index).unwrap_or(i32::MAX),
            vector_entry_id: e.vector_entry_id,
        })
        .collect();
    db.insert_trace_gate_decision_with_chunk_entries(tenant_id, row, chunk_entries)
        .await?;
```

(add `TraceGateChunkVectorEntryRow as StorageTraceGateChunkVectorEntryRow` to the ingest binary's existing `use crate::trace_corpus_storage::{...}`-style alias imports, matching how `StorageTraceGateDecisionRow` is aliased there.)

- [ ] **Step 6: Revocation planner for per-chunk entries**

In `trace-commons-ingest.rs`, add beside `enqueue_worker_queue_invalidation_items_for_revocation` (~50035), following its exact idempotency pattern:

```rust
/// Enqueue one `InvalidateVector` propagation item per per-chunk vector
/// entry recorded for the submission (V37 `trace_gate_chunk_vector_entries`).
/// The existing propagation worker's `InvalidateVector` branch consumes the
/// items unchanged — one entry id per item. Legacy pre-V37 decisions carry
/// no chunk rows and are unaffected. Idempotent per entry id.
async fn enqueue_vector_entry_invalidation_items_for_revocation(
    db: &dyn Database,
    tenant_id: &str,
    submission_id: Uuid,
) -> anyhow::Result<usize> {
    let existing_idempotency_keys = db
        .list_trace_revocation_propagation_items(tenant_id, submission_id)
        .await
        .context("failed to read existing revocation propagation items")?
        .into_iter()
        .map(|item| item.idempotency_key)
        .collect::<BTreeSet<_>>();
    let mut enqueued = 0usize;
    for entry in db
        .list_trace_gate_chunk_vector_entries(tenant_id, submission_id)
        .await
        .context("failed to read chunk vector entries for revocation")?
    {
        let idempotency_key = sha256_prefixed(&format!(
            "trace_revocation_chunk_vector_entry_invalidation:v1:{tenant_id}:{submission_id}:{}",
            entry.vector_entry_id
        ));
        if existing_idempotency_keys.contains(&idempotency_key) {
            continue;
        }
        db.upsert_trace_revocation_propagation_item(StorageTraceRevocationPropagationItemWrite {
            tenant_id: tenant_id.to_string(),
            propagation_item_id: deterministic_trace_uuid_for_external_ref(
                "revocation-chunk-vector-entry-invalidation",
                tenant_id,
                submission_id,
                &entry.vector_entry_id.to_string(),
            ),
            source_submission_id: submission_id,
            target: StorageTraceRevocationPropagationTarget::VectorEntry {
                vector_entry_id: entry.vector_entry_id,
            },
            action: StorageTraceRevocationPropagationAction::InvalidateVector,
            status: StorageTraceRevocationPropagationItemStatus::Pending,
            idempotency_key,
            reason: "revoked trace per-chunk vector entry invalidation".to_string(),
            attempt_count: 0,
            last_error: None,
            next_attempt_at: None,
            completed_at: None,
            evidence_hash: None,
            metadata: BTreeMap::from([(
                "source".to_string(),
                "mirror_revocation_to_db".to_string(),
            )]),
        })
        .await
        .context("failed to upsert chunk vector entry invalidation propagation item")?;
        enqueued += 1;
    }
    Ok(enqueued)
}
```

Wire it at every call site of the sibling planner:

Run: `grep -n "enqueue_worker_queue_invalidation_items_for_revocation(" crates/trace-commons-server/src/bin/trace-commons-ingest.rs`
At each production call site (not the fn definition), add directly after it, mirroring its error-handling shape:

```rust
    enqueue_vector_entry_invalidation_items_for_revocation(db, tenant_id, submission_id).await?;
```

(match the surrounding call's exact receiver/argument spelling — e.g. `db.as_ref()` vs `db` — and error propagation, `?` vs `.context(...)?`.)

- [ ] **Step 7: Write the failing handler-level test**

In `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs`, add a unit test for the planner using the file's existing mock `Database` pattern (the mocks now inherit the default `list_trace_gate_chunk_vector_entries -> empty`; override it in one mock):

```rust
    #[tokio::test]
    async fn revocation_enqueues_one_item_per_chunk_vector_entry() {
        // Mock DB that reports two chunk entries and records upserts.
        // Reuse this file's smallest existing mock Database struct; override
        // list_trace_gate_chunk_vector_entries to return two rows and
        // upsert_trace_revocation_propagation_item to push into a Vec.
        let db = MockDbWithChunkEntries::new(vec![
            TraceGateChunkVectorEntryRow {
                decision_id: Uuid::new_v4(),
                submission_id: SUBMISSION,
                chunk_index: 0,
                vector_entry_id: Uuid::new_v4(),
            },
            TraceGateChunkVectorEntryRow {
                decision_id: Uuid::new_v4(),
                submission_id: SUBMISSION,
                chunk_index: 1,
                vector_entry_id: Uuid::new_v4(),
            },
        ]);
        let enqueued =
            enqueue_vector_entry_invalidation_items_for_revocation(&db, "tenant-a", SUBMISSION)
                .await
                .expect("planner");
        assert_eq!(enqueued, 2);
        let items = db.recorded_propagation_items();
        assert_eq!(items.len(), 2);
        for item in &items {
            assert!(matches!(
                item.target,
                StorageTraceRevocationPropagationTarget::VectorEntry { .. }
            ));
            assert!(matches!(
                item.action,
                StorageTraceRevocationPropagationAction::InvalidateVector
            ));
        }
        // Idempotent: a second run enqueues nothing... only if the mock
        // reflects prior items in list_trace_revocation_propagation_items;
        // make the mock do so.
        let again =
            enqueue_vector_entry_invalidation_items_for_revocation(&db, "tenant-a", SUBMISSION)
                .await
                .expect("planner rerun");
        assert_eq!(again, 0);
    }
```

(`MockDbWithChunkEntries` is a new small mock in the same file: clone the struct + `impl Database` skeleton of the file's existing smallest gate-decision mock — e.g. the one at the `insert_trace_gate_decision` impl near line 63921 — storing `Mutex<Vec<TraceRevocationPropagationItemWrite>>` and the fixed entry list, overriding exactly `list_trace_gate_chunk_vector_entries`, `list_trace_revocation_propagation_items`, and `upsert_trace_revocation_propagation_item`; all other methods keep that mock's unimplemented/default bodies.)

- [ ] **Step 8: Run tests**

Run: `cargo test -p trace-commons-server revocation_enqueues_one_item_per_chunk_vector_entry`
Expected: PASS after implementation (FAIL before Step 6 lands — run once before to confirm).

Run: `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins && RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run && cargo clippy -p trace-commons-server --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`
Expected: clean.

Local PG (optional): `cargo test -p trace-commons-server --test trace_corpus_pg_store chunk_vector_entries`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add crates/trace-commons-server/src/trace_corpus_storage.rs crates/trace-commons-server/src/db/trace_corpus_pg.rs crates/trace-commons-server/src/trace_gate_service.rs crates/trace-commons-server/src/bin/trace-commons-ingest.rs crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs crates/trace-commons-server/tests/trace_corpus_pg_store.rs
git commit -m "persist per-chunk vector entries and enqueue per-entry revocation items"
```

---

### Task 9: End-to-end integration test + operator runbook

**Files:**
- Modify: `crates/trace-commons-server/src/trace_gate_service.rs` (integration test in its existing test module, which already exercises `EnclaveGateService` end-to-end with mock scorer/embedder/index and a real KEK wrapper — see its tests at lines 623-745)
- Create: `docs/operator/large-trace-chunked-scoring.md`
- Modify: `docs/operator/README.md` (index line)

**Interfaces:**
- Consumes: everything produced by Tasks 1-8. No new symbols produced.

- [ ] **Step 1: Write the failing end-to-end test**

In the `trace_gate_service.rs` test module, alongside the existing enclave-service tests (reuse their exact encrypt-envelope helper — the function the test at line ~623 uses to build `(ciphertext, wrapped_dek)` from plaintext bytes; call it the same way, do not invent a new one):

```rust
    #[test]
    fn multi_chunk_trace_records_representative_and_peak_end_to_end() {
        // Build a multi-chunk envelope: 20 events x 8000 chars -> 20 target
        // chunks -> capped at 16.
        let pad = "a".repeat(8_000);
        let events: Vec<serde_json::Value> = (0..20)
            .map(|i| {
                serde_json::json!({
                    "event_type": "assistant_message",
                    "redacted_content": format!("{i}:{pad}"),
                })
            })
            .collect();
        let plaintext =
            serde_json::to_vec(&serde_json::json!({ "events": events })).unwrap();

        let svc = mock_enclave_service(); // this module's existing mock-service fixture
        let tenant = TenantCtx::new("tenant-e2e");
        let (ciphertext, wrapped_dek) = encrypt_test_envelope(&svc, &tenant, &plaintext);
        let d = svc
            .evaluate_trace(
                &tenant,
                &ciphertext,
                &wrapped_dek,
                TraceArtifactKind::ContributionEnvelope,
            )
            .expect("multi-chunk evaluate_trace succeeds");

        assert_eq!(d.chunk_count, 16, "cap must bound the chunk count");
        assert!(d.chunks_capped);
        assert!(d.perplexity_micros > 0);
        assert!(d.peak_perplexity_micros >= d.perplexity_micros || d.chunk_count == 1);
        assert!(d.peak_novelty_micros >= d.novelty_score_micros);
        // Fresh tenant: everything is novel, both gates pass at zero floors,
        // and per-chunk entries land (16 chunks, all above the insert
        // threshold on an empty index).
        assert!(d.perplexity_passed && d.novelty_passed);
        assert_eq!(d.chunk_vector_entries.len(), 16);
        assert_eq!(
            d.vector_entry_id,
            Some(d.chunk_vector_entries[0].vector_entry_id)
        );

        // Re-submit the same trace: every chunk is now a near-duplicate.
        let (ciphertext2, wrapped_dek2) = encrypt_test_envelope(&svc, &tenant, &plaintext);
        let d2 = svc
            .evaluate_trace(
                &tenant,
                &ciphertext2,
                &wrapped_dek2,
                TraceArtifactKind::ContributionEnvelope,
            )
            .expect("duplicate evaluate_trace succeeds");
        assert!(
            d2.novelty_score_micros < 50_000,
            "duplicate trace novelty must collapse below the insert threshold"
        );
        assert!(
            d2.chunk_vector_entries.is_empty() || !d2.novelty_passed,
            "duplicate chunks must be deduped on insert"
        );
    }
```

Notes for the implementer: `mock_enclave_service()` / `encrypt_test_envelope(...)` stand for this module's ACTUAL existing fixture names — open the test at `trace_gate_service.rs:623-745`, reuse the identical setup lines it uses to construct the mock `EnclaveGateService` + encrypted envelope, and inline them if no named helper exists. The fail-closed-on-chunk-error case is already covered at the orchestrator layer (Task 4's `chunk_scorer_error_fails_the_whole_evaluation`) and the embedder-error propagation case already exists in this module (test at ~750-793); do not duplicate them here.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p trace-commons-server multi_chunk_trace_records_representative_and_peak_end_to_end`
Expected: FAIL only if any wiring from Tasks 4-8 is incomplete; if it passes first try, temporarily flip one assertion (e.g. expect `chunk_count == 20`) to prove the test executes the chunked path, observe the failure, and restore it.

- [ ] **Step 3: Run test to verify it passes**

Run: `cargo test -p trace-commons-server multi_chunk_trace_records_representative_and_peak_end_to_end`
Expected: PASS.

- [ ] **Step 4: Write the operator runbook**

Create `docs/operator/large-trace-chunked-scoring.md`:

```markdown
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

Revoking a submission enqueues one `invalidate_vector` propagation item per
recorded chunk entry (plus the legacy single-entry flows for pre-V37 rows).
The propagation worker is unchanged: one vector-entry id per item.

## Failure semantics

Fail-closed v1: any chunk's scorer/embedder error fails the whole
evaluation; the scoring driver retries with backoff via
`trace_gate_evaluation_attempts`. Chunks are scored sequentially per trace —
never a concurrent burst against one pinned backend.
```

Add to the runbook index in `docs/operator/README.md` (match the file's existing list format):

```markdown
- [Large-trace chunked scoring](large-trace-chunked-scoring.md) — chunking knobs, peak/representative columns, per-chunk revocation.
```

- [ ] **Step 5: Full verification sweep**

Run:

```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
cargo test -p trace-commons-gate-enclave
cargo test -p trace-commons-gate-enclave --features near-ai-scorer
cargo test -p trace-commons-server
cargo clippy -p trace-commons-server --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
cargo clippy -p trace-commons-gate-enclave --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
cargo test -p trace-commons-server --test trace_corpus_storage_contract
```

Expected: all clean/pass. (PG-gated: `cargo test -p trace-commons-server --test trace_corpus_pg_store` locally if PostgreSQL is available.)

- [ ] **Step 6: Commit**

```bash
git add crates/trace-commons-server/src/trace_gate_service.rs docs/operator/large-trace-chunked-scoring.md docs/operator/README.md
git commit -m "add multi-chunk end-to-end gate test and chunked-scoring runbook"
```

---

## Deferred / follow-ups (explicitly out of this plan, per spec non-goals)

- Vector-replay coverage of per-chunk entries: `stream_trace_gate_decisions_for_replay` and the vector-payload object-ref flow (`trace-commons-ingest.rs` ~49564-49616, keyed by the single `vector_entry_id`) continue to replay only the decision row's first entry. Full per-chunk replay-payload persistence is a tracked follow-up — the chunk table is the durable record; replay of secondary chunk entries would require per-chunk payload object refs.
- Partial-tolerant per-chunk retry (v1 is fail-closed).
- Vector-index compaction/GC beyond the per-trace cap + insert dedup.
- Manual pilot validation (the 169 KB trace `bfd6d37d` + 4 previously-failing traces) — deployment task, not CI.

## OPEN QUESTIONS

None blocking. Two flagged decisions made inside the plan (not left open): (1) `serde_json` flips from optional to required in `trace-commons-gate-enclave` — no new crate, but surface it in the PR description per the dependency policy; (2) per-chunk entries use a child table (`trace_gate_chunk_vector_entries`) rather than an array/JSON column, justified in Task 8 against the existing `target_json ->> 'vector_entry_id'` revocation matcher and `vector_entry_id IS NOT NULL` replay filter.
