//! Contribution history: what went out, and what it earned.
//!
//! Someone who lets the daemon upload while they are away needs to be able to
//! see what happened without dropping to a terminal. The server already
//! returns everything needed per submission -- status, scopes, pending and
//! final credit, and its own explanation lines -- so this module joins that
//! with the local receipts and caches the result.
//!
//! The daemon owns the polling rather than each application doing its own, so
//! there is one poller instead of three, and history is readable offline from
//! the cache with a staleness marker rather than showing an empty table when
//! the network is down.
//!
//! History records carry **no local path**. This is the surface most likely to
//! be screenshotted, exported, or shared.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use trace_commons_protocol::trace_contribution::TraceSubmissionStatusUpdate;

use crate::config::{ConfigStore, DAEMON_HISTORY_FILE, Receipt};

/// Server-side status meaning "held for operator privacy review". It is not a
/// rejection, and is counted separately everywhere so nobody reads it as one.
pub const STATUS_QUARANTINED: &str = "quarantined";
pub const STATUS_ACCEPTED: &str = "accepted";
pub const STATUS_SUBMITTED: &str = "submitted";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryRecord {
    pub submission_id: Uuid,
    pub submitted_at: DateTime<Utc>,
    pub project_label: String,
    pub source: String,
    pub session_hash: String,
    pub status: String,
    pub consent_scopes: Vec<String>,
    pub credit_points_pending: f32,
    pub credit_points_final: Option<f32>,
    /// The server's own prose about this submission, e.g. why it was held.
    pub explanations: Vec<String>,
    pub last_refreshed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HistoryCounts {
    pub submitted: u32,
    pub accepted: u32,
    pub quarantined: u32,
    pub other: u32,
}

impl HistoryCounts {
    fn count(&mut self, status: &str) {
        match status {
            STATUS_ACCEPTED => self.accepted += 1,
            STATUS_QUARANTINED => self.quarantined += 1,
            STATUS_SUBMITTED => self.submitted += 1,
            _ => self.other += 1,
        }
    }

    pub fn total(&self) -> u32 {
        self.submitted + self.accepted + self.quarantined + self.other
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HistoryRollup {
    pub week: HistoryCounts,
    pub month: HistoryCounts,
    pub all_time: HistoryCounts,
    pub credit_pending: f32,
    pub credit_final: f32,
    /// Surfaced on its own so a contributor sees held-for-review distinctly
    /// from failure.
    pub quarantined: u32,
    pub last_refreshed_at: Option<DateTime<Utc>>,
}

fn scope_names(update: &TraceSubmissionStatusUpdate) -> Vec<String> {
    update
        .consent_scopes
        .iter()
        .filter_map(|s| {
            serde_json::to_value(s)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
        })
        .collect()
}

/// Join local receipts with whatever the server currently says about them.
///
/// A receipt with no server update keeps its locally recorded status, so
/// history is complete offline rather than omitting rows it cannot refresh.
pub fn join(
    receipts: &[Receipt],
    updates: &[TraceSubmissionStatusUpdate],
    labels: &BTreeMap<Uuid, String>,
    refreshed_at: DateTime<Utc>,
) -> Vec<HistoryRecord> {
    let by_id: BTreeMap<Uuid, &TraceSubmissionStatusUpdate> =
        updates.iter().map(|u| (u.submission_id, u)).collect();

    let mut records: Vec<HistoryRecord> = receipts
        .iter()
        .map(|r| {
            let update = by_id.get(&r.submission_id);
            HistoryRecord {
                submission_id: r.submission_id,
                submitted_at: r.submitted_at,
                project_label: labels
                    .get(&r.submission_id)
                    .cloned()
                    .unwrap_or_else(|| "-".to_string()),
                source: r.source.clone(),
                session_hash: r.session_hash.clone(),
                status: update.map(|u| u.status.clone()).unwrap_or(r.status.clone()),
                consent_scopes: update.map(|u| scope_names(u)).unwrap_or_default(),
                credit_points_pending: update.map(|u| u.credit_points_pending).unwrap_or(0.0),
                credit_points_final: update.and_then(|u| u.credit_points_final),
                explanations: update
                    .map(|u| {
                        u.explanation
                            .iter()
                            .chain(u.delayed_credit_explanations.iter())
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default(),
                last_refreshed_at: update.map(|_| refreshed_at),
            }
        })
        .collect();
    records.sort_by_key(|r| std::cmp::Reverse(r.submitted_at));
    records
}

pub fn rollup(records: &[HistoryRecord], now: DateTime<Utc>) -> HistoryRollup {
    let week_cutoff = now - Duration::days(7);
    let month_cutoff = now - Duration::days(30);
    let mut r = HistoryRollup::default();
    for rec in records {
        r.all_time.count(&rec.status);
        if rec.submitted_at >= month_cutoff {
            r.month.count(&rec.status);
        }
        if rec.submitted_at >= week_cutoff {
            r.week.count(&rec.status);
        }
        if rec.status == STATUS_QUARANTINED {
            r.quarantined += 1;
        }
        r.credit_pending += rec.credit_points_pending;
        if let Some(f) = rec.credit_points_final {
            r.credit_final += f;
        }
        if rec.last_refreshed_at > r.last_refreshed_at {
            r.last_refreshed_at = rec.last_refreshed_at;
        }
    }
    r
}

pub struct HistoryCache;

impl HistoryCache {
    pub fn load(store: &ConfigStore) -> Result<Vec<HistoryRecord>> {
        let Some(body) = store.read_daemon_file(DAEMON_HISTORY_FILE)? else {
            return Ok(Vec::new());
        };
        let text = String::from_utf8(body).context("history cache is not utf-8")?;
        let mut out = Vec::new();
        let mut skipped = 0usize;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<HistoryRecord>(line) {
                Ok(r) => out.push(r),
                Err(_) => skipped += 1,
            }
        }
        if skipped > 0 {
            tracing::warn!(skipped, "skipped unparseable history lines");
        }
        Ok(out)
    }

    pub fn save(store: &ConfigStore, records: &[HistoryRecord]) -> Result<()> {
        let mut body = String::new();
        for r in records {
            body.push_str(&serde_json::to_string(r).context("serializing history record")?);
            body.push('\n');
        }
        store.write_daemon_file(DAEMON_HISTORY_FILE, body.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests_support::temp_store;
    use trace_commons_protocol::trace_contribution::ConsentScope;

    fn at(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    fn receipt(id: Uuid, hash: &str, status: &str, when: &str) -> Receipt {
        Receipt {
            submission_id: id,
            session_hash: hash.into(),
            source: "claude-code".into(),
            submitted_at: at(when),
            status: status.into(),
        }
    }

    fn update(
        id: Uuid,
        status: &str,
        pending: f32,
        final_points: Option<f32>,
    ) -> TraceSubmissionStatusUpdate {
        TraceSubmissionStatusUpdate {
            submission_id: id,
            trace_id: Uuid::nil(),
            status: status.into(),
            credit_points_pending: pending,
            credit_points_final: final_points,
            credit_points_ledger: 0.0,
            credit_points_total: None,
            explanation: vec!["held for privacy review".into()],
            delayed_credit_explanations: vec![],
            consent_scopes: vec![ConsentScope::DebuggingEvaluation],
        }
    }

    fn record(status: &str, when: &str) -> HistoryRecord {
        HistoryRecord {
            submission_id: Uuid::new_v4(),
            submitted_at: at(when),
            project_label: "proj".into(),
            source: "claude-code".into(),
            session_hash: "sha256:aa".into(),
            status: status.into(),
            consent_scopes: vec!["debugging_evaluation".into()],
            credit_points_pending: 0.0,
            credit_points_final: None,
            explanations: vec![],
            last_refreshed_at: Some(at("2026-08-08T12:00:00Z")),
        }
    }

    fn labels(id: Uuid) -> BTreeMap<Uuid, String> {
        let mut m = BTreeMap::new();
        m.insert(id, "proj".to_string());
        m
    }

    #[test]
    fn join_prefers_the_server_status_and_carries_no_local_path() {
        let id = Uuid::new_v4();
        let receipts = vec![receipt(
            id,
            "sha256:aa",
            "submitted",
            "2026-08-08T10:00:00Z",
        )];
        let updates = vec![update(id, "accepted", 1.5, Some(2.0))];
        let recs = join(&receipts, &updates, &labels(id), at("2026-08-08T12:00:00Z"));
        assert_eq!(recs[0].status, "accepted");
        assert_eq!(recs[0].credit_points_final, Some(2.0));
        assert_eq!(recs[0].consent_scopes, vec!["debugging_evaluation"]);
        assert_eq!(recs[0].explanations, vec!["held for privacy review"]);
        let json = serde_json::to_string(&recs[0]).unwrap();
        assert!(
            !json.contains("\"path\""),
            "history must never carry a local path: {json}"
        );
    }

    #[test]
    fn join_falls_back_to_the_receipt_when_the_server_has_no_update() {
        // History stays complete offline instead of dropping rows.
        let id = Uuid::new_v4();
        let receipts = vec![receipt(
            id,
            "sha256:aa",
            "submitted",
            "2026-08-08T10:00:00Z",
        )];
        let recs = join(&receipts, &[], &labels(id), at("2026-08-08T12:00:00Z"));
        assert_eq!(recs[0].status, "submitted");
        assert_eq!(recs[0].credit_points_final, None);
        assert_eq!(
            recs[0].last_refreshed_at, None,
            "an unrefreshed row must not claim freshness"
        );
    }

    #[test]
    fn join_orders_newest_first() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let receipts = vec![
            receipt(a, "sha256:aa", "submitted", "2026-08-01T10:00:00Z"),
            receipt(b, "sha256:bb", "submitted", "2026-08-08T10:00:00Z"),
        ];
        let recs = join(&receipts, &[], &BTreeMap::new(), at("2026-08-08T12:00:00Z"));
        assert_eq!(recs[0].submission_id, b);
    }

    #[test]
    fn rollup_counts_quarantined_separately_from_failures() {
        // Quarantine means held for operator privacy review, not rejected.
        let recs = vec![
            record("accepted", "2026-08-08T10:00:00Z"),
            record("quarantined", "2026-08-08T10:00:00Z"),
            record("quarantined", "2026-08-08T10:00:00Z"),
        ];
        let r = rollup(&recs, at("2026-08-08T12:00:00Z"));
        assert_eq!(r.quarantined, 2);
        assert_eq!(r.all_time.accepted, 1);
        assert_eq!(r.all_time.quarantined, 2);
        assert_eq!(r.all_time.other, 0, "quarantine is not an 'other' status");
    }

    #[test]
    fn rollup_windows_split_week_month_and_all_time() {
        let recs = vec![
            record("accepted", "2026-08-07T10:00:00Z"),
            record("accepted", "2026-07-20T10:00:00Z"),
            record("accepted", "2026-01-01T10:00:00Z"),
        ];
        let r = rollup(&recs, at("2026-08-08T12:00:00Z"));
        assert_eq!(r.week.accepted, 1);
        assert_eq!(r.month.accepted, 2);
        assert_eq!(r.all_time.accepted, 3);
    }

    #[test]
    fn rollup_sums_pending_and_final_credit_separately() {
        let mut a = record("accepted", "2026-08-08T10:00:00Z");
        a.credit_points_pending = 1.5;
        a.credit_points_final = Some(2.0);
        let mut b = record("submitted", "2026-08-08T10:00:00Z");
        b.credit_points_pending = 0.5;
        b.credit_points_final = None;
        let r = rollup(&[a, b], at("2026-08-08T12:00:00Z"));
        assert_eq!(r.credit_pending, 2.0);
        assert_eq!(r.credit_final, 2.0);
    }

    #[test]
    fn rollup_of_an_empty_history_is_all_zero() {
        let r = rollup(&[], at("2026-08-08T12:00:00Z"));
        assert_eq!(r, HistoryRollup::default());
        assert_eq!(r.all_time.total(), 0);
    }

    #[test]
    fn rollup_reports_the_freshest_refresh_time() {
        let mut a = record("accepted", "2026-08-08T10:00:00Z");
        a.last_refreshed_at = Some(at("2026-08-08T11:00:00Z"));
        let mut b = record("accepted", "2026-08-08T10:00:00Z");
        b.last_refreshed_at = Some(at("2026-08-08T12:00:00Z"));
        let r = rollup(&[a, b], at("2026-08-08T13:00:00Z"));
        assert_eq!(r.last_refreshed_at, Some(at("2026-08-08T12:00:00Z")));
    }

    #[test]
    fn cache_round_trips_and_preserves_staleness() {
        let (_d, store) = temp_store();
        let recs = vec![record("accepted", "2026-08-08T10:00:00Z")];
        HistoryCache::save(&store, &recs).unwrap();
        let loaded = HistoryCache::load(&store).unwrap();
        assert_eq!(loaded, recs);
        assert_eq!(loaded[0].last_refreshed_at, recs[0].last_refreshed_at);
    }

    #[test]
    fn cache_is_empty_when_the_file_is_absent() {
        let (_d, store) = temp_store();
        assert!(HistoryCache::load(&store).unwrap().is_empty());
    }

    #[test]
    fn a_corrupt_history_line_is_skipped_rather_than_losing_the_cache() {
        let (_d, store) = temp_store();
        let good = serde_json::to_string(&record("accepted", "2026-08-08T10:00:00Z")).unwrap();
        store
            .write_daemon_file(
                DAEMON_HISTORY_FILE,
                format!("{good}\nnot json\n").as_bytes(),
            )
            .unwrap();
        assert_eq!(HistoryCache::load(&store).unwrap().len(), 1);
    }
}
