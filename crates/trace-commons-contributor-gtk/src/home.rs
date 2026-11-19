//! Home reads current answers; missing facts are never zero.
use crate::model::Status;

/// Keep field presence for Home even when an older daemon omits this count.
/// The existing History model defaults legacy counters for its own surface.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct Rollup {
    #[serde(default)]
    pub all_time: Option<ContributionCount>,
    /// Credit not yet settled, drawn only beside the commons' condition.
    #[serde(default)]
    pub credit_pending: Option<f32>,
    /// Credit decided (History's Final): Home's Credit earned.
    #[serde(default)]
    pub credit_final: Option<f32>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ContributionCount {
    #[serde(default)]
    pub contributed: Option<u32>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Counts {
    pub waiting: Option<u64>,
    pub contributed: Option<u32>,
}

impl Counts {
    pub fn from_answers(status: Option<&Status>, rollup: Option<&Rollup>) -> Self {
        Self {
            waiting: status.and_then(|s| s.decisions_owed),
            contributed: rollup
                .and_then(|r| r.all_time.as_ref())
                .and_then(|c| c.contributed),
        }
    }
}

/// What waits on this computer, not contributed (`HomeUncontributed`): the
/// queue's traces that have not left, and their size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Uncontributed {
    pub count: usize,
    pub bytes: u64,
}

impl Uncontributed {
    /// States that have left, or are leaving, this computer.
    pub const GONE: [&'static str; 2] = ["uploading", "uploaded"];

    pub fn of(entries: &[crate::model::QueueEntry]) -> Self {
        entries
            .iter()
            .filter(|entry| !Self::GONE.contains(&entry.state.as_str()))
            .fold(Self::default(), |sum, entry| Self {
                count: sum.count + 1,
                bytes: sum.bytes.saturating_add(entry.size_bytes),
            })
    }

    /// The size beside the count, in the core's words; none with nothing
    /// waiting.
    pub fn size(&self) -> Option<String> {
        (self.count > 0).then(|| {
            trace_commons_contributor::preview_copy::monitor_screens_copy()
                .home_history
                .uncontributed_size
                .replace("{size}", &crate::model::human_bytes(self.bytes))
        })
    }
}

/// Home's Credit earned and Credit pending tiles (`HomeFormat.creditTiles`):
/// earned is History's Final, 0 once History answered with pending credit
/// and nothing decided; pending is a figure only beside the commons'
/// statement of what it waits on (D6). A dash for what is not known.
pub fn credit_tiles(rollup: Option<&Rollup>, condition: Option<&str>) -> (String, String) {
    use trace_commons_contributor::history_copy::credit_figure;
    let dash = || "—".to_owned();
    let earned =
        rollup.and_then(|rollup| rollup.credit_final.or(rollup.credit_pending.map(|_| 0.0)));
    let pending = match (condition, rollup.and_then(|rollup| rollup.credit_pending)) {
        (Some(_), Some(figure)) => Some(figure),
        _ => None,
    };
    (
        earned.map_or_else(dash, |v| credit_figure(f64::from(v))),
        pending.map_or_else(dash, |v| credit_figure(f64::from(v))),
    )
}

pub fn figure(value: Option<impl std::fmt::Display>) -> String {
    value
        .map(|n| n.to_string())
        .unwrap_or_else(|| "—".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queued(state: &str, bytes: u64) -> crate::model::QueueEntry {
        crate::model::QueueEntry {
            state: state.into(),
            size_bytes: bytes,
            ..crate::model::QueueEntry::default()
        }
    }

    /// The Mac's Not contributed tile: every trace not uploading or
    /// uploaded, and their size beside the count in the core's words.
    #[test]
    fn not_contributed_counts_what_has_not_left_and_its_size() {
        let entries = [
            queued("pending", 100 * 1024),
            queued("held", 45 * 1024),
            queued("uploading", 900 * 1024),
            queued("uploaded", 900 * 1024),
        ];
        let waiting = Uncontributed::of(&entries);
        assert_eq!(
            waiting,
            Uncontributed {
                count: 2,
                bytes: 145 * 1024
            }
        );
        let words = trace_commons_contributor::preview_copy::monitor_screens_copy();
        assert_eq!(
            waiting.size().as_deref(),
            Some(
                words
                    .home_history
                    .uncontributed_size
                    .replace("{size}", "145 KB")
                    .as_str()
            )
        );
        assert_eq!(Uncontributed::of(&[]).size(), None);
    }

    /// Credit earned is History's Final, 0 once History answered with only
    /// pending credit; Credit pending is a figure only beside the commons'
    /// condition (D6). An unread rollup is a dash in both, never 0.
    #[test]
    fn the_credit_tiles_are_the_macs() {
        let rollup = |value| serde_json::from_value::<Rollup>(value).unwrap();
        assert_eq!(credit_tiles(None, None), ("—".into(), "—".into()));
        let both = rollup(serde_json::json!({"credit_final": 10.0, "credit_pending": 2.5}));
        assert_eq!(credit_tiles(Some(&both), None), ("10".into(), "—".into()));
        assert_eq!(
            credit_tiles(Some(&both), Some("Paid weekly")),
            ("10".into(), "2.5".into())
        );
        let pending_only = rollup(serde_json::json!({"credit_pending": 1.0}));
        assert_eq!(credit_tiles(Some(&pending_only), None).0, "0");
        assert_eq!(
            credit_tiles(Some(&rollup(serde_json::json!({}))), None).0,
            "—"
        );
    }

    #[test]
    fn unavailable_answers_are_not_zero_or_queue_occupancy() {
        let status: Status =
            serde_json::from_value(serde_json::json!({"queue_depth": 42})).unwrap();
        assert_eq!(
            Counts::from_answers(Some(&status), None),
            Counts {
                waiting: None,
                contributed: None
            }
        );
        assert_eq!(figure(None::<u64>), "—");
        assert_eq!(figure(Some(0)), "0");
    }

    #[test]
    fn older_daemon_missing_contributed_is_unknown() {
        for value in [
            serde_json::json!({}),
            serde_json::json!({"all_time": {"accepted": 3, "submitted": 5}}),
        ] {
            let rollup: Rollup = serde_json::from_value(value).unwrap();
            assert_eq!(Counts::from_answers(None, Some(&rollup)).contributed, None);
        }
    }

    #[test]
    fn current_answers_use_core_contribution_and_decision_counts() {
        let status: Status =
            serde_json::from_value(serde_json::json!({"decisions_owed": 0, "queue_depth": 42}))
                .unwrap();
        let rollup: Rollup = serde_json::from_value(
            serde_json::json!({"all_time": {"accepted": 3, "submitted": 5, "contributed": 8}}),
        )
        .unwrap();
        assert_eq!(
            Counts::from_answers(Some(&status), Some(&rollup)),
            Counts {
                waiting: Some(0),
                contributed: Some(8)
            }
        );
        // A failed response is None, not the earlier successful snapshot.
        assert_eq!(
            Counts::from_answers(None, None),
            Counts {
                waiting: None,
                contributed: None
            }
        );
    }
}
