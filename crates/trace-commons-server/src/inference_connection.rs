// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Operator-owned connection descriptors and their versioned, length-framed identity.

use trace_commons_protocol::inference_connection::{
    ConnectionWitnessConfig, DISCLOSURE_VERSION, InferenceConnectionOffer, MAX_MEASUREMENT_BYTES,
    MAX_MEASUREMENTS, MAX_URL_BYTES, SelectInferenceConnection, SelectedInferenceConnection,
    config_digest, offer_revision, valid_identifier,
};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ConnectionConfigError {
    #[error("inference_connection_identifier_invalid")]
    Identifier,
    #[error("inference_connection_disclosure_invalid")]
    Disclosure,
    #[error("inference_connection_witness_url_invalid")]
    WitnessUrl,
    #[error("inference_connection_signing_address_invalid")]
    SigningAddress,
    #[error("inference_connection_measurements_invalid")]
    Measurements,
    #[error("inference_connection_receipt_url_invalid")]
    ReceiptUrl,
}

/// Constructed from operator configuration only. Client selection carries IDs
/// and digests, never a replacement URL or pin.
#[derive(Clone)]
pub struct OperatorInferenceConnection {
    offer_id: String,
    provider_id: String,
    witness: ConnectionWitnessConfig,
    inference_receipt_endpoint: Option<String>,
    config_digest: String,
    revision: String,
}

impl std::fmt::Debug for OperatorInferenceConnection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OperatorInferenceConnection")
            .field("offer_id", &self.offer_id)
            .field("provider_id", &self.provider_id)
            .field("witness", &self.witness)
            .field(
                "inference_receipt_endpoint",
                &self
                    .inference_receipt_endpoint
                    .as_ref()
                    .map(|_| "[redacted]"),
            )
            .field("config_digest", &self.config_digest)
            .field("revision", &self.revision)
            .finish()
    }
}

impl OperatorInferenceConnection {
    pub fn new(
        offer_id: String,
        provider_id: String,
        disclosure_version: &str,
        witness: ConnectionWitnessConfig,
        inference_receipt_endpoint: Option<String>,
    ) -> Result<Self, ConnectionConfigError> {
        if !valid_identifier(&offer_id) || !valid_identifier(&provider_id) {
            return Err(ConnectionConfigError::Identifier);
        }
        if disclosure_version != DISCLOSURE_VERSION {
            return Err(ConnectionConfigError::Disclosure);
        }
        if !valid_https_url(&witness.url) {
            return Err(ConnectionConfigError::WitnessUrl);
        }
        let address = witness
            .signing_address
            .strip_prefix("0x")
            .ok_or(ConnectionConfigError::SigningAddress)?;
        if address.len() != 40 || !address.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ConnectionConfigError::SigningAddress);
        }
        if witness.expected_measurements.is_empty()
            || witness.expected_measurements.len() > MAX_MEASUREMENTS
            || witness.expected_measurements.iter().any(|entry| {
                entry.is_empty()
                    || entry.len() > MAX_MEASUREMENT_BYTES
                    || !matches!(
                        trace_commons_attestation::measurements::ExpectedMeasurements::from_env_value(
                            Some(entry)
                        ),
                        Ok(Some(_))
                    )
            })
        {
            return Err(ConnectionConfigError::Measurements);
        }
        if inference_receipt_endpoint
            .as_deref()
            .is_some_and(|endpoint| !valid_https_url(endpoint))
        {
            return Err(ConnectionConfigError::ReceiptUrl);
        }

        // The encoding lives in the protocol crate so a client recomputes
        // exactly these digests from the witness material it is sent.
        let config_digest = config_digest(
            &provider_id,
            disclosure_version,
            &witness,
            inference_receipt_endpoint.as_deref(),
        );
        let revision = offer_revision(&offer_id, &provider_id, disclosure_version, &config_digest);
        Ok(Self {
            offer_id,
            provider_id,
            witness,
            inference_receipt_endpoint,
            config_digest,
            revision,
        })
    }

    pub fn offer(&self) -> InferenceConnectionOffer {
        InferenceConnectionOffer {
            offer_id: self.offer_id.clone(),
            revision: self.revision.clone(),
            provider_id: self.provider_id.clone(),
            disclosure_version: DISCLOSURE_VERSION.into(),
            config_digest: self.config_digest.clone(),
        }
    }

    pub fn matches_selection(&self, request: &SelectInferenceConnection) -> bool {
        request.validate().is_ok()
            && request.offer_id == self.offer_id
            && request.revision == self.revision
            && request.config_digest == self.config_digest
    }

    pub fn selected(&self, connection_id: Uuid, state_version: i64) -> SelectedInferenceConnection {
        SelectedInferenceConnection {
            connection_id,
            state_version,
            revision: self.revision.clone(),
            config_digest: self.config_digest.clone(),
            disclosure_version: DISCLOSURE_VERSION.into(),
            witness: self.witness.clone(),
            inference_receipt_endpoint: self.inference_receipt_endpoint.clone(),
        }
    }
}

fn valid_https_url(raw: &str) -> bool {
    if raw.is_empty() || raw.len() > MAX_URL_BYTES {
        return false;
    }
    let Ok(url) = reqwest::Url::parse(raw) else {
        return false;
    };
    url.scheme() == "https"
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn witness() -> ConnectionWitnessConfig {
        ConnectionWitnessConfig {
            url: "https://witness.example/v1".into(),
            signing_address: format!("0x{}", "ab".repeat(20)),
            expected_measurements: vec![format!("mrtd={}", "ab".repeat(48))],
        }
    }

    fn connection() -> OperatorInferenceConnection {
        OperatorInferenceConnection::new(
            "offer".into(),
            "near-ai".into(),
            DISCLOSURE_VERSION,
            witness(),
            None,
        )
        .unwrap()
    }

    #[test]
    fn rejects_bad_urls_pins_and_disclosure() {
        for bad in [
            "http://witness.example",
            "https://user@witness.example",
            "https://witness.example/?token=x",
            "https://witness.example/#x",
            &format!("https://{}", "a".repeat(MAX_URL_BYTES)),
        ] {
            let mut candidate = witness();
            candidate.url = bad.into();
            assert!(matches!(
                OperatorInferenceConnection::new(
                    "offer".into(),
                    "near-ai".into(),
                    DISCLOSURE_VERSION,
                    candidate,
                    None
                ),
                Err(ConnectionConfigError::WitnessUrl)
            ));
            assert!(matches!(
                OperatorInferenceConnection::new(
                    "offer".into(),
                    "near-ai".into(),
                    DISCLOSURE_VERSION,
                    witness(),
                    Some(bad.into())
                ),
                Err(ConnectionConfigError::ReceiptUrl)
            ));
        }
        let mut candidate = witness();
        candidate.signing_address = "invalid".into();
        assert!(matches!(
            OperatorInferenceConnection::new(
                "offer".into(),
                "near-ai".into(),
                DISCLOSURE_VERSION,
                candidate,
                None
            ),
            Err(ConnectionConfigError::SigningAddress)
        ));
        let mut candidate = witness();
        candidate.expected_measurements.clear();
        assert!(matches!(
            OperatorInferenceConnection::new(
                "offer".into(),
                "near-ai".into(),
                DISCLOSURE_VERSION,
                candidate,
                None
            ),
            Err(ConnectionConfigError::Measurements)
        ));
        for measurements in [
            vec!["bad-pin".into()],
            vec!["x".repeat(MAX_MEASUREMENT_BYTES + 1)],
            vec![witness().expected_measurements[0].clone(); MAX_MEASUREMENTS + 1],
        ] {
            let mut candidate = witness();
            candidate.expected_measurements = measurements;
            assert!(matches!(
                OperatorInferenceConnection::new(
                    "offer".into(),
                    "near-ai".into(),
                    DISCLOSURE_VERSION,
                    candidate,
                    None
                ),
                Err(ConnectionConfigError::Measurements)
            ));
        }
        assert!(matches!(
            OperatorInferenceConnection::new(
                "offer".into(),
                "near-ai".into(),
                "unknown",
                witness(),
                None
            ),
            Err(ConnectionConfigError::Disclosure)
        ));
    }

    #[test]
    fn every_config_field_changes_digest_and_revision() {
        let baseline = connection();
        let mut variants = Vec::new();
        variants.push(
            OperatorInferenceConnection::new(
                "offer".into(),
                "other".into(),
                DISCLOSURE_VERSION,
                witness(),
                None,
            )
            .unwrap(),
        );
        let mut changed = witness();
        changed.url.push_str("/other");
        variants.push(
            OperatorInferenceConnection::new(
                "offer".into(),
                "near-ai".into(),
                DISCLOSURE_VERSION,
                changed,
                None,
            )
            .unwrap(),
        );
        let mut changed = witness();
        changed.signing_address = format!("0x{}", "cd".repeat(20));
        variants.push(
            OperatorInferenceConnection::new(
                "offer".into(),
                "near-ai".into(),
                DISCLOSURE_VERSION,
                changed,
                None,
            )
            .unwrap(),
        );
        let mut changed = witness();
        changed
            .expected_measurements
            .push(format!("mrconfigid={}", "cd".repeat(48)));
        variants.push(
            OperatorInferenceConnection::new(
                "offer".into(),
                "near-ai".into(),
                DISCLOSURE_VERSION,
                changed,
                None,
            )
            .unwrap(),
        );
        variants.push(
            OperatorInferenceConnection::new(
                "offer".into(),
                "near-ai".into(),
                DISCLOSURE_VERSION,
                witness(),
                Some("https://receipt.example/v1".into()),
            )
            .unwrap(),
        );
        for variant in variants {
            assert_ne!(baseline.config_digest, variant.config_digest);
            assert_ne!(baseline.revision, variant.revision);
        }
        assert_eq!(baseline.config_digest, connection().config_digest);
        let renamed = OperatorInferenceConnection::new(
            "renamed".into(),
            "near-ai".into(),
            DISCLOSURE_VERSION,
            witness(),
            None,
        )
        .unwrap();
        assert_eq!(baseline.config_digest, renamed.config_digest);
        assert_ne!(baseline.revision, renamed.revision);
    }

    #[test]
    fn catalog_controls_selection() {
        let offer = connection();
        let published = offer.offer();
        let mut request = SelectInferenceConnection {
            offer_id: published.offer_id,
            revision: published.revision,
            config_digest: published.config_digest,
            disclosure_version: published.disclosure_version,
            idempotency_key: Uuid::new_v4(),
            expected_current_version: None,
        };
        assert!(offer.matches_selection(&request));
        request.config_digest = format!("sha256:{}", "0".repeat(64));
        assert!(!offer.matches_selection(&request));
    }

    #[test]
    fn disclosure_version_changes_config_digest_and_revision_encoding() {
        let current = config_digest("near-ai", DISCLOSURE_VERSION, &witness(), None);
        let future = config_digest("near-ai", "future-disclosure-v2", &witness(), None);
        assert_ne!(current, future);
        let current_revision = offer_revision("offer", "near-ai", DISCLOSURE_VERSION, &current);
        let future_revision = offer_revision("offer", "near-ai", "future-disclosure-v2", &future);
        assert_ne!(current_revision, future_revision);
        assert!(matches!(
            OperatorInferenceConnection::new(
                "offer".into(),
                "near-ai".into(),
                "future-disclosure-v2",
                witness(),
                None
            ),
            Err(ConnectionConfigError::Disclosure)
        ));
    }

    /// The digests the server publishes are the ones a client recomputes
    /// from the witness material: the protocol function over the same fields,
    /// and the byte-exact values pinned before the encoding moved there.
    #[test]
    fn published_digests_are_what_a_client_recomputes() {
        let receipt = "https://receipt.example/v1";
        let mut pinned = witness();
        pinned
            .expected_measurements
            .push(format!("rtmr3={}", "cd".repeat(48)));
        let configured = OperatorInferenceConnection::new(
            "offer".into(),
            "near-ai".into(),
            DISCLOSURE_VERSION,
            pinned.clone(),
            Some(receipt.into()),
        )
        .unwrap();
        let offer = configured.offer();
        assert_eq!(
            offer.config_digest,
            "sha256:383855ed69b5a6c0341f46429c4f9328a3b925d4152d687895e061faccf14d54"
        );
        assert_eq!(
            offer.revision,
            "sha256:87d8925d318f67e19233fc39dfa2ea82864c5d5951cc1bfa9591badcee93d3d5"
        );
        assert_eq!(
            offer.config_digest,
            config_digest("near-ai", DISCLOSURE_VERSION, &pinned, Some(receipt))
        );
        let selected = configured.selected(Uuid::new_v4(), 1);
        assert_eq!(selected.verify_digests("offer", "near-ai"), Ok(()));
        assert_eq!(selected.validate(), Ok(()));
    }

    /// The protocol crate's pin check is a copy of the attestation parser's
    /// rule, since a permissive client crate cannot take this dependency
    /// path. Hold the two to the same answer, whitespace-only included.
    #[test]
    fn protocol_pin_check_agrees_with_the_attestation_parser() {
        use trace_commons_attestation::measurements::ExpectedMeasurements;
        use trace_commons_protocol::inference_connection::valid_measurement_entry;
        let pin = |key: &str| format!("{key}={}", "ab".repeat(48));
        let cases = [
            pin("mrtd"),
            pin("MrConfigId"),
            pin("rtmr0"),
            pin("rtmr1"),
            pin("rtmr2"),
            pin("rtmr3"),
            format!("{} , {}", pin("mrtd"), pin("rtmr3")),
            format!("{},", pin("mrtd")),
            format!(",{}", pin("mrtd")),
            format!("  {}  ", pin("mrtd")),
            String::new(),
            " ".into(),
            "\t\n".into(),
            ",".into(),
            " , , ".into(),
            "mrtd".into(),
            "=".into(),
            format!("={}", "ab".repeat(48)),
            pin("mrtdd"),
            pin("compose_hash"),
            pin("os_image_hash"),
            format!("mrtd={}", "ab".repeat(47)),
            format!("mrtd={}0", "ab".repeat(48)),
            format!("mrtd={}", "zz".repeat(48)),
            format!("{},{}", pin("mrtd"), pin("MRTD")),
            format!("{},garbage", pin("mrtd")),
        ];
        for case in cases {
            let attestation = matches!(
                ExpectedMeasurements::from_env_value(Some(&case)),
                Ok(Some(_))
            );
            assert_eq!(
                valid_measurement_entry(&case),
                attestation,
                "parsers disagree on {case:?}"
            );
        }
    }

    #[test]
    fn operator_debug_hides_witness_and_receipt_values() {
        let mut sensitive = witness();
        sensitive.url = "https://private-witness.example/secret-path".into();
        sensitive.signing_address = format!("0x{}", "ab".repeat(20));
        sensitive.expected_measurements = vec![format!("mrtd={}", "cd".repeat(48))];
        let configured = OperatorInferenceConnection::new(
            "offer".into(),
            "near-ai".into(),
            DISCLOSURE_VERSION,
            sensitive.clone(),
            Some("https://private-receipt.example/secret-path".into()),
        )
        .unwrap();
        let output = format!("{configured:?}");
        for secret in [
            sensitive.url.as_str(),
            sensitive.signing_address.as_str(),
            sensitive.expected_measurements[0].as_str(),
            "private-receipt",
        ] {
            assert!(!output.contains(secret), "Debug leaked {secret}");
        }
        assert!(output.contains("[redacted]"));
    }
}
