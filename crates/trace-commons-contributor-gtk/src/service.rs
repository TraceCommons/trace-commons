//! The background service from this window: which dialog a state gives,
//! when one may appear, and the facts the core's classification needs.
//!
//! The classification itself is the core's (`daemon::service`), so the Mac
//! and Windows shells can draw the same answer later. The flow is the
//! owner's (2026-11-17): a dialog appears during first setup and when the
//! person does something that needs the service, never on an ordinary
//! start; after "Not now" it stays quiet for the run, and the Private AI
//! start area keeps a "Set up" button beside its unavailable line.
//!
//! The only thing this window asks of the host is systemd's `RestartUnit` for
//! the channel's fixed unit, on the session bus (besides reading the unit's
//! state). It never runs the install
//! command: that is copied to the clipboard.

use std::path::{Path, PathBuf};

use serde_json::Value;
use trace_commons_contributor::daemon::host_link::{self, LinkState};
use trace_commons_contributor::daemon::service::{ServiceFacts, ServiceState, answer_from_hello};

/// The three dialogs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialog {
    /// "Finish setting up": the install command, Copy, guide, Not now.
    Install,
    /// "The background service isn't running": Start it, Not now.
    Start,
    /// "Update the background service": the install command again.
    Update,
}

/// The dialog a state gives; none while the service runs.
pub fn dialog_for(state: ServiceState) -> Option<Dialog> {
    match state {
        ServiceState::Running => None,
        ServiceState::NotInstalled => Some(Dialog::Install),
        ServiceState::InstalledNotRunning => Some(Dialog::Start),
        ServiceState::TooOld => Some(Dialog::Update),
    }
}

/// What can bring a dialog up. An ordinary app start is deliberately not
/// one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// First setup, before the session folders are declared.
    FirstSetup,
    /// Launch pressed in the Private AI start area.
    Launch,
    /// The "Set up" button beside the unavailable line.
    SetUp,
}

/// Whether `trigger` shows a dialog now. "Not now" quiets the automatic
/// triggers for the rest of the run; "Set up" is the person asking, so it
/// always answers.
pub fn prompt(trigger: Trigger, state: ServiceState, dismissed: bool) -> Option<Dialog> {
    let dialog = dialog_for(state)?;
    match trigger {
        Trigger::SetUp => Some(dialog),
        Trigger::FirstSetup | Trigger::Launch => (!dismissed).then_some(dialog),
    }
}

/// Whether the Private AI start area offers "Set up": in a sandbox whose
/// daemon cannot open a terminal because it is not the host daemon. A
/// window outside a sandbox opens terminals itself and never offers it.
pub fn offers_set_up(snapshot: &Value, sandboxed: bool) -> bool {
    sandboxed
        && snapshot["capabilities"]["daemon_on_host"] != true
        && snapshot["capabilities"]["terminal_launch"] != true
}

/// What pressing Launch does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchPress {
    /// The launch dialog, as before.
    Open,
    /// The background-service dialog.
    OfferService,
    /// Nothing: Launch is insensitive.
    Unavailable,
}

/// Launch's behaviour. Unchanged wherever a terminal can open; in a sandbox
/// without the host daemon it offers the service until "Not now", then is
/// quiet ("Set up" stays).
pub fn launch_press(
    snapshot: &Value,
    sandboxed: bool,
    has_accounts: bool,
    dismissed: bool,
) -> LaunchPress {
    if snapshot["capabilities"]["terminal_launch"] == true && has_accounts {
        LaunchPress::Open
    } else if offers_set_up(snapshot, sandboxed) && !dismissed {
        LaunchPress::OfferService
    } else {
        LaunchPress::Unavailable
    }
}

/// Whether first setup asks about the service before the folders screen:
/// on Linux, unless a headless driver (`--exit-after-realize`,
/// `--start-page`, `--onboarding-page`) runs, which expects that screen.
pub fn first_setup_asks(on_linux: bool, headless_driver: bool) -> bool {
    on_linux && !headless_driver
}

/// Whether the window shows the folders screen for the daemon it is
/// attached to (owner, 2026-11-17): after a switch-over, or any attach to
/// the background service, whose daemon has no session folders declared.
/// `settings` is that daemon's `get_settings`. Never while this window
/// hosts the daemon (it cannot start without them), at most once per
/// attachment (`asked`), and not while first run is open, whose own Tools
/// step asks the same question.
pub fn asks_for_folders(
    hosting: bool,
    settings: &Value,
    asked: bool,
    first_run_open: bool,
) -> bool {
    // `null` is "never asked" (`daemon::settings::roots_declared` needs
    // both); a missing key is a daemon that does not say, never a question.
    let undeclared = ["claude_source", "codex_source"]
        .iter()
        .any(|key| settings.get(key).is_some_and(Value::is_null));
    !hosting && !asked && !first_run_open && undeclared
}

/// Whether a window attached to the host daemon goes back to running its
/// own: the host part is gone (no socket answers, no unit marker), and the
/// last try did not already fail with the link in this same state.
pub fn falls_back(via_host_link: bool, link: LinkState, last_try_failed: bool) -> bool {
    via_host_link && link == LinkState::Absent && !last_try_failed
}

/// What the open dialog does after a fresh look at the service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchStep {
    /// The service runs: close, showing nothing else.
    Close,
    /// The install finished but the service is not up -- this window's own
    /// daemon holds it off, or it is still starting: stop that daemon, ask
    /// systemd to restart the unit, attach.
    SwitchOver,
    /// Keep waiting.
    Keep,
}

/// The switch-overs an open dialog has tried.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Tries {
    /// How many it has made.
    pub made: u32,
    /// How long ago the last one ended; `None` before the first.
    pub since_last: Option<std::time::Duration>,
}

/// The not-installed and too-old dialogs watch for the service while open.
pub fn watch_step(open: Dialog, state: ServiceState, tries: Tries) -> WatchStep {
    match (open, state) {
        (_, ServiceState::Running) => WatchStep::Close,
        // After the command ran: this window's daemon may hold the new
        // unit off (the update case too, where an older daemon ran beside a
        // sandboxed one). Tried again after a pause when a try did not bring
        // it up, a bounded number of times.
        (Dialog::Install | Dialog::Update, ServiceState::InstalledNotRunning)
            if tries.made < SWITCH_TRIES
                && (tries.made == 0
                    || tries
                        .since_last
                        .is_some_and(|ago| ago >= SWITCH_RETRY_AFTER)) =>
        {
            WatchStep::SwitchOver
        }
        _ => WatchStep::Keep,
    }
}

/// How often an open dialog looks again.
pub const WATCH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// How long a switch-over waits for the unit to answer after `RestartUnit`.
/// The unit is `Type=simple` with no `ExecStartPre` (`daemon::install`), and
/// `RestartUnit` skips any `RestartSec` wait, so this covers only the
/// daemon's own start: loading its state, taking `watcher.lock`, binding the
/// sockets -- seconds, not tens of them.
pub const SWITCH_WAIT: std::time::Duration = std::time::Duration::from_secs(20);

/// How long after a switch-over that did not bring the service up an open
/// dialog waits before trying again. Longer than [`SWITCH_WAIT`], so a unit
/// still starting is not restarted again under itself.
pub const SWITCH_RETRY_AFTER: std::time::Duration = std::time::Duration::from_secs(30);

/// How many switch-overs an open dialog makes in all: each restarts the unit
/// and replaces this window's daemon.
pub const SWITCH_TRIES: u32 = 3;

/// The systemd user manager's `RestartUnit(in s name, in s mode, out o job)`
/// (org.freedesktop.systemd1(5)), as one fixed call: destination, object,
/// interface, method, then the arguments. Only the channel's unit.
///
/// Restart, not start: while the unit waits out `RestartSec=30` after a
/// start that failed on this window's lock (`auto-restart`), systemd before
/// v257 queues a `StartUnit` job until that wait ends; a restart job starts
/// the unit at once. A unit that is not running is started.
pub fn restart_unit_call() -> (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    (&'static str, &'static str),
) {
    (
        SYSTEMD_DESTINATION,
        SYSTEMD_PATH,
        SYSTEMD_MANAGER,
        "RestartUnit",
        (
            trace_commons_contributor::channel::SYSTEMD_UNIT_NAME,
            "replace",
        ),
    )
}

const SYSTEMD_DESTINATION: &str = "org.freedesktop.systemd1";
const SYSTEMD_PATH: &str = "/org/freedesktop/systemd1";
const SYSTEMD_MANAGER: &str = "org.freedesktop.systemd1.Manager";

/// How long one systemd call may take.
const BUS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Ask the systemd user manager to restart the unit ([`restart_unit_call`]).
/// Blocking and bounded; callers run it off the GTK thread, after stopping
/// any daemon this window runs, so the unit can take `watcher.lock`. `Ok`
/// means systemd queued the job, not that the daemon came up: the probe
/// says that.
pub fn restart_unit() -> anyhow::Result<()> {
    bounded(|| {
        let (destination, path, interface, method, args) = restart_unit_call();
        let connection = zbus::blocking::Connection::session()?;
        let proxy = zbus::blocking::Proxy::new(&connection, destination, path, interface)?;
        let _job: zbus::zvariant::OwnedObjectPath = proxy.call(method, &args)?;
        Ok(())
    })
    .unwrap_or_else(|| Err(anyhow::anyhow!("systemd-timeout")))
}

/// Whether the unit's file is installed, from systemd's `GetUnitFileState`.
/// `Some(false)` when systemd answers that it has none (or there is no
/// systemd on the bus); `None` when the bus cannot be asked.
pub fn unit_file_installed() -> Option<bool> {
    bounded(|| {
        let connection = zbus::blocking::Connection::session().ok()?;
        let proxy = zbus::blocking::Proxy::new(
            &connection,
            SYSTEMD_DESTINATION,
            SYSTEMD_PATH,
            SYSTEMD_MANAGER,
        )
        .ok()?;
        match proxy.call::<_, _, String>(
            "GetUnitFileState",
            &(trace_commons_contributor::channel::SYSTEMD_UNIT_NAME,),
        ) {
            // The reference does not say whether a missing file is an error
            // or a state; read both as not installed.
            Ok(state) => Some(!matches!(state.as_str(), "" | "invalid" | "not-found")),
            Err(zbus::Error::MethodError(..)) => Some(false),
            Err(_) => None,
        }
    })
    .flatten()
}

/// Whether systemd reports the unit `active`, and for how long: `GetUnit`,
/// then the unit's `ActiveState` and `ActiveEnterTimestamp`
/// (org.freedesktop.systemd1(5): `CLOCK_REALTIME` microseconds, 0 when not
/// recorded this boot). `Some((false, None))` when the unit is not loaded or
/// in any other state (`activating` while it restarts); `None` when the bus
/// cannot be asked.
pub fn unit_active() -> Option<(bool, Option<std::time::Duration>)> {
    bounded(|| {
        let connection = zbus::blocking::Connection::session().ok()?;
        let manager = zbus::blocking::Proxy::new(
            &connection,
            SYSTEMD_DESTINATION,
            SYSTEMD_PATH,
            SYSTEMD_MANAGER,
        )
        .ok()?;
        let path: zbus::zvariant::OwnedObjectPath = match manager.call(
            "GetUnit",
            &(trace_commons_contributor::channel::SYSTEMD_UNIT_NAME,),
        ) {
            Ok(path) => path,
            Err(zbus::Error::MethodError(..)) => return Some((false, None)),
            Err(_) => return None,
        };
        let unit = zbus::blocking::Proxy::new(
            &connection,
            SYSTEMD_DESTINATION,
            path,
            "org.freedesktop.systemd1.Unit",
        )
        .ok()?;
        let state: String = unit.get_property("ActiveState").ok()?;
        if state != "active" {
            return Some((false, None));
        }
        let entered: Option<u64> = unit.get_property("ActiveEnterTimestamp").ok();
        Some((
            true,
            entered.and_then(|usec| active_for(usec, std::time::SystemTime::now())),
        ))
    })
    .flatten()
}

/// How long ago `entered_usec` (`CLOCK_REALTIME` microseconds) was, from
/// `now`. `None` for 0 (not recorded) or a time after `now`: an unknown
/// duration never counts toward the too-old grace.
pub fn active_for(entered_usec: u64, now: std::time::SystemTime) -> Option<std::time::Duration> {
    if entered_usec == 0 {
        return None;
    }
    let entered = std::time::UNIX_EPOCH + std::time::Duration::from_micros(entered_usec);
    now.duration_since(entered).ok()
}

/// Run `f` on its own thread for at most [`BUS_TIMEOUT`].
fn bounded<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(BUS_TIMEOUT).ok()
}

/// The unit file `daemon install` writes, on the host. Never visible from a
/// sandbox, whose config directory is the app's own.
fn host_unit_file() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| {
        dir.join("systemd/user")
            .join(trace_commons_contributor::channel::SYSTEMD_UNIT_NAME)
    })
}

/// `hello` from a daemon at `socket`, `None` when nothing answers.
pub fn hello_at(socket: &Path) -> Option<Value> {
    trace_commons_contributor::daemon::client::try_call_at(socket, "hello", &serde_json::json!({}))
        .ok()
        .flatten()
        .and_then(|response| response.result)
}

/// The daemons to ask `hello`, in order: the one this window is attached
/// to, then one listening on the host link -- never this window's own
/// daemon. The first that answers counts: in a sandbox at first setup the
/// "attached" socket is the sandbox's own, where nobody listens.
pub fn hello_targets(
    hosting: bool,
    attached: Option<&Path>,
    link: LinkState,
    link_socket: Option<&Path>,
) -> Vec<PathBuf> {
    if hosting {
        return Vec::new();
    }
    let mut targets: Vec<PathBuf> = attached.map(Path::to_path_buf).into_iter().collect();
    if let Some(link_socket) = link_socket.filter(|_| link == LinkState::Listening)
        && attached != Some(link_socket)
    {
        targets.push(link_socket.to_path_buf());
    }
    targets
}

/// Wait at most `bound` for the service to answer: on the host link, or,
/// outside a sandbox, on the state directory's socket `local`. Whether it
/// did.
pub fn wait_for_service(bound: std::time::Duration, local: &Path) -> bool {
    let deadline = std::time::Instant::now() + bound;
    let sandboxed = host_link::runs_sandboxed();
    loop {
        let link = host_link::socket_path().map(|sock| host_link::probe(&sock));
        if link == Some(LinkState::Listening)
            || (!sandboxed && trace_commons_contributor::daemon::client::is_running_at(local))
        {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

/// Gather the facts, for a window that hosts its own daemon or is attached
/// to `attached`. Blocking (a probe, a `hello`, one bounded bus call); run
/// it off the GTK thread.
pub fn gather(hosting: bool, attached: Option<&Path>) -> ServiceFacts {
    let link_socket = host_link::socket_path();
    let link = link_socket
        .as_deref()
        .map(host_link::probe)
        .unwrap_or(LinkState::Absent);
    let unit_installed = unit_file_installed().or_else(|| {
        (!host_link::runs_sandboxed())
            .then(host_unit_file)
            .flatten()
            .map(|file| file.exists())
    });
    let active = unit_active();
    let hello = hello_targets(hosting, attached, link, link_socket.as_deref())
        .iter()
        .find_map(|socket| hello_at(socket));
    ServiceFacts {
        link,
        hosting,
        unit_installed,
        unit_active: active.map(|(active, _)| active),
        unit_active_for: active.and_then(|(_, since)| since),
        answer: answer_from_hello(hello.as_ref()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use trace_commons_contributor::daemon::service::DaemonAnswer;

    const STATES: [ServiceState; 4] = [
        ServiceState::Running,
        ServiceState::NotInstalled,
        ServiceState::InstalledNotRunning,
        ServiceState::TooOld,
    ];

    #[test]
    fn each_state_gives_its_dialog() {
        assert_eq!(dialog_for(ServiceState::Running), None);
        assert_eq!(
            dialog_for(ServiceState::NotInstalled),
            Some(Dialog::Install)
        );
        assert_eq!(
            dialog_for(ServiceState::InstalledNotRunning),
            Some(Dialog::Start)
        );
        assert_eq!(dialog_for(ServiceState::TooOld), Some(Dialog::Update));
    }

    #[test]
    fn first_setup_and_launch_prompt_until_not_now() {
        for trigger in [Trigger::FirstSetup, Trigger::Launch] {
            for state in STATES {
                assert_eq!(prompt(trigger, state, false), dialog_for(state));
                // Quiet after "Not now".
                assert_eq!(prompt(trigger, state, true), None);
            }
        }
    }

    #[test]
    fn set_up_always_answers_and_a_running_service_never_prompts() {
        for state in STATES {
            for dismissed in [false, true] {
                assert_eq!(prompt(Trigger::SetUp, state, dismissed), dialog_for(state));
            }
        }
        for trigger in [Trigger::FirstSetup, Trigger::Launch, Trigger::SetUp] {
            assert_eq!(prompt(trigger, ServiceState::Running, false), None);
        }
    }

    fn snapshot(on_host: Option<bool>, terminal: bool) -> Value {
        let mut caps = json!({ "terminal_launch": terminal });
        if let Some(on_host) = on_host {
            caps["daemon_on_host"] = json!(on_host);
        }
        json!({ "capabilities": caps })
    }

    #[test]
    fn set_up_is_offered_only_in_a_sandbox_without_the_host_daemon() {
        assert!(offers_set_up(&snapshot(Some(false), false), true));
        // An older daemon omits the capability: not on the host.
        assert!(offers_set_up(&snapshot(None, false), true));
        assert!(!offers_set_up(&snapshot(Some(true), true), true));
        assert!(!offers_set_up(&snapshot(Some(true), false), true));
        // Outside a sandbox the window opens terminals itself.
        assert!(!offers_set_up(&snapshot(Some(true), false), false));
        assert!(!offers_set_up(&snapshot(None, false), false));
    }

    #[test]
    fn launch_offers_the_service_in_a_sandbox_until_not_now() {
        let sandbox = snapshot(Some(false), false);
        assert_eq!(
            launch_press(&sandbox, true, false, false),
            LaunchPress::OfferService
        );
        assert_eq!(
            launch_press(&sandbox, true, true, false),
            LaunchPress::OfferService
        );
        assert_eq!(
            launch_press(&sandbox, true, true, true),
            LaunchPress::Unavailable
        );
        // Unchanged where a terminal opens.
        let host = snapshot(Some(true), true);
        assert_eq!(launch_press(&host, true, true, false), LaunchPress::Open);
        assert_eq!(launch_press(&host, false, true, false), LaunchPress::Open);
        assert_eq!(
            launch_press(&host, false, false, false),
            LaunchPress::Unavailable
        );
        // Outside a sandbox a missing terminal is not the service's to fix.
        assert_eq!(
            launch_press(&snapshot(Some(true), false), false, true, false),
            LaunchPress::Unavailable
        );
    }

    fn tried(made: u32, ago: Option<u64>) -> Tries {
        Tries {
            made,
            since_last: ago.map(std::time::Duration::from_secs),
        }
    }

    #[test]
    fn an_open_dialog_closes_when_the_service_runs_and_switches_over() {
        for dialog in [Dialog::Install, Dialog::Start, Dialog::Update] {
            for tries in [tried(0, None), tried(SWITCH_TRIES, Some(0))] {
                assert_eq!(
                    watch_step(dialog, ServiceState::Running, tries),
                    WatchStep::Close
                );
            }
        }
        assert_eq!(
            watch_step(
                Dialog::Install,
                ServiceState::InstalledNotRunning,
                tried(0, None)
            ),
            WatchStep::SwitchOver
        );
        assert_eq!(
            watch_step(Dialog::Install, ServiceState::NotInstalled, tried(0, None)),
            WatchStep::Keep
        );
        assert_eq!(
            watch_step(Dialog::Update, ServiceState::TooOld, tried(0, None)),
            WatchStep::Keep
        );
        // Updated, but held off by this window's own daemon.
        assert_eq!(
            watch_step(
                Dialog::Update,
                ServiceState::InstalledNotRunning,
                tried(0, None)
            ),
            WatchStep::SwitchOver
        );
        // "Start it" is the person's to press.
        assert_eq!(
            watch_step(
                Dialog::Start,
                ServiceState::InstalledNotRunning,
                tried(0, None)
            ),
            WatchStep::Keep
        );
    }

    /// A switch-over that did not bring the service up is tried again, so
    /// the dialog does not wait forever on one failed attempt -- but not on
    /// every 2 s look (each try restarts the unit and replaces this
    /// window's daemon), and not without end.
    #[test]
    fn a_failed_switch_over_is_tried_again_after_a_pause_a_few_times() {
        let retry = SWITCH_RETRY_AFTER.as_secs();
        for dialog in [Dialog::Install, Dialog::Update] {
            let step = |tries| watch_step(dialog, ServiceState::InstalledNotRunning, tries);
            // Just failed: wait.
            assert_eq!(step(tried(1, Some(0))), WatchStep::Keep);
            assert_eq!(step(tried(1, Some(retry - 1))), WatchStep::Keep);
            // After the pause: again.
            assert_eq!(step(tried(1, Some(retry))), WatchStep::SwitchOver);
            assert_eq!(
                step(tried(SWITCH_TRIES - 1, Some(retry))),
                WatchStep::SwitchOver
            );
            // Bounded.
            assert_eq!(step(tried(SWITCH_TRIES, Some(retry * 10))), WatchStep::Keep);
        }
        // The pause outlasts a whole switch-over's wait, so a try that is
        // still settling is never cut short by the next.
        assert!(SWITCH_RETRY_AFTER > SWITCH_WAIT);
        assert!(SWITCH_TRIES >= 2);
    }

    #[test]
    fn first_setup_asks_on_linux_unless_a_headless_driver_runs() {
        assert!(first_setup_asks(true, false));
        assert!(!first_setup_asks(false, false));
        // The container smoke run expects the roots screen.
        assert!(!first_setup_asks(true, true));
    }

    /// A dialog comes up from exactly two places: first setup's roots
    /// branch in `main.rs`, and `offer_service` (Launch and "Set up") in
    /// the Private AI start area. Never from an ordinary start (owner,
    /// 2026-11-17). A new call site has to be added here, on purpose.
    #[test]
    fn dialogs_are_presented_only_from_first_setup_and_private_ai() {
        // Split, so this file does not match itself.
        const CALL: &str = concat!("service_dialog::", "present(");
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut found = Vec::new();
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                let calls = text.matches(CALL).count();
                if calls > 0 {
                    let name = path.strip_prefix(&root).unwrap().display().to_string();
                    found.push((name, calls));
                }
            }
        }
        found.sort();
        assert_eq!(
            found,
            vec![
                ("main.rs".to_string(), 1),
                ("ui/managed_sessions.rs".to_string(), 1)
            ]
        );
        // The one in main.rs is inside the roots-not-declared branch.
        let main = std::fs::read_to_string(root.join("main.rs")).unwrap();
        let branch = main.find("ERR_ROOTS_NOT_DECLARED =>").unwrap();
        let call = main.find(CALL).unwrap();
        let after = main[branch..].find("Err(error) =>").unwrap() + branch;
        assert!(branch < call && call < after);
    }

    #[test]
    fn an_attached_daemon_without_folders_gets_the_folders_screen_once() {
        let undeclared = json!({"claude_source": null, "codex_source": null});
        let half = json!({"claude_source": {"watch": "/c"}, "codex_source": null});
        let declared = json!({"claude_source": {"watch": "/c"}, "codex_source": "off"});
        assert!(asks_for_folders(false, &undeclared, false, false));
        assert!(asks_for_folders(false, &half, false, false));
        assert!(!asks_for_folders(false, &declared, false, false));
        // Once per attachment.
        assert!(!asks_for_folders(false, &undeclared, true, false));
        // First run's Tools step asks it.
        assert!(!asks_for_folders(false, &undeclared, false, true));
        // A hosting window cannot have started without them.
        assert!(!asks_for_folders(true, &undeclared, false, false));
        // A daemon that does not report the fields says nothing either way.
        assert!(!asks_for_folders(false, &json!({}), false, false));
        assert!(!asks_for_folders(false, &json!(null), false, false));
    }

    #[test]
    fn the_window_falls_back_only_when_the_host_part_is_gone() {
        assert!(falls_back(true, LinkState::Absent, false));
        // Down is a host daemon restarting or stopped: stay attached.
        assert!(!falls_back(true, LinkState::Down, false));
        assert!(!falls_back(true, LinkState::Listening, false));
        // Not attached over the host link: nothing to fall back from.
        assert!(!falls_back(false, LinkState::Absent, false));
        // A fallback that could not open stays quiet until the link changes.
        assert!(!falls_back(true, LinkState::Absent, true));
    }

    #[test]
    fn the_switch_over_restarts_the_channels_fixed_unit_only() {
        let (destination, path, interface, method, (unit, mode)) = restart_unit_call();
        assert_eq!(destination, "org.freedesktop.systemd1");
        assert_eq!(path, "/org/freedesktop/systemd1");
        assert_eq!(interface, "org.freedesktop.systemd1.Manager");
        // RestartUnit, not StartUnit: before systemd v257 a start job
        // queued while the unit waits out `RestartSec` (auto-restart) runs
        // only when that wait ends; a restart cuts it short.
        assert_eq!(method, "RestartUnit");
        assert_eq!(unit, trace_commons_contributor::channel::SYSTEMD_UNIT_NAME);
        assert_eq!(mode, "replace");
    }

    #[test]
    fn an_unknown_or_future_activation_time_is_no_duration() {
        let now = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000);
        assert_eq!(active_for(0, now), None);
        assert_eq!(active_for(1_001_000_000, now), None);
        assert_eq!(
            active_for(990_000_000, now),
            Some(std::time::Duration::from_secs(10))
        );
    }

    #[test]
    fn hello_goes_to_the_attached_daemon_then_a_listening_link_never_our_own() {
        let attached = Path::new("/state/daemon.sock");
        let link = Path::new("/run/user/1/trace-commons/daemon.sock");
        // The attached socket first, then the listening link: in a sandbox
        // at first setup the attached socket is the sandbox's own, empty.
        assert_eq!(
            hello_targets(false, Some(attached), LinkState::Listening, Some(link)),
            vec![attached.to_path_buf(), link.to_path_buf()]
        );
        assert_eq!(
            hello_targets(false, None, LinkState::Listening, Some(link)),
            vec![link.to_path_buf()]
        );
        // Attached over the link: asked once.
        assert_eq!(
            hello_targets(false, Some(link), LinkState::Listening, Some(link)),
            vec![link.to_path_buf()]
        );
        assert!(hello_targets(false, None, LinkState::Down, Some(link)).is_empty());
        assert_eq!(
            hello_targets(false, Some(attached), LinkState::Absent, Some(link)),
            vec![attached.to_path_buf()]
        );
        // A hosting window's host link is its own daemon.
        assert!(hello_targets(true, Some(attached), LinkState::Listening, Some(link)).is_empty());
    }

    #[test]
    fn hello_at_reads_a_daemons_answer_and_nothing_from_nobody() {
        use std::io::{BufRead, BufReader, Write};
        let dir = std::env::temp_dir().join(format!("tc-svc-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let sock = dir.join("d.sock");
        assert_eq!(hello_at(&sock), None);
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            assert!(line.contains("\"hello\""));
            (&stream)
                .write_all(b"{\"id\":1,\"result\":{\"service_level\":1}}\n")
                .unwrap();
        });
        let hello = hello_at(&sock).expect("answered");
        server.join().unwrap();
        assert_eq!(answer_from_hello(Some(&hello)), DaemonAnswer::Meets);
        let _ = std::fs::remove_dir_all(dir);
    }
}
