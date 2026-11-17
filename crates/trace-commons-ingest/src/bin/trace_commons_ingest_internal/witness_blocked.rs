// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Submissions a contributor's daemon could not send because the witness was
//! busy, timed out or unreachable (`trace_commons_protocol::witness_blocked`,
//! V154).
//!
//! The daemon reports its counts in the `x-trace-witness-blocked` header of
//! its ordinary history read, `POST /v1/contributors/me/submission-status`.
//! [`record_report`] adds them to today's totals for the tenant that request
//! authenticated as. It never changes the answer: no header, a header the
//! protocol's parser refuses, no database, or a failed write are each logged
//! by label (or not at all) and the status read goes on.
//!
//! `GET /v1/admin/witness-blocked-stats?days=N` (admin) reads the totals
//! back across tenants: per day and label, per label, and overall, each with
//! the number of distinct tenants. Never a tenant id.

use trace_commons_protocol::witness_blocked::{WITNESS_BLOCKED_HEADER, WitnessBlockedCounts};
use trace_commons_server::db::witness_blocked::WitnessBlockedStats;

use super::*;

/// Add the report a request carries, if any, to `tenant_id`'s totals.
/// Best-effort: nothing here can fail the request.
pub(super) async fn record_report(state: &AppState, tenant_id: &str, headers: &HeaderMap) {
    let Some(value) = headers.get(WITNESS_BLOCKED_HEADER) else {
        return;
    };
    let counts = match value
        .to_str()
        .map_err(|_| trace_commons_protocol::witness_blocked::WitnessBlockedRefusal::Malformed)
        .and_then(WitnessBlockedCounts::parse_header_value)
    {
        Ok(counts) => counts,
        Err(refusal) => {
            tracing::warn!(label = refusal.as_str(), "witness-blocked report refused");
            return;
        }
    };
    let Some(store) = state.witness_blocked.as_ref() else {
        return;
    };
    if store
        .record_witness_blocked(tenant_id, &counts)
        .await
        .is_err()
    {
        tracing::warn!(
            label = "witness_blocked_record_failed",
            "witness-blocked report not recorded"
        );
    }
}

/// The most days the statistics route reads back, and its default.
const STATS_MAX_DAYS: u64 = 366;
const STATS_DEFAULT_DAYS: u64 = 30;

/// `GET /v1/admin/witness-blocked-stats` query.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StatsQuery {
    /// Days back, counting today, 1 to [`STATS_MAX_DAYS`];
    /// [`STATS_DEFAULT_DAYS`] when absent.
    #[serde(default)]
    days: Option<u64>,
}

/// Answers `GET /v1/admin/witness-blocked-stats`: the totals across tenants
/// over the last `days` UTC days, today included. Admin only.
pub(super) async fn stats_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<StatsQuery>,
) -> ApiResult<Json<WitnessBlockedStats>> {
    let tenant = authenticate_with_tenant_access_grant(state.as_ref(), &headers).await?;
    require_admin(&tenant)?;
    let days = query.days.unwrap_or(STATS_DEFAULT_DAYS);
    if !(1..=STATS_MAX_DAYS).contains(&days) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "witness_blocked_stats_days_out_of_range",
        ));
    }
    let Some(store) = state.witness_blocked.as_ref() else {
        return Err(api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "witness-blocked stats require a configured database",
        ));
    };
    let since = Utc::now().date_naive() - chrono::Days::new(days - 1);
    let stats = store.witness_blocked_stats(since).await.map_err(|_| {
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "witness_blocked_stats_unavailable",
        )
    })?;
    Ok(Json(stats))
}
