//! Fixed official setup commands, opened only by the host's chosen terminal.
use super::{ManagedError, ToolId};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSetupAction {
    Install,
    Update,
}

fn name(tool: ToolId) -> &'static str {
    match tool {
        ToolId::Claude => "claude",
        ToolId::Codex => "codex",
        ToolId::Opencode => "opencode",
    }
}

fn quote(path: &Path) -> Result<String, ManagedError> {
    let value = path.to_str().ok_or(ManagedError::InvalidRequest)?;
    if value.len() > 4096 {
        return Err(ManagedError::InvalidRequest);
    }
    Ok(format!("'{}'", value.replace('\'', "'\\''")))
}

fn installer(tool: ToolId) -> String {
    let (url, interpreter) = match tool {
        ToolId::Claude => ("https://claude.ai/install.sh", "bash"),
        ToolId::Codex => ("https://chatgpt.com/codex/install.sh", "sh"),
        // Fresh installs use the supported current major. Updates below use
        // the installed executable's updater and preserve its existing major.
        ToolId::Opencode => ("https://opencode.ai/v2/install", "bash"),
    };
    // A plain curl | shell pipeline hides curl failures in POSIX shells.
    format!("installer=$(curl -fsSL '{url}') && printf '%s\\n' \"$installer\" | {interpreter}")
}

fn brew_owner(tool: ToolId, native: &Path) -> Option<(PathBuf, &'static str)> {
    for root in native.ancestors() {
        if root.file_name()?.to_str()? != "Caskroom" {
            continue;
        }
        let cask = native.strip_prefix(root).ok()?.iter().next()?.to_str()?;
        let cask = match (tool, cask) {
            (ToolId::Claude, "claude-code") => "claude-code",
            (ToolId::Claude, "claude-code@latest") => "claude-code@latest",
            (ToolId::Codex, "codex") => "codex",
            _ => return None,
        };
        return Some((root.parent()?.to_path_buf(), cask));
    }
    None
}

fn npm_owner(tool: ToolId, native: &Path) -> Option<PathBuf> {
    let package = match tool {
        ToolId::Claude => Path::new("@anthropic-ai/claude-code"),
        ToolId::Codex => Path::new("@openai/codex"),
        ToolId::Opencode => return None,
    };
    for root in native.ancestors() {
        if root.file_name()?.to_str()? == "node_modules"
            && root.parent()?.file_name()? == "lib"
            && native.strip_prefix(root).ok()?.starts_with(package)
        {
            return Some(root.parent()?.parent()?.to_path_buf());
        }
    }
    None
}

// Claude native installs resolve .local/bin/claude into a versioned binary.
// The update can replace that version, so report through its stable symlink.
fn claude_stable_binary(native: &Path) -> Option<PathBuf> {
    for versions in native.ancestors() {
        if versions.file_name()?.to_str()? != "versions" {
            continue;
        }
        let claude = versions.parent()?;
        let share = claude.parent()?;
        let local = share.parent()?;
        if claude.file_name()? == "claude"
            && share.file_name()? == "share"
            && local.file_name()? == ".local"
        {
            return Some(local.join("bin/claude"));
        }
    }
    None
}

/// Build a bounded shell body from fixed vendor commands and a discovered
/// executable. Planning performs no subprocesses and never reads credentials.
/// Setup ends with a version check, never a login or interactive tool session.
pub fn command(
    tool: ToolId,
    action: ToolSetupAction,
    native: Option<&Path>,
) -> Result<String, ManagedError> {
    let mut version = name(tool).to_owned();
    let setup = match action {
        ToolSetupAction::Install => installer(tool),
        ToolSetupAction::Update => {
            let native = native.ok_or(ManagedError::ToolUnavailable)?;
            if !native.is_absolute() || !native.is_file() {
                return Err(ManagedError::ToolUnavailable);
            }
            let native = native
                .canonicalize()
                .map_err(|_| ManagedError::ToolUnavailable)?;
            if let Some((prefix, cask)) = brew_owner(tool, &native) {
                let brew = prefix.join("bin/brew");
                if !brew.is_file() {
                    return Err(ManagedError::ToolUnavailable);
                }
                version = quote(&prefix.join("bin").join(name(tool)))?;
                format!(
                    "{} update && {} upgrade --cask {cask}",
                    quote(&brew)?,
                    quote(&brew)?
                )
            } else if let Some(prefix) = npm_owner(tool, &native) {
                let npm = prefix.join("bin/npm");
                let npm = if npm.is_file() {
                    format!("npm={}", quote(&npm)?)
                } else {
                    "npm=$(command -v npm)".to_owned()
                };
                let package = match tool {
                    ToolId::Claude => "@anthropic-ai/claude-code@latest",
                    ToolId::Codex => "@openai/codex@latest",
                    ToolId::Opencode => unreachable!(),
                };
                version = quote(&prefix.join("bin").join(name(tool)))?;
                // An explicit prefix updates the detected installation even
                // when the terminal selects another Node/npm global prefix.
                format!(
                    "{npm} && \"$npm\" install -g --prefix {} {package}",
                    quote(&prefix)?
                )
            } else {
                match tool {
                    ToolId::Claude => {
                        if let Some(stable) = claude_stable_binary(&native) {
                            version = quote(&stable)?;
                        }
                        format!("{} update", quote(&native)?)
                    }
                    ToolId::Codex => installer(tool),
                    ToolId::Opencode => {
                        // Its own updater preserves npm/bun/brew/native method.
                        version = quote(&native)?;
                        format!("{} upgrade", quote(&native)?)
                    }
                }
            }
        }
    };
    let body = format!("{setup} && {version} --version");
    if body.len() > 16_384 {
        return Err(ManagedError::InvalidRequest);
    }
    Ok(body)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt, process::Command};

    struct Fixture {
        dir: tempfile::TempDir,
        bin: PathBuf,
        log: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::Builder::new()
                .tempdir_in(std::env::temp_dir().canonicalize().unwrap())
                .unwrap();
            let bin = dir.path().join("bin");
            fs::create_dir_all(&bin).unwrap();
            let log = dir.path().join("calls");
            Self { dir, bin, log }
        }
        fn executable(&self, path: &Path, body: &str) {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        fn tool(&self, tool: ToolId) {
            self.executable(
                &self.bin.join(name(tool)),
                &format!(
                    "printf '%s\\n' '{} ' \"$*\" >> \"$TC_SETUP_TEST_LOG\"",
                    name(tool)
                ),
            );
        }
        fn run(&self, body: &str) -> std::process::Output {
            Command::new("/bin/sh")
                .arg("-c")
                .arg(body)
                .env("PATH", format!("{}:/usr/bin:/bin", self.bin.display()))
                .env("TC_SETUP_TEST_LOG", &self.log)
                .output()
                .unwrap()
        }
        fn calls(&self) -> String {
            fs::read_to_string(&self.log).unwrap_or_default()
        }
    }

    #[test]
    fn actions_are_only_the_fixed_wire_choices() {
        assert_eq!(
            serde_json::from_str::<ToolSetupAction>("\"install\"").unwrap(),
            ToolSetupAction::Install
        );
        assert_eq!(
            serde_json::from_str::<ToolSetupAction>("\"update\"").unwrap(),
            ToolSetupAction::Update
        );
        assert!(serde_json::from_str::<ToolSetupAction>("\"update; evil\"").is_err());
    }

    #[test]
    fn each_install_executes_only_the_official_installer_then_version() {
        for (tool, url) in [
            (ToolId::Claude, "https://claude.ai/install.sh"),
            (ToolId::Codex, "https://chatgpt.com/codex/install.sh"),
            (ToolId::Opencode, "https://opencode.ai/v2/install"),
        ] {
            let f = Fixture::new();
            f.tool(tool);
            f.executable(&f.bin.join("curl"), "printf '%s\\n' \"curl $*\" >> \"$TC_SETUP_TEST_LOG\"\nprintf '%s\\n' 'printf \"installed\\n\" >> \"$TC_SETUP_TEST_LOG\"'");
            let body = command(tool, ToolSetupAction::Install, None).unwrap();
            let output = f.run(&body);
            assert!(output.status.success(), "{output:?}");
            let calls = f.calls();
            assert!(calls.contains(url), "{calls}");
            assert!(calls.contains("installed"));
            assert!(calls.contains("--version"));
            assert!(!body.contains("sudo") && !body.contains("login"));
        }
    }

    #[test]
    fn native_updates_quote_hostile_paths_and_do_not_launch_sessions() {
        for tool in [ToolId::Claude, ToolId::Opencode] {
            let f = Fixture::new();
            f.tool(tool);
            let native = f.dir.path().join("native ' $(printf injected); tool");
            f.executable(
                &native,
                "printf '%s\\n' \"native $*\" >> \"$TC_SETUP_TEST_LOG\"",
            );
            let body = command(tool, ToolSetupAction::Update, Some(&native)).unwrap();
            // Updating an installed OpenCode (including v1) invokes its own
            // updater, never the fresh-install v2 migration script.
            assert!(!body.contains("curl") && !body.contains("/v2/install"));
            assert!(f.run(&body).status.success());
            let calls = f.calls();
            assert!(calls.contains(if tool == ToolId::Claude {
                "native update"
            } else {
                "native upgrade"
            }));
            assert!(calls.contains("--version"));
            assert!(!calls.contains("injected"));
        }
    }

    #[test]
    fn standalone_codex_update_uses_the_official_installer() {
        let f = Fixture::new();
        f.tool(ToolId::Codex);
        let native = f.dir.path().join("codex-native");
        f.executable(&native, "exit 99");
        f.executable(
            &f.bin.join("curl"),
            "printf '%s\\n' 'printf \"codex-installer\\n\" >> \"$TC_SETUP_TEST_LOG\"'",
        );
        let body = command(ToolId::Codex, ToolSetupAction::Update, Some(&native)).unwrap();
        assert!(f.run(&body).status.success());
        assert!(f.calls().contains("codex-installer"));
        assert!(f.calls().contains("--version"));
    }

    #[test]
    fn brew_updates_preserve_the_existing_cask_and_matching_prefix() {
        for (tool, cask) in [
            (ToolId::Claude, "claude-code"),
            (ToolId::Claude, "claude-code@latest"),
            (ToolId::Codex, "codex"),
        ] {
            let f = Fixture::new();
            let prefix = f.dir.path().join("brew prefix ' quoted");
            let native = prefix.join("Caskroom").join(cask).join("1.0/bin/native");
            f.executable(&native, "exit 99");
            f.executable(
                &prefix.join("bin/brew"),
                "printf '%s\\n' \"brew $*\" >> \"$TC_SETUP_TEST_LOG\"",
            );
            f.executable(
                &prefix.join("bin").join(name(tool)),
                "printf '%s\\n' \"version $*\" >> \"$TC_SETUP_TEST_LOG\"",
            );
            assert!(
                f.run(&command(tool, ToolSetupAction::Update, Some(&native)).unwrap())
                    .status
                    .success()
            );
            let calls = f.calls();
            assert!(calls.starts_with("brew update\n"));
            assert!(calls.contains(&format!("brew upgrade --cask {cask}")));
            assert!(calls.contains("version --version"));
            fs::write(&f.log, "").unwrap();
            f.executable(
                &prefix.join("bin/brew"),
                "printf '%s\\n' \"brew $*\" >> \"$TC_SETUP_TEST_LOG\"; exit 7",
            );
            assert!(
                !f.run(&command(tool, ToolSetupAction::Update, Some(&native)).unwrap())
                    .status
                    .success()
            );
            assert_eq!(f.calls(), "brew update\n");
        }
    }

    #[test]
    fn npm_updates_use_the_detected_prefix_even_if_runtime_default_differs() {
        for (tool, package) in [
            (ToolId::Claude, "@anthropic-ai/claude-code"),
            (ToolId::Codex, "@openai/codex"),
        ] {
            let f = Fixture::new();
            let prefix = f.dir.path().join("npm prefix ' quoted");
            let native = prefix
                .join("lib/node_modules")
                .join(package)
                .join("bin/cli.js");
            f.executable(&native, "exit 99");
            let npm = prefix.join("bin/npm");
            // Runtime npm's default would target a different installation.
            // It must receive the complete original prefix as one argument.
            f.executable(&npm, &format!(
                "if [ \"$1\" = prefix ]; then printf '%s\\n' /another/prefix; exit 0; fi\n[ \"$1\" = install ] && [ \"$2\" = -g ] && [ \"$3\" = --prefix ] && [ \"$4\" = {} ] && [ \"$5\" = {package}@latest ] || exit 9\nprintf '%s\\n' npm-correct-prefix >> \"$TC_SETUP_TEST_LOG\"",
                quote(&prefix).unwrap()
            ));
            f.executable(
                &prefix.join("bin").join(name(tool)),
                "printf '%s\\n' \"version $*\" >> \"$TC_SETUP_TEST_LOG\"",
            );
            let body = command(tool, ToolSetupAction::Update, Some(&native)).unwrap();
            assert!(f.run(&body).status.success());
            assert!(f.calls().contains("npm-correct-prefix"));
            assert!(f.calls().contains("version --version"));
            // A prefix with no local npm can use the current executable,
            // still passing exactly the original install destination.
            fs::copy(&npm, f.bin.join("npm")).unwrap();
            fs::remove_file(&npm).unwrap();
            fs::write(&f.log, "").unwrap();
            let body = command(tool, ToolSetupAction::Update, Some(&native)).unwrap();
            assert!(f.run(&body).status.success());
            assert!(f.calls().contains("npm-correct-prefix"));
        }
    }

    #[test]
    fn native_claude_version_uses_the_stable_symlink_after_update() {
        let f = Fixture::new();
        let native = f.dir.path().join(".local/share/claude/versions/1.0");
        let stable = f.dir.path().join(".local/bin/claude");
        f.executable(
            &native,
            "[ \"$1\" = update ] || exit 9\nprintf '%s\\n' native-update >> \"$TC_SETUP_TEST_LOG\"",
        );
        f.executable(
            &stable,
            "printf '%s\\n' \"stable $*\" >> \"$TC_SETUP_TEST_LOG\"",
        );
        let body = command(ToolId::Claude, ToolSetupAction::Update, Some(&native)).unwrap();
        assert!(f.run(&body).status.success());
        assert!(f.calls().contains("native-update"));
        assert!(f.calls().contains("stable --version"));
    }

    #[test]
    fn installer_and_native_failures_never_run_the_version_check() {
        for tool in [ToolId::Claude, ToolId::Codex, ToolId::Opencode] {
            let f = Fixture::new();
            f.tool(tool);
            f.executable(&f.bin.join("curl"), "exit 9");
            assert!(
                !f.run(&command(tool, ToolSetupAction::Install, None).unwrap())
                    .status
                    .success()
            );
            assert!(f.calls().is_empty());
        }
        let f = Fixture::new();
        let native = f.dir.path().join("opencode");
        f.executable(
            &native,
            "printf '%s\\n' \"$*\" >> \"$TC_SETUP_TEST_LOG\"; exit 7",
        );
        assert!(
            !f.run(&command(ToolId::Opencode, ToolSetupAction::Update, Some(&native)).unwrap())
                .status
                .success()
        );
        assert_eq!(f.calls(), "upgrade\n");
    }

    #[test]
    fn updates_refuse_missing_or_relative_executables() {
        for tool in [ToolId::Claude, ToolId::Codex, ToolId::Opencode] {
            assert_eq!(
                command(tool, ToolSetupAction::Update, None),
                Err(ManagedError::ToolUnavailable)
            );
            assert_eq!(
                command(tool, ToolSetupAction::Update, Some(Path::new("relative"))),
                Err(ManagedError::ToolUnavailable)
            );
        }
    }
}
