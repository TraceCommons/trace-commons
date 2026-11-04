//! The first run's Traces lines, worded by the commons.
//!
//! `POST /v1/public/first-run-copy` on `trace-commons-ingest` is
//! unauthenticated and outside tenant context, like
//! `/v1/public/first-run-estimate`: the first run asks it before the
//! contributor has an account or has consented to anything. The client sends
//! how many traces it found on the machine and how large they are, both
//! coarsened first ([`coarsen_count`], [`coarsen_bytes`]); the commons answers
//! with the lines to show under the count (owner, 2026-11-04: "the server
//! should send back what to say to the user").
//!
//! The request is a POST so the figures never sit in a URL, where a proxy's
//! access log would keep them. The server refuses a figure that is not
//! already coarse ([`is_coarse_count`], [`is_coarse_bytes`]) with a 400, so a
//! client that skipped the coarsening is caught rather than served.
//!
//! The answer is untrusted text on the client. Each line must pass
//! [`valid_line`]; a set with one bad line is refused whole, and the client
//! shows [`DEFAULT_LINES`] instead. The same lines are what the commons
//! answers by default, so a commons that says nothing new and a commons that
//! cannot be reached read the same.
//!
//! The response type deliberately does not use `deny_unknown_fields`, so a
//! field added later does not break clients already shipped.

use serde::{Deserialize, Serialize};

/// Ingest route.
pub const FIRST_RUN_COPY_PATH: &str = "/v1/public/first-run-copy";

/// The largest request body the server reads. A valid one is under 60
/// bytes; this bounds the read, it is not a policy value.
pub const FIRST_RUN_COPY_MAX_REQUEST_BYTES: usize = 1024;

/// The largest response a client reads: three lines of at most
/// [`FIRST_RUN_LINE_MAX_CHARS`] characters each, escaped, with room to spare.
pub const FIRST_RUN_COPY_MAX_RESPONSE_BYTES: usize = 8 * 1024;

/// The longest line, in characters, either end accepts.
pub const FIRST_RUN_LINE_MAX_CHARS: usize = 200;

/// The largest trace count either end accepts: a bound on a nonsense
/// figure, not a policy value.
pub const FIRST_RUN_MAX_TRACE_COUNT: u64 = 100_000_000;

/// The largest total size in MiB either end accepts (about 950 TiB): a
/// bound, not a policy value. Itself two significant figures, so a figure
/// clamped to it is still coarse.
pub const FIRST_RUN_MAX_TOTAL_MIB: u64 = 1_000_000_000;

/// [`FIRST_RUN_MAX_TOTAL_MIB`] in bytes.
pub const FIRST_RUN_MAX_TOTAL_BYTES: u64 = FIRST_RUN_MAX_TOTAL_MIB * MIB;

/// Bytes in a MiB, the unit the total size is coarsened to.
pub const MIB: u64 = 1 << 20;

/// Words a line may never contain, matched without regard to case: names
/// the product's copy keeps out of the first run.
pub const FORBIDDEN_WORDS: &[&str] = &["trace commons", "tracecommons", "daemon", "passkey"];

/// `POST /v1/public/first-run-copy` body. Both figures already coarse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FirstRunCopyRequest {
    /// The traces found, to two significant figures ([`coarsen_count`]).
    pub trace_count: u64,
    /// Their total size in bytes, as a whole number of MiB to two
    /// significant figures ([`coarsen_bytes`]).
    pub total_bytes: u64,
}

impl FirstRunCopyRequest {
    /// The request for the raw figures, coarsened.
    #[must_use]
    pub fn coarsened(trace_count: u64, total_bytes: u64) -> Self {
        Self {
            trace_count: coarsen_count(trace_count).min(FIRST_RUN_MAX_TRACE_COUNT),
            total_bytes: coarsen_bytes(total_bytes),
        }
    }

    /// Whether a server may read it: both figures coarse and in bounds.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.trace_count <= FIRST_RUN_MAX_TRACE_COUNT
            && self.total_bytes <= FIRST_RUN_MAX_TOTAL_BYTES
            && is_coarse_count(self.trace_count)
            && is_coarse_bytes(self.total_bytes)
    }
}

/// `POST /v1/public/first-run-copy` answer: the lines under the count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirstRunCopyLines {
    /// The line under the count while the use that earns credit is on.
    pub found_credit: String,
    /// The same while that use is off: what the traces could earn.
    pub found_could_credit: String,
    /// Under `found_credit`, smaller. Absent shows nothing there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_depends: Option<String>,
}

impl FirstRunCopyLines {
    /// Whether every line passes [`valid_line`]. One bad line refuses the
    /// whole set, so a client never shows a mix of the commons' lines and
    /// its own.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        valid_line(&self.found_credit)
            && valid_line(&self.found_could_credit)
            && self.credit_depends.as_deref().is_none_or(valid_line)
    }
}

/// The lines the commons answers by default and a client shows when the
/// commons cannot be reached or answers with lines it will not show. #1921's
/// words. DRAFT, NEEDS APPROVAL (2026-11-04).
pub const DEFAULT_FOUND_CREDIT: &str = "We have detected some traces that can earn you credit.";
/// DRAFT, NEEDS APPROVAL (2026-11-04). #1921's words.
pub const DEFAULT_FOUND_COULD_CREDIT: &str =
    "We have detected some traces that could earn you credit.";
/// DRAFT, NEEDS APPROVAL (2026-11-04). #1921's words.
pub const DEFAULT_CREDIT_DEPENDS: &str = "Final credit depends on how many traces are accepted.";

/// [`DEFAULT_FOUND_CREDIT`], [`DEFAULT_FOUND_COULD_CREDIT`] and
/// [`DEFAULT_CREDIT_DEPENDS`] as one answer.
#[must_use]
pub fn default_lines() -> FirstRunCopyLines {
    FirstRunCopyLines {
        found_credit: DEFAULT_FOUND_CREDIT.to_owned(),
        found_could_credit: DEFAULT_FOUND_COULD_CREDIT.to_owned(),
        credit_depends: Some(DEFAULT_CREDIT_DEPENDS.to_owned()),
    }
}

/// Whether a line is plain text a client may show: not blank, at most
/// [`FIRST_RUN_LINE_MAX_CHARS`] characters, no control character (a newline
/// included), no markup or placeholder character (`<`, `>`, `{`, `}`), and
/// none of [`FORBIDDEN_WORDS`].
#[must_use]
pub fn valid_line(line: &str) -> bool {
    if line.trim().is_empty() || line.chars().count() > FIRST_RUN_LINE_MAX_CHARS {
        return false;
    }
    if line
        .chars()
        .any(|c| c.is_control() || matches!(c, '<' | '>' | '{' | '}'))
    {
        return false;
    }
    let lower = line.to_lowercase();
    !FORBIDDEN_WORDS.iter().any(|word| lower.contains(word))
}

/// `value` to two significant figures, half up: 1234 is 1200, 1250 is 1300,
/// 99 is 99. Saturates rather than overflowing.
#[must_use]
pub fn coarsen_count(value: u64) -> u64 {
    if value < 100 {
        return value;
    }
    let mut scale = 1_u64;
    while value / scale >= 100 {
        scale = scale.saturating_mul(10);
    }
    let rounded = (value / scale) + u64::from(value % scale >= scale / 2);
    rounded.saturating_mul(scale)
}

/// Whether `value` has at most two significant figures.
#[must_use]
pub fn is_coarse_count(value: u64) -> bool {
    let mut v = value;
    while v >= 100 {
        if !v.is_multiple_of(10) {
            return false;
        }
        v /= 10;
    }
    true
}

/// `bytes` to whole MiB, half up, then that MiB count to two significant
/// figures, back in bytes. Under half a MiB is 0. Clamped to
/// [`FIRST_RUN_MAX_TOTAL_MIB`].
#[must_use]
pub fn coarsen_bytes(bytes: u64) -> u64 {
    let mib = bytes / MIB + u64::from(bytes % MIB >= MIB / 2);
    coarsen_count(mib).min(FIRST_RUN_MAX_TOTAL_MIB) * MIB
}

/// Whether `bytes` is a whole number of MiB whose count has at most two
/// significant figures.
#[must_use]
pub fn is_coarse_bytes(bytes: u64) -> bool {
    bytes.is_multiple_of(MIB) && is_coarse_count(bytes / MIB)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_round_half_up_to_two_significant_figures() {
        for (raw, coarse) in [
            (0, 0),
            (7, 7),
            (99, 99),
            (100, 100),
            (104, 100),
            (105, 110),
            (1234, 1200),
            (1250, 1300),
            (4123, 4100),
            (9950, 10_000),
            (999_999, 1_000_000),
        ] {
            assert_eq!(coarsen_count(raw), coarse, "{raw}");
        }
    }

    #[test]
    fn a_coarsened_figure_is_always_coarse() {
        let mut values: Vec<u64> = (0..=2000).collect();
        values.extend((0..64).map(|shift| 1_u64 << shift));
        values.extend([999_999, 1_000_001, u64::MAX / 2, u64::MAX - 1, u64::MAX]);
        for value in values {
            assert!(is_coarse_count(coarsen_count(value)), "count {value}");
            assert!(is_coarse_bytes(coarsen_bytes(value)), "bytes {value}");
        }
    }

    #[test]
    fn bytes_coarsen_to_whole_mib_at_two_significant_figures() {
        assert_eq!(coarsen_bytes(0), 0);
        assert_eq!(coarsen_bytes(MIB / 2 - 1), 0);
        assert_eq!(coarsen_bytes(MIB / 2), MIB);
        assert_eq!(coarsen_bytes(1234 * MIB + 7), 1200 * MIB);
        assert!(!is_coarse_bytes(MIB + 1), "not whole MiB");
        assert!(!is_coarse_bytes(123 * MIB), "three figures");
        assert!(is_coarse_bytes(120 * MIB));
    }

    #[test]
    fn a_request_is_valid_only_when_coarse_and_in_bounds() {
        assert!(FirstRunCopyRequest::coarsened(4123, 987_654_321).is_valid());
        assert_eq!(
            FirstRunCopyRequest::coarsened(4123, 987_654_321),
            FirstRunCopyRequest {
                trace_count: 4100,
                total_bytes: 940 * MIB
            }
        );
        let raw = FirstRunCopyRequest {
            trace_count: 4123,
            total_bytes: 0,
        };
        assert!(!raw.is_valid());
        let huge = FirstRunCopyRequest {
            trace_count: 1_000_000_000,
            total_bytes: 0,
        };
        assert!(!huge.is_valid());
        // The largest figures clamp to the bounds and stay valid.
        assert!(FirstRunCopyRequest::coarsened(u64::MAX, u64::MAX).is_valid());
    }

    #[test]
    fn a_request_names_both_figures_and_nothing_else() {
        let json = serde_json::to_value(FirstRunCopyRequest {
            trace_count: 4100,
            total_bytes: 940 * MIB,
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({"trace_count": 4100, "total_bytes": 985_661_440_u64})
        );
        assert!(
            serde_json::from_value::<FirstRunCopyRequest>(
                serde_json::json!({"trace_count": 1, "total_bytes": 0, "extra": 1})
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<FirstRunCopyRequest>(serde_json::json!({"trace_count": 1}))
                .is_err()
        );
    }

    #[test]
    fn the_default_lines_are_valid_and_name_no_amount() {
        let lines = default_lines();
        assert!(lines.is_valid());
        for line in [
            DEFAULT_FOUND_CREDIT,
            DEFAULT_FOUND_COULD_CREDIT,
            DEFAULT_CREDIT_DEPENDS,
        ] {
            assert!(!line.contains('$'), "{line}");
            assert!(!line.contains("point"), "{line}");
        }
    }

    #[test]
    fn a_line_is_plain_short_text_with_no_forbidden_word() {
        assert!(valid_line("You could earn about $12 in credit."));
        for bad in [
            "",
            "   ",
            "two\nlines",
            "a\u{7}bell",
            "<b>bold</b>",
            "a {placeholder}",
            "Trace Commons pays you.",
            "The DAEMON says hi.",
            "Use your passkey.",
        ] {
            assert!(!valid_line(bad), "{bad:?}");
        }
        assert!(valid_line(&"a".repeat(FIRST_RUN_LINE_MAX_CHARS)));
        assert!(!valid_line(&"a".repeat(FIRST_RUN_LINE_MAX_CHARS + 1)));
    }

    #[test]
    fn one_bad_line_refuses_the_set_and_extra_fields_are_read() {
        let mut lines = default_lines();
        lines.credit_depends = Some("daemon".into());
        assert!(!lines.is_valid());
        let mut lines = default_lines();
        lines.credit_depends = None;
        assert!(lines.is_valid());
        let parsed: FirstRunCopyLines = serde_json::from_value(serde_json::json!({
            "found_credit": "a.", "found_could_credit": "b.", "later": true
        }))
        .unwrap();
        assert_eq!(parsed.credit_depends, None);
    }
}
