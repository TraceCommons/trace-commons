//! `ironwire serve` — terminal presentation and process signals.
use anyhow::{Context, Result};
use ironwire_proxy::embed::{self, EmbedError};

/// Turn "address in use" into something a user can act on.
///
/// The common case by far is a second `ironwire serve`, and the second-most
/// common is an unrelated process squatting the port. Those need different
/// responses, and we can tell them apart by asking.
async fn is_ironwire(port: u16) -> bool {
    let health = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .ok();
    match health {
        Some(client) => client
            .get(format!("http://127.0.0.1:{port}/_ironwire/health"))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success()),
        None => false,
    }
}

async fn port_in_use(port: u16) -> anyhow::Error {
    if is_ironwire(port).await {
        anyhow::anyhow!(
            "IronWire is already running on port {port}.\n\
             Use it (`ironwire status`), or stop it and start again."
        )
    } else {
        anyhow::anyhow!(
            "Port {port} is in use by something that is not IronWire.\n\
             Pick another with `ironwire serve --port <n>`, and re-point your \
             clients at it with `ironwire init --port <n>`."
        )
    }
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut terminate = match signal(SignalKind::terminate()) {
            Ok(stream) => stream,
            Err(error) => {
                // Losing SIGTERM handling is not worth refusing to serve; fall
                // back to Ctrl-C only and say so.
                tracing::warn!(%error, "could not listen for SIGTERM; Ctrl-C only");
                let _ = tokio::signal::ctrl_c().await;
                tracing::info!("shutting down");
                return;
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => tracing::info!("interrupted; shutting down"),
            _ = terminate.recv() => tracing::info!("terminated; shutting down"),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("shutting down");
    }
}

/// Start the proxy using the shared assembly, retaining CLI presentation.
pub(crate) async fn run(port_override: Option<u16>) -> Result<()> {
    let paths = super::paths()?;
    let mut published = None;
    let mut proxy = match embed::start_with_policy(
        &paths.home,
        port_override,
        embed::UpdatePolicy::Standalone,
        |port, report| {
            published = announce(
                &ironwire_core::config::PathsConfig::rooted_at(report.home.clone()),
                port,
                report,
            );
        },
    )
    .await
    {
        Ok(proxy) => proxy,
        Err(EmbedError::PortInUse { port }) => return Err(port_in_use(port).await),
        Err(EmbedError::Lock { port }) => {
            if !is_ironwire(port).await {
                anyhow::bail!(
                    "another IronWire owns {} but is not answering health checks yet; wait for it to finish starting or stopping",
                    paths.home.display()
                );
            }
            let requested = port_override.or_else(|| {
                ironwire_core::config::Config::load(&paths)
                    .ok()
                    .map(|config| config.server.port)
            });
            if requested == Some(port) {
                return Err(port_in_use(port).await);
            }
            let requested = requested.map_or_else(|| "<n>".to_owned(), |port| port.to_string());
            anyhow::bail!(
                "another IronWire is already using {}.\n\nIt is listening on port {port}. Two daemons sharing one home overwrite\neach other's consent ledger, so this one will not start.\n\nUse it:            ironwire status --port {port}\nOr give this one its own home:\n\n    IRONWIRE_HOME=~/.ironwire-alt ironwire serve --port {requested}",
                paths.home.display()
            );
        }
        Err(EmbedError::Config) => {
            let config =
                ironwire_core::config::Config::load(&paths).context("loading config.toml")?;
            if config.limits.any_cap() && !config.capture.enabled {
                anyhow::bail!(
                    "[limits] sets a spend cap, but [capture] enabled = false.\n\n\
                     Spend is measured from the local trace ledger, so with capture off \
                     the cap could never fire and you would believe you were protected.\n\n\
                     Set capture.enabled = true, or remove the cap."
                );
            }
            return Err(EmbedError::Config.into());
        }
        Err(error) => return Err(error.into()),
    };
    let result = tokio::select! {
        () = shutdown_signal() => { proxy.shutdown().await; Ok(()) },
        result = proxy.wait() => result.map_err(anyhow::Error::from),
    };
    if let Some((path, endpoint)) = published
        && ironwire_core::discovery::Endpoint::read_from(&path).as_ref() == Some(&endpoint)
    {
        let _ = std::fs::remove_file(path);
    }
    result
}

fn announce(
    paths: &ironwire_core::config::PathsConfig,
    port: u16,
    report: &embed::StartupReport,
) -> Option<(std::path::PathBuf, ironwire_core::discovery::Endpoint)> {
    if report.no_backends {
        eprintln!(
            "No backends are available.\nRun `ironwire connect claude --subscription` or set ANTHROPIC_API_KEY."
        );
    }
    if let Some(error) = &report.ledger_warning {
        eprintln!(
            "Warning: could not open the trace ledger ({error}). Routing continues; `ironwire log` will be empty."
        );
    }
    if report.catalog_serial > 0 {
        println!("  provider catalog: serial {}", report.catalog_serial);
    }
    if let Some(error) = &report.bodies_warning {
        eprintln!(
            "Warning: could not open the captured-body store ({error}). Routing continues; bodies will not be recorded."
        );
    }
    let endpoint = ironwire_core::discovery::Endpoint::new(port, paths.control_token_file());
    let published = match endpoint.publish() {
        Ok(path) => {
            if path.parent() != Some(paths.home.as_path()) {
                println!("  discoverable at: {}", path.display());
            }
            Some(path)
        }
        Err(err) => {
            eprintln!("  could not publish the discovery pointer: {err}");
            None
        }
    };
    println!("IronWire listening on http://127.0.0.1:{port}");
    println!("  Point your agents at it:  ironwire init");
    println!("  Confirm they are:         ironwire doctor");
    println!();
    published.map(|path| (path, endpoint))
}
