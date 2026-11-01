//! Immutable local pricing-table catalog.
//!
//! The production catalog ships two list-price tables, one per source page,
//! embedded at build time: no runtime fetch. The owner checked every rate
//! in both against the live pages on 2026-10-26 (UTC) and found none
//! changed; that refetch is each table's `retrieved_at` and
//! `content_sha256`. This module performs no network access, persistence,
//! or provider inference.
//!
//! How each table was first taken, on 2026-10-21 (UTC), 15:16:38Z:
//!
//! - Anthropic (version 1): `source_url` is where the docs pricing page
//!   redirected; `content_sha256` hashes the raw bytes fetched there, which
//!   carry every rate. The table holds every model the page prices, with
//!   both cache-write lifetimes. The page names models by display name; each
//!   `model` id was confirmed on the same site's models overview (the
//!   current models) and model deprecations page (the older ones, which
//!   carry dated ids that `normalize_model_label` strips; Haiku 3.5 is
//!   `claude-3-5-haiku`, version first).
//! - OpenAI (version 2): `source_url` is where the API pricing page
//!   redirected; `content_sha256` hashes its raw bytes, which carry the
//!   standard-tier rates up to 272K input tokens. The rates above 272K are
//!   rendered by the page's own script, whose URL and sha256 are in
//!   `provenance.source_notes` (the provenance holds one hash field). The
//!   tier labels ("<=272K", ">272K input tokens") are in the page's
//!   selector script, named there too.
//!   Standard tier only (not batch, flex, fast or priority). A model whose
//!   row shows "-" (or has no column) for the cache write has no separate
//!   write charge, so its write is priced at its uncached-input rate; this
//!   covers gpt-5.5, gpt-5.4 and its mini and nano, gpt-5.3-codex and the
//!   older gpt-5 rows. A model with no cached-input rate on the page (the
//!   "-pro" models, the GPT-4 and o-series) has no entry: pricing one would
//!   need a rate the page does not publish.
//!
//! Reproducibility: both vendors rebuild their pages on every deploy, so the
//! page bytes, and the OpenAI script's file name, change between any two
//! fetches. A recorded hash therefore binds the fetch that was made, not
//! the price facts, and a later fetch will not match it. The price facts are
//! the table's entries, which `table_id` digests; the owner review checks
//! those against the live page (and the script named in `source_notes`),
//! not the hash against a refetch.
//!
//! Neither page dates its rates, so every `effective_from` is the start of
//! the 13-week Insights window before the first fetch (the Monday, 00:00
//! UTC), as `provenance.effective_from_basis` says; an older turn is
//! unpriced. The review refetch found the same rates, so it kept that start.
//! The one dated statement is the OpenAI page's footnote, "GPT-5.6 Sol's
//! promotional pricing is available at least through November 21, 2026":
//! both gpt-5.6-sol entries end at 2026-11-22T00:00:00Z (`effective_until`
//! is exclusive, so the 21st still prices), and a later turn is unpriced
//! until a refetch. The page does not say when the promotion began, so the
//! backdated `effective_from` may price some earlier turns at the promotional
//! rate; that cannot be settled from the source.
//!
//! Staleness: a table prices only turns observed within
//! [`PRICE_STALE_AFTER_DAYS`] of its `retrieved_at`; a later turn is
//! unpriced (`StalePrices`) rather than shown at an old price as current.
//! `SessionPrice::prices_as_of` carries the oldest fetch date of the tables
//! behind a figure, for the "prices as of" line.
//!
//! Prompt-size bands: Anthropic states its threshold in tokens (100,000).
//! OpenAI labels its tiers "<=272K input tokens" and ">272K input tokens"
//! (the labels sit in the pricing page's selector script) and gives no
//! token count, so K is read as 1,000: the lower band ends at 272,000 and
//! the higher one starts above it. The bands meet, so every prompt size is
//! priced. Reading K as 1,024 instead would put 272,001 to 278,528 tokens
//! in the lower band; the page's wording (">272K") does not support that.

use std::collections::BTreeSet;

use trace_commons_protocol::insights_pricing::{
    DeterministicCostEstimate, PricingError, PricingTable, PricingUsageInput,
    ValidatedPricingTable, calculate_estimated_cost_validated,
};

const MAX_CATALOG_TABLES: usize = 256;

/// How long after a table's `retrieved_at` its prices still price a turn.
/// List prices change, and the tables ship inside a release with no runtime
/// fetch, so a build older than this reads "Not priced" for new turns rather
/// than showing an old price as current. A turn observed after
/// `retrieved_at + PRICE_STALE_AFTER_DAYS` is unpriced by that table; a
/// release with a fresh fetch prices it again. An owner-set default.
pub const PRICE_STALE_AFTER_DAYS: i64 = 90;
const MAX_ID_BYTES: usize = 96;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PricingCatalogError {
    #[error("insights_pricing_catalog_invalid")]
    InvalidCatalog,
    #[error("insights_pricing_catalog_duplicate_table")]
    DuplicateTable,
    #[error("insights_pricing_catalog_duplicate_version")]
    DuplicateVersion,
    #[error("insights_pricing_input_invalid")]
    InvalidQuery,
    #[error("insights_pricing_missing_price")]
    MissingPrice,
    /// The newest applicable table matched but the arithmetic overflowed.
    #[error("insights_pricing_arithmetic_overflow")]
    ArithmeticOverflow,
    #[error("insights_pricing_window_unresolved")]
    UnresolvedWindow,
    /// Every table that prices the model was fetched more than
    /// [`PRICE_STALE_AFTER_DAYS`] before the observation.
    #[error("insights_pricing_prices_stale")]
    StalePrices,
    /// A saved estimate names a table this catalog release no longer carries,
    /// or carries under a different release version. Recomputation refuses
    /// rather than substituting another table's rates.
    #[error("insights_pricing_catalog_stored_table_unavailable")]
    StoredTableUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PricingCatalog {
    /// Validated once, in [`Self::from_tables`], and immutable after: no
    /// lookup repeats the validation.
    tables: Vec<ValidatedPricingTable>,
}

impl PricingCatalog {
    /// Build one Trace Commons reviewed catalog release sequence. `version`
    /// is global within this sequence, rather than a publisher/provider-local
    /// counter, so two table bodies cannot claim the same release version.
    pub fn from_tables(tables: Vec<PricingTable>) -> Result<Self, PricingCatalogError> {
        if tables.len() > MAX_CATALOG_TABLES {
            return Err(PricingCatalogError::InvalidCatalog);
        }
        let mut validated = Vec::with_capacity(tables.len());
        let mut ids = BTreeSet::new();
        let mut versions = BTreeSet::new();
        for table in tables {
            let table = table
                .into_validated()
                .map_err(|_| PricingCatalogError::InvalidCatalog)?;
            if !ids.insert(table.table_id.clone()) {
                return Err(PricingCatalogError::DuplicateTable);
            }
            if !versions.insert(table.version) {
                return Err(PricingCatalogError::DuplicateVersion);
            }
            validated.push(table);
        }
        validated.sort_by(|left, right| {
            left.version
                .cmp(&right.version)
                .then_with(|| left.table_id.cmp(&right.table_id))
        });
        Ok(Self { tables: validated })
    }

    pub fn tables(&self) -> &[ValidatedPricingTable] {
        &self.tables
    }

    pub fn table(&self, table_id: &str) -> Option<&ValidatedPricingTable> {
        self.tables.iter().find(|table| table.table_id == table_id)
    }

    /// Choose the newest immutable table containing one rate entry that
    /// covers both inclusive observation endpoints, the input's prompt size
    /// (for a model priced by request size) and every exact accounting
    /// category. The counts decide only the prompt-size band: a table whose
    /// entry matches but whose arithmetic overflows is still the one chosen,
    /// so an overflow is reported from it rather than falling back.
    ///
    /// This is the first-estimate path only. An estimate that was already
    /// saved must be recomputed with [`Self::recompute_saved`], which reprices
    /// through the table the estimate recorded. Routing a saved estimate here
    /// would silently move its number when a later release also covers the
    /// same historical window.
    pub fn select_newest_applicable(
        &self,
        input: &PricingUsageInput,
    ) -> Result<&PricingTable, PricingCatalogError> {
        self.resolve(input).map(|(table, _)| table.as_table())
    }

    /// [`Self::select_newest_applicable`] and the estimate from that table in
    /// one pass, so a caller does not price the same turn a second time. An
    /// overflow in the chosen table is `ArithmeticOverflow`, never a fall
    /// back to an older table.
    pub fn price_newest_applicable(
        &self,
        input: &PricingUsageInput,
    ) -> Result<(&PricingTable, DeterministicCostEstimate), PricingCatalogError> {
        let (table, estimate) = self.resolve(input)?;
        let estimate = estimate.map_err(|_| PricingCatalogError::ArithmeticOverflow)?;
        Ok((table.as_table(), estimate))
    }

    /// The chosen table and its result. The result is `Err` only for an
    /// arithmetic overflow, the one failure that still selects its table.
    fn resolve(
        &self,
        input: &PricingUsageInput,
    ) -> Result<
        (
            &ValidatedPricingTable,
            Result<DeterministicCostEstimate, PricingError>,
        ),
        PricingCatalogError,
    > {
        validate_query(input)?;
        let mut matching_identity = false;
        let mut stale_identity = false;
        // Newest first: the first table that applies is the answer. Tables
        // that do not name the model are skipped before any arithmetic.
        for table in self.tables.iter().rev() {
            if !table.entries.iter().any(|entry| {
                entry.provider == input.provider
                    && entry.model == input.model
                    && entry.accounting == input.accounting
            }) {
                continue;
            }
            if is_stale(table, input) {
                stale_identity = true;
                continue;
            }
            matching_identity = true;
            // Overflow is checked only after one entry matched, so it means
            // this table applies.
            match calculate_estimated_cost_validated(table, input) {
                Ok(estimate) => return Ok((table, Ok(estimate))),
                Err(PricingError::ArithmeticOverflow) => {
                    return Ok((table, Err(PricingError::ArithmeticOverflow)));
                }
                Err(PricingError::MissingPrice | PricingError::UnresolvedWindow) => {}
                Err(PricingError::InvalidInput) => return Err(PricingCatalogError::InvalidQuery),
                Err(PricingError::InvalidTable) => {
                    return Err(PricingCatalogError::InvalidCatalog);
                }
            }
        }
        Err(if matching_identity {
            PricingCatalogError::UnresolvedWindow
        } else if stale_identity {
            PricingCatalogError::StalePrices
        } else {
            PricingCatalogError::MissingPrice
        })
    }

    /// Recompute an already-saved estimate against the exact immutable table
    /// it recorded, so unchanged saved usage keeps its number across catalog
    /// releases. `table_id` is a digest over the whole table body, so a match
    /// is a match on the rates themselves; the release version must agree too.
    /// A table this release no longer carries is unavailable, never replaced.
    pub fn recompute_saved(
        &self,
        saved: &DeterministicCostEstimate,
        input: &PricingUsageInput,
    ) -> Result<DeterministicCostEstimate, PricingCatalogError> {
        validate_query(input)?;
        let table = self
            .table(&saved.table_id)
            .filter(|table| table.version == saved.table_version)
            .ok_or(PricingCatalogError::StoredTableUnavailable)?;
        calculate_estimated_cost_validated(table, input).map_err(|error| match error {
            PricingError::MissingPrice => PricingCatalogError::MissingPrice,
            PricingError::UnresolvedWindow => PricingCatalogError::UnresolvedWindow,
            PricingError::InvalidInput => PricingCatalogError::InvalidQuery,
            PricingError::InvalidTable | PricingError::ArithmeticOverflow => {
                PricingCatalogError::InvalidCatalog
            }
        })
    }
}

/// Each shipped table's release version and pinned `table_id`, in version
/// order. A change to any byte of a table's body moves its id.
pub const SHIPPED_PRICING_TABLES: &[(u32, &str)] = &[
    (
        1,
        "sha256:be28771d884edcf6a26a3a5147dcc05cc01a7001d24141be756676a43273d184",
    ),
    (
        2,
        "sha256:174b7e4fe9d1299f7f3f96e0a9335a44cbbfb19e629a6f3e44810868c77160fe",
    ),
];

/// The embedded tables: one per source page, no runtime fetch.
const SHIPPED_PRICING_TABLE_SOURCES: [&str; 2] = [
    include_str!("pricing_tables/anthropic.json"),
    include_str!("pricing_tables/openai.json"),
];

/// The shipped list-price catalog. A table that fails to parse or validate
/// is a test failure (`each_embedded_table_parses_and_validates`); should
/// one ever ship, the catalog is empty, so every turn is unpriced
/// (`MissingPrice`) and nothing reads $0. This crate has no logger, so the
/// fallback is silent.
pub fn production_pricing_catalog() -> PricingCatalog {
    SHIPPED_PRICING_TABLE_SOURCES
        .iter()
        .map(|source| serde_json::from_str::<PricingTable>(source).ok())
        .collect::<Option<Vec<_>>>()
        .and_then(|tables| PricingCatalog::from_tables(tables).ok())
        .filter(|catalog| {
            catalog
                .tables
                .iter()
                .map(|table| (table.version, table.table_id.as_str()))
                .eq(SHIPPED_PRICING_TABLES.iter().copied())
        })
        .unwrap_or(PricingCatalog { tables: Vec::new() })
}

/// The shipped catalog, parsed once per process: the daemon prices every
/// `insights_week` and `insights_benchmarks` request against it.
pub fn shared_pricing_catalog() -> &'static PricingCatalog {
    static CATALOG: std::sync::OnceLock<PricingCatalog> = std::sync::OnceLock::new();
    CATALOG.get_or_init(production_pricing_catalog)
}

/// The table was fetched too long before the observation to price it.
fn is_stale(table: &PricingTable, input: &PricingUsageInput) -> bool {
    table
        .provenance
        .retrieved_at
        .checked_add_signed(chrono::Duration::days(PRICE_STALE_AFTER_DAYS))
        .is_none_or(|limit| input.observed_until > limit)
}

fn validate_query(input: &PricingUsageInput) -> Result<(), PricingCatalogError> {
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
        return Err(PricingCatalogError::InvalidQuery);
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

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use trace_commons_protocol::insights_pricing::{
        BillableTokenCategory, BillableTokenCount, ModelPriceEntry, PRICING_TABLE_SCHEMA_VERSION,
        PricingAccounting, PricingCurrency, PricingTableProvenance, TokenRate,
    };

    use trace_commons_protocol::insights_pricing::calculate_estimated_cost;

    use super::*;
    use crate::insights::session_pricing::{normalize_model_label, pricing_provider};

    fn at(second: i64) -> chrono::DateTime<Utc> {
        Utc.timestamp_opt(second, 0).unwrap()
    }

    fn rates(rate: u64) -> Vec<TokenRate> {
        PricingAccounting::Codex
            .required_categories()
            .iter()
            .map(|category| TokenRate {
                category: *category,
                usd_nanos_per_million_tokens: rate,
            })
            .collect()
    }

    fn table(version: u32, from: i64, until: Option<i64>, rate: u64) -> PricingTable {
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
                source_notes: None,
            },
            entries: vec![ModelPriceEntry {
                provider: "synthetic-provider".to_owned(),
                model: "synthetic-model".to_owned(),
                accounting: PricingAccounting::Codex,
                effective_from: at(from),
                effective_until: until.map(at),
                prompt_tokens_above: None,
                prompt_tokens_at_most: None,
                rates: rates(rate),
            }],
        };
        table.table_id = table.canonical_table_id().unwrap();
        table
    }

    fn input(from: i64, until: i64, tokens: u64) -> PricingUsageInput {
        PricingUsageInput {
            provider: "synthetic-provider".to_owned(),
            model: "synthetic-model".to_owned(),
            accounting: PricingAccounting::Codex,
            observed_from: at(from),
            observed_until: at(until),
            counts: vec![
                BillableTokenCount {
                    category: BillableTokenCategory::UncachedInput,
                    tokens,
                },
                BillableTokenCount {
                    category: BillableTokenCategory::CachedInput,
                    tokens: 0,
                },
                BillableTokenCount {
                    category: BillableTokenCategory::CacheWriteThirtyMinuteInput,
                    tokens: 0,
                },
                BillableTokenCount {
                    category: BillableTokenCategory::Output,
                    tokens: 0,
                },
            ],
        }
    }

    #[test]
    fn the_production_catalog_carries_the_two_shipped_tables() {
        let catalog = production_pricing_catalog();
        assert_eq!(
            catalog
                .tables()
                .iter()
                .map(|table| (table.version, table.table_id.as_str()))
                .collect::<Vec<_>>(),
            SHIPPED_PRICING_TABLES.to_vec()
        );
        assert_eq!(
            catalog.select_newest_applicable(&input(0, 0, 1)),
            Err(PricingCatalogError::MissingPrice),
            "a synthetic model is never priced"
        );
    }

    #[test]
    fn every_shipped_table_names_its_source_page_and_fetch_date() {
        let publishers = production_pricing_catalog()
            .tables()
            .iter()
            .map(|table| table.provenance.publisher.clone())
            .collect::<Vec<_>>();
        assert_eq!(publishers, ["Anthropic", "OpenAI"]);
        for table in production_pricing_catalog().tables() {
            // The insights crate has no `url` dependency; match the fetched
            // origins (the last two are where the first and third redirect).
            assert!(
                [
                    "https://www.anthropic.com/",
                    "https://docs.anthropic.com/",
                    "https://openai.com/",
                    "https://platform.openai.com/",
                    "https://platform.claude.com/",
                    "https://developers.openai.com/",
                ]
                .iter()
                .any(|origin| table.provenance.source_url.starts_with(origin))
            );
            assert!(table.provenance.retrieved_at <= table.provenance.reviewed_at);
            assert!(table.published_at >= table.provenance.retrieved_at);
            assert!(table.provenance.effective_from_basis.is_some());
        }
    }

    /// The hashes in the provenance bind the bytes fetched, and both vendors'
    /// pages change their bytes on every build, so a later fetch never
    /// reproduces them. The notes say so, and name the script that carries
    /// the OpenAI rates above 272K with its own hash, so a reviewer checks
    /// the rates against the page, not the hash against nothing.
    #[test]
    fn every_shipped_table_explains_what_its_hash_does_and_does_not_bind() {
        let catalog = production_pricing_catalog();
        for table in catalog.tables() {
            let notes = table.provenance.source_notes.as_deref().unwrap();
            assert!(notes.contains("change on every"), "{notes}");
            assert!(notes.contains("not the price facts"), "{notes}");
            assert!(notes.contains("table_id"), "{notes}");
        }
        let openai = &catalog.tables()[1];
        let notes = openai.provenance.source_notes.as_deref().unwrap();
        let script = notes
            .split_whitespace()
            .find(|word| word.starts_with("https://developers.openai.com/_astro/pricing."))
            .expect("the pricing script URL");
        assert!(script.trim_end_matches(['.', ',', ';']).ends_with(".js"));
        let hash = notes
            .split(|c: char| !c.is_ascii_hexdigit())
            .find(|word| word.len() == 64)
            .expect("the script's sha256");
        assert!(hash.bytes().all(|b| !b.is_ascii_uppercase()));
    }

    /// Fails until the owner reviews the rates: the merge gate for them.
    #[test]
    fn every_shipped_table_is_owner_reviewed() {
        for table in production_pricing_catalog().tables() {
            assert_ne!(table.provenance.reviewed_by, "Pending owner review");
        }
    }

    #[test]
    fn every_entry_is_a_normalized_label_under_its_accountings_provider() {
        let tables = production_pricing_catalog();
        let entries = tables
            .tables()
            .iter()
            .flat_map(|table| &table.entries)
            .collect::<Vec<_>>();
        assert!(!entries.is_empty());
        for entry in entries {
            assert_eq!(
                normalize_model_label(&entry.model).as_deref(),
                Some(entry.model.as_str())
            );
            assert_eq!(entry.provider, pricing_provider(entry.accounting));
            // Only a rate the page itself ends carries an expiry.
            assert_eq!(
                entry.effective_until.is_some(),
                entry.model == "gpt-5.6-sol",
                "{}",
                entry.model
            );
        }
    }

    /// Each embedded file parses and validates on its own, so a bad table is
    /// named here rather than only emptying the catalog.
    #[test]
    fn each_embedded_table_parses_and_validates() {
        for source in SHIPPED_PRICING_TABLE_SOURCES {
            let table: PricingTable = serde_json::from_str(source).unwrap();
            // A rate or provenance edit moves the digest; the message names
            // the new id to pin in the file and in `SHIPPED_PRICING_TABLES`.
            assert_eq!(
                table.validate(),
                Ok(()),
                "table_id for version {} must be {}",
                table.version,
                table.canonical_table_id().unwrap()
            );
        }
    }

    #[test]
    fn constructor_canonicalizes_and_retains_the_release_sequence() {
        let newest = table(2, 10, None, 20);
        let oldest = table(1, 0, Some(10), 10);
        let catalog = PricingCatalog::from_tables(vec![newest.clone(), oldest.clone()]).unwrap();
        assert_eq!(
            catalog
                .tables()
                .iter()
                .map(|table| table.version)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(
            catalog
                .table(&oldest.table_id)
                .map(|table| table.as_table()),
            Some(&oldest)
        );
        assert_eq!(
            catalog
                .table(&newest.table_id)
                .map(|table| table.as_table()),
            Some(&newest)
        );
    }

    #[test]
    fn duplicate_content_identity_and_release_version_are_rejected() {
        let first = table(1, 0, None, 1);
        assert_eq!(
            PricingCatalog::from_tables(vec![first.clone(), first]),
            Err(PricingCatalogError::DuplicateTable)
        );

        let first = table(1, 0, Some(10), 1);
        let second = table(1, 10, None, 2);
        assert_eq!(
            PricingCatalog::from_tables(vec![first, second]),
            Err(PricingCatalogError::DuplicateVersion)
        );
    }

    #[test]
    fn malformed_table_fails_the_whole_catalog() {
        let valid = table(1, 0, None, 1);
        let mut malformed = table(2, 0, None, 2);
        malformed.table_id = "sha256:wrong".to_owned();
        assert_eq!(
            PricingCatalog::from_tables(vec![valid, malformed]),
            Err(PricingCatalogError::InvalidCatalog)
        );
    }

    /// A later release that also covers the same window must not move a saved
    /// number. Only a first estimate selects the newest applicable table.
    #[test]
    fn a_saved_estimate_reprices_through_its_stored_table_not_the_newest() {
        let first = table(1, 0, None, 4_000_000_000);
        let query = input(10, 20, 3);
        let v1 = PricingCatalog::from_tables(vec![first.clone()]).unwrap();
        let saved =
            calculate_estimated_cost(v1.select_newest_applicable(&query).unwrap(), &query).unwrap();
        assert_eq!(saved.table_id, first.table_id);
        assert_eq!(saved.table_version, 1);
        assert_eq!(saved.estimated_cost_usd_micros, 12);

        let second = table(2, 0, None, 9_000_000_000);
        let v2 = PricingCatalog::from_tables(vec![first.clone(), second.clone()]).unwrap();
        assert_eq!(
            v2.select_newest_applicable(&query).unwrap(),
            &second,
            "a fresh estimate takes the newest applicable release"
        );
        assert_eq!(
            calculate_estimated_cost(&second, &query)
                .unwrap()
                .estimated_cost_usd_micros,
            27,
            "the newer release genuinely prices this usage differently"
        );

        let recomputed = v2.recompute_saved(&saved, &query).unwrap();
        assert_eq!(recomputed, saved, "unchanged saved usage keeps its number");
        assert_eq!(recomputed.table_id, first.table_id);
        assert_eq!(recomputed.table_version, 1);
    }

    #[test]
    fn a_stored_table_this_release_dropped_or_renumbered_is_unavailable() {
        let first = table(1, 0, None, 4_000_000_000);
        let query = input(10, 20, 3);
        let saved = calculate_estimated_cost(&first, &query).unwrap();

        let without = PricingCatalog::from_tables(vec![table(2, 0, None, 9_000_000_000)]).unwrap();
        assert_eq!(
            without.recompute_saved(&saved, &query),
            Err(PricingCatalogError::StoredTableUnavailable)
        );

        let mut renumbered = saved.clone();
        renumbered.table_version = 2;
        let with = PricingCatalog::from_tables(vec![first]).unwrap();
        assert_eq!(
            with.recompute_saved(&renumbered, &query),
            Err(PricingCatalogError::StoredTableUnavailable)
        );
        assert_eq!(with.recompute_saved(&saved, &query).unwrap(), saved);
    }

    #[test]
    fn newest_applicable_table_wins_and_boundaries_remain_half_open() {
        let old = table(1, 0, None, 10);
        let new = table(2, 10, Some(20), 20);
        let catalog = PricingCatalog::from_tables(vec![old.clone(), new.clone()]).unwrap();
        assert_eq!(
            catalog.select_newest_applicable(&input(10, 19, 1)).unwrap(),
            &new
        );
        assert_eq!(
            catalog.select_newest_applicable(&input(20, 20, 1)).unwrap(),
            &old
        );

        let only_bounded = PricingCatalog::from_tables(vec![new]).unwrap();
        assert_eq!(
            only_bounded.select_newest_applicable(&input(10, 20, 1)),
            Err(PricingCatalogError::UnresolvedWindow)
        );
    }

    #[test]
    fn missing_identity_differs_from_an_unresolved_window() {
        let catalog = PricingCatalog::from_tables(vec![table(1, 10, Some(20), 1)]).unwrap();
        assert_eq!(
            catalog.select_newest_applicable(&input(0, 0, 1)),
            Err(PricingCatalogError::UnresolvedWindow)
        );
        let mut unknown = input(10, 10, 1);
        unknown.model = "unknown-model".to_owned();
        assert_eq!(
            catalog.select_newest_applicable(&unknown),
            Err(PricingCatalogError::MissingPrice)
        );
        unknown = input(10, 10, 1);
        unknown.provider = "unqualified-provider".to_owned();
        assert_eq!(
            catalog.select_newest_applicable(&unknown),
            Err(PricingCatalogError::MissingPrice)
        );
    }

    #[test]
    fn invalid_queries_never_search_or_infer() {
        let catalog = PricingCatalog::from_tables(vec![table(1, 0, None, 1)]).unwrap();
        let mut invalid = input(1, 0, 1);
        assert_eq!(
            catalog.select_newest_applicable(&invalid),
            Err(PricingCatalogError::InvalidQuery)
        );
        invalid = input(0, 0, 1);
        invalid.counts.swap(0, 1);
        assert_eq!(
            catalog.select_newest_applicable(&invalid),
            Err(PricingCatalogError::InvalidQuery)
        );
    }

    /// Selection reads the turn's own prompt size. A newer table without a
    /// band for a long prompt is not applicable to it, so the older table
    /// that prices that band is chosen instead of an unresolved newer one.
    #[test]
    fn selection_matches_the_prompt_band_not_a_zeroed_probe() {
        let banded = |version: u32, long: bool| {
            let mut table = table(version, 0, None, 1);
            table.entries[0].prompt_tokens_at_most = Some(100);
            if long {
                let mut long = table.entries[0].clone();
                long.prompt_tokens_at_most = None;
                long.prompt_tokens_above = Some(100);
                table.entries.push(long);
            }
            table.table_id = table.canonical_table_id().unwrap();
            table
        };
        let older = banded(1, true);
        let newer = banded(2, false);
        let catalog = PricingCatalog::from_tables(vec![older.clone(), newer.clone()]).unwrap();
        assert_eq!(
            catalog.select_newest_applicable(&input(0, 0, 100)),
            Ok(&newer)
        );
        assert_eq!(
            catalog.select_newest_applicable(&input(0, 0, 101)),
            Ok(&older)
        );
        assert_eq!(
            PricingCatalog::from_tables(vec![newer])
                .unwrap()
                .select_newest_applicable(&input(0, 0, 101)),
            Err(PricingCatalogError::UnresolvedWindow)
        );
    }

    #[test]
    fn newest_arithmetic_failure_does_not_fall_back_to_an_older_table() {
        let old = table(1, 0, None, 1);
        let new = table(2, 0, None, u64::MAX);
        let catalog = PricingCatalog::from_tables(vec![old, new.clone()]).unwrap();
        let usage = input(0, 0, u64::MAX);
        let selected = catalog.select_newest_applicable(&usage).unwrap();
        assert_eq!(selected, &new);
        assert_eq!(
            calculate_estimated_cost(selected, &usage),
            Err(PricingError::ArithmeticOverflow)
        );
    }

    /// A table prices a turn up to `PRICE_STALE_AFTER_DAYS` after its fetch,
    /// the last instant included, and not after.
    #[test]
    fn a_table_stops_pricing_turns_after_the_staleness_horizon() {
        let catalog = PricingCatalog::from_tables(vec![table(1, 0, None, 1)]).unwrap();
        let retrieved = 100;
        let horizon = retrieved + PRICE_STALE_AFTER_DAYS * 86_400;
        assert!(
            catalog
                .select_newest_applicable(&input(horizon, horizon, 1))
                .is_ok()
        );
        assert_eq!(
            catalog.select_newest_applicable(&input(horizon, horizon + 1, 1)),
            Err(PricingCatalogError::StalePrices)
        );
        // An unknown model is still missing, not stale.
        let mut unknown = input(horizon + 1, horizon + 1, 1);
        unknown.model = "unknown-model".to_owned();
        assert_eq!(
            catalog.select_newest_applicable(&unknown),
            Err(PricingCatalogError::MissingPrice)
        );
        // A newer, fresh table that prices the model wins over a stale one.
        let mut fresh = table(2, 0, None, 2);
        fresh.provenance.retrieved_at = at(horizon);
        fresh.provenance.reviewed_at = at(horizon);
        fresh.published_at = at(horizon);
        fresh.table_id = fresh.canonical_table_id().unwrap();
        let both = PricingCatalog::from_tables(vec![table(1, 0, None, 1), fresh.clone()]).unwrap();
        assert_eq!(
            both.select_newest_applicable(&input(horizon + 1, horizon + 1, 1)),
            Ok(&fresh)
        );
    }
}
