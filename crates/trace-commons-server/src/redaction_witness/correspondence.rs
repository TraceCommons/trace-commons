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
//! Replacements are constrained to the redaction placeholder grammar, so that
//! applying a span list can only ever *remove* information and stamp a marker
//! in its place. Without that constraint a span is an insertion channel: any
//! span, not merely a zero-width one, could swap raw text for arbitrary
//! attacker-chosen content and still satisfy the correspondence check, and
//! the artifact would not derive from the raw text by redaction alone.
//!
//! No function here logs, and callers must not log the values either: `raw`,
//! the returned text, and the span list are all contributor content.

use sha2::{Digest, Sha256};

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
/// Neither formatter carries offsets, lengths or text: these errors reach
/// operational surfaces, and offsets describe where a detector fired. The
/// fields are for tests and for the caller's own control flow, reachable by
/// matching on the variant -- never by formatting it.
///
/// `Debug` is hand-written to delegate to `Display` for that reason. A derived
/// `Debug` prints every field, and `tracing::warn!(?err)` is how an error
/// ordinarily reaches a log in this repo, so guarding `Display` alone would
/// leave the discipline true on the path nobody uses and false on the path
/// everybody uses.
#[derive(Clone, PartialEq, Eq, thiserror::Error)]
pub enum CorrespondenceError {
    /// `start > end`.
    #[error("redaction span {index} is inverted")]
    InvertedSpan {
        index: usize,
        start: usize,
        end: usize,
    },
    /// `start == end`. A redaction removes something; a zero-width span
    /// removes nothing and only inserts, so it is not a redaction even when
    /// its replacement is a well-formed placeholder.
    #[error("redaction span {index} is empty")]
    EmptySpan { index: usize },
    /// The replacement is not a redaction placeholder. Carries no copy of the
    /// offending text: it is contributor content, and the text an attacker
    /// tried to insert is exactly what must not reach a log.
    #[error("redaction span {index} has a replacement that is not a placeholder")]
    MalformedReplacement { index: usize },
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
    ///
    /// Classifiers do genuinely return overlapping spans, which is why the
    /// client's redaction path collapses them. Seeing this error usually means
    /// a caller passed the raw classifier list rather than the collapsed one.
    #[error(
        "redaction spans {first} and {second} overlap; the witness consumes the post-collapse span list, not a raw classifier list"
    )]
    OverlappingSpans { first: usize, second: usize },
    /// The span list applied cleanly, but the result is not the submitted
    /// artifact. The artifact was fabricated, padded, truncated, or taken
    /// from another session.
    ///
    /// Carries nothing -- not the two lengths, not the first differing
    /// offset. Either would describe contributor content, and neither would
    /// change what the caller does, which is refuse.
    #[error("the redacted artifact does not match the raw text with the redaction spans applied")]
    RedactedMismatch,
}

impl std::fmt::Debug for CorrespondenceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Same discipline as Display, and the same text, so that `?err` in a
        // log macro cannot render what `%err` is guarded against.
        std::fmt::Display::fmt(self, formatter)
    }
}

/// The longest label this accepts. Real labels are a closed allowlist of short
/// identifiers (`private_email`, `secret_like`, `unknown`); the cap is generous
/// headroom over those, and exists so the label cannot itself become a payload.
const MAX_LABEL_LEN: usize = 64;

/// Is `replacement` a redaction placeholder -- `[REDACTED]`, or
/// `[REDACTED:<label>]` with a non-empty label of lowercase ASCII letters,
/// digits and underscores?
///
/// The charset is a deliberate superset of the labels the client emits, which
/// come from a closed allowlist in `trace-commons-protocol`. The witness checks
/// mechanics, not redaction policy, so it must not need changing when that
/// allowlist grows a label.
fn is_redaction_placeholder(replacement: &str) -> bool {
    if replacement == "[REDACTED]" {
        return true;
    }
    let Some(label) = replacement
        .strip_prefix("[REDACTED:")
        .and_then(|rest| rest.strip_suffix(']'))
    else {
        return false;
    };
    !label.is_empty()
        && label.len() <= MAX_LABEL_LEN
        && label
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Apply `spans` to `raw`, returning the redacted text.
///
/// `spans` is the **post-collapse** list -- the spans the client actually
/// applied, after its own overlap-collapsing step -- not the list a classifier
/// returned. Classifiers do return overlapping spans, which is why that
/// collapse step exists; a witness handed the raw list would refuse honest
/// submissions with [`CorrespondenceError::OverlappingSpans`]. The client
/// applies policy, the witness only checks mechanics.
///
/// `spans` need not be sorted; disjoint spans describe the same result in any
/// order. They must not overlap, must not be inverted, must not be empty, must
/// not run past the end of `raw` in codepoints, and each replacement must be a
/// redaction placeholder.
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
        if span.start == span.end {
            return Err(CorrespondenceError::EmptySpan { index });
        }
        if span.end > codepoint_len {
            return Err(CorrespondenceError::SpanOutOfRange {
                index,
                end: span.end,
                len: codepoint_len,
            });
        }
        if !is_redaction_placeholder(&span.replacement) {
            return Err(CorrespondenceError::MalformedReplacement { index });
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

/// Evidence that a redacted artifact corresponds to the raw text.
///
/// Carries the artifact's SHA-256 and nothing else. It crosses into
/// certificate construction, where everything it holds becomes a candidate
/// for logging, so it holds only what the certificate binds. Adding the text,
/// the span count or a label here would put contributor content on that path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorrespondenceProof {
    redacted_sha256: String,
}

impl CorrespondenceProof {
    /// Lowercase hex SHA-256 of the redacted artifact's bytes.
    pub fn redacted_sha256(&self) -> &str {
        &self.redacted_sha256
    }
}

/// Check that `redacted` is what applying `spans` to `raw` produces.
///
/// This proves **faithfulness**: the redacted artifact derives from the raw
/// one by redaction alone, so it was not fabricated, padded, truncated, or
/// swapped for another session's output. The placeholder grammar
/// [`apply_spans`] enforces is what makes "by redaction alone" true rather
/// than aspirational -- without it a span is an insertion channel and a
/// matching artifact proves nothing.
///
/// It does **not** prove that enough was redacted. Sufficiency is the
/// redaction policy's job and the PII backstop's; a submission that redacts
/// nothing at all corresponds perfectly to its raw text.
///
/// The comparison is byte equality. Containment or a prefix match would let a
/// contributor append arbitrary text to an otherwise faithful artifact and
/// still pass, which is the whole attack this refuses.
///
/// Neither argument nor the span list may be logged by the caller: all three
/// are contributor content.
pub fn check_correspondence(
    raw: &str,
    redacted: &str,
    spans: &[RedactionSpan],
) -> Result<CorrespondenceProof, CorrespondenceError> {
    let expected = apply_spans(raw, spans)?;
    if expected != redacted {
        return Err(CorrespondenceError::RedactedMismatch);
    }
    Ok(CorrespondenceProof {
        redacted_sha256: hex::encode(Sha256::digest(redacted.as_bytes())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The placeholder every test that is not about the grammar uses, so that
    /// a grammar failure can never be what such a test is actually observing.
    const PLACEHOLDER: &str = "[REDACTED]";

    fn span(start: usize, end: usize, replacement: &str) -> RedactionSpan {
        RedactionSpan {
            start,
            end,
            replacement: replacement.to_string(),
        }
    }

    fn redact(start: usize, end: usize) -> RedactionSpan {
        span(start, end, PLACEHOLDER)
    }

    #[test]
    fn spans_apply_in_order_and_produce_the_expected_text() {
        let out = apply_spans(
            "call alice at 555-0100 today",
            &[
                span(5, 10, "[REDACTED:private_name]"),
                span(14, 22, "[REDACTED:private_phone]"),
            ],
        )
        .unwrap();
        assert_eq!(
            out,
            "call [REDACTED:private_name] at [REDACTED:private_phone] today"
        );
    }

    #[test]
    fn adjacent_spans_are_not_an_overlap() {
        // end == next start touches but does not cover the same character.
        let out = apply_spans("abcdef", &[redact(1, 3), span(3, 5, "[REDACTED:secret]")]).unwrap();
        assert_eq!(out, "a[REDACTED][REDACTED:secret]f");
    }

    #[test]
    fn an_unsorted_span_list_applies_to_the_same_text() {
        let out = apply_spans("abcdef", &[span(3, 5, "[REDACTED:secret]"), redact(1, 3)]).unwrap();
        assert_eq!(out, "a[REDACTED][REDACTED:secret]f");
    }

    #[test]
    fn overlapping_spans_are_refused() {
        let err = apply_spans("hello world", &[redact(0, 5), redact(3, 8)]).unwrap_err();
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
        // Containment is still an overlap.
        let err = apply_spans("hello world", &[redact(0, 9), redact(2, 4)]).unwrap_err();
        assert_eq!(
            err,
            CorrespondenceError::OverlappingSpans {
                first: 0,
                second: 1
            }
        );
    }

    #[test]
    fn the_overlap_error_names_the_post_collapse_contract() {
        // The message a caller reads when it fires is the only place the
        // precondition is stated at runtime, so it must actually say it.
        let rendered = CorrespondenceError::OverlappingSpans {
            first: 0,
            second: 1,
        }
        .to_string();
        assert!(
            rendered.contains("post-collapse"),
            "overlap error must point the caller at the collapse step: {rendered}"
        );
    }

    #[test]
    fn a_span_past_the_end_is_refused() {
        let err = apply_spans("short", &[redact(0, 99)]).unwrap_err();
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
        let err = apply_spans("hello", &[redact(4, 2)]).unwrap_err();
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
        // here, it silently produces "café[REDACTED]e". The answers differ.
        let out = apply_spans("café latte", &[redact(5, 10)]).unwrap();
        assert_eq!(out, "café [REDACTED]");
    }

    #[test]
    fn a_span_over_a_multibyte_character_redacts_the_whole_character() {
        // Codepoint 3 is the accented e alone. Read as bytes, 3..4 would cut
        // the character in half and panic rather than truncate.
        let out = apply_spans("café latte", &[redact(3, 4)]).unwrap();
        assert_eq!(out, "caf[REDACTED] latte");
    }

    #[test]
    fn a_byte_offset_that_is_not_a_codepoint_index_is_refused_not_truncated() {
        // "café" is 4 codepoints and 5 bytes. A caller that passed byte
        // offsets would send end == 5, which is a valid byte boundary and an
        // invalid codepoint index. It is refused, not clamped to the end.
        let err = apply_spans("café", &[redact(0, 5)]).unwrap_err();
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
        let out = apply_spans("café", &[redact(3, 4)]).unwrap();
        assert_eq!(out, "caf[REDACTED]");
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

    // --- the insertion channel ---

    #[test]
    fn a_zero_width_span_is_refused_even_with_a_well_formed_placeholder() {
        // The replacement is impeccable; the span still removes nothing, so
        // it is pure insertion. The grammar rule cannot be what refuses this.
        let err = apply_spans("café latte", &[redact(3, 3)]).unwrap_err();
        assert_eq!(err, CorrespondenceError::EmptySpan { index: 0 });
    }

    #[test]
    fn a_zero_width_span_at_the_very_end_is_refused() {
        let err = apply_spans("café", &[redact(4, 4)]).unwrap_err();
        assert_eq!(err, CorrespondenceError::EmptySpan { index: 0 });
    }

    #[test]
    fn an_arbitrary_replacement_is_refused() {
        // One character out, a whole fabricated passage in. The span is
        // well-formed in every other respect, so the empty-span rule cannot
        // be what refuses this.
        let err =
            apply_spans("hello world", &[span(0, 1, "a long fabricated passage")]).unwrap_err();
        assert_eq!(err, CorrespondenceError::MalformedReplacement { index: 0 });
    }

    #[test]
    fn a_placeholder_with_smuggled_text_after_it_is_refused() {
        let err =
            apply_spans("hello world", &[span(0, 5, "[REDACTED] and also this")]).unwrap_err();
        assert_eq!(err, CorrespondenceError::MalformedReplacement { index: 0 });
    }

    #[test]
    fn a_placeholder_with_smuggled_text_before_it_is_refused() {
        let err = apply_spans("hello world", &[span(0, 5, "smuggled [REDACTED]")]).unwrap_err();
        assert_eq!(err, CorrespondenceError::MalformedReplacement { index: 0 });
    }

    #[test]
    fn a_label_outside_the_charset_is_refused() {
        // Prose inside the label is the obvious way to smuggle through the
        // labelled form; spaces and capitals are not label characters.
        let err = apply_spans("hello world", &[span(0, 5, "[REDACTED:Private Name]")]).unwrap_err();
        assert_eq!(err, CorrespondenceError::MalformedReplacement { index: 0 });
    }

    #[test]
    fn an_empty_label_is_refused() {
        let err = apply_spans("hello world", &[span(0, 5, "[REDACTED:]")]).unwrap_err();
        assert_eq!(err, CorrespondenceError::MalformedReplacement { index: 0 });
    }

    #[test]
    fn an_overlong_label_is_refused() {
        // In-charset but long enough to be a payload rather than a label.
        let label = "a".repeat(MAX_LABEL_LEN + 1);
        let err =
            apply_spans("hello world", &[span(0, 5, &format!("[REDACTED:{label}]"))]).unwrap_err();
        assert_eq!(err, CorrespondenceError::MalformedReplacement { index: 0 });
    }

    #[test]
    fn a_label_at_the_length_limit_is_accepted() {
        let label = "a".repeat(MAX_LABEL_LEN);
        let out =
            apply_spans("hello world", &[span(0, 5, &format!("[REDACTED:{label}]"))]).unwrap();
        assert_eq!(out, format!("[REDACTED:{label}] world"));
    }

    #[test]
    fn every_label_the_client_emits_is_accepted() {
        // The witness must not need changing when the client's allowlist
        // grows, so the grammar is a superset of what it emits today.
        for label in [
            "account_number",
            "credit_card",
            "ip_address",
            "private_address",
            "private_date",
            "private_email",
            "private_location",
            "private_name",
            "private_person",
            "private_phone",
            "private_url",
            "secret",
            "secret_like",
            "ssn",
            "unknown",
        ] {
            let replacement = format!("[REDACTED:{label}]");
            let out = apply_spans("hello world", &[span(0, 5, &replacement)])
                .unwrap_or_else(|err| panic!("{label} should be accepted, got {err}"));
            assert_eq!(out, format!("{replacement} world"));
        }
    }

    #[test]
    fn an_empty_span_carrying_arbitrary_text_reports_the_empty_span() {
        // Both rules would refuse this. Pinning which one fires keeps the
        // message an operator reads pointed at the structural fault rather
        // than sending them hunting for a malformed label.
        let err = apply_spans("hello world", &[span(3, 3, "fabricated")]).unwrap_err();
        assert_eq!(err, CorrespondenceError::EmptySpan { index: 0 });
    }

    // --- hash-only discipline ---

    #[test]
    fn neither_formatter_renders_offsets_or_lengths() {
        // `Display` is the guarded path; `Debug` is the path a caller actually
        // takes, because `tracing::warn!(?err)` is how an error reaches a log
        // here. Both must be safe, so both are asserted, for every variant
        // that carries an offset. The values are distinctive so a substring
        // match cannot pass by coincidence.
        for err in [
            CorrespondenceError::InvertedSpan {
                index: 0,
                start: 4242,
                end: 9931,
            },
            CorrespondenceError::SpanOutOfRange {
                index: 0,
                end: 9931,
                len: 5150,
            },
        ] {
            for rendered in [err.to_string(), format!("{err:?}")] {
                for leaked in ["4242", "9931", "5150"] {
                    assert!(
                        !rendered.contains(leaked),
                        "offset {leaked} reached a formatter: {rendered}"
                    );
                }
            }
        }
    }

    #[test]
    fn debug_renders_exactly_what_display_renders() {
        // Pins the delegation rather than the absence of particular digits:
        // a future variant that gains a field cannot leak it through Debug
        // without also leaking it through Display, where it would be caught.
        for err in [
            CorrespondenceError::InvertedSpan {
                index: 0,
                start: 4242,
                end: 9931,
            },
            CorrespondenceError::EmptySpan { index: 1 },
            CorrespondenceError::MalformedReplacement { index: 2 },
            CorrespondenceError::SpanOutOfRange {
                index: 3,
                end: 9931,
                len: 5150,
            },
            CorrespondenceError::OverlappingSpans {
                first: 4,
                second: 5,
            },
            CorrespondenceError::RedactedMismatch,
        ] {
            assert_eq!(format!("{err:?}"), err.to_string());
        }
    }

    #[test]
    fn a_malformed_replacement_never_reaches_an_error_value() {
        // The rejected text is what an attacker chose to insert. It must not
        // be reachable from the error at all -- not via Display, and not via
        // a field a caller could log with Debug.
        let secret = "smuggled-payload";
        let err = apply_spans("hello world", &[span(0, 5, secret)]).unwrap_err();
        assert_eq!(err, CorrespondenceError::MalformedReplacement { index: 0 });
        assert!(!err.to_string().contains(secret));
        assert!(!format!("{err:?}").contains(secret));
    }
}

#[cfg(test)]
mod correspondence_tests {
    use super::*;

    const RAW: &str = "call alice at 555-0100 today";

    fn spans() -> Vec<RedactionSpan> {
        vec![
            RedactionSpan {
                start: 5,
                end: 10,
                replacement: "[REDACTED:private_name]".to_string(),
            },
            RedactionSpan {
                start: 14,
                end: 22,
                replacement: "[REDACTED:private_phone]".to_string(),
            },
        ]
    }

    fn faithful() -> String {
        apply_spans(RAW, &spans()).unwrap()
    }

    #[test]
    fn a_faithful_redaction_is_proved() {
        let proof = check_correspondence(RAW, &faithful(), &spans()).unwrap();
        assert_eq!(
            proof.redacted_sha256(),
            hex::encode(Sha256::digest(faithful().as_bytes()))
        );
    }

    #[test]
    fn a_fabricated_redacted_artifact_is_refused() {
        let err = check_correspondence(RAW, "entirely different text", &spans()).unwrap_err();
        assert_eq!(err, CorrespondenceError::RedactedMismatch);
    }

    #[test]
    fn extra_text_appended_to_the_redacted_artifact_is_refused() {
        let padded = format!("{} and a fabricated postscript", faithful());
        let err = check_correspondence(RAW, &padded, &spans()).unwrap_err();
        assert_eq!(err, CorrespondenceError::RedactedMismatch);
    }

    #[test]
    fn extra_text_prepended_to_the_redacted_artifact_is_refused() {
        let padded = format!("a fabricated preamble {}", faithful());
        let err = check_correspondence(RAW, &padded, &spans()).unwrap_err();
        assert_eq!(err, CorrespondenceError::RedactedMismatch);
    }

    #[test]
    fn a_truncated_redacted_artifact_is_refused() {
        let faithful = faithful();
        let truncated = &faithful[..faithful.len() - 6];
        let err = check_correspondence(RAW, truncated, &spans()).unwrap_err();
        assert_eq!(err, CorrespondenceError::RedactedMismatch);
    }

    #[test]
    fn a_span_that_hides_nothing_still_has_to_match() {
        // No spans: nothing was redacted, so the artifact must be the raw
        // text itself. Anything else is a swap, and passes no more easily
        // for having claimed no redactions.
        let proof = check_correspondence(RAW, RAW, &[]).unwrap();
        assert_eq!(
            proof.redacted_sha256(),
            hex::encode(Sha256::digest(RAW.as_bytes()))
        );
        let err = check_correspondence(RAW, "something else entirely", &[]).unwrap_err();
        assert_eq!(err, CorrespondenceError::RedactedMismatch);
    }

    #[test]
    fn a_whitespace_only_difference_is_refused() {
        // Byte equality, not a normalizing comparison.
        let err = check_correspondence(RAW, &format!("{} ", faithful()), &spans()).unwrap_err();
        assert_eq!(err, CorrespondenceError::RedactedMismatch);
    }

    #[test]
    fn a_span_list_fault_reports_the_span_fault_not_a_mismatch() {
        // The span list never applies, so there is nothing to compare. The
        // caller must see why the list was refused, not a mismatch that
        // would send them looking at the artifact.
        let err = check_correspondence(
            RAW,
            &faithful(),
            &[RedactionSpan {
                start: 0,
                end: 1,
                replacement: "a fabricated passage".to_string(),
            }],
        )
        .unwrap_err();
        assert_eq!(err, CorrespondenceError::MalformedReplacement { index: 0 });
    }

    #[test]
    fn a_proof_carries_the_hash_and_nothing_else() {
        // Everything the proof carries becomes a candidate for logging when
        // it crosses into certificate construction.
        let proof = check_correspondence(RAW, &faithful(), &spans()).unwrap();
        let rendered = format!("{proof:?}");
        for leaked in ["alice", "555-0100", "private_name", "REDACTED", "call"] {
            assert!(
                !rendered.contains(leaked),
                "proof rendered contributor content: {rendered}"
            );
        }
        assert!(rendered.contains(proof.redacted_sha256()));
    }

    #[test]
    fn neither_formatter_of_the_mismatch_carries_content_or_lengths() {
        let err = CorrespondenceError::RedactedMismatch;
        for rendered in [err.to_string(), format!("{err:?}")] {
            assert_eq!(
                rendered,
                "the redacted artifact does not match the raw text with the redaction spans applied"
            );
        }
    }
}
