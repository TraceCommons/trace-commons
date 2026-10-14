// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Verifying, with the contributor's own NEAR AI token, that they may direct
//! inference credit to a NEAR AI organization. Needs no admin key.
//!
//! `GET {base}/users/me` lists the caller's organizations with their role
//! (`cloud-api:crates/api/src/routes/users.rs:172`, response
//! `models.rs:3170-3184`). This module reads **only** `organizations[].id`,
//! `.role` and `.is_active`, into its own struct: `near_ai_login`'s
//! `IntrospectedUser` is deliberately narrower and is not widened for this.
//!
//! NEAR AI answers 200 with an **empty** list when its own listing fails
//! (`users.rs:230-233`), so an empty list is "unverified", never "has no
//! organizations". Every failure is one label; the token and the
//! organization id never appear in an error.

use crate::near_ai_admin::NearAiOrgId;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RecipientRefused {
    /// The membership could not be established (transport, status, shape,
    /// or an empty list).
    #[error("near_ai_org_membership_unverified")]
    Unverified,
    /// The caller is not a member of the organization, or it is inactive.
    #[error("near_ai_org_not_a_member")]
    NotAMember,
    /// A member, but neither owner nor admin.
    #[error("near_ai_org_role_insufficient")]
    RoleInsufficient,
}

#[derive(Deserialize)]
struct MeOrganizations {
    #[serde(default)]
    organizations: Option<Vec<MeOrganization>>,
}

#[derive(Deserialize)]
struct MeOrganization {
    id: String,
    role: String,
    #[serde(default)]
    is_active: bool,
}

/// A verified election: the organization and the role that permits it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedRecipient {
    pub org: NearAiOrgId,
    /// `owner` or `admin`.
    pub role: &'static str,
}

/// Check that the bearer of `token` is an active `owner` or `admin` of `org`.
pub async fn verify_recipient(
    base_url: &str,
    token: &SecretString,
    org: NearAiOrgId,
    timeout: Duration,
) -> Result<VerifiedRecipient, RecipientRefused> {
    let url = format!("{}/users/me", base_url.trim_end_matches('/'));
    let http = reqwest::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| RecipientRefused::Unverified)?;
    let response = http
        .get(url)
        .bearer_auth(token.expose_secret())
        .send()
        .await
        .map_err(|_| RecipientRefused::Unverified)?;
    if !response.status().is_success() {
        return Err(RecipientRefused::Unverified);
    }
    let me: MeOrganizations = response
        .json()
        .await
        .map_err(|_| RecipientRefused::Unverified)?;
    let organizations = me.organizations.unwrap_or_default();
    if organizations.is_empty() {
        return Err(RecipientRefused::Unverified);
    }
    let Some(found) = organizations
        .iter()
        .find(|o| NearAiOrgId::parse(&o.id) == Some(org))
    else {
        return Err(RecipientRefused::NotAMember);
    };
    if !found.is_active {
        return Err(RecipientRefused::NotAMember);
    }
    let role = match found.role.to_ascii_lowercase().as_str() {
        "owner" => "owner",
        "admin" => "admin",
        _ => return Err(RecipientRefused::RoleInsufficient),
    };
    Ok(VerifiedRecipient { org, role })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const ORG: &str = "6f1c0b6a-0000-4000-8000-00000000abcd";

    async fn server(body: serde_json::Value, status: u16) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/users/me"))
            .and(header("authorization", "Bearer user-token"))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .mount(&server)
            .await;
        server
    }

    async fn check(
        body: serde_json::Value,
        status: u16,
    ) -> Result<VerifiedRecipient, RecipientRefused> {
        let s = server(body, status).await;
        verify_recipient(
            &format!("{}/v1", s.uri()),
            &SecretString::from("user-token".to_string()),
            NearAiOrgId::parse(ORG).unwrap(),
            Duration::from_secs(5),
        )
        .await
    }

    fn me(orgs: serde_json::Value) -> serde_json::Value {
        serde_json::json!({"id": "u", "email": "x@y", "organizations": orgs})
    }

    #[tokio::test]
    async fn an_owner_or_admin_of_an_active_org_is_verified() {
        for role in ["owner", "admin"] {
            let got = check(
                me(serde_json::json!([{"id": ORG, "role": role, "is_active": true, "name": "n"}])),
                200,
            )
            .await
            .unwrap();
            assert_eq!(got.role, role);
        }
    }

    #[tokio::test]
    async fn a_member_an_inactive_org_or_another_org_is_refused() {
        assert_eq!(
            check(
                me(serde_json::json!([{"id": ORG, "role": "member", "is_active": true}])),
                200
            )
            .await,
            Err(RecipientRefused::RoleInsufficient)
        );
        assert_eq!(
            check(
                me(serde_json::json!([{"id": ORG, "role": "owner", "is_active": false}])),
                200
            )
            .await,
            Err(RecipientRefused::NotAMember)
        );
        assert_eq!(
            check(
                me(serde_json::json!([{"id": "00000000-0000-4000-8000-000000000000", "role": "owner", "is_active": true}])),
                200
            )
            .await,
            Err(RecipientRefused::NotAMember)
        );
    }

    #[tokio::test]
    async fn an_empty_or_absent_list_and_a_failed_call_are_unverified_not_none() {
        assert_eq!(
            check(me(serde_json::json!([])), 200).await,
            Err(RecipientRefused::Unverified)
        );
        assert_eq!(
            check(serde_json::json!({"id": "u"}), 200).await,
            Err(RecipientRefused::Unverified)
        );
        assert_eq!(
            check(serde_json::json!({}), 401).await,
            Err(RecipientRefused::Unverified)
        );
    }
}
