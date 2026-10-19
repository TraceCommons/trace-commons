//! Qualified OpenCode 1.19.0 managed profile and exact-session export.
//! Standard profiles are never read or changed. Exports are local transcripts,
//! not evidence of attestation or permission to upload.
use super::{
    AccountView, ManagedError,
    profiles::{NativeProfiles, bounded_output_limit, private_directory},
};
use crate::{config::ConfigStore, source::opencode::parse_export};
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    process::Command,
};
use uuid::Uuid;

pub const STAGING: &str = "managed-opencode-exports";
const EXPORT_LIMIT: u64 = 16 * 1024 * 1024;

pub fn valid_model(model: &str) -> bool {
    !model.is_empty()
        && model.len() <= 256
        && model
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"/._:-".contains(&c))
}

/// Every native invocation gets isolated config, state, auth, cache and data.
/// Pure mode prevents external plugins from changing the selected provider.
pub(crate) fn isolate(command: &mut Command, root: &Path) -> Result<(), ManagedError> {
    private_directory(root)?;
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("OPENCODE_") {
            command.env_remove(key);
        }
    }
    for key in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "OPENAI_API_KEY",
        "OPENAI_BASE_URL",
        "NEARAI_API_KEY",
        "NEAR_AI_SESSION_API_KEY",
        "TRACE_COMMONS_OPENCODE_KEY",
    ] {
        command.env_remove(key);
    }
    command.env("HOME", root).env("USERPROFILE", root);
    for (key, dir) in [
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_DATA_HOME", "data"),
        ("XDG_STATE_HOME", "state"),
        ("XDG_CACHE_HOME", "cache"),
    ] {
        let path = root.join(dir);
        private_directory(&path)?;
        command.env(key, path);
    }
    command
        .arg("--pure")
        .env("OPENCODE_DISABLE_PROJECT_CONFIG", "true")
        .env("OPENCODE_DISABLE_AUTOUPDATE", "true")
        .env("OPENCODE_DISABLE_MODELS_FETCH", "true")
        .env("OPENCODE_EXPERIMENTAL_DISABLE_FILEWATCHER", "true")
        .env(
            "OPENCODE_CONFIG_CONTENT",
            "{\"share\":\"disabled\",\"enabled_providers\":[]}",
        );
    Ok(())
}

pub struct OpenCodeSession {
    root: PathBuf,
    native: PathBuf,
    cwd: PathBuf,
    id: Uuid,
}

impl OpenCodeSession {
    pub fn new(
        store: &ConfigStore,
        account: &AccountView,
        native: &Path,
        id: Uuid,
        cwd: &Path,
    ) -> Result<Self, ManagedError> {
        if !native.is_absolute() || !native.is_file() || !cwd.is_absolute() || !cwd.is_dir() {
            return Err(ManagedError::InvalidRequest);
        }
        let account_root = NativeProfiles::new(store).profile_root(account.id)?;
        let root = account_root.join(id.to_string());
        private_directory(&root)?;
        Ok(Self {
            root,
            native: native.into(),
            cwd: std::fs::canonicalize(cwd).map_err(|_| ManagedError::InvalidRequest)?,
            id,
        })
    }

    fn session_id(&self) -> String {
        format!("ses_{}", self.id.simple())
    }

    fn base_command(&self) -> Result<Command, ManagedError> {
        let mut command = Command::new(&self.native);
        command.current_dir(&self.cwd);
        isolate(&mut command, &self.root)?;
        Ok(command)
    }

    /// Seed a new, empty session through OpenCode's supported import command.
    /// Its exact ID binds the launch and subsequent export without time guesses.
    pub fn prepare(&self) -> Result<(), ManagedError> {
        let now = chrono::Utc::now().timestamp_millis();
        let seed = json!({"info":{"id":self.session_id(),"slug":"managed-session","projectID":"global","directory":self.cwd,"title":"Managed session","version":"1.19.0","time":{"created":now,"updated":now}},"messages":[]});
        let profile =
            ConfigStore::open(self.root.clone()).map_err(|_| ManagedError::StorageUnavailable)?;
        profile
            .write_daemon_file(
                "seed.json",
                &serde_json::to_vec(&seed).map_err(|_| ManagedError::InvalidRequest)?,
            )
            .map_err(|_| ManagedError::StorageUnavailable)?;
        let mut command = self.base_command()?;
        command.arg("import").arg(self.root.join("seed.json"));
        let output = bounded_output_limit(command, 65536)?;
        if !output.status.success() {
            return Err(ManagedError::LaunchUnknown);
        }
        // Import may return success after a refusal. Export confirms identity.
        self.export()?;
        Ok(())
    }

    /// Run on `model`, the only model offered.
    pub fn command(&self, model: &str, base_url: &str, key: &str) -> Result<Command, ManagedError> {
        let only = [super::models::ChatModel {
            id: model.to_owned(),
            name: None,
        }];
        self.command_with_models(model, &only, base_url, key)
    }

    /// Run on `default`, offering every model in `models` (which must hold
    /// it) in OpenCode's own picker, each under its display name or id.
    /// The profile is isolated, so the list never reaches the person's own
    /// OpenCode settings.
    pub fn command_with_models(
        &self,
        default: &str,
        models: &[super::models::ChatModel],
        base_url: &str,
        key: &str,
    ) -> Result<Command, ManagedError> {
        if !models.iter().any(|m| m.id == default) || !models.iter().all(|m| valid_model(&m.id)) {
            return Err(ManagedError::InvalidRequest);
        }
        let mut command = self.base_command()?;
        let selected = format!("trace-managed/{default}");
        let listed: serde_json::Map<String, serde_json::Value> = models
            .iter()
            .map(|m| {
                let name = m.name.as_deref().unwrap_or(&m.id);
                (m.id.clone(), json!({ "name": name }))
            })
            .collect();
        let config = json!({
            "model":selected,"small_model":selected,"share":"disabled",
            "enabled_providers":["trace-managed"],
            "provider":{"trace-managed":{"npm":"@ai-sdk/openai-compatible","name":"Managed connection",
                "options":{"baseURL":base_url,"apiKey":"{env:TRACE_COMMONS_OPENCODE_KEY}"},
                "models":listed
            }}
        });
        command
            .env("OPENCODE_CONFIG_CONTENT", config.to_string())
            .env("TRACE_COMMONS_OPENCODE_KEY", key)
            .args(["--session", &self.session_id(), "--model", &selected]);
        Ok(command)
    }

    fn export(&self) -> Result<Vec<u8>, ManagedError> {
        let mut command = self.base_command()?;
        command.args(["export", &self.session_id()]);
        let output = bounded_output_limit(command, EXPORT_LIMIT)?;
        if !output.status.success() {
            return Err(ManagedError::LaunchUnknown);
        }
        let value: serde_json::Value =
            serde_json::from_slice(&output.stdout).map_err(|_| ManagedError::InvalidRequest)?;
        if value["info"]["id"].as_str() != Some(self.session_id().as_str())
            || value["info"]["directory"].as_str() != self.cwd.to_str()
        {
            return Err(ManagedError::InvalidRequest);
        }
        parse_export(&output.stdout).map_err(|_| ManagedError::InvalidRequest)?;
        Ok(output.stdout)
    }

    /// Validate completely before atomic staging. Failure preserves the native
    /// profile and any previously staged bytes for explicit local recovery.
    pub fn collect(&self, store: &ConfigStore) -> Result<(), ManagedError> {
        let bytes = self.export()?;
        let root = store.dir().join(STAGING);
        private_directory(&root)?;
        let staging = ConfigStore::open(root).map_err(|_| ManagedError::StorageUnavailable)?;
        staging
            .write_daemon_file(&format!("{}.json", self.id), &bytes)
            .map_err(|_| ManagedError::StorageUnavailable)
    }
}
