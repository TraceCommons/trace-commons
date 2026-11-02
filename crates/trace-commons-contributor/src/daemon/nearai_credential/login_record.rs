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
//! The file is one of the enrollment's (`config::ENROLLMENT_FILES`), so
//! unenrolling and logging out remove both.
//!
//! It is its own file, not part of the settings document, for `spend_org`'s
//! reason: preference writers save the in-memory settings snapshot they hold,
//! and a field written beside them would be reverted by the next unrelated
//! toggle. Every function here does file I/O: call it off an async runtime.
//! Writes are best effort. A lost write leaves a tag unknown, never wrong.

use crate::config::{ConfigStore, DAEMON_NEAR_AI_LOGIN_FILE};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Mutex;

#[cfg(test)]
#[path = "login_record_tests.rs"]
mod tests;

#[derive(Default, Serialize, Deserialize)]
struct LoginRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stored: Option<StoredLogin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    commons_login: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct StoredLogin {
    provider: String,
    /// [`sign_in_digest`] of the inference key minted with the session.
    sign_in: String,
}

/// One writer at a time in this process. Not the credential commit lock: a
/// caller may already hold that one.
static WRITE: Mutex<()> = Mutex::new(());

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

fn load(store: &ConfigStore) -> LoginRecord {
    store
        .read_daemon_file(DAEMON_NEAR_AI_LOGIN_FILE)
        .ok()
        .flatten()
        .filter(|bytes| bytes.len() <= 2048)
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn update(store: &ConfigStore, change: impl FnOnce(&mut LoginRecord)) {
    let Ok(_write) = WRITE.lock() else {
        return;
    };
    let mut record = load(store);
    change(&mut record);
    let saved = serde_json::to_vec_pretty(&record)
        .ok()
        .is_some_and(|bytes| {
            store
                .write_daemon_file(DAEMON_NEAR_AI_LOGIN_FILE, &bytes)
                .is_ok()
        });
    if !saved {
        tracing::warn!(target: "near_ai_login_record", "near_ai_login_record_unsaved");
    }
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
pub(crate) fn accepted_by_commons(store: &ConfigStore, provider: Option<&str>) {
    if let Some(tag) = offered(provider) {
        update(store, |record| record.commons_login = Some(tag.to_owned()));
    }
}

/// The login the account last accepted, if known.
pub(crate) fn commons_login(store: &ConfigStore) -> Option<&'static str> {
    offered(load(store).commons_login.as_deref())
}
