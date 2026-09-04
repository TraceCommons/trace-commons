// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reading a witness certificate off an inbound request.
//!
//! A contributor who witnessed their redaction sends the certificate and its
//! signature in two headers alongside the ordinary submission body. The body
//! is unchanged: it is the envelope bytes the witness returned, forwarded
//! verbatim, and the certificate's digest is over exactly those bytes.
//!
//! # Both headers or neither
//!
//! One header alone is a refusal of the bypass, never a silent fall-through to
//! an unwitnessed submission. A caller who sent half a pair has a bug, and
//! quietly treating it as "no certificate" would hide it -- on the one path
//! where the difference decides whether a classifier ever reads the trace.
//!
//! # Fail open on the submission, closed on the bypass
//!
//! Every error here refuses **the bypass**, not the submission. The handler
//! holds the trace exactly as it would have without a certificate. A witness
//! outage must not become a submission outage: a contributor whose enclave is
//! down should still be able to contribute, on the ordinary terms.
//!
//! # Field-by-field decoding, deliberately
//!
//! [`WitnessCertificate`] has no `Deserialize` impl and this module does not
//! give it one. A `serde_json/preserve_order` change moved every untyped-JSON
//! digest in this workspace on 2026-09-01; the length-prefixed encoder in
//! [`WitnessCertificate::signing_bytes`] exists so that cannot happen again,
//! and it only helps if both sides reach it through named fields rather than
//! through whatever order a JSON map happened to preserve. So the decoder
//! names each field, and the wire spelling of the verdict is an explicit
//! closed set rather than a derive.
//!
//! # Logging
//!
//! Nothing here logs, and [`WitnessHeaderError`] is safe under both
//! formatters. Header values are attacker-chosen, so no variant carries one --
//! not a prefix, not a length. The one value any variant carries is a
//! `&'static str` field name this module wrote itself.

use axum::http::HeaderMap;
use trace_commons_protocol::trace_contribution::ResidualPiiRisk;

use super::certificate::{CertificateDetails, WitnessCertificate};

/// Header carrying the certificate as compact JSON, exactly as the witness
/// service put it on its own response.
///
/// The name and the encoding are both the witness's, not this module's
/// choice. A contributor forwards the header value it received byte for
/// byte -- that is the whole reason the witness serves it in a header rather
/// than in a body a client would have to re-render -- so a second spelling
/// here is not an alternative form, it is a header nothing ever sends.
/// `crates/trace-commons-server/tests/witness_certificate_cross_implementation.rs`
/// drives the witness's own router and requires this constant to name a
/// header that response actually carries.
pub const CERTIFICATE_HEADER: &str = "x-trace-witness-certificate";

/// Header carrying the EIP-191 signature over the certificate's signing
/// bytes, as `0x`-prefixed hex. Same rule: the witness's spelling.
pub const SIGNATURE_HEADER: &str = "x-trace-witness-signature";

/// Why a request's witness headers could not be read.
///
/// Each variant is a distinct thing a contributor's client got wrong, and
/// they stay separate because they send that client's author to different
/// places. None of them rejects the submission.
///
/// `Debug` delegates to `Display`, as everywhere else in this module tree,
/// because `tracing::warn!(?err)` is how an error reaches a log here -- and a
/// derived `Debug` on a variant that ever gained a value field would render
/// attacker-chosen bytes into an operator surface.
#[derive(Clone, PartialEq, Eq, thiserror::Error)]
pub enum WitnessHeaderError {
    /// A signature arrived with no certificate to verify.
    #[error("a witness signature header arrived without a certificate header")]
    CertificateMissing,
    /// A certificate arrived with no signature, so nothing about it could be
    /// checked. Not treated as an unwitnessed submission: see the module doc.
    #[error("a witness certificate header arrived without a signature header")]
    SignatureMissing,
    /// A header value is not ASCII, so it is not a header this API defines.
    /// Carries no value: the bytes are attacker-chosen.
    #[error("a witness header value is not valid ASCII")]
    HeaderNotAscii,
    /// The certificate header is not a JSON object.
    #[error("the witness certificate is not a JSON object")]
    CertificateNotJson,
    /// A required certificate field is absent, or is not of the type the wire
    /// format defines. Carries the field NAME, which this module wrote, never
    /// the value, which the sender chose.
    #[error("the witness certificate field {field} is missing or malformed")]
    CertificateFieldMalformed { field: &'static str },
}

impl std::fmt::Debug for WitnessHeaderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, formatter)
    }
}

/// Read the certificate and signature off a request's headers.
///
/// `Ok(None)` means neither header was present: an ordinary unwitnessed
/// submission, which is what almost every submission is and what every
/// submission is today.
///
/// The returned certificate is **unverified**. It carries whatever the sender
/// typed, and it is worth nothing until
/// [`verify_witness_certificate`](super::verification::verify_witness_certificate)
/// has checked the signature against the pinned address, pinned the
/// measurement, and matched the digest against the request body **as
/// received**. The last of those is why the caller must digest the raw body
/// rather than a re-serialisation: the submit handler's own redaction pass
/// rewrites the envelope in place, so the stored bytes are never the received
/// bytes.
pub fn witness_headers(
    headers: &HeaderMap,
) -> Result<Option<(WitnessCertificate, String)>, WitnessHeaderError> {
    let certificate = ascii_header(headers, CERTIFICATE_HEADER)?;
    let signature = ascii_header(headers, SIGNATURE_HEADER)?;

    let (certificate, signature) = match (certificate, signature) {
        (None, None) => return Ok(None),
        (Some(_), None) => return Err(WitnessHeaderError::SignatureMissing),
        (None, Some(_)) => return Err(WitnessHeaderError::CertificateMissing),
        (Some(certificate), Some(signature)) => (certificate, signature),
    };

    let value: serde_json::Value =
        serde_json::from_str(certificate).map_err(|_| WitnessHeaderError::CertificateNotJson)?;
    let object = value
        .as_object()
        .ok_or(WitnessHeaderError::CertificateNotJson)?;

    // Named fields, one at a time. See the module doc for why this is not a
    // derive: the signing bytes are length-prefixed and field-ordered, and
    // that only protects both sides if both sides name their fields.
    let redacted_sha256 = string_field(object, "redacted_sha256")?;
    let redaction_policy_version = string_field(object, "redaction_policy_version")?;
    let witness_measurement = string_field(object, "witness_measurement")?;
    let residual_risk_verdict = verdict_field(object)?;
    let timestamp = object
        .get("timestamp")
        .and_then(serde_json::Value::as_i64)
        .ok_or(WitnessHeaderError::CertificateFieldMalformed { field: "timestamp" })?;

    Ok(Some((
        WitnessCertificate::from_wire(
            redacted_sha256,
            CertificateDetails {
                residual_risk_verdict,
                redaction_policy_version,
                witness_measurement,
                timestamp,
            },
        ),
        signature.to_string(),
    )))
}

fn ascii_header<'h>(
    headers: &'h HeaderMap,
    name: &str,
) -> Result<Option<&'h str>, WitnessHeaderError> {
    match headers.get(name) {
        None => Ok(None),
        Some(value) => value
            .to_str()
            .map(Some)
            .map_err(|_| WitnessHeaderError::HeaderNotAscii),
    }
}

fn string_field(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &'static str,
) -> Result<String, WitnessHeaderError> {
    object
        .get(field)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or(WitnessHeaderError::CertificateFieldMalformed { field })
}

/// The wire spelling of the verdict, as a closed set.
///
/// The mirror of `witness_service::http::verdict_label`, and it must stay
/// exhaustive for the same reason that one is: an unknown tier has to be a
/// refusal, never a value some default arm turns into `Low`. A `Low` reached
/// by falling through would be the single worst bug this feature could have.
fn verdict_field(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<ResidualPiiRisk, WitnessHeaderError> {
    const FIELD: &str = "residual_risk_verdict";
    match object.get(FIELD).and_then(serde_json::Value::as_str) {
        Some("low") => Ok(ResidualPiiRisk::Low),
        Some("medium") => Ok(ResidualPiiRisk::Medium),
        Some("high") => Ok(ResidualPiiRisk::High),
        _ => Err(WitnessHeaderError::CertificateFieldMalformed { field: FIELD }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1";
    const MEASUREMENT: &str = "c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1";
    const POLICY: &str = "policy-v3";

    fn certificate_json() -> serde_json::Value {
        serde_json::json!({
            "redacted_sha256": DIGEST,
            "residual_risk_verdict": "low",
            "redaction_policy_version": POLICY,
            "witness_measurement": MEASUREMENT,
            "timestamp": 1_788_000_000i64,
        })
    }

    /// The wire form: compact JSON, which is what the witness's own response
    /// header carries and what a contributor forwards unchanged.
    fn encoded(value: &serde_json::Value) -> String {
        serde_json::to_string(value).expect("the fixture serialises")
    }

    fn encoded_certificate() -> String {
        encoded(&certificate_json())
    }

    fn headers_with(certificate: Option<&str>, signature: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(certificate) = certificate {
            headers.insert(
                CERTIFICATE_HEADER,
                certificate.parse().expect("a header value"),
            );
        }
        if let Some(signature) = signature {
            headers.insert(SIGNATURE_HEADER, signature.parse().expect("a header value"));
        }
        headers
    }

    #[test]
    fn neither_header_is_an_ordinary_submission() {
        assert!(
            witness_headers(&HeaderMap::new())
                .expect("no headers is not an error")
                .is_none()
        );
    }

    #[test]
    fn a_certificate_without_a_signature_refuses_by_name() {
        let headers = headers_with(Some(&encoded_certificate()), None);
        let err = witness_headers(&headers).expect_err("a half-present pair must refuse");
        assert_eq!(err, WitnessHeaderError::SignatureMissing, "{err}");
    }

    #[test]
    fn a_signature_without_a_certificate_refuses_by_name() {
        let headers = headers_with(None, Some("0x00"));
        let err = witness_headers(&headers).expect_err("a half-present pair must refuse");
        assert_eq!(err, WitnessHeaderError::CertificateMissing, "{err}");
    }

    #[test]
    fn a_certificate_that_is_not_json_refuses_by_name() {
        let headers = headers_with(Some("!!!not-json!!!"), Some("0x00"));
        let err = witness_headers(&headers).expect_err("bad encoding must refuse");
        assert_eq!(err, WitnessHeaderError::CertificateNotJson, "{err}");
    }

    /// Base64 of the certificate JSON was this module's previous wire form,
    /// and nothing ever sent it: the witness serves compact JSON and the
    /// contributor forwards that value verbatim. It must refuse now, or the
    /// header would accept two spellings of which only one is ever produced.
    #[test]
    fn base64_of_the_certificate_is_no_longer_a_certificate() {
        use base64::Engine as _;
        let legacy = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&certificate_json()).expect("the fixture serialises"));
        let headers = headers_with(Some(&legacy), Some("0x00"));
        let err = witness_headers(&headers).expect_err("the old encoding must refuse");
        assert_eq!(err, WitnessHeaderError::CertificateNotJson, "{err}");
    }

    #[test]
    fn json_that_is_not_an_object_refuses_by_name() {
        let array = encoded(&serde_json::json!([1, 2, 3]));
        let headers = headers_with(Some(&array), Some("0x00"));
        let err = witness_headers(&headers).expect_err("a non-object must refuse");
        assert_eq!(err, WitnessHeaderError::CertificateNotJson, "{err}");
    }

    #[test]
    fn every_missing_field_refuses_naming_that_field() {
        for field in [
            "redacted_sha256",
            "redaction_policy_version",
            "witness_measurement",
            "residual_risk_verdict",
            "timestamp",
        ] {
            let mut json = certificate_json();
            json.as_object_mut().expect("an object").remove(field);
            let headers = headers_with(Some(&encoded(&json)), Some("0x00"));
            let err = witness_headers(&headers).expect_err("a missing field must refuse");
            assert_eq!(
                err,
                WitnessHeaderError::CertificateFieldMalformed { field },
                "{err}",
            );
        }
    }

    /// The single worst bug this feature could have: an unrecognised verdict
    /// falling through to `Low`. A tier the wire format does not define is a
    /// refusal, never a default.
    #[test]
    fn an_unknown_verdict_refuses_rather_than_defaulting_to_low() {
        for spelling in [
            serde_json::json!("clean"),
            serde_json::json!("Low"),
            serde_json::json!(""),
            serde_json::json!(0),
            serde_json::json!(null),
        ] {
            let mut json = certificate_json();
            json.as_object_mut()
                .expect("an object")
                .insert("residual_risk_verdict".to_string(), spelling.clone());
            let headers = headers_with(Some(&encoded(&json)), Some("0x00"));
            let err = witness_headers(&headers).expect_err("an unknown verdict must refuse");
            assert_eq!(
                err,
                WitnessHeaderError::CertificateFieldMalformed {
                    field: "residual_risk_verdict"
                },
                "{spelling} was accepted",
            );
        }
    }

    /// Every verdict the wire format defines round-trips to the typed value,
    /// so the closed set above cannot silently lose a tier.
    #[test]
    fn each_defined_verdict_decodes_to_its_own_tier() {
        for (spelling, expected) in [
            ("low", ResidualPiiRisk::Low),
            ("medium", ResidualPiiRisk::Medium),
            ("high", ResidualPiiRisk::High),
        ] {
            let mut json = certificate_json();
            json.as_object_mut()
                .expect("an object")
                .insert("residual_risk_verdict".to_string(), spelling.into());
            let headers = headers_with(Some(&encoded(&json)), Some("0xsig"));
            let (certificate, _) = witness_headers(&headers)
                .expect("a well-formed certificate decodes")
                .expect("both headers are present");
            // Read back through the signing bytes: the certificate's verdict
            // accessor is crate-private to its own module, and the signing
            // bytes are what the signature actually covers anyway.
            let mut other = certificate_json();
            other.as_object_mut().expect("an object").insert(
                "residual_risk_verdict".to_string(),
                verdict_wire_name(expected).into(),
            );
            let (rebuilt, _) =
                witness_headers(&headers_with(Some(&encoded(&other)), Some("0xsig")))
                    .expect("decodes")
                    .expect("present");
            assert_eq!(
                certificate.signing_bytes(),
                rebuilt.signing_bytes(),
                "{spelling} did not decode to {expected:?}",
            );
        }
    }

    fn verdict_wire_name(verdict: ResidualPiiRisk) -> &'static str {
        match verdict {
            ResidualPiiRisk::Low => "low",
            ResidualPiiRisk::Medium => "medium",
            ResidualPiiRisk::High => "high",
        }
    }

    /// The decoded certificate must carry the digest the sender claimed, byte
    /// for byte. A decoder that dropped or normalised it would make every
    /// honest submission fail the artifact check, which is the failure mode
    /// that reads as "the witness is broken".
    #[test]
    fn the_decoded_certificate_carries_the_claimed_digest_and_signature() {
        let headers = headers_with(Some(&encoded_certificate()), Some("0xdeadbeef"));
        let (certificate, signature) = witness_headers(&headers)
            .expect("decodes")
            .expect("both headers are present");
        assert_eq!(signature, "0xdeadbeef");
        // The digest is length-prefixed first in the signing bytes, after the
        // domain separator. Asserting it is present there proves the decoder
        // put it on the certificate rather than dropping it.
        let bytes = certificate.signing_bytes();
        assert!(
            bytes
                .windows(DIGEST.len())
                .any(|window| window == DIGEST.as_bytes()),
            "the claimed digest did not survive decoding",
        );
    }

    #[test]
    fn header_decoding_never_renders_content_in_its_error() {
        // Hash-only: a header is attacker-chosen and must not reach a log via
        // a refusal. Assert on both formatters, as verification.rs does.
        let headers = headers_with(Some("SECRETMARKER!!!"), Some("0xSECRETMARKER"));
        let err = witness_headers(&headers).expect_err("refuses");
        assert!(!format!("{err}").contains("SECRETMARKER"), "{err}");
        assert!(!format!("{err:?}").contains("SECRETMARKER"), "{err:?}");

        // And on the field-level refusal, which is the variant that carries a
        // value at all.
        let mut json = certificate_json();
        json.as_object_mut().expect("an object").insert(
            "witness_measurement".to_string(),
            serde_json::json!({ "nested": "SECRETMARKER" }),
        );
        let headers = headers_with(Some(&encoded(&json)), Some("0x00"));
        let err = witness_headers(&headers).expect_err("refuses");
        assert!(!format!("{err}").contains("SECRETMARKER"), "{err}");
        assert!(!format!("{err:?}").contains("SECRETMARKER"), "{err:?}");
    }

    #[test]
    fn a_non_ascii_header_refuses_without_rendering_it() {
        let mut headers = HeaderMap::new();
        headers.insert(
            CERTIFICATE_HEADER,
            axum::http::HeaderValue::from_bytes(&[0xff, 0xfe]).expect("bytes are a header value"),
        );
        headers.insert(SIGNATURE_HEADER, "0x00".parse().expect("a header value"));
        let err = witness_headers(&headers).expect_err("non-ASCII must refuse");
        assert_eq!(err, WitnessHeaderError::HeaderNotAscii, "{err}");
    }

    /// The timestamp is a security control now -- `verify_witness_certificate`
    /// refuses a certificate outside its freshness window -- so a certificate
    /// that carries no usable one must refuse HERE, before verification is
    /// reached. Absent, null, a string, or a float would each otherwise arrive
    /// as a value the window has no opinion about.
    #[test]
    fn a_timestamp_that_is_not_an_integer_refuses_by_name() {
        for value in [
            serde_json::Value::Null,
            serde_json::json!("1788000000"),
            serde_json::json!(1.5),
            serde_json::json!([]),
        ] {
            let mut json = certificate_json();
            json["timestamp"] = value.clone();
            let headers = headers_with(Some(&encoded(&json)), Some("0x00"));
            let err = witness_headers(&headers).expect_err("a non-integer timestamp must refuse");
            assert_eq!(
                err,
                WitnessHeaderError::CertificateFieldMalformed { field: "timestamp" },
                "{value} was accepted: {err}"
            );
        }
    }

    #[test]
    fn the_header_names_are_the_ones_the_operator_doc_states() {
        assert_eq!(CERTIFICATE_HEADER, "x-trace-witness-certificate");
        assert_eq!(SIGNATURE_HEADER, "x-trace-witness-signature");
    }
}
