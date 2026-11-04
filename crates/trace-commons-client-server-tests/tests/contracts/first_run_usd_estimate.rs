//! The client's built-in pilot price for the first run's estimate is the
//! range the server derives from the pilot's pricing, so a shell that falls
//! back to it says what the server would have published.

use trace_commons_contributor::first_run_copy::PILOT_USD_MICROS_PER_ACCEPTED_TRACE;
use trace_commons_protocol::settlement_estimate::SettlementEstimateTable;
use trace_commons_server::credit_quality::constants_at;
use trace_commons_server::first_run_usd_estimate::{FirstRunUsdInputs, derive_first_run_usd_range};
use trace_commons_server::versioned_pipeline_compat::CompatibilityCreditRule;

/// The pilot's pricing on 2026-11-03: the settlement table pays $0.25 a
/// credit (1,000,000 atomic units), the rail is on, and the graded rule
/// awards at most a delta of 1 point (1,000,000 microcredits). The table's
/// other fields are synthetic.
#[test]
fn the_client_pilot_price_is_the_servers_derivation_of_the_pilot_pricing() {
    let mut table: SettlementEstimateTable = serde_json::from_value(serde_json::json!({
        "schema_version": 2,
        "features_version": "lef1",
        "version": "s-test",
        "instrument_id": "trace_credit",
        "atomic_units_per_credit": 1_000_000,
        "published_at": "2026-10-16T00:00:00Z",
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
    table.usd_micros_per_credit = Some(250_000);
    let derived = derive_first_run_usd_range(FirstRunUsdInputs {
        settlement_table: Some(&table),
        near_ai_rail_enabled: true,
        credit_rule: CompatibilityCreditRule::GradedPerplexityV1,
        delta_microcredits: 1_000_000,
        constants: constants_at(chrono::Utc::now().timestamp()),
    });
    assert_eq!(derived, Ok(PILOT_USD_MICROS_PER_ACCEPTED_TRACE));
}
