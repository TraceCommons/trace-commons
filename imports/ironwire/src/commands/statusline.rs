//! `ironwire statusline` — one line of IronWire state, inside the agent's own UI.
//!
//! The line itself is rendered by [`ironwire_proxy::statusline`], so that a
//! host embedding the proxy prints exactly what this command prints; see that
//! module for what the line says and the three properties it keeps. This
//! command only finds the daemon the CLI would talk to and prints the result.

use anyhow::Result;
use ironwire_proxy::statusline;

use super::{control_token, paths};

/// Render the line.
pub(crate) async fn run(port: Option<u16>) -> Result<()> {
    // Whatever we print, we exit 0: a status line that reports its own failure
    // into someone's editor has made their problem worse, not better.
    if let Some(line) = line(port).await {
        println!("{line}");
    }
    Ok(())
}

async fn line(port: Option<u16>) -> Option<String> {
    let client_model = statusline::client_model_from_stdin();
    // The same daemon every other subcommand reaches: `--port`, or the port in
    // `$IRONWIRE_HOME/config.toml`.
    let paths = paths().ok()?;
    let config = ironwire_core::config::Config::load(&paths).ok()?;
    let port = port.unwrap_or(config.server.port);
    let token = control_token(&paths).ok()?;
    statusline::line(port, &token, client_model.as_deref()).await
}
