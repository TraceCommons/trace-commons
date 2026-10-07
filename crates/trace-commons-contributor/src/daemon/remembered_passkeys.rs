//! The passkeys used on this Mac, remembered so the first run can greet a
//! returning person by the passkey's name (Ron's P-7, "Welcome back") before
//! anyone has signed in.
//!
//! The server cannot answer this: listing an account's passkeys before
//! authentication would tell anyone who asks whether an account exists. So
//! the daemon keeps its own short list, written when a passkey is created,
//! added or used to sign in here.
//!
//! What a record holds, and nothing more:
//!
//! - `name`: the passkey's display name as the person gave it when it was
//!   created or added here. A sign-in carries no name (the server's login
//!   answer has none, and the platform assertion gives the shell none), so a
//!   passkey first used here by signing in is remembered without one.
//! - `account`: the SHA-256 (hex) of the account id the passkey signed in to,
//!   used only to match a later sign-in to the same record. The raw id is
//!   never stored, and the hash never leaves this file.
//! - `last_used`: when it was last created, added or used here.
//!
//! No credential material, no token, no credential id. At most
//! [`MAX_REMEMBERED`] records, most recent first.
//!
//! Signing out does not forget them: a person who signed out and comes back
//! is exactly who P-7 is for. Removing this Mac's contributor state
//! (`ConfigStore::wipe`, the CLI's `logout`) does.
//!
//! The file is written atomically at 0600 inside the 0700 state directory,
//! like the daemon's other private files. Nothing here logs a name or an
//! account; failures are reported by fixed label only.
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::{ConfigStore, REMEMBERED_PASSKEYS_FILE};

/// How many passkeys are remembered. A Mac is rarely used with more than one
/// or two accounts; the bound keeps the list from growing with every account
/// someone tries.
pub const MAX_REMEMBERED: usize = 4;

/// The daemon's own limit on a passkey label (`native_identity::begin`).
const MAX_NAME_CHARS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RememberedPasskey {
    /// SHA-256 hex of the account id. Never the id itself.
    pub account: String,
    /// The passkey's display name, when this Mac learned one.
    pub name: Option<String>,
    pub last_used: DateTime<Utc>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RememberedFile {
    version: u32,
    passkeys: Vec<RememberedPasskey>,
}

/// What `passkey_state` reports from this list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remembered {
    /// How many passkeys this Mac remembers.
    pub count: usize,
    /// The most recently used remembered passkey's name, if it has one.
    pub latest_name: Option<String>,
}

fn account_hash(account_id: &str) -> String {
    hex::encode(Sha256::digest(account_id.as_bytes()))
}

fn valid_name(name: &str) -> bool {
    !name.trim().is_empty()
        && name.chars().count() <= MAX_NAME_CHARS
        && !name.chars().any(char::is_control)
}

fn valid_record(record: &RememberedPasskey) -> bool {
    record.account.len() == 64
        && record.account.bytes().all(|b| b.is_ascii_hexdigit())
        && record.name.as_deref().is_none_or(valid_name)
}

/// The remembered passkeys, most recent first. A missing file is an empty
/// list; an unreadable or malformed one is an error, so the caller can say
/// "unknown" rather than "none". Records that fail validation (a hand-edited
/// file) are dropped rather than shown.
pub fn load(store: &ConfigStore) -> Result<Vec<RememberedPasskey>> {
    let Some(body) = store.read_daemon_file(REMEMBERED_PASSKEYS_FILE)? else {
        return Ok(Vec::new());
    };
    let file: RememberedFile =
        serde_json::from_slice(&body).context("remembered-passkeys-malformed")?;
    let mut passkeys: Vec<RememberedPasskey> =
        file.passkeys.into_iter().filter(valid_record).collect();
    passkeys.sort_by(|a, b| b.last_used.cmp(&a.last_used));
    passkeys.truncate(MAX_REMEMBERED);
    Ok(passkeys)
}

/// The count and latest name `passkey_state` reports.
pub fn summary(store: &ConfigStore) -> Result<Remembered> {
    let passkeys = load(store)?;
    Ok(Remembered {
        count: passkeys.len(),
        latest_name: passkeys.first().and_then(|p| p.name.clone()),
    })
}

/// Remember that the passkey for `account_id` was created, added or used
/// here, at `now`. A `name` replaces the remembered one; `None` (a sign-in)
/// keeps whatever name this Mac already knew for that account. The record
/// moves to the front, and the oldest beyond [`MAX_REMEMBERED`] is dropped.
///
/// A file that cannot be read is replaced: it is this Mac's convenience
/// list, not an authority, and refusing to remember would leave a returning
/// person unrecognised forever.
pub fn remember_at(
    store: &ConfigStore,
    account_id: &str,
    name: Option<&str>,
    now: DateTime<Utc>,
) -> Result<()> {
    let account = account_hash(account_id);
    let mut passkeys = load(store).unwrap_or_default();
    let previous = passkeys
        .iter()
        .position(|p| p.account == account)
        .map(|i| passkeys.remove(i));
    let name = name
        .map(str::trim)
        .filter(|n| valid_name(n))
        .map(str::to_owned)
        .or_else(|| previous.and_then(|p| p.name));
    passkeys.insert(
        0,
        RememberedPasskey {
            account,
            name,
            last_used: now,
        },
    );
    passkeys.truncate(MAX_REMEMBERED);
    let body = serde_json::to_vec(&RememberedFile {
        version: 1,
        passkeys,
    })?;
    store.write_daemon_file(REMEMBERED_PASSKEYS_FILE, &body)
}

/// [`remember_at`] now, best effort: a passkey that was just created or
/// signed in with must not fail because this list could not be written.
pub fn remember(store: &ConfigStore, account_id: &str, name: Option<&str>) {
    if remember_at(store, account_id, name, Utc::now()).is_err() {
        tracing::warn!("remembered-passkeys-write-failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests_support::temp_store;

    fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_790_000_000 + seconds, 0).unwrap()
    }

    #[test]
    fn nothing_remembered_is_an_empty_list() {
        let (_dir, store) = temp_store();
        assert!(load(&store).unwrap().is_empty());
        assert_eq!(
            summary(&store).unwrap(),
            Remembered {
                count: 0,
                latest_name: None
            }
        );
    }

    #[test]
    fn a_created_passkey_is_remembered_by_name_and_hashed_account() {
        let (_dir, store) = temp_store();
        let account = uuid::Uuid::new_v4().to_string();
        remember_at(&store, &account, Some("Work laptop"), at(0)).unwrap();
        let passkeys = load(&store).unwrap();
        assert_eq!(passkeys.len(), 1);
        assert_eq!(passkeys[0].name.as_deref(), Some("Work laptop"));
        assert_eq!(passkeys[0].account, account_hash(&account));
        assert_eq!(passkeys[0].last_used, at(0));
        let raw = String::from_utf8(
            store
                .read_daemon_file(REMEMBERED_PASSKEYS_FILE)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert!(
            !raw.contains(&account),
            "the raw account id is never stored"
        );
    }

    #[test]
    fn a_sign_in_keeps_the_known_name_and_moves_it_to_the_front() {
        let (_dir, store) = temp_store();
        let a = uuid::Uuid::new_v4().to_string();
        let b = uuid::Uuid::new_v4().to_string();
        remember_at(&store, &a, Some("Home"), at(0)).unwrap();
        remember_at(&store, &b, Some("Work"), at(1)).unwrap();
        remember_at(&store, &a, None, at(2)).unwrap();
        let passkeys = load(&store).unwrap();
        assert_eq!(passkeys.len(), 2);
        assert_eq!(passkeys[0].name.as_deref(), Some("Home"));
        assert_eq!(passkeys[0].last_used, at(2));
        assert_eq!(
            summary(&store).unwrap(),
            Remembered {
                count: 2,
                latest_name: Some("Home".into())
            }
        );
    }

    #[test]
    fn a_passkey_first_used_here_by_signing_in_has_no_name() {
        let (_dir, store) = temp_store();
        remember_at(&store, &uuid::Uuid::new_v4().to_string(), None, at(0)).unwrap();
        assert_eq!(
            summary(&store).unwrap(),
            Remembered {
                count: 1,
                latest_name: None
            }
        );
    }

    #[test]
    fn the_list_is_bounded_and_drops_the_oldest() {
        let (_dir, store) = temp_store();
        let accounts: Vec<String> = (0..MAX_REMEMBERED + 2)
            .map(|_| uuid::Uuid::new_v4().to_string())
            .collect();
        for (i, account) in accounts.iter().enumerate() {
            remember_at(&store, account, Some(&format!("Key {i}")), at(i as i64)).unwrap();
        }
        let passkeys = load(&store).unwrap();
        assert_eq!(passkeys.len(), MAX_REMEMBERED);
        assert_eq!(
            passkeys[0].name.as_deref(),
            Some(format!("Key {}", MAX_REMEMBERED + 1).as_str())
        );
        assert!(
            !passkeys
                .iter()
                .any(|p| p.account == account_hash(&accounts[0]))
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, store) = temp_store();
        remember_at(&store, &uuid::Uuid::new_v4().to_string(), Some("K"), at(0)).unwrap();
        let mode = std::fs::metadata(store.daemon_path(REMEMBERED_PASSKEYS_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn signing_out_does_not_forget_but_wiping_this_mac_does() {
        use crate::daemon::commons_credentials::{self, Kind};
        let (_dir, store) = temp_store();
        remember_at(&store, &uuid::Uuid::new_v4().to_string(), Some("K"), at(0)).unwrap();
        commons_credentials::clear(&store, &[Kind::Account]).unwrap();
        assert_eq!(summary(&store).unwrap().count, 1, "sign-out keeps it");
        store.wipe().unwrap();
        assert_eq!(summary(&store).unwrap().count, 0, "wipe forgets it");
        assert!(!store.daemon_path(REMEMBERED_PASSKEYS_FILE).exists());
    }

    #[test]
    fn a_malformed_file_is_unknown_and_tampered_records_are_dropped() {
        let (_dir, store) = temp_store();
        store
            .write_daemon_file(REMEMBERED_PASSKEYS_FILE, b"not json")
            .unwrap();
        assert!(summary(&store).is_err());
        let good = account_hash("a");
        let body = serde_json::json!({"version":1,"passkeys":[
            {"account":"not-a-hash","name":"x","last_used":at(5)},
            {"account":good,"name":"bad\u{7}name","last_used":at(4)},
            {"account":account_hash("b"),"name":"Fine","last_used":at(3)},
        ]});
        store
            .write_daemon_file(REMEMBERED_PASSKEYS_FILE, body.to_string().as_bytes())
            .unwrap();
        assert_eq!(
            summary(&store).unwrap(),
            Remembered {
                count: 1,
                latest_name: Some("Fine".into())
            }
        );
    }

    #[test]
    fn remembering_over_a_malformed_file_replaces_it() {
        let (_dir, store) = temp_store();
        store
            .write_daemon_file(REMEMBERED_PASSKEYS_FILE, b"not json")
            .unwrap();
        remember_at(&store, "a", Some("New"), at(0)).unwrap();
        assert_eq!(summary(&store).unwrap().latest_name.as_deref(), Some("New"));
    }
}
