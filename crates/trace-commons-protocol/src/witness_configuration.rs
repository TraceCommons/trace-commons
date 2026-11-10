//! Signed witness endpoints and their attestation pins.
//!
//! The enrollment URL is an enduring trust anchor: an operator keeps it in
//! `enrollment_urls` even after withdrawing that deployment. Ordered endpoints
//! name the preferred deployment followed by fallbacks. Clients verify the
//! signature before interpreting the payload, bind it to their enrollment URL,
//! enforce increasing versions, and attest each endpoint before using it.
//! This schema is separate from the additive legacy accepted-measurements list.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::witness_accepted_list::{
    AcceptedEntry, AcceptedList, SignedAcceptedList, WITNESS_ACCEPTED_LIST_MAX_SIGNATURES,
    WITNESS_ACCEPTED_LIST_SCHEMA, signature_verifies,
};

/// Public, unauthenticated route for the signed endpoint configuration.
pub const WITNESS_CONFIGURATION_PATH: &str = "/v1/public/witness-configuration";
/// Schema understood by this build. New trust semantics require a new schema.
pub const WITNESS_CONFIGURATION_SCHEMA: &str = "trace-commons.witness-configuration.v1";
/// Maximum size of the entire served signed document.
pub const WITNESS_CONFIGURATION_MAX_BYTES: usize = 64 * 1024;
/// Maximum number of retained enrollment anchors.
pub const WITNESS_CONFIGURATION_MAX_ENROLLMENT_URLS: usize = 32;
/// Maximum number of active deployments, in preference order.
pub const WITNESS_CONFIGURATION_MAX_ENDPOINTS: usize = 8;
/// Maximum number of measurement sets for each active deployment.
pub const WITNESS_CONFIGURATION_MAX_ENTRIES: usize = 32;

/// The payload carried in the existing [`SignedAcceptedList`] envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WitnessConfiguration {
    /// Always [`WITNESS_CONFIGURATION_SCHEMA`].
    pub schema: String,
    /// Positive, monotonically increasing; clients reject repeats and rollback.
    pub version: u64,
    /// Informational publication time; entries carry their own validity windows.
    pub issued_at: DateTime<Utc>,
    /// Enrollment anchors, including old URLs no longer serving the witness.
    pub enrollment_urls: Vec<String>,
    /// Preferred endpoint first, then fallbacks. Empty explicitly withdraws all.
    pub endpoints: Vec<WitnessEndpoint>,
}

/// An active witness deployment and its attestation pins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WitnessEndpoint {
    /// Exact HTTPS base URL. It must also be an enrollment anchor.
    pub url: String,
    /// Accepted measurement sets, signing addresses and validity windows.
    pub entries: Vec<AcceptedEntry>,
}

impl WitnessEndpoint {
    /// Entries in force at `now`, with an exclusive upper bound.
    pub fn entries_valid_at(&self, now: DateTime<Utc>) -> impl Iterator<Item = &AcceptedEntry> {
        self.entries
            .iter()
            .filter(move |entry| entry.is_valid_at(now))
    }
}

impl WitnessConfiguration {
    /// Whether this configuration is rooted in the device's enrollment URL.
    /// Only trailing slashes are ignored; original URL bytes are preserved.
    #[must_use]
    pub fn names_witness(&self, configured_url: &str) -> bool {
        self.enrollment_urls
            .iter()
            .any(|url| normalize_url(url) == normalize_url(configured_url))
    }

    /// Key-sorted compact JSON for an operator to review and sign. Clients
    /// verify the exact received bytes and never canonicalize before checking.
    pub fn canonical_payload(&self) -> Result<String, WitnessConfigurationError> {
        self.validate()?;
        let value = serde_json::to_value(self).map_err(|_| WitnessConfigurationError::Malformed)?;
        crate::canonical_json::to_canonical_string(&value)
            .map_err(|_| WitnessConfigurationError::Malformed)
    }

    /// Validate everything other than signatures and the served-byte limit.
    /// Measurement syntax is additionally checked by the client/operator's
    /// attestation parser, as with the legacy list.
    pub fn validate(&self) -> Result<(), WitnessConfigurationError> {
        if self.schema != WITNESS_CONFIGURATION_SCHEMA {
            return Err(WitnessConfigurationError::SchemaUnsupported);
        }
        if self.version == 0
            || self.enrollment_urls.is_empty()
            || self.enrollment_urls.len() > WITNESS_CONFIGURATION_MAX_ENROLLMENT_URLS
            || self.endpoints.len() > WITNESS_CONFIGURATION_MAX_ENDPOINTS
        {
            return Err(WitnessConfigurationError::Invalid);
        }
        let mut enrollment_urls = std::collections::HashSet::new();
        for url in &self.enrollment_urls {
            if !strict_https_url(url) || !enrollment_urls.insert(normalize_url(url)) {
                return Err(WitnessConfigurationError::Invalid);
            }
        }
        let mut endpoints = std::collections::HashSet::new();
        for endpoint in &self.endpoints {
            if !strict_https_url(&endpoint.url)
                || !enrollment_urls.contains(normalize_url(&endpoint.url))
                || !endpoints.insert(normalize_url(&endpoint.url))
                || endpoint.entries.len() > WITNESS_CONFIGURATION_MAX_ENTRIES
            {
                return Err(WitnessConfigurationError::Invalid);
            }
            // Keep entry/address/window/provenance validation identical to V1.
            AcceptedList {
                schema: WITNESS_ACCEPTED_LIST_SCHEMA.into(),
                version: self.version,
                issued_at: self.issued_at,
                witness_url: endpoint.url.clone(),
                entries: endpoint.entries.clone(),
            }
            .validate()
            .map_err(|_| WitnessConfigurationError::Invalid)?;
        }
        Ok(())
    }
}

/// Fixed labels only: errors never carry signed URLs, key material or payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WitnessConfigurationError {
    /// The served document exceeds its byte bound.
    #[error("witness-configuration-too-large")]
    TooLarge,
    /// Invalid JSON, unknown fields, wrong types or malformed envelope.
    #[error("witness-configuration-malformed")]
    Malformed,
    /// No trusted key verified a signature over the exact payload bytes.
    #[error("witness-configuration-signature-invalid")]
    SignatureInvalid,
    /// This build does not understand the signed schema.
    #[error("witness-configuration-schema-unsupported")]
    SchemaUnsupported,
    /// The signed configuration breaks a semantic or bound rule.
    #[error("witness-configuration-invalid")]
    Invalid,
}

impl WitnessConfigurationError {
    /// Label safe for logs and health responses.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::TooLarge => "witness-configuration-too-large",
            Self::Malformed => "witness-configuration-malformed",
            Self::SignatureInvalid => "witness-configuration-signature-invalid",
            Self::SchemaUnsupported => "witness-configuration-schema-unsupported",
            Self::Invalid => "witness-configuration-invalid",
        }
    }
}

/// Verify a served document before parsing or interpreting its payload.
pub fn verify_signed_configuration(
    document: &[u8],
    trusted_keys: &[[u8; 32]],
) -> Result<WitnessConfiguration, WitnessConfigurationError> {
    let signed = read_envelope(document)?;
    if !signed
        .signatures
        .iter()
        .any(|signature| signature_verifies(signed.payload.as_bytes(), signature, trusted_keys))
    {
        return Err(WitnessConfigurationError::SignatureInvalid);
    }
    read_payload(&signed.payload)
}

/// Check publication shape without trusted keys. This does not establish
/// authenticity; the server uses it to refuse unsigned or malformed files.
pub fn check_configuration_served_shape(
    document: &[u8],
) -> Result<WitnessConfiguration, WitnessConfigurationError> {
    let signed = read_envelope(document)?;
    read_payload(&signed.payload)
}

fn read_envelope(document: &[u8]) -> Result<SignedAcceptedList, WitnessConfigurationError> {
    if document.len() > WITNESS_CONFIGURATION_MAX_BYTES {
        return Err(WitnessConfigurationError::TooLarge);
    }
    let value: serde_json::Value =
        serde_json::from_slice(document).map_err(|_| WitnessConfigurationError::Malformed)?;
    if !value.is_object()
        || !value
            .get("signatures")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|signatures| signatures.iter().all(serde_json::Value::is_object))
    {
        return Err(WitnessConfigurationError::Malformed);
    }
    // Deserialize the original bytes so duplicate struct fields are rejected,
    // rather than silently collapsing them through the intermediate JSON map.
    let signed: SignedAcceptedList =
        serde_json::from_slice(document).map_err(|_| WitnessConfigurationError::Malformed)?;
    let hex_of_len =
        |text: &str, len: usize| text.len() == len && text.bytes().all(|b| b.is_ascii_hexdigit());
    if signed.signatures.is_empty()
        || signed.signatures.len() > WITNESS_ACCEPTED_LIST_MAX_SIGNATURES
        || !signed.signatures.iter().all(|signature| {
            hex_of_len(&signature.key_id, 16) && hex_of_len(&signature.signature, 128)
        })
    {
        return Err(WitnessConfigurationError::Malformed);
    }
    Ok(signed)
}

fn read_payload(payload: &str) -> Result<WitnessConfiguration, WitnessConfigurationError> {
    let value: serde_json::Value =
        serde_json::from_str(payload).map_err(|_| WitnessConfigurationError::Malformed)?;
    if !value.is_object() {
        return Err(WitnessConfigurationError::Malformed);
    }
    if value.get("schema").and_then(serde_json::Value::as_str) != Some(WITNESS_CONFIGURATION_SCHEMA)
    {
        return Err(WitnessConfigurationError::SchemaUnsupported);
    }
    // serde structs also accept positional arrays; forbid those at each level.
    if !value
        .get("endpoints")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|endpoints| {
            endpoints.iter().all(|endpoint| {
                endpoint.is_object()
                    && endpoint
                        .get("entries")
                        .and_then(serde_json::Value::as_array)
                        .is_some_and(|entries| entries.iter().all(serde_json::Value::is_object))
            })
        })
    {
        return Err(WitnessConfigurationError::Malformed);
    }
    let configuration: WitnessConfiguration =
        serde_json::from_str(payload).map_err(|_| WitnessConfigurationError::Malformed)?;
    configuration.validate()?;
    Ok(configuration)
}

fn normalize_url(url: &str) -> &str {
    url.trim_end_matches('/')
}

fn strict_https_url(text: &str) -> bool {
    if text.len() > 2048
        || text
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || matches!(c, '\\' | '?' | '#'))
    {
        return false;
    }
    let Some(rest) = text.strip_prefix("https://") else {
        return false;
    };
    let authority = rest.split('/').next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') {
        return false;
    }
    url::Url::parse(text).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::witness_accepted_list::{
        ListSignature, assemble_signed_list, key_id_for, verify_signed_list,
    };
    use chrono::TimeZone as _;
    use ring::signature::{Ed25519KeyPair, KeyPair as _};

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, day, 0, 0, 0).unwrap()
    }

    fn key() -> Ed25519KeyPair {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
        Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
    }

    fn public(key: &Ed25519KeyPair) -> [u8; 32] {
        key.public_key().as_ref().try_into().unwrap()
    }

    fn configuration() -> WitnessConfiguration {
        WitnessConfiguration {
            schema: WITNESS_CONFIGURATION_SCHEMA.into(),
            version: 3,
            issued_at: at(5),
            enrollment_urls: vec![
                "https://old.example".into(),
                "https://witness.example".into(),
            ],
            endpoints: vec![WitnessEndpoint {
                url: "https://witness.example/".into(),
                entries: vec![AcceptedEntry {
                    measurements: format!(
                        "mrtd={},mrconfigid={}",
                        "aa".repeat(48),
                        "bb".repeat(48)
                    ),
                    signing_address: format!("0x{}", "11".repeat(20)),
                    not_before: at(5),
                    not_after: at(20),
                    commit: Some("85173de19".into()),
                }],
            }],
        }
    }

    fn sign_payload(payload: String, key: &Ed25519KeyPair) -> Vec<u8> {
        let signature = ListSignature {
            key_id: key_id_for(&public(key)),
            signature: hex::encode(key.sign(payload.as_bytes()).as_ref()),
        };
        assemble_signed_list(payload, vec![signature])
    }

    #[test]
    fn signed_configuration_preserves_order_bytes_and_old_enrollment_anchors() {
        let mut configuration = configuration();
        configuration.endpoints.push(WitnessEndpoint {
            url: "https://old.example".into(),
            entries: configuration.endpoints[0].entries.clone(),
        });
        let payload = configuration.canonical_payload().unwrap();
        let key = key();
        let document = sign_payload(payload, &key);
        let verified = verify_signed_configuration(&document, &[public(&key)]).unwrap();
        assert_eq!(verified, configuration);
        assert_eq!(
            check_configuration_served_shape(&document).unwrap(),
            configuration
        );
        assert!(verified.names_witness("https://old.example///"));
        assert!(!verified.names_witness(" https://old.example"));
        assert!(!verified.names_witness("https://unrelated.example"));
        assert_eq!(verified.endpoints[0].url, "https://witness.example/");
        assert_eq!(verified.endpoints[0].entries_valid_at(at(4)).count(), 0);
        assert_eq!(verified.endpoints[0].entries_valid_at(at(5)).count(), 1);
        assert_eq!(verified.endpoints[0].entries_valid_at(at(20)).count(), 0);
        assert!(verify_signed_list(&document, &[public(&key)]).is_err());
    }

    #[test]
    fn tampering_wrong_keys_and_untrusted_payloads_are_refused_before_interpretation() {
        let key = key();
        let document = sign_payload(configuration().canonical_payload().unwrap(), &key);
        assert_eq!(
            verify_signed_configuration(&document, &[]),
            Err(WitnessConfigurationError::SignatureInvalid)
        );
        assert_eq!(
            verify_signed_configuration(&document, &[public(&super::tests::key())]),
            Err(WitnessConfigurationError::SignatureInvalid)
        );
        let mut value: serde_json::Value = serde_json::from_slice(&document).unwrap();
        value["payload"] = "not even JSON".into();
        let altered = serde_json::to_vec(&value).unwrap();
        assert_eq!(
            verify_signed_configuration(&altered, &[public(&key)]),
            Err(WitnessConfigurationError::SignatureInvalid)
        );
        assert_eq!(
            verify_signed_configuration(&sign_payload("not JSON".into(), &key), &[public(&key)]),
            Err(WitnessConfigurationError::Malformed)
        );
        let mut payload = serde_json::to_value(configuration()).unwrap();
        payload["schema"] = "future".into();
        assert_eq!(
            verify_signed_configuration(&sign_payload(payload.to_string(), &key), &[public(&key)]),
            Err(WitnessConfigurationError::SchemaUnsupported)
        );
    }

    #[test]
    fn unknown_fields_and_malformed_signed_payloads_fail_closed() {
        let key = key();
        for mutate in [
            |v: &mut serde_json::Value| v["future"] = true.into(),
            |v: &mut serde_json::Value| v["endpoints"][0]["future"] = true.into(),
            |v: &mut serde_json::Value| v["endpoints"][0]["entries"][0]["future"] = true.into(),
            |v: &mut serde_json::Value| v["endpoints"][0]["entries"][0]["not_after"] = "bad".into(),
            |v: &mut serde_json::Value| {
                v["endpoints"][0] = serde_json::json!(["https://witness.example", []])
            },
            |v: &mut serde_json::Value| v["endpoints"][0]["entries"][0] = serde_json::json!([]),
            |v: &mut serde_json::Value| v["enrollment_urls"] = true.into(),
            |v: &mut serde_json::Value| v["version"] = (-1).into(),
            |v: &mut serde_json::Value| v["issued_at"] = serde_json::Value::Null,
        ] {
            let mut value = serde_json::to_value(configuration()).unwrap();
            mutate(&mut value);
            let signed = sign_payload(value.to_string(), &key);
            assert_eq!(
                verify_signed_configuration(&signed, &[public(&key)]),
                Err(WitnessConfigurationError::Malformed)
            );
            assert_eq!(
                check_configuration_served_shape(&signed),
                Err(WitnessConfigurationError::Malformed)
            );
        }
        let signed = sign_payload("[]".into(), &key);
        assert_eq!(
            verify_signed_configuration(&signed, &[public(&key)]),
            Err(WitnessConfigurationError::Malformed)
        );
    }

    #[test]
    fn duplicate_fields_in_payload_envelope_and_signatures_are_refused() {
        let key = key();
        let canonical = configuration().canonical_payload().unwrap();
        let duplicate_payload =
            canonical.replacen("\"version\":3", "\"version\":2,\"version\":3", 1);
        let signed = sign_payload(duplicate_payload, &key);
        assert_eq!(
            verify_signed_configuration(&signed, &[public(&key)]),
            Err(WitnessConfigurationError::Malformed)
        );
        let document = sign_payload(canonical, &key);
        let text = String::from_utf8(document).unwrap();
        for duplicate in [
            text.replacen("\"payload\":", "\"payload\":\"\",\"payload\":", 1),
            text.replacen(
                "\"key_id\":",
                "\"key_id\":\"0000000000000000\",\"key_id\":",
                1,
            ),
        ] {
            assert_eq!(
                check_configuration_served_shape(duplicate.as_bytes()),
                Err(WitnessConfigurationError::Malformed)
            );
        }
    }

    #[test]
    fn unsigned_malformed_or_oversized_envelopes_are_never_served() {
        let key = key();
        let document = sign_payload(configuration().canonical_payload().unwrap(), &key);
        for mutate in [
            |v: &mut serde_json::Value| v["future"] = true.into(),
            |v: &mut serde_json::Value| v["signatures"] = serde_json::json!([]),
            |v: &mut serde_json::Value| v["signatures"][0]["future"] = true.into(),
            |v: &mut serde_json::Value| v["signatures"][0]["signature"] = "00".into(),
            |v: &mut serde_json::Value| v["signatures"][0]["key_id"] = "zzzzzzzzzzzzzzzz".into(),
            |v: &mut serde_json::Value| {
                v["signatures"] = serde_json::json!([["0123456789abcdef", "00".repeat(64)]])
            },
            |v: &mut serde_json::Value| {
                v["signatures"] = serde_json::Value::Array(vec![v["signatures"][0].clone(); 5])
            },
        ] {
            let mut value: serde_json::Value = serde_json::from_slice(&document).unwrap();
            mutate(&mut value);
            let bytes = serde_json::to_vec(&value).unwrap();
            assert_eq!(
                check_configuration_served_shape(&bytes),
                Err(WitnessConfigurationError::Malformed)
            );
            assert_eq!(
                verify_signed_configuration(&bytes, &[public(&key)]),
                Err(WitnessConfigurationError::Malformed)
            );
        }
        assert_eq!(
            check_configuration_served_shape(b"[]"),
            Err(WitnessConfigurationError::Malformed)
        );
        assert_eq!(
            check_configuration_served_shape(&vec![b' '; WITNESS_CONFIGURATION_MAX_BYTES + 1]),
            Err(WitnessConfigurationError::TooLarge)
        );
    }

    #[test]
    fn urls_are_strict_https_and_preserved_without_parser_normalization() {
        for url in [
            "http://witness.example",
            "https:///witness.example",
            "https://",
            "https:////host",
            "https://user@host",
            "https://@host",
            "https://user:password@host",
            "https://host?",
            "https://host#",
            "https://host/?x",
            "https://host/#x",
            "https://host\\other",
            "https://host/\npath",
            "https://host/\tpath",
            " https://host",
            "https://host/ ",
            "https://host/\u{7f}",
            "https://host/\u{2003}",
            "https://host:bad",
            "https://[bad]",
        ] {
            let mut configuration = configuration();
            configuration.enrollment_urls = vec![url.into()];
            configuration.endpoints.clear();
            assert_eq!(
                configuration.validate(),
                Err(WitnessConfigurationError::Invalid),
                "URL accepted: {url:?}"
            );
        }
        let mut configuration = configuration();
        let url = format!("https://host/{}", "x".repeat(2048 - 13));
        assert_eq!(url.len(), 2048);
        configuration.enrollment_urls = vec![url.clone()];
        configuration.endpoints = vec![WitnessEndpoint {
            url: url.clone(),
            entries: vec![],
        }];
        assert!(configuration.validate().is_ok());
        assert_eq!(configuration.endpoints[0].url, url);
        configuration.enrollment_urls[0].push('x');
        assert_eq!(
            configuration.validate(),
            Err(WitnessConfigurationError::Invalid)
        );
        assert!(strict_https_url("https://[::1]:8443/witness/"));
        assert!(strict_https_url("https://HOST.example:443/a%20b/"));
    }

    #[test]
    fn duplicates_membership_entry_rules_and_bounds_are_enforced() {
        for mutate in [
            |c: &mut WitnessConfiguration| c.version = 0,
            |c: &mut WitnessConfiguration| c.enrollment_urls.clear(),
            |c: &mut WitnessConfiguration| c.enrollment_urls.push("https://old.example///".into()),
            |c: &mut WitnessConfiguration| c.endpoints.push(c.endpoints[0].clone()),
            |c: &mut WitnessConfiguration| c.endpoints[0].url = "https://unlisted.example".into(),
            |c: &mut WitnessConfiguration| c.endpoints[0].entries[0].not_after = at(5),
            |c: &mut WitnessConfiguration| {
                c.endpoints[0].entries[0].signing_address = "0x00".into()
            },
            |c: &mut WitnessConfiguration| c.endpoints[0].entries[0].measurements = "".into(),
            |c: &mut WitnessConfiguration| {
                c.endpoints[0].entries[0].commit = Some("not-a-commit".into())
            },
            |c: &mut WitnessConfiguration| {
                c.endpoints[0].entries = vec![c.endpoints[0].entries[0].clone(); 33]
            },
            |c: &mut WitnessConfiguration| {
                c.enrollment_urls = (0..33).map(|i| format!("https://w{i}.example")).collect()
            },
            |c: &mut WitnessConfiguration| {
                c.enrollment_urls = (0..9).map(|i| format!("https://w{i}.example")).collect();
                c.endpoints = c
                    .enrollment_urls
                    .iter()
                    .map(|url| WitnessEndpoint {
                        url: url.clone(),
                        entries: vec![],
                    })
                    .collect();
            },
        ] {
            let mut configuration = configuration();
            mutate(&mut configuration);
            assert_eq!(
                configuration.validate(),
                Err(WitnessConfigurationError::Invalid)
            );
        }
        let mut maximum = configuration();
        maximum.enrollment_urls = (0..32).map(|i| format!("https://w{i}.example")).collect();
        maximum.endpoints = maximum.enrollment_urls[..8]
            .iter()
            .map(|url| WitnessEndpoint {
                url: url.clone(),
                entries: vec![maximum.endpoints[0].entries[0].clone(); 32],
            })
            .collect();
        assert!(maximum.validate().is_ok());
    }

    #[test]
    fn explicit_withdrawal_is_signed_and_legacy_schema_stays_separate() {
        let mut withdrawn = configuration();
        withdrawn.endpoints.clear();
        let key = key();
        let document = sign_payload(withdrawn.canonical_payload().unwrap(), &key);
        assert_eq!(
            verify_signed_configuration(&document, &[public(&key)]).unwrap(),
            withdrawn
        );
        let legacy = AcceptedList {
            schema: WITNESS_ACCEPTED_LIST_SCHEMA.into(),
            version: 3,
            issued_at: at(5),
            witness_url: "http://legacy.example".into(),
            entries: vec![],
        };
        let legacy_document = sign_payload(legacy.canonical_payload().unwrap(), &key);
        assert_eq!(
            verify_signed_list(&legacy_document, &[public(&key)]).unwrap(),
            legacy
        );
        assert_eq!(
            verify_signed_configuration(&legacy_document, &[public(&key)]),
            Err(WitnessConfigurationError::SchemaUnsupported)
        );
    }
}
