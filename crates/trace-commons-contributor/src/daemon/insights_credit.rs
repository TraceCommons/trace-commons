//! Where NEAR AI credit went, from the route tallies' charges (product
//! owner, 2026-10-28, CodexBar-style): one window's sum, and for the
//! selected week the same sum by project folder. (By task and by tool
//! were removed with Insights' By task and By tool views, owner
//! 2026-10-29.)
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

/// One session's charge and the folder it is grouped by: the local folder
/// label; `None` is the unknown group.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CreditSession<'a> {
    pub charge: &'a NearAiCharge,
    pub last_call_at: DateTime<Utc>,
    /// The Sessions list's folder label.
    pub folder: Option<&'a str>,
    /// Whether it is a trace, counted in `sessions`; `false` for the spend
    /// no session could be named for.
    pub counts_session: bool,
}

/// The sum of the sessions in one window or group.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Sum {
    charge: HourCharge,
    sessions: u64,
}

impl Sum {
    fn add(&mut self, charge: &HourCharge, counts_session: bool) {
        self.charge.absorb(charge);
        self.sessions = self.sessions.saturating_add(u64::from(counts_session));
    }

    fn value(&self) -> serde_json::Value {
        serde_json::json!({
            "usd_micros": self.charge.known(),
            "sessions": self.sessions,
            "calls": self.charge.calls,
            "calls_without_charge": self.charge.calls_without_charge,
            // The calls' tokens; `null` once any call's are unknown. No call
            // is a true 0.
            "tokens": if self.charge.calls == 0 { Some(0) } else { self.charge.tokens },
            // As far as known (product owner, 2026-10-28): the sum of the
            // calls that carried one, a minimum when `*_partial`; `null`
            // only when no call carried one.
            "usd_micros_known": self.charge.known_part().map(|part| part.sum),
            "usd_partial": self.charge.known_part().is_some_and(|part| part.partial),
            "tokens_known": self.charge.tokens_part().map(|part| part.sum),
            "tokens_partial": self.charge.tokens_part().is_some_and(|part| part.partial),
        })
    }
}

/// One window's NEAR AI charge as it crosses the socket: `from` and `to`
/// (RFC 3339), `usd_micros` (`null` when unknown), `sessions`, `calls`,
/// `calls_without_charge` and `tokens` (`null` when unknown); with `groups`, also
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
    let mut by_folder: BTreeMap<Option<&str>, Sum> = BTreeMap::new();
    for session in sessions {
        let Some(charge) = session.charge.within(session.last_call_at, from, to) else {
            continue;
        };
        let counts = session.counts_session;
        total.add(&charge, counts);
        by_folder
            .entry(session.folder)
            .or_default()
            .add(&charge, counts);
    }
    let mut value = total.value();
    value["from"] = serde_json::json!(from.to_rfc3339());
    value["to"] = serde_json::json!(to.to_rfc3339());
    if groups {
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
                tokens: Some(1_000),
                tokens_known: 1_000,
                calls_with_tokens: 1,
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

    fn session<'a>(charge: &'a NearAiCharge, folder: Option<&'a str>) -> CreditSession<'a> {
        CreditSession {
            charge,
            last_call_at: at(0),
            folder,
            counts_session: true,
        }
    }

    #[test]
    fn a_window_sums_only_the_calls_made_in_it() {
        let a = charge(&[(1, Some(100)), (30, Some(5))]);
        let b = charge(&[(2, Some(20))]);
        let sessions = [session(&a, Some("app")), session(&b, None)];
        let day = window_value(&sessions, at(0), at(24), false);
        assert_eq!(day["usd_micros"], 120);
        assert_eq!(day["sessions"], 2);
        assert_eq!(day["calls"], 2);
        assert_eq!(day["calls_without_charge"], 0);
        assert_eq!(day["tokens"], 2_000);
        assert!(day.get("by_folder").is_none());
        let next = window_value(&sessions, at(24), at(48), false);
        assert_eq!(next["usd_micros"], 5);
        assert_eq!(next["sessions"], 1);
        let empty = window_value(&sessions, at(48), at(72), false);
        assert_eq!(empty["tokens"], 0, "no call is a true zero");
        assert_eq!(empty["usd_micros"], 0, "no call in it is a true zero");
        assert_eq!(empty["sessions"], 0);
    }

    #[test]
    fn a_call_without_a_charge_makes_its_groups_unknown_never_zero() {
        let charged = charge(&[(1, Some(300))]);
        let missing = charge(&[(1, Some(50)), (2, None)]);
        let sessions = [
            session(&charged, Some("app")),
            session(&missing, Some("site")),
        ];
        let week = window_value(&sessions, at(0), at(24 * 7), true);
        assert_eq!(week["usd_micros"], serde_json::Value::Null);
        assert_eq!(week["calls_without_charge"], 1);
        assert!(week.get("by_task").is_none(), "by task was removed");
        assert!(week.get("by_tool").is_none(), "by tool was removed");
        let folders = week["by_folder"].as_array().unwrap();
        assert_eq!(folders[0]["key"], "app");
        assert_eq!(folders[0]["usd_micros"], 300);
        assert_eq!(folders[1]["key"], "site");
        assert_eq!(folders[1]["usd_micros"], serde_json::Value::Null);
        assert_eq!(folders[1]["calls_without_charge"], 1);
        // Owner, 2026-10-28: the known part as a minimum, partial; unknown
        // only when no call carried a charge.
        assert_eq!(folders[1]["usd_micros_known"], 50);
        assert_eq!(folders[1]["usd_partial"], true);
        assert_eq!(folders[0]["usd_partial"], false);
        assert_eq!(week["usd_micros_known"], 350);
        assert_eq!(week["usd_partial"], true);
        let none = charge(&[(1, None)]);
        let only_unknown = window_value(&[session(&none, None)], at(0), at(24), false);
        assert_eq!(only_unknown["usd_micros_known"], serde_json::Value::Null);
        assert_eq!(only_unknown["usd_partial"], false);
    }

    #[test]
    fn groups_go_largest_first_with_the_unknown_key_last_on_a_tie() {
        let a = charge(&[(1, Some(10))]);
        let b = charge(&[(1, Some(500))]);
        let c = charge(&[(1, Some(10))]);
        let sessions = [
            session(&a, None),
            session(&b, Some("app")),
            session(&c, Some("site")),
        ];
        let week = window_value(&sessions, at(0), at(24), true);
        let keys: Vec<serde_json::Value> = week["by_folder"]
            .as_array()
            .unwrap()
            .iter()
            .map(|group| group["key"].clone())
            .collect();
        assert_eq!(
            keys,
            vec![
                serde_json::json!("app"),
                serde_json::json!("site"),
                serde_json::Value::Null
            ]
        );
        assert_eq!(week["by_folder"][0]["usd_micros"], 500);
        assert_eq!(week["by_folder"][0]["sessions"], 1);
    }
}
