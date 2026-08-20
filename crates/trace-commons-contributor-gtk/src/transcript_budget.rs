//! How much of a redacted body the transcript tab lays out at once.
//!
//! The tab used to hand its whole body to one `TextBuffer`. That held while
//! a trace was a pilot trace: 169 KB, laid out in a few milliseconds. It
//! does not hold for a real Claude Code session. A 17.5 MB body is well
//! inside what GTK's line-virtualized `TextView` can hold without pinning
//! the main thread the way a single CoreText run did on macOS, but it still
//! applies tags across the whole buffer on every fill and still has no
//! bound on how much it is asked to hold at once, so it gets the same fix.
//!
//! So the tab lays out a bounded slice and says so. What it must never do
//! is imply the slice is the whole thing: this tab's promise is "exactly
//! what would be sent", the approval covers every byte, and a view that
//! quietly shows the first fraction of a body while the button beneath it
//! approves all of it would make that promise false. [`notice`] is
//! therefore not decoration -- it is the sentence that keeps the tab
//! honest, and it states both what is on screen and that approval still
//! covers the rest.
//!
//! Three shells render this and they must agree, for the same reason the
//! submit toast must: the macOS copy is
//! `macos/Sources/TCShellCore/TranscriptBudget.swift` and the Windows copy
//! is `windows/src/TraceCommons.Interop/TranscriptBudget.cs`. All three
//! assert the same worked examples.

/// The slice size, in bytes of UTF-8.
///
/// Measured, not guessed: single-run layout of a transcript is quadratic in
/// its size, and 64 KB is the last size that reads as a pause rather than a
/// freeze. The table of measurements is in the reference implementation,
/// `macos/Sources/TCShellCore/TranscriptBudget.swift`. GTK's `TextView` is
/// line-virtualized and so suffers less than the other two shells, but the
/// budget is shared because the notice it produces is shared.
///
/// Several hundred lines of transcript: far more than the "first screenful"
/// the read gate actually claims.
pub const LIMIT_BYTES: usize = 64 * 1024;

/// A body clamped to the budget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clamped {
    /// The text to lay out. Never longer than [`LIMIT_BYTES`] in UTF-8.
    pub shown: String,
    /// UTF-8 bytes of the original body.
    pub total_bytes: usize,
    /// UTF-8 bytes not laid out. Zero when the whole body fits.
    pub withheld_bytes: usize,
}

impl Clamped {
    /// True when the body did not fit and [`notice`] must be shown.
    pub fn is_clamped(&self) -> bool {
        self.withheld_bytes > 0
    }
}

/// Clamps `text` to the budget, cutting at a line boundary so the slice
/// never ends mid-line -- and, because a line boundary is always a
/// character boundary, never mid-character either.
///
/// A body with no newline inside the budget (minified JSON on one line, for
/// instance) still has to be cut somewhere, so the cut backs off to the
/// nearest UTF-8 character boundary instead. Cutting mid-character would
/// put a replacement character on screen inside a tab whose entire job is
/// showing bytes faithfully.
pub fn clamp(text: &str) -> Clamped {
    let bytes = text.as_bytes();
    let total = bytes.len();
    if total <= LIMIT_BYTES {
        return Clamped {
            shown: text.to_string(),
            total_bytes: total,
            withheld_bytes: 0,
        };
    }

    let mut cut = LIMIT_BYTES;

    // Prefer the last newline in the slice: a whole number of lines is what
    // a person expects to see, and it keeps the cut off the middle of a
    // redaction marker often enough to matter.
    if let Some(newline) = bytes[..cut].iter().rposition(|&b| b == b'\n') {
        cut = newline + 1;
    } else {
        // No newline to cut on. Back off to the nearest character boundary
        // rather than enabling a nightly `floor_char_boundary`: a
        // continuation byte (0b10xxxxxx) is never a valid place to split.
        while cut > 0 && bytes[cut] & 0xC0 == 0x80 {
            cut -= 1;
        }
    }

    // `cut` now sits on a boundary the newline search or the backoff loop
    // guarantees is a valid char boundary, so this cannot panic or produce
    // a replacement character.
    let shown = std::str::from_utf8(&bytes[..cut])
        .expect("cut lands on a UTF-8 character boundary")
        .to_string();
    let withheld = total - cut;
    Clamped {
        shown,
        total_bytes: total,
        withheld_bytes: withheld,
    }
}

/// The sentence shown above a clamped body.
///
/// Says what is displayed, says what is not, and says that approval is
/// unaffected -- in that order, because the reader's first question is "am
/// I seeing all of it" and their second is "does that change what I am
/// about to agree to".
pub fn notice(clamped: &Clamped) -> String {
    if !clamped.is_clamped() {
        return String::new();
    }
    let shown_bytes = clamped.total_bytes - clamped.withheld_bytes;
    format!(
        "Showing the first {} of {}. The rest is not displayed here. Approving still covers the whole body.",
        bytes(shown_bytes),
        bytes(clamped.total_bytes)
    )
}

/// Byte counts in the shell's usual units. Kept here rather than taken from
/// a view helper so the three shells format the notice identically.
fn bytes(count: usize) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    let value = count as f64;
    if value >= MB {
        return format!("{:.1} MB", value / MB);
    }
    if value >= KB {
        return format!("{:.0} KB", value / KB);
    }
    format!("{count} B")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A body under the budget is passed through untouched and carries no
    /// notice. The common case must not gain a truncation warning it does
    /// not deserve.
    #[test]
    fn short_body_is_unchanged() {
        let text = "line one\nline two\n";
        let clamped = clamp(text);
        assert_eq!(clamped.shown, text);
        assert_eq!(clamped.withheld_bytes, 0);
        assert!(!clamped.is_clamped());
        assert_eq!(notice(&clamped), "");
    }

    /// A body exactly at the budget is not clamped. Off-by-one here would
    /// put a "showing the first 64 KB of 64 KB" notice on screen.
    #[test]
    fn body_exactly_at_budget_is_not_clamped() {
        let text = "a".repeat(LIMIT_BYTES);
        let clamped = clamp(&text);
        assert!(!clamped.is_clamped());
        assert_eq!(clamped.shown.len(), LIMIT_BYTES);
        assert_eq!(notice(&clamped), "");
    }

    /// The slice never exceeds the budget, and the withheld count is the
    /// exact remainder -- the notice's arithmetic depends on it.
    #[test]
    fn long_body_is_clamped_to_budget() {
        let line = format!("{}\n", "x".repeat(99));
        let text = line.repeat(20_000); // ~2 MB
        let clamped = clamp(&text);

        assert!(clamped.is_clamped());
        assert!(clamped.shown.len() <= LIMIT_BYTES);
        assert_eq!(clamped.total_bytes, text.len());
        assert_eq!(
            clamped.shown.len() + clamped.withheld_bytes,
            clamped.total_bytes
        );
    }

    /// The cut lands on a line boundary, so the last visible line is whole.
    #[test]
    fn clamp_cuts_on_a_line_boundary() {
        let line = format!("{}\n", "x".repeat(99));
        let text = line.repeat(20_000);
        let clamped = clamp(&text);
        assert!(clamped.shown.ends_with('\n'));
        for l in clamped.shown.split('\n') {
            if l.is_empty() {
                continue;
            }
            assert_eq!(l.len(), 99);
        }
    }

    /// A body with no newline in the budget still gets cut, and the cut
    /// does not split a multi-byte character. This is the minified-JSON
    /// case. Four-byte scalars, so a naive byte cut lands mid-character
    /// with high probability.
    #[test]
    fn clamp_without_newlines_does_not_split_a_character() {
        let text = "🙂".repeat(LIMIT_BYTES);
        let clamped = clamp(&text);

        assert!(clamped.is_clamped());
        assert!(clamped.shown.len() <= LIMIT_BYTES);
        assert!(!clamped.shown.contains('\u{FFFD}'));
        // Round-tripping the slice reproduces its own bytes: proof the cut
        // is character-aligned rather than merely replacement-free.
        assert_eq!(
            clamped.shown.as_bytes(),
            &text.as_bytes()[..clamped.shown.len()]
        );
    }

    /// A multi-byte body that does have newlines keeps its characters whole
    /// too -- the line-boundary path and the character-boundary path must
    /// both hold.
    #[test]
    fn clamp_with_multibyte_lines_keeps_characters_whole() {
        let line = format!("{}\n", "é".repeat(50));
        let text = line.repeat(20_000);
        let clamped = clamp(&text);

        assert!(clamped.is_clamped());
        assert!(!clamped.shown.contains('\u{FFFD}'));
        for l in clamped.shown.split('\n') {
            if l.is_empty() {
                continue;
            }
            assert_eq!(l.chars().count(), 50);
        }
    }

    /// The notice states both numbers and does not imply approval shrank.
    /// This is the sentence that keeps the tab's promise true, so it is
    /// asserted verbatim rather than by shape.
    #[test]
    fn notice_states_shown_total_and_that_approval_is_unaffected() {
        let text = "x\n".repeat(9_000_000); // ~17.2 MB
        let clamped = clamp(&text);
        let notice = notice(&clamped);

        assert_eq!(
            notice,
            "Showing the first 64 KB of 17.2 MB. \
             The rest is not displayed here. Approving still covers the whole body."
        );
    }

    /// The reported "shown" figure is the size of what is actually on
    /// screen, not the budget constant. A cut that backs off to a line
    /// boundary shows slightly less than 64 KB, and the notice must not
    /// round that into a claim about bytes the reader cannot see.
    #[test]
    fn notice_reports_bytes_actually_shown() {
        let line = format!("{}\n", "x".repeat(999));
        let text = line.repeat(2_000);
        let clamped = clamp(&text);
        let shown_bytes = clamped.total_bytes - clamped.withheld_bytes;
        assert_eq!(shown_bytes, clamped.shown.len());
    }

    /// The 17.5 MB body that hung the macOS shell lays out its slice
    /// promptly here too. The budget exists for this case, so the case is
    /// the test.
    #[test]
    fn realistic_large_body_clamps_quickly() {
        let line = format!("{}\n", "y".repeat(175));
        let text = line.repeat(100_000); // ~17.6 MB
        let started = std::time::Instant::now();
        let clamped = clamp(&text);
        let elapsed = started.elapsed();

        assert!(clamped.is_clamped());
        assert!(clamped.shown.len() <= LIMIT_BYTES);
        assert!(
            elapsed.as_secs_f64() < 2.0,
            "clamping should be a scan, not a reflow"
        );
    }
}
