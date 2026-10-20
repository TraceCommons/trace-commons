//! `ironwire watch` — see routing decisions as they happen.
//!
//! The answer to the question `docs/CRITIQUE.md` left open: how does a user
//! find out that their model family just changed?
//!
//! Not in-band. IronWire's only writable channel into a coding agent is the
//! response stream, and putting a line there would put words in the model's
//! mouth and corrupt the transcript the agent stores and replays. So the
//! channel is a second terminal, and the cost of that honesty is that the user
//! has to look. `--only-changes` exists so that looking is cheap: on a healthy
//! system it prints nothing for hours, and the one line it does print is the
//! one that matters.

use anyhow::{Context, Result};
use futures_util::StreamExt;
use ironwire_proxy::events::{Event, line};

use super::control_client;

/// Run `ironwire watch`.
pub(crate) async fn run(port: Option<u16>, only_changes: bool) -> Result<()> {
    let (base, token) = control_client::endpoint(port)?;
    let client = reqwest::Client::builder()
        // No total timeout: this stream is meant to stay open all day.
        .connect_timeout(std::time::Duration::from_secs(5))
        .build()
        .context("building the HTTP client")?;

    let response = client
        .get(format!("{base}/_ironwire/events"))
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| control_client::not_running(port, &e))?;

    if !response.status().is_success() {
        anyhow::bail!(
            "the daemon refused the event stream ({}). Is this the right \
             $IRONWIRE_HOME?",
            response.status()
        );
    }

    if only_changes {
        println!("Watching for family changes and failures. Ctrl-C to stop.");
    } else {
        println!("Watching routing. Ctrl-C to stop.");
    }

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("reading the event stream")?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        // SSE frames end at a blank line, and a frame can arrive split across
        // any number of chunks.
        while let Some(end) = buffer.find("\n\n") {
            let frame = buffer[..end].to_string();
            buffer.drain(..end + 2);
            for event_line in frame.lines() {
                if let Some(payload) = event_line.strip_prefix("data: ") {
                    match serde_json::from_str::<Event>(payload) {
                        Ok(event) => {
                            if !only_changes || event.is_user_visible() {
                                println!("{}", line(&event));
                            }
                        }
                        // A frame we cannot parse is a version skew between the
                        // CLI and the daemon, not a reason to exit.
                        Err(error) => {
                            tracing::debug!(%error, "unparsed event frame");
                        }
                    }
                } else if let Some(note) = event_line.strip_prefix(": lagged ") {
                    // Stated, not hidden: the bus drops rather than blocking
                    // the datapath, so a gap is possible and the user is told.
                    println!("… missed {note} events while catching up");
                }
            }
        }
    }

    println!("The daemon closed the stream.");
    Ok(())
}
