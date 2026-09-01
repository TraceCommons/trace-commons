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
//! This module only parses the report and checks nonce binding. It does not
//! verify the quote's signature chain (Task 2) or pin expected measurements
//! (Task 3).

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;

/// A NEAR AI attestation report, as returned by
/// `GET /v1/attestation/report?nonce=<hex>`.
///
/// `deny_unknown_fields` is deliberately off: the live service sends
/// substantially more than we model here (e.g. `nvidia_payload`,
/// `all_attestations`, `ohttp_attestation`) and will add more over time.
/// Only fields this crate actually uses are modeled.
#[derive(Debug, Clone, Deserialize)]
pub struct AttestationReport {
    pub model_name: String,
    pub signing_address: String,
    pub signing_algo: String,
    pub signing_public_key: String,
    pub request_nonce: String,
    pub intel_quote: String,
    pub info: AttestationInfo,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AttestationInfo {
    pub compose_hash: String,
    pub os_image_hash: String,
    pub mr_aggregated: String,
    pub instance_id: String,
    pub app_id: String,
    pub tcb_info: TcbInfo,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TcbInfo {
    pub mrtd: String,
    pub rtmr0: String,
    pub rtmr1: String,
    pub rtmr2: String,
    pub rtmr3: String,
}

/// The measurement set pinned to a specific enclave image and boot state,
/// pulled out of an [`AttestationReport`] for comparison against a known-good
/// baseline (Task 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Measurements {
    pub mrtd: String,
    pub rtmr0: String,
    pub rtmr1: String,
    pub rtmr2: String,
    pub rtmr3: String,
    pub compose_hash: String,
    pub os_image_hash: String,
    pub mr_aggregated: String,
}

impl AttestationReport {
    /// Parse a report from its JSON body.
    pub fn from_json(body: &str) -> Result<Self> {
        serde_json::from_str(body).context("parsing NEAR AI attestation report JSON")
    }

    /// Decode `intel_quote` from hex into raw quote bytes.
    pub fn quote_bytes(&self) -> Result<Vec<u8>> {
        hex::decode(&self.intel_quote).context("decoding intel_quote as hex")
    }

    /// The measurement set from this report.
    pub fn measurements(&self) -> Measurements {
        Measurements {
            mrtd: self.info.tcb_info.mrtd.clone(),
            rtmr0: self.info.tcb_info.rtmr0.clone(),
            rtmr1: self.info.tcb_info.rtmr1.clone(),
            rtmr2: self.info.tcb_info.rtmr2.clone(),
            rtmr3: self.info.tcb_info.rtmr3.clone(),
            compose_hash: self.info.compose_hash.clone(),
            os_image_hash: self.info.os_image_hash.clone(),
            mr_aggregated: self.info.mr_aggregated.clone(),
        }
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

    const FIXTURE: &str = include_str!("../../tests/fixtures/near_ai_attestation_report.json");

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
        let other = "0".repeat(64);
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
}
