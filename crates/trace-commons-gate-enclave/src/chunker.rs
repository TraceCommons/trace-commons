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
