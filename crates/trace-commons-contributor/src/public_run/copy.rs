use serde::{Deserialize, Serialize};

use trace_commons_protocol::public_run::{PublicRunEvidenceDraft, PublicRunReusePermission};

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct PublicRunReuseChoice {
    pub permission: PublicRunReusePermission,
    pub label: &'static str,
    pub explanation: &'static str,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct PublicRunValueLabel {
    pub value: &'static str,
    pub label: &'static str,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct PublicRunEditorInput {
    pub title: String,
    pub outcome_summary: String,
    pub correction_excerpt: Option<String>,
    pub workflow: String,
    pub reuse_permission: Option<PublicRunReusePermission>,
    pub evidence: Vec<PublicRunEvidenceDraft>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PublicRunEditorValidation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft: Option<trace_commons_protocol::public_run::PublicRunDraft>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<&'static str>,
}

/// Every fixed word on the native session-detail and publication surface.
///
/// This is exported as one JSON object so every shell receives the complete
/// disclosure, review, validation, and action vocabulary from the contributor
/// crate. A shell that cannot decode the whole payload renders no publication
/// controls rather than filling a missing disclosure with local copy.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PublicRunCopy {
    pub all_contributions: &'static str,
    pub view_session: &'static str,
    pub session_detail: &'static str,
    pub reading_record: &'static str,
    pub retry_read: &'static str,
    pub content_unavailable: &'static str,
    pub unavailable_value: &'static str,
    pub creator_report: &'static str,
    pub task: &'static str,
    pub no_task: &'static str,
    pub outcome: &'static str,
    pub outcome_unavailable: &'static str,
    pub decisive_correction: &'static str,
    pub no_correction: &'static str,
    pub supporting_evidence: &'static str,
    pub observed_in_version: &'static str,
    pub no_evidence: &'static str,
    pub contributed_version: &'static str,
    pub contribution_details: &'static str,
    pub processing_status: &'static str,
    pub permitted_uses: &'static str,
    pub no_permitted_uses: &'static str,
    pub permitted_uses_unavailable: &'static str,
    pub unrecognized_value: &'static str,
    pub next_action: &'static str,
    pub withdraw: &'static str,
    pub keep_contribution: &'static str,
    pub envelope_version: &'static str,
    pub consent_policy_version: &'static str,
    pub redaction_version: &'static str,
    pub publication_after_acceptance: &'static str,
    pub public_workflow: &'static str,
    pub published: &'static str,
    pub open_page: &'static str,
    pub edit_page: &'static str,
    pub unpublishing: &'static str,
    pub unpublish: &'static str,
    pub publication_disclosure: &'static str,
    pub page_title: &'static str,
    pub public_outcome: &'static str,
    pub reusable_instructions: &'static str,
    pub select_evidence: &'static str,
    pub publish_correction: &'static str,
    pub reuse_permission: &'static str,
    pub choose_permission: &'static str,
    pub source_public_run: &'static str,
    pub source_placeholder: &'static str,
    pub source_help: &'static str,
    pub cancel_edit: &'static str,
    pub review_page: &'static str,
    pub exact_public_preview: &'static str,
    pub observed_evidence: &'static str,
    pub use_workflow: &'static str,
    pub edit_draft: &'static str,
    pub publishing: &'static str,
    pub publish_page: &'static str,
    pub update_page: &'static str,
    pub validation_required: &'static str,
    pub validation_too_long: &'static str,
    pub validation_correction_too_long: &'static str,
    pub validation_evidence_required: &'static str,
    pub validation_permission_required: &'static str,
    pub validation_source_invalid: &'static str,
    pub session_account_required: &'static str,
    pub session_not_found: &'static str,
    pub session_unavailable: &'static str,
    pub publication_account_required: &'static str,
    pub publication_conflict: &'static str,
    pub publication_trace_not_found: &'static str,
    pub publication_source_not_found: &'static str,
    pub publication_invalid: &'static str,
    pub publication_unavailable: &'static str,
    pub credential_storage_warning: &'static str,
    pub task_outcome_choices: [PublicRunValueLabel; 4],
    pub feedback_choices: [PublicRunValueLabel; 3],
    pub evidence_kind_choices: [PublicRunValueLabel; 9],
    pub contribution_status_choices: [PublicRunValueLabel; 10],
    pub permitted_use_choices: [PublicRunValueLabel; 6],
    pub reuse_permissions: [PublicRunReuseChoice; 2],
}

#[must_use]
pub fn public_run_copy() -> PublicRunCopy {
    PublicRunCopy {
        all_contributions: "All contributions",
        view_session: "View session",
        session_detail: "Session detail",
        reading_record: "Reading your redacted contribution record…",
        retry_read: "Retry read",
        content_unavailable: "Session content is unavailable. Status and contribution metadata remain.",
        unavailable_value: "Unavailable",
        creator_report: "Creator report",
        task: "Task",
        no_task: "No redacted task text is available for this contribution.",
        outcome: "Outcome",
        outcome_unavailable: "No outcome was recorded for this contribution.",
        decisive_correction: "Decisive correction",
        no_correction: "No correction was contributed for this session.",
        supporting_evidence: "Supporting evidence",
        observed_in_version: "Observed in the contributed version",
        no_evidence: "No redacted text evidence is available.",
        contributed_version: "Contributed version",
        contribution_details: "Contribution details",
        processing_status: "Processing status",
        permitted_uses: "Permitted uses",
        no_permitted_uses: "No permitted uses were recorded for this contribution.",
        permitted_uses_unavailable: "Permitted-use details are unavailable from this version of the server.",
        unrecognized_value: "Unrecognized",
        next_action: "Contribution controls",
        withdraw: "Withdraw",
        keep_contribution: "Keep it",
        envelope_version: "Envelope",
        consent_policy_version: "Consent policy",
        redaction_version: "Redaction",
        publication_after_acceptance: "A public page becomes available after this contribution is accepted.",
        public_workflow: "Public workflow",
        published: "Published",
        open_page: "Open page",
        edit_page: "Edit page",
        unpublishing: "Unpublishing…",
        unpublish: "Unpublish",
        publication_disclosure: "Choose the exact fields that will be public. Session publication is separate from Commons contribution and profile attribution.",
        page_title: "Page title",
        public_outcome: "Public outcome summary",
        reusable_instructions: "Reusable instructions",
        select_evidence: "Select one to four exact excerpts from the redacted contribution.",
        publish_correction: "Publish the contributed correction",
        reuse_permission: "Reuse permission",
        choose_permission: "Choose permission",
        source_public_run: "Source public run",
        source_placeholder: "Optional tracecommons.ai/runs link or slug",
        source_help: "Add this when the workflow varies an existing public run.",
        cancel_edit: "Cancel edit",
        review_page: "Create public page",
        exact_public_preview: "Exact public preview",
        observed_evidence: "Observed evidence",
        use_workflow: "Use workflow",
        edit_draft: "Edit draft",
        publishing: "Publishing…",
        publish_page: "Publish page",
        update_page: "Update page",
        validation_required: "Add a title, outcome summary, and reusable instructions.",
        validation_too_long: "Shorten the field that exceeds its limit.",
        validation_correction_too_long: "The contributed correction is too long to publish as one excerpt.",
        validation_evidence_required: "Select at least one supporting excerpt.",
        validation_permission_required: "Choose a reuse permission.",
        validation_source_invalid: "Use a public run slug or a tracecommons.ai/runs link.",
        session_account_required: "Sign in to your Trace Commons account to read this session.",
        session_not_found: "This contribution is no longer available to this account.",
        session_unavailable: "The redacted session record could not be read. Retry the read.",
        publication_account_required: "Sign in to your Trace Commons account, then retry.",
        publication_conflict: "The contribution or approval changed. Review the page again.",
        publication_trace_not_found: "This contribution is no longer available to this account.",
        publication_source_not_found: "The source public run is no longer available.",
        publication_invalid: "The page was refused. Remove private data, verify the evidence, and review it again.",
        publication_unavailable: "The public page could not be changed. Retry the request.",
        credential_storage_warning: "The page changed, but the rotated account session could not be saved. Sign in again before the next account action.",
        task_outcome_choices: [
            PublicRunValueLabel {
                value: "success",
                label: "Completed",
            },
            PublicRunValueLabel {
                value: "partial",
                label: "Partly completed",
            },
            PublicRunValueLabel {
                value: "failure",
                label: "Did not complete",
            },
            PublicRunValueLabel {
                value: "unknown",
                label: "No outcome reported",
            },
        ],
        feedback_choices: [
            PublicRunValueLabel {
                value: "thumbs_up",
                label: "Feedback: thumbs up",
            },
            PublicRunValueLabel {
                value: "thumbs_down",
                label: "Feedback: thumbs down",
            },
            PublicRunValueLabel {
                value: "correction",
                label: "Feedback: correction supplied",
            },
        ],
        evidence_kind_choices: [
            PublicRunValueLabel {
                value: "user_message",
                label: "User message",
            },
            PublicRunValueLabel {
                value: "assistant_message",
                label: "Assistant message",
            },
            PublicRunValueLabel {
                value: "reasoning",
                label: "Reasoning",
            },
            PublicRunValueLabel {
                value: "tool_call",
                label: "Tool call",
            },
            PublicRunValueLabel {
                value: "tool_result",
                label: "Tool result",
            },
            PublicRunValueLabel {
                value: "routing_decision",
                label: "Routing decision",
            },
            PublicRunValueLabel {
                value: "feedback",
                label: "Feedback",
            },
            PublicRunValueLabel {
                value: "http_exchange",
                label: "HTTP exchange",
            },
            PublicRunValueLabel {
                value: "unknown",
                label: "Evidence",
            },
        ],
        contribution_status_choices: [
            PublicRunValueLabel {
                value: "submitted",
                label: "Submitted",
            },
            PublicRunValueLabel {
                value: "received",
                label: "Received",
            },
            PublicRunValueLabel {
                value: "accepted",
                label: "Accepted into the commons",
            },
            PublicRunValueLabel {
                value: "quarantined",
                label: "Held for privacy review",
            },
            PublicRunValueLabel {
                value: "awaiting_pii_backstop",
                label: "Waiting for privacy review",
            },
            PublicRunValueLabel {
                value: "rejected",
                label: "Rejected",
            },
            PublicRunValueLabel {
                value: "revoked",
                label: "Withdrawn",
            },
            PublicRunValueLabel {
                value: "withdrawn",
                label: "Withdrawn",
            },
            PublicRunValueLabel {
                value: "expired",
                label: "Expired",
            },
            PublicRunValueLabel {
                value: "purged",
                label: "Purged",
            },
        ],
        permitted_use_choices: [
            PublicRunValueLabel {
                value: "debugging",
                label: "Debugging",
            },
            PublicRunValueLabel {
                value: "evaluation",
                label: "Evaluation",
            },
            PublicRunValueLabel {
                value: "benchmark_generation",
                label: "Benchmark creation",
            },
            PublicRunValueLabel {
                value: "ranking_model_training",
                label: "Ranking-model training",
            },
            PublicRunValueLabel {
                value: "model_training",
                label: "Model training",
            },
            PublicRunValueLabel {
                value: "aggregate_analytics",
                label: "Aggregate analytics",
            },
        ],
        reuse_permissions: [
            PublicRunReuseChoice {
                permission: PublicRunReusePermission::CcBy40,
                label: PublicRunReusePermission::CcBy40.label(),
                explanation: "Others may reuse it with attribution.",
            },
            PublicRunReuseChoice {
                permission: PublicRunReusePermission::Cc0_10,
                label: PublicRunReusePermission::Cc0_10.label(),
                explanation: "Others may reuse it without attribution.",
            },
        ],
    }
}

#[must_use]
pub fn validate_public_run_editor(input: PublicRunEditorInput) -> PublicRunEditorValidation {
    use trace_commons_protocol::public_run::{
        PublicRunDraft, PublicRunValidationError, validate_slug,
    };

    let copy = public_run_copy();
    let Some(reuse_permission) = input.reuse_permission else {
        return PublicRunEditorValidation {
            draft: None,
            error: Some(copy.validation_permission_required),
        };
    };
    let source_slug = match normalize_source_slug(&input.source) {
        Ok(source_slug) => source_slug,
        Err(()) => {
            return PublicRunEditorValidation {
                draft: None,
                error: Some(copy.validation_source_invalid),
            };
        }
    };
    if let Some(slug) = source_slug.as_deref()
        && validate_slug(slug).is_err()
    {
        return PublicRunEditorValidation {
            draft: None,
            error: Some(copy.validation_source_invalid),
        };
    }
    let draft = PublicRunDraft {
        title: input.title.trim().to_string(),
        outcome_summary: input.outcome_summary.trim().to_string(),
        correction_excerpt: input.correction_excerpt,
        workflow: input.workflow.trim().to_string(),
        reuse_permission,
        evidence: input.evidence,
        source_slug,
    };
    let error = match draft.validate() {
        Ok(()) => None,
        Err(PublicRunValidationError::EmptyField) => Some(copy.validation_required),
        Err(PublicRunValidationError::FieldTooLong)
            if draft.correction_excerpt.as_ref().is_some_and(|correction| {
                correction.chars().count()
                    > trace_commons_protocol::public_run::PUBLIC_RUN_CORRECTION_MAX_CHARS
            }) =>
        {
            Some(copy.validation_correction_too_long)
        }
        Err(PublicRunValidationError::FieldTooLong) => Some(copy.validation_too_long),
        Err(PublicRunValidationError::EvidenceCount)
        | Err(PublicRunValidationError::DuplicateEvidence) => {
            Some(copy.validation_evidence_required)
        }
        Err(PublicRunValidationError::InvalidSourceSlug) => Some(copy.validation_source_invalid),
        Err(_) => Some(copy.publication_invalid),
    };
    PublicRunEditorValidation {
        draft: error.is_none().then_some(draft),
        error,
    }
}

fn normalize_source_slug(source: &str) -> Result<Option<String>, ()> {
    let source = source.trim();
    if source.is_empty() {
        return Ok(None);
    }
    if !source.contains("://") {
        return Ok(Some(source.to_string()));
    }
    let url = url::Url::parse(source).map_err(|_| ())?;
    if url.scheme() != "https"
        || url.host_str() != Some("tracecommons.ai")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(());
    }
    let segments = url.path_segments().ok_or(())?.collect::<Vec<_>>();
    match segments.as_slice() {
        ["runs", slug] => Ok(Some((*slug).to_string())),
        _ => Err(()),
    }
}

#[must_use]
pub fn session_detail_error_line(label: &str) -> &'static str {
    let copy = public_run_copy();
    match label {
        "account-session-required" => copy.session_account_required,
        "session-detail-not-found" => copy.session_not_found,
        _ => copy.session_unavailable,
    }
}

#[must_use]
pub fn publication_error_line(label: &str) -> &'static str {
    let copy = public_run_copy();
    match label {
        "account-session-required" => copy.publication_account_required,
        "public-run-conflict" => copy.publication_conflict,
        "public-run-trace-not-found" => copy.publication_trace_not_found,
        "public-run-source-not-found" => copy.publication_source_not_found,
        "public-run-invalid" => copy.publication_invalid,
        "commons_credential_storage_unavailable" => copy.credential_storage_warning,
        _ => copy.publication_unavailable,
    }
}

#[cfg(test)]
mod tests {
    use trace_commons_protocol::public_run::{PublicRunEvidenceDraft, PublicRunReusePermission};
    use uuid::Uuid;

    use super::*;

    #[test]
    fn publication_copy_is_complete_and_error_labels_have_fixed_fallbacks() {
        let copy = public_run_copy();
        assert_eq!(copy.reuse_permissions.len(), 2);
        assert_eq!(copy.task_outcome_choices.len(), 4);
        assert_eq!(copy.feedback_choices.len(), 3);
        assert_eq!(copy.evidence_kind_choices.len(), 9);
        assert_eq!(copy.contribution_status_choices.len(), 10);
        assert_eq!(copy.permitted_use_choices.len(), 6);
        assert!(
            copy.contribution_status_choices
                .iter()
                .any(|choice| choice.value == "awaiting_pii_backstop")
        );
        assert!(
            copy.permitted_use_choices
                .iter()
                .any(|choice| choice.value == "model_training")
        );
        assert_eq!(
            copy.reuse_permissions[0].permission,
            PublicRunReusePermission::CcBy40
        );
        assert_eq!(
            copy.reuse_permissions[1].permission,
            PublicRunReusePermission::Cc0_10
        );
        assert_eq!(
            session_detail_error_line("session-detail-not-found"),
            copy.session_not_found
        );
        assert_eq!(
            session_detail_error_line("future-detail-error"),
            copy.session_unavailable
        );
        assert_eq!(
            publication_error_line("public-run-conflict"),
            copy.publication_conflict
        );
        assert_eq!(
            publication_error_line("future-publication-error"),
            copy.publication_unavailable
        );
    }

    #[test]
    fn editor_validation_uses_the_shared_contract_and_normalizes_source_urls() {
        let input = PublicRunEditorInput {
            title: "  Repair a stalled upload  ".to_string(),
            outcome_summary: "  The upload completed.  ".to_string(),
            correction_excerpt: None,
            workflow: "  Renew the session, then retry once.  ".to_string(),
            reuse_permission: Some(PublicRunReusePermission::CcBy40),
            evidence: vec![PublicRunEvidenceDraft {
                event_id: Uuid::nil(),
                excerpt: "The retry succeeded.".to_string(),
            }],
            source: "https://tracecommons.ai/runs/run-source-workflow".to_string(),
        };
        let valid = validate_public_run_editor(input.clone());
        let draft = valid.draft.expect("valid draft");
        assert!(valid.error.is_none());
        assert_eq!(draft.title, "Repair a stalled upload");
        assert_eq!(draft.source_slug.as_deref(), Some("run-source-workflow"));

        let mut invalid = input;
        invalid.source = "https://example.com/runs/run-source-workflow".to_string();
        let result = validate_public_run_editor(invalid);
        assert!(result.draft.is_none());
        assert_eq!(
            result.error,
            Some(public_run_copy().validation_source_invalid)
        );
    }

    #[test]
    fn editor_validation_reports_missing_permission_evidence_and_oversized_text() {
        let base = PublicRunEditorInput {
            title: "Repair a stalled upload".to_string(),
            outcome_summary: "The upload completed.".to_string(),
            correction_excerpt: None,
            workflow: "Renew the session, then retry once.".to_string(),
            reuse_permission: None,
            evidence: Vec::new(),
            source: String::new(),
        };
        assert_eq!(
            validate_public_run_editor(base.clone()).error,
            Some(public_run_copy().validation_permission_required)
        );
        let mut missing_evidence = base;
        missing_evidence.reuse_permission = Some(PublicRunReusePermission::CcBy40);
        assert_eq!(
            validate_public_run_editor(missing_evidence.clone()).error,
            Some(public_run_copy().validation_evidence_required)
        );
        missing_evidence.evidence.push(PublicRunEvidenceDraft {
            event_id: Uuid::nil(),
            excerpt: "The retry succeeded.".to_string(),
        });
        missing_evidence.title = "t".repeat(101);
        assert_eq!(
            validate_public_run_editor(missing_evidence).error,
            Some(public_run_copy().validation_too_long)
        );
    }

    #[test]
    fn editor_validation_maps_the_exact_correction_limit() {
        let mut input = PublicRunEditorInput {
            title: "Repair a stalled upload".to_string(),
            outcome_summary: "The upload completed.".to_string(),
            correction_excerpt: Some(
                "c".repeat(trace_commons_protocol::public_run::PUBLIC_RUN_CORRECTION_MAX_CHARS),
            ),
            workflow: "Renew the session, then retry once.".to_string(),
            reuse_permission: Some(PublicRunReusePermission::CcBy40),
            evidence: vec![PublicRunEvidenceDraft {
                event_id: Uuid::nil(),
                excerpt: "The retry succeeded.".to_string(),
            }],
            source: String::new(),
        };
        assert!(validate_public_run_editor(input.clone()).error.is_none());

        input.correction_excerpt = Some(
            "c".repeat(trace_commons_protocol::public_run::PUBLIC_RUN_CORRECTION_MAX_CHARS + 1),
        );
        assert_eq!(
            validate_public_run_editor(input).error,
            Some(public_run_copy().validation_correction_too_long)
        );
    }
}
