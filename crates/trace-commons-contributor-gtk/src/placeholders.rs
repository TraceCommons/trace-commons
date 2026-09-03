//! Naming the redaction marks the transcript already draws.
//!
//! **The marks are not new.** `transcript_paging::marker_spans` has found
//! them since the transcript pane was written, `ui::preview::highlight_
//! redactions` washes each one in the gold tone, and the chunker uses the
//! same scanner so a marker is never cut in half. Nothing here re-scans, and
//! nothing here restyles: this module reads ONE marker's text and says what
//! it names, which is the part a wash cannot carry.
//!
//! The scrubber leaves three marker forms, and they carry different amounts
//! of information. Verified in `trace-commons-protocol`'s
//! `trace_contribution.rs`:
//!
//! * `<PRIVATE_LOCAL_PATH_1>` -- a numbered placeholder, minted by
//!   `apply_placeholder_regex` for exactly two labels, `local_path` and
//!   `private_email`. One token per DISTINCT value, reused wherever that
//!   value recurs, so the same ordinal twice is the same original string
//!   twice. This is the only form carrying an ordinal.
//! * `[REDACTED:private_email]` -- carries a label and no ordinal.
//!   `privacy_filter:{label}`, `tool_sensitive_field` and
//!   `tool_sensitive_field:{action}` all land here.
//! * `[REDACTED]` -- carries neither. Plain `secret`, `secret:{pattern}`
//!   and `sensitive_field` land here.
//!
//! **Secrets mint no numbered placeholder.** An ordinal is never invented
//! for a form that does not carry one, and a label is never guessed: an
//! unnamed mark says only that something was removed, because that is all
//! the marker says.
//!
//! What none of this may be allowed to imply: a region with no mark is not a
//! region with nothing sensitive in it. The detector scans every leaf and the
//! rewriter reaches only typed fields. Naming the marks makes the app look
//! more thorough than it is, which is exactly when the scrubbing caveat
//! earns its place beside them.

/// One redaction mark, and what its marker text names.
pub struct Placeholder {
    /// BYTE offsets into the body this was scanned from. A transcript is
    /// full of multi-byte text and a char index would slice it wrongly.
    pub start: usize,
    pub end: usize,
    /// The label as the marker spelled it -- `LOCAL_PATH` from a numbered
    /// placeholder, `private_email` from a labelled `[REDACTED:...]`.
    /// `None` for a bare `[REDACTED]`, which names no category at all.
    pub label: Option<String>,
    /// Which distinct value of that label this is.
    ///
    /// `None` for every form but the numbered placeholder. Explicit rather
    /// than a zero: a magic 0 would be indistinguishable from a real first
    /// value, and "the same value as an earlier mark" is a claim this must
    /// only make where the marker actually supports it.
    pub ordinal: Option<u32>,
}

/// The label as a person reads it. Handles both cases the two forms use --
/// `LOCAL_PATH` from a placeholder, `private_email` from a labelled token.
///
/// `URL_PATH` is spelled out rather than lowercased: the label is an
/// initialism and "url path" reads as a typo. It is the one word in this
/// vocabulary where case carries meaning, so it is the one exception, not
/// the start of a table.
pub fn display(label: &str) -> String {
    if label == "URL_PATH" {
        return "URL path".to_string();
    }
    label.to_lowercase().replace('_', " ")
}

/// What one marker's exact text names.
///
/// Deliberately strict about the numbered form: an uppercase label of
/// letters, digits and underscores ending on a non-underscore, then the
/// ordinal. Anything else is still a mark -- the shared scanner said so --
/// but an unnamed one. Reporting a guessed label or a guessed ordinal would
/// be a lie about what the scrubber did.
fn classify(marker: &str) -> (Option<String>, Option<u32>) {
    if let Some(inner) = marker
        .strip_prefix("<PRIVATE_")
        .and_then(|rest| rest.strip_suffix('>'))
    {
        let Some(split) = inner.rfind('_') else {
            return (None, None);
        };
        let (label, ordinal) = (&inner[..split], &inner[split + 1..]);
        let named = !label.is_empty()
            && !label.ends_with('_')
            && label
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
        let Ok(ordinal) = ordinal.parse::<u32>() else {
            return (None, None);
        };
        return if named {
            (Some(label.to_string()), Some(ordinal))
        } else {
            (None, None)
        };
    }
    // Two fixed tokens that name a category without the `:` form. Both are
    // in `every_fixed_token_the_pipeline_emits_is_matched`'s list, so both
    // reach here, and without these arms they fell through to "something
    // removed" -- a mark that says less than its own text does.
    if marker == "<REDACTED_PRIVATE_KEY>" {
        return (Some("PRIVATE_KEY".to_string()), None);
    }
    if marker == "[REDACTED_PATH]" {
        // A URL's path component, not a path on this machine. Naming it a
        // local path would point a reader at the wrong thing entirely.
        return (Some("URL_PATH".to_string()), None);
    }
    if let Some(inner) = marker
        .strip_prefix("[REDACTED")
        .and_then(|rest| rest.strip_suffix(']'))
    {
        // `[REDACTED:label]` names a category; bare `[REDACTED]` does not.
        return match inner.strip_prefix(':') {
            Some(label) if !label.is_empty() => (Some(label.to_string()), None),
            _ => (None, None),
        };
    }
    (None, None)
}

/// Every redaction mark in `body`, left to right, with what each one names.
///
/// The spans come from `transcript_paging::marker_spans` -- the SAME scanner
/// the chunker and the existing highlighting use. There is deliberately no
/// second pass: a set of marks this module named but the chunker did not
/// protect would be exactly the ones a chunk boundary cut in half.
pub fn scan(body: &str) -> Vec<Placeholder> {
    crate::transcript_paging::marker_spans(body)
        .into_iter()
        .map(|span| {
            let (label, ordinal) = classify(&body[span.clone()]);
            Placeholder {
                start: span.start,
                end: span.end,
                label,
                ordinal,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_body_with_no_marks_scans_to_nothing() {
        assert!(scan("just some ordinary text").is_empty());
        assert!(scan("").is_empty());
    }

    #[test]
    fn a_numbered_placeholder_carries_a_label_and_an_ordinal() {
        let body = "ran the build in <PRIVATE_LOCAL_PATH_1> and stopped";
        let found = scan(body);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].label.as_deref(), Some("LOCAL_PATH"));
        assert_eq!(found[0].ordinal, Some(1));
        assert_eq!(
            &body[found[0].start..found[0].end],
            "<PRIVATE_LOCAL_PATH_1>"
        );
    }

    #[test]
    fn the_display_name_is_human_readable_in_either_case() {
        assert_eq!(display("CONTEXTUAL_ENTROPY"), "contextual entropy");
        assert_eq!(display("private_email"), "private email");
    }

    /// A labelled `[REDACTED:...]` names its category and carries no
    /// ordinal. `privacy_filter:{label}` and `tool_sensitive_field` arrive
    /// in this form.
    #[test]
    fn a_labelled_redacted_token_names_its_category_and_has_no_ordinal() {
        let found = scan("wrote to <PRIVATE_LOCAL_PATH_2> for [REDACTED:private_email]");
        assert_eq!(found.len(), 2);
        assert_eq!(found[1].label.as_deref(), Some("private_email"));
        assert_eq!(found[1].ordinal, None, "this form carries no ordinal");
    }

    /// The form plain secrets land in. It names nothing, and nothing may be
    /// invented for it.
    #[test]
    fn a_bare_redacted_token_names_nothing() {
        let found = scan("the key was [REDACTED] after that");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].label, None);
        assert_eq!(found[0].ordinal, None);
    }

    /// All three of the scanner's shapes at once, in document order, with
    /// byte offsets that slice back to exactly the marker text.
    #[test]
    fn every_form_is_found_in_order_at_correct_byte_offsets() {
        let body = "a <PRIVATE_LOCAL_PATH_1> b [REDACTED:person_name] c [REDACTED] d";
        let found = scan(body);
        assert_eq!(found.len(), 3);
        assert_eq!(
            found
                .iter()
                .map(|p| &body[p.start..p.end])
                .collect::<Vec<_>>(),
            [
                "<PRIVATE_LOCAL_PATH_1>",
                "[REDACTED:person_name]",
                "[REDACTED]"
            ]
        );
        assert_eq!(
            found
                .iter()
                .map(|p| (p.label.clone(), p.ordinal))
                .collect::<Vec<_>>(),
            [
                (Some("LOCAL_PATH".to_string()), Some(1)),
                (Some("person_name".to_string()), None),
                (None, None),
            ]
        );
    }

    /// The ordinal is the last underscore-delimited run of digits, so a
    /// label that itself ends in a number must not steal it.
    #[test]
    fn a_label_containing_digits_is_parsed_correctly() {
        let found = scan("<PRIVATE_SHA256_KEY_7>");
        assert_eq!(found[0].label.as_deref(), Some("SHA256_KEY"));
        assert_eq!(found[0].ordinal, Some(7));
    }

    /// The same distinct value, twice. The redactor mints one token per
    /// value and reuses it, so this is what lets a mark say "the same
    /// original string as that earlier one".
    #[test]
    fn a_repeated_ordinal_is_the_same_original_value() {
        let found = scan("<PRIVATE_SECRET_1> then <PRIVATE_LOCAL_PATH_3> then <PRIVATE_SECRET_1>");
        assert_eq!(
            found.iter().map(|p| p.ordinal).collect::<Vec<_>>(),
            [Some(1), Some(3), Some(1)]
        );
        assert_eq!(found[0].label, found[2].label);
    }

    /// A shape the shared scanner accepts as a marker but that carries no
    /// parseable ordinal. It is still a mark; it is simply an unnamed one.
    /// Inventing an ordinal here would claim a value identity that the
    /// marker does not assert.
    #[test]
    fn a_marker_with_no_parseable_ordinal_is_unnamed_never_invented() {
        let found = scan("<PRIVATE_LOCAL_PATH_>");
        assert_eq!(found.len(), 1, "the shared scanner treats this as a marker");
        assert_eq!(found[0].label, None);
        assert_eq!(found[0].ordinal, None);
    }

    #[test]
    fn text_that_is_not_a_marker_at_all_is_ignored() {
        assert!(scan("<PRIVATE>").is_empty());
        assert!(scan("<private_local_path_1>").is_empty());
        assert!(scan("PRIVATE_LOCAL_PATH_1").is_empty());
    }

    /// **A known gap, pinned rather than papered over.** PEM private keys
    /// are replaced with the fixed token `<REDACTED_PRIVATE_KEY>`
    /// (`trace_contribution.rs`), which begins `<REDACTED_` -- matching
    /// neither of the shared scanner's two prefixes. So that one form is not
    /// marked in the transcript at all, on any of the three shells.
    ///
    /// Not fixed here: `marker_spans` is shared with the CHUNKER, and
    /// widening it changes where a body may be cut.
    ///
    /// Deliberately asserted against the shared scanner rather than against
    /// a hardcoded "empty": whether this token is marked is `marker_spans`'
    /// answer to give, and it changes the day that pattern is widened. What
    /// this pins is that `classify` NAMES it either way, so the widening is
    /// a one-line change over there and never a silently anonymous chip
    /// over here.
    #[test]
    fn a_pem_private_key_token_is_named_whether_or_not_the_scanner_marks_it() {
        assert_eq!(
            classify("<REDACTED_PRIVATE_KEY>").0.as_deref(),
            Some("PRIVATE_KEY")
        );
        let body = "key: <REDACTED_PRIVATE_KEY> done";
        assert_eq!(
            scan(body).len(),
            crate::transcript_paging::marker_spans(body).len()
        );
    }

    /// Every fixed token the pipeline emits is NAMED.
    ///
    /// The same five `transcript_paging`'s
    /// `every_fixed_token_the_pipeline_emits_is_matched` enumerates, from
    /// the same sources: `apply_redaction_ranges`,
    /// `apply_pem_block_redaction`, `redacted_marker`, and `redaction.rs`'s
    /// `REDACTED`. That guard proves the scanner FINDS them; this one
    /// proves the naming has words for them. A token that is found and then
    /// drawn as an anonymous block is the defect this pair exists to catch,
    /// and neither test catches it alone.
    ///
    /// None carries an ordinal. There is no second number in any of these
    /// tokens to report, and inventing one would claim a distinctness the
    /// token does not have.
    #[test]
    fn every_fixed_token_the_pipeline_emits_is_named() {
        for (token, expected) in [
            ("[REDACTED]", None),
            ("[REDACTED:aws_secret_key]", Some("aws secret key")),
            ("[REDACTED:person_name]", Some("person name")),
            ("[REDACTED_PATH]", Some("URL path")),
            ("<REDACTED_PRIVATE_KEY>", Some("private key")),
        ] {
            let (label, ordinal) = classify(token);
            assert_eq!(
                label.as_deref().map(display),
                expected.map(str::to_string),
                "{token}"
            );
            assert_eq!(ordinal, None, "{token} carries no ordinal");
        }
    }

    /// The naming is driven by the SHARED scan, not by a private grammar of
    /// its own.
    ///
    /// This is the guard on the thing the spec forbids: a second marker pass
    /// that could disagree with the one the chunker splits on. Whatever
    /// `marker_spans` matches in a body of all five fixed tokens is exactly
    /// what comes back here -- so a SIXTH token added to the pipeline and to
    /// that pattern is picked up with no edit, and cannot silently go
    /// unnamed. Deliberately not a hardcoded count: the pattern is the thing
    /// under test, so asserting a number here would just assert a copy of it.
    #[test]
    fn the_named_marks_are_exactly_what_the_shared_scan_found() {
        let body = "<PRIVATE_LOCAL_PATH_1> [REDACTED] [REDACTED:aws_secret_key] \
                    [REDACTED_PATH] <REDACTED_PRIVATE_KEY>";
        let spans = crate::transcript_paging::marker_spans(body);
        assert!(
            !spans.is_empty(),
            "the fixture must contain markers for this to prove anything"
        );
        let found = scan(body);
        assert_eq!(
            found.iter().map(|p| (p.start, p.end)).collect::<Vec<_>>(),
            spans.iter().map(|s| (s.start, s.end)).collect::<Vec<_>>()
        );
    }

    /// Offsets are BYTE offsets into the body, and a transcript is full of
    /// multi-byte text. Slicing at a char index would panic or cut a
    /// codepoint in half.
    #[test]
    fn offsets_are_byte_offsets_and_survive_multibyte_text() {
        let body = "héllo [REDACTED:person_name] wörld";
        let found = scan(body);
        assert_eq!(
            &body[found[0].start..found[0].end],
            "[REDACTED:person_name]"
        );
    }
}
