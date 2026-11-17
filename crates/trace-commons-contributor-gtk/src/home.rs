//! Home reads current answers; missing facts are never zero.
use crate::model::Status;

/// Keep field presence for Home even when an older daemon omits this count.
/// The existing History model defaults legacy counters for its own surface.
#[derive(Debug, Default, serde::Deserialize)]
pub struct Rollup {
    #[serde(default)]
    pub all_time: Option<ContributionCount>,
}

#[derive(Debug, serde::Deserialize)]
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

pub fn figure(value: Option<impl std::fmt::Display>) -> String {
    value
        .map(|n| n.to_string())
        .unwrap_or_else(|| "—".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

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
