// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Participant projections of the reward ledger. No evidence or payout authority.
//! INTEGRATION: used by the Database facade, PostgreSQL adapter and ingest routes.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize, Serializer};
use uuid::Uuid;

use crate::mission_rewards::{RewardActivityKind, RewardError};

pub const MAX_REWARD_MANIFEST_BYTES: u64 = 32 * 1024;

/// Issuer-authored text. Clients must render these fields as text, never HTML.
/// PostgreSQL verifies their hashes against the immutable program terms.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RewardOfferManifest {
    pub schema_version: u32,
    pub definition: String,
    pub rubric: String,
    pub required_evidence: String,
    pub rights: String,
    pub challenge_policy: String,
    pub evaluator_policy: String,
    pub reservation_terms: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RewardReservationRequest {
    pub reservation_id: Uuid,
    pub offer_version_hash: String,
}

impl RewardReservationRequest {
    pub fn validate(&self) -> Result<(), RewardError> {
        let digest = self.offer_version_hash.strip_prefix("sha256:");
        if self.reservation_id.is_nil()
            || !digest.is_some_and(|value| {
                value.len() == 64
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            })
        {
            return Err(RewardError::RequestInvalid);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RewardHistoryQuery {
    #[serde(default = "default_history_limit")]
    pub limit: i32,
    pub before: Option<Uuid>,
}

fn default_history_limit() -> i32 {
    20
}

impl RewardHistoryQuery {
    pub fn validate(&self) -> Result<(), RewardError> {
        if !(1..=100).contains(&self.limit) || self.before.is_some_and(|id| id.is_nil()) {
            return Err(RewardError::RequestInvalid);
        }
        Ok(())
    }
}

/// BIGINT amounts become decimal strings at the HTTP boundary, preserving all
/// digits for native and JavaScript consumers without floating-point money.
fn serialize_units<S: Serializer>(units: &i64, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&units.to_string())
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RewardDenomination {
    CloudCredits,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RewardIdentityMode {
    AccountBound,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RewardOffer {
    pub schema_version: u32,
    pub program_id: Uuid,
    pub offer_version_hash: String,
    pub terms_hash: String,
    pub activity_kind: RewardActivityKind,
    pub denomination: RewardDenomination,
    #[serde(serialize_with = "serialize_units")]
    pub award_units: i64,
    #[serde(serialize_with = "serialize_units")]
    pub capacity_units: i64,
    #[serde(serialize_with = "serialize_units")]
    pub capacity_used_units: i64,
    #[serde(serialize_with = "serialize_units")]
    pub capacity_remaining_units: i64,
    #[serde(serialize_with = "serialize_units")]
    pub participant_cap_units: i64,
    pub closes_at: DateTime<Utc>,
    pub reservation_ttl_seconds: i64,
    pub published_at: DateTime<Utc>,
    pub suspended: bool,
    pub manifest: RewardOfferManifest,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RewardReservationState {
    Reserved,
    Expired,
    Submitted,
    Awarded,
    AwardedInvalidated,
    Rejected,
    Cancelled,
    Invalidated,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RewardDecisionKind {
    Review,
    Cancel,
    Invalidation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RewardParticipantDecision {
    pub decision_id: Uuid,
    pub kind: RewardDecisionKind,
    pub accepted: Option<bool>,
    pub reason: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RewardReservation {
    pub schema_version: u32,
    pub reservation_id: Uuid,
    pub program_id: Uuid,
    pub offer_version_hash: String,
    pub state: RewardReservationState,
    pub invalidated: bool,
    pub denomination: RewardDenomination,
    #[serde(serialize_with = "serialize_units")]
    pub pinned_units: i64,
    #[serde(serialize_with = "serialize_units")]
    pub awarded_units: i64,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub submitted_at: Option<DateTime<Utc>>,
    pub terminal_at: Option<DateTime<Utc>>,
    pub decisions: Vec<RewardParticipantDecision>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RewardParticipantHistory {
    pub schema_version: u32,
    pub identity_mode: RewardIdentityMode,
    pub denomination: RewardDenomination,
    pub limit: i32,
    pub truncated: bool,
    pub next_cursor: Option<Uuid>,
    /// Numeric PostgreSQL aggregate across programs; may exceed BIGINT.
    pub awarded_units: String,
    pub entries: Vec<RewardReservation>,
}

#[cfg(test)]
mod tests {
    use crate::reward_participant::{RewardHistoryQuery, RewardReservationRequest};

    #[test]
    fn reservation_rejects_identity_injection_and_invalid_versions() {
        let id = uuid::Uuid::new_v4();
        let valid = serde_json::json!({"reservation_id":id,"offer_version_hash":format!("sha256:{}", "a".repeat(64))});
        let request: RewardReservationRequest = serde_json::from_value(valid.clone()).unwrap();
        assert!(request.validate().is_ok());
        for key in [
            "account_id",
            "participant_hash",
            "work_hash",
            "consent_hash",
            "accepted",
        ] {
            let mut injected = valid.clone();
            injected[key] = serde_json::json!("injected");
            assert!(serde_json::from_value::<RewardReservationRequest>(injected).is_err());
        }
        for invalid in ["sha256:0", "a", &format!("sha256:{}", "A".repeat(64))] {
            assert!(
                RewardReservationRequest {
                    reservation_id: id,
                    offer_version_hash: invalid.into()
                }
                .validate()
                .is_err()
            );
        }
    }

    #[test]
    fn history_is_bounded() {
        for limit in [-1, 0, 101, i32::MAX] {
            assert!(
                RewardHistoryQuery {
                    limit,
                    before: None
                }
                .validate()
                .is_err()
            );
        }
        for limit in [1, 20, 100] {
            assert!(
                RewardHistoryQuery {
                    limit,
                    before: None
                }
                .validate()
                .is_ok()
            );
        }
    }

    #[test]
    fn public_units_preserve_bigint_precision() {
        #[derive(serde::Serialize)]
        struct Amount(#[serde(serialize_with = "crate::reward_participant::serialize_units")] i64);
        assert_eq!(
            serde_json::to_string(&Amount(i64::MAX)).unwrap(),
            "\"9223372036854775807\""
        );
    }
}
