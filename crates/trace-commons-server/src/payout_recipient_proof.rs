// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Proving a payout recipient on the server, not the client
//! (docs/superpowers/specs/2026-10-21-nearai-credit-payout-design.md,
//! section 5, "Who can link" and "Verification on the server").
//!
//! Each rail's recipient-link kind has a verifier. For `near_ai` the proof
//! is an org id and a fresh NEAR AI access token. [`NearAiOrgVerifier`]
//! spends the token on one `GET {base}/users/me`, reads only the subject id
//! and each org's `id`, `role` and `is_active` (never a name), checks the
//! subject against the account's NEAR AI login anchor when it has one, and
//! checks the org is in the list, active, and owned. It then asks the rail
//! whether the org can receive (`probe_recipient`: a zero cumulative send on
//! the probe program, which writes nothing). The token is dropped when the
//! call returns; it is never stored, logged or placed in an error.
//!
//! Since the owner superseded the passkey confirmation on 2026-10-23, this
//! check and the account session are what stand behind a link: the session
//! says which account, this says the person can sign in to NEAR AI as an
//! owner of the org.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use trace_commons_gate_api::{PayoutRail, RailRefusal, RecipientRef};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::near_account_identity::NearAccountIdentity;

/// How long the `/users/me` call may take. The probe has the rail's own
/// timeout.
pub const NEAR_AI_USERS_ME_TIMEOUT: Duration = Duration::from_secs(10);

/// The NEAR AI role that may direct credit to an org.
const OWNER_ROLE: &str = "owner";

/// A recipient proof, rail by rail. The token is a secret from the moment
/// the request body is read.
pub enum RecipientProof {
    NearAiOrg {
        organization_id: Uuid,
        access_token: SecretString,
    },
}

impl std::fmt::Debug for RecipientProof {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("RecipientProof(..)")
    }
}

impl From<trace_commons_protocol::payout_link::PayoutLinkProof> for RecipientProof {
    fn from(proof: trace_commons_protocol::payout_link::PayoutLinkProof) -> Self {
        match proof {
            trace_commons_protocol::payout_link::PayoutLinkProof::NearAi {
                organization_id,
                near_ai_access_token,
            } => Self::NearAiOrg {
                organization_id,
                access_token: SecretString::from(near_ai_access_token),
            },
        }
    }
}

/// What a passed proof yields: the recipient to seal and the subject that
/// proved it, to index. Neither is printed by `Debug`.
pub struct VerifiedRecipient {
    pub recipient: RecipientRef,
    pub subject_id: Zeroizing<String>,
}

impl std::fmt::Debug for VerifiedRecipient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("VerifiedRecipient(..)")
    }
}

/// Why a proof failed. Each maps to one outcome label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecipientProofRefusal {
    /// NEAR AI did not answer who the token belongs to.
    LoginUnverified,
    /// The account is anchored on another NEAR AI subject.
    SubjectMismatch,
    /// The org is not in the list, not active, or not owned.
    OrgNotOwned,
    /// The rail answered that the org is gone.
    OrgUnavailable,
    /// The rail's recipient list does not include the org.
    OrgNotPermitted,
    /// Anything on our side, or a reply we cannot place.
    Unavailable,
}

impl RecipientProofRefusal {
    pub fn label(self) -> &'static str {
        match self {
            Self::LoginUnverified => "near_ai_login_unverified",
            Self::SubjectMismatch => "subject_mismatch",
            Self::OrgNotOwned => "org_not_owned",
            Self::OrgUnavailable => "near_ai_org_unavailable",
            Self::OrgNotPermitted => "near_ai_org_not_permitted",
            Self::Unavailable => "payout_unavailable",
        }
    }
}

/// Proves a recipient for one rail.
#[async_trait]
pub trait RecipientProofVerifier: Send + Sync {
    /// `login_anchors` are the account's NEAR AI login anchors
    /// (`trace_near_account_anchors.anchor_hash` with `identity_source =
    /// 'near_ai_login'`); empty for an account with none.
    async fn verify(
        &self,
        proof: RecipientProof,
        login_anchors: &[String],
    ) -> Result<VerifiedRecipient, RecipientProofRefusal>;
}

/// What `/users/me` is read into, and nothing else. `serde` drops every
/// other field, so an org's name never becomes a value in this process. Do
/// not add `name`, a `flatten` map or a `serde_json::Value` here.
#[derive(Deserialize)]
struct UsersMe {
    id: String,
    #[serde(default)]
    organizations: Option<Vec<UsersMeOrganization>>,
}

#[derive(Deserialize)]
struct UsersMeOrganization {
    id: String,
    role: String,
    is_active: bool,
}

/// The `near_ai` verifier.
pub struct NearAiOrgVerifier {
    base_url: String,
    rail: Arc<dyn PayoutRail>,
    identity: Arc<NearAccountIdentity>,
    timeout: Duration,
}

impl NearAiOrgVerifier {
    /// `base_url` is the API root including `/v1`: in a shipped build
    /// [`crate::near_ai_api::NEAR_AI_API_BASE_URL`], the same compiled-in
    /// host the rail pays, so the host that verified an org is the host that
    /// pays it.
    pub fn new(
        base_url: impl Into<String>,
        rail: Arc<dyn PayoutRail>,
        identity: Arc<NearAccountIdentity>,
        timeout: Duration,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            rail,
            identity,
            timeout,
        }
    }

    async fn users_me(&self, token: &SecretString) -> Result<UsersMe, RecipientProofRefusal> {
        let http = reqwest::Client::builder()
            .timeout(self.timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| RecipientProofRefusal::Unavailable)?;
        let response = http
            .get(format!("{}/users/me", self.base_url.trim_end_matches('/')))
            .bearer_auth(token.expose_secret())
            .send()
            .await
            .map_err(|_| RecipientProofRefusal::LoginUnverified)?;
        if !response.status().is_success() {
            return Err(RecipientProofRefusal::LoginUnverified);
        }
        let user: UsersMe = response
            .json()
            .await
            .map_err(|_| RecipientProofRefusal::LoginUnverified)?;
        let subject = user.id.trim();
        if subject.is_empty() || subject.len() > 256 {
            return Err(RecipientProofRefusal::LoginUnverified);
        }
        Ok(user)
    }
}

/// Maps the rail's probe answer (spec section 5, step 5). Only the org's
/// own answers name the org; everything else is ours and blames nothing.
pub fn near_ai_probe_refusal(refusal: RailRefusal) -> RecipientProofRefusal {
    match refusal {
        RailRefusal::RecipientUnavailable => RecipientProofRefusal::OrgUnavailable,
        RailRefusal::HoldLine("near_ai_org_not_permitted") => {
            RecipientProofRefusal::OrgNotPermitted
        }
        _ => RecipientProofRefusal::Unavailable,
    }
}

#[async_trait]
impl RecipientProofVerifier for NearAiOrgVerifier {
    async fn verify(
        &self,
        proof: RecipientProof,
        login_anchors: &[String],
    ) -> Result<VerifiedRecipient, RecipientProofRefusal> {
        let RecipientProof::NearAiOrg {
            organization_id,
            access_token,
        } = proof;
        let user = self.users_me(&access_token).await?;
        drop(access_token);
        let subject_id = Zeroizing::new(user.id.trim().to_owned());
        if !login_anchors.is_empty()
            && !login_anchors.contains(&self.identity.login_index_label(&subject_id))
        {
            return Err(RecipientProofRefusal::SubjectMismatch);
        }
        let owned = user.organizations.unwrap_or_default().iter().any(|org| {
            Uuid::parse_str(org.id.trim()).is_ok_and(|id| id == organization_id)
                && org.is_active
                && org.role == OWNER_ROLE
        });
        if !owned {
            return Err(RecipientProofRefusal::OrgNotOwned);
        }
        let recipient = RecipientRef::new(organization_id.to_string())
            .map_err(|_| RecipientProofRefusal::Unavailable)?;
        self.rail
            .probe_recipient(&recipient)
            .await
            .map_err(near_ai_probe_refusal)?;
        Ok(VerifiedRecipient {
            recipient,
            subject_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::near_ai_rail::NearAiRail;
    use crate::near_ai_transfer::{
        NearAiCreditSource, NearAiTransferClient, NearAiTransferCredentials,
    };
    use crate::secrets::SecretsCrypto;
    use crate::trace_artifact_kek::{KmsKeyWrapper, LocalMasterKeyWrapper};
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::{IntoResponse, Response};
    use axum::routing::{get, post};
    use base64::Engine as _;
    use std::sync::Mutex;

    const TOKEN: &str = "synthetic-near-ai-token-SENTINEL";
    const SUBJECT: &str = "synthetic-subject-7";
    const NAME_SENTINEL: &str = "ORG-NAME-SENTINEL";
    const CREDENTIAL: &str = "oc_00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

    fn org() -> Uuid {
        Uuid::from_u128(0x4ec1_0000_0000_4000_8000_0000_0000_0001)
    }

    #[derive(Default)]
    struct Mock {
        users_me: Option<(u16, serde_json::Value)>,
        probe: Option<(u16, serde_json::Value)>,
        users_me_calls: usize,
        probe_bodies: Vec<serde_json::Value>,
        bearer_seen: Vec<String>,
    }

    type Shared = Arc<Mutex<Mock>>;

    async fn users_me(State(state): State<Shared>, headers: HeaderMap) -> Response {
        let mut mock = state.lock().unwrap();
        mock.users_me_calls += 1;
        if let Some(bearer) = headers.get("authorization").and_then(|v| v.to_str().ok()) {
            mock.bearer_seen.push(bearer.to_owned());
        }
        let (status, body) = mock.users_me.clone().unwrap();
        (StatusCode::from_u16(status).unwrap(), axum::Json(body)).into_response()
    }

    async fn transfer(
        State(state): State<Shared>,
        body: axum::Json<serde_json::Value>,
    ) -> Response {
        let mut mock = state.lock().unwrap();
        mock.probe_bodies.push(body.0);
        let (status, body) = mock.probe.clone().unwrap();
        (StatusCode::from_u16(status).unwrap(), axum::Json(body)).into_response()
    }

    async fn serve(mock: Mock) -> (String, Shared) {
        let shared: Shared = Arc::new(Mutex::new(mock));
        let app = axum::Router::new()
            .route("/v1/users/me", get(users_me))
            .route("/v1/organizations/{org}/credit/transfer", post(transfer))
            .with_state(shared.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}/v1"), shared)
    }

    fn identity() -> Arc<NearAccountIdentity> {
        let crypto = SecretsCrypto::new(SecretString::from("a".repeat(32))).unwrap();
        let kek: Arc<dyn KmsKeyWrapper> =
            Arc::new(LocalMasterKeyWrapper::new(crypto, "payout-proof-fixture"));
        Arc::new(
            NearAccountIdentity::from_parts(
                Some(&base64::engine::general_purpose::STANDARD.encode([5u8; 32])),
                Some(kek),
            )
            .unwrap(),
        )
    }

    fn verifier(base: &str) -> NearAiOrgVerifier {
        let client = NearAiTransferClient::at_loopback(
            base,
            NearAiTransferCredentials {
                token: SecretString::from(CREDENTIAL),
                credential_id: Some(Uuid::from_u128(2)),
                sender_org: Uuid::from_u128(3),
                source: NearAiCreditSource::Payment,
            },
            Duration::from_secs(5),
        )
        .unwrap();
        NearAiOrgVerifier::new(
            base,
            Arc::new(NearAiRail::new(client)),
            identity(),
            Duration::from_secs(5),
        )
    }

    fn me(orgs: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "id": SUBJECT,
            "email": "synthetic@example.invalid",
            "username": "synthetic",
            "organizations": orgs,
            "workspaces": [],
        })
    }

    fn owned_org(role: &str, active: bool) -> serde_json::Value {
        serde_json::json!([{
            "id": org().to_string(), "name": NAME_SENTINEL, "description": null,
            "role": role, "is_active": active, "created_at": "2026-10-01T00:00:00Z",
        }])
    }

    fn no_op() -> (u16, serde_json::Value) {
        (
            200,
            serde_json::json!({"amount": 0, "recipient_cumulative": 0, "no_op": true}),
        )
    }

    fn proof() -> RecipientProof {
        RecipientProof::NearAiOrg {
            organization_id: org(),
            access_token: SecretString::from(TOKEN),
        }
    }

    #[tokio::test]
    async fn an_owned_active_org_passes_and_is_probed_once() {
        let (base, mock) = serve(Mock {
            users_me: Some((200, me(owned_org("owner", true)))),
            probe: Some(no_op()),
            ..Mock::default()
        })
        .await;
        let verified = verifier(&base).verify(proof(), &[]).await.unwrap();
        assert_eq!(verified.recipient.expose(), org().to_string());
        assert_eq!(verified.subject_id.as_str(), SUBJECT);
        assert!(!format!("{verified:?}").contains(SUBJECT));
        let mock = mock.lock().unwrap();
        assert_eq!(mock.users_me_calls, 1);
        assert_eq!(mock.bearer_seen, vec![format!("Bearer {TOKEN}")]);
        assert_eq!(mock.probe_bodies.len(), 1);
        let probe = &mock.probe_bodies[0];
        assert_eq!(probe["recipient_cumulative"], 0);
        assert_eq!(probe["payout_program"], "tcprobe-v1");
        assert_eq!(probe["recipient_organization_id"], org().to_string());
        assert!(!probe.to_string().contains(TOKEN));
    }

    #[tokio::test]
    async fn a_non_owner_inactive_or_absent_org_is_refused_before_the_probe() {
        for orgs in [
            owned_org("admin", true),
            owned_org("member", true),
            owned_org("owner", false),
            owned_org("Owner", true),
            serde_json::json!([]),
            serde_json::json!(null),
        ] {
            let (base, mock) = serve(Mock {
                users_me: Some((200, me(orgs.clone()))),
                probe: Some(no_op()),
                ..Mock::default()
            })
            .await;
            assert_eq!(
                verifier(&base).verify(proof(), &[]).await.unwrap_err(),
                RecipientProofRefusal::OrgNotOwned,
                "{orgs}"
            );
            assert!(mock.lock().unwrap().probe_bodies.is_empty(), "{orgs}");
        }
    }

    #[tokio::test]
    async fn the_subject_must_match_the_accounts_login_anchor() {
        let (base, _mock) = serve(Mock {
            users_me: Some((200, me(owned_org("owner", true)))),
            probe: Some(no_op()),
            ..Mock::default()
        })
        .await;
        let verifier = verifier(&base);
        let other = identity().login_index_label("another-subject");
        assert_eq!(
            verifier.verify(proof(), &[other]).await.unwrap_err(),
            RecipientProofRefusal::SubjectMismatch
        );
        let own = identity().login_index_label(SUBJECT);
        verifier.verify(proof(), &[own]).await.unwrap();
    }

    #[tokio::test]
    async fn an_unverified_login_is_refused() {
        for (status, body) in [
            (401, serde_json::json!({"error": "unauthorized"})),
            (200, serde_json::json!({"organizations": []})),
            (200, serde_json::json!({"id": "   "})),
            (302, serde_json::json!({})),
        ] {
            let (base, _mock) = serve(Mock {
                users_me: Some((status, body.clone())),
                probe: Some(no_op()),
                ..Mock::default()
            })
            .await;
            assert_eq!(
                verifier(&base).verify(proof(), &[]).await.unwrap_err(),
                RecipientProofRefusal::LoginUnverified,
                "{status} {body}"
            );
        }
    }

    #[tokio::test]
    async fn probe_404_and_409_and_403_map_to_their_outcomes() {
        let error = |code: &str| serde_json::json!({"error": {"type": code, "message": code}});
        for (status, code, want) in [
            (
                404,
                "organization_unavailable",
                RecipientProofRefusal::OrgUnavailable,
            ),
            (
                409,
                "credential_transfer_limit_exceeded",
                RecipientProofRefusal::OrgNotPermitted,
            ),
            (
                409,
                "organization_transfer_limit_exceeded",
                RecipientProofRefusal::Unavailable,
            ),
            (403, "forbidden", RecipientProofRefusal::Unavailable),
            (404, "not_found", RecipientProofRefusal::Unavailable),
            (500, "internal", RecipientProofRefusal::Unavailable),
        ] {
            let (base, _mock) = serve(Mock {
                users_me: Some((200, me(owned_org("owner", true)))),
                probe: Some((status, error(code))),
                ..Mock::default()
            })
            .await;
            assert_eq!(
                verifier(&base).verify(proof(), &[]).await.unwrap_err(),
                want,
                "{status} {code}"
            );
        }
    }

    #[test]
    fn refusal_labels_are_the_route_outcomes() {
        use trace_commons_protocol::payout_link::PayoutLinkOutcome as Outcome;
        for (refusal, outcome) in [
            (
                RecipientProofRefusal::LoginUnverified,
                Outcome::NearAiLoginUnverified,
            ),
            (
                RecipientProofRefusal::SubjectMismatch,
                Outcome::SubjectMismatch,
            ),
            (RecipientProofRefusal::OrgNotOwned, Outcome::OrgNotOwned),
            (
                RecipientProofRefusal::OrgUnavailable,
                Outcome::NearAiOrgUnavailable,
            ),
            (
                RecipientProofRefusal::OrgNotPermitted,
                Outcome::NearAiOrgNotPermitted,
            ),
            (
                RecipientProofRefusal::Unavailable,
                Outcome::PayoutUnavailable,
            ),
        ] {
            assert_eq!(refusal.label(), outcome.as_str());
        }
    }

    #[test]
    fn the_org_name_is_never_deserialized() {
        // A compile-time fact made visible: the struct has no name field, so
        // a body carrying one parses and the name is dropped.
        let parsed: UsersMe = serde_json::from_value(me(owned_org("owner", true))).unwrap();
        let orgs = parsed.organizations.unwrap();
        assert_eq!(orgs.len(), 1);
        assert_eq!(orgs[0].id, org().to_string());
        assert_eq!(orgs[0].role, "owner");
        assert!(orgs[0].is_active);
    }
}
