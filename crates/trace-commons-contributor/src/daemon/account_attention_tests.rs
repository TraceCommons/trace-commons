//! `status.account_state` (#1836 F1): the two cases #1837 lets a NEAR AI
//! sign-in through unchecked, reported on every status build.

use std::sync::Arc;

use chrono::Utc;
use serde_json::{Value, json};

use super::commons_credentials::{self, Kind};
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

struct Fixture {
    shared: Arc<DaemonShared>,
    _dir: tempfile::TempDir,
}

impl Fixture {
    /// Enrolled in `tenant_id` against `ingest_url`, with an account record
    /// stored when `record` holds.
    fn new(tenant_id: &str, ingest_url: &str, record: bool) -> Self {
        let (dir, store) = crate::config::tests_support::temp_store();
        let shared = Arc::new(DaemonShared::load(store).unwrap());
        let mut cfg = crate::commands::unenrolled_preview_config();
        cfg.ingest_url = ingest_url.to_string();
        cfg.tenant_id = tenant_id.to_string();
        cfg.device_key_id = "device-key-attention".into();
        cfg.user_subject = "device-key-attention".into();
        shared.store.save_config(&cfg).unwrap();
        if record {
            let session = json!({
                "access_token": "tcn1_bmVhcmFp.session",
                "expires_at": Utc::now() + chrono::Duration::hours(1),
                "account_id": "6f1d2c3b-4a59-4e6f-8a7b-9c0d1e2f3a4b",
                "ingest_origin": ingest_url,
            });
            shared
                .store
                .write_daemon_file(ACCOUNT_SESSION_FILE, &serde_json::to_vec(&session).unwrap())
                .unwrap();
        }
        Self { shared, _dir: dir }
    }

    /// A NEAR AI tenant against an accepted address, with a record.
    fn healthy() -> Self {
        Self::new(&near_ai_tenant(), ACCEPTED, true)
    }

    /// Replace the stored record with bytes that are not JSON.
    fn break_record(&self) {
        let expected = commons_credentials::snapshot(&self.shared.store, Kind::Account).unwrap();
        commons_credentials::replace(&self.shared.store, &expected, b"not json", None).unwrap();
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

#[test]
fn a_healthy_account_shows_no_state() {
    let fx = Fixture::healthy();
    fx.hold_near_ai_key();
    assert_eq!(fx.state(), (json!("ok"), Value::Null, json!(false)));
}

#[test]
fn an_unreadable_record_needs_attention() {
    let fx = Fixture::healthy();
    fx.break_record();
    assert_eq!(
        fx.state(),
        (
            json!("needs_attention"),
            json!("account_record_unreadable"),
            json!(false)
        )
    );
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
    fx.break_record();
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

/// The answer is remembered per stored record, not forever: a record that
/// changes is read again.
#[test]
fn a_changed_record_is_read_again() {
    let fx = Fixture::healthy();
    assert_eq!(fx.state().0, json!("ok"));
    fx.break_record();
    assert_eq!(fx.state().0, json!("needs_attention"));
    let expected = commons_credentials::snapshot(&fx.shared.store, Kind::Account).unwrap();
    let good = json!({
        "access_token": "tcn1_bmVhcmFp.session",
        "expires_at": Utc::now() + chrono::Duration::hours(1),
        "account_id": "6f1d2c3b-4a59-4e6f-8a7b-9c0d1e2f3a4b",
        "ingest_origin": ACCEPTED,
    });
    commons_credentials::replace(
        &fx.shared.store,
        &expected,
        &serde_json::to_vec(&good).unwrap(),
        None,
    )
    .unwrap();
    assert_eq!(fx.state().0, json!("ok"));
}

/// The way out: unenroll works on an unreadable record, and the state goes
/// with the enrollment. The daemon never unenrolls on its own: reading the
/// state twice leaves the enrollment in place.
#[test]
fn unenrolling_clears_the_state_and_reading_it_unenrolls_nothing() {
    let fx = Fixture::healthy();
    fx.break_record();
    assert_eq!(fx.state().0, json!("needs_attention"));
    assert_eq!(fx.state().0, json!("needs_attention"));
    assert!(fx.shared.store.load_config().unwrap().is_some());

    let unenrolled = super::unenroll::unenroll(&fx.shared).unwrap_or_else(|_| panic!("unenroll"));
    assert!(unenrolled.removed);
    assert_eq!(fx.state(), (json!("ok"), Value::Null, json!(false)));
}
