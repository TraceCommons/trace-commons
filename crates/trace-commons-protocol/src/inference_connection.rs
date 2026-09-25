//! Wire contract for explicit selection of an operator-published inference connection.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const DISCLOSURE_VERSION: &str = "inference-connection-disclosure-v1";
pub const MAX_IDENTIFIER_BYTES: usize = 64;
pub const MAX_URL_BYTES: usize = 2048;
pub const MAX_MEASUREMENTS: usize = 16;
pub const MAX_MEASUREMENT_BYTES: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferenceConnectionOffer {
    pub offer_id: String,
    pub revision: String,
    pub provider_id: String,
    pub disclosure_version: String,
    pub config_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectInferenceConnection {
    pub offer_id: String,
    pub revision: String,
    pub config_digest: String,
    pub disclosure_version: String,
    pub idempotency_key: Uuid,
    pub expected_current_version: Option<i64>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionWitnessConfig {
    pub url: String,
    pub signing_address: String,
    pub expected_measurements: Vec<String>,
}

impl std::fmt::Debug for ConnectionWitnessConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConnectionWitnessConfig")
            .field("url", &"[redacted]")
            .field("signing_address", &"[redacted]")
            .field("expected_measurements", &"[redacted]")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectedInferenceConnection {
    pub connection_id: Uuid,
    pub state_version: i64,
    pub revision: String,
    pub config_digest: String,
    pub disclosure_version: String,
    pub witness: ConnectionWitnessConfig,
    pub inference_receipt_endpoint: Option<String>,
}

impl std::fmt::Debug for SelectedInferenceConnection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SelectedInferenceConnection")
            .field("connection_id", &"[redacted]")
            .field("state_version", &self.state_version)
            .field("revision", &"[redacted]")
            .field("config_digest", &"[redacted]")
            .field("disclosure_version", &"[redacted]")
            .field("witness", &self.witness)
            .field(
                "inference_receipt_endpoint",
                &self
                    .inference_receipt_endpoint
                    .as_ref()
                    .map(|_| "[redacted]"),
            )
            .finish()
    }
}

/// Bounded, syntax-only request validation. Authorization and catalog matching
/// must be performed by the server against its operator-controlled offers.
impl SelectInferenceConnection {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !valid_identifier(&self.offer_id) {
            return Err("offer_id_invalid");
        }
        if !valid_digest(&self.revision) || !valid_digest(&self.config_digest) {
            return Err("digest_invalid");
        }
        if self.disclosure_version != DISCLOSURE_VERSION {
            return Err("disclosure_version_invalid");
        }
        if self.idempotency_key.is_nil() {
            return Err("idempotency_key_invalid");
        }
        if self
            .expected_current_version
            .is_some_and(|version| version < 1)
        {
            return Err("expected_current_version_invalid");
        }
        Ok(())
    }
}

pub fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_IDENTIFIER_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

pub fn valid_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn wire_shapes_and_optional_receipt() {
        let offer = InferenceConnectionOffer {
            offer_id: "near-ai".into(),
            revision: format!("sha256:{}", "a".repeat(64)),
            provider_id: "near-ai".into(),
            disclosure_version: DISCLOSURE_VERSION.into(),
            config_digest: format!("sha256:{}", "b".repeat(64)),
        };
        let offer_value = serde_json::to_value(&offer).unwrap();
        assert_eq!(
            serde_json::from_value::<InferenceConnectionOffer>(offer_value).unwrap(),
            offer
        );
        let selected = SelectedInferenceConnection {
            connection_id: Uuid::nil(),
            state_version: 1,
            revision: offer.revision.clone(),
            config_digest: offer.config_digest.clone(),
            disclosure_version: offer.disclosure_version.clone(),
            witness: ConnectionWitnessConfig {
                url: "https://witness.example/v1".into(),
                signing_address: format!("0x{}", "ab".repeat(20)),
                expected_measurements: vec![format!("mrtd={}", "ab".repeat(48))],
            },
            inference_receipt_endpoint: None,
        };
        let selected_value = serde_json::to_value(&selected).unwrap();
        assert!(selected_value["inference_receipt_endpoint"].is_null());
        assert_eq!(
            serde_json::from_value::<SelectedInferenceConnection>(selected_value).unwrap(),
            selected
        );
        let witness_value = serde_json::to_value(&selected.witness).unwrap();
        assert_eq!(
            serde_json::from_value::<ConnectionWitnessConfig>(witness_value).unwrap(),
            selected.witness
        );
        let mut unknown_witness = serde_json::to_value(&selected.witness).unwrap();
        unknown_witness["provider_url"] = json!("https://attacker.example");
        assert!(serde_json::from_value::<ConnectionWitnessConfig>(unknown_witness).is_err());
    }

    #[test]
    fn selection_rejects_unknown_and_duplicate_fields_and_invalid_values() {
        let mut value = json!({"offer_id":"near-ai","revision":format!("sha256:{}", "a".repeat(64)),"config_digest":format!("sha256:{}", "b".repeat(64)),"disclosure_version":DISCLOSURE_VERSION,"idempotency_key":Uuid::new_v4(),"expected_current_version":null});
        let request: SelectInferenceConnection = serde_json::from_value(value.clone()).unwrap();
        assert!(request.validate().is_ok());
        value["url"] = json!("https://attacker.example");
        assert!(serde_json::from_value::<SelectInferenceConnection>(value).is_err());
        let duplicate = format!(
            r#"{{"offer_id":"near-ai","offer_id":"other","revision":"{}","config_digest":"{}","disclosure_version":"{}","idempotency_key":"{}","expected_current_version":null}}"#,
            request.revision, request.config_digest, DISCLOSURE_VERSION, request.idempotency_key
        );
        assert!(serde_json::from_str::<SelectInferenceConnection>(&duplicate).is_err());
        for change in [
            json!({"offer_id":"x".repeat(65)}),
            json!({"disclosure_version":"unknown"}),
            json!({"revision":"sha256:AB"}),
            json!({"expected_current_version":0}),
        ] {
            let mut altered: Value = serde_json::to_value(&request).unwrap();
            altered
                .as_object_mut()
                .unwrap()
                .extend(change.as_object().unwrap().clone());
            assert!(
                serde_json::from_value::<SelectInferenceConnection>(altered)
                    .unwrap()
                    .validate()
                    .is_err()
            );
        }
    }

    #[test]
    fn selected_and_witness_debug_hide_connection_details() {
        let witness = ConnectionWitnessConfig {
            url: "https://private-witness.example/secret-path".into(),
            signing_address: "0xprivate-signing-address".into(),
            expected_measurements: vec!["private-measurement-pin".into()],
        };
        let selected = SelectedInferenceConnection {
            connection_id: Uuid::new_v4(),
            state_version: 1,
            revision: "sha256:revision".into(),
            config_digest: "sha256:configuration".into(),
            disclosure_version: DISCLOSURE_VERSION.into(),
            witness: witness.clone(),
            inference_receipt_endpoint: Some("https://private-receipt.example/secret-path".into()),
        };
        let output = format!("{witness:?} {selected:?}");
        for secret in [
            "private-witness",
            "secret-path",
            "private-signing-address",
            "private-measurement-pin",
            "private-receipt",
            &selected.connection_id.to_string(),
        ] {
            assert!(!output.contains(secret), "Debug leaked {secret}");
        }
        assert!(output.contains("[redacted]"));
    }
}
