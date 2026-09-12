//! Typed presentation of declarations and user-linked artifacts. No attribution
//! or outcome arithmetic is performed by the shell.
use super::{copy, local_date};
use trace_commons_contributor::insights::{
    OutcomeLink,
    claude_task_attribution::{ClaudeTaskAttributionEvidence, ClaudeTaskAttributionState},
    models::{DeclarationKind, ModelObservations, RecordCoordinates},
    outcomes::OutcomeEvidence,
};

pub(super) fn render_claude_attribution(
    evidence: Option<&ClaudeTaskAttributionEvidence>,
) -> Option<String> {
    let evidence = evidence?;
    let mut text = format!(
        "{}\n{}\n{}: {}\n",
        copy("claude_attribution_title"),
        copy("claude_attribution_notice"),
        copy("source_digest"),
        evidence.source_digest
    );
    match &evidence.state {
        ClaudeTaskAttributionState::Attributed { branch } => text.push_str(&format!(
            "{}\n{}: {}\n{}: {}\n{}: {}",
            copy("claude_attribution_attributed"),
            copy("claude_attribution_terminal_line"),
            branch.terminal_physical_line,
            copy("claude_attribution_tool_pairs"),
            branch.tool_pairs.len(),
            copy("claude_attribution_background"),
            branch.background_work.len()
        )),
        ClaudeTaskAttributionState::Unavailable { .. } => {
            text.push_str(copy("claude_attribution_unavailable"));
        }
    }
    Some(text)
}

pub(super) fn render_models(observation: Option<&ModelObservations>) -> String {
    let mut text = format!("{}\n{}\n", copy("model_title"), copy("model_notice"));
    let Some(model) = observation else {
        text.push_str(&format!(
            "{}: {}\n{}",
            copy("model_labels"),
            copy("unknown"),
            copy("model_legacy")
        ));
        return text;
    };
    text.push_str(&format!(
        "{}: {}\n{}\n{}: {}\n",
        copy("model_labels"),
        if model.declared_models.is_empty() {
            copy("unknown").to_owned()
        } else {
            model.declared_models.join(", ")
        },
        copy(if model.mixed_declared_models {
            "model_mixed"
        } else {
            "model_not_proven_mixed"
        }),
        copy("source_digest"),
        model.source_digest
    ));
    if model.declared_models.is_empty() {
        text.push_str(copy("model_no_labels"));
        text.push('\n');
    }
    for (key, value) in [
        ("model_record_count", model.record_count),
        ("model_candidates", model.candidate_records),
        ("model_valid", model.valid_declarations),
        ("model_missing", model.missing_declarations),
        ("model_invalid", model.invalid_declarations),
        ("model_omitted", model.omitted_declarations),
    ] {
        text.push_str(&format!("{}: {}\n", copy(key), value));
    }
    if model.model_labels_omitted {
        text.push_str(copy("model_labels_omitted"));
        text.push('\n');
    }
    text.push_str(copy(match model.coordinates {
        RecordCoordinates::JsonlPhysicalLinesOneBased => {
            "model_coordinates_jsonl_physical_lines_one_based"
        }
        RecordCoordinates::TrajectoryArrayIndexesZeroBased => {
            "model_coordinates_trajectory_array_indexes_zero_based"
        }
    }));
    text.push_str(&format!("\n{}\n", copy("model_references")));
    if model.declarations.is_empty() {
        text.push_str(copy("unknown"));
    }
    for declaration in &model.declarations {
        text.push_str(&format!(
            "{} · {}: {} · {}\n",
            declaration.model,
            copy("model_record_index"),
            declaration.record_index,
            copy(match declaration.kind {
                DeclarationKind::CodexSessionMetadata => "model_kind_codex_session_metadata",
                DeclarationKind::CodexTurnContext => "model_kind_codex_turn_context",
                DeclarationKind::CodexAssistantMessage => "model_kind_codex_assistant_message",
                DeclarationKind::ClaudeAssistantMessage => {
                    "model_kind_claude_assistant_message"
                }
                DeclarationKind::TrajectoryMetadata => "model_kind_trajectory_metadata",
            })
        ));
    }

    text
}

pub(super) fn render_link(link: &OutcomeLink) -> String {
    let mut text = format!(
        "{}\n{}: {}\n{}: {}\n{}: {}\n",
        copy("link_user_provenance"),
        copy("link_id"),
        link.id,
        copy("source_digest"),
        link.source_digest,
        copy("link_recorded_at"),
        local_date(&link.linked_at)
    );
    match &link.evidence {
        OutcomeEvidence::GitCommit(git) => {
            text.push_str(&format!(
                "{}\n{}: {}\n{}: {}\n{}: {}\n{}: {}\n{}: {}\n",
                copy("link_git_provenance"),
                copy("git_repository_digest"),
                git.repository_path_digest,
                copy("git_object"),
                git.object_id,
                copy("git_tree"),
                git.tree_id,
                copy("git_parents"),
                if git.parent_ids.is_empty() {
                    copy("git_no_parents").to_owned()
                } else {
                    git.parent_ids.join(", ")
                },
                copy("git_inspected_at"),
                local_date(&git.inspected_at)
            ));
        }
        OutcomeEvidence::TestReport(report) => {
            text.push_str(&format!(
                "{}\n{}: {}\n{}: {}\n{}: {}\n{}: {}\n{}: {}\n{}: {}\n{}: {}\n{}: {}\n",
                copy("link_test_provenance"),
                copy("test_runner"),
                report.runner,
                copy("test_passed"),
                report.passed,
                copy("test_failed"),
                report.failed,
                copy("test_skipped"),
                report.skipped,
                copy("test_observed_at"),
                local_date(&report.observed_at),
                copy("test_imported_at"),
                local_date(&report.imported_at),
                copy("artifact_digest"),
                report.artifact_digest,
                copy("test_claimed_commit"),
                report.commit_id.as_deref().unwrap_or(copy("unknown"))
            ));
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use trace_commons_contributor::insights::{
        OutcomeLinkProvenance, SourceFormat,
        claude_task_attribution::ClaudeTaskUnavailableReason,
        models::{ModelDeclaration, ModelObservationScope},
        outcomes::{
            GitCommitEvidence, GitEvidenceProvenance, TestEvidenceProvenance, TestReportEvidence,
        },
    };

    #[test]
    fn unavailable_claude_branch_keeps_the_scope_limitation_visible() {
        let evidence = ClaudeTaskAttributionEvidence {
            schema_version: 1,
            extractor_version: 1,
            profile_id: "claude-code-v2.1.260-observed-agent-branch-v1".into(),
            observed_writer_version: "2.1.260".into(),
            qualification_scope: "observed_writer_agent_branch_records".into(),
            source_digest: "a".repeat(64),
            record_count: 3,
            recognized_records: 2,
            root_session_identity_sha256: None,
            agent_branch_identity_sha256: None,
            declared_model: None,
            model_selector: None,
            context_window_selector: None,
            state: ClaudeTaskAttributionState::Unavailable {
                reason: ClaudeTaskUnavailableReason::UnsupportedRecord,
                physical_line: Some(3),
            },
        };
        evidence.validate().unwrap();
        let text = render_claude_attribution(Some(&evidence)).unwrap();
        assert!(text.contains(copy("claude_attribution_notice")));
        assert!(text.contains(copy("claude_attribution_unavailable")));
        assert!(text.contains(&evidence.source_digest));
    }

    #[test]
    fn claude_declarations_render_physical_line_evidence() {
        let observation = ModelObservations {
            schema_version: 3,
            scope: ModelObservationScope::DeclaredMetadataOnly,
            source_format: SourceFormat::ClaudeCode,
            source_digest: "c".repeat(64),
            coordinates: RecordCoordinates::JsonlPhysicalLinesOneBased,
            record_count: 7,
            candidate_records: 4,
            valid_declarations: 2,
            missing_declarations: 1,
            invalid_declarations: 1,
            omitted_declarations: 0,
            model_labels_omitted: false,
            mixed_declared_models: false,
            declared_models: vec!["claude-declared".into()],
            declarations: vec![
                ModelDeclaration {
                    model: "claude-declared".into(),
                    record_index: 4,
                    kind: DeclarationKind::ClaudeAssistantMessage,
                },
                ModelDeclaration {
                    model: "claude-declared".into(),
                    record_index: 6,
                    kind: DeclarationKind::ClaudeAssistantMessage,
                },
            ],
        };

        observation.validate().unwrap();
        let text = render_models(Some(&observation));
        assert!(text.contains(copy("model_kind_claude_assistant_message")));
        assert!(text.contains(copy("model_coordinates_jsonl_physical_lines_one_based")));
        assert!(text.contains(&format!("{}: 4", copy("model_candidates"))));
        assert!(text.contains(&format!("{}: 2", copy("model_valid"))));
        assert!(text.contains(&format!("{}: 1", copy("model_missing"))));
        assert!(text.contains(&format!("{}: 1", copy("model_invalid"))));
        assert!(text.contains(&format!("{}: 6", copy("model_record_index"))));
    }

    #[test]
    fn declarations_preserve_missingness_bounds_and_coordinate_origin() {
        let legacy = render_models(None);
        assert!(legacy.contains(copy("unknown")));
        assert!(legacy.contains(copy("model_legacy")));
        let mut observation = ModelObservations {
            schema_version: 1,
            scope: ModelObservationScope::DeclaredMetadataOnly,
            source_format: SourceFormat::Codex,
            source_digest: "a".repeat(64),
            coordinates: RecordCoordinates::JsonlPhysicalLinesOneBased,
            record_count: 8,
            candidate_records: 6,
            valid_declarations: 3,
            missing_declarations: 2,
            invalid_declarations: 1,
            omitted_declarations: 2,
            model_labels_omitted: true,
            mixed_declared_models: false,
            declared_models: vec!["declared-only".into()],
            declarations: vec![ModelDeclaration {
                model: "declared-only".into(),
                record_index: 4,
                kind: DeclarationKind::CodexTurnContext,
            }],
        };
        let text = render_models(Some(&observation));
        for key in [
            "model_notice",
            "model_not_proven_mixed",
            "model_labels_omitted",
            "model_coordinates_jsonl_physical_lines_one_based",
            "model_kind_codex_turn_context",
        ] {
            assert!(text.contains(copy(key)), "{key}");
        }
        assert!(text.contains(&format!("{}: 2", copy("model_missing"))));
        assert!(text.contains(&format!("{}: 1", copy("model_invalid"))));
        assert!(text.contains(&format!("{}: 2", copy("model_omitted"))));
        assert!(text.contains(&format!("{}: 4", copy("model_record_index"))));
        assert!(text.contains(&observation.source_digest));
        observation.mixed_declared_models = true;
        observation.declared_models.push("other-declared".into());
        assert!(render_models(Some(&observation)).contains(copy("model_mixed")));
        observation.coordinates = RecordCoordinates::TrajectoryArrayIndexesZeroBased;
        observation.declarations[0].record_index = 0;
        observation.declarations[0].kind = DeclarationKind::TrajectoryMetadata;
        let text = render_models(Some(&observation));
        assert!(text.contains(copy(
            "model_coordinates_trajectory_array_indexes_zero_based"
        )));
        assert!(text.contains(&format!("{}: 0", copy("model_record_index"))));
    }

    #[test]
    fn linked_artifacts_keep_user_and_producer_authority_separate() {
        let now = chrono::Utc::now();
        let mut link = OutcomeLink {
            id: "link-id".into(),
            source_digest: "a".repeat(64),
            linked_at: now,
            provenance: OutcomeLinkProvenance::UserLinked,
            evidence: OutcomeEvidence::TestReport(TestReportEvidence {
                schema_version: 1,
                runner: "synthetic-producer".into(),
                passed: 0,
                failed: 2,
                skipped: 1,
                observed_at: now,
                commit_id: None,
                artifact_digest: "b".repeat(64),
                imported_at: now,
                provenance: TestEvidenceProvenance::ImportedReport,
            }),
        };
        let text = render_link(&link);
        assert!(text.contains(copy("link_user_provenance")));
        assert!(text.contains(copy("link_test_provenance")));
        assert!(text.contains(&format!("{}: 0", copy("test_passed"))));
        assert!(text.contains(&format!(
            "{}: {}",
            copy("test_claimed_commit"),
            copy("unknown")
        )));
        assert!(text.contains(&"b".repeat(64)));
        link.evidence = OutcomeEvidence::GitCommit(GitCommitEvidence {
            repository_path_digest: "c".repeat(64),
            object_id: "d".repeat(40),
            tree_id: "e".repeat(40),
            parent_ids: vec![],
            inspected_at: now,
            provenance: GitEvidenceProvenance::InspectedLocalObject,
        });
        let text = render_link(&link);
        assert!(text.contains(copy("link_git_provenance")));
        assert!(text.contains(copy("git_no_parents")));
        assert!(text.contains(&"c".repeat(64)));
        assert!(!text.contains(copy("link_test_provenance")));
    }
}
