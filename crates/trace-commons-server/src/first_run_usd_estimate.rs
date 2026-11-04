// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The first run's dollar range per accepted trace, derived from the
//! pricing the server pays with (`GET /v1/public/first-run-estimate`, see
//! `trace_commons_protocol::first_run_estimate`).
//!
//! An operator can still set the range by hand
//! (`TRACE_COMMONS_FIRST_RUN_USD_ESTIMATE_MICROS_MIN` / `_MAX`), and that
//! wins. Without it, ingest derives the range from three things it already
//! holds at boot:
//!
//! - **Dollars per credit**: the installed settlement table's
//!   `usd_micros_per_credit` over `atomic_units_per_credit`. It is the rate
//!   the NEAR AI payout rail records as its epoch and pays at
//!   (`payout_rate::SettlementTableRate`, owner decision D2 of the payout
//!   design), so it is the price a paid trace is actually paid at. The v1
//!   credit estimate table carries no dollar rate and is not used.
//! - **Credit per accepted trace**: `main`'s `NoveltyUtility` delta and
//!   credit rule, as the running gate holds them (`MainGateConfig`). Under
//!   `flat` every passing trace earns the delta, so both ends are the delta,
//!   but only on `main`'s legacy gate path, whose `NoveltyUtility` events
//!   settle: with a pipeline runtime assembled, a `flat` compatibility leg is
//!   never batched (Ruling S10) and the rail pays only from batches, so
//!   nothing is published (2026-11-03 paying-bundle design, section 3).
//!   Under `graded_perplexity_v1` the award is `delta * f * a`
//!   ([`graded_perplexity_award`]): the top is a trace at or above the
//!   perplexity ceiling (`f = 1`, `a = 1`), which is the delta; the bottom is
//!   a trace at the perplexity floor with an ordinary profile (`f` at its
//!   graded floor, `a = 1`). The anomaly term can take a spiky trace lower,
//!   to no award at all, but a trace with no award is not paid, and the
//!   2026-11-03 paying-bundle design prices its award table on `a = 1` (the
//!   observed capped median ratio, 8.18, is well under the soft limit).
//! - **Whether the rail pays at all**: with the NEAR AI rail `disabled` no
//!   credit is paid at any rate, so nothing is published.
//!
//! Both ends are cut down to whole cents (the protocol publishes only whole
//! cents), never rounded up, so the range never says more than the rule
//! pays. Anything missing, zero, or outside `UsdMicrosRange::is_valid` is
//! `None`, and the route serves `{}` as before: a guessed number is never
//! published.

use trace_commons_protocol::first_run_estimate::{USD_MICROS_PER_CENT, UsdMicrosRange};
use trace_commons_protocol::settlement_estimate::SettlementEstimateTable;

use crate::credit_quality::CreditQualityConstants;
use crate::versioned_pipeline_compat::{CompatibilityCreditRule, graded_perplexity_award};

/// Why no range was derived, as one safe label for the boot log. None
/// carries a rate, a delta or a table version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirstRunUsdUnderived {
    /// No settlement table is installed.
    NoSettlementTable,
    /// The table carries no positive dollar rate.
    NoRate,
    /// The NEAR AI payout rail is disabled.
    RailDisabled,
    /// The `NoveltyUtility` delta is 0: no trace earns credit.
    NoDelta,
    /// The `flat` rule with a pipeline runtime assembled: its leg is never
    /// batched, so the rail never pays it.
    RuleUnpaid,
    /// The derived range is not one the protocol publishes (an end below a
    /// cent, or above the protocol's bound).
    OutOfRange,
}

impl FirstRunUsdUnderived {
    pub fn as_label(self) -> &'static str {
        match self {
            Self::NoSettlementTable => "first_run_usd_no_settlement_table",
            Self::NoRate => "first_run_usd_no_rate",
            Self::RailDisabled => "first_run_usd_rail_disabled",
            Self::NoDelta => "first_run_usd_no_delta",
            Self::RuleUnpaid => "first_run_usd_rule_unpaid",
            Self::OutOfRange => "first_run_usd_out_of_range",
        }
    }
}

/// The inputs, as ingest holds them at boot.
#[derive(Debug, Clone, Copy)]
pub struct FirstRunUsdInputs<'a> {
    pub settlement_table: Option<&'a SettlementEstimateTable>,
    /// Whether the NEAR AI payout rail is anything but `disabled`.
    pub near_ai_rail_enabled: bool,
    /// Whether ingest assembled a versioned pipeline runtime.
    pub pipeline_runtime_assembled: bool,
    pub credit_rule: CompatibilityCreditRule,
    /// `main`'s `NoveltyUtility` delta in trace-credit microcredits (the
    /// settlement table's atomic units).
    pub delta_microcredits: u64,
    /// The credit-quality calibration in force now.
    pub constants: &'a CreditQualityConstants,
}

/// The derived range, or why there is none. See the module docs.
pub fn derive_first_run_usd_range(
    inputs: FirstRunUsdInputs<'_>,
) -> Result<UsdMicrosRange, FirstRunUsdUnderived> {
    let table = inputs
        .settlement_table
        .ok_or(FirstRunUsdUnderived::NoSettlementTable)?;
    let usd_micros_per_credit = table
        .usd_micros_per_credit
        .filter(|&rate| rate > 0)
        .ok_or(FirstRunUsdUnderived::NoRate)?;
    if table.atomic_units_per_credit == 0 {
        return Err(FirstRunUsdUnderived::NoRate);
    }
    if !inputs.near_ai_rail_enabled {
        return Err(FirstRunUsdUnderived::RailDisabled);
    }
    if inputs.delta_microcredits == 0 {
        return Err(FirstRunUsdUnderived::NoDelta);
    }
    let (min_units, max_units) = match inputs.credit_rule {
        CompatibilityCreditRule::Flat if inputs.pipeline_runtime_assembled => {
            return Err(FirstRunUsdUnderived::RuleUnpaid);
        }
        CompatibilityCreditRule::Flat => (inputs.delta_microcredits, inputs.delta_microcredits),
        CompatibilityCreditRule::GradedPerplexityV1 => {
            let k = inputs.constants;
            // An ordinary profile: peak equals the representative, ratio 1.
            let at = |ppl_micros: i64| {
                let ppl = u64::try_from(ppl_micros.max(0)).unwrap_or(0);
                graded_perplexity_award(inputs.delta_microcredits, ppl, ppl, k).microcredits
            };
            (at(k.ppl_floor_micros), at(k.ppl_ceil_micros))
        }
    };
    let to_cents = |units: u64| -> u64 {
        let usd_micros = u128::from(units) * u128::from(usd_micros_per_credit)
            / u128::from(table.atomic_units_per_credit);
        let cents = usd_micros / u128::from(USD_MICROS_PER_CENT);
        u64::try_from(cents * u128::from(USD_MICROS_PER_CENT)).unwrap_or(u64::MAX)
    };
    let range = UsdMicrosRange {
        min: to_cents(min_units),
        max: to_cents(max_units),
    };
    if range.is_valid() {
        Ok(range)
    } else {
        Err(FirstRunUsdUnderived::OutOfRange)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credit_quality::{CREDIT_QUALITY_CONSTANTS_V3, constants_at};

    /// A synthetic settlement table in the protocol fixture's shape.
    fn table(usd_micros_per_credit: Option<u64>, units: u64) -> SettlementEstimateTable {
        let mut table: SettlementEstimateTable = serde_json::from_value(serde_json::json!({
            "schema_version": 2,
            "features_version": "lef1",
            "version": "s-test",
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
        table.usd_micros_per_credit = usd_micros_per_credit;
        table.atomic_units_per_credit = units;
        table
    }

    fn inputs(
        table: Option<&SettlementEstimateTable>,
        rule: CompatibilityCreditRule,
        delta_microcredits: u64,
    ) -> FirstRunUsdInputs<'_> {
        FirstRunUsdInputs {
            settlement_table: table,
            near_ai_rail_enabled: true,
            // The graded rule is only ever read with a pipeline; `flat`
            // pays on the legacy path, without one.
            pipeline_runtime_assembled: rule == CompatibilityCreditRule::GradedPerplexityV1,
            credit_rule: rule,
            delta_microcredits,
            constants: &CREDIT_QUALITY_CONSTANTS_V3,
        }
    }

    /// The pilot's settings on 2026-11-03: $0.25 a credit, the graded rule,
    /// a delta of 1 point (1,000,000 microcredits), the V3 calibration. The
    /// top is the delta, $0.25; the bottom is the graded floor 0.25 of it,
    /// $0.0625, cut to $0.06.
    #[test]
    fn the_pilot_settings_derive_six_to_twenty_five_cents() {
        let t = table(Some(250_000), 1_000_000);
        let range = derive_first_run_usd_range(inputs(
            Some(&t),
            CompatibilityCreditRule::GradedPerplexityV1,
            1_000_000,
        ))
        .expect("derived");
        assert_eq!(
            range,
            UsdMicrosRange {
                min: 60_000,
                max: 250_000
            }
        );
        // The calibration in force now is V3.
        assert_eq!(
            constants_at(chrono::Utc::now().timestamp()).version,
            CREDIT_QUALITY_CONSTANTS_V3.version
        );
    }

    #[test]
    fn a_flat_rule_pays_the_delta_at_both_ends() {
        let t = table(Some(250_000), 1_000_000);
        let range =
            derive_first_run_usd_range(inputs(Some(&t), CompatibilityCreditRule::Flat, 2_500_000))
                .expect("derived");
        assert_eq!(
            range,
            UsdMicrosRange {
                min: 620_000,
                max: 620_000
            },
            "2.5 points at $0.25 is $0.625, cut to $0.62"
        );
    }

    #[test]
    fn the_rate_is_per_credit_in_the_tables_atomic_units() {
        // A table with 1,000 atomic units a credit: a delta of 1,000,000
        // microcredits is 1,000 credits.
        let t = table(Some(10_000), 1_000);
        let range =
            derive_first_run_usd_range(inputs(Some(&t), CompatibilityCreditRule::Flat, 1_000_000))
                .expect("derived");
        assert_eq!(range.max, 10_000_000);
    }

    #[test]
    fn missing_or_zero_inputs_derive_nothing() {
        let rated = table(Some(250_000), 1_000_000);
        let unrated = table(None, 1_000_000);
        let zero_rate = table(Some(0), 1_000_000);
        let graded = CompatibilityCreditRule::GradedPerplexityV1;
        let err = |i| derive_first_run_usd_range(i).unwrap_err();
        assert_eq!(
            err(inputs(None, graded, 1_000_000)),
            FirstRunUsdUnderived::NoSettlementTable
        );
        assert_eq!(
            err(inputs(Some(&unrated), graded, 1_000_000)),
            FirstRunUsdUnderived::NoRate
        );
        assert_eq!(
            err(inputs(Some(&zero_rate), graded, 1_000_000)),
            FirstRunUsdUnderived::NoRate
        );
        assert_eq!(
            err(inputs(Some(&rated), graded, 0)),
            FirstRunUsdUnderived::NoDelta
        );
        let mut disabled = inputs(Some(&rated), graded, 1_000_000);
        disabled.near_ai_rail_enabled = false;
        assert_eq!(err(disabled), FirstRunUsdUnderived::RailDisabled);
        // `flat` under a pipeline runtime: the leg is never batched or paid.
        let mut flat_on_pipeline = inputs(Some(&rated), CompatibilityCreditRule::Flat, 1_000_000);
        flat_on_pipeline.pipeline_runtime_assembled = true;
        assert_eq!(err(flat_on_pipeline), FirstRunUsdUnderived::RuleUnpaid);
    }

    #[test]
    fn a_range_the_protocol_would_refuse_derives_nothing() {
        // A bottom under a cent: 0.25 of 1 credit at $0.02 is $0.005.
        let cheap = table(Some(20_000), 1_000_000);
        assert_eq!(
            derive_first_run_usd_range(inputs(
                Some(&cheap),
                CompatibilityCreditRule::GradedPerplexityV1,
                1_000_000
            ))
            .unwrap_err(),
            FirstRunUsdUnderived::OutOfRange
        );
        // A top above the protocol's $100 bound.
        let dear = table(Some(250_000), 1_000_000);
        assert_eq!(
            derive_first_run_usd_range(inputs(
                Some(&dear),
                CompatibilityCreditRule::Flat,
                1_000_000_000
            ))
            .unwrap_err(),
            FirstRunUsdUnderived::OutOfRange
        );
    }
}
