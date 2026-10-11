//! Span types and decoding shared by every classify-shaped privacy backend.
//!
//! Offsets returned by a classifier are CODEPOINT offsets, not byte offsets.
//! The conversion to byte indices happens here, once, so that no backend can
//! get it wrong independently. A byte-for-codepoint slip does not fail loudly:
//! it redacts the wrong characters, leaves the PII in place, and reports
//! success.

use serde::Deserialize;

use crate::trace_contribution::{
    RedactionReport, SafePrivacyFilterRedaction, SafePrivacyFilterSummary, TraceContributionError,
    safe_privacy_filter_label,
};

/// Whether a non-2xx from a classifier says "not now" rather than "not this".
///
/// A 5xx is the classifier's problem, and so are 429 (rate limited) and 408
/// (request timeout): all three are about its load, not the text we sent, so
/// they are retried and, if they outlive the retries, typed transient so the
/// trace is not charged for them. Every other 4xx is about our request, and
/// retrying it would only hide the bug.
pub(crate) fn is_transient_classifier_status(status: reqwest::StatusCode) -> bool {
    status.is_server_error()
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status == reqwest::StatusCode::REQUEST_TIMEOUT
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub(crate) struct ClassifySpan {
    pub(crate) category: String,
    pub(crate) start: usize,
    pub(crate) end: usize,
    #[serde(default)]
    pub(crate) score: f64,
    /// The text the classifier says the span covers, when it reports one.
    /// NEAR AI always does; that adapter checks it against its own slice of
    /// the request (see `verify_span_texts` there) so an offset slip fails
    /// closed instead of redacting the wrong characters. Never logged.
    #[serde(default)]
    pub(crate) text: Option<String>,
}

/// Bytes `text` occupies inside a JSON string literal, quotes excluded --
/// exactly what `serde_json` writes: `"` and `\` and the short-escape
/// controls (`\n`, `\r`, `\t`, backspace, form feed) take 2 bytes, every
/// other control below 0x20 takes 6 (`\u00XX`), and everything else,
/// including non-ASCII UTF-8, is copied as is. Additive over concatenation,
/// so a pack's escaped size is the sum of its parts.
#[cfg(feature = "near-ai-privacy-filter")]
pub(crate) fn json_escaped_len(text: &str) -> usize {
    text.bytes()
        .map(|byte| match byte {
            b'"' | b'\\' | b'\n' | b'\r' | b'\t' | 0x08 | 0x0c => 2,
            0x00..=0x1f => 6,
            _ => 1,
        })
        .sum()
}
pub(crate) fn apply_spans(
    backend: &'static str,
    text: &str,
    spans: &[ClassifySpan],
) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
    let mut report = RedactionReport::default();
    let mut by_label = std::collections::BTreeMap::new();
    let span_count = spans.len() as u32;

    // NEAR AI reports span offsets as Unicode codepoint indices, not byte
    // offsets. Build a codepoint -> byte-offset table once so we can both
    // validate the offsets and translate them before any byte slicing.
    // `boundaries[i]` is the byte offset of codepoint `i`; the final entry
    // is `text.len()`, so a valid end index is `<= boundaries.len() - 1`.
    let mut boundaries: Vec<usize> = text.char_indices().map(|(b, _)| b).collect();
    boundaries.push(text.len());

    // Validate offsets and labels; populate summary book-keeping per raw
    // span (matches sidecar accounting). Offsets are converted from
    // codepoint indices to byte offsets here.
    let mut byte_spans: Vec<ClassifySpan> = Vec::with_capacity(spans.len());
    for span in spans {
        if span.start > span.end || span.end >= boundaries.len() {
            return Err(TraceContributionError::RedactionFailed {
                reason: format!("{backend} privacy classifier returned out-of-range span"),
            });
        }
        byte_spans.push(ClassifySpan {
            category: span.category.clone(),
            start: boundaries[span.start],
            end: boundaries[span.end],
            score: span.score,
            text: None,
        });
        let label = safe_privacy_filter_label(Some(&span.category), &mut report);
        *by_label.entry(label.clone()).or_insert(0u32) += 1;
        report.increment(format!("privacy_filter:{label}"));
        if label.eq_ignore_ascii_case("secret") {
            report.blocked_secret_detected = true;
        }
        if !report.pii_labels_present.contains(&label) {
            report.pii_labels_present.push(label);
        }
    }

    // Build redacted text. Collapse overlapping spans: sort by start,
    // pick widest end; on overlap pick the highest-score category. Offsets
    // below are byte offsets (already translated from codepoint indices).
    let mut sorted: Vec<ClassifySpan> = byte_spans;
    sorted.sort_by_key(|s| (s.start, std::cmp::Reverse(s.end)));

    let mut collapsed: Vec<ClassifySpan> = Vec::new();
    for span in sorted {
        match collapsed.last_mut() {
            Some(prev) if span.start < prev.end => {
                if span.end > prev.end {
                    prev.end = span.end;
                }
                if span.score > prev.score {
                    prev.category = span.category;
                    prev.score = span.score;
                }
            }
            _ => collapsed.push(span),
        }
    }

    let mut redacted_text = String::with_capacity(text.len());
    let mut cursor = 0usize;
    let mut dummy_report = RedactionReport::default();
    for span in &collapsed {
        redacted_text.push_str(&text[cursor..span.start]);
        let label = safe_privacy_filter_label(Some(&span.category), &mut dummy_report);
        redacted_text.push_str(&format!("[REDACTED:{label}]"));
        cursor = span.end;
    }
    redacted_text.push_str(&text[cursor..]);

    Ok(Some(SafePrivacyFilterRedaction {
        private_edits: Some(crate::private_edit_map::PrivateRedactionEdits(
            collapsed
                .iter()
                .map(|span| crate::token_distribution::RedactionEdit {
                    original: crate::token_distribution::ByteSpan {
                        start: span.start as u64,
                        end: span.end as u64,
                    },
                    replacement: format!(
                        "[REDACTED:{}]",
                        safe_privacy_filter_label(
                            Some(&span.category),
                            &mut RedactionReport::default()
                        )
                    )
                    .into_bytes(),
                })
                .collect(),
        )),
        redacted_text,
        summary: SafePrivacyFilterSummary {
            schema_version: 1,
            output_mode: "redacted_text_only".to_string(),
            span_count,
            by_label,
            decoded_mismatch: false,
            classify_policy: None,
            events_examined: 0,
            events_skipped_by_policy: 0,
        },
        report,
    }))
}

/// The tokenizer the hosted classifier actually uses.
///
/// Identified by measurement rather than assumption: `o200k_base` reproduced
/// the endpoint's own `usage.input_tokens` exactly on 17 of 17 samples --
/// prose, source code, identifier-dense text, hex digests, long words and
/// repeated characters, from 5 bytes to 8 KB. `cl100k_base` matched only 6 of
/// 9 on the same short set, so the choice is not arbitrary and should not be
/// changed without re-running that comparison.
pub(crate) fn classifier_bpe() -> Option<&'static tiktoken_rs::CoreBPE> {
    static BPE: std::sync::OnceLock<Option<tiktoken_rs::CoreBPE>> = std::sync::OnceLock::new();
    BPE.get_or_init(|| tiktoken_rs::o200k_base().ok()).as_ref()
}
/// Count the tokens the classifier will charge for `text`.
pub(crate) fn classifier_token_count(text: &str) -> Option<usize> {
    classifier_bpe().map(|bpe| bpe.encode_ordinary(text).len())
}
/// Split `text` into contiguous byte ranges, each within `max_tokens` of the
/// classifier's budget, covering the whole input on char boundaries.
///
/// Segments are cut at newlines where possible -- PII rarely spans lines, and
/// a window that ends mid-entity risks splitting one across two requests. A
/// single line that alone exceeds the budget (a long log line, a base64 blob)
/// is bisected until its pieces fit.
///
/// Falls back to a conservative byte split if the tokenizer is unavailable:
/// under-filling requests costs throughput, over-filling costs 502s, so the
/// safe direction is down.
pub(crate) fn chunk_token_ranges(text: &str, max_tokens: usize) -> Vec<std::ops::Range<usize>> {
    if classifier_bpe().is_none() {
        // No tokenizer: fall back to the dense-content byte equivalent, which
        // is the smallest realistic window for this budget.
        return chunk_byte_ranges(text, max_tokens.saturating_mul(3).max(1));
    }
    let max_tokens = max_tokens.max(1);

    // Line-ish segments, each carrying its own token cost.
    let mut segments: Vec<std::ops::Range<usize>> = Vec::new();
    let mut seg_start = 0usize;
    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            segments.push(seg_start..index + 1);
            seg_start = index + 1;
        }
    }
    if seg_start < text.len() {
        segments.push(seg_start..text.len());
    }

    // Any segment too big on its own is bisected until each piece fits.
    let mut sized: Vec<(std::ops::Range<usize>, usize)> = Vec::new();
    let mut pending: Vec<std::ops::Range<usize>> = segments;
    pending.reverse();
    while let Some(range) = pending.pop() {
        let tokens = classifier_token_count(&text[range.clone()]).unwrap_or(usize::MAX);
        if tokens <= max_tokens || range.len() <= 1 {
            sized.push((range, tokens));
            continue;
        }
        // Bisect on a char boundary and re-measure both halves.
        let mut mid = range.start + range.len() / 2;
        while mid > range.start && !text.is_char_boundary(mid) {
            mid -= 1;
        }
        if mid == range.start {
            sized.push((range, tokens));
            continue;
        }
        pending.push(mid..range.end);
        pending.push(range.start..mid);
    }

    // Greedily pack segments up to the budget. Per-segment counts can differ
    // slightly from the count of the joined text, because BPE merges across a
    // boundary; the budget's margin under the measured limit absorbs that.
    let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
    let mut current: Option<std::ops::Range<usize>> = None;
    let mut running = 0usize;
    for (range, tokens) in sized {
        match current {
            Some(ref mut open) if running + tokens <= max_tokens => {
                open.end = range.end;
                running += tokens;
            }
            Some(open) => {
                ranges.push(open);
                running = tokens;
                current = Some(range);
            }
            None => {
                running = tokens;
                current = Some(range);
            }
        }
    }
    if let Some(open) = current {
        ranges.push(open);
    }
    if ranges.is_empty() {
        ranges.push(0..text.len());
    }
    ranges
}
/// Split `text` into contiguous byte ranges each no larger than `max_bytes`,
/// always on char boundaries and covering the whole input. Windows prefer to
/// end at a newline within the limit (PII rarely spans lines); a run with no
/// newline under the limit is hard-split at the nearest lower char boundary.
pub(crate) fn chunk_byte_ranges(text: &str, max_bytes: usize) -> Vec<std::ops::Range<usize>> {
    let max_bytes = max_bytes.max(1);
    let mut ranges = Vec::new();
    let mut start = 0;
    while start < text.len() {
        if text.len() - start <= max_bytes {
            ranges.push(start..text.len());
            break;
        }
        // Provisional hard cap, walked back to a char boundary.
        let mut end = start + max_bytes;
        while end > start && !text.is_char_boundary(end) {
            end -= 1;
        }
        // Prefer to break just after the last newline inside the window.
        if let Some(nl) = text[start..end].rfind('\n') {
            end = start + nl + 1;
        }
        // Guard against no progress (e.g. a single multibyte char wider
        // than the char-boundary walk left us): force at least one char.
        if end <= start {
            end = start + max_bytes;
            while end < text.len() && !text.is_char_boundary(end) {
                end += 1;
            }
        }
        ranges.push(start..end);
        start = end;
    }
    if ranges.is_empty() {
        ranges.push(0..text.len());
    }
    ranges
}
/// Merge per-window spans into a single redaction over `text`. Each window
/// carries its starting codepoint index; its spans are reported relative to
/// that window, so shift them into full-text codepoint coordinates before
/// the shared `apply_spans` validation/redaction pass.
pub(crate) fn apply_windowed_spans(
    backend: &'static str,
    text: &str,
    windows: &[(usize, Vec<ClassifySpan>)],
) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
    let mut all_spans = Vec::new();
    for (codepoint_start, spans) in windows {
        for span in spans {
            all_spans.push(ClassifySpan {
                category: span.category.clone(),
                start: codepoint_start + span.start,
                end: codepoint_start + span.end,
                score: span.score,
                text: None,
            });
        }
    }
    apply_spans(backend, text, &all_spans)
}

/// What separates two texts packed into one classify request.
///
/// A blank line: the classifier is trained on prose, where a paragraph break
/// ends an entity, so a name ending one text and a name starting the next are
/// not read as one. Spans the classifier nevertheless reports across it are
/// split at the text boundaries (see [`redact_items_batched`]); a span that
/// lies wholly inside the separator touches no text and is dropped.
#[cfg(feature = "near-ai-privacy-filter")]
pub(crate) const BATCH_ITEM_SEPARATOR: &str = "\n\n";

/// One classify request's worth of transport for [`redact_items_batched`].
#[cfg(feature = "near-ai-privacy-filter")]
#[async_trait::async_trait]
pub(crate) trait WindowClassifier: Sync {
    /// Classify one window of a text too large for a single request. This is
    /// the path every window took before packing, so a backend's per-window
    /// memo applies here unchanged.
    async fn classify_window(
        &self,
        window: &str,
    ) -> Result<Vec<ClassifySpan>, TraceContributionError>;

    /// Classify a request built from several whole texts joined by
    /// [`BATCH_ITEM_SEPARATOR`]. Defaults to [`Self::classify_window`]; a
    /// backend whose memo is keyed per text overrides it to skip a lookup
    /// that can only miss.
    async fn classify_pack(&self, pack: &str) -> Result<Vec<ClassifySpan>, TraceContributionError> {
        self.classify_window(pack).await
    }

    /// Spans previously recorded for exactly this text, in its own codepoint
    /// coordinates. A hit means the text is not sent again.
    fn cached_item_spans(&self, _item: &str) -> Option<Vec<ClassifySpan>> {
        None
    }

    /// Record the spans a packed request produced for one of its texts.
    fn remember_item_spans(&self, _item: &str, _spans: &[ClassifySpan]) {}
}

/// Token cost of `text` against the classifier's budget, with the same
/// fallback [`chunk_token_ranges`] uses when no tokenizer is available.
#[cfg(feature = "near-ai-privacy-filter")]
fn batch_token_cost(text: &str) -> usize {
    classifier_token_count(text).unwrap_or_else(|| text.len().div_ceil(3))
}

/// Windows for a text too large for one request: [`chunk_token_ranges`]'s
/// token windows -- identical to `redact_text`'s when every window's body
/// fits -- with any window whose JSON-escaped size exceeds
/// `max_input_json_bytes` cut further, after its last newline under the cap
/// where there is one, so no request goes out over the body limit.
#[cfg(feature = "near-ai-privacy-filter")]
pub(crate) fn window_ranges(
    text: &str,
    max_tokens: usize,
    max_input_json_bytes: usize,
) -> Vec<std::ops::Range<usize>> {
    let mut ranges = Vec::new();
    for range in chunk_token_ranges(text, max_tokens) {
        if json_escaped_len(&text[range.clone()]) <= max_input_json_bytes {
            ranges.push(range);
            continue;
        }
        let mut start = range.start;
        while start < range.end {
            let mut used = 0usize;
            let mut end = start;
            let mut last_newline_end: Option<usize> = None;
            for (offset, character) in text[start..range.end].char_indices() {
                let mut buffer = [0u8; 4];
                let cost = json_escaped_len(character.encode_utf8(&mut buffer));
                if used + cost > max_input_json_bytes && end > start {
                    break;
                }
                used += cost;
                end = start + offset + character.len_utf8();
                if character == '\n' {
                    last_newline_end = Some(end);
                }
            }
            if end < range.end {
                if let Some(newline_end) = last_newline_end {
                    end = newline_end;
                }
            }
            ranges.push(start..end);
            start = end;
        }
    }
    ranges
}

#[cfg(feature = "near-ai-privacy-filter")]
enum BatchUnit {
    /// Whole texts (indices into the unique-text table) joined by the
    /// separator, with each member's codepoint range inside `text`.
    Pack {
        text: String,
        members: Vec<(usize, std::ops::Range<usize>)>,
    },
    /// One window of a single text over the budget, chunked exactly as
    /// `redact_text` chunks it.
    Window {
        unique: usize,
        range: std::ops::Range<usize>,
        codepoint_start: usize,
    },
}

/// Clip `spans`, in a packed request's codepoint coordinates, to the texts
/// `members` places in it, returning each member's spans in its own
/// coordinates.
///
/// Spans are validated against the whole request first (the same rule as
/// `apply_spans`), so a malformed response fails the request rather than
/// being silently attributed. A positive-width span keeps, for every text it
/// overlaps, exactly the overlapping part -- a span straddling a boundary
/// redacts both sides -- and is dropped if it lies wholly in a separator. A
/// zero-width span stays where it lies, inside (or at either edge of) one
/// text.
#[cfg(feature = "near-ai-privacy-filter")]
fn split_pack_spans(
    backend: &'static str,
    members: &[(usize, std::ops::Range<usize>)],
    spans: &[ClassifySpan],
) -> Result<Vec<Vec<ClassifySpan>>, TraceContributionError> {
    let total = members.last().map(|(_, range)| range.end).unwrap_or(0);
    let mut per_member: Vec<Vec<ClassifySpan>> = vec![Vec::new(); members.len()];
    for span in spans {
        if span.start > span.end || span.end > total {
            return Err(TraceContributionError::RedactionFailed {
                reason: format!("{backend} privacy classifier returned out-of-range span"),
            });
        }
        // Members are ascending and disjoint: skip those ending before the
        // span starts.
        let first = members.partition_point(|(_, range)| range.end < span.start);
        for (slot, (_, range)) in members.iter().enumerate().skip(first) {
            if range.start > span.end {
                break;
            }
            let start = span.start.max(range.start);
            let end = span.end.min(range.end);
            let attributable = if span.start == span.end {
                range.start <= span.start && span.start <= range.end
            } else {
                start < end
            };
            if !attributable {
                continue;
            }
            per_member[slot].push(ClassifySpan {
                category: span.category.clone(),
                start: start - range.start,
                end: end - range.start,
                score: span.score,
                text: None,
            });
            if span.start == span.end {
                break;
            }
        }
    }
    Ok(per_member)
}

/// Redact `items` with as few classify requests as the budget allows.
///
/// Every non-blank text is classified -- none is skipped -- and each result is
/// built by [`apply_spans`] / [`apply_windowed_spans`] over that text alone, so
/// its redacted text, report and summary have exactly the shape a single-text
/// `redact_text` call produces:
///
/// - A blank text returns `None` without a request, as `redact_text` does.
/// - Identical texts are classified once and share the result.
/// - A text whose token cost exceeds `max_tokens`, or whose JSON-escaped size
///   exceeds `max_input_json_bytes`, is windowed on its own
///   ([`window_ranges`]: [`chunk_token_ranges`] as before, cut further where
///   a window's body would be too large); it never shares a request.
/// - Smaller texts are packed whole, in order, separated by
///   [`BATCH_ITEM_SEPARATOR`], into requests of at most `max_tokens` tokens
///   and `max_input_json_bytes` escaped input bytes.
///
/// Spans come back in the packed request's CODEPOINT coordinates and are
/// mapped to texts by offset ([`split_pack_spans`]). The redacted request is
/// never split back apart on the separator -- a redaction may consume it.
///
/// Requests run at most `max_concurrent_requests` at a time, in order; the
/// first failing request's error is returned and no results are.
#[cfg(feature = "near-ai-privacy-filter")]
pub(crate) async fn redact_items_batched(
    backend: &'static str,
    classifier: &dyn WindowClassifier,
    items: &[&str],
    pack_max_tokens: usize,
    window_max_tokens: usize,
    max_input_json_bytes: usize,
    max_concurrent_requests: usize,
) -> Result<Vec<Option<SafePrivacyFilterRedaction>>, TraceContributionError> {
    use futures::stream::{StreamExt, TryStreamExt};

    let window_max_tokens = window_max_tokens.max(1);
    let max_tokens = pack_max_tokens.clamp(1, window_max_tokens);
    let max_input_json_bytes = max_input_json_bytes.max(1);

    // Unique non-blank texts, in first-appearance order.
    let mut unique_index: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    let mut uniques: Vec<&str> = Vec::new();
    let mut item_unique: Vec<Option<usize>> = Vec::with_capacity(items.len());
    for item in items {
        if item.trim().is_empty() {
            item_unique.push(None);
            continue;
        }
        let next = uniques.len();
        let index = *unique_index.entry(item).or_insert_with(|| {
            uniques.push(item);
            next
        });
        item_unique.push(Some(index));
    }

    // Per unique text: spans for a packed (or cached) text, or windows for an
    // oversized one.
    let mut packed_spans: Vec<Option<Vec<ClassifySpan>>> = vec![None; uniques.len()];
    let mut oversized: Vec<bool> = vec![false; uniques.len()];
    let mut units: Vec<BatchUnit> = Vec::new();

    let separator_cost = batch_token_cost(BATCH_ITEM_SEPARATOR);
    let separator_json = json_escaped_len(BATCH_ITEM_SEPARATOR);
    let separator_codepoints = BATCH_ITEM_SEPARATOR.chars().count();
    let mut open_text = String::new();
    let mut open_members: Vec<(usize, std::ops::Range<usize>)> = Vec::new();
    let mut open_cost = 0usize;
    let mut open_json = 0usize;
    let mut open_codepoints = 0usize;

    // Units are emitted in text order -- the open pack is flushed before an
    // oversized text's windows -- so with requests resolved in order, the
    // error returned is the one the earliest failing text produces, as when
    // each text was its own request. Transient vs permanent decides whether
    // the trace is charged, so which error wins matters.
    for (unique, text) in uniques.iter().enumerate() {
        let cost = batch_token_cost(text);
        let json = json_escaped_len(text);
        if cost > max_tokens || json > max_input_json_bytes {
            oversized[unique] = true;
            if !open_members.is_empty() {
                units.push(BatchUnit::Pack {
                    text: std::mem::take(&mut open_text),
                    members: std::mem::take(&mut open_members),
                });
                open_cost = 0;
                open_json = 0;
                open_codepoints = 0;
            }
            let mut codepoint_start = 0usize;
            // Alone, it gets exactly `redact_text`'s windows: the window
            // budget, not the (possibly smaller) pack budget.
            for range in window_ranges(text, window_max_tokens, max_input_json_bytes) {
                let codepoints = text[range.clone()].chars().count();
                units.push(BatchUnit::Window {
                    unique,
                    range,
                    codepoint_start,
                });
                codepoint_start += codepoints;
            }
            continue;
        }
        if let Some(spans) = classifier.cached_item_spans(text) {
            packed_spans[unique] = Some(spans);
            continue;
        }
        if !open_members.is_empty()
            && (open_cost + separator_cost + cost > max_tokens
                || open_json + separator_json + json > max_input_json_bytes)
        {
            units.push(BatchUnit::Pack {
                text: std::mem::take(&mut open_text),
                members: std::mem::take(&mut open_members),
            });
            open_cost = 0;
            open_json = 0;
            open_codepoints = 0;
        }
        if !open_members.is_empty() {
            open_text.push_str(BATCH_ITEM_SEPARATOR);
            open_cost += separator_cost;
            open_json += separator_json;
            open_codepoints += separator_codepoints;
        }
        let codepoints = text.chars().count();
        open_text.push_str(text);
        open_members.push((unique, open_codepoints..open_codepoints + codepoints));
        open_cost += cost;
        open_json += json;
        open_codepoints += codepoints;
    }
    if !open_members.is_empty() {
        units.push(BatchUnit::Pack {
            text: open_text,
            members: open_members,
        });
    }

    // `buffered` preserves order, so responses line up with `units` and the
    // first failing request's error is the one returned.
    // Boxed up front: a lazily mapped stream of borrowing async blocks trips
    // the compiler's higher-ranked `Send` inference inside `async_trait`.
    let requests: Vec<
        futures::future::BoxFuture<'_, Result<Vec<ClassifySpan>, TraceContributionError>>,
    > = units
        .iter()
        .map(|unit| match unit {
            BatchUnit::Pack { text, .. } => classifier.classify_pack(text),
            BatchUnit::Window { unique, range, .. } => {
                classifier.classify_window(&uniques[*unique][range.clone()])
            }
        })
        .collect();
    let responses: Vec<Vec<ClassifySpan>> = futures::stream::iter(requests)
        .buffered(max_concurrent_requests.max(1))
        .try_collect()
        .await?;

    let mut windows: Vec<Vec<(usize, Vec<ClassifySpan>)>> = vec![Vec::new(); uniques.len()];
    for (unit, spans) in units.iter().zip(responses) {
        match unit {
            BatchUnit::Window {
                unique,
                codepoint_start,
                ..
            } => windows[*unique].push((*codepoint_start, spans)),
            BatchUnit::Pack { members, .. } => {
                let per_member = split_pack_spans(backend, members, &spans)?;
                for ((unique, _), member_spans) in members.iter().zip(per_member) {
                    classifier.remember_item_spans(uniques[*unique], &member_spans);
                    packed_spans[*unique] = Some(member_spans);
                }
            }
        }
    }

    let mut unique_results: Vec<Option<SafePrivacyFilterRedaction>> =
        Vec::with_capacity(uniques.len());
    for (unique, text) in uniques.iter().enumerate() {
        let result = if oversized[unique] {
            apply_windowed_spans(backend, text, &windows[unique])?
        } else {
            let spans = packed_spans[unique].take().unwrap_or_default();
            apply_spans(backend, text, &spans)?
        };
        unique_results.push(result);
    }

    Ok(item_unique
        .into_iter()
        .map(|unique| unique.and_then(|unique| unique_results[unique].clone()))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(category: &str, start: usize, end: usize, score: f64) -> ClassifySpan {
        ClassifySpan {
            category: category.into(),
            start,
            end,
            score,
            text: None,
        }
    }

    /// Records every request and finds nothing.
    #[cfg(feature = "near-ai-privacy-filter")]
    #[derive(Default)]
    struct RecordingClassifier {
        requests: std::sync::Mutex<Vec<String>>,
    }

    #[cfg(feature = "near-ai-privacy-filter")]
    #[async_trait::async_trait]
    impl WindowClassifier for RecordingClassifier {
        async fn classify_window(
            &self,
            window: &str,
        ) -> Result<Vec<ClassifySpan>, TraceContributionError> {
            self.requests.lock().unwrap().push(window.to_string());
            Ok(Vec::new())
        }
    }

    #[cfg(feature = "near-ai-privacy-filter")]
    const BODY_CAP: usize = 200 * 1024;

    #[cfg(feature = "near-ai-privacy-filter")]
    #[test]
    fn json_escaped_len_matches_serde_json() {
        for text in [
            "",
            "plain ascii",
            "quote \" backslash \\ slash /",
            "\n\r\t\u{8}\u{c}",
            "\u{0}\u{1}\u{1f}\u{7f}",
            "Zoë 李小龍 😀 👨‍👩‍👧",
        ] {
            assert_eq!(
                json_escaped_len(text),
                serde_json::to_string(text).unwrap().len() - 2,
                "{text:?}"
            );
        }
    }

    #[cfg(feature = "near-ai-privacy-filter")]
    #[tokio::test]
    async fn whitespace_heavy_texts_pack_under_the_body_cap() {
        // Few tokens, many escaped bytes: runs of tabs and newlines.
        let owned: Vec<String> = (0..2_000)
            .map(|i| format!("row {i}:{}{}|", "\t".repeat(200), "\n".repeat(200)))
            .collect();
        let items: Vec<&str> = owned.iter().map(String::as_str).collect();
        let classifier = RecordingClassifier::default();
        let results =
            redact_items_batched("test", &classifier, &items, 32_000, 32_000, BODY_CAP, 1)
                .await
                .unwrap();
        let requests = classifier.requests.lock().unwrap().clone();
        let largest = requests.iter().map(|r| json_escaped_len(r)).max().unwrap();
        assert!(
            largest <= BODY_CAP,
            "a pack's escaped input is {largest} bytes, over the {BODY_CAP} cap"
        );
        // Still packed, and every text still classified.
        assert!(requests.len() < items.len() / 10);
        for (item, result) in items.iter().zip(&results) {
            assert_eq!(&result.as_ref().unwrap().redacted_text, item);
            assert_eq!(
                requests.iter().filter(|r| r.contains(*item)).count(),
                1,
                "each text is sent exactly once"
            );
        }
    }

    #[cfg(feature = "near-ai-privacy-filter")]
    #[tokio::test]
    async fn one_escape_heavy_text_over_the_body_cap_is_windowed_under_it() {
        // A padded table with escaped cells: runs of spaces and `=` cost
        // ~64-128 body bytes per token, and quotes/backslashes 2 body bytes
        // each, so a window inside the token budget is megabytes of body.
        let text: String = (0..3_000)
            .map(|i| format!("| \\\"{i}\\\" |{}|{}|\n", " ".repeat(300), "=".repeat(300)))
            .collect();
        assert!(json_escaped_len(&text) > 2 * BODY_CAP);
        let classifier = RecordingClassifier::default();
        let results =
            redact_items_batched("test", &classifier, &[&text], 32_000, 32_000, BODY_CAP, 1)
                .await
                .unwrap();
        let requests = classifier.requests.lock().unwrap().clone();
        let largest = requests.iter().map(|r| json_escaped_len(r)).max().unwrap();
        assert!(
            largest <= BODY_CAP,
            "a window's escaped input is {largest} bytes, over the {BODY_CAP} cap"
        );
        assert_eq!(requests.concat(), text, "windows cover the text exactly");
        assert_eq!(results[0].as_ref().unwrap().redacted_text, text);
    }

    #[cfg(feature = "near-ai-privacy-filter")]
    #[test]
    fn pack_spans_are_clipped_to_each_text_and_never_cross_a_boundary() {
        // "abc" at 0..3, separator 3..5, "defg" at 5..9, separator 9..11,
        // "hi" at 11..13.
        let members = vec![(0, 0..3), (1, 5..9), (2, 11..13)];
        let spans = vec![
            span("private_name", 3, 5, 0.9),  // wholly in a separator: dropped
            span("private_name", 2, 6, 0.8),  // straddles 0|1: both sides
            span("private_name", 1, 13, 0.7), // straddles all three
            span("private_name", 9, 9, 0.6),  // zero-width at the end of 1
        ];
        let per = split_pack_spans("near-ai", &members, &spans).unwrap();
        let ranges = |m: &Vec<ClassifySpan>| m.iter().map(|s| (s.start, s.end)).collect::<Vec<_>>();
        assert_eq!(ranges(&per[0]), vec![(2, 3), (1, 3)]);
        assert_eq!(ranges(&per[1]), vec![(0, 1), (0, 4), (4, 4)]);
        assert_eq!(ranges(&per[2]), vec![(0, 2)]);
    }

    #[cfg(feature = "near-ai-privacy-filter")]
    #[test]
    fn an_out_of_range_pack_span_fails_the_request() {
        let members = vec![(0, 0..3), (1, 5..9)];
        for bad in [
            span("private_name", 0, 10, 0.9),
            span("private_name", 4, 2, 0.9),
        ] {
            let err = split_pack_spans("near-ai", &members, &[bad]).unwrap_err();
            assert!(err.to_string().contains("out-of-range"));
        }
    }

    #[test]
    fn replaces_single_span() {
        let text = "email me at alice@example.com please";
        let spans = vec![span("private_email", 12, 29, 0.99)];
        let result = apply_spans("near-ai", text, &spans).unwrap().unwrap();
        assert_eq!(
            result.redacted_text,
            "email me at [REDACTED:private_email] please"
        );
        assert_eq!(result.summary.span_count, 1);
        assert_eq!(result.summary.by_label.get("private_email"), Some(&1));
    }
    #[test]
    fn maps_codepoint_offsets_over_multibyte_text() {
        // NEAR AI returns Unicode codepoint offsets, not byte offsets. When
        // multibyte characters precede the span, treating the offsets as
        // byte indices slices the wrong region. Offsets below are codepoint
        // indices exactly as the API emits them ('cafe' with an accented e
        // plus an emoji before the email).
        let text = "café 😀 reach me jane@example.com now";
        let spans = vec![span("private_email", 15, 32, 0.99)];
        let result = apply_spans("near-ai", text, &spans).unwrap().unwrap();
        assert_eq!(
            result.redacted_text,
            "café 😀 reach me[REDACTED:private_email] now"
        );
    }
    #[test]
    fn collapses_overlapping_spans_keeps_highest_score() {
        let text = "abcdefghij";
        let spans = vec![
            span("private_email", 1, 5, 0.4),
            span("private_phone", 3, 7, 0.9),
        ];
        let result = apply_spans("near-ai", text, &spans).unwrap().unwrap();
        assert_eq!(result.redacted_text, "a[REDACTED:private_phone]hij");
        // span_count is raw, even though only one collapsed redaction.
        assert_eq!(result.summary.span_count, 2);
    }
    #[test]
    fn redacts_multibyte_codepoint_span_without_splitting() {
        // Codepoint indices always land on character boundaries, so a span
        // over a multibyte character redacts the whole character. 'héllo':
        // 'é' is codepoint index 1.
        let text = "héllo";
        let spans = vec![span("private_name", 1, 2, 0.9)];
        let result = apply_spans("near-ai", text, &spans).unwrap().unwrap();
        assert_eq!(result.redacted_text, "h[REDACTED:private_name]llo");
    }
    #[test]
    fn rejects_out_of_range_span() {
        // Codepoint index 9999 is far beyond the 5-codepoint string.
        let text = "short";
        let spans = vec![span("private_name", 0, 9999, 0.9)];
        let err = apply_spans("near-ai", text, &spans).unwrap_err();
        assert!(err.to_string().contains("out-of-range"));
    }
    #[test]
    fn rejects_out_of_range_span_over_multibyte_text() {
        // 'café' is 4 codepoints; index 5 is out of range even though the
        // byte length is 5 (the trailing accented byte must not be treated
        // as a valid codepoint index).
        let text = "café";
        let spans = vec![span("private_name", 0, 5, 0.9)];
        let err = apply_spans("near-ai", text, &spans).unwrap_err();
        assert!(err.to_string().contains("out-of-range"));
    }
    #[test]
    fn unknown_category_maps_to_unknown_with_warning() {
        let text = "secret-text";
        let spans = vec![span("brand_new_category", 0, 6, 0.5)];
        let result = apply_spans("near-ai", text, &spans).unwrap().unwrap();
        assert_eq!(result.redacted_text, "[REDACTED:unknown]-text");
        assert!(
            result
                .report
                .warnings
                .iter()
                .any(|w| w.to_lowercase().contains("unsupported"))
        );
    }
    #[test]
    fn known_categories_land_in_allowlist() {
        let table = [
            "private_email",
            "private_phone",
            "account_number",
            "private_address",
            "private_name",
            "secret",
        ];
        for raw in table {
            let mut r = RedactionReport::default();
            let label = safe_privacy_filter_label(Some(raw), &mut r);
            assert_eq!(label, raw, "{raw} should pass through allow-list");
        }
    }
}
