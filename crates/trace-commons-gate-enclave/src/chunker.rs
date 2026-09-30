// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

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
    /// Who authored each run of `text`, in chars, tiling it exactly. Feeds
    /// per-author perplexity (shadow mode); never changes what is scored.
    pub spans: Vec<AuthorSpan>,
}

/// Who authored a run of scored chars. Drives per-author perplexity, a
/// shadow signal; it never changes what text is scored.
///
/// `reasoning` is deliberately `Other`: its presence follows the
/// contributor's `--no-reasoning` choice, and a score must not move with a
/// consent flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorKind {
    AgentProse,
    ToolResult,
    Other,
}

impl AuthorKind {
    pub const COUNT: usize = 3;

    pub fn index(self) -> usize {
        match self {
            AuthorKind::AgentProse => 0,
            AuthorKind::ToolResult => 1,
            AuthorKind::Other => 2,
        }
    }

    fn of_event_type(event_type: &str) -> Self {
        match event_type {
            "assistant_message" => AuthorKind::AgentProse,
            "tool_result" => AuthorKind::ToolResult,
            _ => AuthorKind::Other,
        }
    }
}

/// A run of chars of one kind, in chars relative to the owning text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorSpan {
    pub start: u32,
    pub len: u32,
    pub kind: AuthorKind,
}

/// One canonically rendered event with its author spans. `text` is exactly
/// what [`render_event_text`] produces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedEvent {
    pub text: String,
    pub spans: Vec<AuthorSpan>,
}

/// Append a span, merging into the previous one when the kind matches.
/// Zero-length spans are dropped so spans always tile with no empties.
fn push_span(spans: &mut Vec<AuthorSpan>, len: usize, kind: AuthorKind) {
    let len = u32::try_from(len).unwrap_or(u32::MAX);
    if len == 0 {
        return;
    }
    if let Some(last) = spans.last_mut() {
        if last.kind == kind {
            last.len = last.len.saturating_add(len);
            return;
        }
    }
    let start = spans
        .last()
        .map(|s| s.start.saturating_add(s.len))
        .unwrap_or(0);
    spans.push(AuthorSpan { start, len, kind });
}

/// The part of `spans` covering chars `[from, from + len)`, rebased to 0.
fn slice_spans(spans: &[AuthorSpan], from: usize, len: usize) -> Vec<AuthorSpan> {
    let (from, to) = (from as u64, (from + len) as u64);
    let mut out = Vec::new();
    for s in spans {
        let (s_from, s_to) = (s.start as u64, s.start as u64 + s.len as u64);
        let (lo, hi) = (s_from.max(from), s_to.min(to));
        if lo < hi {
            push_span(&mut out, (hi - lo) as usize, s.kind);
        }
    }
    out
}

fn render_event(event_type: &str, tool_name: Option<&str>, content: &str) -> RenderedEvent {
    let text = render_event_text(event_type, tool_name, content);
    let content_chars = content.chars().count();
    // `render_event_text` is `<prefix><content>\n`, so the prefix length is
    // what is left. Derived rather than re-formatted so the two can never
    // disagree about the prefix.
    let prefix_chars = text.chars().count() - content_chars - 1;
    let mut spans = Vec::new();
    push_span(&mut spans, prefix_chars, AuthorKind::Other);
    push_span(
        &mut spans,
        content_chars,
        AuthorKind::of_event_type(event_type),
    );
    push_span(&mut spans, 1, AuthorKind::Other);
    RenderedEvent { text, spans }
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
    Some(
        parse_envelope_events(plaintext)?
            .into_iter()
            .map(|e| e.text)
            .collect(),
    )
}

/// As [`parse_envelope_rendered_events`], keeping each event's author spans.
pub fn parse_envelope_events(plaintext: &[u8]) -> Option<Vec<RenderedEvent>> {
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
                render_event(event_type, tool_name, content)
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
    let typed: Vec<RenderedEvent> = events
        .iter()
        .map(|text| {
            let mut spans = Vec::new();
            push_span(&mut spans, text.chars().count(), AuthorKind::Other);
            RenderedEvent {
                text: text.clone(),
                spans,
            }
        })
        .collect();
    chunk_events(&typed, cfg)
}

/// [`chunk_rendered_events`] over typed events: identical packing and
/// identical text, with each event's author spans carried through every
/// path (greedy packing, oversized-event windows, the cap).
pub fn chunk_events(events: &[RenderedEvent], cfg: &ChunkerConfig) -> ChunkPlan {
    let target = cfg.target_chars();
    let max = cfg.max_chars();
    let mut packed: Vec<(String, Vec<AuthorSpan>)> = Vec::new();
    let mut current = String::new();
    let mut current_spans: Vec<AuthorSpan> = Vec::new();
    let mut current_chars = 0usize;
    for event in events {
        let event_chars = event.text.chars().count();
        if event_chars > max {
            // Oversized event: flush the open chunk, then fixed windows.
            if !current.is_empty() {
                packed.push((
                    std::mem::take(&mut current),
                    std::mem::take(&mut current_spans),
                ));
                current_chars = 0;
            }
            let mut from = 0usize;
            for window in split_fixed_char_windows(&event.text, target) {
                let len = window.chars().count();
                packed.push((window, slice_spans(&event.spans, from, len)));
                from += len;
            }
            continue;
        }
        if !current.is_empty() && current_chars + event_chars > target {
            packed.push((
                std::mem::take(&mut current),
                std::mem::take(&mut current_spans),
            ));
            current_chars = 0;
        }
        current.push_str(&event.text);
        for s in &event.spans {
            push_span(&mut current_spans, s.len as usize, s.kind);
        }
        current_chars += event_chars;
    }
    if !current.is_empty() {
        packed.push((current, current_spans));
    }
    if packed.is_empty() {
        packed.push((String::new(), Vec::new()));
    }
    finalize_plan(packed, cfg)
}

/// Identifier for the chunk-SELECTION algorithm (which chunks survive the
/// cap), distinct from the chunk-PACKING knobs. Stamped into the gate
/// version hash so decisions made under different selection arithmetic are
/// never comparable under one version stamp. Bump this on any change to
/// [`strided_selection_indices`].
pub const CHUNK_SELECTION_ALGORITHM: &str = "stride_endpoint_inclusive.v1";

/// Identifier for the canonical EVENT RENDERER — the text every downstream
/// similarity signal is computed over (chunk text for perplexity/novelty and
/// the vector index, and the cross-trace `dedup_simhash`). Distinct from
/// [`CHUNK_SELECTION_ALGORITHM`], which names which chunks survive the cap:
/// two decisions can select identical chunks and still be incomparable
/// because the text inside them was rendered differently. Stamped into the
/// gate version hash and, composed with the simhash algorithm, into
/// `trace_gate_decisions.dedup_signal_version`. Bump this on any change to
/// [`render_event_text`] or to which envelope fields
/// [`parse_envelope_rendered_events`] reads.
pub const CANONICAL_RENDER_VERSION: &str = "events.v1";

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

fn finalize_plan(packed: Vec<(String, Vec<AuthorSpan>)>, cfg: &ChunkerConfig) -> ChunkPlan {
    let cap = cfg.chunk_cap.max(1);
    let total = packed.len();
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
    let mut packed: Vec<Option<(String, Vec<AuthorSpan>)>> = packed.into_iter().map(Some).collect();
    let chunks = strided_selection_indices(total, cap)
        .into_iter()
        .map(|i| {
            let (text, spans) = packed[i]
                .take()
                .expect("strided selection indices are unique");
            TraceChunk {
                chunk_index: i as u32,
                text,
                spans,
            }
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
    if let Some(events) = parse_envelope_events(plaintext) {
        return chunk_events(&events, cfg);
    }
    let text = String::from_utf8_lossy(plaintext);
    let packed = split_fixed_char_windows(&text, cfg.target_chars())
        .into_iter()
        .map(|w| {
            let mut spans = Vec::new();
            push_span(&mut spans, w.chars().count(), AuthorKind::Other);
            (w, spans)
        })
        .collect();
    finalize_plan(packed, cfg)
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

    /// Same minimal single-event envelope as `envelope_json`, but with
    /// caller-supplied sibling keys spliced onto the event and onto the
    /// envelope itself. `event_type` and `redacted_content` are fixed so
    /// two envelopes built from this helper differ ONLY in the extra keys.
    fn envelope_json_with_extra_fields(
        event_extra: &[(&str, &str)],
        envelope_extra: &[(&str, &str)],
    ) -> Vec<u8> {
        let mut event = serde_json::json!({
            "event_type": "user_message",
            "redacted_content": "hello",
        });
        for (k, v) in event_extra {
            event[*k] = serde_json::json!(v);
        }
        let mut envelope = serde_json::json!({ "events": [event] });
        for (k, v) in envelope_extra {
            envelope[*k] = serde_json::json!(v);
        }
        serde_json::to_vec(&envelope).unwrap()
    }

    /// Render every event in the envelope and concatenate — the same text
    /// both the perplexity scorer and the novelty/dedup signal consume.
    fn render_all_events(plaintext: &[u8]) -> String {
        // `expect`, not `unwrap_or_default`: a parse failure here would make
        // every caller compare "" with "", which passes while proving
        // nothing. The one guard standing between attestation material and
        // the scored text must not be able to degrade into a tautology.
        parse_envelope_rendered_events(plaintext)
            .expect("the fixture envelope must parse")
            .concat()
    }

    #[test]
    fn only_redacted_content_reaches_the_scored_text() {
        // The chunker takes every event with no type filter, so the ONLY
        // thing keeping non-content fields out of perplexity and dedup is
        // that it reads `redacted_content` and nothing else. Attestation
        // material will live in a sibling field; this asserts that adding
        // one changes no scored byte.
        let plain = envelope_json_with_extra_fields(&[], &[]);
        let with_extra = envelope_json_with_extra_fields(
            &[("attestation_receipt", "0xdeadbeef...")],
            &[("intel_quote", "aabbcc...")],
        );
        let rendered = render_all_events(&plain);
        // Belt to the `expect` above's braces: an envelope that parses but
        // renders nothing would also make the comparison vacuous.
        assert!(
            !rendered.is_empty(),
            "the fixture must render some scored text, or this asserts nothing"
        );
        assert_eq!(
            rendered,
            render_all_events(&with_extra),
            "a non-content field changed the scored text; attestation data would be scored"
        );
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

    fn assert_spans_tile(chunk: &TraceChunk) {
        let mut cursor = 0u32;
        for s in &chunk.spans {
            assert_eq!(s.start, cursor, "spans must be contiguous");
            assert!(s.len > 0, "no empty spans");
            cursor += s.len;
        }
        assert_eq!(
            cursor as usize,
            chunk.text.chars().count(),
            "spans must tile the text"
        );
        for w in chunk.spans.windows(2) {
            assert_ne!(w[0].kind, w[1].kind, "adjacent same-kind spans must merge");
        }
    }

    fn kind_chars(chunk: &TraceChunk, kind: AuthorKind) -> String {
        let chars: Vec<char> = chunk.text.chars().collect();
        chunk
            .spans
            .iter()
            .filter(|s| s.kind == kind)
            .flat_map(|s| chars[s.start as usize..(s.start + s.len) as usize].iter())
            .collect()
    }

    fn typed_envelope(events: &[(&str, Option<&str>, &str)]) -> Vec<u8> {
        let events: Vec<serde_json::Value> = events
            .iter()
            .map(|(ty, tool, content)| {
                let mut e = serde_json::json!({"event_type": ty, "redacted_content": content});
                if let Some(t) = tool {
                    e["tool_name"] = serde_json::json!(t);
                }
                e
            })
            .collect();
        serde_json::to_vec(&serde_json::json!({ "events": events })).unwrap()
    }

    #[test]
    fn spans_mark_only_the_content_of_prose_and_tool_results() {
        let env = typed_envelope(&[
            ("user_message", None, "fix it"),
            ("assistant_message", None, "on it"),
            ("tool_call", Some("Bash"), ""),
            ("tool_result", Some("Bash"), "ok\n"),
            ("reasoning", None, "hmm"),
        ]);
        let plan = chunk_envelope_plaintext(&env, &cfg(2048, 3072, 16));
        assert_eq!(plan.chunks.len(), 1);
        let c = &plan.chunks[0];
        assert_spans_tile(c);
        assert_eq!(kind_chars(c, AuthorKind::AgentProse), "on it");
        assert_eq!(kind_chars(c, AuthorKind::ToolResult), "ok\n");
        // Prefixes, newlines, user text, reasoning and the empty tool_call
        // scaffold are all Other.
        assert!(kind_chars(c, AuthorKind::Other).contains("reasoning: hmm"));
        assert!(kind_chars(c, AuthorKind::Other).contains("tool_call (Bash): "));
    }

    #[test]
    fn adding_spans_does_not_change_the_scored_text() {
        let env = typed_envelope(&[
            ("user_message", None, "héllo wörld"),
            ("assistant_message", None, "naïve café"),
            ("tool_result", Some("Read"), "x".repeat(40).as_str()),
        ]);
        let config = cfg(4, 6, 16);
        let legacy = chunk_rendered_events(&parse_envelope_rendered_events(&env).unwrap(), &config);
        let typed = chunk_envelope_plaintext(&env, &config);
        let texts = |p: &ChunkPlan| p.chunks.iter().map(|c| c.text.clone()).collect::<Vec<_>>();
        assert_eq!(texts(&legacy), texts(&typed));
        assert_eq!(legacy.chunks_capped, typed.chunks_capped);
    }

    #[test]
    fn an_oversized_event_keeps_exact_spans_across_its_windows() {
        let env = typed_envelope(&[("tool_result", Some("Read"), "y".repeat(100).as_str())]);
        let plan = chunk_envelope_plaintext(&env, &cfg(5, 6, 64));
        assert!(plan.chunks.len() > 1);
        for c in &plan.chunks {
            assert_spans_tile(c);
        }
        let all: String = plan
            .chunks
            .iter()
            .map(|c| kind_chars(c, AuthorKind::ToolResult))
            .collect();
        assert_eq!(all, "y".repeat(100));
        // The prefix lives in the first window only.
        assert!(kind_chars(&plan.chunks[0], AuthorKind::Other).starts_with("tool_result (Read): "));
        assert_eq!(kind_chars(&plan.chunks[1], AuthorKind::Other), "");
    }

    #[test]
    fn spans_survive_the_strided_cap() {
        let contents: Vec<String> = (0..40)
            .map(|i| format!("assistant text number {i} ").repeat(3))
            .collect();
        let events: Vec<(&str, Option<&str>, &str)> = contents
            .iter()
            .map(|c| ("assistant_message", None, c.as_str()))
            .collect();
        let plan = chunk_envelope_plaintext(&typed_envelope(&events), &cfg(8, 64, 4));
        assert!(plan.chunks_capped);
        assert_eq!(plan.chunks.len(), 4);
        for c in &plan.chunks {
            assert_spans_tile(c);
        }
    }

    #[test]
    fn unstructured_plaintext_is_all_other() {
        let plan = chunk_envelope_plaintext(b"not json at all", &cfg(2048, 3072, 16));
        let c = &plan.chunks[0];
        assert_spans_tile(c);
        assert!(c.spans.iter().all(|s| s.kind == AuthorKind::Other));
    }

    #[test]
    fn legacy_string_events_are_all_other() {
        let plan = chunk_rendered_events(
            &["assistant_message: hi\n".to_string()],
            &cfg(2048, 3072, 16),
        );
        assert_spans_tile(&plan.chunks[0]);
        assert_eq!(kind_chars(&plan.chunks[0], AuthorKind::AgentProse), "");
    }
}
