// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Earned account trust (Flow 3) worker routes, shadow only. See
//! `docs/superpowers/specs/2026-09-26-earned-account-trust-design.md`.
//!
//! Decision 7 of that spec: during shadow these run under `require_admin`, as
//! the contributor-cap and dedup passes do. A scoped worker bearer is required
//! before any of them runs unattended in production. Nothing here changes an
//! allowance.

use super::*;
use trace_commons_server::account_trust_growth::{RecordFactsSummary, record_account_trust_facts};

#[derive(Debug, Deserialize)]
pub(super) struct RecordAccountTrustFactsQuery {
    #[serde(default)]
    limit: Option<i64>,
    #[serde(default)]
    dry_run: Option<bool>,
}

/// `POST /v1/admin/record-account-trust-facts?limit=N&dry_run=true|false`
///
/// Walks open accounts in anchored tenants and records each missing fact
/// through the V85 definer function. Synchronous and bounded by `limit`
/// (default 1000 sources, at most 10000), so the acknowledgement carries the
/// counts. Idempotent: a re-run records nothing new. The response is
/// label-only counts; no account, tenant, submission or source identifier.
pub(super) async fn record_account_trust_facts_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<RecordAccountTrustFactsQuery>,
) -> ApiResult<Json<RecordFactsSummary>> {
    let tenant = authenticate_with_tenant_access_grant(state.as_ref(), &headers).await?;
    require_admin(&tenant)?;
    let Some(db) = state.db_mirror.as_ref() else {
        return Err(api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "account trust fact recording requires a configured DB mirror",
        ));
    };
    let summary =
        record_account_trust_facts(db.as_ref(), query.limit, query.dry_run.unwrap_or(false))
            .await
            .map_err(|error| {
                tracing::warn!(
                    error_label =
                        trace_commons_server::account_trust_growth::database_error_label(&error),
                    "earned-trust fact recording refused"
                );
                api_error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "account_trust_worker_unavailable",
                )
            })?;
    tracing::info!(
        accounts_scanned = summary.accounts_scanned,
        candidates = summary.candidates,
        declined = summary.declined,
        failed = summary.failed,
        dry_run = summary.dry_run,
        "earned-trust fact recording pass completed"
    );
    Ok(Json(summary))
}
