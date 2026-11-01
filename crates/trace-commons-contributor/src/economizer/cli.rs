//! `near-ai economizer`: turn it on or off, see what it learned about a
//! folder, and give left-out tools back.

use clap::Subcommand;
use serde_json::json;
use std::path::PathBuf;

use super::Harness;
use super::store::{Store, repo_key};
use super::tools::Thresholds;
use crate::config::ConfigStore;

const HARNESSES: [Harness; 3] = [Harness::Claude, Harness::Codex, Harness::Opencode];

#[derive(Debug, Subcommand)]
pub enum EconomizerCommand {
    /// Turn it on for new managed Claude Code, Codex and OpenCode sessions.
    On,
    /// Turn it off. Running sessions stop using it at their next event.
    Off,
    /// Show what it learned about a folder.
    Status {
        /// The folder (default: the current directory)
        #[arg(long)]
        cwd: Option<PathBuf>,
    },
    /// Give back every tool it leaves out in a folder. They are never left
    /// out there again.
    Restore {
        /// The folder (default: the current directory)
        #[arg(long)]
        cwd: Option<PathBuf>,
        /// Only this tool's (default: every tool's)
        #[arg(long, value_enum)]
        harness: Option<Harness>,
    },
}

fn label(harness: Harness) -> &'static str {
    match harness {
        Harness::Claude => "Claude Code",
        Harness::Codex => "Codex",
        Harness::Opencode => "OpenCode",
    }
}

pub fn run(config: &ConfigStore, command: EconomizerCommand, as_json: bool) -> anyhow::Result<()> {
    let store = Store::new(config.dir());
    let folder = |cwd: Option<PathBuf>| -> anyhow::Result<PathBuf> {
        match cwd {
            Some(cwd) => Ok(cwd),
            None => Ok(std::env::current_dir()?),
        }
    };
    match command {
        EconomizerCommand::On | EconomizerCommand::Off => {
            let on = matches!(command, EconomizerCommand::On);
            store.set_enabled(on)?;
            if as_json {
                println!("{}", json!({ "enabled": on }));
            } else if on {
                println!("Economizer on for new managed sessions.");
            } else {
                println!("Economizer off.");
            }
        }
        EconomizerCommand::Status { cwd } => {
            let cwd = folder(cwd)?;
            let mut rows = Vec::new();
            for harness in HARNESSES {
                let state = store.repo(&repo_key(&cwd, harness));
                let left_out = state.tools.deny_list(harness, &Thresholds::default());
                let facts: Vec<String> = state
                    .facts
                    .told()
                    .map(|fact| format!("{} {}", fact.prefix, fact.program))
                    .collect();
                let forming = state.facts.facts.iter().filter(|f| !f.told()).count();
                rows.push((harness, state.sessions, left_out, facts, forming));
            }
            if as_json {
                let tools: Vec<_> = rows
                    .iter()
                    .map(|(harness, sessions, left_out, facts, forming)| {
                        json!({
                            "harness": harness,
                            "sessions": sessions,
                            "left_out": left_out,
                            "facts": facts,
                            "facts_forming": forming,
                        })
                    })
                    .collect();
                println!("{}", json!({ "enabled": store.enabled(), "tools": tools }));
            } else {
                println!("Economizer: {}", if store.enabled() { "on" } else { "off" });
                for (harness, sessions, left_out, facts, forming) in rows {
                    println!("{}:", label(harness));
                    println!("  Sessions learned from here: {sessions}");
                    println!(
                        "  Left out: {}",
                        if left_out.is_empty() {
                            "none".to_owned()
                        } else {
                            left_out.join(", ")
                        }
                    );
                    println!(
                        "  Commands it tells the agent: {}",
                        if facts.is_empty() {
                            "none".to_owned()
                        } else {
                            facts.join(", ")
                        }
                    );
                    println!("  Commands still being confirmed: {forming}");
                }
            }
        }
        EconomizerCommand::Restore { cwd, harness } => {
            let cwd = folder(cwd)?;
            let mut restored = Vec::new();
            for harness in HARNESSES
                .into_iter()
                .filter(|h| harness.is_none_or(|only| only == *h))
            {
                let key = repo_key(&cwd, harness);
                let mut state = store.repo(&key);
                let given = state.tools.restore(harness, &Thresholds::default());
                if !given.is_empty() {
                    store.save_repo(&key, &state)?;
                    restored.push((harness, given));
                }
            }
            if as_json {
                let rows: Vec<_> = restored
                    .iter()
                    .map(|(harness, tools)| json!({ "harness": harness, "restored": tools }))
                    .collect();
                println!("{}", json!({ "restored": rows }));
            } else if restored.is_empty() {
                println!("Nothing was left out here.");
            } else {
                for (harness, tools) in restored {
                    println!(
                        "{}: given back from the next session: {}",
                        label(harness),
                        tools.join(", ")
                    );
                }
            }
        }
    }
    Ok(())
}
