//! `ironwire status` — what capacity you have, and how sure we are.

use anyhow::Result;

use super::control_client::ControlClient;
use crate::render;
use crate::style::Style;

/// Print the daemon's view of every backend.
pub(crate) async fn run(port: Option<u16>, json: bool, style: Style) -> Result<()> {
    let status = ControlClient::new(port)?.status().await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&status)?);
    } else {
        print!("{}", render::status(&status, style));
    }
    Ok(())
}
