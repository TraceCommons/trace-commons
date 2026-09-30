//! Unsure spans: places in the REDACTED preview body that look like
//! personal data the scrubber did not mark.
//!
//! The review sheet in the Flow 2 design puts a hint under such a line --
//! "looks like an email. Not matched. Your call." -- so the contributor's
//! eye goes where pattern-based scrubbing is least trustworthy. This module
//! finds those places. It never changes a byte of the body and it redacts
//! nothing: a hint is a pointer for a person, not a second scrubber, and
//! the decision stays theirs.
//!
//! # What crosses the boundary
//!
//! Offsets and a fixed label, never the text. The shell already holds the
//! body (`preview_body`), so it can render the hinted bytes itself; the
//! daemon repeating them would put trace content on a second channel for no
//! gain. The offsets index **exactly** the string `preview::body_of` returns
//! for the envelope, the same string `preview_body` pages and
//! `preview_turns` indexes, and the IPC method serving them is anchored by
//! `body_digest` for the same reason `preview_turns` is: against any other
//! string an offset is not stale but wrong.
//!
//! # Conservative, on purpose
//!
//! Every detector here prefers a miss to a false alarm. A hint that fires on
//! every timestamp teaches a contributor to ignore hints, and an ignored
//! hint is worse than none. So:
//!
//! - only the contents of JSON strings are scanned, never keys' structure,
//!   numbers or punctuation;
//! - a candidate never spans a JSON escape sequence (`\n`, `\"`, `\u00e9`):
//!   each string is split at its escapes and each run scanned on its own, so
//!   an offset can never point into the middle of one, and a value broken
//!   up by an escape is simply not hinted;
//! - redaction placeholders (`<PRIVATE_EMAIL_1>`, `[REDACTED]`) cannot match,
//!   because the characters that delimit them are outside every detector's
//!   alphabet -- which is what makes "a matched email yields no hint" hold.
//!
//! # Fail-closed
//!
//! If the body is not the document this scanner was written for (an
//! unterminated string, a stray control byte), or if any span fails to
//! re-verify against the exact bytes it points at, the whole result is
//! refused with [`REASON_UNSURE_INDEX_FAILED`]. A hint drawn over the wrong
//! text is worse than no hint, and a partial list presented as the whole
//! one would read as "the rest is fine".

use serde::Serialize;

/// An email address, or an obfuscated one (`name [at] example [dot] com`).
pub const LABEL_LOOKS_LIKE_EMAIL: &str = "looks-like-email";
/// A phone number in international (`+44 20 7946 0958`) or North American
/// (`(415) 555-0100`, `415-555-0100`) form.
pub const LABEL_LOOKS_LIKE_PHONE: &str = "looks-like-phone";
/// A string shaped like a well-known credential (`sk-…`, `ghp_…`, `AKIA…`).
pub const LABEL_LOOKS_LIKE_KEY: &str = "looks-like-key";

/// Every label a span can carry. Closed: a shell may hold one sentence per
/// label (see `preview_copy::unsure_hint_line`).
pub const UNSURE_LABELS: &[&str] = &[
    LABEL_LOOKS_LIKE_EMAIL,
    LABEL_LOOKS_LIKE_PHONE,
    LABEL_LOOKS_LIKE_KEY,
];

/// The fixed label for a body this module could not index exactly.
pub const REASON_UNSURE_INDEX_FAILED: &str = "preview-unsure-index-failed";

/// The most spans one response carries. A span serializes to about 60
/// bytes, so this keeps the response well under the 1 MiB line cap. The
/// total is always reported beside the list, and the list is flagged when it
/// was cut, so a shell never presents a partial list as the whole one.
pub const MAX_UNSURE_SPANS: usize = 2000;

/// One hint: a half-open range of UTF-8 byte offsets into the preview body,
/// on character boundaries, and what it looks like.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnsureSpan {
    pub label: &'static str,
    pub byte_offset: usize,
    pub byte_len: usize,
}

/// The unsure spans in `body`, in body order, never overlapping.
///
/// `body` must be the exact string `preview::body_of` returned; every
/// offset indexes it. Errors are [`REASON_UNSURE_INDEX_FAILED`], fixed and
/// content-free.
pub fn unsure_spans_in(body: &str) -> Result<Vec<UnsureSpan>, &'static str> {
    let mut spans = Vec::new();
    for (start, end) in string_runs(body).ok_or(REASON_UNSURE_INDEX_FAILED)? {
        scan_run(body, start, end, &mut spans);
    }
    spans.sort_by(|a, b| {
        a.byte_offset
            .cmp(&b.byte_offset)
            .then(b.byte_len.cmp(&a.byte_len))
    });
    let mut kept: Vec<UnsureSpan> = Vec::with_capacity(spans.len());
    for span in spans {
        if kept
            .last()
            .is_some_and(|last| span.byte_offset < last.byte_offset + last.byte_len)
        {
            continue;
        }
        kept.push(span);
    }
    for span in &kept {
        verify(body, span)?;
    }
    Ok(kept)
}

/// Re-check one span against the exact bytes it points at: on character
/// boundaries, inside the body, free of JSON escapes and quotes, and matched
/// whole by the detector its label names. Anything else refuses the result.
fn verify(body: &str, span: &UnsureSpan) -> Result<(), &'static str> {
    let end = span
        .byte_offset
        .checked_add(span.byte_len)
        .ok_or(REASON_UNSURE_INDEX_FAILED)?;
    let slice = body
        .get(span.byte_offset..end)
        .ok_or(REASON_UNSURE_INDEX_FAILED)?;
    if slice.is_empty() || slice.contains(['\\', '"']) {
        return Err(REASON_UNSURE_INDEX_FAILED);
    }
    let mut again = Vec::new();
    scan_run(slice, 0, slice.len(), &mut again);
    let whole = UnsureSpan {
        label: span.label,
        byte_offset: 0,
        byte_len: slice.len(),
    };
    if !again.contains(&whole) {
        return Err(REASON_UNSURE_INDEX_FAILED);
    }
    Ok(())
}

/// The escape-free runs inside every JSON string literal in `body`, as
/// half-open byte ranges. `None` for a document with an unterminated string
/// or a raw control byte inside one -- neither of which `serde_json` emits,
/// and both of which mean this is not the body the offsets would index.
fn string_runs(body: &str) -> Option<Vec<(usize, usize)>> {
    let bytes = body.as_bytes();
    let mut runs = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'"' {
            i += 1;
            continue;
        }
        // Inside a string literal.
        i += 1;
        let mut run_start = i;
        loop {
            let c = *bytes.get(i)?;
            match c {
                b'"' => {
                    if run_start < i {
                        runs.push((run_start, i));
                    }
                    i += 1;
                    break;
                }
                b'\\' => {
                    if run_start < i {
                        runs.push((run_start, i));
                    }
                    let escape = *bytes.get(i + 1)?;
                    i += if escape == b'u' { 6 } else { 2 };
                    if i > bytes.len() {
                        return None;
                    }
                    run_start = i;
                }
                c if c < 0x20 => return None,
                _ => i += 1,
            }
        }
    }
    Some(runs)
}

/// Every detector, over `body[start..end]`, which is one escape-free run.
fn scan_run(body: &str, start: usize, end: usize, out: &mut Vec<UnsureSpan>) {
    let run = &body.as_bytes()[start..end];
    let mut push = |label: &'static str, s: usize, e: usize| {
        out.push(UnsureSpan {
            label,
            byte_offset: start + s,
            byte_len: e - s,
        });
    };
    for (s, e) in emails(run) {
        push(LABEL_LOOKS_LIKE_EMAIL, s, e);
    }
    for (s, e) in phones(run) {
        push(LABEL_LOOKS_LIKE_PHONE, s, e);
    }
    for (s, e) in keys(run) {
        push(LABEL_LOOKS_LIKE_KEY, s, e);
    }
}

fn is_local(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'%' | b'+' | b'-')
}

fn is_label(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'-'
}

/// `[at]`, `(at)` or `{at}` (any case, optional single spaces inside and
/// around) starting at `i`; returns the index just past it.
fn obfuscated_word(run: &[u8], i: usize, word: &[u8]) -> Option<usize> {
    let mut j = i;
    if run.get(j) == Some(&b' ') {
        j += 1;
    }
    let close = match run.get(j)? {
        b'[' => b']',
        b'(' => b')',
        b'{' => b'}',
        _ => return None,
    };
    j += 1;
    if run.get(j) == Some(&b' ') {
        j += 1;
    }
    let found = run.get(j..j + word.len())?;
    if !found.eq_ignore_ascii_case(word) {
        return None;
    }
    j += word.len();
    if run.get(j) == Some(&b' ') {
        j += 1;
    }
    if run.get(j) != Some(&close) {
        return None;
    }
    j += 1;
    if run.get(j) == Some(&b' ') {
        j += 1;
    }
    Some(j)
}

/// A domain starting at `i`: labels separated by `.` or an obfuscated
/// `[dot]`, at least two labels, the last all letters and at least two long.
/// Returns the end, or `None`.
fn domain_at(run: &[u8], i: usize, allow_obfuscated_dot: bool) -> Option<usize> {
    let mut labels = 0usize;
    let mut j = i;
    let mut end: Option<usize>;
    loop {
        let label_start = j;
        while j < run.len() && is_label(run[j]) {
            j += 1;
        }
        if j == label_start || run[label_start] == b'-' || run[j - 1] == b'-' {
            // A domain that ends in a malformed label is not one.
            return None;
        }
        labels += 1;
        let tld = &run[label_start..j];
        end =
            (labels >= 2 && tld.len() >= 2 && tld.iter().all(u8::is_ascii_alphabetic)).then_some(j);
        // A separator must be followed by another label to count.
        if run.get(j) == Some(&b'.') && run.get(j + 1).is_some_and(|c| is_label(*c)) {
            j += 1;
            continue;
        }
        if allow_obfuscated_dot {
            if let Some(next) = obfuscated_word(run, j, b"dot") {
                if run.get(next).is_some_and(|c| is_label(*c)) {
                    j = next;
                    continue;
                }
            }
        }
        break;
    }
    // Nothing that continues a domain may follow the match.
    let end = end?;
    if run.get(end).is_some_and(|c| is_label(*c)) {
        return None;
    }
    Some(end)
}

/// Email addresses and bracket-obfuscated ones, as `(start, end)` in `run`.
fn emails(run: &[u8]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < run.len() {
        // The local part: a maximal run, so the start is a boundary.
        if !is_local(run[i]) || (i > 0 && is_local(run[i - 1])) {
            i += 1;
            continue;
        }
        let local_start = i;
        let mut j = i;
        while j < run.len() && is_local(run[j]) {
            j += 1;
        }
        let local_end = j;
        // A local part must start and end on an alphanumeric: `.x@` and
        // `x.@` are not addresses, and it keeps `--foo@` out.
        let local = &run[local_start..local_end];
        let well_formed_local = local.first().is_some_and(u8::is_ascii_alphanumeric)
            && local.last().is_some_and(u8::is_ascii_alphanumeric);
        let found = if !well_formed_local {
            None
        } else if run.get(local_end) == Some(&b'@') {
            domain_at(run, local_end + 1, false)
        } else {
            obfuscated_word(run, local_end, b"at").and_then(|d| domain_at(run, d, true))
        };
        if let Some(end) = found {
            out.push((local_start, end));
            i = end;
        } else {
            i = local_end.max(i + 1);
        }
    }
    out
}

fn digits_in(bytes: &[u8]) -> usize {
    bytes.iter().filter(|c| c.is_ascii_digit()).count()
}

/// Whether the byte before `start` and the byte at `end` leave the match
/// standing alone: not glued to a letter, digit, `.`, `+`, `-`, `_` or `/`
/// (a version string, a decimal, a path, a range).
fn stands_alone(run: &[u8], start: usize, end: usize) -> bool {
    let glue = |c: u8| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'+' | b'-' | b'_' | b'/');
    let before = start == 0 || !glue(run[start - 1]);
    let after = end >= run.len() || !glue(run[end]);
    before && after
}

/// Phone numbers, as `(start, end)` in `run`. Two shapes only:
///
/// - international: `+`, then 8 to 15 digits in groups separated by single
///   spaces, dots or dashes, with an optional parenthesised area code;
/// - North American: `(ddd) ddd-dddd`, `ddd-ddd-dddd` or `ddd.ddd.dddd`.
///
/// A bare run of digits is never a phone here: it is far more often an id,
/// a count or a timestamp.
fn phones(run: &[u8]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < run.len() {
        let found = if run[i] == b'+' {
            international_at(run, i)
        } else if run[i] == b'(' || run[i].is_ascii_digit() {
            north_american_at(run, i)
        } else {
            None
        };
        match found {
            Some(end) if stands_alone(run, i, end) => {
                out.push((i, end));
                i = end;
            }
            _ => i += 1,
        }
    }
    out
}

fn international_at(run: &[u8], start: usize) -> Option<usize> {
    let mut j = start + 1;
    let mut end = None;
    loop {
        let paren = run.get(j) == Some(&b'(');
        if paren {
            j += 1;
        }
        let d = j;
        while j < run.len() && run[j].is_ascii_digit() {
            j += 1;
        }
        if j == d {
            break;
        }
        if paren {
            if run.get(j) != Some(&b')') {
                break;
            }
            j += 1;
        }
        let digits = digits_in(&run[start..j]);
        if digits > 15 {
            return None;
        }
        if digits >= 8 {
            end = Some(j);
        }
        match run.get(j) {
            Some(b' ' | b'.' | b'-')
                if run
                    .get(j + 1)
                    .is_some_and(|c| c.is_ascii_digit() || *c == b'(') =>
            {
                j += 1;
            }
            _ => break,
        }
    }
    // One unbroken group of digits after `+` is E.164; more groups are the
    // human spelling. Either way the digit count decided `end`.
    end
}

fn exact_digits(run: &[u8], at: usize, n: usize) -> Option<usize> {
    let slice = run.get(at..at + n)?;
    slice.iter().all(u8::is_ascii_digit).then_some(at + n)
}

fn north_american_at(run: &[u8], start: usize) -> Option<usize> {
    if run[start] == b'(' {
        let j = exact_digits(run, start + 1, 3)?;
        if run.get(j) != Some(&b')') {
            return None;
        }
        let mut j = j + 1;
        if run.get(j) == Some(&b' ') {
            j += 1;
        }
        let j = exact_digits(run, j, 3)?;
        if !matches!(run.get(j), Some(b'-' | b'.' | b' ')) {
            return None;
        }
        return exact_digits(run, j + 1, 4);
    }
    let j = exact_digits(run, start, 3)?;
    let sep = *run.get(j)?;
    if !matches!(sep, b'-' | b'.') {
        return None;
    }
    let j = exact_digits(run, j + 1, 3)?;
    if run.get(j) != Some(&sep) {
        return None;
    }
    exact_digits(run, j + 1, 4)
}

/// Well-known credential prefixes and the fewest body characters after the
/// prefix for a match. The scrubber's own secret patterns cover these; a
/// hint here is a survivor, which is exactly what a person should look at.
const KEY_PREFIXES: &[(&str, usize)] = &[
    ("sk-", 20),
    ("sk_live_", 16),
    ("sk_test_", 16),
    ("rk_live_", 16),
    ("ghp_", 30),
    ("gho_", 30),
    ("ghu_", 30),
    ("ghs_", 30),
    ("ghr_", 30),
    ("github_pat_", 30),
    ("glpat-", 20),
    ("xoxb-", 10),
    ("xoxp-", 10),
    ("xoxa-", 10),
    ("xoxr-", 10),
    ("xoxs-", 10),
    ("AIza", 35),
];

fn is_key_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-')
}

/// Credential-shaped strings, as `(start, end)` in `run`.
fn keys(run: &[u8]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < run.len() {
        if i > 0 && is_key_char(run[i - 1]) {
            i += 1;
            continue;
        }
        let mut found = None;
        for (prefix, min) in KEY_PREFIXES {
            let p = prefix.as_bytes();
            if run.get(i..i + p.len()) != Some(p) {
                continue;
            }
            let mut j = i + p.len();
            while j < run.len() && is_key_char(run[j]) {
                j += 1;
            }
            if j - (i + p.len()) >= *min {
                found = Some(j);
                break;
            }
        }
        // AWS access key id: `AKIA` and exactly sixteen upper-case
        // alphanumerics, standing alone.
        if found.is_none() && run.get(i..i + 4) == Some(b"AKIA") {
            let body = run.get(i + 4..i + 20);
            if body.is_some_and(|b| {
                b.iter()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            }) && !run.get(i + 20).is_some_and(|c| is_key_char(*c))
            {
                found = Some(i + 20);
            }
        }
        match found {
            Some(end) => {
                out.push((i, end));
                i = end;
            }
            None => i += 1,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-string body, as `body_of` would print it.
    fn body(text: &str) -> String {
        serde_json::to_string_pretty(&serde_json::json!([{ "redacted_content": text }])).unwrap()
    }

    fn texts(body: &str) -> Vec<(&'static str, &str)> {
        unsure_spans_in(body)
            .unwrap()
            .into_iter()
            .map(|s| (s.label, &body[s.byte_offset..s.byte_offset + s.byte_len]))
            .collect()
    }

    #[test]
    fn an_unmatched_email_is_hinted_at_its_exact_offsets() {
        let b = body("Use the staging key from ops@acme.io for the test.");
        let spans = unsure_spans_in(&b).unwrap();
        assert_eq!(spans.len(), 1, "{spans:?}");
        let at = b.find("ops@acme.io").unwrap();
        assert_eq!(
            spans[0],
            UnsureSpan {
                label: LABEL_LOOKS_LIKE_EMAIL,
                byte_offset: at,
                byte_len: "ops@acme.io".len(),
            }
        );
    }

    #[test]
    fn a_redacted_email_is_not_hinted() {
        // What the scrubber leaves where it matched an email.
        assert!(
            unsure_spans_in(&body("Use the key from <PRIVATE_EMAIL_1> for the test."))
                .unwrap()
                .is_empty()
        );
        assert!(
            unsure_spans_in(&body("token [REDACTED] and [REDACTED:private_email]"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn obfuscated_emails_are_hinted() {
        let b = body("write to ops [at] acme [dot] io or ops(at)acme.io");
        assert_eq!(
            texts(&b),
            vec![
                (LABEL_LOOKS_LIKE_EMAIL, "ops [at] acme [dot] io"),
                (LABEL_LOOKS_LIKE_EMAIL, "ops(at)acme.io"),
            ]
        );
    }

    #[test]
    fn near_misses_that_are_not_addresses_are_not_hinted() {
        for text in [
            "@decorator and @scope/pkg",
            "user@localhost",
            "see v1.2.3 and 2026-09-28",
            "a@b.c",
            "x.@acme.io",
        ] {
            assert!(
                unsure_spans_in(&body(text)).unwrap().is_empty(),
                "{text}: {:?}",
                texts(&body(text))
            );
        }
    }

    #[test]
    fn phones_in_two_shapes_are_hinted_and_bare_digits_are_not() {
        let b = body("call +44 20 7946 0958 or (415) 555-0100 or 415-555-0100");
        assert_eq!(
            texts(&b),
            vec![
                (LABEL_LOOKS_LIKE_PHONE, "+44 20 7946 0958"),
                (LABEL_LOOKS_LIKE_PHONE, "(415) 555-0100"),
                (LABEL_LOOKS_LIKE_PHONE, "415-555-0100"),
            ]
        );
        for text in [
            "id 4155550100 and 12345678",
            "2026-09-28T10:00:00Z",
            "10.0.0.1 and 192.168.100.1000",
            "@@ -12,7 +12,8 @@",
            "+1 point",
        ] {
            assert!(
                unsure_spans_in(&body(text)).unwrap().is_empty(),
                "{text}: {:?}",
                texts(&body(text))
            );
        }
    }

    #[test]
    fn credential_shapes_are_hinted() {
        let b = body("keys sk-abcdefghijklmnopqrstuvwx and AKIAIOSFODNN7EXAMPLE end");
        assert_eq!(
            texts(&b),
            vec![
                (LABEL_LOOKS_LIKE_KEY, "sk-abcdefghijklmnopqrstuvwx"),
                (LABEL_LOOKS_LIKE_KEY, "AKIAIOSFODNN7EXAMPLE"),
            ]
        );
        assert!(
            unsure_spans_in(&body("risk-free and sk-short"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_span_never_crosses_a_json_escape() {
        // `\n` is two bytes in the body. Scanning the raw body would take
        // the `n` into the local part and point one byte early.
        let b = body("line one\nops@acme.io");
        let spans = unsure_spans_in(&b).unwrap();
        assert_eq!(spans.len(), 1);
        assert_eq!(
            &b[spans[0].byte_offset..spans[0].byte_offset + spans[0].byte_len],
            "ops@acme.io"
        );
        // And an address broken by an escape is not guessed at.
        assert!(unsure_spans_in(&body("ops\"@acme.io")).unwrap().is_empty());
    }

    #[test]
    fn offsets_are_utf8_byte_offsets() {
        let b = body("caf\u{e9} \u{1F600} ops@acme.io");
        let spans = unsure_spans_in(&b).unwrap();
        assert_eq!(spans.len(), 1);
        assert_eq!(
            &b[spans[0].byte_offset..spans[0].byte_offset + spans[0].byte_len],
            "ops@acme.io"
        );
    }

    #[test]
    fn a_body_it_cannot_index_exactly_is_refused() {
        assert_eq!(
            unsure_spans_in("[\"unterminated ops@acme.io"),
            Err(REASON_UNSURE_INDEX_FAILED)
        );
        assert_eq!(
            unsure_spans_in("[\"raw\ncontrol ops@acme.io\"]"),
            Err(REASON_UNSURE_INDEX_FAILED)
        );
        assert_eq!(
            unsure_spans_in("[\"bad \\u12\"]"),
            Err(REASON_UNSURE_INDEX_FAILED)
        );
    }

    #[test]
    fn a_span_that_does_not_reverify_is_refused() {
        let b = body("ops@acme.io");
        let at = b.find("ops@acme.io").unwrap();
        let off_by_one = UnsureSpan {
            label: LABEL_LOOKS_LIKE_EMAIL,
            byte_offset: at + 1,
            byte_len: "ops@acme.io".len(),
        };
        assert_eq!(verify(&b, &off_by_one), Err(REASON_UNSURE_INDEX_FAILED));
        let wrong_label = UnsureSpan {
            label: LABEL_LOOKS_LIKE_PHONE,
            byte_offset: at,
            byte_len: "ops@acme.io".len(),
        };
        assert_eq!(verify(&b, &wrong_label), Err(REASON_UNSURE_INDEX_FAILED));
    }
}
