//! Server-directed daily limits, scoped to the current account lifecycle.
//! The allowance is runtime-only; local settings remain the contributor's choices.

use super::{commons_credentials, ipc::DaemonShared, settings::DaemonSettings};
use chrono::{DateTime, Utc};
use std::{sync::Mutex, time::Duration};
use trace_commons_protocol::upload_allowance::{DailyUploadAllowance, UploadAllowanceResponse};

const REFRESH_INTERVAL: chrono::TimeDelta = chrono::TimeDelta::minutes(5);

#[derive(Default)]
pub(crate) struct UploadAllowanceState(Mutex<Cache>);

#[derive(Default)]
struct Cache {
    scope: Option<String>,
    allowance: Option<DailyUploadAllowance>,
    last_attempt: Option<DateTime<Utc>>,
}

impl UploadAllowanceState {
    fn prepare(&self, scope: Option<String>, now: DateTime<Utc>, dry_run: bool) -> bool {
        let mut cache = self.0.lock().expect("upload allowance lock");
        if cache.scope != scope {
            *cache = Cache {
                scope,
                ..Default::default()
            };
        }
        if dry_run
            || cache.scope.is_none()
            || cache
                .last_attempt
                .is_some_and(|last| now < last + REFRESH_INTERVAL)
        {
            return false;
        }
        cache.last_attempt = Some(now);
        true
    }

    fn record(&self, scope: &str, allowance: Option<DailyUploadAllowance>) {
        let mut cache = self.0.lock().expect("upload allowance lock");
        if cache.scope.as_deref() == Some(scope) {
            cache.allowance = allowance;
        }
    }

    fn current(&self, scope: Option<&str>) -> Option<DailyUploadAllowance> {
        let cache = self.0.lock().expect("upload allowance lock");
        if scope.is_some() && cache.scope.as_deref() == scope {
            cache.allowance
        } else {
            None
        }
    }
}

fn apply_allowance(
    mut settings: DaemonSettings,
    allowance: Option<DailyUploadAllowance>,
) -> DaemonSettings {
    if let Some(allowance) = allowance.filter(DailyUploadAllowance::validate) {
        if !settings.upload_count_is_explicit() {
            settings.max_uploads_per_day = settings
                .max_uploads_per_day
                .max(allowance.max_uploads_per_day);
        }
        if !settings.upload_bytes_is_explicit() {
            settings.max_bytes_per_day =
                settings.max_bytes_per_day.max(allowance.max_bytes_per_day);
        }
    }
    settings
}

/// Both upload enforcement and status use this same runtime snapshot.
pub(super) fn effective_settings(shared: &DaemonShared) -> DaemonSettings {
    let settings = shared.settings.lock().expect("settings lock").clone();
    let scope = commons_credentials::account_scope(&shared.store).ok();
    let allowance = shared.upload_allowance.current(scope.as_deref());
    apply_allowance(settings, allowance)
}

/// Refresh regardless of upload-cap state or automatic-admission eligibility.
/// Local lifecycle checks invalidate an allowance even between network polls.
pub(super) async fn refresh(shared: &DaemonShared, now: DateTime<Utc>, dry_run: bool) {
    let scope = commons_credentials::account_scope(&shared.store).ok();
    if !shared.upload_allowance.prepare(scope.clone(), now, dry_run) {
        return;
    }
    let Some(scope) = scope else {
        return;
    };
    let Ok(Some(cfg)) = shared.store.load_config() else {
        shared.upload_allowance.record(&scope, None);
        return;
    };
    let Some(session) = shared.account_admission.session(shared).await else {
        // A locked, slow or temporarily unavailable credential store also
        // returns None. Lifecycle scope already invalidates sign-out; keep
        // the current identity's last valid allowance on a failed read.
        return;
    };
    if commons_credentials::scope_for_snapshot(&session.snapshot)
        .ok()
        .as_deref()
        != Some(scope.as_str())
        || super::account_admission::session_tenant(&session.session.access_token).as_deref()
            != Some(cfg.tenant_id.as_str())
    {
        shared.upload_allowance.record(&scope, None);
        return;
    }
    let Ok(client) = trace_commons_operator_client::Client::builder(
        &cfg.ingest_url,
        "TRACE_COMMONS_CONTRIBUTOR_UNUSED_BEARER_ENV",
    )
    .bearer_token(&session.session.access_token)
    .host_allowlist(crate::config::allowlist_for(cfg.allowed_hosts.as_deref()))
    .timeout(Duration::from_secs(10))
    .build() else {
        return;
    };
    let response = client
        .call_json_with_response_headers::<(), UploadAllowanceResponse>(
            reqwest::Method::GET,
            "/v1/account/upload-allowance",
            &[],
            None,
            trace_commons_protocol::ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER,
            trace_commons_protocol::ACCOUNT_NATIVE_SESSION_EXPIRES_AT_HEADER,
        )
        .await;
    if let Some(rotated) = crate::account_auth::RotatedToken::from_headers(
        response.response_header,
        response.companion_header,
    ) {
        if super::run_blocking(|| {
            crate::account_auth::store_rotated_token(&shared.store, &session, rotated)
        })
        .is_err()
        {
            tracing::debug!("upload allowance session rotation could not be stored");
        }
        shared.account_admission.drop_session();
    }
    // A logout, configuration change or new sign-in during HTTP discards the reply.
    if commons_credentials::account_scope(&shared.store)
        .ok()
        .as_deref()
        != Some(scope.as_str())
    {
        return;
    }
    if response.status != Some(reqwest::StatusCode::OK) {
        return;
    }
    let Ok(body) = response.result else {
        return;
    };
    if body.allowance.as_ref().is_some_and(|a| !a.validate()) {
        return;
    }
    shared.upload_allowance.record(&scope, body.allowance);
    shared.publish(super::ipc::EVENT_STATUS_CHANGED, serde_json::json!({}));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::{settings::DaemonSettings, state::DaemonState, uploader};

    #[test]
    fn refreshed_allowance_unblocks_a_spent_budget_without_resetting_usage() {
        let settings = DaemonSettings::default();
        let mut usage = DaemonState::new();
        usage.uploads_today = settings.max_uploads_per_day;
        usage.bytes_today = settings.max_bytes_per_day;
        assert!(!uploader::cap_check(&usage, 1, &settings));
        let allowance = trace_commons_protocol::upload_allowance::DailyUploadAllowance {
            schema_version: 1,
            max_uploads_per_day: 200,
            max_bytes_per_day: 2_147_483_648,
        };
        let effective = apply_allowance(settings.clone(), Some(allowance));
        assert!(uploader::cap_check(&usage, 1, &effective));
        assert_eq!(usage.uploads_today, 50);
        assert_eq!(usage.bytes_today, 209_715_200);
        assert_eq!(settings.max_uploads_per_day, 50);
    }
}

#[cfg(test)]
#[path = "upload_allowance_tests.rs"]
mod transport_tests;
