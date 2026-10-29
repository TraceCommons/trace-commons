//! The macOS app's "install the command-line tool" action, modelled on VS
//! Code's "Install 'code' command in PATH".
//!
//! The app bundles the CLI at `<App>.app/Contents/MacOS/near-ai`. Installing
//! puts a symlink to it in [`LINK_DIR`], named for the build's channel
//! ([`command_name_for`]), so production and staging can both be installed.
//! A symlink, never a copy: the app updates itself in place, so a link into
//! the bundle keeps naming the current CLI.
//!
//! Everything here is a decision, not an action. The shell asks [`probe`]
//! what is at the link path (reads only, unprivileged), and runs the
//! AppleScript this module built, which asks for an administrator password
//! once. The rules:
//!
//! - The link target is the running app's own bundled CLI: an absolute
//!   `.app` path, no control characters, not App Translocation's temporary
//!   copy, and its `Contents/MacOS/near-ai` a regular executable file.
//! - The link name is one of two fixed names.
//! - Nothing the app did not create is overwritten: a link path holding a
//!   file, a directory, or a symlink anywhere other than a Near.ai bundle's
//!   CLI is [`LinkState::Foreign`], and no script is offered for it.
//! - The privileged script is fixed text plus single-quoted paths. Its only
//!   writes are `mkdir` of [`LINK_DIR`] when missing, `ln -sfn` and `rm` of
//!   the link path. Before writing it re-checks, as root, the state the
//!   unprivileged probe saw (nothing there, or a symlink whose target is
//!   exactly the one read), and exits [`CHANGED_EXIT`] otherwise, so a
//!   file put there in between is never replaced.
//! - The shell quoting is wrapped in an AppleScript string literal escaped
//!   separately, so no path becomes script code at either layer.

use std::path::{Component, Path, PathBuf};

use serde::Serialize;

/// Where the command goes: on the default `PATH` of every macOS shell.
pub const LINK_DIR: &str = "/usr/local/bin";

/// The CLI's place inside the app bundle (`macos/scripts/make-app-bundle.sh`).
pub const BUNDLED_CLI: [&str; 3] = ["Contents", "MacOS", "near-ai"];

/// What the privileged script exits with when the link path no longer holds
/// what the probe saw. The shell re-probes and shows what is there now.
pub const CHANGED_EXIT: u8 = 17;

/// The command's name on a channel: `near-ai` for production,
/// `near-ai-staging` for staging, so the two builds never take each other's
/// command. None for any other channel name.
pub fn command_name_for(channel: &str) -> Option<&'static str> {
    match channel {
        "production" => Some("near-ai"),
        "staging" => Some("near-ai-staging"),
        _ => None,
    }
}

/// This build's command name ([`crate::channel::CHANNEL`]).
pub fn command_name() -> Option<&'static str> {
    command_name_for(crate::channel::CHANNEL)
}

/// What is at the link path, as an unprivileged `lstat` and `readlink` see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Existing {
    /// Nothing, not even a dangling symlink.
    Absent,
    /// A symlink, and its target exactly as `readlink` returns it.
    Symlink(PathBuf),
    /// A file, a directory, or anything else that is not a symlink.
    Other,
}

/// Where the link stands, from the app's side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkState {
    /// Nothing at the link path. Install is offered.
    NotInstalled,
    /// Our symlink, to this app's own CLI. Uninstall is offered.
    Installed,
    /// Our symlink, to a Near.ai bundle's CLI somewhere else (the app was
    /// moved, or another copy installed it). Repointing and uninstall are
    /// offered.
    Stale,
    /// Something this app did not create. Nothing is offered.
    Foreign,
    /// This process is not a bundled app with a usable CLI (a bare
    /// `swift run`, a translocated copy, an unknown channel). The shell
    /// shows nothing.
    Unavailable,
}

/// Why the running app's CLI cannot be the link target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetRefusal {
    /// The bundle path is relative, not UTF-8, or not a `.app`.
    NotABundle,
    /// The path holds a control character: refused rather than escaped.
    ControlCharacter,
    /// Gatekeeper's temporary read-only copy, gone at the next launch.
    Translocated,
    /// `Contents/MacOS/near-ai` is missing, not a regular file, or not
    /// executable.
    NoExecutable,
}

/// The link path for `name` in `dir`.
pub fn link_path_in(dir: &Path, name: &str) -> PathBuf {
    dir.join(name)
}

/// Whether any `/`-separated segment is `.` or `..`. Read from the text:
/// `Path::components` drops an interior `.`.
fn has_dot_segment(text: &str) -> bool {
    text.split('/')
        .any(|segment| segment == "." || segment == "..")
}

fn has_control_character(path: &Path) -> bool {
    path.to_str()
        .is_none_or(|text| text.chars().any(char::is_control))
}

/// The bundled CLI's path inside `bundle`, if `bundle` reads as an app
/// bundle this module may link to. Pure: nothing is read from disk.
pub fn bundled_cli_path(bundle: &Path) -> Result<PathBuf, TargetRefusal> {
    let text = bundle.to_str().ok_or(TargetRefusal::NotABundle)?;
    if has_control_character(bundle) {
        return Err(TargetRefusal::ControlCharacter);
    }
    let text = text.trim_end_matches('/');
    if !bundle.is_absolute() || !text.ends_with(".app") || text.len() <= ".app".len() + 1 {
        return Err(TargetRefusal::NotABundle);
    }
    if has_dot_segment(text) {
        return Err(TargetRefusal::NotABundle);
    }
    if text.contains("/AppTranslocation/") {
        return Err(TargetRefusal::Translocated);
    }
    let mut cli = PathBuf::from(text);
    for part in BUNDLED_CLI {
        cli.push(part);
    }
    Ok(cli)
}

/// The running app's CLI, checked on disk: [`bundled_cli_path`], and the
/// file there is a regular file (not a symlink) with an execute bit.
pub fn own_target(bundle: &Path) -> Result<PathBuf, TargetRefusal> {
    let cli = bundled_cli_path(bundle)?;
    let metadata = std::fs::symlink_metadata(&cli).map_err(|_| TargetRefusal::NoExecutable)?;
    if !metadata.file_type().is_file() || !is_executable(&metadata) {
        return Err(TargetRefusal::NoExecutable);
    }
    Ok(cli)
}

#[cfg(unix)]
fn is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &std::fs::Metadata) -> bool {
    false
}

/// Whether `target` is a Near.ai app bundle's CLI: an absolute path ending
/// `<Near.ai…>.app/Contents/MacOS/near-ai`, with no `..` or control
/// characters. A relative target is never ours: this module only ever
/// writes absolute ones.
pub fn is_ours(target: &Path) -> bool {
    if !target.is_absolute()
        || has_control_character(target)
        || target.to_str().is_none_or(has_dot_segment)
    {
        return false;
    }
    let names: Vec<&str> = target
        .components()
        .filter_map(|part| match part {
            Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .collect();
    let Some(split) = names.len().checked_sub(BUNDLED_CLI.len() + 1) else {
        return false;
    };
    let bundle = names[split];
    names[split + 1..] == BUNDLED_CLI
        && bundle.starts_with(crate::brand::APP_NAME)
        && bundle.ends_with(".app")
}

/// Where the link stands, given what is at its path and this app's CLI.
pub fn classify(existing: &Existing, own_target: &Path) -> LinkState {
    match existing {
        Existing::Absent => LinkState::NotInstalled,
        Existing::Other => LinkState::Foreign,
        Existing::Symlink(target) if target == own_target => LinkState::Installed,
        Existing::Symlink(target) if is_ours(target) => LinkState::Stale,
        Existing::Symlink(_) => LinkState::Foreign,
    }
}

/// A POSIX shell single-quoted word: `'` becomes `'\''`, and nothing inside
/// is expanded.
pub fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// An AppleScript string literal's body: backslash, quote and line breaks
/// escaped, independently of the shell quoting inside it.
pub fn applescript_literal(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

/// The one AppleScript statement the shell runs, around a shell script.
pub fn privileged_applescript(shell_script: &str) -> String {
    format!(
        "do shell script \"{}\" with administrator privileges",
        applescript_literal(shell_script)
    )
}

/// The re-check of what the probe saw, as root, before anything is
/// written: nothing at the link path, or a symlink whose target is exactly
/// the one read. Anything else exits [`CHANGED_EXIT`].
fn precondition(link: &str, existing: &Existing) -> Option<String> {
    let changed = CHANGED_EXIT;
    match existing {
        Existing::Absent => Some(format!(
            "if [ -e {link} ] || [ -L {link} ]; then exit {changed}; fi"
        )),
        Existing::Symlink(target) => {
            let target = target.to_str().filter(|text| !text.is_empty())?;
            if target.chars().any(char::is_control) {
                return None;
            }
            let target = shell_quote(target);
            Some(format!(
                "if [ ! -L {link} ] || [ \"$(/usr/bin/readlink {link})\" != {target} ]; \
                 then exit {changed}; fi"
            ))
        }
        Existing::Other => None,
    }
}

fn quoted_path(path: &Path) -> Option<String> {
    let text = path.to_str()?;
    if !path.is_absolute() || text.chars().any(char::is_control) {
        return None;
    }
    Some(shell_quote(text))
}

/// The shell script that installs or repoints the link, or None when the
/// state allows neither (installed already, foreign, unavailable) or a path
/// cannot be used. Creates the link's directory (mode 755) when missing.
pub fn install_script(link: &Path, target: &Path, existing: &Existing) -> Option<String> {
    if !matches!(
        classify(existing, target),
        LinkState::NotInstalled | LinkState::Stale
    ) {
        return None;
    }
    let dir = quoted_path(link.parent()?)?;
    let quoted_link = quoted_path(link)?;
    let quoted_target = quoted_path(target)?;
    let check = precondition(&quoted_link, existing)?;
    Some(format!(
        "/bin/mkdir -p -m 755 {dir} || exit {changed}; {check}; \
         /bin/ln -sfn {quoted_target} {quoted_link}",
        changed = CHANGED_EXIT,
    ))
}

/// The shell script that removes our link, or None when there is no link
/// of ours to remove. Removes only the symlink itself.
pub fn uninstall_script(link: &Path, target: &Path, existing: &Existing) -> Option<String> {
    if !matches!(
        classify(existing, target),
        LinkState::Installed | LinkState::Stale
    ) {
        return None;
    }
    let quoted_link = quoted_path(link)?;
    let check = precondition(&quoted_link, existing)?;
    Some(format!("{check}; /bin/rm {quoted_link}"))
}

/// What the link path holds, read without following the link.
pub fn read_existing(link: &Path) -> Existing {
    match std::fs::symlink_metadata(link) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Existing::Absent,
        // Unreadable is not nothing: never offer to write over it.
        Err(_) => Existing::Other,
        Ok(metadata) if metadata.file_type().is_symlink() => match std::fs::read_link(link) {
            Ok(target) => Existing::Symlink(target),
            Err(_) => Existing::Other,
        },
        Ok(_) => Existing::Other,
    }
}

/// Everything a shell needs to draw the row and act on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CliToolStatus {
    /// `near-ai` or `near-ai-staging`.
    pub command_name: &'static str,
    /// The full link path, which the row shows.
    pub link_path: String,
    pub state: LinkState,
    /// The AppleScript that installs or repoints the link, when offered.
    pub install_script: Option<String>,
    /// The AppleScript that removes our link, when offered.
    pub uninstall_script: Option<String>,
}

/// Probe `link_dir` for this build's command, for the app at `bundle`.
/// Reads only.
pub fn probe(bundle: &Path, link_dir: &Path) -> CliToolStatus {
    probe_named(command_name(), bundle, link_dir)
}

fn probe_named(name: Option<&'static str>, bundle: &Path, link_dir: &Path) -> CliToolStatus {
    let Some(name) = name else {
        return CliToolStatus {
            command_name: "",
            link_path: String::new(),
            state: LinkState::Unavailable,
            install_script: None,
            uninstall_script: None,
        };
    };
    let link = link_path_in(link_dir, name);
    let link_path = link.to_string_lossy().into_owned();
    let Ok(target) = own_target(bundle) else {
        return CliToolStatus {
            command_name: name,
            link_path,
            state: LinkState::Unavailable,
            install_script: None,
            uninstall_script: None,
        };
    };
    let existing = read_existing(&link);
    CliToolStatus {
        command_name: name,
        link_path,
        state: classify(&existing, &target),
        install_script: install_script(&link, &target, &existing)
            .map(|script| privileged_applescript(&script)),
        uninstall_script: uninstall_script(&link, &target, &existing)
            .map(|script| privileged_applescript(&script)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUNDLE: &str = "/Applications/Near.ai Staging.app";
    const TARGET: &str = "/Applications/Near.ai Staging.app/Contents/MacOS/near-ai";
    const LINK: &str = "/usr/local/bin/near-ai-staging";

    fn p(text: &str) -> PathBuf {
        PathBuf::from(text)
    }

    #[test]
    fn each_channel_has_its_own_command_name() {
        assert_eq!(command_name_for("production"), Some("near-ai"));
        assert_eq!(command_name_for("staging"), Some("near-ai-staging"));
        assert_eq!(command_name_for("nightly"), None);
        assert_eq!(command_name_for(""), None);
        assert_eq!(
            command_name(),
            command_name_for(crate::channel::CHANNEL),
            "this build's name follows its channel"
        );
        assert!(command_name().is_some());
    }

    #[test]
    fn the_bundled_cli_is_inside_the_bundle() {
        assert_eq!(bundled_cli_path(&p(BUNDLE)), Ok(p(TARGET)));
        assert_eq!(
            bundled_cli_path(&p("/Applications/Near.ai.app/")),
            Ok(p("/Applications/Near.ai.app/Contents/MacOS/near-ai"))
        );
    }

    #[test]
    fn a_bundle_path_that_is_not_an_app_is_refused() {
        for bundle in [
            "Near.ai.app",
            "/Applications/Near.ai",
            "/usr/local/bin",
            "/.app",
            "/Applications/../tmp/Near.ai.app",
            "/Applications/./Near.ai.app",
        ] {
            assert_eq!(
                bundled_cli_path(&p(bundle)),
                Err(TargetRefusal::NotABundle),
                "{bundle}"
            );
        }
        assert_eq!(
            bundled_cli_path(&p("/Applications/Near\nai.app")),
            Err(TargetRefusal::ControlCharacter)
        );
        assert_eq!(
            bundled_cli_path(&p(
                "/private/var/folders/x/AppTranslocation/ABC/d/Near.ai.app"
            )),
            Err(TargetRefusal::Translocated)
        );
    }

    #[test]
    fn only_a_near_ai_bundles_cli_is_ours() {
        assert!(is_ours(&p(TARGET)));
        assert!(is_ours(&p(
            "/Users/me/Downloads/Near.ai.app/Contents/MacOS/near-ai"
        )));
        for target in [
            "/opt/homebrew/bin/near-ai",
            "/Applications/Other.app/Contents/MacOS/near-ai",
            "/Applications/Near.ai.app/Contents/MacOS/TraceCommonsApp",
            "/Applications/Near.ai/Contents/MacOS/near-ai",
            "Near.ai.app/Contents/MacOS/near-ai",
            "../Near.ai.app/Contents/MacOS/near-ai",
            "/Applications/../Near.ai.app/Contents/MacOS/near-ai",
            "/Applications/./Near.ai.app/Contents/MacOS/near-ai",
            "/Contents/MacOS/near-ai",
        ] {
            assert!(!is_ours(&p(target)), "{target}");
        }
    }

    #[test]
    fn the_link_path_is_classified_by_what_is_there() {
        let target = p(TARGET);
        assert_eq!(
            classify(&Existing::Absent, &target),
            LinkState::NotInstalled
        );
        assert_eq!(
            classify(&Existing::Symlink(target.clone()), &target),
            LinkState::Installed
        );
        assert_eq!(
            classify(
                &Existing::Symlink(p("/Users/me/Near.ai Staging.app/Contents/MacOS/near-ai")),
                &target
            ),
            LinkState::Stale
        );
        assert_eq!(
            classify(&Existing::Symlink(p("/opt/homebrew/bin/near-ai")), &target),
            LinkState::Foreign
        );
        assert_eq!(classify(&Existing::Other, &target), LinkState::Foreign);
    }

    #[test]
    fn shell_quoting_keeps_spaces_and_quotes_inside_one_word() {
        assert_eq!(shell_quote("/a b"), "'/a b'");
        assert_eq!(shell_quote("/it's"), r"'/it'\''s'");
        assert_eq!(shell_quote("$(rm -rf /)"), "'$(rm -rf /)'");
        assert_eq!(shell_quote("a\"b\\c`d"), "'a\"b\\c`d'");
    }

    #[test]
    fn the_applescript_literal_escapes_quotes_backslashes_and_line_breaks() {
        assert_eq!(applescript_literal(r#"a"b\c"#), r#"a\"b\\c"#);
        assert_eq!(applescript_literal("a\nb\rc"), r"a\nb\rc");
        let script = privileged_applescript(r#"echo "x" \"#);
        assert_eq!(
            script,
            r#"do shell script "echo \"x\" \\" with administrator privileges"#
        );
    }

    /// The exact text a fresh install runs as root.
    #[test]
    fn a_fresh_install_is_mkdir_a_recheck_and_ln() {
        let script = install_script(&p(LINK), &p(TARGET), &Existing::Absent).unwrap();
        assert_eq!(
            script,
            "/bin/mkdir -p -m 755 '/usr/local/bin' || exit 17; \
             if [ -e '/usr/local/bin/near-ai-staging' ] || [ -L '/usr/local/bin/near-ai-staging' ]; \
             then exit 17; fi; \
             /bin/ln -sfn '/Applications/Near.ai Staging.app/Contents/MacOS/near-ai' \
             '/usr/local/bin/near-ai-staging'"
        );
        assert_eq!(
            privileged_applescript(&script),
            "do shell script \"/bin/mkdir -p -m 755 '/usr/local/bin' || exit 17; \
             if [ -e '/usr/local/bin/near-ai-staging' ] || [ -L '/usr/local/bin/near-ai-staging' ]; \
             then exit 17; fi; \
             /bin/ln -sfn '/Applications/Near.ai Staging.app/Contents/MacOS/near-ai' \
             '/usr/local/bin/near-ai-staging'\" with administrator privileges"
        );
    }

    /// Repointing and removing both require, as root, the exact symlink the
    /// probe read.
    #[test]
    fn repoint_and_remove_recheck_the_symlink_they_read() {
        let old = p("/Users/me/Near.ai Staging.app/Contents/MacOS/near-ai");
        let existing = Existing::Symlink(old);
        let check = "if [ ! -L '/usr/local/bin/near-ai-staging' ] || \
             [ \"$(/usr/bin/readlink '/usr/local/bin/near-ai-staging')\" != \
             '/Users/me/Near.ai Staging.app/Contents/MacOS/near-ai' ]; then exit 17; fi";
        assert_eq!(
            install_script(&p(LINK), &p(TARGET), &existing).unwrap(),
            format!(
                "/bin/mkdir -p -m 755 '/usr/local/bin' || exit 17; {check}; \
                 /bin/ln -sfn '{TARGET}' '{LINK}'"
            )
        );
        assert_eq!(
            uninstall_script(&p(LINK), &p(TARGET), &existing).unwrap(),
            format!("{check}; /bin/rm '{LINK}'")
        );
    }

    #[test]
    fn nothing_is_offered_over_a_foreign_file_or_link() {
        for existing in [
            Existing::Other,
            Existing::Symlink(p("/opt/homebrew/bin/near-ai")),
        ] {
            assert_eq!(install_script(&p(LINK), &p(TARGET), &existing), None);
            assert_eq!(uninstall_script(&p(LINK), &p(TARGET), &existing), None);
        }
        // Installed already: only uninstall.
        let ours = Existing::Symlink(p(TARGET));
        assert_eq!(install_script(&p(LINK), &p(TARGET), &ours), None);
        assert!(uninstall_script(&p(LINK), &p(TARGET), &ours).is_some());
        // Not installed: only install.
        assert_eq!(
            uninstall_script(&p(LINK), &p(TARGET), &Existing::Absent),
            None
        );
    }

    /// A quote in a path stays inside its word at both layers.
    #[test]
    fn a_quote_in_the_bundle_path_never_ends_a_word() {
        let target = p("/Users/o'neil/My \"Apps\"/Near.ai.app/Contents/MacOS/near-ai");
        let script = install_script(&p(LINK), &target, &Existing::Absent).unwrap();
        assert!(script.ends_with(
            r#"/bin/ln -sfn '/Users/o'\''neil/My "Apps"/Near.ai.app/Contents/MacOS/near-ai' '/usr/local/bin/near-ai-staging'"#
        ));
        let applescript = privileged_applescript(&script);
        let body = applescript
            .strip_prefix("do shell script \"")
            .and_then(|rest| rest.strip_suffix("\" with administrator privileges"))
            .unwrap();
        // Every quote inside the literal is escaped, so none ends it.
        let mut chars = body.chars();
        while let Some(c) = chars.next() {
            match c {
                '\\' => assert!(matches!(chars.next(), Some('\\' | '"' | 'n' | 'r'))),
                '"' => panic!("an unescaped quote ends the AppleScript literal: {body}"),
                _ => {}
            }
        }
    }

    /// The scripts write with three commands only, by absolute path.
    #[test]
    fn the_scripts_run_only_fixed_commands() {
        let old = Existing::Symlink(p("/Users/me/Near.ai.app/Contents/MacOS/near-ai"));
        let scripts = [
            install_script(&p(LINK), &p(TARGET), &Existing::Absent).unwrap(),
            install_script(&p(LINK), &p(TARGET), &old).unwrap(),
            uninstall_script(&p(LINK), &p(TARGET), &old).unwrap(),
        ];
        for script in scripts {
            // Drop every quoted word, then what is left is the template.
            let mut template = String::new();
            let mut quoted = false;
            for c in script.chars() {
                if c == '\'' {
                    quoted = !quoted;
                } else if !quoted {
                    template.push(c);
                }
            }
            for word in template.split(|c: char| c.is_whitespace() || c == ';') {
                let word = word.trim_matches(|c| c == '"' || c == '$' || c == '(' || c == ')');
                assert!(
                    [
                        "",
                        "/bin/mkdir",
                        "-p",
                        "-m",
                        "755",
                        "||",
                        "exit",
                        "17",
                        "if",
                        "[",
                        "]",
                        "-e",
                        "-L",
                        "!",
                        "then",
                        "fi",
                        "/bin/ln",
                        "-sfn",
                        "/bin/rm",
                        "/usr/bin/readlink",
                        "!=",
                    ]
                    .contains(&word),
                    "unexpected word {word:?} in {script}"
                );
            }
        }
    }

    #[test]
    fn a_path_with_a_control_character_gets_no_script() {
        let bad = p("/Applications/Near.ai\n.app/Contents/MacOS/near-ai");
        assert_eq!(install_script(&p(LINK), &bad, &Existing::Absent), None);
        let bad_old = Existing::Symlink(p("/x/Near.ai.app/Contents/MacOS/near-ai\n"));
        assert_eq!(uninstall_script(&p(LINK), &p(TARGET), &bad_old), None);
    }

    #[cfg(unix)]
    mod on_disk {
        use super::super::*;
        use std::os::unix::fs::{PermissionsExt, symlink};

        fn fake_bundle(root: &Path, name: &str) -> PathBuf {
            let bundle = root.join(name);
            let macos = bundle.join("Contents/MacOS");
            std::fs::create_dir_all(&macos).unwrap();
            let cli = macos.join("near-ai");
            std::fs::write(&cli, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
            bundle
        }

        fn status(bundle: &Path, dir: &Path) -> CliToolStatus {
            probe_named(Some("near-ai-staging"), bundle, dir)
        }

        /// Run the shell script, unprivileged, against a temporary link
        /// directory. Never the AppleScript, never as root.
        fn run(script: &str) -> Option<i32> {
            std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(script)
                .status()
                .unwrap()
                .code()
        }

        #[test]
        fn the_probe_reads_each_state() {
            let root = tempfile::tempdir().unwrap();
            let bundle = fake_bundle(root.path(), "Near.ai Staging.app");
            let dir = root.path().join("bin");
            std::fs::create_dir(&dir).unwrap();
            let link = dir.join("near-ai-staging");
            let target = bundle.join("Contents/MacOS/near-ai");

            let empty = status(&bundle, &dir);
            assert_eq!(empty.state, LinkState::NotInstalled);
            assert_eq!(empty.link_path, link.to_string_lossy());
            assert!(empty.install_script.is_some() && empty.uninstall_script.is_none());

            symlink(&target, &link).unwrap();
            let installed = status(&bundle, &dir);
            assert_eq!(installed.state, LinkState::Installed);
            assert!(installed.install_script.is_none() && installed.uninstall_script.is_some());

            std::fs::remove_file(&link).unwrap();
            symlink(
                "/Old Place/Near.ai Staging.app/Contents/MacOS/near-ai",
                &link,
            )
            .unwrap();
            assert_eq!(status(&bundle, &dir).state, LinkState::Stale);

            std::fs::remove_file(&link).unwrap();
            std::fs::write(&link, "someone else's").unwrap();
            let foreign = status(&bundle, &dir);
            assert_eq!(foreign.state, LinkState::Foreign);
            assert!(foreign.install_script.is_none() && foreign.uninstall_script.is_none());
        }

        #[test]
        fn a_bundle_without_an_executable_cli_is_unavailable() {
            let root = tempfile::tempdir().unwrap();
            let bundle = fake_bundle(root.path(), "Near.ai.app");
            let cli = bundle.join("Contents/MacOS/near-ai");
            std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert_eq!(status(&bundle, root.path()).state, LinkState::Unavailable);
            std::fs::remove_file(&cli).unwrap();
            symlink("/bin/sh", &cli).unwrap();
            assert_eq!(own_target(&bundle), Err(TargetRefusal::NoExecutable));
            assert_eq!(
                status(&root.path().join("nope.app"), root.path()).state,
                LinkState::Unavailable
            );
            assert_eq!(
                probe_named(None, &bundle, root.path()).state,
                LinkState::Unavailable
            );
        }

        /// The scripts do what they say, quoting a path with a space and a
        /// quote, and create a missing link directory with mode 755.
        #[test]
        fn install_repoint_and_uninstall_work_on_a_temporary_directory() {
            let root = tempfile::tempdir().unwrap();
            let bundle = fake_bundle(root.path(), "Near.ai it's Staging.app");
            let target = bundle.join("Contents/MacOS/near-ai");
            let dir = root.path().join("new bin");
            let link = dir.join("near-ai-staging");

            let script = install_script(&link, &target, &read_existing(&link)).unwrap();
            assert_eq!(run(&script), Some(0));
            assert_eq!(std::fs::read_link(&link).unwrap(), target);
            let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o755);

            // The app moved: repoint the stale link.
            let moved = fake_bundle(&root.path().join("moved"), "Near.ai Staging.app");
            let moved_target = moved.join("Contents/MacOS/near-ai");
            let existing = read_existing(&link);
            assert_eq!(classify(&existing, &moved_target), LinkState::Stale);
            let script = install_script(&link, &moved_target, &existing).unwrap();
            assert_eq!(run(&script), Some(0));
            assert_eq!(std::fs::read_link(&link).unwrap(), moved_target);

            let script = uninstall_script(&link, &moved_target, &read_existing(&link)).unwrap();
            assert_eq!(run(&script), Some(0));
            assert_eq!(read_existing(&link), Existing::Absent);
            assert!(
                target.exists() && moved_target.exists(),
                "only the link goes"
            );
        }

        /// A file that appears between the probe and the script is never
        /// replaced: the script's own re-check refuses.
        #[test]
        fn the_script_refuses_what_appeared_after_the_probe() {
            let root = tempfile::tempdir().unwrap();
            let bundle = fake_bundle(root.path(), "Near.ai.app");
            let target = bundle.join("Contents/MacOS/near-ai");
            let link = root.path().join("near-ai");

            let install = install_script(&link, &target, &Existing::Absent).unwrap();
            std::fs::write(&link, "not ours").unwrap();
            assert_eq!(run(&install), Some(i32::from(CHANGED_EXIT)));
            assert_eq!(std::fs::read_to_string(&link).unwrap(), "not ours");

            // A symlink swapped for another target is refused too.
            std::fs::remove_file(&link).unwrap();
            symlink(&target, &link).unwrap();
            let uninstall = uninstall_script(&link, &target, &read_existing(&link)).unwrap();
            std::fs::remove_file(&link).unwrap();
            symlink("/bin/sh", &link).unwrap();
            assert_eq!(run(&uninstall), Some(i32::from(CHANGED_EXIT)));
            assert_eq!(std::fs::read_link(&link).unwrap(), Path::new("/bin/sh"));

            // And a directory where the link should be.
            std::fs::remove_file(&link).unwrap();
            std::fs::create_dir(&link).unwrap();
            assert_eq!(run(&install), Some(i32::from(CHANGED_EXIT)));
            assert!(std::fs::read_dir(&link).unwrap().next().is_none());
        }
    }
}
