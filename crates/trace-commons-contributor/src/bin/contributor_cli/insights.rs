use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Subcommand, ValueEnum};
use trace_commons_contributor::insights::service::{
    LocalInsightsOperation, LocalInsightsRequest, LocalInsightsResponse, execute,
};
use trace_commons_contributor::insights::usage::UsageSource;
use trace_commons_contributor::insights::{
    LocalInsight, LocalInsightStore, SourceFormat, TaskCategory, TaskOutcome, analyze_file,
    service::open_store,
};
use trace_commons_protocol::insights::MetricId;

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
    /// Inspect source-native usage in one file; does not save or estimate prices
    Usage {
        #[arg(long, value_enum)]
        source: NativeUsageSource,
        #[arg(long)]
        file: PathBuf,
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
        InsightsCommand::Analyze { source, file, save } => {
            let insight = if *save {
                store(args)?.import((*source).into(), file)?
            } else {
                analyze_file((*source).into(), file)?
            };
            render(&insight, json)?;
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
        InsightsCommand::Explain { id } => render(&store(args)?.explain(id)?, json)?,
        InsightsCommand::Delete { id } => {
            let deleted = store(args)?.delete(id)?;
            if json {
                println!("{}", serde_json::json!({ "deleted": deleted }));
            } else if deleted {
                println!("Deleted the saved insight and its source references.");
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
        println!("Evidence references:");
        for evidence in &insight.report.evidence {
            println!("  {} — SHA-256 {}", evidence.id, evidence.source_digest);
        }
    }
    Ok(())
}
