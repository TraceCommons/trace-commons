//! Versioned, local descriptive findings for the unified Insights experience.
//!
//! These contracts confer no access, contribution, publication, or training
//! permission. Only local execution is supported. Remote providers, semantic
//! judgments, comparisons, missions, and reward settlement remain unimplemented.
//! A valid result establishes structural consistency, not truthful measurement.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

pub const INSIGHT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    Local,
}

/// Identity of an installed implementation, not a signed trust assertion.
/// The initial schema supports only deterministic descriptive counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderManifest {
    pub id: String,
    pub version: String,
    pub rubric_version: String,
    pub execution_mode: ExecutionMode,
    pub schema_version: u32,
}

impl ProviderManifest {
    pub fn first_party() -> Self {
        Self {
            id: "trace-commons-local".into(),
            version: "1".into(),
            rubric_version: "descriptive-counts-v1".into(),
            execution_mode: ExecutionMode::Local,
            schema_version: INSIGHT_SCHEMA_VERSION,
        }
    }

    fn validate(&self) -> Result<(), InsightValidationError> {
        if self.schema_version != INSIGHT_SCHEMA_VERSION {
            return Err(InsightValidationError::SchemaVersion);
        }
        if [&self.id, &self.version, &self.rubric_version]
            .iter()
            .any(|value| !is_label(value))
        {
            return Err(InsightValidationError::InvalidField);
        }
        Ok(())
    }
}

/// An opaque source identifier and SHA-256 of its exact input bytes.
/// Do not put source paths or trace content in this reference. The host resolves
/// it only within the user's explicitly selected, local evidence set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRef {
    pub id: String,
    pub source_digest: String,
}

/// Coverage counts the eligible observations whose value is actually available,
/// not the number of traces whose schema merely permits that field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Coverage {
    pub observed: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricId {
    /// Source sessions, not inferred task episodes.
    Sessions,
    Events,
    /// Native input/output categories; never inferred from text length.
    InputTokens,
    OutputTokens,
    ToolCalls,
    /// Observed failures among tool results with an explicit outcome.
    ToolFailures,
    /// Explicit task outcomes, not tool success or transcript assertions.
    KnownOutcomes,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InsightMetric {
    pub id: MetricId,
    /// Sum over observed inputs only. `None` means unknown, never zero.
    /// A partial sum must always be presented alongside its coverage.
    pub value: Option<u64>,
    pub coverage: Coverage,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InsightReport {
    pub schema_version: u32,
    pub provider: ProviderManifest,
    pub evidence: Vec<EvidenceRef>,
    pub metrics: Vec<InsightMetric>,
}

/// Errors contain safe labels only, never caller-supplied identifiers or data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InsightValidationError {
    #[error("insight-schema-unsupported")]
    SchemaVersion,
    #[error("insight-field-invalid")]
    InvalidField,
    #[error("insight-evidence-invalid")]
    InvalidEvidence,
    #[error("insight-evidence-unknown")]
    UnknownEvidence,
    #[error("insight-provider-incompatible")]
    IncompatibleProvider,
    #[error("insight-metric-duplicate")]
    DuplicateMetric,
    #[error("insight-coverage-invalid")]
    InvalidCoverage,
}

impl InsightReport {
    /// Structural validation only. Hosts accepting provider results must also
    /// use `validate_for` to bind provenance and evidence to their request.
    pub fn validate(&self) -> Result<(), InsightValidationError> {
        if self.schema_version != INSIGHT_SCHEMA_VERSION {
            return Err(InsightValidationError::SchemaVersion);
        }
        self.provider.validate()?;
        let evidence = evidence_index(&self.evidence)?;
        let mut metrics = BTreeSet::new();
        for metric in &self.metrics {
            if !metrics.insert(metric.id) {
                return Err(InsightValidationError::DuplicateMetric);
            }
            let coverage = metric.coverage;
            if coverage.observed > coverage.total
                || (metric.value.is_none() && coverage.observed != 0)
                || (metric.value.is_some()
                    && coverage.observed == 0
                    && !(coverage.total == 0 && metric.value == Some(0)))
            {
                return Err(InsightValidationError::InvalidCoverage);
            }
            if coverage.observed > 0 && metric.evidence_ids.is_empty() {
                return Err(InsightValidationError::InvalidEvidence);
            }
            let mut references = BTreeSet::new();
            for id in &metric.evidence_ids {
                if !evidence.contains_key(id.as_str()) {
                    return Err(InsightValidationError::UnknownEvidence);
                }
                if !references.insert(id) {
                    return Err(InsightValidationError::InvalidEvidence);
                }
            }
        }
        Ok(())
    }

    /// Bind a result to the installed provider/rubric and exact source digests
    /// selected by the host. A self-declared evidence catalog is not authority.
    /// The caller must resolve fresh digests and enforce deletion before reuse;
    /// this method neither reads files nor proves source freshness itself.
    pub fn validate_for(
        &self,
        provider: &ProviderManifest,
        allowed_evidence: &[EvidenceRef],
    ) -> Result<(), InsightValidationError> {
        self.validate()?;
        provider.validate()?;
        if &self.provider != provider {
            return Err(InsightValidationError::IncompatibleProvider);
        }
        let allowed = evidence_index(allowed_evidence)?;
        for reference in &self.evidence {
            match allowed.get(reference.id.as_str()) {
                None => return Err(InsightValidationError::UnknownEvidence),
                Some(digest) if *digest != reference.source_digest => {
                    return Err(InsightValidationError::InvalidEvidence);
                }
                Some(_) => {}
            }
        }
        Ok(())
    }
}

fn is_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

fn evidence_index(
    evidence: &[EvidenceRef],
) -> Result<BTreeMap<&str, &str>, InsightValidationError> {
    let mut index = BTreeMap::new();
    for reference in evidence {
        if !is_label(&reference.id)
            || reference.source_digest.len() != 64
            || !reference
                .source_digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || index
                .insert(reference.id.as_str(), reference.source_digest.as_str())
                .is_some()
        {
            return Err(InsightValidationError::InvalidEvidence);
        }
    }
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> InsightReport {
        InsightReport {
            schema_version: INSIGHT_SCHEMA_VERSION,
            provider: ProviderManifest::first_party(),
            evidence: vec![EvidenceRef {
                id: "source-1".into(),
                source_digest: "ab".repeat(32),
            }],
            metrics: vec![InsightMetric {
                id: MetricId::ToolFailures,
                value: Some(0),
                coverage: Coverage {
                    observed: 3,
                    total: 5,
                },
                evidence_ids: vec!["source-1".into()],
            }],
        }
    }

    #[test]
    fn unknown_and_observed_zero_round_trip_distinctly() {
        let zero = report();
        zero.validate().unwrap();
        let mut unknown = zero.clone();
        unknown.metrics[0].value = None;
        unknown.metrics[0].coverage.observed = 0;
        unknown.validate().unwrap();
        let zero_json = serde_json::to_vec(&zero).unwrap();
        let unknown_json = serde_json::to_vec(&unknown).unwrap();
        assert_ne!(zero_json, unknown_json);
        assert_eq!(
            serde_json::from_slice::<InsightReport>(&zero_json).unwrap(),
            zero
        );
        assert_eq!(
            serde_json::from_slice::<InsightReport>(&unknown_json).unwrap(),
            unknown
        );
    }

    #[test]
    fn rejects_unknown_and_duplicate_evidence_references() {
        let mut value = report();
        value.metrics[0].evidence_ids = vec!["invented".into()];
        assert_eq!(
            value.validate(),
            Err(InsightValidationError::UnknownEvidence)
        );
        value.metrics[0].evidence_ids = vec!["source-1".into(), "source-1".into()];
        assert_eq!(
            value.validate(),
            Err(InsightValidationError::InvalidEvidence)
        );
        value.metrics[0].evidence_ids.clear();
        assert_eq!(
            value.validate(),
            Err(InsightValidationError::InvalidEvidence)
        );
    }

    #[test]
    fn binds_source_digest_and_provider_to_host_selection() {
        let mut value = report();
        let provider = value.provider.clone();
        let evidence = value.evidence.clone();
        value.validate_for(&provider, &evidence).unwrap();
        assert_eq!(
            value.validate_for(&provider, &[]),
            Err(InsightValidationError::UnknownEvidence)
        );
        value.evidence[0].source_digest = "cd".repeat(32);
        assert_eq!(
            value.validate_for(&provider, &evidence),
            Err(InsightValidationError::InvalidEvidence)
        );
        value.evidence = evidence.clone();
        value.provider.id = "independent-local".into();
        assert_eq!(
            value.validate_for(&provider, &evidence),
            Err(InsightValidationError::IncompatibleProvider)
        );
        value.validate_for(&value.provider, &evidence).unwrap();
        value.provider = provider.clone();
        value.provider.rubric_version = "different-rubric".into();
        assert_eq!(
            value.validate_for(&provider, &evidence),
            Err(InsightValidationError::IncompatibleProvider)
        );
    }

    #[test]
    fn rejects_incompatible_schemas_and_unsupported_remote_mode() {
        let mut value = report();
        value.schema_version += 1;
        assert_eq!(value.validate(), Err(InsightValidationError::SchemaVersion));
        value.schema_version = INSIGHT_SCHEMA_VERSION;
        value.provider.schema_version += 1;
        assert_eq!(value.validate(), Err(InsightValidationError::SchemaVersion));
        let mut json = serde_json::to_value(report()).unwrap();
        json["provider"]["execution_mode"] = "remote".into();
        assert!(serde_json::from_value::<InsightReport>(json).is_err());
    }

    #[test]
    fn rejects_fabricated_zero_and_invalid_coverage() {
        let mut value = report();
        value.metrics[0].coverage.observed = 0;
        assert_eq!(
            value.validate(),
            Err(InsightValidationError::InvalidCoverage)
        );
        value.metrics[0].coverage.observed = 6;
        assert_eq!(
            value.validate(),
            Err(InsightValidationError::InvalidCoverage)
        );
        value.metrics[0].coverage.observed = 1;
        value.metrics[0].value = None;
        assert_eq!(
            value.validate(),
            Err(InsightValidationError::InvalidCoverage)
        );
        value.metrics[0].coverage = Coverage {
            observed: 0,
            total: 0,
        };
        value.metrics[0].value = Some(0);
        value.metrics[0].evidence_ids.clear();
        value.validate().unwrap();
    }

    #[test]
    fn rejects_duplicate_metrics_and_malformed_source_catalogs() {
        let mut value = report();
        value.metrics.push(value.metrics[0].clone());
        assert_eq!(
            value.validate(),
            Err(InsightValidationError::DuplicateMetric)
        );
        value.metrics.pop();
        value.evidence.push(value.evidence[0].clone());
        assert_eq!(
            value.validate(),
            Err(InsightValidationError::InvalidEvidence)
        );
        value.evidence.pop();
        value.evidence[0].source_digest = "not-a-digest".into();
        assert_eq!(
            value.validate(),
            Err(InsightValidationError::InvalidEvidence)
        );
    }
}
