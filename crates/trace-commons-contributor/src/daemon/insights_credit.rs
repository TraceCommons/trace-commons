//! `insights_credit`: where NEAR AI credit went, by task, tool and folder,
//! over the counter pass (feed T). Product owner, 2026-10-28.
//!
//! Gated exactly as `insights_model_answers` is: off answers
//! `enabled: false` without opening the store or the key; a store or key
//! that cannot be read answers `readable: false` with its fixed reason.
//! Sessions in a Never folder enter no total.
//!
//! # What crosses the socket
//!
//! Integer USD micros that NEAR AI charged (from the route tallies, never a
//! list price), session counts, the fixed task and tool labels, and the
//! folder labels `insights_sessions` already shows. No session reference,
//! digest or path.

use super::insights_sessions::{folder_labels, gate};
use super::ipc::{DaemonShared, Request, Response};

/// `insights_credit`.
pub fn handle_credit(shared: &DaemonShared, req: &Request) -> Response {
    let (enabled, never) = match gate(shared) {
        Ok(gate) => gate,
        Err(answer) => return Response::ok(req.id, answer),
    };
    // A poisoned lock reads as off: fail closed, never a guess.
    let ledger_feed = shared
        .settings
        .lock()
        .map(|settings| settings.insights_ledger_feed)
        .unwrap_or(false);
    let ledger_present = ledger_feed && shared.routing_ledger().is_some();
    let folders = if enabled {
        folder_labels(shared)
    } else {
        Vec::new()
    };
    Response::ok(
        req.id,
        shared.insights_counter.credit_value(
            enabled,
            &never,
            &folders,
            ledger_feed,
            ledger_present,
            chrono::Utc::now(),
        ),
    )
}
