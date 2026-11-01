//! Per-tool launch options. Quoting groups argv; no shell is ever invoked.

use super::{ManagedError, ToolId};
use crate::{config::ConfigStore, daemon::settings::DaemonSettings};
use std::collections::BTreeMap;

const MAX_BYTES: usize = 16 * 1024;

pub fn snapshot(store: &ConfigStore) -> Result<BTreeMap<ToolId, String>, ManagedError> {
    let mut values = DaemonSettings::load(store)
        .map_err(|_| ManagedError::StorageUnavailable)?
        .managed_cli_options;
    // A saved value this build no longer accepts is still shown, so the
    // person can change it; a launch with it is refused
    // ([`arguments`] parses it again).
    for tool in [ToolId::Claude, ToolId::Codex, ToolId::Opencode] {
        values.entry(tool).or_default();
    }
    Ok(values)
}

pub fn arguments(store: &ConfigStore, tool: ToolId) -> Result<Vec<String>, ManagedError> {
    let values = snapshot(store)?;
    parse(
        tool,
        values.get(&tool).map(String::as_str).unwrap_or_default(),
    )
}

/// POSIX-style single/double quotes and backslash escapes, without expansions,
/// comments, command substitution, globbing, or evaluation of shell operators.
pub fn parse(tool: ToolId, options: &str) -> Result<Vec<String>, ManagedError> {
    if options.len() > MAX_BYTES || options.contains('\0') {
        return Err(ManagedError::InvalidCliOptions);
    }
    let mut args = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut started = false;
    let mut chars = options.chars().peekable();
    while let Some(ch) = chars.next() {
        match (quote, ch) {
            (Some('\''), '\'') | (Some('"'), '"') => quote = None,
            (Some('\''), _) => word.push(ch),
            (_, '\\') => {
                let next = chars.next().ok_or(ManagedError::InvalidCliOptions)?;
                if quote == Some('"') && !matches!(next, '$' | '`' | '"' | '\\' | '\n') {
                    word.push('\\');
                }
                if next != '\n' {
                    word.push(next);
                    started = true;
                }
            }
            (None, '\'' | '"') => {
                quote = Some(ch);
                started = true;
            }
            (None, ch) if ch.is_whitespace() => {
                if started {
                    args.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            _ => {
                word.push(ch);
                started = true;
            }
        }
    }
    if quote.is_some() {
        return Err(ManagedError::InvalidCliOptions);
    }
    if started {
        args.push(word);
    }
    validate(tool, &args)?;
    Ok(args)
}

/// Managed account isolation, routing and session identity belong to the launcher.
/// Model, reasoning, sandbox and permission choices remain configurable.
fn validate(tool: ToolId, args: &[String]) -> Result<(), ManagedError> {
    let invalid = || ManagedError::InvalidCliOptions;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let flag = arg.split('=').next().unwrap_or_default();
        if flag == "--" {
            return Err(invalid());
        }
        match tool {
            ToolId::Claude => {
                if matches!(
                    flag,
                    "--settings"
                        | "--setting-sources"
                        | "--session-id"
                        | "--resume"
                        | "--continue"
                        // Forking writes the resumed turns to a new
                        // conversation the launcher would not collect.
                        | "--fork-session"
                        | "-r"
                        | "-c"
                ) || arg
                    .strip_prefix('-')
                    .filter(|short| !short.starts_with('-'))
                    .is_some_and(|short| {
                        // Boolean short flags may be clustered before continue or
                        // resume (-pc, -prSESSION). Stop at any value-taking flag:
                        // -dcore names debug categories, not a continue request.
                        matches!(
                            short.trim_start_matches(['p', 'h', 'v']).chars().next(),
                            Some('c' | 'r')
                        )
                    })
                {
                    return Err(invalid());
                }
            }
            ToolId::Codex => {
                // The value of a flag that takes exactly one is never a
                // subcommand (`-m fork` names a model). `-i` is left out: it
                // takes several, so what follows it may still be checked.
                if matches!(
                    arg.as_str(),
                    "-m" | "--model"
                        | "-s"
                        | "--sandbox"
                        | "-a"
                        | "--ask-for-approval"
                        | "--enable"
                        | "--disable"
                        | "--add-dir"
                        | "--remote-auth-token-env"
                ) {
                    iter.next().ok_or_else(invalid)?;
                    continue;
                }
                // Codex continues a conversation through its `resume` and
                // `fork` subcommands, which the launcher alone adds.
                if matches!(
                    flag,
                    "--profile"
                        | "-p"
                        | "--cd"
                        | "-C"
                        | "--remote"
                        | "--oss"
                        | "--local-provider"
                        | "resume"
                        | "fork"
                        | "--last"
                ) || (arg.starts_with("-p") && !arg.starts_with("--"))
                    || (arg.starts_with("-C") && !arg.starts_with("--"))
                {
                    return Err(invalid());
                }
                let config = if matches!(arg.as_str(), "-c" | "--config") {
                    Some(iter.next().ok_or_else(invalid)?.as_str())
                } else if let Some(value) = arg.strip_prefix("--config=") {
                    Some(value)
                } else if arg.starts_with("-c") && !arg.starts_with("--") {
                    Some(&arg[2..])
                } else {
                    None
                };
                if let Some(config) = config {
                    let (key, _) = config.split_once('=').ok_or_else(invalid)?;
                    // Decode TOML key syntax, including quoted keys and Unicode
                    // escapes, exactly as Codex's configuration loader does.
                    let parsed: toml::Table = format!("{key}=0").parse().map_err(|_| invalid())?;
                    let root = parsed.keys().next().ok_or_else(invalid)?.as_str();
                    if matches!(
                        root,
                        "model_provider"
                            | "model_providers"
                            | "model_catalog_json"
                            | "profile"
                            | "profiles"
                            | "cli_auth_credentials_store"
                            | "forced_login_method"
                            | "forced_chatgpt_workspace_id"
                            | "chatgpt_base_url"
                    ) {
                        return Err(invalid());
                    }
                }
            }
            ToolId::Opencode => {
                if matches!(
                    flag,
                    "--session"
                        | "-s"
                        | "--model"
                        | "-m"
                        | "--continue"
                        | "-c"
                        | "--fork"
                        | "--config"
                        | "--attach"
                        | "--server"
                        | "--standalone"
                        | "--no-standalone"
                        | "--directory"
                ) || ["-s", "-m", "-c"]
                    .iter()
                    .any(|prefix| arg.starts_with(prefix) && !arg.starts_with("--"))
                {
                    return Err(invalid());
                }
            }
        }
    }
    Ok(())
}
