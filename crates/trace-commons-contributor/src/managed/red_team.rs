//! Red Team managed sessions: staging inputs, building `rts::launch::LaunchSpec`,
//! running the sandboxed session on a blocking thread, and handling the
//! `red-team tool <name>` bridge subcommand.

#[cfg(any(all(target_os = "macos", target_arch = "aarch64"), windows))]
use super::ToolId;
use super::{ManagedError, sessions::LaunchContext};
use crate::config::ConfigStore;
use std::path::{Path, PathBuf};
#[cfg(any(all(target_os = "macos", target_arch = "aarch64"), windows, test))]
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

/// Whether Red Team can start here and, on Windows, why not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetupStatus {
    pub state: super::tool_setup::ToolSetupState,
    /// The host problem the app explains or offers to fix (Windows only).
    pub problem: Option<super::red_team_host::HostProblem>,
}

/// Check whether Red Team prerequisites are installed inside `<config>/red-team/runtime`.
pub fn setup_state(store: &ConfigStore) -> super::tool_setup::ToolSetupState {
    setup_status(store).state
}

/// [`setup_state`] with the host problem behind a `NeedsSetup`.
///
/// The check builds a blocking HTTP client, which panics when it is built or
/// dropped on an async worker thread (the daemon answers the managed snapshot
/// from one), so it always runs on its own short-lived thread.
pub fn setup_status(store: &ConfigStore) -> SetupStatus {
    std::thread::scope(|scope| {
        scope
            .spawn(|| status_with(store, run_setup_check, host_membership_problem))
            .join()
            .unwrap_or(SetupStatus {
                state: super::tool_setup::ToolSetupState::NeedsSetup,
                problem: None,
            })
    })
}

/// The status from the setup check's result (`Err` is its error text) and the
/// host's group-membership problem, which the check does not see.
pub(crate) fn status_with<C, M>(store: &ConfigStore, check: C, membership: M) -> SetupStatus
where
    C: FnOnce(&Path, &Path) -> Result<(), String>,
    M: FnOnce() -> Option<super::red_team_host::HostProblem>,
{
    use super::tool_setup::ToolSetupState;
    let prefix = runtime_prefix(store);
    let sessions_dir = store.dir().join(RED_TEAM_SESSIONS_DIR);
    let result = check(&prefix, &sessions_dir);
    let problem = match &result {
        Err(message) => super::red_team_host::classify_setup_error(message),
        Ok(()) => None,
    }
    .or_else(membership);
    SetupStatus {
        state: if result.is_ok() && problem.is_none() {
            ToolSetupState::Ready
        } else {
            ToolSetupState::NeedsSetup
        },
        problem,
    }
}

#[cfg(windows)]
fn host_membership_problem() -> Option<super::red_team_host::HostProblem> {
    (!super::red_team_host::user_in_hyperv_administrators())
        .then_some(super::red_team_host::HostProblem::NotInHyperVAdministrators)
}

#[cfg(not(windows))]
fn host_membership_problem() -> Option<super::red_team_host::HostProblem> {
    None
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn run_setup_check(prefix: &Path, sessions_dir: &Path) -> Result<(), String> {
    let fetch = rts::setup::HttpsFetch::new().map_err(|e| e.to_string())?;
    let defaults = rts::config::Defaults::for_prefix(prefix, sessions_dir);
    let supervisor = rts::install::prefix_supervisor(prefix);
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("near-ai"));
    let host = rts::session::RealHost;
    let (_tmp, empty_git) =
        rts::launch::private_git_config("rts-check-").map_err(|e| e.to_string())?;
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
    let release = super::red_team_release::platform_release().to_archive();
    rts::setup::run_in_prefix(&opts, &ctx, prefix, &release).map(|_| ())
}

#[cfg(windows)]
fn run_setup_check(prefix: &Path, sessions_dir: &Path) -> Result<(), String> {
    rts::setup::run_in_prefix(
        &rts::setup::SetupOptions {
            check: true,
            offline: true,
        },
        &windows_setup_context(prefix, sessions_dir)?.context(),
        prefix,
        &super::red_team_release::platform_release().to_archive(),
    )
    .map(|_| ())
}

#[cfg(not(any(all(target_os = "macos", target_arch = "aarch64"), windows)))]
fn run_setup_check(_prefix: &Path, _sessions_dir: &Path) -> Result<(), String> {
    Err("Red Team sessions are not available on this platform".into())
}

/// What `rts::setup::run_in_prefix` needs on Windows, which uses only the
/// fetcher and host (it has no mvm).
#[cfg(windows)]
struct WindowsSetup {
    defaults: rts::config::Defaults,
    supervisor: PathBuf,
    exe: PathBuf,
    fetch: rts::setup::HttpsFetch,
    host: rts::session::RealHost,
    mvm: rts::mvm::SpawnRunner,
    prefix: PathBuf,
    _tmp: Box<dyn std::any::Any>,
}

#[cfg(windows)]
impl WindowsSetup {
    fn context(&self) -> rts::setup::SetupContext<'_> {
        rts::setup::SetupContext {
            home: &self.prefix,
            defaults: &self.defaults,
            explicit_mvmctl: false,
            supervisor: &self.supervisor,
            rts: &self.exe,
            fetch: &self.fetch,
            host: &self.host,
            mvm_plain: &self.mvm,
            mvm: &self.mvm,
        }
    }
}

#[cfg(windows)]
fn windows_setup_context(prefix: &Path, sessions_dir: &Path) -> Result<WindowsSetup, String> {
    let defaults = rts::config::Defaults::for_prefix(prefix, sessions_dir);
    let (tmp, empty_git) =
        rts::launch::private_git_config("rts-setup-").map_err(|e| e.to_string())?;
    let mvm = rts::mvm::SpawnRunner {
        mvmctl: defaults.mvmctl.clone(),
        empty_git_config: empty_git,
        extra_env: std::collections::HashMap::new(),
        remove_env: Vec::new(),
    };
    Ok(WindowsSetup {
        supervisor: rts::install::prefix_supervisor(prefix),
        exe: std::env::current_exe().unwrap_or_else(|_| PathBuf::from("near-ai.exe")),
        fetch: rts::setup::HttpsFetch::new().map_err(|e| e.to_string())?,
        host: rts::session::RealHost,
        mvm,
        prefix: prefix.to_path_buf(),
        defaults,
        _tmp: Box::new(tmp),
    })
}

/// Install Red Team prerequisites from the pinned signed release archive into
/// `<config>/red-team/runtime`. On Windows this also installs OpenCode and the
/// Hyper-V guest files, and refuses before downloading anything when the host
/// cannot run the sandbox.
#[cfg(any(all(target_os = "macos", target_arch = "aarch64"), windows))]
pub fn setup_install(store: &ConfigStore) -> Result<(), ManagedError> {
    let prefix = runtime_prefix(store);
    if let Some(parent) = prefix.parent() {
        super::profiles::private_directory(parent)?;
    }
    super::profiles::private_directory(&prefix)?;
    let sessions_dir = store.dir().join(RED_TEAM_SESSIONS_DIR);
    let release = super::red_team_release::platform_release().to_archive();
    let opts = rts::setup::SetupOptions {
        check: false,
        offline: false,
    };
    #[cfg(windows)]
    {
        let setup = windows_setup_context(&prefix, &sessions_dir)
            .map_err(|_| ManagedError::ToolUnavailable)?;
        rts::setup::run_in_prefix(&opts, &setup.context(), &prefix, &release)
            .map(|_| ())
            .map_err(|_| ManagedError::ToolUnavailable)
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        let defaults = rts::config::Defaults::for_prefix(&prefix, &sessions_dir);
        let supervisor = rts::install::prefix_supervisor(&prefix);
        let exe = std::env::current_exe().map_err(|_| ManagedError::ToolUnavailable)?;
        let fetch = rts::setup::HttpsFetch::new().map_err(|_| ManagedError::ToolUnavailable)?;
        let host = rts::session::RealHost;
        let (_tmp, empty_git) = rts::launch::private_git_config("rts-setup-")
            .map_err(|_| ManagedError::StorageUnavailable)?;
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
        let env: Vec<(String, String)> = std::env::vars().collect();
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
        rts::setup::run_in_prefix(&opts, &ctx, &prefix, &release)
            .map(|_| ())
            .map_err(|_| ManagedError::ToolUnavailable)
    }
}

#[cfg(not(any(all(target_os = "macos", target_arch = "aarch64"), windows)))]
pub fn setup_install(_store: &ConfigStore) -> Result<(), ManagedError> {
    Err(ManagedError::RedTeamUnavailable)
}

/// Copy each validated input file or directory into
/// `<config>/red-team/<session_id>/in/`. Never symlinks; editing a staged copy
/// never mutates the original.
#[cfg(any(all(target_os = "macos", target_arch = "aarch64"), windows, test))]
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
    let mut skipped = Vec::new();
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
        copy_no_symlinks(src, &dst, &mut skipped)?;
    }
    if !skipped.is_empty() {
        let mut note =
            String::from("Symlinks in the input folders were skipped, not followed or copied:\n");
        for path in &skipped {
            note.push_str(&path.display().to_string());
            note.push('\n');
        }
        std::fs::write(session_root.join(SKIPPED_SYMLINKS_FILE), note)?;
        tracing::info!(
            count = skipped.len(),
            "red team: skipped symlinks inside input folders"
        );
    }
    Ok(Some(in_dir))
}

/// Beside `in/` (never inside it): the symlinks staging skipped.
#[cfg(any(all(target_os = "macos", target_arch = "aarch64"), windows, test))]
pub(crate) const SKIPPED_SYMLINKS_FILE: &str = "skipped-symlinks.txt";

/// Copies `src` to `dst`. A symlink inside a folder is never followed or
/// copied: it is recorded in `skipped` (a project's node_modules/.bin is
/// full of them). The selected input itself is checked by `stage_inputs`.
#[cfg(any(all(target_os = "macos", target_arch = "aarch64"), windows, test))]
fn copy_no_symlinks(src: &Path, dst: &Path, skipped: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    let meta = std::fs::symlink_metadata(src)?;
    if meta.file_type().is_symlink() {
        skipped.push(src.to_path_buf());
        return Ok(());
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
            copy_no_symlinks(&child_src, &child_dst, skipped)?;
        }
        return Ok(());
    }
    Err(ManagedError::SandboxInputRefused(format!(
        "unsupported input entry type: {}",
        src.display()
    ))
    .into())
}

/// The NEAR AI environment rts attests against and proxies to: staging builds
/// use staging, production builds production, matching the channel's
/// `NEAR_AI_INFERENCE_BASE` (a test holds the two together). rts defaults to
/// production, which refuses a staging key.
/// How strictly rts checks NEAR AI's attestation. Production requires the
/// gateway and model compose hashes to be pinned. NEAR redeploys the staging
/// gateway several times a day, so staging accepts any compose the TEE
/// measured (mr_config_id == sha256(app_compose)); the TDX quote, TCB status,
/// nonce binding and GPU checks still apply.
#[cfg(any(all(target_os = "macos", target_arch = "aarch64"), windows))]
pub(crate) fn attest_options() -> rts::attest::AttestOptions {
    let mut options = rts::attest::AttestOptions::default();
    if cfg!(feature = "near-ai-staging") {
        options.compose_policy = rts::attest::ComposePolicy::MrConfigOnly;
    }
    options
}

#[cfg(any(all(target_os = "macos", target_arch = "aarch64"), windows))]
pub(crate) const fn near_environment() -> rts::inference::NearEnvironment {
    if cfg!(feature = "near-ai-staging") {
        rts::inference::NearEnvironment::Staging
    } else {
        rts::inference::NearEnvironment::Production
    }
}

#[cfg(any(all(target_os = "macos", target_arch = "aarch64"), windows))]
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
        attest: attest_options(),
        credential: rts::launch::CredentialSource::Supplied(
            ironwire_proxy::embed::HostSecret::from(key_str.to_owned()),
        ),
        initial_prompt,
        no_splash: false,
        session_id,
        tool_command,
        ironwire,
        env,
        near: near_environment(),
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
#[cfg(any(all(target_os = "macos", target_arch = "aarch64"), windows, test))]
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
#[cfg(any(all(target_os = "macos", target_arch = "aarch64"), windows))]
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
        copy_no_symlinks(&src, &dst, &mut Vec::new())?;
    }
    Ok(())
}

/// Run a Red Team managed session through `rts::launch::run`.
#[cfg(any(all(target_os = "macos", target_arch = "aarch64"), windows))]
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

#[cfg(not(any(all(target_os = "macos", target_arch = "aarch64"), windows)))]
pub async fn run(_store: &ConfigStore, _context: &LaunchContext) -> anyhow::Result<RunOutcome> {
    Err(ManagedError::RedTeamUnavailable.into())
}

/// Handle the hidden `red-team tool <name>` subcommand invoked by OpenCode's
/// plugin shim inside a Red Team session.
#[cfg(any(all(target_os = "macos", target_arch = "aarch64"), windows))]
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

#[cfg(not(any(all(target_os = "macos", target_arch = "aarch64"), windows)))]
pub fn run_tool(_name: &str) -> i32 {
    eprintln!("Red Team sessions are only supported on Apple silicon Macs and Windows.");
    1
}

#[cfg(test)]
#[path = "red_team_tests.rs"]
mod tests;
