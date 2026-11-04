//! `status.account_state` (#1836 F1): the two cases #1837 lets a NEAR AI
//! sign-in through unchecked, reported on every status build without a
//! credential-store read.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use serde_json::{Value, json};

use super::commons_credentials::{self, Kind};
use super::credential_store::{CredentialError, CredentialReference, SecretBackend};
use super::ipc::DaemonShared;
use super::settings::NearAiInferenceCredential;
use crate::config::ACCOUNT_SESSION_FILE;

/// An address the signup allowlist accepts, and one it refuses (loopback,
/// plain HTTP).
const ACCEPTED: &str = "https://commons.example";
const REFUSED: &str = "http://127.0.0.1:9";

fn near_ai_tenant() -> String {
    format!("nearai-{}", "ab".repeat(32))
}

/// Runs on a read before it answers; `Some` makes the read fail with it.
type OnRead = Box<dyn Fn() -> Option<CredentialError> + Send + Sync>;

/// An OS store that counts reads, can lose every entry, and can stop
/// answering, as a locked keychain does.
#[derive(Default)]
struct Store {
    entries: Mutex<std::collections::HashMap<String, Vec<u8>>>,
    reads: AtomicUsize,
    locked: AtomicBool,
    on_read: Mutex<Option<OnRead>>,
}

impl SecretBackend for Store {
    fn read(&self, reference: &CredentialReference) -> Result<Vec<u8>, CredentialError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if let Some(on_read) = self.on_read.lock().unwrap().as_ref()
            && let Some(error) = on_read()
        {
            return Err(error);
        }
        if self.locked.load(Ordering::SeqCst) {
            return Err(CredentialError::Unavailable);
        }
        self.entries
            .lock()
            .unwrap()
            .get(&reference.storage_key()?)
            .cloned()
            .ok_or(CredentialError::NoEntry)
    }
    fn write(&self, reference: &CredentialReference, bytes: &[u8]) -> Result<(), CredentialError> {
        self.entries
            .lock()
            .unwrap()
            .insert(reference.storage_key()?, bytes.to_vec());
        Ok(())
    }
    fn delete(&self, reference: &CredentialReference) -> Result<(), CredentialError> {
        self.entries
            .lock()
            .unwrap()
            .remove(&reference.storage_key()?);
        Ok(())
    }
}

struct Fixture {
    shared: Arc<DaemonShared>,
    os: Arc<Store>,
    _dir: tempfile::TempDir,
}

fn session(ingest_url: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "access_token": "tcn1_bmVhcmFp.session",
        "expires_at": Utc::now() + chrono::Duration::hours(1),
        "account_id": "6f1d2c3b-4a59-4e6f-8a7b-9c0d1e2f3a4b",
        "ingest_origin": ingest_url,
    }))
    .unwrap()
}

impl Fixture {
    /// Enrolled in `tenant_id` against `ingest_url`, with an account record
    /// stored in the OS store when `record` holds.
    fn new(tenant_id: &str, ingest_url: &str, record: bool) -> Self {
        let (dir, store) = crate::config::tests_support::temp_store();
        let os = Arc::new(Store::default());
        commons_credentials::install_test_backend(&store, os.clone());
        let shared = Arc::new(DaemonShared::load(store).unwrap());
        let mut cfg = crate::commands::unenrolled_preview_config();
        cfg.ingest_url = ingest_url.to_string();
        cfg.tenant_id = tenant_id.to_string();
        cfg.device_key_id = "device-key-attention".into();
        cfg.user_subject = "device-key-attention".into();
        shared.store.save_config(&cfg).unwrap();
        let fx = Self {
            shared,
            os,
            _dir: dir,
        };
        if record {
            fx.write_record(&session(ingest_url));
        }
        fx
    }

    /// A NEAR AI tenant against an accepted address, with a record.
    fn healthy() -> Self {
        Self::new(&near_ai_tenant(), ACCEPTED, true)
    }

    fn write_record(&self, bytes: &[u8]) {
        let expected = commons_credentials::snapshot(&self.shared.store, Kind::Account).unwrap();
        commons_credentials::replace(&self.shared.store, &expected, bytes, None).unwrap();
    }

    /// What every account feature does: read the record.
    fn read_record(&self) -> Result<(), commons_credentials::AccountLoadFailure> {
        commons_credentials::load_account(&self.shared.store).map(|_| ())
    }

    /// Hold a NEAR AI inference key, as a stored NEAR AI login leaves.
    fn hold_near_ai_key(&self) {
        self.shared.settings.lock().unwrap().near_ai_inference = Some(NearAiInferenceCredential {
            key: "sk-synthetic-attention".into(),
            key_id: "key-id".into(),
            key_prefix: "sk-synthetic".into(),
            organization_id: "org".into(),
            workspace_id: "workspace".into(),
            minted_at: Utc::now(),
        });
    }

    fn state(&self) -> (Value, Value, Value) {
        let status = self.shared.status_value();
        (
            status["account_state"].clone(),
            status["account_state_reason"].clone(),
            status["near_ai_login_unconfirmed"].clone(),
        )
    }
}

fn unreadable() -> (Value, Value, Value) {
    (
        json!("needs_attention"),
        json!("account_record_unreadable"),
        json!(false),
    )
}

#[test]
fn a_healthy_account_shows_no_state() {
    let fx = Fixture::healthy();
    fx.hold_near_ai_key();
    fx.read_record().unwrap();
    assert_eq!(fx.state(), (json!("ok"), Value::Null, json!(false)));
}

/// A payload that is not JSON, once anything has read it.
#[test]
fn an_unreadable_record_needs_attention() {
    let fx = Fixture::healthy();
    fx.write_record(b"not json");
    assert_eq!(
        fx.read_record(),
        Err(commons_credentials::AccountLoadFailure::Unreadable)
    );
    assert_eq!(fx.state(), unreadable());
}

/// An OS entry that is gone is as unreadable as a malformed one: no retry
/// reads it, and the account cannot renew.
#[test]
fn a_missing_os_entry_needs_attention() {
    let fx = Fixture::healthy();
    fx.os.entries.lock().unwrap().clear();
    assert_eq!(
        fx.read_record(),
        Err(commons_credentials::AccountLoadFailure::Unreadable)
    );
    assert_eq!(fx.state(), unreadable());
}

/// Storage that does not answer (locked, a prompt) may pass: not attention.
#[test]
fn locked_storage_is_not_attention() {
    let fx = Fixture::healthy();
    fx.os.locked.store(true, Ordering::SeqCst);
    assert_eq!(
        fx.read_record(),
        Err(commons_credentials::AccountLoadFailure::Storage)
    );
    assert_eq!(fx.state().0, json!("ok"));
}

/// A status build never reads the credential store, however many times it
/// runs, and so can never wait on or raise a prompt.
#[test]
fn status_reads_no_credential_store() {
    let fx = Fixture::healthy();
    fx.write_record(b"not json");
    let before = fx.os.reads.load(Ordering::SeqCst);
    for _ in 0..5 {
        let _ = fx.shared.status_value();
    }
    assert_eq!(fx.os.reads.load(Ordering::SeqCst), before);
    // Nothing has read the record yet, so nothing is known about it.
    assert_eq!(fx.state().0, json!("ok"));
}

/// Nor does it write: a legacy, pre-record account file stays as it is,
/// where a read would move it into the OS store.
#[test]
fn status_migrates_nothing() {
    let fx = Fixture::new(&near_ai_tenant(), ACCEPTED, false);
    fx.shared
        .store
        .write_daemon_file(ACCOUNT_SESSION_FILE, &session(ACCEPTED))
        .unwrap();
    let entries = fx.os.entries.lock().unwrap().len();
    let _ = fx.shared.status_value();
    let file = fx
        .shared
        .store
        .read_daemon_file(ACCOUNT_SESSION_FILE)
        .unwrap()
        .unwrap();
    assert!(!String::from_utf8_lossy(&file).contains("commons_credential_version"));
    assert_eq!(fx.os.entries.lock().unwrap().len(), entries);
}

#[test]
fn a_refused_address_needs_attention() {
    let fx = Fixture::new(&near_ai_tenant(), REFUSED, true);
    assert_eq!(
        fx.state(),
        (
            json!("needs_attention"),
            json!("account_address_refused"),
            json!(false)
        )
    );
}

/// A NEAR AI login stored while the state holds was never checked against
/// the account, so it is flagged; the flag follows the state, not the key.
#[test]
fn a_near_ai_login_held_in_that_state_is_unconfirmed() {
    let fx = Fixture::new(&near_ai_tenant(), REFUSED, true);
    fx.hold_near_ai_key();
    assert_eq!(fx.state().2, json!(true));

    let fx = Fixture::healthy();
    fx.hold_near_ai_key();
    fx.write_record(b"not json");
    let _ = fx.read_record();
    assert_eq!(fx.state().0, json!("needs_attention"));
    assert_eq!(fx.state().2, json!(true));
}

/// #1837's "not bound" cases are not attention: signed out (no record), a
/// tenant that is not a NEAR AI one, a device that joined by code, and an
/// unenrolled device.
#[test]
fn not_bound_devices_show_no_state() {
    let signed_out = Fixture::new(&near_ai_tenant(), REFUSED, false);
    assert_eq!(signed_out.state().0, json!("ok"));

    let legacy = Fixture::new("tenant-a", REFUSED, true);
    assert_eq!(legacy.state().0, json!("ok"));

    let by_code = Fixture::new(&near_ai_tenant(), REFUSED, true);
    by_code
        .shared
        .store
        .write_daemon_file(super::device_codes::JOINED_BY_CODE_FILE, b"1\n")
        .unwrap();
    assert_eq!(by_code.state().0, json!("ok"));

    let (_dir, store) = crate::config::tests_support::temp_store();
    let unenrolled = DaemonShared::load(store).unwrap();
    let status = unenrolled.status_value();
    assert_eq!(status["account_state"], "ok");
    assert_eq!(status["account_state_reason"], Value::Null);
    assert_eq!(status["near_ai_login_unconfirmed"], false);
}

/// What a read found holds only for the record it read: a record that
/// changes is not attention until it is read again.
#[test]
fn a_changed_record_is_judged_afresh() {
    let fx = Fixture::healthy();
    fx.write_record(b"not json");
    let _ = fx.read_record();
    assert_eq!(fx.state().0, json!("needs_attention"));
    fx.write_record(&session(ACCEPTED));
    assert_eq!(fx.state().0, json!("ok"));
    fx.read_record().unwrap();
    assert_eq!(fx.state().0, json!("ok"));
}

/// The way out: unenroll works on an unreadable record, and the state goes
/// with the enrollment. The daemon never unenrolls on its own: reading the
/// state twice leaves the enrollment in place.
#[test]
fn unenrolling_clears_the_state_and_reading_it_unenrolls_nothing() {
    let fx = Fixture::healthy();
    fx.write_record(b"not json");
    let _ = fx.read_record();
    assert_eq!(fx.state().0, json!("needs_attention"));
    assert_eq!(fx.state().0, json!("needs_attention"));
    assert!(fx.shared.store.load_config().unwrap().is_some());

    let unenrolled = super::unenroll::unenroll(&fx.shared).unwrap_or_else(|_| panic!("unenroll"));
    assert!(unenrolled.removed);
    assert_eq!(fx.state(), (json!("ok"), Value::Null, json!(false)));
}

/// #1837's sign-in check and the status share one record classification:
/// an OS entry that is gone is unreadable there too, not "storage may
/// pass" forever.
#[test]
fn the_sign_in_check_reads_a_missing_entry_as_unreadable() {
    let fx = Fixture::healthy();
    fx.os.entries.lock().unwrap().clear();
    assert!(matches!(
        super::account_session_renewal::load_account_record(&fx.shared.store),
        Err(super::account_session_renewal::AccountRecordError::Malformed)
    ));
}

/// The answer outlives the daemon: after a restart, status still says what
/// the last read found, until the record changes.
#[test]
fn the_state_survives_a_restart() {
    let fx = Fixture::healthy();
    fx.write_record(b"not json");
    let _ = fx.read_record();
    commons_credentials::forget_observations_for_test(&fx.shared.store);
    assert_eq!(fx.state(), unreadable());
}

/// A settings write that rewrites the configuration but not the account it
/// names leaves the state in place.
#[test]
fn an_unrelated_settings_write_keeps_the_state() {
    let fx = Fixture::healthy();
    fx.write_record(b"not json");
    let _ = fx.read_record();
    let mut cfg = fx.shared.store.load_config().unwrap().unwrap();
    cfg.consent_scopes.push("attention-test-scope".into());
    fx.shared.store.save_config(&cfg).unwrap();
    assert_eq!(fx.state(), unreadable());
}

/// A renewal or switch that lands during the read (here: the credential
/// generation moves, and the old entry is gone) is not a record no retry
/// reads: storage, so the sign-in check stays fail-closed and nothing is
/// shown.
#[test]
fn a_record_that_moved_during_the_read_is_not_unreadable() {
    let fx = Fixture::healthy();
    let generation = fx
        .shared
        .store
        .daemon_path("commons-credential-generation.json");
    *fx.os.on_read.lock().unwrap() = Some(Box::new(move || {
        std::fs::write(&generation, br#"{"moved":true}"#).unwrap();
        Some(CredentialError::NoEntry)
    }));
    assert_eq!(
        fx.read_record(),
        Err(commons_credentials::AccountLoadFailure::Storage)
    );
    *fx.os.on_read.lock().unwrap() = None;
    assert_eq!(fx.state().0, json!("ok"));
}

/// A build that lost access to its own entry (a code-signature mismatch on
/// macOS) never regains it: unreadable, so setting up again is offered. But
/// that is this build's answer, not the files': it is not saved, so the
/// restart an update brings starts without it.
#[test]
fn an_unentitled_store_is_unreadable_for_this_build_only() {
    let fx = Fixture::healthy();
    fx.read_record().unwrap();
    *fx.os.on_read.lock().unwrap() = Some(Box::new(|| Some(CredentialError::Unentitled)));
    assert_eq!(
        fx.read_record(),
        Err(commons_credentials::AccountLoadFailure::Unreadable)
    );
    assert_eq!(fx.state(), unreadable());
    commons_credentials::forget_observations_for_test(&fx.shared.store);
    assert_eq!(fx.state().0, json!("ok"));
}

/// An answer whose file could not be written is not remembered either, so
/// the next read writes it, and it then survives a restart.
#[test]
fn a_failed_save_is_tried_again() {
    let fx = Fixture::healthy();
    fx.write_record(b"not json");
    let blocker = fx
        .shared
        .store
        .daemon_path(commons_credentials::OBSERVATION_FILE);
    std::fs::create_dir_all(&blocker).unwrap();
    let _ = fx.read_record();
    assert_eq!(fx.state().0, json!("ok"), "nothing remembered unsaved");
    std::fs::remove_dir(&blocker).unwrap();
    let _ = fx.read_record();
    commons_credentials::forget_observations_for_test(&fx.shared.store);
    assert_eq!(fx.state(), unreadable());
}
