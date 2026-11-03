//! Which NEAR AI login goes with this device's account.
//!
//! An account is bound to one NEAR AI account, and each NEAR AI login (the
//! NEAR wallet, GitHub, Google) can be a different NEAR AI account. A person
//! who enrolled with the wallet and later signs in with GitHub gets a NEAR AI
//! sign-in that works and an account that refuses it. So the daemon
//! remembers the login the account last accepted, and the browser's login
//! page lists it first (`chooser`).
//!
//! Two records, tags only -- never an account id, a subject, or a token:
//!
//! - **The stored session's login**: written with the session, under the
//!   same commit (`ceremony::persist_checked`), and bound to that sign-in by
//!   a digest of the inference key minted beside it. A rotation keeps the
//!   key, so it keeps the binding; a new sign-in replaces both. A reader
//!   names the key of the session it is about to spend, so a tag written for
//!   another sign-in, or one whose write was lost, reads as unknown rather
//!   than as the wrong login. Signing out of NEAR AI clears it.
//! - **The account's login**: written only when the account accepted a NEAR
//!   AI login -- first enrollment, a renewal with the stored login, or a
//!   sign-in checked before it was stored ([`accepted_by_commons`]).
//!
//! Unenrolling clears the account's login ([`forget_commons_login_or_remove`])
//! and keeps the stored session's, because it keeps that session. Logging out
//! removes the file ([`remove`]). Both go through this module's lock, and the
//! account's login is written only while this device is enrolled, so a write
//! in flight cannot bring a removed account's login back. Once unenrolling
//! clears the NEAR AI session too (#1843), the two cases are the same.
//!
//! It is its own file, not part of the settings document, for `spend_org`'s
//! reason: preference writers save the in-memory settings snapshot they hold,
//! and a field written beside them would be reverted by the next unrelated
//! toggle. Every function here does file I/O: call it off an async runtime.
//! Writes are best effort. A lost write leaves a tag unknown, never wrong.

use crate::config::{CONFIG_FILE, ConfigStore, DAEMON_NEAR_AI_LOGIN_FILE};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Mutex;

#[cfg(test)]
#[path = "login_record_tests.rs"]
mod tests;

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct LoginRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stored: Option<StoredLogin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    commons_login: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredLogin {
    provider: String,
    /// [`sign_in_digest`] of the inference key minted with the session.
    sign_in: String,
}

/// One writer at a time in this process. Not the credential commit lock: a
/// caller may already hold that one. It guards order, not data, so a panic
/// that poisoned it leaves nothing inconsistent: [`write_turn`] takes it
/// regardless, and the record never becomes unwritable for the rest of the
/// daemon's life.
static WRITE: Mutex<()> = Mutex::new(());

fn write_turn() -> std::sync::MutexGuard<'static, ()> {
    WRITE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The build's tag for `tag`, or `None` when this build does not offer it.
fn offered(tag: Option<&str>) -> Option<&'static str> {
    let tag = tag?;
    crate::channel::NEAR_AI_SIGN_IN_PROVIDERS
        .iter()
        .find(|offered| **offered == tag)
        .copied()
}

/// Which sign-in a session belongs to, without keeping the key id itself.
fn sign_in_digest(key_id: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"trace-commons near-ai-login sign-in v1\0");
    hash.update(key_id.as_bytes());
    format!("{:x}", hash.finalize())
}

/// What is on disk, read once.
enum OnDisk {
    Missing,
    Parsed(LoginRecord),
    /// There, but not readable as a record: an I/O error, oversized, or not
    /// one this build parses. A reader treats it as empty; a writer never
    /// treats it as already holding what it would write.
    Unreadable,
}

fn read(store: &ConfigStore) -> OnDisk {
    match store.read_daemon_file(DAEMON_NEAR_AI_LOGIN_FILE) {
        Ok(None) => OnDisk::Missing,
        Ok(Some(bytes)) if bytes.len() <= 2048 => serde_json::from_slice(&bytes)
            .map(OnDisk::Parsed)
            .unwrap_or(OnDisk::Unreadable),
        Ok(Some(_)) | Err(_) => OnDisk::Unreadable,
    }
}

fn load(store: &ConfigStore) -> LoginRecord {
    match read(store) {
        OnDisk::Parsed(record) => record,
        OnDisk::Missing | OnDisk::Unreadable => LoginRecord::default(),
    }
}

/// Change the record under [`WRITE`]. A change that leaves nothing to keep
/// never creates the file.
fn update(store: &ConfigStore, change: impl FnOnce(&mut LoginRecord)) {
    if update_checked(store, change).is_err() {
        tracing::warn!(target: "near_ai_login_record", "near_ai_login_record_unsaved");
    }
}

/// [`update`] for a caller that must know the write landed.
fn update_checked(
    store: &ConfigStore,
    change: impl FnOnce(&mut LoginRecord),
) -> anyhow::Result<()> {
    let _write = write_turn();
    write_locked(store, change)
}

fn write_locked(store: &ConfigStore, change: impl FnOnce(&mut LoginRecord)) -> anyhow::Result<()> {
    let on_disk = read(store);
    let mut record = match &on_disk {
        OnDisk::Parsed(before) => before.clone(),
        OnDisk::Missing | OnDisk::Unreadable => LoginRecord::default(),
    };
    change(&mut record);
    // No write only when the file provably already says this: a record read
    // and unchanged, or no file and nothing to keep. An unreadable file is
    // always rewritten, so a caller that must clear it learns when it cannot.
    match &on_disk {
        OnDisk::Parsed(before) if *before == record => return Ok(()),
        OnDisk::Missing if record == LoginRecord::default() => return Ok(()),
        OnDisk::Parsed(_) | OnDisk::Missing | OnDisk::Unreadable => {}
    }
    let bytes = serde_json::to_vec_pretty(&record)?;
    store.write_daemon_file(DAEMON_NEAR_AI_LOGIN_FILE, &bytes)
}

/// Whether this device is enrolled: the account's login is kept only then.
fn enrolled(store: &ConfigStore) -> bool {
    store.dir().join(CONFIG_FILE).exists()
}

/// The session just stored comes from `provider`, and was minted with the
/// inference key `key_id`.
pub(crate) fn record_stored_login(store: &ConfigStore, provider: &str, key_id: &str) {
    let tag = offered(Some(provider));
    let sign_in = sign_in_digest(key_id);
    update(store, |record| {
        record.stored = tag.map(|tag| StoredLogin {
            provider: tag.to_owned(),
            sign_in,
        });
    });
}

/// The login the stored session came from, if the session minted with
/// `key_id` is the one the record describes.
pub(crate) fn stored_login(store: &ConfigStore, key_id: &str) -> Option<&'static str> {
    let stored = load(store).stored?;
    (stored.sign_in == sign_in_digest(key_id))
        .then(|| offered(Some(&stored.provider)))
        .flatten()
}

/// The NEAR AI session is gone, and with it what is known about its login.
pub(crate) fn clear_stored_login(store: &ConfigStore) {
    update(store, |record| record.stored = None);
}

/// The account accepted a NEAR AI login made with `provider`. `None` (a
/// login whose provider is not known) changes nothing: an old tag is better
/// than a guess.
///
/// Checked under the record's lock: on a device no longer enrolled -- an
/// unenroll or a logout that landed while this was in flight -- nothing is
/// written, so the next account is never shown this one's login.
pub(crate) fn accepted_by_commons(store: &ConfigStore, provider: Option<&str>) {
    let Some(tag) = offered(provider) else {
        return;
    };
    let _write = write_turn();
    if enrolled(store)
        && write_locked(store, |record| record.commons_login = Some(tag.to_owned())).is_err()
    {
        tracing::warn!(target: "near_ai_login_record", "near_ai_login_record_unsaved");
    }
}

/// This device just enrolled in an account through a NEAR AI login made
/// with `provider`. A new account: its login replaces whatever the record
/// held, and `None` clears it, so an earlier account's login is never shown
/// as this one's. The stored session's login is kept.
pub(crate) fn enrolled_with(store: &ConfigStore, provider: Option<&str>) {
    let tag = offered(provider);
    let _write = write_turn();
    if enrolled(store)
        && write_locked(store, |record| {
            record.commons_login = tag.map(str::to_owned)
        })
        .is_err()
    {
        tracing::warn!(target: "near_ai_login_record", "near_ai_login_record_unsaved");
    }
}

/// The device is leaving its account (unenroll): no login goes with one any
/// more. The stored session's login stays with the session. An error means
/// the old account's login may still be on disk, so unenroll fails on it
/// rather than let a next account be shown that login.
pub(crate) fn forget_commons_login(store: &ConfigStore) -> anyhow::Result<()> {
    update_checked(store, |record| record.commons_login = None)
}

/// [`forget_commons_login`], or, when the record cannot be rewritten,
/// [`remove`] it: it is only a label, and losing the stored session's half
/// costs a reminder, while keeping the old account's login would show it to
/// the next account. An error means neither worked.
pub(crate) fn forget_commons_login_or_remove(store: &ConfigStore) -> anyhow::Result<()> {
    forget_or_remove_with(store, forget_commons_login)
}

fn forget_or_remove_with(
    store: &ConfigStore,
    forget: impl FnOnce(&ConfigStore) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    forget(store).or_else(|_| remove(store))
}

/// Remove the whole record (logout), under the record's lock so a write in
/// flight finishes first and cannot recreate it afterwards.
pub(crate) fn remove(store: &ConfigStore) -> anyhow::Result<()> {
    let _write = write_turn();
    store.remove_files_and_temps(&[DAEMON_NEAR_AI_LOGIN_FILE], |_| false)
}

/// The login the account last accepted, if known -- and only while this
/// device is enrolled. A leftover from an earlier account (an unenroll whose
/// clear failed, followed by a new enrollment before anything rewrote it) is
/// never shown: the next account's acceptance overwrites it, and until then
/// nothing reads it. Unenrolled, there is no account whose login to show.
pub(crate) fn commons_login(store: &ConfigStore) -> Option<&'static str> {
    if !enrolled(store) {
        return None;
    }
    offered(load(store).commons_login.as_deref())
}
