//! Immutable pricing evidence and checked deterministic cost arithmetic.
//!
//! Tables contain reviewed inputs but do not establish billed cost. The
//! calculator performs no model, timestamp, service-tier, or catalog lookup.

use std::cell::Cell;
use std::cmp::Ordering;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

pub const PRICING_TABLE_SCHEMA_VERSION: u32 = 2;
pub const PRICING_CALCULATOR_VERSION: &str = "deterministic-usd-cost-v1";
pub const COST_FRACTION_DENOMINATOR: u64 = 1_000_000;
const MICROS_ROUNDING_DENOMINATOR: u128 = 1_000_000_000;
const MAX_PRICE_ENTRIES: usize = 1024;
const MAX_RATES_PER_ENTRY: usize = 5;
const MAX_ID_BYTES: usize = 96;
const MAX_PROVENANCE_TEXT_BYTES: usize = 256;
const MAX_SOURCE_NOTES_BYTES: usize = 1024;
const MAX_SOURCE_URL_BYTES: usize = 2048;
const TABLE_DIGEST_DOMAIN: &[u8] = b"trace-commons-insights-pricing-table-v1\0";

thread_local! {
    static TABLE_DIGESTS_COMPUTED: Cell<u64> = const { Cell::new(0) };
}

/// How many table digests this thread has computed (each one serializes and
/// SHA-256s a whole table). A diagnostic for tests that assert a hot path
/// validates nothing per call; not part of any wire or storage contract.
#[doc(hidden)]
pub fn table_digests_computed_on_this_thread() -> u64 {
    TABLE_DIGESTS_COMPUTED.with(Cell::get)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PricingCurrency {
    Usd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PricingAccounting {
    Codex,
    ClaudeCode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BillableTokenCategory {
    UncachedInput,
    CachedInput,
    CacheReadInput,
    CacheWriteFiveMinuteInput,
    CacheWriteOneHourInput,
    /// OpenAI's cache write at its default 30-minute lifetime. A total rate
    /// in place of the uncached-input rate, not a surcharge on top of it.
    CacheWriteThirtyMinuteInput,
    Output,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PricingTableProvenance {
    pub publisher: String,
    pub source_url: String,
    pub retrieved_at: DateTime<Utc>,
    pub content_sha256: String,
    pub reviewed_by: String,
    pub reviewed_at: DateTime<Utc>,
    /// Where the entries' `effective_from` came from when the source page
    /// states no date. Absent fields are not serialized, so a table without
    /// one keeps its digest body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_from_basis: Option<String>,
    /// How the source was fetched and what its hash does and does not
    /// bind: scripts the page loads, and that page bytes change per build.
    /// At most `MAX_SOURCE_NOTES_BYTES`. Not serialized when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenRate {
    pub category: BillableTokenCategory,
    pub usd_nanos_per_million_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelPriceEntry {
    pub provider: String,
    pub model: String,
    pub accounting: PricingAccounting,
    pub effective_from: DateTime<Utc>,
    pub effective_until: Option<DateTime<Utc>>,
    /// A prompt-size band, for a model a vendor prices by request size. The
    /// entry applies only to a request whose prompt (every input category,
    /// cached and cache-write tokens included; never output) is above
    /// `prompt_tokens_above` and at most `prompt_tokens_at_most`. Absent
    /// bounds are unbounded and not serialized.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens_above: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens_at_most: Option<u64>,
    pub rates: Vec<TokenRate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PricingTable {
    pub schema_version: u32,
    pub table_id: String,
    pub version: u32,
    pub currency: PricingCurrency,
    pub published_at: DateTime<Utc>,
    pub provenance: PricingTableProvenance,
    pub entries: Vec<ModelPriceEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BillableTokenCount {
    pub category: BillableTokenCategory,
    pub tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PricingUsageInput {
    pub provider: String,
    pub model: String,
    pub accounting: PricingAccounting,
    /// Inclusive timestamp of the baseline counter observation.
    pub observed_from: DateTime<Utc>,
    /// Inclusive timestamp of the final counter observation.
    pub observed_until: DateTime<Utc>,
    pub counts: Vec<BillableTokenCount>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CostBreakdownItem {
    pub category: BillableTokenCategory,
    pub tokens: u64,
    pub usd_nanos_per_million_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeterministicCostEstimate {
    pub schema_version: u32,
    pub calculator_version: String,
    pub table_id: String,
    pub table_version: u32,
    pub currency: PricingCurrency,
    pub provider: String,
    pub model: String,
    pub accounting: PricingAccounting,
    pub observed_from: DateTime<Utc>,
    pub observed_until: DateTime<Utc>,
    /// Each item's exact numerator is tokens times its integer rate, over
    /// [`COST_FRACTION_DENOMINATOR`]. Categories are not separately rounded.
    pub breakdown: Vec<CostBreakdownItem>,
    /// The sum of exact category numerators, rounded once to USD micros.
    pub estimated_cost_usd_micros: u64,
    pub nonzero_rounded_to_zero: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PricingError {
    #[error("insights_pricing_table_invalid")]
    InvalidTable,
    #[error("insights_pricing_input_invalid")]
    InvalidInput,
    #[error("insights_pricing_missing_price")]
    MissingPrice,
    #[error("insights_pricing_window_unresolved")]
    UnresolvedWindow,
    #[error("insights_pricing_arithmetic_overflow")]
    ArithmeticOverflow,
}

impl PricingAccounting {
    pub const fn required_categories(self) -> &'static [BillableTokenCategory] {
        use BillableTokenCategory::*;
        match self {
            Self::Codex => &[
                UncachedInput,
                CachedInput,
                CacheWriteThirtyMinuteInput,
                Output,
            ],
            Self::ClaudeCode => &[
                UncachedInput,
                CacheReadInput,
                CacheWriteFiveMinuteInput,
                CacheWriteOneHourInput,
                Output,
            ],
        }
    }
}

impl PricingTable {
    pub fn canonical_table_id(&self) -> Result<String, PricingError> {
        TABLE_DIGESTS_COMPUTED.with(|count| count.set(count.get().saturating_add(1)));
        let body = PricingTableDigestBody {
            schema_version: self.schema_version,
            version: self.version,
            currency: self.currency,
            published_at: self.published_at,
            provenance: &self.provenance,
            entries: &self.entries,
        };
        let encoded = serde_json::to_vec(&body).map_err(|_| PricingError::InvalidTable)?;
        let mut hasher = Sha256::new();
        hasher.update(TABLE_DIGEST_DOMAIN);
        hasher.update(encoded);
        Ok(format!("sha256:{}", hex::encode(hasher.finalize())))
    }

    pub fn validate(&self) -> Result<(), PricingError> {
        if self.schema_version != PRICING_TABLE_SCHEMA_VERSION
            || self.version == 0
            || self.entries.is_empty()
            || self.entries.len() > MAX_PRICE_ENTRIES
            || !valid_provenance(&self.provenance)
            || self.canonical_table_id()? != self.table_id
        {
            return Err(PricingError::InvalidTable);
        }
        for entry in &self.entries {
            validate_entry(entry)?;
        }
        if !self
            .entries
            .windows(2)
            .all(|pair| entry_order(&pair[0], &pair[1]) == Ordering::Less)
        {
            return Err(PricingError::InvalidTable);
        }
        // Sorted, so each entry need only be checked against the later
        // entries of its own provider, model and accounting.
        for (index, left) in self.entries.iter().enumerate() {
            for right in self.entries[index + 1..]
                .iter()
                .take_while(|right| same_series(left, right))
            {
                if windows_overlap(left, right) && bands_overlap(left, right) {
                    return Err(PricingError::InvalidTable);
                }
            }
        }
        Ok(())
    }
}

#[derive(Serialize)]
struct PricingTableDigestBody<'a> {
    schema_version: u32,
    version: u32,
    currency: PricingCurrency,
    published_at: DateTime<Utc>,
    provenance: &'a PricingTableProvenance,
    entries: &'a [ModelPriceEntry],
}

/// A [`PricingTable`] that passed [`PricingTable::validate`] and can no
/// longer change: the field is private and there is no mutable access. A
/// holder prices many turns from it with
/// [`calculate_estimated_cost_validated`], which does not repeat the
/// validation (it serializes and SHA-256s the whole table).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedPricingTable(PricingTable);

impl ValidatedPricingTable {
    pub fn as_table(&self) -> &PricingTable {
        &self.0
    }
}

impl std::ops::Deref for ValidatedPricingTable {
    type Target = PricingTable;

    fn deref(&self) -> &PricingTable {
        &self.0
    }
}

impl PricingTable {
    /// Validate once and keep the proof in the type.
    pub fn into_validated(self) -> Result<ValidatedPricingTable, PricingError> {
        self.validate()?;
        Ok(ValidatedPricingTable(self))
    }
}

pub fn calculate_estimated_cost(
    table: &PricingTable,
    input: &PricingUsageInput,
) -> Result<DeterministicCostEstimate, PricingError> {
    table.validate()?;
    calculate_checked(table, input)
}

/// [`calculate_estimated_cost`] for a table already validated: the same
/// result without the per-call validation.
pub fn calculate_estimated_cost_validated(
    table: &ValidatedPricingTable,
    input: &PricingUsageInput,
) -> Result<DeterministicCostEstimate, PricingError> {
    calculate_checked(&table.0, input)
}

fn calculate_checked(
    table: &PricingTable,
    input: &PricingUsageInput,
) -> Result<DeterministicCostEstimate, PricingError> {
    validate_input(input)?;
    let matching = table
        .entries
        .iter()
        .filter(|entry| {
            entry.provider == input.provider
                && entry.model == input.model
                && entry.accounting == input.accounting
        })
        .collect::<Vec<_>>();
    if matching.is_empty() {
        return Err(PricingError::MissingPrice);
    }
    // At most five u64 counts, so the u128 sum cannot overflow.
    let prompt_tokens = input
        .counts
        .iter()
        .filter(|count| count.category != BillableTokenCategory::Output)
        .map(|count| u128::from(count.tokens))
        .sum::<u128>();
    // A prompt between two bands, or a window no entry covers, matches no
    // entry: unresolved, never priced at a neighbouring rate.
    let applicable = matching
        .into_iter()
        .filter(|entry| {
            entry.effective_from <= input.observed_from
                && entry
                    .effective_until
                    .is_none_or(|until| input.observed_until < until)
                && entry
                    .prompt_tokens_above
                    .is_none_or(|above| prompt_tokens > u128::from(above))
                && entry
                    .prompt_tokens_at_most
                    .is_none_or(|at_most| prompt_tokens <= u128::from(at_most))
        })
        .collect::<Vec<_>>();
    let [entry] = applicable.as_slice() else {
        return Err(PricingError::UnresolvedWindow);
    };

    let mut exact_numerator = 0u128;
    let mut breakdown = Vec::with_capacity(input.counts.len());
    // Rates are matched by category, never by position. `validate_entry` and
    // `validate_input` both pin their vectors to `required_categories()`, so a
    // reordering on either side must still meet the same rate.
    for count in &input.counts {
        let rate = entry
            .rates
            .iter()
            .find(|rate| rate.category == count.category)
            .ok_or(PricingError::InvalidTable)?;
        let category_numerator = u128::from(count.tokens)
            .checked_mul(u128::from(rate.usd_nanos_per_million_tokens))
            .ok_or(PricingError::ArithmeticOverflow)?;
        exact_numerator = exact_numerator
            .checked_add(category_numerator)
            .ok_or(PricingError::ArithmeticOverflow)?;
        breakdown.push(CostBreakdownItem {
            category: count.category,
            tokens: count.tokens,
            usd_nanos_per_million_tokens: rate.usd_nanos_per_million_tokens,
        });
    }
    let rounded = exact_numerator
        .checked_add(MICROS_ROUNDING_DENOMINATOR / 2)
        .ok_or(PricingError::ArithmeticOverflow)?
        / MICROS_ROUNDING_DENOMINATOR;
    let estimated_cost_usd_micros =
        u64::try_from(rounded).map_err(|_| PricingError::ArithmeticOverflow)?;
    Ok(DeterministicCostEstimate {
        schema_version: PRICING_TABLE_SCHEMA_VERSION,
        calculator_version: PRICING_CALCULATOR_VERSION.to_owned(),
        table_id: table.table_id.clone(),
        table_version: table.version,
        currency: table.currency,
        provider: input.provider.clone(),
        model: input.model.clone(),
        accounting: input.accounting,
        observed_from: input.observed_from,
        observed_until: input.observed_until,
        breakdown,
        estimated_cost_usd_micros,
        nonzero_rounded_to_zero: exact_numerator != 0 && estimated_cost_usd_micros == 0,
    })
}

fn validate_entry(entry: &ModelPriceEntry) -> Result<(), PricingError> {
    if !safe_id(&entry.provider)
        || !safe_id(&entry.model)
        || entry
            .effective_until
            .is_some_and(|until| until <= entry.effective_from)
        || entry
            .prompt_tokens_above
            .zip(entry.prompt_tokens_at_most)
            .is_some_and(|(above, at_most)| above >= at_most)
        || entry.rates.len() > MAX_RATES_PER_ENTRY
        || entry
            .rates
            .iter()
            .map(|rate| rate.category)
            .collect::<Vec<_>>()
            != entry.accounting.required_categories()
    {
        return Err(PricingError::InvalidTable);
    }
    Ok(())
}

fn validate_input(input: &PricingUsageInput) -> Result<(), PricingError> {
    if !safe_id(&input.provider)
        || !safe_id(&input.model)
        || input.observed_from > input.observed_until
        || input
            .counts
            .iter()
            .map(|count| count.category)
            .collect::<Vec<_>>()
            != input.accounting.required_categories()
    {
        return Err(PricingError::InvalidInput);
    }
    Ok(())
}

fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(&byte))
}

fn valid_text(value: &str) -> bool {
    valid_text_up_to(value, MAX_PROVENANCE_TEXT_BYTES)
}

fn valid_text_up_to(value: &str, max_bytes: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_bytes
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn valid_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_provenance(value: &PricingTableProvenance) -> bool {
    let url = Url::parse(&value.source_url).ok();
    valid_text(&value.publisher)
        && valid_text(&value.reviewed_by)
        && value.effective_from_basis.as_deref().is_none_or(valid_text)
        && value
            .source_notes
            .as_deref()
            .is_none_or(|notes| valid_text_up_to(notes, MAX_SOURCE_NOTES_BYTES))
        && valid_digest(&value.content_sha256)
        && value.reviewed_at >= value.retrieved_at
        && value.source_url.len() <= MAX_SOURCE_URL_BYTES
        && url.is_some_and(|url| {
            url.scheme() == "https"
                && url.has_host()
                && url.username().is_empty()
                && url.password().is_none()
        })
}

fn entry_order(left: &ModelPriceEntry, right: &ModelPriceEntry) -> Ordering {
    (
        &left.provider,
        &left.model,
        left.accounting,
        left.effective_from,
        left.prompt_tokens_above,
        left.prompt_tokens_at_most,
    )
        .cmp(&(
            &right.provider,
            &right.model,
            right.accounting,
            right.effective_from,
            right.prompt_tokens_above,
            right.prompt_tokens_at_most,
        ))
}

/// Half-open `[effective_from, effective_until)` windows share an instant.
fn windows_overlap(left: &ModelPriceEntry, right: &ModelPriceEntry) -> bool {
    left.effective_until
        .is_none_or(|until| right.effective_from < until)
        && right
            .effective_until
            .is_none_or(|until| left.effective_from < until)
}

/// Some prompt size `p` satisfies `above < p <= at_most` for both entries.
fn bands_overlap(left: &ModelPriceEntry, right: &ModelPriceEntry) -> bool {
    let lower = |entry: &ModelPriceEntry| entry.prompt_tokens_above.map_or(-1, i128::from);
    let upper = |entry: &ModelPriceEntry| entry.prompt_tokens_at_most.map_or(i128::MAX, i128::from);
    lower(left).max(lower(right)) < upper(left).min(upper(right))
}

fn same_series(left: &ModelPriceEntry, right: &ModelPriceEntry) -> bool {
    left.provider == right.provider
        && left.model == right.model
        && left.accounting == right.accounting
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(second: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(second, 0).unwrap()
    }

    fn rates(accounting: PricingAccounting, rate: u64) -> Vec<TokenRate> {
        accounting
            .required_categories()
            .iter()
            .map(|category| TokenRate {
                category: *category,
                usd_nanos_per_million_tokens: rate,
            })
            .collect()
    }

    fn entry(from: i64, until: Option<i64>, rate: u64) -> ModelPriceEntry {
        ModelPriceEntry {
            provider: "synthetic-provider".to_owned(),
            model: "synthetic-model".to_owned(),
            accounting: PricingAccounting::Codex,
            effective_from: at(from),
            effective_until: until.map(at),
            prompt_tokens_above: None,
            prompt_tokens_at_most: None,
            rates: rates(PricingAccounting::Codex, rate),
        }
    }

    fn table(entries: Vec<ModelPriceEntry>) -> PricingTable {
        let mut table = PricingTable {
            schema_version: PRICING_TABLE_SCHEMA_VERSION,
            table_id: String::new(),
            version: 1,
            currency: PricingCurrency::Usd,
            published_at: at(100),
            provenance: PricingTableProvenance {
                publisher: "Synthetic fixture".to_owned(),
                source_url: "https://example.invalid/prices/v1".to_owned(),
                retrieved_at: at(100),
                content_sha256:
                    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                        .to_owned(),
                reviewed_by: "Synthetic reviewer".to_owned(),
                reviewed_at: at(101),
                effective_from_basis: None,
                source_notes: None,
            },
            entries,
        };
        table.table_id = table.canonical_table_id().unwrap();
        table
    }

    fn input(from: i64, until: i64, tokens: [u64; 4]) -> PricingUsageInput {
        PricingUsageInput {
            provider: "synthetic-provider".to_owned(),
            model: "synthetic-model".to_owned(),
            accounting: PricingAccounting::Codex,
            observed_from: at(from),
            observed_until: at(until),
            counts: PricingAccounting::Codex
                .required_categories()
                .iter()
                .zip(tokens)
                .map(|(category, tokens)| BillableTokenCount {
                    category: *category,
                    tokens,
                })
                .collect(),
        }
    }

    fn claude_table() -> PricingTable {
        // Synthetic rates: 5-minute write 7, 1-hour write 11 nanos per
        // million tokens; every other category 1.
        let mut entry = entry(0, None, 1);
        entry.accounting = PricingAccounting::ClaudeCode;
        entry.rates = PricingAccounting::ClaudeCode
            .required_categories()
            .iter()
            .map(|category| TokenRate {
                category: *category,
                usd_nanos_per_million_tokens: match category {
                    BillableTokenCategory::CacheWriteFiveMinuteInput => 7,
                    BillableTokenCategory::CacheWriteOneHourInput => 11,
                    _ => 1,
                },
            })
            .collect();
        table(vec![entry])
    }

    fn claude_input(tokens: [u64; 5]) -> PricingUsageInput {
        PricingUsageInput {
            provider: "synthetic-provider".to_owned(),
            model: "synthetic-model".to_owned(),
            accounting: PricingAccounting::ClaudeCode,
            observed_from: at(1),
            observed_until: at(2),
            counts: PricingAccounting::ClaudeCode
                .required_categories()
                .iter()
                .zip(tokens)
                .map(|(category, tokens)| BillableTokenCount {
                    category: *category,
                    tokens,
                })
                .collect(),
        }
    }

    #[test]
    fn claude_accounting_prices_each_cache_write_lifetime_at_its_own_rate() {
        // 1_000_000_000 tokens of each write lifetime: 7 + 11 = 18 micros.
        let estimate = calculate_estimated_cost(
            &claude_table(),
            &claude_input([0, 0, 1_000_000_000, 1_000_000_000, 0]),
        )
        .unwrap();
        assert_eq!(estimate.estimated_cost_usd_micros, 18);
        assert_eq!(
            estimate
                .breakdown
                .iter()
                .map(|b| b.category)
                .collect::<Vec<_>>(),
            PricingAccounting::ClaudeCode.required_categories()
        );
    }

    fn banded(above: Option<u64>, at_most: Option<u64>, rate: u64) -> ModelPriceEntry {
        let mut banded = entry(0, None, rate);
        banded.prompt_tokens_above = above;
        banded.prompt_tokens_at_most = at_most;
        banded
    }

    #[test]
    fn codex_accounting_prices_a_cache_write_at_the_thirty_minute_rate() {
        use BillableTokenCategory::*;
        assert_eq!(
            PricingAccounting::Codex.required_categories(),
            &[
                UncachedInput,
                CachedInput,
                CacheWriteThirtyMinuteInput,
                Output
            ]
        );
        assert_eq!(
            serde_json::to_value(CacheWriteThirtyMinuteInput).unwrap(),
            "cache_write_thirty_minute_input"
        );
        // Synthetic rates: the write 9 nanos per million tokens, every other
        // category 1. 1_000_000_000 written tokens: 9 micros.
        let mut write = entry(0, None, 1);
        write.rates[2].usd_nanos_per_million_tokens = 9;
        let estimate =
            calculate_estimated_cost(&table(vec![write]), &input(0, 0, [0, 0, 1_000_000_000, 0]))
                .unwrap();
        assert_eq!(estimate.estimated_cost_usd_micros, 9);
    }

    /// Rates of 1 and 2 USD per million tokens are 1 and 2 micros a token.
    #[test]
    fn a_prompt_prices_at_the_band_its_own_input_size_falls_in() {
        let table = table(vec![
            banded(None, Some(100), 1_000_000_000),
            banded(Some(100), None, 2_000_000_000),
        ]);
        assert!(table.validate().is_ok());
        let cost = |tokens| {
            calculate_estimated_cost(&table, &input(0, 0, tokens))
                .unwrap()
                .estimated_cost_usd_micros
        };
        // Prompt 60 + 30 + 10 = 100: the base band, output included at 1.
        assert_eq!(cost([60, 30, 10, 1_000]), 1_100);
        // Prompt 101: the whole turn moves to the long band.
        assert_eq!(cost([61, 30, 10, 1_000]), 2_202);
    }

    #[test]
    fn a_prompt_between_two_bands_is_unresolved_not_priced_at_either() {
        let table = table(vec![
            banded(None, Some(100), 1_000_000_000),
            banded(Some(105), None, 2_000_000_000),
        ]);
        assert!(table.validate().is_ok());
        assert_eq!(
            calculate_estimated_cost(&table, &input(0, 0, [103, 0, 0, 0])),
            Err(PricingError::UnresolvedWindow)
        );
        assert!(calculate_estimated_cost(&table, &input(0, 0, [106, 0, 0, 0])).is_ok());
    }

    #[test]
    fn overlapping_or_empty_bands_never_validate() {
        let overlapping = table(vec![banded(None, Some(100), 1), banded(Some(99), None, 2)]);
        assert_eq!(overlapping.validate(), Err(PricingError::InvalidTable));

        let unbanded_and_banded = table(vec![banded(None, None, 1), banded(Some(100), None, 2)]);
        assert_eq!(
            unbanded_and_banded.validate(),
            Err(PricingError::InvalidTable)
        );

        let empty = table(vec![banded(Some(100), Some(100), 1)]);
        assert_eq!(empty.validate(), Err(PricingError::InvalidTable));

        let unordered = table(vec![banded(Some(100), None, 2), banded(None, Some(100), 1)]);
        assert_eq!(unordered.validate(), Err(PricingError::InvalidTable));

        // Disjoint bands may share a window; a band's later window follows
        // its earlier one.
        let mut early_long = banded(Some(100), None, 2);
        early_long.effective_until = Some(at(10));
        let mut later_long = banded(Some(100), None, 3);
        later_long.effective_from = at(10);
        let dated = table(vec![banded(None, Some(100), 1), early_long, later_long]);
        assert_eq!(dated.validate(), Ok(()));
    }

    /// Absent optional fields are not serialized, so a table that uses
    /// neither keeps the digest body it had before they existed.
    #[test]
    fn unused_band_and_basis_fields_are_absent_from_the_digest_body() {
        let plain = table(vec![entry(0, None, 1)]);
        let value = serde_json::to_value(&plain).unwrap();
        assert!(value.pointer("/entries/0/prompt_tokens_above").is_none());
        assert!(value.pointer("/entries/0/prompt_tokens_at_most").is_none());
        assert!(value.pointer("/provenance/effective_from_basis").is_none());
        assert!(value.pointer("/provenance/source_notes").is_none());
    }

    #[test]
    fn source_notes_are_bounded_text_and_move_the_digest() {
        let plain = table(vec![entry(0, None, 1)]);
        let mut noted = plain.clone();
        noted.provenance.source_notes = Some("Page bytes change on every build.".to_owned());
        noted.table_id = noted.canonical_table_id().unwrap();
        assert_eq!(noted.validate(), Ok(()));
        assert_ne!(noted.table_id, plain.table_id);

        for bad in [String::new(), " padded".to_owned(), "x".repeat(1_025)] {
            let mut invalid = plain.clone();
            invalid.provenance.source_notes = Some(bad);
            invalid.table_id = invalid.canonical_table_id().unwrap();
            assert_eq!(invalid.validate(), Err(PricingError::InvalidTable));
        }
        let mut longest = plain;
        longest.provenance.source_notes = Some("x".repeat(1_024));
        longest.table_id = longest.canonical_table_id().unwrap();
        assert_eq!(longest.validate(), Ok(()));
    }

    #[test]
    fn an_effective_from_basis_is_bounded_text() {
        let mut stated = table(vec![entry(0, None, 1)]);
        stated.provenance.effective_from_basis =
            Some("No date on the page; window start used".to_owned());
        stated.table_id = stated.canonical_table_id().unwrap();
        assert_eq!(stated.validate(), Ok(()));

        for bad in [String::new(), " padded".to_owned(), "x".repeat(257)] {
            let mut invalid = table(vec![entry(0, None, 1)]);
            invalid.provenance.effective_from_basis = Some(bad);
            invalid.table_id = invalid.canonical_table_id().unwrap();
            assert_eq!(invalid.validate(), Err(PricingError::InvalidTable));
        }
    }

    #[test]
    fn a_schema_v1_table_never_validates() {
        let mut old = table(vec![entry(0, None, 1)]);
        old.schema_version = 1;
        old.table_id = old.canonical_table_id().unwrap();
        assert_eq!(old.validate(), Err(PricingError::InvalidTable));
    }

    #[test]
    fn split_category_serde_spellings_are_pinned() {
        assert_eq!(
            serde_json::to_value(BillableTokenCategory::CacheWriteFiveMinuteInput).unwrap(),
            "cache_write_five_minute_input"
        );
        assert_eq!(
            serde_json::to_value(BillableTokenCategory::CacheWriteOneHourInput).unwrap(),
            "cache_write_one_hour_input"
        );
    }

    #[test]
    fn canonical_table_digest_and_body_substitution_are_bound() {
        let first = table(vec![entry(0, None, 1)]);
        assert!(first.validate().is_ok());
        assert_eq!(first.canonical_table_id().unwrap(), first.table_id);

        let mut changed = first.clone();
        changed.entries[0].rates[0].usd_nanos_per_million_tokens = 2;
        assert_eq!(changed.validate(), Err(PricingError::InvalidTable));
        assert_ne!(changed.canonical_table_id().unwrap(), first.table_id);
    }

    #[test]
    fn a_validated_table_prices_like_the_checked_path_without_rehashing() {
        let validated = claude_table().into_validated().unwrap();
        let input = claude_input([5, 6, 7, 8, 9]);
        let digests = table_digests_computed_on_this_thread();
        let fast = calculate_estimated_cost_validated(&validated, &input).unwrap();
        assert_eq!(table_digests_computed_on_this_thread(), digests);
        assert_eq!(
            fast,
            calculate_estimated_cost(validated.as_table(), &input).unwrap()
        );
        assert!(table_digests_computed_on_this_thread() > digests);

        let mut changed = claude_table();
        changed.entries[0].rates[0].usd_nanos_per_million_tokens = 2;
        assert_eq!(
            changed.into_validated().unwrap_err(),
            PricingError::InvalidTable
        );
    }

    #[test]
    fn provenance_version_and_https_credentials_are_validated() {
        let mut invalid = table(vec![entry(0, None, 1)]);
        invalid.version = 0;
        invalid.table_id = invalid.canonical_table_id().unwrap();
        assert_eq!(invalid.validate(), Err(PricingError::InvalidTable));

        let mut credentials = table(vec![entry(0, None, 1)]);
        credentials.provenance.source_url = "https://secret@example.invalid/prices/v1".to_owned();
        credentials.table_id = credentials.canonical_table_id().unwrap();
        assert_eq!(credentials.validate(), Err(PricingError::InvalidTable));

        let mut wrong_digest = table(vec![entry(0, None, 1)]);
        wrong_digest.provenance.content_sha256 = "sha256:ABC".to_owned();
        wrong_digest.table_id = wrong_digest.canonical_table_id().unwrap();
        assert_eq!(wrong_digest.validate(), Err(PricingError::InvalidTable));
    }

    #[test]
    fn tables_require_canonical_complete_nonoverlapping_entries() {
        let overlapping = table(vec![entry(0, Some(20), 1), entry(10, None, 2)]);
        assert_eq!(overlapping.validate(), Err(PricingError::InvalidTable));

        let duplicate = table(vec![entry(0, Some(20), 1), entry(0, Some(20), 1)]);
        assert_eq!(duplicate.validate(), Err(PricingError::InvalidTable));

        let unordered = table(vec![entry(20, None, 2), entry(0, Some(20), 1)]);
        assert_eq!(unordered.validate(), Err(PricingError::InvalidTable));

        let mut missing_category = table(vec![entry(0, None, 1)]);
        missing_category.entries[0].rates.pop();
        missing_category.table_id = missing_category.canonical_table_id().unwrap();
        assert_eq!(missing_category.validate(), Err(PricingError::InvalidTable));
    }

    #[test]
    fn half_open_windows_include_start_and_exclude_final_at_end() {
        let table = table(vec![entry(0, Some(10), 1), entry(10, Some(20), 2)]);
        assert!(calculate_estimated_cost(&table, &input(0, 9, [1, 1, 0, 1])).is_ok());
        assert!(calculate_estimated_cost(&table, &input(10, 10, [1, 1, 0, 1])).is_ok());
        assert_eq!(
            calculate_estimated_cost(&table, &input(0, 10, [1, 1, 0, 1])),
            Err(PricingError::UnresolvedWindow)
        );
    }

    #[test]
    fn gaps_and_crossings_are_unresolved() {
        let table = table(vec![entry(0, Some(10), 1), entry(20, None, 2)]);
        assert_eq!(
            calculate_estimated_cost(&table, &input(10, 10, [1, 1, 0, 1])),
            Err(PricingError::UnresolvedWindow)
        );
        assert_eq!(
            calculate_estimated_cost(&table, &input(9, 20, [1, 1, 0, 1])),
            Err(PricingError::UnresolvedWindow)
        );
    }

    #[test]
    fn provider_model_accounting_and_categories_are_exact() {
        let table = table(vec![entry(0, None, 1)]);
        let mut wrong_model = input(0, 0, [1, 1, 0, 1]);
        wrong_model.model = "another-model".to_owned();
        assert_eq!(
            calculate_estimated_cost(&table, &wrong_model),
            Err(PricingError::MissingPrice)
        );

        let mut wrong_category = input(0, 0, [1, 1, 0, 1]);
        wrong_category.counts.swap(0, 1);
        assert_eq!(
            calculate_estimated_cost(&table, &wrong_category),
            Err(PricingError::InvalidInput)
        );
    }

    #[test]
    fn zero_submicro_and_half_tie_are_distinct_and_rounded_once() {
        let zero =
            calculate_estimated_cost(&table(vec![entry(0, None, 1)]), &input(0, 0, [0, 0, 0, 0]))
                .unwrap();
        assert_eq!(zero.estimated_cost_usd_micros, 0);
        assert!(!zero.nonzero_rounded_to_zero);

        let submicro =
            calculate_estimated_cost(&table(vec![entry(0, None, 1)]), &input(0, 0, [1, 0, 0, 0]))
                .unwrap();
        assert_eq!(submicro.estimated_cost_usd_micros, 0);
        assert!(submicro.nonzero_rounded_to_zero);

        let tie = calculate_estimated_cost(
            &table(vec![entry(0, None, 250_000_000)]),
            &input(0, 0, [1, 1, 0, 0]),
        )
        .unwrap();
        assert_eq!(tie.estimated_cost_usd_micros, 1);
        assert!(!tie.nonzero_rounded_to_zero);
    }

    #[test]
    fn exact_category_fractions_sum_before_rounding() {
        let estimate = calculate_estimated_cost(
            &table(vec![entry(0, None, 200_000_000)]),
            &input(0, 0, [1, 1, 1, 1]),
        )
        .unwrap();
        assert_eq!(estimate.estimated_cost_usd_micros, 1);
        assert_eq!(estimate.breakdown.len(), 4);
        assert!(estimate.breakdown.iter().all(|item| item.tokens == 1));
    }

    #[test]
    fn checked_sum_rounding_and_narrowing_overflow_fail_closed() {
        let overflow = calculate_estimated_cost(
            &table(vec![entry(0, None, u64::MAX)]),
            &input(0, 0, [u64::MAX, u64::MAX, u64::MAX, u64::MAX]),
        );
        assert_eq!(overflow, Err(PricingError::ArithmeticOverflow));

        let narrowing = calculate_estimated_cost(
            &table(vec![entry(0, None, u64::MAX)]),
            &input(0, 0, [u64::MAX, 0, 0, 0]),
        );
        assert_eq!(narrowing, Err(PricingError::ArithmeticOverflow));
    }

    #[test]
    fn serialized_contract_contains_no_u128_or_float_values() {
        let estimate =
            calculate_estimated_cost(&table(vec![entry(0, None, 10)]), &input(0, 0, [1, 2, 0, 3]))
                .unwrap();
        let value = serde_json::to_value(&estimate).unwrap();
        assert_eq!(
            serde_json::to_string(&estimate).unwrap(),
            r#"{"schema_version":2,"calculator_version":"deterministic-usd-cost-v1","table_id":"sha256:caf7f71500c8ed26c429bd2a75b545ee611cc68150d5b4a836b21b5d7177b4a2","table_version":1,"currency":"usd","provider":"synthetic-provider","model":"synthetic-model","accounting":"codex","observed_from":"1970-01-01T00:00:00Z","observed_until":"1970-01-01T00:00:00Z","breakdown":[{"category":"uncached_input","tokens":1,"usd_nanos_per_million_tokens":10},{"category":"cached_input","tokens":2,"usd_nanos_per_million_tokens":10},{"category":"cache_write_thirty_minute_input","tokens":0,"usd_nanos_per_million_tokens":10},{"category":"output","tokens":3,"usd_nanos_per_million_tokens":10}],"estimated_cost_usd_micros":0,"nonzero_rounded_to_zero":true}"#
        );
        assert!(
            value
                .pointer("/estimated_cost_usd_micros")
                .unwrap()
                .is_u64()
        );
        assert!(value.pointer("/breakdown/0/tokens").unwrap().is_u64());
        assert!(
            value
                .pointer("/breakdown/0/usd_nanos_per_million_tokens")
                .unwrap()
                .is_u64()
        );
    }
}
