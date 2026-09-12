//! Dispatch seam for trusted local descriptive providers.
//!
//! Providers receive only event classifications and explicit tool outcomes,
//! plus opaque evidence references, never trace bodies or paths. This is an in-process interface, not a sandbox
//! for arbitrary plugins. Validation binds a result to its selected provider
//! and evidence; it does not establish measurement truth or grant remote access.

use serde::{Deserialize, Serialize};
use trace_commons_protocol::insights::{
    Coverage, EvidenceRef, INSIGHT_SCHEMA_VERSION, InsightMetric, InsightReport, MetricId,
    ProviderManifest,
};

use crate::source::{SessionEvent, SessionEventKind};

pub const PROVIDER_REQUEST_VERSION: u32 = 1;
pub const MAX_PROVIDER_EVENTS: usize = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisPurpose {
    PrivateDescriptive,
}

/// Constructed by the host from explicitly selected local evidence. Serialized
/// requests are data, not authorization to discover or read additional sources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderRequest {
    pub schema_version: u32,
    pub purpose: AnalysisPurpose,
    pub provider: ProviderManifest,
    pub evidence: EvidenceRef,
}

impl ProviderRequest {
    pub fn first_party(evidence: EvidenceRef) -> Self {
        Self {
            schema_version: PROVIDER_REQUEST_VERSION,
            purpose: AnalysisPurpose::PrivateDescriptive,
            provider: ProviderManifest::first_party(),
            evidence,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservedEventKind {
    Classified,
    Opaque,
    ToolCall,
    ToolResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventObservation {
    pub kind: ObservedEventKind,
    pub tool_success: Option<bool>,
}

impl From<&SessionEvent> for EventObservation {
    fn from(event: &SessionEvent) -> Self {
        let kind = match event.kind {
            SessionEventKind::Opaque => ObservedEventKind::Opaque,
            SessionEventKind::ToolCall => ObservedEventKind::ToolCall,
            SessionEventKind::ToolResult => ObservedEventKind::ToolResult,
            _ => ObservedEventKind::Classified,
        };
        Self {
            kind,
            tool_success: (kind == ObservedEventKind::ToolResult)
                .then_some(event.success)
                .flatten(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProviderError {
    #[error("insights-provider-request-invalid")]
    InvalidRequest,
    #[error("insights-provider-incompatible")]
    Incompatible,
    #[error("insights-provider-input-limit")]
    InputLimit,
    #[error("insights-provider-unavailable")]
    Unavailable,
    #[error("insights-provider-result-invalid")]
    InvalidResult,
}

/// Only trusted implementations explicitly selected by the host may run here.
/// Persisted reports remain first-party-only; this seam does not install or
/// select third-party implementations in the product.
/// Implementations must return safe typed failures and perform no external IO.
/// Remote evaluation requires a separate authorization and execution boundary.
pub trait LocalInsightProvider {
    fn manifest(&self) -> ProviderManifest;
    fn evaluate(
        &self,
        request: &ProviderRequest,
        events: &[EventObservation],
    ) -> Result<InsightReport, ProviderError>;
}

/// Host-built pairing of one selected evidence reference and its projection.
/// Fields and construction are restricted to the local analysis module so a
/// provider caller cannot pair arbitrary events with another source's digest.
/// The host still owns parsing and source authenticity; this is not attestation.
pub struct ProviderInput {
    request: ProviderRequest,
    events: Vec<EventObservation>,
}

impl ProviderInput {
    pub(super) fn first_party(evidence: EvidenceRef, events: &[SessionEvent]) -> Self {
        Self {
            request: ProviderRequest::first_party(evidence),
            events: events.iter().map(EventObservation::from).collect(),
        }
    }
}

pub fn dispatch(
    provider: &dyn LocalInsightProvider,
    input: &ProviderInput,
) -> Result<InsightReport, ProviderError> {
    dispatch_projected(provider, &input.request, &input.events)
}

fn dispatch_projected(
    provider: &dyn LocalInsightProvider,
    request: &ProviderRequest,
    events: &[EventObservation],
) -> Result<InsightReport, ProviderError> {
    if request.schema_version != PROVIDER_REQUEST_VERSION || events.is_empty() {
        return Err(ProviderError::InvalidRequest);
    }
    if events.len() > MAX_PROVIDER_EVENTS {
        return Err(ProviderError::InputLimit);
    }
    if events
        .iter()
        .any(|event| event.tool_success.is_some() && event.kind != ObservedEventKind::ToolResult)
    {
        return Err(ProviderError::InvalidRequest);
    }
    // Reuse protocol validation for manifest labels and digest syntax, before
    // invoking the implementation. Its self-declared manifest is not authority.
    let envelope = InsightReport {
        schema_version: INSIGHT_SCHEMA_VERSION,
        provider: request.provider.clone(),
        evidence: vec![request.evidence.clone()],
        metrics: vec![],
    };
    envelope
        .validate()
        .map_err(|_| ProviderError::InvalidRequest)?;
    if provider.manifest() != request.provider {
        return Err(ProviderError::Incompatible);
    }
    let result = provider.evaluate(request, events)?;
    result
        .validate_for(&request.provider, std::slice::from_ref(&request.evidence))
        .map_err(|_| ProviderError::InvalidResult)?;
    if result.evidence != envelope.evidence {
        return Err(ProviderError::InvalidResult);
    }
    Ok(result)
}

pub struct FirstPartyProvider;

impl LocalInsightProvider for FirstPartyProvider {
    fn manifest(&self) -> ProviderManifest {
        ProviderManifest::first_party()
    }

    fn evaluate(
        &self,
        request: &ProviderRequest,
        events: &[EventObservation],
    ) -> Result<InsightReport, ProviderError> {
        let count = events.len() as u64;
        let classified = events
            .iter()
            .filter(|e| e.kind != ObservedEventKind::Opaque)
            .count() as u64;
        let calls = events
            .iter()
            .filter(|e| e.kind == ObservedEventKind::ToolCall)
            .count() as u64;
        let results = events
            .iter()
            .filter(|e| e.kind == ObservedEventKind::ToolResult)
            .collect::<Vec<_>>();
        let observed = results.iter().filter(|e| e.tool_success.is_some()).count() as u64;
        let failures = results
            .iter()
            .filter(|e| e.tool_success == Some(false))
            .count() as u64;
        let metric = |id, value, observed, total| InsightMetric {
            id,
            value,
            coverage: Coverage { observed, total },
            evidence_ids: vec![request.evidence.id.clone()],
        };
        Ok(InsightReport {
            schema_version: INSIGHT_SCHEMA_VERSION,
            provider: self.manifest(),
            evidence: vec![request.evidence.clone()],
            metrics: vec![
                metric(MetricId::Sessions, Some(1), 1, 1),
                metric(MetricId::Events, Some(count), count, count),
                metric(
                    MetricId::ToolCalls,
                    (classified > 0).then_some(calls),
                    classified,
                    count,
                ),
                metric(
                    MetricId::ToolFailures,
                    (observed > 0).then_some(failures),
                    observed,
                    results.len() as u64,
                ),
                metric(MetricId::InputTokens, None, 0, 1),
                metric(MetricId::OutputTokens, None, 0, 1),
                metric(MetricId::KnownOutcomes, None, 0, 1),
            ],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn request() -> ProviderRequest {
        ProviderRequest::first_party(EvidenceRef {
            id: "selected".into(),
            source_digest: "a".repeat(64),
        })
    }
    fn events() -> Vec<EventObservation> {
        vec![
            EventObservation {
                kind: ObservedEventKind::Opaque,
                tool_success: None,
            },
            EventObservation {
                kind: ObservedEventKind::ToolCall,
                tool_success: None,
            },
            EventObservation {
                kind: ObservedEventKind::ToolResult,
                tool_success: Some(false),
            },
            EventObservation {
                kind: ObservedEventKind::ToolResult,
                tool_success: None,
            },
        ]
    }
    struct TestProvider {
        manifest: ProviderManifest,
        calls: Cell<u32>,
        fail: bool,
        forge_evidence: bool,
    }
    impl LocalInsightProvider for TestProvider {
        fn manifest(&self) -> ProviderManifest {
            self.manifest.clone()
        }
        fn evaluate(
            &self,
            request: &ProviderRequest,
            events: &[EventObservation],
        ) -> Result<InsightReport, ProviderError> {
            self.calls.set(self.calls.get() + 1);
            if self.fail {
                return Err(ProviderError::Unavailable);
            }
            let mut result = FirstPartyProvider.evaluate(request, events)?;
            result.provider = self.manifest();
            if self.forge_evidence {
                result.evidence[0].source_digest = "b".repeat(64);
            }
            Ok(result)
        }
    }
    fn test_provider() -> TestProvider {
        let mut manifest = ProviderManifest::first_party();
        manifest.id = "independent-test-provider".into();
        TestProvider {
            manifest,
            calls: Cell::new(0),
            fail: false,
            forge_evidence: false,
        }
    }

    #[test]
    fn first_party_preserves_counts_and_partial_outcome_coverage() {
        let report = dispatch_projected(&FirstPartyProvider, &request(), &events()).unwrap();
        let metric = |id| report.metrics.iter().find(|m| m.id == id).unwrap();
        assert_eq!(metric(MetricId::Events).value, Some(4));
        assert_eq!(
            metric(MetricId::ToolCalls).coverage,
            Coverage {
                observed: 3,
                total: 4
            }
        );
        assert_eq!(metric(MetricId::ToolFailures).value, Some(1));
        assert_eq!(
            metric(MetricId::ToolFailures).coverage,
            Coverage {
                observed: 1,
                total: 2
            }
        );
        assert_eq!(metric(MetricId::KnownOutcomes).value, None);
        assert_eq!(metric(MetricId::InputTokens).value, None);
    }

    #[test]
    fn complete_report_matches_existing_schema_and_metric_order() {
        let report = dispatch_projected(&FirstPartyProvider, &request(), &events()).unwrap();
        let expected = serde_json::json!({
            "schema_version": 1,
            "provider": {
                "id": "trace-commons-local", "version": "1",
                "rubric_version": "descriptive-counts-v1", "execution_mode": "local", "schema_version": 1
            },
            "evidence": [{"id": "selected", "source_digest": "a".repeat(64)}],
            "metrics": [
                {"id":"sessions","value":1,"coverage":{"observed":1,"total":1},"evidence_ids":["selected"]},
                {"id":"events","value":4,"coverage":{"observed":4,"total":4},"evidence_ids":["selected"]},
                {"id":"tool_calls","value":1,"coverage":{"observed":3,"total":4},"evidence_ids":["selected"]},
                {"id":"tool_failures","value":1,"coverage":{"observed":1,"total":2},"evidence_ids":["selected"]},
                {"id":"input_tokens","value":null,"coverage":{"observed":0,"total":1},"evidence_ids":["selected"]},
                {"id":"output_tokens","value":null,"coverage":{"observed":0,"total":1},"evidence_ids":["selected"]},
                {"id":"known_outcomes","value":null,"coverage":{"observed":0,"total":1},"evidence_ids":["selected"]}
            ]
        });
        assert_eq!(serde_json::to_value(report).unwrap(), expected);
    }

    #[test]
    fn host_selects_provider_and_rejects_incompatible_rubric_before_execution() {
        let provider = test_provider();
        let mut request = request();
        assert_eq!(
            dispatch_projected(&provider, &request, &events()),
            Err(ProviderError::Incompatible)
        );
        assert_eq!(provider.calls.get(), 0);
        request.provider = provider.manifest();
        let report = dispatch_projected(&provider, &request, &events()).unwrap();
        assert_eq!(report.provider.id, "independent-test-provider");
        request.provider.rubric_version = "other-rubric".into();
        assert_eq!(
            dispatch_projected(&provider, &request, &events()),
            Err(ProviderError::Incompatible)
        );
        assert_eq!(provider.calls.get(), 1);
    }

    #[test]
    fn failures_and_forged_evidence_never_become_first_party_fallbacks() {
        let mut provider = test_provider();
        let mut request = request();
        request.provider = provider.manifest();
        provider.fail = true;
        assert_eq!(
            dispatch_projected(&provider, &request, &events()),
            Err(ProviderError::Unavailable)
        );
        provider.fail = false;
        provider.forge_evidence = true;
        assert_eq!(
            dispatch_projected(&provider, &request, &events()),
            Err(ProviderError::InvalidResult)
        );
        assert_eq!(provider.calls.get(), 2);
    }

    #[test]
    fn invalid_requests_are_rejected_before_provider_runs() {
        let provider = test_provider();
        let mut request = request();
        request.provider = provider.manifest();
        request.schema_version += 1;
        assert_eq!(
            dispatch_projected(&provider, &request, &events()),
            Err(ProviderError::InvalidRequest)
        );
        request.schema_version = PROVIDER_REQUEST_VERSION;
        request.evidence.source_digest = "private-not-a-digest".into();
        assert_eq!(
            dispatch_projected(&provider, &request, &events()),
            Err(ProviderError::InvalidRequest)
        );
        request.evidence.source_digest = "a".repeat(64);
        assert_eq!(
            dispatch_projected(&provider, &request, &[]),
            Err(ProviderError::InvalidRequest)
        );
        let invalid = [EventObservation {
            kind: ObservedEventKind::ToolCall,
            tool_success: Some(true),
        }];
        assert_eq!(
            dispatch_projected(&provider, &request, &invalid),
            Err(ProviderError::InvalidRequest)
        );
        let oversized = vec![events()[0]; MAX_PROVIDER_EVENTS + 1];
        assert_eq!(
            dispatch_projected(&provider, &request, &oversized),
            Err(ProviderError::InputLimit)
        );
        assert_eq!(provider.calls.get(), 0);
    }

    #[test]
    fn event_projection_drops_bodies_paths_and_non_tool_success() {
        let event = SessionEvent {
            kind: SessionEventKind::Assistant,
            content: Some("PRIVATE_TRACE_CONTENT".into()),
            success: Some(true),
            ..Default::default()
        };
        let observation = EventObservation::from(&event);
        assert_eq!(
            observation,
            EventObservation {
                kind: ObservedEventKind::Classified,
                tool_success: None
            }
        );
        assert!(!format!("{observation:?}").contains("PRIVATE_TRACE_CONTENT"));
        let encoded = serde_json::to_string(&request()).unwrap();
        assert_eq!(
            serde_json::from_str::<ProviderRequest>(&encoded).unwrap(),
            request()
        );
        assert!(
            serde_json::from_str::<ProviderRequest>(
                &encoded.replace("private_descriptive", "public_comparison")
            )
            .is_err()
        );
    }
}
