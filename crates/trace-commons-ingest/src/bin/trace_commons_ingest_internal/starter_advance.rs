// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The NEAR AI starter advance, first slice: record plausibility features at
//! submission and report what `starter-plausibility-v1` would decide. Nothing
//! here sends, grants or scores. See
//! `docs/superpowers/specs/2026-10-21-nearai-starter-advance-design.md`.
//!
//! `TRACE_COMMONS_NEAR_AI_STARTER_MODE`:
//!
//! - absent or `disabled` (the default): nothing is recorded.
//! - `dry_run`: each submission's hash-only features are recorded (V131),
//!   and `POST /v1/admin/starter-advance/dry-run-report` reports would-grant
//!   counts. Requires the DB mirror.
//! - `live` refuses startup with `near_ai_starter_live_unavailable` until the
//!   payout pass that sends an advance exists (spec section 7, PR 3).

use super::*;
use trace_commons_server::account_trust_growth::database_error_label;
use trace_commons_server::starter_advance_report::starter_dry_run_report;
use trace_commons_server::starter_plausibility::{StarterDryRunReport, extract_features};

pub(super) const STARTER_MODE_ENV: &str = "TRACE_COMMONS_NEAR_AI_STARTER_MODE";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum StarterMode {
    #[default]
    Disabled,
    DryRun,
}

impl StarterMode {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::DryRun => "dry_run",
        }
    }
}

/// Reads the mode at startup. An unknown value, `live`, or `dry_run` without
/// a DB mirror refuses startup with a label that names the problem.
pub(super) fn starter_mode_from_env(has_db: bool) -> anyhow::Result<StarterMode> {
    starter_mode_from_value(std::env::var(STARTER_MODE_ENV).ok().as_deref(), has_db)
}

pub(super) fn starter_mode_from_value(
    value: Option<&str>,
    has_db: bool,
) -> anyhow::Result<StarterMode> {
    let mode = match value.map(str::trim) {
        None | Some("") | Some("disabled") => StarterMode::Disabled,
        Some("dry_run") => StarterMode::DryRun,
        Some("live") => anyhow::bail!("near_ai_starter_live_unavailable"),
        Some(_) => anyhow::bail!("near_ai_starter_mode_invalid"),
    };
    if mode == StarterMode::DryRun && !has_db {
        anyhow::bail!("near_ai_starter_database_unavailable");
    }
    Ok(mode)
}

/// Records one submission's features when the mode is not `disabled`. Runs
/// the write on its own task so an upload never waits for it or fails
/// because of it; a failure is logged as a label only.
pub(super) fn observe_submission(
    state: &AppState,
    tenant_id: &str,
    submission_id: Uuid,
    envelope: &TraceContributionEnvelope,
    received_at: DateTime<Utc>,
) {
    if state.near_ai_starter_mode == StarterMode::Disabled {
        return;
    }
    let Some(db) = state.db_mirror.clone() else {
        return;
    };
    let features = extract_features(envelope, received_at);
    let tenant_id = tenant_id.to_string();
    tokio::spawn(async move {
        if let Err(error) = db
            .record_starter_plausibility_features(&tenant_id, submission_id, &features)
            .await
        {
            tracing::warn!(
                error_label = database_error_label(&error),
                "starter plausibility features not recorded"
            );
        }
    });
}

#[derive(Debug, Deserialize)]
pub(super) struct StarterDryRunReportQuery {
    #[serde(default)]
    limit: Option<i64>,
}

/// `POST /v1/admin/starter-advance/dry-run-report?limit=N`
///
/// Evaluates up to `limit` open accounts (default 1,000, at most 10,000)
/// against `starter-plausibility-v1` at the current time and returns
/// label-only counts: accounts scanned, accounts with features, would-grant,
/// and the first rule not met. No account, tenant or submission identifier
/// appears in the response or the log. Admin only. Enumerates accounts
/// through the earned-trust worker seam, so the ingest login needs
/// `trace_account_trust_worker` (`docs/operator/account-trust.md`).
pub(super) async fn starter_dry_run_report_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<StarterDryRunReportQuery>,
) -> ApiResult<Json<StarterDryRunReport>> {
    let tenant = authenticate_with_tenant_access_grant(state.as_ref(), &headers).await?;
    require_admin(&tenant)?;
    let db = state.db_mirror.as_ref().ok_or_else(|| {
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "near_ai_starter_database_unavailable",
        )
    })?;
    let report = starter_dry_run_report(db.as_ref(), query.limit, Utc::now())
        .await
        .map_err(|error| {
            tracing::warn!(
                error_label = database_error_label(&error),
                "starter dry-run report refused"
            );
            api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "near_ai_starter_report_unavailable",
            )
        })?;
    tracing::info!(
        rule_version = report.rule_version,
        accounts_scanned = report.accounts_scanned,
        accounts_with_features = report.accounts_with_features,
        would_grant = report.would_grant,
        failed = report.failed,
        starter_mode = state.near_ai_starter_mode.as_str(),
        "starter dry-run report completed"
    );
    Ok(Json(report))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starter_mode_defaults_to_disabled() {
        assert_eq!(
            starter_mode_from_value(None, false).unwrap(),
            StarterMode::Disabled
        );
        assert_eq!(
            starter_mode_from_value(Some(""), false).unwrap(),
            StarterMode::Disabled
        );
        assert_eq!(
            starter_mode_from_value(Some("disabled"), true).unwrap(),
            StarterMode::Disabled
        );
    }

    #[test]
    fn dry_run_needs_the_database() {
        assert_eq!(
            starter_mode_from_value(Some("dry_run"), true).unwrap(),
            StarterMode::DryRun
        );
        assert_eq!(
            starter_mode_from_value(Some("dry_run"), false)
                .unwrap_err()
                .to_string(),
            "near_ai_starter_database_unavailable"
        );
    }

    #[test]
    fn live_and_unknown_modes_refuse_startup() {
        assert_eq!(
            starter_mode_from_value(Some("live"), true)
                .unwrap_err()
                .to_string(),
            "near_ai_starter_live_unavailable"
        );
        assert_eq!(
            starter_mode_from_value(Some("DRY_RUN"), true)
                .unwrap_err()
                .to_string(),
            "near_ai_starter_mode_invalid"
        );
    }
}
