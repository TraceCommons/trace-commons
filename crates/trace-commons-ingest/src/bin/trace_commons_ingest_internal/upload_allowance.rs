// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Daily client allowances scaled by the account's validated earned trust.

use super::*;
use trace_commons_protocol::upload_allowance::{
    self as protocol, DailyUploadAllowance, UploadAllowanceResponse,
};

const UPLOADS_ENV: &str = "TRACE_COMMONS_CLIENT_MAX_UPLOADS_PER_DAY";
const BYTES_ENV: &str = "TRACE_COMMONS_CLIENT_MAX_BYTES_PER_DAY";

fn config_from_values(
    uploads: Option<&str>,
    bytes: Option<&str>,
) -> Result<Option<DailyUploadAllowance>, &'static str> {
    let (uploads, bytes) = match (uploads, bytes) {
        (None, None) => return Ok(None),
        (Some(uploads), Some(bytes)) => (uploads, bytes),
        _ => return Err("client_upload_allowance_configuration_incomplete"),
    };
    let allowance = DailyUploadAllowance {
        schema_version: 1,
        max_uploads_per_day: uploads
            .parse()
            .map_err(|_| "client_upload_allowance_invalid")?,
        max_bytes_per_day: bytes
            .parse()
            .map_err(|_| "client_upload_allowance_invalid")?,
    };
    if !allowance.validate() {
        return Err("client_upload_allowance_invalid");
    }
    Ok(Some(allowance))
}

pub(super) fn config_from_env() -> anyhow::Result<Option<DailyUploadAllowance>> {
    fn read(key: &str) -> anyhow::Result<Option<String>> {
        match std::env::var(key) {
            Ok(value) => Ok(Some(value)),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(std::env::VarError::NotUnicode(_)) => {
                anyhow::bail!("client_upload_allowance_invalid")
            }
        }
    }
    let uploads = read(UPLOADS_ENV)?;
    let bytes = read(BYTES_ENV)?;
    config_from_values(uploads.as_deref(), bytes.as_deref()).map_err(anyhow::Error::msg)
}

/// Scale from the same effective allowance admission has already validated.
/// No raw tier, evaluator facts or policy details cross this client surface.
fn earned_allowance(
    configured_base: Option<DailyUploadAllowance>,
    admission_base: i64,
    effective: Option<i64>,
) -> Option<DailyUploadAllowance> {
    let Some(effective) = effective else {
        return configured_base;
    };
    if admission_base <= 0 || effective <= admission_base {
        return configured_base;
    }
    let base = configured_base.unwrap_or(DailyUploadAllowance {
        schema_version: 1,
        max_uploads_per_day: protocol::DEFAULT_MAX_UPLOADS_PER_DAY,
        max_bytes_per_day: protocol::DEFAULT_MAX_BYTES_PER_DAY,
    });
    // Products of u64 daily caps and i64 admission limits fit in u128.
    // Divide before clipping so a fractional trust increase remains useful.
    let scale = |value: u64, ceiling: u64| {
        ((u128::from(value) * effective as u128) / admission_base as u128).min(u128::from(ceiling))
            as u64
    };
    Some(DailyUploadAllowance {
        schema_version: 1,
        max_uploads_per_day: scale(
            u64::from(base.max_uploads_per_day),
            u64::from(protocol::MAX_UPLOADS_PER_DAY),
        ) as u32,
        max_bytes_per_day: scale(base.max_bytes_per_day, protocol::MAX_BYTES_PER_DAY),
    })
}

async fn account_allowance(
    state: &AppState,
    ctx: &AccountCtx,
) -> ApiResult<Option<DailyUploadAllowance>> {
    let base = state.client_upload_allowance;
    let Some(config) = state.account_admission.as_ref() else {
        return Ok(base);
    };
    if !trace_commons_protocol::admission::is_anchored_tenant(&ctx.tenant_id) {
        // Legacy invite identities retain their independent admission path.
        return Ok(base);
    }
    let unavailable = || {
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "client_upload_allowance_unavailable",
        )
    };
    let unlinked = || api_error(StatusCode::FORBIDDEN, "account_identity_unlinked");
    let db = state.db_mirror.as_ref().ok_or_else(unavailable)?;
    let mut resolved = None;
    for principal in ctx.principal_set.to_vec() {
        if let Ok(account) = trace_commons_server::account_trust::resolve_contribution_account(
            db.as_ref(),
            &ctx.tenant_id,
            &principal,
        )
        .await
            && account.account_id() == ctx.account_id.as_uuid()
        {
            resolved = Some((account, principal));
            break;
        }
    }
    let (account, principal) = resolved.ok_or_else(unlinked)?;
    let status = db
        .account_admission_status(&account, &principal, &config.policy)
        .await
        .map_err(|_| unavailable())?
        .ok_or_else(unlinked)?;
    Ok(match status.effective_allowance {
        Some(effective) if status.authority == "bounded" => {
            earned_allowance(base, config.policy.bounded_allowance(), Some(effective))
        }
        _ => base,
    })
}

/// The normal account middleware authenticates, rotates sessions, and refuses
/// unbound/closed accounts. All trust reads use its tenant and account context.
pub(super) async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
) -> ApiResult<axum::response::Response> {
    let allowance = account_allowance(&state, &ctx).await?;
    Ok((
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(UploadAllowanceResponse { allowance }),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_allowance_configuration_is_absent_or_complete_and_bounded() {
        assert_eq!(config_from_values(None, None).unwrap(), None);
        let configured = config_from_values(Some("200"), Some("2147483648"))
            .unwrap()
            .unwrap();
        assert_eq!(configured.schema_version, 1);
        assert_eq!(configured.max_uploads_per_day, 200);
        assert_eq!(configured.max_bytes_per_day, 2_147_483_648);
        assert!(configured.validate());
        for (uploads, bytes) in [
            (Some("200"), None),
            (None, Some("2147483648")),
            (Some(""), Some("2147483648")),
            (Some("0"), Some("2147483648")),
            (Some("1001"), Some("2147483648")),
            (Some("200"), Some("0")),
            (Some("200"), Some("5368709121")),
            (Some("200"), Some("-1")),
        ] {
            assert!(
                config_from_values(uploads, bytes).is_err(),
                "{uploads:?} {bytes:?}"
            );
        }
    }

    #[test]
    fn earned_upload_allowance_scales_each_accounts_validated_entitlement() {
        assert_eq!(earned_allowance(None, 100, Some(100)), None);
        let two = earned_allowance(None, 100, Some(200)).unwrap();
        assert_eq!(two.max_uploads_per_day, 100);
        assert_eq!(two.max_bytes_per_day, 419_430_400);
        let three = earned_allowance(None, 100, Some(300)).unwrap();
        assert_eq!(three.max_uploads_per_day, 150);
        assert_eq!(three.max_bytes_per_day, 629_145_600);
        let fractional = earned_allowance(None, 100, Some(150)).unwrap();
        assert_eq!(fractional.max_uploads_per_day, 75);
        assert_eq!(fractional.max_bytes_per_day, 314_572_800);
        assert!(two.validate() && three.validate() && fractional.validate());
    }

    #[test]
    fn earned_upload_allowance_preserves_base_and_invitation_paths_and_clips_safely() {
        let base = config_from_values(Some("200"), Some("2147483648")).unwrap();
        assert_eq!(
            earned_allowance(base, 100, None),
            base,
            "invited has no finite admission bound"
        );
        assert_eq!(earned_allowance(None, 100, None), None);
        assert_eq!(earned_allowance(base, 100, Some(100)), base);
        assert_eq!(earned_allowance(None, 100, Some(0)), None);
        assert_eq!(earned_allowance(None, 0, Some(100)), None);
        let scaled = earned_allowance(base, 100, Some(300)).unwrap();
        assert_eq!(scaled.max_uploads_per_day, 600);
        assert_eq!(scaled.max_bytes_per_day, protocol::MAX_BYTES_PER_DAY);
        let ceiling = earned_allowance(None, 1, Some(i64::MAX)).unwrap();
        assert_eq!(ceiling.max_uploads_per_day, protocol::MAX_UPLOADS_PER_DAY);
        assert_eq!(ceiling.max_bytes_per_day, protocol::MAX_BYTES_PER_DAY);
        let large = earned_allowance(None, i64::MAX / 2, Some(i64::MAX)).unwrap();
        assert_eq!(large.max_uploads_per_day, 100);
        assert_eq!(large.max_bytes_per_day, 419_430_400);
    }
}
