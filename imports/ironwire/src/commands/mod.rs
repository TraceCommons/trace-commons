//! CLI subcommands.
//!
//! Every command that inspects or changes routing goes through the daemon's
//! control API rather than reading state off disk, so the CLI and the menu bar
//! app can never disagree about what is happening (`docs/DESIGN.md` §6).

pub(crate) mod connect;
pub(crate) mod control_client;
pub(crate) mod doctor;
pub(crate) mod init;
pub(crate) mod log;
pub(crate) mod logins;
pub(crate) mod pin;
pub(crate) mod privacy;
pub(crate) mod serve;
pub(crate) mod service;
pub(crate) mod status;
pub(crate) mod statusline;
pub(crate) mod update;
pub(crate) mod watch;

use anyhow::{Context, Result};
use ironwire_core::config::PathsConfig;
pub(crate) use ironwire_proxy::embed::files::control_token;
use ironwire_proxy::embed::files::restrict_permissions;

/// Resolve `$IRONWIRE_HOME`, creating it with owner-only permissions.
pub(crate) fn paths() -> Result<PathsConfig> {
    let paths = PathsConfig::resolve().context("resolving $IRONWIRE_HOME")?;
    std::fs::create_dir_all(&paths.home)
        .with_context(|| format!("creating {}", paths.home.display()))?;
    restrict_permissions(&paths.home, 0o700)?;
    Ok(paths)
}
