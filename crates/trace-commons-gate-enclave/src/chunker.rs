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
    /// Hard cap on chunks per trace (default 16). Beyond it, an evenly
    /// strided subset spanning the whole trace is scored (see
    /// [`strided_selection_indices`]) and the rest are dropped and counted
    /// — never silently.
    pub chunk_cap: usize,
}

impl ChunkerConfig {
    fn target_chars(&self) -> usize {
        self.target_tokens
            .saturating_mul(APPROX_CHARS_PER_TOKEN)
            .max(1)
    }
    fn max_chars(&self) -> usize {
        self.max_tokens
            .saturating_mul(APPROX_CHARS_PER_TOKEN)
            .max(1)
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
/// `max_chars` splits into `target_chars` fixed windows. Applies the cap via
/// coverage-preserving strided selection.
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

/// Identifier for the chunk-SELECTION algorithm (which chunks survive the
/// cap), distinct from the chunk-PACKING knobs. Stamped into the gate
/// version hash so decisions made under different selection arithmetic are
/// never comparable under one version stamp. Bump this on any change to
/// [`strided_selection_indices`].
pub const CHUNK_SELECTION_ALGORITHM: &str = "stride_endpoint_inclusive.v1";

/// Deterministically choose exactly `min(total, cap)` positions spread
/// evenly across `0..total`, endpoint-inclusive.
///
/// Replaces prefix truncation. Prefix-keeping made the gate judge a long
/// trace on its opening — the most boilerplate, most cross-session-repeated
/// part (system prompt, env banner, first file reads) — which biases the
/// novelty signal toward "duplicate" precisely for the longest traces.
///
/// Properties (all asserted in tests):
///  - returns exactly `min(total, cap)` indices, never more: chunk count
///    drives both scorer cost and fail-closed failure exposure, so this
///    change is cost-neutral by construction;
///  - strictly increasing, hence unique and chronological;
///  - index 0 is always first and `total - 1` is always last whenever more
///    than one chunk is scored, so the trace's ending — where novel content
///    concentrates — is always scored;
///  - pure integer arithmetic, no RNG / clock / map iteration: identical
///    input always yields an identical selection, which the attestation
///    chain requires;
///  - when `total <= cap` it degenerates to `0..total`, i.e. the uncapped
///    path is unchanged.
pub fn strided_selection_indices(total: usize, cap: usize) -> Vec<usize> {
    let cap = cap.max(1);
    let keep = total.min(cap);
    if keep == 0 {
        return Vec::new();
    }
    if keep == 1 {
        return vec![0];
    }
    // Endpoint-inclusive stride with round-half-up, in u128 so the multiply
    // cannot overflow: idx(j) = round(j * (total - 1) / (keep - 1)).
    // Because total - 1 >= keep - 1, consecutive indices differ by at least
    // floor((total - 1) / (keep - 1)) >= 1, so they are strictly increasing
    // and unique.
    let span = (total - 1) as u128;
    let steps = (keep - 1) as u128;
    (0..keep)
        .map(|j| (((j as u128) * span + steps / 2) / steps) as usize)
        .collect()
}

fn finalize_plan(texts: Vec<String>, cfg: &ChunkerConfig) -> ChunkPlan {
    let cap = cfg.chunk_cap.max(1);
    let total = texts.len();
    // Unchanged meaning: capped iff more chunks existed than the cap allows,
    // and the drop count is how many the cap removed.
    let (chunks_capped, dropped_chunk_count) = if total > cap {
        (true, (total - cap) as u32)
    } else {
        (false, 0)
    };
    // `chunk_index` is the ORIGINAL position in the trace, not the position
    // within the surviving set. Original indices stay unique within a
    // decision (the selection is strictly increasing), which is all the
    // `(tenant_id, decision_id, chunk_index)` primary key needs; nothing
    // downstream requires contiguity or a zero start — per-chunk vector
    // entries are already sparse today, since only chunks clearing
    // `embed_insert_novelty_micros` are inserted.
    let mut texts: Vec<Option<String>> = texts.into_iter().map(Some).collect();
    let chunks = strided_selection_indices(total, cap)
        .into_iter()
        .map(|i| TraceChunk {
            chunk_index: i as u32,
            text: texts[i]
                .take()
                .expect("strided selection indices are unique"),
        })
        .collect();
    ChunkPlan {
        chunks,
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
        let plan = chunk_envelope_plaintext(br#"{"schema_version":"x"}"#, &cfg(2048, 3072, 16));
        assert_eq!(
            plan.chunks.len(),
            1,
            "no-events JSON falls back to raw text"
        );
    }

    #[test]
    fn empty_plaintext_yields_single_empty_chunk() {
        let plan = chunk_envelope_plaintext(b"", &cfg(2048, 3072, 16));
        assert_eq!(plan.chunks.len(), 1);
        assert_eq!(plan.chunks[0].text, "");
        assert!(!plan.chunks_capped);
    }

    /// Build N distinct one-event-per-chunk texts, each 100 chars of a
    /// content marker so the chunk a given event lands in is identifiable.
    fn marked_contents(n: usize) -> Vec<String> {
        (0..n)
            .map(|i| format!("{:*<100}", format!("mark{i}-")))
            .collect()
    }

    fn plan_for_marked(n: usize, cap: usize) -> ChunkPlan {
        let contents = marked_contents(n);
        let refs: Vec<&str> = contents.iter().map(|s| s.as_str()).collect();
        // target 25 tokens = 100 chars -> exactly one event per chunk.
        chunk_envelope_plaintext(&envelope_json(&refs), &cfg(25, 50, cap))
    }

    #[test]
    fn selection_count_is_exactly_min_total_cap() {
        for (total, cap) in [(1, 16), (10, 16), (16, 16), (17, 16), (100, 16), (5, 1)] {
            let plan = plan_for_marked(total, cap);
            assert_eq!(
                plan.chunks.len(),
                total.min(cap),
                "total={total} cap={cap} must select exactly min(total, cap)"
            );
        }
    }

    #[test]
    fn selection_is_deterministic_across_repeated_calls() {
        let a = plan_for_marked(97, 16);
        let b = plan_for_marked(97, 16);
        let c = plan_for_marked(97, 16);
        assert_eq!(a, b);
        assert_eq!(b, c);
    }

    #[test]
    fn selection_spans_the_whole_array_not_just_the_prefix() {
        let total = 100usize;
        let cap = 16usize;
        let plan = plan_for_marked(total, cap);
        let idx: Vec<u32> = plan.chunks.iter().map(|c| c.chunk_index).collect();
        // First and last chunks of the trace are always selected.
        assert_eq!(idx.first().copied(), Some(0));
        assert_eq!(
            idx.last().copied(),
            Some((total - 1) as u32),
            "the final chunk of the trace must be scored"
        );
        // Strictly increasing, no duplicates.
        assert!(idx.windows(2).all(|w| w[0] < w[1]), "indices must ascend");
        // Coverage is real: the selection reaches far past the cap.
        assert!(
            idx.iter().any(|i| *i as usize >= total / 2),
            "selection must reach the back half of the trace"
        );
        // Text matches the origin position, i.e. we kept the right chunk.
        for chunk in &plan.chunks {
            assert!(
                chunk.text.contains(&format!("mark{}-", chunk.chunk_index)),
                "chunk_index must be the ORIGINAL position of the kept text"
            );
        }
    }

    #[test]
    fn selection_is_evenly_strided() {
        // 100 -> 16: ideal stride 99/15 = 6.6. Every gap must be 6 or 7.
        let plan = plan_for_marked(100, 16);
        let idx: Vec<u32> = plan.chunks.iter().map(|c| c.chunk_index).collect();
        for w in idx.windows(2) {
            let gap = w[1] - w[0];
            assert!((6..=7).contains(&gap), "uneven stride gap {gap} in {idx:?}");
        }
    }

    #[test]
    fn uncapped_path_is_unchanged_contiguous_from_zero() {
        // Uncapped traces must be byte-identical to the pre-stride behavior:
        // every chunk kept, indices 0..n contiguous, in order.
        let total = 12usize;
        let plan = plan_for_marked(total, 16);
        assert!(!plan.chunks_capped);
        assert_eq!(plan.dropped_chunk_count, 0);
        assert_eq!(plan.chunks.len(), total);
        for (i, chunk) in plan.chunks.iter().enumerate() {
            assert_eq!(chunk.chunk_index, i as u32);
            assert!(chunk.text.contains(&format!("mark{i}-")));
        }
    }

    #[test]
    fn capped_flags_keep_their_meaning() {
        let plan = plan_for_marked(100, 16);
        assert!(plan.chunks_capped);
        assert_eq!(plan.dropped_chunk_count, (100 - 16) as u32);
        let exact = plan_for_marked(16, 16);
        assert!(!exact.chunks_capped);
        assert_eq!(exact.dropped_chunk_count, 0);
    }

    #[test]
    fn strided_selection_indices_are_unique_and_bounded() {
        for total in 1..200usize {
            for cap in [1usize, 2, 3, 7, 16, 64] {
                let sel = strided_selection_indices(total, cap);
                assert_eq!(sel.len(), total.min(cap), "total={total} cap={cap}");
                assert!(sel.iter().all(|i| *i < total), "total={total} cap={cap}");
                assert!(
                    sel.windows(2).all(|w| w[0] < w[1]),
                    "total={total} cap={cap} indices must be strictly increasing: {sel:?}"
                );
                assert_eq!(sel[0], 0, "first chunk is always pinned");
                assert_eq!(
                    *sel.last().unwrap(),
                    if total.min(cap) == 1 { 0 } else { total - 1 },
                    "last chunk is pinned whenever more than one chunk is scored"
                );
            }
        }
    }

    #[test]
    fn chunking_is_deterministic() {
        let plaintext = envelope_json(&["alpha", "beta", "gamma"]);
        let a = chunk_envelope_plaintext(&plaintext, &cfg(8, 16, 16));
        let b = chunk_envelope_plaintext(&plaintext, &cfg(8, 16, 16));
        assert_eq!(a, b);
    }
}
