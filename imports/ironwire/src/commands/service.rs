//! `ironwire service` — run the daemon in the background.
//!
//! Three platforms, three mechanisms, one rule: **always a user agent, never a
//! system service.** The daemon reads the OAuth tokens Claude Code and Codex
//! stored in this user's home directory and binds loopback only. A root-run
//! system service reaching into a user's credential files is a category error,
//! and there is deliberately no flag to ask for one.
//!
//! Nothing here is required. `ironwire serve` in the foreground stays
//! first-class, and is the answer wherever there is no user-scoped supervisor.
//!
//! Installing *starts*. A command that writes a unit file and then hands back
//! the one line that would make it true has not done the thing it was asked to
//! do — the user asked for the daemon to be running in the background, and the
//! file is a means to that, not the deliverable.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

/// What the current platform uses to keep a user agent running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Manager {
    /// macOS launchd, per-user domain.
    Launchd,
    /// systemd, `--user` scope.
    SystemdUser,
    /// Windows Task Scheduler, at logon.
    SchTasks,
}

impl Manager {
    /// Detect the mechanism for this machine.
    fn detect() -> Option<Self> {
        if cfg!(target_os = "macos") {
            return Some(Self::Launchd);
        }
        if cfg!(target_os = "windows") {
            return Some(Self::SchTasks);
        }
        if cfg!(target_os = "linux") {
            // `systemctl --user` needs a user bus. In a container or over a
            // bare SSH session there often is not one, and pretending
            // otherwise produces a baffling failure at enable time.
            return std::path::Path::new("/run/systemd/system")
                .exists()
                .then_some(Self::SystemdUser);
        }
        None
    }

    fn label(self) -> &'static str {
        match self {
            Self::Launchd => "launchd (user agent)",
            Self::SystemdUser => "systemd (user unit)",
            Self::SchTasks => "Task Scheduler (at logon)",
        }
    }
}

/// What happened when we asked for the daemon in the background.
#[derive(Debug)]
pub(crate) enum Outcome {
    /// Installed and started. It comes back after a reboot.
    Running(Manager),
    /// The unit is on disk but the start failed. `retry` is the command that
    /// would finish the job, so the caller can print something actionable
    /// rather than a transport error.
    Installed { retry: String },
    /// Nowhere to install to. The foreground is the answer here.
    Unsupported,
}

/// Run `ironwire service <action>`.
pub(crate) fn run(action: &str, port: Option<u16>) -> Result<()> {
    match action {
        "install" => match install_and_start(port)? {
            Outcome::Running(manager) => {
                println!("IronWire is running as a {}.", manager.label());
                println!("{}", linger_note(manager));
                Ok(())
            }
            Outcome::Installed { retry } => {
                println!();
                println!("The unit is installed but did not start. Finish with:");
                println!("    {retry}");
                Ok(())
            }
            Outcome::Unsupported => Err(unsupported()),
        },
        "uninstall" => match Manager::detect() {
            Some(manager) => uninstall(manager),
            None => Err(unsupported()),
        },
        "status" => match Manager::detect() {
            Some(manager) => status(manager),
            None => Err(unsupported()),
        },
        other => bail!("unknown action `{other}` (try: install, uninstall, status)"),
    }
}

/// Write the unit and start it.
///
/// The one entry point `init` uses, so "set up IronWire" and "run IronWire as a
/// service" cannot drift into meaning different things.
pub(crate) fn install_and_start(port: Option<u16>) -> Result<Outcome> {
    let Some(manager) = Manager::detect() else {
        return Ok(Outcome::Unsupported);
    };
    let exe = binary()?;
    let unit = match manager {
        Manager::Launchd => Some(install_launchd(&exe, port)?),
        Manager::SystemdUser => Some(install_systemd(&exe, port)?),
        Manager::SchTasks => None,
    };

    match start(manager, unit.as_deref(), &exe, port) {
        Ok(()) => Ok(Outcome::Running(manager)),
        Err(error) => {
            println!("Could not start it: {error}");
            Ok(Outcome::Installed {
                retry: start_command(manager, unit.as_deref()),
            })
        }
    }
}

/// Bring the installed unit up now, so it is running when this returns.
fn start(manager: Manager, unit: Option<&Path>, exe: &Path, port: Option<u16>) -> Result<()> {
    match manager {
        Manager::SystemdUser => {
            // A unit written but not re-read is the classic "it works after a
            // reboot" bug, so the reload is not optional.
            sh("systemctl", &["--user", "daemon-reload"])?;
            sh("systemctl", &["--user", "enable", "--now", "ironwire"])
        }
        Manager::Launchd => {
            let path = unit
                .context("a launchd install has a plist")?
                .display()
                .to_string();
            match sh("launchctl", &["load", "-w", &path]) {
                // Re-running `install` after an upgrade is normal, and launchd
                // calls that an error. The end state is what we promised.
                Err(error) if already_running(&error) => Ok(()),
                other => other,
            }
        }
        Manager::SchTasks => {
            let port_arg = port.map_or_else(String::new, |p| format!(" --port {p}"));
            let action = format!("\"{}\"{port_arg} serve", exe.display());
            sh(
                "schtasks",
                &[
                    "/Create", "/TN", TASK_NAME, "/SC", "ONLOGON", "/TR", &action, "/F",
                ],
            )?;
            // `ONLOGON` means the next logon, and the user asked for it now.
            sh("schtasks", &["/Run", "/TN", TASK_NAME])
        }
    }
}

/// The command that would finish an install that failed to start.
fn start_command(manager: Manager, unit: Option<&Path>) -> String {
    match manager {
        Manager::SystemdUser => "systemctl --user enable --now ironwire".to_string(),
        Manager::Launchd => format!(
            "launchctl load -w {}",
            unit.map_or_else(|| "<plist>".to_string(), |p| p.display().to_string())
        ),
        Manager::SchTasks => format!("schtasks /Run /TN {TASK_NAME}"),
    }
}

/// What keeps the daemon alive when nobody is logged in.
///
/// Never run for the user: lingering keeps a process of theirs running after
/// they log out, which is a decision about their machine, not a detail of ours.
fn linger_note(manager: Manager) -> String {
    match manager {
        Manager::SystemdUser => {
            "To keep it running while you are logged out: loginctl enable-linger $USER".to_string()
        }
        Manager::Launchd => "Logs: ~/Library/Logs/ironwire.log".to_string(),
        Manager::SchTasks => format!("Check it with: schtasks /Query /TN {TASK_NAME}"),
    }
}

/// The Windows task name, in one place so create, run, query and delete agree.
const TASK_NAME: &str = "IronWire";

/// Run a command, and turn a non-zero exit into an error carrying its stderr.
///
/// The stderr is the whole point: `systemctl` explains itself well, and
/// swallowing that in favour of "exit code 1" would make a failure here
/// undiagnosable.
fn sh(program: &str, args: &[&str]) -> Result<()> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("running `{program}`"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = stderr.trim();
    if detail.is_empty() {
        bail!("`{program} {}` failed", args.join(" "));
    }
    bail!("{detail}");
}

/// launchd's way of saying it is already up, which is success for our purposes.
fn already_running(error: &anyhow::Error) -> bool {
    let text = error.to_string().to_ascii_lowercase();
    text.contains("already loaded") || text.contains("service already loaded")
}

fn unsupported() -> anyhow::Error {
    anyhow::anyhow!(
        "no supported service manager here.\n\
         \n\
         On Linux this usually means there is no systemd user bus — common in\n\
         containers and over a bare SSH session.\n\
         \n\
         Run it in the foreground instead, or under whatever supervisor you\n\
         already use:\n\
         \n\
             ironwire serve"
    )
}

fn binary() -> Result<PathBuf> {
    // The absolute path of *this* binary. A service unit that says `ironwire`
    // depends on a PATH the supervisor may not have, which is the classic way
    // a unit that works interactively fails at boot.
    std::env::current_exe().context("locating the ironwire binary")
}

/// Where each manager keeps the artifact we wrote, if it keeps one.
///
/// Task Scheduler has no file: the task *is* the registration, which is why
/// `install_and_start` passes `None` for it and `status` has to ask `schtasks`.
pub(crate) fn unit_path(manager: Manager) -> Result<Option<PathBuf>> {
    Ok(match manager {
        Manager::Launchd => Some(launchd_plist_path()?),
        Manager::SystemdUser => Some(systemd_unit_path()?),
        Manager::SchTasks => None,
    })
}

// ---------------------------------------------------------------- launchd

fn launchd_plist_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("locating your home directory")?;
    Ok(home
        .join("Library")
        .join("LaunchAgents")
        .join("dev.ironwire.daemon.plist"))
}

fn install_launchd(exe: &std::path::Path, port: Option<u16>) -> Result<PathBuf> {
    let path = launchd_plist_path()?;
    let home = dirs::home_dir().context("locating your home directory")?;
    let logs = home.join("Library").join("Logs");
    let mut args = format!(
        "    <string>{}</string>\n    <string>serve</string>\n",
        xml_escape(&exe.display().to_string())
    );
    if let Some(port) = port {
        args.push_str(&format!(
            "    <string>--port</string>\n    <string>{port}</string>\n"
        ));
    }

    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>dev.ironwire.daemon</string>
  <key>ProgramArguments</key>
  <array>
{args}  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <dict><key>SuccessfulExit</key><false/></dict>
  <key>ProcessType</key>
  <string>Background</string>
  <key>StandardOutPath</key>
  <string>{log}</string>
  <key>StandardErrorPath</key>
  <string>{log}</string>
</dict>
</plist>
"#,
        log = xml_escape(&logs.join("ironwire.log").display().to_string()),
    );

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::create_dir_all(&logs).ok();
    announce_write(&path);
    std::fs::write(&path, plist).with_context(|| format!("writing {}", path.display()))?;

    println!("Logs: {}", logs.join("ironwire.log").display());
    Ok(path)
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// ---------------------------------------------------------------- systemd

fn systemd_unit_path() -> Result<PathBuf> {
    let base = dirs::config_dir()
        .or_else(dirs::home_dir)
        .context("locating your config directory")?;
    Ok(base.join("systemd").join("user").join("ironwire.service"))
}

fn install_systemd(exe: &std::path::Path, port: Option<u16>) -> Result<PathBuf> {
    let path = systemd_unit_path()?;
    let port_arg = port.map_or_else(String::new, |p| format!(" --port {p}"));

    // The hardening directives are cheap and they bound the blast radius of a
    // bug in a process that holds credentials. `ProtectHome=read-write` rather
    // than `true`, because reading `~/.claude` and `~/.codex` is the job.
    let unit = format!(
        "# Written by `ironwire service install`.\n\
         #\n\
         # A **user** unit. The daemon reads credentials from this user's home\n\
         # directory and binds 127.0.0.1 only; running it as root would give a\n\
         # system service access to a user's OAuth tokens.\n\
         \n\
         [Unit]\n\
         Description=IronWire — localhost router for coding agents\n\
         Documentation=https://github.com/nearai/ironwire\n\
         After=network-online.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={exe}{port_arg} serve\n\
         Restart=on-failure\n\
         RestartSec=5s\n\
         NoNewPrivileges=true\n\
         PrivateTmp=true\n\
         ProtectSystem=strict\n\
         ProtectHome=read-write\n\
         ProtectKernelTunables=true\n\
         ProtectKernelModules=true\n\
         RestrictNamespaces=true\n\
         RestrictSUIDSGID=true\n\
         LockPersonality=true\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        exe = exe.display(),
    );

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    announce_write(&path);
    std::fs::write(&path, unit).with_context(|| format!("writing {}", path.display()))?;

    println!("Logs: journalctl --user -u ironwire -f");
    Ok(path)
}

// ---------------------------------------------------------------- teardown

/// Stop it and take the unit back out.
///
/// Symmetric with install: it *stops*, rather than printing the command that
/// would. A stop that fails is reported and does not block the removal — the
/// user asked for this gone, and a unit file left behind because `systemctl`
/// was unhappy is the worse outcome.
fn uninstall(manager: Manager) -> Result<()> {
    match manager {
        Manager::Launchd => {
            let path = launchd_plist_path()?;
            if path.exists() {
                report(sh(
                    "launchctl",
                    &["unload", "-w", &path.display().to_string()],
                ));
            }
            remove_if_present(&path)
        }
        Manager::SystemdUser => {
            report(sh("systemctl", &["--user", "disable", "--now", "ironwire"]));
            remove_if_present(&systemd_unit_path()?)?;
            report(sh("systemctl", &["--user", "daemon-reload"]));
            Ok(())
        }
        Manager::SchTasks => {
            report(sh("schtasks", &["/Delete", "/TN", TASK_NAME, "/F"]));
            println!("Removed the {TASK_NAME} logon task.");
            Ok(())
        }
    }
}

/// Say that a teardown step failed, and carry on with the rest.
fn report(result: Result<()>) {
    if let Err(error) = result {
        println!("  (continuing) {error}");
    }
}

fn remove_if_present(path: &std::path::Path) -> Result<()> {
    if path.exists() {
        std::fs::remove_file(path).with_context(|| format!("removing {}", path.display()))?;
        println!("Removed {}", path.display());
    } else {
        println!("{} does not exist — nothing to remove.", path.display());
    }
    Ok(())
}

fn status(manager: Manager) -> Result<()> {
    println!("Service manager: {}", manager.label());
    match unit_path(manager)? {
        Some(path) if path.exists() => println!("Installed:       {}", path.display()),
        Some(path) => {
            println!("Installed:       no ({} does not exist)", path.display());
            println!();
            println!("    ironwire service install");
        }
        None => println!("Check with:      schtasks /Query /TN {TASK_NAME}"),
    }
    Ok(())
}

/// TRUST.md: name the file before touching it, every time.
fn announce_write(path: &std::path::Path) {
    println!("Writing {}", path.display());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plist_escapes_a_path_that_would_break_the_xml() {
        // Homes with an `&` in them are rare and the failure is total: launchd
        // silently refuses to load a malformed plist.
        assert_eq!(
            xml_escape("/Users/a&b/<bin>/ironwire"),
            "/Users/a&amp;b/&lt;bin&gt;/ironwire"
        );
    }

    /// Re-running `install` after an upgrade is the common case, and launchd
    /// calls an already-loaded agent an error. Treating that as a failure would
    /// make every upgrade look broken.
    #[test]
    fn launchd_saying_it_is_already_loaded_is_not_a_failure() {
        assert!(already_running(&anyhow::anyhow!(
            "Load failed: 37: Service already loaded"
        )));
        assert!(!already_running(&anyhow::anyhow!(
            "Load failed: 5: Input/output error"
        )));
    }

    /// A start that fails has to leave the user a command that finishes the
    /// job, or the install is a dead end.
    #[test]
    fn a_failed_start_names_the_command_that_would_finish_it() {
        assert!(start_command(Manager::SystemdUser, None).contains("enable --now"));
        assert!(
            start_command(Manager::Launchd, Some(std::path::Path::new("/tmp/a.plist")))
                .contains("/tmp/a.plist")
        );
        assert!(start_command(Manager::SchTasks, None).contains(TASK_NAME));
    }

    #[test]
    fn every_action_is_recognised_or_named() {
        // A typo should say what the options are, not fail obscurely.
        let err = run("strat", None).unwrap_err().to_string();
        assert!(
            err.contains("install") || err.contains("service manager"),
            "got: {err}"
        );
    }
}
