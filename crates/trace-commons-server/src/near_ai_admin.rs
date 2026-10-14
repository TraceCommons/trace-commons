// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A typed client for the NEAR AI (`nearai/cloud-api`) admin routes the
//! inference-credit rail uses, behind a trait object so tests substitute a
//! fake.
//!
//! The routes and their semantics are recorded in
//! `docs/superpowers/specs/2026-10-14-near-ai-inference-credit-settlement-design.md`
//! section 1. In short: `PATCH .../limits` sets an **absolute** ceiling per
//! `(organization, credit type)` with no compare-and-swap and no idempotency
//! key; `GET .../limits/history` lists every write with its author and
//! reason, newest first, and answers 404 for an organization with no rows.
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

/// `spendLimit` in a request: integer nano-USD (scale 9) and a currency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpendLimitRequest {
    pub amount: i64,
    pub currency: String,
}

/// `spendLimit` in a response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpendLimit {
    pub amount: i64,
    pub scale: i64,
    pub currency: String,
}

/// Body of `PATCH /admin/organizations/{org_id}/limits`
/// (`cloud-api:crates/api/src/models.rs:4610`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateLimitsRequest {
    #[serde(rename = "type")]
    pub credit_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(rename = "spendLimit")]
    pub spend_limit: SpendLimitRequest,
    #[serde(rename = "changedBy", skip_serializing_if = "Option::is_none")]
    pub changed_by: Option<String>,
    #[serde(rename = "changeReason", skip_serializing_if = "Option::is_none")]
    pub change_reason: Option<String>,
}

/// Response of the PATCH (`models.rs:4656`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateLimitsResponse {
    pub organization_id: String,
    #[serde(rename = "type")]
    pub credit_type: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(rename = "spendLimit")]
    pub spend_limit: SpendLimit,
    pub updated_at: String,
}

/// One row of `GET .../limits/history` (`models.rs:4669`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LimitsHistoryEntry {
    pub id: String,
    #[serde(rename = "organizationId")]
    pub organization_id: String,
    #[serde(rename = "type")]
    pub credit_type: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(rename = "spendLimit")]
    pub spend_limit: SpendLimit,
    #[serde(rename = "effectiveFrom")]
    pub effective_from: String,
    #[serde(rename = "effectiveUntil")]
    pub effective_until: Option<String>,
    #[serde(rename = "changedBy")]
    pub changed_by: Option<String>,
    #[serde(rename = "changeReason")]
    pub change_reason: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

/// A page of history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LimitsHistoryPage {
    pub history: Vec<LimitsHistoryEntry>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

/// A failed admin call, by class. Carries no body, no id, no key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AdminApiError {
    /// The request never reached the server (connect, DNS, TLS).
    #[error("near_ai_admin_unreachable")]
    Unreachable,
    /// Sent, and no answer arrived in time. The write may have landed.
    #[error("near_ai_admin_ambiguous")]
    Ambiguous,
    /// 401: missing, expired or User-Agent-mismatched key.
    #[error("near_ai_admin_unauthorized")]
    Unauthorized,
    /// 403: the key lacks the permission (a read-only key on a write).
    #[error("near_ai_admin_forbidden")]
    Forbidden,
    /// 404 on a write: the organization is missing or inactive.
    #[error("near_ai_org_not_found")]
    NotFound,
    /// 400: NEAR AI refused the value.
    #[error("near_ai_limit_rejected")]
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
            Self::Rejected => "near_ai_limit_rejected",
            Self::Unavailable => "near_ai_admin_unavailable",
            Self::Malformed => "near_ai_admin_response_malformed",
        }
    }
}

/// The admin routes the rail needs.
#[async_trait]
pub trait NearAiAdminApi: Send + Sync {
    /// One page of history. `Ok(None)` is NEAR AI's 404 for an organization
    /// with no history rows.
    async fn limits_history_page(
        &self,
        org: NearAiOrgId,
        limit: i64,
        offset: i64,
    ) -> Result<Option<LimitsHistoryPage>, AdminApiError>;

    /// Set the absolute ceiling for one credit type.
    async fn update_limits(
        &self,
        org: NearAiOrgId,
        request: &UpdateLimitsRequest,
    ) -> Result<UpdateLimitsResponse, AdminApiError>;
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

    fn org_url(&self, org: NearAiOrgId, suffix: &str) -> String {
        format!(
            "{}/admin/organizations/{}/{}",
            self.base_url,
            org.as_uuid(),
            suffix
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
        // Anything after the connection opened may have reached the handler.
        AdminApiError::Ambiguous
    } else {
        AdminApiError::Unavailable
    }
}

#[async_trait]
impl NearAiAdminApi for HttpNearAiAdminApi {
    async fn limits_history_page(
        &self,
        org: NearAiOrgId,
        limit: i64,
        offset: i64,
    ) -> Result<Option<LimitsHistoryPage>, AdminApiError> {
        let response = self
            .http
            .get(self.org_url(org, "limits/history"))
            .query(&[("limit", limit), ("offset", offset)])
            .bearer_auth(self.key.expose_secret())
            .send()
            .await
            .map_err(|e| classify_send_error(&e, false))?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(classify_status(status));
        }
        response
            .json::<LimitsHistoryPage>()
            .await
            .map(Some)
            .map_err(|_| AdminApiError::Malformed)
    }

    async fn update_limits(
        &self,
        org: NearAiOrgId,
        request: &UpdateLimitsRequest,
    ) -> Result<UpdateLimitsResponse, AdminApiError> {
        let response = self
            .http
            .patch(self.org_url(org, "limits"))
            .bearer_auth(self.key.expose_secret())
            .json(request)
            .send()
            .await
            .map_err(|e| classify_send_error(&e, true))?;
        let status = response.status();
        if !status.is_success() {
            return Err(classify_status(status));
        }
        // A 2xx means the write committed; an unreadable body does not undo
        // it. The rail re-reads history either way.
        response
            .json::<UpdateLimitsResponse>()
            .await
            .map_err(|_| AdminApiError::Malformed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_update_body_matches_cloud_api_field_names() {
        let body = UpdateLimitsRequest {
            credit_type: "grant".into(),
            source: Some("trace-commons".into()),
            spend_limit: SpendLimitRequest {
                amount: 1_000_000_000,
                currency: "USD".into(),
            },
            changed_by: Some("trace-commons-settlement".into()),
            change_reason: Some("trace-commons:settle:v1:sha256:00".into()),
        };
        let v = serde_json::to_value(&body).unwrap();
        assert_eq!(v["type"], "grant");
        assert_eq!(v["spendLimit"]["amount"], 1_000_000_000i64);
        assert_eq!(v["changedBy"], "trace-commons-settlement");
        assert_eq!(v["changeReason"], "trace-commons:settle:v1:sha256:00");
        assert!(v.get("credit_type").is_none());
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
