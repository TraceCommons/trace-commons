//! Talking to the running daemon.
//!
//! Every inspection and every routing change goes through here rather than
//! reading state off disk, so `ironwire status` reports what the daemon is
//! *actually* doing (`docs/DESIGN.md` §6).

use anyhow::{Context, Result, bail};
use ironwire_proxy::control::{LogView, ProbeView, StatusView};

use super::{control_token, paths};

/// A client for the local control API.
pub(crate) struct ControlClient {
    base: String,
    token: String,
    client: reqwest::Client,
}

impl ControlClient {
    /// Build a client for the daemon on `port`.
    pub(crate) fn new(port: Option<u16>) -> Result<Self> {
        let paths = paths()?;
        let config = ironwire_core::config::Config::load(&paths)?;
        let port = port.unwrap_or(config.server.port);
        Ok(Self {
            base: format!("http://127.0.0.1:{port}/_ironwire"),
            token: control_token(&paths)?,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .context("building the control client")?,
        })
    }

    /// Fetch the daemon's state.
    pub(crate) async fn status(&self) -> Result<StatusView> {
        let response = self
            .client
            .get(format!("{}/status", self.base))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| not_running_inner(&e))?;
        if !response.status().is_success() {
            bail!("control API returned {}", response.status());
        }
        response.json().await.context("parsing the status response")
    }

    /// Fetch recent exchanges from the local ledger.
    pub(crate) async fn log(&self, limit: usize) -> Result<LogView> {
        let response = self
            .client
            .get(format!("{}/log?limit={limit}", self.base))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| not_running_inner(&e))?;
        if !response.status().is_success() {
            bail!("control API returned {}", response.status());
        }
        response.json().await.context("parsing the log response")
    }

    /// Hit every backend for real.
    pub(crate) async fn probe(&self) -> Result<Vec<ProbeView>> {
        let response = self
            .client
            .post(format!("{}/probe", self.base))
            .bearer_auth(&self.token)
            // A probe talks to real providers, so it needs longer than the
            // default control-plane timeout.
            .timeout(std::time::Duration::from_secs(45))
            .send()
            .await
            .map_err(|e| not_running_inner(&e))?;
        if !response.status().is_success() {
            bail!("control API returned {}", response.status());
        }
        response.json().await.context("parsing the probe response")
    }

    /// Force all traffic onto a backend, or clear the force.
    pub(crate) async fn pin(&self, backend: Option<String>, model: Option<String>) -> Result<()> {
        let response = self
            .client
            .post(format!("{}/pin", self.base))
            .bearer_auth(&self.token)
            .json(&serde_json::json!({ "backend": backend, "model": model }))
            .send()
            .await
            .map_err(|e| not_running_inner(&e))?;
        if !response.status().is_success() {
            // The daemon knows which backends exist and says so in the body.
            // Passing that through beats "control API returned 400", which
            // tells the user nothing they can act on.
            let status = response.status();
            if let Ok(body) = response.json::<serde_json::Value>().await
                && let Some(error) = body["error"].as_str()
            {
                let available = body["available"]
                    .as_array()
                    .map(|ids| {
                        ids.iter()
                            .filter_map(serde_json::Value::as_str)
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .filter(|list| !list.is_empty())
                    .unwrap_or_else(|| "none".to_string());
                bail!("{error}.\n\nConnected backends: {available}");
            }
            bail!("control API returned {status}");
        }
        Ok(())
    }
}

/// Base URL and token for a caller that needs to hold its own long-lived
/// connection — `watch` streams for hours, which the shared client's timeout
/// would cut off.
pub(crate) fn endpoint(port: Option<u16>) -> Result<(String, String)> {
    let paths = paths()?;
    let config = ironwire_core::config::Config::load(&paths)?;
    let port = port.unwrap_or(config.server.port);
    Ok((format!("http://127.0.0.1:{port}"), control_token(&paths)?))
}

/// Public form of [`not_running`], for callers outside this module.
pub(crate) fn not_running(_port: Option<u16>, error: &reqwest::Error) -> anyhow::Error {
    not_running_inner(error)
}

/// A connection refused here almost always means the daemon is not running,
/// and saying that beats surfacing a transport error the user has to decode.
fn not_running_inner(error: &reqwest::Error) -> anyhow::Error {
    if error.is_connect() {
        anyhow::anyhow!("IronWire is not running. Start it with `ironwire serve`.")
    } else {
        anyhow::anyhow!("could not reach the IronWire daemon: {error}")
    }
}
