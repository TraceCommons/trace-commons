//! The hooks a managed Codex launch declares on its command line, each
//! trusted by hash, so no person's hook trust is changed.
//!
//! Codex takes `-c hooks.<Event>=[...]` as one session-flags source,
//! `/<session-flags>/config.toml`. Each declared group gets the key
//! `<source>:<event in snake case>:<group>:<hook>`, and runs only when
//! `hooks.state.<key>.trusted_hash` equals the hash of its normalized
//! declaration ([`hash`]). Both were checked against Codex 0.164's own
//! `hooks/list` (key, `currentHash` and `trustStatus`), on Unix only:
//! elsewhere [`configure`] refuses, and the launch goes on without hooks.
//!
//! Every hook is declared in a group of its own. All of a launch's hooks
//! go in one call, since a second `-c hooks.SessionStart=` or
//! `-c hooks.state=` would replace the first.

use super::ManagedError;
use sha2::{Digest, Sha256};
use std::process::Command;

const SOURCE: &str = "/<session-flags>/config.toml";

/// One hook: a command run on `event` (`SessionStart`, `PostToolUse`, ...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hook {
    pub event: &'static str,
    pub command: String,
    pub timeout: u64,
}

/// `PostToolUse` as `post_tool_use`.
fn snake(event: &str) -> String {
    let mut out = String::new();
    for (i, c) in event.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// The trust key of the `group`th group declared for `event`.
fn key(event: &str, group: usize) -> Result<String, ManagedError> {
    // Only Unix's session-flags origin has been checked against the native
    // hook engine. An unknown origin must leave the hooks off, not trust a
    // guessed key.
    if cfg!(unix) {
        Ok(format!("{SOURCE}:{}:{group}:0", snake(event)))
    } else {
        Err(ManagedError::InvalidRequest)
    }
}

/// Codex 0.164 drops optional fields, adds the effective timeout and the
/// async flag, and hashes the sorted compact JSON of the group.
pub fn hash(hook: &Hook) -> Result<String, ManagedError> {
    let mut identity = serde_json::json!({
        "event_name": snake(hook.event),
        "hooks": [{"type": "command", "command": hook.command, "timeout": hook.timeout, "async": false}],
    });
    identity.sort_all_objects();
    let bytes = serde_json::to_vec(&identity).map_err(|_| ManagedError::InvalidRequest)?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

/// Whether the person's own CLI options already declare hooks. Those cannot
/// be silently replaced, so the launch then declares none.
fn has_cli_hooks(args: &[&str]) -> Result<bool, ManagedError> {
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let config = if matches!(*arg, "-c" | "--config") {
            Some(*args.next().ok_or(ManagedError::InvalidRequest)?)
        } else if let Some(value) = arg.strip_prefix("--config=") {
            Some(value)
        } else if arg.starts_with("-c") && !arg.starts_with("--") {
            Some(&arg[2..])
        } else {
            None
        };
        if let Some(config) = config {
            let (key, _) = config.split_once('=').ok_or(ManagedError::InvalidRequest)?;
            let key: toml::Table = format!("{key}=0")
                .parse()
                .map_err(|_| ManagedError::InvalidRequest)?;
            if key.contains_key("hooks") {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// The `-c` arguments that declare and trust `hooks`. Pure.
pub fn arguments(hooks: &[Hook]) -> Result<Vec<String>, ManagedError> {
    let mut events: Vec<&'static str> = Vec::new();
    for hook in hooks {
        if !events.contains(&hook.event) {
            events.push(hook.event);
        }
    }
    let mut args = Vec::new();
    let mut trust = Vec::new();
    for event in events {
        let mut groups = Vec::new();
        for (group, hook) in hooks.iter().filter(|hook| hook.event == event).enumerate() {
            let quoted = toml::Value::String(hook.command.clone()).to_string();
            groups.push(format!(
                "{{hooks=[{{type=\"command\",command={quoted},timeout={}}}]}}",
                hook.timeout
            ));
            // A TOML table's display is not an inline value. Serialize the
            // state map explicitly so the CLI parses each key as one key.
            let key = toml::Value::String(key(event, group)?).to_string();
            let hash = toml::Value::String(hash(hook)?).to_string();
            trust.push(format!("{key}={{trusted_hash={hash}}}"));
        }
        args.push("-c".to_owned());
        args.push(format!("hooks.{event}=[{}]", groups.join(",")));
    }
    if !trust.is_empty() {
        args.push("-c".to_owned());
        args.push(format!("hooks.state={{{}}}", trust.join(",")));
    }
    Ok(args)
}

/// Declare and trust `hooks` on `command`, after every other argument, so a
/// hook the person passed is seen. Refuses when they passed their own CLI
/// hooks or a `--`; the caller then launches without any.
pub fn configure(command: &mut Command, hooks: &[Hook]) -> Result<(), ManagedError> {
    if hooks.is_empty() {
        return Ok(());
    }
    let args = command
        .get_args()
        .map(|arg| arg.to_str().ok_or(ManagedError::InvalidRequest))
        .collect::<Result<Vec<_>, _>>()?;
    if args.contains(&"--") || has_cli_hooks(&args)? {
        return Err(ManagedError::InvalidRequest);
    }
    let added = arguments(hooks)?;
    // Codex 0.164's daemon_startup::config_exclusion forces an embedded
    // server for these hook overrides. No TUI-only flag is needed, so
    // subcommands such as `exec` retain their supported CLI arguments.
    command.args(added);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hook(event: &'static str, command: &str, timeout: u64) -> Hook {
        Hook {
            event,
            command: command.to_owned(),
            timeout,
        }
    }

    /// Both values were read from Codex 0.164's `hooks/list` for this
    /// declaration: two SessionStart groups and one PostToolUse group.
    #[cfg(unix)]
    #[test]
    fn keys_and_hashes_match_codex() {
        assert_eq!(
            hash(&hook("SessionStart", "echo b", 10)).unwrap(),
            "sha256:221f6c9c63abf3d54a52befb16d7b675f2554198be45244b4829fc87939b45b2"
        );
        assert_eq!(
            hash(&hook("PostToolUse", "echo c", 10)).unwrap(),
            "sha256:cbde2c1f0e4fc14bb62189db97ccbab5e0387d0cea1a433895bd1644f1a724b0"
        );
        assert_eq!(
            hash(&hook("SessionStart", "/usr/bin/true", 600)).unwrap(),
            "sha256:5bcf5439a4b245c57c9e7f3910368077fa8b85b8231cd151f801f58f1414d57e"
        );
        assert_eq!(
            key("SessionStart", 1).unwrap(),
            "/<session-flags>/config.toml:session_start:1:0"
        );
        assert_eq!(
            key("UserPromptSubmit", 0).unwrap(),
            "/<session-flags>/config.toml:user_prompt_submit:0:0"
        );
    }

    #[cfg(unix)]
    #[test]
    fn every_hook_is_declared_once_with_one_trust_map() {
        let args = arguments(&[
            hook("SessionStart", "bind 'x'", 3),
            hook("SessionStart", "econ", 10),
            hook("PostToolUse", "econ", 10),
        ])
        .unwrap();
        assert_eq!(args.len(), 6, "{args:?}");
        assert_eq!(args[0], "-c");
        assert_eq!(
            args[1],
            r#"hooks.SessionStart=[{hooks=[{type="command",command="bind 'x'",timeout=3}]},{hooks=[{type="command",command="econ",timeout=10}]}]"#
        );
        assert_eq!(
            args[3],
            r#"hooks.PostToolUse=[{hooks=[{type="command",command="econ",timeout=10}]}]"#
        );
        assert!(args[5].starts_with("hooks.state={\"/<session-flags>/config.toml:session_start:0:0\"={trusted_hash=\"sha256:"));
        assert!(args[5].contains("session_start:1:0"));
        assert!(args[5].contains("post_tool_use:0:0"));
        // Each value parses as TOML, as Codex parses it.
        for value in [&args[1], &args[3], &args[5]] {
            let (_, value) = value.split_once('=').unwrap();
            format!("x={value}").parse::<toml::Table>().unwrap();
        }
    }

    #[test]
    fn a_person_s_own_cli_hooks_are_never_replaced() {
        let mut command = Command::new("codex");
        command.args(["-c", "hooks.Stop=[]"]);
        assert!(configure(&mut command, &[hook("Stop", "econ", 10)]).is_err());
        let mut command = Command::new("codex");
        command.args(["-c", "model=\"x\""]);
        assert!(configure(&mut command, &[]).is_ok());
        assert_eq!(command.get_args().count(), 2);
    }
}
