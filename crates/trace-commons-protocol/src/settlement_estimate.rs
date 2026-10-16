//! The settlement table (schema v2) and the machine-wide dollar estimate
//! drawn from it, made on the contributor's device before anything is sent.
//!
//! A v1 [`LocalEstimateTable`] carries a band of displayed credit per tier. A
//! [`SettlementEstimateTable`] places a trace in a tier with the same `lef1`
//! scoring (the same weights, cut-offs and [`tier_index`]) and carries, per
//! tier, the mean settled award in `trace_credit` atomic units, plus one
//! ratio band fitted on batches of traces and an operator-set dollar rate.
//!
//! Rules this module holds:
//!
//! - **One parser, client and server.** [`SettlementEstimateTable::from_slice`]
//!   and [`SettlementEstimateTable::from_value`] apply every refusal rule; a
//!   refused table is no table.
//! - **Nothing rather than a guess.** No rate, a stale table, a table dated
//!   too far ahead, or no trace whose tier is known: nothing is drawn, never
//!   $0 and never a placeholder.
//! - **No overflow.** Sums and dollar conversions run in `u128` and saturate.
//! - **No content.** Inputs are [`LocalEstimateFeatures`], which are counts
//!   and ratios only.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::local_credit_estimate::{
    ESTIMATE_DISPLAY_STEP, ESTIMATE_MAX_CHUNK_PARAMETER, ESTIMATE_MAX_TIERS, ESTIMATE_MAX_WEIGHTS,
    EstimateBand, EstimateTier, EstimateWeight, LOCAL_ESTIMATE_FEATURES_VERSION,
    LocalEstimateFeatures, LocalEstimateTable, bounded, estimate_score, tier_index, valid_label,
};

/// The settlement table's schema version.
pub const SETTLEMENT_TABLE_SCHEMA_VERSION: u32 = 2;

/// Where the server publishes the settlement table. Unauthenticated, outside
/// tenant context.
pub const SETTLEMENT_TABLE_PATH: &str = "/v1/credit-estimate/settlement-table";

/// The only instrument a settlement table may price.
pub const SETTLEMENT_INSTRUMENT_ID: &str = "trace_credit";

/// A table document larger than this is refused before it is parsed.
pub const SETTLEMENT_TABLE_MAX_BYTES: usize = 16 * 1024;

/// A table older than this, by `published_at`, draws nothing. Seven days,
/// as for the v1 table.
pub const SETTLEMENT_TABLE_MAX_AGE_SECS: i64 = 7 * 86_400;

/// A table dated further ahead than this draws nothing: a far-future date
/// would otherwise never go stale.
pub const SETTLEMENT_TABLE_MAX_FUTURE_SECS: i64 = 86_400;

/// Micro-dollars in a dollar.
pub const USD_MICROS_PER_DOLLAR: u64 = 1_000_000;

/// While both ends of a band are under this, they round to
/// [`FINE_STEP_USD_MICROS`]; otherwise to whole dollars.
pub const FINE_ROUNDING_BELOW_USD_MICROS: u64 = 10_000_000;

/// The rounding step under [`FINE_ROUNDING_BELOW_USD_MICROS`].
pub const FINE_STEP_USD_MICROS: u64 = 500_000;

/// Parts per million, the unit of every `_micros` ratio and share.
const MICROS: u64 = 1_000_000;

/// One tier of a settlement table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettlementTier {
    /// The mean settled award per trace in the tier, in atomic units,
    /// counting a trace withheld at zero as 0. Always positive.
    pub mean_atomic_units: u64,
    /// The share of the tier's traces withheld at zero, in parts per
    /// million. Explanatory only: the mean already includes it.
    pub withheld_share_micros: u32,
}

/// The ratio band that scales a summed mean into the displayed band, in
/// parts per million.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AggregateBand {
    pub low_ratio_micros: u64,
    pub high_ratio_micros: u64,
}

/// A published settlement table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettlementEstimateTable {
    /// Must equal [`SETTLEMENT_TABLE_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// Must equal [`LOCAL_ESTIMATE_FEATURES_VERSION`].
    pub features_version: String,
    /// This table's own version label.
    pub version: String,
    /// An opaque label for the award configuration the table was fitted
    /// against. Absent on a fitted candidate; the operator sets it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing_version: Option<String>,
    /// Must equal [`SETTLEMENT_INSTRUMENT_ID`].
    pub instrument_id: String,
    /// Atomic units in one credit. Never 0.
    pub atomic_units_per_credit: u64,
    /// Micro-dollars of inference one credit buys. Absent on a fitted
    /// candidate; a table without a positive rate parses but draws nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usd_micros_per_credit: Option<u64>,
    pub published_at: DateTime<Utc>,
    pub bytes_per_token: u32,
    pub chunk_target_tokens: u32,
    pub chunk_cap: u32,
    #[serde(default)]
    pub weights: Vec<EstimateWeight>,
    /// Strictly increasing score cut-offs, one fewer than `tiers`.
    #[serde(default)]
    pub cut_offs: Vec<f64>,
    /// One entry per tier, lowest tier first.
    pub tiers: Vec<SettlementTier>,
    pub aggregate_band: AggregateBand,
    /// Rows the table was fitted on.
    pub fit_rows: u64,
}

/// Why a settlement table was refused. Messages are labels only.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SettlementTableError {
    #[error("settlement table is not a valid table document")]
    Malformed,
    #[error("settlement table is too large")]
    TooLarge,
    #[error("settlement table schema version is not supported")]
    UnsupportedSchema,
    #[error("settlement table is for other feature definitions")]
    FeaturesVersion,
    #[error("settlement table label is invalid")]
    Label,
    #[error("settlement table prices another instrument")]
    Instrument,
    #[error("settlement table chunk parameters are invalid")]
    ChunkParameters,
    #[error("settlement table has too many or too few tiers")]
    Tiers,
    #[error("settlement table tier mean is not positive")]
    TierMean,
    #[error("settlement table cut-offs are invalid")]
    CutOffs,
    #[error("settlement table has too many or repeated weights")]
    Weights,
    #[error("settlement table aggregate band is invalid")]
    AggregateBand,
    #[error("settlement table atomic units per credit is zero")]
    AtomicUnits,
    #[error("settlement table carries a number out of range")]
    OutOfRange,
}

impl SettlementEstimateTable {
    /// Parse and validate a table document from bytes, refusing an oversized
    /// one before parsing it.
    pub fn from_slice(bytes: &[u8]) -> Result<Self, SettlementTableError> {
        if bytes.len() > SETTLEMENT_TABLE_MAX_BYTES {
            return Err(SettlementTableError::TooLarge);
        }
        let value: Value =
            serde_json::from_slice(bytes).map_err(|_| SettlementTableError::Malformed)?;
        Self::from_value(&value)
    }

    /// Parse and validate a table document. Anything this build cannot read
    /// completely is refused.
    pub fn from_value(value: &Value) -> Result<Self, SettlementTableError> {
        // Read the schema first: another schema may not have this shape at
        // all, and "unsupported" is the truer refusal than "malformed".
        let schema = value
            .get("schema_version")
            .and_then(Value::as_u64)
            .ok_or(SettlementTableError::Malformed)?;
        if schema != u64::from(SETTLEMENT_TABLE_SCHEMA_VERSION) {
            return Err(SettlementTableError::UnsupportedSchema);
        }
        // Bound the arrays before deserializing them.
        if value
            .get("weights")
            .and_then(Value::as_array)
            .is_some_and(|weights| weights.len() > ESTIMATE_MAX_WEIGHTS)
        {
            return Err(SettlementTableError::Weights);
        }
        if value
            .get("tiers")
            .and_then(Value::as_array)
            .is_some_and(|tiers| tiers.len() > ESTIMATE_MAX_TIERS)
        {
            return Err(SettlementTableError::Tiers);
        }
        let table: Self =
            serde_json::from_value(value.clone()).map_err(|_| SettlementTableError::Malformed)?;
        table.validate()?;
        Ok(table)
    }

    /// The checks [`Self::from_value`] applies, for a table built in code.
    pub fn validate(&self) -> Result<(), SettlementTableError> {
        if self.schema_version != SETTLEMENT_TABLE_SCHEMA_VERSION {
            return Err(SettlementTableError::UnsupportedSchema);
        }
        if self.features_version != LOCAL_ESTIMATE_FEATURES_VERSION {
            return Err(SettlementTableError::FeaturesVersion);
        }
        if !valid_label(&self.version)
            || self
                .pricing_version
                .as_deref()
                .is_some_and(|label| !valid_label(label))
        {
            return Err(SettlementTableError::Label);
        }
        if self.instrument_id != SETTLEMENT_INSTRUMENT_ID {
            return Err(SettlementTableError::Instrument);
        }
        if self.atomic_units_per_credit == 0 {
            return Err(SettlementTableError::AtomicUnits);
        }
        let chunk_parameter_ok = |value: u32| (1..=ESTIMATE_MAX_CHUNK_PARAMETER).contains(&value);
        if !chunk_parameter_ok(self.bytes_per_token)
            || !chunk_parameter_ok(self.chunk_target_tokens)
            || !chunk_parameter_ok(self.chunk_cap)
        {
            return Err(SettlementTableError::ChunkParameters);
        }
        let tiers = self.tiers.len();
        if !(2..=ESTIMATE_MAX_TIERS).contains(&tiers) {
            return Err(SettlementTableError::Tiers);
        }
        for tier in &self.tiers {
            if tier.mean_atomic_units == 0 {
                return Err(SettlementTableError::TierMean);
            }
            if u64::from(tier.withheld_share_micros) > MICROS {
                return Err(SettlementTableError::OutOfRange);
            }
        }
        if self.weights.len() > ESTIMATE_MAX_WEIGHTS {
            return Err(SettlementTableError::Weights);
        }
        let mut terms = BTreeSet::new();
        for weight in &self.weights {
            if !terms.insert(weight.term) {
                return Err(SettlementTableError::Weights);
            }
            if !bounded(weight.weight) {
                return Err(SettlementTableError::OutOfRange);
            }
        }
        if self.cut_offs.len() != tiers - 1 {
            return Err(SettlementTableError::CutOffs);
        }
        if !self.cut_offs.iter().all(|&cut| bounded(cut)) {
            return Err(SettlementTableError::OutOfRange);
        }
        if !self.cut_offs.windows(2).all(|pair| pair[0] < pair[1]) {
            return Err(SettlementTableError::CutOffs);
        }
        let band = &self.aggregate_band;
        if band.low_ratio_micros == 0 || band.low_ratio_micros > band.high_ratio_micros {
            return Err(SettlementTableError::AggregateBand);
        }
        Ok(())
    }

    /// Whether the table may be drawn at `now`: it carries a positive dollar
    /// rate, it is younger than [`SETTLEMENT_TABLE_MAX_AGE_SECS`], and it is
    /// dated no further ahead than [`SETTLEMENT_TABLE_MAX_FUTURE_SECS`].
    #[must_use]
    pub fn drawable_at(&self, now: DateTime<Utc>) -> bool {
        if self.usd_micros_per_credit.unwrap_or(0) == 0 {
            return false;
        }
        let age = (now - self.published_at).num_seconds();
        (-SETTLEMENT_TABLE_MAX_FUTURE_SECS..SETTLEMENT_TABLE_MAX_AGE_SECS).contains(&age)
    }

    /// An unvalidated v1 view carrying this table's weights, cut-offs and
    /// chunk parameters, so [`estimate_score`] and
    /// [`LocalEstimateFeatures::term_value`] score a trace exactly as they do
    /// under v1. Its bands are placeholders; never draw from it.
    #[must_use]
    pub fn scoring_view(&self) -> LocalEstimateTable {
        LocalEstimateTable {
            schema_version: crate::local_credit_estimate::ESTIMATE_TABLE_SCHEMA_VERSION,
            features_version: self.features_version.clone(),
            version: self.version.clone(),
            credit_quality_calibration: self.version.clone(),
            bytes_per_token: self.bytes_per_token,
            chunk_target_tokens: self.chunk_target_tokens,
            chunk_cap: self.chunk_cap,
            weights: self.weights.clone(),
            cut_offs: self.cut_offs.clone(),
            bands: self
                .tiers
                .iter()
                .map(|_| EstimateBand {
                    low: ESTIMATE_DISPLAY_STEP,
                    high: ESTIMATE_DISPLAY_STEP,
                })
                .collect(),
            withheld_share: None,
        }
    }

    /// The tier a trace falls in, or `None` when its tier cannot be known:
    /// features of another version, no content, or a weighted feature that
    /// is unknown.
    #[must_use]
    pub fn tier_index(&self, features: &LocalEstimateFeatures) -> Option<usize> {
        if features.version != self.features_version || features.content_bytes == 0 {
            return None;
        }
        let score = estimate_score(features, &self.scoring_view())?;
        let index = tier_index(score, &self.cut_offs);
        (index < self.tiers.len()).then_some(index)
    }

    /// The tier mean for a trace, in atomic units, or `None` when its tier
    /// cannot be known.
    #[must_use]
    pub fn expected_atomic_units(&self, features: &LocalEstimateFeatures) -> Option<u64> {
        let index = self.tier_index(features)?;
        Some(self.tiers.get(index)?.mean_atomic_units)
    }

    fn tier_label(&self, index: usize) -> Option<EstimateTier> {
        let labels: &[EstimateTier] = match self.tiers.len() {
            2 => &[EstimateTier::Lower, EstimateTier::Higher],
            3 => &[
                EstimateTier::Lower,
                EstimateTier::Middle,
                EstimateTier::Higher,
            ],
            _ => &[],
        };
        labels.get(index).copied()
    }

    /// The rounded dollar band for a sum of tier means, or `None` without a
    /// rate.
    fn usd_band(&self, atomic_units: u128) -> Option<(u64, u64)> {
        let rate = self.usd_micros_per_credit.filter(|&rate| rate > 0)?;
        let per_credit = self.atomic_units_per_credit;
        let band = &self.aggregate_band;
        let low = scale_to_usd_micros(atomic_units, rate, per_credit, band.low_ratio_micros, false);
        let high =
            scale_to_usd_micros(atomic_units, rate, per_credit, band.high_ratio_micros, true);
        Some(round_usd_band(low, high))
    }
}

/// The machine-wide estimate: a rounded dollar band over every trace whose
/// tier is known, and how many traces were and were not known.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SettlementTotal {
    /// Rounded down. 0 means the line reads "up to" the high end.
    pub low_usd_micros: u64,
    /// Rounded up.
    pub high_usd_micros: u64,
    /// Traces placed in a tier.
    pub known: u64,
    /// Traces whose tier could not be known; they contribute nothing.
    pub unknown: u64,
}

impl SettlementTotal {
    /// Whether the rounded low end is $0, so the band reads "up to" its high
    /// end rather than "$0 to".
    #[must_use]
    pub fn up_to(&self) -> bool {
        self.low_usd_micros == 0
    }

    /// The midpoint of the rounded band.
    #[must_use]
    pub fn mid_usd_micros(&self) -> u64 {
        let mid = (u128::from(self.low_usd_micros) + u128::from(self.high_usd_micros)) / 2;
        u64::try_from(mid).unwrap_or(u64::MAX)
    }
}

/// One trace's band: its tier mean scaled by the batch ratio band, rounded as
/// the total is. A ratio band fitted on batches understates a single trace's
/// spread, so this is always labelled an estimate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SettlementEntryEstimate {
    pub low_usd_micros: u64,
    pub high_usd_micros: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier: Option<EstimateTier>,
}

/// The machine-wide total over `features`, one item per trace (`None` for a
/// trace with no readable features). `None` when the table is not drawable
/// at `now` or no trace's tier is known: nothing is drawn, never $0.
#[must_use]
pub fn settlement_total<'a>(
    features: impl IntoIterator<Item = Option<&'a LocalEstimateFeatures>>,
    table: &SettlementEstimateTable,
    now: DateTime<Utc>,
) -> Option<SettlementTotal> {
    if !table.drawable_at(now) {
        return None;
    }
    let mut sum: u128 = 0;
    let mut known: u64 = 0;
    let mut unknown: u64 = 0;
    for item in features {
        match item.and_then(|features| table.expected_atomic_units(features)) {
            Some(mean) => {
                sum = sum.saturating_add(u128::from(mean));
                known = known.saturating_add(1);
            }
            None => unknown = unknown.saturating_add(1),
        }
    }
    if known == 0 {
        return None;
    }
    let (low_usd_micros, high_usd_micros) = table.usd_band(sum)?;
    Some(SettlementTotal {
        low_usd_micros,
        high_usd_micros,
        known,
        unknown,
    })
}

/// One trace's band, or `None` when the table is not drawable at `now` or
/// the trace's tier cannot be known.
#[must_use]
pub fn entry_estimate(
    features: &LocalEstimateFeatures,
    table: &SettlementEstimateTable,
    now: DateTime<Utc>,
) -> Option<SettlementEntryEstimate> {
    if !table.drawable_at(now) {
        return None;
    }
    let index = table.tier_index(features)?;
    let mean = table.tiers.get(index)?.mean_atomic_units;
    let (low_usd_micros, high_usd_micros) = table.usd_band(u128::from(mean))?;
    Some(SettlementEntryEstimate {
        low_usd_micros,
        high_usd_micros,
        tier: table.tier_label(index),
    })
}

/// Round a band for display. While the unrounded high end is under
/// [`FINE_ROUNDING_BELOW_USD_MICROS`] (so both ends are), the low end is
/// floored and the high end ceiled to [`FINE_STEP_USD_MICROS`]; otherwise to
/// whole dollars. Saturates to `u64::MAX`.
#[must_use]
pub fn round_usd_band(low_usd_micros: u128, high_usd_micros: u128) -> (u64, u64) {
    let step = if high_usd_micros < u128::from(FINE_ROUNDING_BELOW_USD_MICROS) {
        u128::from(FINE_STEP_USD_MICROS)
    } else {
        u128::from(USD_MICROS_PER_DOLLAR)
    };
    let low = low_usd_micros / step * step;
    let high = high_usd_micros.div_ceil(step).saturating_mul(step);
    let saturate = |value: u128| u64::try_from(value).unwrap_or(u64::MAX);
    (saturate(low), saturate(high))
}

/// `atomic_units * rate / per_credit * ratio_micros / 1e6`, floored or
/// ceiled. Divides once when the product fits in `u128`; otherwise divides
/// in two exact stages, which cannot overflow before the result itself
/// does, and saturates.
fn scale_to_usd_micros(
    atomic_units: u128,
    rate_usd_micros: u64,
    atomic_units_per_credit: u64,
    ratio_micros: u64,
    ceil: bool,
) -> u128 {
    let numerator = atomic_units
        .checked_mul(u128::from(rate_usd_micros))
        .and_then(|value| value.checked_mul(u128::from(ratio_micros)));
    // `atomic_units_per_credit * MICROS` fits: both are at most 2^64.
    let denominator = u128::from(atomic_units_per_credit) * u128::from(MICROS);
    if let Some(numerator) = numerator {
        return divide(numerator, denominator, ceil);
    }
    let usd_micros = mul_div(atomic_units, rate_usd_micros, atomic_units_per_credit, ceil);
    mul_div(usd_micros, ratio_micros, MICROS, ceil)
}

fn divide(numerator: u128, denominator: u128, ceil: bool) -> u128 {
    if ceil {
        numerator.div_ceil(denominator)
    } else {
        numerator / denominator
    }
}

/// `value * factor / divisor` without an intermediate overflow: the
/// remainder term is below `divisor * factor`, which fits in `u128`
/// because both are at most `u64::MAX`. Saturates when the result does not
/// fit. `divisor` is never 0 (validated).
fn mul_div(value: u128, factor: u64, divisor: u64, ceil: bool) -> u128 {
    let (factor, divisor) = (u128::from(factor), u128::from(divisor));
    let whole = (value / divisor).saturating_mul(factor);
    let remainder = (value % divisor) * factor;
    whole.saturating_add(divide(remainder, divisor, ceil))
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Duration, Utc};
    use serde_json::{Value, json};

    use super::*;
    use crate::local_credit_estimate::{
        EstimateTier, LOCAL_ESTIMATE_FEATURES_VERSION, LocalEstimateFeatures, LocalEstimateTable,
        estimate,
    };

    fn t0() -> DateTime<Utc> {
        "2026-10-20T00:00:00Z".parse().expect("t0")
    }

    fn table_a_value() -> Value {
        json!({
            "schema_version": 2,
            "features_version": "lef1",
            "version": "s3",
            "pricing_version": "p2",
            "instrument_id": "trace_credit",
            "atomic_units_per_credit": 1_000_000,
            "usd_micros_per_credit": 1_000_000,
            "published_at": "2026-10-20T00:00:00Z",
            "bytes_per_token": 4,
            "chunk_target_tokens": 2048,
            "chunk_cap": 16,
            "weights": [{"term": "ln_content_bytes", "weight": 1.0}],
            "cut_offs": [5.0],
            "tiers": [
                {"mean_atomic_units": 1_000_000, "withheld_share_micros": 300_000},
                {"mean_atomic_units": 4_000_000, "withheld_share_micros": 100_000}
            ],
            "aggregate_band": {"low_ratio_micros": 600_000, "high_ratio_micros": 1_500_000},
            "fit_rows": 40
        })
    }

    fn table_a() -> SettlementEstimateTable {
        SettlementEstimateTable::from_value(&table_a_value()).expect("table_a parses")
    }

    fn with(mutate: impl FnOnce(&mut Value)) -> Value {
        let mut value = table_a_value();
        mutate(&mut value);
        value
    }

    fn f(content_bytes: u64) -> LocalEstimateFeatures {
        LocalEstimateFeatures {
            version: LOCAL_ESTIMATE_FEATURES_VERSION.to_string(),
            content_bytes,
            tool_result_bytes: 0,
            agent_prose_bytes: 0,
            user_messages: 1,
            distinct_tools: 0,
            byte_entropy_milli: None,
            deflate_ratio_milli: None,
        }
    }

    #[test]
    fn each_refusal_rule_refuses() {
        let refused = |value: Value| SettlementEstimateTable::from_value(&value).unwrap_err();
        assert_eq!(
            refused(with(|v| v["schema_version"] = json!(1))),
            SettlementTableError::UnsupportedSchema
        );
        assert_eq!(
            refused(with(|v| v["features_version"] = json!("lef2"))),
            SettlementTableError::FeaturesVersion
        );
        assert_eq!(
            refused(with(|v| {
                v["tiers"] = json!([{"mean_atomic_units": 1, "withheld_share_micros": 0}]);
            })),
            SettlementTableError::Tiers
        );
        assert_eq!(
            refused(with(|v| {
                let tier = json!({"mean_atomic_units": 1, "withheld_share_micros": 0});
                v["tiers"] = json!([tier, tier, tier, tier]);
                v["cut_offs"] = json!([1.0, 2.0, 3.0]);
            })),
            SettlementTableError::Tiers
        );
        assert_eq!(
            refused(with(|v| v["tiers"][1]["mean_atomic_units"] = json!(0))),
            SettlementTableError::TierMean
        );
        assert_eq!(
            refused(with(|v| {
                let tier = json!({"mean_atomic_units": 1, "withheld_share_micros": 0});
                v["tiers"] = json!([tier, tier, tier]);
                v["cut_offs"] = json!([5.0, 5.0]);
            })),
            SettlementTableError::CutOffs
        );
        assert_eq!(
            refused(with(|v| {
                v["aggregate_band"] =
                    json!({"low_ratio_micros": 1_500_000, "high_ratio_micros": 600_000});
            })),
            SettlementTableError::AggregateBand
        );
        assert_eq!(
            refused(with(|v| v["aggregate_band"]["low_ratio_micros"] = json!(0))),
            SettlementTableError::AggregateBand
        );
        assert_eq!(
            refused(with(|v| v["instrument_id"] = json!("other"))),
            SettlementTableError::Instrument
        );
        assert_eq!(
            refused(with(|v| v["atomic_units_per_credit"] = json!(0))),
            SettlementTableError::AtomicUnits
        );

        let mut padded = serde_json::to_vec(&table_a_value()).expect("serialize");
        padded.extend(std::iter::repeat_n(b' ', SETTLEMENT_TABLE_MAX_BYTES));
        assert_eq!(
            SettlementEstimateTable::from_slice(&padded).unwrap_err(),
            SettlementTableError::TooLarge
        );
        // The unpadded document is well under the limit and parses.
        let plain = serde_json::to_vec(&table_a_value()).expect("serialize");
        assert_eq!(
            SettlementEstimateTable::from_slice(&plain).expect("parses"),
            table_a()
        );
    }

    #[test]
    fn a_table_without_a_rate_parses_but_is_not_drawable() {
        let no_rate = SettlementEstimateTable::from_value(&with(|v| {
            v.as_object_mut()
                .expect("object")
                .remove("usd_micros_per_credit");
        }))
        .expect("a table without a rate parses");
        assert_eq!(no_rate.usd_micros_per_credit, None);
        assert!(!no_rate.drawable_at(t0()));

        let zero_rate =
            SettlementEstimateTable::from_value(&with(|v| v["usd_micros_per_credit"] = json!(0)))
                .expect("a zero rate parses");
        assert!(!zero_rate.drawable_at(t0()));
        assert_eq!(settlement_total([Some(&f(100))], &zero_rate, t0()), None);

        assert!(table_a().drawable_at(t0()));
    }

    #[test]
    fn a_stale_table_is_not_drawable() {
        let table = table_a();
        assert!(table.drawable_at(t0() + Duration::days(7) - Duration::seconds(1)));
        assert!(!table.drawable_at(t0() + Duration::days(7)));
    }

    #[test]
    fn a_far_future_table_is_not_drawable() {
        let table = table_a();
        assert!(!table.drawable_at(t0() - Duration::days(1) - Duration::seconds(1)));
        assert!(table.drawable_at(t0() - Duration::days(1)));
        assert_eq!(
            settlement_total(
                [Some(&f(100))],
                &table,
                t0() - Duration::days(1) - Duration::seconds(1)
            ),
            None
        );
    }

    #[test]
    fn tier_assignment_matches_v1_for_the_same_weights_and_cut_offs() {
        let v1 = LocalEstimateTable::from_value(&json!({
            "schema_version": 1,
            "features_version": "lef1",
            "version": "t9",
            "credit_quality_calibration": "cq3",
            "bytes_per_token": 4,
            "chunk_target_tokens": 2048,
            "chunk_cap": 16,
            "weights": [{"term": "ln_content_bytes", "weight": 1.0}],
            "cut_offs": [5.0],
            "bands": [{"low": 1.0, "high": 2.0}, {"low": 2.0, "high": 3.0}]
        }))
        .expect("v1 table parses");
        let table = table_a();
        let v1_index = |tier: Option<EstimateTier>| match tier {
            Some(EstimateTier::Lower) => 0,
            Some(EstimateTier::Higher) => 1,
            other => panic!("unexpected v1 tier {other:?}"),
        };
        for (content_bytes, expected) in [(100, 0), (1000, 1), (147, 0), (148, 1)] {
            let features = f(content_bytes);
            let v1_tier = estimate(&features, &v1).expect("v1 estimate").tier;
            assert_eq!(v1_index(v1_tier), expected, "v1 at {content_bytes}");
            assert_eq!(
                table.tier_index(&features),
                Some(expected),
                "v2 at {content_bytes}"
            );
        }
        assert_eq!(table.expected_atomic_units(&f(100)), Some(1_000_000));
        assert_eq!(table.expected_atomic_units(&f(1000)), Some(4_000_000));
    }

    #[test]
    fn the_sum_band_and_dollars_are_hand_computed() {
        let table = table_a();
        let (low, high) = (f(100), f(1000));

        // 1 + 1 + 4 credits = $6: low $3.60 floors to $3.50, high $9.00 stays.
        let total = settlement_total([Some(&low), Some(&low), Some(&high)], &table, t0())
            .expect("drawable");
        assert_eq!(
            (total.low_usd_micros, total.high_usd_micros),
            (3_500_000, 9_000_000)
        );
        assert_eq!((total.known, total.unknown), (3, 0));
        assert!(!total.up_to());
        assert_eq!(total.mid_usd_micros(), 6_250_000);

        // 1 + 1 + 1 + 4 credits = $7: high $10.50 is not under $10, so both
        // ends go to whole dollars: low $4.20 -> $4, high $10.50 -> $11.
        let total = settlement_total(
            [Some(&low), Some(&low), Some(&low), Some(&high)],
            &table,
            t0(),
        )
        .expect("drawable");
        assert_eq!(
            (total.low_usd_micros, total.high_usd_micros),
            (4_000_000, 11_000_000)
        );
    }

    #[test]
    fn the_rounding_steps_are_pinned() {
        // Under $10: $0.50 steps, low down, high up.
        assert_eq!(round_usd_band(3_600_000, 9_000_000), (3_500_000, 9_000_000));
        assert_eq!(round_usd_band(3_500_000, 9_000_001), (3_500_000, 9_500_000));
        // A high end of exactly $10 is not under $10.
        assert_eq!(
            round_usd_band(4_200_000, 10_000_000),
            (4_000_000, 10_000_000)
        );
        assert_eq!(round_usd_band(0, 1), (0, 500_000));
        assert_eq!(round_usd_band(u128::MAX, u128::MAX), (u64::MAX, u64::MAX));
    }

    #[test]
    fn a_zero_low_end_reads_up_to() {
        let table = SettlementEstimateTable::from_value(&with(|v| {
            v["usd_micros_per_credit"] = json!(100_000)
        }))
        .expect("parses");
        // One credit at $0.10: low $0.06 -> $0, high $0.15 -> $0.50.
        let total = settlement_total([Some(&f(100))], &table, t0()).expect("drawable");
        assert_eq!((total.low_usd_micros, total.high_usd_micros), (0, 500_000));
        assert!(total.up_to());
    }

    #[test]
    fn unknown_traces_are_counted_and_contribute_nothing() {
        let table = table_a();
        let other_version = LocalEstimateFeatures {
            version: "lef2".to_string(),
            ..f(1000)
        };
        let known = f(100);
        let empty = f(0);
        let mixed = settlement_total(
            [Some(&known), None, Some(&empty), Some(&other_version)],
            &table,
            t0(),
        )
        .expect("one known trace is enough");
        assert_eq!((mixed.known, mixed.unknown), (1, 3));
        let alone = settlement_total([Some(&known)], &table, t0()).expect("drawable");
        assert_eq!(
            (mixed.low_usd_micros, mixed.high_usd_micros),
            (alone.low_usd_micros, alone.high_usd_micros)
        );
    }

    #[test]
    fn nothing_is_drawn_without_a_known_trace() {
        let table = table_a();
        assert_eq!(settlement_total([None, None], &table, t0()), None);
        assert_eq!(settlement_total([Some(&f(0))], &table, t0()), None);
        assert_eq!(
            settlement_total(
                std::iter::empty::<Option<&LocalEstimateFeatures>>(),
                &table,
                t0()
            ),
            None
        );
    }

    #[test]
    fn an_entry_band_uses_the_batch_ratios() {
        let table = table_a();
        // Four credits at $1: $2.40 -> $2.00, $6.00 stays.
        let entry = entry_estimate(&f(1000), &table, t0()).expect("entry estimate");
        assert_eq!(
            entry,
            SettlementEntryEstimate {
                low_usd_micros: 2_000_000,
                high_usd_micros: 6_000_000,
                tier: Some(EstimateTier::Higher),
            }
        );
        assert_eq!(
            entry_estimate(&f(100), &table, t0()).and_then(|e| e.tier),
            Some(EstimateTier::Lower)
        );
        assert_eq!(entry_estimate(&f(0), &table, t0()), None);
        assert_eq!(
            entry_estimate(&f(1000), &table, t0() + Duration::days(7)),
            None
        );
    }

    #[test]
    fn settlement_total_does_not_overflow_at_u64_means() {
        let table = SettlementEstimateTable::from_value(&with(|v| {
            v["tiers"][0]["mean_atomic_units"] = json!(u64::MAX);
            v["tiers"][1]["mean_atomic_units"] = json!(u64::MAX);
        }))
        .expect("parses");
        let features = f(100);
        let total = settlement_total(std::iter::repeat_n(Some(&features), 10_000), &table, t0())
            .expect("drawable");
        assert_eq!(total.known, 10_000);
        assert_eq!(total.high_usd_micros, u64::MAX);
        assert_eq!(total.low_usd_micros, u64::MAX);
        assert_eq!(total.mid_usd_micros(), u64::MAX);
    }

    #[test]
    fn an_absurd_table_saturates_instead_of_wrapping() {
        // Large enough that the single-division numerator overflows u128.
        let table = SettlementEstimateTable::from_value(&with(|v| {
            v["tiers"][0]["mean_atomic_units"] = json!(u64::MAX);
            v["usd_micros_per_credit"] = json!(u64::MAX);
            v["atomic_units_per_credit"] = json!(u64::MAX);
            v["aggregate_band"] =
                json!({"low_ratio_micros": 1_000_000, "high_ratio_micros": u64::MAX});
        }))
        .expect("parses");
        let total = settlement_total([Some(&f(100))], &table, t0()).expect("drawable");
        // Exactly one credit at u64::MAX micro-dollars per credit, times 1.0,
        // floored to whole dollars: the staged path is exact, not saturated.
        assert_eq!(total.low_usd_micros, u64::MAX / 1_000_000 * 1_000_000);
        assert_eq!(total.high_usd_micros, u64::MAX);
    }

    #[test]
    fn the_table_round_trips_and_omits_absent_options() {
        let table = table_a();
        let value = serde_json::to_value(&table).expect("serialize");
        assert_eq!(
            SettlementEstimateTable::from_value(&value).expect("round trip"),
            table
        );
        let candidate = SettlementEstimateTable {
            usd_micros_per_credit: None,
            pricing_version: None,
            ..table
        };
        let value = serde_json::to_value(&candidate).expect("serialize");
        assert!(value.get("usd_micros_per_credit").is_none());
        assert!(value.get("pricing_version").is_none());
        assert_eq!(
            SettlementEstimateTable::from_value(&with(|v| {
                v["unexpected"] = json!(1);
            }))
            .unwrap_err(),
            SettlementTableError::Malformed
        );
    }
}
