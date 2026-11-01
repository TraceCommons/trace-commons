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

/// Whether a `-c` override, as a TOML table, reaches one of Codex's
/// secondary model keys ([`super::codex::SECONDARY_MODEL_KEYS`]): by its
/// dotted key, or by setting a table that holds it (`agents={...}`).
fn names_secondary_model(table: &toml::Table) -> bool {
    fn walk(prefix: &str, table: &toml::Table) -> bool {
        table.iter().any(|(key, value)| {
            let path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            super::codex::SECONDARY_MODEL_KEYS.contains(&path.as_str())
                || value.as_table().is_some_and(|inner| walk(&path, inner))
        })
    }
    walk("", table)
}

/// Managed account isolation, routing and session identity belong to the launcher.
/// Model, reasoning, sandbox and permission choices remain configurable,
/// except a model other than the session's own (#1798 review): Claude
/// Code's `--fallback-model` and `--agents`, and Codex's secondary model
/// keys, are refused, so no secondary model can be less private than the
/// model in use.
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
                        | "--fallback-model"
                        | "--agents"
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
                    if names_secondary_model(&parsed)
                        || config
                            .parse::<toml::Table>()
                            .is_ok_and(|whole| names_secondary_model(&whole))
                    {
                        return Err(invalid());
                    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The person's own `-c` cannot name a model Codex uses besides the
    /// session's, by dotted key, quoted key or a table holding it; the
    /// tables' other keys stay theirs to set.
    #[test]
    fn codex_options_cannot_set_a_secondary_model() {
        for refused in [
            r#"-c review_model="openai/gpt-5.5""#,
            r#"--config review_model="openai/gpt-5.5""#,
            r#"--config=agents.default_subagent_model="openai/gpt-5.5""#,
            r#"-c 'agents."default_subagent_model"="openai/gpt-5.5"'"#,
            r#"-c memories.extract_model="x/y""#,
            r#"-c memories.consolidation_model="x/y""#,
            r#"-c 'agents={default_subagent_model="x/y"}'"#,
            r#"-c 'memories={extract_model="x/y", use_memories=true}'"#,
        ] {
            assert_eq!(
                parse(ToolId::Codex, refused),
                Err(ManagedError::InvalidCliOptions),
                "{refused}"
            );
        }
        for allowed in [
            "-c agents.max_depth=2",
            "-c memories.use_memories=false",
            "-c 'agents={max_depth=2}'",
            "-m z-ai/glm-5.3",
        ] {
            assert!(parse(ToolId::Codex, allowed).is_ok(), "{allowed}");
        }
    }
}

#[cfg(test)]
mod claude_tests {
    use super::*;

    /// #1798 review: Claude Code's own secondary-model flags could send
    /// work to a model less private than the session's, so they are
    /// refused, as Codex's secondary keys are. `--model` stays: the
    /// launcher follows it (`models::flag_model`).
    #[test]
    fn claude_options_cannot_name_a_secondary_model() {
        for refused in [
            "--fallback-model anthropic/claude-opus-5-5",
            "--fallback-model=anthropic/claude-opus-5-5",
            r#"--agents '{"x":{"description":"d","prompt":"p","model":"anthropic/claude-opus-5-5"}}'"#,
            "--agents=agents.json",
        ] {
            assert_eq!(
                parse(ToolId::Claude, refused),
                Err(ManagedError::InvalidCliOptions),
                "{refused}"
            );
        }
        assert!(parse(ToolId::Claude, "--model Qwen/Qwen3.8-27B").is_ok());
    }
}
