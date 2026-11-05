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

#[derive(Deserialize, Clone)]
pub(crate) struct ClassifySpan {
    pub(crate) category: String,
    pub(crate) start: usize,
    pub(crate) end: usize,
    #[serde(default)]
    pub(crate) score: f64,
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
#[cfg(feature = "self-hosted-privacy-filter")]
pub(crate) fn classifier_bpe() -> Option<&'static tiktoken_rs::CoreBPE> {
    static BPE: std::sync::OnceLock<Option<tiktoken_rs::CoreBPE>> = std::sync::OnceLock::new();
    BPE.get_or_init(|| tiktoken_rs::o200k_base().ok()).as_ref()
}
/// Count the tokens the classifier will charge for `text`.
#[cfg(feature = "self-hosted-privacy-filter")]
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
#[cfg(feature = "self-hosted-privacy-filter")]
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
#[cfg(feature = "self-hosted-privacy-filter")]
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
            });
        }
    }
    apply_spans(backend, text, &all_spans)
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

/// The tokenizer windowing the self-hosted backend uses. These tests lived
/// with the NEAR AI adapter until it stopped windowing by tokens (it sizes
/// windows by request body now); the fixtures are sized for the 32,000-token
/// budget that backend used, which is why that number appears here.
#[cfg(all(test, feature = "self-hosted-privacy-filter"))]
mod tokenizer_window_tests {
    use super::*;

    const MAX_CLASSIFY_INPUT_TOKENS: usize = 32_000;

    /// The tokenizer must be the one the endpoint actually charges against.
    /// These counts are the endpoint's own `usage.input_tokens`, recorded
    /// 2026-08-27; `cl100k_base` disagrees on several of them.
    #[test]
    fn tokenizer_reproduces_the_endpoints_own_counts() {
        let cases: &[(&str, usize)] = &[
            ("hello", 1),
            ("hello world", 2),
            ("The quick brown fox jumps over the lazy dog.", 10),
            ("alice@example.com", 3),
            ("/usr/local/lib/python3.11/site-packages/", 11),
            ("a7f3c9d2e1b48856f0c1d2e3a4b5c6d7", 29),
            ("tenant_id submission_id auth_principal_ref", 8),
            ("supercalifragilisticexpialidocious", 10),
        ];
        for (text, expected) in cases {
            assert_eq!(
                classifier_token_count(text),
                Some(*expected),
                "token count drifted from the endpoint's accounting for {text:?}"
            );
        }
    }

    /// The property the whole change exists for: no window may exceed the
    /// budget, whatever the content's token density.
    #[test]
    fn every_window_fits_the_token_budget() {
        let prose = "Please email alice@example.com about invoice 12345. ".repeat(600);
        let dense = (0..600)
            .map(|i| format!("user{i:04}@example.com /home/u{i:04}/src/f{i:04}.rs"))
            .collect::<Vec<_>>()
            .join(" ");
        let hex = "a7f3c9d2e1b48856f0c1d2e3a4b5c6d7".repeat(400);

        for (label, text) in [("prose", &prose), ("dense", &dense), ("hex", &hex)] {
            let ranges = chunk_token_ranges(text, MAX_CLASSIFY_INPUT_TOKENS);
            for range in &ranges {
                let tokens = classifier_token_count(&text[range.clone()]).expect("tokenizer");
                assert!(
                    tokens <= MAX_CLASSIFY_INPUT_TOKENS,
                    "{label}: window of {tokens} tokens exceeds the budget of {}",
                    MAX_CLASSIFY_INPUT_TOKENS
                );
            }
        }
    }

    /// Ranges must be contiguous and cover the whole field: a gap would drop
    /// text from classification entirely, which is a silent privacy hole
    /// rather than a performance bug.
    #[test]
    fn token_windows_cover_the_whole_field_without_gaps() {
        let text = (0..300)
            .map(|i| format!("line {i} with alice@example.com and /var/log/f{i}.log"))
            .collect::<Vec<_>>()
            .join("\n");
        let ranges = chunk_token_ranges(&text, MAX_CLASSIFY_INPUT_TOKENS);

        assert_eq!(ranges.first().expect("at least one window").start, 0);
        assert_eq!(ranges.last().expect("at least one window").end, text.len());
        for pair in ranges.windows(2) {
            assert_eq!(pair[0].end, pair[1].start, "gap or overlap between windows");
        }
        let rebuilt: String = ranges.iter().map(|r| &text[r.clone()]).collect();
        assert_eq!(rebuilt, text, "windows must reconstruct the field exactly");
    }

    /// Budgeting in tokens is what lets sparse content travel in FEWER
    /// requests than dense content of the same size -- the thing a byte
    /// budget structurally cannot do, and the reason dense windows used to
    /// 502 while prose windows of identical size were fine.
    #[test]
    fn sparse_content_packs_into_fewer_windows_than_dense() {
        // Line-delimited on purpose. The greedy packer this test is about
        // works on line segments; an UNBROKEN run takes the bisection path
        // instead, which halves until each piece fits and so quantises the
        // window count to a power of two. Two inputs of very different token
        // density then land on the same count -- prose at 149k tokens and
        // dense at 250k both produced 8 windows -- and the comparison says
        // nothing. The original fixtures were single lines and passed only
        // because, at the old 1,000-token budget, they happened to fall in
        // different bisection bands.
        // Sized from measurement: at 16,000 lines prose is ~176k tokens (~6
        // windows) and dense ~256k (~8), a margin of two. At 4,000 both round
        // to 2 and tie; a margin of one is fragile to packing slack.
        let lines = 16_000;
        let prose = "The agent reviewed the report and summarised the findings.\n".repeat(lines);
        let dense = (0..lines)
            .map(|i| format!("user{i:04}@example.com /home/u{i:04}/src/f{i:04}.rs\n"))
            .collect::<String>();

        let prose_tokens = classifier_token_count(&prose).expect("tokenizer");
        let dense_tokens = classifier_token_count(&dense).expect("tokenizer");
        assert!(
            prose_tokens > MAX_CLASSIFY_INPUT_TOKENS && dense_tokens > MAX_CLASSIFY_INPUT_TOKENS,
            "both fixtures must exceed the budget or neither splits"
        );
        assert!(
            dense_tokens > prose_tokens,
            "the dense fixture must actually be denser: prose {prose_tokens}, dense {dense_tokens}"
        );

        let prose_windows = chunk_token_ranges(&prose, MAX_CLASSIFY_INPUT_TOKENS).len();
        let dense_windows = chunk_token_ranges(&dense, MAX_CLASSIFY_INPUT_TOKENS).len();
        assert!(
            prose_windows < dense_windows,
            "prose took {prose_windows} windows and dense took {dense_windows}; \
             token budgeting should give sparse content larger windows"
        );
    }

    /// A single line longer than the budget still has to be split, or one
    /// base64 blob or long log line stalls its whole field.
    #[test]
    fn an_oversized_single_line_is_split() {
        // Size the fixture FROM the budget. A literal repeat count was sized
        // for a 1,000-token budget and silently stopped exceeding it when the
        // budget rose to 32,000 -- the test still passed its inner assertion
        // while no longer testing splitting at all.
        let unit = "a7f3c9d2e1b48856f0c1d2e3a4b5c6d7";
        let unit_tokens = classifier_token_count(unit).expect("tokenizer");
        let repeats = (MAX_CLASSIFY_INPUT_TOKENS / unit_tokens) * 2 + 8;
        let one_line = unit.repeat(repeats);
        assert!(!one_line.contains('\n'));
        assert!(
            classifier_token_count(&one_line).expect("tokenizer") > MAX_CLASSIFY_INPUT_TOKENS,
            "fixture must exceed the budget or this test proves nothing"
        );
        let ranges = chunk_token_ranges(&one_line, MAX_CLASSIFY_INPUT_TOKENS);
        assert!(ranges.len() > 1, "an oversized single line must be split");
        for range in &ranges {
            let tokens = classifier_token_count(&one_line[range.clone()]).expect("tokenizer");
            assert!(tokens <= MAX_CLASSIFY_INPUT_TOKENS);
        }
    }

    #[test]
    fn chunk_byte_ranges_cover_text_and_respect_limit() {
        let text = "line one\nline two\nline three\n";
        let ranges = chunk_byte_ranges(text, 12);
        assert_eq!(ranges.first().unwrap().start, 0);
        assert_eq!(ranges.last().unwrap().end, text.len());
        let mut prev = 0;
        for r in &ranges {
            assert_eq!(r.start, prev, "ranges must be contiguous");
            assert!(r.end - r.start <= 12, "range {r:?} exceeds limit");
            assert!(text.is_char_boundary(r.start) && text.is_char_boundary(r.end));
            prev = r.end;
        }
        let joined: String = ranges.iter().map(|r| &text[r.clone()]).collect();
        assert_eq!(joined, text, "windows must reconstruct the input");
        // Newline-preferring: no window splits mid-line here.
        for r in &ranges {
            assert!(text[r.clone()].ends_with('\n') || r.end == text.len());
        }
    }

    #[test]
    fn chunk_byte_ranges_hard_splits_a_long_unbroken_run() {
        // A single line longer than the limit must still be split, on a
        // char boundary, into covering windows.
        let text = "café".repeat(10); // 50 bytes, no newline, multibyte
        let ranges = chunk_byte_ranges(&text, 8);
        let mut prev = 0;
        for r in &ranges {
            assert_eq!(r.start, prev);
            assert!(r.end - r.start <= 8);
            assert!(text.is_char_boundary(r.start) && text.is_char_boundary(r.end));
            prev = r.end;
        }
        assert_eq!(prev, text.len());
    }
}
