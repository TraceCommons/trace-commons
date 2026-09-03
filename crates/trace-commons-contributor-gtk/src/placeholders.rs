//! Finding the redactor's placeholders in a preview body.
//!
//! `DeterministicTraceRedactor` does not delete a matched value -- it
//! substitutes `<PRIVATE_<LABEL>_<n>>`, one token per distinct value, reused
//! wherever that value recurs. Those tokens have always been in the bytes
//! the daemon returns; this shell just rendered them as ordinary transcript
//! text and the contributor scrolled past them.
//!
//! Marking them is the whole of "show me what got removed", and it beats a
//! list because it also answers *where*. No new protocol field, no new
//! content across any boundary: the token is already what is on screen.
//!
//! What it must not be allowed to imply: a region with no placeholder is not
//! a region with nothing sensitive in it. The detector scans every leaf and
//! the rewriter reaches only typed fields, so highlighting makes the app look
//! more thorough than it is. `copy::SCRUBBING_CAVEAT` is what says so, and it
//! belongs beside these marks.

/// One place the redactor removed a value.
pub struct Placeholder {
    /// BYTE offsets into the body this was scanned from. A transcript is
    /// full of multi-byte text and a char index would slice it wrongly.
    pub start: usize,
    pub end: usize,
    /// The raw label as the redactor spelled it: `LOCAL_PATH`, `SECRET`.
    pub label: String,
    /// Which distinct value of that label this is. The redactor mints one
    /// placeholder per value, so the same ordinal twice is the same original
    /// string twice.
    pub ordinal: u32,
}

/// The label as a person reads it.
pub fn display(label: &str) -> String {
    label.to_lowercase().replace('_', " ")
}

/// Every placeholder in `body`, left to right.
///
/// Written as a hand-rolled scan rather than a regex because this crate has
/// no regex dependency and adding one for eight lines is not a trade worth
/// making. Deliberately strict about the shape: an uppercase label of
/// letters, digits and underscores ending on a non-underscore, then the
/// ordinal. A transcript can contain prose that looks approximately like a
/// token, and marking a contributor's own sentence as a redaction would be a
/// lie about what the scrubber did.
pub fn scan(body: &str) -> Vec<Placeholder> {
    const OPEN: &str = "<PRIVATE_";
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(rel) = body[from..].find(OPEN) {
        let start = from + rel;
        let after = start + OPEN.len();
        let Some(close_rel) = body[after..].find('>') else {
            break;
        };
        let close = after + close_rel;
        let inner = &body[after..close];
        from = close + 1;

        let Some(split) = inner.rfind('_') else {
            continue;
        };
        let (label, ordinal) = (&inner[..split], &inner[split + 1..]);
        if label.is_empty() || ordinal.is_empty() {
            continue;
        }
        if !label
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        {
            continue;
        }
        if label.ends_with('_') {
            continue;
        }
        let Ok(ordinal) = ordinal.parse::<u32>() else {
            continue;
        };
        out.push(Placeholder {
            start,
            end: close + 1,
            label: label.to_string(),
            ordinal,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_body_with_no_placeholders_scans_to_nothing() {
        assert!(scan("just some ordinary text").is_empty());
        assert!(scan("").is_empty());
    }

    #[test]
    fn a_single_placeholder_is_found() {
        let body = "ran the build in <PRIVATE_LOCAL_PATH_1> and stopped";
        let found = scan(body);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].label, "LOCAL_PATH");
        assert_eq!(found[0].ordinal, 1);
        assert_eq!(
            &body[found[0].start..found[0].end],
            "<PRIVATE_LOCAL_PATH_1>"
        );
    }

    #[test]
    fn the_display_name_is_human_readable() {
        assert_eq!(display("CONTEXTUAL_ENTROPY"), "contextual entropy");
    }

    #[test]
    fn multiple_placeholders_are_found_in_order() {
        let found = scan("<PRIVATE_SECRET_1> then <PRIVATE_LOCAL_PATH_3> then <PRIVATE_SECRET_1>");
        assert_eq!(
            found.iter().map(|p| p.label.as_str()).collect::<Vec<_>>(),
            ["SECRET", "LOCAL_PATH", "SECRET"]
        );
        assert_eq!(
            found.iter().map(|p| p.ordinal).collect::<Vec<_>>(),
            [1, 3, 1]
        );
    }

    /// The ordinal is the last underscore-delimited run of digits, so a
    /// label that itself ends in a number must not steal it.
    #[test]
    fn a_label_containing_digits_is_parsed_correctly() {
        let found = scan("<PRIVATE_SHA256_KEY_7>");
        assert_eq!(found[0].label, "SHA256_KEY");
        assert_eq!(found[0].ordinal, 7);
    }

    #[test]
    fn text_that_merely_looks_like_a_placeholder_is_ignored() {
        assert!(scan("<PRIVATE>").is_empty());
        assert!(scan("<PRIVATE_LOCAL_PATH_>").is_empty());
        assert!(scan("<private_local_path_1>").is_empty());
        assert!(scan("PRIVATE_LOCAL_PATH_1").is_empty());
    }

    /// Offsets are BYTE offsets into the body, and a transcript is full of
    /// multi-byte text. Slicing at a char index would panic or cut a
    /// codepoint in half.
    #[test]
    fn offsets_are_byte_offsets_and_survive_multibyte_text() {
        let body = "héllo <PRIVATE_SECRET_1> wörld";
        let found = scan(body);
        assert_eq!(&body[found[0].start..found[0].end], "<PRIVATE_SECRET_1>");
    }
}
