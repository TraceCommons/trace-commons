//! Anonymous usage counts, US only (DRAFT, NEEDS APPROVAL, 2026-11-04).
//!
//! Two public routes on `trace-commons-ingest`, both unauthenticated and
//! outside tenant context:
//!
//! - `GET /v1/public/usage-region` answers [`UsageRegion`]: whether this
//!   request's address is in the United States. The server looks the address
//!   up and keeps nothing about it. Anything it cannot answer is `false`.
//! - `POST /v1/usage-events` takes a [`UsageEventBatch`] of weekly totals
//!   and stores them as aggregate rows.
//!
//! What a client may send is fixed here, in [`USAGE_EVENTS`]: how often a
//! screen was opened or a control was used, per ISO week, with one anonymous
//! install id. The install id is a random UUIDv4 made on the computer the
//! first time counting turns on; it is not derived from, and never sent with,
//! any device key, account, enrollment, path or hardware id, and turning the
//! setting off deletes it. There is no other identifier in any request, and
//! no content: no paths, prompts, file names, model names or trace data.

use serde::{Deserialize, Serialize};

/// Ingest route for the region check.
pub const USAGE_REGION_PATH: &str = "/v1/public/usage-region";

/// Ingest route for weekly totals.
pub const USAGE_EVENTS_PATH: &str = "/v1/usage-events";

/// The largest region document a client reads. A valid one is about 30
/// bytes; this bounds the read.
pub const USAGE_REGION_MAX_BYTES: usize = 1024;

/// The most rows one batch may carry: every event for a few weeks.
pub const USAGE_BATCH_MAX_ROWS: usize = 256;

/// The largest batch body the server reads. A full batch is about 20 KiB.
pub const USAGE_BATCH_MAX_BYTES: usize = 64 * 1024;

/// The largest count one row may carry: a bound on a bug, not a policy.
pub const USAGE_COUNT_MAX: u64 = 100_000;

/// Every event a client may count. Nothing outside this list is ever
/// recorded or accepted.
pub const USAGE_EVENTS: &[&str] = &[
    "tab_home_opened",
    "tab_private_ai_opened",
    "tab_traces_opened",
    "tab_insights_opened",
    "insights_overview_opened",
    "insights_breakdown_opened",
    "insights_sessions_opened",
    "insights_analyze_opened",
    "insights_benchmarks_opened",
    "insights_refresh_clicked",
    "insights_analyze_clicked",
    "setup_finished",
];

/// Whether `event` is on [`USAGE_EVENTS`].
#[must_use]
pub fn is_usage_event(event: &str) -> bool {
    USAGE_EVENTS.contains(&event)
}

/// `GET /v1/public/usage-region` body.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageRegion {
    /// True only when the server could place the request in the United
    /// States. Absent or false means: send nothing.
    #[serde(default)]
    pub analytics_allowed: bool,
}

/// One weekly total.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageEventRow {
    /// One of [`USAGE_EVENTS`].
    pub event: String,
    /// How many times in `week`.
    pub count: u64,
    /// ISO week, `YYYY-Www` (for example `2026-W45`).
    pub week: String,
}

/// `POST /v1/usage-events` body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageEventBatch {
    /// The anonymous install id: a random UUIDv4, nothing else.
    pub install_id: uuid::Uuid,
    pub rows: Vec<UsageEventRow>,
}

/// Why a batch was refused. Fixed labels only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UsageBatchError {
    #[error("usage-batch-too-large")]
    TooLarge,
    #[error("usage-install-id-invalid")]
    InstallIdInvalid,
    #[error("usage-event-unknown")]
    UnknownEvent,
    #[error("usage-count-invalid")]
    CountInvalid,
    #[error("usage-week-invalid")]
    WeekInvalid,
}

impl UsageEventBatch {
    /// Check every row against the allow-list and the bounds. A batch is
    /// taken whole or refused whole.
    pub fn validate(&self) -> Result<(), UsageBatchError> {
        if self.rows.len() > USAGE_BATCH_MAX_ROWS {
            return Err(UsageBatchError::TooLarge);
        }
        // Random, version 4: anything else was not made by
        // `new_install_id` and may carry meaning.
        if self.install_id.get_version_num() != 4 {
            return Err(UsageBatchError::InstallIdInvalid);
        }
        for row in &self.rows {
            if !is_usage_event(&row.event) {
                return Err(UsageBatchError::UnknownEvent);
            }
            if row.count == 0 || row.count > USAGE_COUNT_MAX {
                return Err(UsageBatchError::CountInvalid);
            }
            if !is_iso_week(&row.week) {
                return Err(UsageBatchError::WeekInvalid);
            }
        }
        Ok(())
    }
}

/// A fresh anonymous install id: random, version 4, from nothing on the
/// computer.
#[must_use]
pub fn new_install_id() -> uuid::Uuid {
    uuid::Uuid::new_v4()
}

/// `YYYY-Www`, week 01 to 53.
#[must_use]
pub fn is_iso_week(week: &str) -> bool {
    let bytes = week.as_bytes();
    if bytes.len() != 8 || bytes[4] != b'-' || bytes[5] != b'W' {
        return false;
    }
    if !bytes[..4].iter().all(u8::is_ascii_digit) || !bytes[6..].iter().all(u8::is_ascii_digit) {
        return false;
    }
    let number = u32::from(bytes[6] - b'0') * 10 + u32::from(bytes[7] - b'0');
    (1..=53).contains(&number)
}

/// The ISO week of `date`, in the [`is_iso_week`] form.
#[must_use]
pub fn iso_week_of(date: chrono::NaiveDate) -> String {
    use chrono::Datelike;
    let week = date.iso_week();
    format!("{:04}-W{:02}", week.year(), week.week())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(event: &str, count: u64, week: &str) -> UsageEventRow {
        UsageEventRow {
            event: event.into(),
            count,
            week: week.into(),
        }
    }

    #[test]
    fn only_listed_events_are_usage_events() {
        assert!(is_usage_event("tab_insights_opened"));
        assert!(!is_usage_event("Tab_Insights_Opened"));
        assert!(!is_usage_event("opened /Users/someone/secret"));
        assert!(!is_usage_event(""));
    }

    #[test]
    fn a_batch_with_an_unlisted_event_is_refused_whole() {
        let batch = UsageEventBatch {
            install_id: new_install_id(),
            rows: vec![
                row("tab_insights_opened", 3, "2026-W45"),
                row("clicked_something_else", 1, "2026-W45"),
            ],
        };
        assert_eq!(batch.validate(), Err(UsageBatchError::UnknownEvent));
    }

    #[test]
    fn counts_and_weeks_are_bounded() {
        for (count, week, expected) in [
            (0, "2026-W45", Err(UsageBatchError::CountInvalid)),
            (
                USAGE_COUNT_MAX + 1,
                "2026-W45",
                Err(UsageBatchError::CountInvalid),
            ),
            (1, "2026-45", Err(UsageBatchError::WeekInvalid)),
            (1, "2026-W00", Err(UsageBatchError::WeekInvalid)),
            (1, "2026-W54", Err(UsageBatchError::WeekInvalid)),
            (1, "2026-W45", Ok(())),
        ] {
            let batch = UsageEventBatch {
                install_id: new_install_id(),
                rows: vec![row("setup_finished", count, week)],
            };
            assert_eq!(batch.validate(), expected, "{count} {week}");
        }
    }

    #[test]
    fn only_a_random_install_id_is_taken() {
        let mut batch = UsageEventBatch {
            install_id: new_install_id(),
            rows: vec![row("setup_finished", 1, "2026-W45")],
        };
        assert_eq!(batch.install_id.get_version_num(), 4);
        assert_eq!(batch.validate(), Ok(()));
        for not_random in [
            uuid::Uuid::nil(),
            // A name-based (v5) id, as one derived from a device key would be.
            uuid::Uuid::parse_str("2ed6657d-e927-568b-95e1-2665a8aea6a2").unwrap(),
        ] {
            batch.install_id = not_random;
            assert_eq!(batch.validate(), Err(UsageBatchError::InstallIdInvalid));
        }
        assert_ne!(new_install_id(), new_install_id());
    }

    #[test]
    fn an_oversized_batch_is_refused() {
        let batch = UsageEventBatch {
            install_id: new_install_id(),
            rows: vec![row("setup_finished", 1, "2026-W45"); USAGE_BATCH_MAX_ROWS + 1],
        };
        assert_eq!(batch.validate(), Err(UsageBatchError::TooLarge));
    }

    #[test]
    fn a_batch_carries_no_field_but_the_install_id_and_the_rows() {
        let id = new_install_id();
        let taken = serde_json::from_value::<UsageEventBatch>(serde_json::json!({
            "install_id": id, "rows": []
        }))
        .unwrap();
        assert_eq!(taken.install_id, id);
        for refused in [
            serde_json::json!({"install_id": id, "rows": [], "device_id": "abc"}),
            serde_json::json!({"install_id": id, "rows": [], "account_id": "abc"}),
            serde_json::json!({"rows": []}),
            serde_json::json!({"install_id": "not-a-uuid", "rows": []}),
        ] {
            assert!(
                serde_json::from_value::<UsageEventBatch>(refused.clone()).is_err(),
                "{refused}"
            );
        }
        let refused_row = serde_json::from_value::<UsageEventRow>(serde_json::json!({
            "event": "setup_finished", "count": 1, "week": "2026-W45", "path": "/x"
        }));
        assert!(refused_row.is_err());
    }

    #[test]
    fn iso_weeks_follow_the_iso_calendar() {
        let d = |y, m, day| chrono::NaiveDate::from_ymd_opt(y, m, day).unwrap();
        assert_eq!(iso_week_of(d(2026, 11, 4)), "2026-W45");
        // 1 January 2027 is a Friday: it belongs to the last week of 2026.
        assert_eq!(iso_week_of(d(2027, 1, 1)), "2026-W53");
        assert!(is_iso_week(&iso_week_of(d(2027, 1, 1))));
    }

    #[test]
    fn a_missing_answer_reads_as_not_allowed() {
        let region: UsageRegion = serde_json::from_str("{}").unwrap();
        assert!(!region.analytics_allowed);
    }
}
