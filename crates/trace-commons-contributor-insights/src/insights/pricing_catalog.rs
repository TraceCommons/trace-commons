//! Immutable local pricing-table catalog.
//!
//! The production catalog ships two list-price tables, one per source page,
//! embedded at build time: no runtime fetch. Both await the owner's review
//! (`reviewed_by` reads "Pending owner review" until then). This module
//! performs no network access, persistence, or provider inference.
//!
//! How each table was taken, on 2026-10-20 (UTC):
//!
//! - Anthropic (version 1): `source_url` is where the docs pricing page
//!   redirected; `content_sha256` hashes the raw bytes fetched there, which
//!   carry every rate. The page names models by display name; each `model`
//!   id was confirmed on the same site's models overview and model IDs
//!   pages.
//! - OpenAI (version 2): `source_url` is where the API pricing page
//!   redirected; `content_sha256` hashes its raw bytes, which carry the
//!   standard-tier rates up to 272K input tokens. The rates above 272K are
//!   rendered by the page's own script,
//!   `https://developers.openai.com/_astro/pricing.N3LdDDY-.js`, fetched
//!   20:25:37Z, sha256
//!   `c0735a5bd831d52feb823dbf578a7c6d5d2f5ecd2ea0c598d36ee14460cce1bc`;
//!   the provenance holds one hash, so this one is recorded here.
//!   Standard tier only (not batch, flex, fast or priority). A model whose
//!   row shows no cache-write rate has no entry.
//!
//! Neither page dates its rates, so every `effective_from` is the start of
//! the 13-week Insights window before the fetch (the Monday, 00:00 UTC),
//! as `provenance.effective_from_basis` says; an older turn is unpriced.
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
    calculate_estimated_cost,
};

const MAX_CATALOG_TABLES: usize = 256;
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
    #[error("insights_pricing_window_unresolved")]
    UnresolvedWindow,
    /// A saved estimate names a table this catalog release no longer carries,
    /// or carries under a different release version. Recomputation refuses
    /// rather than substituting another table's rates.
    #[error("insights_pricing_catalog_stored_table_unavailable")]
    StoredTableUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PricingCatalog {
    tables: Vec<PricingTable>,
}

impl PricingCatalog {
    /// Build one Trace Commons reviewed catalog release sequence. `version`
    /// is global within this sequence, rather than a publisher/provider-local
    /// counter, so two table bodies cannot claim the same release version.
    pub fn from_tables(mut tables: Vec<PricingTable>) -> Result<Self, PricingCatalogError> {
        if tables.len() > MAX_CATALOG_TABLES {
            return Err(PricingCatalogError::InvalidCatalog);
        }
        let mut ids = BTreeSet::new();
        let mut versions = BTreeSet::new();
        for table in &tables {
            table
                .validate()
                .map_err(|_| PricingCatalogError::InvalidCatalog)?;
            if !ids.insert(table.table_id.as_str()) {
                return Err(PricingCatalogError::DuplicateTable);
            }
            if !versions.insert(table.version) {
                return Err(PricingCatalogError::DuplicateVersion);
            }
        }
        tables.sort_by(|left, right| {
            left.version
                .cmp(&right.version)
                .then_with(|| left.table_id.cmp(&right.table_id))
        });
        Ok(Self { tables })
    }

    pub fn tables(&self) -> &[PricingTable] {
        &self.tables
    }

    pub fn table(&self, table_id: &str) -> Option<&PricingTable> {
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
        validate_query(input)?;
        let mut matching_identity = false;
        let mut applicable = None;
        for table in &self.tables {
            matching_identity |= table.entries.iter().any(|entry| {
                entry.provider == input.provider
                    && entry.model == input.model
                    && entry.accounting == input.accounting
            });
            // Overflow is checked only after one entry matched, so it means
            // this table applies.
            match calculate_estimated_cost(table, input) {
                Ok(_) | Err(PricingError::ArithmeticOverflow) => applicable = Some(table),
                Err(PricingError::MissingPrice | PricingError::UnresolvedWindow) => {}
                Err(PricingError::InvalidInput) => return Err(PricingCatalogError::InvalidQuery),
                Err(PricingError::InvalidTable) => {
                    return Err(PricingCatalogError::InvalidCatalog);
                }
            }
        }
        applicable.ok_or(if matching_identity {
            PricingCatalogError::UnresolvedWindow
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
        calculate_estimated_cost(table, input).map_err(|error| match error {
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
        "sha256:cc9f2ae4075adad77dc148f24e3d7a4e52b1fbd1452b5be140090a2751e7eecd",
    ),
    (
        2,
        "sha256:18fb3575c45e8f10c09231aa47d134de3ab1010e7ff845dd61186dc9954df4d7",
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
            assert!(entry.effective_until.is_none());
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
        assert_eq!(catalog.table(&oldest.table_id), Some(&oldest));
        assert_eq!(catalog.table(&newest.table_id), Some(&newest));
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
}
