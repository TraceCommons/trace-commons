// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Proving a payout recipient on the server, not the client
//! (docs/superpowers/specs/2026-10-21-nearai-credit-payout-design.md,
//! section 5, "Who can link" and "Verification on the server").
//!
//! Each rail's recipient-link kind has a verifier. For `near_ai` the proof
//! is an org id and a fresh NEAR AI access token. [`NearAiOrgVerifier`]
//! spends the token on one `GET {base}/users/me`, reads only the subject id
//! and each org's `id`, `role` and `is_active` (never a name), checks that
//! the login is the account's own, and checks the org is in the list,
//! active, and owned. The login is the account's when its subject is one of
//! the account's NEAR AI login anchors, or, for a NEAR AI user who signs in
//! with a NEAR wallet, when that wallet is one of the account's wallet
//! anchors (see [`wallet_account_id`]). An account with neither kind of
//! anchor is refused before the token is spent. It then asks the rail
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
    /// The account has no anchor a NEAR AI login could match: no NEAR AI
    /// login anchor, and no wallet anchor (or no NEAR network to key one
    /// under). A principal-linked legacy account is one. On the wire it
    /// is `subject_mismatch`, the generic refusal shipped apps already show;
    /// [`Self::log_reason`] tells the two apart in logs.
    NoLoginAnchor,
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
            Self::SubjectMismatch | Self::NoLoginAnchor => "subject_mismatch",
            Self::OrgNotOwned => "org_not_owned",
            Self::OrgUnavailable => "near_ai_org_unavailable",
            Self::OrgNotPermitted => "near_ai_org_not_permitted",
            Self::Unavailable => "payout_unavailable",
        }
    }

    /// The cause, for log lines only: [`Self::label`] except where two
    /// causes share a wire label.
    pub fn log_reason(self) -> &'static str {
        match self {
            Self::NoLoginAnchor => "no_near_ai_login",
            other => other.label(),
        }
    }
}

/// The account's anchors a NEAR AI login can be matched against
/// (`trace_near_account_anchors.anchor_hash`, by `identity_source`).
#[derive(Clone, Default, PartialEq, Eq)]
pub struct AccountAnchors {
    /// `near_ai_login` anchors: blind indexes of NEAR AI subjects.
    pub login: Vec<String>,
    /// `wallet` anchors: blind indexes of `(network, NEAR account)`.
    pub wallet: Vec<String>,
    /// The deployment's NEAR network, which wallet anchors are keyed under.
    /// `None` means no wallet anchor can be matched.
    pub network: Option<String>,
}

impl AccountAnchors {
    /// Nothing a login could match, so the token need not be spent.
    pub fn none_matchable(&self) -> bool {
        self.login.is_empty() && (self.wallet.is_empty() || self.network.is_none())
    }
}

impl std::fmt::Debug for AccountAnchors {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AccountAnchors(..)")
    }
}

/// Proves a recipient for one rail.
#[async_trait]
pub trait RecipientProofVerifier: Send + Sync {
    /// `anchors` are what the proving login must match. An account with
    /// none it could match is refused as `NoLoginAnchor`.
    async fn verify(
        &self,
        proof: RecipientProof,
        anchors: &AccountAnchors,
    ) -> Result<VerifiedRecipient, RecipientProofRefusal>;
}

/// What `/users/me` is read into, and nothing else. `serde` drops every
/// other field, so an org's name never becomes a value in this process. Do
/// not add `name`, a `flatten` map or a `serde_json::Value` here.
///
/// `auth_provider` and `email` are read only to recover the NEAR account of
/// a wallet sign-in ([`wallet_account_id`]); the address is taken out and
/// zeroized before anything else is read, and never logged or stored.
#[derive(Deserialize)]
struct UsersMe {
    id: String,
    #[serde(default)]
    auth_provider: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    organizations: Option<Vec<UsersMeOrganization>>,
}

/// The NEAR account a NEAR AI wallet sign-in names, if this login is one.
///
/// NEAR AI signs a wallet in only after verifying a NEP-413 signature and
/// the account's access key, and then stores the user with provider `near`
/// and login address `{account_id}@near` (nearai/cloud-api
/// `crates/services/src/auth/near.rs`, `crates/services/src/email.rs`;
/// `/v1/users/me` documents the placeholder in `crates/api/src/models.rs`).
/// The address is not editable, and a linked identity never rewrites it.
/// Stricter than NEAR AI's own parser: exact `near` domain, and a NEAR
/// account id's lowercase alphabet. A user whose primary sign-in is not the
/// wallet names no wallet here, even if a wallet is linked to it.
fn wallet_account_id<'a>(auth_provider: Option<&str>, email: &'a str) -> Option<&'a str> {
    if auth_provider != Some("near") {
        return None;
    }
    let (account, domain) = email.rsplit_once('@')?;
    let valid = domain == "near"
        && (2..=64).contains(&account.len())
        && account.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        });
    valid.then_some(account)
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
        anchors: &AccountAnchors,
    ) -> Result<VerifiedRecipient, RecipientProofRefusal> {
        let RecipientProof::NearAiOrg {
            organization_id,
            access_token,
        } = proof;
        // An account with nothing a login could match links nothing (owner
        // decision 2026-11-02). Refused before the token is spent.
        if anchors.none_matchable() {
            return Err(RecipientProofRefusal::NoLoginAnchor);
        }
        let mut user = self.users_me(&access_token).await?;
        drop(access_token);
        let email = Zeroizing::new(user.email.take().unwrap_or_default());
        let subject_id = Zeroizing::new(user.id.trim().to_owned());
        let login_match = anchors
            .login
            .contains(&self.identity.login_index_label(&subject_id));
        let wallet_match = match (
            anchors.network.as_deref(),
            wallet_account_id(user.auth_provider.as_deref(), &email),
        ) {
            (Some(network), Some(account)) => anchors
                .wallet
                .contains(&self.identity.index_label(network, account)),
            _ => false,
        };
        drop(email);
        if !(login_match || wallet_match) {
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

    /// The account's login anchors when it is anchored on the token's
    /// subject.
    fn anchored() -> AccountAnchors {
        AccountAnchors {
            login: vec![identity().login_index_label(SUBJECT)],
            ..AccountAnchors::default()
        }
    }

    /// A wallet-origin account: one wallet anchor for `account` on mainnet,
    /// no NEAR AI login anchor.
    fn wallet_anchored(account: &str) -> AccountAnchors {
        AccountAnchors {
            login: Vec::new(),
            wallet: vec![identity().index_label("mainnet", account)],
            network: Some("mainnet".to_owned()),
        }
    }

    /// `/users/me` as NEAR AI answers it for a user who signs in with a
    /// NEAR wallet: provider `near`, login address `{account_id}@near`.
    fn wallet_me(auth_provider: &str, email: &str) -> serde_json::Value {
        let mut me = me(owned_org("owner", true));
        me["auth_provider"] = serde_json::json!(auth_provider);
        me["email"] = serde_json::json!(email);
        me
    }

    /// A NEAR AI wallet sign-in is the wallet: its login address names the
    /// NEAR account, and the account's wallet anchor is that account's
    /// blind index. Nothing else names a wallet.
    #[tokio::test]
    async fn a_near_ai_wallet_login_matches_the_accounts_wallet_anchor() {
        let (base, _mock) = serve(Mock {
            users_me: Some((200, wallet_me("near", "alice.near@near"))),
            probe: Some(no_op()),
            ..Mock::default()
        })
        .await;
        let verifier = verifier(&base);
        verifier
            .verify(proof(), &wallet_anchored("alice.near"))
            .await
            .unwrap();
        // Another wallet's anchor does not match.
        assert_eq!(
            verifier
                .verify(proof(), &wallet_anchored("bob.near"))
                .await
                .unwrap_err(),
            RecipientProofRefusal::SubjectMismatch
        );
        // The same name keyed under another network does not match.
        let mut testnet = wallet_anchored("alice.near");
        testnet.wallet = vec![identity().index_label("testnet", "alice.near")];
        assert_eq!(
            verifier.verify(proof(), &testnet).await.unwrap_err(),
            RecipientProofRefusal::SubjectMismatch
        );
    }

    #[tokio::test]
    async fn only_a_near_provider_placeholder_address_names_a_wallet() {
        for (provider, email) in [
            ("github", "alice.near@near"),
            ("google", "alice.near@near"),
            ("near", "alice.near@near.org"),
            ("near", "alice.near@NEAR"),
            ("near", "Alice.near@near"),
            ("near", "@near"),
            ("near", "alice near@near"),
            ("near", "alice.near"),
            ("", "alice.near@near"),
        ] {
            let (base, _mock) = serve(Mock {
                users_me: Some((200, wallet_me(provider, email))),
                probe: Some(no_op()),
                ..Mock::default()
            })
            .await;
            assert_eq!(
                verifier(&base)
                    .verify(proof(), &wallet_anchored("alice.near"))
                    .await
                    .unwrap_err(),
                RecipientProofRefusal::SubjectMismatch,
                "{provider} {email}"
            );
        }
    }

    /// Wallet anchors without the deployment's network cannot be matched:
    /// with no login anchor either, the token is never spent.
    #[tokio::test]
    async fn wallet_anchors_without_a_network_are_no_anchor() {
        let (base, mock) = serve(Mock {
            users_me: Some((200, wallet_me("near", "alice.near@near"))),
            probe: Some(no_op()),
            ..Mock::default()
        })
        .await;
        let mut anchors = wallet_anchored("alice.near");
        anchors.network = None;
        assert_eq!(
            verifier(&base).verify(proof(), &anchors).await.unwrap_err(),
            RecipientProofRefusal::NoLoginAnchor
        );
        assert_eq!(mock.lock().unwrap().users_me_calls, 0);
    }

    /// An account with no NEAR AI login anchor (a wallet-origin account)
    /// links nothing, and the token is never spent (owner decision
    /// 2026-11-02, B9 in the one-connected-account spec).
    #[tokio::test]
    async fn an_account_with_no_login_anchor_is_refused_before_users_me() {
        let (base, mock) = serve(Mock {
            users_me: Some((200, me(owned_org("owner", true)))),
            probe: Some(no_op()),
            ..Mock::default()
        })
        .await;
        assert_eq!(
            verifier(&base)
                .verify(proof(), &AccountAnchors::default())
                .await
                .unwrap_err(),
            RecipientProofRefusal::NoLoginAnchor
        );
        let mock = mock.lock().unwrap();
        assert_eq!(mock.users_me_calls, 0);
        assert!(mock.bearer_seen.is_empty());
        assert!(mock.probe_bodies.is_empty());
    }

    #[tokio::test]
    async fn an_owned_active_org_passes_and_is_probed_once() {
        let (base, mock) = serve(Mock {
            users_me: Some((200, me(owned_org("owner", true)))),
            probe: Some(no_op()),
            ..Mock::default()
        })
        .await;
        let verified = verifier(&base).verify(proof(), &anchored()).await.unwrap();
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
                verifier(&base)
                    .verify(proof(), &anchored())
                    .await
                    .unwrap_err(),
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
            verifier
                .verify(
                    proof(),
                    &AccountAnchors {
                        login: vec![other],
                        ..AccountAnchors::default()
                    }
                )
                .await
                .unwrap_err(),
            RecipientProofRefusal::SubjectMismatch
        );
        let own = identity().login_index_label(SUBJECT);
        verifier
            .verify(
                proof(),
                &AccountAnchors {
                    login: vec![own],
                    ..AccountAnchors::default()
                },
            )
            .await
            .unwrap();
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
                verifier(&base)
                    .verify(proof(), &anchored())
                    .await
                    .unwrap_err(),
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
                verifier(&base)
                    .verify(proof(), &anchored())
                    .await
                    .unwrap_err(),
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
            (
                RecipientProofRefusal::NoLoginAnchor,
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
        assert_eq!(
            RecipientProofRefusal::NoLoginAnchor.log_reason(),
            "no_near_ai_login"
        );
        assert_eq!(
            RecipientProofRefusal::SubjectMismatch.log_reason(),
            "subject_mismatch"
        );
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
