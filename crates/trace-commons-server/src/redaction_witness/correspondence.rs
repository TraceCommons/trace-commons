// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Applying a redaction span list to raw text, exactly.
//!
//! Offsets are CODEPOINT indices, never byte indices. The privacy-filter
//! adapter in `trace-commons-protocol` already learned this: a byte-for-
//! codepoint slip does not fail loudly, it redacts the wrong characters and
//! reports success. Here the consequence is worse -- a witness that applied
//! spans differently from the client would certify the wrong artifact, or
//! refuse an honest one.
//!
//! A span list that does not apply cleanly is a refusal. There is no fallback
//! to accepting the submitted artifact, and no silent truncation, collapsing
//! or reordering of overlapping spans.
//!
//! No function here logs, and callers must not log the values either: `raw`,
//! the returned text, and the span list are all contributor content.

/// One redaction: the half-open codepoint range `[start, end)` of the raw text
/// that the client removed, and the text it put in place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactionSpan {
    /// Codepoint index of the first redacted character.
    pub start: usize,
    /// Codepoint index one past the last redacted character.
    pub end: usize,
    /// The text substituted for the redacted range.
    pub replacement: String,
}

/// Why a span list could not be applied.
///
/// `Display` deliberately carries no offsets and no text: these errors reach
/// operational surfaces, and offsets describe where a detector fired. The
/// fields are for tests and for the caller's own control flow.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CorrespondenceError {
    /// `start > end`.
    #[error("redaction span {index} is inverted")]
    InvertedSpan {
        index: usize,
        start: usize,
        end: usize,
    },
    /// `end` is past the last codepoint of the raw text. A byte offset
    /// supplied where a codepoint index was expected usually lands here.
    #[error("redaction span {index} ends past the input")]
    SpanOutOfRange {
        index: usize,
        end: usize,
        len: usize,
    },
    /// Two spans cover the same region. Applying either one silently would
    /// change what the other asserts, so both are refused.
    #[error("redaction spans {first} and {second} overlap")]
    OverlappingSpans { first: usize, second: usize },
}

/// Apply `spans` to `raw`, returning the redacted text.
///
/// `spans` need not be sorted; disjoint spans describe the same result in any
/// order. They must not overlap, must not be inverted, and must not run past
/// the end of `raw` in codepoints.
pub fn apply_spans(raw: &str, spans: &[RedactionSpan]) -> Result<String, CorrespondenceError> {
    // `boundaries[i]` is the byte offset of codepoint `i`, with a final entry
    // of `raw.len()`, so a valid `end` is `<= boundaries.len() - 1`. Building
    // the table is the only place a codepoint index ever becomes a byte
    // index, and every slice below indexes through it.
    let mut boundaries: Vec<usize> = raw.char_indices().map(|(byte, _)| byte).collect();
    boundaries.push(raw.len());
    let codepoint_len = boundaries.len() - 1;

    for (index, span) in spans.iter().enumerate() {
        if span.start > span.end {
            return Err(CorrespondenceError::InvertedSpan {
                index,
                start: span.start,
                end: span.end,
            });
        }
        if span.end > codepoint_len {
            return Err(CorrespondenceError::SpanOutOfRange {
                index,
                end: span.end,
                len: codepoint_len,
            });
        }
    }

    // Disjoint spans describe the same result in any order, so sort a view of
    // the indices rather than requiring the caller to pre-sort. Ties break on
    // the original index, keeping the whole pass deterministic.
    let mut order: Vec<usize> = (0..spans.len()).collect();
    order.sort_by_key(|&index| (spans[index].start, index));

    // With starts sorted, comparing each span against its immediate
    // predecessor is sufficient: a span contained in an earlier one starts
    // before that one ends, so it is caught at that pair and returns here.
    // Nothing therefore needs to track the widest end seen so far.
    let mut previous: Option<(usize, usize)> = None;
    for &index in &order {
        if let Some((first, end)) = previous {
            if spans[index].start < end {
                return Err(CorrespondenceError::OverlappingSpans {
                    first,
                    second: index,
                });
            }
        }
        previous = Some((index, spans[index].end));
    }

    let mut out = String::with_capacity(raw.len());
    let mut cursor = 0usize;
    for &index in &order {
        let span = &spans[index];
        out.push_str(&raw[cursor..boundaries[span.start]]);
        out.push_str(&span.replacement);
        cursor = boundaries[span.end];
    }
    out.push_str(&raw[cursor..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(start: usize, end: usize, replacement: &str) -> RedactionSpan {
        RedactionSpan {
            start,
            end,
            replacement: replacement.to_string(),
        }
    }

    #[test]
    fn spans_apply_in_order_and_produce_the_expected_text() {
        let out = apply_spans(
            "call alice at 555-0100 today",
            &[span(5, 10, "[NAME]"), span(14, 22, "[PHONE]")],
        )
        .unwrap();
        assert_eq!(out, "call [NAME] at [PHONE] today");
    }

    #[test]
    fn adjacent_spans_are_not_an_overlap() {
        // end == next start touches but does not cover the same character.
        let out = apply_spans("abcdef", &[span(1, 3, "X"), span(3, 5, "Y")]).unwrap();
        assert_eq!(out, "aXYf");
    }

    #[test]
    fn an_unsorted_span_list_applies_to_the_same_text() {
        let out = apply_spans("abcdef", &[span(3, 5, "Y"), span(1, 3, "X")]).unwrap();
        assert_eq!(out, "aXYf");
    }

    #[test]
    fn overlapping_spans_are_refused() {
        let err = apply_spans("hello world", &[span(0, 5, "X"), span(3, 8, "Y")]).unwrap_err();
        assert_eq!(
            err,
            CorrespondenceError::OverlappingSpans {
                first: 0,
                second: 1
            }
        );
    }

    #[test]
    fn a_later_span_swallowed_by_an_earlier_one_is_refused() {
        // Sorting by start alone would place these adjacent; containment is
        // still an overlap and the widest end so far is what must be tracked.
        let err = apply_spans("hello world", &[span(0, 9, "X"), span(2, 4, "Y")]).unwrap_err();
        assert_eq!(
            err,
            CorrespondenceError::OverlappingSpans {
                first: 0,
                second: 1
            }
        );
    }

    #[test]
    fn a_span_past_the_end_is_refused() {
        let err = apply_spans("short", &[span(0, 99, "X")]).unwrap_err();
        assert_eq!(
            err,
            CorrespondenceError::SpanOutOfRange {
                index: 0,
                end: 99,
                len: 5
            }
        );
    }

    #[test]
    fn an_inverted_span_is_refused() {
        let err = apply_spans("hello", &[span(4, 2, "X")]).unwrap_err();
        assert_eq!(
            err,
            CorrespondenceError::InvertedSpan {
                index: 0,
                start: 4,
                end: 2
            }
        );
    }

    #[test]
    fn offsets_are_codepoints_not_bytes() {
        // "café latte" is 10 codepoints and 11 bytes; the accented e sits at
        // codepoint 3 / bytes 3..5. Codepoints 5..10 are "latte"; the SAME
        // numbers read as bytes are " latt", which is also a valid char
        // boundary pair -- so a byte-index implementation does not panic
        // here, it silently produces "caféREDACTEDe". The two answers differ.
        let out = apply_spans("café latte", &[span(5, 10, "REDACTED")]).unwrap();
        assert_eq!(out, "café REDACTED");
    }

    #[test]
    fn a_span_over_a_multibyte_character_redacts_the_whole_character() {
        // Codepoint 3 is the accented e alone. Read as bytes, 3..4 would cut
        // the character in half and panic rather than truncate.
        let out = apply_spans("café latte", &[span(3, 4, "E")]).unwrap();
        assert_eq!(out, "cafE latte");
    }

    #[test]
    fn a_byte_offset_that_is_not_a_codepoint_index_is_refused_not_truncated() {
        // "café" is 4 codepoints and 5 bytes. A caller that passed byte
        // offsets would send end == 5, which is a valid byte boundary and an
        // invalid codepoint index. It is refused, not clamped to the end.
        let err = apply_spans("café", &[span(0, 5, "X")]).unwrap_err();
        assert_eq!(
            err,
            CorrespondenceError::SpanOutOfRange {
                index: 0,
                end: 5,
                len: 4
            }
        );
    }

    #[test]
    fn a_span_ending_exactly_at_the_end_is_accepted() {
        let out = apply_spans("café", &[span(3, 4, "E")]).unwrap();
        assert_eq!(out, "cafE");
    }

    #[test]
    fn an_empty_span_list_returns_the_input_unchanged() {
        let out = apply_spans("café latte", &[]).unwrap();
        assert_eq!(out, "café latte");
    }

    #[test]
    fn an_empty_input_with_no_spans_is_empty() {
        let out = apply_spans("", &[]).unwrap();
        assert_eq!(out, "");
    }

    #[test]
    fn the_error_display_carries_no_offsets_or_text() {
        let rendered = CorrespondenceError::SpanOutOfRange {
            index: 0,
            end: 99,
            len: 5,
        }
        .to_string();
        assert!(!rendered.contains("99"), "offsets must not reach Display");
        assert!(!rendered.contains('5'), "lengths must not reach Display");
    }
}
