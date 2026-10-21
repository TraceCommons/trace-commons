//! A session's stored turns priced at published API list prices.
//!
//! Local only: no I/O, no network, no persistence. Nothing here is stored or
//! put on a wire; callers price turns when an answer is built, from the
//! tokens the store already holds.
//!
//! "At API prices" is a list-price estimate: what the vendor's API would
//! charge for these tokens, not what a subscription plan costs.
//!
//! Codex turns. PR 1's Codex reader stores each `token_count` event through
//! [`codex_turn_usage`], and through nothing else: `uncached_input =
//! input_tokens - cached_input_tokens - cache_write_input_tokens`,
//! `cache_read = cached_input_tokens`, `cache_write =
//! TotalOnly(cache_write_input_tokens)`, `output = output_tokens` (reasoning
//! is a subset of output). So `context()` is `input_tokens`, and the write
//! is never inside `uncached_input`; storing `input - cached` instead would
//! price every written token twice. OpenAI's
//! pricing page lists a separate cache-write rate, and its prompt-caching
//! guide says that rate replaces the uncached-input rate for those tokens
//! rather than adding to it. The write is priced at the 30-minute rate:
//! the guide makes 30 minutes the default lifetime unless a request asks for
//! 12 hours, and the Codex CLI checked on 2026-10-20 (0.163.0) sets no cache
//! lifetime (its binary holds no `ttl` field or `12h` value). No 12-hour
//! category exists; a Codex release that asks for 12 hours needs one.
//!
//! A turn is priced or unpriced, never zero. A session with any unpriced
//! counted turn has no total; the total is never the sum of the priced part.
//!
//! Each figure is the checked sum of exact numerators (tokens times USD nanos
//! per million tokens), rounded half-up to USD micros once. A model whose
//! turns each price but whose sum does not fit in `u64` micros has no figure:
//! its priced turns are counted as unpriced with [`UnpricedReason::Overflow`],
//! so `usd_micros` is `Some` exactly when every turn priced.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use trace_commons_protocol::insights_pricing::{
    BillableTokenCategory, BillableTokenCount, PricingAccounting, PricingError, PricingUsageInput,
    calculate_estimated_cost,
};
use trace_commons_protocol::insights_usage_series::{CacheWrite, KnownTurnUsage, TurnRecord};

use super::pricing_catalog::{PricingCatalog, PricingCatalogError};

/// The pricing basis of every figure here: published API list prices.
pub const PRICING_BASIS_API_LIST: &str = "api_list";

const SYNTHETIC_LABEL: &str = "<synthetic>";
const MAX_LABEL_BYTES: usize = 96;
const DATE_STAMP_DIGITS: usize = 8;
const LATEST_SUFFIX: &str = "-latest";
const MICROS_ROUNDING_DENOMINATOR: u128 = 1_000_000_000;

/// One pricing table a priced turn used.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct PricingTableRef {
    pub table_id: String,
    pub version: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnpricedReason {
    UsageUnknown,
    Undated,
    NoLabel,
    CacheLifetimeUnknown,
    MissingPrice,
    WindowUnresolved,
    Overflow,
    InvalidCatalog,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ModelPrice {
    pub priced_turns: u32,
    pub unpriced_turns: u32,
    /// `Some` only when `unpriced_turns == 0` and `priced_turns > 0`.
    pub usd_micros: Option<u64>,
    pub unpriced_reasons: BTreeMap<UnpricedReason, u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SessionPrice {
    /// Keyed by normalized model label.
    pub by_model: BTreeMap<String, ModelPrice>,
    /// Unlabelled zero-usage turns (the `<synthetic>` shape): no model, not
    /// counted, not unpriced.
    pub skipped_turns: u32,
    /// Counted turns with no usable label; each makes the total unknown.
    pub unlabelled_turns: u32,
    /// `Some` only when every counted turn is priced and at least one is.
    pub total_usd_micros: Option<u64>,
    /// Tables the priced turns used, sorted and distinct.
    pub tables: Vec<PricingTableRef>,
}

/// Lowercase a declared model label and drop one trailing `-YYYYMMDD` or
/// `-latest`. `None` for `<synthetic>`, an empty result, or a label outside
/// the pricing id charset (`[A-Za-z0-9-_.:]`, at most 96 bytes).
pub fn normalize_model_label(label: &str) -> Option<String> {
    if is_synthetic(label) {
        return None;
    }
    let lowered = label.to_ascii_lowercase();
    let stem = strip_date_stamp(&lowered)
        .or_else(|| lowered.strip_suffix(LATEST_SUFFIX))
        .unwrap_or(&lowered);
    pricing_id(stem).then(|| stem.to_owned())
}

/// One Codex `token_count` event's `last_token_usage` as stored counters,
/// the only mapping the pricing here is correct for (see the module doc).
/// `None` when cached and written tokens exceed the input, or a counter
/// does not fit the store: usage unknown, never a wrapped or clamped count.
pub fn codex_turn_usage(
    input_tokens: u64,
    cached_input_tokens: u64,
    cache_write_input_tokens: u64,
    output_tokens: u64,
) -> Option<KnownTurnUsage> {
    let uncached = input_tokens
        .checked_sub(cached_input_tokens)?
        .checked_sub(cache_write_input_tokens)?;
    Some(KnownTurnUsage {
        uncached_input: u32::try_from(uncached).ok()?,
        cache_read: u32::try_from(cached_input_tokens).ok()?,
        cache_write: CacheWrite::TotalOnly(u32::try_from(cache_write_input_tokens).ok()?),
        output: u32::try_from(output_tokens).ok()?,
    })
}

/// The publisher whose list prices apply to an accounting family. Never
/// inferred from the label.
pub fn pricing_provider(accounting: PricingAccounting) -> &'static str {
    match accounting {
        PricingAccounting::ClaudeCode => "anthropic",
        PricingAccounting::Codex => "openai",
    }
}

#[derive(Default)]
struct ModelTally {
    priced_turns: u32,
    unpriced_reasons: BTreeMap<UnpricedReason, u32>,
    /// `None` once a checked sum failed.
    exact: Option<u128>,
    tables: BTreeSet<PricingTableRef>,
}

pub fn price_session(
    catalog: &PricingCatalog,
    accounting: PricingAccounting,
    model_labels: &[String],
    turns: &[TurnRecord],
) -> SessionPrice {
    let mut price = SessionPrice::default();
    let mut tallies: BTreeMap<String, ModelTally> = BTreeMap::new();
    for turn in turns {
        let label = turn
            .model_label_ix
            .and_then(|ix| model_labels.get(usize::from(ix)));
        let Some(label) = label else {
            price.tally_unattributed(turn, true);
            continue;
        };
        let Some(model) = normalize_model_label(label) else {
            price.tally_unattributed(turn, is_synthetic(label));
            continue;
        };
        let tally = tallies.entry(model.clone()).or_insert_with(|| ModelTally {
            exact: Some(0),
            ..ModelTally::default()
        });
        match price_turn(catalog, accounting, &model, turn) {
            Ok((numerator, table)) => {
                tally.priced_turns = tally.priced_turns.saturating_add(1);
                tally.exact = tally.exact.and_then(|sum| sum.checked_add(numerator));
                tally.tables.insert(table);
            }
            Err(reason) => {
                let count = tally.unpriced_reasons.entry(reason).or_default();
                *count = count.saturating_add(1);
            }
        }
    }

    let mut total = Some(0u128);
    let mut tables = BTreeSet::new();
    for (model, mut tally) in tallies {
        let micros = tally.exact.and_then(round_to_micros);
        if micros.is_none() && tally.priced_turns > 0 {
            let count = tally
                .unpriced_reasons
                .entry(UnpricedReason::Overflow)
                .or_default();
            *count = count.saturating_add(tally.priced_turns);
            tally.priced_turns = 0;
        }
        let unpriced_turns = tally
            .unpriced_reasons
            .values()
            .fold(0u32, |sum, count| sum.saturating_add(*count));
        let complete = unpriced_turns == 0 && tally.priced_turns > 0;
        if complete {
            total = total.and_then(|sum| tally.exact.and_then(|exact| sum.checked_add(exact)));
        } else {
            total = None;
        }
        if tally.priced_turns > 0 {
            tables.extend(tally.tables);
        }
        price.by_model.insert(
            model,
            ModelPrice {
                priced_turns: tally.priced_turns,
                unpriced_turns,
                usd_micros: if complete { micros } else { None },
                unpriced_reasons: tally.unpriced_reasons,
            },
        );
    }
    price.total_usd_micros = if price.unlabelled_turns == 0 && !price.by_model.is_empty() {
        total.and_then(round_to_micros)
    } else {
        None
    };
    price.tables = tables.into_iter().collect();
    price
}

impl SessionPrice {
    /// A turn with no usable model. It is skipped only in the `<synthetic>`
    /// shape (no label or the `<synthetic>` label, and known usage totalling
    /// zero); any other such turn is counted and leaves the total unknown, so
    /// its tokens are never dropped from a total.
    fn tally_unattributed(&mut self, turn: &TurnRecord, may_skip: bool) {
        if may_skip && turn.known_usage().is_some_and(|usage| usage.total() == 0) {
            self.skipped_turns = self.skipped_turns.saturating_add(1);
        } else {
            self.unlabelled_turns = self.unlabelled_turns.saturating_add(1);
        }
    }
}

/// Price one labelled turn: its exact numerator and the table it used.
fn price_turn(
    catalog: &PricingCatalog,
    accounting: PricingAccounting,
    model: &str,
    turn: &TurnRecord,
) -> Result<(u128, PricingTableRef), UnpricedReason> {
    let usage = turn.known_usage().ok_or(UnpricedReason::UsageUnknown)?;
    let at = turn.at.ok_or(UnpricedReason::Undated)?;
    let input = PricingUsageInput {
        provider: pricing_provider(accounting).to_owned(),
        model: model.to_owned(),
        accounting,
        observed_from: at,
        observed_until: at,
        counts: billable_counts(accounting, &usage)?,
    };
    let table = catalog
        .select_newest_applicable(&input)
        .map_err(|error| match error {
            PricingCatalogError::MissingPrice => UnpricedReason::MissingPrice,
            PricingCatalogError::UnresolvedWindow => UnpricedReason::WindowUnresolved,
            PricingCatalogError::InvalidQuery
            | PricingCatalogError::InvalidCatalog
            | PricingCatalogError::DuplicateTable
            | PricingCatalogError::DuplicateVersion
            | PricingCatalogError::StoredTableUnavailable => UnpricedReason::InvalidCatalog,
        })?;
    let estimate = calculate_estimated_cost(table, &input).map_err(|error| match error {
        PricingError::ArithmeticOverflow => UnpricedReason::Overflow,
        PricingError::MissingPrice => UnpricedReason::MissingPrice,
        PricingError::UnresolvedWindow => UnpricedReason::WindowUnresolved,
        PricingError::InvalidTable | PricingError::InvalidInput => UnpricedReason::InvalidCatalog,
    })?;
    // Recomputed from the breakdown: the estimate's micros are already
    // rounded, and a session rounds once over exact numerators.
    let numerator = estimate.breakdown.iter().try_fold(0u128, |sum, item| {
        u128::from(item.tokens)
            .checked_mul(u128::from(item.usd_nanos_per_million_tokens))
            .and_then(|part| sum.checked_add(part))
            .ok_or(UnpricedReason::Overflow)
    })?;
    Ok((
        numerator,
        PricingTableRef {
            table_id: table.table_id.clone(),
            version: table.version,
        },
    ))
}

/// The accounting family's categories, in its required order.
///
/// Claude: a `TotalOnly` write (the proxy ledger's shape) is never
/// list-priced, an empty one included (spec section 3).
///
/// Codex: the stored `uncached_input` excludes the cache write (see the
/// module doc), so the write is priced once, at its own rate. A Claude-shaped
/// split write has no OpenAI lifetime and is unpriced unless empty.
fn billable_counts(
    accounting: PricingAccounting,
    usage: &KnownTurnUsage,
) -> Result<Vec<BillableTokenCount>, UnpricedReason> {
    use BillableTokenCategory::*;
    let count = |category, tokens: u32| BillableTokenCount {
        category,
        tokens: u64::from(tokens),
    };
    Ok(match accounting {
        PricingAccounting::ClaudeCode => {
            let CacheWrite::Split { m5, h1 } = usage.cache_write else {
                return Err(UnpricedReason::CacheLifetimeUnknown);
            };
            vec![
                count(UncachedInput, usage.uncached_input),
                count(CacheReadInput, usage.cache_read),
                count(CacheWriteFiveMinuteInput, m5),
                count(CacheWriteOneHourInput, h1),
                count(Output, usage.output),
            ]
        }
        PricingAccounting::Codex => {
            let write = match usage.cache_write {
                CacheWrite::TotalOnly(write) => write,
                CacheWrite::Split { m5: 0, h1: 0 } => 0,
                CacheWrite::Split { .. } => return Err(UnpricedReason::CacheLifetimeUnknown),
            };
            vec![
                count(UncachedInput, usage.uncached_input),
                count(CachedInput, usage.cache_read),
                count(CacheWriteThirtyMinuteInput, write),
                count(Output, usage.output),
            ]
        }
    })
}

fn round_to_micros(numerator: u128) -> Option<u64> {
    let rounded =
        numerator.checked_add(MICROS_ROUNDING_DENOMINATOR / 2)? / MICROS_ROUNDING_DENOMINATOR;
    u64::try_from(rounded).ok()
}

fn is_synthetic(label: &str) -> bool {
    label.eq_ignore_ascii_case(SYNTHETIC_LABEL)
}

fn strip_date_stamp(label: &str) -> Option<&str> {
    let split = label.len().checked_sub(DATE_STAMP_DIGITS + 1)?;
    let (stem, stamp) = (label.get(..split)?, label.get(split..)?);
    let digits = stamp.strip_prefix('-')?;
    digits
        .bytes()
        .all(|byte| byte.is_ascii_digit())
        .then_some(stem)
}

fn pricing_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_LABEL_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(&byte))
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, TimeZone, Utc};
    use trace_commons_protocol::insights_pricing::{
        BillableTokenCategory, ModelPriceEntry, PRICING_TABLE_SCHEMA_VERSION, PricingAccounting,
        PricingCurrency, PricingTable, PricingTableProvenance, TokenRate,
    };
    use trace_commons_protocol::insights_usage_series::{CacheWrite, MAX_SERIES_TURNS, TurnRecord};

    use super::*;
    use crate::insights::pricing_catalog::PricingCatalog;

    const MILLION: u32 = 1_000_000;

    fn at(second: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(second, 0).unwrap()
    }

    /// Synthetic rates in USD nanos per million tokens, distinct per category
    /// so a swapped category changes the figure.
    fn synthetic_rate(category: BillableTokenCategory) -> u64 {
        use BillableTokenCategory::*;
        match category {
            UncachedInput => 1_000_000_000,
            CachedInput | CacheReadInput => 2_000_000_000,
            CacheWriteFiveMinuteInput => 3_000_000_000,
            CacheWriteOneHourInput => 5_000_000_000,
            CacheWriteThirtyMinuteInput => 11_000_000_000,
            Output => 7_000_000_000,
        }
    }

    fn rates(
        accounting: PricingAccounting,
        rate: impl Fn(BillableTokenCategory) -> u64,
    ) -> Vec<TokenRate> {
        accounting
            .required_categories()
            .iter()
            .map(|category| TokenRate {
                category: *category,
                usd_nanos_per_million_tokens: rate(*category),
            })
            .collect()
    }

    fn entry(
        accounting: PricingAccounting,
        model: &str,
        from: i64,
        until: Option<i64>,
        rate: impl Fn(BillableTokenCategory) -> u64,
    ) -> ModelPriceEntry {
        ModelPriceEntry {
            provider: pricing_provider(accounting).to_owned(),
            model: model.to_owned(),
            accounting,
            effective_from: at(from),
            effective_until: until.map(at),
            prompt_tokens_above: None,
            prompt_tokens_at_most: None,
            rates: rates(accounting, rate),
        }
    }

    fn table(version: u32, entries: Vec<ModelPriceEntry>) -> PricingTable {
        let mut table = PricingTable {
            schema_version: PRICING_TABLE_SCHEMA_VERSION,
            table_id: String::new(),
            version,
            currency: PricingCurrency::Usd,
            published_at: at(100 + i64::from(version)),
            provenance: PricingTableProvenance {
                publisher: "Synthetic fixture".to_owned(),
                source_url: format!("https://example.invalid/prices/{version}"),
                retrieved_at: at(100),
                content_sha256:
                    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                        .to_owned(),
                reviewed_by: "Synthetic reviewer".to_owned(),
                reviewed_at: at(101),
                effective_from_basis: None,
            },
            entries,
        };
        table.table_id = table.canonical_table_id().unwrap();
        table
    }

    fn catalog(tables: Vec<PricingTable>) -> PricingCatalog {
        PricingCatalog::from_tables(tables).unwrap()
    }

    fn claude_catalog(models: &[&str]) -> PricingCatalog {
        catalog(vec![table(
            1,
            models
                .iter()
                .map(|model| {
                    entry(
                        PricingAccounting::ClaudeCode,
                        model,
                        0,
                        None,
                        synthetic_rate,
                    )
                })
                .collect(),
        )])
    }

    fn turn(
        ordinal: u32,
        when: Option<i64>,
        label: Option<u16>,
        counts: [Option<u32>; 3],
        cache_write: Option<CacheWrite>,
    ) -> TurnRecord {
        let [uncached_input, cache_read, output] = counts;
        TurnRecord {
            ordinal,
            at: when.map(at),
            uncached_input,
            cache_read,
            cache_write,
            output,
            model_label_ix: label,
            gap_secs: None,
            msg_key: None,
        }
    }

    /// One labelled Claude turn: 1M uncached, 1M read, 1M 5-minute write,
    /// 2M 1-hour write, 1M output. Synthetic rates make it 1+2+3+10+7 = 23
    /// USD, so 23_000_000 micros.
    fn priced_claude_turn(ordinal: u32, label: u16) -> TurnRecord {
        turn(
            ordinal,
            Some(10),
            Some(label),
            [Some(MILLION), Some(MILLION), Some(MILLION)],
            Some(CacheWrite::Split {
                m5: MILLION,
                h1: 2 * MILLION,
            }),
        )
    }

    fn labels(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn labels_normalize_by_case_date_stamp_and_latest() {
        assert_eq!(
            normalize_model_label("Test-Model-A-20261001").as_deref(),
            Some("test-model-a")
        );
        assert_eq!(
            normalize_model_label("test-model-a-latest").as_deref(),
            Some("test-model-a")
        );
        assert_eq!(
            normalize_model_label("test-model-a-2026100").as_deref(),
            Some("test-model-a-2026100"),
            "seven digits are not a date stamp"
        );
        assert_eq!(
            normalize_model_label("test-model-a-20261001-20261002").as_deref(),
            Some("test-model-a-20261001"),
            "only one trailing stamp is removed"
        );
        assert_eq!(normalize_model_label("<synthetic>"), None);
        assert_eq!(normalize_model_label("vendor/model"), None);
        assert_eq!(normalize_model_label("-latest"), None);
        assert_eq!(normalize_model_label(""), None);
        assert_eq!(normalize_model_label(&"a".repeat(97)), None);
        assert_eq!(
            normalize_model_label(&"a".repeat(96)).as_deref(),
            Some("a".repeat(96).as_str())
        );
    }

    #[test]
    fn a_claude_split_write_prices_each_part_at_its_rate() {
        let price = price_session(
            &claude_catalog(&["test-model-a"]),
            PricingAccounting::ClaudeCode,
            &labels(&["Test-Model-A-20261001"]),
            &[priced_claude_turn(0, 0)],
        );
        let model = &price.by_model["test-model-a"];
        assert_eq!(model.priced_turns, 1);
        assert_eq!(model.unpriced_turns, 0);
        // A swap of the two write rates would give 1+2+6+5+7 = 21.
        assert_eq!(model.usd_micros, Some(23_000_000));
        assert_eq!(price.total_usd_micros, Some(23_000_000));
    }

    #[test]
    fn a_total_only_write_is_never_list_priced() {
        let mut total_only = priced_claude_turn(0, 0);
        total_only.cache_write = Some(CacheWrite::TotalOnly(3 * MILLION));
        let price = price_session(
            &claude_catalog(&["test-model-a"]),
            PricingAccounting::ClaudeCode,
            &labels(&["test-model-a"]),
            &[total_only],
        );
        let model = &price.by_model["test-model-a"];
        assert_eq!(
            model.unpriced_reasons[&UnpricedReason::CacheLifetimeUnknown],
            1
        );
        assert_eq!(model.priced_turns, 0);
        assert_eq!(model.usd_micros, None);
        assert_eq!(price.total_usd_micros, None);
    }

    /// Spec section 3: a `TotalOnly` write (the proxy ledger's shape) is
    /// never list-priced, an empty one included. A ledger session shows its
    /// charge, not an API-price figure, and pricing only its write-free
    /// calls would give some ledger sessions a figure and not others.
    #[test]
    fn an_empty_total_only_claude_write_is_still_never_list_priced() {
        let mut empty = priced_claude_turn(0, 0);
        empty.cache_write = Some(CacheWrite::TotalOnly(0));
        let price = price_session(
            &claude_catalog(&["test-model-a"]),
            PricingAccounting::ClaudeCode,
            &labels(&["test-model-a"]),
            &[empty],
        );
        let model = &price.by_model["test-model-a"];
        assert_eq!(model.priced_turns, 0);
        assert_eq!(
            model.unpriced_reasons[&UnpricedReason::CacheLifetimeUnknown],
            1
        );
        assert_eq!(model.usd_micros, None);
        assert_eq!(price.total_usd_micros, None);
    }

    #[test]
    fn a_codex_cache_write_is_priced_at_its_write_rate_never_on_top() {
        let codex = catalog(vec![table(
            1,
            vec![entry(
                PricingAccounting::Codex,
                "test-model-a",
                0,
                None,
                synthetic_rate,
            )],
        )]);
        let with_write = |write: u32| {
            price_session(
                &codex,
                PricingAccounting::Codex,
                &labels(&["test-model-a"]),
                &[turn(
                    0,
                    Some(10),
                    Some(0),
                    [Some(MILLION), Some(MILLION), Some(MILLION)],
                    Some(CacheWrite::TotalOnly(write)),
                )],
            )
        };
        // The stored uncached count already excludes the write, so the
        // write adds its own tokens at its own rate and nothing else moves:
        // 1M uncached + 1M cached + 1M output = 1+2+7, and 1M written at 11.
        assert_eq!(with_write(0).total_usd_micros, Some(10_000_000));
        assert_eq!(with_write(MILLION).total_usd_micros, Some(21_000_000));
        assert_eq!(with_write(5).total_usd_micros, Some(10_000_055));

        // A Claude-shaped split write has no OpenAI lifetime.
        let split = price_session(
            &codex,
            PricingAccounting::Codex,
            &labels(&["test-model-a"]),
            &[turn(
                0,
                Some(10),
                Some(0),
                [Some(MILLION), Some(MILLION), Some(MILLION)],
                Some(CacheWrite::Split { m5: 1, h1: 0 }),
            )],
        );
        assert_eq!(
            split.by_model["test-model-a"].unpriced_reasons[&UnpricedReason::CacheLifetimeUnknown],
            1
        );
        assert_eq!(split.total_usd_micros, None);
    }

    /// The one mapping from a Codex `token_count` event to stored counters:
    /// the write comes out of uncached input, so it is priced once.
    #[test]
    fn a_codex_event_maps_to_stored_counters_with_the_write_taken_out() {
        assert_eq!(
            codex_turn_usage(1_000, 600, 300, 50),
            Some(KnownTurnUsage {
                uncached_input: 100,
                cache_read: 600,
                cache_write: CacheWrite::TotalOnly(300),
                output: 50,
            })
        );
        let usage = codex_turn_usage(1_000, 600, 300, 50).unwrap();
        assert_eq!(usage.context(), 1_000, "context is the event's input");
        // Reads and writes above the input are a shape this client does not
        // understand: unknown, never a wrapped or clamped count.
        assert_eq!(codex_turn_usage(1_000, 600, 401, 50), None);
        assert_eq!(codex_turn_usage(1_000, 1_001, 0, 50), None);
        assert_eq!(codex_turn_usage(u64::from(u32::MAX) + 1, 0, 0, 0), None);
        assert_eq!(codex_turn_usage(0, 0, 0, u64::from(u32::MAX) + 1), None);
        assert_eq!(
            codex_turn_usage(u64::from(u32::MAX), 0, 0, u64::from(u32::MAX))
                .map(|usage| usage.total()),
            Some(2 * u64::from(u32::MAX))
        );
    }

    #[test]
    fn one_unpriced_turn_leaves_the_total_unknown_and_other_models_priced() {
        let price = price_session(
            &claude_catalog(&["test-model-a"]),
            PricingAccounting::ClaudeCode,
            &labels(&["test-model-a", "test-model-b"]),
            &[
                priced_claude_turn(0, 0),
                priced_claude_turn(1, 0),
                priced_claude_turn(2, 1),
            ],
        );
        assert_eq!(price.by_model["test-model-a"].priced_turns, 2);
        assert_eq!(price.by_model["test-model-a"].usd_micros, Some(46_000_000));
        let b = &price.by_model["test-model-b"];
        assert_eq!(b.usd_micros, None);
        assert_eq!(b.unpriced_reasons[&UnpricedReason::MissingPrice], 1);
        assert_eq!(price.total_usd_micros, None, "never the priced part alone");
    }

    #[test]
    fn an_undated_turn_or_one_before_every_entry_is_unpriced() {
        let two_releases = catalog(vec![
            table(
                1,
                vec![entry(
                    PricingAccounting::ClaudeCode,
                    "test-model-a",
                    1_000,
                    Some(2_000),
                    synthetic_rate,
                )],
            ),
            table(
                2,
                vec![entry(
                    PricingAccounting::ClaudeCode,
                    "test-model-a",
                    2_000,
                    None,
                    |_| 9_000_000_000,
                )],
            ),
        ]);
        let mut undated = priced_claude_turn(0, 0);
        undated.at = None;
        let mut early = priced_claude_turn(1, 0);
        early.at = Some(at(500));
        let mut in_first = priced_claude_turn(2, 0);
        in_first.at = Some(at(1_500));
        let price = price_session(
            &two_releases,
            PricingAccounting::ClaudeCode,
            &labels(&["test-model-a"]),
            &[undated, early, in_first.clone()],
        );
        let model = &price.by_model["test-model-a"];
        assert_eq!(model.unpriced_reasons[&UnpricedReason::Undated], 1);
        assert_eq!(model.unpriced_reasons[&UnpricedReason::WindowUnresolved], 1);
        assert_eq!(model.priced_turns, 1);
        assert_eq!(model.unpriced_turns, 2);
        assert_eq!(model.usd_micros, None);
        assert_eq!(price.total_usd_micros, None);

        let dated_only = price_session(
            &two_releases,
            PricingAccounting::ClaudeCode,
            &labels(&["test-model-a"]),
            &[in_first],
        );
        assert_eq!(
            dated_only.total_usd_micros,
            Some(23_000_000),
            "a turn in the first window takes the first rate, not the newest"
        );
    }

    #[test]
    fn an_unlabelled_zero_usage_turn_is_skipped_not_unpriced() {
        let zero = |ordinal, label| {
            turn(
                ordinal,
                Some(10),
                label,
                [Some(0), Some(0), Some(0)],
                Some(CacheWrite::Split { m5: 0, h1: 0 }),
            )
        };
        let price = price_session(
            &claude_catalog(&["test-model-a"]),
            PricingAccounting::ClaudeCode,
            &labels(&["test-model-a", "<synthetic>"]),
            &[priced_claude_turn(0, 0), zero(1, None), zero(2, Some(1))],
        );
        assert_eq!(price.skipped_turns, 2);
        assert_eq!(price.unlabelled_turns, 0);
        assert_eq!(price.by_model.len(), 1);
        assert_eq!(price.total_usd_micros, Some(23_000_000));
    }

    #[test]
    fn an_unlabelled_turn_with_usage_counts_and_blocks_the_total() {
        let mut unlabelled = priced_claude_turn(1, 0);
        unlabelled.model_label_ix = None;
        let price = price_session(
            &claude_catalog(&["test-model-a"]),
            PricingAccounting::ClaudeCode,
            &labels(&["test-model-a"]),
            &[priced_claude_turn(0, 0), unlabelled],
        );
        assert_eq!(price.unlabelled_turns, 1);
        assert_eq!(price.skipped_turns, 0);
        assert_eq!(price.by_model["test-model-a"].usd_micros, Some(23_000_000));
        assert_eq!(price.total_usd_micros, None);
    }

    #[test]
    fn a_synthetic_label_with_usage_counts_and_blocks_the_total() {
        // Skipping is for the zero-usage shape only; tokens under a
        // `<synthetic>` label are real usage nothing can price.
        let synthetic_with_usage = priced_claude_turn(1, 1);
        let synthetic_unknown = turn(2, Some(10), Some(1), [Some(0), Some(0), None], None);
        let price = price_session(
            &claude_catalog(&["test-model-a"]),
            PricingAccounting::ClaudeCode,
            &labels(&["test-model-a", "<synthetic>"]),
            &[
                priced_claude_turn(0, 0),
                synthetic_with_usage,
                synthetic_unknown,
            ],
        );
        assert_eq!(price.skipped_turns, 0);
        assert_eq!(price.unlabelled_turns, 2);
        assert_eq!(price.by_model["test-model-a"].usd_micros, Some(23_000_000));
        assert_eq!(price.total_usd_micros, None, "never the priced part alone");
    }

    #[test]
    fn an_unlabelled_turn_with_unknown_usage_blocks_the_total() {
        let unknown = turn(1, Some(10), None, [Some(0), Some(0), None], None);
        let price = price_session(
            &claude_catalog(&["test-model-a"]),
            PricingAccounting::ClaudeCode,
            &labels(&["test-model-a"]),
            &[priced_claude_turn(0, 0), unknown],
        );
        assert_eq!(price.unlabelled_turns, 1);
        assert_eq!(price.skipped_turns, 0);
        assert_eq!(price.total_usd_micros, None);
    }

    #[test]
    fn the_total_rounds_once_over_exact_numerators() {
        let one_million_nanos = |_| 1_000_000;
        let catalog = catalog(vec![table(
            1,
            vec![
                entry(
                    PricingAccounting::ClaudeCode,
                    "test-model-a",
                    0,
                    None,
                    one_million_nanos,
                ),
                entry(
                    PricingAccounting::ClaudeCode,
                    "test-model-b",
                    0,
                    None,
                    one_million_nanos,
                ),
            ],
        )]);
        // 400 tokens at 1_000_000 nanos per million = 0.4 micro exactly.
        let small = |ordinal, label| {
            turn(
                ordinal,
                Some(10),
                Some(label),
                [Some(400), Some(0), Some(0)],
                Some(CacheWrite::Split { m5: 0, h1: 0 }),
            )
        };
        let price = price_session(
            &catalog,
            PricingAccounting::ClaudeCode,
            &labels(&["test-model-a", "test-model-b"]),
            &[small(0, 0), small(1, 1)],
        );
        // Two models, each 0.4 micro exactly: each model rounds to 0, total 0.8 rounds to 1.
        assert_eq!(price.by_model["test-model-a"].usd_micros, Some(0));
        assert_eq!(price.by_model["test-model-b"].usd_micros, Some(0));
        assert_eq!(price.total_usd_micros, Some(1));
    }

    #[test]
    fn maximal_counts_over_the_turn_cap_never_wrap() {
        let max = u32::MAX;
        let turns = (0..MAX_SERIES_TURNS)
            .map(|ordinal| {
                turn(
                    u32::try_from(ordinal).unwrap(),
                    Some(10),
                    Some(0),
                    [Some(max), Some(max), Some(max)],
                    Some(CacheWrite::Split { m5: max, h1: max }),
                )
            })
            .collect::<Vec<_>>();
        let price_at = |rate: u64| {
            price_session(
                &catalog(vec![table(
                    1,
                    vec![entry(
                        PricingAccounting::ClaudeCode,
                        "test-model-a",
                        0,
                        None,
                        move |_| rate,
                    )],
                )]),
                PricingAccounting::ClaudeCode,
                &labels(&["test-model-a"]),
                &turns,
            )
        };
        let turn_count = u64::try_from(MAX_SERIES_TURNS).unwrap();

        // 1 USD per million tokens: every token is one micro, summed exactly.
        let modest = price_at(1_000_000_000);
        let exact = turn_count * 5 * u64::from(max);
        assert_eq!(modest.by_model["test-model-a"].priced_turns, 8_192);
        assert_eq!(modest.by_model["test-model-a"].usd_micros, Some(exact));
        assert_eq!(modest.total_usd_micros, Some(exact));

        // Each turn fits in u64 micros at this rate; their sum does not.
        let rate = 400_000_000_000_000_000;
        let one_turn = u128::from(5 * u64::from(max)) * u128::from(rate) / 1_000_000_000;
        assert!(u64::try_from(one_turn).is_ok());
        assert!(u64::try_from(one_turn * u128::from(turn_count)).is_err());
        let summed_overflow = price_at(rate);
        let model = &summed_overflow.by_model["test-model-a"];
        assert_eq!(model.usd_micros, None);
        assert_eq!(model.priced_turns, 0);
        assert_eq!(model.unpriced_turns, 8_192);
        assert_eq!(model.unpriced_reasons[&UnpricedReason::Overflow], 8_192);
        assert_eq!(summed_overflow.total_usd_micros, None);

        // The largest rate overflows every single turn.
        let per_turn_overflow = price_at(u64::MAX);
        let model = &per_turn_overflow.by_model["test-model-a"];
        assert_eq!(model.usd_micros, None);
        assert_eq!(model.unpriced_reasons[&UnpricedReason::Overflow], 8_192);
        assert_eq!(per_turn_overflow.total_usd_micros, None);
    }

    #[test]
    fn an_empty_catalog_prices_nothing_and_says_why() {
        let price = price_session(
            &catalog(Vec::new()),
            PricingAccounting::ClaudeCode,
            &labels(&["test-model-a"]),
            &[
                priced_claude_turn(0, 0),
                priced_claude_turn(1, 0),
                priced_claude_turn(2, 0),
            ],
        );
        let model = &price.by_model["test-model-a"];
        assert_eq!(model.unpriced_reasons[&UnpricedReason::MissingPrice], 3);
        assert_eq!(model.unpriced_turns, 3);
        assert_eq!(model.usd_micros, None);
        assert_eq!(price.total_usd_micros, None);
        assert!(price.tables.is_empty());
    }

    #[test]
    fn tables_used_are_listed_once_in_order() {
        let first = table(
            1,
            vec![entry(
                PricingAccounting::ClaudeCode,
                "test-model-a",
                0,
                Some(1_000),
                synthetic_rate,
            )],
        );
        let second = table(
            2,
            vec![entry(
                PricingAccounting::ClaudeCode,
                "test-model-a",
                1_000,
                None,
                synthetic_rate,
            )],
        );
        let mut turns = Vec::new();
        for (ordinal, second) in [(0, 10), (1, 20), (2, 1_500), (3, 1_600)] {
            let mut priced = priced_claude_turn(ordinal, 0);
            priced.at = Some(at(second));
            turns.push(priced);
        }
        let price = price_session(
            &catalog(vec![second.clone(), first.clone()]),
            PricingAccounting::ClaudeCode,
            &labels(&["test-model-a"]),
            &turns,
        );
        let mut expected = vec![
            PricingTableRef {
                table_id: first.table_id.clone(),
                version: 1,
            },
            PricingTableRef {
                table_id: second.table_id.clone(),
                version: 2,
            },
        ];
        expected.sort();
        assert_eq!(price.tables.len(), 2);
        assert_eq!(price.tables, expected);
        assert_eq!(price.total_usd_micros, Some(4 * 23_000_000));
    }

    /// The shipped tables, end to end. Expected figures are worked by hand
    /// from the page rates in USD per million tokens, so they are micros
    /// per token-rate product over one million.
    #[test]
    fn a_shipped_rate_prices_a_known_turn_end_to_end() {
        use crate::insights::pricing_catalog::{
            SHIPPED_PRICING_TABLES, production_pricing_catalog,
        };

        let catalog = production_pricing_catalog();
        let effective_from = catalog.tables()[0].entries[0].effective_from;
        let dated =
            |ordinal: u32, offset: i64, counts: [u32; 3], write: CacheWrite| -> TurnRecord {
                let mut record = turn(ordinal, Some(0), Some(0), counts.map(Some), Some(write));
                record.at = Some(effective_from + chrono::Duration::seconds(offset));
                record
            };
        let price = |accounting, label: &str, turns: &[TurnRecord]| {
            price_session(&catalog, accounting, &labels(&[label]), turns)
        };

        // Opus 5.5: 1,000 x 4 + 10,000 x 0.20 + 2,000 x 5 + 3,000 x 8
        // + 500 x 20 = 50,000 micros.
        let claude = price(
            PricingAccounting::ClaudeCode,
            "claude-opus-5-5",
            &[dated(
                0,
                0,
                [1_000, 10_000, 500],
                CacheWrite::Split {
                    m5: 2_000,
                    h1: 3_000,
                },
            )],
        );
        assert_eq!(claude.total_usd_micros, Some(50_000));

        // GPT-6 Astra: 1,000 x 10 + 10,000 x 1 + 2,000 x 12.50 + 500 x 50
        // = 70,000 micros.
        let codex = price(
            PricingAccounting::Codex,
            "gpt-6-astra",
            &[dated(
                0,
                0,
                [1_000, 10_000, 500],
                CacheWrite::TotalOnly(2_000),
            )],
        );
        assert_eq!(codex.total_usd_micros, Some(70_000));

        let mut used = claude.tables.clone();
        used.extend(codex.tables.clone());
        assert_eq!(
            used.iter()
                .map(|table| (table.version, table.table_id.as_str()))
                .collect::<Vec<_>>(),
            SHIPPED_PRICING_TABLES.to_vec()
        );

        // One second before every entry: unpriced, never the shipped rate.
        let early = price(
            PricingAccounting::ClaudeCode,
            "claude-opus-5-5",
            &[dated(
                0,
                -1,
                [1_000, 10_000, 500],
                CacheWrite::Split { m5: 0, h1: 0 },
            )],
        );
        assert_eq!(
            early.by_model["claude-opus-5-5"].unpriced_reasons[&UnpricedReason::WindowUnresolved],
            1
        );
        assert_eq!(early.total_usd_micros, None);

        // Haiku 5.5 at 100,000 prompt tokens: 100,000 x 0.10 + 1,000 x 0.50
        // = 10,500. One more prompt token moves the whole request to the
        // higher prices: 100,001 x 0.50 + 1,000 x 2.50 = 52,500.5, rounded
        // half up once.
        let haiku = |uncached| {
            price(
                PricingAccounting::ClaudeCode,
                "claude-haiku-5-5",
                &[dated(
                    0,
                    0,
                    [uncached, 0, 1_000],
                    CacheWrite::Split { m5: 0, h1: 0 },
                )],
            )
            .total_usd_micros
        };
        assert_eq!(haiku(100_000), Some(10_500));
        assert_eq!(haiku(100_001), Some(52_501));

        // The page labels its tiers "<=272K" and ">272K" input tokens, so the
        // bands meet at 272,000: a prompt of 272,001 is already in the higher
        // band, and no prompt size falls between the two.
        let astra = |uncached| {
            price(
                PricingAccounting::Codex,
                "gpt-6-astra",
                &[dated(0, 0, [uncached, 0, 0], CacheWrite::TotalOnly(0))],
            )
            .total_usd_micros
        };
        assert_eq!(astra(272_000), Some(2_720_000));
        assert_eq!(astra(272_001), Some(5_440_020));
        assert_eq!(astra(275_000), Some(5_500_000));
        assert_eq!(astra(278_528), Some(5_570_560));
        assert_eq!(astra(278_529), Some(5_570_580));
    }
}
