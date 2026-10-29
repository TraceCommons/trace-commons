//! Wire contract for linking an account to a payout recipient on one rail
//! (docs/superpowers/specs/2026-10-21-nearai-credit-payout-design.md,
//! section 5).
//!
//! Three routes, all on the contributor's signed-in account session:
//!
//! - `POST /v1/account/payout-rails/{rail_id}/link-requests` starts a link
//!   or an unlink. The server verifies the proof, checks the recipient with
//!   the rail, and stores a request that changes nothing yet; the answer is
//!   `pending_confirmation` with a handle.
//! - `POST /v1/account/payout-rails/{rail_id}/link-requests/{handle}/confirm`
//!   confirms it from the same account. Only then is the link (or the unlink)
//!   saved.
//! - `GET /v1/account/payout-rails` lists each enabled rail's link state,
//!   never a recipient or a name.
//!
//! The owner superseded the passkey confirmation on 2026-10-23: the signed-in
//! session is the authority, there is no step-up page, and nothing requires
//! a passkey. The two calls stay so the app can show the consent between
//! them. `expected_current_version` is the "changed since shown" check: the
//! server refuses a start or a confirm when the account's link on that rail
//! changed after the app read its status.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// The contract version every response carries.
pub const PAYOUT_LINK_CONTRACT_VERSION: &str = "payout-link-v1";

/// The NEAR AI payout rail's id. Never `near`, which names the NEP-141 rail.
pub const NEAR_AI_PAYOUT_RAIL_ID: &str = "near_ai";

/// The longest NEAR AI access token the server accepts in a proof.
pub const MAX_NEAR_AI_ACCESS_TOKEN_LEN: usize = 8192;

/// What a link request asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayoutLinkAction {
    Link,
    Unlink,
}

/// The per-rail proof a `link` carries, tagged by rail id. Only `near_ai`
/// exists; any other tag fails to parse.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "rail", deny_unknown_fields)]
pub enum PayoutLinkProof {
    /// A NEAR AI organization the person owns, and a fresh NEAR AI access
    /// token the server spends once on `GET /v1/users/me` to check that,
    /// then drops. The token is never stored or logged.
    #[serde(rename = "near_ai")]
    NearAi {
        organization_id: Uuid,
        near_ai_access_token: String,
    },
}

impl PayoutLinkProof {
    /// The rail this proof is for.
    pub fn rail_id(&self) -> &'static str {
        match self {
            Self::NearAi { .. } => NEAR_AI_PAYOUT_RAIL_ID,
        }
    }
}

impl std::fmt::Debug for PayoutLinkProof {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NearAi { .. } => formatter.debug_struct("NearAi").finish_non_exhaustive(),
        }
    }
}

/// The body of a start call.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PayoutLinkRequest {
    pub action: PayoutLinkAction,
    /// A fresh, non-nil UUID per attempt. A replay with the same key and the
    /// same request answers the same handle; the same key with a different
    /// request is `idempotency_conflict`.
    pub idempotency_key: Uuid,
    /// The `version` the app last read from the status route for this rail;
    /// `None` (or 0) when it read no link history.
    #[serde(default)]
    pub expected_current_version: Option<i64>,
    /// Required for `link`, refused for `unlink`.
    #[serde(default)]
    pub proof: Option<PayoutLinkProof>,
}

impl std::fmt::Debug for PayoutLinkRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PayoutLinkRequest")
            .field("action", &self.action)
            .field("expected_current_version", &self.expected_current_version)
            .field("proof", &self.proof)
            .finish_non_exhaustive()
    }
}

/// Why a request is malformed before anything is checked with anyone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayoutLinkRequestInvalid {
    NilIdempotencyKey,
    NegativeVersion,
    LinkWithoutProof,
    UnlinkWithProof,
    ProofForAnotherRail,
    TokenMissingOrTooLong,
    NilOrganization,
}

impl PayoutLinkRequest {
    /// The expected version as stored: `None` for "no link history".
    pub fn expected_version(&self) -> Option<i64> {
        self.expected_current_version.filter(|version| *version > 0)
    }

    /// Checks the shape against the rail named in the path.
    pub fn validate(&self, rail_id: &str) -> Result<(), PayoutLinkRequestInvalid> {
        if self.idempotency_key.is_nil() {
            return Err(PayoutLinkRequestInvalid::NilIdempotencyKey);
        }
        if self.expected_current_version.is_some_and(|v| v < 0) {
            return Err(PayoutLinkRequestInvalid::NegativeVersion);
        }
        match (self.action, &self.proof) {
            (PayoutLinkAction::Link, None) => Err(PayoutLinkRequestInvalid::LinkWithoutProof),
            (PayoutLinkAction::Unlink, Some(_)) => Err(PayoutLinkRequestInvalid::UnlinkWithProof),
            (PayoutLinkAction::Unlink, None) => Ok(()),
            (PayoutLinkAction::Link, Some(proof)) => {
                if proof.rail_id() != rail_id {
                    return Err(PayoutLinkRequestInvalid::ProofForAnotherRail);
                }
                match proof {
                    PayoutLinkProof::NearAi {
                        organization_id,
                        near_ai_access_token,
                    } => {
                        if organization_id.is_nil() {
                            return Err(PayoutLinkRequestInvalid::NilOrganization);
                        }
                        let token = near_ai_access_token.trim();
                        if token.is_empty() || token.len() > MAX_NEAR_AI_ACCESS_TOKEN_LEN {
                            return Err(PayoutLinkRequestInvalid::TokenMissingOrTooLong);
                        }
                        Ok(())
                    }
                }
            }
        }
    }
}

/// Every outcome label the link routes answer with. A refusal is the label
/// as the `error` of an error body; a success is the label as `outcome`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayoutLinkOutcome {
    PendingConfirmation,
    Confirmed,
    Expired,
    NotFound,
    IdempotencyConflict,
    VersionConflict,
    /// A link or unlink is already waiting for the rail's next pass.
    PayoutLinkChangePending,
    /// An unlink with no link to stop.
    PayoutNotLinked,
    PayoutRailUnknown,
    PayoutUnavailable,
    /// The account is closed.
    PayoutAccountIneligible,
    InvalidPayoutLinkRequest,
    // `near_ai` verifier refusals.
    SubjectMismatch,
    OrgNotOwned,
    NearAiLoginUnverified,
    NearAiOrgUnavailable,
    NearAiOrgNotPermitted,
}

impl PayoutLinkOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PendingConfirmation => "pending_confirmation",
            Self::Confirmed => "confirmed",
            Self::Expired => "expired",
            Self::NotFound => "not_found",
            Self::IdempotencyConflict => "idempotency_conflict",
            Self::VersionConflict => "version_conflict",
            Self::PayoutLinkChangePending => "payout_link_change_pending",
            Self::PayoutNotLinked => "payout_not_linked",
            Self::PayoutRailUnknown => "payout_rail_unknown",
            Self::PayoutUnavailable => "payout_unavailable",
            Self::PayoutAccountIneligible => "payout_account_ineligible",
            Self::InvalidPayoutLinkRequest => "invalid_payout_link_request",
            Self::SubjectMismatch => "subject_mismatch",
            Self::OrgNotOwned => "org_not_owned",
            Self::NearAiLoginUnverified => "near_ai_login_unverified",
            Self::NearAiOrgUnavailable => "near_ai_org_unavailable",
            Self::NearAiOrgNotPermitted => "near_ai_org_not_permitted",
        }
    }
}

/// The answer to a start call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PayoutLinkStarted {
    pub contract_version: String,
    pub rail_id: String,
    pub action: PayoutLinkAction,
    /// Always `pending_confirmation`.
    pub outcome: PayoutLinkOutcome,
    /// Opaque; pass it to the confirm route from the same account.
    pub handle: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    /// For a `link`, the recipient ref the confirmed link will report (see
    /// [`near_ai_recipient_ref`]).
    #[serde(default)]
    pub recipient_ref: Option<String>,
}

/// The answer to a confirm call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PayoutLinkConfirmed {
    pub contract_version: String,
    pub rail_id: String,
    pub action: PayoutLinkAction,
    /// Always `confirmed`.
    pub outcome: PayoutLinkOutcome,
    /// The account's link version on this rail after the change.
    pub version: i64,
}

/// Which credit a stream carries: the account's own (`earned`), or the
/// referral bonus it earns on the accounts it referred (`referral`;
/// docs/superpowers/specs/2026-10-29-referral-rewards-design.md, R7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayoutStreamKind {
    Earned,
    Referral,
}

/// One stream's amounts on a rail, in the rail's atomic units (nano-USD on
/// `near_ai`), as decimal strings so a `u128` survives JSON. `owed` is from
/// accruals (earned) or referral bonus rows (referral); `sent` sums that
/// stream's confirmed lines across every link; `waiting` is `owed - sent`
/// floored at zero.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PayoutStreamStatus {
    pub stream: PayoutStreamKind,
    pub owed: String,
    pub sent: String,
    pub waiting: String,
    /// The label of a hold that stops this stream's sends, when one does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold_label: Option<String>,
}

/// One rail's link state. Never a recipient, an org id or a name.
///
/// Not `deny_unknown_fields`, unlike the other answers here: a status gains
/// fields as rails gain streams, and an app reading an older contract
/// should show what it knows. `streams` is absent from an answer that has
/// none, so an answer without streams is the pre-referral shape exactly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayoutRailLinkStatus {
    pub rail_id: String,
    /// `disabled` never appears; `dry_run` or `live`.
    pub mode: String,
    /// The account has a link on this rail that is not closed.
    pub linked: bool,
    /// The newest link's recipient ref, which the app recomputes from the
    /// idempotency key and the org it chose.
    #[serde(default)]
    pub recipient_ref: Option<String>,
    /// The newest link's state: `waiting`, `active` or `closing`.
    #[serde(default)]
    pub state: Option<String>,
    /// The value to send as `expected_current_version`. 0 with no history.
    pub version: i64,
    /// `pending_confirmation` while an unexpired request waits.
    #[serde(default)]
    pub request_state: Option<String>,
    /// An unlink was confirmed and the rail has not closed the link yet.
    pub unlink_requested: bool,
    /// The rail answered that the recipient is gone.
    pub recipient_unavailable: bool,
    /// Own credit and referral credit, one entry per stream, earned first.
    /// Empty (and left out of the JSON) while the rail runs no referral
    /// stream.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub streams: Vec<PayoutStreamStatus>,
}

/// The answer to the status route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PayoutRailsStatus {
    pub contract_version: String,
    pub rails: Vec<PayoutRailLinkStatus>,
}

/// `sha256("nearai-payout-org-ref-v1:" || idempotency_key || ":" || org_id)`
/// with both UUIDs in their lowercase hyphenated form, as `sha256:` + hex.
/// The app keeps the key and the org id together, so it can recompute this
/// and check the server holds the org it chose without the server echoing
/// the org.
pub fn near_ai_recipient_ref(idempotency_key: Uuid, organization_id: Uuid) -> String {
    format!(
        "sha256:{:x}",
        Sha256::digest(
            format!("nearai-payout-org-ref-v1:{idempotency_key}:{organization_id}").as_bytes()
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "11111111-2222-4333-8444-555555555555";
    const ORG: &str = "66666666-7777-4888-9999-aaaaaaaaaaaa";
    const TOKEN: &str = "synthetic-access-token-SENTINEL";

    fn link_body() -> serde_json::Value {
        serde_json::json!({
            "action": "link",
            "idempotency_key": KEY,
            "expected_current_version": 2,
            "proof": {"rail": "near_ai", "organization_id": ORG, "near_ai_access_token": TOKEN},
        })
    }

    #[test]
    fn link_requests_refuse_unknown_fields_and_unknown_rails() {
        let parsed: PayoutLinkRequest = serde_json::from_value(link_body()).unwrap();
        assert_eq!(parsed.action, PayoutLinkAction::Link);
        assert_eq!(parsed.expected_version(), Some(2));
        parsed.validate(NEAR_AI_PAYOUT_RAIL_ID).unwrap();

        let mut extra = link_body();
        extra["account_id"] = serde_json::json!("x");
        assert!(serde_json::from_value::<PayoutLinkRequest>(extra).is_err());

        let mut proof_extra = link_body();
        proof_extra["proof"]["name"] = serde_json::json!("Synthetic Org");
        assert!(serde_json::from_value::<PayoutLinkRequest>(proof_extra).is_err());

        for rail in ["near", "test_keyed", "NEAR_AI"] {
            let mut other = link_body();
            other["proof"]["rail"] = serde_json::json!(rail);
            assert!(
                serde_json::from_value::<PayoutLinkRequest>(other).is_err(),
                "{rail}"
            );
        }

        // A proof for near_ai sent to another rail's path is refused.
        assert_eq!(
            parsed.validate("test_keyed"),
            Err(PayoutLinkRequestInvalid::ProofForAnotherRail)
        );

        let mut nil = link_body();
        nil["idempotency_key"] = serde_json::json!(Uuid::nil());
        let nil: PayoutLinkRequest = serde_json::from_value(nil).unwrap();
        assert_eq!(
            nil.validate(NEAR_AI_PAYOUT_RAIL_ID),
            Err(PayoutLinkRequestInvalid::NilIdempotencyKey)
        );

        let unlink: PayoutLinkRequest = serde_json::from_value(serde_json::json!({
            "action": "unlink", "idempotency_key": KEY,
        }))
        .unwrap();
        unlink.validate(NEAR_AI_PAYOUT_RAIL_ID).unwrap();
        assert_eq!(unlink.expected_version(), None);
        let mut unlink_with_proof = link_body();
        unlink_with_proof["action"] = serde_json::json!("unlink");
        let unlink_with_proof: PayoutLinkRequest =
            serde_json::from_value(unlink_with_proof).unwrap();
        assert_eq!(
            unlink_with_proof.validate(NEAR_AI_PAYOUT_RAIL_ID),
            Err(PayoutLinkRequestInvalid::UnlinkWithProof)
        );
        let link_without: PayoutLinkRequest = serde_json::from_value(serde_json::json!({
            "action": "link", "idempotency_key": KEY,
        }))
        .unwrap();
        assert_eq!(
            link_without.validate(NEAR_AI_PAYOUT_RAIL_ID),
            Err(PayoutLinkRequestInvalid::LinkWithoutProof)
        );
        let mut empty_token = link_body();
        empty_token["proof"]["near_ai_access_token"] = serde_json::json!("  ");
        let empty_token: PayoutLinkRequest = serde_json::from_value(empty_token).unwrap();
        assert_eq!(
            empty_token.validate(NEAR_AI_PAYOUT_RAIL_ID),
            Err(PayoutLinkRequestInvalid::TokenMissingOrTooLong)
        );
        let mut zero = link_body();
        zero["expected_current_version"] = serde_json::json!(0);
        let zero: PayoutLinkRequest = serde_json::from_value(zero).unwrap();
        assert_eq!(zero.expected_version(), None);
    }

    #[test]
    fn the_access_token_never_appears_in_debug() {
        let parsed: PayoutLinkRequest = serde_json::from_value(link_body()).unwrap();
        let printed = format!("{parsed:?} {:?}", parsed.proof);
        assert!(!printed.contains(TOKEN), "{printed}");
        assert!(!printed.contains("SENTINEL"), "{printed}");
        assert!(!printed.contains(ORG), "{printed}");
        assert!(!printed.contains(KEY), "{printed}");
    }

    #[test]
    fn recipient_ref_is_recomputable_by_the_client() {
        let key = Uuid::parse_str(KEY).unwrap();
        let org = Uuid::parse_str(ORG).unwrap();
        let expected = format!(
            "sha256:{}",
            hex::encode(Sha256::digest(
                format!("nearai-payout-org-ref-v1:{KEY}:{ORG}").as_bytes()
            ))
        );
        assert_eq!(near_ai_recipient_ref(key, org), expected);
        assert_eq!(near_ai_recipient_ref(key, org).len(), 71);
        assert_ne!(
            near_ai_recipient_ref(key, org),
            near_ai_recipient_ref(Uuid::new_v4(), org),
            "a fresh key gives a fresh ref, so the ref does not name the org"
        );
        assert!(!near_ai_recipient_ref(key, org).contains(ORG));
    }

    fn status_without_streams() -> serde_json::Value {
        serde_json::json!({
            "rail_id": "near_ai",
            "mode": "live",
            "linked": true,
            "recipient_ref": null,
            "state": "active",
            "version": 1,
            "request_state": null,
            "unlink_requested": false,
            "recipient_unavailable": false,
        })
    }

    #[test]
    fn a_status_without_streams_reads_and_writes_the_old_shape() {
        let parsed: PayoutRailLinkStatus =
            serde_json::from_value(status_without_streams()).unwrap();
        assert!(parsed.streams.is_empty());
        let written = serde_json::to_value(&parsed).unwrap();
        assert!(written.get("streams").is_none(), "{written}");
        assert_eq!(written, status_without_streams());
        // A newer server's field an older app does not know is tolerated.
        let mut newer = status_without_streams();
        newer["something_new"] = serde_json::json!(1);
        assert!(serde_json::from_value::<PayoutRailLinkStatus>(newer).is_ok());
    }

    #[test]
    fn streams_round_trip_with_amounts_as_strings() {
        let mut body = status_without_streams();
        body["streams"] = serde_json::json!([
            {"stream": "earned", "owed": "340282366920938463463374607431768211455",
             "sent": "0", "waiting": "340282366920938463463374607431768211455"},
            {"stream": "referral", "owed": "100", "sent": "40", "waiting": "60",
             "hold_label": "referral_code_suspended"},
        ]);
        let parsed: PayoutRailLinkStatus = serde_json::from_value(body.clone()).unwrap();
        assert_eq!(parsed.streams.len(), 2);
        assert_eq!(parsed.streams[0].stream, PayoutStreamKind::Earned);
        assert_eq!(parsed.streams[0].owed.parse::<u128>().unwrap(), u128::MAX);
        assert_eq!(parsed.streams[0].hold_label, None);
        assert_eq!(parsed.streams[1].stream, PayoutStreamKind::Referral);
        assert_eq!(
            parsed.streams[1].hold_label.as_deref(),
            Some("referral_code_suspended")
        );
        let written = serde_json::to_value(&parsed).unwrap();
        assert_eq!(written, body);
        assert!(written["streams"][0].get("hold_label").is_none());

        // A stream entry is closed: an unknown stream or field is refused.
        let mut unknown = body.clone();
        unknown["streams"][0]["stream"] = serde_json::json!("bonus");
        assert!(serde_json::from_value::<PayoutRailLinkStatus>(unknown).is_err());
        let mut extra = body;
        extra["streams"][1]["account_id"] = serde_json::json!("x");
        assert!(serde_json::from_value::<PayoutRailLinkStatus>(extra).is_err());
    }

    #[test]
    fn outcome_labels_match_their_wire_form() {
        for outcome in [
            PayoutLinkOutcome::PendingConfirmation,
            PayoutLinkOutcome::Confirmed,
            PayoutLinkOutcome::VersionConflict,
            PayoutLinkOutcome::PayoutLinkChangePending,
            PayoutLinkOutcome::NearAiOrgNotPermitted,
            PayoutLinkOutcome::SubjectMismatch,
        ] {
            assert_eq!(
                serde_json::to_value(outcome).unwrap(),
                serde_json::json!(outcome.as_str())
            );
        }
    }
}
