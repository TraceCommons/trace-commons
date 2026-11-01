// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The NEAR AI catalog check (spec section 5, check 2).
//!
//! A model is callable only while its live catalog entry
//! (`GET /v1/model/list`) has `metadata.verifiable: true` **and**
//! `metadata.attestationSupported: true`. The flags are necessary, not
//! sufficient: `moonshotai/kimi-k2.6` reports both while listing an
//! OpenRouter `base_url`. The production allowlist and the per-request
//! receipt are what make up the rest (see [`super::production`] and
//! [`super::receipt_check`]).
//!
//! The entry also carries the prices the limiter meters with, as
//! `{amount, scale: 9, currency: "USD"}` per token, which is integer
//! nano-USD. A price at another scale or in another currency makes the
//! entry unusable rather than converted: the limiter's units are fixed.
//!
//! This module parses and holds state. Fetching is the binary's job (slice
//! 5): the boot fetch, whose failure refuses the boot, and the hourly
//! refresh, which feeds [`CatalogState::record_refresh`] or
//! [`CatalogState::record_refresh_failure`]. A failed refresh keeps the last
//! good entries for up to [`CATALOG_STALE_AFTER_SECS`], then every model is
//! refused.

use std::collections::BTreeMap;

use serde::Deserialize;

/// How often the binary refreshes the catalog.
pub const CATALOG_REFRESH_INTERVAL_SECS: i64 = 60 * 60;
/// How long the last good catalog stays usable when refreshes fail.
pub const CATALOG_STALE_AFTER_SECS: i64 = 24 * 60 * 60;
/// The catalog's price scale: amounts are 10^-9 USD per token.
pub const CATALOG_PRICE_SCALE: u32 = 9;

/// Per-token prices in integer nano-USD.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenPrices {
    pub input_nanousd: u64,
    pub output_nanousd: u64,
}

/// What the check reads from one catalog entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    pub model_id: String,
    pub verifiable: bool,
    pub attestation_supported: bool,
    /// `None` when either price is missing, negative, at another scale, or
    /// not in USD: the entry is then not callable.
    pub prices: Option<TokenPrices>,
}

impl CatalogEntry {
    /// Both attestation flags, and prices the limiter can meter.
    pub fn is_callable(&self) -> Option<TokenPrices> {
        if self.verifiable && self.attestation_supported {
            self.prices
        } else {
            None
        }
    }
}

/// The catalog page did not parse. Carries nothing from the body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CatalogError {
    /// Not JSON, or no `models` array.
    #[error("judgement_catalog_malformed")]
    Malformed,
    /// Two entries name one model: which one is meant cannot be decided.
    #[error("judgement_catalog_duplicate_model")]
    DuplicateModel,
}

#[derive(Deserialize)]
struct RawPrice {
    amount: serde_json::Value,
    scale: serde_json::Value,
    currency: serde_json::Value,
}

/// Parse one `/v1/model/list` page into entries keyed by model id.
///
/// An entry that is not the expected shape is kept with both flags `false`
/// rather than failing the page, so an unrelated model's odd entry never
/// takes a pinned model down; the pinned model's own odd entry makes it
/// uncallable. An entry with no string `modelId` is skipped, since it names
/// no model. Two entries for one model refuse the page.
pub fn parse_catalog_page(body: &[u8]) -> Result<BTreeMap<String, CatalogEntry>, CatalogError> {
    let document: serde_json::Value =
        serde_json::from_slice(body).map_err(|_| CatalogError::Malformed)?;
    let models = document
        .get("models")
        .and_then(serde_json::Value::as_array)
        .ok_or(CatalogError::Malformed)?;
    let mut entries = BTreeMap::new();
    for model in models {
        let Some(model_id) = model.get("modelId").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let flag = |name: &str| {
            model
                .get("metadata")
                .and_then(|metadata| metadata.get(name))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
        };
        let prices = match (
            price_nanousd(model.get("inputCostPerToken")),
            price_nanousd(model.get("outputCostPerToken")),
        ) {
            (Some(input_nanousd), Some(output_nanousd)) => Some(TokenPrices {
                input_nanousd,
                output_nanousd,
            }),
            _ => None,
        };
        let entry = CatalogEntry {
            model_id: model_id.to_string(),
            verifiable: flag("verifiable"),
            attestation_supported: flag("attestationSupported"),
            prices,
        };
        if entries.insert(model_id.to_string(), entry).is_some() {
            return Err(CatalogError::DuplicateModel);
        }
    }
    Ok(entries)
}

/// A `{amount, scale: 9, currency: "USD"}` price as nano-USD: a
/// non-negative integer amount at exactly scale 9 in USD, or nothing.
fn price_nanousd(price: Option<&serde_json::Value>) -> Option<u64> {
    let raw: RawPrice = serde_json::from_value(price?.clone()).ok()?;
    if raw.scale.as_u64() != Some(u64::from(CATALOG_PRICE_SCALE)) || raw.currency != "USD" {
        return None;
    }
    raw.amount.as_u64()
}

/// The last good catalog, and when it was fetched.
#[derive(Debug, Default)]
pub struct CatalogState {
    last_good: Option<(i64, BTreeMap<String, CatalogEntry>)>,
}

/// Why a model is not callable now. All map to `judgement_model_unattested`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CatalogRefusal {
    /// No catalog has ever been fetched.
    #[error("judgement_catalog_absent")]
    Absent,
    /// The last good catalog is older than [`CATALOG_STALE_AFTER_SECS`].
    #[error("judgement_catalog_stale")]
    Stale,
    /// The model is not in the catalog.
    #[error("judgement_catalog_model_absent")]
    ModelAbsent,
    /// A flag is off, or the prices are unusable.
    #[error("judgement_catalog_model_unattested")]
    ModelUnattested,
}

impl CatalogState {
    /// Replace the catalog with a freshly fetched one.
    pub fn record_refresh(&mut self, now: i64, entries: BTreeMap<String, CatalogEntry>) {
        self.last_good = Some((now, entries));
    }

    /// A refresh failed: keep the last good catalog as it was.
    pub fn record_refresh_failure(&mut self, now: i64) {
        let _ = now;
    }

    /// The prices of `model_id` if it is callable at `now`.
    pub fn check(&self, model_id: &str, now: i64) -> Result<TokenPrices, CatalogRefusal> {
        let (fetched_at, entries) = self.last_good.as_ref().ok_or(CatalogRefusal::Absent)?;
        if now.saturating_sub(*fetched_at) > CATALOG_STALE_AFTER_SECS {
            return Err(CatalogRefusal::Stale);
        }
        entries
            .get(model_id)
            .ok_or(CatalogRefusal::ModelAbsent)?
            .is_callable()
            .ok_or(CatalogRefusal::ModelUnattested)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIVE: &str = include_str!("fixtures/near_ai_model_list_2026_10_20.json");
    const NOW: i64 = 1_792_540_800;

    fn live() -> BTreeMap<String, CatalogEntry> {
        parse_catalog_page(LIVE.as_bytes()).expect("the live page parses")
    }

    #[test]
    fn the_live_catalog_reads_flags_and_nano_usd_prices() {
        let entries = live();
        let qwen = &entries["Qwen/Qwen3.8-27B"];
        assert!(qwen.verifiable && qwen.attestation_supported);
        assert_eq!(
            qwen.prices,
            Some(TokenPrices {
                input_nanousd: 40,
                output_nanousd: 1350
            })
        );
        let jev = &entries["typesafe/jev-1.13"];
        assert!(!jev.verifiable && !jev.attestation_supported);
        // A zero price is a price.
        assert_eq!(
            jev.prices,
            Some(TokenPrices {
                input_nanousd: 42,
                output_nanousd: 0
            })
        );
    }

    #[test]
    fn jev_is_never_callable_and_the_flags_alone_pass_kimi() {
        let mut state = CatalogState::default();
        state.record_refresh(NOW, live());
        assert_eq!(
            state.check("typesafe/jev-1.13", NOW).unwrap_err(),
            CatalogRefusal::ModelUnattested
        );
        // Kimi passes the catalog flags. That is why the flags are not the
        // whole production check: the allowlist and the receipt are.
        assert!(state.check("moonshotai/kimi-k2.6", NOW).is_ok());
        assert!(state.check("Qwen/Qwen3.8-27B", NOW).is_ok());
        assert_eq!(
            state.check("nobody/none", NOW).unwrap_err(),
            CatalogRefusal::ModelAbsent
        );
    }

    #[test]
    fn a_flag_flip_refuses_until_it_flips_back() {
        let mut state = CatalogState::default();
        state.record_refresh(NOW, live());
        let mut flipped = live();
        flipped
            .get_mut("Qwen/Qwen3.8-27B")
            .expect("present")
            .attestation_supported = false;
        state.record_refresh(NOW + 3600, flipped);
        assert_eq!(
            state.check("Qwen/Qwen3.8-27B", NOW + 3600).unwrap_err(),
            CatalogRefusal::ModelUnattested
        );
        state.record_refresh(NOW + 7200, live());
        assert!(state.check("Qwen/Qwen3.8-27B", NOW + 7200).is_ok());
    }

    #[test]
    fn failed_refreshes_keep_the_last_good_catalog_for_a_day_then_refuse() {
        let mut state = CatalogState::default();
        assert_eq!(
            state.check("Qwen/Qwen3.8-27B", NOW).unwrap_err(),
            CatalogRefusal::Absent
        );
        state.record_refresh(NOW, live());
        state.record_refresh_failure(NOW + 3600);
        assert!(
            state
                .check("Qwen/Qwen3.8-27B", NOW + CATALOG_STALE_AFTER_SECS)
                .is_ok()
        );
        assert_eq!(
            state
                .check("Qwen/Qwen3.8-27B", NOW + CATALOG_STALE_AFTER_SECS + 1)
                .unwrap_err(),
            CatalogRefusal::Stale
        );
    }

    #[test]
    fn prices_at_another_scale_or_currency_make_the_entry_uncallable() {
        for price in [
            r#"{"amount": 40, "scale": 6, "currency": "USD"}"#,
            r#"{"amount": 40, "scale": 9, "currency": "EUR"}"#,
            r#"{"amount": -1, "scale": 9, "currency": "USD"}"#,
            r#"{"amount": 1.5, "scale": 9, "currency": "USD"}"#,
            r#"{"amount": "40", "scale": 9, "currency": "USD"}"#,
        ] {
            let page = format!(
                r#"{{"models":[{{"modelId":"m","inputCostPerToken":{price},
                "outputCostPerToken":{{"amount":1,"scale":9,"currency":"USD"}},
                "metadata":{{"verifiable":true,"attestationSupported":true}}}}]}}"#
            );
            let entries = parse_catalog_page(page.as_bytes()).expect("the page parses");
            assert_eq!(entries["m"].prices, None, "{price}");
            let mut state = CatalogState::default();
            state.record_refresh(NOW, entries);
            assert_eq!(
                state.check("m", NOW).unwrap_err(),
                CatalogRefusal::ModelUnattested
            );
        }
    }

    #[test]
    fn a_non_boolean_flag_is_false() {
        let page = r#"{"models":[{"modelId":"m",
            "inputCostPerToken":{"amount":1,"scale":9,"currency":"USD"},
            "outputCostPerToken":{"amount":1,"scale":9,"currency":"USD"},
            "metadata":{"verifiable":"true","attestationSupported":1}}]}"#;
        let entries = parse_catalog_page(page.as_bytes()).expect("parses");
        assert!(!entries["m"].verifiable);
        assert!(!entries["m"].attestation_supported);
    }

    #[test]
    fn a_malformed_or_ambiguous_page_is_refused() {
        assert_eq!(
            parse_catalog_page(b"not json").unwrap_err(),
            CatalogError::Malformed
        );
        assert_eq!(
            parse_catalog_page(br#"{"data":[]}"#).unwrap_err(),
            CatalogError::Malformed
        );
        assert_eq!(
            parse_catalog_page(br#"{"models":[{"modelId":"m"},{"modelId":"m"}]}"#).unwrap_err(),
            CatalogError::DuplicateModel
        );
    }
}
