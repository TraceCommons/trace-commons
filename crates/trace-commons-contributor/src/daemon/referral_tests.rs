//! The referral IPC methods against a stub commons
//! (`crate::referral::test_support`) on 127.0.0.1. Every code and session
//! here is synthetic.

use axum::http::StatusCode;
use serde_json::{Value, json};

use super::*;
use crate::referral::test_support::{self as ingest, Server as Ingest};

const SESSION: &str = "tcn1_dGVuYW50.account";
const CODE_ID: &str = "11111111-2222-4333-8444-555555555555";

struct Fixture {
    shared: DaemonShared,
    dir: tempfile::TempDir,
    ingest: Ingest,
}

impl Fixture {
    /// Enrolled against the stub commons and signed in to the account.
    async fn new() -> Self {
        let ingest = ingest::spawn().await;
        let (dir, store) = crate::config::tests_support::temp_store();
        let shared = DaemonShared::load(store).unwrap();
        let mut cfg = crate::commands::unenrolled_preview_config();
        cfg.ingest_url = ingest.base.clone();
        shared.store.save_config(&cfg).unwrap();
        let session = crate::account_auth::AccountSession {
            access_token: SESSION.to_string(),
            expires_at: Utc::now() + chrono::TimeDelta::hours(6),
            account_id: "acct".to_string(),
        };
        shared
            .store
            .write_daemon_file(
                crate::config::ACCOUNT_SESSION_FILE,
                &serde_json::to_vec(&session).unwrap(),
            )
            .unwrap();
        Self {
            shared,
            dir,
            ingest,
        }
    }

    fn seen(&self) -> Vec<ingest::Seen> {
        self.ingest.seen.lock().unwrap().clone()
    }

    /// Every byte this daemon wrote under its state directory.
    fn files(&self) -> String {
        let mut all = String::new();
        let mut stack = vec![self.dir.path().to_path_buf()];
        while let Some(path) = stack.pop() {
            if path.is_dir() {
                for entry in std::fs::read_dir(&path).unwrap() {
                    stack.push(entry.unwrap().path());
                }
            } else if let Ok(bytes) = std::fs::read(&path) {
                all.push_str(&String::from_utf8_lossy(&bytes));
            }
        }
        all
    }

    fn audit_rows(&self) -> Vec<(String, Option<String>)> {
        audit::load(&self.shared.store)
            .unwrap()
            .into_iter()
            .map(|entry| (entry.action, entry.detail))
            .collect()
    }

    async fn call(&self, method: &str, params: Value) -> Response {
        super::super::ipc::handle_request_async(&self.shared, &req(method, params)).await
    }
}

fn req(method: &str, params: Value) -> Request {
    Request {
        id: 7,
        method: method.to_string(),
        params,
    }
}

fn ok(response: Response) -> Value {
    if let Some(error) = &response.error {
        panic!("expected success, got {}", error.message);
    }
    response.result.expect("a result")
}

fn refusal(response: Response) -> String {
    response.error.expect("a refusal").message
}

fn summary(id: &str, code: &str, state: &str, referees: u64, owed: u64) -> Value {
    json!({
        "code_id": id, "code": code, "state": state,
        "created_at": "2026-10-29T00:00:00Z", "referee_count": referees,
        "bonus_owed_nano_usd": owed,
    })
}

#[tokio::test]
async fn create_list_and_turn_off_a_code() {
    let f = Fixture::new().await;
    let created = ok(f.call("referral_codes_create", json!({})).await);
    assert_eq!(created["code"], "TC-7K3M9-QX2RD");
    assert_eq!(created["code_id"], CODE_ID);

    let listed = ok(f.call("referral_codes_list", json!({})).await);
    assert_eq!(listed["codes"].as_array().unwrap().len(), 1);
    assert_eq!(listed["codes"][0]["state"], "live");
    assert_eq!(listed["codes"][0]["code"], "TC-7K3M9-QX2RD");
    assert_eq!(listed["live_codes"], 1);
    assert_eq!(listed["can_create"], true);

    let disabled = ok(f
        .call("referral_codes_disable", json!({"code_id": CODE_ID}))
        .await);
    assert_eq!(disabled, json!({"code_id": CODE_ID, "state": "disabled"}));
    let listed = ok(f.call("referral_codes_list", json!({})).await);
    assert_eq!(listed["codes"][0]["state"], "disabled");
    assert_eq!(listed["live_codes"], 0);

    assert!(
        f.seen()
            .iter()
            .all(|s| s.auth == format!("Bearer {SESSION}"))
    );
    // Label-only audit rows, and no code anywhere on disk.
    assert_eq!(
        f.audit_rows(),
        vec![
            (AUDIT_CODE_CREATED.to_string(), None),
            (AUDIT_CODE_DISABLED.to_string(), None),
        ]
    );
    let files = f.files();
    assert!(!files.contains("7K3M9"), "a code was written to disk");
    assert!(!files.contains(CODE_ID), "a code id was written to disk");
}

#[tokio::test]
async fn the_list_totals_people_joined_and_what_waits_to_be_sent() {
    let f = Fixture::new().await;
    {
        let mut stub = f.ingest.stub.lock().unwrap();
        stub.codes = (0..5)
            .map(|n| {
                summary(
                    &format!("1111111{n}-2222-4333-8444-555555555555"),
                    &format!("TC-7K3M9-QX2R{n}"),
                    if n == 4 { "suspended" } else { "live" },
                    n,
                    n * 1_000_000,
                )
            })
            .collect();
        stub.bonus_owed = 10_000_000;
        stub.bonus_sent = 4_000_000;
    }
    let listed = ok(f.call("referral_codes_list", json!({})).await);
    assert_eq!(listed["people_joined"], 10);
    assert_eq!(listed["live_codes"], 4);
    assert_eq!(listed["can_create"], true);
    assert_eq!(listed["bonus_owed_nano_usd"], 10_000_000);
    assert_eq!(listed["bonus_sent_nano_usd"], 4_000_000);
    assert_eq!(listed["bonus_waiting_nano_usd"], 6_000_000);
    assert_eq!(listed["codes"][4]["state"], "suspended");

    f.ingest.stub.lock().unwrap().codes[4]["state"] = json!("live");
    let listed = ok(f.call("referral_codes_list", json!({})).await);
    assert_eq!(listed["live_codes"], 5);
    assert_eq!(listed["can_create"], false, "five live codes is the limit");
}

#[tokio::test]
async fn applying_sends_only_the_canonical_code_and_refuses_a_non_code_here() {
    let f = Fixture::new().await;
    let status = ok(f.call("referral_status", json!({})).await);
    assert_eq!(status["applied"], false);
    assert_eq!(status["can_apply"], true);

    for typed in ["", "not a code", "TC-ABCDE-1234U"] {
        assert_eq!(
            refusal(f.call("referral_apply", json!({"code": typed})).await),
            ERR_CODE_INVALID,
            "{typed:?}"
        );
    }
    assert_eq!(
        refusal(f.call("referral_apply", json!({"code": 12})).await),
        "code"
    );
    assert_eq!(refusal(f.call("referral_apply", json!({})).await), "code");
    assert!(
        f.seen().iter().all(|s| s.method == "GET"),
        "a non-code was sent"
    );

    let applied = ok(f
        .call("referral_apply", json!({"code": "tc-abcde-12345"}))
        .await);
    assert_eq!(applied["outcome"], "applied");
    let sent = f.seen().last().unwrap().body.clone();
    assert_eq!(
        serde_json::from_str::<Value>(&sent).unwrap(),
        json!({"code": "ABCDE12345"})
    );

    let status = ok(f.call("referral_status", json!({})).await);
    assert_eq!(status["applied"], true);
    assert_eq!(status["applied_via"], "later");
    assert_eq!(status["can_apply"], false);
    assert_eq!(f.audit_rows(), vec![(AUDIT_APPLIED.to_string(), None)]);
}

#[tokio::test]
async fn a_refused_apply_answers_the_one_outward_label() {
    let f = Fixture::new().await;
    assert_eq!(
        refusal(
            f.call("referral_apply", json!({"code": "ZZZZZ99999"}))
                .await
        ),
        "referral_not_applied"
    );
    // A second apply after one stands is refused the same way.
    ok(f.call("referral_apply", json!({"code": "ABCDE12345"}))
        .await);
    assert_eq!(
        refusal(
            f.call("referral_apply", json!({"code": "ABCDE12345"}))
                .await
        ),
        "referral_not_applied"
    );
    assert_eq!(f.audit_rows(), vec![(AUDIT_APPLIED.to_string(), None)]);
}

#[tokio::test]
async fn create_refusals_keep_the_protocol_label() {
    let f = Fixture::new().await;
    for (status, label) in [
        (StatusCode::SERVICE_UNAVAILABLE, "referral_unavailable"),
        (StatusCode::CONFLICT, "referral_code_limit_reached"),
        (StatusCode::FORBIDDEN, "referral_account_ineligible"),
    ] {
        f.ingest.stub.lock().unwrap().reply = Some((
            "/v1/account/referral-codes".into(),
            status,
            json!({"error": label}),
        ));
        assert_eq!(
            refusal(f.call("referral_codes_create", json!({})).await),
            label
        );
    }
    assert!(f.audit_rows().is_empty());
}

#[tokio::test]
async fn disable_needs_a_code_id_and_names_an_unknown_one() {
    let f = Fixture::new().await;
    for params in [
        json!({}),
        json!({"code_id": "not-a-uuid"}),
        json!({"code_id": Uuid::nil().to_string()}),
    ] {
        assert_eq!(
            refusal(f.call("referral_codes_disable", params).await),
            "code_id"
        );
    }
    assert!(f.seen().is_empty());
    assert_eq!(
        refusal(
            f.call("referral_codes_disable", json!({"code_id": CODE_ID}))
                .await
        ),
        "referral_code_not_found"
    );
}

#[tokio::test]
async fn a_rotated_session_is_kept() {
    let f = Fixture::new().await;
    f.ingest.stub.lock().unwrap().rotate_to = Some("tcn1_dGVuYW50.rotated".into());
    let answered = ok(f.call("referral_status", json!({})).await);
    assert!(
        answered.get("credential_warning").is_none(),
        "the rotation was stored: {answered}"
    );
    let stored = crate::account_auth::try_load_session_with_snapshot(&f.shared.store)
        .unwrap()
        .expect("still signed in");
    assert_eq!(stored.session.access_token, "tcn1_dGVuYW50.rotated");
    // And presented from then on.
    ok(f.call("referral_codes_list", json!({})).await);
    assert_eq!(
        f.seen().last().unwrap().auth,
        "Bearer tcn1_dGVuYW50.rotated"
    );
}

#[tokio::test]
async fn every_method_requires_an_account_session() {
    let f = Fixture::new().await;
    f.shared
        .store
        .remove_daemon_file(crate::config::ACCOUNT_SESSION_FILE)
        .unwrap();
    for (method, params) in [
        ("referral_codes_list", json!({})),
        ("referral_codes_create", json!({})),
        ("referral_codes_disable", json!({"code_id": CODE_ID})),
        ("referral_apply", json!({"code": "ABCDE12345"})),
        ("referral_status", json!({})),
    ] {
        assert!(
            super::super::ipc::METHODS.contains(&method),
            "{method} must be advertised"
        );
        let response = f.call(method, params).await;
        assert_eq!(refusal(response), ERR_ACCOUNT_SESSION_REQUIRED, "{method}");
        // The synchronous dispatcher names the async requirement.
        let sync = super::super::ipc::handle_request(&f.shared, &req(method, json!({})));
        assert_eq!(refusal(sync), "referral-requires-async", "{method}");
    }
    assert!(f.seen().is_empty());
}
