//! The submit toast: the daemon's counts, said in one sentence.
//!
//! One-click submit sends a session nobody previewed, so the toast is the
//! only place a contributor learns what happened -- what went, what
//! scrubbing did to it, what was held, and what never left. That makes the
//! wording a contract rather than a presentation detail, and it is fixed in
//! `docs/superpowers/specs/2026-08-20-one-click-submit-design.md` under
//! "The toast: normative copy". The clause strings themselves live in
//! [`crate::copy`] with the rest of this shell's words; this module is only
//! the assembly.
//!
//! Three shells render this sentence and they must render it identically.
//! The Swift copy is `macos/Sources/TCShellCore/SubmitToast.swift`, the C#
//! copy is `windows/src/TraceCommons.Interop/SubmitToast.cs`, and each
//! carries the spec's four worked examples as a test for exactly one
//! reason: a sentence reworded in one client is the drift this section of
//! the spec exists to prevent.
//!
//! Deliberately pure -- counts in, a string and a bool out. No GTK types,
//! no I/O, so the assertion that the three shells agree runs on a machine
//! with no display, and so the Swift and C# copies can assert the same
//! thing without a running app.

use crate::copy;

/// What the shell shows after a submit, and whether Undo goes with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmitToast {
    /// The whole sentence, ready to display.
    pub line: String,
    /// Whether to offer Undo alongside it.
    ///
    /// True only when something was actually sent. `ui::preview` used to
    /// offer Undo on any `Ok` response, which was correct while every
    /// approval succeeded and is wrong now that entries can be skipped: a
    /// skipped entry with an undo timer behind it reads as sent.
    pub offer_undo: bool,
}

/// Render the toast from an `approve` response.
///
/// `redactions` is the sum of the response's `redactions` map -- the toast
/// names a count, never a category, because the preview sheet is where a
/// contributor sees which detector fired. `skipped` is the response's wire
/// reason labels, in response order; the rendered sentence uses the human
/// labels, distinct, in the spec table's order, so neither a wire label nor
/// an entry id ever reaches a contributor.
pub fn toast(approved: u64, redactions: u64, flagged: u64, skipped: &[&str]) -> SubmitToast {
    let mut clauses = vec![
        copy::submit_sent_clause(approved),
        copy::submit_scrub_clause(redactions),
    ];

    if flagged > 0 {
        clauses.push(copy::submit_flagged_clause(flagged));
    }

    if !skipped.is_empty() {
        clauses.push(copy::submit_skipped_clause(skipped));
    }

    SubmitToast {
        line: clauses.join(" "),
        offer_undo: approved > 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spec_worked_examples_render_exactly() {
        assert_eq!(
            toast(1, 4, 1, &[]).line,
            "Sent. Scrubbing removed 4 things. 1 flagged."
        );
        assert_eq!(
            toast(47, 213, 3, &[]).line,
            "Sent 47 sessions. Scrubbing removed 213 things. 3 flagged."
        );
        assert_eq!(
            toast(
                44,
                213,
                0,
                &[
                    "envelope-too-large",
                    "envelope-too-large",
                    "envelope-too-large"
                ]
            )
            .line,
            "Sent 44 sessions. Scrubbing removed 213 things. 3 not sent: too large to send."
        );
        assert_eq!(
            toast(0, 0, 0, &["not-pending", "not-pending"]).line,
            "Nothing sent. Scrubbing matched nothing. 2 not sent: already decided."
        );
    }

    #[test]
    fn undo_is_offered_only_when_something_was_sent() {
        assert!(toast(1, 0, 0, &[]).offer_undo);
        assert!(!toast(0, 0, 0, &["not-pending"]).offer_undo);
    }

    #[test]
    fn a_wire_label_never_reaches_the_contributor() {
        let line = toast(0, 0, 0, &["envelope-too-large"]).line;
        assert!(
            !line.contains("envelope-too-large"),
            "wire label leaked: {line}"
        );
        assert!(line.contains("too large to send"));
    }

    /// Every wire label the daemon can send, and one it cannot.
    ///
    /// The unknown case is the one that matters: a label this shell has not
    /// been taught is still a label, and echoing it would put daemon
    /// vocabulary in front of a contributor at exactly the moment the shell
    /// is least sure what happened.
    #[test]
    fn every_wire_label_maps_to_a_human_one() {
        for wire in [
            "not-enrolled",
            "not-pending",
            "not-pinned",
            "envelope-too-large",
            "session-file-vanished",
            "preview-failed",
            "some-label-this-shell-has-never-seen",
        ] {
            let line = toast(0, 0, 0, &[wire]).line;
            assert!(!line.contains(wire), "wire label leaked: {line}");
        }
    }

    /// The clause is a count of entries and a list of distinct reasons, and
    /// those are two different numbers whenever entries share a reason.
    #[test]
    fn distinct_reasons_are_listed_once_in_the_specs_order() {
        assert_eq!(
            toast(
                0,
                0,
                0,
                &[
                    "preview-failed",
                    "not-pending",
                    "not-enrolled",
                    "not-pending",
                ]
            )
            .line,
            "Nothing sent. Scrubbing matched nothing. 4 not sent: not connected to a commons, \
             already decided, could not be read."
        );
    }

    /// Clauses 3 and 4 appear only when non-zero; clauses 1 and 2 always do.
    #[test]
    fn the_optional_clauses_are_absent_when_empty() {
        assert_eq!(toast(1, 0, 0, &[]).line, "Sent. Scrubbing matched nothing.");
        assert_eq!(
            toast(2, 1, 0, &[]).line,
            "Sent 2 sessions. Scrubbing removed 1 thing."
        );
    }

    /// A zero redaction count is a fact the contributor is owed, not an
    /// absence to omit -- a session that obviously touched a `.env` and
    /// reports nothing removed is a signal.
    #[test]
    fn scrubbing_reports_zero_rather_than_saying_nothing() {
        assert!(
            toast(1, 0, 0, &[])
                .line
                .contains("Scrubbing matched nothing")
        );
    }
}
