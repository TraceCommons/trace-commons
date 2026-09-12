use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Subcommand, ValueEnum};
use trace_commons_contributor::insights::comparison_tasks::{
    CheckoutProvenance, ComparisonConfigurationV1, ComparisonTaskContextInput, ContextDigest,
    ContextString,
};
use trace_commons_contributor::insights::service::{
    LocalInsightsOperation, LocalInsightsRequest, LocalInsightsResponse, execute,
};
use trace_commons_contributor::insights::usage::UsageSource;
use trace_commons_contributor::insights::{
    LocalInsight, LocalInsightStore, MutationEffects, SourceFormat, TaskCategory, TaskOutcome,
    service::open_store,
};
use trace_commons_protocol::insights::MetricId;
use trace_commons_protocol::insights_cards::InsightQuestionId;

#[derive(Args)]
pub(super) struct InsightsArgs {
    /// Local insights directory, separate from enrollment state
    #[arg(long, global = true)]
    store_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: InsightsCommand,
}

#[derive(Subcommand)]
enum InsightsCommand {
    /// Show shared cards for explicitly selected snapshots and episode groups
    Cards {
        #[arg(long = "snapshot")]
        snapshot_ids: Vec<String>,
        #[arg(long = "episode")]
        episode_ids: Vec<String>,
    },
    /// Group explicitly selected whole saved snapshots; not inferred task boundaries
    EpisodeCreate {
        #[arg(long = "snapshot", required = true)]
        snapshot_ids: Vec<String>,
    },
    /// List user-selected episode groups, including overlap
    EpisodeList,
    /// Resolve current evidence for one saved episode
    EpisodeExplain { id: String },
    /// Replace the whole-snapshot membership and clear its independent assessment
    EpisodeReplaceMembers {
        id: String,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long = "snapshot", required = true)]
        snapshot_ids: Vec<String>,
    },
    /// Record your independent episode assessment, not verified success
    EpisodeAnnotate {
        id: String,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long, value_parser = ["refactor", "tests", "docs", "debugging", "other", "unknown"])]
        category: String,
        #[arg(long, value_parser = ["accepted", "partial", "rejected", "unknown"])]
        outcome: String,
    },
    /// Clear an episode assessment while preserving members
    EpisodeClearAssessment {
        id: String,
        #[arg(long)]
        expected_revision: u64,
    },
    /// Remove a group and its assessment; preserve its snapshots and original files
    EpisodeDelete {
        id: String,
        #[arg(long)]
        expected_revision: u64,
    },
    /// Manage user-reviewed comparison tasks built from frozen episodes
    ComparisonTask {
        #[command(subcommand)]
        command: ComparisonTaskCommand,
    },
    /// Analyze one selected file locally; persist only when --save is supplied
    Analyze {
        #[arg(long, value_enum)]
        source: Source,
        #[arg(long)]
        file: PathBuf,
        /// Save derived counts and hashed references, without transcript bodies
        #[arg(long)]
        save: bool,
    },
    /// List previously saved local insights
    List,
    /// Summarize current saved session snapshots and separate user-reported assessments
    Summary,
    /// Show the measurements and evidence references for a saved insight
    Explain { id: String },
    /// Delete a saved insight and its source references; leave the original file intact
    Delete { id: String },
    /// Record your assessment of a saved snapshot, separate from verified outcomes
    Annotate {
        id: String,
        #[arg(long, value_parser = ["refactor", "tests", "docs", "debugging", "other", "unknown"])]
        category: String,
        #[arg(long, value_parser = ["accepted", "partial", "rejected", "unknown"])]
        outcome: String,
    },
    /// Remove your assessment without deleting the saved snapshot
    ClearAnnotation { id: String },
    /// Explicitly link a local commit object; does not establish acceptance or merge
    LinkGit {
        id: String,
        #[arg(long)]
        repository: PathBuf,
        #[arg(long)]
        commit: String,
    },
    /// Link a structured test report as user-supplied evidence, without executing tests
    LinkTestReport {
        id: String,
        #[arg(long)]
        file: PathBuf,
    },
    /// Remove a saved evidence link, preserving the original repository or report
    UnlinkEvidence { id: String, evidence_id: String },
    /// Inspect source-native usage in one file; does not save or estimate prices
    Usage {
        #[arg(long, value_enum)]
        source: NativeUsageSource,
        #[arg(long)]
        file: PathBuf,
    },
}

#[derive(Subcommand)]
enum ComparisonTaskCommand {
    Create {
        #[arg(long = "episode", required = true)]
        episode_ids: Vec<String>,
    },
    List,
    Explain {
        id: String,
    },
    ReplaceEpisodes {
        id: String,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long = "episode", required = true)]
        episode_ids: Vec<String>,
    },
    SetContext {
        id: String,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        project_id: String,
        #[arg(long)]
        task_date: String,
        #[arg(long)]
        language: Option<String>,
        #[arg(long)]
        harness_id: Option<String>,
        #[arg(long)]
        harness_version: Option<String>,
        #[arg(long, default_value = "unknown", value_parser = ["unknown", "none", "minimal", "low", "medium", "high", "xhigh"])]
        reasoning_effort: String,
        #[arg(long)]
        tool_policy_id: Option<String>,
        #[arg(long)]
        tool_policy_version: Option<String>,
        #[arg(long)]
        prompt_template_digest: Option<String>,
    },
    SetOutcome {
        id: String,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long, value_parser = ["pending", "accepted", "partial", "rejected", "unknown"])]
        outcome: String,
    },
    ClearOutcome {
        id: String,
        #[arg(long)]
        expected_revision: u64,
    },
    Reconfirm {
        id: String,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        material_digest: String,
    },
    Delete {
        id: String,
        #[arg(long)]
        expected_revision: u64,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum NativeUsageSource {
    Codex,
    ClaudeCode,
}

#[derive(Clone, Copy, ValueEnum)]
enum Source {
    Codex,
    Trajectory,
}

impl From<Source> for SourceFormat {
    fn from(value: Source) -> Self {
        match value {
            Source::Codex => Self::Codex,
            Source::Trajectory => Self::Trajectory,
        }
    }
}

fn store(args: &InsightsArgs) -> Result<LocalInsightStore> {
    open_store(args.store_dir.as_deref())
}

pub(super) fn run(args: &InsightsArgs, json: bool) -> Result<()> {
    match &args.command {
        InsightsCommand::Cards {
            snapshot_ids,
            episode_ids,
        } => {
            let response = execute(LocalInsightsRequest {
                store_dir: args.store_dir.clone(),
                operation: LocalInsightsOperation::QuestionCards {
                    questions: InsightQuestionId::ALL.to_vec(),
                    snapshot_ids: snapshot_ids.clone(),
                    episode_ids: episode_ids.clone(),
                },
            })?;
            if json {
                println!("{}", serde_json::to_string_pretty(&response)?);
            } else if let LocalInsightsResponse::QuestionCards { text, .. } = response {
                println!("{text}");
            }
        }

        InsightsCommand::EpisodeCreate { snapshot_ids } => render_episode_operation(
            args,
            LocalInsightsOperation::EpisodeCreate {
                snapshot_ids: snapshot_ids.clone(),
            },
            json,
        )?,
        InsightsCommand::EpisodeList => {
            render_episode_operation(args, LocalInsightsOperation::EpisodeList {}, json)?
        }
        InsightsCommand::EpisodeExplain { id } => render_episode_operation(
            args,
            LocalInsightsOperation::EpisodeExplain { id: id.clone() },
            json,
        )?,
        InsightsCommand::EpisodeReplaceMembers {
            id,
            expected_revision,
            snapshot_ids,
        } => render_episode_operation(
            args,
            LocalInsightsOperation::EpisodeReplaceMembers {
                id: id.clone(),
                expected_revision: *expected_revision,
                snapshot_ids: snapshot_ids.clone(),
            },
            json,
        )?,
        InsightsCommand::EpisodeAnnotate {
            id,
            expected_revision,
            category,
            outcome,
        } => render_episode_operation(
            args,
            LocalInsightsOperation::EpisodeAnnotate {
                id: id.clone(),
                expected_revision: *expected_revision,
                category: serde_json::from_value(category.clone().into())?,
                outcome: serde_json::from_value(outcome.clone().into())?,
            },
            json,
        )?,
        InsightsCommand::EpisodeClearAssessment {
            id,
            expected_revision,
        } => render_episode_operation(
            args,
            LocalInsightsOperation::EpisodeClearAssessment {
                id: id.clone(),
                expected_revision: *expected_revision,
            },
            json,
        )?,
        InsightsCommand::EpisodeDelete {
            id,
            expected_revision,
        } => render_episode_operation(
            args,
            LocalInsightsOperation::EpisodeDelete {
                id: id.clone(),
                expected_revision: *expected_revision,
            },
            json,
        )?,
        InsightsCommand::ComparisonTask { command } => {
            let operation = match command {
                ComparisonTaskCommand::Create { episode_ids } => {
                    LocalInsightsOperation::ComparisonTaskCreate {
                        episode_ids: episode_ids.clone(),
                    }
                }
                ComparisonTaskCommand::List => LocalInsightsOperation::ComparisonTaskList {},
                ComparisonTaskCommand::Explain { id } => {
                    LocalInsightsOperation::ComparisonTaskExplain { id: id.clone() }
                }
                ComparisonTaskCommand::ReplaceEpisodes {
                    id,
                    expected_revision,
                    episode_ids,
                } => LocalInsightsOperation::ComparisonTaskReplaceEpisodes {
                    id: id.clone(),
                    expected_revision: *expected_revision,
                    episode_ids: episode_ids.clone(),
                },
                ComparisonTaskCommand::SetContext {
                    id,
                    expected_revision,
                    project_id,
                    task_date,
                    language,
                    harness_id,
                    harness_version,
                    reasoning_effort,
                    tool_policy_id,
                    tool_policy_version,
                    prompt_template_digest,
                } => {
                    let configuration = ComparisonConfigurationV1 {
                        harness_id: context_string(harness_id),
                        harness_version: context_string(harness_version),
                        reasoning_effort: serde_json::from_value(reasoning_effort.clone().into())?,
                        tool_policy_id: context_string(tool_policy_id),
                        tool_policy_version: context_string(tool_policy_version),
                        prompt_template_digest: prompt_template_digest.as_ref().map_or(
                            ContextDigest::Unknown,
                            |digest| ContextDigest::Known {
                                digest: digest.clone(),
                            },
                        ),
                    };
                    let context = ComparisonTaskContextInput {
                        project_id: project_id.clone(),
                        category: TaskCategory::Refactor,
                        task_date: task_date.parse()?,
                        checkout_provenance: CheckoutProvenance::Unavailable,
                        language: context_string(language),
                        configuration,
                    };
                    LocalInsightsOperation::ComparisonTaskSetContext {
                        id: id.clone(),
                        expected_revision: *expected_revision,
                        context,
                    }
                }
                ComparisonTaskCommand::SetOutcome {
                    id,
                    expected_revision,
                    outcome,
                } => LocalInsightsOperation::ComparisonTaskSetOutcome {
                    id: id.clone(),
                    expected_revision: *expected_revision,
                    outcome: serde_json::from_value(outcome.clone().into())?,
                },
                ComparisonTaskCommand::ClearOutcome {
                    id,
                    expected_revision,
                } => LocalInsightsOperation::ComparisonTaskClearOutcome {
                    id: id.clone(),
                    expected_revision: *expected_revision,
                },
                ComparisonTaskCommand::Reconfirm {
                    id,
                    expected_revision,
                    material_digest,
                } => LocalInsightsOperation::ComparisonTaskReconfirm {
                    id: id.clone(),
                    expected_revision: *expected_revision,
                    displayed_material_digest: material_digest.clone(),
                },
                ComparisonTaskCommand::Delete {
                    id,
                    expected_revision,
                } => LocalInsightsOperation::ComparisonTaskDelete {
                    id: id.clone(),
                    expected_revision: *expected_revision,
                },
            };
            render_comparison_task_operation(args, operation, json)?;
        }
        InsightsCommand::Analyze { source, file, save } => {
            let LocalInsightsResponse::Analyze {
                insight,
                mutation_effects,
            } = execute(LocalInsightsRequest {
                store_dir: args.store_dir.clone(),
                operation: LocalInsightsOperation::Analyze {
                    source: (*source).into(),
                    file: file.clone(),
                    save: *save,
                },
            })?
            else {
                anyhow::bail!("insights-analyze-response-invalid");
            };
            if *save && json {
                #[derive(serde::Serialize)]
                struct Saved<'a> {
                    #[serde(flatten)]
                    insight: &'a LocalInsight,
                    mutation_effects: &'a MutationEffects,
                }
                println!(
                    "{}",
                    serde_json::to_string_pretty(&Saved {
                        insight: &insight,
                        mutation_effects: &mutation_effects
                    })?
                );
            } else {
                render(&insight, json)?;
                render_mutation_effects(&mutation_effects);
            }
        }
        InsightsCommand::List => {
            let insights = trace_commons_contributor::insights::service::list_saved(
                args.store_dir.as_deref(),
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&insights)?);
            } else if insights.is_empty() {
                println!("No saved insights. Analyze a session with --save to keep its results.");
            } else {
                for insight in insights {
                    render(&insight, false)?;
                }
            }
        }
        InsightsCommand::Summary => {
            let summary = trace_commons_contributor::insights::summary::read_saved(
                args.store_dir.as_deref(),
            )?;
            if !json {
                println!(
                    "All currently saved, explicitly selected session snapshots; not verified completed tasks."
                );
                println!(
                    "Assessments are user-reported. Unassessed and explicitly unknown are separate."
                );
                println!(
                    "Metric sums cover observed values only. Read snapshot availability and record coverage together."
                );
                println!(
                    "Analysis dates describe saved observations, not activity time. No model ranking, time savings, or cost is inferred."
                );
            }
            println!("{}", serde_json::to_string_pretty(&summary)?);
        }
        InsightsCommand::Explain { id } => render(&store(args)?.explain(id)?, json)?,
        InsightsCommand::Delete { id } => {
            let LocalInsightsResponse::Delete {
                deleted,
                mutation_effects,
            } = execute(LocalInsightsRequest {
                store_dir: args.store_dir.clone(),
                operation: LocalInsightsOperation::Delete { id: id.clone() },
            })?
            else {
                anyhow::bail!("insights-delete-response-invalid");
            };
            if json {
                println!(
                    "{}",
                    serde_json::json!({ "deleted": deleted, "mutation_effects": mutation_effects })
                );
            } else if deleted {
                println!("Deleted the saved insight and its source references.");
                render_mutation_effects(&mutation_effects);
            } else {
                println!("No saved insight matched that identifier.");
            }
        }
        InsightsCommand::Annotate {
            id,
            category,
            outcome,
        } => {
            let category: TaskCategory = serde_json::from_value(category.clone().into())?;
            let outcome: TaskOutcome = serde_json::from_value(outcome.clone().into())?;
            render(&store(args)?.annotate(id, category, outcome)?, json)?;
        }
        InsightsCommand::ClearAnnotation { id } => {
            render(&store(args)?.clear_annotation(id)?, json)?
        }
        InsightsCommand::LinkGit {
            id,
            repository,
            commit,
        } => {
            render_operation(
                args,
                LocalInsightsOperation::LinkGit {
                    id: id.clone(),
                    repository: repository.clone(),
                    commit: commit.clone(),
                },
                json,
            )?;
        }
        InsightsCommand::LinkTestReport { id, file } => {
            render_operation(
                args,
                LocalInsightsOperation::LinkTestReport {
                    id: id.clone(),
                    file: file.clone(),
                },
                json,
            )?;
        }
        InsightsCommand::UnlinkEvidence { id, evidence_id } => {
            render_operation(
                args,
                LocalInsightsOperation::UnlinkEvidence {
                    id: id.clone(),
                    evidence_id: evidence_id.clone(),
                },
                json,
            )?;
        }
        InsightsCommand::Usage { source, file } => {
            let source = match source {
                NativeUsageSource::Codex => UsageSource::Codex,
                NativeUsageSource::ClaudeCode => UsageSource::ClaudeCode,
            };
            let LocalInsightsResponse::Usage { usage } = execute(LocalInsightsRequest {
                store_dir: args.store_dir.clone(),
                operation: LocalInsightsOperation::Usage {
                    source,
                    file: file.clone(),
                },
            })?
            else {
                anyhow::bail!("insights-usage-response-invalid")
            };
            if !json {
                println!(
                    "Source-native usage from one selected file. Not a bill or a per-model allocation."
                );
                println!(
                    "Record coverage: {}/{}",
                    usage.complete_records, usage.usage_records
                );
            }
            println!("{}", serde_json::to_string_pretty(&usage)?);
        }
    }
    Ok(())
}

fn context_string(value: &Option<String>) -> ContextString {
    value
        .as_ref()
        .map_or(ContextString::Unknown, |value| ContextString::Known {
            value: value.clone(),
        })
}

fn render_comparison_task_operation(
    args: &InsightsArgs,
    operation: LocalInsightsOperation,
    json: bool,
) -> Result<()> {
    let response = execute(LocalInsightsRequest {
        store_dir: args.store_dir.clone(),
        operation,
    })?;
    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
        return Ok(());
    }
    let copy = trace_commons_contributor::insights::service::ui_copy();
    match response {
        LocalInsightsResponse::ComparisonTaskList { tasks } => {
            if tasks.is_empty() {
                println!("{}", copy["comparison_task_empty"]);
            }
            for detail in tasks {
                render_comparison_task(&detail.task, Some(&detail.stale_reasons), &copy)?;
            }
        }
        LocalInsightsResponse::ComparisonTaskExplain { detail } => {
            render_comparison_task(&detail.task, Some(&detail.stale_reasons), &copy)?
        }
        LocalInsightsResponse::ComparisonTask {
            task,
            mutation_effects,
        }
        | LocalInsightsResponse::ComparisonTaskDelete {
            task,
            mutation_effects,
        } => {
            render_comparison_task(&task, None, &copy)?;
            render_mutation_effects(&mutation_effects);
        }
        _ => anyhow::bail!("insights-comparison-task-response-invalid"),
    }
    Ok(())
}

fn render_comparison_task(
    task: &trace_commons_contributor::insights::comparison_tasks::LocalComparisonTaskV1,
    reasons: Option<
        &[trace_commons_contributor::insights::comparison_tasks::ComparisonTaskStaleReason],
    >,
    copy: &std::collections::BTreeMap<String, String>,
) -> Result<()> {
    println!("{} — {}", copy["comparison_task_title"], task.id);
    println!(
        "Revision: {} · Material revision: {}",
        task.revision, task.material_revision
    );
    println!(
        "{}: {}",
        copy["comparison_task_material_digest"], task.material_digest
    );
    println!(
        "{}",
        if task
            .context
            .as_ref()
            .is_some_and(|context| context.is_complete())
        {
            &copy["comparison_task_context_complete"]
        } else {
            &copy["comparison_task_context_incomplete"]
        }
    );
    if let Some(outcome) = &task.outcome {
        println!(
            "User-reported outcome: {}",
            serde_json::to_value(outcome.value)?
                .as_str()
                .unwrap_or("unknown")
        );
    } else {
        println!("{}", copy["comparison_task_outcome_unassessed"]);
    }
    let confirmation_current = task
        .independence_confirmation
        .as_ref()
        .is_some_and(|value| {
            value.material_revision == task.material_revision
                && value.material_digest == task.material_digest
        });
    println!(
        "{}",
        if confirmation_current {
            &copy["comparison_task_confirmation_current"]
        } else {
            &copy["comparison_task_confirmation_missing"]
        }
    );
    println!("{}", copy["comparison_task_attribution_pending"]);
    if let Some(reasons) = reasons {
        println!(
            "Review reasons: {}",
            reasons
                .iter()
                .filter_map(|reason| serde_json::to_value(reason).ok())
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    Ok(())
}

fn render_mutation_effects(effects: &MutationEffects) {
    if !effects.invalidated_episode_ids.is_empty() {
        let copy = trace_commons_contributor::insights::service::ui_copy();
        println!("{}", copy["episode_invalidated_notice"]);
        for id in &effects.invalidated_episode_ids {
            println!("  {id}");
        }
    }
    for effect in &effects.stale_comparison_tasks {
        let reasons = effect
            .reasons
            .iter()
            .filter_map(|reason| serde_json::to_value(reason).ok())
            .filter_map(|value| value.as_str().map(str::to_owned))
            .collect::<Vec<_>>()
            .join(",");
        println!("Comparison task {} is stale: {reasons}", effect.task_id);
    }
}

fn render_episode_operation(
    args: &InsightsArgs,
    operation: LocalInsightsOperation,
    json: bool,
) -> Result<()> {
    let response = execute(LocalInsightsRequest {
        store_dir: args.store_dir.clone(),
        operation,
    })?;
    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
        return Ok(());
    }
    let copy = trace_commons_contributor::insights::service::ui_copy();
    println!("{}", copy["episode_scope"]);
    println!("{}", copy["episode_assessment_notice"]);
    println!("{}", copy["episode_overlap_notice"]);
    match response {
        LocalInsightsResponse::EpisodeList { episodes } => {
            if episodes.is_empty() {
                println!("{}", copy["episode_empty"]);
            }
            for entry in episodes {
                render_episode(&entry.episode, &copy)?;
                render_overlap(&entry.overlapping_episode_ids, &copy);
            }
        }
        LocalInsightsResponse::EpisodeExplain { detail } => {
            render_episode(&detail.episode, &copy)?;
            if detail.overlap.is_empty() {
                render_overlap(&[], &copy);
            }
            for overlap in &detail.overlap {
                println!("  {}", overlap.snapshot_id);
                render_overlap(&overlap.episode_ids, &copy);
            }
            println!("{}: {}", copy["episode_resolved"], detail.resolved_at);
            println!("{}:", copy["episode_member_evidence"]);
            for member in &detail.members {
                render(member, false)?;
            }
        }
        LocalInsightsResponse::EpisodeDelete {
            episode,
            mutation_effects,
        } => {
            println!("{}", copy["episode_deleted"]);
            render_episode(&episode, &copy)?;
            render_mutation_effects(&mutation_effects);
        }
        LocalInsightsResponse::EpisodeCreate { episode } => render_episode(&episode, &copy)?,
        LocalInsightsResponse::EpisodeReplaceMembers {
            episode,
            mutation_effects,
        }
        | LocalInsightsResponse::EpisodeAnnotate {
            episode,
            mutation_effects,
        }
        | LocalInsightsResponse::EpisodeClearAssessment {
            episode,
            mutation_effects,
        } => {
            render_episode(&episode, &copy)?;
            render_mutation_effects(&mutation_effects);
        }
        _ => anyhow::bail!("insights-episode-response-invalid"),
    }
    Ok(())
}

fn render_overlap(ids: &[String], copy: &std::collections::BTreeMap<String, String>) {
    if ids.is_empty() {
        println!("{}", copy["episode_no_overlap"]);
    } else {
        println!("{}: {}", copy["episode_overlaps"], ids.join(", "));
    }
}

fn render_episode(
    episode: &trace_commons_contributor::insights::episodes::LocalEpisode,
    copy: &std::collections::BTreeMap<String, String>,
) -> Result<()> {
    println!("\n{} — {}", copy["episode_title"], episode.id);
    println!(
        "{}: {} · {}: {}",
        copy["episode_revision"],
        episode.revision,
        copy["episode_membership_revision"],
        episode.membership_revision
    );
    println!("{}:", copy["episode_members"]);
    for member in &episode.members {
        println!(
            "  {} · {}: {}",
            member.snapshot_id, copy["source_digest"], member.source_digest
        );
    }
    if let Some(assessment) = &episode.manual_assessment {
        let category = serde_json::to_value(assessment.category)?;
        let outcome = serde_json::to_value(assessment.outcome)?;
        println!(
            "{}: {} / {} ({})",
            copy["episode_assessment"],
            copy[&format!("category_{}", category.as_str().unwrap_or("unknown"))],
            copy[&format!("outcome_{}", outcome.as_str().unwrap_or("unknown"))],
            assessment.recorded_at
        );
    } else {
        println!(
            "{}: {}",
            copy["episode_assessment"], copy["episode_unassessed"]
        );
    }
    Ok(())
}

fn render_operation(
    args: &InsightsArgs,
    operation: LocalInsightsOperation,
    json: bool,
) -> Result<()> {
    let response = execute(LocalInsightsRequest {
        store_dir: args.store_dir.clone(),
        operation,
    })?;
    let insight = match response {
        LocalInsightsResponse::LinkGit { insight }
        | LocalInsightsResponse::LinkTestReport { insight }
        | LocalInsightsResponse::UnlinkEvidence { insight } => insight,
        _ => anyhow::bail!("insights-evidence-response-invalid"),
    };
    render(&insight, json)
}

fn render(insight: &LocalInsight, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(insight)?);
    } else {
        println!("Trace Commons Insights — {}", insight.id);
        println!("One selected session; task boundaries have not been verified.");
        println!(
            "Snapshot analyzed at {}. Reimport to refresh.",
            insight.analyzed_at
        );
        println!(
            "Cost, independently verified task outcomes, and model comparisons lack sufficient evidence."
        );
        if let Some(annotation) = &insight.manual_annotation {
            println!(
                "Your assessment: {} / {} (user-reported, {}).",
                serde_json::to_value(annotation.category)?
                    .as_str()
                    .unwrap_or("unknown"),
                serde_json::to_value(annotation.outcome)?
                    .as_str()
                    .unwrap_or("unknown"),
                annotation.recorded_at
            );
        }
        println!(
            "Analysis by {} (version {}, rubric {}).",
            insight.report.provider.id,
            insight.report.provider.version,
            insight.report.provider.rubric_version
        );
        for metric in &insight.report.metrics {
            let label = match metric.id {
                MetricId::Sessions => "Session snapshots",
                MetricId::Events => "Recorded events",
                MetricId::InputTokens => "Input tokens",
                MetricId::OutputTokens => "Output tokens",
                MetricId::ToolCalls => "Tool calls",
                MetricId::ToolFailures => "Reported tool failures",
                MetricId::KnownOutcomes => "Known task outcomes",
            };
            let value = metric
                .value
                .map(|count| count.to_string())
                .unwrap_or_else(|| "unknown".into());
            println!(
                "{label}: {value} (coverage: {}/{} {})",
                metric.coverage.observed,
                metric.coverage.total,
                match metric.id {
                    MetricId::ToolFailures => "tool results",
                    MetricId::Events | MetricId::ToolCalls => "normalized events",
                    _ => "session snapshots",
                }
            );
        }
        if let Some(models) = &insight.model_observations {
            println!(
                "Declared model metadata only; not verified serving identity or per-model work allocation:"
            );
            println!("{}", serde_json::to_string_pretty(models)?);
        } else {
            println!(
                "Model observations unavailable in this saved snapshot; explicitly reimport to refresh."
            );
        }
        if !insight.outcome_links.is_empty() {
            println!(
                "Explicit user-linked evidence; Git objects and imported reports do not establish accepted work:"
            );
            println!("{}", serde_json::to_string_pretty(&insight.outcome_links)?);
        }
        println!("Evidence references:");
        for evidence in &insight.report.evidence {
            println!("  {} — SHA-256 {}", evidence.id, evidence.source_digest);
        }
    }
    Ok(())
}
