//! Where NEAR AI credit went, from the route tallies' charges (product
//! owner, 2026-10-28, CodexBar-style): one window's sum, and for the
//! selected week the same sum by inferred task, by tool and by project
//! folder.
//!
//! - Owner decision 2026-10-28 (overrides D5 for Insights): Insights may
//!   show money. Only the NEAR AI charge the ledger recorded is summed
//!   here, never a list price.
//! - Owner decision 2026-10-28 (overrides D7 for this view): the drill-down
//!   may break down by project folder name. The name is the Sessions list's
//!   own local label, answered over the local socket and shown on this Mac
//!   only; it is never sent anywhere. A session with no folder the daemon
//!   knows (an Analyze snapshot, a folder no longer listed) is the unknown
//!   group, `key: null`.
//! - A call with no charge makes its session's figure, and every group and
//!   sum that session is in, unknown (`usd_micros: null`), never zero.
//!   `calls_without_charge` says how many calls did it.
//! - A window takes the calls that started in it, by clock hour
//!   (`NearAiCharge::within`), so a session that crossed midnight is split
//!   across the two days. A session with no call in the window is in no
//!   figure of it.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

use super::insights_route_tally::{HourCharge, NearAiCharge};

/// One session's charge and what it is grouped by. The labels are fixed
/// words or the local folder label; `None` is the unknown group.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CreditSession<'a> {
    pub charge: &'a NearAiCharge,
    pub last_call_at: DateTime<Utc>,
    /// `bug_fix`, `refactor`, `tests`, `docs`, `other` or `no_edits`.
    pub task: Option<&'a str>,
    /// `claude_code` or `codex`.
    pub tool: Option<&'a str>,
    /// The Sessions list's folder label.
    pub folder: Option<&'a str>,
}

/// The sum of the sessions in one window or group.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Sum {
    charge: HourCharge,
    sessions: u64,
}

impl Sum {
    fn add(&mut self, charge: &HourCharge) {
        self.charge.absorb(charge);
        self.sessions = self.sessions.saturating_add(1);
    }

    fn value(&self) -> serde_json::Value {
        serde_json::json!({
            "usd_micros": self.charge.known(),
            "sessions": self.sessions,
            "calls": self.charge.calls,
            "calls_without_charge": self.charge.calls_without_charge,
        })
    }
}

/// One window's NEAR AI charge as it crosses the socket: `from` and `to`
/// (RFC 3339), `usd_micros` (`null` when unknown), `sessions`, `calls` and
/// `calls_without_charge`; with `groups`, also `by_task[]`, `by_tool[]` and
/// `by_folder[]`, each `{key, usd_micros, sessions, calls,
/// calls_without_charge}`.
///
/// Groups are in order of known charge, largest first, then the unknown
/// ones; ties by key, the unknown key (`null`) last.
#[must_use]
pub(crate) fn window_value(
    sessions: &[CreditSession<'_>],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    groups: bool,
) -> serde_json::Value {
    let mut total = Sum::default();
    let mut by_task: BTreeMap<Option<&str>, Sum> = BTreeMap::new();
    let mut by_tool: BTreeMap<Option<&str>, Sum> = BTreeMap::new();
    let mut by_folder: BTreeMap<Option<&str>, Sum> = BTreeMap::new();
    for session in sessions {
        let Some(charge) = session.charge.within(session.last_call_at, from, to) else {
            continue;
        };
        total.add(&charge);
        by_task.entry(session.task).or_default().add(&charge);
        by_tool.entry(session.tool).or_default().add(&charge);
        by_folder.entry(session.folder).or_default().add(&charge);
    }
    let mut value = total.value();
    value["from"] = serde_json::json!(from.to_rfc3339());
    value["to"] = serde_json::json!(to.to_rfc3339());
    if groups {
        value["by_task"] = groups_value(by_task);
        value["by_tool"] = groups_value(by_tool);
        value["by_folder"] = groups_value(by_folder);
    }
    value
}

fn groups_value(groups: BTreeMap<Option<&str>, Sum>) -> serde_json::Value {
    let mut sorted: Vec<(Option<&str>, Sum)> = groups.into_iter().collect();
    sorted.sort_by(|(a_key, a), (b_key, b)| {
        // Known first, the larger first; then by key, unknown last.
        b.charge
            .known()
            .is_some()
            .cmp(&a.charge.known().is_some())
            .then(b.charge.known().cmp(&a.charge.known()))
            .then(a_key.is_none().cmp(&b_key.is_none()))
            .then(a_key.cmp(b_key))
    });
    serde_json::Value::Array(
        sorted
            .into_iter()
            .map(|(key, sum)| {
                let mut value = sum.value();
                value["key"] = serde_json::json!(key);
                value
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn at(hour: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 26, 0, 0, 0).unwrap() + Duration::hours(hour)
    }

    /// A charge with one call per `(hour, micros)`; `None` micros is a call
    /// with no charge.
    fn charge(calls: &[(i64, Option<u64>)]) -> NearAiCharge {
        let mut charge = NearAiCharge::default();
        for (hour, micros) in calls {
            let slot = charge
                .by_hour
                .entry(super::super::insights_route_tally::unix_hour(at(*hour)))
                .or_default();
            slot.absorb(&HourCharge {
                calls: 1,
                usd_micros: micros.unwrap_or(0),
                calls_without_charge: u64::from(micros.is_none()),
                overflowed: false,
            });
            match micros {
                Some(m) => {
                    charge.usd_micros += m;
                    charge.charged_calls += 1;
                }
                None => charge.calls_without_charge += 1,
            }
        }
        charge
    }

    fn session<'a>(
        charge: &'a NearAiCharge,
        task: Option<&'a str>,
        tool: Option<&'a str>,
        folder: Option<&'a str>,
    ) -> CreditSession<'a> {
        CreditSession {
            charge,
            last_call_at: at(0),
            task,
            tool,
            folder,
        }
    }

    #[test]
    fn a_window_sums_only_the_calls_made_in_it() {
        let a = charge(&[(1, Some(100)), (30, Some(5))]);
        let b = charge(&[(2, Some(20))]);
        let sessions = [
            session(&a, Some("bug_fix"), Some("codex"), Some("app")),
            session(&b, Some("docs"), Some("claude_code"), None),
        ];
        let day = window_value(&sessions, at(0), at(24), false);
        assert_eq!(day["usd_micros"], 120);
        assert_eq!(day["sessions"], 2);
        assert_eq!(day["calls"], 2);
        assert_eq!(day["calls_without_charge"], 0);
        assert!(day.get("by_task").is_none());
        let next = window_value(&sessions, at(24), at(48), false);
        assert_eq!(next["usd_micros"], 5);
        assert_eq!(next["sessions"], 1);
        let empty = window_value(&sessions, at(48), at(72), false);
        assert_eq!(empty["usd_micros"], 0, "no call in it is a true zero");
        assert_eq!(empty["sessions"], 0);
    }

    #[test]
    fn a_call_without_a_charge_makes_its_groups_unknown_never_zero() {
        let charged = charge(&[(1, Some(300))]);
        let missing = charge(&[(1, Some(50)), (2, None)]);
        let sessions = [
            session(&charged, Some("bug_fix"), Some("codex"), Some("app")),
            session(&missing, Some("docs"), Some("codex"), Some("site")),
        ];
        let week = window_value(&sessions, at(0), at(24 * 7), true);
        assert_eq!(week["usd_micros"], serde_json::Value::Null);
        assert_eq!(week["calls_without_charge"], 1);
        assert_eq!(week["by_tool"][0]["key"], "codex");
        assert_eq!(week["by_tool"][0]["usd_micros"], serde_json::Value::Null);
        let tasks = week["by_task"].as_array().unwrap();
        assert_eq!(tasks[0]["key"], "bug_fix");
        assert_eq!(tasks[0]["usd_micros"], 300);
        assert_eq!(tasks[1]["key"], "docs");
        assert_eq!(tasks[1]["usd_micros"], serde_json::Value::Null);
        assert_eq!(tasks[1]["calls_without_charge"], 1);
    }

    #[test]
    fn groups_go_largest_first_with_the_unknown_key_last_on_a_tie() {
        let a = charge(&[(1, Some(10))]);
        let b = charge(&[(1, Some(500))]);
        let c = charge(&[(1, Some(10))]);
        let sessions = [
            session(&a, None, Some("codex"), None),
            session(&b, Some("tests"), Some("codex"), Some("app")),
            session(&c, Some("docs"), Some("claude_code"), Some("site")),
        ];
        let week = window_value(&sessions, at(0), at(24), true);
        let keys: Vec<serde_json::Value> = week["by_task"]
            .as_array()
            .unwrap()
            .iter()
            .map(|group| group["key"].clone())
            .collect();
        assert_eq!(
            keys,
            vec![
                serde_json::json!("tests"),
                serde_json::json!("docs"),
                serde_json::Value::Null
            ]
        );
        assert_eq!(week["by_folder"][2]["key"], serde_json::Value::Null);
        assert_eq!(week["by_tool"][0]["key"], "codex");
        assert_eq!(week["by_tool"][0]["usd_micros"], 510);
        assert_eq!(week["by_tool"][0]["sessions"], 2);
    }
}
