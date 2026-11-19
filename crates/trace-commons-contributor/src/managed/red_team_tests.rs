use super::*;
use crate::config::ConfigStore;
use crate::daemon::ipc::{DaemonShared, Request};
use crate::daemon::managed::handle;
use crate::managed::sandbox::SandboxSettings;
use crate::managed::secrets::RedactedSecret;
use crate::managed::sessions::{
    CollectionState, LaunchContext, LaunchPurpose, LaunchRequest, PreparedLaunch, SessionState,
    SessionView,
};
use crate::managed::{AccountId, AccountView, ConnectionKind, Generation, RED_TEAM_FLAG, ToolId};
use serde_json::json;
use uuid::Uuid;

fn test_context(cwd: &Path, inputs: Vec<PathBuf>, key: &str) -> LaunchContext {
    let id = Uuid::new_v4();
    LaunchContext {
        cli_options: vec![],
        session: SessionView {
            model: None,
            background_model: None,
            collect_traces: true,
            collection: CollectionState::Pending,
            purpose: LaunchPurpose::Coding,
            id,
            tool: ToolId::RedTeam,
            connection: ConnectionKind::NearAiSignIn,
            account_id: AccountId(Uuid::new_v4()),
            account_label: "NEAR AI".into(),
            selection_generation: Generation(0),
            project_label: "proj".into(),
            cwd: cwd.to_path_buf(),
            started_at: chrono::Utc::now(),
            state: SessionState::Starting,
            exit_code: None,
            can_focus: false,
            can_stop: false,
            can_start_again: false,
            can_resume: false,
            can_dismiss: false,
            resumed_from: None,
        },
        proof: "proof".into(),
        route_port: None,
        route_model: None,
        route_models: None,
        route_native_responses: None,
        resume_of: None,
        sandbox: Some(SandboxSettings {
            inputs,
            allow_hosts: vec![],
            packages: false,
            cpus: None,
            memory_mib: None,
        }),
        red_team_key: Some(RedactedSecret::new(key.to_owned())),
    }
}

#[cfg(target_os = "macos")]
#[test]
fn redeem_returns_the_key_only_for_red_team() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let mut shared = DaemonShared::load(store).unwrap();
    shared.feature_flags = crate::feature_flags::Runtime::open(
        crate::feature_flags::registry().unwrap(),
        crate::feature_flags::test_signing::pin(crate::feature_flags::embedded_profile().unwrap()),
        shared.store.dir(),
        chrono::Utc::now(),
    )
    .unwrap();
    let now = chrono::Utc::now();
    let manifest = json!({
        "schema_version": 1,
        "environment": shared.feature_flags.snapshot().environment,
        "revision": 1001,
        "issued_at": now,
        "expires_at": now + chrono::Duration::hours(1),
        "flags": { RED_TEAM_FLAG: true }
    });
    shared
        .feature_flags
        .accept(
            &crate::feature_flags::test_signing::sign(&serde_json::to_vec(&manifest).unwrap()),
            now,
        )
        .unwrap();

    // Claude saved NEAR AI account -> redeem must NOT return red_team_key.
    let claude_acct: AccountView = serde_json::from_value(
        handle(
            &shared,
            &Request {
                id: 1,
                method: "managed_account_add".into(),
                params: json!({"tool": "claude", "connection": "near_ai", "label": "Claude Key"}),
            },
        )
        .result
        .unwrap(),
    )
    .unwrap();
    let _ = handle(
        &shared,
        &Request {
            id: 2,
            method: "managed_account_set_key".into(),
            params: json!({"account_id": claude_acct.id, "key": "sk-claude-secret"}),
        },
    );
    let claude_prep: PreparedLaunch = serde_json::from_value(
        handle(
            &shared,
            &Request {
                id: 3,
                method: "managed_launch_prepare".into(),
                params: json!(LaunchRequest {
                    request_id: Uuid::new_v4(),
                    tool: ToolId::Claude,
                    connection: ConnectionKind::NearAi,
                    account_id: claude_acct.id,
                    cwd: dir.path().to_path_buf(),
                    expected_generation: Generation(0),
                    save_default: false,
                    collect_traces: false,
                    purpose: LaunchPurpose::Coding,
                    model: None,
                    background_model: None,
                    keep_saved_model: false,
                    cli_options: None,
                    resume_of: None,
                    sandbox: None,
                }),
            },
        )
        .result
        .unwrap(),
    )
    .unwrap();
    let claude_ctx: LaunchContext = serde_json::from_value(
        handle(
            &shared,
            &Request {
                id: 4,
                method: "managed_launch_redeem".into(),
                params: json!({"session_id": claude_prep.session_id, "ticket": claude_prep.ticket}),
            },
        )
        .result
        .unwrap(),
    )
    .unwrap();
    assert!(
        claude_ctx.red_team_key.is_none(),
        "Claude redeem must never carry red_team_key"
    );

    // Red Team saved NEAR AI account -> redeem MUST return red_team_key.
    let rt_acct: AccountView = serde_json::from_value(
        handle(
            &shared,
            &Request {
                id: 5,
                method: "managed_account_add".into(),
                params: json!({"tool": "red_team", "connection": "near_ai", "label": "RT Key"}),
            },
        )
        .result
        .unwrap(),
    )
    .unwrap();
    let _ = handle(
        &shared,
        &Request {
            id: 6,
            method: "managed_account_set_key".into(),
            params: json!({"account_id": rt_acct.id, "key": "sk-red-team-secret"}),
        },
    );
    let rt_prep: PreparedLaunch = serde_json::from_value(
        handle(
            &shared,
            &Request {
                id: 7,
                method: "managed_launch_prepare".into(),
                params: json!(LaunchRequest {
                    request_id: Uuid::new_v4(),
                    tool: ToolId::RedTeam,
                    connection: ConnectionKind::NearAi,
                    account_id: rt_acct.id,
                    cwd: dir.path().to_path_buf(),
                    expected_generation: Generation(0),
                    save_default: false,
                    collect_traces: false,
                    purpose: LaunchPurpose::Coding,
                    model: None,
                    background_model: None,
                    keep_saved_model: false,
                    cli_options: None,
                    resume_of: None,
                    sandbox: None,
                }),
            },
        )
        .result
        .unwrap(),
    )
    .unwrap();
    let rt_ctx: LaunchContext = serde_json::from_value(
        handle(
            &shared,
            &Request {
                id: 8,
                method: "managed_launch_redeem".into(),
                params: json!({"session_id": rt_prep.session_id, "ticket": rt_prep.ticket}),
            },
        )
        .result
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        rt_ctx.red_team_key.as_ref().map(|k| k.expose()),
        Some("sk-red-team-secret")
    );
    // Debug must never print the secret.
    assert!(!format!("{:?}", rt_ctx.red_team_key).contains("sk-red-team-secret"));
}

#[cfg(target_os = "macos")]
#[test]
fn the_key_is_never_in_the_spec_environment() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let secret = "sk-super-secret-red-team-key-9999";
    let ctx = test_context(dir.path(), vec![], secret);
    let spec = spec_for(&store, &ctx, None).unwrap();
    assert!(
        !spec.env.iter().any(|(_, v)| v == secret),
        "secret key must never appear in spec.env"
    );
    assert!(
        !spec
            .env
            .iter()
            .any(|(k, _)| matches!(k.as_str(), "NEARAI_API_KEY" | "NEAR_AI_SESSION_API_KEY")),
        "NEAR AI key env vars must be stripped from spec.env"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn the_spec_captures_receipts_into_the_managed_route_home() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let ctx = test_context(dir.path(), vec![], "sk-receipt-test");
    let spec = spec_for(&store, &ctx, None).unwrap();
    assert_eq!(
        spec.ironwire.home,
        Some(crate::managed::route::home_for(&store, ctx.session.id))
    );
    assert!(spec.ironwire.capture.enabled);
    assert!(spec.ironwire.capture.receipts);
    assert!(!spec.ironwire.capture.logprobs);
}

#[cfg(target_os = "macos")]
#[test]
fn the_tool_command_is_this_executable() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let ctx = test_context(dir.path(), vec![], "sk-tool-cmd-test");
    let spec = spec_for(&store, &ctx, None).unwrap();
    let exe = std::env::current_exe()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        spec.tool_command,
        Some(vec![exe, "red-team".to_owned(), "tool".to_owned()])
    );
}

#[test]
fn inputs_are_copied_not_linked() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let original = dir.path().join("notes.txt");
    std::fs::write(&original, b"original-content").unwrap();

    let session_id = Uuid::new_v4();
    let staged_dir = stage_inputs(&store, session_id, std::slice::from_ref(&original))
        .unwrap()
        .expect("staged in dir");
    let staged_file = staged_dir.join("notes.txt");
    let meta = std::fs::symlink_metadata(&staged_file).unwrap();
    assert!(meta.is_file());
    assert!(!meta.file_type().is_symlink());

    // Editing the staged copy must leave the original untouched.
    std::fs::write(&staged_file, b"mutated-in-sandbox").unwrap();
    assert_eq!(std::fs::read(&original).unwrap(), b"original-content");
}

#[test]
fn route_config_is_unchanged_for_other_sessions() {
    let url = "https://cloud-api.near.ai/v1";
    let expected = "[capture]\nenabled = true\nbodies = false\nreceipts = true\n[privacy]\nmode = \"full\"\ntrusted_backends = [\"nearai\"]\n[[backends]]\nid = \"nearai\"\nkind = \"nearai\"\nbase_url = \"https://cloud-api.near.ai/v1\"\n";
    assert_eq!(crate::managed::route::ironwire_config(false, url), expected);
}

#[test]
fn red_team_setup_checks_inside_our_config_folder() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let mut seen_prefix = None;
    let state = setup_state_with(&store, |prefix, _sessions| {
        seen_prefix = Some(prefix.to_path_buf());
        true
    });
    assert_eq!(state, crate::managed::tool_setup::ToolSetupState::Ready);
    assert_eq!(seen_prefix, Some(store.dir().join("red-team/runtime")));
}

#[test]
fn the_release_pin_is_well_formed() {
    let pin = crate::managed::red_team_release::RTS_RELEASE;
    assert!(
        pin.url
            .starts_with("https://github.com/nearai/nearai-red-sandbox/releases/download/"),
        "release URL must point to nearai-red-sandbox GitHub releases: {}",
        pin.url
    );
    assert_eq!(pin.sha256.len(), 32);
    assert!(
        pin.sha256.iter().any(|&b| b != 0),
        "release sha256 digest must be non-zero"
    );
}
