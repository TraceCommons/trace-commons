// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Authority and privacy dependencies for the versioned pipeline.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use trace_commons_protocol::trace_contribution::{PrivacyFilterAdapter, PrivacyFilterBackendTag};
use trace_commons_protocol::trace_contribution::{
    ResidualRiskCondition, TraceContributionEnvelope, rescrub_trace_envelope,
};

use crate::trace_authority::SubmissionAuthority;

pub const PIPELINE_AUTHORITY_CONTROL_MISSING_LABEL: &str = "authority_control_missing";
/// The tenant's authority could not be read (its policy row, for a tenant
/// whose policy `main` reads from the database). Fails closed: never a
/// fallback to another source or to "no policy".
pub const PIPELINE_AUTHORITY_READ_FAILED_LABEL: &str = "pipeline_authority_read_failed";
pub const PIPELINE_PRIVACY_CONTROL_MISSING_LABEL: &str = "privacy_control_missing";
pub const PIPELINE_PRIVACY_CLASSIFICATION_FAILED_LABEL: &str = "privacy_classification_failed";

#[async_trait]
pub trait PipelineAuthorityProvider: Send + Sync {
    fn authority_for_tenant(&self, tenant_id: &str) -> Option<SubmissionAuthority>;
    /// The tenant's authority as of now, which is what the pipeline asks:
    /// at the receipt and before each `NoveltyUtility` credit check. A
    /// provider whose answer needs a read (a policy in the database)
    /// overrides this; an `Err` refuses with its label. The default is
    /// [`Self::authority_for_tenant`].
    async fn resolve_authority(
        &self,
        tenant_id: &str,
    ) -> anyhow::Result<Option<SubmissionAuthority>> {
        Ok(self.authority_for_tenant(tenant_id))
    }
    /// The one answer to whether this provider may serve a routed or
    /// drained tenant: `false` by default, and readiness fails closed on it.
    fn production_qualified(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone)]
pub struct StaticPipelineAuthorityProvider {
    authorities: BTreeMap<String, SubmissionAuthority>,
    fallback: Option<SubmissionAuthority>,
}

impl StaticPipelineAuthorityProvider {
    pub fn new(authorities: BTreeMap<String, SubmissionAuthority>) -> Self {
        Self {
            authorities,
            fallback: None,
        }
    }

    /// A test double: every tenant gets `fallback`. It is in the library,
    /// not behind `#[cfg(test)]`, because the integration tests and the
    /// ingest binary's tests link the library built without `cfg(test)`.
    /// A `StaticPipelineAuthorityProvider` is never production-qualified
    /// (`production_qualified` keeps the trait's `false`), so the
    /// qualification gate refuses a runtime that routes or drains a tenant
    /// through it (`each_pipeline_test_double_fails_the_qualification_gate`
    /// in the ingest binary's tests).
    #[doc(hidden)]
    pub fn test_only(fallback: SubmissionAuthority) -> Self {
        Self {
            authorities: BTreeMap::new(),
            fallback: Some(fallback),
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
}

#[async_trait]
pub trait PipelinePrivacyBoundary: Send + Sync {
    async fn rescrub(
        &self,
        envelope: &mut TraceContributionEnvelope,
    ) -> anyhow::Result<Vec<ResidualRiskCondition>>;

    /// The one answer to whether this boundary may serve a routed or
    /// drained tenant: `false` by default, and readiness fails closed on it.
    fn production_qualified(&self) -> bool {
        false
    }
    /// Whether `rescrub` runs a prose-PII classifier over the envelope, the
    /// filtering `main`'s `TRACE_COMMONS_REQUIRE_PRIVACY_FILTER` demands. It
    /// states what the boundary does, apart from its qualification: ingest
    /// refuses a runtime whose boundary does not, while that flag is set
    /// (Zaki review 1, round 2, finding 21).
    fn classifies_prose_pii(&self) -> bool {
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
}

/// The production privacy boundary: `main`'s deterministic rescrub, then the
/// prose-PII classifier the assembly builds it with, whose findings join the
/// residual-risk basis. It is the boundary `main`'s
/// `TRACE_COMMONS_REQUIRE_PRIVACY_FILTER` asks for when its adapter is a
/// real classifier backend: `backend` is the adapter's
/// `PrivacyFilterBackendTag`, as `main`'s `privacy_filter_adapter_from_env`
/// pairs them.
pub struct ClassifierRedactorPipelinePrivacyBoundary {
    adapter: Arc<dyn PrivacyFilterAdapter>,
    backend: PrivacyFilterBackendTag,
    policy: trace_commons_protocol::trace_contribution::PiiClassifyPolicy,
}

impl ClassifierRedactorPipelinePrivacyBoundary {
    pub fn new(
        adapter: Arc<dyn PrivacyFilterAdapter>,
        backend: PrivacyFilterBackendTag,
        policy: trace_commons_protocol::trace_contribution::PiiClassifyPolicy,
    ) -> Self {
        Self {
            adapter,
            backend,
            policy,
        }
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

    /// Qualified only over a real classifier backend (Zaki review 3, Z3-1):
    /// over the no-op adapter, the `None` backend's, it filters nothing.
    fn production_qualified(&self) -> bool {
        self.classifies_prose_pii()
    }

    /// Every `rescrub` runs the classifier this boundary was built with
    /// (`rescrub_envelope_prose_pii_with`), which classifies prose PII only
    /// when the backend is a real one: the `None` backend's adapter
    /// (`NoopPrivacyFilterAdapter`) finds nothing.
    fn classifies_prose_pii(&self) -> bool {
        !matches!(self.backend, PrivacyFilterBackendTag::None)
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
            PrivacyFilterBackendTag::Sidecar,
            PiiClassifyPolicy::AllEvents,
        )
    }

    /// `production_qualified` is the one answer a boundary gives (Zaki
    /// review 1, round 2, simplification), and Zaki review 3, Z3-1: the
    /// classifier-backed boundary is qualified, and classifies prose PII,
    /// only over a real classifier backend. Over the no-op adapter (the
    /// `None` backend's, `NoopPrivacyFilterAdapter`) it is neither, like the
    /// deterministic test boundary.
    #[test]
    fn each_privacy_boundary_reports_what_it_is() {
        let classifier = boundary();
        assert!(classifier.production_qualified());
        assert!(classifier.classifies_prose_pii());
        let noop = ClassifierRedactorPipelinePrivacyBoundary::new(
            Arc::new(trace_commons_protocol::trace_contribution::NoopPrivacyFilterAdapter),
            PrivacyFilterBackendTag::None,
            PiiClassifyPolicy::AllEvents,
        );
        assert!(!noop.production_qualified());
        assert!(!noop.classifies_prose_pii());
        assert!(!DeterministicPipelinePrivacyBoundary.production_qualified());
        assert!(!DeterministicPipelinePrivacyBoundary.classifies_prose_pii());
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
            PrivacyFilterBackendTag::Sidecar,
            PiiClassifyPolicy::AllEvents,
        );
        let mut envelope = envelope_with_text("ordinary text").await;
        assert!(boundary.rescrub(&mut envelope).await.is_err());
    }
}
