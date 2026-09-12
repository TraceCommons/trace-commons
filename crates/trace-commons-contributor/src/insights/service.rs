//! Account-free local Insights entry point shared by native shells and CLI.
use std::{io::Write, path::PathBuf};

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use trace_commons_protocol::insights_cards::{InsightCardResult, InsightQuestionId};

use super::comparison_specs::{
    ComparisonSpecificationDraftInput, ComparisonSpecificationError, ComparisonSpecificationV1,
    DescriptiveComparisonResultV1,
};
use super::comparison_task_store::ComparisonTaskStoreError;
use super::comparison_tasks::{
    ComparisonTaskContextInput, ComparisonTaskDetail, ComparisonTaskOutcome,
    ComparisonTaskValidationError, LocalComparisonTaskV1,
};
use super::episode_store::EpisodeStoreError;
use super::episodes::{EpisodeDetail, EpisodeListEntry, EpisodeValidationError, LocalEpisode};
use super::usage::{UsageSource, UsageSummary, extract_usage};
use super::{
    LocalInsight, LocalInsightStore, MutationEffects, SourceFormat, TaskCategory, TaskOutcome,
    analyze_file,
};

/// Bound request bytes before parsing or reading caller-owned FFI memory.
pub const MAX_REQUEST_BYTES: usize = 64 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug)]
pub struct ResponseTooLarge;
impl std::fmt::Display for ResponseTooLarge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("insights_response_too_large")
    }
}
impl std::error::Error for ResponseTooLarge {}

/// Shared desktop vocabulary; shells render observations without inventing claims.
pub fn ui_copy() -> std::collections::BTreeMap<String, String> {
    [
        ("title", "Insights"),
        ("episode_title", "Episodes"),
        ("episode_scope", "Episodes group whole saved snapshots you select. They do not establish task boundaries or independent tasks."),
        ("episode_assessment_notice", "An episode assessment is your independent report. Member assessments, model declarations, and linked artifacts do not verify its outcome."),
        ("episode_overlap_notice", "Episodes may reuse the same snapshots. Inspect overlap before interpreting these groups."),
        ("episode_invalidated_notice", "These episode groups and their assessments were removed because a member snapshot was removed or replaced. The grouping is lost; surviving snapshots and original files remain."),
        ("episode_deleted", "Removed the episode and its assessment. Saved snapshots and original files remain."),
        ("episode_revision_conflict", "This episode changed in another window or client. Refresh and review it before editing again."),
        ("episode_membership_changed", "Changing episode members clears its previous assessment."),
        ("episode_empty", "No saved episodes. Select whole saved snapshots to create a group."),
        ("episode_saved", "Saved episodes"),
        ("episode_count", "Saved episode groups"),
        ("episode_create", "Create episode"),
        ("episode_create_notice", "Select one or more whole saved snapshots. A snapshot can belong to more than one episode."),
        ("episode_select_members", "Select saved snapshots"),
        ("episode_selected_members", "Selected snapshots"),
        ("episode_member_limit", "Select between 1 and 64 saved snapshots."),
        ("episode_limit", "This store already has 256 episodes. Delete a group before creating another."),
        ("episode_missing_members", "One or more selected snapshots are no longer saved. Refresh the selection."),
        ("episode_invalid", "The episode data or selection could not be read. Refresh before trying again."),
        ("episode_response_too_large", "This episode has too much evidence to display at once. Inspect its snapshots separately."),
        ("episode_selection_empty", "Select at least one saved snapshot to create an episode."),
        ("episode_create_success", "Episode created from the selected saved snapshots."),
        ("episode_open", "Inspect episode"),
        ("episode_id", "Episode ID"),
        ("episode_created_at", "Created at"),
        ("episode_updated_at", "Episode edited at"),
        ("episode_revision", "Revision"),
        ("episode_membership_revision", "Membership revision"),
        ("episode_members", "Whole saved snapshot members"),
        ("episode_member_id", "Saved snapshot ID"),
        ("episode_edit_members", "Edit members"),
        ("episode_edit_members_notice", "Review the complete selection before saving. Removing or adding a member clears the episode assessment."),
        ("episode_save_members", "Save members"),
        ("episode_members_saved", "Episode members saved."),
        ("episode_assessment", "Independent user-reported episode assessment"),
        ("episode_assessment_notice_short", "Assess this episode independently from its member snapshots and linked evidence."),
        ("episode_save_assessment", "Save episode assessment"),
        ("episode_assessment_saved", "Episode assessment saved."),
        ("episode_unassessed", "Unassessed"),
        ("episode_clear_assessment", "Clear episode assessment"),
        ("episode_clear_assessment_confirm", "Clear this episode assessment? The episode and its members will remain."),
        ("episode_assessment_cleared", "Episode assessment cleared."),
        ("episode_delete", "Delete episode"),
        ("episode_delete_confirm", "Delete this episode and its assessment? Saved snapshots and original files will remain."),
        ("episode_overlaps", "Overlapping episode IDs"),
        ("episode_no_overlap", "No overlapping episodes"),
        ("episode_member_evidence", "Current saved member evidence"),
        ("episode_resolved", "Evidence resolved at"),
        ("episode_missing", "This episode is no longer available. Refresh saved episodes."),
        ("comparison_task_title", "Comparison task"),
        ("comparison_specification_title", "Comparison specification"),
        ("comparison_specifications_empty", "No comparison specifications have been saved."),
        ("comparison_specification_refresh", "Refresh saved specifications"),
        ("comparison_specification_draft", "Draft a retrospective comparison"),
        ("comparison_specification_need_context", "Add complete context to saved comparison tasks before drafting a specification."),
        ("comparison_specification_stratum", "Exact project, language, and configuration"),
        ("comparison_specification_matching_tasks", "matching tasks"),
        ("comparison_specification_candidate_declarations", "Candidate cohort declarations"),
        ("comparison_specification_need_cohorts", "At least two declared model labels are needed across matching task evidence."),
        ("comparison_specification_date_start", "Task window starts"),
        ("comparison_specification_date_end", "Task window ends"),
        ("comparison_specification_cutoff", "Evidence cutoff"),
        ("comparison_specification_cutoff_notice", "The cutoff freezes the task material and outcomes available at that time. Later evidence is excluded."),
        ("comparison_specification_preview", "Preview comparison"),
        ("comparison_specification_save", "Save immutable specification"),
        ("comparison_specification_saved", "Saved comparison specifications"),
        ("comparison_specification_saved_notice", "Comparison specification saved."),
        ("comparison_specification_evaluate", "Evaluate saved specification"),
        ("comparison_specification_explain_result", "Verify this result"),
        ("comparison_specification_explain_confirm", "Verify this exact audit digest against current local evidence?"),
        ("comparison_specification_result", "Categorical outcome counts"),
        ("comparison_specification_included", "Included tasks"),
        ("comparison_specification_assessed", "Assessed outcomes"),
        ("comparison_specification_exclusions", "Excluded tasks"),
        ("comparison_specification_cutoff_evidence", "Evidence captured at the cutoff"),
        ("comparison_specification_usage_observed", "Tasks with observed attributed token counts"),
        ("comparison_specification_usage_unavailable", "Tasks without observed attributed token counts"),
        ("comparison_specification_observed_tokens", "Observed attributed tokens (partial coverage)"),
        ("comparison_specification_error", "The comparison specification could not be loaded or evaluated. Review the selected evidence and try again."),
        ("insights_store_title", "Insights store"),
        ("insights_store_unavailable", "Insights store unavailable"),
        ("insights_store_duplicate", "Choose one --insights-store directory and relaunch."),
        ("insights_store_missing_path", "--insights-store requires an absolute directory path."),
        ("insights_store_relative_path", "The Insights store path must be absolute."),
        ("insights_store_path_missing", "The selected Insights store directory does not exist."),
        ("insights_store_not_directory", "The selected Insights store path is not a directory."),
        ("comparison_specification_committed_reload_failed", "The specification was saved, but refreshed details could not be loaded. Refresh before continuing."),
        ("comparison_preview_notice", "Preview only. This specification has not been saved."),
        ("comparison_retrospective_notice", "Retrospective user specification based on existing evidence."),
        ("comparison_descriptive_notice", "Descriptive outcomes only. Uncertainty is not yet calibrated; no model advantage is established."),
        ("comparison_exact_notice", "Simultaneous 95% intervals cover the three assessed-outcome frequency differences, conditional on assessed outcomes. They do not measure task completion or general quality."),
        ("comparison_exact_orientation", "Difference orientation: second cohort minus first cohort."),
        ("comparison_exact_support_unavailable", "Interval unavailable: each cohort needs at least 2 assessed tasks."),
        ("comparison_exact_insufficient_precision", "Interval too wide for a directional conclusion."),
        ("comparison_exact_indeterminate_boundary", "Boundary case; directional conclusion withheld."),
        ("comparison_exact_includes_zero", "Interval includes zero; a difference is unresolved."),
        ("comparison_exact_excludes_zero", "Interval excludes zero."),
        ("comparison_exact_positive_direction", "A positive difference means a higher frequency of this outcome in the second cohort; it does not uniformly mean better."),
        ("comparison_exact_minus", "minus"),
        ("comparison_exact_versus", "versus"),
        ("comparison_exact_observed_difference", "Observed difference"),
        ("comparison_exact_interval", "Simultaneous interval"),
        ("comparison_exact_percentage_points", "percentage points"),
        ("comparison_no_eligible_evidence", "No eligible evidence matches this comparison. Review the exclusions below."),
        ("comparison_denominator_notice", "Assessed outcomes include accepted, partial, and rejected tasks. Pending, unknown, and unassessed outcomes are excluded from that denominator. Observed tokens may cover only part of a task."),
        ("comparison_exclusion_evidence_after_cutoff", "Evidence changed or was recorded after the cutoff"),
        ("comparison_exclusion_cutoff_time_unavailable", "Evidence recording time is unavailable"),
        ("comparison_exclusion_category_mismatch", "Task category does not match"),
        ("comparison_exclusion_date_outside_window", "Task date is outside the selected window"),
        ("comparison_exclusion_stratum_mismatch", "Project, language, or configuration does not match"),
        ("comparison_exclusion_context_unavailable", "Task context is incomplete"),
        ("comparison_exclusion_cohort_unavailable", "Declared model cohort is unavailable"),
        ("comparison_exclusion_cohort_not_selected", "Declared model cohort is not selected"),
        ("comparison_exclusion_evidence_stale", "Bound evidence is stale"),
        ("comparison_exclusion_independence_unconfirmed", "Task independence needs review"),
        ("comparison_exclusion_overlapping_task_evidence", "Evidence overlaps another task"),
        ("comparison_exclusion_source_attribution_unavailable", "Source attribution is not qualified"),
        ("comparison_exclusion_source_attribution_stale", "Source attribution no longer matches the evidence"),
        ("comparison_task_empty", "No comparison tasks have been saved."),
        ("comparison_task_material_digest", "Material evidence digest"),
        ("comparison_task_context_complete", "Comparable context is complete."),
        ("comparison_task_context_incomplete", "Comparable context is incomplete."),
        ("comparison_task_outcome_unassessed", "Outcome is unassessed."),
        ("comparison_task_confirmation_current", "Independence review is current."),
        ("comparison_task_confirmation_missing", "Independence review is missing or stale."),
        ("comparison_task_attribution_pending", "Source attribution is pending qualification; this task is not comparison-eligible."),
        ("comparison_task_create", "Create comparison task"),
        ("comparison_task_list", "Saved comparison tasks"),
        ("comparison_task_open", "Review comparison task"),
        ("comparison_task_back", "Back to comparison tasks"),
        ("comparison_task_delete", "Delete comparison task"),
        ("comparison_task_delete_confirm", "Delete this comparison task? Its episodes and snapshots will remain."),
        ("comparison_task_deleted", "Comparison task deleted."),
        ("comparison_task_replace_episodes", "Replace episode evidence"),
        ("comparison_task_set_context", "Save task context"),
        ("comparison_task_set_outcome", "Save user-reported outcome"),
        ("comparison_task_clear_outcome", "Clear outcome"),
        ("comparison_task_reconfirm", "Confirm one work item and all known attempts"),
        ("comparison_task_reconfirm_notice", "Confirm only after reviewing the displayed material evidence digest and all frozen attempts."),
        ("comparison_task_revision_conflict", "This task changed in another window. Refresh and review it before editing."),
        ("comparison_task_digest_conflict", "The task evidence changed after it was displayed. Refresh and review the new digest."),
        ("comparison_task_stale_evidence", "The bound episode or snapshot evidence changed. Replace or review it before confirming."),
        ("comparison_task_committed_reload_failed", "The change was saved, but refreshed task details could not be loaded. Refresh before editing again."),
        ("comparison_task_project_id", "Project ID"),
        ("comparison_task_category", "Category"),
        ("comparison_task_task_date", "Task date"),
        ("comparison_task_language", "Language"),
        ("comparison_task_harness_id", "Harness ID"),
        ("comparison_task_harness_version", "Harness version"),
        ("comparison_task_reasoning_effort", "Reasoning effort"),
        ("comparison_task_tool_policy_id", "Tool-policy profile ID"),
        ("comparison_task_tool_policy_version", "Tool-policy profile version"),
        ("comparison_task_prompt_template_digest", "Prompt-template digest"),
        ("comparison_task_configuration_fingerprint", "Configuration fingerprint"),
        ("comparison_task_unknown", "Unknown"),
        ("comparison_task_stale_reasons", "Review reasons"),
        ("comparison_task_stale_none", "No stale evidence was detected."),
        ("comparison_task_stale_episode_missing", "A frozen episode is no longer available."),
        ("comparison_task_stale_episode_revision_changed", "An episode changed after this task froze its evidence."),
        ("comparison_task_stale_episode_membership_changed", "An episode's membership changed after this task froze its evidence."),
        ("comparison_task_stale_snapshot_missing_or_replaced", "A frozen snapshot is missing or was replaced."),
        ("comparison_task_stale_outcome_material_changed", "The outcome was recorded against earlier task material."),
        ("comparison_task_stale_independence_material_changed", "The independence review was recorded against earlier task material."),
        ("comparison_task_stale_context_incomplete", "Comparable context is incomplete."),
        ("comparison_task_stale_attribution_pending_qualification", "Source attribution is pending qualification."),
        ("comparison_task_stale_overlapping_task_evidence", "This evidence is also frozen into another task."),
        ("comparison_task_revision", "Task revision"),
        ("comparison_task_resolved", "Evidence checked at"),
        ("comparison_task_frozen_evidence", "Frozen episode evidence"),
        ("comparison_task_advanced_evidence", "Identifiers and digests"),
        ("comparison_task_project", "Project"),
        ("comparison_task_new_project", "New local project"),
        ("comparison_task_current_missing", "Current episode is unavailable; frozen evidence remains inspectable."),
        ("comparison_task_current_matches_frozen", "Current episode matches the frozen revision and membership."),
        ("comparison_task_current_changed", "Current episode differs from the frozen revision or membership."),
        ("comparison_task_open_current_episode", "Inspect current episode"),
        ("comparison_task_open_frozen_snapshot", "Inspect frozen snapshot evidence"),
        ("comparison_task_reasoning_unknown", "Unknown"),
        ("comparison_task_reasoning_none", "None"),
        ("comparison_task_reasoning_minimal", "Minimal"),
        ("comparison_task_reasoning_low", "Low"),
        ("comparison_task_reasoning_medium", "Medium"),
        ("comparison_task_reasoning_high", "High"),
        ("comparison_task_reasoning_xhigh", "Extra high"),
        ("comparison_task_overlap", "Overlapping comparison tasks"),
        ("episode_list_unavailable", "Saved episodes could not be refreshed. Try again."),
        ("episode_detail_unavailable", "This episode could not be refreshed. Return to saved episodes and try again."),
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
        ("boundary_notice", "One selected file. It may be a session fragment or a cumulative transcript; human authorship, complete conversation membership, and task boundaries have not been verified."),
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
        ("codex", "Codex rollout"), ("claude_code", "Claude Code session"), ("trajectory", "Trajectory"),
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
        ("model_kind_claude_assistant_message", "Claude assistant message metadata"),
        ("claude_attribution_title", "Claude agent-branch evidence"),
        ("claude_attribution_notice", "Observed branch structure only; this does not establish a whole task, human prompt, outcome, independence, or background-process completion."),
        ("claude_attribution_attributed", "Observed branch segment ended with supported structure"),
        ("claude_attribution_unavailable", "Task attribution unavailable for this selected file"),
        ("claude_attribution_terminal_line", "Terminal physical line"),
        ("claude_attribution_tool_pairs", "Paired tool protocol records"),
        ("claude_attribution_background", "Observed background launches with completion unavailable"),
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
    ].into_iter().map(|(key, value)| (key.to_owned(), value.to_owned()))
    .chain(super::card_presentation::ui_copy()).collect()
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
    QuestionCards {
        questions: Vec<InsightQuestionId>,
        snapshot_ids: Vec<String>,
        episode_ids: Vec<String>,
    },
    EpisodeCreate {
        snapshot_ids: Vec<String>,
    },
    EpisodeList {},
    EpisodeExplain {
        id: String,
    },
    EpisodeReplaceMembers {
        id: String,
        expected_revision: u64,
        snapshot_ids: Vec<String>,
    },
    EpisodeAnnotate {
        id: String,
        expected_revision: u64,
        category: TaskCategory,
        outcome: TaskOutcome,
    },
    EpisodeClearAssessment {
        id: String,
        expected_revision: u64,
    },
    EpisodeDelete {
        id: String,
        expected_revision: u64,
    },
    ComparisonTaskCreate {
        episode_ids: Vec<String>,
    },
    ComparisonTaskList {},
    ComparisonTaskExplain {
        id: String,
    },
    ComparisonTaskReplaceEpisodes {
        id: String,
        expected_revision: u64,
        episode_ids: Vec<String>,
    },
    ComparisonTaskSetContext {
        id: String,
        expected_revision: u64,
        context: ComparisonTaskContextInput,
    },
    ComparisonTaskSetOutcome {
        id: String,
        expected_revision: u64,
        outcome: ComparisonTaskOutcome,
    },
    ComparisonTaskClearOutcome {
        id: String,
        expected_revision: u64,
    },
    ComparisonTaskReconfirm {
        id: String,
        expected_revision: u64,
        displayed_material_digest: String,
    },
    ComparisonTaskDelete {
        id: String,
        expected_revision: u64,
    },
    ComparisonPreviewSpec {
        input: ComparisonSpecificationDraftInput,
    },
    ComparisonSaveSpec {
        input: ComparisonSpecificationDraftInput,
    },
    ComparisonListSpecs {},
    ComparisonGetSpec {
        id: String,
    },
    ComparisonEvaluate {
        id: String,
    },
    ComparisonExplainResult {
        specification_id: String,
        audit_digest: String,
    },
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
    QuestionCards {
        result: Box<InsightCardResult>,
        text: String,
    },
    EpisodeCreate {
        episode: Box<LocalEpisode>,
    },
    EpisodeList {
        episodes: Vec<EpisodeListEntry>,
    },
    EpisodeExplain {
        detail: Box<EpisodeDetail>,
    },
    EpisodeReplaceMembers {
        episode: Box<LocalEpisode>,
        #[serde(default)]
        mutation_effects: MutationEffects,
    },
    EpisodeAnnotate {
        episode: Box<LocalEpisode>,
        #[serde(default)]
        mutation_effects: MutationEffects,
    },
    EpisodeClearAssessment {
        episode: Box<LocalEpisode>,
        #[serde(default)]
        mutation_effects: MutationEffects,
    },
    EpisodeDelete {
        episode: Box<LocalEpisode>,
        #[serde(default)]
        mutation_effects: MutationEffects,
    },
    ComparisonTask {
        task: Box<LocalComparisonTaskV1>,
        #[serde(default)]
        mutation_effects: MutationEffects,
    },
    ComparisonTaskList {
        tasks: Vec<ComparisonTaskDetail>,
    },
    ComparisonTaskExplain {
        detail: Box<ComparisonTaskDetail>,
    },
    ComparisonTaskDelete {
        task: Box<LocalComparisonTaskV1>,
        #[serde(default)]
        mutation_effects: MutationEffects,
    },
    ComparisonPreviewSpec {
        specification: Box<ComparisonSpecificationV1>,
        result: Box<DescriptiveComparisonResultV1>,
    },
    ComparisonSpecification {
        specification: Box<ComparisonSpecificationV1>,
    },
    ComparisonSpecificationList {
        specifications: Vec<ComparisonSpecificationV1>,
    },
    ComparisonResult {
        result: Box<DescriptiveComparisonResultV1>,
    },
    Analyze {
        insight: Box<LocalInsight>,
        #[serde(default)]
        mutation_effects: MutationEffects,
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
        #[serde(default)]
        mutation_effects: MutationEffects,
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
    match existing_store(store_dir)? {
        Some(store) => store.list(),
        None => Ok(Vec::new()),
    }
}

fn existing_store(store_dir: Option<&std::path::Path>) -> Result<Option<LocalInsightStore>> {
    let path = store_path(store_dir)?;
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => bail!("insights-local-directory-unavailable"),
        Ok(_) => LocalInsightStore::open(&path).map(Some),
    }
}

/// Synchronous local IO. Native callers must schedule this off the UI thread.
/// Dropping a UI task does not cancel a save/delete that has already started.
pub fn execute(request: LocalInsightsRequest) -> Result<LocalInsightsResponse> {
    let response = execute_inner(request)?;
    response_json(&response)?;
    Ok(response)
}

fn execute_inner(request: LocalInsightsRequest) -> Result<LocalInsightsResponse> {
    let store = || open_store(request.store_dir.as_deref());
    let episode_store = || {
        existing_store(request.store_dir.as_deref())?
            .ok_or_else(|| anyhow!(EpisodeStoreError::NotFound))
    };
    let comparison_store = || {
        existing_store(request.store_dir.as_deref())?
            .ok_or_else(|| anyhow!(ComparisonTaskStoreError::NotFound))
    };
    let specification_store = || {
        existing_store(request.store_dir.as_deref())?
            .ok_or_else(|| anyhow!(ComparisonSpecificationError::NotFound))
    };
    Ok(match request.operation {
        LocalInsightsOperation::QuestionCards {
            questions,
            snapshot_ids,
            episode_ids,
        } => {
            use super::card_store::{CardStoreError, empty_card_request, validate_card_selection};
            validate_card_selection(&questions, &snapshot_ids, &episode_ids)?;
            let input = match existing_store(request.store_dir.as_deref())? {
                Some(store) => {
                    store.resolve_card_request(&questions, &snapshot_ids, &episode_ids)?
                }
                None if !snapshot_ids.is_empty() => {
                    return Err(CardStoreError::MissingSnapshot.into());
                }
                None if !episode_ids.is_empty() => {
                    return Err(CardStoreError::MissingEpisode.into());
                }
                None => empty_card_request(&questions)?,
            };
            let result = super::cards::project_first_party_question_cards(&input)?;
            let text = super::card_presentation::render_text(&result, &input)?;
            LocalInsightsResponse::QuestionCards {
                result: Box::new(result),
                text,
            }
        }
        LocalInsightsOperation::EpisodeCreate { snapshot_ids } => {
            super::episodes::validate_snapshot_ids(&snapshot_ids)?;
            let store = existing_store(request.store_dir.as_deref())?
                .ok_or_else(|| anyhow!(EpisodeStoreError::MissingMembers))?;
            LocalInsightsResponse::EpisodeCreate {
                episode: Box::new(store.episode_create(&snapshot_ids)?),
            }
        }
        LocalInsightsOperation::EpisodeList {} => LocalInsightsResponse::EpisodeList {
            episodes: match existing_store(request.store_dir.as_deref())? {
                Some(store) => store.episode_list()?,
                None => Vec::new(),
            },
        },
        LocalInsightsOperation::EpisodeExplain { id } => {
            super::episodes::validate_episode_id(&id)?;
            LocalInsightsResponse::EpisodeExplain {
                detail: Box::new(episode_store()?.episode_explain(&id)?),
            }
        }
        LocalInsightsOperation::EpisodeReplaceMembers {
            id,
            expected_revision,
            snapshot_ids,
        } => {
            super::episodes::validate_episode_id(&id)?;
            super::episodes::validate_snapshot_ids(&snapshot_ids)?;
            let result = episode_store()?.episode_replace_members_with_effects(
                &id,
                expected_revision,
                &snapshot_ids,
            )?;
            LocalInsightsResponse::EpisodeReplaceMembers {
                episode: Box::new(result.value),
                mutation_effects: result.mutation_effects,
            }
        }
        LocalInsightsOperation::EpisodeAnnotate {
            id,
            expected_revision,
            category,
            outcome,
        } => {
            super::episodes::validate_episode_id(&id)?;
            let result = episode_store()?.episode_annotate_with_effects(
                &id,
                expected_revision,
                category,
                outcome,
            )?;
            LocalInsightsResponse::EpisodeAnnotate {
                episode: Box::new(result.value),
                mutation_effects: result.mutation_effects,
            }
        }
        LocalInsightsOperation::EpisodeClearAssessment {
            id,
            expected_revision,
        } => {
            super::episodes::validate_episode_id(&id)?;
            let result =
                episode_store()?.episode_clear_assessment_with_effects(&id, expected_revision)?;
            LocalInsightsResponse::EpisodeClearAssessment {
                episode: Box::new(result.value),
                mutation_effects: result.mutation_effects,
            }
        }
        LocalInsightsOperation::EpisodeDelete {
            id,
            expected_revision,
        } => {
            super::episodes::validate_episode_id(&id)?;
            let result = episode_store()?.episode_delete_with_effects(&id, expected_revision)?;
            LocalInsightsResponse::EpisodeDelete {
                episode: Box::new(result.value),
                mutation_effects: result.mutation_effects,
            }
        }
        LocalInsightsOperation::ComparisonTaskCreate { episode_ids } => {
            let result = existing_store(request.store_dir.as_deref())?
                .ok_or(ComparisonTaskStoreError::MissingEpisode)?
                .comparison_task_create_with_effects(&episode_ids)?;
            LocalInsightsResponse::ComparisonTask {
                task: Box::new(result.value),
                mutation_effects: result.mutation_effects,
            }
        }
        LocalInsightsOperation::ComparisonTaskList {} => {
            LocalInsightsResponse::ComparisonTaskList {
                tasks: match existing_store(request.store_dir.as_deref())? {
                    Some(store) => store.comparison_task_list()?,
                    None => Vec::new(),
                },
            }
        }
        LocalInsightsOperation::ComparisonTaskExplain { id } => {
            super::comparison_tasks::validate_uuid(&id)?;
            LocalInsightsResponse::ComparisonTaskExplain {
                detail: Box::new(comparison_store()?.comparison_task_explain(&id)?),
            }
        }
        LocalInsightsOperation::ComparisonTaskReplaceEpisodes {
            id,
            expected_revision,
            episode_ids,
        } => {
            let result = comparison_store()?.comparison_task_replace_episodes_with_effects(
                &id,
                expected_revision,
                &episode_ids,
            )?;
            LocalInsightsResponse::ComparisonTask {
                task: Box::new(result.value),
                mutation_effects: result.mutation_effects,
            }
        }
        LocalInsightsOperation::ComparisonTaskSetContext {
            id,
            expected_revision,
            context,
        } => LocalInsightsResponse::ComparisonTask {
            task: Box::new(comparison_store()?.comparison_task_set_context(
                &id,
                expected_revision,
                context,
            )?),
            mutation_effects: MutationEffects::default(),
        },
        LocalInsightsOperation::ComparisonTaskSetOutcome {
            id,
            expected_revision,
            outcome,
        } => LocalInsightsResponse::ComparisonTask {
            task: Box::new(comparison_store()?.comparison_task_set_outcome(
                &id,
                expected_revision,
                outcome,
            )?),
            mutation_effects: MutationEffects::default(),
        },
        LocalInsightsOperation::ComparisonTaskClearOutcome {
            id,
            expected_revision,
        } => LocalInsightsResponse::ComparisonTask {
            task: Box::new(
                comparison_store()?.comparison_task_clear_outcome(&id, expected_revision)?,
            ),
            mutation_effects: MutationEffects::default(),
        },
        LocalInsightsOperation::ComparisonTaskReconfirm {
            id,
            expected_revision,
            displayed_material_digest,
        } => LocalInsightsResponse::ComparisonTask {
            task: Box::new(comparison_store()?.comparison_task_reconfirm(
                &id,
                expected_revision,
                &displayed_material_digest,
            )?),
            mutation_effects: MutationEffects::default(),
        },
        LocalInsightsOperation::ComparisonTaskDelete {
            id,
            expected_revision,
        } => {
            let result =
                comparison_store()?.comparison_task_delete_with_effects(&id, expected_revision)?;
            LocalInsightsResponse::ComparisonTaskDelete {
                task: Box::new(result.value),
                mutation_effects: result.mutation_effects,
            }
        }
        LocalInsightsOperation::ComparisonPreviewSpec { input } => {
            let store = existing_store(request.store_dir.as_deref())?
                .ok_or(ComparisonSpecificationError::NoSavedTasks)?;
            let (specification, result) = store.comparison_specification_preview(
                input.evidence_cutoff,
                input.cohort_labels,
                input.date_start,
                input.date_end,
                input.stratum,
            )?;
            LocalInsightsResponse::ComparisonPreviewSpec {
                specification: Box::new(specification),
                result: Box::new(result),
            }
        }
        LocalInsightsOperation::ComparisonSaveSpec { input } => {
            LocalInsightsResponse::ComparisonSpecification {
                specification: Box::new(store()?.comparison_specification_save(
                    input.evidence_cutoff,
                    input.cohort_labels,
                    input.date_start,
                    input.date_end,
                    input.stratum,
                )?),
            }
        }
        LocalInsightsOperation::ComparisonListSpecs {} => {
            LocalInsightsResponse::ComparisonSpecificationList {
                specifications: match existing_store(request.store_dir.as_deref())? {
                    Some(store) => store.comparison_specification_list()?,
                    None => Vec::new(),
                },
            }
        }
        LocalInsightsOperation::ComparisonGetSpec { id } => {
            LocalInsightsResponse::ComparisonSpecification {
                specification: Box::new(specification_store()?.comparison_specification_get(&id)?),
            }
        }
        LocalInsightsOperation::ComparisonEvaluate { id } => {
            LocalInsightsResponse::ComparisonResult {
                result: Box::new(specification_store()?.comparison_specification_evaluate(&id)?),
            }
        }
        LocalInsightsOperation::ComparisonExplainResult {
            specification_id,
            audit_digest,
        } => LocalInsightsResponse::ComparisonResult {
            result: Box::new(
                specification_store()?
                    .comparison_result_explain(&specification_id, &audit_digest)?,
            ),
        },
        LocalInsightsOperation::Analyze { source, file, save } => {
            let (insight, mutation_effects) = if save {
                let result = store()?.import_with_effects(source, &file)?;
                (result.value, result.mutation_effects)
            } else {
                (analyze_file(source, &file)?, MutationEffects::default())
            };
            LocalInsightsResponse::Analyze {
                insight: Box::new(insight),
                mutation_effects,
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
        LocalInsightsOperation::Delete { id } => {
            let result = store()?.delete_with_effects(&id)?;
            LocalInsightsResponse::Delete {
                deleted: result.value,
                mutation_effects: result.mutation_effects,
            }
        }
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
    let response = execute_inner(request).map_err(public_error)?;
    response_json(&response).map_err(public_error)
}

/// Only concrete, payload-free types may cross the JSON/FFI error boundary.
fn public_error(error: anyhow::Error) -> anyhow::Error {
    if let Some(error) = error.downcast_ref::<super::card_store::CardStoreError>() {
        return anyhow!(error.to_string());
    }
    if let Some(error) = error.downcast_ref::<super::cards::QuestionCardProjectionError>() {
        return anyhow!(error.to_string());
    }
    if let Some(error) = error.downcast_ref::<EpisodeValidationError>() {
        return anyhow!(match error {
            EpisodeValidationError::Invalid => "insights_episode_invalid",
            EpisodeValidationError::MemberLimit => "insights_episode_member_limit",
            EpisodeValidationError::DuplicateMember => "insights_episode_duplicate_member",
        });
    }
    if let Some(error) = error.downcast_ref::<EpisodeStoreError>() {
        return anyhow!(match error {
            EpisodeStoreError::NotFound => "insights_episode_not_found",
            EpisodeStoreError::MissingMembers => "insights_episode_missing_members",
            EpisodeStoreError::RevisionConflict => "insights_episode_revision_conflict",
            EpisodeStoreError::Full => "insights_episode_limit_exceeded",
            EpisodeStoreError::RevisionOverflow => "insights_episode_revision_overflow",
        });
    }
    if let Some(error) = error.downcast_ref::<ComparisonTaskValidationError>() {
        return anyhow!(error.to_string());
    }
    if let Some(error) = error.downcast_ref::<ComparisonTaskStoreError>() {
        return anyhow!(error.to_string());
    }
    if let Some(error) = error.downcast_ref::<ComparisonSpecificationError>() {
        return anyhow!(error.to_string());
    }
    if error.downcast_ref::<ResponseTooLarge>().is_some() {
        return anyhow!(ResponseTooLarge);
    }
    anyhow!("insights-operation-failed")
}

fn response_json(response: &LocalInsightsResponse) -> Result<String> {
    struct Bounded {
        bytes: Vec<u8>,
        exceeded: bool,
    }
    impl Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > MAX_RESPONSE_BYTES.saturating_sub(self.bytes.len()) {
                self.exceeded = true;
                return Err(std::io::Error::other("bounded response"));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Bounded {
        bytes: Vec::new(),
        exceeded: false,
    };
    let result = serde_json::to_writer(&mut writer, response);
    if writer.exceeded {
        return Err(ResponseTooLarge.into());
    }
    result.map_err(|_| anyhow!("insights-response-invalid"))?;
    String::from_utf8(writer.bytes).map_err(|_| anyhow!("insights-response-invalid"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_reads_on_an_absent_store_are_read_only_and_missing_selections_are_explicit() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("never-created");
        let request = |snapshot_ids| LocalInsightsRequest {
            store_dir: Some(directory.clone()),
            operation: LocalInsightsOperation::QuestionCards {
                questions: InsightQuestionId::ALL.to_vec(),
                snapshot_ids,
                episode_ids: vec![],
            },
        };
        let LocalInsightsResponse::QuestionCards { result, text } =
            execute(request(vec![])).unwrap()
        else {
            panic!("card response expected");
        };
        assert_eq!(result.cards.len(), 4);
        assert!(text.contains("Applicable versioned pricing evidence is not available."));
        assert!(!directory.exists());
        let error = execute(request(vec!["a".repeat(64)])).unwrap_err();
        assert_eq!(
            public_error(error).to_string(),
            "insights_card_snapshot_not_found"
        );
        assert!(!directory.exists());
        let error = execute(request(vec!["PRIVATE_INVALID_PATH".into()])).unwrap_err();
        assert_eq!(
            public_error(error).to_string(),
            "insights_card_invalid_selection"
        );
        assert!(!directory.exists());
    }

    #[test]
    fn card_errors_are_typed_and_do_not_forward_source_context() {
        let error: anyhow::Error = super::super::card_store::CardStoreError::MissingEpisode.into();
        assert_eq!(
            public_error(error.context("PRIVATE_SOURCE")).to_string(),
            "insights_card_episode_not_found"
        );
        assert_eq!(
            public_error(anyhow!("insights_card_episode_not_found")).to_string(),
            "insights-operation-failed"
        );
    }

    #[test]
    fn episode_error_whitelist_is_typed_and_never_forwards_context_or_impostors() {
        let errors: Vec<anyhow::Error> = vec![
            EpisodeValidationError::Invalid.into(),
            EpisodeValidationError::MemberLimit.into(),
            EpisodeValidationError::DuplicateMember.into(),
            EpisodeStoreError::NotFound.into(),
            EpisodeStoreError::MissingMembers.into(),
            EpisodeStoreError::RevisionConflict.into(),
            EpisodeStoreError::Full.into(),
            EpisodeStoreError::RevisionOverflow.into(),
            ResponseTooLarge.into(),
        ];
        for error in errors {
            let expected = error.to_string();
            assert_eq!(
                public_error(error.context("PRIVATE_PATH_AND_CONTENT")).to_string(),
                expected
            );
            assert_eq!(
                public_error(anyhow!(expected)).to_string(),
                "insights-operation-failed"
            );
        }
        assert_eq!(
            public_error(anyhow!("PRIVATE_SOURCE parser detail")).to_string(),
            "insights-operation-failed"
        );
    }

    #[test]
    fn response_limit_rejects_oversize_without_truncating_or_allocating_its_tail() {
        let response = LocalInsightsResponse::Copy {
            copy: [("fixture".into(), "x".repeat(MAX_RESPONSE_BYTES))].into(),
        };
        let error = response_json(&response).unwrap_err();
        assert!(error.downcast_ref::<ResponseTooLarge>().is_some());
        assert_eq!(
            public_error(error).to_string(),
            "insights_response_too_large"
        );
        let response = LocalInsightsResponse::Copy {
            copy: [("fixture".into(), "x".repeat(MAX_RESPONSE_BYTES - 1024))].into(),
        };
        let encoded = response_json(&response).unwrap();
        assert!(encoded.len() <= MAX_RESPONSE_BYTES);
        assert!(serde_json::from_str::<LocalInsightsResponse>(&encoded).is_ok());
    }

    fn wire(store: &std::path::Path, operation: serde_json::Value) -> Result<serde_json::Value> {
        let request =
            serde_json::to_vec(&serde_json::json!({"store_dir":store,"operation":operation}))?;
        Ok(serde_json::from_str(&dispatch_json(&request)?)?)
    }

    #[test]
    fn episode_absence_and_invalid_requests_create_no_state() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("absent");
        assert_eq!(
            wire(&store, serde_json::json!({"type":"episode_list"})).unwrap(),
            serde_json::json!({"type":"episode_list","episodes":[]})
        );
        let id = uuid::Uuid::new_v4().to_string();
        for kind in [
            "episode_explain",
            "episode_clear_assessment",
            "episode_delete",
        ] {
            let mut operation = serde_json::json!({"type":kind,"id":id});
            if kind != "episode_explain" {
                operation["expected_revision"] = 1.into();
            }
            assert_eq!(
                wire(&store, operation).unwrap_err().to_string(),
                "insights_episode_not_found"
            );
        }
        assert_eq!(
            wire(
                &store,
                serde_json::json!({"type":"episode_create","snapshot_ids":["a".repeat(64)]})
            )
            .unwrap_err()
            .to_string(),
            "insights_episode_missing_members"
        );
        assert_eq!(
            wire(
                &store,
                serde_json::json!({"type":"episode_create","snapshot_ids":[]})
            )
            .unwrap_err()
            .to_string(),
            "insights_episode_member_limit"
        );
        assert_eq!(
            wire(
                &store,
                serde_json::json!({"type":"episode_explain","id":"PRIVATE_INPUT"})
            )
            .unwrap_err()
            .to_string(),
            "insights_episode_invalid"
        );
        assert_eq!(wire(&store, serde_json::json!({"type":"episode_delete","id":id,"expected_revision":1,"secret":"PRIVATE_INPUT"})).unwrap_err().to_string(), "insights-request-invalid");
        assert!(!store.exists());
    }

    #[test]
    fn episode_wire_edits_conflicts_overlap_and_cleanup_preserve_snapshot_summary() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("insights");
        let source = dir.path().join("source.jsonl");
        std::fs::write(&source, b"{\"role\":\"meta\",\"source\":\"claude-code\",\"model\":\"fixture\"}\n{\"role\":\"user\",\"timestamp\":\"2026-09-11T12:00:00Z\",\"content\":\"PRIVATE_BODY\"}\n").unwrap();
        let snapshot = wire(
            &store,
            serde_json::json!({"type":"analyze","source":"trajectory","file":source,"save":true}),
        )
        .unwrap();
        let snapshot_id = snapshot["insight"]["id"].as_str().unwrap();
        assert_eq!(
            snapshot["mutation_effects"]["invalidated_episode_ids"],
            serde_json::json!([])
        );
        let summary = wire(&store, serde_json::json!({"type":"summary"})).unwrap();
        let create = serde_json::json!({"type":"episode_create","snapshot_ids":[snapshot_id]});
        let first = wire(&store, create.clone()).unwrap();
        let second = wire(&store, create).unwrap();
        let id = first["episode"]["id"].as_str().unwrap();
        let other = second["episode"]["id"].as_str().unwrap();
        assert_ne!(id, other);
        let edited = wire(&store, serde_json::json!({"type":"episode_annotate","id":id,"expected_revision":1,"category":"tests","outcome":"accepted"})).unwrap();
        assert_eq!(edited["episode"]["revision"], 2);
        assert_eq!(edited["episode"]["membership_revision"], 1);
        let conflict = wire(
            &store,
            serde_json::json!({"type":"episode_delete","id":id,"expected_revision":1}),
        )
        .unwrap_err();
        assert_eq!(conflict.to_string(), "insights_episode_revision_conflict");
        let detail = wire(
            &store,
            serde_json::json!({"type":"episode_explain","id":id}),
        )
        .unwrap();
        assert_eq!(
            detail["detail"]["overlap"][0]["episode_ids"],
            serde_json::json!([other])
        );
        assert_eq!(detail["detail"]["members"][0]["id"], snapshot_id);
        assert!(!detail.to_string().contains("PRIVATE_BODY"));
        assert_eq!(
            wire(&store, serde_json::json!({"type":"summary"})).unwrap(),
            summary
        );
        let cleared = wire(
            &store,
            serde_json::json!({"type":"episode_clear_assessment","id":id,"expected_revision":2}),
        )
        .unwrap();
        assert!(cleared["episode"]["manual_assessment"].is_null());
        let deleted = wire(
            &store,
            serde_json::json!({"type":"delete","id":snapshot_id}),
        )
        .unwrap();
        let mut removed = vec![id, other];
        removed.sort();
        assert_eq!(
            deleted["mutation_effects"]["invalidated_episode_ids"],
            serde_json::json!(removed)
        );
        assert_eq!(
            wire(&store, serde_json::json!({"type":"episode_list"})).unwrap()["episodes"],
            serde_json::json!([])
        );
        assert!(source.exists());
    }

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
        let LocalInsightsResponse::Analyze { insight, .. } =
            call(LocalInsightsOperation::Analyze {
                source: SourceFormat::Trajectory,
                file: file.clone(),
                save: false,
            })
        else {
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
            LocalInsightsResponse::Delete { deleted: true, .. }
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
        assert_eq!(value["copy"]["insights_store_title"], "Insights store");
        assert_eq!(
            value["copy"]["insights_store_not_directory"],
            "The selected Insights store path is not a directory."
        );
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

    #[test]
    fn retrospective_specification_service_lifecycle_is_typed_and_digest_bound() {
        let temp = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let store = temp.path().join("insights");
        let input = serde_json::json!({
            "evidence_cutoff":"2026-09-11T00:00:00Z",
            "cohort_labels":["model-a","model-b"],
            "date_start":"2026-09-01",
            "date_end":"2026-09-10",
            "stratum":{
                "project_id":"00000000-0000-4000-8000-000000000001",
                "language":"rust",
                "configuration_fingerprint":"11".repeat(32)
            }
        });
        let call = |operation: serde_json::Value| {
            let request = serde_json::json!({"store_dir":store,"operation":operation});
            let response = dispatch_json(&serde_json::to_vec(&request).unwrap()).unwrap();
            serde_json::from_str::<serde_json::Value>(&response).unwrap()
        };
        let saved = call(serde_json::json!({"type":"comparison_save_spec","input":input}));
        let id = saved["specification"]["id"].as_str().unwrap();
        assert_eq!(
            saved["specification"]["provenance"],
            "retrospective_user_specification"
        );
        let listed = call(serde_json::json!({"type":"comparison_list_specs"}));
        assert_eq!(listed["specifications"].as_array().unwrap().len(), 1);
        let result = call(serde_json::json!({"type":"comparison_evaluate","id":id}));
        assert_eq!(result["result"]["cohorts"].as_array().unwrap().len(), 2);
        assert!(
            result["result"]["included_task_ids"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let audit = result["result"]["audit_digest"].as_str().unwrap();
        let explained = call(serde_json::json!({
            "type":"comparison_explain_result",
            "specification_id":id,
            "audit_digest":audit
        }));
        assert_eq!(explained, result);

        let bad = serde_json::json!({"store_dir":store,"operation":{
            "type":"comparison_explain_result",
            "specification_id":id,
            "audit_digest":"ff".repeat(32)
        }});
        assert_eq!(
            dispatch_json(&serde_json::to_vec(&bad).unwrap())
                .unwrap_err()
                .to_string(),
            "insights-comparison-result-stale"
        );
    }
}
