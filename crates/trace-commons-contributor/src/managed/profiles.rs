//! Native tools own their credentials. We choose an isolated root per account.

use super::{AccountId, AccountView, AuthState, ConnectionKind, ManagedError, ToolId};
use crate::config::ConfigStore;
use std::{
    path::{Path, PathBuf},
    process::Command,
};

pub enum NativeAction {
    Version,
    Run,
    Login,
    Status,
    Logout,
}

pub struct NativeProfiles {
    root: PathBuf,
}

const AUTH_ENV: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_PROFILE",
    "ANTHROPIC_FEDERATION_RULE_ID",
    "ANTHROPIC_ORGANIZATION_ID",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "NEARAI_API_KEY",
    "NEAR_AI_SESSION_API_KEY",
    "OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    "CODEX_API_KEY",
    "CODEX_ACCESS_TOKEN",
    "CODEX_HOME",
    "CLAUDE_CONFIG_DIR",
];

impl NativeProfiles {
    pub fn new(store: &ConfigStore) -> Self {
        Self {
            root: store.dir().join("managed-profiles"),
        }
    }

    pub fn profile_root(&self, id: AccountId) -> Result<PathBuf, ManagedError> {
        private_directory(&self.root)?;
        let path = self.root.join(id.to_string());
        private_directory(&path)?;
        Ok(path)
    }

    pub fn command(
        &self,
        account: &AccountView,
        native: &Path,
        action: NativeAction,
        cwd: &Path,
    ) -> Result<Command, ManagedError> {
        if !native.is_absolute() || !native.is_file() || !cwd.is_dir() {
            return Err(ManagedError::InvalidRequest);
        }
        let root = self.profile_root(account.id)?;
        if matches!(action, NativeAction::Run) {
            check_project_config(account.tool, cwd)?;
        }
        let mut command = Command::new(native);
        command.current_dir(cwd);
        for key in AUTH_ENV {
            command.env_remove(key);
        }
        match account.tool {
            ToolId::Claude => {
                command.env("CLAUDE_CONFIG_DIR", &root);
                match action {
                    NativeAction::Version => {
                        command.arg("--version");
                    }
                    NativeAction::Run => {}
                    NativeAction::Login => {
                        command.args(["auth", "login", "--claudeai"]);
                    }
                    NativeAction::Status => {
                        command.args(["auth", "status", "--json"]);
                    }
                    NativeAction::Logout => {
                        command.args(["auth", "logout"]);
                    }
                }
            }
            ToolId::Codex => {
                command.env("CODEX_HOME", &root);
                match action {
                    NativeAction::Version => {
                        command.arg("--version");
                    }
                    NativeAction::Run => {}
                    NativeAction::Login => {
                        command.arg("login");
                    }
                    NativeAction::Status => {
                        command.args(["login", "status"]);
                    }
                    NativeAction::Logout => {
                        command.arg("logout");
                    }
                }
            }
        }
        if matches!(action, NativeAction::Login)
            && account.connection != ConnectionKind::Subscription
        {
            return Err(ManagedError::InvalidRequest);
        }
        Ok(command)
    }

    pub fn remove(&self, account: &AccountView, native: &Path) -> Result<(), ManagedError> {
        self.verify_version(account, native)?;
        let root = self.profile_root(account.id)?;
        let result = bounded_output(self.command(account, native, NativeAction::Logout, &root)?)?;
        if !result.status.success() {
            return Err(ManagedError::StorageUnavailable);
        }
        self.remove_files(account.id)
    }

    pub fn remove_files(&self, id: AccountId) -> Result<(), ManagedError> {
        let root = self.profile_root(id)?;
        std::fs::remove_dir_all(root).map_err(|_| ManagedError::StorageUnavailable)
    }

    pub fn verify_version(&self, account: &AccountView, native: &Path) -> Result<(), ManagedError> {
        let root = self.profile_root(account.id)?;
        let version =
            bounded_output(self.command(account, native, NativeAction::Version, &root)?)?;
        if !version.status.success() {
            return Err(ManagedError::UnsupportedVersion);
        }
        let version = String::from_utf8_lossy(&version.stdout);
        let numeric = version
            .split_whitespace()
            .find_map(|part| {
                let numbers: Vec<_> = part
                    .split('.')
                    .map(str::parse::<u32>)
                    .collect::<Result<_, _>>()
                    .ok()?;
                if numbers.len() == 3 {
                    Some((numbers[0], numbers[1], numbers[2]))
                } else {
                    None
                }
            })
            .ok_or(ManagedError::UnsupportedVersion)?;
        let supported = match account.tool {
            ToolId::Claude => numeric.0 == 2 && numeric >= (2, 1, 289),
            ToolId::Codex => numeric.0 == 0 && numeric >= (0, 160, 0),
        };
        if !supported {
            return Err(ManagedError::UnsupportedVersion);
        }
        Ok(())
    }

    pub fn inspect(&self, account: &AccountView, native: &Path) -> Result<AuthState, ManagedError> {
        self.verify_version(account, native)?;
        let root = self.profile_root(account.id)?;
        let output = bounded_output(self.command(account, native, NativeAction::Status, &root)?)?;
        if !output.status.success() {
            return Ok(AuthState::SignInRequired);
        }
        let ready = match account.tool {
            ToolId::Claude => {
                let value: serde_json::Value = serde_json::from_slice(&output.stdout)
                    .map_err(|_| ManagedError::UnsupportedVersion)?;
                value["loggedIn"] == true && value["authMethod"] == "claude.ai"
            }
            ToolId::Codex => output
                .stdout
                .windows(b"Logged in using ChatGPT".len())
                .chain(output.stderr.windows(b"Logged in using ChatGPT".len()))
                .any(|s| s == b"Logged in using ChatGPT"),
        };
        Ok(if ready {
            AuthState::Ready
        } else {
            AuthState::SignInRequired
        })
    }
}

/// Status helpers may wait for native OS authentication. Bound the wait and
/// keep their identity-bearing output off the terminal and out of logs.
pub(crate) fn bounded_output(mut command: Command) -> Result<std::process::Output, ManagedError> {
    use std::io::{Read, Seek};
    use std::process::Stdio;
    let mut stdout = tempfile::tempfile().map_err(|_| ManagedError::StorageUnavailable)?;
    let mut stderr = tempfile::tempfile().map_err(|_| ManagedError::StorageUnavailable)?;
    command
        .stdin(Stdio::null())
        .stdout(
            stdout
                .try_clone()
                .map_err(|_| ManagedError::StorageUnavailable)?,
        )
        .stderr(
            stderr
                .try_clone()
                .map_err(|_| ManagedError::StorageUnavailable)?,
        );
    let mut child = command
        .spawn()
        .map_err(|_| ManagedError::UnsupportedVersion)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(25))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ManagedError::LaunchUnknown);
            }
        }
    };
    let read = |file: &mut std::fs::File| -> Result<Vec<u8>, ManagedError> {
        file.rewind()
            .map_err(|_| ManagedError::StorageUnavailable)?;
        let mut bytes = vec![];
        file.take(65537)
            .read_to_end(&mut bytes)
            .map_err(|_| ManagedError::StorageUnavailable)?;
        if bytes.len() > 65536 {
            return Err(ManagedError::UnsupportedVersion);
        }
        Ok(bytes)
    };
    Ok(std::process::Output {
        status,
        stdout: read(&mut stdout)?,
        stderr: read(&mut stderr)?,
    })
}

pub(crate) fn private_directory(path: &Path) -> Result<(), ManagedError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(ManagedError::StorageUnavailable);
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder
                .create(path)
                .map_err(|_| ManagedError::StorageUnavailable)?;
        }
        Err(_) => return Err(ManagedError::StorageUnavailable),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| ManagedError::StorageUnavailable)?;
    }
    Ok(())
}

/// Project configuration can outrank the isolated profile. Never claim a
/// selected subscription while a project requests a different credential.
fn check_project_config(tool: ToolId, cwd: &Path) -> Result<(), ManagedError> {
    let project_root = cwd
        .ancestors()
        .find(|p| p.join(".git").exists())
        .unwrap_or(cwd);
    for ancestor in cwd.ancestors() {
        let names: &[&str] = match tool {
            ToolId::Claude => &[".claude/settings.json", ".claude/settings.local.json"],
            ToolId::Codex => &[".codex/config.toml"],
        };
        for name in names {
            let path = ancestor.join(name);
            let bytes = match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => return Err(ManagedError::Conflict),
            };
            if bytes.len() > 1024 * 1024 {
                return Err(ManagedError::Conflict);
            }
            match tool {
                ToolId::Claude => {
                    let value: serde_json::Value =
                        serde_json::from_slice(&bytes).map_err(|_| ManagedError::Conflict)?;
                    if !value.is_object()
                        || value.get("apiKeyHelper").is_some()
                        || value.get("forceLoginMethod").is_some()
                        || value.get("forceLoginOrgUUID").is_some()
                        || value.get("forceLoginGatewayUrl").is_some()
                    {
                        return Err(ManagedError::Conflict);
                    }
                    if let Some(env) = value.get("env") {
                        let env = env.as_object().ok_or(ManagedError::Conflict)?;
                        if env.keys().any(|k| AUTH_ENV.contains(&k.as_str())) {
                            return Err(ManagedError::Conflict);
                        }
                    }
                }
                ToolId::Codex => {
                    let text = std::str::from_utf8(&bytes).map_err(|_| ManagedError::Conflict)?;
                    let value: toml::Value =
                        toml::from_str(text).map_err(|_| ManagedError::Conflict)?;
                    if codex_auth_override(&value) {
                        return Err(ManagedError::Conflict);
                    }
                }
            }
        }
        if ancestor == project_root {
            break;
        }
    }
    Ok(())
}

fn codex_auth_override(value: &toml::Value) -> bool {
    match value {
        toml::Value::Table(table) => table.iter().any(|(key, value)| {
            matches!(
                key.as_str(),
                "model_provider"
                    | "model_providers"
                    | "base_url"
                    | "env_key"
                    | "http_headers"
                    | "env_http_headers"
                    | "experimental_bearer_token"
                    | "forced_login_method"
                    | "forced_chatgpt_workspace_id"
                    | "chatgpt_base_url"
                    | "cli_auth_credentials_store"
                    | "CODEX_HOME"
            ) || codex_auth_override(value)
        }),
        toml::Value::Array(values) => values.iter().any(codex_auth_override),
        _ => false,
    }
}

pub fn find_native(tool: ToolId) -> Result<PathBuf, ManagedError> {
    let name = match tool {
        ToolId::Claude => "claude",
        ToolId::Codex => "codex",
    };
    let paths = std::env::var_os("PATH").unwrap_or_default();
    let mut folders: Vec<PathBuf> = std::env::split_paths(&paths).collect();
    if let Some(home) = dirs::home_dir() {
        folders.push(home.join(".local/bin"));
        folders.push(home.join(".cargo/bin"));
    }
    #[cfg(target_os = "macos")]
    folders.extend([
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ]);
    for folder in folders {
        // Relative PATH entries depend on the launch project and are not a
        // stable executable identity for the daemon/helper handoff.
        if !folder.is_absolute() {
            continue;
        }
        let file = folder.join(if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.into()
        });
        if file.is_file() {
            let resolved =
                std::fs::canonicalize(file).map_err(|_| ManagedError::UnsupportedVersion)?;
            if std::env::current_exe()
                .ok()
                .and_then(|p| std::fs::canonicalize(p).ok())
                .as_ref()
                != Some(&resolved)
            {
                return Ok(resolved);
            }
        }
    }
    Err(ManagedError::UnsupportedVersion)
}
