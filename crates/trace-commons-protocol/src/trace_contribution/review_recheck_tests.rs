use super::*;

struct PersonFilter {
    fail_on: Option<&'static str>,
}

#[async_trait::async_trait]
impl PrivacyFilterAdapter for PersonFilter {
    async fn redact_text(
        &self,
        text: &str,
    ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
        if self.fail_on.is_some_and(|marker| text.contains(marker)) {
            return Err(TraceContributionError::RedactionFailed {
                reason: "test_classifier_unavailable".into(),
            });
        }
        if !text.contains("Jane Doe") {
            return Ok(None);
        }
        let mut report = RedactionReport::default();
        report
            .counts
            .insert("privacy_filter:private_person".into(), 1);
        report.pii_labels_present.push("private_person".into());
        Ok(Some(SafePrivacyFilterRedaction {
            private_edits: None,
            redacted_text: text.replace("Jane Doe", "[REDACTED:private_person]"),
            summary: SafePrivacyFilterSummary {
                schema_version: 1,
                output_mode: "redacted_text_only".into(),
                span_count: 1,
                by_label: BTreeMap::from([("private_person".into(), 1)]),
                decoded_mismatch: false,
                classify_policy: None,
                events_examined: 0,
                events_skipped_by_policy: 0,
            },
            report,
        }))
    }
}

fn filter() -> PersonFilter {
    PersonFilter { fail_on: None }
}

async fn envelope() -> TraceContributionEnvelope {
    let now = Utc::now();
    let raw = RawTraceContribution::from_capture_turns(
        &[RawTraceCaptureTurn {
            user_input: "List the files.".into(),
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

#[tokio::test]
async fn review_recheck_covers_notes_and_hindsight_without_changing_legacy_pass() {
    let mut original = envelope().await;
    original.replay.replay_notes = vec!["Ask Jane Doe.".into()];
    original.hindsight = Some(HindsightRelabelingCandidate {
        original_goal_summary: Some("Help Jane Doe.".into()),
        achieved_subgoals: vec!["Reached Jane Doe.".into()],
        failure_type: Some(TraceFailureMode::Other("Jane Doe was unavailable.".into())),
        ..Default::default()
    });
    let mut legacy = original.clone();
    rescrub_envelope_prose_pii_with(&filter(), &mut legacy, PiiClassifyPolicy::AllEvents)
        .await
        .unwrap();
    assert_eq!(legacy.replay.replay_notes, original.replay.replay_notes);
    assert_eq!(legacy.hindsight, original.hindsight);
    assert_eq!(legacy.privacy.residual_pii_risk, ResidualPiiRisk::High);

    let result =
        recheck_envelope_for_review_with(&filter(), &mut original, PiiClassifyPolicy::AllEvents)
            .await
            .unwrap();
    assert!(result.coverage_complete);
    assert!(result.residual_scan_complete);
    assert!(result.useful_classifier_result);
    assert_eq!(result.correction_status, ReviewCorrectionStatus::Absent);
    assert_eq!(original.privacy.residual_pii_risk, ResidualPiiRisk::Medium);
    assert!(
        !serde_json::to_string(&original.replay)
            .unwrap()
            .contains("Jane Doe")
    );
    assert!(
        !serde_json::to_string(&original.hindsight)
            .unwrap()
            .contains("Jane Doe")
    );
}

#[tokio::test]
async fn review_recheck_preserves_correction_and_holds_classifier_findings() {
    let mut envelope = envelope().await;
    let correction = "Jane Doe expected the staging API at /Users/private/project";
    envelope.outcome.human_correction = Some(correction.into());
    let before_counts = envelope.privacy.redaction_counts.clone();
    let result =
        recheck_envelope_for_review_with(&filter(), &mut envelope, PiiClassifyPolicy::AllEvents)
            .await
            .unwrap();
    assert_eq!(
        envelope.outcome.human_correction.as_deref(),
        Some(correction)
    );
    assert_eq!(result.correction_status, ReviewCorrectionStatus::Unresolved);
    assert!(!result.coverage_complete);
    assert_eq!(envelope.privacy.residual_pii_risk, ResidualPiiRisk::High);
    assert_eq!(envelope.privacy.redaction_counts, before_counts);
    assert!(
        result
            .basis
            .contains(&ResidualRiskCondition::CoverageIncomplete)
    );
}

#[tokio::test]
async fn review_recheck_checks_unchanged_correction_without_rewriting_paths() {
    let mut envelope = envelope().await;
    let correction = "Use /Users/private/project/staging.toml instead";
    envelope.outcome.human_correction = Some(correction.into());
    envelope
        .replay
        .replay_notes
        .push("Jane Doe requested staging.".into());
    let result =
        recheck_envelope_for_review_with(&filter(), &mut envelope, PiiClassifyPolicy::AllEvents)
            .await
            .unwrap();
    assert_eq!(
        envelope.outcome.human_correction.as_deref(),
        Some(correction)
    );
    assert_eq!(
        result.correction_status,
        ReviewCorrectionStatus::CheckedUnchanged
    );
    assert!(result.coverage_complete);
    assert_eq!(envelope.privacy.residual_pii_risk, ResidualPiiRisk::Medium);
}

#[tokio::test]
async fn review_recheck_preserves_and_holds_a_correction_credential() {
    let mut envelope = envelope().await;
    let correction = format!(
        "Use sk-{} instead",
        "aB3cD4eF5gH6iJ7kL8mN9pQ0rS1tU2vW3xY4zA5b"
    );
    envelope.outcome.human_correction = Some(correction.clone());
    let result =
        recheck_envelope_for_review_with(&filter(), &mut envelope, PiiClassifyPolicy::AllEvents)
            .await
            .unwrap();
    assert_eq!(
        envelope.outcome.human_correction.as_deref(),
        Some(correction.as_str())
    );
    assert_eq!(result.correction_status, ReviewCorrectionStatus::Unresolved);
    assert_eq!(envelope.privacy.residual_pii_risk, ResidualPiiRisk::High);
    assert!(
        result
            .basis
            .contains(&ResidualRiskCondition::ResidualSurvivor)
    );
}

#[tokio::test]
async fn review_recheck_empty_classifier_cannot_lower_prior_high() {
    let mut envelope = envelope().await;
    let result =
        recheck_envelope_for_review_with(&filter(), &mut envelope, PiiClassifyPolicy::AllEvents)
            .await
            .unwrap();
    assert!(result.coverage_complete);
    assert!(result.residual_scan_complete);
    assert!(!result.useful_classifier_result);
    assert_eq!(envelope.privacy.residual_pii_risk, ResidualPiiRisk::High);
}

#[tokio::test]
async fn review_recheck_failure_is_atomic_even_after_side_prose_was_changed() {
    let mut envelope = envelope().await;
    envelope.consent.message_text_included = false;
    envelope.replay.replay_notes = vec!["Ask Jane Doe.".into(), "Fail here".into()];
    let before = serde_json::to_vec(&envelope).unwrap();
    let result = recheck_envelope_for_review_with(
        &PersonFilter {
            fail_on: Some("Fail here"),
        },
        &mut envelope,
        PiiClassifyPolicy::AllEvents,
    )
    .await;
    assert!(result.is_err());
    assert_eq!(serde_json::to_vec(&envelope).unwrap(), before);
}

#[tokio::test]
async fn review_recheck_covers_side_prose_beyond_the_ordinary_field_limit() {
    let mut envelope = envelope().await;
    envelope.replay.replay_notes = vec!["x".repeat(STRUCTURED_PAYLOAD_MAX_FIELD_BYTES + 1)];
    let result =
        recheck_envelope_for_review_with(&filter(), &mut envelope, PiiClassifyPolicy::AllEvents)
            .await
            .unwrap();
    assert!(result.coverage_complete);
    assert_eq!(envelope.privacy.residual_pii_risk, ResidualPiiRisk::High);
    assert!(
        !result
            .basis
            .contains(&ResidualRiskCondition::CoverageIncomplete)
    );
}

#[tokio::test]
async fn review_recheck_does_not_rewrite_flag_keys_or_hide_their_findings() {
    let mut envelope = envelope().await;
    envelope
        .ironclaw
        .feature_flags
        .insert("Jane Doe".into(), "enabled".into());
    let result =
        recheck_envelope_for_review_with(&filter(), &mut envelope, PiiClassifyPolicy::AllEvents)
            .await
            .unwrap();
    assert!(envelope.ironclaw.feature_flags.contains_key("Jane Doe"));
    assert!(result.basis.contains(&ResidualRiskCondition::KeyFinding));
    assert!(!result.coverage_complete);
    assert_eq!(envelope.privacy.residual_pii_risk, ResidualPiiRisk::High);
}

#[tokio::test]
async fn review_recheck_all_events_covers_tool_pii_and_preserves_event_policy_choice() {
    let mut envelope = envelope().await;
    envelope.events[0].event_type = TraceContributionEventType::ToolResult;
    envelope.events[0].redacted_content = Some("Ask Jane Doe.".into());
    let mut prose_only = envelope.clone();
    let prose =
        recheck_envelope_for_review_with(&filter(), &mut prose_only, PiiClassifyPolicy::ProseOnly)
            .await
            .unwrap();
    assert_eq!(prose.policy, PiiClassifyPolicy::ProseOnly);
    assert!(
        prose_only.events[0]
            .redacted_content
            .as_deref()
            .unwrap()
            .contains("Jane Doe")
    );
    let all =
        recheck_envelope_for_review_with(&filter(), &mut envelope, PiiClassifyPolicy::AllEvents)
            .await
            .unwrap();
    assert_eq!(all.policy, PiiClassifyPolicy::AllEvents);
    assert!(all.coverage_complete);
    assert!(
        !envelope.events[0]
            .redacted_content
            .as_deref()
            .unwrap()
            .contains("Jane Doe")
    );
}

#[tokio::test]
async fn review_recheck_covers_all_deterministic_side_prose_and_preserves_identity() {
    let mut envelope = envelope().await;
    let source_id = envelope.submission_id;
    let source_trace = envelope.trace_id;
    let contributor = envelope.contributor.clone();
    let text = "Ask Jane Doe.";
    envelope
        .ironclaw
        .feature_flags
        .insert("flag".into(), text.into());
    envelope.ironclaw.model_name = Some(text.into());
    envelope.conversation_id = Some(text.into());
    envelope.value.explanation = vec![text.into()];
    envelope.events[0].tool_name = Some(text.into());
    envelope.outcome.error_taxonomy = vec![text.into()];
    envelope.outcome.failure_modes = vec![TraceFailureMode::Other(text.into())];
    envelope.replay.required_tools = vec![text.into()];
    envelope
        .replay
        .tool_manifest_hashes
        .insert("tool".into(), text.into());
    envelope.replay.expected_assertions = vec![serde_json::json!({"assertion":text})];
    envelope.trace_card.tool_categories = vec![text.into()];
    envelope.value_card.limitations = vec![text.into()];
    envelope.value_card.user_visible_explanation = vec![text.into()];
    let result =
        recheck_envelope_for_review_with(&filter(), &mut envelope, PiiClassifyPolicy::AllEvents)
            .await
            .unwrap();
    assert!(result.coverage_complete);
    assert!(
        !serde_json::to_string(&envelope)
            .unwrap()
            .contains("Jane Doe")
    );
    assert_eq!(envelope.submission_id, source_id);
    assert_eq!(envelope.trace_id, source_trace);
    assert_eq!(envelope.contributor, contributor);
}

#[test]
fn review_recheck_evidence_has_fixed_serialized_labels() {
    let evidence = ReviewPrivacyRecheckEvidence {
        basis: vec![ResidualRiskCondition::FoundAndRemoved],
        coverage_complete: true,
        correction_status: ReviewCorrectionStatus::CheckedUnchanged,
        residual_scan_complete: true,
        useful_classifier_result: true,
        policy: PiiClassifyPolicy::AllEvents,
    };
    let value = serde_json::to_value(&evidence).unwrap();
    assert_eq!(value["policy"], "all-events");
    assert_eq!(value["correction_status"], "checked_unchanged");
    assert_eq!(
        serde_json::from_value::<ReviewPrivacyRecheckEvidence>(value).unwrap(),
        evidence
    );
}

struct RecordingFilter(std::sync::Mutex<Vec<String>>);

#[async_trait::async_trait]
impl PrivacyFilterAdapter for RecordingFilter {
    async fn redact_text(
        &self,
        text: &str,
    ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
        self.0.lock().unwrap().push(text.into());
        Ok(None)
    }
}

#[tokio::test]
async fn review_recheck_removes_event_credentials_before_external_calls_and_never_sends_correction_credentials()
 {
    let mut envelope = envelope().await;
    let credential = format!("sk-{}", "aB3cD4eF5gH6iJ7kL8mN9pQ0rS1tU2vW3xY4zA5b");
    envelope.events[0].redacted_content = Some(format!("The key is {credential}"));
    envelope.outcome.human_correction = Some(credential.clone());
    let adapter = RecordingFilter(std::sync::Mutex::new(Vec::new()));
    let evidence =
        recheck_envelope_for_review_with(&adapter, &mut envelope, PiiClassifyPolicy::AllEvents)
            .await
            .unwrap();
    let calls = adapter.0.lock().unwrap();
    assert!(!calls.is_empty());
    assert!(calls.iter().all(|text| !text.contains(&credential)));
    assert!(
        !envelope.events[0]
            .redacted_content
            .as_deref()
            .unwrap()
            .contains(&credential)
    );
    assert_eq!(
        envelope.outcome.human_correction.as_deref(),
        Some(credential.as_str())
    );
    assert_eq!(
        evidence.correction_status,
        ReviewCorrectionStatus::Unresolved
    );
    assert!(!evidence.coverage_complete);
}

#[tokio::test]
async fn review_recheck_holds_credential_keys_before_any_external_egress() {
    let mut envelope = envelope().await;
    let credential = format!("sk-{}", "aB3cD4eF5gH6iJ7kL8mN9pQ0rS1tU2vW3xY4zA5b");
    envelope
        .ironclaw
        .feature_flags
        .insert(credential.clone(), "enabled".into());
    envelope
        .replay
        .tool_manifest_hashes
        .insert(credential.clone(), "manifest".into());
    envelope.events[0].structured_payload = serde_json::json!({credential.clone():"content"});
    let adapter = RecordingFilter(std::sync::Mutex::new(Vec::new()));
    let evidence =
        recheck_envelope_for_review_with(&adapter, &mut envelope, PiiClassifyPolicy::AllEvents)
            .await
            .unwrap();
    assert!(adapter.0.lock().unwrap().is_empty());
    assert!(envelope.ironclaw.feature_flags.contains_key(&credential));
    assert!(
        envelope
            .replay
            .tool_manifest_hashes
            .contains_key(&credential)
    );
    assert!(
        envelope.events[0]
            .structured_payload
            .get(&credential)
            .is_some()
    );
    assert!(evidence.basis.contains(&ResidualRiskCondition::KeyFinding));
    assert!(!evidence.coverage_complete);
    assert_eq!(envelope.privacy.residual_pii_risk, ResidualPiiRisk::High);
}

#[tokio::test]
async fn review_side_prose_key_guard_does_not_send_credentials() {
    let adapter = RecordingFilter(std::sync::Mutex::new(Vec::new()));
    let credential = format!("sk-{}", "aB3cD4eF5gH6iJ7kL8mN9pQ0rS1tU2vW3xY4zA5b");
    let mut side = ReviewSideProse::new(&adapter, StructuredPayloadBudget::review());
    assert!(!side.inspect_key(&credential).await.unwrap());
    assert!(side.report.key_finding_detected);
    assert!(adapter.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn review_recheck_unavailable_key_scan_preserves_source_without_external_calls() {
    let mut envelope = envelope().await;
    let mut nested = serde_json::json!("leaf");
    for _ in 0..80 {
        nested = serde_json::json!([nested]);
    }
    envelope.replay.expected_assertions = vec![nested];
    envelope.outcome.human_correction = Some("Keep the correction as written.".into());
    let original = serde_json::to_vec(&envelope).unwrap();
    let adapter = RecordingFilter(std::sync::Mutex::new(Vec::new()));
    let evidence =
        recheck_envelope_for_review_with(&adapter, &mut envelope, PiiClassifyPolicy::AllEvents)
            .await
            .unwrap();
    assert!(adapter.0.lock().unwrap().is_empty());
    assert!(!evidence.coverage_complete);
    assert!(!evidence.residual_scan_complete);
    assert!(!evidence.useful_classifier_result);
    assert_eq!(
        evidence.correction_status,
        ReviewCorrectionStatus::Unresolved
    );
    assert!(
        evidence
            .basis
            .contains(&ResidualRiskCondition::ResidualScanUnavailable)
    );
    assert_eq!(envelope.privacy.residual_pii_risk, ResidualPiiRisk::High);
    assert_eq!(serde_json::to_vec(&envelope).unwrap(), original);
}

#[test]
fn review_only_residual_cleaning_uses_exact_existing_severity_exemptions() {
    for label in NON_SEVERITY_REDACTION_LABELS {
        let mut report = RedactionReport::default();
        report.counts.insert((*label).into(), 1);
        report.pii_labels_present.push((*label).into());
        let legacy = PostScrubAssessment {
            complete_coverage: true,
            useful_classifier_result: true,
            findings: RedactionReport::default(),
            residual_findings: report.clone(),
        };
        assert!(!legacy.residual_clean());
        let reviewed = PostScrubAssessment {
            residual_findings: review_severity_residuals(report),
            ..legacy
        };
        assert!(reviewed.residual_clean(), "exact non-risk label {label}");
        assert_eq!(
            resolve_post_scrub_risk(ResidualPiiRisk::High, ResidualPiiRisk::Medium, &reviewed),
            ResidualPiiRisk::Medium
        );
    }
    for label in [
        "private_email",
        "phone_number",
        "person_name",
        "secret:pem_private_key",
        "privacy_filter:local_path",
        "future_unknown_label",
    ] {
        let mut report = RedactionReport::default();
        report.counts.insert(label.into(), 1);
        report.pii_labels_present.push(label.into());
        let assessment = PostScrubAssessment {
            complete_coverage: true,
            useful_classifier_result: true,
            findings: RedactionReport::default(),
            residual_findings: review_severity_residuals(report),
        };
        assert!(
            !assessment.residual_clean(),
            "risk-bearing or unknown label {label}"
        );
        assert_eq!(
            resolve_post_scrub_risk(ResidualPiiRisk::High, ResidualPiiRisk::Medium, &assessment),
            ResidualPiiRisk::High
        );
    }
    for flag in 0..3 {
        let mut report = RedactionReport::default();
        report.counts.insert("local_path".into(), 1);
        match flag {
            0 => report.blocked_secret_detected = true,
            1 => report.key_finding_detected = true,
            _ => report.coverage_incomplete = true,
        }
        let assessment = PostScrubAssessment {
            complete_coverage: true,
            useful_classifier_result: true,
            findings: RedactionReport::default(),
            residual_findings: review_severity_residuals(report),
        };
        assert!(!assessment.residual_clean());
        assert_eq!(
            resolve_post_scrub_risk(ResidualPiiRisk::High, ResidualPiiRisk::Medium, &assessment),
            ResidualPiiRisk::High
        );
    }
}

#[tokio::test]
async fn review_recheck_preserves_and_holds_correction_email_and_phone() {
    for correction in ["Contact ada@example.com", "Call (415) 555-0132"] {
        let mut envelope = envelope().await;
        envelope.outcome.human_correction = Some(correction.into());
        envelope
            .replay
            .replay_notes
            .push("Jane Doe requested staging.".into());
        let result = recheck_envelope_for_review_with(
            &filter(),
            &mut envelope,
            PiiClassifyPolicy::AllEvents,
        )
        .await
        .unwrap();
        assert_eq!(
            envelope.outcome.human_correction.as_deref(),
            Some(correction)
        );
        assert_eq!(result.correction_status, ReviewCorrectionStatus::Unresolved);
        assert!(!result.coverage_complete);
        assert_eq!(envelope.privacy.residual_pii_risk, ResidualPiiRisk::High);
        assert!(
            result
                .basis
                .contains(&ResidualRiskCondition::CoverageIncomplete)
        );
    }
}

#[tokio::test]
async fn review_recheck_many_small_tool_events_need_a_separate_bounded_profile() {
    let mut envelope = envelope().await;
    let mut event = envelope.events[0].clone();
    event.event_type = TraceContributionEventType::ToolResult;
    event.redacted_content = None;
    event.tool_name = Some("read_file".into());
    event.structured_payload = serde_json::json!({"result":"completed"});
    envelope.events = (0..5_000)
        .map(|_| {
            let mut event = event.clone();
            event.event_id = Uuid::new_v4();
            event
        })
        .collect();
    let mut legacy = envelope.clone();
    let legacy_evidence = rescrub_envelope_prose_pii_impl(
        &filter(),
        &mut legacy,
        PiiClassifyPolicy::AllEvents,
        false,
    )
    .await
    .unwrap();
    assert!(!legacy_evidence.coverage_complete);
    let reviewed =
        recheck_envelope_for_review_with(&filter(), &mut envelope, PiiClassifyPolicy::AllEvents)
            .await
            .unwrap();
    assert!(reviewed.coverage_complete);
    assert!(reviewed.residual_scan_complete);
    assert_eq!(envelope.events.len(), 5_000);
}

#[tokio::test]
async fn review_recheck_fixed_profile_caps_still_fail_before_external_calls() {
    let adapter = RecordingFilter(std::sync::Mutex::new(Vec::new()));
    let mut budget = StructuredPayloadBudget::review();
    assert_eq!(budget.limits.aggregate_bytes, 32 * 1024 * 1024);
    assert_eq!(budget.limits.field_bytes, 32 * 1024 * 1024);
    assert_eq!(budget.limits.nodes, 4_194_304);
    assert_eq!(budget.limits.depth, 24);
    budget.aggregate_bytes = budget.limits.aggregate_bytes - 1;
    let mut side = ReviewSideProse::new(&adapter, budget);
    let mut text = "more than one byte".to_owned();
    assert!(!side.redact(&mut text).await.unwrap());
    assert!(side.report.coverage_incomplete);
    assert!(adapter.0.lock().unwrap().is_empty());

    let mut budget = StructuredPayloadBudget::review();
    // Exercise the same fixed-limit code with a tiny private fixture so the
    // test need not allocate a 32 MiB+ field just to prove it is not sent.
    budget.limits.field_bytes = 16;
    let mut side = ReviewSideProse::new(&adapter, budget);
    let mut text = "this exceeds the private test field limit".to_owned();
    assert!(!side.redact(&mut text).await.unwrap());
    assert!(side.report.coverage_incomplete);
    assert!(adapter.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn review_recheck_large_realistic_fields_are_locally_scrubbed_before_external_calls() {
    let mut envelope = envelope().await;
    let credential = format!("sk-{}", "aB3cD4eF5gH6iJ7kL8mN9pQ0rS1tU2vW3xY4zA5b");
    let mut field = "ordinary output ".repeat(3_000);
    assert!(field.len() > 47_000);
    field.push_str(&credential);
    envelope.events[0].tool_name = Some("read_file".into());
    envelope.events[0].event_type = TraceContributionEventType::ToolResult;
    envelope.events[0].structured_payload = serde_json::json!({"stdout":field.clone()});
    envelope.replay.expected_assertions = vec![serde_json::json!({"description":field})];
    envelope.events[0].redacted_content = Some("completed output ".repeat(145_000));
    assert!(envelope.events[0].redacted_content.as_ref().unwrap().len() > 2_300_000);
    let adapter = RecordingFilter(std::sync::Mutex::new(Vec::new()));
    let evidence =
        recheck_envelope_for_review_with(&adapter, &mut envelope, PiiClassifyPolicy::AllEvents)
            .await
            .unwrap();
    assert!(evidence.coverage_complete);
    assert!(evidence.residual_scan_complete);
    let calls = adapter.0.lock().unwrap();
    assert!(calls.iter().any(|text| text.len() > 2_300_000));
    assert!(
        calls
            .iter()
            .any(|text| text.len() > 47_000 && text.len() < 100_000)
    );
    assert!(calls.iter().all(|text| !text.contains(&credential)));
    assert!(
        !serde_json::to_string(&envelope.events)
            .unwrap()
            .contains(&credential)
    );
    assert!(
        !serde_json::to_string(&envelope.replay.expected_assertions)
            .unwrap()
            .contains(&credential)
    );
}

struct AuditedPersonFilter(std::sync::Mutex<Vec<String>>);

#[async_trait::async_trait]
impl PrivacyFilterAdapter for AuditedPersonFilter {
    async fn redact_text(
        &self,
        text: &str,
    ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
        self.0.lock().unwrap().push(text.into());
        filter().redact_text(text).await
    }
}

#[tokio::test]
async fn review_recheck_covers_event_failure_prose_and_scrubs_credentials_before_egress() {
    let mut envelope = envelope().await;
    let credential = format!("sk-{}", "aB3cD4eF5gH6iJ7kL8mN9pQ0rS1tU2vW3xY4zA5b");
    envelope.events[0].failure_modes = vec![TraceFailureMode::Other(format!(
        "Ask Jane Doe; the key is {credential}"
    ))];
    let adapter = AuditedPersonFilter(std::sync::Mutex::new(Vec::new()));
    let evidence =
        recheck_envelope_for_review_with(&adapter, &mut envelope, PiiClassifyPolicy::AllEvents)
            .await
            .unwrap();
    assert!(evidence.coverage_complete);
    assert!(evidence.useful_classifier_result);
    let TraceFailureMode::Other(detail) = &envelope.events[0].failure_modes[0] else {
        panic!("fixture changed");
    };
    assert!(!detail.contains("Jane Doe"));
    assert!(!detail.contains(&credential));
    assert!(
        adapter
            .0
            .lock()
            .unwrap()
            .iter()
            .all(|text| !text.contains(&credential))
    );
}

#[tokio::test]
async fn review_recheck_holds_original_email_and_phone_keys_without_rewriting() {
    for key in ["ada@example.com", "Call (415) 555-0132"] {
        let mut envelope = envelope().await;
        envelope
            .ironclaw
            .feature_flags
            .insert(key.into(), "enabled".into());
        envelope
            .replay
            .tool_manifest_hashes
            .insert(key.into(), "manifest".into());
        envelope.events[0].structured_payload = serde_json::json!({key:"content"});
        let original = serde_json::to_vec(&envelope).unwrap();
        let adapter = RecordingFilter(std::sync::Mutex::new(Vec::new()));
        let evidence =
            recheck_envelope_for_review_with(&adapter, &mut envelope, PiiClassifyPolicy::AllEvents)
                .await
                .unwrap();
        assert!(adapter.0.lock().unwrap().is_empty());
        assert!(envelope.ironclaw.feature_flags.contains_key(key));
        assert!(envelope.replay.tool_manifest_hashes.contains_key(key));
        assert!(envelope.events[0].structured_payload.get(key).is_some());
        assert!(evidence.basis.contains(&ResidualRiskCondition::KeyFinding));
        assert!(!evidence.coverage_complete);
        assert_eq!(envelope.privacy.residual_pii_risk, ResidualPiiRisk::High);
        assert_eq!(serde_json::to_vec(&envelope).unwrap(), original);
    }
}

#[tokio::test]
async fn review_recheck_preserves_all_benign_path_map_keys_and_values() {
    let mut envelope = envelope().await;
    let keys = ["/Users/private/project/a", "/Users/private/project/b"];
    for (index, key) in keys.into_iter().enumerate() {
        envelope
            .ironclaw
            .feature_flags
            .insert(key.into(), format!("enabled{index}"));
        envelope
            .replay
            .tool_manifest_hashes
            .insert(key.into(), format!("manifest{index}"));
    }
    envelope.replay.replay_notes = vec!["Ask Jane Doe.".into()];
    let flags = envelope.ironclaw.feature_flags.clone();
    let manifests = envelope.replay.tool_manifest_hashes.clone();
    let evidence =
        recheck_envelope_for_review_with(&filter(), &mut envelope, PiiClassifyPolicy::AllEvents)
            .await
            .unwrap();
    assert_eq!(envelope.ironclaw.feature_flags, flags);
    assert_eq!(envelope.replay.tool_manifest_hashes, manifests);
    assert!(evidence.coverage_complete);
    assert_eq!(envelope.privacy.residual_pii_risk, ResidualPiiRisk::Medium);
}

#[derive(Clone, Copy)]
enum ClassifierFault {
    Mismatch,
    MismatchEmpty,
    RewriteWithoutCounts,
    KeyFindingWithoutCounts,
    IncompleteWithoutCounts,
}

struct FaultFilter {
    marker: &'static str,
    fault: ClassifierFault,
}

#[async_trait::async_trait]
impl PrivacyFilterAdapter for FaultFilter {
    async fn redact_text(
        &self,
        text: &str,
    ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
        if text != self.marker {
            return Ok(None);
        }
        let mut report = RedactionReport::default();
        if matches!(self.fault, ClassifierFault::Mismatch) {
            report
                .counts
                .insert("privacy_filter:private_person".into(), 1);
        }
        report.key_finding_detected =
            matches!(self.fault, ClassifierFault::KeyFindingWithoutCounts);
        report.coverage_incomplete = matches!(self.fault, ClassifierFault::IncompleteWithoutCounts);
        let changed = matches!(
            self.fault,
            ClassifierFault::Mismatch
                | ClassifierFault::MismatchEmpty
                | ClassifierFault::RewriteWithoutCounts
        );
        Ok(Some(SafePrivacyFilterRedaction {
            private_edits: None,
            redacted_text: if changed {
                "misaligned_response_marker".into()
            } else {
                text.into()
            },
            report,
            summary: SafePrivacyFilterSummary {
                schema_version: 1,
                output_mode: "redacted_text_only".into(),
                span_count: 0,
                by_label: BTreeMap::new(),
                decoded_mismatch: matches!(
                    self.fault,
                    ClassifierFault::Mismatch | ClassifierFault::MismatchEmpty
                ),
                classify_policy: None,
                events_examined: 0,
                events_skipped_by_policy: 0,
            },
        }))
    }
}

#[tokio::test]
async fn review_recheck_rejects_mismatched_event_structured_and_side_outputs_without_applying_edits()
 {
    for fault in [ClassifierFault::Mismatch, ClassifierFault::MismatchEmpty] {
        for surface in [
            "event",
            "structured_value",
            "structured_key",
            "side_prose",
            "metadata_key",
        ] {
            let marker = "alignment_fixture_marker";
            let mut envelope = envelope().await;
            envelope.events[0].tool_name = Some("read_file".into());
            match surface {
                "event" => envelope.events[0].redacted_content = Some(marker.into()),
                "structured_value" => {
                    envelope.events[0].structured_payload = serde_json::json!({"stdout":marker})
                }
                "structured_key" => {
                    envelope.events[0].structured_payload = serde_json::json!({marker:"completed"})
                }
                "metadata_key" => {
                    envelope
                        .ironclaw
                        .feature_flags
                        .insert(marker.into(), "enabled".into());
                }
                _ => envelope.replay.replay_notes = vec![marker.into()],
            }
            let adapter = FaultFilter { marker, fault };
            let evidence = recheck_envelope_for_review_with(
                &adapter,
                &mut envelope,
                PiiClassifyPolicy::AllEvents,
            )
            .await
            .unwrap();
            assert!(!evidence.coverage_complete, "{surface}");
            assert!(
                evidence
                    .basis
                    .contains(&ResidualRiskCondition::CoverageIncomplete),
                "{surface}"
            );
            assert_eq!(envelope.privacy.residual_pii_risk, ResidualPiiRisk::High);
            assert!(
                !serde_json::to_string(&envelope)
                    .unwrap()
                    .contains("misaligned_response_marker")
            );
            assert!(
                !envelope
                    .privacy
                    .redaction_counts
                    .contains_key("privacy_filter:private_person")
            );
            match surface {
                "event" => assert_eq!(envelope.events[0].redacted_content.as_deref(), Some(marker)),
                "structured_value" => {
                    assert_eq!(envelope.events[0].structured_payload["stdout"], marker)
                }
                "structured_key" => {
                    assert!(envelope.events[0].structured_payload.get(marker).is_some())
                }
                "metadata_key" => assert!(envelope.ironclaw.feature_flags.contains_key(marker)),
                _ => assert_eq!(envelope.replay.replay_notes[0], marker),
            }
        }
    }
}

#[tokio::test]
async fn review_recheck_holds_key_rewrites_and_flags_even_without_counts() {
    for fault in [
        ClassifierFault::RewriteWithoutCounts,
        ClassifierFault::KeyFindingWithoutCounts,
        ClassifierFault::IncompleteWithoutCounts,
    ] {
        for surface in ["structured_key", "metadata_key"] {
            let marker = "key_fixture_marker";
            let mut envelope = envelope().await;
            if surface == "structured_key" {
                envelope.events[0].structured_payload = serde_json::json!({marker:"completed"});
            } else {
                envelope
                    .ironclaw
                    .feature_flags
                    .insert(marker.into(), "enabled".into());
            }
            let adapter = FaultFilter { marker, fault };
            let evidence = recheck_envelope_for_review_with(
                &adapter,
                &mut envelope,
                PiiClassifyPolicy::AllEvents,
            )
            .await
            .unwrap();
            assert!(!evidence.coverage_complete);
            assert_eq!(envelope.privacy.residual_pii_risk, ResidualPiiRisk::High);
            assert!(
                evidence.basis.contains(&ResidualRiskCondition::KeyFinding)
                    || evidence
                        .basis
                        .contains(&ResidualRiskCondition::CoverageIncomplete)
            );
            if surface == "structured_key" {
                assert!(envelope.events[0].structured_payload.get(marker).is_some());
            } else {
                assert!(envelope.ironclaw.feature_flags.contains_key(marker));
            }
            assert!(
                !serde_json::to_string(&envelope)
                    .unwrap()
                    .contains("misaligned_response_marker")
            );
        }
    }
}
