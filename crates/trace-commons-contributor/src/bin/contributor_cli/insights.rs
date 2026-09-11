use std::path::PathBuf;

use anyhow::{Result, anyhow};
use clap::{Args, Subcommand, ValueEnum};
use trace_commons_contributor::insights::{
    LocalInsight, LocalInsightStore, SourceFormat, analyze_file,
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
    let path = match &args.store_dir {
        Some(path) => path.clone(),
        None => dirs::data_local_dir()
            .ok_or_else(|| anyhow!("insights-local-directory-unavailable"))?
            .join("trace-commons")
            .join("insights"),
    };
    LocalInsightStore::open(&path)
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
            let insights = store(args)?.list()?;
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
        println!("Cost, task outcome, and model comparisons lack sufficient evidence.");
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
