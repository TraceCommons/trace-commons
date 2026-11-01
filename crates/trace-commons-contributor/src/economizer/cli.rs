//! `near-ai economizer`: turn it on or off, see what it learned about a
//! folder, and give left-out tools back.

use clap::Subcommand;
use serde_json::json;
use std::path::PathBuf;

use super::Harness;
use super::store::{StateBusy, Store, repo_key};
use super::summary::{self, FolderSummary};
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
                let key = repo_key(&cwd, harness);
                let state = store.repo(&key);
                let row = FolderSummary::of(String::new(), harness, key, &state);
                let facts: Vec<String> = row
                    .told
                    .iter()
                    .map(|fact| format!("{} {}", fact.prefix, fact.program))
                    .collect();
                rows.push((harness, row.sessions, row.left_out, facts, row.forming));
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
            let mut busy = Vec::new();
            for harness in HARNESSES
                .into_iter()
                .filter(|h| harness.is_none_or(|only| only == *h))
            {
                // Under the folder's lock (`summary::restore`), so a session
                // ending at the same time neither loses its fold nor undoes
                // the Give back. A busy folder does not stop the others:
                // what was given back is still reported, then the busy ones
                // as an error.
                match summary::restore(&store, harness, &repo_key(&cwd, harness)) {
                    Ok(given) if !given.is_empty() => restored.push((harness, given)),
                    Ok(_) => {}
                    Err(error) if error.is::<StateBusy>() => busy.push(harness),
                    Err(error) => {
                        // Not one trying again can fix: report what was
                        // given back so far, then the cause.
                        if !restored.is_empty() {
                            report_restored(&restored, &busy, as_json);
                        }
                        return Err(error);
                    }
                }
            }
            report_restored(&restored, &busy, as_json);
            if !busy.is_empty() {
                let names: Vec<_> = busy.iter().map(|harness| label(*harness)).collect();
                anyhow::bail!(
                    "Couldn't give back for {} here: a session is writing. Try again.",
                    names.join(", ")
                );
            }
        }
    }
    Ok(())
}

/// Print what Give back gave back, and which harnesses were busy.
fn report_restored(restored: &[(Harness, Vec<String>)], busy: &[Harness], as_json: bool) {
    if as_json {
        let rows: Vec<_> = restored
            .iter()
            .map(|(harness, tools)| json!({ "harness": harness, "restored": tools }))
            .collect();
        println!("{}", json!({ "restored": rows, "busy": busy }));
    } else if restored.is_empty() && busy.is_empty() {
        println!("Nothing was left out here.");
    } else {
        for (harness, tools) in restored {
            println!(
                "{}: given back from the next session: {}",
                label(*harness),
                tools.join(", ")
            );
        }
    }
}
