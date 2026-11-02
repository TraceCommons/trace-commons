//! Which NEAR AI login goes with this device's Trace Commons account.
//!
//! A Commons account is bound to one NEAR AI account, and each NEAR AI login
//! (the NEAR wallet, GitHub, Google) can be a different NEAR AI account. A
//! person who enrolled with the wallet and later signs in with GitHub gets a
//! NEAR AI sign-in that works and a Commons that refuses it. So the daemon
//! remembers the login the Commons last accepted, and the browser's login
//! page lists it first and says so (`chooser`).
//!
//! Two tags, nothing else -- never an account id, a subject, or a token:
//!
//! - `stored_login`: the login the stored NEAR AI session came from. Written
//!   when a sign-in's session has been stored (`ceremony::BrowserSignIn`).
//! - `commons_login`: the login the Commons last accepted. Written only when
//!   the Commons accepted a NEAR AI login: first enrollment, a renewal that
//!   used the stored login, or a sign-in the Commons checked before it was
//!   stored ([`accepted_by_commons`]). Removed by unenrolling.
//!
//! The record is its own daemon file, not a field of the settings document,
//! for `spend_org`'s reason: preference writers save the in-memory settings
//! snapshot they hold, and a field written beside them would be reverted by
//! the next unrelated toggle.
//!
//! Every write is best effort. Losing the record costs a reminder, never a
//! sign-in, so no caller fails over it. Tags are checked against the build's
//! logins on read and write; an unknown tag reads as none.

use crate::config::ConfigStore;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

/// The daemon file holding both tags.
pub const LOGIN_RECORD_FILE: &str = "near-ai-login.json";

#[cfg(test)]
#[path = "login_record_tests.rs"]
mod tests;

#[derive(Default, Serialize, Deserialize)]
struct LoginRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stored_login: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    commons_login: Option<String>,
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

fn load(store: &ConfigStore) -> LoginRecord {
    store
        .read_daemon_file(LOGIN_RECORD_FILE)
        .ok()
        .flatten()
        .filter(|bytes| bytes.len() <= 1024)
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
        .is_some_and(|bytes| store.write_daemon_file(LOGIN_RECORD_FILE, &bytes).is_ok());
    if !saved {
        tracing::warn!(target: "near_ai_login_record", "near_ai_login_record_unsaved");
    }
}

/// The stored NEAR AI session now comes from `provider`.
pub(crate) fn record_stored_login(store: &ConfigStore, provider: &str) {
    if let Some(tag) = offered(Some(provider)) {
        update(store, |record| record.stored_login = Some(tag.to_owned()));
    }
}

/// The login the stored NEAR AI session came from, if known.
pub(crate) fn stored_login(store: &ConfigStore) -> Option<&'static str> {
    offered(load(store).stored_login.as_deref())
}

/// The Commons accepted a NEAR AI login made with `provider`. `None` (a
/// login whose provider is not known, such as a session stored before this
/// record existed) changes nothing: an old tag is better than a guess.
pub(crate) fn accepted_by_commons(store: &ConfigStore, provider: Option<&str>) {
    if let Some(tag) = offered(provider) {
        update(store, |record| record.commons_login = Some(tag.to_owned()));
    }
}

/// The login the Commons last accepted, if known.
pub(crate) fn commons_login(store: &ConfigStore) -> Option<&'static str> {
    offered(load(store).commons_login.as_deref())
}

/// The device left its Commons account: no login goes with one any more.
pub(crate) fn forget_commons_login(store: &ConfigStore) {
    update(store, |record| record.commons_login = None);
}
