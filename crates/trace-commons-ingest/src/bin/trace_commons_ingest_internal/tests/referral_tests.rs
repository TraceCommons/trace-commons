// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The referral routes (referral spec section 6): making, listing and
//! turning off codes, applying one later, the referee's status, and the
//! sign-up apply, on the contributor's signed-in account session.
//!
//! Real: the router and account middleware, a native session, the referral
//! store over PostgreSQL, the code seal and index. Every value is
//! synthetic.
//!
//! PostgreSQL-backed, self-skipping without
//! `TRACE_COMMONS_PG_TEST_DATABASE_URL`, and, like the rest of the suite,
//! run with `--test-threads=1`. Each account is in a tenant of its own,
//! never the shared `tenant-a`.

use super::*;
use axum::body::Body;
use tower::ServiceExt;
use trace_commons_protocol::referral::{ReferralOutcome, normalize_referral_code};
use trace_commons_server::db::ReferralStore;
use trace_commons_server::referral_config::ReferralConfig;

const CODES: &str = "/v1/account/referral-codes";
const REFERRAL: &str = "/v1/account/referral";

fn identity() -> Arc<trace_commons_server::near_account_identity::NearAccountIdentity> {
    use base64::Engine as _;
    let crypto =
        trace_commons_server::secrets::SecretsCrypto::new(SecretString::from("q".repeat(32)))
            .expect("fixture SecretsCrypto");
    let kek: Arc<dyn trace_commons_server::trace_artifact_kek::KmsKeyWrapper> = Arc::new(
        trace_commons_server::trace_artifact_kek::LocalMasterKeyWrapper::new(
            crypto,
            "referral-route-fixture",
        ),
    );
    Arc::new(
        trace_commons_server::near_account_identity::NearAccountIdentity::from_parts(
            Some(&base64::engine::general_purpose::STANDARD.encode([5u8; 32])),
            Some(kek),
        )
        .expect("fixture identity"),
    )
}

/// A program that opened a day ago, with the default settings otherwise.
fn open_program() -> ReferralConfig {
    ReferralConfig {
        program_start: Some(Utc::now() - Duration::days(1)),
        ..ReferralConfig::default()
    }
}

struct Harness {
    backend: Arc<PgBackend>,
    state: Arc<AppState>,
    _root: tempfile::TempDir,
}

struct Account {
    tenant: String,
    id: Uuid,
    token: String,
}

async fn harness_with(config: ReferralConfig) -> Option<Harness> {
    let backend = required_postgres_backend_for_ingest_test().await?;
    let root = tempfile::tempdir().unwrap();
    let mut state = test_state(root.path().to_path_buf());
    let inner = Arc::get_mut(&mut state).unwrap();
    inner.db_mirror = Some(backend.clone() as Arc<dyn Database>);
    inner.referral_store = Some(backend.clone() as Arc<dyn ReferralStore>);
    inner.near_account_identity = Some(identity());
    inner.referral_config = Arc::new(config);
    Some(Harness {
        backend,
        state,
        _root: root,
    })
}

async fn harness() -> Option<Harness> {
    harness_with(open_program()).await
}

impl Harness {
    async fn account(&self) -> Account {
        self.account_in(&format!(
            "nearai-referral-route-{}",
            Uuid::new_v4().simple()
        ))
        .await
    }

    async fn account_in(&self, tenant: &str) -> Account {
        let id = Uuid::new_v4();
        {
            let mut client = self.backend.trace_pool_for_test().get().await.unwrap();
            let tx = client.transaction().await.unwrap();
            tx.execute(
                "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
                &[&tenant],
            )
            .await
            .unwrap();
            tx.execute(
                "INSERT INTO trace_tenants (tenant_id) VALUES ($1) ON CONFLICT DO NOTHING",
                &[&tenant],
            )
            .await
            .unwrap();
            tx.execute(
                "INSERT INTO trace_accounts (tenant_id, account_id) VALUES ($1, $2)",
                &[&tenant, &id],
            )
            .await
            .unwrap();
            tx.commit().await.unwrap();
        }
        let secret = generate_session_secret();
        self.backend
            .issue_native_session(
                tenant,
                id,
                trace_commons_server::db::NewSession {
                    token_hash: &hash_secret(&secret),
                    client_kind: NATIVE_SESSION_CLIENT_KIND,
                    expires_at: Utc::now() + Duration::hours(1),
                },
                trace_commons_server::db::RedeemAudit {
                    action: "account_native_login".to_string(),
                    outcome: "success".to_string(),
                    metadata: serde_json::json!({}),
                },
            )
            .await
            .unwrap();
        Account {
            tenant: tenant.to_owned(),
            id,
            token: native_token_value(tenant, &secret),
        }
    }

    async fn call(
        &self,
        method: &str,
        uri: &str,
        bearer: Option<&str>,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        self.call_from(method, uri, bearer, body, None).await
    }

    /// [`Self::call`] with an `X-Forwarded-For` line as the edge wrote it.
    async fn call_from(
        &self,
        method: &str,
        uri: &str,
        bearer: Option<&str>,
        body: Option<serde_json::Value>,
        forwarded_for: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        let mut request = axum::http::Request::builder()
            .method(method)
            .uri(uri)
            .header(CONTENT_TYPE, "application/json");
        if let Some(forwarded_for) = forwarded_for {
            request = request.header("x-forwarded-for", forwarded_for);
        }
        if let Some(bearer) = bearer {
            request = request.header(AUTHORIZATION, format!("Bearer {bearer}"));
        }
        let body = body.map_or_else(Body::empty, |body| Body::from(body.to_string()));
        let response = app(self.state.clone())
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 256 * 1024)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    async fn create(&self, account: &Account) -> (StatusCode, serde_json::Value) {
        self.call("POST", CODES, Some(&account.token), None).await
    }

    /// Makes a code and returns `(code_id, TC-XXXXX-XXXXX)`.
    async fn code(&self, account: &Account) -> (String, String) {
        let (status, body) = self.create(account).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        (
            body["code_id"].as_str().unwrap().to_owned(),
            body["code"].as_str().unwrap().to_owned(),
        )
    }

    async fn list(&self, account: &Account) -> serde_json::Value {
        let (status, body) = self.call("GET", CODES, Some(&account.token), None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    async fn apply(&self, account: &Account, code: &str) -> (StatusCode, serde_json::Value) {
        self.call(
            "POST",
            REFERRAL,
            Some(&account.token),
            Some(serde_json::json!({ "code": code })),
        )
        .await
    }

    async fn status(&self, account: &Account) -> serde_json::Value {
        let (status, body) = self.call("GET", REFERRAL, Some(&account.token), None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    /// The precise refusal labels recorded for `account`'s tenant.
    async fn refusals(&self, account: &Account) -> Vec<String> {
        self.backend
            .raw_pool_for_tests_and_diagnostics()
            .get()
            .await
            .unwrap()
            .query(
                "SELECT label FROM trace_referral_audit
                  WHERE tenant_id = $1 AND action = 'referral_refused'
                  ORDER BY created_at, audit_id",
                &[&account.tenant],
            )
            .await
            .unwrap()
            .iter()
            .map(|row| row.get(0))
            .collect()
    }

    async fn applied_via(&self, account: &Account) -> Option<String> {
        self.backend
            .raw_pool_for_tests_and_diagnostics()
            .get()
            .await
            .unwrap()
            .query_opt(
                "SELECT applied_via FROM trace_referral_attributions
                  WHERE tenant_id = $1 AND account_id = $2",
                &[&account.tenant, &account.id],
            )
            .await
            .unwrap()
            .map(|row| row.get(0))
    }
}

#[tokio::test]
async fn a_code_is_made_listed_and_turned_off() {
    let Some(h) = harness().await else { return };
    let account = h.account().await;
    let (code_id, code) = h.code(&account).await;
    assert!(code.starts_with("TC-") && code.len() == 14, "{code}");
    assert!(normalize_referral_code(&code).is_some());

    let list = h.list(&account).await;
    assert_eq!(list["contract_version"], "referral-v1");
    assert_eq!(list["max_live_codes"], 5);
    assert_eq!(list["codes"].as_array().unwrap().len(), 1);
    assert_eq!(list["codes"][0]["code_id"], code_id.as_str());
    assert_eq!(
        list["codes"][0]["code"],
        code.as_str(),
        "read back from its seal"
    );
    assert_eq!(list["codes"][0]["state"], "live");
    assert_eq!(list["codes"][0]["referee_count"], 0);
    assert_eq!(list["bonus_owed_nano_usd"], 0);
    assert_eq!(list["bonus_sent_nano_usd"], 0);
    assert!(
        !list.to_string().contains("sha256:"),
        "no index or ref leaves: {list}"
    );

    let (status, body) = h
        .call(
            "POST",
            &format!("{CODES}/{code_id}/disable"),
            Some(&account.token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "disabled");
    // Again: the same answer, nothing changes.
    let (status, body) = h
        .call(
            "POST",
            &format!("{CODES}/{code_id}/disable"),
            Some(&account.token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "disabled");
    assert_eq!(h.list(&account).await["codes"][0]["state"], "disabled");

    for unknown in [Uuid::new_v4().to_string(), "not-a-uuid".to_owned()] {
        let (status, body) = h
            .call(
                "POST",
                &format!("{CODES}/{unknown}/disable"),
                Some(&account.token),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        assert_eq!(body["error"], "referral_code_not_found");
    }
    // Another account cannot turn it off, or see it.
    let other = h.account().await;
    let (status, _) = h
        .call(
            "POST",
            &format!("{CODES}/{code_id}/disable"),
            Some(&other.token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(h.list(&other).await["codes"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn a_sixth_live_code_is_refused() {
    let Some(h) = harness().await else { return };
    let account = h.account().await;
    for _ in 0..5 {
        h.code(&account).await;
    }
    let (status, body) = h.create(&account).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "referral_code_limit_reached");
}

#[tokio::test]
async fn a_code_applied_later_earns_its_referrer_and_tells_the_referee_only_that() {
    let Some(h) = harness().await else { return };
    let referrer = h.account().await;
    let (_, code) = h.code(&referrer).await;
    let referee = h.account().await;

    let before = h.status(&referee).await;
    assert_eq!(before["applied"], false);
    assert!(before["apply_until"].is_string(), "{before}");

    // Typed loosely: lower case, spaces, no prefix.
    let typed = code
        .trim_start_matches("TC-")
        .replace('-', " ")
        .to_lowercase();
    let (status, body) = h.apply(&referee, &typed).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["outcome"], "applied");
    let applied_at: DateTime<Utc> = serde_json::from_value(body["applied_at"].clone()).unwrap();
    let earn_until: DateTime<Utc> = serde_json::from_value(body["earn_until"].clone()).unwrap();
    let span = earn_until - applied_at;
    assert!(
        span <= Duration::days(92) && span > Duration::days(91),
        "92 days from the account's creation: {span}"
    );
    assert_eq!(h.applied_via(&referee).await.as_deref(), Some("later"));

    let after = h.status(&referee).await;
    assert_eq!(after["applied"], true);
    assert_eq!(after["applied_via"], "later");
    assert_eq!(after["earn_until"], body["earn_until"]);
    assert!(after["apply_until"].is_null());
    let shown = after.to_string();
    assert!(!shown.contains(&referrer.tenant) && !shown.contains(&referrer.id.to_string()));

    let list = h.list(&referrer).await;
    assert_eq!(list["codes"][0]["referee_count"], 1);
}

#[tokio::test]
async fn every_refusal_answers_one_label_and_records_the_precise_one() {
    let Some(h) = harness().await else { return };
    let referrer = h.account().await;
    let (_, code) = h.code(&referrer).await;
    let (_, other_code) = h.code(&h.account().await).await;
    let referee = h.account().await;

    // Not a code, an unknown code, then applied, then a second code.
    for typed in ["not a code", "TC-00000-00000"] {
        let (status, body) = h.apply(&referee, typed).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body, serde_json::json!({"error": "referral_not_applied"}));
    }
    let (status, _) = h.apply(&referee, &code).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = h.apply(&referee, &other_code).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body, serde_json::json!({"error": "referral_not_applied"}));
    // One's own code.
    let (status, body) = h.apply(&referrer, &code).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body, serde_json::json!({"error": "referral_not_applied"}));

    assert_eq!(
        h.refusals(&referee).await,
        vec![
            "referral_code_unknown",
            "referral_code_unknown",
            "referral_already_referred"
        ]
    );
    assert_eq!(h.refusals(&referrer).await, vec!["referral_self"]);

    // A body that does not parse is a request error, not a refusal.
    let (status, body) = h
        .call(
            "POST",
            REFERRAL,
            Some(&referee.token),
            Some(serde_json::json!({"code": code, "referrer": "x"})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], "invalid_referral_request");
}

#[tokio::test]
async fn before_the_program_opens_no_code_is_made_or_applied() {
    let Some(h) = harness_with(ReferralConfig::default()).await else {
        return;
    };
    let account = h.account().await;
    let (status, body) = h.create(&account).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["error"], "referral_unavailable");
    let (status, body) = h.apply(&account, "TC-7K3M9-QX2RD").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"], "referral_not_applied");
    assert_eq!(h.refusals(&account).await, vec!["referral_unavailable"]);
    let status = h.status(&account).await;
    assert_eq!(status["applied"], false);
    assert!(status["apply_until"].is_null(), "{status}");
}

#[tokio::test]
async fn without_the_store_or_the_keys_the_routes_are_unavailable() {
    let Some(h) = harness().await else { return };
    let account = h.account().await;
    let mut state = test_state(h._root.path().to_path_buf());
    let inner = Arc::get_mut(&mut state).unwrap();
    inner.db_mirror = Some(h.backend.clone() as Arc<dyn Database>);
    inner.referral_config = Arc::new(open_program());
    inner.near_account_identity = Some(identity());
    let without_store = Harness {
        backend: h.backend.clone(),
        state,
        _root: tempfile::tempdir().unwrap(),
    };
    for (method, uri) in [("POST", CODES), ("GET", CODES), ("GET", REFERRAL)] {
        let (status, body) = without_store
            .call(method, uri, Some(&account.token), None)
            .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{uri} {body}");
        assert_eq!(body["error"], "referral_unavailable");
    }

    let mut state = test_state(h._root.path().to_path_buf());
    let inner = Arc::get_mut(&mut state).unwrap();
    inner.db_mirror = Some(h.backend.clone() as Arc<dyn Database>);
    inner.referral_store = Some(h.backend.clone() as Arc<dyn ReferralStore>);
    inner.referral_config = Arc::new(open_program());
    let without_keys = Harness {
        backend: h.backend.clone(),
        state,
        _root: tempfile::tempdir().unwrap(),
    };
    let (status, body) = without_keys.create(&account).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    let (status, body) = without_keys.apply(&account, "TC-7K3M9-QX2RD").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(
        h.refusals(&account).await.is_empty(),
        "nothing was attempted"
    );
}

#[tokio::test]
async fn the_routes_need_an_account_session() {
    let Some(h) = harness().await else { return };
    for (method, uri) in [
        ("POST", CODES.to_owned()),
        ("GET", CODES.to_owned()),
        ("POST", format!("{CODES}/{}/disable", Uuid::new_v4())),
        ("POST", REFERRAL.to_owned()),
        ("GET", REFERRAL.to_owned()),
    ] {
        let (status, _) = h.call(method, &uri, None, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {uri}");
        // A device bearer is not an account session.
        let (status, _) = h.call(method, &uri, Some("token-a"), None).await;
        assert!(
            status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN,
            "{method} {uri}: {status}"
        );
    }
}

#[tokio::test]
async fn a_signup_code_is_applied_after_provisioning_and_never_fails_it() {
    let Some(h) = harness().await else { return };
    let referrer = h.account().await;
    let (_, code) = h.code(&referrer).await;

    let referee = h.account().await;
    assert_eq!(
        super::referral_routes::signup_referral(&h.state, &referee.tenant, referee.id, &code).await,
        ReferralOutcome::Applied
    );
    assert_eq!(h.applied_via(&referee).await.as_deref(), Some("signup"));

    // Junk, an over-long input, and a second code are not applied, and say
    // nothing more.
    let junk = h.account().await;
    let long = "7".repeat(200);
    for typed in ["???", long.as_str()] {
        assert_eq!(
            super::referral_routes::signup_referral(&h.state, &junk.tenant, junk.id, typed).await,
            ReferralOutcome::NotApplied
        );
    }
    assert_eq!(
        super::referral_routes::signup_referral(&h.state, &referee.tenant, referee.id, &code).await,
        ReferralOutcome::NotApplied
    );

    // Without a store the account still stands and the answer is simply
    // `not_applied`.
    let mut state = test_state(h._root.path().to_path_buf());
    Arc::get_mut(&mut state).unwrap().referral_config = Arc::new(open_program());
    let lone = h.account().await;
    assert_eq!(
        super::referral_routes::signup_referral(&state, &lone.tenant, lone.id, &code).await,
        ReferralOutcome::NotApplied
    );
    assert_eq!(h.applied_via(&lone).await, None);
}

fn mark_key() -> trace_commons_server::referral_network::ReferralNetworkMarkKey {
    trace_commons_server::referral_network::ReferralNetworkMarkKey::from_bytes(&[4u8; 32]).unwrap()
}

impl Harness {
    /// The account's network marks, read as the migrator.
    async fn marks(&self, account: &Account) -> Vec<String> {
        self.backend
            .raw_pool_for_tests_and_diagnostics()
            .get()
            .await
            .unwrap()
            .query(
                "SELECT mark FROM trace_referral_network_marks
                  WHERE tenant_id = $1 AND account_id = $2 ORDER BY mark",
                &[&account.tenant, &account.id],
            )
            .await
            .unwrap()
            .iter()
            .map(|row| row.get(0))
            .collect()
    }
}

/// Owner decision 2026-10-29: only what the server observes counts. The
/// referral routes record a keyed mark of the network the trusted edge saw
/// (the rightmost `X-Forwarded-For` hop, never one the caller wrote to its
/// left), for an account in the program only.
#[tokio::test]
async fn referral_calls_record_the_edges_network_mark_for_program_accounts_only() {
    let Some(mut h) = harness().await else { return };
    Arc::get_mut(&mut h.state)
        .unwrap()
        .referral_network_mark_key = Some(Arc::new(mark_key()));
    let edge = |address: &str| {
        mark_key()
            .mark_for_address(address.parse().unwrap())
            .unwrap()
    };
    let referrer = h.account().await;

    // Not in the program yet: listing leaves no mark.
    let (status, _) = h
        .call_from(
            "GET",
            CODES,
            Some(&referrer.token),
            None,
            Some("198.51.100.1, 203.0.113.10"),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(h.marks(&referrer).await.is_empty());

    // Making a code puts the account in the program and records the edge's
    // hop, not the caller's.
    let (status, body) = h
        .call_from(
            "POST",
            CODES,
            Some(&referrer.token),
            None,
            Some("198.51.100.1, 203.0.113.10"),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let code = body["code"].as_str().unwrap().to_owned();
    assert_eq!(h.marks(&referrer).await, vec![edge("203.0.113.10")]);
    assert_ne!(edge("203.0.113.10"), edge("198.51.100.1"));

    // The referee applies from another host on the same /24: the same mark,
    // in the referee's own tenant.
    let referee = h.account().await;
    let (status, body) = h
        .call_from(
            "POST",
            REFERRAL,
            Some(&referee.token),
            Some(serde_json::json!({ "code": code })),
            Some("203.0.113.99"),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(h.marks(&referee).await, h.marks(&referrer).await);

    // An unattributed, malformed or non-public address records nothing.
    for forwarded_for in [None, Some("garbage"), Some("10.0.0.1"), Some("127.0.0.1")] {
        let (status, _) = h
            .call_from("GET", REFERRAL, Some(&referee.token), None, forwarded_for)
            .await;
        assert_eq!(status, StatusCode::OK);
    }
    assert_eq!(h.marks(&referee).await.len(), 1);
    // A new network for the referee is a second mark.
    let (status, _) = h
        .call_from(
            "GET",
            REFERRAL,
            Some(&referee.token),
            None,
            Some("2001:db8:77::5"),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(h.marks(&referee).await.len(), 2);

    // Without the key nothing is recorded.
    let mut keyless = harness().await.unwrap();
    let other = keyless.account().await;
    let (_, _) = keyless.code(&other).await;
    Arc::get_mut(&mut keyless.state)
        .unwrap()
        .referral_network_mark_key = None;
    let (status, _) = keyless
        .call_from("GET", CODES, Some(&other.token), None, Some("203.0.113.5"))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(keyless.marks(&other).await.is_empty());
}
