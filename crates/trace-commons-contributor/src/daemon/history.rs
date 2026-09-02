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

use std::collections::{BTreeMap, BTreeSet};

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
/// Local status this cache stamps onto a record once `daemon::withdraw` has
/// had the server confirm a withdrawal. Not a status the server itself ever
/// returns from submission-status read-back -- `join` only ever writes the
/// four statuses above from the server's response, so a record can only
/// carry this one by going through `mark_withdrawn`.
pub const STATUS_WITHDRAWN: &str = "withdrawn";

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
    /// Set locally by `mark_withdrawn` once the server has confirmed a
    /// withdrawal for this submission. `#[serde(default)]` so a cache file
    /// written before this field existed still parses.
    #[serde(default)]
    pub withdrawn_at: Option<DateTime<Utc>>,
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
                // `join` rebuilds every record from receipts + the server's
                // own status read-back, which does not yet report a
                // withdrawn status of its own (the server endpoint that
                // would is being built separately -- see
                // `docs/superpowers/specs/2026-08-08-trace-withdrawal-design.md`).
                // A caller that refreshes history after `mark_withdrawn` set
                // this must re-apply it; `join` cannot know about it here
                // because it has no access to the cache it is about to
                // replace.
                withdrawn_at: None,
            }
        })
        .collect();
    records.sort_by_key(|r| std::cmp::Reverse(r.submitted_at));
    records
}

/// Add a record for every receipt the cache does not already know about,
/// carrying the receipt's own locally recorded status (`submitted`).
///
/// This exists because "uploaded, no verdict yet" is a real state that was
/// invisible for up to a full `history_poll_secs` -- half an hour on the
/// machine this was diagnosed on. The state itself needed nothing new to be
/// persisted: `submit` already writes a receipt with status `submitted` the
/// moment an upload lands, and [`join`] already reports exactly that for a
/// receipt the server has said nothing about. The only thing missing was
/// that nothing rebuilt the cache between the upload and the next poll, so
/// the rollup's `submitted` bucket read zero while a dozen traces were
/// genuinely in flight.
///
/// So this is deliberately the *cheap* half: no network call, purely local,
/// and additive only.
///
/// - It never rewrites a record that is already cached. A verdict already
///   read back, or a local `withdrawn` marker, must not be reverted to
///   `submitted` by a pass that spoke to no server.
/// - `last_refreshed_at` stays `None`, because nothing has been refreshed
///   from the server for this row. Claiming otherwise would be the same
///   dishonesty in the other direction.
/// - Nothing here can make the count only go up: the next [`join`] rebuilds
///   every record from receipts plus the server's read-back, so a verdict,
///   a withdrawal, or a quarantine moves the row out of `submitted` on its
///   own.
///
/// Superseded, refused, and stale-approval entries never reach this at all:
/// no upload happened, so no receipt was written.
///
/// Returns whether anything was added.
pub fn merge_new_receipts(
    records: &mut Vec<HistoryRecord>,
    receipts: &[Receipt],
    labels: &BTreeMap<Uuid, String>,
) -> bool {
    let known: std::collections::BTreeSet<Uuid> = records.iter().map(|r| r.submission_id).collect();
    let mut added = false;
    for r in receipts {
        if known.contains(&r.submission_id) {
            continue;
        }
        records.push(HistoryRecord {
            submission_id: r.submission_id,
            submitted_at: r.submitted_at,
            project_label: labels
                .get(&r.submission_id)
                .cloned()
                .unwrap_or_else(|| "-".to_string()),
            source: r.source.clone(),
            session_hash: r.session_hash.clone(),
            status: r.status.clone(),
            consent_scopes: Vec::new(),
            credit_points_pending: 0.0,
            credit_points_final: None,
            explanations: Vec::new(),
            last_refreshed_at: None,
            withdrawn_at: None,
        });
        added = true;
    }
    if added {
        records.sort_by_key(|r| std::cmp::Reverse(r.submitted_at));
    }
    added
}

/// Stamp a local-only withdrawal marker onto every record for
/// `submission_id`: `status` becomes [`STATUS_WITHDRAWN`] and `withdrawn_at`
/// is set to `at`. Returns whether any record matched.
///
/// This is local bookkeeping only -- it does not call the server. Callers in
/// `daemon::withdraw` call this after the server has already confirmed the
/// withdrawal, so the history a contributor sees reflects it without needing
/// a full `refresh_history` round trip.
///
/// In the ordinary case exactly one record matches (submission ids are
/// unique), but every matching record is stamped rather than assuming that,
/// so this stays correct if that ever changes.
pub fn mark_withdrawn(
    records: &mut [HistoryRecord],
    submission_id: Uuid,
    at: DateTime<Utc>,
) -> bool {
    let mut changed = false;
    for r in records.iter_mut() {
        if r.submission_id == submission_id {
            r.status = STATUS_WITHDRAWN.to_string();
            r.withdrawn_at = Some(at);
            changed = true;
        }
    }
    changed
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

/// What has gone out since `since`, for the digest's contribution line.
///
/// `since` is the last digest instant; `None` means there has never been one,
/// and then everything in the cache counts. Withdrawn submissions are left
/// out: a contributor who took something back should not be told it was
/// contributed, and the withdrawal is the more recent fact about it.
///
/// The labels come back as a set because the digest names projects, not
/// submissions, and a busy project would otherwise be listed once per trace.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ContributedSince {
    pub count: usize,
    pub project_labels: BTreeSet<String>,
    pub credit_pending: f32,
}

pub fn contributed_since(
    records: &[HistoryRecord],
    since: Option<DateTime<Utc>>,
) -> ContributedSince {
    let mut out = ContributedSince::default();
    for rec in records {
        if rec.withdrawn_at.is_some() {
            continue;
        }
        // Strictly after: a record stamped at the exact instant of the last
        // digest was already counted by it, and counting it twice is how a
        // running total drifts upward on a quiet machine.
        if let Some(since) = since {
            if rec.submitted_at <= since {
                continue;
            }
        }
        out.count += 1;
        if !rec.project_label.is_empty() {
            out.project_labels.insert(rec.project_label.clone());
        }
        out.credit_pending += rec.credit_points_pending;
    }
    out
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
            withdrawn_at: None,
        }
    }

    #[test]
    fn contributed_since_counts_only_what_is_newer_than_the_last_digest() {
        let records = vec![
            record(STATUS_ACCEPTED, "2026-08-08T10:00:00Z"),
            record(STATUS_ACCEPTED, "2026-08-08T13:00:00Z"),
            record(STATUS_SUBMITTED, "2026-08-08T14:00:00Z"),
        ];
        let out = contributed_since(&records, Some(at("2026-08-08T12:00:00Z")));
        assert_eq!(out.count, 2);
    }

    /// A record stamped at the exact instant of the last digest was already
    /// counted by it. Counting it again is how a running total drifts upward
    /// on a machine that is doing nothing.
    #[test]
    fn contributed_since_excludes_the_boundary_instant() {
        let records = vec![record(STATUS_ACCEPTED, "2026-08-08T12:00:00Z")];
        let out = contributed_since(&records, Some(at("2026-08-08T12:00:00Z")));
        assert_eq!(out.count, 0);
    }

    #[test]
    fn contributed_since_counts_everything_when_there_has_never_been_a_digest() {
        let records = vec![
            record(STATUS_ACCEPTED, "2020-01-01T00:00:00Z"),
            record(STATUS_ACCEPTED, "2026-08-08T13:00:00Z"),
        ];
        assert_eq!(contributed_since(&records, None).count, 2);
    }

    /// Someone who took a submission back should not then be told it was
    /// contributed. The withdrawal is the more recent fact about it.
    #[test]
    fn contributed_since_leaves_out_withdrawn_submissions() {
        let mut withdrawn = record(STATUS_ACCEPTED, "2026-08-08T13:00:00Z");
        withdrawn.withdrawn_at = Some(at("2026-08-08T14:00:00Z"));
        let records = vec![withdrawn, record(STATUS_ACCEPTED, "2026-08-08T13:00:00Z")];
        assert_eq!(contributed_since(&records, None).count, 1);
    }

    /// The digest names projects, not submissions. A busy project would
    /// otherwise be listed once per trace.
    #[test]
    fn contributed_since_collapses_projects_and_drops_blank_labels() {
        let mut a = record(STATUS_ACCEPTED, "2026-08-08T13:00:00Z");
        a.project_label = "api".into();
        let mut b = record(STATUS_ACCEPTED, "2026-08-08T13:30:00Z");
        b.project_label = "api".into();
        let mut blank = record(STATUS_ACCEPTED, "2026-08-08T13:40:00Z");
        blank.project_label = String::new();
        let out = contributed_since(&[a, b, blank], None);
        assert_eq!(out.count, 3);
        assert_eq!(out.project_labels.len(), 1);
        assert!(out.project_labels.contains("api"));
    }

    #[test]
    fn contributed_since_sums_pending_credit_over_the_window_only() {
        let mut old = record(STATUS_ACCEPTED, "2026-08-08T10:00:00Z");
        old.credit_points_pending = 9.0;
        let mut fresh = record(STATUS_ACCEPTED, "2026-08-08T13:00:00Z");
        fresh.credit_points_pending = 1.5;
        let out = contributed_since(&[old, fresh], Some(at("2026-08-08T12:00:00Z")));
        assert!((out.credit_pending - 1.5).abs() < f32::EPSILON, "{:?}", out);
    }

    fn labels(id: Uuid) -> BTreeMap<Uuid, String> {
        let mut m = BTreeMap::new();
        m.insert(id, "proj".to_string());
        m
    }

    #[test]
    fn a_receipt_the_cache_has_never_seen_becomes_a_sent_and_waiting_row() {
        // The state that read zero for half an hour after every upload.
        let id = Uuid::new_v4();
        let receipts = vec![receipt(
            id,
            "sha256:aa",
            "submitted",
            "2026-08-08T10:00:00Z",
        )];
        let mut records = Vec::new();
        assert!(merge_new_receipts(&mut records, &receipts, &labels(id)));
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].submission_id, id);
        assert_eq!(records[0].status, STATUS_SUBMITTED);
        assert_eq!(records[0].project_label, "proj");
        // Nothing was refreshed from the server, and the row says so
        // rather than claiming a read-back that never happened.
        assert_eq!(records[0].last_refreshed_at, None);
        assert_eq!(records[0].credit_points_pending, 0.0);
        assert_eq!(records[0].credit_points_final, None);
        // And it lands in the bucket a contributor reads as "sent, waiting
        // to hear back".
        let r = rollup(&records, at("2026-08-08T12:00:00Z"));
        assert_eq!(r.all_time.submitted, 1);
        assert_eq!(r.all_time.accepted, 0);
    }

    #[test]
    fn merging_never_reverts_a_verdict_already_read_back() {
        // A local pass that spoke to no server must not overwrite a status
        // that came from one.
        let id = Uuid::new_v4();
        let mut records = vec![HistoryRecord {
            submission_id: id,
            status: STATUS_ACCEPTED.into(),
            credit_points_final: Some(2.0),
            ..record("accepted", "2026-08-08T10:00:00Z")
        }];
        let receipts = vec![receipt(
            id,
            "sha256:aa",
            "submitted",
            "2026-08-08T10:00:00Z",
        )];
        assert!(!merge_new_receipts(&mut records, &receipts, &labels(id)));
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, STATUS_ACCEPTED);
        assert_eq!(records[0].credit_points_final, Some(2.0));
    }

    #[test]
    fn merging_never_resurrects_a_withdrawn_row_as_sent() {
        let id = Uuid::new_v4();
        let mut records = vec![HistoryRecord {
            submission_id: id,
            ..record("submitted", "2026-08-08T10:00:00Z")
        }];
        assert!(mark_withdrawn(&mut records, id, at("2026-08-08T11:00:00Z")));
        let receipts = vec![receipt(
            id,
            "sha256:aa",
            "submitted",
            "2026-08-08T10:00:00Z",
        )];
        assert!(!merge_new_receipts(&mut records, &receipts, &labels(id)));
        assert_eq!(records[0].status, STATUS_WITHDRAWN);
        assert_eq!(records[0].withdrawn_at, Some(at("2026-08-08T11:00:00Z")));
    }

    #[test]
    fn a_verdict_moves_the_row_out_of_sent_and_waiting() {
        // The count must be able to come back down. A locally merged row is
        // replaced wholesale by the next server read-back.
        let id = Uuid::new_v4();
        let receipts = vec![receipt(
            id,
            "sha256:aa",
            "submitted",
            "2026-08-08T10:00:00Z",
        )];
        let mut records = Vec::new();
        merge_new_receipts(&mut records, &receipts, &labels(id));
        assert_eq!(
            rollup(&records, at("2026-08-08T12:00:00Z"))
                .all_time
                .submitted,
            1
        );

        let refreshed = join(
            &receipts,
            &[update(id, STATUS_ACCEPTED, 1.5, Some(2.0))],
            &labels(id),
            at("2026-08-08T12:00:00Z"),
        );
        let r = rollup(&refreshed, at("2026-08-08T12:00:00Z"));
        assert_eq!(r.all_time.submitted, 0);
        assert_eq!(r.all_time.accepted, 1);
    }

    #[test]
    fn merged_rows_stay_newest_first() {
        let old = Uuid::new_v4();
        let new = Uuid::new_v4();
        let mut records = Vec::new();
        merge_new_receipts(
            &mut records,
            &[
                receipt(old, "sha256:aa", "submitted", "2026-08-08T10:00:00Z"),
                receipt(new, "sha256:bb", "submitted", "2026-08-08T18:00:00Z"),
            ],
            &BTreeMap::new(),
        );
        assert_eq!(records[0].submission_id, new);
        assert_eq!(records[1].submission_id, old);
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

    #[test]
    fn a_cache_line_written_before_withdrawn_at_existed_still_parses() {
        // `#[serde(default)]` on `withdrawn_at`: a line with no such field
        // (an older cache file) must still load, not be silently skipped as
        // a corrupt line.
        let (_d, store) = temp_store();
        let mut v = serde_json::to_value(record("accepted", "2026-08-08T10:00:00Z")).unwrap();
        v.as_object_mut().unwrap().remove("withdrawn_at");
        store
            .write_daemon_file(
                DAEMON_HISTORY_FILE,
                format!("{}\n", serde_json::to_string(&v).unwrap()).as_bytes(),
            )
            .unwrap();
        let loaded = HistoryCache::load(&store).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].withdrawn_at, None);
    }

    #[test]
    fn mark_withdrawn_stamps_the_matching_record_and_reports_it_changed() {
        let id = Uuid::new_v4();
        let mut recs = vec![record("quarantined", "2026-08-08T10:00:00Z")];
        recs[0].submission_id = id;
        let at_time = at("2026-08-08T13:00:00Z");
        assert!(mark_withdrawn(&mut recs, id, at_time));
        assert_eq!(recs[0].status, STATUS_WITHDRAWN);
        assert_eq!(recs[0].withdrawn_at, Some(at_time));
    }

    #[test]
    fn mark_withdrawn_reports_false_when_nothing_matches() {
        let mut recs = vec![record("accepted", "2026-08-08T10:00:00Z")];
        assert!(!mark_withdrawn(&mut recs, Uuid::new_v4(), Utc::now()));
        assert_eq!(recs[0].status, "accepted");
    }
}
