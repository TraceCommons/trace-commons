//! Immutable local pricing-table catalog.
//!
//! The production catalog is empty until separately reviewed rate data lands.
//! This module performs no network access, persistence, or provider inference.

use std::collections::BTreeSet;

use trace_commons_protocol::insights_pricing::{
    BillableTokenCount, DeterministicCostEstimate, PricingError, PricingTable, PricingUsageInput,
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
    /// covers both inclusive observation endpoints and every exact accounting
    /// category. Caller-provided counts are not calculated here.
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
        let zero_input = PricingUsageInput {
            provider: input.provider.clone(),
            model: input.model.clone(),
            accounting: input.accounting,
            observed_from: input.observed_from,
            observed_until: input.observed_until,
            counts: input
                .counts
                .iter()
                .map(|count| BillableTokenCount {
                    category: count.category,
                    tokens: 0,
                })
                .collect(),
        };
        let mut matching_identity = false;
        let mut applicable = None;
        for table in &self.tables {
            matching_identity |= table.entries.iter().any(|entry| {
                entry.provider == input.provider
                    && entry.model == input.model
                    && entry.accounting == input.accounting
            });
            match calculate_estimated_cost(table, &zero_input) {
                Ok(_) => applicable = Some(table),
                Err(PricingError::MissingPrice | PricingError::UnresolvedWindow) => {}
                Err(PricingError::InvalidInput) => return Err(PricingCatalogError::InvalidQuery),
                Err(PricingError::InvalidTable | PricingError::ArithmeticOverflow) => {
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

pub fn production_pricing_catalog() -> PricingCatalog {
    PricingCatalog { tables: Vec::new() }
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
        BillableTokenCategory, ModelPriceEntry, PRICING_TABLE_SCHEMA_VERSION, PricingAccounting,
        PricingCurrency, PricingTableProvenance, TokenRate,
    };

    use super::*;

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
            },
            entries: vec![ModelPriceEntry {
                provider: "synthetic-provider".to_owned(),
                model: "synthetic-model".to_owned(),
                accounting: PricingAccounting::Codex,
                effective_from: at(from),
                effective_until: until.map(at),
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
                    category: BillableTokenCategory::Output,
                    tokens: 0,
                },
            ],
        }
    }

    #[test]
    fn production_catalog_is_validated_empty_and_contains_no_rates() {
        let catalog = production_pricing_catalog();
        assert!(catalog.tables().is_empty());
        assert_eq!(catalog.table("anything"), None);
        assert_eq!(
            catalog.select_newest_applicable(&input(0, 0, 1)),
            Err(PricingCatalogError::MissingPrice)
        );
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
