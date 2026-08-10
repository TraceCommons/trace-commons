//! Talk to a real daemon and print what it said.
//!
//! This is the milestone check, and it is deliberately not a test double: it
//! opens the same `Backend` the window uses, against whatever state
//! directory it is pointed at, and prints the raw JSON of `status` and
//! `list_pending` plus the typed rendering of both. If this works, the crate
//! links the contributor core, speaks `trace_commons.daemon.v1_1`, and the
//! typed layer agrees with the wire -- before a single widget exists.
//!
//! It prints `project_label`, never a path, for the same reason the window
//! does.

use anyhow::Result;
use trace_commons_contributor_gtk::backend::Backend;
use trace_commons_contributor_gtk::model::{QueueEntry, Status, human_bytes, human_when};

fn main() -> Result<()> {
    let dir = match std::env::args().nth(1) {
        Some(d) => std::path::PathBuf::from(d),
        None => trace_commons_contributor_gtk::state_dir()?,
    };
    let backend = Backend::open(dir)?;
    println!(
        "mode: {}",
        if backend.hosts_the_loop() {
            "hosting the loop in-process (nothing else held the lock)"
        } else {
            "attached to a daemon that was already running"
        }
    );

    let hello = backend.call("hello", serde_json::json!({}))?;
    println!(
        "schema: {}",
        hello
            .get("schema_version")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
    );

    let status_json = backend.call("status", serde_json::json!({}))?;
    println!(
        "\n--- status (raw) ---\n{}",
        serde_json::to_string_pretty(&status_json)?
    );
    let status: Status = serde_json::from_value(status_json)?;
    println!(
        "typed: logged_in={} paused={} queue_depth={} health={}",
        status.logged_in,
        status.paused,
        status.queue_depth,
        status.health.last_error_label.as_deref().unwrap_or("ok")
    );

    let pending_json = backend.call("list_pending", serde_json::json!({}))?;
    println!(
        "\n--- list_pending (raw) ---\n{}",
        serde_json::to_string_pretty(&pending_json)?
    );
    let entries: Vec<QueueEntry> =
        serde_json::from_value(pending_json.get("pending").cloned().unwrap_or_default())?;
    println!("typed: {} pending", entries.len());
    for e in &entries {
        println!(
            "  {} - {} - {} - {} on disk",
            e.project_label,
            e.agent_label(),
            human_when(e.discovered_at),
            human_bytes(e.size_bytes),
        );
        // Preview is what carries the redacted opening prompt and the
        // redaction receipt; the queue entry does not.
        match backend.preview(&e.entry_id) {
            Ok((summary, body)) => println!(
                "    would send {} - {} - body available: {}",
                human_bytes(summary.would_send_bytes),
                summary.scrubbed_line(),
                body.is_some()
            ),
            Err(err) => println!("    preview unavailable: {err}"),
        }
    }
    Ok(())
}
