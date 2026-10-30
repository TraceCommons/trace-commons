//! Persistent OpenCode routing. The user's config remains OpenCode's schema;
//! a private adjacent receipt records only the slots this connection changed.
//! Neither managed profiles nor their provider policies are used here.

use super::*;
use crate::managed::{ToolId, models::ChatModel, profiles};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{Read, Write};

pub(super) const ID: &str = "opencode";
const PROVIDER: &str = crate::channel::STATE_DIR_NAME;
const LIMIT: usize = 1024 * 1024;
pub(super) const ERR_VERSION: &str = "harness-opencode-version";
/// No OpenCode executable was found to ask its version, as when the binary
/// was removed and only its config folder (which alone counts as installed
/// in `tool`) is left.
pub(super) const ERR_NO_EXECUTABLE: &str = "harness-opencode-no-executable";
pub(super) const ERR_MODELS: &str = "harness-models-unavailable";

pub(super) struct Context {
    pub major: u32,
    pub models: Vec<ChatModel>,
}

fn receipt_name() -> String {
    format!(".{PROVIDER}-routing.json")
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Slot {
    path: Vec<String>,
    // An omitted previous value means the key was absent. Explicit JSON null
    // is a present value and must survive the persisted receipt round trip.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_value"
    )]
    before: Option<Value>,
    installed: Value,
}

fn present_value<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    version: u32,
    major: u32,
    slots: Vec<Slot>,
    containers: Vec<Vec<String>>,
}

pub(super) struct OpenCodeEdit {
    pub path: PathBuf,
    pub changes: Vec<String>,
    pub occupied: Vec<(String, String)>,
    existing: Option<String>,
    contents: String,
    receipt_path: PathBuf,
    receipt_before: Option<String>,
    receipt_after: Option<String>,
    other_channel_receipt: Option<PathBuf>,
}

fn directory() -> Option<PathBuf> {
    #[cfg(test)]
    return TEST_DIRECTORY.with(|p| p.borrow().clone());
    #[cfg(not(test))]
    {
        let root = match std::env::var_os("XDG_CONFIG_HOME") {
            Some(root) if !root.is_empty() => {
                let root = PathBuf::from(root);
                if !root.is_absolute() {
                    return None;
                }
                root
            }
            _ => dirs::home_dir()?.join(".config"),
        };
        Some(root.join(ID))
    }
}

fn config_path(dir: &Path) -> PathBuf {
    // Never create a competing .json beside an existing .jsonc. Even a valid
    // JSON .jsonc can be edited, but actual comments/trailing commas refuse.
    let jsonc = dir.join("opencode.jsonc");
    if jsonc.exists() {
        jsonc
    } else {
        dir.join("opencode.json")
    }
}

fn read(path: &Path) -> Result<Option<String>, PlanOutcome> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() && meta.len() <= LIMIT as u64 => {
            let file = std::fs::File::open(path).map_err(|_| PlanOutcome::Unparseable)?;
            let mut body = String::new();
            file.take(LIMIT as u64 + 1)
                .read_to_string(&mut body)
                .map_err(|_| PlanOutcome::Unparseable)?;
            if body.len() > LIMIT {
                return Err(PlanOutcome::Unparseable);
            }
            Ok(Some(body))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        _ => Err(PlanOutcome::Unparseable),
    }
}

fn root(text: Option<&str>) -> Result<Value, PlanOutcome> {
    match text {
        None => Ok(json!({})),
        Some(text) => serde_json::from_str::<Value>(text)
            .ok()
            .filter(Value::is_object)
            .ok_or(PlanOutcome::Unparseable),
    }
}

fn at<'a>(root: &'a Value, path: &[String]) -> Option<&'a Value> {
    path.iter().try_fold(root, |value, key| value.get(key))
}

fn put(root: &mut Value, path: &[String], value: Option<Value>) -> Result<(), PlanOutcome> {
    let (key, parents) = path.split_last().ok_or(PlanOutcome::Unparseable)?;
    let mut parent = root;
    for segment in parents {
        let object = parent.as_object_mut().ok_or(PlanOutcome::Unparseable)?;
        parent = object.entry(segment).or_insert_with(|| json!({}));
    }
    let object = parent.as_object_mut().ok_or(PlanOutcome::Unparseable)?;
    match value {
        Some(value) => {
            object.insert(key.clone(), value);
        }
        None => {
            object.remove(key);
        }
    }
    Ok(())
}

fn path(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| (*s).to_owned()).collect()
}

fn add(
    root: &mut Value,
    receipt: &mut Receipt,
    path: Vec<String>,
    value: Value,
) -> Result<(), PlanOutcome> {
    for length in 1..path.len() {
        let prefix = path[..length].to_vec();
        if at(root, &prefix).is_none() && !receipt.containers.contains(&prefix) {
            receipt.containers.push(prefix);
        }
    }
    let before = at(root, &path).cloned();
    put(root, &path, Some(value.clone()))?;
    receipt.slots.push(Slot {
        path,
        before,
        installed: value,
    });
    Ok(())
}

fn provider_table(major: u32) -> &'static str {
    if major == 1 { "provider" } else { "providers" }
}

fn model_slot(path: &[String], major: u32) -> bool {
    path == self::path(&["model"])
        || (major == 1 && path == self::path(&["small_model"]))
        || (path.len() == 3
            && path[0] == if major == 1 { "agent" } else { "agents" }
            && path[2] == "model")
}

fn selects_owned_provider(value: Option<&Value>) -> bool {
    value.is_some_and(|value| {
        value
            .as_str()
            .and_then(|model| model.strip_prefix(PROVIDER))
            .and_then(|model| model.strip_prefix('/'))
            .is_some_and(|model| !model.is_empty())
            || value.get("providerID").and_then(Value::as_str) == Some(PROVIDER)
    })
}

fn valid_receipt(receipt: &Receipt) -> bool {
    if receipt.version != 1 || !matches!(receipt.major, 1 | 2) || receipt.slots.is_empty() {
        return false;
    }
    let table = provider_table(receipt.major);
    let allowed = |p: &[String]| {
        model_slot(p, receipt.major) || (p.len() >= 3 && p[0] == table && p[1] == PROVIDER)
    };
    receipt.slots.iter().all(|slot| allowed(&slot.path))
        && receipt.containers.iter().all(|container| {
            receipt
                .slots
                .iter()
                .any(|slot| slot.path.starts_with(container) && slot.path.len() > container.len())
        })
}

fn receipt(text: Option<&str>) -> Result<Option<Receipt>, PlanOutcome> {
    text.map(|text| {
        serde_json::from_str::<Receipt>(text)
            .ok()
            .filter(valid_receipt)
            .ok_or(PlanOutcome::Unparseable)
    })
    .transpose()
}

fn routed(root: &Value, port: Option<u16>) -> bool {
    if !selects_owned_provider(root.get("model")) {
        return false;
    }
    [
        (&root["provider"][PROVIDER]["options"]["baseURL"]),
        (&root["providers"][PROVIDER]["settings"]["baseURL"]),
    ]
    .iter()
    .any(|value| {
        value
            .as_str()
            .and_then(endpoint::parse_openai)
            .is_some_and(|url| port.is_none_or(|p| p == url.port))
    })
}

pub(super) fn tool() -> tools::Tool {
    let dir = directory();
    let config = dir.as_deref().map(config_path);
    let value = config
        .as_deref()
        .and_then(|p| read(p).ok())
        .flatten()
        .and_then(|s| root(Some(&s)).ok());
    let held = dir
        .as_ref()
        .is_some_and(|p| p.join(receipt_name()).exists());
    tools::Tool {
        id: ID.into(),
        name: "OpenCode".into(),
        installed: dir.as_deref().is_some_and(Path::is_dir) || native_installed(),
        wired: held || value.as_ref().is_some_and(|v| routed(v, None)),
        config_path: config,
        // IronWire's CLI has no OpenCode implementation. Do not advertise a
        // command that cannot perform this app-owned, previewed operation.
        connect_command: String::new(),
    }
}

pub(super) fn connected(config: &Path, port: Option<u16>) -> bool {
    port.is_some()
        && read(config)
            .ok()
            .flatten()
            .and_then(|s| root(Some(&s)).ok())
            .is_some_and(|v| routed(&v, port))
}

fn native_installed() -> bool {
    #[cfg(test)]
    return false;
    #[cfg(not(test))]
    profiles::find_native(ToolId::Opencode).is_ok()
}

impl OpenCodeEdit {
    pub(super) fn plan(port: Option<u16>, context: Option<&Context>) -> Result<Self, PlanOutcome> {
        let dir = directory().ok_or(PlanOutcome::NoConfigPath)?;
        // Two competing config files have precedence implications. Require a
        // person to resolve that rather than silently editing the wrong one.
        if dir.join("opencode.json").exists() && dir.join("opencode.jsonc").exists() {
            return Err(PlanOutcome::Unparseable);
        }
        let config = config_path(&dir);
        let receipt_path = dir.join(receipt_name());
        let existing = read(&config)?;
        let receipt_before = read(&receipt_path)?;
        let mut value = root(existing.as_deref())?;
        let saved = receipt(receipt_before.as_deref())?;
        let mut edit = Self {
            path: config,
            changes: vec![],
            occupied: vec![],
            existing,
            contents: String::new(),
            receipt_path,
            receipt_before,
            receipt_after: None,
            other_channel_receipt: None,
        };
        match port {
            Some(port) => {
                let context = context.ok_or(PlanOutcome::ModelsUnavailable)?;
                if !matches!(context.major, 1 | 2) {
                    return Err(PlanOutcome::UnsupportedVersion);
                }
                let table = provider_table(context.major);
                // Both channels edit this same OpenCode global file. Their
                // provider and undo namespaces are separate, but layering two
                // active undo records would make a later restore ambiguous.
                let other_receipt = dir.join(format!(
                    ".{}-routing.json",
                    crate::channel::OTHER_STATE_DIR_NAME
                ));
                if !absent(&other_receipt) {
                    edit.occupied.push(("OpenCode connection".into(), "Another Trace Commons channel already has a saved connection; disconnect it first".into()));
                    edit.contents = edit.existing.clone().unwrap_or_default();
                    return Ok(edit);
                }
                edit.other_channel_receipt = Some(other_receipt);
                if saved.is_some() || value.get(table).and_then(|p| p.get(PROVIDER)).is_some() {
                    edit.occupied.push((
                        format!("{table}.{PROVIDER}"),
                        "Existing provider or connection record".into(),
                    ));
                    edit.contents = edit.existing.clone().unwrap_or_default();
                    edit.receipt_after = edit.receipt_before.clone();
                    return Ok(edit);
                }
                // A V1 config cannot simply be interpreted as V2 or vice versa.
                let other = if context.major == 1 {
                    "providers"
                } else {
                    "provider"
                };
                if value.get(other).is_some() {
                    return Err(PlanOutcome::UnsupportedVersion);
                }
                if let Some(slot) = blocking_policy(&value, context.major) {
                    edit.occupied.push((
                        slot,
                        "Existing provider restrictions need review; settings left unchanged"
                            .into(),
                    ));
                    edit.contents = edit.existing.clone().unwrap_or_default();
                    return Ok(edit);
                }
                let models: Vec<_> = context
                    .models
                    .iter()
                    .filter(|m| crate::managed::opencode::valid_model(&m.id))
                    .collect();
                let current = value["model"].as_str();
                let chosen = models
                    .iter()
                    .find(|m| Some(m.id.as_str()) == current)
                    .copied()
                    .or_else(|| {
                        current
                            .and_then(|current| current.split_once('/'))
                            .and_then(|(_, suffix)| models.iter().find(|m| m.id == suffix).copied())
                    })
                    .or_else(|| models.first().copied())
                    .ok_or(PlanOutcome::ModelsUnavailable)?;
                let selected = format!("{PROVIDER}/{}", chosen.id);
                let mut saved = Receipt {
                    version: 1,
                    major: context.major,
                    slots: vec![],
                    containers: vec![],
                };
                add(&mut value, &mut saved, path(&["model"]), json!(selected))?;
                if context.major == 1 {
                    add(
                        &mut value,
                        &mut saved,
                        path(&["small_model"]),
                        json!(selected),
                    )?;
                } else {
                    for agent in ["title", "compaction"] {
                        add(
                            &mut value,
                            &mut saved,
                            path(&["agents", agent, "model"]),
                            json!(selected),
                        )?;
                    }
                }
                // Explicit agent models outrank the global default. Include
                // every existing override in this same preview and undo record.
                let agent_table = if context.major == 1 {
                    "agent"
                } else {
                    "agents"
                };
                let agent_models: Vec<_> = value
                    .get(agent_table)
                    .and_then(Value::as_object)
                    .into_iter()
                    .flat_map(|agents| agents.iter())
                    .filter(|(_, config)| config.get("model").is_some())
                    .map(|(name, _)| name.clone())
                    .collect();
                let mut changed_agents = 0;
                for name in agent_models {
                    let slot = path(&[agent_table, &name, "model"]);
                    if !saved.slots.iter().any(|s| s.path == slot) {
                        add(&mut value, &mut saved, slot, json!(selected))?;
                        changed_agents += 1;
                    }
                }
                let (package, settings) = if context.major == 1 {
                    ("npm", "options")
                } else {
                    ("package", "settings")
                };
                add(
                    &mut value,
                    &mut saved,
                    path(&[table, PROVIDER, package]),
                    json!(if context.major == 1 {
                        "@ai-sdk/openai-compatible"
                    } else {
                        "@opencode/ai/providers/openai-compatible"
                    }),
                )?;
                add(
                    &mut value,
                    &mut saved,
                    path(&[table, PROVIDER, "name"]),
                    json!("Trace Commons"),
                )?;
                add(
                    &mut value,
                    &mut saved,
                    path(&[table, PROVIDER, settings, "baseURL"]),
                    json!(endpoint::openai_url(port, None)),
                )?;
                // A non-secret local placeholder; the shared proxy owns auth.
                add(
                    &mut value,
                    &mut saved,
                    path(&[table, PROVIDER, settings, "apiKey"]),
                    json!(format!("{PROVIDER}-local")),
                )?;
                for model in models {
                    add(
                        &mut value,
                        &mut saved,
                        path(&[table, PROVIDER, "models", &model.id, "name"]),
                        json!(model.name.as_deref().unwrap_or(&model.id)),
                    )?;
                }
                edit.changes = vec![
                    format!(
                        "Add {table}.{PROVIDER} at {}",
                        endpoint::openai_url(port, None)
                    ),
                    format!(
                        "Default model: {selected}; save previous model selections for Disconnect"
                    ),
                ];
                if changed_agents > 0 {
                    edit.changes.push(format!("Route {changed_agents} explicit agent model selections through {selected}; save their previous selections"));
                }
                edit.receipt_after = Some(
                    serde_json::to_string_pretty(&saved).map_err(|_| PlanOutcome::Unparseable)?,
                );
            }
            None => {
                if let Some(saved) = saved {
                    for slot in &saved.slots {
                        let current = at(&value, &slot.path);
                        // Changing models within this provider retains the
                        // connection. Its selection must be undone with its
                        // provider, otherwise Disconnect would strand it.
                        if current == Some(&slot.installed)
                            || (model_slot(&slot.path, saved.major)
                                && selects_owned_provider(current))
                        {
                            put(&mut value, &slot.path, slot.before.clone())?;
                        }
                    }
                    let agent_table = if saved.major == 1 { "agent" } else { "agents" };
                    let additional: Vec<_> = value
                        .get(agent_table)
                        .and_then(Value::as_object)
                        .into_iter()
                        .flat_map(|agents| agents.iter())
                        .filter(|(_, config)| selects_owned_provider(config.get("model")))
                        .map(|(name, _)| name.clone())
                        .collect();
                    for name in &additional {
                        // This override was added after Connect. Remove only
                        // its reference to our provider, preserving the agent
                        // itself so it inherits the restored global default.
                        put(&mut value, &path(&[agent_table, name, "model"]), None)?;
                    }
                    for container in saved.containers.iter().rev() {
                        if at(&value, container)
                            .and_then(Value::as_object)
                            .is_some_and(|v| v.is_empty())
                        {
                            put(&mut value, container, None)?;
                        }
                    }
                    edit.changes.push("Remove Trace Commons routing settings and restore previous model selections; keep other-provider selections and unrelated edits".into());
                    if !additional.is_empty() {
                        edit.changes.push(format!("Remove {} additional agent model overrides using Trace Commons; inherit the restored default", additional.len()));
                    }
                } else if routed(&value, None) {
                    edit.occupied.push((
                        PROVIDER.into(),
                        "Connection record is missing; settings left unchanged".into(),
                    ));
                }
            }
        }
        edit.contents = if edit.changes.is_empty() {
            edit.existing.clone().unwrap_or_default()
        } else {
            format!(
                "{}\n",
                serde_json::to_string_pretty(&value).map_err(|_| PlanOutcome::Unparseable)?
            )
        };
        // A connection must be readable by its own bounded undo path. The
        // per-slot receipt can exceed the model-list input because it records
        // paths and previous/installed values, so check both serialized outputs.
        if edit.contents.len() > LIMIT
            || edit
                .receipt_after
                .as_ref()
                .is_some_and(|text| text.len() > LIMIT)
        {
            return Err(PlanOutcome::ModelsUnavailable);
        }
        // Accept only a plan whose serialized ownership record can actually
        // be read by Disconnect; agent keys are JSON keys, not path segments.
        receipt(edit.receipt_after.as_deref())?;
        Ok(edit)
    }

    pub(super) fn commit_if_unchanged(&self) -> Result<Option<PathBuf>, tools::CommitError> {
        // A shared lock serializes commits across processes AND build channels.
        // The lock inode stays in place after unlocking; removing it would let
        // another process lock a different inode for this same configuration.
        let directory = self
            .path
            .parent()
            .ok_or_else(|| tools::CommitError::Io(std::io::Error::other("no config directory")))?;
        let _lock = commit_lock(directory).map_err(tools::CommitError::Io)?;
        let unchanged = read(&self.path).ok() == Some(self.existing.clone())
            && read(&self.receipt_path).ok() == Some(self.receipt_before.clone())
            && self.other_channel_receipt.as_deref().is_none_or(absent);
        if !unchanged {
            return Err(tools::CommitError::Changed);
        }
        self.write().map_err(tools::CommitError::Io)
    }

    fn write(&self) -> std::io::Result<Option<PathBuf>> {
        let dir = self
            .path
            .parent()
            .ok_or_else(|| std::io::Error::other("no config directory"))?;
        std::fs::create_dir_all(dir)?;
        let backup = self.path.with_extension(format!(
            "{}.{PROVIDER}-backup",
            self.path.extension().unwrap_or_default().to_string_lossy()
        ));
        let backup_path = match &self.existing {
            Some(existing) => private_create(&backup, existing.as_bytes())?.then_some(backup),
            None => None,
        };
        // Write the receipt before connecting: failure can leave an undo record,
        // never newly routed settings without one. On disconnect keep the record
        // until the config write has succeeded, so retry remains possible.
        if let Some(after) = &self.receipt_after {
            atomic_private(&self.receipt_path, after.as_bytes())?;
        }
        atomic_private(&self.path, self.contents.as_bytes())?;
        if self.receipt_after.is_none() && self.receipt_before.is_some() {
            std::fs::remove_file(&self.receipt_path)?;
            sync_directory(dir)?;
        }
        Ok(backup_path)
    }
}

fn absent(path: &Path) -> bool {
    matches!(std::fs::symlink_metadata(path), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
}

struct CommitLock(std::fs::File);

impl Drop for CommitLock {
    fn drop(&mut self) {
        // Closing only this handle leaves the lock held while a concurrently
        // spawned child still has an inherited descriptor before exec. End
        // ownership at the transaction boundary, even if that handle is open.
        let _ = self.0.unlock();
    }
}

fn commit_lock(directory: &Path) -> std::io::Result<CommitLock> {
    std::fs::create_dir_all(directory)?;
    let path = directory.join(".opencode-routing.lock");
    private_create(&path, b"")?;
    let metadata = std::fs::symlink_metadata(&path)?;
    if !metadata.is_file() {
        return Err(std::io::Error::other("routing lock is not a regular file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(std::io::Error::other("routing lock is not private"));
        }
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)?;
    let after = std::fs::symlink_metadata(&path)?;
    if !after.is_file() {
        return Err(std::io::Error::other("routing lock changed"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let opened = file.metadata()?;
        if opened.dev() != metadata.dev()
            || opened.ino() != metadata.ino()
            || opened.dev() != after.dev()
            || opened.ino() != after.ino()
        {
            return Err(std::io::Error::other("routing lock changed"));
        }
    }
    file.try_lock()
        .map_err(|_| std::io::Error::other("routing configuration is busy"))?;
    Ok(CommitLock(file))
}

fn private_create(path: &Path, bytes: &[u8]) -> std::io::Result<bool> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(mut file) => {
            file.write_all(bytes)?;
            file.sync_all()?;
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(error),
    }
}

fn atomic_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = tempfile::NamedTempFile::new_in(
        path.parent()
            .ok_or_else(|| std::io::Error::other("no parent"))?,
    )?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    sync_directory(
        path.parent()
            .ok_or_else(|| std::io::Error::other("no parent"))?,
    )?;
    Ok(())
}

fn sync_directory(directory: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    std::fs::File::open(directory)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = directory;
    Ok(())
}

fn blocking_policy(value: &Value, major: u32) -> Option<String> {
    if major == 1 {
        for key in ["enabled_providers", "disabled_providers"] {
            if let Some(setting) = value.get(key) {
                let Some(entries) = setting.as_array() else {
                    return Some(key.into());
                };
                if entries.iter().any(|item| !item.is_string()) {
                    return Some(key.into());
                }
                let contains = entries.iter().any(|id| id.as_str() == Some(PROVIDER));
                if (key == "enabled_providers" && !contains)
                    || (key == "disabled_providers" && contains)
                {
                    return Some(key.into());
                }
            }
        }
    } else if let Some(policies) = value.get("policies") {
        let Some(rules) = policies.as_array() else {
            return Some("policies".into());
        };
        // Do not reinterpret or weaken a contributor's provider policy. A
        // policy touching provider use needs an explicit human reconciliation.
        if rules.iter().any(|rule| {
            rule.get("action")
                .and_then(Value::as_str)
                .is_none_or(|action| {
                    action == "*" || action == "provider.use" || action == "provider.*"
                })
        }) {
            return Some("policies".into());
        }
    }
    None
}

/// Only a Connect click probes the native version, with isolated standard
/// directories. Merely listing tools never executes a contributor's binary.
pub(super) fn native_major() -> Result<u32, &'static str> {
    let native = profiles::find_native(ToolId::Opencode).map_err(|_| ERR_NO_EXECUTABLE)?;
    let isolated = tempfile::tempdir().map_err(|_| ERR_VERSION)?;
    let mut command = std::process::Command::new(native);
    crate::managed::opencode::isolate(&mut command, isolated.path()).map_err(|_| ERR_VERSION)?;
    command.current_dir(isolated.path()).arg("--version");
    let result = profiles::bounded_output_limit(command, 4096).map_err(|_| ERR_VERSION)?;
    if !result.status.success() {
        return Err(ERR_VERSION);
    }
    parse_major(&String::from_utf8_lossy(&result.stdout)).ok_or(ERR_VERSION)
}

fn parse_major(text: &str) -> Option<u32> {
    text.split_whitespace().find_map(|word| {
        let parts: Vec<_> = word
            .trim_start_matches('v')
            .split('.')
            .map(str::parse::<u32>)
            .collect::<Result<_, _>>()
            .ok()?;
        (parts.len() == 3 && matches!(parts[0], 1 | 2)).then_some(parts[0])
    })
}

pub(super) async fn live_models(port: u16) -> Result<Vec<ChatModel>, &'static str> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|_| ERR_MODELS)?;
    let mut response = client
        .get(format!("http://127.0.0.1:{port}/openai/v1/models"))
        .send()
        .await
        .map_err(|_| ERR_MODELS)?;
    if !response.status().is_success() {
        return Err(ERR_MODELS);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| ERR_MODELS)? {
        if body.len() + chunk.len() > LIMIT {
            return Err(ERR_MODELS);
        }
        body.extend_from_slice(&chunk);
    }
    let value = serde_json::from_slice(&body).map_err(|_| ERR_MODELS)?;
    let models = crate::managed::models::chat_models(&value);
    if models.is_empty() {
        Err(ERR_MODELS)
    } else {
        Ok(models)
    }
}

#[cfg(test)]
thread_local! {
    static TEST_DIRECTORY: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        dir: tempfile::TempDir,
        config_dir: PathBuf,
        previous: Option<PathBuf>,
    }

    impl Fixture {
        fn new(value: &Value) -> Self {
            Self::new_in(value, "")
        }

        fn new_in(value: &Value, suffix: &str) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let config_dir = dir.path().join(suffix);
            std::fs::create_dir_all(&config_dir).unwrap();
            std::fs::write(
                config_dir.join("opencode.json"),
                serde_json::to_vec_pretty(value).unwrap(),
            )
            .unwrap();
            let previous = TEST_DIRECTORY.with(|p| p.replace(Some(config_dir.clone())));
            Self {
                dir,
                config_dir,
                previous,
            }
        }

        fn config(&self) -> PathBuf {
            self.config_dir.join("opencode.json")
        }
        fn receipt(&self) -> PathBuf {
            self.config_dir.join(receipt_name())
        }
        fn value(&self) -> Value {
            serde_json::from_str(&std::fs::read_to_string(self.config()).unwrap()).unwrap()
        }
        fn write(&self, value: &Value) {
            std::fs::write(self.config(), serde_json::to_vec_pretty(value).unwrap()).unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            TEST_DIRECTORY.with(|p| p.replace(self.previous.take()));
        }
    }

    fn context(major: u32) -> Context {
        Context {
            major,
            models: vec![
                ChatModel {
                    id: "anthropic/fixture-chat".into(),
                    name: Some("Fixture chat".into()),
                    tier: None,
                },
                ChatModel {
                    id: "openai/fixture-chat".into(),
                    name: None,
                    tier: None,
                },
            ],
        }
    }

    fn connect(store: &PlanStore, context: &Context) -> PlanView {
        plan_with_opencode(
            store,
            &Catalog::default(),
            ID,
            HarnessAction::Connect,
            Some(4321),
            true,
            Some(context),
        )
        .unwrap()
    }

    #[test]
    fn both_majors_select_a_real_model_and_restore_previous_selections() {
        for major in [1, 2] {
            let initial = if major == 1 {
                json!({"model":"openai/fixture-chat","small_model":"vendor/small","provider":{"other":{"name":"Personal"}},"theme":"mine"})
            } else {
                json!({"model":"openai/fixture-chat","agents":{"title":{"model":"other/title","prompt":"keep"}},"providers":{"other":{"name":"Personal"}},"theme":"mine"})
            };
            let fixture = Fixture::new(&initial);
            let before = std::fs::read(fixture.config()).unwrap();
            let store = PlanStore::default();
            let preview = connect(&store, &context(major));
            assert_eq!(preview.outcome, PlanOutcome::Changes);
            assert_eq!(
                std::fs::read(fixture.config()).unwrap(),
                before,
                "preview must not write"
            );
            assert!(!fixture.receipt().exists());
            let committed = commit(&store, preview.plan_id.unwrap()).unwrap();
            assert_eq!(
                std::fs::read(committed.backup_path.unwrap()).unwrap(),
                before
            );
            let value = fixture.value();
            assert_eq!(value["model"], format!("{PROVIDER}/openai/fixture-chat"));
            assert_eq!(
                value[provider_table(major)][PROVIDER]
                    [if major == 1 { "options" } else { "settings" }]["baseURL"],
                "http://127.0.0.1:4321/openai/v1"
            );
            assert!(
                value[provider_table(major)][PROVIDER]["models"]["openai/fixture-chat"].is_object()
            );
            assert_eq!(
                value[provider_table(major)][PROVIDER][if major == 1 { "npm" } else { "package" }],
                if major == 1 {
                    "@ai-sdk/openai-compatible"
                } else {
                    "@opencode/ai/providers/openai-compatible"
                }
            );
            assert_eq!(value["theme"], "mine");
            assert!(connected(&fixture.config(), Some(4321)));
            assert!(!connected(&fixture.config(), Some(9000)));
            let disconnect = plan(
                &store,
                &Catalog::default(),
                ID,
                HarnessAction::Disconnect,
                None,
                false,
            )
            .unwrap();
            assert_eq!(disconnect.outcome, PlanOutcome::Changes);
            commit(&store, disconnect.plan_id.unwrap()).unwrap();
            assert_eq!(fixture.value(), initial);
            assert!(!fixture.receipt().exists());
        }
    }

    #[test]
    fn no_default_uses_only_a_live_listed_model_and_disconnect_removes_added_containers() {
        for major in [1, 2] {
            let fixture = Fixture::new(&json!({"share":"disabled"}));
            let store = PlanStore::default();
            commit(&store, connect(&store, &context(major)).plan_id.unwrap()).unwrap();
            assert_eq!(
                fixture.value()["model"],
                format!("{PROVIDER}/anthropic/fixture-chat")
            );
            let undo = plan(
                &store,
                &Catalog::default(),
                ID,
                HarnessAction::Disconnect,
                None,
                false,
            )
            .unwrap();
            commit(&store, undo.plan_id.unwrap()).unwrap();
            assert_eq!(fixture.value(), json!({"share":"disabled"}));
        }
    }

    #[test]
    fn disconnect_preserves_user_changes_inside_and_outside_the_provider() {
        for major in [1, 2] {
            let fixture = Fixture::new(&json!({"model":"old/model"}));
            let store = PlanStore::default();
            commit(&store, connect(&store, &context(major)).plan_id.unwrap()).unwrap();
            let mut edited = fixture.value();
            edited["model"] = json!("new/user-choice");
            edited["userSetting"] = json!(["kept"]);
            edited[provider_table(major)][PROVIDER]["models"]["user-added"] =
                json!({"name":"Mine"});
            fixture.write(&edited);
            let undo = plan(
                &store,
                &Catalog::default(),
                ID,
                HarnessAction::Disconnect,
                None,
                false,
            )
            .unwrap();
            commit(&store, undo.plan_id.unwrap()).unwrap();
            let after = fixture.value();
            assert_eq!(after["model"], "new/user-choice");
            assert_eq!(after["userSetting"], json!(["kept"]));
            assert_eq!(
                after[provider_table(major)][PROVIDER],
                json!({"models":{"user-added":{"name":"Mine"}}})
            );
        }
    }

    #[test]
    fn disconnect_restores_own_provider_model_changes_and_clears_new_agent_references() {
        for major in [1, 2] {
            let agents = if major == 1 { "agent" } else { "agents" };
            let mut initial = json!({"model":"personal/original", "theme":"before"});
            initial[agents] =
                json!({"build":{"model":"personal/build", "prompt":"original prompt"}});
            if major == 1 {
                initial["small_model"] = json!("personal/small");
            }
            let fixture = Fixture::new(&initial);
            let store = PlanStore::default();
            commit(&store, connect(&store, &context(major)).plan_id.unwrap()).unwrap();
            let mut edited = fixture.value();
            let another_model = if major == 1 {
                json!(format!("{PROVIDER}/openai/fixture-chat"))
            } else {
                json!({"providerID":PROVIDER, "model":"openai/fixture-chat"})
            };
            edited["model"] = another_model.clone();
            edited[agents]["build"]["model"] = another_model.clone();
            edited[agents]["late"] = json!({"model":another_model, "prompt":"keep late agent"});
            edited[agents]["external"] =
                json!({"model":"other-provider/new-choice", "prompt":"keep external agent"});
            edited["theme"] = json!("after");
            if major == 1 {
                edited["small_model"] = json!(format!("{PROVIDER}/openai/fixture-chat"));
            } else {
                edited["policies"] =
                    json!([{"action":"tool.use", "resource":"*", "effect":"deny"}]);
            }
            fixture.write(&edited);
            assert!(
                connected(&fixture.config(), Some(4321)),
                "changing the selected model within this provider retains routing"
            );
            let undo = plan(
                &store,
                &Catalog::default(),
                ID,
                HarnessAction::Disconnect,
                None,
                false,
            )
            .unwrap();
            assert!(
                undo.changes
                    .iter()
                    .any(|change| change.contains("1 additional agent model"))
            );
            commit(&store, undo.plan_id.unwrap()).unwrap();
            let after = fixture.value();
            assert_eq!(after["model"], initial["model"]);
            assert_eq!(after[agents]["build"], initial[agents]["build"]);
            assert_eq!(after[agents]["late"], json!({"prompt":"keep late agent"}));
            assert_eq!(after[agents]["external"], edited[agents]["external"]);
            assert_eq!(after["theme"], "after");
            if major == 1 {
                assert_eq!(after["small_model"], initial["small_model"]);
            } else {
                assert_eq!(after["policies"], edited["policies"]);
            }
            assert!(after.get(provider_table(major)).is_none());
            assert!(!fixture.receipt().exists());
        }
    }

    #[test]
    fn an_existing_provider_namespace_is_occupied_and_never_overwritten() {
        let mut initial = json!({"provider":{}});
        initial["provider"][PROVIDER] = json!({"options":{"baseURL":"https://mine.example/v1"}});
        let fixture = Fixture::new(&initial);
        let before = std::fs::read(fixture.config()).unwrap();
        let store = PlanStore::default();
        let preview = connect(&store, &context(1));
        assert_eq!(preview.outcome, PlanOutcome::Noop);
        assert!(preview.plan_id.is_none());
        assert_eq!(preview.occupied.len(), 1);
        assert_eq!(std::fs::read(fixture.config()).unwrap(), before);
    }

    #[test]
    fn provider_and_undo_state_are_scoped_to_this_build_channel() {
        let other = crate::channel::OTHER_STATE_DIR_NAME;
        assert_ne!(PROVIDER, other);
        assert_eq!(
            receipt_name(),
            format!(".{}-routing.json", crate::channel::STATE_DIR_NAME)
        );
        assert!(selects_owned_provider(Some(&json!(format!(
            "{PROVIDER}/fixture/chat"
        )))));
        assert!(!selects_owned_provider(Some(&json!(format!(
            "{other}/fixture/chat"
        )))));
        assert!(!selects_owned_provider(Some(
            &json!({"providerID":other,"model":"fixture/chat"})
        )));
        let fixture = Fixture::new(&json!({"model":"personal/original"}));
        let other_receipt = fixture.config_dir.join(format!(".{other}-routing.json"));
        let store = PlanStore::default();
        let pending = connect(&store, &context(1));
        std::fs::write(&other_receipt, "another channel's private receipt").unwrap();
        assert_eq!(
            commit(&store, pending.plan_id.unwrap()).unwrap_err(),
            ERR_CONFIG_CHANGED
        );
        let refused = connect(&store, &context(1));
        assert_eq!(refused.outcome, PlanOutcome::Noop);
        assert!(refused.plan_id.is_none());
        assert_eq!(refused.occupied.len(), 1);
        assert_eq!(fixture.value(), json!({"model":"personal/original"}));
        assert!(!fixture.receipt().exists());
        assert_eq!(
            std::fs::read_to_string(other_receipt).unwrap(),
            "another channel's private receipt"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_finished_transaction_releases_its_lock_while_an_inherited_handle_remains_open() {
        let fixture = Fixture::new(&json!({"model":"personal/original"}));
        let store = PlanStore::default();
        let preview = connect(&store, &context(1));
        let held = commit_lock(&fixture.config_dir).unwrap();
        // A duplicate shares the lock's open file description, just as a
        // concurrently forked child does before close-on-exec takes effect.
        let inherited = held.0.try_clone().unwrap();
        drop(held);
        commit(&store, preview.plan_id.unwrap())
            .expect("the finished transaction must release the lock before the child closes it");
        assert!(connected(&fixture.config(), Some(4321)));
        assert!(fixture.receipt().is_file());
        drop(inherited);
    }

    #[test]
    fn concurrent_commit_lock_refuses_before_any_routing_or_ownership_write() {
        let fixture = Fixture::new(&json!({"model":"personal/original"}));
        let store = PlanStore::default();
        let preview = connect(&store, &context(1));
        assert!(
            !fixture.config_dir.join(".opencode-routing.lock").exists(),
            "preview writes no lock or config"
        );
        let held = commit_lock(&fixture.config_dir).unwrap();
        assert_eq!(
            commit(&store, preview.plan_id.unwrap()).unwrap_err(),
            ERR_COMMIT_FAILED
        );
        assert_eq!(fixture.value(), json!({"model":"personal/original"}));
        assert!(!fixture.receipt().exists());
        drop(held);
        assert!(
            fixture.config_dir.join(".opencode-routing.lock").is_file(),
            "unlock must retain the shared inode"
        );
        let fresh = connect(&store, &context(1));
        commit(&store, fresh.plan_id.unwrap()).unwrap();
        assert!(connected(&fixture.config(), Some(4321)));
    }

    #[test]
    fn disconnect_never_removes_another_channels_provider_selection_or_receipt() {
        for major in [1, 2] {
            let fixture = Fixture::new(&json!({"model":"personal/original"}));
            let store = PlanStore::default();
            commit(&store, connect(&store, &context(major)).plan_id.unwrap()).unwrap();
            let other = crate::channel::OTHER_STATE_DIR_NAME;
            let other_receipt = fixture.config_dir.join(format!(".{other}-routing.json"));
            std::fs::write(&other_receipt, "other receipt kept").unwrap();
            let mut edited = fixture.value();
            edited["model"] = json!(format!("{other}/fixture/chat"));
            edited[provider_table(major)][other] = json!({"name":"Other channel, preserved"});
            fixture.write(&edited);
            let undo = plan(
                &store,
                &Catalog::default(),
                ID,
                HarnessAction::Disconnect,
                None,
                false,
            )
            .unwrap();
            commit(&store, undo.plan_id.unwrap()).unwrap();
            let after = fixture.value();
            assert_eq!(after["model"], edited["model"]);
            assert_eq!(
                after[provider_table(major)][other],
                edited[provider_table(major)][other]
            );
            assert!(after[provider_table(major)].get(PROVIDER).is_none());
            assert!(!fixture.receipt().exists());
            assert_eq!(
                std::fs::read_to_string(other_receipt).unwrap(),
                "other receipt kept"
            );
        }
    }

    #[test]
    fn provider_restrictions_refuse_without_weakening_user_policies() {
        for (major, value) in [
            (1, json!({"enabled_providers":["openai"]})),
            (1, json!({"disabled_providers":[PROVIDER]})),
            (
                2,
                json!({"policies":[{"action":"provider.use","resource":"*","effect":"deny"}]}),
            ),
            (
                2,
                json!({"policies":[{"action":"*","resource":"*","effect":"deny"}]}),
            ),
        ] {
            let fixture = Fixture::new(&value);
            let store = PlanStore::default();
            let preview = connect(&store, &context(major));
            assert_eq!(preview.outcome, PlanOutcome::Noop);
            assert!(preview.plan_id.is_none());
            assert_eq!(preview.occupied.len(), 1);
            assert_eq!(fixture.value(), value);
            assert!(!fixture.receipt().exists());
        }
    }

    #[test]
    fn existing_agent_overrides_are_previewed_routed_and_restored_without_losing_policies() {
        for major in [1, 2] {
            let agents = if major == 1 { "agent" } else { "agents" };
            let mut initial = json!({"model":"user-provider/openai/fixture-chat"});
            initial[agents] = json!({"build":{"model":"old/build","prompt":"keep"},"custom":{"model":"old/custom"}});
            if major == 1 {
                initial["enabled_providers"] = json!([PROVIDER, "openai"]);
                initial["disabled_providers"] = json!(["other"]);
            } else {
                initial["policies"] = json!([{"action":"tool.use","resource":"*","effect":"deny"}]);
            }
            let fixture = Fixture::new(&initial);
            let store = PlanStore::default();
            let preview = connect(&store, &context(major));
            assert!(
                preview
                    .changes
                    .iter()
                    .any(|change| change.contains("2 explicit agent"))
            );
            commit(&store, preview.plan_id.unwrap()).unwrap();
            let connected = fixture.value();
            assert_eq!(
                connected["model"],
                format!("{PROVIDER}/openai/fixture-chat"),
                "match the existing provider model suffix before choosing a different live model"
            );
            for name in ["build", "custom"] {
                assert_eq!(connected[agents][name]["model"], connected["model"]);
            }
            assert_eq!(connected[agents]["build"]["prompt"], "keep");
            let undo = plan(
                &store,
                &Catalog::default(),
                ID,
                HarnessAction::Disconnect,
                None,
                false,
            )
            .unwrap();
            commit(&store, undo.plan_id.unwrap()).unwrap();
            assert_eq!(fixture.value(), initial);
        }
    }

    #[test]
    #[ignore = "requires TC_OPENCODE_TEST_BIN pointing to a qualified native OpenCode installation; reads only an isolated temporary global config"]
    fn native_global_config_qualification() {
        let binary = std::env::var_os("TC_OPENCODE_TEST_BIN").expect("TC_OPENCODE_TEST_BIN");
        let fixture = Fixture::new_in(
            &json!({"model":"user/old","share":"disabled"}),
            "config/opencode",
        );
        let command = || {
            let mut command = std::process::Command::new(&binary);
            crate::managed::opencode::isolate(&mut command, fixture.dir.path()).unwrap();
            command.env_remove("OPENCODE_CONFIG_CONTENT");
            command.current_dir(fixture.dir.path());
            command
        };
        let mut version = command();
        version.arg("--version");
        let version = profiles::bounded_output_limit(version, 4096).unwrap();
        assert!(version.status.success());
        let major = parse_major(&String::from_utf8_lossy(&version.stdout)).unwrap();
        let store = PlanStore::default();
        commit(&store, connect(&store, &context(major)).plan_id.unwrap()).unwrap();
        let mut debug = command();
        if major == 1 {
            debug.args(["debug", "config"]);
        } else {
            // V2 debug config joins the shared service on its fixed port.
            // The private API reads and validates the same global documents
            // without contacting the contributor's running service.
            debug.args(["api", "--standalone", "GET", "/api/config"]);
        }
        let debug = profiles::bounded_output_limit(debug, LIMIT as u64).unwrap();
        assert!(
            debug.status.success(),
            "native OpenCode {major} config qualification failed: {}",
            String::from_utf8_lossy(&debug.stderr)
        );
        let output = String::from_utf8_lossy(&debug.stdout);
        let value: Value = serde_json::from_slice(&debug.stdout)
            .expect("native config output must be JSON, never HTML or a success-only exit");
        if major == 1 {
            assert_eq!(value["model"], format!("{PROVIDER}/anthropic/fixture-chat"));
        } else {
            let loaded = value
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| {
                    entry["type"] == "document" && entry["info"]["providers"][PROVIDER].is_object()
                })
                .expect("native V2 must load the generated global config");
            assert_eq!(
                loaded["info"]["model"],
                json!({"providerID":PROVIDER,"model":"anthropic/fixture-chat"})
            );
            assert_eq!(
                loaded["info"]["providers"][PROVIDER]["package"],
                "@opencode/ai/providers/openai-compatible"
            );
        }
        assert!(
            output.contains("http://127.0.0.1:4321/openai/v1"),
            "native config output must contain the proxy base"
        );
    }

    #[test]
    fn malformed_json_jsonc_and_competing_files_are_refused_without_a_plan() {
        let fixture = Fixture::new(&json!({}));
        for body in [
            "broken",
            "{\"model\":\"mine\",}",
            "{//keep this comment\n}",
            "[]",
        ] {
            std::fs::write(fixture.config(), body).unwrap();
            let store = PlanStore::default();
            assert_eq!(
                connect(&store, &context(1)).outcome,
                PlanOutcome::Unparseable
            );
            assert_eq!(std::fs::read_to_string(fixture.config()).unwrap(), body);
        }
        fixture.write(&json!({}));
        std::fs::write(fixture.dir.path().join("opencode.jsonc"), "{}").unwrap();
        assert_eq!(
            connect(&PlanStore::default(), &context(1)).outcome,
            PlanOutcome::Unparseable
        );
    }

    #[cfg(unix)]
    #[test]
    fn outer_connect_plan_refuses_fifo_without_opening_the_legacy_unbounded_reader() {
        let fixture = Fixture::new(&json!({}));
        std::fs::remove_file(fixture.config()).unwrap();
        assert!(
            std::process::Command::new("mkfifo")
                .arg(fixture.config())
                .status()
                .unwrap()
                .success()
        );
        let directory = fixture.config_dir.clone();
        let (sent, received) = std::sync::mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            TEST_DIRECTORY.with(|p| p.replace(Some(directory)));
            sent.send(connect(&PlanStore::default(), &context(1)))
                .unwrap();
        });
        let result = received.recv_timeout(Duration::from_secs(2));
        if matches!(result, Err(std::sync::mpsc::RecvTimeoutError::Timeout)) {
            // Release the old blocking reader before failing the regression,
            // so a reintroduced bug cannot hang the test process indefinitely.
            let mut release = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(fixture.config())
                .unwrap();
            release.write_all(b"{}").unwrap();
            drop(release);
        }
        worker.join().unwrap();
        let result =
            result.expect("outer OpenCode plan must refuse a FIFO promptly without opening it");
        assert_eq!(result.outcome, PlanOutcome::Unparseable);
        assert!(result.plan_id.is_none());
        assert!(!fixture.receipt().exists());
    }

    #[test]
    fn empty_agent_names_round_trip_through_the_persisted_receipt_on_both_majors() {
        for major in [1, 2] {
            let agents = if major == 1 { "agent" } else { "agents" };
            let mut initial = json!({"model":"personal/original"});
            initial[agents] = json!({"":{"model":"personal/empty-name","prompt":"keep"}});
            let fixture = Fixture::new(&initial);
            let store = PlanStore::default();
            let preview = connect(&store, &context(major));
            assert_eq!(preview.outcome, PlanOutcome::Changes);
            commit(&store, preview.plan_id.unwrap()).unwrap();
            assert_eq!(
                fixture.value()[agents][""]["model"],
                format!("{PROVIDER}/anthropic/fixture-chat")
            );
            let undo = plan(
                &store,
                &Catalog::default(),
                ID,
                HarnessAction::Disconnect,
                None,
                false,
            )
            .unwrap();
            assert_eq!(undo.outcome, PlanOutcome::Changes);
            commit(&store, undo.plan_id.unwrap()).unwrap();
            assert_eq!(fixture.value(), initial);
            assert!(!fixture.receipt().exists());
        }
    }

    #[test]
    fn null_and_absent_global_helper_and_agent_models_keep_their_presence_after_disconnect() {
        for major in [1, 2] {
            for present in [false, true] {
                let agents = if major == 1 { "agent" } else { "agents" };
                let mut initial = json!({"theme":"kept"});
                initial[agents] = json!({"build":{"prompt":"keep"},"title":{},"compaction":{}});
                if present {
                    initial["model"] = Value::Null;
                    if major == 1 {
                        initial["small_model"] = Value::Null;
                    }
                    for agent in ["build", "title", "compaction"] {
                        initial[agents][agent]["model"] = Value::Null;
                    }
                }
                let fixture = Fixture::new(&initial);
                let store = PlanStore::default();
                commit(&store, connect(&store, &context(major)).plan_id.unwrap()).unwrap();
                let undo = plan(
                    &store,
                    &Catalog::default(),
                    ID,
                    HarnessAction::Disconnect,
                    None,
                    false,
                )
                .unwrap();
                assert_eq!(undo.outcome, PlanOutcome::Changes);
                commit(&store, undo.plan_id.unwrap()).unwrap();
                assert_eq!(
                    fixture.value(),
                    initial,
                    "major {major}, original model presence {present}"
                );
                assert_eq!(fixture.value().get("model").is_some(), present);
            }
        }
    }

    #[test]
    fn config_or_ownership_receipt_change_invalidates_a_plan_and_plan_ids_are_single_use() {
        let fixture = Fixture::new(&json!({}));
        let store = PlanStore::default();
        let first = connect(&store, &context(1)).plan_id.unwrap();
        fixture.write(&json!({"user":"changed"}));
        assert_eq!(commit(&store, first).unwrap_err(), ERR_CONFIG_CHANGED);
        assert_eq!(commit(&store, first).unwrap_err(), ERR_PLAN_UNKNOWN);
        let second = connect(&store, &context(1)).plan_id.unwrap();
        std::fs::write(fixture.receipt(), "{}").unwrap();
        assert_eq!(commit(&store, second).unwrap_err(), ERR_CONFIG_CHANGED);
        assert_eq!(fixture.value(), json!({"user":"changed"}));
        std::fs::remove_file(fixture.receipt()).unwrap();
        let third = connect(&store, &context(1)).plan_id.unwrap();
        commit(&store, third).unwrap();
        assert_eq!(commit(&store, third).unwrap_err(), ERR_PLAN_UNKNOWN);
    }

    #[test]
    fn stale_port_connection_remains_removable_with_no_listener_or_credential() {
        let fixture = Fixture::new(&json!({}));
        let store = PlanStore::default();
        commit(&store, connect(&store, &context(2)).plan_id.unwrap()).unwrap();
        let rows = list(&Catalog::default(), &FamilyActivity::default(), Some(9999));
        let row = rows.iter().find(|r| r.id == ID).unwrap();
        assert!(!row.connected);
        assert!(row.wired);
        assert!(offers_disconnect(row));
        let undo = plan(
            &store,
            &Catalog::default(),
            ID,
            HarnessAction::Disconnect,
            None,
            false,
        )
        .unwrap();
        commit(&store, undo.plan_id.unwrap()).unwrap();
        assert_eq!(fixture.value(), json!({}));
    }

    #[test]
    fn unsupported_versions_mixed_schemas_and_missing_live_models_are_explicit_refusals() {
        let fixture = Fixture::new(&json!({}));
        let store = PlanStore::default();
        assert_eq!(
            connect(&store, &context(3)).outcome,
            PlanOutcome::UnsupportedVersion
        );
        assert_eq!(
            connect(
                &store,
                &Context {
                    major: 1,
                    models: vec![]
                }
            )
            .outcome,
            PlanOutcome::ModelsUnavailable
        );
        assert_eq!(
            plan(
                &store,
                &Catalog::default(),
                ID,
                HarnessAction::Connect,
                Some(4321),
                true
            )
            .unwrap()
            .outcome,
            PlanOutcome::ModelsUnavailable
        );
        fixture.write(&json!({"provider":{"user":{"name":"mine"}}}));
        assert_eq!(
            connect(&store, &context(2)).outcome,
            PlanOutcome::UnsupportedVersion
        );
        for bad in ["", "3.0.0", "1.19", "unknown", "1.19.0-beta"] {
            assert_eq!(parse_major(bad), None);
        }
        assert_eq!(parse_major("opencode v1.19.4"), Some(1));
        assert_eq!(parse_major("2.1.4"), Some(2));
    }

    #[test]
    fn oversized_serialized_config_or_receipt_is_refused_before_any_write() {
        let fixture = Fixture::new(&json!({"untouched":"x".repeat(LIMIT - 64)}));
        let store = PlanStore::default();
        let preview = connect(&store, &context(1));
        assert_eq!(preview.outcome, PlanOutcome::ModelsUnavailable);
        assert!(preview.plan_id.is_none());
        assert!(!fixture.receipt().exists());
        fixture.write(&json!({}));
        // The model-list JSON fits the input bound while the expanded ownership
        // receipt would not. It must not create an impossible-to-disconnect edit.
        let models: Vec<_> = (0..4000)
            .map(|i| ChatModel {
                id: format!("vendor/model-{i}"),
                name: None,
                tier: None,
            })
            .collect();
        let catalogue =
            json!({"data": models.iter().map(|m| json!({"id":m.id})).collect::<Vec<_>>()});
        assert!(catalogue.to_string().len() < LIMIT);
        let preview = connect(&store, &Context { major: 2, models });
        assert_eq!(preview.outcome, PlanOutcome::ModelsUnavailable);
        assert!(preview.plan_id.is_none());
        assert_eq!(fixture.value(), json!({}));
        assert!(!fixture.receipt().exists());
    }

    #[cfg(unix)]
    #[test]
    fn backups_and_receipts_are_private_and_symlink_files_are_not_rewritten() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let fixture = Fixture::new(&json!({"private":"config"}));
        let store = PlanStore::default();
        let committed = commit(&store, connect(&store, &context(1)).plan_id.unwrap()).unwrap();
        for path in [committed.backup_path.unwrap(), fixture.receipt()] {
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o077,
                0
            );
        }
        let undo = plan(
            &store,
            &Catalog::default(),
            ID,
            HarnessAction::Disconnect,
            None,
            false,
        )
        .unwrap();
        commit(&store, undo.plan_id.unwrap()).unwrap();
        let elsewhere = fixture.dir.path().join("elsewhere.json");
        std::fs::write(&elsewhere, "{}").unwrap();
        std::fs::remove_file(fixture.config()).unwrap();
        symlink(&elsewhere, fixture.config()).unwrap();
        assert_eq!(
            connect(&store, &context(1)).outcome,
            PlanOutcome::Unparseable
        );
        assert_eq!(std::fs::read_to_string(elsewhere).unwrap(), "{}");
    }

    #[tokio::test]
    async fn model_discovery_uses_fixed_loopback_endpoint_and_rejects_empty_or_redirected_lists() {
        use axum::{Router, routing::get};
        let app = Router::new().route(
            "/openai/v1/models",
            get(|| async {
                axum::Json(json!({"data":[{"id":"fixture/chat"},{"id":"embedding-model"}]}))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        assert_eq!(
            live_models(port).await.unwrap(),
            vec![ChatModel {
                id: "fixture/chat".into(),
                name: None,
                tier: Some(crate::managed::models::ModelTier::Incognito),
            }]
        );
        server.abort();
        for (status, body) in [
            (200, "{\"data\":[]}".to_string()),
            (302, "{}".to_string()),
            (200, "x".repeat(LIMIT + 1)),
        ] {
            let app = Router::new().route(
                "/openai/v1/models",
                get(move || {
                    let body = body.clone();
                    async move {
                        (
                            axum::http::StatusCode::from_u16(status).unwrap(),
                            [("location", "https://must-not-follow.invalid")],
                            body,
                        )
                    }
                }),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            assert_eq!(live_models(port).await.unwrap_err(), ERR_MODELS);
            server.abort();
        }
    }
}
