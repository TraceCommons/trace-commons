// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Account identity and inactive, explicitly configured contribution policy.

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
/// never creates a fact. Recording a fact never raises an allowance: no
/// admission policy reads these rows (the growth rule is shadow-only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustFactSource {
    /// A submission ID with an `accepted` credit-ledger event.
    AcceptedSubmission(Uuid),
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
}

impl TrustFactSource {
    pub fn source_id(self) -> Uuid {
        match self {
            Self::AcceptedSubmission(id)
            | Self::GateEvaluation(id)
            | Self::SubmissionWithdrawn(id)
            | Self::SubmissionRevoked(id)
            | Self::SubmissionQuarantined(id)
            | Self::AbusePenalty(id) => id,
        }
    }

    pub fn kind(self) -> &'static str {
        match self {
            Self::AcceptedSubmission(_) => "accepted_submission",
            Self::GateEvaluation(_) => "gate_evaluation",
            Self::SubmissionWithdrawn(_) => "submission_withdrawn",
            Self::SubmissionRevoked(_) => "submission_revoked",
            Self::SubmissionQuarantined(_) => "submission_quarantined",
            Self::AbusePenalty(_) => "abuse_penalty",
        }
    }

    /// The inverse of [`Self::kind`]; `None` for any label outside the closed set.
    pub fn from_storage(kind: &str, source_id: Uuid) -> Option<Self> {
        Some(match kind {
            "accepted_submission" => Self::AcceptedSubmission(source_id),
            "gate_evaluation" => Self::GateEvaluation(source_id),
            "submission_withdrawn" => Self::SubmissionWithdrawn(source_id),
            "submission_revoked" => Self::SubmissionRevoked(source_id),
            "submission_quarantined" => Self::SubmissionQuarantined(source_id),
            "abuse_penalty" => Self::AbusePenalty(source_id),
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

/// A policy definition is parsed only when an operator explicitly supplies it.
/// This module does not activate a policy or choose production quota values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedPolicy {
    version: String,
    processing_cost_bound: i64,
    bounded_allowance: i64,
    period: PolicyPeriod,
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

    pub fn period(&self) -> &PolicyPeriod {
        &self.period
    }
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
        || raw.growth_rule != "none"
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
    })
}
