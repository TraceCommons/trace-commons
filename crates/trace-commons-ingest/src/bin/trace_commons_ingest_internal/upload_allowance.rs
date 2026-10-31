// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `GET /v1/account/upload-allowance`, which no longer advertises a client
//! daily limit, and the independent upload-trust policy's parser.
//!
//! The contributor clients have no daily upload limit. The endpoint stays,
//! answering `{"allowance": null}` to every authenticated account, because
//! older clients read an explicit `null` as "no server allowance" and treat
//! a missing field as malformed. The earned-trust growth rule that scales
//! the server's account admission budget is unchanged; it lives in
//! `account_trust_growth_routes` and `admission`.

use super::*;
use trace_commons_protocol::upload_allowance::UploadAllowanceResponse;

const TRUST_POLICY_JSON: &str = "TRACE_COMMONS_CLIENT_UPLOAD_TRUST_POLICY_JSON";
const TRUST_POLICY_VERSION: &str = "TRACE_COMMONS_CLIENT_UPLOAD_TRUST_POLICY_VERSION";

/// The independent upload-trust evaluator's applied growth rule. Parsed
/// together with its bounded reader policy, so a configuration whose two
/// halves disagree is still refused at startup.
#[derive(Debug, Clone)]
pub(super) struct ClientUploadTrustPolicy {
    pub(super) growth: trace_commons_server::account_trust_rule::GrowthPolicy,
}

pub(super) fn trust_policy_from_env() -> anyhow::Result<Option<ClientUploadTrustPolicy>> {
    trust_policy_from_values(|key| std::env::var(key))
}

pub(super) fn trust_policy_from_values(
    read: impl Fn(&str) -> Result<String, std::env::VarError>,
) -> anyhow::Result<Option<ClientUploadTrustPolicy>> {
    let optional = |key| match read(key) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(_) => Err(anyhow::anyhow!("client_upload_trust_policy_invalid")),
    };
    let json = optional(TRUST_POLICY_JSON)?;
    let version = optional(TRUST_POLICY_VERSION)?;
    let (json, version) = match (json, version) {
        (None, None) => return Ok(None),
        (Some(json), Some(version)) => (json, version),
        _ => anyhow::bail!("client_upload_trust_policy_incomplete"),
    };
    let growth =
        trace_commons_server::account_trust_rule::parse_growth_policy(&json, &[version.as_str()])
            .map_err(|_| anyhow::anyhow!("client_upload_trust_policy_invalid"))?;
    trace_commons_server::account_trust::parse_bounded_policy(&json, &[version.as_str()])
        .map_err(|_| anyhow::anyhow!("client_upload_trust_policy_invalid"))?;
    Ok(Some(ClientUploadTrustPolicy { growth }))
}

/// The account middleware authenticates, rotates sessions, and refuses
/// unbound or closed accounts before this runs; every account it lets
/// through gets the same answer.
pub(super) async fn handler(
    State(_state): State<Arc<AppState>>,
    Extension(_ctx): Extension<AccountCtx>,
) -> ApiResult<axum::response::Response> {
    Ok((
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(UploadAllowanceResponse { allowance: None }),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn independent_json() -> String {
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../deploy/staging-gcp/client-upload-trust-policy.json.template"
        ))
        .replace("${TC_UPLOAD_TRUST_GATE_POLICY_VERSION}", "gate-v1")
        .replace(
            "${TC_UPLOAD_TRUST_DEDUP_SIGNAL_VERSION}",
            "events.v2+simhash.v2",
        )
    }

    #[test]
    fn independent_upload_trust_requires_a_complete_matching_tiered_policy() {
        let parse = |json: Option<String>, version: Option<&str>| {
            trust_policy_from_values(|key| match key {
                TRUST_POLICY_JSON => json.clone().ok_or(std::env::VarError::NotPresent),
                TRUST_POLICY_VERSION => version
                    .map(str::to_owned)
                    .ok_or(std::env::VarError::NotPresent),
                _ => Err(std::env::VarError::NotPresent),
            })
        };
        assert!(parse(None, None).unwrap().is_none());
        assert!(parse(Some(independent_json()), None).is_err());
        assert!(parse(None, Some("staging-earned-upload-v1")).is_err());
        assert!(parse(Some("{}".to_owned()), Some("staging-earned-upload-v1")).is_err());
        assert!(parse(Some(independent_json()), Some("different-version")).is_err());
        let policy = parse(Some(independent_json()), Some("staging-earned-upload-v1"))
            .unwrap()
            .unwrap();
        assert_eq!(policy.growth.version(), "staging-earned-upload-v1");
        assert!(policy.growth.require_gate_pass());
    }
}
