// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Payout rail rates and their epochs
//! (docs/superpowers/specs/2026-10-21-nearai-credit-payout-design.md,
//! section 3 "Rate source" and section 4 "Rate").
//!
//! A rail's rate source yields one immutable rational per version. At boot,
//! for each rail whose mode is not `disabled`, ingest records that rational
//! as a rate epoch, `(rail_id, version)`, with `observed_at` set by the
//! database clock. A version re-installed with a different rate refuses
//! boot (`payout_rate_version_conflict`): a version names one rate forever.
//!
//! The rate itself stays out of logs, errors and the repository; only the
//! rail id and the version label are ever reported.

use chrono::{DateTime, Utc};
use trace_commons_gate_api::{RailId, RailRate};
use trace_commons_protocol::settlement_estimate::SettlementEstimateTable;

use crate::db::{PayoutRateEpoch, PayoutRateEpochRecord, PayoutStore};

/// The NEAR AI rail's id as a log field.
pub const NEAR_AI_PAYOUT_RAIL_LABEL: &str = trace_commons_gate_api::NEAR_AI_RAIL_ID;

/// The NEAR AI rail's mode setting. Every mode defaults to `disabled`.
pub const NEAR_AI_PAYOUT_MODE_ENV: &str = "TRACE_COMMONS_NEAR_AI_PAYOUT_MODE";

/// A rail's mode (spec section 4, "Modes and the kill switch"). Parsing is
/// closed: an unknown value refuses boot rather than meaning `disabled`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PayoutMode {
    /// No accrual, no pass, no rate epoch, no link probe on the rail.
    #[default]
    Disabled,
    /// Accrual runs; the pass counts and never writes to the rail.
    DryRun,
    Live,
}

impl PayoutMode {
    pub fn as_label(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::DryRun => "dry_run",
            Self::Live => "live",
        }
    }

    /// `None` (unset) is `Disabled`; any value other than the three labels
    /// is refused.
    pub fn parse(value: Option<&str>) -> Result<Self, PayoutRateError> {
        match value {
            None => Ok(Self::Disabled),
            Some("disabled") => Ok(Self::Disabled),
            Some("dry_run") => Ok(Self::DryRun),
            Some("live") => Ok(Self::Live),
            Some(_) => Err(PayoutRateError::ModeInvalid),
        }
    }

    /// The NEAR AI rail's mode from [`NEAR_AI_PAYOUT_MODE_ENV`].
    pub fn near_ai_from_env() -> Result<Self, PayoutRateError> {
        match std::env::var(NEAR_AI_PAYOUT_MODE_ENV) {
            Ok(value) => Self::parse(Some(value.as_str())),
            Err(std::env::VarError::NotPresent) => Ok(Self::Disabled),
            Err(std::env::VarError::NotUnicode(_)) => Err(PayoutRateError::ModeInvalid),
        }
    }
}

/// Why a rate could not be built or recorded. Each variant is one safe
/// label; none carries a rate, a version or a database message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PayoutRateError {
    /// `(rail_id, version)` is recorded with a different rate.
    #[error("payout_rate_version_conflict")]
    VersionConflict,
    /// The source's version or pricing label is not a safe label.
    #[error("payout_rate_label_invalid")]
    LabelInvalid,
    /// A rail is enabled but ingest has no payout store.
    #[error("payout_store_unavailable")]
    StoreUnavailable,
    /// The epoch could not be read or written.
    #[error("payout_rate_epoch_unrecorded")]
    EpochUnrecorded,
    /// The mode setting is not `disabled`, `dry_run` or `live`.
    #[error("payout_mode_invalid")]
    ModeInvalid,
}

impl PayoutRateError {
    pub fn label(self) -> &'static str {
        match self {
            Self::VersionConflict => "payout_rate_version_conflict",
            Self::LabelInvalid => "payout_rate_label_invalid",
            Self::StoreUnavailable => "payout_store_unavailable",
            Self::EpochUnrecorded => "payout_rate_epoch_unrecorded",
            Self::ModeInvalid => "payout_mode_invalid",
        }
    }
}

/// A rail's rate source: the rational in force now, or `None` when the
/// source has no positive rate (no rate, no payout; nothing falls back).
pub trait RailRateSource: Send + Sync {
    fn current(&self) -> Option<RailRate>;

    /// A label recorded beside the epoch (for `near_ai`, the settlement
    /// table's `pricing_version`).
    fn pricing_label(&self) -> Option<&str> {
        None
    }
}

/// The NEAR AI rail's rate source (owner decision D2): the installed
/// settlement table's `usd_micros_per_credit`. One credit is
/// `atomic_units_per_credit` award units and buys `usd_micros_per_credit`
/// micro-dollars, which is `* 1000` nano-USD, the rail's atomic unit. So
/// `numerator = usd_micros_per_credit * 1000` and `denominator =
/// atomic_units_per_credit`, left unreduced. The version is the table's
/// `version`, with `pricing_version` kept beside it as a label.
#[derive(Clone, PartialEq, Eq)]
pub struct SettlementTableRate {
    rate: Option<RailRate>,
    pricing_label: Option<String>,
}

impl std::fmt::Debug for SettlementTableRate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SettlementTableRate")
            .field("rate", &self.rate)
            .field("pricing_label", &self.pricing_label)
            .finish()
    }
}

/// Nano-USD per micro-dollar.
const NANO_PER_MICRO: u128 = 1_000;

/// The database's rule for a pricing label (`trace_payout_rate_epochs`).
fn is_safe_pricing_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

impl SettlementTableRate {
    /// A table without a positive `usd_micros_per_credit` (or with zero
    /// `atomic_units_per_credit`, which the parser already refuses) yields
    /// a source with no rate. A table with a rate whose `version` or
    /// `pricing_version` is not a safe label is refused here, at boot,
    /// rather than at the database.
    pub fn new(table: &SettlementEstimateTable) -> Result<Self, PayoutRateError> {
        let usd_micros = table.usd_micros_per_credit.unwrap_or(0);
        if usd_micros == 0 || table.atomic_units_per_credit == 0 {
            return Ok(Self {
                rate: None,
                pricing_label: None,
            });
        }
        let pricing_label = match table.pricing_version.as_deref() {
            None => None,
            Some(label) if is_safe_pricing_label(label) => Some(label.to_owned()),
            Some(_) => return Err(PayoutRateError::LabelInvalid),
        };
        let rate = RailRate::new(
            table.version.clone(),
            u128::from(usd_micros) * NANO_PER_MICRO,
            u128::from(table.atomic_units_per_credit),
        )
        .map_err(|_| PayoutRateError::LabelInvalid)?;
        Ok(Self {
            rate: Some(rate),
            pricing_label,
        })
    }
}

impl RailRateSource for SettlementTableRate {
    fn current(&self) -> Option<RailRate> {
        self.rate.clone()
    }

    fn pricing_label(&self) -> Option<&str> {
        self.pricing_label.as_deref()
    }
}

/// What recording a rail's epoch at boot did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateEpochAtBoot {
    Recorded,
    AlreadyRecorded,
    /// The source has no positive rate: no epoch, and the rail accrues and
    /// sends nothing until one is installed.
    NoRate,
}

impl RateEpochAtBoot {
    pub fn as_label(self) -> &'static str {
        match self {
            Self::Recorded => "payout_rate_epoch_recorded",
            Self::AlreadyRecorded => "payout_rate_epoch_already_recorded",
            Self::NoRate => "payout_rate_unavailable",
        }
    }
}

/// Records `source`'s current rate as an epoch on `rail`. A conflict (the
/// version is recorded with another rate) is
/// [`PayoutRateError::VersionConflict`], and the caller refuses to start.
pub async fn record_rate_epoch(
    store: &dyn PayoutStore,
    rail: &RailId,
    source: &dyn RailRateSource,
) -> Result<RateEpochAtBoot, PayoutRateError> {
    let Some(rate) = source.current() else {
        return Ok(RateEpochAtBoot::NoRate);
    };
    match store
        .record_payout_rate_epoch(rail, &rate, source.pricing_label())
        .await
        .map_err(|_| PayoutRateError::EpochUnrecorded)?
    {
        PayoutRateEpochRecord::Recorded => Ok(RateEpochAtBoot::Recorded),
        PayoutRateEpochRecord::AlreadyRecorded => Ok(RateEpochAtBoot::AlreadyRecorded),
        PayoutRateEpochRecord::Conflict => Err(PayoutRateError::VersionConflict),
    }
}

/// The boot hook for the NEAR AI rail. `Disabled` records nothing and needs
/// nothing (`Ok(None)`). Any other mode needs the payout store, and records
/// the installed settlement table's rate as an epoch; with no table, or a
/// table without a positive rate, it records no epoch and boot goes on.
pub async fn near_ai_rate_epoch_at_boot(
    mode: PayoutMode,
    table: Option<&SettlementEstimateTable>,
    store: Option<&dyn PayoutStore>,
) -> Result<Option<RateEpochAtBoot>, PayoutRateError> {
    if mode == PayoutMode::Disabled {
        return Ok(None);
    }
    let store = store.ok_or(PayoutRateError::StoreUnavailable)?;
    let Some(table) = table else {
        return Ok(Some(RateEpochAtBoot::NoRate));
    };
    let source = SettlementTableRate::new(table)?;
    record_rate_epoch(store, &RailId::near_ai(), &source)
        .await
        .map(Some)
}

/// The version of the NEAR AI rail's rate in force for this process: the
/// installed settlement table's, or `None` with no table or a table without
/// a positive rate. Stored epochs never count here. They price batches by
/// date, but only while a rate source is installed: a rail whose source is
/// gone accrues and sends nothing and is never qualified, however many
/// epochs an earlier boot recorded.
pub fn near_ai_rate_version_in_force(
    table: Option<&SettlementEstimateTable>,
) -> Result<Option<String>, PayoutRateError> {
    let Some(table) = table else {
        return Ok(None);
    };
    Ok(SettlementTableRate::new(table)?
        .current()
        .map(|rate| rate.version().to_owned()))
}

/// The epoch that prices a batch created at `at` on one rail: the latest
/// epoch observed at or before `at`, or, for a batch older than the rail's
/// first epoch, the first. `None` only when the rail has no epoch at all.
/// `epochs` may be in any order. A pure function of stored rows, so it
/// gives the same answer whenever accrual runs.
pub fn epoch_in_force(epochs: &[PayoutRateEpoch], at: DateTime<Utc>) -> Option<&PayoutRateEpoch> {
    let key = |epoch: &&PayoutRateEpoch| (epoch.observed_at, epoch.rate.version().to_owned());
    epochs
        .iter()
        .filter(|epoch| epoch.observed_at <= at)
        .max_by_key(key)
        .or_else(|| epochs.iter().min_by_key(key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use trace_commons_protocol::settlement_estimate::SettlementEstimateTable;

    /// A synthetic settlement table in the protocol fixture's shape, with
    /// synthetic rates only.
    fn table(usd_micros_per_credit: Option<u64>, units: u64) -> SettlementEstimateTable {
        let mut table: SettlementEstimateTable = serde_json::from_value(serde_json::json!({
            "schema_version": 2,
            "features_version": "lef1",
            "version": "s3",
            "instrument_id": "trace_credit",
            "atomic_units_per_credit": 1_000_000,
            "published_at": "2026-10-20T00:00:00Z",
            "bytes_per_token": 4,
            "chunk_target_tokens": 2048,
            "chunk_cap": 16,
            "tiers": [
                {"mean_atomic_units": 1_000_000, "withheld_share_micros": 300_000},
                {"mean_atomic_units": 4_000_000, "withheld_share_micros": 100_000}
            ],
            "cut_offs": [5.0],
            "aggregate_band": {"low_ratio_micros": 600_000, "high_ratio_micros": 1_500_000},
            "fit_rows": 40
        }))
        .expect("synthetic table");
        table.version = "settle-test-1".to_owned();
        table.pricing_version = Some("pricing-test-1".to_owned());
        table.usd_micros_per_credit = usd_micros_per_credit;
        table.atomic_units_per_credit = units;
        table
    }

    #[test]
    fn settlement_table_rate_maps_to_a_rational() {
        let source = SettlementTableRate::new(&table(Some(2_000_000), 1_000_000)).unwrap();
        let rate = source.current().expect("a positive rate");
        assert_eq!(rate.version(), "settle-test-1");
        assert_eq!(rate.numerator(), 2_000_000_000);
        assert_eq!(rate.denominator(), 1_000_000);
        assert_eq!(source.pricing_label(), Some("pricing-test-1"));
        // One credit (10^6 award units) at 2 synthetic dollars is 2 * 10^9
        // nano-USD.
        assert_eq!(rate.price(1_000_000).unwrap(), 2_000_000_000);
        assert!(!format!("{source:?}").contains("2000000"));
    }

    #[test]
    fn a_table_without_a_positive_rate_yields_no_epoch() {
        assert_eq!(
            SettlementTableRate::new(&table(None, 1_000_000))
                .unwrap()
                .current(),
            None
        );
        assert_eq!(
            SettlementTableRate::new(&table(Some(0), 1_000_000))
                .unwrap()
                .current(),
            None
        );
    }

    #[test]
    fn an_unsafe_table_label_is_refused() {
        let mut bad_version = table(Some(2_000_000), 1_000_000);
        bad_version.version = "has space".to_owned();
        assert_eq!(
            SettlementTableRate::new(&bad_version),
            Err(PayoutRateError::LabelInvalid)
        );
        let mut bad_pricing = table(Some(2_000_000), 1_000_000);
        bad_pricing.pricing_version = Some("a/b".to_owned());
        assert_eq!(
            SettlementTableRate::new(&bad_pricing),
            Err(PayoutRateError::LabelInvalid)
        );
    }

    #[test]
    fn every_mode_parses_closed_and_defaults_to_disabled() {
        assert_eq!(PayoutMode::parse(None), Ok(PayoutMode::Disabled));
        assert_eq!(PayoutMode::default(), PayoutMode::Disabled);
        for mode in [PayoutMode::Disabled, PayoutMode::DryRun, PayoutMode::Live] {
            assert_eq!(PayoutMode::parse(Some(mode.as_label())), Ok(mode));
        }
        for bad in ["", "Live", "on", "true", "dry-run"] {
            assert_eq!(
                PayoutMode::parse(Some(bad)),
                Err(PayoutRateError::ModeInvalid)
            );
        }
    }

    #[tokio::test]
    async fn a_disabled_rail_records_nothing_and_needs_no_store() {
        let outcome = near_ai_rate_epoch_at_boot(
            PayoutMode::Disabled,
            Some(&table(Some(2_000_000), 1_000_000)),
            None,
        )
        .await;
        assert_eq!(outcome, Ok(None));
    }

    #[tokio::test]
    async fn an_enabled_rail_without_a_store_refuses_boot() {
        for mode in [PayoutMode::DryRun, PayoutMode::Live] {
            let outcome =
                near_ai_rate_epoch_at_boot(mode, Some(&table(Some(2_000_000), 1_000_000)), None)
                    .await;
            assert_eq!(outcome, Err(PayoutRateError::StoreUnavailable));
        }
    }

    #[test]
    fn the_rate_in_force_is_the_current_sources_version_only() {
        // No table, or a table without a positive rate: no rate in force,
        // whatever epochs an earlier boot stored.
        assert_eq!(near_ai_rate_version_in_force(None), Ok(None));
        assert_eq!(
            near_ai_rate_version_in_force(Some(&table(None, 1_000_000))),
            Ok(None)
        );
        assert_eq!(
            near_ai_rate_version_in_force(Some(&table(Some(0), 1_000_000))),
            Ok(None)
        );
        assert_eq!(
            near_ai_rate_version_in_force(Some(&table(Some(2_000_000), 1_000_000))),
            Ok(Some("settle-test-1".to_owned()))
        );
    }

    fn epoch(version: &str, second: i64) -> PayoutRateEpoch {
        PayoutRateEpoch {
            rate: RailRate::new(version, 1, 1).unwrap(),
            pricing_label: None,
            observed_at: Utc.timestamp_opt(second, 0).unwrap(),
        }
    }

    #[test]
    fn the_epoch_in_force_is_the_latest_at_or_before_the_batch() {
        let epochs = [epoch("v2", 200), epoch("v1", 100), epoch("v3", 300)];
        let at = |second| Utc.timestamp_opt(second, 0).unwrap();
        let version = |second| {
            epoch_in_force(&epochs, at(second))
                .map(|epoch| epoch.rate.version().to_owned())
                .unwrap()
        };
        assert_eq!(version(50), "v1", "older than the first uses the first");
        assert_eq!(version(100), "v1");
        assert_eq!(version(250), "v2");
        assert_eq!(version(300), "v3");
        assert_eq!(version(10_000), "v3");
        assert!(epoch_in_force(&[], at(100)).is_none());
    }
}
