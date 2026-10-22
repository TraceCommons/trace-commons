// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Observe the measurement sets NEAR AI staging's model enclaves run under,
//! for the staging pin re-sign tool (`trace-commons-staging-pins observe`).
//! Staging builds only.
//!
//! A set is printed only from a quote that went through every check a
//! staging app makes before it trusts a model key, in the same order
//! (`routing::proof_attestor` in the contributor):
//!
//! 1. the gateway's ed25519 report for the model, for a nonce this process
//!    chose ([`AttestedKeyReportClient::fetch_ed25519_report_json`]);
//! 2. the JSON-level binding of every entry for the model to that nonce
//!    (`model_ed25519_keys`);
//! 3. each entry's quote verified against Intel collateral fetched for that
//!    quote, at the caller's clock;
//! 4. Intel's TCB verdict exactly [`REQUIRED_TCB_STATUS`];
//! 5. the nonce and key re-checked against the **verified** quotes
//!    (`quote_bound_keys`).
//!
//! Only then are the registers read, out of the verified quote structure.
//! One refused model refuses the whole run: printing the sets of the models
//! that passed would let an operator sign a pin set that silently omits one.
//!
//! Refusals are labels. No key, nonce, quote, measurement or URL reaches
//! one, so a refusal can be printed as it is. The tests are in
//! `tests/staging_pins_tool.rs`, against a mock gateway.

use trace_commons_attestation::measurements::{ExpectedMeasurements, MeasurementField};
use trace_commons_attestation::quote::{
    Collateral, QuoteVerifyError, REQUIRED_TCB_STATUS, VerifiedQuote,
};
use trace_commons_attestation::receipt::{
    AttestedKeyError, model_ed25519_keys, model_entry_quotes, quote_bound_keys,
};

use super::client::{AttestationClientError, AttestedKeyReportClient};
use super::key_drift::DrillNonceSource;

/// The registers a set names unless the operator asks for others: what
/// production pins today (`mrtd` and `rtmr0-3`). Whether staging should drop
/// `rtmr3` or add `mrconfigid` is staging spec section 10, item 6; changing
/// this constant, or passing `--registers`, is the whole change.
pub const DEFAULT_REGISTERS: [MeasurementField; 5] = [
    MeasurementField::Mrtd,
    MeasurementField::Rtmr0,
    MeasurementField::Rtmr1,
    MeasurementField::Rtmr2,
    MeasurementField::Rtmr3,
];

/// Why a run printed nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObserveRefusal {
    /// The model's position on the command line, from 1 (0 when the run
    /// itself was refused). A position rather than the model id keeps the
    /// refusal a label.
    pub model_position: usize,
    /// A stable label; never a key, nonce, quote, measurement or URL.
    pub label: String,
}

/// Parse a comma-separated register list (`mrtd,rtmr0,...`), refusing an
/// unknown name, a repeat or an empty list.
pub fn parse_registers(text: &str) -> Result<Vec<MeasurementField>, String> {
    let mut fields = Vec::new();
    for name in text.split(',').map(str::trim) {
        let lowered = name.to_ascii_lowercase();
        let field = MeasurementField::ALL
            .into_iter()
            .find(|field| field.as_str() == lowered)
            .ok_or_else(|| "registers_unknown".to_string())?;
        if fields.contains(&field) {
            return Err("registers_repeated".to_string());
        }
        fields.push(field);
    }
    Ok(fields)
}

/// Observe every model's measurement sets: the distinct sets, canonical,
/// across every enclave serving every model. Each model is asked with a
/// fresh nonce from `nonce_source`.
///
/// The sets come back in the order first seen: model order as given, then
/// quote order within a model. Not sorted, because `sign` pins the staging
/// ingest to the first set, which must be a set of the first model's
/// rather than whichever happens to sort lowest.
///
/// `now_unix` is the clock the collateral is evaluated against; the tool
/// passes wall-clock time, and only a test pins it.
///
/// `verify` is the quote verifier: the tool passes
/// [`trace_commons_attestation::quote::verify_quote`]. It is a parameter, as
/// in the contributor's `proof_attestor`, so a test can reach the checks
/// that follow verification (the TCB verdict, the nonce in the verified
/// quote), which no checked-in quote fails on its own.
pub async fn observe_measurement_sets(
    clients: &[&dyn AttestedKeyReportClient],
    nonce_source: &dyn DrillNonceSource,
    now_unix: u64,
    registers: &[MeasurementField],
    verify: &(dyn Fn(&[u8], &Collateral, u64) -> Result<VerifiedQuote, QuoteVerifyError> + Sync),
) -> Result<Vec<String>, ObserveRefusal> {
    let refused = |model_position: usize, label: String| ObserveRefusal {
        model_position,
        label,
    };
    if registers.is_empty() {
        return Err(refused(0, "registers_empty".to_string()));
    }
    if clients.is_empty() {
        return Err(refused(0, "no_models".to_string()));
    }
    let mut sets: Vec<String> = Vec::new();
    for (index, client) in clients.iter().enumerate() {
        let quotes = observe_model(*client, &nonce_source.nonce(), now_unix, verify)
            .await
            .map_err(|label| refused(index + 1, label))?;
        for quote in &quotes {
            let set = set_of(quote, registers).map_err(|label| refused(index + 1, label))?;
            if !sets.contains(&set) {
                sets.push(set);
            }
        }
    }
    Ok(sets)
}

/// Every verified quote for one model, or the label of the first check that
/// refused.
async fn observe_model(
    client: &dyn AttestedKeyReportClient,
    nonce: &str,
    now_unix: u64,
    verify: &(dyn Fn(&[u8], &Collateral, u64) -> Result<VerifiedQuote, QuoteVerifyError> + Sync),
) -> Result<Vec<VerifiedQuote>, String> {
    // A nonce that is not 32 bytes of hex binds nothing; refuse before the
    // network call.
    if nonce.len() != 64 || !nonce.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("nonce_malformed".to_string());
    }
    let report = client
        .fetch_ed25519_report_json(nonce)
        .await
        .map_err(|error| client_label(&error))?;
    let model = client.model();
    let claimed = model_ed25519_keys(&report, nonce, model)
        .map_err(|error| format!("report_{}", key_label(error)))?;
    let quotes = model_entry_quotes(&report, model)
        .map_err(|error| format!("report_{}", key_label(error)))?;

    let mut verified = Vec::with_capacity(quotes.len());
    for quote in &quotes {
        let collateral = client
            .fetch_collateral_for(quote)
            .await
            .map_err(|error| client_label(&error))?;
        let checked = verify(quote, &collateral, now_unix).map_err(|error| {
            match error {
                QuoteVerifyError::CollateralMalformed { .. } => "collateral_malformed",
                QuoteVerifyError::VerificationFailed { .. } => "quote_verification_failed",
                QuoteVerifyError::NotTdx => "quote_not_tdx",
            }
            .to_string()
        })?;
        if checked.tcb_status != REQUIRED_TCB_STATUS {
            return Err("quote_tcb_not_up_to_date".to_string());
        }
        verified.push(checked);
    }

    let report_data: Vec<&[u8]> = verified.iter().map(|q| q.report_data.as_slice()).collect();
    quote_bound_keys(&claimed, &report_data, nonce)
        .map_err(|error| format!("quote_{}", key_label(error)))?;
    Ok(verified)
}

/// The named registers of a verified quote, in the canonical pin syntax.
fn set_of(quote: &VerifiedQuote, registers: &[MeasurementField]) -> Result<String, String> {
    let raw = registers
        .iter()
        .map(|field| format!("{}={}", field.as_str(), field.read(quote)))
        .collect::<Vec<_>>()
        .join(",");
    match ExpectedMeasurements::from_env_value(Some(&raw)) {
        Ok(Some(set)) => Ok(set.to_pin_string()),
        _ => Err("quote_registers_unreadable".to_string()),
    }
}

/// A transport failure's label: the step and the condition, plus the
/// missing control's name when that is the cause. Never the message.
fn client_label(error: &AttestationClientError) -> String {
    match error.missing_control() {
        Some(control) => format!("{}_missing_control:{control}", error.step().as_str()),
        None => format!("{}_{}", error.step().as_str(), error.code()),
    }
}

fn key_label(error: AttestedKeyError) -> &'static str {
    match error {
        AttestedKeyError::Malformed => "shape",
        AttestedKeyError::NotEd25519 => "not_ed25519",
        AttestedKeyError::NonceMismatch => "nonce_mismatch",
        AttestedKeyError::ReportDataMismatch => "report_data_mismatch",
        AttestedKeyError::KeyMalformed => "key_malformed",
        AttestedKeyError::ModelNotAttested => "model_not_attested",
        AttestedKeyError::SignatureKindUnrecognised => "signature_kind_unrecognised",
    }
}
