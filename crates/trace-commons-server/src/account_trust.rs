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

/// A stable server-side source. Recording either fact never raises allowance;
/// a later reviewed policy may evaluate the deduplicated history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustFactSource {
    AcceptedSubmission(Uuid),
    GateEvaluation(Uuid),
}

impl TrustFactSource {
    pub fn source_id(self) -> Uuid {
        match self {
            Self::AcceptedSubmission(id) | Self::GateEvaluation(id) => id,
        }
    }

    pub fn kind(self) -> &'static str {
        match self {
            Self::AcceptedSubmission(_) => "accepted_submission",
            Self::GateEvaluation(_) => "gate_evaluation",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustFactOutcome {
    Accepted,
    EvaluatedPassed,
    EvaluatedFailed,
    EvaluatedNotAccepted,
}

impl TrustFactOutcome {
    pub fn from_storage(value: &str) -> Option<Self> {
        match value {
            "accepted" => Some(Self::Accepted),
            "evaluated_passed" => Some(Self::EvaluatedPassed),
            "evaluated_failed" => Some(Self::EvaluatedFailed),
            "evaluated_not_accepted" => Some(Self::EvaluatedNotAccepted),
            _ => None,
        }
    }
}

impl TrustAccount {
    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    pub fn account_id(&self) -> Uuid {
        self.account_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContributionAuthority {
    Invited,
    Bounded { policy_version: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountTrustSnapshot {
    account: TrustAccount,
    version: i64,
    authority: ContributionAuthority,
}

impl AccountTrustSnapshot {
    pub fn account(&self) -> &TrustAccount {
        &self.account
    }

    pub fn version(&self) -> i64 {
        self.version
    }

    pub fn authority(&self) -> &ContributionAuthority {
        &self.authority
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
    let in_near_namespace = ["near-", "nearai-"].iter().any(|prefix| {
        tenant_id
            .strip_prefix(prefix)
            .is_some_and(trace_commons_protocol::admission::is_hash)
    });
    if !in_near_namespace {
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
