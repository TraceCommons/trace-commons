//! `ironwire log` — what your agents actually sent, and what it cost.
//!
//! The local ledger has to be worth having for someone who will never share
//! anything (`docs/TRUST.md` §4). This command is where that value shows up.

use anyhow::Result;

use super::control_client::ControlClient;
use crate::render;
use crate::style::Style;

/// Print recent exchanges and the last 24 hours' totals.
pub(crate) async fn run(port: Option<u16>, limit: usize, json: bool, style: Style) -> Result<()> {
    let view = ControlClient::new(port)?.log(limit).await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&view)?);
    } else {
        print!("{}", render::log(&view, style));
    }
    Ok(())
}
