use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Subcommand};
use trace_commons_contributor::mission_draft::MissionDraftInbox;
use trace_commons_contributor::mission_draft_service::ui_copy;

#[derive(Args)]
pub(super) struct MissionDraftsArgs {
    /// Local mission draft directory, separate from enrollment state
    #[arg(long, global = true)]
    store_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: MissionDraftsCommand,
}

#[derive(Subcommand)]
enum MissionDraftsCommand {
    /// Add one explicitly selected, structurally valid proposal
    Import {
        #[arg(long)]
        file: PathBuf,
    },
    /// List locally imported draft proposals
    List,
    /// Show one local proposal and its non-authoritative structural review
    Show { id: String },
    /// Delete one local draft; never deletes its source file
    Delete { id: String },
}

pub(super) fn run(args: &MissionDraftsArgs, json: bool) -> Result<()> {
    let inbox = MissionDraftInbox::resolve(args.store_dir.as_deref())?;
    let copy = ui_copy();
    match &args.command {
        MissionDraftsCommand::Import { file } => {
            let result = inbox.import_file(file)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!(
                    "{}",
                    if result.inserted {
                        &copy["added"]
                    } else {
                        &copy["duplicate"]
                    }
                );
                println!("Proposal SHA-256: {}", result.id);
                println!("{}", copy["review_notice"]);
            }
        }
        MissionDraftsCommand::List => {
            let drafts = inbox.list()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&drafts)?);
            } else if drafts.is_empty() {
                println!("{}", copy["empty"]);
            } else {
                println!("Local mission drafts: {}", drafts.len());
                for draft in drafts {
                    println!(
                        "{} ({} sources; curator review required)",
                        draft.id, draft.source_count
                    );
                }
            }
        }
        MissionDraftsCommand::Show { id } => {
            let draft = inbox.show(id)?;
            // JSON escaping keeps untrusted proposal control characters from
            // becoming terminal control sequences in the human drilldown.
            println!("{}", serde_json::to_string_pretty(&draft)?);
        }
        MissionDraftsCommand::Delete { id } => {
            let result = inbox.delete(id)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("{}", copy["deleted"]);
                println!("Proposal SHA-256: {}", result.id);
            }
        }
    }
    Ok(())
}
