// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The production check (spec section 5): which models a production build
//! may call, checked fail-closed at boot and on every call.
//!
//! A model is callable only if all of these hold:
//!
//! 1. its id is in [`PRODUCTION_MODEL_ALLOWLIST`], compiled in, so widening
//!    it is a code change and a new measurement;
//! 2. its live catalog entry has both attestation flags and usable prices
//!    ([`super::catalog`]);
//! 3. its per-model receipt signer pin is configured
//!    (`TRACE_COMMONS_JUDGEMENT_MODEL_KEY_PINS`, the witness's
//!    `model=key[,model=key...]` spelling and parser), and at boot every
//!    pinned key is among the keys the gateway report's DCAP-verified quotes
//!    bind ([`require_pins_attested`]);
//! 4. per request, its receipt verifies ([`super::receipt_check`]);
//! 5. it is not latched (below).
//!
//! # The rotation latch
//!
//! NEAR AI does not announce a new provider-TEE key in advance, so a
//! rotation shows up as a receipt from an unpinned signer, after the call was
//! made and billed. Without a latch every later request would make the same
//! billed calls and fail the same way until a redeploy. So the first receipt
//! for a model that fails as [`ReceiptRefusal::SignerUnpinned`],
//! [`ReceiptRefusal::NotProviderTee`] or [`ReceiptRefusal::WrongModel`], or a
//! verified usage above the reservation, latches that model
//! ([`ProductionCheck::latch`]). [`ProductionCheck::call_check`] then refuses
//! it before anything is reserved, so nothing more is sent or charged.
//!
//! The latch is in memory and clears only when the pins change: a boot
//! whose pins the live gateway report attests builds a fresh check. There
//! is no other way to clear it (a pin update without a release is open
//! decision S1).
//!
//! [`ReceiptRefusal::SignerUnpinned`]: super::receipt_check::ReceiptRefusal::SignerUnpinned
//! [`ReceiptRefusal::NotProviderTee`]: super::receipt_check::ReceiptRefusal::NotProviderTee
//! [`ReceiptRefusal::WrongModel`]: super::receipt_check::ReceiptRefusal::WrongModel
//!
//! # The allowlist is empty
//!
//! Owner decision 1 (2026-10-20): there is no interim production model.
//! Production waits for the TEE-hosted decision model on NEAR AI; until it
//! is in the catalog, returns a valid per-request `provider_tee` receipt
//! and passes this check, the allowlist is empty and a production service
//! refuses to start. Adding it is slice 9.
//!
//! # What this slice leaves to the binary
//!
//! Fetching the gateway report and DCAP-verifying its `model_entry_quotes`
//! is slice 5's boot sequence; [`require_pins_attested`] is the comparison
//! it ends in, over keys `quote_bound_keys` already verified. The catalog
//! fetch and its hourly refresh are slice 5 too.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::RwLock;

use trace_commons_protocol::insights_judgement::JudgementRefusal;

use super::catalog::{CatalogState, TokenPrices};
use crate::witness_service::inference::{ModelKeyPinMalformed, parse_model_key_pins};

/// The models a production build may call. Empty by owner decision 1; see
/// the module docs.
pub const PRODUCTION_MODEL_ALLOWLIST: &[&str] = &[];

/// Models that must never be on the production allowlist: development
/// placeholders and test doubles. `typesafe/jev-1.13` is a pass-through to
/// OpenRouter outside NEAR AI's TEE guarantees.
pub const DEVELOPMENT_ONLY_MODEL_IDS: &[&str] =
    &["typesafe/jev-1.13", "trace-commons/mock-judgement-model"];

/// Environment variable carrying the per-model receipt signer pins.
pub const MODEL_KEY_PINS_ENV: &str = "TRACE_COMMONS_JUDGEMENT_MODEL_KEY_PINS";

/// Why the boot check refused a model. Each refuses the boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProductionBootRefusal {
    #[error("judgement_model_not_allowlisted")]
    NotAllowlisted,
    #[error("judgement_model_catalog_refused")]
    CatalogRefused,
    #[error("judgement_model_key_pin_absent")]
    KeyPinAbsent,
    #[error("judgement_model_key_pin_unattested")]
    KeyPinUnattested,
}

/// Parse `TRACE_COMMONS_JUDGEMENT_MODEL_KEY_PINS` with the witness's parser.
pub fn parse_judgement_model_key_pins(
    spec: &str,
) -> Result<BTreeMap<String, Vec<String>>, ModelKeyPinMalformed> {
    parse_model_key_pins(spec)
}

/// Whether every key pinned for a model is among the keys the gateway
/// report's verified quotes bind. A pin the live report does not attest
/// refuses the boot. An empty pin set attests nothing and refuses.
pub fn require_pins_attested(
    pinned: &[String],
    quote_bound_keys: &[String],
) -> Result<(), ProductionBootRefusal> {
    let attested = !pinned.is_empty()
        && pinned.iter().all(|pin| {
            trace_commons_attestation::receipt::signer_is_attested_for_model(pin, quote_bound_keys)
        });
    if attested {
        Ok(())
    } else {
        Err(ProductionBootRefusal::KeyPinUnattested)
    }
}

/// The production check's state: the allowlist, the pins, and the catalog.
pub struct ProductionCheck {
    allowlist: Vec<String>,
    pins: BTreeMap<String, Vec<String>>,
    catalog: RwLock<CatalogState>,
    /// Models latched since boot. See the module docs.
    latched: RwLock<BTreeSet<String>>,
}

impl std::fmt::Debug for ProductionCheck {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProductionCheck")
            .field("allowlist", &self.allowlist)
            .field("pinned_models", &self.pins.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl ProductionCheck {
    /// The production check over [`PRODUCTION_MODEL_ALLOWLIST`].
    pub fn production(pins: BTreeMap<String, Vec<String>>) -> Self {
        Self::with_allowlist(PRODUCTION_MODEL_ALLOWLIST, pins)
    }

    /// A check over another allowlist: tests only, so the mock can be
    /// driven through the same check production uses.
    #[cfg(test)]
    pub(crate) fn for_tests(allowlist: &[&str], pins: BTreeMap<String, Vec<String>>) -> Self {
        Self::with_allowlist(allowlist, pins)
    }

    /// A development build's check over its own allowlist: the
    /// development models it compiles in. Never in a production build, so
    /// a development model cannot pass a production check.
    #[cfg(feature = "judgement-development-models")]
    pub fn development(allowlist: &[&str], pins: BTreeMap<String, Vec<String>>) -> Self {
        Self::with_allowlist(allowlist, pins)
    }

    fn with_allowlist(allowlist: &[&str], pins: BTreeMap<String, Vec<String>>) -> Self {
        Self {
            allowlist: allowlist.iter().map(|m| (*m).to_string()).collect(),
            pins,
            catalog: RwLock::new(CatalogState::default()),
            latched: RwLock::new(BTreeSet::new()),
        }
    }

    /// Stop calling `model_id` until the pins change. Returns whether this
    /// call set it, so the caller logs the latch once.
    pub fn latch(&self, model_id: &str) -> bool {
        self.latched
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(model_id.to_string())
    }

    /// Whether `model_id` is latched.
    pub fn is_latched(&self, model_id: &str) -> bool {
        self.latched
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .contains(model_id)
    }

    /// The catalog, for the binary's refresh loop.
    pub fn catalog(&self) -> &RwLock<CatalogState> {
        &self.catalog
    }

    /// The pinned receipt signers for `model_id`.
    pub fn model_pins(&self, model_id: &str) -> Option<&[String]> {
        self.pins.get(model_id).map(Vec::as_slice)
    }

    /// The boot check for the configured model: allowlisted, callable in the
    /// catalog, and pinned. The gateway attestation of the pins is
    /// [`require_pins_attested`], which the binary runs after this.
    pub fn boot_check(&self, model_id: &str, now: i64) -> Result<(), ProductionBootRefusal> {
        if !self.allowlist.iter().any(|allowed| allowed == model_id) {
            return Err(ProductionBootRefusal::NotAllowlisted);
        }
        self.catalog
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .check(model_id, now)
            .map_err(|_| ProductionBootRefusal::CatalogRefused)?;
        if self.model_pins(model_id).is_none_or(<[String]>::is_empty) {
            return Err(ProductionBootRefusal::KeyPinAbsent);
        }
        Ok(())
    }

    /// The per-call check: the model is allowlisted, pinned, not latched,
    /// and callable in the catalog now. Returns the prices to meter with.
    pub fn call_check(&self, model_id: &str, now: i64) -> Result<TokenPrices, JudgementRefusal> {
        let allowlisted = self.allowlist.iter().any(|allowed| allowed == model_id);
        let pinned = self
            .model_pins(model_id)
            .is_some_and(|pins| !pins.is_empty());
        if !allowlisted || !pinned || self.is_latched(model_id) {
            return Err(JudgementRefusal::ModelUnattested);
        }
        self.catalog
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .check(model_id, now)
            .map_err(|_| JudgementRefusal::ModelUnattested)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::judgement_service::catalog::parse_catalog_page;

    const LIVE: &str = include_str!("fixtures/near_ai_model_list_2026_10_20.json");
    const NOW: i64 = 1_792_540_800;
    const QWEN: &str = "Qwen/Qwen3.8-27B";
    const KEY: &str = "aba45f0b8f90869baab26db02e8b01354bb8f8730769c60650cb7a635da602d4";

    fn pins(model: &str) -> BTreeMap<String, Vec<String>> {
        parse_judgement_model_key_pins(&format!("{model}={KEY}")).expect("pins")
    }

    fn with_live_catalog(check: ProductionCheck) -> ProductionCheck {
        check
            .catalog()
            .write()
            .expect("lock")
            .record_refresh(NOW, parse_catalog_page(LIVE.as_bytes()).expect("parses"));
        check
    }

    #[test]
    fn the_production_allowlist_holds_no_development_model() {
        for id in DEVELOPMENT_ONLY_MODEL_IDS {
            assert!(
                !PRODUCTION_MODEL_ALLOWLIST.contains(id),
                "{id} must never be callable in production"
            );
        }
        assert!(
            DEVELOPMENT_ONLY_MODEL_IDS.contains(&crate::judgement_service::mock::MOCK_MODEL_ID)
        );
    }

    /// A development adapter's id is refused by a production check even
    /// when everything else about it passes: pinned, and in the catalog
    /// with both attestation flags and prices.
    #[test]
    fn a_production_check_refuses_every_development_model_that_otherwise_passes() {
        use crate::judgement_service::catalog::CatalogEntry;
        let passing = |id: &str| {
            let mut entries = BTreeMap::new();
            entries.insert(
                id.to_string(),
                CatalogEntry {
                    model_id: id.to_string(),
                    verifiable: true,
                    attestation_supported: true,
                    prices: Some(TokenPrices {
                        input_nanousd: 40,
                        output_nanousd: 1350,
                    }),
                },
            );
            entries
        };
        for id in DEVELOPMENT_ONLY_MODEL_IDS {
            let check = ProductionCheck::production(pins(id));
            check
                .catalog()
                .write()
                .expect("lock")
                .record_refresh(NOW, passing(id));
            // The same pins and entry pass a check whose allowlist holds the
            // id, so only the production allowlist refuses it.
            let allowed = ProductionCheck::for_tests(&[id], pins(id));
            allowed
                .catalog()
                .write()
                .expect("lock")
                .record_refresh(NOW, passing(id));
            allowed
                .call_check(id, NOW)
                .expect("passes without the allowlist");
            assert_eq!(
                check.boot_check(id, NOW).unwrap_err(),
                ProductionBootRefusal::NotAllowlisted,
                "{id}"
            );
            assert_eq!(
                check.call_check(id, NOW).unwrap_err(),
                JudgementRefusal::ModelUnattested,
                "{id}"
            );
        }
    }

    /// Owner decision 1: no interim production model, so a production
    /// service refuses to start whatever it is configured with. When slice 9
    /// adds the decision model, this test changes with it, deliberately.
    #[test]
    fn the_production_allowlist_is_empty_until_the_decision_model() {
        assert!(PRODUCTION_MODEL_ALLOWLIST.is_empty());
        let check = with_live_catalog(ProductionCheck::production(pins(QWEN)));
        assert_eq!(
            check.boot_check(QWEN, NOW).unwrap_err(),
            ProductionBootRefusal::NotAllowlisted
        );
        assert_eq!(
            check.call_check(QWEN, NOW).unwrap_err(),
            JudgementRefusal::ModelUnattested
        );
    }

    #[test]
    fn a_production_check_refuses_jev_even_if_allowlisted_by_mistake() {
        let jev = "typesafe/jev-1.13";
        let check = with_live_catalog(ProductionCheck::for_tests(&[jev], pins(jev)));
        assert_eq!(
            check.boot_check(jev, NOW).unwrap_err(),
            ProductionBootRefusal::CatalogRefused
        );
        assert_eq!(
            check.call_check(jev, NOW).unwrap_err(),
            JudgementRefusal::ModelUnattested
        );
    }

    #[test]
    fn an_allowlisted_attested_pinned_model_passes() {
        let check = with_live_catalog(ProductionCheck::for_tests(&[QWEN], pins(QWEN)));
        check.boot_check(QWEN, NOW).expect("passes");
        assert_eq!(
            check.call_check(QWEN, NOW).expect("callable"),
            TokenPrices {
                input_nanousd: 40,
                output_nanousd: 1350
            }
        );
        assert_eq!(check.model_pins(QWEN), Some(&[KEY.to_string()][..]));
    }

    #[test]
    fn an_unpinned_model_is_refused() {
        let check = with_live_catalog(ProductionCheck::for_tests(
            &[QWEN],
            pins("moonshotai/kimi-k2.6"),
        ));
        assert_eq!(
            check.boot_check(QWEN, NOW).unwrap_err(),
            ProductionBootRefusal::KeyPinAbsent
        );
        assert_eq!(
            check.call_check(QWEN, NOW).unwrap_err(),
            JudgementRefusal::ModelUnattested
        );
    }

    #[test]
    fn no_catalog_refuses_the_boot() {
        let check = ProductionCheck::for_tests(&[QWEN], pins(QWEN));
        assert_eq!(
            check.boot_check(QWEN, NOW).unwrap_err(),
            ProductionBootRefusal::CatalogRefused
        );
    }

    #[test]
    fn a_stale_catalog_refuses_calls() {
        let check = with_live_catalog(ProductionCheck::for_tests(&[QWEN], pins(QWEN)));
        assert_eq!(
            check
                .call_check(
                    QWEN,
                    NOW + crate::judgement_service::catalog::CATALOG_STALE_AFTER_SECS + 1
                )
                .unwrap_err(),
            JudgementRefusal::ModelUnattested
        );
    }

    #[test]
    fn a_latched_model_is_refused_until_a_fresh_check_is_built() {
        let check = with_live_catalog(ProductionCheck::for_tests(&[QWEN], pins(QWEN)));
        assert!(!check.is_latched(QWEN));
        assert!(check.latch(QWEN));
        // Latching twice reports it was already set.
        assert!(!check.latch(QWEN));
        assert_eq!(
            check.call_check(QWEN, NOW).unwrap_err(),
            JudgementRefusal::ModelUnattested
        );
        // Another model is unaffected.
        assert!(!check.is_latched("moonshotai/kimi-k2.6"));
        // A boot with pins builds a fresh, unlatched check.
        let rebooted = with_live_catalog(ProductionCheck::for_tests(&[QWEN], pins(QWEN)));
        rebooted.call_check(QWEN, NOW).expect("callable");
    }

    #[test]
    fn every_pin_must_be_quote_bound() {
        let key_b = "cb".repeat(32);
        let pinned = vec![KEY.to_string(), key_b.clone()];
        assert!(require_pins_attested(&pinned, &[key_b.clone(), KEY.to_string()]).is_ok());
        // Case does not matter; the attestation crate renders lowercase.
        assert!(require_pins_attested(&[KEY.to_uppercase()], &[KEY.to_string()]).is_ok());
        assert_eq!(
            require_pins_attested(&pinned, &[KEY.to_string()]).unwrap_err(),
            ProductionBootRefusal::KeyPinUnattested
        );
        assert_eq!(
            require_pins_attested(&[], &[KEY.to_string()]).unwrap_err(),
            ProductionBootRefusal::KeyPinUnattested
        );
        assert_eq!(
            require_pins_attested(&pinned, &[]).unwrap_err(),
            ProductionBootRefusal::KeyPinUnattested
        );
    }
}
