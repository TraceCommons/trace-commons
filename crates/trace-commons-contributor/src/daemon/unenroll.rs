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
//! The Insights counter store (feed T) and its key go too, on every call,
//! enrolled or not (owner decision D4, open). The in-process Insights store
//! of files the contributor analyzed is not the daemon's and stays.
//!
//! The NEAR AI sign-in goes too, in the socket handler after the blocking
//! body: the inference key, its session and the spending-organization
//! record, with the key revoked at NEAR AI, best effort (owner decision
//! 2026-11-02, `nearai_credential::disconnect`).
//!
//! So does the change of login (F10): a ceremony in progress and a login
//! held aside for one (each held key revoked as it drops), and the last
//! NEAR AI sign-in's outcome, so `login_change_available`,
//! `login_change_held` and `login_mismatch` all read false afterwards.
//!
//! What stays: receipts, history and the audit log (the local record of what
//! this device did -- the audit log also records this call), settings, folder
//! rules, the queue, and the NEAR AI notice marker, which are not an
//! enrollment.
//!
//! What also goes: the in-app suggestion ledger, the verdict news with its
//! high-water marks, the idle-session batching set, and the attention log
//! with the last-notified stamp (`DaemonState::clear_nudges`), so a next
//! account never inherits this one's "Not now"s, stamps, news, announced
//! sessions or notification budget. The notification switches are settings
//! about this Mac and stay. Subscription-usage permissions are turned off,
//! and their held readings are discarded. The marks return to
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
//! An upload in flight is settled first (`settle_uploads`): nothing new is
//! claimed, the one in flight gets a few seconds to finish, then is stopped
//! at an await point and its entry returned to `Approved` by the upload
//! pass. Only an entry that still will not leave `Uploading` refuses the
//! call (`upload-in-flight`): those bytes may be going under the old
//! enrollment, and pulling its key out from under them helps nobody. The pass lock is held throughout, so no watcher pass or
//! identity switch interleaves, and the policy and queue locks, so the upload
//! pass cannot claim an entry between the check and the return.
//!
//! The audit row goes down first, label-only: `unenrolled`, with the count of
//! approvals returned. No tenant, account, key id or path.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

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

/// No account any more, so no login goes with one: the next account on this
/// machine is never shown this one's. Run once the config is gone, so an
/// acceptance in flight either landed before (and is cleared here) or finds
/// no enrollment and writes nothing (`login_record::accepted_by_commons`).
/// The stored NEAR AI session's login goes with the session, which the
/// NEAR AI half of `handle_unenroll` clears (`ceremony::forget_locked`
/// removes it). This never blocks the
/// unenroll: a record that cannot be rewritten is removed, and if even that
/// fails, the next unenroll call (enrolled or not) tries again.
fn forget_accounts_login(shared: &DaemonShared) {
    crate::daemon::account_login_change::forget(shared.store.dir());
    // And the last NEAR AI sign-in's outcome: one refused as another
    // account is this account's mismatch, not the next one's.
    super::nearai_credential::ceremony::forget_attempt(shared.store.dir());
    if super::nearai_credential::login_record::forget_commons_login_or_remove(&shared.store)
        .is_err()
    {
        tracing::warn!(target: "unenroll", "near_ai_login_record_not_cleared");
    }
}

/// The blocking body. Errors are `(code, label)`.
pub(crate) fn unenroll(shared: &DaemonShared) -> Result<Unenrolled, (&'static str, &'static str)> {
    // Pass lock, then policy before queue, as everywhere: the same order
    // the watcher and the upload pass's claim take them in.
    let _pass = shared
        .pass_lock
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _policy = shared.policy.lock().expect("policy lock");
    let mut queue = shared.queue.lock().expect("queue lock");
    // A witness call in flight counts too: its raw session went under the
    // enrollment being removed, and a result pinned after it would be
    // dropped anyway (`witness_worker` checks the hold before it records).
    if queue.all().iter().any(|e| e.state == QueueState::Uploading)
        || shared.witness_worker.in_flight() > 0
    {
        return Err((ERR_BUSY, ERR_UPLOAD_IN_FLIGHT));
    }
    if !holds_enrollment(&shared.store) {
        // Every call, enrolled or not, so a call that removed the
        // enrollment and then failed here is finished by the next one.
        forget_accounts_login(shared);
        clear_insights_counter(shared)?;
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
    forget_accounts_login(shared);
    let insights_cleanup = clear_insights_counter(shared);
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
    // The contribution-mission catalogue came with this enrollment: a next
    // account's `mission_fit` starts unknown. A leaf lock, taken alone.
    super::mission_matching::clear_catalogue(&shared.mission_catalogue);
    shared.account_admission.forget_enrollment();
    if insights_cleanup.is_err() {
        shared.publish(EVENT_QUEUE_CHANGED, json!({}));
    }
    insights_cleanup?;
    Ok(Unenrolled {
        removed: true,
        approvals_returned: approved.len(),
    })
}

/// Remove the Insights counter store (feed T) and forget its key: owner
/// decision D4, open, clears it on unenroll. Under the pass lock the caller
/// holds, so no watcher tick makes a counter-pass job meanwhile. The pass
/// itself runs off the tick, on its own thread; `clear` moves the count
/// every job is made under, so a pass already running, or a job waiting
/// behind it, writes nothing afterwards and keeps no key.
fn clear_insights_counter(shared: &DaemonShared) -> Result<(), (&'static str, &'static str)> {
    // The setting goes off first, so the next watcher tick does not rebuild
    // under a new key the store this clears.
    let permission_revocation = turn_off_insights_readers(shared);
    // Claude is available without an opt-in. Explicitly forget all readings
    // and pending generations rather than relying on a preference change.
    super::subscription_usage::forget_readings(shared);
    super::subscription_usage::settings_changed(shared);
    let counter_cleanup = shared
        .insights_counter
        .clear()
        .map_err(|_| (ERR_UNAVAILABLE, ERR_UNENROLL_FAILED));
    permission_revocation.and(counter_cleanup)
}

/// Off in memory always; on disk when the settings file can be read, since
/// a file that cannot be read is not written over, as in `set_settings`. A
/// revocation that cannot be persisted is reported after enrollment cleanup,
/// so the user can retry without restoring the old enrollment. Commit lock,
/// then settings, in the order `set_settings` takes them.
fn turn_off_insights_readers(shared: &DaemonShared) -> Result<(), (&'static str, &'static str)> {
    let persisted = (|| -> anyhow::Result<()> {
        let locks = super::nearai_credential::session::coordination(shared.store.dir())?;
        let _commit = locks
            .commit
            .lock()
            .map_err(|_| anyhow::anyhow!("settings commit lock"))?;
        let mut settings = shared.settings.lock().expect("settings lock");
        settings.insights_counter_pass = false;
        settings.subscription_usage_codex_enabled = false;
        settings.subscription_usage_claude_enabled = false;
        let mut persisted = super::settings::DaemonSettings::load(&shared.store)?;
        if persisted.insights_counter_pass
            || persisted.subscription_usage_codex_enabled
            || persisted.subscription_usage_claude_enabled
        {
            persisted.insights_counter_pass = false;
            persisted.subscription_usage_codex_enabled = false;
            persisted.subscription_usage_claude_enabled = false;
            persisted.save_locked(&shared.store)?;
        }
        Ok(())
    })();
    if persisted.is_err() {
        let mut settings = shared.settings.lock().expect("settings lock");
        settings.insights_counter_pass = false;
        settings.subscription_usage_codex_enabled = false;
        settings.subscription_usage_claude_enabled = false;
        // A fixed label only.
        tracing::warn!("insights readers could not be turned off on disk");
        return Err((ERR_UNAVAILABLE, ERR_UNENROLL_FAILED));
    }
    Ok(())
}

/// How long an upload in flight is given to finish on its own before it is
/// stopped.
#[cfg(not(test))]
const SETTLE_GRACE: Duration = Duration::from_secs(5);
#[cfg(test)]
const SETTLE_GRACE: Duration = Duration::from_millis(200);

/// How long a stopped upload is given to return its entry to `Approved`.
/// It is dropped at its next await point; blocking work in progress (a
/// session read, a redaction) finishes first.
#[cfg(not(test))]
const SETTLE_AFTER_INTERRUPT: Duration = Duration::from_secs(20);
#[cfg(test)]
const SETTLE_AFTER_INTERRUPT: Duration = Duration::from_secs(5);

const SETTLE_POLL: Duration = Duration::from_millis(50);

/// The diagnostics event when this call stopped an upload in flight.
pub const UPLOAD_INTERRUPTED_EVENT: &str = "unenroll_interrupted_upload";

fn upload_in_flight(shared: &DaemonShared) -> bool {
    shared.witness_worker.in_flight() > 0
        || shared
            .queue
            .lock()
            .expect("queue lock")
            .all()
            .iter()
            .any(|e| e.state == QueueState::Uploading)
}

async fn wait_for_no_upload(shared: &DaemonShared, within: Duration) -> bool {
    let started = Instant::now();
    loop {
        if !upload_in_flight(shared) {
            return true;
        }
        if started.elapsed() >= within {
            return false;
        }
        tokio::time::sleep(SETTLE_POLL).await;
    }
}

/// With uploads held (`DaemonShared::uploads_held`), let an upload in flight
/// finish for [`SETTLE_GRACE`], then stop it (`upload_interrupt`) and wait
/// up to [`SETTLE_AFTER_INTERRUPT`] for the upload pass to return its entry
/// to `Approved`. The blocking body still refuses `upload-in-flight` if one
/// is left, so this never races a live upload's write: it only stops one
/// at an await point, where nothing it began is half-written.
async fn settle_uploads(shared: &DaemonShared) {
    if wait_for_no_upload(shared, SETTLE_GRACE).await {
        return;
    }
    crate::daemon::diagnostics_log::record_labels(
        tracing::Level::WARN,
        UPLOAD_INTERRUPTED_EVENT,
        &[("reason", "unenroll")],
    );
    shared.upload_interrupt.send_modify(|count| *count += 1);
    let _ = wait_for_no_upload(shared, SETTLE_AFTER_INTERRUPT).await;
}

/// The socket entry point. Async-only: it waits on the pass lock, which a
/// full scan holds for seconds, and the credential store cleanup can block
/// on the OS keychain.
///
/// The NEAR AI sign-in goes after the enrollment, on every call, enrolled or
/// not: Private AI does not outlive the account (owner decision 2026-11-02),
/// and a device that holds only a NEAR AI sign-in is still cleared. Its key
/// is revoked at NEAR AI, best effort. A NEAR AI half that could not be
/// cleared is reported in `near_ai_cleared` rather than as a failed
/// unenroll: the enrollment is gone either way, and calling again clears
/// what is left.
///
/// It also goes when the enrollment's own removal failed
/// (`unenroll-failed`): a malformed account record fails that every time,
/// and a later step can fail after the files are gone, and neither may
/// leave Private AI signed in. Not on `upload-in-flight` or
/// `audit-write-failed`, which change nothing.
///
/// `status_changed` is published once, after both halves, so a shell that
/// rebuilds on it never reads a status that still shows the NEAR AI key.
pub(super) async fn handle_unenroll(shared: &DaemonShared, req: &Request) -> Response {
    // Nothing new goes in flight from here to the end of this call, and an
    // upload already in flight is given a moment, then stopped.
    shared.uploads_held.store(true, Ordering::SeqCst);
    // Witness jobs not yet started never start: the approvals they were for
    // are about to be returned.
    shared.witness_worker.clear_queued();
    settle_uploads(shared).await;
    let removed = super::run_blocking(|| unenroll(shared));
    shared.uploads_held.store(false, Ordering::SeqCst);
    let done = match removed {
        Ok(done) => done,
        Err((code, label)) => {
            if label == ERR_UNENROLL_FAILED {
                if let Ok(near_ai) = super::nearai_credential::disconnect(shared).await {
                    if near_ai.removed {
                        shared.publish(EVENT_STATUS_CHANGED, json!({}));
                    }
                }
            }
            return Response::err(req.id, code, label);
        }
    };
    if done.removed {
        // The OS entries the references pointed at. Journaled by the clear
        // above, so a locked keychain only defers this to the next credential
        // operation.
        let _ = super::run_blocking(|| commons_credentials::cleanup(&shared.store));
        shared.publish(EVENT_QUEUE_CHANGED, json!({}));
    }
    let (near_ai_cleared, near_ai_revoked, near_ai_removed) =
        match super::nearai_credential::disconnect(shared).await {
            Ok(near_ai) => (true, near_ai.revoked, near_ai.removed),
            Err(_) => (false, None, false),
        };
    if done.removed || near_ai_removed {
        shared.publish(EVENT_STATUS_CHANGED, json!({}));
    }
    Response::ok(
        req.id,
        json!({
            "unenrolled": true,
            "removed": done.removed,
            "approvals_returned": done.approvals_returned,
            // Whether the NEAR AI sign-in could be cleared here.
            "near_ai_cleared": near_ai_cleared,
            // Whether the NEAR AI key is gone at NEAR AI; `null` when none
            // was stored. `false` is when a shell says
            // `credential_forget_not_revoked`.
            "near_ai_revoked": near_ai_revoked,
        }),
    )
}

#[cfg(test)]
#[path = "unenroll_tests.rs"]
mod tests;
