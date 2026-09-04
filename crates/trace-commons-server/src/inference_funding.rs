// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Read-only decoding of NEAR AI's organization balance contract.
//!
//! This is organization accounting evidence, never a contributor entitlement.
//! No credential, network request, funding mutation, or redemption is performed.
//! The caller must resolve the expected organization from trusted account mapping;
//! the response's organization is not authorization. See the funding contract spec.

use chrono::{DateTime, TimeDelta, Utc};
use serde::Deserialize;
use uuid::Uuid;

/// Bound decoding before serde allocates from an untrusted provider response.
pub const MAX_BALANCE_BYTES: usize = 64 * 1024;

/// NEAR AI documents USD amounts as signed int64 nano-dollars (scale 9).
/// Negative remaining balances are preserved, not clamped into a claim of zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsdNanoDollars(pub i64);

/// Absence of a provider limit is not an unlimited contributor allowance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrganizationRemaining {
    Unspecified,
    Reported {
        spend_limit: UsdNanoDollars,
        remaining: UsdNanoDollars,
    },
}

/// Time since the caller observed the response, not since the last usage update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationFreshness {
    Fresh,
    Stale,
}

/// Safe, identity-free accounting result. It deliberately has no spendable field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrganizationBalanceObservation {
    pub total_spent: UsdNanoDollars,
    pub remaining: OrganizationRemaining,
    pub usage_updated_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
}

impl OrganizationBalanceObservation {
    /// A fresh read can legitimately contain old usage on an idle account.
    /// The caller chooses its refresh policy; no economic or freshness policy
    /// is silently supplied by this decoder.
    pub fn freshness(
        &self,
        now: DateTime<Utc>,
        max_age: TimeDelta,
    ) -> Result<ObservationFreshness, BalanceDecodeError> {
        if max_age < TimeDelta::zero() || now < self.observed_at {
            return Err(BalanceDecodeError::InvalidObservationTime);
        }
        Ok(if now.signed_duration_since(self.observed_at) <= max_age {
            ObservationFreshness::Fresh
        } else {
            ObservationFreshness::Stale
        })
    }
}

/// Fixed labels only: never retain provider text, organization IDs, or serde errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BalanceDecodeError {
    #[error("inference_balance_response_too_large")]
    ResponseTooLarge,
    #[error("inference_balance_response_malformed")]
    Malformed,
    #[error("inference_balance_organization_mismatch")]
    OrganizationMismatch,
    #[error("inference_balance_currency_unsupported")]
    CurrencyUnsupported,
    #[error("inference_balance_snapshot_inconsistent")]
    SnapshotInconsistent,
    #[error("inference_balance_observation_time_invalid")]
    InvalidObservationTime,
}

// Private wire types intentionally have no Debug or Serialize implementation.
// Unknown fields are tolerated for provider evolution. Display strings and
// source/type names are neither trusted for arithmetic nor returned to callers.
#[derive(Deserialize)]
struct ProviderBalance {
    organization_id: Uuid,
    total_spent: i64,
    spend_limit: Option<i64>,
    remaining: Option<i64>,
    updated_at: DateTime<Utc>,
    credit_limits: Vec<ProviderCreditLimit>,
}

#[derive(Deserialize)]
struct ProviderCreditLimit {
    currency: String,
}

/// Decode a successful balance response against an already-authorized org.
///
/// `observed_at` is the trusted local response-receipt time, not provider input.
/// Missing/null limit and remaining together mean unspecified. A half-specified
/// or arithmetically inconsistent snapshot is refused and should be re-fetched;
/// the upstream builds usage and limit reads concurrently, not atomically.
///
/// # Errors
/// Returns a label-only error on malformed, mismatched, or ambiguous evidence.
pub fn decode_organization_balance(
    body: &[u8],
    expected_organization: Uuid,
    observed_at: DateTime<Utc>,
) -> Result<OrganizationBalanceObservation, BalanceDecodeError> {
    if body.len() > MAX_BALANCE_BYTES {
        return Err(BalanceDecodeError::ResponseTooLarge);
    }
    let balance: ProviderBalance =
        serde_json::from_slice(body).map_err(|_| BalanceDecodeError::Malformed)?;
    if balance.organization_id != expected_organization {
        return Err(BalanceDecodeError::OrganizationMismatch);
    }
    if balance
        .credit_limits
        .iter()
        .any(|row| row.currency != "USD")
    {
        return Err(BalanceDecodeError::CurrencyUnsupported);
    }
    let remaining = match (balance.spend_limit, balance.remaining) {
        (None, None) => OrganizationRemaining::Unspecified,
        (Some(limit), Some(remaining))
            if limit.checked_sub(balance.total_spent) == Some(remaining) =>
        {
            OrganizationRemaining::Reported {
                spend_limit: UsdNanoDollars(limit),
                remaining: UsdNanoDollars(remaining),
            }
        }
        _ => return Err(BalanceDecodeError::SnapshotInconsistent),
    };
    Ok(OrganizationBalanceObservation {
        total_spent: UsdNanoDollars(balance.total_spent),
        remaining,
        usage_updated_at: balance.updated_at,
        observed_at,
    })
}
