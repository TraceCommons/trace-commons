//! When is a local session finished enough to offer for upload?
//!
//! Neither Claude Code nor Codex writes an end-of-session marker, so the
//! daemon infers one. A session counts as finished when it has gone quiet for
//! the quiescence window *and* its size held steady across two consecutive
//! polls. The second condition matters because mtime granularity and clock
//! skew both lie, while a changing byte count does not.
//!
//! Re-uploading a grown session is deliberately bounded. The session hash
//! covers the whole file, so every re-upload re-sends all prior content: it
//! pays the privacy-filter bill again over the same text, and produces
//! near-identical envelopes that server-side duplicate clustering collapses,
//! diluting the contributor's own credit. So growth must be material, and it
//! can only happen a few times per session.
//!
//! Pure functions only. Everything here is decided from a stat result.

use chrono::{DateTime, Duration, Utc};
use std::path::PathBuf;

use super::settings::DaemonSettings;
use super::state::PriorUpload;

/// One stat of a session file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub modified_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Eligibility {
    Eligible,
    /// Written to too recently to be considered finished.
    NotQuiescent,
    /// Size changed since the previous poll, or this is the first sighting so
    /// there is no previous poll to compare against.
    Unstable,
    /// Already uploaded at exactly this size.
    AlreadyUploaded,
    /// Grew since the last upload, but not by enough to be worth re-sending
    /// the whole file.
    GrowthBelowThreshold,
    /// Grew materially, but this session has been re-uploaded enough times.
    ReuploadCapReached,
}

/// How long an armed project's session must have been quiet before the
/// daemon sends it with nobody looking.
///
/// Twenty-four hours, against the thirty minutes that decides when a session
/// is *offered*. The two windows answer different questions and only look
/// alike.
///
/// Thirty minutes is right for offering: the entry lands in the queue, a
/// contributor reads the preview whenever they next look, and they are the
/// last check. Thirty minutes of quiet is a lunch break, a meeting, or a
/// commute -- extremely common mid-session -- and under review that costs
/// nothing, because the human sees a half-finished trace and can decline it.
///
/// With nobody looking it costs a great deal. The session resumes, grows,
/// and takes the re-upload path: each re-upload re-sends the *whole* file,
/// pays the privacy filter again over the same text, and produces
/// near-identical envelopes that server-side duplicate clustering collapses
/// -- diluting the contributor's own credit. `max_reuploads` is 3, so it is
/// also a budget that premature sending burns.
///
/// A day rather than half of one because the commonest long pause is an
/// evening finish resumed the next morning. Twelve hours sends a session
/// last written at 6pm at 6am, just before a 9am resume -- long enough to
/// feel safe and short enough to lose exactly that case. Twenty-four hours
/// does not: the resume rewrites the file and restarts this clock.
///
/// Nothing is lost by waiting. An entry held here sits in the queue, and a
/// session that grows while it waits is handled by `Queue::supersede`, which
/// costs no re-upload budget and no duplicate penalty at all. Holding turns
/// the expensive path into the free one.
///
/// What this delays is a *session*, not the arming. The clock is each
/// observation's own `modified_at`, so arming a project whose sessions have
/// already been quiet for a day approves that backlog on the next poll --
/// there is no cool-off on the decision itself. Only sessions still being
/// written wait. `ProjectPolicy` does record the arming instant if a
/// cool-off on the decision is ever wanted; nothing reads it for that today.
///
/// Neither input is enforceable against the contributor: `now` is the wall
/// clock and `modified_at` is the file's own mtime, so one `touch -d` or one
/// clock nudge skips the window. That is acceptable because this protects a
/// contributor from sending something they have not finished, not from
/// themselves -- but it is not a control, and nothing should be built on it
/// as though it were.
pub const ARMED_SETTLE_SECS: i64 = 86_400;

/// Whether an armed project's session has been quiet long enough to send
/// unattended.
///
/// Read against the same `modified_at` the offer decision uses, which for a
/// grouped source is the whole group's most recent write -- a conversation
/// whose delegated transcript is still being written has not settled, and
/// must not be sent because its parent happens to have stopped.
pub fn armed_settle_elapsed(modified_at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    now.signed_duration_since(modified_at) >= Duration::seconds(ARMED_SETTLE_SECS)
}

/// Decide whether `obs` should be offered for upload.
///
/// `previous_size` is the size recorded at the previous poll; `prior` is what
/// was last uploaded for this path, if anything.
pub fn evaluate(
    obs: &Observation,
    previous_size: Option<u64>,
    prior: Option<&PriorUpload>,
    now: DateTime<Utc>,
    settings: &DaemonSettings,
) -> Eligibility {
    let quiet_for = now.signed_duration_since(obs.modified_at);
    if quiet_for < Duration::seconds(settings.quiescence_secs as i64) {
        return Eligibility::NotQuiescent;
    }

    // A first sighting has nothing to compare against, so it waits one poll
    // rather than risking a session that merely paused mid-write.
    if previous_size != Some(obs.size_bytes) {
        return Eligibility::Unstable;
    }

    let Some(prior) = prior else {
        return Eligibility::Eligible;
    };

    // Truncation or rotation: not growth, and not something to re-upload.
    if obs.size_bytes <= prior.size_bytes {
        return if obs.size_bytes == prior.size_bytes {
            Eligibility::AlreadyUploaded
        } else {
            Eligibility::GrowthBelowThreshold
        };
    }

    let factor_threshold = if (settings.growth_factor - 2.0).abs() < f64::EPSILON {
        prior.size_bytes.saturating_mul(2)
    } else {
        (prior.size_bytes as f64 * settings.growth_factor) as u64
    };
    let grew_by_factor = obs.size_bytes >= factor_threshold;
    let grew_absolutely =
        obs.size_bytes.saturating_sub(prior.size_bytes) >= settings.growth_min_new_bytes;
    if !(grew_by_factor || grew_absolutely) {
        return Eligibility::GrowthBelowThreshold;
    }

    if prior.upload_count >= settings.max_reuploads {
        return Eligibility::ReuploadCapReached;
    }

    Eligibility::Eligible
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    fn obs(size: u64, modified: &str) -> Observation {
        Observation {
            path: PathBuf::from("/tmp/s.jsonl"),
            size_bytes: size,
            modified_at: at(modified),
        }
    }

    fn uploaded(size: u64, count: u32) -> PriorUpload {
        PriorUpload {
            hash: "sha256:aa".into(),
            size_bytes: size,
            upload_count: count,
        }
    }

    #[test]
    fn not_quiescent_until_the_window_elapses() {
        let s = DaemonSettings::default();
        let o = obs(100, "2026-08-08T12:00:00Z");
        assert_eq!(
            evaluate(&o, Some(100), None, at("2026-08-08T12:20:00Z"), &s),
            Eligibility::NotQuiescent
        );
    }

    #[test]
    fn eligible_after_the_window_with_a_stable_size() {
        let s = DaemonSettings::default();
        let o = obs(100, "2026-08-08T12:00:00Z");
        assert_eq!(
            evaluate(&o, Some(100), None, at("2026-08-08T12:31:00Z"), &s),
            Eligibility::Eligible
        );
    }

    #[test]
    fn unstable_when_the_size_changed_since_the_previous_poll() {
        // An agent that paused long enough to look quiescent, then wrote
        // again, is still working.
        let s = DaemonSettings::default();
        let o = obs(200, "2026-08-08T12:00:00Z");
        assert_eq!(
            evaluate(&o, Some(100), None, at("2026-08-08T12:31:00Z"), &s),
            Eligibility::Unstable
        );
    }

    #[test]
    fn a_first_sighting_is_never_immediately_eligible() {
        let s = DaemonSettings::default();
        let o = obs(100, "2026-08-08T12:00:00Z");
        assert_eq!(
            evaluate(&o, None, None, at("2026-08-08T12:31:00Z"), &s),
            Eligibility::Unstable
        );
    }

    #[test]
    fn already_uploaded_at_the_same_size_is_not_requeued() {
        let s = DaemonSettings::default();
        let o = obs(100, "2026-08-08T12:00:00Z");
        assert_eq!(
            evaluate(
                &o,
                Some(100),
                Some(&uploaded(100, 1)),
                at("2026-08-08T12:31:00Z"),
                &s
            ),
            Eligibility::AlreadyUploaded
        );
    }

    #[test]
    fn growth_below_both_thresholds_is_rejected() {
        // 100 -> 150 is neither a doubling nor 64 KiB of new content.
        let s = DaemonSettings::default();
        let o = obs(150, "2026-08-08T12:00:00Z");
        assert_eq!(
            evaluate(
                &o,
                Some(150),
                Some(&uploaded(100, 1)),
                at("2026-08-08T12:31:00Z"),
                &s
            ),
            Eligibility::GrowthBelowThreshold
        );
    }

    #[test]
    fn doubling_in_size_requeues() {
        let s = DaemonSettings::default();
        let o = obs(200, "2026-08-08T12:00:00Z");
        assert_eq!(
            evaluate(
                &o,
                Some(200),
                Some(&uploaded(100, 1)),
                at("2026-08-08T12:31:00Z"),
                &s
            ),
            Eligibility::Eligible
        );
    }

    #[test]
    fn large_absolute_growth_requeues_without_doubling() {
        // The case that actually matters: a big session gaining real content.
        let s = DaemonSettings::default();
        let o = obs(1_065_536, "2026-08-08T12:00:00Z");
        assert_eq!(
            evaluate(
                &o,
                Some(1_065_536),
                Some(&uploaded(1_000_000, 1)),
                at("2026-08-08T12:31:00Z"),
                &s
            ),
            Eligibility::Eligible
        );
    }

    #[test]
    fn the_reupload_cap_stops_a_long_running_session() {
        let s = DaemonSettings::default();
        let o = obs(1000, "2026-08-08T12:00:00Z");
        assert_eq!(
            evaluate(
                &o,
                Some(1000),
                Some(&uploaded(100, 3)),
                at("2026-08-08T12:31:00Z"),
                &s
            ),
            Eligibility::ReuploadCapReached
        );
    }

    #[test]
    fn a_shrinking_file_is_never_eligible() {
        // Truncation or log rotation, not new work.
        let s = DaemonSettings::default();
        let o = obs(50, "2026-08-08T12:00:00Z");
        assert_eq!(
            evaluate(
                &o,
                Some(50),
                Some(&uploaded(100, 1)),
                at("2026-08-08T12:31:00Z"),
                &s
            ),
            Eligibility::GrowthBelowThreshold
        );
    }

    #[test]
    fn a_custom_growth_factor_is_honoured() {
        // growth_factor is a real knob, not decoration.
        let s = DaemonSettings {
            growth_factor: 1.5,
            growth_min_new_bytes: u64::MAX,
            ..Default::default()
        };
        let o = obs(150, "2026-08-08T12:00:00Z");
        assert_eq!(
            evaluate(
                &o,
                Some(150),
                Some(&uploaded(100, 1)),
                at("2026-08-08T12:31:00Z"),
                &s
            ),
            Eligibility::Eligible
        );
    }

    fn at2(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[test]
    fn an_armed_session_does_not_settle_inside_the_window() {
        let written = at2("2026-09-01T18:00:00Z");
        assert!(!armed_settle_elapsed(written, at2("2026-09-01T18:30:00Z")));
        assert!(!armed_settle_elapsed(written, at2("2026-09-02T06:00:00Z")));
        assert!(!armed_settle_elapsed(written, at2("2026-09-02T17:59:00Z")));
    }

    #[test]
    fn an_armed_session_settles_once_the_window_elapses() {
        let written = at2("2026-09-01T18:00:00Z");
        assert!(armed_settle_elapsed(written, at2("2026-09-02T18:00:00Z")));
        assert!(armed_settle_elapsed(written, at2("2026-09-03T09:00:00Z")));
    }

    /// The case the window exists for: a session finished in the evening and
    /// resumed the next morning. Twelve hours would have sent it at 06:00,
    /// three hours before the resume, and the resume would then have cost a
    /// re-upload and a duplicate penalty.
    #[test]
    fn an_overnight_resume_beats_the_window() {
        let finished_evening = at2("2026-09-01T18:00:00Z");
        assert!(!armed_settle_elapsed(
            finished_evening,
            at2("2026-09-02T09:00:00Z")
        ));
        // ...and the resume rewrites the file, which restarts the clock.
        let resumed = at2("2026-09-02T09:00:00Z");
        assert!(!armed_settle_elapsed(resumed, at2("2026-09-02T18:00:00Z")));
    }

    /// The settle window is longer than the offer window, and by a lot.
    /// If these ever converge, arming has quietly become "send after half an
    /// hour with nobody looking", which is the thing this prevents.
    #[test]
    fn the_settle_window_far_outlasts_the_quiescence_window() {
        let settings = DaemonSettings::default();
        assert!(
            ARMED_SETTLE_SECS >= settings.quiescence_secs as i64 * 40,
            "settle {} vs quiescence {}",
            ARMED_SETTLE_SECS,
            settings.quiescence_secs
        );
    }

    /// A clock that went backwards must never look like a settled session.
    #[test]
    fn a_future_write_never_counts_as_settled() {
        assert!(!armed_settle_elapsed(
            at2("2026-09-03T00:00:00Z"),
            at2("2026-09-01T00:00:00Z")
        ));
    }
}
