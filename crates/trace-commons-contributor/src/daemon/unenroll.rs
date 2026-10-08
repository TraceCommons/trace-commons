//! `unenroll`: drop this Mac's enrollment, locally only.
//!
//! Owner decision (2026-10-07): the enrollment goes from THIS MAC ONLY. No
//! server call is made -- the device registration and the account stay as
//! they are on the server, and submitted traces stay where they are. What
//! goes is everything that lets this daemon act as that enrollment: the
//! contributor config (tenant, endpoints, device identity), the device key,
//! the account session and a staged migration key in the OS credential
//! store, and the files that belong to the enrollment
//! (`config::ENROLLMENT_FILES`, the stored approved envelopes). Afterwards
//! `status.logged_in` is false and the daemon can be enrolled again, under
//! any account, or left watching only.
//!
//! What stays: receipts, history and the audit log (the local record of what
//! this device did -- the audit log also records this call), settings, folder
//! rules, the queue, the NEAR AI notice marker, and the remembered passkeys,
//! which are not an enrollment.
//!
//! What also goes: the in-app suggestion ledger and the verdict news with its
//! high-water marks (`DaemonState::clear_nudges`), so a next account never
//! inherits this one's "Not now"s, stamps or news. The marks return to
//! unseeded, so the history cache this Mac keeps is seeded silently on the
//! next account's first poll rather than replayed as its news.
//!
//! # Order, and why it fails closed
//!
//! `uploader::enrollment_is_live` -- what `status.logged_in` reports and what
//! every send path needs -- is config present AND device key loadable. The
//! credentials go first, inside `commons_credentials::clear_with`, which also
//! advances the credential generation (so an enrollment, sign-in or switch in
//! flight cannot publish against a snapshot taken before this call); the
//! config and the other enrollment files go in its `after` step, under the
//! same commit lock.
//!
//! - A crash before the device key reference is removed leaves the
//!   enrollment whole: still enrolled, and nothing was lost.
//! - A crash after it and before the config is removed leaves a config with
//!   no device key. That reads as not enrolled (the AND above), nothing can
//!   sign an upload claim, and calling `unenroll` again finishes the job:
//!   this handler acts whenever any part of an enrollment is on disk, not only
//!   when `enrollment_is_live` holds.
//!
//! The reverse order -- config first -- also reads as not enrolled after a
//! crash, but leaves the device key and session behind on a machine that
//! says it is not enrolled: a next enrollment would reuse that device key and
//! link the two accounts to one device on the server. So credentials first.
//!
//! # Queued entries: held, never sent under another enrollment
//!
//! Every approved, unsent entry is returned to waiting
//! (`approval-inputs-changed`, the label the uploader already uses when an
//! approval's identity or endpoints move) and every preview pin is released;
//! the pinned envelope files are deleted with the enrollment. The sessions
//! stay queued, so nothing the person might still want is dropped, but no
//! approval given under the old enrollment survives to be sent under a later
//! one. Two guards already stood behind that and still do: with no config the
//! upload pass sends nothing, and an approval records an input fingerprint
//! that includes the enrollment's identity, so a re-enrollment (which makes a
//! new device key) would re-ask anyway. Returning them now makes the queue say
//! what is true instead of showing approvals that can never go.
//!
//! The consent gate's hold (`config::consent_hold`) leaves approvals in
//! place because the same person's later choice releases them. Nothing
//! releases an approval across an enrollment, so this is a return, not a hold.
//!
//! # Write in flight
//!
//! Refused while an upload is in flight (`upload-in-flight`): those bytes are
//! already going under the old enrollment, and pulling its key out from under
//! them helps nobody. The pass lock is held throughout, so no watcher pass or
//! identity switch interleaves, and the policy and queue locks, so the upload
//! pass cannot claim an entry between the check and the return.
//!
//! The audit row goes down first, label-only: `unenrolled`, with the count of
//! approvals returned. No tenant, account, key id or path.

use chrono::Utc;
use serde_json::json;

use super::audit::{self, AuditEntry};
use super::commons_credentials::{self, Kind};
use super::ipc::{
    DaemonShared, ERR_BUSY, ERR_UNAVAILABLE, EVENT_QUEUE_CHANGED, EVENT_STATUS_CHANGED, Request,
    Response,
};
use super::queue::QueueState;
use crate::config::{ACCOUNT_SESSION_FILE, ConfigStore, ENROLLMENT_FILES, STAGED_DEVICE_KEY_FILE};

/// The audit action this call records.
pub const AUDIT_ACTION: &str = "unenrolled";
/// Refused: an upload is in flight under the enrollment.
pub const ERR_UPLOAD_IN_FLIGHT: &str = "upload-in-flight";
/// Refused: the audit row could not be written, so nothing was removed.
pub const ERR_AUDIT_WRITE_FAILED: &str = "audit-write-failed";
/// The credentials or files could not be removed. Whatever was removed
/// stays removed; calling again finishes it.
pub const ERR_UNENROLL_FAILED: &str = "unenroll-failed";

/// What one call did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Unenrolled {
    /// Whether any part of an enrollment was on disk to remove.
    pub removed: bool,
    /// Approved, unsent entries returned to waiting.
    pub approvals_returned: usize,
}

/// Whether any part of an enrollment is on disk: the config, a credential
/// reference, or another enrollment file. Deliberately not
/// `enrollment_is_live`, so the half state a crash can leave is still
/// something this call removes.
fn holds_enrollment(store: &ConfigStore) -> bool {
    store.device_key_path().exists()
        || store.daemon_path(ACCOUNT_SESSION_FILE).exists()
        || store.daemon_path(STAGED_DEVICE_KEY_FILE).exists()
        || ENROLLMENT_FILES
            .iter()
            .any(|name| store.daemon_path(name).exists())
}

/// The blocking body. Errors are `(code, label)`.
pub(crate) fn unenroll(shared: &DaemonShared) -> Result<Unenrolled, (&'static str, &'static str)> {
    // Pass lock, then policy before queue, as everywhere: the same order
    // the watcher and the upload pass's claim take them in.
    let _pass = shared.pass_lock.lock().expect("pass lock");
    let _policy = shared.policy.lock().expect("policy lock");
    let mut queue = shared.queue.lock().expect("queue lock");
    if queue.all().iter().any(|e| e.state == QueueState::Uploading) {
        return Err((ERR_BUSY, ERR_UPLOAD_IN_FLIGHT));
    }
    if !holds_enrollment(&shared.store) {
        return Ok(Unenrolled {
            removed: false,
            approvals_returned: 0,
        });
    }
    let approved: Vec<uuid::Uuid> = queue
        .all()
        .iter()
        .filter(|e| e.state == QueueState::Approved)
        .map(|e| e.entry_id)
        .collect();
    // First, so a removal nobody can see never happens. A count, nothing
    // that names the enrollment.
    if audit::append(
        &shared.store,
        &AuditEntry {
            at: Utc::now(),
            action: AUDIT_ACTION.to_string(),
            project_label: None,
            detail: Some(format!("approvals_returned={}", approved.len())),
        },
    )
    .is_err()
    {
        return Err((ERR_UNAVAILABLE, ERR_AUDIT_WRITE_FAILED));
    }
    // Credentials first, then the config and the rest in the same commit
    // section: see the module doc.
    commons_credentials::clear_with(&shared.store, &Kind::ALL, || {
        shared.store.remove_enrollment_files()
    })
    .map_err(|_| (ERR_UNAVAILABLE, ERR_UNENROLL_FAILED))?;
    for id in &approved {
        queue.revoke_approval(*id, super::preview::REASON_INPUTS_CHANGED);
    }
    let pinned: Vec<uuid::Uuid> = queue
        .all()
        .iter()
        .filter(|e| e.previewed_envelope_digest.is_some())
        .map(|e| e.entry_id)
        .collect();
    for id in pinned {
        queue.release_preview_pin(id);
    }
    if queue.save(&shared.store).is_err() {
        // The in-memory queue is authoritative and the next save persists
        // it; with no config nothing is sent meanwhile.
        tracing::warn!("could not persist the queue after unenroll");
    }
    drop(queue);
    // Nudge: every suggestion stamp belongs to this enrollment, so a next
    // account starts with none of them. Taken after the queue guard is
    // released, the order `status_value` already takes state in. The
    // suggestions switch is a setting about this Mac and stays.
    {
        let mut state = shared.state.lock().expect("state lock");
        if state.clear_nudges() && state.save(&shared.store).is_err() {
            // Cleared in memory, which is what every reader consults; the
            // next tick's save persists it.
            tracing::warn!("could not persist the daemon state after unenroll");
        }
    }
    shared.account_admission.forget_enrollment();
    if let Ok(mut ceremonies) = shared.native_identity.lock() {
        ceremonies.clear();
    }
    Ok(Unenrolled {
        removed: true,
        approvals_returned: approved.len(),
    })
}

/// The socket entry point. Async-only: it waits on the pass lock, which a
/// full scan holds for seconds, and the credential store cleanup can block
/// on the OS keychain.
pub(super) async fn handle_unenroll(shared: &DaemonShared, req: &Request) -> Response {
    match super::run_blocking(|| unenroll(shared)) {
        Ok(done) => {
            if done.removed {
                // The OS entries the references pointed at. Journaled by the
                // clear above, so a locked keychain only defers this to the
                // next credential operation.
                let _ = super::run_blocking(|| commons_credentials::cleanup(&shared.store));
                shared.publish(EVENT_QUEUE_CHANGED, json!({}));
                shared.publish(EVENT_STATUS_CHANGED, json!({}));
            }
            Response::ok(
                req.id,
                json!({
                    "unenrolled": true,
                    "removed": done.removed,
                    "approvals_returned": done.approvals_returned,
                }),
            )
        }
        Err((code, label)) => Response::err(req.id, code, label),
    }
}

#[cfg(test)]
#[path = "unenroll_tests.rs"]
mod tests;
