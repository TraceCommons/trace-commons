//! Telling the contributor there is something to look at, without becoming
//! noise.
//!
//! Notifications batch. A busy day across several repositories should be one
//! interruption, not a dozen, and the queue is durable so nothing is lost by
//! not interrupting: an ignored digest costs nothing.
//!
//! The daemon owns the *decision* to notify, because that policy is shared by
//! every application that attaches to it. Delivery is a `digest_due` event on
//! the subscription stream, which is the path the native applications use.
//! The local shell-out below exists so the daemon is useful on its own before
//! any of them ship, and is off unless explicitly enabled.

use std::collections::BTreeSet;

use chrono::{DateTime, Duration, Utc};

use super::queue::QueueEntry;
use super::settings::DigestSchedule;

/// Whether a digest should fire now.
///
/// Never fires with nothing to say: a notification that says nothing is worse
/// than silence.
///
/// "Nothing to say" used to mean an empty queue alone, which was right while
/// every upload passed through review -- an empty queue then meant an idle
/// period. It stopped being right once a project could be armed to contribute
/// without asking: an armed project never queues anything, so `pending` stays
/// 0 no matter how much it sends, and the contributor who most wanted to stop
/// supervising heard nothing at all. Contribution is now a reason to speak in
/// its own right.
///
/// `contributed` does not get its own clock. The interval is the whole point
/// of a digest -- one interruption per period, whatever the period held.
pub fn digest_due(
    last_digest_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    interval_secs: u64,
    pending: usize,
    contributed: usize,
) -> bool {
    if pending == 0 && contributed == 0 {
        return false;
    }
    interval_elapsed(last_digest_at, now, interval_secs)
}

/// The clock half of [`digest_due`], on its own.
///
/// `digest_due` needs a contribution count, and getting one means reading and
/// parsing the history file. That is wasted on almost every tick: the poll
/// runs far more often than the digest interval, and no count can make a
/// digest fire before the interval has elapsed. Callers use this first and
/// only pay for the history read when the answer could matter.
///
/// Deliberately the same expression `digest_due` uses rather than a copy of
/// it -- a pre-check that disagreed with the real predicate would suppress
/// digests that were genuinely due, which is the failure this whole path
/// exists to prevent.
pub fn interval_elapsed(
    last_digest_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    interval_secs: u64,
) -> bool {
    match last_digest_at {
        None => true,
        Some(last) => now.signed_duration_since(last) >= Duration::seconds(interval_secs as i64),
    }
}

/// Resolve a naive local instant against `tz`, choosing a definite answer for
/// the two cases a DST transition can produce.
///
/// `Ambiguous` (a fall-back repeats a local hour) picks the EARLIER of the two
/// occurrences. That choice only has to be self-consistent, because
/// [`evening_window_elapsed`] compares `now` against the same function's
/// answer for "today's target" -- picking the later occurrence would work
/// just as well, as long as every caller agrees, which is why one function
/// makes the choice rather than leaving it to each call site.
///
/// `None` (a spring-forward skips a local hour, so the naive target never
/// happened) walks forward a minute at a time until a real instant is found.
/// A one-hour spring-forward gap is the largest in real use, so this always
/// terminates quickly; the bound below is generous rather than tight.
fn resolve_local<Tz: chrono::TimeZone>(tz: &Tz, naive: chrono::NaiveDateTime) -> DateTime<Utc> {
    use chrono::LocalResult;
    match tz.from_local_datetime(&naive) {
        LocalResult::Single(dt) => dt.with_timezone(&Utc),
        LocalResult::Ambiguous(earliest, _latest) => earliest.with_timezone(&Utc),
        LocalResult::None => {
            let mut candidate = naive;
            for _ in 0..240 {
                candidate += Duration::minutes(1);
                if let LocalResult::Single(dt) = tz.from_local_datetime(&candidate) {
                    return dt.with_timezone(&Utc);
                }
            }
            // Never reached in any real zone (the widest spring-forward gap
            // is two hours); a naive UTC reading beats panicking.
            DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc)
        }
    }
}

/// `hour:00:00` on `now`'s local calendar date, in `tz`, as a UTC instant.
fn evening_target_utc<Tz: chrono::TimeZone>(
    now: DateTime<Utc>,
    hour: u8,
    tz: &Tz,
) -> DateTime<Utc> {
    let local_date = now.with_timezone(tz).date_naive();
    let naive = local_date
        .and_hms_opt(u32::from(hour.min(23)), 0, 0)
        .expect("hour is clamped to 0..=23");
    resolve_local(tz, naive)
}

/// The next evening target strictly after `now`: today's if it has not
/// passed yet, otherwise tomorrow's. For `status.next_digest_at` under
/// `DigestSchedule::Evening` -- unlike [`evening_window_elapsed`], this does
/// not need a `last_digest_at` because it only ever looks forward.
pub fn next_evening_at<Tz: chrono::TimeZone>(
    now: DateTime<Utc>,
    hour: u8,
    tz: &Tz,
) -> DateTime<Utc> {
    let today = evening_target_utc(now, hour, tz);
    if now < today {
        return today;
    }
    let tomorrow = now.with_timezone(tz).date_naive() + Duration::days(1);
    let naive = tomorrow
        .and_hms_opt(u32::from(hour.min(23)), 0, 0)
        .expect("hour is clamped to 0..=23");
    resolve_local(tz, naive)
}

/// The clock half of the evening schedule, parallel to [`interval_elapsed`].
///
/// Fires once `now` reaches today's local `hour`, and not again the same
/// day: the second half of the `&&` is false as soon as a digest has fired
/// at or after today's target, however many times this is called afterward.
///
/// A missed window -- the daemon asleep, paused, or simply not polled across
/// one or more evenings -- fires exactly once, at the next call after the
/// most recent target has passed, never once per missed day: `last_digest_at`
/// only has to be older than TODAY's target, no matter how many evenings
/// separate them.
///
/// DST is handled by delegating every local/UTC conversion to `tz` itself
/// (via [`evening_target_utc`]) rather than adding or subtracting a fixed
/// offset, so a fall change does not fire a second time and a spring change
/// does not silently skip a day.
#[must_use]
pub fn evening_window_elapsed<Tz: chrono::TimeZone>(
    last_digest_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    hour: u8,
    tz: &Tz,
) -> bool {
    let target = evening_target_utc(now, hour, tz);
    if now < target {
        return false;
    }
    match last_digest_at {
        None => true,
        Some(last) => last < target,
    }
}

/// [`digest_due`], for [`DigestSchedule::Evening`]. Never fires with nothing
/// to say, exactly as the interval version does.
#[must_use]
pub fn evening_digest_due<Tz: chrono::TimeZone>(
    last_digest_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    hour: u8,
    tz: &Tz,
    pending: usize,
    contributed: usize,
) -> bool {
    if pending == 0 && contributed == 0 {
        return false;
    }
    evening_window_elapsed(last_digest_at, now, hour, tz)
}

/// The clock half of [`digest_due_for_schedule`], dispatching on
/// [`DigestSchedule`] so the cheap pre-check callers use before reading
/// history (see [`interval_elapsed`]'s doc) can never disagree with the real
/// predicate about which schedule is in force.
#[must_use]
pub fn schedule_elapsed<Tz: chrono::TimeZone>(
    schedule: DigestSchedule,
    last_digest_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    interval_secs: u64,
    tz: &Tz,
) -> bool {
    match schedule {
        DigestSchedule::Interval => interval_elapsed(last_digest_at, now, interval_secs),
        DigestSchedule::Evening { hour } => evening_window_elapsed(last_digest_at, now, hour, tz),
    }
}

/// [`digest_due`], generalised over [`DigestSchedule`] (K9, #1118 open
/// decision #5: interval vs. a fixed evening time). `Interval` behaves
/// exactly as [`digest_due`] does -- this is the same expression, not a
/// second opinion of it -- so choosing it changes nothing for an existing
/// install.
#[must_use]
pub fn digest_due_for_schedule<Tz: chrono::TimeZone>(
    schedule: DigestSchedule,
    last_digest_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    interval_secs: u64,
    tz: &Tz,
    pending: usize,
    contributed: usize,
) -> bool {
    if pending == 0 && contributed == 0 {
        return false;
    }
    schedule_elapsed(schedule, last_digest_at, now, interval_secs, tz)
}

/// The digest line: how many sessions, and which projects they came from.
///
/// Built from labels only. A notification is rendered by a desktop
/// environment, may be logged by it, and must never contain a path.
pub fn digest_text(pending: &[&QueueEntry]) -> String {
    let count = pending.len();
    let noun = if count == 1 { "session" } else { "sessions" };
    let projects: BTreeSet<&str> = pending
        .iter()
        .map(|e| e.project_label.as_str())
        .filter(|l| !l.is_empty())
        .collect();
    if projects.is_empty() {
        return format!("{count} {noun} ready to contribute");
    }
    let named: Vec<&str> = projects.iter().take(3).copied().collect();
    let more = projects.len().saturating_sub(named.len());
    let list = named.join(", ");
    if more > 0 {
        format!("{count} {noun} ready to contribute from {list} and {more} more")
    } else {
        format!("{count} {noun} ready to contribute from {list}")
    }
}

/// The contribution half of the digest: what went out without being asked
/// about since the last one, and what credit it is carrying.
///
/// Built from labels only, for the same reason `digest_text` is: a
/// notification is rendered by a desktop environment, may be logged by it,
/// and must never contain a path.
///
/// Credit is stated only when there is some. A trailing "0 credit pending"
/// reads as a failure rather than as a fresh start, and the first digest
/// after arming a project is exactly when that would show. It is always
/// named `pending`, never "earned" -- settlement is off on every deployment
/// shipped so far (see `docs/operator/settlement-mode.md`), so a bare figure
/// would be read as money that exists.
///
/// **DRAFT, NEEDS APPROVAL** on the project clause: K9 (#1118) rewords it to
/// match the WYSIWYG design's Flow 2/3 evening-digest alerts exactly --
/// "1 session contributed from orchard-api. 6.0 credit pending." and
/// "2 sessions contributed. 10.5 credit pending." The design names a project
/// only when there is exactly one; contributions spread across more than one
/// project say only the count, with no partial list and no "and N more" --
/// unlike [`digest_text`]'s pending-queue sentence, which still names up to
/// three. A single project is still the common case this sentence exists
/// for (an armed folder usually contributes from one place at a time), so
/// the one-project rule is not expected to make this line silent about
/// where things came from as often as it looks.
pub fn contribution_text(count: usize, projects: &BTreeSet<String>, credit_pending: f32) -> String {
    let noun = if count == 1 { "session" } else { "sessions" };
    let mut line = format!("{count} {noun} contributed");
    if projects.len() == 1 {
        if let Some(only) = projects.iter().next() {
            line.push_str(&format!(" from {only}"));
        }
    }
    // One decimal place: credit is a score, not an amount, and trailing
    // precision invites the reader to treat it as a balance.
    //
    // Rounded explicitly, half away from zero, rather than left to each
    // language's default: Rust's `{:.1}` rounds half to even and .NET's
    // "0.0" rounds half away from zero, so 4.25 rendered as 4.2 here and
    // 4.3 on Windows -- the same contribution, a different figure depending
    // which machine the contributor read it on. `f32::round` is
    // half-away-from-zero, and the other two shells now round the same way
    // before formatting.
    if credit_pending > 0.0 {
        let rounded = (credit_pending * 10.0).round() / 10.0;
        line.push_str(&format!(". {rounded:.1} credit pending."));
    }
    line
}

/// Best-effort local OS notification.
///
/// Never fails the pipeline: a missing notifier, a headless machine, or a
/// daemon with no desktop session is a logged label, not a failed upload.
/// The text is passed as one argument, never interpolated into a shell.
pub fn emit_local(text: &str) {
    #[cfg(target_os = "macos")]
    let attempt = std::process::Command::new("osascript")
        .arg("-e")
        .arg(format!(
            "display notification {} with title \"Trace Commons\"",
            applescript_string(text)
        ))
        .output();

    #[cfg(target_os = "linux")]
    let attempt = std::process::Command::new("notify-send")
        .arg("Trace Commons")
        .arg(text)
        .output();

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let attempt: std::io::Result<std::process::Output> = {
        let _ = text;
        Err(std::io::Error::other("unsupported-platform"))
    };

    match attempt {
        Ok(out) if out.status.success() => {}
        _ => tracing::debug!("notifier-unavailable"),
    }
}

/// Quote a string for embedding in an AppleScript literal.
#[cfg(target_os = "macos")]
fn applescript_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::queue::{QueueEntry, entry_id_for};
    use std::path::PathBuf;

    use crate::daemon::test_support::at;

    fn labels(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|n| (*n).to_string()).collect()
    }

    fn entry(label: &str) -> QueueEntry {
        QueueEntry {
            entry_id: entry_id_for(label),
            session_hash: format!("sha256:{label}"),
            source: "claude-code".into(),
            project_key: format!("/Users/z/code/{label}"),
            project_label: label.into(),
            path: PathBuf::from("/Users/z/.claude/projects/x/s.jsonl"),
            size_bytes: 10,
            discovered_at: at("2026-08-08T12:00:00Z"),
            ..Default::default()
        }
    }

    #[test]
    fn a_digest_is_not_due_before_the_interval_elapses() {
        assert!(!digest_due(
            Some(at("2026-08-08T12:00:00Z")),
            at("2026-08-08T14:00:00Z"),
            14400,
            3,
            0
        ));
    }

    #[test]
    fn a_digest_is_due_after_the_interval_with_pending_work() {
        assert!(digest_due(
            Some(at("2026-08-08T12:00:00Z")),
            at("2026-08-08T16:01:00Z"),
            14400,
            3,
            0
        ));
    }

    #[test]
    fn a_digest_is_never_due_with_nothing_to_say() {
        assert!(!digest_due(
            Some(at("2026-08-08T12:00:00Z")),
            at("2026-08-09T12:00:00Z"),
            14400,
            0,
            0
        ));
        assert!(!digest_due(None, at("2026-08-08T12:00:00Z"), 14400, 0, 0));
    }

    #[test]
    fn the_first_digest_is_due_immediately_when_work_exists() {
        assert!(digest_due(None, at("2026-08-08T12:00:00Z"), 14400, 1, 0));
    }

    /// The hole this closes. An armed project uploads without ever queuing
    /// anything, so `pending` stays 0 forever and the old gate refused every
    /// digest -- a contributor who armed everything heard nothing at all,
    /// which is the opposite of what arming is supposed to feel like.
    #[test]
    fn a_digest_is_due_for_contributions_alone_with_an_empty_queue() {
        assert!(digest_due(
            Some(at("2026-08-08T12:00:00Z")),
            at("2026-08-08T16:01:00Z"),
            14400,
            0,
            7
        ));
        assert!(digest_due(None, at("2026-08-08T12:00:00Z"), 14400, 0, 1));
    }

    /// Contributions do not get their own faster clock. The interval is the
    /// whole point of a digest: one interruption per period, whatever the
    /// period contained.
    #[test]
    fn contributions_do_not_shorten_the_interval() {
        assert!(!digest_due(
            Some(at("2026-08-08T12:00:00Z")),
            at("2026-08-08T14:00:00Z"),
            14400,
            0,
            99
        ));
    }

    #[test]
    fn contribution_text_names_the_one_project_and_never_a_path() {
        let text = contribution_text(3, &labels(&["orchard-api"]), 0.0);
        assert!(text.contains("3 sessions contributed"), "{text}");
        assert!(text.contains("orchard-api"), "{text}");
        assert!(!text.contains('/'), "must not contain a path: {text}");
    }

    #[test]
    fn contribution_text_is_singular_for_one_session() {
        let text = contribution_text(1, &labels(&["proj"]), 0.0);
        assert!(text.contains("1 session contributed"), "{text}");
        assert!(!text.contains("sessions"), "{text}");
    }

    /// K9 (#1118): the design's second Flow 3 alert has no project name at
    /// all when more than one contributed -- no partial list, no "and N
    /// more", unlike the pending-queue sentence `digest_text` still gives.
    #[test]
    fn contribution_text_omits_projects_when_more_than_one() {
        let text = contribution_text(
            9,
            &labels(&["orchard-api", "acme-billing", "portfolio"]),
            0.0,
        );
        // The whole sentence, not a substring check: proves nothing beyond
        // the count and noun survives, no partial list and no "and N more".
        assert_eq!(text, "9 sessions contributed");
    }

    #[test]
    fn contribution_text_copes_with_missing_labels() {
        assert_eq!(
            contribution_text(2, &labels(&[]), 0.0),
            "2 sessions contributed"
        );
    }

    /// The WYSIWYG design's own worked examples (Flow 2's evening digest),
    /// verbatim. See `consent_copy`'s doc style for why exact strings like
    /// these are pinned rather than approximated with `contains`.
    #[test]
    fn contribution_text_matches_the_design_flow_2_example() {
        assert_eq!(
            contribution_text(1, &labels(&["orchard-api"]), 6.0),
            "1 session contributed from orchard-api. 6.0 credit pending."
        );
    }

    /// Flow 3's evening digest example: two sessions, no project named.
    #[test]
    fn contribution_text_matches_the_design_flow_3_example() {
        assert_eq!(
            contribution_text(2, &labels(&["orchard-api", "portfolio"]), 10.5),
            "2 sessions contributed. 10.5 credit pending."
        );
    }

    /// Credit is the other half of the value exchange and is the reason this
    /// line exists at all -- but only when there is some. A trailing
    /// "0 credit pending" reads as a failure rather than as a fresh start.
    #[test]
    fn contribution_text_states_credit_only_when_there_is_some() {
        let with = contribution_text(2, &labels(&["proj"]), 4.25);
        assert!(with.contains("4.3 credit pending"), "{with}");
        let without = contribution_text(2, &labels(&["proj"]), 0.0);
        assert!(!without.contains("credit"), "{without}");
    }

    /// The figure is pending, and saying so is the difference between a
    /// record and a promise. Settlement is off on every deployment shipped
    /// so far, so a bare number would be read as money.
    #[test]
    fn contribution_text_never_calls_pending_credit_earned() {
        let text = contribution_text(2, &labels(&["proj"]), 4.25);
        assert!(text.contains("pending"), "{text}");
        for word in ["earned", "paid", "settled", "worth"] {
            assert!(!text.contains(word), "must not say {word}: {text}");
        }
    }

    #[test]
    fn digest_text_names_projects_and_never_a_path() {
        let a = entry("proj");
        let b = entry("proj");
        let c = entry("other");
        let text = digest_text(&[&a, &b, &c]);
        assert!(text.contains("3 sessions"), "{text}");
        assert!(text.contains("proj"), "{text}");
        assert!(text.contains("other"), "{text}");
        assert!(
            !text.contains('/'),
            "digest text must not contain a path: {text}"
        );
    }

    #[test]
    fn digest_text_is_singular_for_one_session() {
        let a = entry("proj");
        let text = digest_text(&[&a]);
        assert!(text.contains("1 session ready"), "{text}");
        assert!(!text.contains("sessions"), "{text}");
    }

    #[test]
    fn digest_text_summarises_rather_than_listing_every_project() {
        let entries: Vec<QueueEntry> = ["a", "b", "c", "d", "e"].iter().map(|l| entry(l)).collect();
        let refs: Vec<&QueueEntry> = entries.iter().collect();
        let text = digest_text(&refs);
        assert!(text.contains("and 2 more"), "{text}");
    }

    #[test]
    fn digest_text_copes_with_missing_labels() {
        let mut a = entry("proj");
        a.project_label = String::new();
        let text = digest_text(&[&a]);
        assert_eq!(text, "1 session ready to contribute");
    }

    /// The poll loop uses `interval_elapsed` to decide whether reading the
    /// history file is worth it, then `digest_due` to decide whether to
    /// speak. If the first were ever stricter than the second, a digest that
    /// was genuinely due would be skipped with no count to explain why --
    /// silence, which is the exact failure the contribution half of the
    /// digest exists to prevent.
    ///
    /// So: whenever there is something to say, the two must agree exactly.
    #[test]
    fn interval_elapsed_never_suppresses_a_due_digest() {
        let now = at("2026-08-08T12:00:00Z");
        let intervals = [0u64, 1, 3600, 14400, 86400];
        let lasts = [
            None,
            Some(at("2026-08-08T12:00:00Z")),
            Some(at("2026-08-08T11:59:59Z")),
            Some(at("2026-08-08T11:00:00Z")),
            Some(at("2026-08-08T08:00:00Z")),
            Some(at("2026-08-07T12:00:00Z")),
        ];

        for interval in intervals {
            for last in lasts {
                let gate = interval_elapsed(last, now, interval);
                for (pending, contributed) in [(1usize, 0usize), (0, 1), (1, 1), (3, 2)] {
                    assert_eq!(
                        digest_due(last, now, interval, pending, contributed),
                        gate,
                        "interval={interval} last={last:?} pending={pending} \
                         contributed={contributed}: the pre-check and the real \
                         predicate disagree"
                    );
                }
                // And with nothing to say, the pre-check may be true while
                // digest_due is false -- that direction only costs a wasted
                // history read, never a missed digest.
                assert!(!digest_due(last, now, interval, 0, 0));
            }
        }
    }

    // -----------------------------------------------------------------
    // K9 (#1118): the evening digest schedule, open decision #5.
    // -----------------------------------------------------------------
    //
    // `chrono::Utc` stands in for a plain, DST-free local timezone in the
    // tests that are not specifically about a DST transition, so those
    // tests read as ordinary clock arithmetic. The DST tests below use
    // `chrono_tz::America::New_York`, a real zone with known 2026
    // transitions, rather than the CI runner's own timezone -- the whole
    // point of taking `Tz` generically is that a test never has to depend
    // on where it runs.

    #[test]
    fn evening_digest_is_not_due_before_the_hour() {
        assert!(!evening_window_elapsed(
            None,
            at("2026-08-08T17:59:00Z"),
            18,
            &Utc,
        ));
    }

    #[test]
    fn evening_digest_is_due_once_the_hour_arrives() {
        assert!(evening_window_elapsed(
            None,
            at("2026-08-08T18:00:00Z"),
            18,
            &Utc,
        ));
        // Later the same evening, still due if it has not fired yet.
        assert!(evening_window_elapsed(
            None,
            at("2026-08-08T23:30:00Z"),
            18,
            &Utc,
        ));
    }

    /// Fires once, not on every poll for the rest of the evening.
    #[test]
    fn evening_digest_does_not_fire_twice_the_same_day() {
        let fired_at = at("2026-08-08T18:00:00Z");
        assert!(!evening_window_elapsed(
            Some(fired_at),
            at("2026-08-08T18:00:01Z"),
            18,
            &Utc,
        ));
        assert!(!evening_window_elapsed(
            Some(fired_at),
            at("2026-08-08T23:59:00Z"),
            18,
            &Utc,
        ));
    }

    /// The next evening is a fresh window: firing yesterday does not hold
    /// off tonight's digest.
    #[test]
    fn evening_digest_fires_again_the_following_evening() {
        assert!(evening_window_elapsed(
            Some(at("2026-08-08T18:00:00Z")),
            at("2026-08-09T18:00:00Z"),
            18,
            &Utc,
        ));
    }

    /// The daemon was asleep, paused, or simply not polled across one or
    /// more whole evenings. The next check fires exactly once -- at the
    /// next opportunity -- never once per missed evening.
    #[test]
    fn evening_digest_fires_once_after_missing_several_days() {
        let last = at("2026-08-05T10:00:00Z");
        // Three evenings passed with nobody home to hear them.
        assert!(evening_window_elapsed(
            Some(last),
            at("2026-08-08T19:00:00Z"),
            18,
            &Utc,
        ));
    }

    #[test]
    fn evening_digest_never_fires_with_nothing_to_say() {
        assert!(!evening_digest_due(
            None,
            at("2026-08-08T18:00:00Z"),
            18,
            &Utc,
            0,
            0,
        ));
        assert!(evening_digest_due(
            None,
            at("2026-08-08T18:00:00Z"),
            18,
            &Utc,
            1,
            0,
        ));
    }

    /// Spring forward: America/New_York jumps from 02:00 to 03:00 local on
    /// 2026-03-08, so the naive local instant `02:00` that day never
    /// happens. A schedule asking for that hour must still resolve to a
    /// single, later UTC instant instead of panicking or silently skipping
    /// the day.
    #[test]
    fn evening_digest_handles_the_spring_forward_gap() {
        let tz = chrono_tz::America::New_York;
        // 02:00 local does not exist; the first local instant that does is
        // 03:00 EDT, which is 07:00 UTC.
        let resolved = evening_target_utc(at("2026-03-08T12:00:00Z"), 2, &tz);
        assert_eq!(resolved, at("2026-03-08T07:00:00Z"));

        assert!(!evening_window_elapsed(
            None,
            at("2026-03-08T06:59:00Z"),
            2,
            &tz,
        ));
        assert!(evening_window_elapsed(None, resolved, 2, &tz));
        // Does not fire twice on the transition day itself.
        assert!(!evening_window_elapsed(
            Some(resolved),
            at("2026-03-08T10:00:00Z"),
            2,
            &tz,
        ));
    }

    /// Fall back: America/New_York repeats 01:00-01:59 local on
    /// 2026-11-01. The earlier occurrence (still EDT, 05:00 UTC) is what a
    /// schedule resolves to, consistently, so the repeated local hour does
    /// not read as a second window.
    #[test]
    fn evening_digest_handles_the_fall_back_ambiguity() {
        let tz = chrono_tz::America::New_York;
        let resolved = evening_target_utc(at("2026-11-01T12:00:00Z"), 1, &tz);
        assert_eq!(resolved, at("2026-11-01T05:00:00Z"));

        assert!(!evening_window_elapsed(
            None,
            at("2026-11-01T04:59:00Z"),
            1,
            &tz,
        ));
        assert!(evening_window_elapsed(None, resolved, 1, &tz));
        // The local hour repeats (05:00-06:00 UTC is 01:00 EDT then 01:00
        // EST again), but it is still one window: no second fire in it.
        assert!(!evening_window_elapsed(
            Some(resolved),
            at("2026-11-01T05:45:00Z"),
            1,
            &tz,
        ));
    }

    #[test]
    fn next_evening_at_is_today_before_the_hour_and_tomorrow_after() {
        assert_eq!(
            next_evening_at(at("2026-08-08T10:00:00Z"), 18, &Utc),
            at("2026-08-08T18:00:00Z")
        );
        assert_eq!(
            next_evening_at(at("2026-08-08T18:00:00Z"), 18, &Utc),
            at("2026-08-09T18:00:00Z")
        );
        assert_eq!(
            next_evening_at(at("2026-08-08T23:59:00Z"), 18, &Utc),
            at("2026-08-09T18:00:00Z")
        );
    }

    #[test]
    fn schedule_elapsed_dispatches_on_the_schedule() {
        let last = Some(at("2026-08-08T12:00:00Z"));
        let now = at("2026-08-08T18:00:00Z");
        assert_eq!(
            schedule_elapsed(DigestSchedule::Interval, last, now, 14_400, &Utc),
            interval_elapsed(last, now, 14_400),
        );
        assert_eq!(
            schedule_elapsed(
                DigestSchedule::Evening { hour: 18 },
                last,
                now,
                14_400,
                &Utc
            ),
            evening_window_elapsed(last, now, 18, &Utc),
        );
    }

    /// The default schedule changes nothing: `digest_due_for_schedule` under
    /// `Interval` must agree with the plain `digest_due` on every input, for
    /// every install that never opts into `Evening`.
    #[test]
    fn digest_due_for_schedule_interval_matches_digest_due_exactly() {
        let now = at("2026-08-08T16:01:00Z");
        for last in [None, Some(at("2026-08-08T12:00:00Z"))] {
            for (pending, contributed) in [(0usize, 0usize), (1, 0), (0, 1), (3, 2)] {
                assert_eq!(
                    digest_due_for_schedule(
                        DigestSchedule::Interval,
                        last,
                        now,
                        14_400,
                        &Utc,
                        pending,
                        contributed,
                    ),
                    digest_due(last, now, 14_400, pending, contributed),
                    "last={last:?} pending={pending} contributed={contributed}"
                );
            }
        }
    }

    #[test]
    fn digest_due_for_schedule_evening_matches_evening_digest_due() {
        let now = at("2026-08-08T18:00:00Z");
        for last in [None, Some(at("2026-08-08T12:00:00Z"))] {
            for (pending, contributed) in [(0usize, 0usize), (1, 0), (0, 1)] {
                assert_eq!(
                    digest_due_for_schedule(
                        DigestSchedule::Evening { hour: 18 },
                        last,
                        now,
                        14_400,
                        &Utc,
                        pending,
                        contributed,
                    ),
                    evening_digest_due(last, now, 18, &Utc, pending, contributed),
                    "last={last:?} pending={pending} contributed={contributed}"
                );
            }
        }
    }
}
