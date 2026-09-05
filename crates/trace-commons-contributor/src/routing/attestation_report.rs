//! Reading the gateway signing key out of a NEAR AI attestation report.
//!
//! `GET {base}/attestation/report?model=..&nonce=..` returns, among much
//! else, a `gateway_attestation` whose `report_data` is
//! `signing_address || request_nonce`. That concatenation, inside a TDX
//! quote, is what binds the key to a nonce this client chose -- so a key
//! read from a report whose `report_data` carries OUR nonce is one that was
//! attested for us, now, rather than one copied from an old report.
//!
//! **This module does not verify the quote.** It reads the report's
//! self-description and checks its internal consistency. Until quote
//! verification exists, a key from here is a claim by NEAR AI, not a proof,
//! and the config gate that enables this check says so.
//!
//! Nothing here logs. The report holds keys and identifiers; none of them
//! belong on an operational surface.

use trace_commons_attestation::receipt::ReceiptAlgo;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AttestationReportError {
    #[error("attestation report is not the expected JSON shape")]
    Malformed,
    #[error("attestation report gateway is not ed25519")]
    NotEd25519,
    #[error("attestation report was issued for a different nonce")]
    NonceMismatch,
    #[error("attestation report_data does not commit to the listed key and nonce")]
    ReportDataMismatch,
    #[error("attestation report gateway key is not 32 bytes of hex")]
    KeyMalformed,
    #[error("attestation report base URL is not a valid https URL")]
    UrlInvalid,
}

/// The URL a fetch would call.
///
/// # Errors
///
/// [`AttestationReportError::UrlInvalid`] when `base` does not parse or is
/// not https.
pub fn attestation_report_url(
    base: &str,
    model: &str,
    nonce: &str,
) -> Result<url::Url, AttestationReportError> {
    let mut url = url::Url::parse(base).map_err(|_| AttestationReportError::UrlInvalid)?;
    if url.scheme() != "https" {
        return Err(AttestationReportError::UrlInvalid);
    }
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| AttestationReportError::UrlInvalid)?;
        segments.pop_if_empty().push("attestation").push("report");
    }
    url.query_pairs_mut()
        .append_pair("model", model)
        .append_pair("nonce", nonce);
    Ok(url)
}

/// The gateway's ed25519 signing key, if the report binds it to `expected_nonce`.
///
/// # Errors
///
/// [`AttestationReportError`] when the report is not the expected shape, the
/// gateway is not ed25519, the report was issued for a different nonce, or
/// `report_data` does not commit to the listed key and nonce.
pub fn gateway_ed25519_key(
    report_json: &str,
    expected_nonce: &str,
) -> Result<String, AttestationReportError> {
    let document: serde_json::Value =
        serde_json::from_str(report_json).map_err(|_| AttestationReportError::Malformed)?;
    let gateway = document
        .get("gateway_attestation")
        .ok_or(AttestationReportError::Malformed)?;
    let field = |name: &str| {
        gateway
            .get(name)
            .and_then(serde_json::Value::as_str)
            .ok_or(AttestationReportError::Malformed)
    };

    let algo = field("signing_algo")?;
    if ReceiptAlgo::from_wire(algo) != Some(ReceiptAlgo::Ed25519) {
        return Err(AttestationReportError::NotEd25519);
    }
    let key = field("signing_address")?.to_ascii_lowercase();
    // A key this short (or empty) could `strip_prefix` trivially against any
    // `report_data`, making the check below vacuous. Refused here so
    // `gateway_ed25519_key` can never return a string that is not actually a
    // 32-byte hex key -- callers should not have to re-check that.
    if key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(AttestationReportError::KeyMalformed);
    }
    // `request_nonce` is the provider's own label for the value; required for
    // shape, but the binding this function trusts is `report_data` itself,
    // checked below against the key it names and the nonce we asked for.
    let _ = field("request_nonce")?;
    let report_data = field("report_data")?.to_ascii_lowercase();
    let expected_nonce = expected_nonce.to_ascii_lowercase();

    // `report_data` must commit to the listed key first: a report_data whose
    // prefix is not this report's own `signing_address` names a different
    // key than the one it claims, and that is refused before the nonce is
    // even inspected.
    let attested_nonce = report_data
        .strip_prefix(key.as_str())
        .ok_or(AttestationReportError::ReportDataMismatch)?;
    if attested_nonce != expected_nonce {
        return Err(AttestationReportError::NonceMismatch);
    }
    Ok(key)
}

/// Whether a verified receipt's signer is the gateway key a report attested.
///
/// Case-insensitive, because the report renders the key lowercase and a
/// receipt might not. Exact otherwise: a prefix, a suffix, or a different
/// scheme's address does not match.
#[must_use]
pub fn signer_is_attested(receipt_signer: &str, attested_key: &str) -> bool {
    !attested_key.is_empty() && receipt_signer.eq_ignore_ascii_case(attested_key)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape NEAR AI actually returns, reduced to the fields read.
    /// `report_data` is `signing_address || request_nonce`, which is what
    /// binds the key to a caller-chosen nonce inside the TDX quote.
    const NONCE: &str = "482934fb749d13aa81b2e543a253cf4d8cc847dab55a8d49989effd5023ddb5d";
    const KEY: &str = "cb6fc58f6bd685919fa42fb54d3fcfe03222e324bdda91f0bac6d5c73dc4f1c6";

    /// The `gateway_attestation` subtree of a report NEAR AI actually
    /// returned (2026-09-05), reduced to the fields this parser reads. Every
    /// other fixture in this module is authored beside the code; this one
    /// pins that the real response nests these fields the way the parser
    /// expects and that `report_data` really is `signing_address ||
    /// request_nonce`. `NONCE` and `KEY` above are the live values, so this
    /// composes with them directly.
    const LIVE_REPORT: &str = r#"{"gateway_attestation":{"signing_address":"cb6fc58f6bd685919fa42fb54d3fcfe03222e324bdda91f0bac6d5c73dc4f1c6","signing_algo":"ed25519","request_nonce":"482934fb749d13aa81b2e543a253cf4d8cc847dab55a8d49989effd5023ddb5d","report_data":"cb6fc58f6bd685919fa42fb54d3fcfe03222e324bdda91f0bac6d5c73dc4f1c6482934fb749d13aa81b2e543a253cf4d8cc847dab55a8d49989effd5023ddb5d"},"model_attestations":[],"ohttp_attestation":{}}"#;

    fn report(nonce_in_report_data: &str) -> String {
        format!(
            r#"{{"gateway_attestation":{{"signing_address":"{KEY}","signing_algo":"ed25519","request_nonce":"{NONCE}","report_data":"{KEY}{nonce_in_report_data}"}}}}"#
        )
    }

    #[test]
    fn the_gateway_key_is_read_when_report_data_binds_the_nonce_we_sent() {
        assert_eq!(gateway_ed25519_key(&report(NONCE), NONCE).unwrap(), KEY);
    }

    /// A report whose report_data carries a different nonce is stale or
    /// replayed, and its key is not accepted. This is the whole point of
    /// sending a nonce.
    #[test]
    fn a_report_for_a_different_nonce_is_refused() {
        let other = "0".repeat(64);
        assert_eq!(
            gateway_ed25519_key(&report(&other), NONCE).unwrap_err(),
            AttestationReportError::NonceMismatch
        );
    }

    /// report_data must equal key || nonce exactly. A report that lists the
    /// key but whose report_data does not commit to it is not a binding.
    #[test]
    fn report_data_that_does_not_commit_to_the_key_is_refused() {
        let body = format!(
            r#"{{"gateway_attestation":{{"signing_address":"{KEY}","signing_algo":"ed25519","request_nonce":"{NONCE}","report_data":"{}{NONCE}"}}}}"#,
            "f".repeat(64)
        );
        assert_eq!(
            gateway_ed25519_key(&body, NONCE).unwrap_err(),
            AttestationReportError::ReportDataMismatch
        );
    }

    /// `strip_prefix` on a too-short (or empty) key would trivially succeed
    /// against any `report_data`, so this checks the key's shape before the
    /// split ever happens. Each case's `report_data` is set to exactly the
    /// key followed by the real nonce -- the shape that would otherwise
    /// pass -- so the test would pass the pre-guard code and fails only on
    /// this check.
    #[test]
    fn a_report_whose_key_is_not_32_bytes_is_refused() {
        let cases = ["", "cb", &"g".repeat(64)];
        for bad_key in cases {
            let body = format!(
                r#"{{"gateway_attestation":{{"signing_address":"{bad_key}","signing_algo":"ed25519","request_nonce":"{NONCE}","report_data":"{bad_key}{NONCE}"}}}}"#
            );
            assert_eq!(
                gateway_ed25519_key(&body, NONCE).unwrap_err(),
                AttestationReportError::KeyMalformed,
                "key {bad_key:?} should be refused as malformed"
            );
        }
    }

    #[test]
    fn a_non_ed25519_gateway_is_refused() {
        let body = format!(
            r#"{{"gateway_attestation":{{"signing_address":"{KEY}","signing_algo":"ecdsa","request_nonce":"{NONCE}","report_data":"{KEY}{NONCE}"}}}}"#
        );
        assert_eq!(
            gateway_ed25519_key(&body, NONCE).unwrap_err(),
            AttestationReportError::NotEd25519
        );
    }

    #[test]
    fn the_report_url_carries_model_and_nonce() {
        let u = attestation_report_url(
            "https://cloud-api.near.ai/v1",
            "Qwen/Qwen3.6-35B-A3B-FP8",
            NONCE,
        )
        .unwrap();
        assert_eq!(u.path(), "/v1/attestation/report");
        assert!(u.query().unwrap().contains(&format!("nonce={NONCE}")));
        assert!(
            u.query()
                .unwrap()
                .contains("model=Qwen%2FQwen3.6-35B-A3B-FP8")
        );
    }

    #[test]
    fn the_signer_matches_the_attested_key_case_insensitively() {
        assert!(signer_is_attested(&KEY.to_ascii_uppercase(), KEY));
        assert!(signer_is_attested(KEY, KEY));
    }

    #[test]
    fn a_different_signer_does_not_match() {
        assert!(!signer_is_attested(
            "0x614bc66ff0407dbb70b9c7ca1f5e983e4a02c921",
            KEY
        ));
        assert!(
            !signer_is_attested(&KEY[..62], KEY),
            "a prefix is not a match"
        );
        assert!(!signer_is_attested("", KEY));
        assert!(
            !signer_is_attested(KEY, ""),
            "an empty attested key matches nothing"
        );
    }

    /// The gateway_attestation subtree of a report NEAR AI actually returned,
    /// reduced to the fields this parser reads. Every other fixture here is
    /// authored beside the code; this one pins that the real response nests
    /// these fields the way the parser expects and that report_data really is
    /// signing_address || request_nonce. The nonce is the one that fetch sent.
    #[test]
    fn the_live_report_parses_to_the_attested_gateway_key() {
        assert_eq!(gateway_ed25519_key(LIVE_REPORT, NONCE).unwrap(), KEY);
    }
}
