// Some tests here are Apple silicon macOS-only; elsewhere their shared imports
// and helpers are unused, which -D warnings turns into a build failure.
#![cfg_attr(
    not(all(target_os = "macos", target_arch = "aarch64")),
    allow(unused_imports, dead_code)
)]
use super::*;
use crate::config::ConfigStore;
use crate::daemon::ipc::{DaemonShared, Request};
use crate::daemon::managed::handle;
use crate::managed::red_team_host::HostProblem;
use crate::managed::sandbox::SandboxSettings;
use crate::managed::secrets::RedactedSecret;
use crate::managed::sessions::{
    CollectionState, LaunchContext, LaunchPurpose, LaunchRequest, PreparedLaunch, SessionState,
    SessionView,
};
use crate::managed::tool_setup::ToolSetupState;
use crate::managed::{AccountId, AccountView, ConnectionKind, Generation, ToolId};
use serde_json::json;
use uuid::Uuid;

#[cfg(windows)]
#[test]
fn windows_setup_probe_captures_output_without_allocating_a_console() {
    use std::time::Duration;

    let temp = tempfile::tempdir().unwrap();
    let setup = windows_setup_context(&temp.path().join("runtime"), temp.path()).unwrap();
    let powershell = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    let script = r#"
        Add-Type -Namespace GatewayTests -Name Console -MemberDefinition '[DllImport("kernel32.dll")] public static extern System.IntPtr GetConsoleWindow();'
        if ([GatewayTests.Console]::GetConsoleWindow() -ne [IntPtr]::Zero) { exit 1 }
        [Console]::WriteLine('probe complete')
    "#;
    let argv = vec![
        powershell.to_str().unwrap().to_owned(),
        "-NoProfile".into(),
        "-NonInteractive".into(),
        "-Command".into(),
        script.into(),
    ];
    let options = rts::setup::SetupOptions {
        check: true,
        offline: true,
    };
    let output = setup
        .context(&options)
        .host
        .run(&argv, Duration::from_secs(30))
        .unwrap();
    assert_eq!(output.code, Some(0));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "probe complete"
    );
}

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
            output_dir: None,
            allow_hosts: vec![],
            packages: false,
            cpus: None,
            memory_mib: None,
        }),
        red_team_key: Some(RedactedSecret::new(key.to_owned())),
        initial_prompt: None,
    }
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[tokio::test]
async fn redeem_returns_the_key_only_for_red_team() {
    use crate::daemon::managed::handle_async_with;
    use crate::daemon::nearai_credential::balance::BalanceReport;
    use crate::managed::signin;

    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let shared = DaemonShared::load(store).unwrap();
    let cred = crate::daemon::settings::NearAiInferenceCredential {
        key: "sk-red-team-secret".into(),
        key_id: "rt-key".into(),
        key_prefix: "sk-re".into(),
        organization_id: "org".into(),
        workspace_id: "ws".into(),
        minted_at: chrono::Utc::now(),
    };
    shared.settings.lock().unwrap().near_ai_inference = Some(cred.clone());

    // Claude subscription account -> redeem must NOT return red_team_key.
    let claude_acct: AccountView = serde_json::from_value(
        handle(
            &shared,
            &Request {
                id: 1,
                method: "managed_account_add".into(),
                params: json!({"tool": "claude", "connection": "subscription", "label": "Claude Sub"}),
            },
        )
        .result
        .unwrap(),
    )
    .unwrap();
    let claude_prep: PreparedLaunch = serde_json::from_value(
        handle(
            &shared,
            &Request {
                id: 3,
                method: "managed_launch_prepare".into(),
                params: json!(LaunchRequest {
                    request_id: Uuid::new_v4(),
                    tool: ToolId::Claude,
                    connection: ConnectionKind::Subscription,
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
                    initial_prompt: None,
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

    // Red Team on NEAR AI sign-in -> redeem MUST return red_team_key.
    shared.settings.lock().unwrap().near_ai_inference = Some(cred.clone());
    let rt_account_id = signin::account_id(ToolId::RedTeam).unwrap();
    let rt_prep: PreparedLaunch = serde_json::from_value(
        handle_async_with(
            &shared,
            &Request {
                id: 7,
                method: "managed_launch_prepare".into(),
                params: json!(LaunchRequest {
                    request_id: Uuid::new_v4(),
                    tool: ToolId::RedTeam,
                    connection: ConnectionKind::NearAiSignIn,
                    account_id: rt_account_id,
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
                    initial_prompt: None,
                }),
            },
            |_| async { BalanceReport::Unavailable },
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();
    shared.settings.lock().unwrap().near_ai_inference = Some(cred);
    let rt_ctx: LaunchContext = serde_json::from_value(
        handle_async_with(
            &shared,
            &Request {
                id: 8,
                method: "managed_launch_redeem".into(),
                params: json!({"session_id": rt_prep.session_id, "ticket": rt_prep.ticket}),
            },
            |_| async { BalanceReport::Unavailable },
        )
        .await
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

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
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

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
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

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn the_spec_targets_the_builds_near_ai_environment() {
    use rts::inference::NearEnvironment;
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let ctx = test_context(dir.path(), vec![], "sk-near-env-test");
    let spec = spec_for(&store, &ctx, None).unwrap();
    let expected = if cfg!(feature = "near-ai-staging") {
        NearEnvironment::Staging
    } else {
        NearEnvironment::Production
    };
    assert_eq!(spec.near, expected);
    assert_eq!(super::near_environment(), expected);
}

/// NEAR redeploys the staging gateway several times a day, so a staging
/// build does not require its compose hash to be pinned (staging 0.24.68:
/// 10d29b73 pinned in the morning, 4c544d60 live by the afternoon). Every
/// other check stays on, and production stays strictly pinned.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn only_staging_accepts_an_unpinned_compose_that_the_tee_measured() {
    use rts::attest::{AttestOptions, ComposePolicy};
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let ctx = test_context(dir.path(), vec![], "sk-compose-policy-test");
    let spec = spec_for(&store, &ctx, None).unwrap();
    let expected = if cfg!(feature = "near-ai-staging") {
        AttestOptions {
            compose_policy: ComposePolicy::MrConfigOnly,
            ..AttestOptions::default()
        }
    } else {
        AttestOptions::default()
    };
    assert_eq!(spec.attest, expected);
    assert!(!spec.attest.allow_tcb_out_of_date);
    assert!(!spec.attest.allow_unattested_provider);
}

/// rts and the daemon must never disagree about which NEAR AI the key is for:
/// the environment rts attests and proxies to is the channel's own inference
/// base, the one the model catalogue is read from.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn the_near_environment_is_the_channels_inference_base() {
    assert_eq!(
        super::near_environment().base_url(),
        crate::channel::NEAR_AI_INFERENCE_BASE
    );
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
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

/// A project folder with node_modules has symlinks inside it (.bin/*).
/// Staging skips them without following, copies the rest, and lists what
/// it skipped beside (not inside) the staged folder (staging 0.24.68).
#[cfg(unix)]
#[test]
fn symlinks_inside_an_input_folder_are_skipped_not_followed() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let outside = dir.path().join("secret.txt");
    std::fs::write(&outside, b"outside-the-folder").unwrap();
    let project = dir.path().join("site");
    let bin = project.join("node_modules").join(".bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(project.join("index.js"), b"code").unwrap();
    std::fs::write(project.join("node_modules").join("tool.js"), b"tool").unwrap();
    std::os::unix::fs::symlink("../tool.js", bin.join("tool")).unwrap();
    std::os::unix::fs::symlink(&outside, project.join("leak")).unwrap();

    let session_id = Uuid::new_v4();
    let staged = stage_inputs(&store, session_id, std::slice::from_ref(&project))
        .unwrap()
        .expect("staged in dir");
    let site = staged.join("site");
    assert_eq!(std::fs::read(site.join("index.js")).unwrap(), b"code");
    assert_eq!(
        std::fs::read(site.join("node_modules").join("tool.js")).unwrap(),
        b"tool"
    );
    assert!(std::fs::symlink_metadata(site.join("leak")).is_err());
    assert!(std::fs::symlink_metadata(site.join("node_modules/.bin/tool")).is_err());
    assert!(site.join("node_modules/.bin").is_dir());

    let note =
        std::fs::read_to_string(staged.parent().unwrap().join(SKIPPED_SYMLINKS_FILE)).unwrap();
    assert!(
        note.contains(&project.join("leak").display().to_string()),
        "{note}"
    );
    assert!(
        note.contains(&bin.join("tool").display().to_string()),
        "{note}"
    );
    assert!(!note.contains("outside-the-folder"));
}

#[cfg(unix)]
#[test]
fn a_selected_input_that_is_itself_a_symlink_is_still_refused() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let real = dir.path().join("real");
    std::fs::create_dir(&real).unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert!(stage_inputs(&store, Uuid::new_v4(), std::slice::from_ref(&link)).is_err());
}

#[test]
fn route_config_is_unchanged_for_other_sessions() {
    let url = "https://cloud-api.near.ai/v1";
    let expected = "[capture]\nenabled = true\nbodies = false\nreceipts = true\n[privacy]\nmode = \"full\"\ntrusted_backends = [\"nearai\"]\n[[backends]]\nid = \"nearai\"\nkind = \"nearai\"\nbase_url = \"https://cloud-api.near.ai/v1\"\n";
    assert_eq!(crate::managed::route::ironwire_config(false, url), expected);
}

fn status(
    check: Result<(), &str>,
    membership: Option<HostProblem>,
) -> (SetupStatus, Option<std::path::PathBuf>) {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let mut seen_prefix = None;
    let status = status_with(
        &store,
        |prefix, _sessions| {
            seen_prefix = Some(prefix.to_path_buf());
            check.map_err(str::to_owned)
        },
        || membership,
    );
    let expected = store.dir().join("red-team/runtime");
    assert_eq!(seen_prefix, Some(expected.clone()));
    (status, Some(expected))
}

#[test]
fn red_team_setup_checks_inside_our_config_folder() {
    let (status, _) = status(Ok(()), None);
    assert_eq!(status.state, ToolSetupState::Ready);
    assert_eq!(status.problem, None);
}

#[test]
fn a_failed_check_needs_setup_without_a_host_problem_when_rts_names_none() {
    let (status, _) = status(Err("the rts guest file x is missing"), None);
    assert_eq!(status.state, ToolSetupState::NeedsSetup);
    assert_eq!(status.problem, None);
}

#[test]
fn windows_home_and_missing_hyper_v_are_host_problems() {
    let (home, _) = status(Err("Windows Home is not supported: rts runs"), None);
    assert_eq!(home.state, ToolSetupState::NeedsSetup);
    assert_eq!(home.problem, Some(HostProblem::HomeEdition));
    let (off, _) = status(Err("Hyper-V is not available on this computer (x)"), None);
    assert_eq!(off.problem, Some(HostProblem::HyperVMissing));
}

#[test]
fn a_user_outside_hyper_v_administrators_is_not_ready_even_with_everything_installed() {
    let (status, _) = status(Ok(()), Some(HostProblem::NotInHyperVAdministrators));
    assert_eq!(status.state, ToolSetupState::NeedsSetup);
    assert_eq!(status.problem, Some(HostProblem::NotInHyperVAdministrators));
}

#[test]
fn the_check_names_the_edition_before_the_group() {
    let (status, _) = status(
        Err("Windows Home is not supported: x"),
        Some(HostProblem::NotInHyperVAdministrators),
    );
    assert_eq!(status.problem, Some(HostProblem::HomeEdition));
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

#[test]
fn a_finished_red_team_session_publishes_its_opencode_store() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let ctx = test_context(dir.path(), vec![], "sk-collect-test");
    let session_dir = dir.path().join("session-out");
    let opencode_dir = session_dir.join("opencode");
    std::fs::create_dir_all(&opencode_dir).unwrap();
    std::fs::write(opencode_dir.join("opencode.db"), b"SQLite format 3\0test").unwrap();

    let outcome = finalize_outcome(&store, &ctx, 0, &session_dir);
    assert_eq!(outcome.state, SessionState::Exited);
    assert_eq!(outcome.collection, Some(CollectionState::Collected));
    assert!(crate::managed::native_collection::published(
        &store,
        ToolId::RedTeam,
        ctx.session.id
    ));
    let staged_db = store
        .dir()
        .join(crate::managed::native_collection::STAGING)
        .join("opencode")
        .join(ctx.session.id.to_string())
        .join("opencode.db");
    assert!(staged_db.is_file());
}

#[test]
fn a_session_that_never_started_opencode_publishes_nothing_and_ends_failed() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let ctx = test_context(dir.path(), vec![], "sk-no-opencode");
    let session_dir = dir.path().join("session-empty");
    std::fs::create_dir_all(&session_dir).unwrap();

    let outcome = finalize_outcome(&store, &ctx, 0, &session_dir);
    assert_eq!(outcome.state, SessionState::Failed);
    assert_eq!(outcome.collection, Some(CollectionState::Failed));
    assert!(!crate::managed::native_collection::published(
        &store,
        ToolId::RedTeam,
        ctx.session.id
    ));
}

#[test]
fn collection_off_publishes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let mut ctx = test_context(dir.path(), vec![], "sk-off");
    ctx.session.collect_traces = false;
    let session_dir = dir.path().join("session-off");
    let opencode_dir = session_dir.join("opencode");
    std::fs::create_dir_all(&opencode_dir).unwrap();
    std::fs::write(opencode_dir.join("opencode.db"), b"SQLite format 3\0test").unwrap();

    let outcome = finalize_outcome(&store, &ctx, 0, &session_dir);
    assert_eq!(outcome.state, SessionState::Exited);
    assert_eq!(outcome.collection, None);
    assert!(!crate::managed::native_collection::published(
        &store,
        ToolId::RedTeam,
        ctx.session.id
    ));
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn the_ledger_is_in_the_managed_route_home() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let ctx = test_context(dir.path(), vec![], "sk-ledger");
    let spec = spec_for(&store, &ctx, None).unwrap();
    let expected_home = store
        .dir()
        .join("managed-routes")
        .join(ctx.session.id.to_string());
    assert_eq!(spec.ironwire.home, Some(expected_home));
}

#[cfg(any(all(target_os = "macos", target_arch = "aarch64"), windows))]
#[test]
fn the_attestation_policy_feed_keys_are_compiled_in() {
    let hex: Vec<String> = rts::attest::feed::FEED_KEYS
        .iter()
        .map(hex::encode)
        .collect();
    assert_eq!(
        hex,
        [
            "7b528575c138cd711ed863eb59a7a95d78d79d222a8a962ddb25196ccc14db5c",
            "39982511b3d64da04fbb22cbec7e6040ae5c844ec750a428e6528f221ffd4d17",
        ]
    );
}

#[cfg(any(all(target_os = "macos", target_arch = "aarch64"), windows))]
#[test]
fn the_gateway_prompt_names_the_near_ai_forget_command() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let ctx = test_context(dir.path(), vec![], "sk-red-team-secret");
    let spec = spec_for(&store, &ctx, None).unwrap();
    #[cfg(windows)]
    let expected = "near-ai-gateway-helper red-team trust forget";
    #[cfg(not(windows))]
    let expected = "near-ai red-team trust forget";
    assert_eq!(spec.trust_forget_command.as_deref(), Some(expected));
}

#[cfg(any(all(target_os = "macos", target_arch = "aarch64"), windows))]
fn trust_env(home: &Path) -> Vec<(String, String)> {
    let h = home.to_string_lossy().to_string();
    vec![("HOME".into(), h.clone()), ("USERPROFILE".into(), h)]
}

#[cfg(any(all(target_os = "macos", target_arch = "aarch64"), windows))]
#[test]
fn red_team_trust_forget_removes_one_build_and_lists_the_rest() {
    use rts::attest::{Role, trusted};
    let dir = tempfile::tempdir().unwrap();
    let env = trust_env(dir.path());
    let home = rts::install::home_in(&env, cfg!(windows)).unwrap();
    let entry = |byte: u8| trusted::TrustedEntry {
        environment: near_environment(),
        role: Role::Gateway,
        sha256: [byte; 32],
        app_name: None,
        trusted_at: "2026-11-25".into(),
    };
    trusted::add(&home, entry(0xf4)).unwrap();
    trusted::add(&home, entry(0xa1)).unwrap();
    let (out, err, code) = trust_command(TrustAction::Forget("f4f4f4f4".into()), &env);
    assert_eq!(code, 0, "{err:?}");
    assert_eq!(
        out,
        vec![format!(
            "forgot {} gateway {}",
            trusted::env_name(near_environment()),
            "f4".repeat(32)
        )]
    );
    let (out, _, code) = trust_command(TrustAction::List, &env);
    assert_eq!(code, 0);
    let joined = out.join("\n");
    assert!(joined.contains(&"a1".repeat(4)), "{joined}");
    assert!(!joined.contains("f4f4f4f4"), "{joined}");
}

#[cfg(any(all(target_os = "macos", target_arch = "aarch64"), windows))]
#[test]
fn red_team_trust_list_never_prints_store_text_raw() {
    let dir = tempfile::tempdir().unwrap();
    let env = trust_env(dir.path());
    let home = rts::install::home_in(&env, cfg!(windows)).unwrap();
    let path = rts::install::trusted_composes_path(&home);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "{\"version\":1,\"entries\":[],\"\\u001b[2J\":1}").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let (out, err, _) = trust_command(TrustAction::List, &env);
    assert!(!out.is_empty());
    for line in out.iter().chain(err.iter()) {
        assert!(!line.contains('\u{1b}'), "{line:?}");
    }
}
