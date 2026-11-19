use super::*;
use crate::daemon::ipc::Request;
use crate::managed::sessions::{
    LaunchContext, LaunchPurpose, LaunchRequest, LifecycleReport, PreparedLaunch, SessionState,
};
use crate::managed::{
    ConnectionKind, Generation, RED_TEAM_FLAG, RED_TEAM_NEEDS_NEAR_AI_LABEL,
    RED_TEAM_UNAVAILABLE_LABEL, ToolId,
};
use serde_json::{Value, json};
use uuid::Uuid;

fn test_shared(dir: &std::path::Path) -> DaemonShared {
    let store = ConfigStore::open(dir.join("state")).unwrap();
    let mut shared = DaemonShared::load(store).unwrap();
    shared.feature_flags = crate::feature_flags::Runtime::open(
        crate::feature_flags::registry().unwrap(),
        crate::feature_flags::test_signing::pin(crate::feature_flags::embedded_profile().unwrap()),
        shared.store.dir(),
        chrono::Utc::now(),
    )
    .unwrap();
    shared
}

fn set_red_team_flag(shared: &DaemonShared, enabled: bool) {
    static NEXT_REVISION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let snapshot = shared.feature_flags.snapshot();
    let now = chrono::Utc::now();
    let revision = NEXT_REVISION
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        .max(snapshot.revision.unwrap_or(0) + 1);
    let manifest = json!({
        "schema_version": 1,
        "environment": snapshot.environment,
        "revision": revision,
        "issued_at": now,
        "expires_at": now + chrono::Duration::hours(1),
        "flags": { RED_TEAM_FLAG: enabled }
    });
    shared
        .feature_flags
        .accept(
            &crate::feature_flags::test_signing::sign(&serde_json::to_vec(&manifest).unwrap()),
            now,
        )
        .unwrap();
}

fn req(method: &str, params: Value) -> Request {
    Request {
        id: 1,
        method: method.into(),
        params,
    }
}

fn red_team_launch_request(
    connection: ConnectionKind,
    account_id: AccountId,
    cwd: &std::path::Path,
) -> Value {
    json!(LaunchRequest {
        request_id: Uuid::new_v4(),
        tool: ToolId::RedTeam,
        connection,
        account_id,
        cwd: cwd.to_path_buf(),
        expected_generation: Generation(0),
        save_default: false,
        collect_traces: true,
        purpose: LaunchPurpose::Coding,
        model: None,
        background_model: None,
        keep_saved_model: false,
        cli_options: None,
        resume_of: None,
        sandbox: None,
    })
}

#[test]
fn the_snapshot_omits_red_team_while_the_flag_is_off() {
    let dir = tempfile::tempdir().unwrap();
    let shared = test_shared(dir.path());
    assert!(!red_team_available(&shared));

    let resp = handle(&shared, &req("managed_snapshot", json!({})));
    assert!(resp.error.is_none(), "{resp:?}");
    let tools = resp.result.unwrap()["tools"].as_array().unwrap().clone();
    assert!(
        !tools.iter().any(|t| t["tool"] == "red_team"),
        "tools must omit red_team while flag is off: {tools:?}"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn the_snapshot_offers_red_team_with_the_flag_on() {
    let dir = tempfile::tempdir().unwrap();
    let shared = test_shared(dir.path());
    set_red_team_flag(&shared, true);
    assert!(red_team_available(&shared));

    let resp = handle(&shared, &req("managed_snapshot", json!({})));
    assert!(resp.error.is_none(), "{resp:?}");
    let tools = resp.result.unwrap()["tools"].as_array().unwrap().clone();
    assert!(
        tools.iter().any(|t| t["tool"] == "red_team"),
        "tools must include red_team when flag is on on macOS: {tools:?}"
    );
}

#[test]
fn launch_prepare_refuses_red_team_while_the_flag_is_off() {
    let dir = tempfile::tempdir().unwrap();
    let shared = test_shared(dir.path());
    let account_id = signin::account_id(ToolId::RedTeam).unwrap();
    let resp = handle(
        &shared,
        &req(
            "managed_launch_prepare",
            red_team_launch_request(ConnectionKind::NearAiSignIn, account_id, dir.path()),
        ),
    );
    let err = resp.error.expect("flag off must refuse red_team launch");
    assert_eq!(err.code, RED_TEAM_UNAVAILABLE_LABEL);
}

#[cfg(target_os = "macos")]
#[test]
fn launch_prepare_refuses_red_team_on_a_subscription_or_api_key_account() {
    let dir = tempfile::tempdir().unwrap();
    let shared = test_shared(dir.path());
    set_red_team_flag(&shared, true);

    for connection in [ConnectionKind::Subscription, ConnectionKind::ApiKey] {
        let added = handle(
            &shared,
            &req(
                "managed_account_add",
                json!({
                    "tool": "red_team",
                    "connection": connection,
                    "label": format!("bad-{connection:?}")
                }),
            ),
        );
        assert!(added.error.is_none(), "{added:?}");
        let account: AccountView = serde_json::from_value(added.result.unwrap()).unwrap();

        let resp = handle(
            &shared,
            &req(
                "managed_launch_prepare",
                red_team_launch_request(connection, account.id, dir.path()),
            ),
        );
        let err = resp
            .error
            .expect("non-NEAR AI connection must be refused for red_team");
        assert_eq!(err.code, RED_TEAM_NEEDS_NEAR_AI_LABEL);
    }
}

#[cfg(target_os = "macos")]
#[test]
fn turning_the_flag_off_leaves_a_running_red_team_session_alone() {
    let dir = tempfile::tempdir().unwrap();
    let shared = test_shared(dir.path());
    set_red_team_flag(&shared, true);

    let added = handle(
        &shared,
        &req(
            "managed_account_add",
            json!({
                "tool": "red_team",
                "connection": "near_ai",
                "label": "NEAR AI Key"
            }),
        ),
    );
    assert!(added.error.is_none(), "{added:?}");
    let account: AccountView = serde_json::from_value(added.result.unwrap()).unwrap();

    let keyed = handle(
        &shared,
        &req(
            "managed_account_set_key",
            json!({
                "account_id": account.id,
                "key": "sk-near-red-team-test-key"
            }),
        ),
    );
    assert!(keyed.error.is_none(), "{keyed:?}");

    let prepared_resp = handle(
        &shared,
        &req(
            "managed_launch_prepare",
            red_team_launch_request(ConnectionKind::NearAi, account.id, dir.path()),
        ),
    );
    assert!(prepared_resp.error.is_none(), "{prepared_resp:?}");
    let prepared: PreparedLaunch = serde_json::from_value(prepared_resp.result.unwrap()).unwrap();

    let redeemed = handle(
        &shared,
        &req(
            "managed_launch_redeem",
            json!({
                "session_id": prepared.session_id,
                "ticket": prepared.ticket,
            }),
        ),
    );
    assert!(redeemed.error.is_none(), "{redeemed:?}");
    let context: LaunchContext = serde_json::from_value(redeemed.result.unwrap()).unwrap();

    let reported = handle(
        &shared,
        &req(
            "managed_session_report",
            json!(LifecycleReport {
                model: None,
                collection: None,
                session_id: prepared.session_id,
                proof: context.proof,
                state: SessionState::Running,
                process_id: Some(4242),
                process_identity: None,
                exit_code: None,
            }),
        ),
    );
    assert!(reported.error.is_none(), "{reported:?}");

    // Flip the flag off while the session is running.
    set_red_team_flag(&shared, false);
    assert!(!red_team_available(&shared));

    // The running session stays Running in managed_snapshot.
    let snap = handle(&shared, &req("managed_snapshot", json!({})));
    assert!(snap.error.is_none(), "{snap:?}");
    let sessions = snap.result.unwrap()["sessions"].as_array().unwrap().clone();
    let row = sessions
        .iter()
        .find(|s| s["id"] == prepared.session_id.to_string())
        .expect("running red_team session must remain listed");
    assert_eq!(row["state"], "running");

    // New launches are refused.
    let second = handle(
        &shared,
        &req(
            "managed_launch_prepare",
            red_team_launch_request(ConnectionKind::NearAi, account.id, dir.path()),
        ),
    );
    assert_eq!(second.error.unwrap().code, RED_TEAM_UNAVAILABLE_LABEL);
}
