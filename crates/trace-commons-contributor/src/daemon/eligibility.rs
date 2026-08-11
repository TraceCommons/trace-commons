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
}
