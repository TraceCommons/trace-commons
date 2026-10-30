//! Folder-specific launch choices, independent of the selected account.

use super::{ManagedError, ToolId};
use crate::{config::ConfigStore, daemon::settings::DaemonSettings};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FolderSettings {
    pub tool: ToolId,
    pub cwd: PathBuf,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub collect_traces: bool,
    pub cli_options: String,
}

#[derive(Serialize, Deserialize)]
pub struct FolderSnapshot {
    pub tool: ToolId,
    pub model: Option<String>,
    pub collect_traces: bool,
    pub cli_options: String,
    pub remembered: bool,
}

pub fn canonical(cwd: &Path) -> Result<PathBuf, ManagedError> {
    if !cwd.is_absolute() || !cwd.is_dir() {
        return Err(ManagedError::InvalidRequest);
    }
    std::fs::canonicalize(cwd).map_err(|_| ManagedError::InvalidRequest)
}

pub fn lookup(
    store: &ConfigStore,
    tool: ToolId,
    cwd: &Path,
) -> Result<FolderSnapshot, ManagedError> {
    let cwd = canonical(cwd)?;
    let settings = DaemonSettings::load(store).map_err(|_| ManagedError::StorageUnavailable)?;
    let saved = settings
        .managed_folder_settings
        .iter()
        .find(|entry| entry.tool == tool && entry.cwd == cwd);
    let result = match saved {
        Some(entry) => FolderSnapshot {
            tool,
            model: entry.model.clone(),
            collect_traces: entry.collect_traces,
            cli_options: entry.cli_options.clone(),
            remembered: true,
        },
        None => FolderSnapshot {
            tool,
            model: None,
            collect_traces: false,
            cli_options: settings
                .managed_cli_options
                .get(&tool)
                .cloned()
                .unwrap_or_default(),
            remembered: false,
        },
    };
    super::options::parse(tool, &result.cli_options)?;
    Ok(result)
}

pub fn apply_patch(
    settings: &mut DaemonSettings,
    value: &serde_json::Value,
) -> Result<(), &'static str> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Patch {
        tool: ToolId,
        cwd: PathBuf,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        collect_traces: bool,
        #[serde(default)]
        cli_options: String,
        #[serde(default)]
        reset: bool,
    }
    let updates: Vec<Patch> =
        serde_json::from_value(value.clone()).map_err(|_| "settings-invalid-value")?;
    let mut next = settings.managed_folder_settings.clone();
    let mut seen = std::collections::BTreeSet::new();
    for update in updates {
        let cwd = canonical(&update.cwd).map_err(|_| "settings-invalid-value")?;
        if !seen.insert((update.tool, cwd.clone())) {
            return Err("settings-invalid-value");
        }
        super::options::parse(update.tool, &update.cli_options)
            .map_err(|_| "managed-cli-options-invalid")?;
        if update
            .model
            .as_deref()
            .is_some_and(|model| !super::opencode::valid_model(model))
        {
            return Err("settings-invalid-value");
        }
        next.retain(|entry| entry.tool != update.tool || entry.cwd != cwd);
        if !update.reset {
            next.push(FolderSettings {
                tool: update.tool,
                cwd,
                model: update.model,
                collect_traces: update.collect_traces,
                cli_options: update.cli_options,
            });
        }
    }
    settings.managed_folder_settings = next;
    Ok(())
}

/// A request that names no folder runs in the person's home directory. The
/// daemon resolves it here so no shell guesses at a path. The folder then
/// meets every check a picked one does: it must exist and be a directory,
/// it is canonicalized, and the tool's project-config check runs on it.
pub fn default_cwd(home: Option<PathBuf>) -> Result<PathBuf, ManagedError> {
    canonical(&home.ok_or(ManagedError::InvalidRequest)?)
}

/// `params` with a `cwd` of `home` where it names none. A `cwd` that is
/// present is left for the usual checks, even if it is empty or null.
pub fn with_default_cwd(
    mut params: serde_json::Value,
    home: Option<PathBuf>,
) -> Result<serde_json::Value, ManagedError> {
    if let Some(object) = params.as_object_mut()
        && !object.contains_key("cwd")
    {
        object.insert("cwd".into(), serde_json::json!(default_cwd(home)?));
    }
    Ok(params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_request_without_a_folder_gets_the_home_directory() {
        let home = tempfile::tempdir().unwrap();
        let out = with_default_cwd(json!({"tool":"claude"}), Some(home.path().into())).unwrap();
        assert_eq!(
            out["cwd"],
            json!(std::fs::canonicalize(home.path()).unwrap())
        );
        assert_eq!(out["tool"], "claude");
    }

    #[test]
    fn a_named_folder_is_left_alone_for_the_usual_checks() {
        let home = tempfile::tempdir().unwrap();
        for named in [json!("/x/y"), json!(""), json!(null)] {
            let out =
                with_default_cwd(json!({"cwd": named.clone()}), Some(home.path().into())).unwrap();
            assert_eq!(out["cwd"], named);
        }
    }

    #[test]
    fn no_home_directory_refuses_instead_of_guessing() {
        assert!(with_default_cwd(json!({}), None).is_err());
        assert!(with_default_cwd(json!({}), Some("relative".into())).is_err());
        let gone = tempfile::tempdir().unwrap().path().join("missing");
        assert!(with_default_cwd(json!({}), Some(gone)).is_err());
    }
}
