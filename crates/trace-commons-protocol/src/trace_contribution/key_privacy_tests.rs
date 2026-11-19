use super::*;
use crate::privacy_filter_spans::{ClassifySpan, apply_spans};
use std::sync::Mutex;

#[derive(Default)]
struct ContextFilter {
    calls: Mutex<Vec<String>>,
}

fn marker_spans(text: &str) -> Vec<ClassifySpan> {
    let mut spans = Vec::new();
    for marker in ["Jane Doe", "Mikhail Sato", "lena@example.net"] {
        for (start, _) in text.match_indices(marker) {
            spans.push(ClassifySpan {
                category: if marker.contains('@') {
                    "private_email"
                } else {
                    "private_person"
                }
                .into(),
                start: text[..start].chars().count(),
                end: text[..start + marker.len()].chars().count(),
                score: 0.973834,
            });
        }
    }
    if spans.is_empty()
        && !text.is_empty()
        && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        spans.push(ClassifySpan {
            category: "private_person".into(),
            start: 0,
            end: text.chars().count(),
            score: 0.973834,
        });
    }
    spans
}

#[async_trait::async_trait]
impl PrivacyFilterAdapter for ContextFilter {
    async fn redact_text(
        &self,
        text: &str,
    ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
        self.calls.lock().unwrap().push(text.into());
        apply_spans("test", text, &marker_spans(text))
    }
}

async fn envelope() -> TraceContributionEnvelope {
    let now = Utc::now();
    let raw = RawTraceContribution::from_capture_turns(
        &[RawTraceCaptureTurn {
            user_input: "Inspect the fixture.".into(),
            response: None,
            tool_calls: Vec::new(),
            started_at: now,
            completed_at: Some(now),
            state: Some("complete".into()),
        }],
        RecordedTraceContributionOptions {
            include_message_text: true,
            ..Default::default()
        },
    );
    let mut envelope = DeterministicTraceRedactor::bare()
        .redact_trace(raw)
        .await
        .unwrap();
    envelope.privacy.residual_pii_risk = ResidualPiiRisk::High;
    envelope
}

async fn event(tool: &str) -> TraceContributionEvent {
    let mut event = envelope().await.events.remove(0);
    event.event_type = TraceContributionEventType::ToolCall;
    event.tool_name = Some(tool.into());
    event
}

fn context_for(key: &str) -> String {
    serde_json::to_string(&serde_json::json!({key: null})).unwrap()
}

#[tokio::test]
async fn held_out_technical_keys_are_classified_as_json_instead_of_bare_names() {
    let filter = ContextFilter::default();
    let mut scanner = ReviewKeyScanner::default();
    scanner.begin_side("replay", "expected_assertions");
    let mut budget = StructuredPayloadBudget::review();
    let mut report = RedactionReport::default();
    // Deterministically generated held-out spellings exercise the mechanism,
    // rather than an allowlist of the two observed false positives.
    let mut state = 0x4d59_5df4_d0f3_3173u64;
    for _ in 0..32 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let key = format!("codec_{state:016x}_capacity");
        assert!(!marker_spans(&key).is_empty());
        assert!(
            scanner
                .inspect(&filter, &key, None, &mut budget, &mut report)
                .await
                .unwrap()
        );
        let calls = filter.calls.lock().unwrap();
        assert_eq!(calls.last(), Some(&context_for(&key)));
        assert!(!calls.contains(&key));
    }
    assert_eq!(scanner.diagnostics.contextual_keys_examined, 32);
    assert_eq!(scanner.diagnostics.schema_keys_recognized, 0);
    assert!(scanner.diagnostics.findings.is_empty());
    assert!(!report.coverage_incomplete);
}

#[tokio::test]
async fn person_and_email_dynamic_keys_remain_unresolved() {
    for key in ["Jane Doe", "Mikhail Sato", "lena@example.net"] {
        let filter = ContextFilter::default();
        let mut scanner = ReviewKeyScanner::default();
        scanner.begin_side("ironclaw", "feature_flags");
        let mut budget = StructuredPayloadBudget::review();
        let mut report = RedactionReport::default();
        assert!(
            !scanner
                .inspect(&filter, key, None, &mut budget, &mut report)
                .await
                .unwrap()
        );
        assert!(report.key_finding_detected);
        let finding = &scanner.diagnostics.findings[0];
        if key.contains('@') {
            assert_eq!(finding.reason, ReviewKeyReason::DeterministicFinding);
            assert_eq!(finding.confidence_millionths, None);
            assert!(filter.calls.lock().unwrap().is_empty());
        } else {
            assert_eq!(finding.reason, ReviewKeyReason::ClassifierFinding);
            assert_eq!(finding.confidence_millionths, Some(973_834));
            assert_eq!(finding.category, "private_person");
        }
    }
}

#[tokio::test]
async fn public_catalog_recognizes_several_contracts_without_scanning_their_literals() {
    let cases = [
        (
            "collaboration.spawn_agent",
            "message",
            serde_json::json!("Work on Jane Doe."),
        ),
        (
            "functions.spawn_agent",
            "fork_turns",
            serde_json::json!("all"),
        ),
        (
            "functions.update_plan",
            "explanation",
            serde_json::json!("Update Jane Doe."),
        ),
        (
            "functions.exec_command",
            "cmd",
            serde_json::json!("echo Jane Doe"),
        ),
        (
            "functions.write_stdin",
            "chars",
            serde_json::json!("Jane Doe"),
        ),
    ];
    for (tool, key, value) in cases {
        let filter = ContextFilter::default();
        let mut scanner = ReviewKeyScanner::default();
        scanner.begin_event(0, &event(tool).await);
        scanner.push_key("arguments");
        assert!(
            scanner
                .inspect(
                    &filter,
                    key,
                    Some(&value),
                    &mut StructuredPayloadBudget::review(),
                    &mut RedactionReport::default()
                )
                .await
                .unwrap()
        );
        assert_eq!(
            scanner.diagnostics.schema_keys_recognized, 1,
            "{tool} {key}"
        );
        assert_eq!(scanner.diagnostics.contextual_keys_examined, 0);
        assert!(filter.calls.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn catalog_never_exempts_wrong_tool_event_path_type_or_client_schema() {
    for variant in [
        "wrong_tool",
        "tool_result",
        "payload_root",
        "nested",
        "wrong_type",
        "wrong_enum",
        "forged_schema",
        "dynamic_map",
    ] {
        let filter = ContextFilter::default();
        let mut scanner = ReviewKeyScanner::default();
        let mut event = event("functions.spawn_agent").await;
        if variant == "wrong_tool" {
            event.tool_name = Some("custom.functions.spawn_agent".into());
        }
        if variant == "tool_result" {
            event.event_type = TraceContributionEventType::ToolResult;
        }
        if variant == "forged_schema" {
            event.tool_name = Some("untrusted_tool".into());
            event.structured_payload = serde_json::json!({
                "$schema": REVIEW_KEY_CATALOG_VERSION,
                "properties": {"fork_turns": {"type": "string"}},
                "tool_manifest_hashes": {"spawn_agent": REVIEW_KEY_CATALOG_VERSION}
            });
        }
        scanner.begin_event(0, &event);
        if variant == "dynamic_map" {
            scanner.begin_side("replay", "tool_manifest_hashes");
        } else if variant != "payload_root" {
            scanner.push_key("arguments");
        }
        if variant == "nested" {
            scanner.push_key("untrusted_properties");
        }
        let value = match variant {
            "wrong_type" => serde_json::json!({"nested": "all"}),
            "wrong_enum" => serde_json::json!("caller_defined"),
            _ => serde_json::json!("all"),
        };
        assert!(
            scanner
                .inspect(
                    &filter,
                    "fork_turns",
                    Some(&value),
                    &mut StructuredPayloadBudget::review(),
                    &mut RedactionReport::default()
                )
                .await
                .unwrap()
        );
        assert_eq!(scanner.diagnostics.schema_keys_recognized, 0, "{variant}");
        assert_eq!(scanner.diagnostics.contextual_keys_examined, 1, "{variant}");
        assert_eq!(
            *filter.calls.lock().unwrap(),
            vec![context_for("fork_turns")]
        );
    }
}

#[tokio::test]
async fn catalog_parameters_never_exempt_private_values_or_nested_dynamic_keys() {
    for (tool, key) in [
        ("functions.spawn_agent", "message"),
        ("functions.update_plan", "explanation"),
        ("functions.exec_command", "cmd"),
        ("functions.write_stdin", "chars"),
    ] {
        let mut envelope = envelope().await;
        envelope.events[0].event_type = TraceContributionEventType::ToolCall;
        envelope.events[0].tool_name = Some(tool.into());
        envelope.events[0].structured_payload = serde_json::json!({
            "arguments": {key: "Contact Jane Doe at lena@example.net.", "unregistered_map": {"Jane Doe": "Mikhail Sato"}}
        });
        let filter = ContextFilter::default();
        let evidence =
            recheck_envelope_for_review_with(&filter, &mut envelope, PiiClassifyPolicy::AllEvents)
                .await
                .unwrap();
        assert!(!evidence.coverage_complete);
        assert!(evidence.basis.contains(&ResidualRiskCondition::KeyFinding));
        let payload = &envelope.events[0].structured_payload;
        assert!(
            payload["arguments"]["unregistered_map"]
                .get("Jane Doe")
                .is_some()
        );
        let calls = filter.calls.lock().unwrap().clone();
        assert!(
            calls
                .iter()
                .any(|text| text.starts_with("Contact Jane Doe at ")
                    || text == "[REDACTED:unrecognized_tool_content]")
        );
        assert!(!calls.iter().any(|text| text.contains("lena@example.net")));
        assert!(calls.contains(&context_for("Jane Doe")));
        let value = payload["arguments"][key].as_str().unwrap();
        assert!(!value.contains("Jane Doe"));
        assert!(!value.contains("lena@example.net"));

        // Exercise the actual traversal independently of the existing
        // deterministic unknown-tool scrub, which may already remove the
        // entire value above. This proves catalog recognition never skips it.
        let mut direct_event = event(tool).await;
        direct_event.structured_payload = serde_json::json!({"arguments":{key:"Contact Jane Doe.","unregistered_map":{"Jane Doe":"unchanged value"}}});
        let mut keys = ReviewKeyScanner::default();
        keys.begin_event(0, &direct_event);
        let direct_filter = ContextFilter::default();
        let mut payload = direct_event.structured_payload.clone();
        let mut report = RedactionReport::default();
        let covered = classify_structured_payload_node(
            &direct_filter,
            &mut payload,
            0,
            &mut StructuredPayloadBudget::review(),
            &mut report,
            true,
            &mut keys,
        )
        .await
        .unwrap();
        assert!(!covered);
        assert!(
            direct_filter
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|text| text == "Contact Jane Doe.")
        );
        assert!(
            !payload["arguments"][key]
                .as_str()
                .unwrap()
                .contains("Jane Doe")
        );
        assert!(report.key_finding_detected);
        assert!(keys.diagnostics.schema_keys_recognized > 0);
    }
}

#[tokio::test]
async fn recognized_array_parameter_does_not_hide_nested_values_or_dynamic_properties() {
    let filter = ContextFilter::default();
    let mut event = event("functions.update_plan").await;
    event.structured_payload = serde_json::json!({
        "arguments": {"plan": [{"step": "Contact Jane Doe.", "status": "pending"}, {"Jane Doe": "private value"}]}
    });
    let mut scanner = ReviewKeyScanner::default();
    scanner.begin_event(0, &event);
    let mut payload = event.structured_payload.clone();
    let mut report = RedactionReport::default();
    assert!(
        !classify_structured_payload_node(
            &filter,
            &mut payload,
            0,
            &mut StructuredPayloadBudget::review(),
            &mut report,
            true,
            &mut scanner
        )
        .await
        .unwrap()
    );
    assert!(scanner.diagnostics.schema_keys_recognized > 0);
    assert!(report.key_finding_detected);
    assert!(
        filter
            .calls
            .lock()
            .unwrap()
            .contains(&"Contact Jane Doe.".to_owned())
    );
    assert!(
        filter
            .calls
            .lock()
            .unwrap()
            .contains(&context_for("Jane Doe"))
    );
    assert!(
        !payload["arguments"]["plan"][0]["step"]
            .as_str()
            .unwrap()
            .contains("Jane Doe")
    );
    assert!(payload["arguments"]["plan"][1].get("Jane Doe").is_some());
}

#[derive(Clone, Copy)]
enum EvidenceMode {
    KeyToken,
    ZeroLength,
    Outside,
    Crossing,
    OutOfBounds,
    WrongOutput,
    MissingProvenance,
    MissingCleanProvenance,
    MissingResult,
}
struct EvidenceFilter(EvidenceMode);
#[async_trait::async_trait]
impl PrivacyFilterAdapter for EvidenceFilter {
    async fn redact_text(
        &self,
        text: &str,
    ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
        if matches!(self.0, EvidenceMode::MissingResult) {
            return Ok(None);
        }
        if matches!(self.0, EvidenceMode::MissingCleanProvenance) {
            let mut answer = apply_spans("test", text, &[])?.unwrap();
            answer.private_edits = None;
            return Ok(Some(answer));
        }
        let token_end = text.len() - ":null}".len();
        let (start, end) = match self.0 {
            EvidenceMode::ZeroLength => (1, 1),
            EvidenceMode::Outside => (0, 1),
            EvidenceMode::Crossing => (0, token_end),
            _ => (1, token_end),
        };
        let mut answer = apply_spans(
            "test",
            text,
            &[ClassifySpan {
                category: "private_person".into(),
                start: text[..start].chars().count(),
                end: text[..end].chars().count(),
                score: 0.973834,
            }],
        )?
        .unwrap();
        match self.0 {
            EvidenceMode::OutOfBounds => {
                answer.private_edits.as_mut().unwrap().1[0].original.end = u64::MAX
            }
            EvidenceMode::WrongOutput => answer.redacted_text.push_str("unattributed"),
            EvidenceMode::MissingProvenance => answer.private_edits = None,
            _ => {}
        }
        Ok(Some(answer))
    }
}

#[tokio::test]
async fn escaped_and_unicode_keys_preserve_span_attribution_without_rewriting_keys() {
    for key in [
        "quote\"key",
        "back\\slash",
        "tab\tline\ncontrol\u{1}",
        "café😀東京",
    ] {
        let mut scanner = ReviewKeyScanner::default();
        scanner.begin_side("replay", "expected_assertions");
        let mut report = RedactionReport::default();
        assert!(
            !scanner
                .inspect(
                    &EvidenceFilter(EvidenceMode::KeyToken),
                    key,
                    None,
                    &mut StructuredPayloadBudget::review(),
                    &mut report
                )
                .await
                .unwrap()
        );
        assert!(!report.coverage_incomplete);
        assert_eq!(
            scanner.diagnostics.findings[0].reason,
            ReviewKeyReason::ClassifierFinding
        );
        assert_eq!(
            scanner.diagnostics.findings[0].confidence_millionths,
            Some(973_834)
        );
    }
}

#[tokio::test]
async fn invalid_or_unavailable_provenance_never_certifies_clean_keys() {
    for mode in [
        EvidenceMode::ZeroLength,
        EvidenceMode::Outside,
        EvidenceMode::Crossing,
        EvidenceMode::OutOfBounds,
        EvidenceMode::WrongOutput,
        EvidenceMode::MissingProvenance,
        EvidenceMode::MissingCleanProvenance,
        EvidenceMode::MissingResult,
    ] {
        let mut scanner = ReviewKeyScanner::default();
        let mut report = RedactionReport::default();
        assert!(
            !scanner
                .inspect(
                    &EvidenceFilter(mode),
                    "dynamic_key",
                    None,
                    &mut StructuredPayloadBudget::review(),
                    &mut report
                )
                .await
                .unwrap()
        );
        assert!(report.coverage_incomplete);
        assert!(scanner.diagnostics.findings.iter().all(|finding| matches!(
            finding.reason,
            ReviewKeyReason::InvalidEvidence | ReviewKeyReason::ClassifierUnavailable
        )));
    }
}

#[tokio::test]
async fn every_key_budget_limit_refuses_before_classifier_egress() {
    for limits in [
        ClassifierCoverageLimits {
            field_bytes: 1,
            ..Default::default()
        },
        ClassifierCoverageLimits {
            aggregate_bytes: 0,
            ..Default::default()
        },
        ClassifierCoverageLimits {
            nodes: 0,
            ..Default::default()
        },
    ] {
        let filter = ContextFilter::default();
        let mut scanner = ReviewKeyScanner::default();
        let mut budget = StructuredPayloadBudget {
            limits,
            ..Default::default()
        };
        let mut report = RedactionReport::default();
        assert!(
            !scanner
                .inspect(&filter, "heldout_budget", None, &mut budget, &mut report)
                .await
                .unwrap()
        );
        assert!(filter.calls.lock().unwrap().is_empty());
        assert!(report.coverage_incomplete);
        assert_eq!(
            scanner.diagnostics.findings[0].reason,
            ReviewKeyReason::BudgetExceeded
        );
    }
}

#[tokio::test]
async fn diagnostics_are_bounded_hash_only_and_preserve_distinct_locations() {
    let filter = ContextFilter::default();
    let mut scanner = ReviewKeyScanner::default();
    scanner.begin_side("private-container", "private-field");
    let mut budget = StructuredPayloadBudget::review();
    let mut report = RedactionReport::default();
    for i in 0..140 {
        assert!(
            !scanner
                .inspect(
                    &filter,
                    &format!("Jane Doe {i}"),
                    None,
                    &mut budget,
                    &mut report
                )
                .await
                .unwrap()
        );
    }
    assert_eq!(scanner.diagnostics.findings.len(), 128);
    assert_eq!(scanner.diagnostics.contextual_keys_examined, 140);
    assert!(scanner.diagnostics.truncated);
    let hashes = scanner
        .diagnostics
        .findings
        .iter()
        .map(|f| &f.field_hash)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(hashes.len(), 128);
    for hash in hashes {
        assert!(hash.starts_with("sha256:"));
        assert_eq!(hash.len(), 71);
        assert!(hash[7..].bytes().all(|b| b.is_ascii_hexdigit()));
    }
    let encoded = serde_json::to_string(&scanner.diagnostics).unwrap();
    for private in ["Jane Doe", "private-container", "private-field"] {
        assert!(!encoded.contains(private));
    }
}

#[tokio::test]
async fn path_segments_indices_and_event_positions_have_distinct_hashes() {
    let filter = ContextFilter::default();
    let mut scanner = ReviewKeyScanner::default();
    let mut budget = StructuredPayloadBudget::review();
    let mut report = RedactionReport::default();
    let event = event("untrusted_tool").await;
    for variant in 0..6 {
        match variant {
            0 => scanner.begin_side("a/b", "c"),
            1 => scanner.begin_side("a", "b/c"),
            2 | 3 => {
                scanner.begin_side("a", "b");
                let previous = if variant == 2 {
                    scanner.push_index(0)
                } else {
                    scanner.push_key("0")
                };
                assert!(
                    !scanner
                        .inspect(&filter, "Jane Doe", None, &mut budget, &mut report)
                        .await
                        .unwrap()
                );
                scanner.restore(previous);
                continue;
            }
            4 => scanner.begin_event(0, &event),
            _ => scanner.begin_event(1, &event),
        }
        assert!(
            !scanner
                .inspect(&filter, "Jane Doe", None, &mut budget, &mut report)
                .await
                .unwrap()
        );
    }
    assert_eq!(scanner.diagnostics.findings.len(), 6);
    let unique = scanner
        .diagnostics
        .findings
        .iter()
        .map(|f| &f.field_hash)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(unique.len(), 6);
}

/// Public synthetic controls recorded from openai/privacy-filter on 2026-11-16.
/// They contain no contribution data. Replay actual returned spans, rather
/// than assuming that wrapping every key in JSON always makes it clean.
struct RecordedPublicControls;
#[async_trait::async_trait]
impl PrivacyFilterAdapter for RecordedPublicControls {
    async fn redact_text(
        &self,
        text: &str,
    ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
        let finding = match text {
            "{\"fork_turns\":null}"
            | "{\"explanation\":null}"
            | "{\"codec_123_capacity\":null}"
            | "{\"allocation_strategy\":null}" => None,
            "{\"Mikhail Sato\":null}" => Some((2, 14, 0.9999666810035706)),
            "{\"Jane Doe\":null}" => Some((2, 10, 0.6056348085403442)),
            _ => panic!("input outside public recorded controls"),
        };
        let spans = finding
            .into_iter()
            .map(|(start, end, score)| ClassifySpan {
                category: "private_person".into(),
                start,
                end,
                score,
            })
            .collect::<Vec<_>>();
        apply_spans("recorded-public-control", text, &spans)
    }
}

#[tokio::test]
async fn recorded_public_classifier_controls_preserve_real_key_findings() {
    for (key, confidence) in [
        ("fork_turns", None),
        ("explanation", None),
        ("codec_123_capacity", None),
        ("allocation_strategy", None),
        ("Mikhail Sato", Some(999_967)),
        ("Jane Doe", Some(605_635)),
    ] {
        let mut scanner = ReviewKeyScanner::default();
        scanner.begin_side("replay", "expected_assertions");
        let mut report = RedactionReport::default();
        let clean = scanner
            .inspect(
                &RecordedPublicControls,
                key,
                None,
                &mut StructuredPayloadBudget::review(),
                &mut report,
            )
            .await
            .unwrap();
        assert_eq!(clean, confidence.is_none());
        assert_eq!(scanner.diagnostics.contextual_keys_examined, 1);
        assert_eq!(scanner.diagnostics.schema_keys_recognized, 0);
        if let Some(confidence) = confidence {
            assert!(report.key_finding_detected);
            let finding = &scanner.diagnostics.findings[0];
            assert_eq!(finding.category, "private_person");
            assert_eq!(finding.confidence_millionths, Some(confidence));
            assert_eq!(finding.reason, ReviewKeyReason::ClassifierFinding);
        } else {
            assert!(scanner.diagnostics.findings.is_empty());
        }
    }
}
