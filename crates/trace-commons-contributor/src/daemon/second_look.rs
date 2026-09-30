//! "Worth a second look": the one predicate that decides it.
//!
//! The Flow 2 design marks a session "worth a second look" when the
//! scrubber was unsure about it, says such a session waits for a person, and
//! says the state is never a colour alone. The daemon answers that once,
//! here, and every surface reads the answer rather than re-deriving it:
//!
//! - `list_pending` and the `snapshot` event, through `ipc::entry_value`,
//!   from the [`ScrubRecord`] the pinning build stored on the queue entry;
//! - `preview` and the scheduler's `preview_ready` event, from the summary
//!   that preview just built;
//! - the Scrub check setting (K4 of #1118), which holds an unsure session in
//!   an armed folder for a person, through [`unattended_hold`]. The uploader
//!   asks it with the scrub of the envelope it has just built, after
//!   redaction and before the send, so the count is exact and never
//!   `NotYetScrubbed`.
//!
//! # The three reasons
//!
//! - [`REASON_NOTHING_MATCHED`]: the scrubber took out **no personal detail**
//!   -- nothing but local paths, or nothing at all. Path removals are left
//!   out of this test on purpose: nearly every real session carries absolute
//!   paths, so counting them would make "nothing matched" almost never fire,
//!   and an Automatic-mode session whose only marks were paths would be sent
//!   unattended on the strength of a scrub that found nothing personal.
//! - [`REASON_LOOKS_UNSURE`]: the unsure-span detector (`unsure_spans`) found
//!   something in the redacted body that looks like an email, phone number or
//!   key the scrubber did not mark -- or could not index the body at all,
//!   which counts as unsure (fail-closed).
//! - [`REASON_TRIMMED_TO_FIT`]: the conversation was trimmed to fit the byte
//!   budget, so part of it is not in what would be sent.
//!
//! # Not yet scrubbed is not zero marks
//!
//! Before a build of the bytes that would be sent has been counted, nobody
//! has counted. That is [`Scrub::NotYetScrubbed`], and it is a different fact
//! from a scrubbed session with zero marks. [`second_look_reasons`] never
//! says `nothing-matched` or `looks-unsure` for it; it can still say
//! `trimmed-to-fit`, which is decided at discovery.
//!
//! A caller that must decide whether to move a session on its own -- K4 --
//! must treat `NotYetScrubbed` as undecided and scrub first. An empty reason
//! list for an unscrubbed session means "no reason known yet", not "fine".
//!
//! Labels and counts only: nothing in this module keeps trace content.

use serde::{Deserialize, Serialize};

/// The scrubber took out no personal detail: no removal outside the
/// path families ([`PATH_FAMILIES`]).
pub const REASON_NOTHING_MATCHED: &str = "nothing-matched";

/// The redacted body holds something that looks like personal data the
/// scrubber did not mark, or the body could not be checked for it.
pub const REASON_LOOKS_UNSURE: &str = "looks-unsure";

/// The session was trimmed to fit the byte budget: some of it is not in
/// what would be sent (today, delegated subagent transcripts the source left
/// out; see `QueueEntry::subagents_dropped`).
pub const REASON_TRIMMED_TO_FIT: &str = "trimmed-to-fit";

/// Every reason [`second_look_reasons`] can return, in the order it returns
/// them. Closed: a shell may hold a sentence per label.
pub const SECOND_LOOK_REASONS: &[&str] = &[
    REASON_NOTHING_MATCHED,
    REASON_LOOKS_UNSURE,
    REASON_TRIMMED_TO_FIT,
];

/// Redaction families that do not count as a personal-detail match for
/// [`REASON_NOTHING_MATCHED`]. They still count as marks for display.
pub const PATH_FAMILIES: &[&str] = &["local_path"];

/// Wire value of [`Scrub::NotYetScrubbed`].
pub const SCRUB_NOT_YET_SCRUBBED: &str = "not-yet-scrubbed";
/// Wire value of [`Scrub::Scrubbed`].
pub const SCRUB_SCRUBBED: &str = "scrubbed";

/// What one scrubbed build of a session says. Counts only.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScrubCounts {
    /// Every value the scrubber took out (the row's "7 marks"): the sum of
    /// the redaction counts that removed something
    /// ([`crate::redaction_labels::removed_total`]). A secret found and left
    /// in place (`residual_secret_at:*`) is not a mark.
    pub marks: u32,
    /// [`Self::marks`] without the [`PATH_FAMILIES`]. Zero is
    /// [`REASON_NOTHING_MATCHED`].
    pub content_marks: u32,
    /// Unsure spans the detector found in the redacted body.
    pub unsure_spans: u32,
    /// The detector could not index the body exactly. Counts as unsure.
    #[serde(default)]
    pub unsure_unreadable: bool,
}

impl ScrubCounts {
    /// Count one build: `redactions` is the summary's redaction map and
    /// `body` is `preview::body_of` for the same envelope.
    pub fn of(redactions: &std::collections::BTreeMap<String, u32>, body: &str) -> Self {
        let marks = crate::redaction_labels::removed_total(redactions);
        let paths: u32 = redactions
            .iter()
            .filter(|(label, _)| {
                crate::redaction_labels::is_removal(label)
                    && PATH_FAMILIES.contains(&crate::redaction_labels::family(label))
            })
            .map(|(_, n)| *n)
            .sum();
        let (unsure_spans, unsure_unreadable) = match super::unsure_spans::unsure_spans_in(body) {
            Ok(spans) => (u32::try_from(spans.len()).unwrap_or(u32::MAX), false),
            Err(_) => (0, true),
        };
        ScrubCounts {
            marks,
            content_marks: marks.saturating_sub(paths),
            unsure_spans,
            unsure_unreadable,
        }
    }
}

/// A [`ScrubCounts`] bound to the exact bytes it describes: the digest of
/// the envelope the entry is pinned to. Stored on the queue entry; read back
/// only while the entry's pin still names the same digest, so a released,
/// replaced or revoked pin -- a re-enrolment, a filter change, an approval
/// revoked and re-offered -- turns the entry back into not yet scrubbed
/// without anyone having to remember to clear it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScrubRecord {
    pub envelope_digest: String,
    pub counts: ScrubCounts,
}

/// Whether the bytes that would be sent have been scrubbed and counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scrub {
    /// No count describes the bytes that would be sent.
    NotYetScrubbed,
    /// A build was counted.
    Scrubbed(ScrubCounts),
}

impl Scrub {
    /// The wire label: [`SCRUB_SCRUBBED`] or [`SCRUB_NOT_YET_SCRUBBED`].
    pub fn label(self) -> &'static str {
        match self {
            Scrub::Scrubbed(_) => SCRUB_SCRUBBED,
            Scrub::NotYetScrubbed => SCRUB_NOT_YET_SCRUBBED,
        }
    }

    /// The counts, or `None` when not yet scrubbed.
    pub fn counts(self) -> Option<ScrubCounts> {
        match self {
            Scrub::Scrubbed(counts) => Some(counts),
            Scrub::NotYetScrubbed => None,
        }
    }
}

/// Why a session is worth a second look, as fixed labels from
/// [`SECOND_LOOK_REASONS`], in that order, each at most once. Empty means
/// no reason is known.
///
/// - `Scrubbed` with `content_marks == 0` yields [`REASON_NOTHING_MATCHED`]
///   (path removals alone are not a match).
/// - `Scrubbed` with `unsure_spans > 0` or `unsure_unreadable` yields
///   [`REASON_LOOKS_UNSURE`].
/// - `subagents_dropped > 0` yields [`REASON_TRIMMED_TO_FIT`], scrubbed or
///   not.
/// - `NotYetScrubbed` never yields the first two.
///
/// **An empty list is only an all-clear when `scrub` is `Scrubbed`.** For a
/// `NotYetScrubbed` session it means the scrubber has not counted the bytes
/// that would be sent; a caller that would otherwise let the session move on
/// its own (the Scrub check's Automatic mode) must scrub it first and ask
/// again.
pub fn second_look_reasons(scrub: Scrub, subagents_dropped: u32) -> Vec<&'static str> {
    let mut reasons = Vec::new();
    if let Scrub::Scrubbed(counts) = scrub {
        if counts.content_marks == 0 {
            reasons.push(REASON_NOTHING_MATCHED);
        }
        if counts.unsure_spans > 0 || counts.unsure_unreadable {
            reasons.push(REASON_LOOKS_UNSURE);
        }
    }
    if subagents_dropped > 0 {
        reasons.push(REASON_TRIMMED_TO_FIT);
    }
    reasons
}

/// The fields every surface publishes, inserted into a JSON object:
/// `scrub` (always), `marks`, `content_marks` and `unsure_spans` (only when
/// scrubbed -- ABSENT, never `0` or `null`, before) and `second_look`
/// (always, possibly empty).
///
/// One writer, so `entry_value` and the preview summaries cannot publish the
/// state under two spellings.
pub fn insert_fields(value: &mut serde_json::Value, scrub: Scrub, subagents_dropped: u32) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    object.insert("scrub".into(), serde_json::Value::from(scrub.label()));
    if let Some(counts) = scrub.counts() {
        object.insert("marks".into(), serde_json::Value::from(counts.marks));
        object.insert(
            "content_marks".into(),
            serde_json::Value::from(counts.content_marks),
        );
        object.insert(
            "unsure_spans".into(),
            serde_json::Value::from(counts.unsure_spans),
        );
    }
    object.insert(
        "second_look".into(),
        serde_json::Value::from(second_look_reasons(scrub, subagents_dropped)),
    );
}

// ---------------------------------------------------------------------------
// The Scrub check (K4 of #1118)
// ---------------------------------------------------------------------------

/// The label a session approved on the contributor's behalf is held under,
/// under the Automatic Scrub check, when [`second_look_reasons`] flags it --
/// or when it has not been scrubbed at all, which is never taken as fine.
///
/// One of `queue::REASONS_NEEDING_A_PERSON`: the watcher does not approve it
/// again and a group approve leaves it out. The particular reasons are not
/// in the label. When the local envelope can be saved, the hold pins it and
/// records its counts beside the digest of the bytes they describe
/// (`QueueEntry::scrub`), so the held entry reads `scrubbed` and its
/// `second_look` comes from that build. Without a saved local envelope it
/// stays held and reads `not-yet-scrubbed` until a review pins and counts it.
pub const REASON_SECOND_LOOK_REVIEW_REQUIRED: &str = "second-look-review-required";

/// The label an unattended approval is revoked under while the Scrub check
/// is Manual: everything waits for a person.
///
/// Deliberately **not** one of `queue::REASONS_NEEDING_A_PERSON`. While
/// Manual is set the watcher approves nothing on anyone's behalf anyway, so
/// the list adds nothing there; and once Automatic is set again, an armed
/// folder's session going back through the Automatic check -- the hold
/// above included -- is exactly what the contributor asked for. Only an
/// approval that predates the switch to Manual ever carries this label.
pub const REASON_SCRUB_CHECK_MANUAL: &str = "scrub-check-manual";

/// Whether a session approved on the contributor's behalf must wait for a
/// person instead of being sent, and under which label. `None` means it may
/// go.
///
/// - Manual: always held, whatever the scrub says.
/// - Automatic: held when [`second_look_reasons`] names any reason, **and
///   held when `scrub` is [`Scrub::NotYetScrubbed`]**. An empty reason list
///   is an all-clear only for a scrubbed session; a caller that has not
///   scrubbed yet gets a hold, never a pass.
///
/// The uploader calls this with the scrub of the envelope it has just built
/// (so in practice always `Scrubbed`); the `NotYetScrubbed` arm is what
/// keeps any other caller honest.
pub fn unattended_hold(
    check: super::settings::ScrubCheck,
    scrub: Scrub,
    subagents_dropped: u32,
) -> Option<&'static str> {
    match check {
        super::settings::ScrubCheck::Manual => Some(REASON_SCRUB_CHECK_MANUAL),
        super::settings::ScrubCheck::Automatic => {
            if scrub == Scrub::NotYetScrubbed
                || !second_look_reasons(scrub, subagents_dropped).is_empty()
            {
                Some(REASON_SECOND_LOOK_REVIEW_REQUIRED)
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::settings::ScrubCheck;

    #[test]
    fn manual_holds_everything_whatever_the_scrub_says() {
        for scrub in [Scrub::NotYetScrubbed, counts(0, 0, 0), counts(7, 7, 0)] {
            for dropped in [0, 2] {
                assert_eq!(
                    unattended_hold(ScrubCheck::Manual, scrub, dropped),
                    Some(REASON_SCRUB_CHECK_MANUAL)
                );
            }
        }
    }

    #[test]
    fn automatic_holds_what_is_worth_a_second_look_and_passes_a_clean_scrub() {
        let auto = ScrubCheck::Automatic;
        assert_eq!(
            unattended_hold(auto, counts(0, 0, 0), 0),
            Some(REASON_SECOND_LOOK_REVIEW_REQUIRED),
            "nothing matched"
        );
        assert_eq!(
            unattended_hold(auto, counts(3, 0, 0), 0),
            Some(REASON_SECOND_LOOK_REVIEW_REQUIRED),
            "nothing matched: only paths were removed"
        );
        assert_eq!(
            unattended_hold(auto, counts(4, 4, 1), 0),
            Some(REASON_SECOND_LOOK_REVIEW_REQUIRED),
            "looks unsure"
        );
        assert_eq!(
            unattended_hold(
                auto,
                Scrub::Scrubbed(ScrubCounts {
                    marks: 4,
                    content_marks: 4,
                    unsure_spans: 0,
                    unsure_unreadable: true,
                }),
                0
            ),
            Some(REASON_SECOND_LOOK_REVIEW_REQUIRED),
            "an unreadable body looks unsure"
        );
        assert_eq!(
            unattended_hold(auto, counts(4, 4, 0), 1),
            Some(REASON_SECOND_LOOK_REVIEW_REQUIRED),
            "trimmed to fit"
        );
        assert_eq!(unattended_hold(auto, counts(4, 4, 0), 0), None);
    }

    /// No reasons for a session nobody scrubbed means nobody has looked,
    /// not that it is fine.
    #[test]
    fn automatic_never_passes_a_session_nobody_has_scrubbed() {
        assert!(second_look_reasons(Scrub::NotYetScrubbed, 0).is_empty());
        assert_eq!(
            unattended_hold(ScrubCheck::Automatic, Scrub::NotYetScrubbed, 0),
            Some(REASON_SECOND_LOOK_REVIEW_REQUIRED)
        );
    }

    fn counts(marks: u32, content_marks: u32, unsure_spans: u32) -> Scrub {
        Scrub::Scrubbed(ScrubCounts {
            marks,
            content_marks,
            unsure_spans,
            unsure_unreadable: false,
        })
    }

    fn map(pairs: &[(&str, u32)]) -> std::collections::BTreeMap<String, u32> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    const CLEAN_BODY: &str = "[\n  {\n    \"redacted_content\": \"hello\"\n  }\n]";

    #[test]
    fn zero_marks_is_nothing_matched() {
        assert_eq!(
            second_look_reasons(counts(0, 0, 0), 0),
            vec![REASON_NOTHING_MATCHED]
        );
        assert!(second_look_reasons(counts(7, 7, 0), 0).is_empty());
    }

    #[test]
    fn path_removals_alone_are_nothing_matched() {
        let only_paths = ScrubCounts::of(&map(&[("local_path", 12)]), CLEAN_BODY);
        assert_eq!(only_paths.marks, 12, "paths still show as marks");
        assert_eq!(only_paths.content_marks, 0);
        assert_eq!(
            second_look_reasons(Scrub::Scrubbed(only_paths), 0),
            vec![REASON_NOTHING_MATCHED]
        );
        let with_email = ScrubCounts::of(
            &map(&[("local_path", 12), ("private_email", 1)]),
            CLEAN_BODY,
        );
        assert_eq!(with_email.content_marks, 1);
        assert!(second_look_reasons(Scrub::Scrubbed(with_email), 0).is_empty());
    }

    #[test]
    fn an_unsure_span_or_an_unreadable_body_is_looks_unsure() {
        assert_eq!(
            second_look_reasons(counts(3, 3, 1), 0),
            vec![REASON_LOOKS_UNSURE]
        );
        let key = ScrubCounts::of(
            &map(&[("private_email", 1)]),
            "[\n  {\n    \"redacted_content\": \"AKIAIOSFODNN7EXAMPLE\"\n  }\n]",
        );
        assert_eq!(key.unsure_spans, 1);
        assert_eq!(
            second_look_reasons(Scrub::Scrubbed(key), 0),
            vec![REASON_LOOKS_UNSURE]
        );
        let unreadable = ScrubCounts::of(&map(&[("private_email", 1)]), "[\"unterminated");
        assert!(unreadable.unsure_unreadable);
        assert_eq!(
            second_look_reasons(Scrub::Scrubbed(unreadable), 0),
            vec![REASON_LOOKS_UNSURE]
        );
    }

    #[test]
    fn a_trimmed_session_is_trimmed_to_fit_scrubbed_or_not() {
        assert_eq!(
            second_look_reasons(counts(3, 3, 0), 1),
            vec![REASON_TRIMMED_TO_FIT]
        );
        assert_eq!(
            second_look_reasons(Scrub::NotYetScrubbed, 2),
            vec![REASON_TRIMMED_TO_FIT]
        );
        assert_eq!(
            second_look_reasons(counts(0, 0, 2), 1),
            SECOND_LOOK_REASONS.to_vec(),
            "every reason, in the published order"
        );
    }

    #[test]
    fn not_yet_scrubbed_is_never_reported_as_zero_marks() {
        assert!(second_look_reasons(Scrub::NotYetScrubbed, 0).is_empty());

        let mut unscrubbed = serde_json::json!({});
        insert_fields(&mut unscrubbed, Scrub::NotYetScrubbed, 0);
        assert_eq!(unscrubbed["scrub"], SCRUB_NOT_YET_SCRUBBED);
        for key in ["marks", "content_marks", "unsure_spans"] {
            assert!(
                unscrubbed.get(key).is_none(),
                "no {key} before anything counted: {unscrubbed}"
            );
        }

        let mut clean = serde_json::json!({});
        insert_fields(&mut clean, counts(0, 0, 0), 0);
        assert_eq!(clean["scrub"], SCRUB_SCRUBBED);
        assert_eq!(clean["marks"], 0);
        assert_eq!(clean["second_look"][0], REASON_NOTHING_MATCHED);
    }

    #[test]
    fn a_surviving_secret_is_not_a_mark() {
        let c = ScrubCounts::of(
            &map(&[("residual_secret_at:events.0.correction", 1)]),
            CLEAN_BODY,
        );
        assert_eq!((c.marks, c.content_marks), (0, 0));
    }
}
