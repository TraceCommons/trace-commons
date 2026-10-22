// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Parsing and nonce-binding verification for NEAR AI's Intel TDX attestation
//! report.
//!
//! NEAR AI's inference endpoint runs in a TEE and exposes a report over
//! `GET /v1/attestation/report?nonce=<hex>`. The report echoes the requested
//! nonce back as plain JSON (`request_nonce`), which by itself proves
//! nothing -- an attacker can replay an old signed quote and relabel the
//! echo. What makes the report fresh rather than replayable is that the
//! nonce is bound *inside* the signed `intel_quote` itself (as TDX report
//! data), so [`AttestationReport::quote_binds_nonce`] is what this module
//! exists to check.
//!
//! This module only parses the report and checks nonce binding. Verifying the
//! quote's signature chain against Intel collateral lives in [`quote`], and
//! pinning the image measurements it carries lives in [`measurements`].

pub mod client;
pub mod drill;
pub mod key_drift;
pub mod measurements;
/// What the staging pin re-sign tool observes: NEAR AI staging's verified
/// enclave measurements. Staging builds only.
#[cfg(feature = "near-ai-staging")]
pub mod pin_observe;
pub mod signer_resolver;

/// Receipt and quote verification moved to the permissive
/// `trace-commons-attestation` crate so that client-side code can verify an
/// attestation before sending raw bytes. Re-exported here so existing
/// `crate::near_attestation::{receipt, quote}` paths keep working.
pub use trace_commons_attestation::{quote, receipt};

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;

/// A NEAR AI attestation report for one model enclave, as returned by
/// `GET /v1/attestation/report?nonce=<hex>`.
///
/// Two response shapes carry one of these. A per-model completions host
/// answers with the report itself at the top level (the *single* shape). The
/// gateway -- the only attestation host NEAR AI staging has -- answers with
/// its own quote under `gateway_attestation` and one of these per model under
/// `model_attestations` (the *gateway* shape).
/// [`AttestationReport::from_json_for_model`] reads both and, for the gateway
/// shape, selects the requested model's entry; the gateway's own quote is
/// never read.
///
/// `deny_unknown_fields` is deliberately off: the live service sends
/// substantially more than we model here (e.g. `nvidia_payload`,
/// `all_attestations`, `ohttp_attestation`) and will add more over time.
/// Only fields this crate actually uses are modeled.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AttestationReport {
    pub model_name: String,
    pub signing_address: String,
    pub signing_algo: String,
    pub signing_public_key: String,
    pub request_nonce: String,
    pub intel_quote: String,
    pub info: AttestationInfo,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AttestationInfo {
    pub compose_hash: String,
    pub os_image_hash: String,
    pub mr_aggregated: String,
    pub instance_id: String,
    pub app_id: String,
    /// The registers the report claims for itself, in unsigned JSON.
    ///
    /// Optional because a gateway-shape `model_attestations` entry carries no
    /// `tcb_info`. Absent means the report makes no claim to compare -- it is
    /// never read as agreement with anything, and nothing gates on it either
    /// way: the registers that are checked come out of the verified quote.
    #[serde(default)]
    pub tcb_info: Option<TcbInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TcbInfo {
    pub mrtd: String,
    pub rtmr0: String,
    pub rtmr1: String,
    pub rtmr2: String,
    pub rtmr3: String,
}

/// The measurement set as the report *claims* it in unsigned JSON.
///
/// The name carries the warning because a doc comment is not a guard: these
/// values are the server's own assertion about itself, copied out of
/// `info.tcb_info`, and nothing has checked them against the signed quote.
/// Pinning against them would verify nothing at all. Measurement pinning must
/// consume [`quote::VerifiedQuote`], whose registers are read out of the
/// verified quote structure.
///
/// This exists so that the JSON claim can be *compared* with the verified
/// values -- a mismatch is worth reporting -- and for nothing else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedJsonMeasurements {
    pub mrtd: String,
    pub rtmr0: String,
    pub rtmr1: String,
    pub rtmr2: String,
    pub rtmr3: String,
    pub compose_hash: String,
    pub os_image_hash: String,
    pub mr_aggregated: String,
}

/// Why a report body did not yield the requested model's report.
///
/// Every variant has a stable [`label`](Self::label) for evidence. None
/// carries the body, a quote, an address or a URL.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReportShapeError {
    /// The body is not JSON, or not a report in either shape.
    #[error("attestation report is not in a recognised shape")]
    Malformed {
        /// A truncated SHA-256 of the parse error, safe to log.
        detail_hash: String,
    },
    /// A gateway-shape report with no entry whose `model_name` is exactly the
    /// requested model.
    #[error("the gateway report has no attestation for the requested model")]
    ModelNotAttested,
    /// A gateway-shape report with more than one entry for the requested
    /// model. Picking one would let the order of a JSON array decide which
    /// enclave is attested.
    #[error("the gateway report attests the requested model more than once")]
    ModelAttestedMoreThanOnce,
    /// The requested model's entry is not an ECDSA attestation, so its
    /// report data does not carry a raw signing address.
    #[error("the requested model's attestation is not ECDSA")]
    ModelEntryNotEcdsa,
    /// `gateway_attestation` with no `model_attestations`: a report about the
    /// gateway, which attests no model.
    #[error("the report attests the gateway and no model")]
    GatewayOnly,
    /// A top-level single report *and* `model_attestations`. Which one the
    /// endpoint meant is not something to guess.
    #[error("the report carries both the single and the gateway shape")]
    BothShapes,
}

impl ReportShapeError {
    /// A stable label for evidence. The condition, never the message.
    pub fn label(&self) -> &'static str {
        match self {
            // The label the drill has always reported for an unparseable
            // report, kept so existing evidence reads the same.
            Self::Malformed { .. } => "malformed_response",
            Self::ModelNotAttested => "model_not_attested",
            Self::ModelAttestedMoreThanOnce => "model_attested_more_than_once",
            Self::ModelEntryNotEcdsa => "model_entry_not_ecdsa",
            Self::GatewayOnly => "gateway_report_without_model_attestations",
            Self::BothShapes => "report_carries_both_shapes",
        }
    }
}

fn malformed(error: impl std::fmt::Display) -> ReportShapeError {
    use sha2::Digest;
    ReportShapeError::Malformed {
        detail_hash: hex::encode(&sha2::Sha256::digest(error.to_string().as_bytes())[..8]),
    }
}

impl AttestationReport {
    /// Parse a report from its JSON body, in the single shape only.
    pub fn from_json(body: &str) -> Result<Self> {
        serde_json::from_str(body).context("parsing NEAR AI attestation report JSON")
    }

    /// Parse the report for `model` out of a body in either shape.
    ///
    /// - **Gateway shape** (`model_attestations` present): exactly one entry
    ///   whose `model_name` equals `model` byte for byte is selected and
    ///   parsed; zero or several refuse. Nothing is ever read from
    ///   `gateway_attestation` -- the gateway's quote attests the gateway, not
    ///   the model, and every check downstream (collateral, nonce offset,
    ///   signer binding, measurement pins) must run on the model's quote.
    /// - **Single shape** (no `model_attestations`): parsed exactly as
    ///   [`Self::from_json`] parses it, and `model` is not consulted. That is
    ///   how a per-model completions host has always been read, and this
    ///   function does not change it.
    pub fn from_json_for_model(body: &str, model: &str) -> Result<Self, ReportShapeError> {
        let document: serde_json::Value = serde_json::from_str(body).map_err(malformed)?;
        let Some(object) = document.as_object() else {
            return Err(malformed("report is not a JSON object"));
        };
        let Some(entries) = object.get("model_attestations") else {
            if object.contains_key("gateway_attestation") && !object.contains_key("intel_quote") {
                return Err(ReportShapeError::GatewayOnly);
            }
            // From the original bytes, not from `document`: this is the
            // exact call `from_json` makes, so the single shape is parsed
            // the way it always was, down to duplicate-key handling.
            return serde_json::from_str(body).map_err(malformed);
        };
        if object.contains_key("intel_quote") {
            return Err(ReportShapeError::BothShapes);
        }
        let Some(entries) = entries.as_array() else {
            return Err(malformed("model_attestations is not an array"));
        };
        let mut matching = entries.iter().filter(|entry| {
            entry.get("model_name").and_then(serde_json::Value::as_str) == Some(model)
        });
        let entry = match (matching.next(), matching.next()) {
            (None, _) => return Err(ReportShapeError::ModelNotAttested),
            (Some(_), Some(_)) => return Err(ReportShapeError::ModelAttestedMoreThanOnce),
            (Some(entry), None) => entry,
        };
        if entry
            .get("signing_algo")
            .and_then(serde_json::Value::as_str)
            != Some("ecdsa")
        {
            return Err(ReportShapeError::ModelEntryNotEcdsa);
        }
        serde_json::from_value(entry.clone()).map_err(malformed)
    }

    /// Decode `intel_quote` from hex into raw quote bytes.
    pub fn quote_bytes(&self) -> Result<Vec<u8>> {
        hex::decode(&self.intel_quote).context("decoding intel_quote as hex")
    }

    /// The measurement set this report claims for itself, unverified.
    ///
    /// See [`UnverifiedJsonMeasurements`] before using this for anything.
    ///
    /// `None` when the report carries no `info.tcb_info` (a gateway-shape
    /// model entry): there is then no claim to compare, which is not the same
    /// as a claim that agrees.
    pub fn unverified_json_measurements(&self) -> Option<UnverifiedJsonMeasurements> {
        let tcb_info = self.info.tcb_info.as_ref()?;
        Some(UnverifiedJsonMeasurements {
            mrtd: tcb_info.mrtd.clone(),
            rtmr0: tcb_info.rtmr0.clone(),
            rtmr1: tcb_info.rtmr1.clone(),
            rtmr2: tcb_info.rtmr2.clone(),
            rtmr3: tcb_info.rtmr3.clone(),
            compose_hash: self.info.compose_hash.clone(),
            os_image_hash: self.info.os_image_hash.clone(),
            mr_aggregated: self.info.mr_aggregated.clone(),
        })
    }

    /// Whether `nonce` (64 hex characters) is bound inside the signed quote.
    ///
    /// This decodes both the quote and the nonce to raw bytes and searches
    /// for the nonce bytes as a contiguous subsequence of the quote bytes.
    /// The search is deliberately over decoded bytes, not hex substrings: a
    /// hex-substring search can also match on an unlucky nibble alignment
    /// that does not correspond to the nonce actually being present as
    /// bytes, which would make this check meaningless.
    ///
    /// A `nonce` that is not exactly 64 hex characters is a caller error and
    /// returns `Err`, not `Ok(false)` -- `Ok(false)` would tell a caller
    /// "not attested" when the truth is "you asked wrong".
    pub fn quote_binds_nonce(&self, nonce: &str) -> Result<bool> {
        if nonce.len() != 64 {
            bail!(
                "nonce must be exactly 64 hex characters, got {} characters",
                nonce.len()
            );
        }
        let nonce_bytes = hex::decode(nonce).map_err(|e| anyhow!("nonce is not valid hex: {e}"))?;
        let quote_bytes = self.quote_bytes()?;
        Ok(contains_subsequence(&quote_bytes, &nonce_bytes))
    }
}

fn contains_subsequence(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || needle.len() > haystack.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!(
        "../../../trace-commons-attestation/tests/fixtures/near_ai_attestation_report.json"
    );

    #[test]
    fn parses_a_real_report() {
        let r = AttestationReport::from_json(FIXTURE).expect("fixture parses");
        assert_eq!(r.signing_algo, "ecdsa");
        assert!(r.signing_address.starts_with("0x"));
        assert!(!r.intel_quote.is_empty());
    }

    #[test]
    fn quote_bytes_decode_from_hex() {
        let r = AttestationReport::from_json(FIXTURE).unwrap();
        let q = r.quote_bytes().expect("quote decodes");
        // The fixture's quote is 10,012 hex chars.
        assert_eq!(q.len(), 10_012 / 2);
    }

    #[test]
    fn the_fixtures_nonce_is_bound_into_the_quote() {
        // This is the property that makes the report fresh rather than replayable:
        // the nonce we asked for is inside the signed quote, not merely echoed
        // beside it. If this ever passes for a nonce we did not send, the check
        // that matters is gone.
        let r = AttestationReport::from_json(FIXTURE).unwrap();
        let nonce = fixture_nonce();
        assert!(r.quote_binds_nonce(&nonce).unwrap());
    }

    #[test]
    fn a_nonce_we_did_not_send_is_not_bound() {
        let r = AttestationReport::from_json(FIXTURE).unwrap();
        // Not "0".repeat(64): a TDX quote is full of zero-padding, and 32
        // consecutive zero bytes are trivially a byte-substring of this
        // fixture's quote (offset 112), which would make this "negative"
        // case pass for the wrong reason. "f".repeat(64) is verified absent.
        let other = "f".repeat(64);
        assert!(!r.quote_binds_nonce(&other).unwrap());
    }

    #[test]
    fn an_echoed_nonce_alone_does_not_satisfy_the_binding() {
        // Defends the exact confusion this check exists to prevent: request_nonce
        // is JSON beside the quote and proves nothing. Rewrite the echo to a value
        // that is NOT in the quote and the binding must still fail.
        let mut v: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
        let forged = "a".repeat(64);
        v["request_nonce"] = serde_json::json!(forged);
        let r = AttestationReport::from_json(&v.to_string()).unwrap();
        assert!(!r.quote_binds_nonce(&forged).unwrap());
    }

    #[test]
    fn a_malformed_nonce_is_an_error_not_false() {
        let r = AttestationReport::from_json(FIXTURE).unwrap();
        let short = "ab".repeat(16); // 32 hex chars
        assert!(r.quote_binds_nonce(&short).is_err());
    }

    fn fixture_nonce() -> String {
        let v: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
        v["_fixture_nonce"].as_str().unwrap().to_string()
    }

    // -- the gateway shape ---------------------------------------------------
    //
    // NEAR AI's gateway (staging's only attestation host) answers
    // `/attestation/report?model=..` with the gateway's own quote under
    // `gateway_attestation` and one entry per model under
    // `model_attestations`. These tests build that shape from the captured
    // single-enclave fixture: its quote becomes the model entry's quote, and
    // the gateway carries a synthetic quote that must never be read as the
    // model's.

    /// The model the fixture quote was captured for.
    const FIXTURE_MODEL: &str = "Qwen/Qwen3.6-35B-A3B-FP8";

    /// A synthetic gateway quote: valid hex, distinct from the fixture's, and
    /// containing the fixture nonce so a parser that read it would still bind
    /// the nonce -- which is what makes "the gateway quote was not used" a
    /// real assertion rather than a vacuous one.
    fn gateway_quote() -> String {
        format!("{}{}{}", "ab".repeat(64), fixture_nonce(), "cd".repeat(64))
    }

    const GATEWAY_ADDRESS: &str = "0x1111111111111111111111111111111111111111";

    /// One `model_attestations` entry built from the fixture, as the gateway
    /// serves it: no `tcb_info` under `info`, plus fields this crate ignores.
    fn model_entry(model: &str) -> serde_json::Value {
        let mut entry: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
        let object = entry.as_object_mut().unwrap();
        object.remove("_fixture_note");
        object.remove("_fixture_nonce");
        object.remove("ohttp_key_config");
        object.insert("model_name".into(), serde_json::json!(model));
        object.insert("nvidia_payload".into(), serde_json::json!("{}"));
        object.insert("event_log".into(), serde_json::json!("[]"));
        object.insert(
            "compose_manager_attestation".into(),
            serde_json::json!({ "actions": [], "quote": "00" }),
        );
        object["info"].as_object_mut().unwrap().remove("tcb_info");
        object["info"]
            .as_object_mut()
            .unwrap()
            .insert("app_name".into(), serde_json::json!("dstack-nvidia-0.5.12"));
        entry
    }

    fn gateway_report(entries: Vec<serde_json::Value>) -> String {
        let fixture: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
        serde_json::json!({
            "gateway_attestation": {
                "signing_address": GATEWAY_ADDRESS,
                "signing_algo": "ecdsa",
                "intel_quote": gateway_quote(),
                "event_log": "[]",
                "report_data": "00",
                "request_nonce": fixture_nonce(),
                "info": fixture["info"].clone(),
                "vpc": {}
            },
            "model_attestations": entries,
            "ohttp_key_config": "00",
            "ohttp_attestation": {}
        })
        .to_string()
    }

    #[test]
    fn a_gateway_report_parses_to_the_requested_models_entry() {
        let body = gateway_report(vec![
            model_entry("zai-org/GLM-5"),
            model_entry(FIXTURE_MODEL),
        ]);
        let r = AttestationReport::from_json_for_model(&body, FIXTURE_MODEL)
            .expect("one entry names the model");
        let single = AttestationReport::from_json(FIXTURE).unwrap();
        assert_eq!(r.model_name, FIXTURE_MODEL);
        assert_eq!(r.intel_quote, single.intel_quote);
        assert_eq!(r.signing_address, single.signing_address);
        assert_eq!(r.signing_algo, "ecdsa");
        assert_eq!(r.signing_public_key, single.signing_public_key);
        assert_eq!(r.request_nonce, single.request_nonce);
        assert!(r.quote_binds_nonce(&fixture_nonce()).unwrap());
    }

    #[test]
    fn the_gateway_quote_is_never_used_for_the_model() {
        let body = gateway_report(vec![model_entry(FIXTURE_MODEL)]);
        let r = AttestationReport::from_json_for_model(&body, FIXTURE_MODEL).unwrap();
        assert_ne!(r.intel_quote, gateway_quote());
        assert_ne!(r.signing_address, GATEWAY_ADDRESS);

        // With no entry for the model, the gateway's own quote -- which here
        // even binds our nonce -- must not be offered in its place.
        let body = gateway_report(vec![model_entry("zai-org/GLM-5")]);
        assert_eq!(
            AttestationReport::from_json_for_model(&body, FIXTURE_MODEL).unwrap_err(),
            ReportShapeError::ModelNotAttested
        );
    }

    #[test]
    fn a_gateway_report_with_no_entry_for_the_model_is_refused() {
        for entries in [vec![], vec![model_entry("zai-org/GLM-5")]] {
            let body = gateway_report(entries);
            let error = AttestationReport::from_json_for_model(&body, FIXTURE_MODEL).unwrap_err();
            assert_eq!(error, ReportShapeError::ModelNotAttested);
            assert_eq!(error.label(), "model_not_attested");
        }
    }

    #[test]
    fn the_model_name_match_is_exact() {
        let body = gateway_report(vec![
            model_entry(&FIXTURE_MODEL.to_ascii_lowercase()),
            model_entry(&format!("{FIXTURE_MODEL}-v2")),
            model_entry(&format!(" {FIXTURE_MODEL}")),
        ]);
        assert_eq!(
            AttestationReport::from_json_for_model(&body, FIXTURE_MODEL).unwrap_err(),
            ReportShapeError::ModelNotAttested
        );
    }

    #[test]
    fn two_entries_for_the_model_are_refused_not_picked_between() {
        let body = gateway_report(vec![model_entry(FIXTURE_MODEL), model_entry(FIXTURE_MODEL)]);
        let error = AttestationReport::from_json_for_model(&body, FIXTURE_MODEL).unwrap_err();
        assert_eq!(error, ReportShapeError::ModelAttestedMoreThanOnce);
        assert_eq!(error.label(), "model_attested_more_than_once");
    }

    #[test]
    fn a_non_ecdsa_model_entry_is_refused_at_selection() {
        let mut entry = model_entry(FIXTURE_MODEL);
        entry["signing_algo"] = serde_json::json!("ed25519");
        let body = gateway_report(vec![entry]);
        let error = AttestationReport::from_json_for_model(&body, FIXTURE_MODEL).unwrap_err();
        assert_eq!(error, ReportShapeError::ModelEntryNotEcdsa);
        assert_eq!(error.label(), "model_entry_not_ecdsa");
    }

    #[test]
    fn a_gateway_only_report_is_refused_rather_than_read_as_the_model() {
        let mut v: serde_json::Value =
            serde_json::from_str(&gateway_report(vec![model_entry(FIXTURE_MODEL)])).unwrap();
        v.as_object_mut().unwrap().remove("model_attestations");
        let error =
            AttestationReport::from_json_for_model(&v.to_string(), FIXTURE_MODEL).unwrap_err();
        assert_eq!(error, ReportShapeError::GatewayOnly);
        assert_eq!(error.label(), "gateway_report_without_model_attestations");
    }

    #[test]
    fn a_report_carrying_both_shapes_is_refused() {
        let mut v: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
        v["model_attestations"] = serde_json::json!([model_entry(FIXTURE_MODEL)]);
        let error =
            AttestationReport::from_json_for_model(&v.to_string(), FIXTURE_MODEL).unwrap_err();
        assert_eq!(error, ReportShapeError::BothShapes);
        assert_eq!(error.label(), "report_carries_both_shapes");
    }

    #[test]
    fn a_body_that_is_not_a_report_is_malformed() {
        for body in ["not json", "[]", "{}", r#"{"model_attestations": {}}"#] {
            let error = AttestationReport::from_json_for_model(body, FIXTURE_MODEL).unwrap_err();
            assert!(
                matches!(error, ReportShapeError::Malformed { .. }),
                "{body}: {error:?}"
            );
            assert_eq!(error.label(), "malformed_response");
        }
    }

    /// Production pin: a per-model completions host's single-report body
    /// parses exactly as `from_json` always parsed it, and the requested
    /// model is not consulted -- the single shape never checked it, and
    /// starting to would change production behaviour.
    #[test]
    fn the_single_report_shape_is_unchanged() {
        let single = AttestationReport::from_json(FIXTURE).unwrap();
        for model in [FIXTURE_MODEL, "Qwen/Qwen3.8-27B", ""] {
            let parsed = AttestationReport::from_json_for_model(FIXTURE, model).unwrap();
            assert_eq!(parsed, single, "{model}");
        }
        assert!(single.info.tcb_info.is_some());
        let claim = single
            .unverified_json_measurements()
            .expect("the single report claims its registers");
        let v: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
        assert_eq!(claim.mrtd, v["info"]["tcb_info"]["mrtd"].as_str().unwrap());
        assert_eq!(
            claim.rtmr3,
            v["info"]["tcb_info"]["rtmr3"].as_str().unwrap()
        );
    }

    /// A model entry carries no `info.tcb_info`. That is "no claim to
    /// compare", not a claim that agrees with anything.
    #[test]
    fn a_model_entry_without_tcb_info_makes_no_register_claim() {
        let body = gateway_report(vec![model_entry(FIXTURE_MODEL)]);
        let r = AttestationReport::from_json_for_model(&body, FIXTURE_MODEL).unwrap();
        assert!(r.info.tcb_info.is_none());
        assert_eq!(r.unverified_json_measurements(), None);
    }
}
