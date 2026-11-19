//! Red Team managed sessions: staging inputs, building `rts::launch::LaunchSpec`,
//! running the sandboxed session on a blocking thread, and handling the
//! `red-team tool <name>` bridge subcommand.

use super::{ManagedError, ToolId, sessions::LaunchContext};
use crate::config::ConfigStore;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Relative path under the config directory where Red Team session outputs
/// (`<id>/out`, `<id>/opencode`, `tools.jsonl`) are stored.
pub const RED_TEAM_SESSIONS_DIR: &str = "red-team/sessions";

/// Relative path under the config directory where Red Team runtime assets
/// (`mvmctl`, guest image helpers, plugin shim) are installed.
pub const RED_TEAM_RUNTIME_DIR: &str = "red-team/runtime";

/// Copy each validated input file or directory into
/// `<config>/red-team/<session_id>/in/`. Never symlinks; editing a staged copy
/// never mutates the original.
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

    let exe = std::env::current_exe()?
        .to_string_lossy()
        .into_owned();
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

/// Run a Red Team managed session through `rts::launch::run`.
#[cfg(target_os = "macos")]
pub async fn run(store: &ConfigStore, context: &LaunchContext) -> anyhow::Result<i32> {
    let spec = spec_for(store, context, None)?;
    let outcome = tokio::task::spawn_blocking(move || rts::launch::run(spec))
        .await
        .map_err(|e| anyhow::anyhow!("red-team session task failed: {e}"))?
        .map_err(anyhow::Error::msg)?;
    Ok(outcome.exit_code)
}

#[cfg(not(target_os = "macos"))]
pub async fn run(_store: &ConfigStore, _context: &LaunchContext) -> anyhow::Result<i32> {
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
