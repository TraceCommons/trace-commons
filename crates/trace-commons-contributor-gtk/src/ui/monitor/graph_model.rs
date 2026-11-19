//! Shared over kept, per period: the Traces graph's bucketing.
//!
//! A port of `TracesGraphModel` and the pure parts of `TracesGraphFooter`
//! in `macos/Sources/TraceCommonsApp/Views/Monitor/MonitorShell.swift`.
//! Shared counts sessions that left this machine (History, by submission
//! time); kept counts sessions still here (the queue, by discovery time).
//! Counts, not bytes. Pure, so the bucketing is tested.
//!
//! Previous, next and zoom change only the graph's range.
//!
//! Deliberate platform difference: the weekday and day labels are chrono's
//! (`%a`, `%A`, `%B`), which are not localized. The Mac formats them with
//! the system's `Date.FormatStyle`.

use chrono::{DateTime, Datelike, Duration, NaiveDateTime, TimeZone, Timelike, Utc};
use trace_commons_contributor::preview_copy::MonitorTracesGraphCopy;

/// The three ranges: 24 hours in 2-hour buckets, 11 days, and 42 days.
/// Zooming out widens the range.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Range {
    Hours,
    #[default]
    Days,
    Weeks,
}

impl Range {
    pub const ALL: [Range; 3] = [Range::Hours, Range::Days, Range::Weeks];

    /// How many buckets.
    pub const fn count(self) -> i64 {
        match self {
            Range::Hours => 12,
            Range::Days => 11,
            Range::Weeks => 42,
        }
    }

    /// A bucket's width: 2 hours, or a day.
    fn step(self) -> Duration {
        match self {
            Range::Hours => Duration::hours(2),
            Range::Days | Range::Weeks => Duration::days(1),
        }
    }

    /// The window's span as the range pill says it: 24 (hours), 11 or 42
    /// (days).
    pub const fn span(self) -> i64 {
        match self {
            Range::Hours => self.count() * 2,
            Range::Days | Range::Weeks => self.count(),
        }
    }

    /// One step wider, or `None` at the widest.
    pub const fn zoomed_out(self) -> Option<Range> {
        match self {
            Range::Hours => Some(Range::Days),
            Range::Days => Some(Range::Weeks),
            Range::Weeks => None,
        }
    }

    /// One step narrower, or `None` at the narrowest.
    pub const fn zoomed_in(self) -> Option<Range> {
        match self {
            Range::Hours => None,
            Range::Days => Some(Range::Hours),
            Range::Weeks => Some(Range::Days),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bucket {
    /// The bucket's start, as local wall-clock time in the graph's zone.
    pub start: NaiveDateTime,
    pub shared: usize,
    pub kept: usize,
    /// The label under its bar; empty where the Mac leaves one out.
    pub label: String,
}

/// History statuses that left this machine and still stand: withdrawn is
/// not shared any more.
pub const CONTRIBUTED_STATUSES: [&str; 3] = ["accepted", "submitted", "quarantined"];

/// When each contribution left, for one tool or all of them. `rows` is
/// `(status, tool, submitted_at)`.
pub fn shared_times<'a>(
    rows: impl IntoIterator<Item = (&'a str, Option<&'a str>, Option<DateTime<Utc>>)>,
    tool: Option<&str>,
) -> Vec<DateTime<Utc>> {
    rows.into_iter()
        .filter(|(status, _, _)| CONTRIBUTED_STATUSES.contains(status))
        .filter(|(_, source, _)| tool.is_none() || *source == tool)
        .filter_map(|(_, _, at)| at)
        .collect()
}

/// When each session still waiting was found, for one tool or all.
/// `sessions` is `(tool, discovered_at)`.
pub fn kept_times<'a>(
    sessions: impl IntoIterator<Item = (&'a str, Option<DateTime<Utc>>)>,
    tool: Option<&str>,
) -> Vec<DateTime<Utc>> {
    sessions
        .into_iter()
        .filter(|(source, _)| tool.is_none_or(|tool| tool == *source))
        .filter_map(|(_, at)| at)
        .collect()
}

/// The range's buckets ending in the current one (this hour's 2-hour
/// bucket, or today), `offset` whole windows back (0 is now, -1 the window
/// before it).
pub fn buckets<Tz: TimeZone>(
    shared: &[DateTime<Utc>],
    kept: &[DateTime<Utc>],
    offset: i64,
    now: DateTime<Tz>,
    range: Range,
) -> Vec<Bucket> {
    let zone = now.timezone();
    let local = now.naive_local();
    let anchor = if range == Range::Hours {
        local.date().and_hms_opt(local.hour(), 0, 0)
    } else {
        local.date().and_hms_opt(0, 0, 0)
    }
    .unwrap_or(local);
    // Bucketed on the wall clock, so a day is a calendar day across a
    // change of offset.
    let wall = |at: &DateTime<Utc>| at.with_timezone(&zone).naive_local();
    let shared: Vec<NaiveDateTime> = shared.iter().map(wall).collect();
    let kept: Vec<NaiveDateTime> = kept.iter().map(wall).collect();
    (0..range.count())
        .map(|index| {
            let back = range.count() - 1 - index - offset * range.count();
            let start = anchor - range.step() * back as i32;
            let end = start + range.step();
            let inside = |at: &&NaiveDateTime| **at >= start && **at < end;
            Bucket {
                start,
                shared: shared.iter().filter(inside).count(),
                kept: kept.iter().filter(inside).count(),
                label: label(range, start, index),
            }
        })
        .collect()
}

/// Every third 2-hour bucket's hour; every day's short weekday at 11 days;
/// every sixth day's at 42.
pub fn label(range: Range, start: NaiveDateTime, index: i64) -> String {
    match range {
        Range::Hours if index % 3 == 0 => format!("{}:00", start.hour()),
        Range::Days => weekday(start),
        Range::Weeks if index % 6 == 0 => weekday(start),
        Range::Hours | Range::Weeks => String::new(),
    }
}

/// "Sat": the short weekday.
pub fn weekday(at: NaiveDateTime) -> String {
    at.format("%a").to_string()
}

/// "Monday, October 5": a hovered bar's day.
pub fn day(at: NaiveDateTime) -> String {
    // The weekday, the month and the day of the month, each formatted by
    // itself: no pattern here reads as a phrase.
    format!("{}, {} {}", at.format("%A"), at.format("%B"), at.day())
}

/// A count, or a dash while History has not been read: an unread record is
/// never drawn as nothing shared.
pub fn figure(count: usize, known: bool) -> String {
    if known {
        count.to_string()
    } else {
        "\u{2014}".to_string()
    }
}

/// The range pill: the hovered bar's day, else the window in the core's
/// words ("Last 11 days", "11 days, 2 windows back").
pub fn range_label(
    range: Range,
    offset: i64,
    hovered: Option<NaiveDateTime>,
    words: &MonitorTracesGraphCopy,
) -> String {
    if let Some(hovered) = hovered {
        return day(hovered);
    }
    let hours = range == Range::Hours;
    let back = -offset;
    let template = match (hours, back) {
        (true, 0) => words.last_hours,
        (false, 0) => words.last_days,
        (true, 1) => words.hours_back_one,
        (false, 1) => words.days_back_one,
        (true, _) => words.hours_back,
        (false, _) => words.days_back,
    };
    template
        .replace("{hours}", &range.span().to_string())
        .replace("{days}", &range.span().to_string())
        .replace("{count}", &back.to_string())
}

/// A bar's text equivalent, in the core's words.
pub fn bar_description(bucket: &Bucket, words: &MonitorTracesGraphCopy) -> String {
    words
        .bar
        .replace("{label}", &day(bucket.start))
        .replace("{count}", &bucket.shared.to_string())
        .replace("{total}", &bucket.kept.to_string())
}

/// The graph's state: its range and how many whole windows back it shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Window {
    pub range: Range,
    /// 0 is now; negative is earlier. Never positive.
    pub offset: i64,
}

impl Window {
    /// A period step. The graph never goes past now.
    pub fn stepped(self, by: i64) -> Self {
        Self {
            offset: (self.offset + by).min(0),
            ..self
        }
    }

    /// A zoom resets to now. `None` (already at the end) changes nothing.
    pub fn zoomed(self, to: Option<Range>) -> Self {
        match to {
            Some(range) => Self { range, offset: 0 },
            None => self,
        }
    }

    pub fn jumped_to_now(self) -> Self {
        Self { offset: 0, ..self }
    }

    /// Next and jump-to-now have nowhere to go at now.
    pub fn at_now(self) -> bool {
        self.offset == 0
    }
}

/// The focus button's tip: nothing selected, focused, or the tool it would
/// show (`MonitorShellCopy.focusTip`).
pub fn focus_tip(tool: Option<&str>, focused: bool) -> String {
    let shell = trace_commons_contributor::preview_copy::monitor_screens_copy().shell;
    match tool {
        None => shell.focus_needs_selection.to_string(),
        Some(_) if focused => shell.focus_whole_map.to_string(),
        Some(tool) => shell.focus_tool.replace("{tool}", tool),
    }
}

#[cfg(test)]
mod tests {
    //! The graph cases of
    //! `macos/Tests/TraceCommonsAppTests/MonitorShellTests.swift`.
    use super::*;
    use trace_commons_contributor::preview_copy::monitor_screens_copy;

    /// 2025-10-09 08:53:20 UTC, the instant the Swift tests fix.
    fn now() -> DateTime<Utc> {
        Utc.timestamp_opt(1_760_000_000, 0).unwrap()
    }

    fn today() -> DateTime<Utc> {
        Utc.from_utc_datetime(&now().date_naive().and_hms_opt(0, 0, 0).unwrap())
    }

    #[test]
    fn the_graph_buckets_count_per_day() {
        let day = Duration::days(1);
        let minute = Duration::seconds(60);
        let shared = [today() + minute, today() - day + minute, today() - day * 20];
        let kept = [today() + minute * 2, today() + minute * 2];
        let buckets_now = buckets(&shared, &kept, 0, now(), Range::Days);
        assert_eq!(buckets_now.len(), 11);
        let last = buckets_now.last().unwrap();
        assert_eq!(last.start, today().naive_utc());
        assert_eq!(last.shared, 1);
        assert_eq!(last.kept, 2);
        assert_eq!(buckets_now[buckets_now.len() - 2].shared, 1);
        assert_eq!(
            buckets_now.iter().map(|b| b.shared).sum::<usize>(),
            2,
            "twenty days back is outside the window"
        );
        let earlier = buckets(&shared, &kept, -1, now(), Range::Days);
        assert_eq!(
            earlier.last().unwrap().start,
            (today() - day * 11).naive_utc()
        );
        assert_eq!(earlier.iter().map(|b| b.shared).sum::<usize>(), 1);
    }

    #[test]
    fn the_graph_zooms_between_three_ranges() {
        let hour = now().date_naive().and_hms_opt(8, 0, 0).unwrap();
        let hours = buckets(&[now()], &[], 0, now(), Range::Hours);
        assert_eq!(hours.len(), 12);
        assert_eq!(hours.last().unwrap().start, hour);
        assert_eq!(hours.last().unwrap().shared, 1);
        assert_eq!(hours[0].label, format!("{}:00", hours[0].start.hour()));
        assert_eq!(hours[1].label, "", "every third hour bucket is labelled");
        assert_eq!((hour - hours[0].start).num_hours(), 22);

        let days = buckets(&[], &[], 0, now(), Range::Days);
        assert_eq!(days.len(), 11);
        assert!(days.iter().all(|b| !b.label.is_empty()));
        assert_eq!(days.last().unwrap().label, weekday(now().naive_utc()));
        assert_eq!(days.last().unwrap().label, "Thu");

        let weeks = buckets(&[], &[], -1, now(), Range::Weeks);
        assert_eq!(weeks.len(), 42);
        assert_eq!(
            weeks.iter().filter(|b| !b.label.is_empty()).count(),
            7,
            "every sixth day is labelled"
        );
        assert_eq!(
            weeks.last().unwrap().start,
            (today() - Duration::days(42)).naive_utc()
        );

        assert_eq!(Range::Days.zoomed_out(), Some(Range::Weeks));
        assert_eq!(Range::Weeks.zoomed_out(), None);
        assert_eq!(Range::Days.zoomed_in(), Some(Range::Hours));
        assert_eq!(Range::Hours.zoomed_in(), None);
        assert_eq!(Range::ALL.map(Range::span), [24, 11, 42]);
        assert!(Range::Hours < Range::Days && Range::Days < Range::Weeks);
        assert_eq!(Range::default(), Range::Days);
    }

    #[test]
    fn a_day_is_a_calendar_day_in_the_graphs_own_zone() {
        // 23:30 UTC on the 8th is 01:30 on the 9th two hours east.
        let east = chrono::FixedOffset::east_opt(2 * 3600).unwrap();
        let late = Utc.with_ymd_and_hms(2025, 10, 8, 23, 30, 0).unwrap();
        let in_utc = buckets(&[late], &[], 0, now(), Range::Days);
        assert_eq!(in_utc.last().unwrap().shared, 0);
        assert_eq!(in_utc[in_utc.len() - 2].shared, 1);
        let in_east = buckets(&[late], &[], 0, now().with_timezone(&east), Range::Days);
        assert_eq!(in_east.last().unwrap().shared, 1);
    }

    #[test]
    fn the_range_pill_and_bars_use_the_cores_words() {
        let words = monitor_screens_copy().traces_graph;
        assert_eq!(range_label(Range::Days, 0, None, &words), "Last 11 days");
        assert_eq!(range_label(Range::Hours, 0, None, &words), "Last 24 hours");
        assert_eq!(
            range_label(Range::Weeks, -1, None, &words),
            "42 days, 1 window back"
        );
        assert_eq!(
            range_label(Range::Days, -3, None, &words),
            "11 days, 3 windows back"
        );
        assert_eq!(
            range_label(Range::Hours, -2, None, &words),
            "24 hours, 2 windows back"
        );
        let date = Utc.timestamp_opt(1_759_665_600, 0).unwrap().naive_utc();
        assert_eq!(day(date), "Sunday, October 5");
        assert_eq!(
            range_label(Range::Days, 0, Some(date), &words),
            "Sunday, October 5",
            "a hovered bar's day replaces the window"
        );
        let bucket = Bucket {
            start: date,
            shared: 3,
            kept: 2,
            label: String::new(),
        };
        assert_eq!(
            bar_description(&bucket, &words),
            "Sunday, October 5: 3 shared, 2 kept"
        );
        assert_eq!(words.zoom_out_glyph, "\u{2212}");
        assert_eq!(words.zoom_in_glyph, "+");
        assert_eq!(words.jump_to_now_glyph, "\u{203a}|");
    }

    #[test]
    fn shared_counts_contributions_that_stand() {
        assert_eq!(
            CONTRIBUTED_STATUSES,
            ["accepted", "submitted", "quarantined"]
        );
        assert_eq!(figure(3, true), "3");
        assert_eq!(
            figure(0, false),
            "\u{2014}",
            "an unread History is not nothing shared"
        );
        let at = Some(now());
        let rows = [
            ("accepted", Some("codex"), at),
            ("submitted", Some("claude_code"), at),
            ("quarantined", Some("codex"), at),
            ("withdrawn", Some("codex"), at),
            ("accepted", Some("codex"), None),
        ];
        assert_eq!(shared_times(rows, None).len(), 3);
        assert_eq!(shared_times(rows, Some("codex")).len(), 2);
        assert_eq!(shared_times(rows, Some("cursor")).len(), 0);
    }

    #[test]
    fn kept_counts_sessions_still_waiting_for_one_tool_or_all() {
        let at = Some(now());
        let sessions = [("codex", at), ("claude_code", at), ("codex", None)];
        assert_eq!(kept_times(sessions, None).len(), 2);
        assert_eq!(kept_times(sessions, Some("codex")).len(), 1);
    }

    #[test]
    fn previous_next_and_zoom_change_only_the_range() {
        let start = Window::default();
        assert_eq!(start.range, Range::Days);
        assert!(start.at_now());
        let back = start.stepped(-1).stepped(-1);
        assert_eq!(back.offset, -2);
        assert_eq!(back.range, Range::Days);
        assert!(!back.at_now());
        // Next never passes now.
        assert_eq!(back.stepped(1).stepped(1).stepped(1).offset, 0);
        // A zoom resets to now and swaps the range.
        let zoomed = back.zoomed(back.range.zoomed_out());
        assert_eq!(
            zoomed,
            Window {
                range: Range::Weeks,
                offset: 0
            }
        );
        // At the widest, zooming out again changes nothing, the offset included.
        let far = zoomed.stepped(-4);
        assert_eq!(far.zoomed(far.range.zoomed_out()), far);
        assert_eq!(far.jumped_to_now().offset, 0);
        assert_eq!(far.jumped_to_now().range, Range::Weeks);
    }

    #[test]
    fn the_focus_tip_names_what_pressing_it_does() {
        let shell = monitor_screens_copy().shell;
        assert_eq!(focus_tip(None, false), shell.focus_needs_selection);
        assert_eq!(focus_tip(None, true), shell.focus_needs_selection);
        assert_eq!(focus_tip(Some("Codex"), true), shell.focus_whole_map);
        assert_eq!(focus_tip(Some("Codex"), false), "Show Codex in the map");
    }
}
