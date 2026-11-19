//! Red Team sandbox settings: file inputs, network allowlist, and resource limits.

use super::{ManagedError, ToolId};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Sandbox configuration carried on a Red Team launch request and context.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxSettings {
    #[serde(default)]
    pub inputs: Vec<PathBuf>,
    #[serde(default)]
    pub allow_hosts: Vec<String>,
    #[serde(default)]
    pub packages: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpus: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_mib: Option<u32>,
}

/// Validate and normalize `sandbox` settings for `tool`.
///
/// - Any `Some` value on a non-Red-Team tool is refused with `InvalidRequest`.
/// - Each input path is checked for existence, must not be a symlink, is
///   canonicalized, and must not be `/` or `$HOME`.
/// - Network allowlist entries are validated (and `packages: true` adds the
///   `packages` preset) using `rts::network::expand` on macOS.
pub fn validate(
    tool: ToolId,
    s: Option<&SandboxSettings>,
) -> Result<Option<SandboxSettings>, ManagedError> {
    validate_with_home(tool, s, dirs::home_dir().as_deref())
}

pub(crate) fn validate_with_home(
    tool: ToolId,
    s: Option<&SandboxSettings>,
    home: Option<&Path>,
) -> Result<Option<SandboxSettings>, ManagedError> {
    if tool != ToolId::RedTeam {
        return if s.is_some() {
            Err(ManagedError::InvalidRequest)
        } else {
            Ok(None)
        };
    }
    let Some(s) = s else {
        return Ok(None);
    };

    let canonical_home = home.and_then(|p| std::fs::canonicalize(p).ok());
    let mut inputs = Vec::with_capacity(s.inputs.len());
    for input in &s.inputs {
        let meta = std::fs::symlink_metadata(input).map_err(|_| {
            ManagedError::SandboxInputRefused(format!(
                "input path does not exist: {}",
                input.display()
            ))
        })?;
        if meta.file_type().is_symlink() {
            return Err(ManagedError::SandboxInputRefused(format!(
                "input path must not be a symlink: {}",
                input.display()
            )));
        }
        let canonical = std::fs::canonicalize(input).map_err(|_| {
            ManagedError::SandboxInputRefused(format!(
                "input path cannot be resolved: {}",
                input.display()
            ))
        })?;
        if canonical.parent().is_none() {
            return Err(ManagedError::SandboxInputRefused(format!(
                "input path must not be the filesystem root: {}",
                input.display()
            )));
        }
        if canonical_home.as_ref().is_some_and(|h| *h == canonical) {
            return Err(ManagedError::SandboxInputRefused(format!(
                "input path must not be the home directory: {}",
                input.display()
            )));
        }
        inputs.push(canonical);
    }

    let mut written = s.allow_hosts.clone();
    if s.packages && !written.iter().any(|h| h.eq_ignore_ascii_case("packages")) {
        written.push("packages".to_owned());
    }

    let allow_hosts = expand_hosts(&written)?;

    Ok(Some(SandboxSettings {
        inputs,
        allow_hosts,
        packages: s.packages,
        cpus: s.cpus,
        memory_mib: s.memory_mib,
    }))
}

#[cfg(target_os = "macos")]
fn expand_hosts(written: &[String]) -> Result<Vec<String>, ManagedError> {
    let entries = rts::network::expand(written).map_err(ManagedError::SandboxHostRefused)?;
    Ok(entries.iter().map(rts::network::Entry::display).collect())
}

#[cfg(not(target_os = "macos"))]
fn expand_hosts(written: &[String]) -> Result<Vec<String>, ManagedError> {
    Ok(written.to_vec())
}

#[cfg(test)]
#[path = "sandbox_tests.rs"]
mod tests;
