//! Isolated Hermes classic-CLI launches on an explicit OpenAI-compatible route.
//! Contract inspected in NousResearch/hermes-agent at 84a0301415ed (2026-11-05).
//! Ordinary Hermes configuration, credentials and conversations are not reused.

use super::{
    ManagedError,
    profiles::{bounded_output, private_directory},
};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

/// Keep only host/terminal plumbing. In particular, do not inherit a remote
/// gateway, config backend, provider credentials, Python injection or proxies.
pub(crate) fn isolate(command: &mut Command, root: &Path) -> Result<(), ManagedError> {
    private_directory(root)?;
    // With no home .env Hermes lets its installation's .env overwrite the
    // launch environment. An empty, private file makes that layer fill gaps
    // only, preserving our explicit route and key without persisting secrets.
    crate::config::ConfigStore::open(root.to_path_buf())
        .and_then(|store| store.write_daemon_file(".env", b""))
        .map_err(|_| ManagedError::StorageUnavailable)?;
    let managed = root.join("managed");
    private_directory(&managed)?;
    let allowed = [
        "PATH",
        "HOME",
        "USERPROFILE",
        "USER",
        "LOGNAME",
        "SHELL",
        "TERM",
        "COLORTERM",
        "TERM_PROGRAM",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "TZ",
        "TMPDIR",
        "TMP",
        "TEMP",
        "SystemRoot",
        "WINDIR",
        "COMSPEC",
        "PATHEXT",
        "LOCALAPPDATA",
        "APPDATA",
        "SSH_AUTH_SOCK",
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XDG_RUNTIME_DIR",
    ];
    // Explicit removals survive the supervisor's Unix command wrapper;
    // Command::env_clear's hidden flag would not survive that reconstruction.
    for (name, _) in std::env::vars_os() {
        if !name.to_str().is_some_and(|name| {
            allowed
                .iter()
                .any(|allowed| name.eq_ignore_ascii_case(allowed))
        }) {
            command.env_remove(name);
        }
    }
    command
        .env("HERMES_HOME", root)
        .env("HERMES_MANAGED_DIR", managed)
        .env("HERMES_CONFIG_BACKEND", "file")
        .env("HERMES_TUI_GATEWAY_URL", "")
        .env("HERMES_SAFE_MODE", "1")
        .env("HERMES_IGNORE_USER_CONFIG", "1")
        .env("HERMES_IGNORE_RULES", "1")
        .args(["--profile", "default"]);
    Ok(())
}

pub(crate) fn supported_help(output: &std::process::Output) -> bool {
    let help = String::from_utf8_lossy(&output.stdout);
    output.status.success()
        && [
            "--cli",
            "--safe-mode",
            "--provider",
            "--model",
            "--base-url",
        ]
        .iter()
        .all(|flag| {
            help.split_whitespace()
                .any(|word| word == *flag || word.trim_end_matches(',') == *flag)
        })
}

/// Hermes's gateway may outlive its terminal. Own only this launch's fresh
/// home, and stop that home's gateway on every normal/error exit. Never use
/// `--all`, which would stop the person's unrelated Hermes sessions.
pub(crate) struct HermesSession {
    native: PathBuf,
    root: PathBuf,
}

impl HermesSession {
    pub(crate) fn new(native: &Path, root: PathBuf) -> Result<Self, ManagedError> {
        private_directory(&root)?;
        Ok(Self {
            native: native.into(),
            root,
        })
    }

    pub(crate) fn command(&self, cwd: &Path) -> Result<Command, ManagedError> {
        let mut command = Command::new(&self.native);
        command.current_dir(cwd);
        isolate(&mut command, &self.root)?;
        command.args(["chat", "--cli", "--safe-mode"]);
        Ok(command)
    }
}

impl Drop for HermesSession {
    fn drop(&mut self) {
        // No owner was started if setup failed before spawning the CLI.
        if !self.root.join("gateway.pid").exists() {
            return;
        }
        let mut command = Command::new(&self.native);
        command.current_dir(&self.root);
        if isolate(&mut command, &self.root).is_ok() {
            command.args(["gateway", "stop", "--yes"]);
            let _ = bounded_output(command);
        }
    }
}

/// The key stays in the child's environment, never argv or a config file.
/// An exact OPENAI_BASE_URL pairing authorizes this key for the custom host.
pub(crate) fn route(
    command: &mut Command,
    model: &str,
    base: &str,
    key: &str,
) -> Result<(), ManagedError> {
    if !super::opencode::valid_model(model) || key.is_empty() {
        return Err(ManagedError::InvalidRequest);
    }
    command
        .args(["--provider", "custom", "--model", model, "--base-url", base])
        .env("OPENAI_BASE_URL", base)
        .env("OPENAI_API_KEY", key);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn the_child_gets_only_its_isolated_home_and_selected_route() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();
        let native = home.path().join("hermes");
        std::fs::write(&native, r#"#!/usr/bin/env python3
import json, os, pathlib, sys
root = pathlib.Path(os.environ['HERMES_HOME'])
if sys.argv[3:] == ['gateway', 'stop', '--yes']:
    (root / 'stopped').touch()
else:
    assert sys.argv[1:6] == ['--profile', 'default', 'chat', '--cli', '--safe-mode']
    (root / 'gateway.pid').write_text('fixture')
    print(json.dumps({'home':str(root), 'cwd':os.getcwd(), 'model':sys.argv[sys.argv.index('--model')+1],
        'base':sys.argv[sys.argv.index('--base-url')+1], 'env_base':os.environ['OPENAI_BASE_URL'],
        'key':os.environ['OPENAI_API_KEY'], 'key_in_argv':any('fixture-key' in a for a in sys.argv),
        'remote':os.environ['HERMES_TUI_GATEWAY_URL'],
        'foreign':any(os.environ.get(k) for k in ['ANTHROPIC_API_KEY','OPENROUTER_API_KEY','PYTHONPATH','HTTPS_PROXY'])}))
"#).unwrap();
        std::fs::set_permissions(&native, std::fs::Permissions::from_mode(0o700)).unwrap();
        let root = home.path().join("isolated");
        let session = HermesSession::new(&native, root.clone()).unwrap();
        let mut command = session.command(home.path()).unwrap();
        route(
            &mut command,
            "private/model",
            "http://127.0.0.1:43210/openai/v1",
            "fixture-key",
        )
        .unwrap();
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["home"], root.to_str().unwrap());
        assert_eq!(value["cwd"], home.path().to_str().unwrap());
        assert_eq!(value["model"], "private/model");
        assert_eq!(value["base"], "http://127.0.0.1:43210/openai/v1");
        assert_eq!(value["env_base"], value["base"]);
        assert_eq!(value["key"], "fixture-key");
        assert_eq!(value["remote"], "");
        assert_eq!(value["foreign"], false);
        assert_eq!(value["key_in_argv"], false);
        assert_eq!(std::fs::read(root.join(".env")).unwrap(), b"");
        drop(session);
        assert!(root.join("stopped").exists());
    }
}
