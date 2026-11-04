//! Version-aware OpenCode managed profiles and exact-session export.
//! Standard profiles are never read or changed. Exports are local transcripts,
//! not evidence of attestation or permission to upload.
use super::{
    AccountView, ConnectionKind, ManagedError,
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
/// Project configuration is disabled; version-specific flags are applied later.
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
        .env("OPENCODE_DISABLE_PROJECT_CONFIG", "true")
        .env("OPENCODE_DISABLE_CLAUDE_CODE", "true")
        .env("OPENCODE_DISABLE_FILEWATCHER", "true")
        .env("OPENCODE_DISABLE_AUTOUPDATE", "true")
        .env("OPENCODE_DISABLE_MODELS_FETCH", "true")
        .env("OPENCODE_EXPERIMENTAL_DISABLE_FILEWATCHER", "true")
        .env(
            "OPENCODE_CONFIG_CONTENT",
            "{\"share\":\"disabled\",\"enabled_providers\":[]}",
        );
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Version {
    V1,
    V2,
}

impl Version {
    pub(crate) fn from_native(version: Option<(u32, u32, u32)>) -> Result<Self, ManagedError> {
        match version.map(|v| v.0) {
            Some(1) => Ok(Self::V1),
            Some(2) => Ok(Self::V2),
            _ => Err(ManagedError::UnsupportedVersion),
        }
    }

    pub(crate) fn interactive(self, command: &mut Command) {
        command.arg(match self {
            Self::V1 => "--pure",
            Self::V2 => "--standalone",
        });
    }
}

// v2 has no --pure or CLI switch to ignore machine/MDM configuration.
// Those settings outrank inline configuration, including provider and plugin
// policy. Refuse them rather than launch outside the selected managed route.
fn ensure_v2_system_configuration_absent() -> Result<(), ManagedError> {
    #[cfg(target_os = "macos")]
    let paths = {
        let mut identity = Command::new("/usr/bin/id");
        identity.arg("-un");
        let output = bounded_output_limit(identity, 1024)?;
        let username = std::str::from_utf8(&output.stdout)
            .ok()
            .map(str::trim)
            .filter(|s| !s.is_empty() && Path::new(s).components().count() == 1)
            .filter(|s| *s != "." && *s != "..")
            .ok_or(ManagedError::SystemConfiguration)?;
        if !output.status.success() {
            return Err(ManagedError::SystemConfiguration);
        }
        vec![
            PathBuf::from("/Library/Application Support/opencode"),
            PathBuf::from("/Library/Managed Preferences/ai.opencode.managed.plist"),
            PathBuf::from("/Library/Managed Preferences")
                .join(username)
                .join("ai.opencode.managed.plist"),
        ]
    };
    #[cfg(windows)]
    let paths = vec![
        std::env::var_os("ProgramData")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
            .join("opencode"),
    ];
    #[cfg(not(any(target_os = "macos", windows)))]
    let paths = vec![PathBuf::from("/etc/opencode")];
    require_absent(&paths)
}

fn require_absent(paths: &[PathBuf]) -> Result<(), ManagedError> {
    for path in paths {
        match std::fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err(ManagedError::SystemConfiguration),
        }
    }
    Ok(())
}

/// OpenCode's output limit for a model it does not know, used when NEAR AI
/// lists a window but no output limit.
const OPENCODE_DEFAULT_OUTPUT: u64 = 32_000;

/// A model's `limit` in OpenCode's config: NEAR AI's window and output
/// limit, so OpenCode compacts before NEAR AI refuses a request as too
/// long. Without it OpenCode assumes 200,000 and 32,000 for a model it
/// does not know. None when NEAR AI lists no window.
fn limit(model: &super::models::ChatModel) -> Option<serde_json::Value> {
    let context = model.context_length?;
    let output = model
        .max_output_length
        .unwrap_or(OPENCODE_DEFAULT_OUTPUT)
        .min(context);
    Some(json!({ "context": context, "output": output }))
}

/// The title OpenCode shows for a managed session, also in its terminal tab
/// ("OC | <title>"). Only inference through NEAR AI is private.
fn session_title(connection: ConnectionKind) -> &'static str {
    match connection {
        ConnectionKind::NearAi | ConnectionKind::NearAiSignIn => "NEAR AI private session",
        ConnectionKind::ApiKey | ConnectionKind::Subscription => "Managed session",
    }
}

pub struct OpenCodeSession {
    root: PathBuf,
    title: &'static str,
    native: PathBuf,
    cwd: PathBuf,
    id: Uuid,
    version: Version,
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
        let profiles = NativeProfiles::new(store);
        let version = Version::from_native(profiles.native_version(account, native)?)?;
        let account_root = profiles.profile_root(account.id)?;
        let root = account_root.join(id.to_string());
        private_directory(&root)?;
        Ok(Self {
            root,
            title: session_title(account.connection),
            native: native.into(),
            cwd: std::fs::canonicalize(cwd).map_err(|_| ManagedError::InvalidRequest)?,
            id,
            version,
        })
    }

    fn session_id(&self) -> String {
        format!("ses_{}", self.id.simple())
    }

    fn base_command(&self) -> Result<Command, ManagedError> {
        let mut command = Command::new(&self.native);
        command.current_dir(&self.cwd);
        isolate(&mut command, &self.root)?;
        if self.version == Version::V2 {
            ensure_v2_system_configuration_absent()?;
        }
        Ok(command)
    }

    /// Seed a new, empty session through OpenCode's supported import command.
    /// Its exact ID binds the launch and subsequent export without time guesses.
    pub fn prepare(&self) -> Result<(), ManagedError> {
        let now = chrono::Utc::now().timestamp_millis();
        let seed = match self.version {
            Version::V1 => {
                json!({"info":{"id":self.session_id(),"slug":"managed-session","projectID":"global","directory":self.cwd,"title":self.title,"version":"1.19.0","time":{"created":now,"updated":now}},"messages":[]})
            }
            Version::V2 => {
                json!({"info":{"id":self.session_id(),"projectID":"global","location":{"directory":self.cwd},"title":self.title,"cost":0,"tokens":{"input":0,"output":0,"reasoning":0,"cache":{"read":0,"write":0}},"time":{"created":now,"updated":now}},"messages":[]})
            }
        };
        let profile =
            ConfigStore::open(self.root.clone()).map_err(|_| ManagedError::StorageUnavailable)?;
        profile
            .write_daemon_file(
                "seed.json",
                &serde_json::to_vec(&seed).map_err(|_| ManagedError::InvalidRequest)?,
            )
            .map_err(|_| ManagedError::StorageUnavailable)?;
        let mut command = self.base_command()?;
        match self.version {
            Version::V1 => {
                command
                    .args(["--pure", "import"])
                    .arg(self.root.join("seed.json"));
            }
            Version::V2 => {
                command
                    .args(["session", "import", "--standalone", "--directory"])
                    .arg(&self.cwd)
                    .arg(self.root.join("seed.json"));
            }
        }
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
            tier: None,
            ..Default::default()
        }];
        self.command_with_models(model, None, &only, base_url, key)
    }

    /// Run on `default`, offering every model in `models` (which must hold
    /// it) in OpenCode's own picker, each under its display name or id.
    /// The profile is isolated, so the list never reaches the person's own
    /// OpenCode settings. The background model (v1's `small_model`, which
    /// v2 reads as its title agent's model) is `background` when it is no
    /// less private than `default`, else OpenCode's background default
    /// ([`super::models::background_for`]); compaction stays on `default`.
    pub fn command_with_models(
        &self,
        default: &str,
        background: Option<&str>,
        models: &[super::models::ChatModel],
        base_url: &str,
        key: &str,
    ) -> Result<Command, ManagedError> {
        if !models.iter().any(|m| m.id == default) || !models.iter().all(|m| valid_model(&m.id)) {
            return Err(ManagedError::InvalidRequest);
        }
        let mut command = self.base_command()?;
        let selected = format!("trace-managed/{default}");
        let small =
            super::models::background_for(super::ToolId::Opencode, default, background, models)
                .map_or(default, |m| m.id.as_str());
        let small = format!("trace-managed/{small}");
        let listed: serde_json::Map<String, serde_json::Value> = models
            .iter()
            .map(|m| {
                let name = m.name.as_deref().unwrap_or(&m.id);
                let mut entry = json!({ "name": name });
                if let Some(limit) = limit(m) {
                    entry["limit"] = limit;
                }
                (m.id.clone(), entry)
            })
            .collect();
        let mut options = json!({"baseURL":base_url,"apiKey":"{env:TRACE_COMMONS_OPENCODE_KEY}"});
        // Only the host-owned loopback route needs a transcript join key.
        // Direct saved-key providers must not receive the native session id.
        if reqwest::Url::parse(base_url).is_ok_and(|url| {
            url.scheme() == "http" && url.host_str() == Some("127.0.0.1") && url.port().is_some()
        }) {
            options["headers"] = json!({"x-ironwire-session-id":self.session_id()});
        }
        let config = match self.version {
            Version::V1 => json!({
                "model":selected,"small_model":small,"share":"disabled",
                "enabled_providers":["trace-managed"],
                "provider":{"trace-managed":{"npm":"@ai-sdk/openai-compatible","name":"Managed connection",
                    "options":options,"models":listed
                }}
            }),
            Version::V2 => {
                let mut settings = options;
                let headers = settings.as_object_mut().and_then(|s| s.remove("headers"));
                let mut config = json!({
                    "model":selected,"share":"disabled","update":"disable","plugins":[],
                    "policies":[
                        {"action":"provider.use","resource":"*","effect":"deny"},
                        {"action":"provider.use","resource":"trace-managed","effect":"allow"},
                        {"action":"integration.use","resource":"plugin:*","effect":"deny"}
                    ],
                    "agents":{"title":{"model":small},"compaction":{"model":selected}},
                    "providers":{"trace-managed":{"package":"@ai-sdk/openai-compatible","name":"Managed connection",
                        "settings":settings,"models":listed
                    }}
                });
                if let Some(headers) = headers {
                    config["providers"]["trace-managed"]["headers"] = headers;
                }
                config
            }
        };
        command
            .env("OPENCODE_CONFIG_CONTENT", config.to_string())
            .env("TRACE_COMMONS_OPENCODE_KEY", key);
        self.version.interactive(&mut command);
        command.args(["--session", &self.session_id()]);
        // v2 takes --model on `run`, but not on the interactive root command.
        // Its explicit model selection lives in the isolated configuration.
        if self.version == Version::V1 {
            command.args(["--model", &selected]);
        }
        Ok(command)
    }

    fn export(&self) -> Result<Vec<u8>, ManagedError> {
        let mut command = self.base_command()?;
        match self.version {
            Version::V1 => {
                command.args(["--pure", "export", &self.session_id()]);
            }
            Version::V2 => {
                command.args(["session", "export", "--standalone", &self.session_id()]);
            }
        }
        let output = bounded_output_limit(command, EXPORT_LIMIT)?;
        if !output.status.success() {
            return Err(ManagedError::LaunchUnknown);
        }
        let value: serde_json::Value =
            serde_json::from_slice(&output.stdout).map_err(|_| ManagedError::InvalidRequest)?;
        let directory = match self.version {
            Version::V1 => &value["info"]["directory"],
            Version::V2 => &value["info"]["location"]["directory"],
        };
        if value["info"]["id"].as_str() != Some(self.session_id().as_str())
            || directory.as_str() != self.cwd.to_str()
        {
            return Err(ManagedError::InvalidRequest);
        }
        parse_export(&output.stdout).map_err(|_| ManagedError::InvalidRequest)?;
        Ok(output.stdout)
    }

    /// Validate completely before atomic staging. Failure preserves the native
    /// profile and any previously staged bytes for explicit local recovery.
    pub fn collect(&self, store: &ConfigStore) -> Result<(), ManagedError> {
        self.collect_as(store, self.id)
    }

    /// What collecting this session needs again, for a collection plan.
    pub fn plan(&self) -> super::collection_plan::OpenCodePlan {
        super::collection_plan::OpenCodePlan {
            native: self.native.clone(),
            session: self.id,
            version: self.version,
        }
    }

    /// The session a collection plan names, to export it again. Its
    /// directory is derived from the account and must already be there;
    /// nothing is created or seeded.
    pub fn reopen(
        store: &ConfigStore,
        account: super::AccountId,
        plan: &super::collection_plan::OpenCodePlan,
        cwd: &Path,
    ) -> Result<Self, ManagedError> {
        if !plan.native.is_absolute()
            || !plan.native.is_file()
            || !cwd.is_absolute()
            || !cwd.is_dir()
        {
            return Err(ManagedError::InvalidRequest);
        }
        let root = NativeProfiles::new(store)
            .profile_root(account)?
            .join(plan.session.to_string());
        if !std::fs::symlink_metadata(&root)
            .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
        {
            return Err(ManagedError::NotFound);
        }
        Ok(Self {
            root,
            // A reopened session is only exported, never seeded again.
            title: session_title(ConnectionKind::ApiKey),
            native: plan.native.clone(),
            cwd: std::fs::canonicalize(cwd).map_err(|_| ManagedError::InvalidRequest)?,
            id: plan.session,
            version: plan.version,
        })
    }

    /// [`Self::collect`], staged as `<row>.json`: a resumed session's row
    /// exports its root's whole conversation without replacing the root
    /// row's earlier export.
    pub fn collect_as(&self, store: &ConfigStore, row: Uuid) -> Result<(), ManagedError> {
        let bytes = self.export()?;
        let root = store.dir().join(STAGING);
        private_directory(&root)?;
        let staging = ConfigStore::open(root).map_err(|_| ManagedError::StorageUnavailable)?;
        staging
            .write_daemon_file(&format!("{row}.json"), &bytes)
            .map_err(|_| ManagedError::StorageUnavailable)
    }
}

/// Whether row `row`'s export was staged ([`OpenCodeSession::collect_as`]).
pub fn exported(store: &ConfigStore, row: Uuid) -> bool {
    std::fs::symlink_metadata(store.dir().join(STAGING).join(format!("{row}.json")))
        .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// OpenCode shows the session title in its terminal tab ("OC | <title>").
    /// Only a session whose inference runs through NEAR AI is called private.
    #[test]
    fn session_title_names_near_ai_private_sessions_only() {
        assert_eq!(
            session_title(ConnectionKind::NearAiSignIn),
            "NEAR AI private session"
        );
        assert_eq!(
            session_title(ConnectionKind::NearAi),
            "NEAR AI private session"
        );
        assert_eq!(session_title(ConnectionKind::ApiKey), "Managed session");
        assert_eq!(
            session_title(ConnectionKind::Subscription),
            "Managed session"
        );
    }

    #[test]
    fn v2_refuses_external_system_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("system");
        assert!(require_absent(std::slice::from_ref(&path)).is_ok());
        std::fs::create_dir(&path).unwrap();
        assert!(matches!(
            require_absent(std::slice::from_ref(&path)),
            Err(ManagedError::SystemConfiguration)
        ));
        std::fs::remove_dir(&path).unwrap();
        std::fs::write(&path, "{}").unwrap();
        assert!(matches!(
            require_absent(std::slice::from_ref(&path)),
            Err(ManagedError::SystemConfiguration)
        ));
        #[cfg(unix)]
        {
            std::fs::remove_file(&path).unwrap();
            std::os::unix::fs::symlink(dir.path().join("missing"), &path).unwrap();
            assert!(matches!(
                require_absent(&[path]),
                Err(ManagedError::SystemConfiguration)
            ));
        }
    }

    /// Reopening for collection creates nothing: the session's directory
    /// must already be there, and the executable and folder must be real.
    #[test]
    fn a_session_reopened_for_collection_must_already_exist() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(dir.path().join("state")).unwrap();
        let account = super::super::AccountId(Uuid::new_v4());
        let native = dir.path().join("opencode");
        std::fs::write(&native, b"").unwrap();
        let plan = super::super::collection_plan::OpenCodePlan {
            native: native.clone(),
            session: Uuid::new_v4(),
            version: Version::V2,
        };
        let cwd = dir.path();
        assert!(matches!(
            OpenCodeSession::reopen(&store, account, &plan, cwd),
            Err(ManagedError::NotFound)
        ));
        let root = NativeProfiles::new(&store)
            .profile_root(account)
            .unwrap()
            .join(plan.session.to_string());
        assert!(!root.exists(), "nothing is created");
        std::fs::create_dir(&root).unwrap();
        let session = OpenCodeSession::reopen(&store, account, &plan, cwd).unwrap();
        assert_eq!(session.plan(), plan);
        assert_eq!(
            session.session_id(),
            format!("ses_{}", plan.session.simple())
        );
        for bad in [
            super::super::collection_plan::OpenCodePlan {
                native: "opencode".into(),
                ..plan.clone()
            },
            super::super::collection_plan::OpenCodePlan {
                native: dir.path().into(),
                ..plan.clone()
            },
        ] {
            assert!(matches!(
                OpenCodeSession::reopen(&store, account, &bad, cwd),
                Err(ManagedError::InvalidRequest)
            ));
        }
        assert!(matches!(
            OpenCodeSession::reopen(&store, account, &plan, std::path::Path::new("relative")),
            Err(ManagedError::InvalidRequest)
        ));
    }

    #[test]
    fn v2_uses_private_server_and_native_provider_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let session = OpenCodeSession {
            title: "Managed session",
            root: dir.path().join("profile"),
            native: dir.path().join("opencode"),
            cwd: dir.path().into(),
            id: Uuid::new_v4(),
            version: Version::V2,
        };
        let command = session
            .command("fixture-model", "http://127.0.0.1:4321/v1", "synthetic-key")
            .unwrap();
        let args: Vec<_> = command.get_args().map(|v| v.to_str().unwrap()).collect();
        assert_eq!(args, ["--standalone", "--session", &session.session_id()]);
        let config = command
            .get_envs()
            .find(|(key, _)| *key == "OPENCODE_CONFIG_CONTENT")
            .and_then(|(_, v)| v)
            .unwrap();
        let config: serde_json::Value = serde_json::from_str(config.to_str().unwrap()).unwrap();
        assert_eq!(config["model"], "trace-managed/fixture-model");
        assert_eq!(config["agents"]["title"]["model"], config["model"]);
        assert_eq!(config["agents"]["compaction"]["model"], config["model"]);
        let provider = &config["providers"]["trace-managed"];
        assert_eq!(provider["package"], "@ai-sdk/openai-compatible");
        assert_eq!(provider["settings"]["baseURL"], "http://127.0.0.1:4321/v1");
        assert_eq!(
            provider["settings"]["apiKey"],
            "{env:TRACE_COMMONS_OPENCODE_KEY}"
        );
        assert_eq!(
            provider["headers"]["x-ironwire-session-id"],
            session.session_id()
        );
        assert_eq!(
            config["policies"],
            json!([
                {"action":"provider.use","resource":"*","effect":"deny"},
                {"action":"provider.use","resource":"trace-managed","effect":"allow"},
                {"action":"integration.use","resource":"plugin:*","effect":"deny"}
            ])
        );
        assert_eq!(config["plugins"], json!([]));
        assert!(!config.to_string().contains("synthetic-key"));
        let direct = session
            .command(
                "fixture-model",
                "https://api.openai.com/v1",
                "synthetic-key",
            )
            .unwrap();
        let direct = direct
            .get_envs()
            .find(|(key, _)| *key == "OPENCODE_CONFIG_CONTENT")
            .and_then(|(_, v)| v)
            .unwrap();
        let direct: serde_json::Value = serde_json::from_str(direct.to_str().unwrap()).unwrap();
        assert!(
            direct["providers"]["trace-managed"]
                .get("headers")
                .is_none()
        );
    }

    /// OpenCode's background slots (v1's `small_model`, v2's title agent)
    /// take the background model only when it is no less private than the
    /// main model; a stale Incognito choice beside a NEAR AI private main
    /// model is replaced, and compaction stays on the main model.
    #[test]
    fn opencode_background_slots_are_never_less_private_than_the_model() {
        use super::super::models::{ChatModel, ModelTier};
        let models: Vec<ChatModel> = [
            ("anthropic/claude-haiku-4-5", ModelTier::Incognito),
            ("Qwen/Qwen3.8-27B", ModelTier::NearAi),
            ("z-ai/glm-5.3-flash", ModelTier::NearAi),
        ]
        .into_iter()
        .map(|(id, tier)| ChatModel {
            id: id.into(),
            tier: Some(tier),
            ..Default::default()
        })
        .collect();
        let dir = tempfile::tempdir().unwrap();
        for version in [Version::V1, Version::V2] {
            let session = OpenCodeSession {
                root: dir.path().join("profile"),
                title: "Managed session",
                native: dir.path().join("opencode"),
                cwd: dir.path().into(),
                id: Uuid::new_v4(),
                version,
            };
            for (background, expected) in [
                (None, "z-ai/glm-5.3-flash"),
                (Some("anthropic/claude-haiku-4-5"), "z-ai/glm-5.3-flash"),
                (Some("Qwen/Qwen3.8-27B"), "Qwen/Qwen3.8-27B"),
            ] {
                let command = session
                    .command_with_models(
                        "Qwen/Qwen3.8-27B",
                        background,
                        &models,
                        "http://127.0.0.1:4321/openai/v1",
                        "k",
                    )
                    .unwrap();
                let config = command
                    .get_envs()
                    .find(|(key, _)| *key == "OPENCODE_CONFIG_CONTENT")
                    .and_then(|(_, v)| v)
                    .unwrap();
                let config: serde_json::Value =
                    serde_json::from_str(config.to_str().unwrap()).unwrap();
                let expected = format!("trace-managed/{expected}");
                assert_eq!(config["model"], "trace-managed/Qwen/Qwen3.8-27B");
                match version {
                    Version::V1 => assert_eq!(config["small_model"], expected),
                    Version::V2 => {
                        assert_eq!(config["agents"]["title"]["model"], expected);
                        assert_eq!(config["agents"]["compaction"]["model"], config["model"]);
                    }
                }
                assert!(
                    !config.to_string().contains("\"trace-managed/anthropic/"),
                    "{config}"
                );
            }
        }
    }

    #[test]
    fn provider_header_binds_the_exact_seeded_session() {
        let dir = tempfile::tempdir().unwrap();
        let session = OpenCodeSession {
            title: "Managed session",
            root: dir.path().join("profile"),
            native: dir.path().join("opencode"),
            cwd: dir.path().into(),
            id: Uuid::new_v4(),
            version: Version::V1,
        };
        let command = session
            .command_with_models(
                "fixture-model",
                None,
                &[super::super::models::ChatModel {
                    id: "fixture-model".into(),
                    name: None,
                    tier: None,
                    ..Default::default()
                }],
                "http://127.0.0.1:4321/openai/v1",
                "fixture-key",
            )
            .unwrap();
        let config = command
            .get_envs()
            .find(|(key, _)| *key == "OPENCODE_CONFIG_CONTENT")
            .and_then(|(_, value)| value)
            .unwrap();
        let config: serde_json::Value = serde_json::from_str(config.to_str().unwrap()).unwrap();
        assert_eq!(
            config["provider"]["trace-managed"]["options"]["headers"]["x-ironwire-session-id"],
            session.session_id()
        );
        let args: Vec<_> = command.get_args().map(|v| v.to_str().unwrap()).collect();
        let session_arg = args.windows(2).find(|pair| pair[0] == "--session").unwrap();
        assert_eq!(session_arg[1], session.session_id());
        let direct = session
            .command_with_models(
                "fixture-model",
                None,
                &[super::super::models::ChatModel {
                    id: "fixture-model".into(),
                    name: None,
                    tier: None,
                    ..Default::default()
                }],
                "https://api.openai.com/v1",
                "fixture-key",
            )
            .unwrap();
        let direct_config = direct
            .get_envs()
            .find(|(key, _)| *key == "OPENCODE_CONFIG_CONTENT")
            .and_then(|(_, value)| value)
            .unwrap();
        let direct_config: serde_json::Value =
            serde_json::from_str(direct_config.to_str().unwrap()).unwrap();
        assert!(
            direct_config["provider"]["trace-managed"]["options"]
                .get("headers")
                .is_none()
        );
    }

    /// Each listed model carries NEAR AI's window and output limit in both
    /// config versions; a model NEAR AI gives no window for carries none,
    /// and a missing output limit is OpenCode's own, never above the window.
    #[test]
    fn models_carry_near_ai_context_limits() {
        let dir = tempfile::tempdir().unwrap();
        let model =
            |id: &str, context: Option<u64>, output: Option<u64>| super::super::models::ChatModel {
                id: id.into(),
                context_length: context,
                max_output_length: output,
                ..Default::default()
            };
        let models = [
            model("a/sized", Some(1_048_576), Some(128_000)),
            model("b/no-output", Some(262_144), None),
            model("c/tiny", Some(16_384), None),
            model("d/unsized", None, Some(8_192)),
        ];
        for (version, key) in [(Version::V1, "provider"), (Version::V2, "providers")] {
            let session = OpenCodeSession {
                title: "Managed session",
                root: dir.path().join("profile"),
                native: dir.path().join("opencode"),
                cwd: dir.path().into(),
                id: Uuid::new_v4(),
                version,
            };
            if version == Version::V2 && ensure_v2_system_configuration_absent().is_err() {
                continue;
            }
            let command = session
                .command_with_models(
                    "a/sized",
                    None,
                    &models,
                    "http://127.0.0.1:4321/openai/v1",
                    "k",
                )
                .unwrap();
            let config = command
                .get_envs()
                .find(|(key, _)| *key == "OPENCODE_CONFIG_CONTENT")
                .and_then(|(_, value)| value)
                .unwrap();
            let config: serde_json::Value = serde_json::from_str(config.to_str().unwrap()).unwrap();
            let listed = &config[key]["trace-managed"]["models"];
            assert_eq!(
                listed["a/sized"]["limit"],
                json!({"context": 1_048_576, "output": 128_000})
            );
            assert_eq!(
                listed["b/no-output"]["limit"],
                json!({"context": 262_144, "output": 32_000})
            );
            assert_eq!(
                listed["c/tiny"]["limit"],
                json!({"context": 16_384, "output": 16_384})
            );
            assert!(listed["d/unsized"].get("limit").is_none());
        }
    }
}
