//! `ironwire update` — and the daemon's background check.
//!
//! IronWire is **notify-only**. It never downloads or applies an update: it is
//! a daemon holding credentials in the middle of a streamed response, and
//! `docs/PROTOCOL.md` §5 says an interrupted stream past the first byte cannot
//! be recovered. A proxy that restarts itself unprompted would be causing the
//! outage it exists to prevent.
//!
//! So this module reports, and tells the user the command that belongs to
//! *their* install — the package manager's, when a package manager owns it.

use anyhow::Result;
use ironwire_proxy::embed::updates::{check_now, install_method};
use ironwire_update::{InstallMethod, UpdateStatus};

/// Show what the running daemon knows, or check directly if it is not running.
pub(crate) async fn run(port: Option<u16>) -> Result<()> {
    let paths = super::paths()?;
    let install = install_method();

    // Prefer the daemon's cached answer so `ironwire update` costs no request.
    let status = match super::control_client::ControlClient::new(port) {
        Ok(client) => match client.status().await {
            Ok(status) if status.update != UpdateStatus::Unknown => status.update,
            _ => check_now(&paths, install).await,
        },
        Err(_) => check_now(&paths, install).await,
    };

    println!("ironwire {}", env!("CARGO_PKG_VERSION"));
    match status {
        UpdateStatus::UpToDate => println!("Up to date."),
        UpdateStatus::Unknown => {
            println!("Could not determine the latest release.");
            println!("Update checks may be disabled, or the network is unavailable.");
        }
        UpdateStatus::Available {
            latest,
            summary,
            upgrade_command,
        } => {
            println!("{latest} is available.");
            if let Some(summary) = summary {
                println!("  {summary}");
            }
            print_upgrade(install, upgrade_command.as_deref());
        }
        UpdateStatus::Unsupported {
            latest,
            minimum_supported,
            upgrade_command,
        } => {
            // Worth stronger words: below the floor, IronWire is likely broken
            // against current provider APIs rather than merely old.
            println!("This version is below the supported floor ({minimum_supported}).");
            println!("Providers may have changed in ways this build does not handle.");
            println!("{latest} is available.");
            print_upgrade(install, upgrade_command.as_deref());
        }
    }
    Ok(())
}

fn print_upgrade(install: InstallMethod, command: Option<&str>) {
    match command {
        Some(command) if install == InstallMethod::ShellInstaller => {
            println!("\n  {command}");
        }
        Some(command) => {
            // Deliberately the package manager's command: self-updating a
            // managed install desyncs it from its manager.
            println!("\n  {command}");
        }
        None => println!("\nThis looks like a local build — upgrade it the way you built it."),
    }
}
