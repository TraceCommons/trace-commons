// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A typed client for the NEAR AI (`nearai/cloud-api`) admin route the
//! inference-credit rail uses, behind a trait object so tests substitute a
//! fake.
//!
//! The route is **proposed**, not yet in NEAR AI's tree:
//! `PUT|GET /v1/admin/organizations/{org_id}/trace-commons/credit`, a
//! dedicated `trace_commons` credit type whose limit only ever rises to the
//! cumulative total the caller states, written under the organization
//! accounting lock -- the staking-farm pattern
//! (`cloud-api:crates/database/src/repositories/organization_staking_farm_sources.rs:286`).
//! See `docs/superpowers/specs/2026-10-14-near-ai-trace-commons-credit-type-proposal.md`.
//!
//! # Secrets and identifiers
//!
//! The admin key reaches exactly one place, the `Authorization` header. The
//! organization id reaches exactly one place, the request path. Neither
//! appears in an error: [`AdminApiError`] is a status class and nothing
//! else, and response bodies are never read on a failure -- NEAR AI's error
//! messages echo the organization id.

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// A NEAR AI organization id. `Debug` is redacted.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct NearAiOrgId(uuid::Uuid);

impl NearAiOrgId {
    pub fn parse(value: &str) -> Option<Self> {
        uuid::Uuid::parse_str(value.trim()).ok().map(Self)
    }
    pub fn from_uuid(value: uuid::Uuid) -> Self {
        Self(value)
    }
    pub fn as_uuid(&self) -> uuid::Uuid {
        self.0
    }
    /// The audit reference: a domain-separated hash, never the id.
    pub fn ref_hash(&self) -> String {
        crate::settlement_rail::sha256_ref(
            "trace-commons:near-ai-org-ref:v1",
            &[&self.0.to_string()],
        )
    }
}

impl std::fmt::Debug for NearAiOrgId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NearAiOrgId(<redacted>)")
    }
}

/// Body of the proposed `PUT .../trace-commons/credit`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetTraceCommonsCreditRequest {
    /// The organization's cumulative Trace Commons credit, nano-USD (scale 9).
    #[serde(rename = "cumulativeNanoUsd")]
    pub cumulative_nano_usd: i64,
    #[serde(rename = "statementRef")]
    pub statement_ref: String,
}

/// Response of the PUT and the GET.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceCommonsCreditResponse {
    pub organization_id: String,
    #[serde(rename = "cumulativeNanoUsd")]
    pub cumulative_nano_usd: i64,
    pub applied: bool,
}

/// A failed admin call, by class. Carries no body, no id, no key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AdminApiError {
    /// The request never reached the server (connect, DNS, TLS).
    #[error("near_ai_admin_unreachable")]
    Unreachable,
    /// Sent, and no answer arrived in time. The write may have landed; a
    /// cumulative write is safe to repeat.
    #[error("near_ai_admin_ambiguous")]
    Ambiguous,
    /// 401: missing, expired or User-Agent-mismatched key.
    #[error("near_ai_admin_unauthorized")]
    Unauthorized,
    /// 403: the key lacks the permission (a read-only key on a write).
    #[error("near_ai_admin_forbidden")]
    Forbidden,
    /// 404: the organization is missing or inactive.
    #[error("near_ai_org_not_found")]
    NotFound,
    /// 400: NEAR AI refused the value.
    #[error("near_ai_credit_rejected")]
    Rejected,
    /// 429 or 5xx.
    #[error("near_ai_admin_unavailable")]
    Unavailable,
    /// A 2xx whose body was not the documented shape.
    #[error("near_ai_admin_response_malformed")]
    Malformed,
}

impl AdminApiError {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Unreachable => "near_ai_admin_unreachable",
            Self::Ambiguous => "near_ai_admin_ambiguous",
            Self::Unauthorized => "near_ai_admin_unauthorized",
            Self::Forbidden => "near_ai_admin_forbidden",
            Self::NotFound => "near_ai_org_not_found",
            Self::Rejected => "near_ai_credit_rejected",
            Self::Unavailable => "near_ai_admin_unavailable",
            Self::Malformed => "near_ai_admin_response_malformed",
        }
    }
}

/// The admin routes the rail needs.
#[async_trait]
pub trait NearAiAdminApi: Send + Sync {
    /// The organization's current cumulative Trace Commons credit.
    async fn get_trace_commons_credit(
        &self,
        org: NearAiOrgId,
    ) -> Result<TraceCommonsCreditResponse, AdminApiError>;

    /// Raise it to a cumulative total. Monotonic and idempotent on NEAR AI's
    /// side.
    async fn set_trace_commons_credit(
        &self,
        org: NearAiOrgId,
        request: &SetTraceCommonsCreditRequest,
    ) -> Result<TraceCommonsCreditResponse, AdminApiError>;
}

/// The HTTP implementation.
pub struct HttpNearAiAdminApi {
    base_url: String,
    key: SecretString,
    http: reqwest::Client,
}

impl HttpNearAiAdminApi {
    /// `base_url` is the API root including `/v1`. `user_agent` must be the
    /// exact User-Agent the key was minted with: NEAR AI binds admin keys to
    /// it (`cloud-api:crates/database/src/repositories/admin_access_token.rs:140`).
    pub fn new(
        base_url: &str,
        key: SecretString,
        user_agent: &str,
        timeout: Duration,
    ) -> Result<Self, AdminApiError> {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .user_agent(user_agent)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| AdminApiError::Unreachable)?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            key,
            http,
        })
    }

    fn url(&self, org: NearAiOrgId) -> String {
        format!(
            "{}/admin/organizations/{}/trace-commons/credit",
            self.base_url,
            org.as_uuid()
        )
    }
}

fn classify_status(status: reqwest::StatusCode) -> AdminApiError {
    match status.as_u16() {
        401 => AdminApiError::Unauthorized,
        403 => AdminApiError::Forbidden,
        404 => AdminApiError::NotFound,
        400 | 422 => AdminApiError::Rejected,
        _ => AdminApiError::Unavailable,
    }
}

fn classify_send_error(err: &reqwest::Error, is_write: bool) -> AdminApiError {
    if err.is_connect() {
        AdminApiError::Unreachable
    } else if is_write {
        AdminApiError::Ambiguous
    } else {
        AdminApiError::Unavailable
    }
}

async fn read_body(
    response: reqwest::Response,
) -> Result<TraceCommonsCreditResponse, AdminApiError> {
    let status = response.status();
    if !status.is_success() {
        return Err(classify_status(status));
    }
    response
        .json::<TraceCommonsCreditResponse>()
        .await
        .map_err(|_| AdminApiError::Malformed)
}

#[async_trait]
impl NearAiAdminApi for HttpNearAiAdminApi {
    async fn get_trace_commons_credit(
        &self,
        org: NearAiOrgId,
    ) -> Result<TraceCommonsCreditResponse, AdminApiError> {
        let response = self
            .http
            .get(self.url(org))
            .bearer_auth(self.key.expose_secret())
            .send()
            .await
            .map_err(|e| classify_send_error(&e, false))?;
        read_body(response).await
    }

    async fn set_trace_commons_credit(
        &self,
        org: NearAiOrgId,
        request: &SetTraceCommonsCreditRequest,
    ) -> Result<TraceCommonsCreditResponse, AdminApiError> {
        let response = self
            .http
            .put(self.url(org))
            .bearer_auth(self.key.expose_secret())
            .json(request)
            .send()
            .await
            .map_err(|e| classify_send_error(&e, true))?;
        read_body(response).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_body_matches_the_proposed_field_names() {
        let body = SetTraceCommonsCreditRequest {
            cumulative_nano_usd: 1_000_000_000,
            statement_ref: "sha256:00".into(),
        };
        let v = serde_json::to_value(&body).unwrap();
        assert_eq!(v["cumulativeNanoUsd"], 1_000_000_000i64);
        assert_eq!(v["statementRef"], "sha256:00");
        assert_eq!(v.as_object().unwrap().len(), 2);
    }

    #[test]
    fn an_org_id_never_debug_prints_and_hashes_with_a_domain() {
        let org = NearAiOrgId::parse("6f1c0b6a-0000-4000-8000-00000000abcd").unwrap();
        assert_eq!(format!("{org:?}"), "NearAiOrgId(<redacted>)");
        assert!(!org.ref_hash().contains("6f1c0b6a"));
        assert!(NearAiOrgId::parse("not-a-uuid").is_none());
    }

    #[test]
    fn errors_render_as_labels_only() {
        for e in [
            AdminApiError::Unreachable,
            AdminApiError::Ambiguous,
            AdminApiError::Unauthorized,
            AdminApiError::Forbidden,
            AdminApiError::NotFound,
            AdminApiError::Rejected,
            AdminApiError::Unavailable,
            AdminApiError::Malformed,
        ] {
            assert_eq!(e.to_string(), e.label());
            assert!(crate::settlement_rail::is_safe_label(e.label()));
        }
    }
}
