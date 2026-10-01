//! Every command the app registers must be granted to the main window.
//!
//! `build.rs` lists the commands in `TAURI_COMMANDS`, and Tauri generates an
//! `allow-<command>` permission for each. A command only reaches the webview
//! when `permissions/default.toml` (the `default` set the `default`
//! capability grants) names that permission. Miss one and the call fails at
//! runtime with "not allowed", which the shell can only report as missing
//! copy: fifteen commands were registered and silently refused this way,
//! including the copy that enables the first run's Done button.

use std::collections::BTreeSet;
use std::path::Path;

fn read(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

/// The quoted names inside `const TAURI_COMMANDS: &[&str] = &[ ... ];`.
fn registered_commands() -> BTreeSet<String> {
    let source = read("build.rs");
    let start = source
        .find("const TAURI_COMMANDS")
        .expect("build.rs declares TAURI_COMMANDS");
    let body = &source[start..];
    let end = body.find("];").expect("TAURI_COMMANDS is closed");
    let commands: BTreeSet<String> = body[..end]
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect();
    assert!(!commands.is_empty(), "TAURI_COMMANDS parsed as empty");
    commands
}

/// The `allow-*` entries of the `[default]` permission set, as command names.
fn granted_commands() -> BTreeSet<String> {
    let manifest = read("permissions/default.toml");
    let start = manifest
        .find("permissions = [")
        .expect("default.toml has a permissions list");
    let body = &manifest[start..];
    let end = body.find(']').expect("the permissions list is closed");
    body[..end]
        .split('"')
        .skip(1)
        .step_by(2)
        .filter_map(|permission| permission.strip_prefix("allow-"))
        .map(|command| command.replace('-', "_"))
        .collect()
}

#[test]
fn every_registered_command_is_granted_to_the_main_window() {
    let missing: Vec<_> = registered_commands()
        .difference(&granted_commands())
        .cloned()
        .collect();
    assert!(
        missing.is_empty(),
        "registered in build.rs but not granted in permissions/default.toml \
         (add `allow-<command>` for each): {missing:?}"
    );
}

#[test]
fn every_granted_command_is_registered() {
    let stale: Vec<_> = granted_commands()
        .difference(&registered_commands())
        .cloned()
        .collect();
    assert!(
        stale.is_empty(),
        "granted in permissions/default.toml but not registered in build.rs: {stale:?}"
    );
}
