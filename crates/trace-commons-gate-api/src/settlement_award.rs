// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The award policy seam for settlement rails.
//!
//! A settlement rail publishes one recipient's cumulative total of an
//! instrument. How much of the instrument a recipient has earned is decided
//! here, by a [`SettlementAwardPolicy`] the server holds as a trait object.
//! The policy is asked two things:
//!
//! - which period a finalized batch falls in ([`SettlementAwardPolicy::period_key`]);
//! - what the recipient's **cumulative** award for that period is, given
//!   everything settled in it so far ([`SettlementAwardPolicy::period_target`]).
//!
//! The server adds `target - already_awarded` to the recipient's total
//! when the target is higher, and nothing when it is not: a target that
//! falls never claws back. Because the target is recomputed from period
//! totals on every batch, a policy keeps no state and needs no rounding
//! carry.
//!
//! The production policy is supplied by a private backend at this seam.
//! [`crate::reference::ReferenceFlatRateAwardPolicy`] is real but
//! deliberately simple and uncalibrated, like the reference scorers.
//!
//! # Hash-only
//!
//! A [`PeriodSummary`] carries hashes and counts, never a tenant id, an
//! account reference or a trace. An [`AwardError`] is a label from
//! `^[a-z0-9_]{1,64}$`. [`SettlementAwardPolicy::policy_ref`] is stored
//! verbatim beside every accrual, so a policy whose parameters are not
//! public returns a version label there, not its parameters.

use sha2::{Digest, Sha256};

/// A period a policy groups batches into, such as a UTC calendar month.
/// Stored as a column: 1 to 64 of `[a-z0-9_.-]`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PeriodKey(String);

impl PeriodKey {
    pub fn new(value: impl Into<String>) -> Result<Self, AwardError> {
        let value = value.into();
        let ok = !value.is_empty()
            && value.len() <= 64
            && value.bytes().all(|b| {
                b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'.' | b'-')
            });
        if ok {
            Ok(Self(value))
        } else {
            Err(AwardError::new("award_period_key_malformed"))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What has been settled to one recipient in one period, including the
/// batch now being accrued.
///
/// A future extension point is the batch's per-trace line items
/// (`trace_credit_settlement_batches.line_items_json`), for a policy that
/// awards by score rather than by total; they are not carried yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeriodSummary {
    pub period_key: PeriodKey,
    /// A hash of the recipient as the rail names it. Never a tenant id.
    pub recipient_ref_hash: String,
    /// The instrument the award is denominated in.
    pub instrument_id: String,
    /// Atomic units per whole unit of the instrument is `10^decimals`.
    pub instrument_decimals: u8,
    /// Settled Trace Credit, in microcredits, over every batch accrued to
    /// this recipient in this period, INCLUDING the batch being accrued.
    pub period_source_micros: u128,
    /// Finalized batches accrued in this period, INCLUDING this one.
    pub period_batch_count: u64,
    /// Source submissions across those batches, INCLUDING this one's.
    pub period_submission_count: u64,
    /// Instrument units already awarded in this period, EXCLUDING this
    /// batch. The server adds `target - this` when the target is higher.
    pub period_awarded_units: u128,
}

/// A policy's answer: the recipient's cumulative award for the period so
/// far, in the instrument's atomic units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeriodAward {
    pub target_units: u128,
}

/// A policy refusal, by label. The batch is not accrued and is retried
/// later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{label}")]
pub struct AwardError {
    label: &'static str,
}

impl AwardError {
    /// `label` should be `^[a-z0-9_]{1,64}$`; anything else is replaced
    /// with `award_policy_error` so a free-form string cannot reach a
    /// stored row.
    pub const fn new(label: &'static str) -> Self {
        Self { label }
    }

    pub fn label(&self) -> &'static str {
        let ok = !self.label.is_empty()
            && self.label.len() <= 64
            && self
                .label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if ok { self.label } else { "award_policy_error" }
    }
}

/// Decides how much instrument credit a recipient has earned. Pure: no
/// clock, no I/O; the server calls it while holding the recipient's ledger
/// row lock. Held as `Arc<dyn SettlementAwardPolicy>`.
pub trait SettlementAwardPolicy: Send + Sync {
    /// Canonical text naming the policy and its configuration, stored on
    /// every accrual: 1 to 128 of `[a-z0-9_.:/-]`.
    fn policy_ref(&self) -> String;

    /// `sha256:<hex>` of [`Self::policy_ref`], domain separated.
    fn policy_ref_hash(&self) -> String {
        policy_ref_hash(&self.policy_ref())
    }

    /// The period a batch finalized at `finalized_at_unix_seconds` (UTC)
    /// belongs to.
    fn period_key(&self, finalized_at_unix_seconds: i64) -> Result<PeriodKey, AwardError>;

    /// The recipient's cumulative award for `summary`'s period.
    fn period_target(&self, summary: &PeriodSummary) -> Result<PeriodAward, AwardError>;
}

/// The hash [`SettlementAwardPolicy::policy_ref_hash`] defaults to.
pub fn policy_ref_hash(policy_ref: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"trace-commons:settlement-award-policy:v1\n");
    hasher.update(policy_ref.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

/// Whether `policy_ref` fits the stored column.
pub fn is_valid_policy_ref(policy_ref: &str) -> bool {
    !policy_ref.is_empty()
        && policy_ref.len() <= 128
        && policy_ref.bytes().all(|b| {
            b.is_ascii_lowercase()
                || b.is_ascii_digit()
                || matches!(b, b'_' | b'.' | b':' | b'/' | b'-')
        })
}

/// The UTC calendar month containing `unix_seconds`, as `YYYY-MM`.
/// Refuses a time before 1970.
pub fn utc_month_period_key(unix_seconds: i64) -> Result<PeriodKey, AwardError> {
    if unix_seconds < 0 {
        return Err(AwardError::new("award_period_time_out_of_range"));
    }
    // Days since 1970-01-01 to a proleptic Gregorian date (Hinnant's
    // civil_from_days, restricted to non-negative days).
    let z = unix_seconds / 86_400 + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    PeriodKey::new(format!("{year:04}-{month:02}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn month(unix_seconds: i64) -> String {
        utc_month_period_key(unix_seconds)
            .unwrap()
            .as_str()
            .to_string()
    }

    #[test]
    fn the_month_key_turns_over_at_utc_midnight_on_the_first() {
        assert_eq!(month(0), "1970-01");
        // 2024-02-29T23:59:59Z, a leap day, then 2024-03-01T00:00:00Z.
        assert_eq!(month(1_709_251_199), "2024-02");
        assert_eq!(month(1_709_251_200), "2024-03");
        // 2023-02-28T23:59:59Z, no leap day, then 2023-03-01T00:00:00Z.
        assert_eq!(month(1_677_628_799), "2023-02");
        assert_eq!(month(1_677_628_800), "2023-03");
        // 2025-12-31T23:59:59Z, then 2026-01-01T00:00:00Z.
        assert_eq!(month(1_767_225_599), "2025-12");
        assert_eq!(month(1_767_225_600), "2026-01");
        // 2000-02-29T12:00:00Z: a century that is a leap year.
        assert_eq!(month(951_825_600), "2000-02");
        // 2100-03-01T00:00:00Z: a century that is not.
        assert_eq!(month(4_107_542_400), "2100-03");
    }

    #[test]
    fn a_time_before_the_epoch_refuses() {
        assert_eq!(
            utc_month_period_key(-1).unwrap_err().label(),
            "award_period_time_out_of_range"
        );
    }

    #[test]
    fn a_period_key_is_checked_against_the_column_shape() {
        assert!(PeriodKey::new("2026-10").is_ok());
        assert!(PeriodKey::new("week.2026_41").is_ok());
        for bad in ["", "Has Space", "2026/10", &"a".repeat(65)] {
            assert_eq!(
                PeriodKey::new(bad).unwrap_err().label(),
                "award_period_key_malformed",
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_free_form_error_label_is_replaced() {
        assert_eq!(
            AwardError::new("award_cap_reached").label(),
            "award_cap_reached"
        );
        assert_eq!(AwardError::new("Has Space").label(), "award_policy_error");
        assert_eq!(AwardError::new("").label(), "award_policy_error");
    }

    #[test]
    fn the_policy_ref_hash_is_domain_separated_and_shaped() {
        let h = policy_ref_hash("flat_rate:7/3");
        assert!(h.starts_with("sha256:") && h.len() == 71);
        assert_ne!(h, policy_ref_hash("flat_rate:7/4"));
        assert_ne!(
            h,
            format!("sha256:{:x}", Sha256::digest(b"flat_rate:7/3")),
            "domain separated"
        );
        assert!(is_valid_policy_ref("flat_rate:7/3"));
        assert!(!is_valid_policy_ref("Flat Rate"));
        assert!(!is_valid_policy_ref(&"a".repeat(129)));
    }
}
