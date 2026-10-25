// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Operator-controlled daily client allowances, separate from admission.

use super::*;
use trace_commons_protocol::upload_allowance::{DailyUploadAllowance, UploadAllowanceResponse};

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

/// The authenticated account middleware supplies tenant/account scope and
/// refuses unbound accounts before this read-only handler can be reached.
pub(super) async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(_ctx): Extension<AccountCtx>,
) -> axum::response::Response {
    (
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(UploadAllowanceResponse {
            allowance: state.client_upload_allowance,
        }),
    )
        .into_response()
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
}
