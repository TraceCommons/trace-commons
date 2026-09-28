//! "Worth a second look": the one predicate that decides it.
//!
//! The Flow 2 design groups two watcher states under that heading -- a
//! session where **nothing matched** (the scrubber made zero marks) and a
//! session **trimmed to fit** the byte budget -- and says both wait and
//! neither is a colour alone. Both are facts the daemon already holds, so
//! the daemon answers the question once, here, and every surface reads the
//! answer rather than re-deriving it:
//!
//! - `list_pending` and the `snapshot` event, through
//!   `ipc::entry_value`, from the mark count the latest preview recorded on
//!   the queue entry;
//! - `preview` and the scheduler's `preview_ready` event, from the summary
//!   that preview just built;
//! - the Scrub check setting (K4 of #1118), which holds an unsure session in
//!   an armed folder for a person, calls [`second_look_reasons`] directly.
//!
//! # Not yet scrubbed is not zero marks
//!
//! Before any preview has run for an entry, nobody has counted its marks.
//! That is [`Scrub::NotYetScrubbed`], and it is a different fact from
//! `Scrubbed { marks: 0 }`: reporting an unscrubbed session as "nothing
//! matched" would tell a contributor something false about a session the
//! scrubber never read, and reporting it as clean would be worse. So the
//! mark count is an `Option` everywhere it travels, `None` never collapses
//! to `0`, and [`second_look_reasons`] never says `nothing-matched` for it.
//!
//! What the predicate *can* still say about an unscrubbed session is
//! `trimmed-to-fit`: the trim is decided when the transcript is loaded, and
//! the queue entry records it (`subagents_dropped`) at discovery.
//!
//! A caller that must decide whether to move a session on its own -- K4 --
//! must treat `NotYetScrubbed` as undecided and scrub first. An empty reason
//! list for an unscrubbed session means "no reason known yet", not "fine".
//!
//! Labels only: nothing in this module ever sees trace content.

/// The scrubber made no marks on this session: no detector matched.
///
/// This is not the same as "clean". A session with nothing to redact and a
/// session whose private details every detector missed look identical from
/// here, which is exactly why it waits for a person.
pub const REASON_NOTHING_MATCHED: &str = "nothing-matched";

/// The session was trimmed to fit the byte budget: some of it is not in
/// what would be sent (today, delegated subagent transcripts the source left
/// out; see `QueueEntry::subagents_dropped`).
pub const REASON_TRIMMED_TO_FIT: &str = "trimmed-to-fit";

/// Every reason [`second_look_reasons`] can return, in the order it returns
/// them. Closed: a shell may hold a sentence per label.
pub const SECOND_LOOK_REASONS: &[&str] = &[REASON_NOTHING_MATCHED, REASON_TRIMMED_TO_FIT];

/// Wire value of [`Scrub::NotYetScrubbed`].
pub const SCRUB_NOT_YET_SCRUBBED: &str = "not-yet-scrubbed";
/// Wire value of [`Scrub::Scrubbed`].
pub const SCRUB_SCRUBBED: &str = "scrubbed";

/// Whether the scrubber has run on a session, and how many marks it made.
///
/// "Marks" are removals: the sum of the redaction counts that actually took
/// something out of what would be sent
/// ([`crate::redaction_labels::removed_total`]). A secret the scrubber found
/// and left in place (`residual_secret_at:*`) is not a mark -- it is the
/// opposite of one -- and never makes a session look scrubbed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scrub {
    /// No preview has run for this entry, so nobody has counted.
    NotYetScrubbed,
    /// A preview ran and made this many marks.
    Scrubbed { marks: u32 },
}

impl Scrub {
    /// From the optional count a queue entry carries. `None` is
    /// [`Scrub::NotYetScrubbed`]; it is never read as zero.
    pub fn from_marks(marks: Option<u32>) -> Self {
        match marks {
            Some(marks) => Scrub::Scrubbed { marks },
            None => Scrub::NotYetScrubbed,
        }
    }

    /// From a preview's redaction count map.
    pub fn from_redactions(redactions: &std::collections::BTreeMap<String, u32>) -> Self {
        Scrub::Scrubbed {
            marks: crate::redaction_labels::removed_total(redactions),
        }
    }

    /// The mark count, or `None` when not yet scrubbed.
    pub fn marks(self) -> Option<u32> {
        match self {
            Scrub::Scrubbed { marks } => Some(marks),
            Scrub::NotYetScrubbed => None,
        }
    }

    /// The wire label: [`SCRUB_SCRUBBED`] or [`SCRUB_NOT_YET_SCRUBBED`].
    pub fn label(self) -> &'static str {
        match self {
            Scrub::Scrubbed { .. } => SCRUB_SCRUBBED,
            Scrub::NotYetScrubbed => SCRUB_NOT_YET_SCRUBBED,
        }
    }
}

/// Why a session is worth a second look, as fixed labels from
/// [`SECOND_LOOK_REASONS`]. Empty means no reason is known.
///
/// - `scrub` -- [`Scrub::Scrubbed`] with the marks the latest preview made,
///   or [`Scrub::NotYetScrubbed`]. `Scrubbed { marks: 0 }` yields
///   [`REASON_NOTHING_MATCHED`]; `NotYetScrubbed` never does.
/// - `subagents_dropped` -- how many delegated transcripts the source left
///   out to fit the byte budget; non-zero yields [`REASON_TRIMMED_TO_FIT`].
///   Known from discovery, so it is reported whether or not the session has
///   been scrubbed.
///
/// Reasons come back in [`SECOND_LOOK_REASONS`] order, each at most once.
///
/// **An empty list is only an all-clear when `scrub` is `Scrubbed`.** For a
/// `NotYetScrubbed` session it means the scrubber has not run; a caller that
/// would otherwise let the session move on its own (the Scrub check's
/// Automatic mode) must scrub it first and ask again.
pub fn second_look_reasons(scrub: Scrub, subagents_dropped: u32) -> Vec<&'static str> {
    let mut reasons = Vec::new();
    if scrub == (Scrub::Scrubbed { marks: 0 }) {
        reasons.push(REASON_NOTHING_MATCHED);
    }
    if subagents_dropped > 0 {
        reasons.push(REASON_TRIMMED_TO_FIT);
    }
    reasons
}

/// The three fields every surface publishes, inserted into a JSON object:
/// `scrub` (always), `marks` (only when scrubbed -- ABSENT, never `0` or
/// `null`, before the first preview) and `second_look` (always, possibly
/// empty).
///
/// One writer, so `entry_value` and the preview summaries cannot publish
/// the state under two spellings.
pub fn insert_fields(value: &mut serde_json::Value, scrub: Scrub, subagents_dropped: u32) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    object.insert("scrub".into(), serde_json::Value::from(scrub.label()));
    if let Some(marks) = scrub.marks() {
        object.insert("marks".into(), serde_json::Value::from(marks));
    }
    object.insert(
        "second_look".into(),
        serde_json::Value::from(second_look_reasons(scrub, subagents_dropped)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_marks_is_nothing_matched() {
        assert_eq!(
            second_look_reasons(Scrub::Scrubbed { marks: 0 }, 0),
            vec![REASON_NOTHING_MATCHED]
        );
        assert!(second_look_reasons(Scrub::Scrubbed { marks: 7 }, 0).is_empty());
    }

    #[test]
    fn a_trimmed_session_is_trimmed_to_fit_scrubbed_or_not() {
        assert_eq!(
            second_look_reasons(Scrub::Scrubbed { marks: 3 }, 1),
            vec![REASON_TRIMMED_TO_FIT]
        );
        assert_eq!(
            second_look_reasons(Scrub::NotYetScrubbed, 2),
            vec![REASON_TRIMMED_TO_FIT]
        );
        assert_eq!(
            second_look_reasons(Scrub::Scrubbed { marks: 0 }, 1),
            SECOND_LOOK_REASONS.to_vec(),
            "both reasons, in the published order"
        );
    }

    #[test]
    fn not_yet_scrubbed_is_never_reported_as_zero_marks() {
        assert_eq!(Scrub::from_marks(None), Scrub::NotYetScrubbed);
        assert_ne!(Scrub::from_marks(None), Scrub::Scrubbed { marks: 0 });
        assert!(second_look_reasons(Scrub::from_marks(None), 0).is_empty());

        let mut unscrubbed = serde_json::json!({});
        insert_fields(&mut unscrubbed, Scrub::NotYetScrubbed, 0);
        assert_eq!(unscrubbed["scrub"], SCRUB_NOT_YET_SCRUBBED);
        assert!(
            unscrubbed.get("marks").is_none(),
            "no count is published before a preview has counted: {unscrubbed}"
        );

        let mut clean = serde_json::json!({});
        insert_fields(&mut clean, Scrub::from_marks(Some(0)), 0);
        assert_eq!(clean["scrub"], SCRUB_SCRUBBED);
        assert_eq!(clean["marks"], 0);
        assert_eq!(clean["second_look"][0], REASON_NOTHING_MATCHED);
    }

    #[test]
    fn a_surviving_secret_is_not_a_mark() {
        let mut redactions = std::collections::BTreeMap::new();
        redactions.insert("residual_secret_at:events.0.correction".to_string(), 1);
        assert_eq!(
            Scrub::from_redactions(&redactions),
            Scrub::Scrubbed { marks: 0 }
        );
        redactions.insert("local_path".to_string(), 4);
        assert_eq!(
            Scrub::from_redactions(&redactions),
            Scrub::Scrubbed { marks: 4 }
        );
    }
}
