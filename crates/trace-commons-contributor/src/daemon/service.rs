//! The background service: whether the host contributor daemon -- the
//! systemd user unit `daemon install` writes -- is there for a shell to use.
//!
//! Every shell asks the same question and gets the same answer here, from
//! facts it gathers itself: the host link's probe (`host_link::probe`),
//! whether the window runs a daemon in its own process, whether the unit's
//! file is installed (when the shell can tell), and the answering daemon's
//! `hello`. The Linux GTK shell draws the answer as one of three dialogs
//! (owner-approved flow, 2026-11-17); the words are
//! `managed::copy::background_service`, the commands are the fixed strings in
//! `channel` ([`SERVICE_INSTALL_COMMAND`], [`SERVICE_STATUS_COMMAND`],
//! [`SERVICE_INSTALL_GUIDE_URL`]).
//!
//! Pure: nothing here touches the disk, a socket or the bus.

use super::host_link::LinkState;

pub use crate::channel::{
    SERVICE_INSTALL_COMMAND, SERVICE_INSTALL_GUIDE_URL, SERVICE_STATUS_COMMAND,
};

/// The command that declares a folder to the host daemon from a terminal
/// opened in that folder, for a tool whose `set_settings` key is
/// `settings_key`: the copy-only fallback when a sandboxed window cannot
/// learn the host path of a folder the chooser handed it (a document portal
/// without `GetHostPaths`). Fixed text: the folder is the terminal's `$PWD`,
/// expanded by the person's shell, never a path from this process, which
/// only holds the portal's. `None` for a tool `daemon settings --set` cannot
/// declare: only Claude Code and Codex take a bare path
/// (`claude_root`, `codex_root`); the others need a declaration object.
pub fn host_folder_command(settings_key: &str) -> Option<String> {
    let path_key = match settings_key {
        "claude_source" => "claude_root",
        "codex_source" => "codex_root",
        _ => return None,
    };
    Some(format!(
        "{} daemon settings --set \"{path_key}=$PWD\"",
        crate::channel::HOST_CLI
    ))
}

/// The `hello` key that carries [`SERVICE_LEVEL`].
pub const HELLO_SERVICE_LEVEL_KEY: &str = "service_level";

/// What this daemon offers a shell that attaches to it as its background
/// service, as one number in `hello`. Raised when a shell comes to need
/// something an older daemon lacks; a shell compares it with
/// [`MIN_SERVICE_LEVEL`]. A daemon from before this field omits it, and an
/// omitted level is too old: the host link, which a Flatpak window needs,
/// arrived together with it.
pub const SERVICE_LEVEL: u64 = 1;

/// The lowest [`SERVICE_LEVEL`] this build's shells work with.
pub const MIN_SERVICE_LEVEL: u64 = 1;

// This daemon always meets its own shells.
const _: () = assert!(SERVICE_LEVEL >= MIN_SERVICE_LEVEL);

/// How long a unit must have been `active`, with no host link and no marker,
/// before a shell reads it as a daemon from before the host link. A current
/// daemon started by the unit takes `watcher.lock` and writes the marker
/// within its first moments, or exits on the lock; it is never active and
/// markerless this long.
pub const TOO_OLD_GRACE: std::time::Duration = std::time::Duration::from_secs(15);

/// The background service, as every shell names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceState {
    /// A daemon outside this window answers, at a level this shell needs.
    Running,
    /// Nothing says the service was ever installed.
    NotInstalled,
    /// The unit is installed (its file, or the marker its daemon left this
    /// login), and nothing usable answers: stopped, crashed, restarting, or
    /// held off by the daemon this window runs.
    InstalledNotRunning,
    /// A daemon answers, below [`MIN_SERVICE_LEVEL`].
    TooOld,
}

/// What the daemon this window would use answered to `hello`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonAnswer {
    /// It answered at [`MIN_SERVICE_LEVEL`] or above.
    Meets,
    /// It answered below it, or without a level.
    TooOld,
    /// Nothing outside this window answered: no daemon was asked, or the
    /// one asked did not answer.
    NoAnswer,
}

/// Read a `hello` result. `None` is no answer.
pub fn answer_from_hello(hello: Option<&serde_json::Value>) -> DaemonAnswer {
    let Some(hello) = hello else {
        return DaemonAnswer::NoAnswer;
    };
    match hello[HELLO_SERVICE_LEVEL_KEY].as_u64() {
        Some(level) if level >= MIN_SERVICE_LEVEL => DaemonAnswer::Meets,
        _ => DaemonAnswer::TooOld,
    }
}

/// What a shell gathered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceFacts {
    /// The host link's probe; `Absent` where there is no runtime directory.
    pub link: LinkState,
    /// This window runs the daemon in its own process. Its daemon is never
    /// the service, and while it runs the unit cannot (the watcher lock, or
    /// on the host the state directory's lock), so a host link it serves
    /// itself says nothing.
    pub hosting: bool,
    /// Whether the unit's file is installed, when the shell can tell (the
    /// file on the host, systemd's `GetUnitFileState` from a sandbox).
    /// `None` when it cannot.
    pub unit_installed: Option<bool>,
    /// Whether systemd reports the unit `active` (its `ActiveState`), when
    /// the shell can ask. A unit held off by this window's lock is
    /// restarting (`activating`), not active.
    pub unit_active: Option<bool>,
    /// How long the unit has been `active` (its `ActiveEnterTimestamp`
    /// against now), when the shell can ask. `None` when it cannot, the
    /// unit is not active, or the timestamp is unset or in the future.
    pub unit_active_for: Option<std::time::Duration>,
    /// `hello` from a daemon outside this window: the one it is attached
    /// to, or one listening on the host link.
    pub answer: DaemonAnswer,
}

/// The one answer every shell draws.
pub fn classify(facts: &ServiceFacts) -> ServiceState {
    match facts.answer {
        DaemonAnswer::Meets => ServiceState::Running,
        DaemonAnswer::TooOld => ServiceState::TooOld,
        // The unit has run past the grace period, with no host link and no
        // marker: a daemon from before the host link (and `service_level`),
        // unreachable from a sandbox. Positive evidence only: a current unit
        // daemon writes the marker when it takes `watcher.lock` (so it reads
        // `Down` even if it could not bind the link), and one that loses the
        // lock exits within the grace period. Running the install command
        // again replaces an old one.
        DaemonAnswer::NoAnswer
            if facts.unit_active == Some(true)
                && facts.link == LinkState::Absent
                && facts.unit_active_for.is_some_and(|d| d >= TOO_OLD_GRACE) =>
        {
            ServiceState::TooOld
        }
        DaemonAnswer::NoAnswer => {
            let installed = facts.unit_installed == Some(true)
                || facts.link == LinkState::Down
                || (facts.link == LinkState::Listening && !facts.hosting);
            if installed {
                ServiceState::InstalledNotRunning
            } else {
                ServiceState::NotInstalled
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn facts(
        link: LinkState,
        hosting: bool,
        unit: Option<bool>,
        answer: DaemonAnswer,
    ) -> ServiceFacts {
        ServiceFacts {
            link,
            hosting,
            unit_installed: unit,
            unit_active: None,
            unit_active_for: None,
            answer,
        }
    }

    fn active_for(seen: &mut ServiceFacts, secs: Option<u64>) {
        seen.unit_active = Some(true);
        seen.unit_active_for = secs.map(std::time::Duration::from_secs);
    }

    #[test]
    fn an_active_unit_nobody_reaches_on_the_host_link_is_too_old() {
        // From a sandbox: an older host daemon has run past the grace period,
        // wrote no marker and serves no host link.
        for hosting in [false, true] {
            let mut seen = facts(
                LinkState::Absent,
                hosting,
                Some(true),
                DaemonAnswer::NoAnswer,
            );
            active_for(&mut seen, Some(TOO_OLD_GRACE.as_secs() + 1));
            assert_eq!(classify(&seen), ServiceState::TooOld);
        }
        // Not active (held off by this window's lock, restarting): not too
        // old, just not running.
        let mut seen = facts(LinkState::Absent, true, Some(true), DaemonAnswer::NoAnswer);
        seen.unit_active = Some(false);
        assert_eq!(classify(&seen), ServiceState::InstalledNotRunning);
        // Active and listening but not answering yet: starting.
        let mut seen = facts(
            LinkState::Listening,
            false,
            Some(true),
            DaemonAnswer::NoAnswer,
        );
        seen.unit_active = Some(true);
        assert_eq!(classify(&seen), ServiceState::InstalledNotRunning);
    }

    /// Too old needs positive evidence of an old daemon: active past the
    /// grace period with no marker. A current unit daemon that lost
    /// `watcher.lock` right after the install command is active only until
    /// it exits; one that claimed the lock wrote the marker (`Down`) even if
    /// it could not bind the link.
    #[test]
    fn an_active_unit_is_too_old_only_after_the_grace_with_no_marker() {
        let grace = TOO_OLD_GRACE.as_secs();
        for hosting in [false, true] {
            // Just started (Type=simple reads active at once), or the
            // duration is unknown: not too old.
            for secs in [Some(0), Some(grace - 1), None] {
                let mut seen = facts(
                    LinkState::Absent,
                    hosting,
                    Some(true),
                    DaemonAnswer::NoAnswer,
                );
                active_for(&mut seen, secs);
                assert_eq!(
                    classify(&seen),
                    ServiceState::InstalledNotRunning,
                    "{secs:?}"
                );
            }
            // A current daemon that claimed the lock left the marker: down,
            // however long it has been active.
            let mut seen = facts(LinkState::Down, hosting, Some(true), DaemonAnswer::NoAnswer);
            active_for(&mut seen, Some(grace * 100));
            assert_eq!(classify(&seen), ServiceState::InstalledNotRunning);
        }
        // Exactly at the grace: too old.
        let mut seen = facts(LinkState::Absent, false, Some(true), DaemonAnswer::NoAnswer);
        active_for(&mut seen, Some(grace));
        assert_eq!(classify(&seen), ServiceState::TooOld);
    }

    #[test]
    fn an_answer_at_the_minimum_is_running_whatever_else_is_seen() {
        for link in [LinkState::Listening, LinkState::Down, LinkState::Absent] {
            for unit in [None, Some(false), Some(true)] {
                assert_eq!(
                    classify(&facts(link, false, unit, DaemonAnswer::Meets)),
                    ServiceState::Running
                );
            }
        }
    }

    #[test]
    fn an_answer_below_the_minimum_is_too_old() {
        assert_eq!(
            classify(&facts(
                LinkState::Listening,
                false,
                Some(true),
                DaemonAnswer::TooOld
            )),
            ServiceState::TooOld
        );
        // An older daemon on the host serves no host link: the window on the
        // host reaches it on the state directory's socket.
        assert_eq!(
            classify(&facts(LinkState::Absent, false, None, DaemonAnswer::TooOld)),
            ServiceState::TooOld
        );
    }

    #[test]
    fn nothing_seen_is_not_installed() {
        for hosting in [false, true] {
            for unit in [None, Some(false)] {
                assert_eq!(
                    classify(&facts(
                        LinkState::Absent,
                        hosting,
                        unit,
                        DaemonAnswer::NoAnswer
                    )),
                    ServiceState::NotInstalled
                );
            }
        }
    }

    #[test]
    fn a_unit_marker_or_file_without_an_answer_is_installed_not_running() {
        // The unit's marker with nobody listening: stopped or restarting.
        assert_eq!(
            classify(&facts(LinkState::Down, false, None, DaemonAnswer::NoAnswer)),
            ServiceState::InstalledNotRunning
        );
        // The unit's file, held off by the daemon this window runs.
        assert_eq!(
            classify(&facts(
                LinkState::Absent,
                true,
                Some(true),
                DaemonAnswer::NoAnswer
            )),
            ServiceState::InstalledNotRunning
        );
        // Something listens on the host link and does not answer.
        assert_eq!(
            classify(&facts(
                LinkState::Listening,
                false,
                None,
                DaemonAnswer::NoAnswer
            )),
            ServiceState::InstalledNotRunning
        );
    }

    #[test]
    fn the_host_link_a_hosting_window_serves_itself_says_nothing() {
        // On the host, a window running its own daemon serves the host link.
        assert_eq!(
            classify(&facts(
                LinkState::Listening,
                true,
                Some(false),
                DaemonAnswer::NoAnswer
            )),
            ServiceState::NotInstalled
        );
    }

    #[test]
    fn the_level_compares_against_the_minimum() {
        assert_eq!(answer_from_hello(None), DaemonAnswer::NoAnswer);
        // A daemon from before the field.
        assert_eq!(
            answer_from_hello(Some(&json!({"schema_version": "trace_commons.daemon.v1"}))),
            DaemonAnswer::TooOld
        );
        assert_eq!(
            answer_from_hello(Some(&json!({"service_level": MIN_SERVICE_LEVEL}))),
            DaemonAnswer::Meets
        );
        assert_eq!(
            answer_from_hello(Some(&json!({"service_level": MIN_SERVICE_LEVEL + 1}))),
            DaemonAnswer::Meets
        );
        assert_eq!(
            answer_from_hello(Some(&json!({"service_level": MIN_SERVICE_LEVEL - 1}))),
            DaemonAnswer::TooOld
        );
        // Not a number: a level is never guessed.
        for odd in [json!("1"), json!(-1), json!(1.5), json!(null)] {
            assert_eq!(
                answer_from_hello(Some(&json!({"service_level": odd}))),
                DaemonAnswer::TooOld,
                "{odd}"
            );
        }
    }

    /// Fixed strings: the dialogs show exactly these. Production values.
    #[cfg(not(feature = "near-ai-staging"))]
    #[test]
    fn the_commands_are_fixed() {
        assert_eq!(
            SERVICE_INSTALL_COMMAND,
            "curl -fsSL https://raw.githubusercontent.com/TraceCommons/trace-commons/main/scripts/install.sh | sh \
             && \"$HOME/.local/bin/near-ai\" daemon install \
             && systemctl --user daemon-reload \
             && systemctl --user enable trace-commons-contributor.service \
             && systemctl --user restart trace-commons-contributor.service"
        );
        assert_eq!(
            SERVICE_STATUS_COMMAND,
            "systemctl --user status trace-commons-contributor.service"
        );
        assert_eq!(
            SERVICE_INSTALL_GUIDE_URL,
            "https://docs.tracecommons.ai/cli/quickstart/"
        );
    }

    /// The folder command: this channel's CLI, the tool's path key, the
    /// terminal's own folder, and nothing for a tool that needs an object.
    #[test]
    fn the_host_folder_command_declares_the_terminals_folder() {
        let cli = crate::channel::HOST_CLI;
        assert_eq!(
            host_folder_command("claude_source").as_deref(),
            Some(format!("{cli} daemon settings --set \"claude_root=$PWD\"").as_str())
        );
        assert_eq!(
            host_folder_command("codex_source").as_deref(),
            Some(format!("{cli} daemon settings --set \"codex_root=$PWD\"").as_str())
        );
        for key in [
            "gemini_source",
            "cline_source",
            "opencode_source",
            "trajectory_source",
            "claude_root",
            "",
        ] {
            assert_eq!(host_folder_command(key), None, "{key}");
        }
        #[cfg(not(feature = "near-ai-staging"))]
        assert_eq!(cli, "\"$HOME/.local/bin/near-ai\"");
        #[cfg(feature = "near-ai-staging")]
        assert_eq!(cli, "\"$HOME/.local/bin/near-ai-staging\"");
        assert!(SERVICE_INSTALL_COMMAND.contains(&format!("&& {cli} daemon install &&")));
    }

    /// Whatever the channel: the commands name this build's unit, and the
    /// install command ends by (re)starting it, so running it again over an
    /// older daemon replaces it.
    #[test]
    fn the_commands_name_this_channels_unit() {
        let unit = crate::channel::SYSTEMD_UNIT_NAME;
        assert_eq!(
            SERVICE_STATUS_COMMAND,
            format!("systemctl --user status {unit}")
        );
        assert!(SERVICE_INSTALL_COMMAND.contains(" daemon install && "));
        assert!(SERVICE_INSTALL_COMMAND.contains(&format!("systemctl --user enable {unit} && ")));
        assert!(SERVICE_INSTALL_COMMAND.ends_with(&format!("systemctl --user restart {unit}")));
        assert!(SERVICE_INSTALL_GUIDE_URL.starts_with("https://"));
        for command in [SERVICE_INSTALL_COMMAND, SERVICE_STATUS_COMMAND] {
            assert!(
                !command.contains('{') && !command.contains('\n'),
                "{command}"
            );
        }
    }
}
