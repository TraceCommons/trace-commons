//! Account-free local Insights entry point shared by native shells and CLI.
use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use super::usage::{UsageSource, UsageSummary, extract_usage};
use super::{
    LocalInsight, LocalInsightStore, SourceFormat, TaskCategory, TaskOutcome, analyze_file,
};

/// Bound request bytes before parsing or reading caller-owned FFI memory.
pub const MAX_REQUEST_BYTES: usize = 64 * 1024;

/// Shared desktop vocabulary; shells render observations without inventing claims.
pub fn ui_copy() -> std::collections::BTreeMap<String, String> {
    [
        ("title", "Insights"),
        ("intro", "Analyze a file on this device without an account or upload."),
        ("snapshot_notice", "This is a dated snapshot. Reimport the file to refresh it."),
        ("unknown_notice", "Cost, independently verified outcomes, and model comparisons need more evidence."),
        ("coverage_notice", "Coverage describes recognized evidence. Missing values are unknown, not zero."),
        ("cancellation_notice", "Closing this view stops updates to the screen. A save or deletion already started may finish."),
        ("assessment_notice", "Your assessment is user-reported and separate from verified outcomes."),
        ("evidence_notice", "Evidence identifies the source snapshot by digest. Event-level explanations are not available yet."),
        ("empty", "No saved insights. Choose a file to analyze; saving is optional."),
        ("error", "The local operation could not be completed. Check the selected file or saved snapshot and try again."),
        ("save_notice", "Saving reads the selected file again and stores derived observations. The original transcript stays on this device."),
        ("delete_notice", "Delete the saved insight and its references? The original file will remain intact."),
        ("boundary_notice", "One selected session; task boundaries have not been verified."),
        ("unsaved", "This analysis has not been saved."),
        ("saved_status", "Snapshot saved. The original file was not modified."),
        ("refreshed", "Saved insights refreshed. Source files were not read again."),
        ("deleted", "Saved insight deleted. The original file was not modified."),
        ("already_absent", "That saved insight is already absent."),
        ("cancelled", "Screen updates cancelled. Refresh saved insights after pending work finishes."),
        ("partial_notice", "Partial recognition: some source records are not classified."),
        ("choose_file", "Choose file"), ("analyze", "Analyze"),
        ("save", "Re-read and save"), ("refresh", "Refresh saved insights"),
        ("delete", "Delete saved insight"), ("cancel", "Cancel"),
        ("explain", "Show evidence"), ("save_assessment", "Save assessment"),
        ("clear_assessment", "Clear assessment"), ("contributions", "Contributions"),
        ("source", "Source format"), ("file", "Selected file"),
        ("saved", "Saved insights"), ("result", "Analysis"),
        ("provider", "Analyzer"), ("rubric", "Rubric version"),
        ("analyzed_at", "Analyzed at"), ("coverage", "Coverage"),
        ("evidence", "Evidence"), ("source_digest", "Source digest"),
        ("assessment", "Your assessment"), ("category", "Task category"),
        ("outcome", "Outcome"), ("recorded_at", "Recorded at"),
        ("unknown", "Unknown"), ("working", "Working…"),
        ("no_file", "No file selected"), ("cost", "Estimated cost"),
        ("codex", "Codex rollout"), ("trajectory", "Trajectory"),
        ("metric_sessions", "Sessions"), ("metric_events", "Events"),
        ("metric_input_tokens", "Input tokens"), ("metric_output_tokens", "Output tokens"),
        ("metric_tool_calls", "Tool calls"), ("metric_tool_failures", "Reported tool failures"),
        ("metric_known_outcomes", "Verified outcomes"),
        ("category_refactor", "Refactor"), ("category_tests", "Tests"),
        ("category_docs", "Documentation"), ("category_debugging", "Debugging"),
        ("category_other", "Other"), ("category_unknown", "Unknown"),
        ("outcome_accepted", "Accepted"), ("outcome_partial", "Partial"),
        ("outcome_rejected", "Rejected"), ("outcome_unknown", "Unknown"),
        ("model_title", "Declared models"),
        ("model_notice", "These names come from supported trace metadata. They do not verify which model served a request or allocate work, tokens, or outcomes."),
        ("model_legacy", "Model declarations are unavailable in this snapshot. Reimport the source to inspect them."),
        ("model_mixed", "Multiple model names are declared."),
        ("model_not_proven_mixed", "No mixed declarations were observed. Missing metadata may hide other models."),
        ("model_labels", "Declared names"),
        ("model_record_count", "Source lines or array entries"),
        ("model_no_labels", "No valid model names were retained."),
        ("model_candidates", "Supported metadata records"),
        ("model_valid", "Valid declarations"),
        ("model_missing", "Missing declarations"),
        ("model_invalid", "Invalid declarations"),
        ("model_omitted", "References omitted by limits"),
        ("model_labels_omitted", "Additional model names were omitted by limits."),
        ("model_references", "Declaration references"),
        ("model_record_index", "Record index"),
        ("model_coordinates_jsonl_physical_lines_one_based", "JSONL line numbers, starting at 1"),
        ("model_coordinates_trajectory_array_indexes_zero_based", "Trajectory array indexes, starting at 0"),
        ("model_kind_codex_session_metadata", "Session metadata"),
        ("model_kind_codex_turn_context", "Turn context"),
        ("model_kind_codex_assistant_message", "Assistant message metadata"),
        ("model_kind_trajectory_metadata", "Trajectory metadata"),
        ("linked_evidence_title", "Linked outcome evidence"),
        ("link_notice", "You choose these associations. Linking evidence does not verify task success or attribute work to a model."),
        ("link_saved_required", "Save this snapshot before linking evidence."),
        ("link_empty", "No outcome evidence linked."),
        ("link_repository", "Selected repository"),
        ("choose_repository", "Choose repository"),
        ("link_commit", "Full commit object ID"),
        ("link_git", "Inspect and link commit"),
        ("choose_test_report", "Choose test report"),
        ("link_test_report", "Import and link report"),
        ("unlink_evidence", "Remove evidence link"),
        ("link_git_notice", "Inspects an exact local commit. This does not establish merge, acceptance, or revert status."),
        ("link_test_notice", "Imports a structured report as an assertion from its producer. This does not run tests or verify their result."),
        ("test_report_format", "Accepted report: JSON with schema_version 1, runner, passed, failed, skipped, observed_at, and optional commit_id."),
        ("link_changed_selection", "The selected snapshot changed. Choose the evidence again."),
        ("link_success", "Evidence linked to this saved snapshot."),
        ("unlink_success", "Evidence link removed. Original files were preserved."),
        ("link_git_provenance", "Inspected local Git object"),
        ("link_test_provenance", "Imported test report"),
        ("link_user_provenance", "Linked by you"),
        ("git_repository_digest", "Repository path digest"),
        ("git_object", "Commit object"),
        ("git_tree", "Tree object"),
        ("git_no_parents", "No parent objects (root commit)."),
        ("git_parents", "Parent objects"),
        ("test_runner", "Reported runner"),
        ("test_passed", "Reported passed"),
        ("test_failed", "Reported failed"),
        ("test_skipped", "Reported skipped"),
        ("test_observed_at", "Reported test time"),
        ("test_imported_at", "Imported at"),
        ("git_inspected_at", "Inspected at"),
        ("artifact_digest", "Report artifact digest"),
        ("test_claimed_commit", "Claimed commit (not verified)"),
        ("link_recorded_at", "Linked at"),
        ("link_id", "Evidence link ID"),
        ("summary_title", "Saved history summary"),
        ("summary_scope", "Your saved session snapshots"),
        ("summary_empty", "Save an insight to start your history summary."),
        ("summary_unavailable", "The saved history summary could not be refreshed. Try refreshing saved insights."),
        ("summary_snapshots", "Saved snapshots"),
        ("summary_assessed", "Assessed by you"),
        ("summary_unassessed", "Not assessed"),
        ("summary_categories", "Your task categories"),
        ("summary_outcomes", "Your reported outcomes"),
        ("summary_metrics", "Observed totals"),
        ("summary_observed_sum", "Observed total"),
        ("summary_available", "Snapshots with a value"),
        ("summary_missing", "Snapshots without a value"),
        ("summary_record_coverage", "Recognized evidence"),
        ("summary_analysis_range", "Snapshot analysis dates"),
        ("summary_evidence", "Contributing snapshots"),
        ("summary_open_snapshot", "Open snapshot"),
        ("summary_no_evidence", "No snapshots contribute a known value."),
        ("summary_limitations", "How to read this summary"),
        ("summary_unit_session_snapshots", "session snapshots"),
        ("summary_unit_normalized_events", "normalized events"),
        ("summary_unit_tool_results", "tool results"),
        ("summary_limitation_selected_saved_sessions_are_not_verified_tasks", "This summary covers saved sessions. They are not independently verified completed tasks."),
        ("summary_limitation_assessments_are_user_reported", "Categories and outcomes are your assessments. Unknown and not assessed are separate."),
        ("summary_limitation_observed_sums_require_both_coverages", "Totals include available observations. Check both missing snapshots and recognized evidence for incomplete coverage."),
        ("summary_limitation_analysis_dates_are_not_activity_time", "Dates show when snapshots were analyzed, not when the work happened."),
        ("summary_limitation_source_formats_are_not_model_identity", "Source formats identify the imported file format, not which model performed the work."),
        ("summary_limitation_no_model_rankings_time_savings_or_cost", "These observations do not establish model rankings, time saved, or cost."),
    ].into_iter().map(|(key, value)| (key.to_owned(), value.to_owned())).collect()
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocalInsightsRequest {
    #[serde(default)]
    pub store_dir: Option<PathBuf>,
    pub operation: LocalInsightsOperation,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum LocalInsightsOperation {
    Analyze {
        source: SourceFormat,
        file: PathBuf,
        #[serde(default)]
        save: bool,
    },
    List {},
    Copy {},
    Summary {},
    Explain {
        id: String,
    },
    Delete {
        id: String,
    },
    Annotate {
        id: String,
        category: TaskCategory,
        outcome: TaskOutcome,
    },
    ClearAnnotation {
        id: String,
    },
    LinkGit {
        id: String,
        repository: PathBuf,
        commit: String,
    },
    LinkTestReport {
        id: String,
        file: PathBuf,
    },
    UnlinkEvidence {
        id: String,
        evidence_id: String,
    },
    Usage {
        source: UsageSource,
        file: PathBuf,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LocalInsightsResponse {
    Analyze {
        insight: Box<LocalInsight>,
    },
    List {
        insights: Vec<LocalInsight>,
    },
    Summary {
        summary: Box<super::summary::SavedInsightsSummary>,
    },
    Copy {
        copy: std::collections::BTreeMap<String, String>,
    },
    Explain {
        insight: Box<LocalInsight>,
    },
    Delete {
        deleted: bool,
    },
    Annotate {
        insight: Box<LocalInsight>,
    },
    ClearAnnotation {
        insight: Box<LocalInsight>,
    },
    LinkGit {
        insight: Box<LocalInsight>,
    },
    LinkTestReport {
        insight: Box<LocalInsight>,
    },
    UnlinkEvidence {
        insight: Box<LocalInsight>,
    },
    Usage {
        usage: UsageSummary,
    },
}

/// Resolve only local Insights storage; never resolve enrollment configuration.
pub fn open_store(store_dir: Option<&std::path::Path>) -> Result<LocalInsightStore> {
    LocalInsightStore::open(&store_path(store_dir)?)
}

fn store_path(store_dir: Option<&std::path::Path>) -> Result<PathBuf> {
    Ok(match store_dir {
        Some(path) => path.to_path_buf(),
        None => dirs::data_local_dir()
            .ok_or_else(|| anyhow!("insights-local-directory-unavailable"))?
            .join("trace-commons")
            .join("insights"),
    })
}

/// Reading an empty history must not create state before an explicit save.
pub fn list_saved(store_dir: Option<&std::path::Path>) -> Result<Vec<LocalInsight>> {
    let path = store_path(store_dir)?;
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(_) => bail!("insights-local-directory-unavailable"),
        Ok(_) => LocalInsightStore::open(&path)?.list(),
    }
}

/// Synchronous local IO. Native callers must schedule this off the UI thread.
/// Dropping a UI task does not cancel a save/delete that has already started.
pub fn execute(request: LocalInsightsRequest) -> Result<LocalInsightsResponse> {
    let store = || open_store(request.store_dir.as_deref());
    Ok(match request.operation {
        LocalInsightsOperation::Analyze { source, file, save } => {
            let insight = if save {
                store()?.import(source, &file)?
            } else {
                analyze_file(source, &file)?
            };
            LocalInsightsResponse::Analyze {
                insight: Box::new(insight),
            }
        }
        LocalInsightsOperation::List {} => LocalInsightsResponse::List {
            insights: list_saved(request.store_dir.as_deref())?,
        },
        LocalInsightsOperation::Summary {} => LocalInsightsResponse::Summary {
            summary: Box::new(super::summary::read_saved(request.store_dir.as_deref())?),
        },
        LocalInsightsOperation::Copy {} => LocalInsightsResponse::Copy { copy: ui_copy() },
        LocalInsightsOperation::Explain { id } => LocalInsightsResponse::Explain {
            insight: Box::new(store()?.explain(&id)?),
        },
        LocalInsightsOperation::Delete { id } => LocalInsightsResponse::Delete {
            deleted: store()?.delete(&id)?,
        },
        LocalInsightsOperation::Annotate {
            id,
            category,
            outcome,
        } => LocalInsightsResponse::Annotate {
            insight: Box::new(store()?.annotate(&id, category, outcome)?),
        },
        LocalInsightsOperation::ClearAnnotation { id } => LocalInsightsResponse::ClearAnnotation {
            insight: Box::new(store()?.clear_annotation(&id)?),
        },
        LocalInsightsOperation::LinkGit {
            id,
            repository,
            commit,
        } => {
            let store = store()?;
            store.explain(&id)?;
            let evidence = super::outcomes::inspect_git_commit(&repository, &commit)?;
            LocalInsightsResponse::LinkGit {
                insight: Box::new(
                    store
                        .link_outcome(&id, super::outcomes::OutcomeEvidence::GitCommit(evidence))?,
                ),
            }
        }
        LocalInsightsOperation::LinkTestReport { id, file } => {
            let store = store()?;
            store.explain(&id)?;
            let evidence = super::outcomes::import_test_report(&file)?;
            LocalInsightsResponse::LinkTestReport {
                insight: Box::new(
                    store.link_outcome(
                        &id,
                        super::outcomes::OutcomeEvidence::TestReport(evidence),
                    )?,
                ),
            }
        }
        LocalInsightsOperation::UnlinkEvidence { id, evidence_id } => {
            LocalInsightsResponse::UnlinkEvidence {
                insight: Box::new(store()?.unlink_outcome(&id, &evidence_id)?),
            }
        }
        LocalInsightsOperation::Usage { source, file } => LocalInsightsResponse::Usage {
            usage: extract_usage(source, &super::bounded_read(&file)?)?,
        },
    })
}

/// Strict typed JSON boundary. Errors deliberately contain fixed labels only.
pub fn dispatch_json(bytes: &[u8]) -> Result<String> {
    if bytes.len() > MAX_REQUEST_BYTES {
        bail!("insights-request-too-large");
    }
    let text = std::str::from_utf8(bytes).map_err(|_| anyhow!("insights-request-invalid-utf8"))?;
    let request = serde_json::from_str(text).map_err(|_| anyhow!("insights-request-invalid"))?;
    let response = execute(request).map_err(|_| anyhow!("insights-operation-failed"))?;
    serde_json::to_string(&response).map_err(|_| anyhow!("insights-response-invalid"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_untrusted_request_without_echoing_content() {
        for request in [
            br#"{"operation":{"type":"list","secret":"private"}}"#.as_slice(),
            br#"{"operation":{"type":"copy","secret":"private"}}"#,
            br#"{"operation":{"type":"upload"}}"#,
            br#"{"operation":{"type":"summary","secret":"private"}}"#,
            br#"{"operation":{"type":"list"},"secret":"private"}"#,
        ] {
            assert_eq!(
                dispatch_json(request).unwrap_err().to_string(),
                "insights-request-invalid"
            );
        }
        assert_eq!(
            dispatch_json(&[0xff]).unwrap_err().to_string(),
            "insights-request-invalid-utf8"
        );
        assert_eq!(
            dispatch_json(&vec![b' '; MAX_REQUEST_BYTES + 1])
                .unwrap_err()
                .to_string(),
            "insights-request-too-large"
        );
    }

    #[test]
    fn selected_file_lifecycle_keeps_unsaved_analysis_ephemeral() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("session.jsonl");
        let store = temp.path().join("insights");
        std::fs::write(
            &file,
            concat!(
                "{\"role\":\"meta\",\"source\":\"claude-code\",\"model\":\"fixture\"}\n",
                "{\"role\":\"user\",\"timestamp\":\"2026-09-11T12:00:00Z\",\"content\":\"PRIVATE_BODY\"}\n"
            ),
        )
        .unwrap();
        let call = |operation| {
            execute(LocalInsightsRequest {
                store_dir: Some(store.clone()),
                operation,
            })
            .unwrap()
        };
        let LocalInsightsResponse::Analyze { insight } = call(LocalInsightsOperation::Analyze {
            source: SourceFormat::Trajectory,
            file: file.clone(),
            save: false,
        }) else {
            panic!("expected analysis")
        };
        assert!(!store.exists());
        assert!(
            !serde_json::to_string(&insight)
                .unwrap()
                .contains("PRIVATE_BODY")
        );
        call(LocalInsightsOperation::Analyze {
            source: SourceFormat::Trajectory,
            file: file.clone(),
            save: true,
        });
        let LocalInsightsResponse::Explain { insight: saved } =
            call(LocalInsightsOperation::Explain {
                id: insight.id.clone(),
            })
        else {
            panic!("expected explanation")
        };
        assert_eq!(saved.id, insight.id);
        let LocalInsightsResponse::Annotate { insight: annotated } =
            call(LocalInsightsOperation::Annotate {
                id: saved.id.clone(),
                category: TaskCategory::Docs,
                outcome: TaskOutcome::Partial,
            })
        else {
            panic!("expected annotated snapshot")
        };
        assert_eq!(
            annotated.manual_annotation.unwrap().outcome,
            TaskOutcome::Partial
        );
        let LocalInsightsResponse::ClearAnnotation { insight: cleared } =
            call(LocalInsightsOperation::ClearAnnotation {
                id: saved.id.clone(),
            })
        else {
            panic!("expected cleared snapshot")
        };
        assert!(cleared.manual_annotation.is_none());
        assert!(matches!(
            call(LocalInsightsOperation::Delete { id: insight.id }),
            LocalInsightsResponse::Delete { deleted: true }
        ));
        assert!(
            matches!(call(LocalInsightsOperation::List {}), LocalInsightsResponse::List { insights } if insights.is_empty())
        );
        assert!(file.exists());
    }

    #[test]
    fn native_usage_json_is_ephemeral_and_redacts_source_content() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("native.jsonl");
        let store = temp.path().join("unused-store");
        std::fs::write(
            &file,
            serde_json::json!({
                "type":"assistant", "message":{
                    "id":"message-1", "model":"fixture", "content":"PRIVATE_BODY",
                    "usage":{"input_tokens":12,"cache_read_input_tokens":3,
                        "cache_creation_input_tokens":4,"output_tokens":5}
                }
            })
            .to_string(),
        )
        .unwrap();
        let request = serde_json::json!({"store_dir":store,"operation":{
            "type":"usage","source":"claude_code","file":file
        }});
        let response = dispatch_json(&serde_json::to_vec(&request).unwrap()).unwrap();
        assert!(!response.contains("PRIVATE_BODY"));
        assert!(!response.contains("native.jsonl"));
        let value: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(value["type"], "usage");
        assert_eq!(value["usage"]["complete_records"], 1);
        assert!(value["usage"]["counts"].is_object());
        assert!(!store.exists());
    }

    #[test]
    fn desktop_copy_is_available_without_resolving_storage() {
        let temp = tempfile::tempdir().unwrap();
        let request = serde_json::json!({"store_dir": temp.path().join("not-created"),
            "operation":{"type":"copy"}});
        let response = dispatch_json(&serde_json::to_vec(&request).unwrap()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(value["type"], "copy");
        assert_eq!(value["copy"]["title"], "Insights");
        assert_eq!(value["copy"]["save"], "Re-read and save");
        assert!(
            value["copy"]["assessment_notice"]
                .as_str()
                .unwrap()
                .contains("user-reported")
        );
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn read_only_history_still_refuses_a_symlink_store() {
        let temp = tempfile::tempdir().unwrap();
        let link = temp.path().join("store");
        std::os::unix::fs::symlink(temp.path().join("absent-target"), &link).unwrap();
        assert!(list_saved(Some(&link)).is_err());
        assert!(!temp.path().join("absent-target").exists());
    }

    #[test]
    fn list_needs_only_an_explicit_local_directory() {
        let temp = tempfile::tempdir().unwrap();
        let request = LocalInsightsRequest {
            store_dir: Some(temp.path().join("insights")),
            operation: LocalInsightsOperation::List {},
        };
        let response = dispatch_json(&serde_json::to_vec(&request).unwrap()).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&response).unwrap(),
            serde_json::json!({"type":"list","insights":[]})
        );
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
    }
}
