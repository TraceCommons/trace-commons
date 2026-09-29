//! The NEAR AI measurement pins an ingest deployment enforces, as it publishes
//! them to a contributor device.
//!
//! `GET /v1/contributors/me/near-ai-measurements` answers with
//! [`NearAiMeasurementPins`]. A client that ties a NEAR AI receipt's signer to a
//! verified TDX quote pins the quote's image against exactly these sets, so
//! the device and ingest agree on which NEAR AI image counts.
//!
//! Pins only. Each set is the `key=<96 hex>,...` syntax of
//! `trace_commons_attestation::measurements::ExpectedMeasurements`; measurement
//! values are public image identifiers. No URL, key or account reference is
//! carried.
//!
//! **Only [`PinState::Configured`] with a non-empty `sets` pins anything.**
//! Every other state -- ingest has nothing configured, or its configuration
//! does not parse -- means *no image is pinned*, and a client must then
//! verify nothing. There is no state that means "anything goes".

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// The route this is served at.
pub const NEAR_AI_MEASUREMENTS_PATH: &str = "/v1/contributors/me/near-ai-measurements";

/// What ingest's own pin configuration is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinState {
    /// Ingest enforces these sets.
    Configured,
    /// Ingest has no NEAR AI measurements configured, so it pins nothing and
    /// neither may a client.
    Unconfigured,
    /// Ingest's configuration is present but does not parse. Ingest refuses
    /// under it, and so must a client.
    Invalid,
}

/// The published pin sets, with a digest that changes whenever they do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NearAiMeasurementPins {
    /// What ingest's configuration is.
    pub state: PinState,
    /// The pinned sets, each canonical `key=value,...`. Empty unless
    /// `state` is [`PinState::Configured`]. Any one matching is a pass.
    pub sets: Vec<String>,
    /// Lowercase hex SHA-256 over `state` and `sets` ([`Self::digest_of`]).
    /// A client compares it to tell that the set changed, and refuses a
    /// document whose digest does not match its own contents.
    pub digest: String,
}

impl NearAiMeasurementPins {
    /// Build a document, computing its digest. `sets` is dropped for any
    /// state but [`PinState::Configured`].
    #[must_use]
    pub fn new(state: PinState, sets: Vec<String>) -> Self {
        let sets = if state == PinState::Configured {
            sets
        } else {
            Vec::new()
        };
        let digest = Self::digest_of(state, &sets);
        Self {
            state,
            sets,
            digest,
        }
    }

    /// The digest of a state and its sets: SHA-256 over the state label, then
    /// each set, newline-terminated. Sets are canonical and contain no newline,
    /// so the encoding is unambiguous.
    #[must_use]
    pub fn digest_of(state: PinState, sets: &[String]) -> String {
        let label = match state {
            PinState::Configured => "configured",
            PinState::Unconfigured => "unconfigured",
            PinState::Invalid => "invalid",
        };
        let mut hasher = Sha256::new();
        hasher.update(label.as_bytes());
        hasher.update(b"\n");
        for set in sets {
            hasher.update(set.as_bytes());
            hasher.update(b"\n");
        }
        hex::encode(hasher.finalize())
    }

    /// The sets a client may pin against: the published ones when the
    /// document is configured and self-consistent, and **none** otherwise.
    #[must_use]
    pub fn usable_sets(&self) -> &[String] {
        let consistent = self.digest == Self::digest_of(self.state, &self.sets);
        if self.state == PinState::Configured && consistent {
            &self.sets
        } else {
            &[]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_configured_consistent_document_pins_anything() {
        let set = format!("mrconfigid={}", "aa".repeat(48));
        let configured = NearAiMeasurementPins::new(PinState::Configured, vec![set.clone()]);
        assert_eq!(configured.usable_sets(), std::slice::from_ref(&set));

        for state in [PinState::Unconfigured, PinState::Invalid] {
            let document = NearAiMeasurementPins::new(state, vec![set.clone()]);
            assert!(document.sets.is_empty(), "{state:?} carries no sets");
            assert!(document.usable_sets().is_empty(), "{state:?} pins nothing");
        }

        let empty = NearAiMeasurementPins::new(PinState::Configured, Vec::new());
        assert!(empty.usable_sets().is_empty());
    }

    /// A document whose sets were changed without its digest is not used.
    #[test]
    fn a_digest_that_does_not_match_its_contents_pins_nothing() {
        let mut document = NearAiMeasurementPins::new(
            PinState::Configured,
            vec![format!("mrtd={}", "aa".repeat(48))],
        );
        document.sets.push(format!("mrtd={}", "bb".repeat(48)));
        assert!(document.usable_sets().is_empty());
    }

    #[test]
    fn the_digest_moves_with_the_sets_and_the_state() {
        let a = NearAiMeasurementPins::new(PinState::Configured, vec!["mrtd=aa".into()]);
        let b = NearAiMeasurementPins::new(PinState::Configured, vec!["mrtd=bb".into()]);
        let none = NearAiMeasurementPins::new(PinState::Unconfigured, Vec::new());
        let bad = NearAiMeasurementPins::new(PinState::Invalid, Vec::new());
        assert_ne!(a.digest, b.digest);
        assert_ne!(none.digest, bad.digest);
        assert_eq!(a.digest.len(), 64);
    }

    #[test]
    fn the_wire_shape_is_labels_and_strings() {
        let document = NearAiMeasurementPins::new(PinState::Unconfigured, Vec::new());
        let json = serde_json::to_value(&document).unwrap();
        assert_eq!(json["state"], "unconfigured");
        assert_eq!(json["sets"], serde_json::json!([]));
        let back: NearAiMeasurementPins = serde_json::from_value(json).unwrap();
        assert_eq!(back, document);
    }
}
