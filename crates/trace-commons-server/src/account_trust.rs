// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Account identity and explicitly configured contribution policy.

use crate::db::Database;
use serde::Deserialize;
use uuid::Uuid;

/// A durable account resolved from a live, authenticated NEAR principal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustAccount {
    tenant_id: String,
    account_id: Uuid,
}

/// A stable server-side source. The definer function verifies each against
/// the server row it names before recording it; a caller-supplied claim alone
/// never creates a fact. Recording a fact alone never raises an allowance;
/// externally configured growth consumes a separately recorded evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustFactSource {
    /// A submission ID with an `accepted` credit-ledger event.
    AcceptedSubmission(Uuid),
    /// A server-accepted submission without requiring a monetary credit event.
    /// Only gate-required policies may qualify this source.
    AcceptedSubmissionStatus(Uuid),
    /// A gate decision ID.
    GateEvaluation(Uuid),
    /// A submission ID whose `withdrawn_at` is set: a contributor withdrawal.
    SubmissionWithdrawn(Uuid),
    /// A submission ID revoked by an operator or policy, not withdrawn.
    SubmissionRevoked(Uuid),
    /// A submission ID the privacy pipeline quarantined.
    SubmissionQuarantined(Uuid),
    /// An `abuse_penalty` credit-ledger event ID.
    AbusePenalty(Uuid),
    /// A submission ID whose v2 witness certificate attests a provider-TEE
    /// final call over the current accepted artifact (V76, V126).
    AttestedSubmission(Uuid),
}

impl TrustFactSource {
    pub fn source_id(self) -> Uuid {
        match self {
            Self::AcceptedSubmission(id)
            | Self::AcceptedSubmissionStatus(id)
            | Self::GateEvaluation(id)
            | Self::SubmissionWithdrawn(id)
            | Self::SubmissionRevoked(id)
            | Self::SubmissionQuarantined(id)
            | Self::AbusePenalty(id)
            | Self::AttestedSubmission(id) => id,
        }
    }

    pub fn kind(self) -> &'static str {
        match self {
            Self::AcceptedSubmission(_) => "accepted_submission",
            Self::AcceptedSubmissionStatus(_) => "accepted_submission_status",
            Self::GateEvaluation(_) => "gate_evaluation",
            Self::SubmissionWithdrawn(_) => "submission_withdrawn",
            Self::SubmissionRevoked(_) => "submission_revoked",
            Self::SubmissionQuarantined(_) => "submission_quarantined",
            Self::AbusePenalty(_) => "abuse_penalty",
            Self::AttestedSubmission(_) => "attested_submission",
        }
    }

    /// The inverse of [`Self::kind`]; `None` for any label outside the closed set.
    pub fn from_storage(kind: &str, source_id: Uuid) -> Option<Self> {
        Some(match kind {
            "accepted_submission" => Self::AcceptedSubmission(source_id),
            "accepted_submission_status" => Self::AcceptedSubmissionStatus(source_id),
            "gate_evaluation" => Self::GateEvaluation(source_id),
            "submission_withdrawn" => Self::SubmissionWithdrawn(source_id),
            "submission_revoked" => Self::SubmissionRevoked(source_id),
            "submission_quarantined" => Self::SubmissionQuarantined(source_id),
            "abuse_penalty" => Self::AbusePenalty(source_id),
            "attested_submission" => Self::AttestedSubmission(source_id),
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustFactOutcome {
    Accepted,
    /// Accepted through a review approved by a principal of the same account.
    /// Recorded so the exclusion is auditable; it never counts toward trust.
    AcceptedSelfReviewed,
    /// Accepted, but the approving principal could not be established.
    /// Fails closed: it never counts toward trust.
    AcceptedApproverUnknown,
    EvaluatedPassed,
    EvaluatedFailed,
    EvaluatedNotAccepted,
    Withdrawn,
    Revoked,
    Quarantined,
    Penalized,
    /// A provider-TEE attested final call over the accepted artifact.
    AttestedProviderTee,
}

impl TrustFactOutcome {
    pub fn from_storage(value: &str) -> Option<Self> {
        match value {
            "accepted" => Some(Self::Accepted),
            "accepted_self_reviewed" => Some(Self::AcceptedSelfReviewed),
            "accepted_approver_unknown" => Some(Self::AcceptedApproverUnknown),
            "evaluated_passed" => Some(Self::EvaluatedPassed),
            "evaluated_failed" => Some(Self::EvaluatedFailed),
            "evaluated_not_accepted" => Some(Self::EvaluatedNotAccepted),
            "withdrawn" => Some(Self::Withdrawn),
            "revoked" => Some(Self::Revoked),
            "quarantined" => Some(Self::Quarantined),
            "penalized" => Some(Self::Penalized),
            "attested_provider_tee" => Some(Self::AttestedProviderTee),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::AcceptedSelfReviewed => "accepted_self_reviewed",
            Self::AcceptedApproverUnknown => "accepted_approver_unknown",
            Self::EvaluatedPassed => "evaluated_passed",
            Self::EvaluatedFailed => "evaluated_failed",
            Self::EvaluatedNotAccepted => "evaluated_not_accepted",
            Self::Withdrawn => "withdrawn",
            Self::Revoked => "revoked",
            Self::Quarantined => "quarantined",
            Self::Penalized => "penalized",
            Self::AttestedProviderTee => "attested_provider_tee",
        }
    }
}

impl TrustAccount {
    /// An account named by the earned-trust worker's enumeration seam, which
    /// lists only open accounts in anchored tenants. It carries no
    /// authentication: use it for batch fact recording and evaluation, never
    /// to authorize a request.
    pub(crate) fn from_worker_enumeration(tenant_id: String, account_id: Uuid) -> Self {
        Self {
            tenant_id,
            account_id,
        }
    }

    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    pub fn account_id(&self) -> Uuid {
        self.account_id
    }
}

/// Safe public refusal labels: no tenant, principal, or database detail escapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TrustRefusal {
    #[error("account_trust_refused")]
    Refused,
    #[error("account_identity_unlinked")]
    Unlinked,
    #[error("account_trust_unavailable")]
    Unavailable,
}

/// Pass only tenant and principal values obtained from the authenticated
/// request context. Never pass envelope attribution or a client account ID.
/// The database join rechecks live device, principal, and account membership.
pub async fn resolve_contribution_account(
    db: &dyn Database,
    tenant_id: &str,
    principal_ref: &str,
) -> Result<TrustAccount, TrustRefusal> {
    if !trace_commons_protocol::admission::is_anchored_tenant(tenant_id) {
        return Err(TrustRefusal::Unlinked);
    }
    if principal_ref.is_empty() {
        return Err(TrustRefusal::Refused);
    }
    let account_id = db
        .get_near_provisioned_account(tenant_id, principal_ref)
        .await
        .map_err(|_| TrustRefusal::Unavailable)?
        .ok_or(TrustRefusal::Refused)?;
    Ok(TrustAccount {
        tenant_id: tenant_id.to_string(),
        account_id,
    })
}

/// The principal a device key is provisioned under:
/// `principal_sha256:` + hex(sha256(`device:<tenant>:<device_key_id>`)).
///
/// Every NEAR and NEAR AI provisioning path keys `trace_near_provisioned_devices`,
/// `trace_account_principals` and the onboarding tenant-access grant by this
/// value, and the upload-claim issuer looks grants up by it. It is NOT the
/// principal ingest derives for a signed upload claim (that one hashes the
/// claim's actor under the `signed_claim` method), so account resolution has
/// to recover it from the claim: see [`device_claim_key_id`].
pub fn device_principal_ref(tenant_id: &str, device_key_id: &str) -> String {
    use sha2::{Digest as _, Sha256};
    let digest = Sha256::digest(format!(
        "device:{}:{}",
        tenant_id.trim(),
        device_key_id.trim()
    ));
    format!("principal_sha256:{}", hex::encode(digest))
}

/// A device key id as the onboarding registry stores it: `sha256:` and 64
/// lowercase hex digits.
pub fn is_device_key_id(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    })
}

/// Longest per-user subject the issuer accepts on a device claim.
pub const DEVICE_CLAIM_SUBJECT_MAX_LEN: usize = 128;

/// Whether `subject` is a per-user subject the issuer would accept: non-empty,
/// at most [`DEVICE_CLAIM_SUBJECT_MAX_LEN`] bytes, ASCII alphanumerics and
/// `:`, `_`, `-` only.
pub fn is_device_claim_subject(subject: &str) -> bool {
    !subject.is_empty()
        && subject.len() <= DEVICE_CLAIM_SUBJECT_MAX_LEN
        && subject
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'_' | b'-'))
}

/// The upload-claim actor (`sub` and `principal_ref`) the issuer mints for a
/// device-key-authenticated request: the bare device key id, or, when the
/// device asserts a per-user subject, `instance:<tenant>:<device_key_id>:user:<subject>`.
///
/// The issuer builds the actor with this function and ingest parses it back
/// with [`device_claim_key_id`]; keep the two together.
pub fn device_claim_actor(tenant_id: &str, device_key_id: &str, subject: Option<&str>) -> String {
    match subject {
        Some(subject) => format!("instance:{tenant_id}:{device_key_id}:user:{subject}"),
        None => device_key_id.to_string(),
    }
}

/// The current and legacy provenance identities of a verified device claim.
/// This does not authenticate an actor: the caller must first verify the
/// registered device signature. Parsing binds a subject-scoped actor to the
/// exact device and tenant, avoiding caller-supplied arbitrary principal refs.
pub fn device_upload_principal_refs(
    tenant_id: &str,
    device_key_id: &str,
    actor_ref: &str,
) -> Option<[String; 2]> {
    use sha2::{Digest as _, Sha256};
    if device_claim_key_id(tenant_id, actor_ref)? != device_key_id {
        return None;
    }
    let material = format!("signed:{tenant_id}:{actor_ref}");
    let bound = format!("12:signed_claim:{}:{material}", material.len());
    Some([bound, material].map(|value| {
        format!(
            "principal_sha256:{}",
            hex::encode(Sha256::digest(value.as_bytes()))
        )
    }))
}

/// The device key id inside a device-shaped upload-claim actor for `tenant_id`,
/// the inverse of [`device_claim_actor`]. `None` for anything else, including
/// an `instance:` actor naming a different tenant.
///
/// Parsed by length, not by splitting on `:`: the key id and the subject both
/// contain colons.
pub fn device_claim_key_id<'a>(tenant_id: &str, actor: &'a str) -> Option<&'a str> {
    if is_device_key_id(actor) {
        return Some(actor);
    }
    const KEY_LEN: usize = "sha256:".len() + 64;
    let rest = actor
        .strip_prefix("instance:")?
        .strip_prefix(tenant_id)?
        .strip_prefix(':')?;
    let key = rest.get(..KEY_LEN)?;
    let subject = rest.get(KEY_LEN..)?.strip_prefix(":user:")?;
    (!tenant_id.is_empty() && is_device_key_id(key) && is_device_claim_subject(subject))
        .then_some(key)
}

/// Whether an actor has the shape of a device claim actor for any tenant. The
/// issuer refuses such an actor on every path that did not verify the device
/// key, so that the shape alone is evidence a device signature was checked.
pub fn is_device_shaped_actor(actor: &str) -> bool {
    is_device_key_id(actor) || actor.starts_with("instance:")
}

/// A policy definition is parsed only when an operator explicitly supplies it.
/// This module does not activate a policy or choose production quota values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedPolicy {
    version: String,
    processing_cost_bound: i64,
    bounded_allowance: i64,
    period: PolicyPeriod,
    external_growth: Option<ExternalGrowth>,
}

impl BoundedPolicy {
    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn processing_cost_bound(&self) -> i64 {
        self.processing_cost_bound
    }

    pub fn bounded_allowance(&self) -> i64 {
        self.bounded_allowance
    }

    pub fn external_growth(&self) -> Option<&ExternalGrowth> {
        self.external_growth.as_ref()
    }

    pub fn period(&self) -> &PolicyPeriod {
        &self.period
    }
}

/// Explicit bounds for consuming recorded `applied` evaluations. Admission
/// reads them the same way whoever wrote them (V114).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalGrowth {
    pub policy_version: String,
    pub allowance_ceiling: i64,
    pub evaluation_max_age_seconds: i64,
    pub source: GrowthSource,
}

/// Who computes the `applied` evaluations admission reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrowthSource {
    /// `"growth_rule":"external"`: a separate evaluator login holding
    /// `trace_account_trust_evaluator` (V114).
    External,
    /// `"growth_rule":"tiered-v1"`: this server's own evaluator runs the
    /// policy's `growth` object (`account_trust_rule`) and writes through
    /// V126. The growth policy version is the admission policy's version.
    TieredV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyPeriod {
    Lifetime,
    Fixed { seconds: i64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("account_trust_policy_invalid")]
pub struct PolicyError;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPolicy {
    version: String,
    processing_cost_bound: i64,
    bounded_allowance: i64,
    period: RawPeriod,
    growth_rule: String,
    growth_policy_version: Option<String>,
    allowance_ceiling: Option<i64>,
    evaluation_max_age_seconds: Option<i64>,
    /// `tiered-v1` only; validated by `account_trust_rule::parse_growth_policy`.
    growth: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
enum RawPeriod {
    Lifetime {},
    Fixed { seconds: i64 },
}

/// An allowlist of reviewed versions is mandatory. The fixture versions in
/// tests are not recognized by production unless explicitly supplied here.
pub fn parse_bounded_policy(
    json: &str,
    supported_versions: &[&str],
) -> Result<BoundedPolicy, PolicyError> {
    let raw: RawPolicy = serde_json::from_str(json).map_err(|_| PolicyError)?;
    if raw.version.is_empty()
        || raw.version.len() > 64
        || !raw
            .version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        || !supported_versions.contains(&raw.version.as_str())
        || raw.processing_cost_bound <= 0
        || raw.bounded_allowance <= 0
        || raw.processing_cost_bound > raw.bounded_allowance
        || raw
            .bounded_allowance
            .checked_add(raw.processing_cost_bound)
            .is_none()
    {
        return Err(PolicyError);
    }
    let external_growth = match raw.growth_rule.as_str() {
        "none"
            if raw.growth_policy_version.is_none()
                && raw.allowance_ceiling.is_none()
                && raw.evaluation_max_age_seconds.is_none()
                && raw.growth.is_none() =>
        {
            None
        }
        crate::account_trust_rule::GROWTH_RULE_TIERED_V1
            if raw.growth_policy_version.is_none()
                && raw.allowance_ceiling.is_none()
                && raw.evaluation_max_age_seconds.is_none() =>
        {
            let growth = crate::account_trust_rule::parse_growth_policy(json, supported_versions)?;
            if growth
                .evaluation_max_age_seconds()
                .checked_mul(1000)
                .is_none()
            {
                return Err(PolicyError);
            }
            Some(ExternalGrowth {
                policy_version: raw.version.clone(),
                allowance_ceiling: growth.allowance_ceiling(),
                evaluation_max_age_seconds: growth.evaluation_max_age_seconds(),
                source: GrowthSource::TieredV1,
            })
        }
        "external" if raw.growth.is_none() => {
            let policy_version = raw.growth_policy_version.ok_or(PolicyError)?;
            let allowance_ceiling = raw.allowance_ceiling.ok_or(PolicyError)?;
            let evaluation_max_age_seconds = raw.evaluation_max_age_seconds.ok_or(PolicyError)?;
            if policy_version.is_empty()
                || policy_version.len() > 64
                || !policy_version
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
                || allowance_ceiling < raw.bounded_allowance
                || allowance_ceiling
                    .checked_add(raw.processing_cost_bound)
                    .is_none()
                || evaluation_max_age_seconds <= 0
                || evaluation_max_age_seconds.checked_mul(1000).is_none()
            {
                return Err(PolicyError);
            }
            Some(ExternalGrowth {
                policy_version,
                allowance_ceiling,
                evaluation_max_age_seconds,
                source: GrowthSource::External,
            })
        }
        _ => return Err(PolicyError),
    };
    let period = match raw.period {
        RawPeriod::Lifetime {} => PolicyPeriod::Lifetime,
        RawPeriod::Fixed { seconds } if seconds > 0 && seconds.checked_mul(1000).is_some() => {
            PolicyPeriod::Fixed { seconds }
        }
        RawPeriod::Fixed { .. } => return Err(PolicyError),
    };
    Ok(BoundedPolicy {
        version: raw.version,
        processing_cost_bound: raw.processing_cost_bound,
        bounded_allowance: raw.bounded_allowance,
        period,
        external_growth,
    })
}

#[cfg(test)]
mod device_principal_tests {
    use super::*;

    const KEY_A: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const KEY_B: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const SUBJECT: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn device_principal_ref_is_the_pinned_provisioning_value() {
        // Same pin as `db::postgres::onboarding_device_principal_ref`'s test:
        // the rows on the pilot are keyed by exactly this derivation.
        assert_eq!(
            device_principal_ref("tenant-1", "sha256:device-key"),
            "principal_sha256:2bf5c8fb2e00d4b044f1e3d24aaf864d21197baaaca9de3da5de631a951caf9d"
        );
    }

    #[test]
    fn device_claim_actor_round_trips_both_forms() {
        let tenant = "nearai-tenant";
        let bare = device_claim_actor(tenant, KEY_A, None);
        assert_eq!(bare, KEY_A);
        assert_eq!(device_claim_key_id(tenant, &bare), Some(KEY_A));
        let instance = device_claim_actor(tenant, KEY_A, Some(SUBJECT));
        assert_eq!(
            instance,
            format!("instance:{tenant}:{KEY_A}:user:{SUBJECT}")
        );
        assert_eq!(device_claim_key_id(tenant, &instance), Some(KEY_A));
        let other = device_claim_actor(tenant, KEY_B, Some(SUBJECT));
        assert_eq!(device_claim_key_id(tenant, &other), Some(KEY_B));
    }

    #[test]
    fn device_claim_key_id_refuses_other_shapes() {
        let tenant = "nearai-tenant";
        // A different tenant's instance actor names no device here.
        let foreign = device_claim_actor("nearai-other", KEY_A, Some(SUBJECT));
        assert_eq!(device_claim_key_id(tenant, &foreign), None);
        // A tenant that is only a prefix of the embedded one.
        let longer = device_claim_actor("nearai-tenant2", KEY_A, Some(SUBJECT));
        assert_eq!(device_claim_key_id(tenant, &longer), None);
        let rejected = [
            "principal:agent-1".to_string(),
            String::new(),
            "sha256:AAAA".to_string(),
            KEY_A.to_uppercase(),
            format!("instance:{tenant}:{KEY_A}"),
            format!("instance:{tenant}:{KEY_A}:user:"),
            format!("instance:{tenant}:{KEY_A}:user:bad/slash"),
            format!("instance:{tenant}:{KEY_A}:usr:{SUBJECT}"),
            format!("instance:{tenant}:sha256:short:user:{SUBJECT}"),
            format!("instance:{tenant}:{KEY_A}:user:{}", "a".repeat(129)),
        ];
        for actor in &rejected {
            assert_eq!(device_claim_key_id(tenant, actor), None, "{actor}");
        }
        assert_eq!(
            device_claim_key_id("", &format!("instance::{KEY_A}:user:{SUBJECT}")),
            None
        );
    }

    #[test]
    fn device_shaped_actor_covers_both_forms() {
        assert!(is_device_shaped_actor(KEY_A));
        assert!(is_device_shaped_actor("instance:anything"));
        assert!(!is_device_shaped_actor("principal:agent-1"));
    }
}
