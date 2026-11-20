//! Red Team managed sessions: staging inputs, building `rts::launch::LaunchSpec`,
//! running the sandboxed session on a blocking thread, and handling the
//! `red-team tool <name>` bridge subcommand.

#[cfg(target_os = "macos")]
use super::ToolId;
use super::{ManagedError, sessions::LaunchContext};
use crate::config::ConfigStore;
use std::path::{Path, PathBuf};
#[cfg(any(target_os = "macos", test))]
use uuid::Uuid;

/// Relative path under the config directory where Red Team session outputs
/// (`<id>/out`, `<id>/opencode`, `tools.jsonl`) are stored.
pub const RED_TEAM_SESSIONS_DIR: &str = "red-team/sessions";

/// Relative path under the config directory where Red Team runtime assets
/// (`mvmctl`, guest image helpers, plugin shim) are installed.
pub const RED_TEAM_RUNTIME_DIR: &str = "red-team/runtime";

/// The runtime prefix under our config directory (`<config>/red-team/runtime`).
pub fn runtime_prefix(store: &ConfigStore) -> PathBuf {
    store.dir().join(RED_TEAM_RUNTIME_DIR)
}

/// Check whether Red Team prerequisites are installed inside `<config>/red-team/runtime`.
pub fn setup_state(store: &ConfigStore) -> super::tool_setup::ToolSetupState {
    setup_state_with(store, run_setup_check)
}

pub(crate) fn setup_state_with<F>(
    store: &ConfigStore,
    check: F,
) -> super::tool_setup::ToolSetupState
where
    F: FnOnce(&Path, &Path) -> bool,
{
    let prefix = runtime_prefix(store);
    let sessions_dir = store.dir().join(RED_TEAM_SESSIONS_DIR);
    if check(&prefix, &sessions_dir) {
        super::tool_setup::ToolSetupState::Ready
    } else {
        super::tool_setup::ToolSetupState::NeedsSetup
    }
}

#[cfg(target_os = "macos")]
fn run_setup_check(prefix: &Path, sessions_dir: &Path) -> bool {
    let Ok(fetch) = rts::setup::HttpsFetch::new() else {
        return false;
    };
    let defaults = rts::config::Defaults::for_prefix(prefix, sessions_dir);
    let supervisor = rts::install::prefix_supervisor(prefix);
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("near-ai"));
    let host = rts::session::RealHost;
    let Ok((_tmp, empty_git)) = rts::launch::private_git_config("rts-check-") else {
        return false;
    };
    let mvm_plain = rts::mvm::SpawnRunner {
        mvmctl: defaults.mvmctl.clone(),
        empty_git_config: empty_git.clone(),
        extra_env: std::collections::HashMap::new(),
        remove_env: Vec::new(),
    };
    let mvm = rts::mvm::SpawnRunner {
        mvmctl: defaults.mvmctl.clone(),
        empty_git_config: empty_git,
        extra_env: std::collections::HashMap::new(),
        remove_env: Vec::new(),
    };
    let ctx = rts::setup::SetupContext {
        home: prefix,
        defaults: &defaults,
        explicit_mvmctl: false,
        supervisor: &supervisor,
        rts: &exe,
        fetch: &fetch,
        host: &host,
        mvm_plain: &mvm_plain,
        mvm: &mvm,
    };
    let opts = rts::setup::SetupOptions {
        check: true,
        offline: true,
    };
    let release = super::red_team_release::RTS_RELEASE.to_archive();
    rts::setup::run_in_prefix(&opts, &ctx, prefix, &release).is_ok()
}

#[cfg(not(target_os = "macos"))]
fn run_setup_check(_prefix: &Path, _sessions_dir: &Path) -> bool {
    false
}

/// Install Red Team prerequisites from the pinned signed release archive into
/// `<config>/red-team/runtime`.
#[cfg(target_os = "macos")]
pub fn setup_install(store: &ConfigStore) -> Result<(), ManagedError> {
    let prefix = runtime_prefix(store);
    if let Some(parent) = prefix.parent() {
        super::profiles::private_directory(parent)?;
    }
    super::profiles::private_directory(&prefix)?;
    let sessions_dir = store.dir().join(RED_TEAM_SESSIONS_DIR);
    let defaults = rts::config::Defaults::for_prefix(&prefix, &sessions_dir);
    let supervisor = rts::install::prefix_supervisor(&prefix);
    let exe = std::env::current_exe().map_err(|_| ManagedError::ToolUnavailable)?;
    let fetch = rts::setup::HttpsFetch::new().map_err(|_| ManagedError::ToolUnavailable)?;
    let host = rts::session::RealHost;
    let (_tmp, empty_git) = rts::launch::private_git_config("rts-setup-")
        .map_err(|_| ManagedError::StorageUnavailable)?;
    let env: Vec<(String, String)> = std::env::vars().collect();
    let mvm_plain = rts::mvm::SpawnRunner {
        mvmctl: defaults.mvmctl.clone(),
        empty_git_config: empty_git.clone(),
        extra_env: std::collections::HashMap::from([
            ("HOME".into(), prefix.to_string_lossy().into_owned()),
            (
                "MVM_HOME".into(),
                prefix.join("mvm").to_string_lossy().into_owned(),
            ),
        ]),
        remove_env: Vec::new(),
    };
    let mvm = rts::mvm::SpawnRunner {
        mvmctl: defaults.mvmctl.clone(),
        empty_git_config: empty_git,
        extra_env: rts::session::mvm_environment(&env, &supervisor),
        remove_env: Vec::new(),
    };
    let ctx = rts::setup::SetupContext {
        home: &prefix,
        defaults: &defaults,
        explicit_mvmctl: false,
        supervisor: &supervisor,
        rts: &exe,
        fetch: &fetch,
        host: &host,
        mvm_plain: &mvm_plain,
        mvm: &mvm,
    };
    let opts = rts::setup::SetupOptions {
        check: false,
        offline: false,
    };
    let release = super::red_team_release::RTS_RELEASE.to_archive();
    rts::setup::run_in_prefix(&opts, &ctx, &prefix, &release)
        .map(|_| ())
        .map_err(|_| ManagedError::ToolUnavailable)
}

#[cfg(not(target_os = "macos"))]
pub fn setup_install(_store: &ConfigStore) -> Result<(), ManagedError> {
    Err(ManagedError::RedTeamUnavailable)
}

/// Copy each validated input file or directory into
/// `<config>/red-team/<session_id>/in/`. Never symlinks; editing a staged copy
/// never mutates the original.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn stage_inputs(
    store: &ConfigStore,
    session_id: Uuid,
    inputs: &[PathBuf],
) -> anyhow::Result<Option<PathBuf>> {
    if inputs.is_empty() {
        return Ok(None);
    }
    let session_root = store.dir().join("red-team").join(session_id.to_string());
    if let Some(parent) = session_root.parent() {
        super::profiles::private_directory(parent)?;
    }
    super::profiles::private_directory(&session_root)?;
    let in_dir = session_root.join("in");
    super::profiles::private_directory(&in_dir)?;

    let mut used_names = std::collections::HashSet::new();
    for src in inputs {
        let meta = std::fs::symlink_metadata(src)?;
        if meta.file_type().is_symlink() {
            return Err(ManagedError::SandboxInputRefused(format!(
                "input path must not be a symlink: {}",
                src.display()
            ))
            .into());
        }
        let base = src
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("input")
            .to_owned();
        let mut name = base.clone();
        let mut idx = 1usize;
        while !used_names.insert(name.clone()) {
            name = format!("{base}-{idx}");
            idx += 1;
        }
        let dst = in_dir.join(&name);
        copy_no_symlinks(src, &dst)?;
    }
    Ok(Some(in_dir))
}

#[cfg(any(target_os = "macos", test))]
fn copy_no_symlinks(src: &Path, dst: &Path) -> anyhow::Result<()> {
    let meta = std::fs::symlink_metadata(src)?;
    if meta.file_type().is_symlink() {
        return Err(ManagedError::SandboxInputRefused(format!(
            "refusing to copy symlink: {}",
            src.display()
        ))
        .into());
    }
    if meta.is_file() {
        std::fs::copy(src, dst)?;
        return Ok(());
    }
    if meta.is_dir() {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let child_src = entry.path();
            let child_dst = dst.join(entry.file_name());
            copy_no_symlinks(&child_src, &child_dst)?;
        }
        return Ok(());
    }
    Err(ManagedError::SandboxInputRefused(format!(
        "unsupported input entry type: {}",
        src.display()
    ))
    .into())
}

#[cfg(target_os = "macos")]
pub(crate) fn spec_for(
    store: &ConfigStore,
    context: &LaunchContext,
    initial_prompt: Option<String>,
) -> anyhow::Result<rts::launch::LaunchSpec> {
    if context.session.tool != ToolId::RedTeam {
        return Err(ManagedError::InvalidRequest.into());
    }
    let key = context
        .red_team_key
        .as_ref()
        .ok_or(ManagedError::NearAiSignInRequired)?;
    let key_str = key.expose();

    let sandbox = context.sandbox.clone().unwrap_or_default();
    let staged_in = stage_inputs(store, context.session.id, &sandbox.inputs)?;

    let runtime_prefix = store.dir().join(RED_TEAM_RUNTIME_DIR);
    let sessions_dir = store.dir().join(RED_TEAM_SESSIONS_DIR);
    let defaults = rts::config::Defaults::for_prefix(&runtime_prefix, &sessions_dir);
    let mut config = rts::config::parse_config("{}", &context.session.cwd, &defaults)
        .map_err(anyhow::Error::msg)?;
    config.inputs_dir = staged_in;
    config.network = sandbox.allow_hosts;
    config.update_check = false;
    config
        .limits
        .resize(sandbox.cpus, sandbox.memory_mib)
        .map_err(anyhow::Error::msg)?;
    if let Some(model) = context.session.model.as_deref()
        && !model.is_empty()
    {
        config.provider.models = vec![model.to_owned()];
    }

    let simple_hex = context.session.id.simple().to_string();
    let session_id = Some(simple_hex[..12].to_owned());

    let exe = std::env::current_exe()?.to_string_lossy().into_owned();
    let tool_command = Some(vec![exe, "red-team".to_owned(), "tool".to_owned()]);

    let bodies = crate::daemon::settings::DaemonSettings::load(store)
        .map(|s| s.ironwire_attested_bodies)
        .unwrap_or(false);
    let route_home = super::route::home_for(store, context.session.id);
    if let Some(parent) = route_home.parent() {
        super::profiles::private_directory(parent)?;
    }

    let store_clone = store.clone();
    let key_for_attestor = key_str.to_owned();
    let ironwire = rts::inference::IronWireSetup {
        home: Some(route_home),
        capture: rts::inference::Capture {
            enabled: true,
            bodies,
            logprobs: false,
            receipts: true,
        },
        configure: Some(Box::new(move |opts| {
            super::route::attach_signer_attestor(&store_clone, &key_for_attestor, opts)
        })),
    };

    let env = std::env::vars()
        .filter(|(k, v)| {
            v != key_str && !matches!(k.as_str(), "NEARAI_API_KEY" | "NEAR_AI_SESSION_API_KEY")
        })
        .collect();

    Ok(rts::launch::LaunchSpec {
        config,
        attest: rts::attest::AttestOptions::default(),
        credential: rts::launch::CredentialSource::Supplied(
            ironwire_proxy::embed::HostSecret::from(key_str.to_owned()),
        ),
        initial_prompt,
        no_splash: false,
        session_id,
        tool_command,
        ironwire,
        env,
    })
}

/// Result of a completed Red Team session run, including the final session
/// state and collection state derived from whether the OpenCode store was
/// published.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunOutcome {
    pub exit_code: i32,
    pub state: super::sessions::SessionState,
    pub collection: Option<super::sessions::CollectionState>,
}

/// Publish the preserved OpenCode store when `collect_traces` is set and
/// compute the final `(SessionState, Option<CollectionState>)` for the row.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn finalize_outcome(
    store: &ConfigStore,
    context: &LaunchContext,
    exit_code: i32,
    session_dir: &Path,
) -> RunOutcome {
    use super::sessions::{CollectionState, SessionState};
    if !context.session.collect_traces {
        return RunOutcome {
            exit_code,
            state: if exit_code == 0 {
                SessionState::Exited
            } else {
                SessionState::Failed
            },
            collection: None,
        };
    }
    match super::native_collection::publish_opencode_store(
        store,
        context.session.id,
        &session_dir.join("opencode"),
    ) {
        Ok(()) => RunOutcome {
            exit_code,
            state: if exit_code == 0 {
                SessionState::Exited
            } else {
                SessionState::Failed
            },
            collection: Some(CollectionState::Collected),
        },
        Err(_) => RunOutcome {
            exit_code: if exit_code == 0 { 1 } else { exit_code },
            state: SessionState::Failed,
            collection: Some(CollectionState::Failed),
        },
    }
}

/// Copy exported files from `<session_dir>/out` into the operator's chosen
/// output folder. Symlinks are skipped rather than followed. Only `run`
/// calls it, so it is built where `run` is.
#[cfg(target_os = "macos")]
pub(crate) fn export_outputs(session_dir: &Path, output_dir: &Path) -> anyhow::Result<()> {
    let out_dir = session_dir.join("out");
    if !out_dir.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(output_dir)?;
    for entry in std::fs::read_dir(&out_dir)? {
        let entry = entry?;
        let src = entry.path();
        let meta = std::fs::symlink_metadata(&src)?;
        if meta.file_type().is_symlink() {
            continue;
        }
        let dst = output_dir.join(entry.file_name());
        copy_no_symlinks(&src, &dst)?;
    }
    Ok(())
}

/// Run a Red Team managed session through `rts::launch::run`.
#[cfg(target_os = "macos")]
pub async fn run(store: &ConfigStore, context: &LaunchContext) -> anyhow::Result<RunOutcome> {
    let spec = spec_for(store, context, context.initial_prompt.clone())?;
    let outcome = tokio::task::spawn_blocking(move || rts::launch::run(spec))
        .await
        .map_err(|e| anyhow::anyhow!("red-team session task failed: {e}"))?
        .map_err(anyhow::Error::msg)?;
    if let Some(output_dir) = context
        .sandbox
        .as_ref()
        .and_then(|s| s.output_dir.as_deref())
    {
        export_outputs(&outcome.session_dir, output_dir)?;
    }
    Ok(finalize_outcome(
        store,
        context,
        outcome.exit_code,
        &outcome.session_dir,
    ))
}

#[cfg(not(target_os = "macos"))]
pub async fn run(_store: &ConfigStore, _context: &LaunchContext) -> anyhow::Result<RunOutcome> {
    Err(ManagedError::RedTeamUnavailable.into())
}

/// Handle the hidden `red-team tool <name>` subcommand invoked by OpenCode's
/// plugin shim inside a Red Team session.
#[cfg(target_os = "macos")]
pub fn run_tool(name: &str) -> i32 {
    use std::io::Write;
    let outcome = rts::bridge::run(name);
    if !outcome.stdout.is_empty() {
        let _ = std::io::stdout().write_all(outcome.stdout.as_bytes());
    }
    if !outcome.stderr.is_empty() {
        let _ = std::io::stderr().write_all(outcome.stderr.as_bytes());
    }
    outcome.code
}

#[cfg(not(target_os = "macos"))]
pub fn run_tool(_name: &str) -> i32 {
    eprintln!("Red Team sessions are only supported on macOS.");
    1
}

#[cfg(test)]
#[path = "red_team_tests.rs"]
mod tests;
