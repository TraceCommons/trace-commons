// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Operator-owned connection descriptors and their versioned, length-framed identity.

use sha2::{Digest, Sha256};
use trace_commons_protocol::inference_connection::{
    ConnectionWitnessConfig, DISCLOSURE_VERSION, InferenceConnectionOffer, MAX_MEASUREMENT_BYTES,
    MAX_MEASUREMENTS, MAX_URL_BYTES, SelectInferenceConnection, SelectedInferenceConnection,
    valid_identifier,
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
#[derive(Clone, Debug)]
pub struct OperatorInferenceConnection {
    offer_id: String,
    provider_id: String,
    witness: ConnectionWitnessConfig,
    inference_receipt_endpoint: Option<String>,
    config_digest: String,
    revision: String,
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

        let mut config = Vec::new();
        config.extend_from_slice(b"trace-commons/inference-connection-config/v1\0");
        frame(&mut config, provider_id.as_bytes());
        frame(&mut config, disclosure_version.as_bytes());
        frame(&mut config, witness.url.as_bytes());
        frame(&mut config, witness.signing_address.as_bytes());
        config.extend_from_slice(&(witness.expected_measurements.len() as u32).to_be_bytes());
        for measurement in &witness.expected_measurements {
            frame(&mut config, measurement.as_bytes());
        }
        match &inference_receipt_endpoint {
            Some(endpoint) => {
                config.push(1);
                frame(&mut config, endpoint.as_bytes());
            }
            None => config.push(0),
        }
        let config_digest = digest(&config);
        let mut descriptor = b"trace-commons/inference-connection-offer/v1\0".to_vec();
        frame(&mut descriptor, offer_id.as_bytes());
        frame(&mut descriptor, provider_id.as_bytes());
        frame(&mut descriptor, disclosure_version.as_bytes());
        frame(&mut descriptor, config_digest.as_bytes());
        let revision = digest(&descriptor);
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

fn frame(output: &mut Vec<u8>, bytes: &[u8]) {
    output.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    output.extend_from_slice(bytes);
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
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
    fn framing_disambiguates_field_boundaries_and_catalog_controls_selection() {
        let mut left = Vec::new();
        frame(&mut left, b"ab");
        frame(&mut left, b"c");
        let mut right = Vec::new();
        frame(&mut right, b"a");
        frame(&mut right, b"bc");
        assert_ne!(left, right);
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
}
