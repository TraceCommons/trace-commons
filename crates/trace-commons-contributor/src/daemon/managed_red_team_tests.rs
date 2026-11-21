// The tests here are macOS-only; elsewhere their shared imports and helpers
// are unused, which -D warnings turns into a build failure (Linux and Windows CI).
#![cfg_attr(not(target_os = "macos"), allow(unused_imports, dead_code))]
use super::*;
use crate::daemon::ipc::Request;
#[cfg(not(target_os = "macos"))]
use crate::managed::RED_TEAM_UNAVAILABLE_LABEL;
use crate::managed::sessions::{LaunchPurpose, LaunchRequest};
use crate::managed::{ConnectionKind, Generation, RED_TEAM_NEEDS_NEAR_AI_LABEL, ToolId};
use serde_json::{Value, json};
use uuid::Uuid;

fn test_shared(dir: &std::path::Path) -> DaemonShared {
    let store = ConfigStore::open(dir.join("state")).unwrap();
    let mut shared = DaemonShared::load(store).unwrap();
    shared.feature_flags = crate::feature_flags::Runtime::open(
        crate::feature_flags::registry().unwrap(),
        crate::feature_flags::test_signing::testing_profile(),
        shared.store.dir(),
        chrono::Utc::now(),
    )
    .unwrap();
    shared
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
        initial_prompt: None,
    })
}

#[cfg(target_os = "macos")]
#[test]
fn the_snapshot_offers_red_team_on_macos_without_any_flag() {
    let dir = tempfile::tempdir().unwrap();
    let shared = test_shared(dir.path());
    assert!(red_team_available(&shared));

    let resp = handle(&shared, &req("managed_snapshot", json!({})));
    assert!(resp.error.is_none(), "{resp:?}");
    let tools = resp.result.unwrap()["tools"].as_array().unwrap().clone();
    assert!(
        tools.iter().any(|t| t["tool"] == "red_team"),
        "tools must include red_team on macOS: {tools:?}"
    );
}

#[cfg(not(target_os = "macos"))]
#[test]
fn the_snapshot_omits_red_team_off_macos() {
    let dir = tempfile::tempdir().unwrap();
    let shared = test_shared(dir.path());
    assert!(!red_team_available(&shared));

    let resp = handle(&shared, &req("managed_snapshot", json!({})));
    assert!(resp.error.is_none(), "{resp:?}");
    let tools = resp.result.unwrap()["tools"].as_array().unwrap().clone();
    assert!(
        !tools.iter().any(|t| t["tool"] == "red_team"),
        "tools must omit red_team off macOS: {tools:?}"
    );
}

#[cfg(not(target_os = "macos"))]
#[test]
fn launch_prepare_refuses_red_team_off_macos() {
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
    let err = resp.error.expect("off macOS must refuse red_team launch");
    assert_eq!(err.code, RED_TEAM_UNAVAILABLE_LABEL);
}

#[cfg(target_os = "macos")]
#[test]
fn launch_prepare_refuses_red_team_on_a_subscription_or_api_key_account() {
    let dir = tempfile::tempdir().unwrap();
    let shared = test_shared(dir.path());

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
