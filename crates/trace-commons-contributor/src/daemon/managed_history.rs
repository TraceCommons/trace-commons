//! Owned proxy history snapshots. No network or database reads on trace loads.
use super::ipc::DaemonShared;
use crate::routing::ironwire::{SummaryGroup, SummaryTotal, SummaryView};
use crate::routing::managed_history::ManagedHistory;
use crate::routing::{RoutedExchange, RoutingLedger};
use chrono::{DateTime, Utc};
use std::sync::Arc;

pub(crate) async fn refresh(shared: &DaemonShared) {
    let _guard = shared.managed_history_refresh.lock().await;
    let now = Utc::now();
    if shared
        .managed_history_checked_at
        .read()
        .ok()
        .and_then(|at| *at)
        .is_some_and(|at| now.signed_duration_since(at) < chrono::Duration::seconds(2))
    {
        return;
    }
    let previous_at = shared
        .managed_history_checked_at
        .read()
        .ok()
        .and_then(|at| *at)
        .unwrap_or(now);
    let store = shared.store.clone();
    let loaded = tokio::task::spawn_blocking(move || {
        ManagedHistory::load(&store, chrono::DateTime::UNIX_EPOCH)
            .map(|history| history.map(Arc::new))
            .map_err(|_| ())
    })
    .await
    .unwrap_or(Err(()));
    let observed = matches!(&loaded, Ok(Some(_)));
    let absent = matches!(&loaded, Ok(None));
    let changed = if let Ok(mut snapshot) = shared.managed_history.write() {
        let changed = match (&*snapshot, &loaded) {
            (Ok(None), Ok(None)) | (Err(()), Err(())) => false,
            (Ok(Some(before)), Ok(Some(after))) => {
                !before.same_rows(after)
                    || !before.same_window(
                        after,
                        previous_at - chrono::Duration::hours(24),
                        now - chrono::Duration::hours(24),
                    )
            }
            _ => true,
        };
        *snapshot = loaded;
        changed
    } else {
        false
    };
    if let Ok(mut at) = shared.managed_history_checked_at.write() {
        *at = Some(now);
    }
    if observed || absent {
        if let Ok(mut at) = shared.managed_history_observed_at.write() {
            *at = observed.then_some(now);
        }
    }
    if changed {
        shared.publish(super::ipc::EVENT_STATUS_CHANGED, serde_json::json!({}));
    }
}

pub(crate) fn snapshot(shared: &DaemonShared) -> Result<Option<Arc<ManagedHistory>>, ()> {
    shared.managed_history.read().map_err(|_| ())?.clone()
}

/// A trace overlay over distinct owned stores. Row IDs remain store-local:
/// this snapshot is never fed into a singleton cursor or proof-detail lookup.
pub(crate) fn combined(
    shared: &DaemonShared,
    primary: Option<Arc<dyn RoutingLedger>>,
    primary_bodies: Option<std::path::PathBuf>,
    bodies_enabled: bool,
) -> Option<Arc<dyn RoutingLedger>> {
    match snapshot(shared).ok().flatten() {
        Some(history) => Some(Arc::new(Combined {
            primary,
            history,
            primary_bodies,
            bodies_enabled,
        })),
        None => primary,
    }
}
struct Combined {
    primary: Option<Arc<dyn RoutingLedger>>,
    history: Arc<ManagedHistory>,
    primary_bodies: Option<std::path::PathBuf>,
    bodies_enabled: bool,
}
impl RoutingLedger for Combined {
    fn exchanges_since(&self, from: DateTime<Utc>) -> Vec<RoutedExchange> {
        let mut rows = self
            .primary
            .as_ref()
            .map(|p| p.exchanges_since(from))
            .unwrap_or_default();
        rows.extend(self.history.rows_since(from));
        rows.sort_by_key(|row| (row.started_at, row.id));
        rows
    }
    fn bodies_dir_for(
        &self,
        row: &RoutedExchange,
        _fallback: Option<&std::path::Path>,
    ) -> Option<std::path::PathBuf> {
        if !self.bodies_enabled {
            return None;
        }
        let managed = self.history.contains_row(row);
        let primary = self.primary.as_ref().is_some_and(|p| {
            p.exchanges_since(row.started_at)
                .iter()
                .any(|candidate| candidate == row)
        });
        match (managed, primary) {
            (true, false) => self.history.bodies_dir_for(row),
            (false, true) => self.primary_bodies.clone(),
            // Never guess between stores with colliding IDs/body references.
            _ => None,
        }
    }
}

pub(crate) fn merge(a: SummaryView, b: SummaryView) -> Option<SummaryView> {
    if a.since != b.since || !a.valid(a.since) || !b.valid(b.since) {
        return None;
    }
    let enabled = a.enabled || b.enabled;
    let receipts = enabled && (!a.enabled || a.receipts) && (!b.enabled || b.receipts);
    let since = a.since;
    let routed = add_total(a.routed, b.routed)?;
    let outside = add_total(a.outside, b.outside)?;
    let unknown = add_total(a.unknown, b.unknown)?;
    let mut groups = std::collections::BTreeMap::new();
    for group in a.groups.into_iter().chain(b.groups) {
        let key = (
            group.model.clone(),
            group.backend.clone(),
            group.route.clone(),
            group.work_kind.clone(),
        );
        if let Some(previous) = groups.remove(&key) {
            let previous: SummaryGroup = previous;
            groups.insert(
                key,
                SummaryGroup {
                    total: add_total(previous.total, group.total)?,
                    ..previous
                },
            );
        } else {
            groups.insert(key, group);
        }
        if groups.len() > 4096 {
            return None;
        }
    }
    let merged = SummaryView {
        enabled,
        receipts,
        since,
        groups: groups.into_values().collect(),
        routed,
        outside,
        unknown,
    };
    merged.valid(since).then_some(merged)
}
fn add_total(mut a: SummaryTotal, b: SummaryTotal) -> Option<SummaryTotal> {
    a.calls = a.calls.checked_add(b.calls)?;
    a.priced_calls = a.priced_calls.checked_add(b.priced_calls)?;
    a.cost_usd += b.cost_usd;
    if !a.cost_usd.is_finite() || a.cost_usd * 1_000_000.0 >= u64::MAX as f64 {
        return None;
    }
    a.proof.verified = a.proof.verified.checked_add(b.proof.verified)?;
    a.proof.gateway_only = a.proof.gateway_only.checked_add(b.proof.gateway_only)?;
    a.proof.unattested = a.proof.unattested.checked_add(b.proof.unattested)?;
    a.proof.pending = a.proof.pending.checked_add(b.proof.pending)?;
    a.proof.unavailable = a.proof.unavailable.checked_add(b.proof.unavailable)?;
    a.proof.failed = a.proof.failed.checked_add(b.proof.failed)?;
    a.proof.outside = a.proof.outside.checked_add(b.proof.outside)?;
    a.proof.unrecorded = a.proof.unrecorded.checked_add(b.proof.unrecorded)?;
    Some(a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::ironwire::SummaryProofCounts;
    fn total(proof: SummaryProofCounts) -> SummaryTotal {
        SummaryTotal {
            calls: proof.verified + proof.gateway_only,
            priced_calls: 0,
            cost_usd: 0.0,
            proof,
        }
    }
    fn proof(verified: u64, gateway_only: u64) -> SummaryProofCounts {
        SummaryProofCounts {
            verified,
            gateway_only,
            unattested: 0,
            pending: 0,
            unavailable: 0,
            failed: 0,
            outside: 0,
            unrecorded: 0,
        }
    }
    fn summary(verified: u64, gateway: u64) -> SummaryView {
        let t = total(proof(verified, gateway));
        SummaryView {
            enabled: true,
            receipts: false,
            since: DateTime::UNIX_EPOCH,
            groups: vec![SummaryGroup {
                group_id: String::new(),
                model: Some("model".into()),
                backend: "nearai".into(),
                route: "routed".into(),
                work_kind: None,
                total: t.clone(),
            }],
            routed: t,
            outside: total(proof(0, 0)),
            unknown: total(proof(0, 0)),
        }
    }
    #[test]
    fn shared_and_managed_counts_add_without_promoting_gateway_proof() {
        let merged = merge(summary(1, 0), summary(0, 2)).unwrap();
        assert_eq!(merged.routed.calls, 3);
        assert_eq!(merged.routed.proof.verified, 1);
        assert_eq!(merged.routed.proof.gateway_only, 2);
        assert_eq!(merged.groups.len(), 1);
        assert_eq!(merged.groups[0].total.calls, 3);
    }
    #[test]
    fn overflow_is_unknown_instead_of_a_plausible_partial_total() {
        assert!(merge(summary(u64::MAX, 0), summary(1, 0)).is_none());
    }
    #[test]
    fn combined_overlay_never_guesses_between_store_local_ids_or_reads_without_consent() {
        use crate::routing::managed_history::tests::fixture;
        let (_dir, store) = crate::config::tests_support::temp_store();
        let root = store.dir().join("managed-routes");
        let home = fixture(&root, "verified");
        let history = Arc::new(
            ManagedHistory::load(&store, DateTime::UNIX_EPOCH)
                .unwrap()
                .unwrap(),
        );
        let row = history.rows_since(DateTime::UNIX_EPOCH).remove(0);
        let mut ledger = Combined {
            primary: None,
            history,
            primary_bodies: None,
            bodies_enabled: true,
        };
        assert_eq!(ledger.bodies_dir_for(&row, None), Some(home.join("bodies")));
        ledger.bodies_enabled = false;
        assert!(
            ledger
                .bodies_dir_for(&row, Some(std::path::Path::new("/wrong")))
                .is_none()
        );
        ledger.bodies_enabled = true;
        ledger.primary = Some(Arc::new(crate::routing::FixedLedger::new(vec![
            row.clone(),
        ])));
        ledger.primary_bodies = Some(home.join("other-bodies"));
        assert_eq!(ledger.exchanges_since(DateTime::UNIX_EPOCH).len(), 2);
        assert!(ledger.bodies_dir_for(&row, None).is_none());
    }
}
