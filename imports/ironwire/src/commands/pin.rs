//! `ironwire pin` — force all traffic onto one backend, or clear the force.
//!
//! A pin overrides preference but never eligibility: IronWire will not serve a
//! pinned route that would corrupt the request (`docs/DESIGN.md` §3).

use anyhow::Result;

use super::control_client::ControlClient;

/// Set or clear the pin.
pub(crate) async fn run(
    port: Option<u16>,
    backend: Option<String>,
    model: Option<String>,
) -> Result<()> {
    let client = ControlClient::new(port)?;
    match &backend {
        Some(id) => {
            client.pin(backend.clone(), model.clone()).await?;
            match model {
                Some(m) => println!("Pinned to {id} ({m})."),
                None => println!("Pinned to {id}."),
            }
            println!("Clear with `ironwire pin`. The pin lives in the running");
            println!("daemon, so restarting it also clears the pin.");
        }
        None => {
            client.pin(None, None).await?;
            println!("Pin cleared; routing is back under policy.");
        }
    }
    Ok(())
}
