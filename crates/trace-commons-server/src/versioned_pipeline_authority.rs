// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Authority and privacy dependencies for the versioned pipeline.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use trace_commons_protocol::trace_contribution::PrivacyFilterAdapter;
use trace_commons_protocol::trace_contribution::{
    ResidualRiskCondition, TraceContributionEnvelope, rescrub_trace_envelope,
};

use crate::trace_authority::SubmissionAuthority;

pub const PIPELINE_AUTHORITY_CONTROL_MISSING_LABEL: &str = "authority_control_missing";
pub const PIPELINE_PRIVACY_CONTROL_MISSING_LABEL: &str = "privacy_control_missing";
pub const PIPELINE_PRIVACY_CLASSIFICATION_FAILED_LABEL: &str = "privacy_classification_failed";

pub trait PipelineAuthorityProvider: Send + Sync {
    fn authority_for_tenant(&self, tenant_id: &str) -> Option<SubmissionAuthority>;
    fn dependency_identity(&self) -> &str;
    fn production_qualified(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone)]
pub struct StaticPipelineAuthorityProvider {
    authorities: BTreeMap<String, SubmissionAuthority>,
    fallback: Option<SubmissionAuthority>,
    identity: String,
}

impl StaticPipelineAuthorityProvider {
    pub fn new(
        authorities: BTreeMap<String, SubmissionAuthority>,
        identity: impl Into<String>,
    ) -> Self {
        Self {
            authorities,
            fallback: None,
            identity: identity.into(),
        }
    }

    #[doc(hidden)]
    pub fn test_only(fallback: SubmissionAuthority) -> Self {
        Self {
            authorities: BTreeMap::new(),
            fallback: Some(fallback),
            identity: "static_authority_test_only".to_string(),
        }
    }
}

impl PipelineAuthorityProvider for StaticPipelineAuthorityProvider {
    fn authority_for_tenant(&self, tenant_id: &str) -> Option<SubmissionAuthority> {
        self.authorities
            .get(tenant_id)
            .cloned()
            .or_else(|| self.fallback.clone())
    }

    fn dependency_identity(&self) -> &str {
        &self.identity
    }
}

#[async_trait]
pub trait PipelinePrivacyBoundary: Send + Sync {
    async fn rescrub(
        &self,
        envelope: &mut TraceContributionEnvelope,
    ) -> anyhow::Result<Vec<ResidualRiskCondition>>;

    fn dependency_identity(&self) -> &str;
    fn is_production_compatible(&self) -> bool;
    fn production_qualified(&self) -> bool {
        false
    }
}

pub struct DeterministicPipelinePrivacyBoundary;

#[async_trait]
impl PipelinePrivacyBoundary for DeterministicPipelinePrivacyBoundary {
    async fn rescrub(
        &self,
        envelope: &mut TraceContributionEnvelope,
    ) -> anyhow::Result<Vec<ResidualRiskCondition>> {
        rescrub_trace_envelope(envelope).map_err(Into::into)
    }

    fn dependency_identity(&self) -> &str {
        "deterministic_privacy_test_only"
    }

    fn is_production_compatible(&self) -> bool {
        false
    }
}

pub struct ClassifierRedactorPipelinePrivacyBoundary {
    adapter: Arc<dyn PrivacyFilterAdapter>,
    policy: trace_commons_protocol::trace_contribution::PiiClassifyPolicy,
    identity: String,
}

impl ClassifierRedactorPipelinePrivacyBoundary {
    pub fn new(
        adapter: Arc<dyn PrivacyFilterAdapter>,
        policy: trace_commons_protocol::trace_contribution::PiiClassifyPolicy,
        identity: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let identity = identity.into();
        anyhow::ensure!(
            !identity.trim().is_empty(),
            PIPELINE_PRIVACY_CONTROL_MISSING_LABEL
        );
        Ok(Self {
            adapter,
            policy,
            identity,
        })
    }
}

#[async_trait]
impl PipelinePrivacyBoundary for ClassifierRedactorPipelinePrivacyBoundary {
    async fn rescrub(
        &self,
        envelope: &mut TraceContributionEnvelope,
    ) -> anyhow::Result<Vec<ResidualRiskCondition>> {
        let mut basis = rescrub_trace_envelope(envelope)?;
        let classifier_basis =
            trace_commons_protocol::trace_contribution::rescrub_envelope_prose_pii_with(
                self.adapter.as_ref(),
                envelope,
                self.policy,
            )
            .await
            .map_err(|_| anyhow::anyhow!(PIPELINE_PRIVACY_CLASSIFICATION_FAILED_LABEL))?;
        for condition in classifier_basis {
            if !basis.contains(&condition) {
                basis.push(condition);
            }
        }
        Ok(basis)
    }

    fn dependency_identity(&self) -> &str {
        &self.identity
    }

    fn is_production_compatible(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use trace_commons_protocol::trace_contribution::{
        DeterministicTraceRedactor, PiiClassifyPolicy, RawTraceCaptureTurn, RawTraceContribution,
        RecordedTraceContributionOptions, RedactionReport, ResidualPiiRisk,
        SafePrivacyFilterRedaction, SafePrivacyFilterSummary, TraceContributionError,
        TraceRedactor,
    };

    struct PersonClassifier;

    #[async_trait]
    impl PrivacyFilterAdapter for PersonClassifier {
        async fn redact_text(
            &self,
            text: &str,
        ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
            if !text.contains("Jane Doe") {
                return Ok(None);
            }
            let mut report = RedactionReport::default();
            report.counts.insert("privacy_filter:person".to_string(), 1);
            report.pii_labels_present.push("person".to_string());
            Ok(Some(SafePrivacyFilterRedaction {
                private_edits: None,
                redacted_text: text.replace("Jane Doe", "[REDACTED:person]"),
                summary: SafePrivacyFilterSummary {
                    schema_version: 1,
                    output_mode: "redacted_text_only".to_string(),
                    span_count: 1,
                    by_label: BTreeMap::from([("person".to_string(), 1)]),
                    decoded_mismatch: false,
                    classify_policy: None,
                    events_examined: 0,
                    events_skipped_by_policy: 0,
                },
                report,
            }))
        }
    }

    async fn envelope_with_text(text: &str) -> TraceContributionEnvelope {
        let now = Utc::now();
        let raw = RawTraceContribution::from_capture_turns(
            &[RawTraceCaptureTurn {
                user_input: text.to_string(),
                response: None,
                tool_calls: Vec::new(),
                started_at: now,
                completed_at: Some(now),
                state: Some("complete".to_string()),
            }],
            RecordedTraceContributionOptions {
                include_message_text: true,
                ..RecordedTraceContributionOptions::default()
            },
        );
        let mut envelope = DeterministicTraceRedactor::try_default()
            .unwrap()
            .redact_trace(raw)
            .await
            .unwrap();
        envelope.privacy.residual_pii_risk = ResidualPiiRisk::Low;
        envelope
    }

    fn boundary() -> ClassifierRedactorPipelinePrivacyBoundary {
        ClassifierRedactorPipelinePrivacyBoundary::new(
            Arc::new(PersonClassifier),
            PiiClassifyPolicy::AllEvents,
            "person_classifier_test",
        )
        .unwrap()
    }

    #[tokio::test]
    async fn ordinary_identifier_prefixes_do_not_add_privacy_findings() {
        for text in [
            "task-123",
            "risk-model",
            "disk-cache",
            "desk-layout",
            "mask-policy",
        ] {
            let mut envelope = envelope_with_text(text).await;
            let basis = boundary().rescrub(&mut envelope).await.unwrap();
            assert_eq!(envelope.privacy.residual_pii_risk, ResidualPiiRisk::Medium);
            assert_eq!(basis, vec![ResidualRiskCondition::ConsentContentFlag]);
            assert!(envelope.events.iter().any(|event| {
                event
                    .redacted_content
                    .as_deref()
                    .is_some_and(|content| content.contains(text))
            }));
        }
    }

    #[tokio::test]
    async fn classifier_pii_is_transformed_and_quarantinable() {
        let mut envelope = envelope_with_text("Send this to Jane Doe.").await;
        boundary().rescrub(&mut envelope).await.unwrap();
        assert!(envelope.events.iter().all(|event| {
            event
                .redacted_content
                .as_deref()
                .is_none_or(|content| !content.contains("Jane Doe"))
        }));
        assert!(envelope.events.iter().any(|event| {
            event
                .redacted_content
                .as_deref()
                .is_some_and(|content| content.contains("[REDACTED:person]"))
        }));
        assert!(envelope.privacy.residual_pii_risk >= ResidualPiiRisk::Medium);
    }

    #[tokio::test]
    async fn classifier_failure_fails_closed() {
        struct FailingClassifier;
        #[async_trait]
        impl PrivacyFilterAdapter for FailingClassifier {
            async fn redact_text(
                &self,
                _text: &str,
            ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
                Err(TraceContributionError::RedactionFailed {
                    reason: "unavailable".to_string(),
                })
            }
        }
        let boundary = ClassifierRedactorPipelinePrivacyBoundary::new(
            Arc::new(FailingClassifier),
            PiiClassifyPolicy::AllEvents,
            "failing_classifier_test",
        )
        .unwrap();
        let mut envelope = envelope_with_text("ordinary text").await;
        assert!(boundary.rescrub(&mut envelope).await.is_err());
    }
}
