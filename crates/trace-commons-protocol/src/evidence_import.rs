//! Bounded local imports. Parsing proves consistency, never admission or trust.
use crate::trace_contribution::{RawTraceContribution, TraceContributionEventType};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MAX_IMPORT_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_IMPORT_TRACE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_IMPORT_BODY_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceImport {
    pub schema_version: u32,
    pub trace: RawTraceContribution,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inference: Option<ImportedInference>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportedInference {
    pub exchange_id: String,
    pub coverage: InferenceCoverage,
    pub request_body: String,
    pub response_body: String,
    pub request_sha256: String,
    pub response_sha256: String,
    pub upstream_id: String,
    pub served_model: Option<String>,
    pub status: u16,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub receipt: ImportedReceipt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InferenceCoverage {
    FinalCallOnly,
}

/// Existing witness receipt shape; crypto and discriminator semantics remain
/// owned by trace-commons-attestation, not a second protocol verifier.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportedReceipt {
    pub text: String,
    pub signature: String,
    pub signing_address: String,
    pub signing_algo: String,
    pub signature_kind: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ImportError {
    #[error("import-too-large")]
    TooLarge,
    #[error("import-malformed")]
    Malformed,
    #[error("import-version-unsupported")]
    Version,
    #[error("import-event-reference-invalid")]
    EventReference,
    #[error("import-inference-ambiguous")]
    AmbiguousInference,
    #[error("import-body-digest-mismatch")]
    DigestMismatch,
    #[error("import-field-invalid")]
    InvalidField,
}

impl std::fmt::Debug for EvidenceImport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EvidenceImport")
            .field("events", &self.trace.events.len())
            .field("has_inference", &self.inference.is_some())
            .finish_non_exhaustive()
    }
}

/// A `std::io::Write` sink that counts serialized bytes without keeping them,
/// refusing as soon as the trace budget is passed. Used only to size a value
/// that is already in memory; it never touches the filesystem.
struct TraceByteBudget {
    remaining: usize,
    exceeded: bool,
}

impl std::io::Write for TraceByteBudget {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self.remaining.checked_sub(buf.len()) {
            Some(remaining) => {
                self.remaining = remaining;
                Ok(buf.len())
            }
            None => {
                self.exceeded = true;
                Err(std::io::Error::other("trace-over-budget"))
            }
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl EvidenceImport {
    pub fn parse(bytes: &[u8]) -> Result<Self, ImportError> {
        if bytes.len() > MAX_IMPORT_BYTES {
            return Err(ImportError::TooLarge);
        }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| ImportError::Malformed)?;
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), ImportError> {
        if self.schema_version != 1 {
            return Err(ImportError::Version);
        }
        // Measured through a counting sink rather than `to_vec`. The trace can
        // be the whole 16 MiB budget, and a buffer allocated purely to read its
        // `len()` would roughly double peak memory on every import, including
        // the common small one. The sink also stops the serializer the moment
        // the budget is passed, so a hostile document does not get to make us
        // walk all of it.
        let mut measured = TraceByteBudget {
            remaining: MAX_IMPORT_TRACE_BYTES,
            exceeded: false,
        };
        if let Err(error) = serde_json::to_writer(&mut measured, &self.trace) {
            // `exceeded` distinguishes our own early stop from a genuine
            // serialization failure; only the latter is malformed input.
            return Err(if measured.exceeded {
                ImportError::TooLarge
            } else {
                let _ = error;
                ImportError::Malformed
            });
        }
        let source = self
            .trace
            .ironclaw
            .feature_flags
            .get("agent")
            .ok_or(ImportError::InvalidField)?;
        if source.is_empty()
            || source.len() > 64
            || !source
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(ImportError::InvalidField);
        }
        let mut seen = std::collections::HashSet::new();
        if self.trace.events.is_empty() {
            return Err(ImportError::EventReference);
        }
        for event in &self.trace.events {
            if event.parent_event_id.is_some_and(|id| !seen.contains(&id))
                || !seen.insert(event.event_id)
            {
                return Err(ImportError::EventReference);
            }
        }
        if let Some(inference) = &self.inference {
            // The isolated call is appended only by the witness transport. A
            // companion HttpExchange would make current last-call semantics
            // ambiguous; it must not be selected by timestamp or source name.
            if self
                .trace
                .events
                .iter()
                .any(|e| e.event_type == TraceContributionEventType::HttpExchange)
            {
                return Err(ImportError::AmbiguousInference);
            }
            inference.validate()?;
        }
        Ok(())
    }
}

impl ImportedInference {
    pub fn validate(&self) -> Result<(), ImportError> {
        if self.request_body.len() > MAX_IMPORT_BODY_BYTES
            || self.response_body.len() > MAX_IMPORT_BODY_BYTES
        {
            return Err(ImportError::TooLarge);
        }
        if !opaque_id(&self.exchange_id)
            || !opaque_id(&self.upstream_id)
            || self
                .served_model
                .as_ref()
                .is_some_and(|s| s.is_empty() || s.len() > 256)
            || !(100..=599).contains(&self.status)
            || self.receipt.text.is_empty()
            || self.receipt.text.len() > 1024
            || self.receipt.signature.len() > 256
            || self.receipt.signing_address.len() > 130
            || self.receipt.signing_algo.len() > 32
            || self.receipt.signature_kind.len() > 32
        {
            return Err(ImportError::InvalidField);
        }
        if hex::encode(Sha256::digest(self.request_body.as_bytes())) != self.request_sha256
            || hex::encode(Sha256::digest(self.response_body.as_bytes())) != self.response_sha256
        {
            return Err(ImportError::DigestMismatch);
        }
        Ok(())
    }
}

fn opaque_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace_contribution::{RawTraceCaptureTurn, RecordedTraceContributionOptions};
    fn ordinary() -> EvidenceImport {
        let mut trace = RawTraceContribution::from_capture_turns(
            &[RawTraceCaptureTurn {
                user_input: "test".into(),
                response: None,
                tool_calls: Vec::new(),
                started_at: chrono::Utc::now(),
                completed_at: None,
                state: None,
            }],
            RecordedTraceContributionOptions {
                include_message_text: true,
                ..Default::default()
            },
        );
        trace
            .ironclaw
            .feature_flags
            .insert("agent".into(), "another-client".into());
        EvidenceImport {
            schema_version: 1,
            trace,
            inference: None,
        }
    }
    #[test]
    fn exact_document_byte_limit_is_accepted_and_one_more_is_refused() {
        let mut bytes = serde_json::to_vec(&ordinary()).unwrap();
        bytes.resize(MAX_IMPORT_BYTES, b' ');
        assert!(EvidenceImport::parse(&bytes).is_ok());
        bytes.push(b' ');
        assert!(matches!(
            EvidenceImport::parse(&bytes),
            Err(ImportError::TooLarge)
        ));
    }
    #[test]
    fn exact_trace_byte_budget_is_accepted_and_one_more_is_refused() {
        // The budget is measured through a counting sink rather than a second
        // buffer, so the boundary is worth pinning: an off-by-one in the sink
        // would silently move the limit, and nothing else measures the trace.
        let mut doc = ordinary();
        // Measure with the field already a string, so padding is the only
        // thing that changes size afterwards.
        doc.trace.events[0].content = Some(String::new());
        let baseline = serde_json::to_vec(&doc.trace).unwrap().len();
        let pad = MAX_IMPORT_TRACE_BYTES - baseline;
        doc.trace.events[0].content = Some("a".repeat(pad));
        // ASCII padding inside an existing JSON string costs exactly one byte
        // per character, so the serialized trace is now exactly the budget.
        assert_eq!(
            serde_json::to_vec(&doc.trace).unwrap().len(),
            MAX_IMPORT_TRACE_BYTES
        );
        assert_eq!(doc.validate(), Ok(()));
        doc.trace.events[0].content = Some("a".repeat(pad + 1));
        assert_eq!(doc.validate(), Err(ImportError::TooLarge));
    }
    #[test]
    fn future_version_and_forward_parent_reference_are_refused() {
        let mut doc = ordinary();
        doc.schema_version = 2;
        assert_eq!(doc.validate(), Err(ImportError::Version));
        doc.schema_version = 1;
        doc.trace.events[0].parent_event_id = Some(uuid::Uuid::new_v4());
        assert_eq!(doc.validate(), Err(ImportError::EventReference));
    }
    #[test]
    fn caller_claimed_invite_or_verified_flag_is_not_part_of_the_contract() {
        for field in [
            "invited",
            "attested",
            "verified",
            "file_path",
            "receipt_url",
        ] {
            let mut value = serde_json::to_value(ordinary()).unwrap();
            value[field] = serde_json::json!(true);
            assert!(matches!(
                EvidenceImport::parse(&serde_json::to_vec(&value).unwrap()),
                Err(ImportError::Malformed)
            ));
        }
    }
}
