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
//! The only thing this window asks of the host is systemd's `StartUnit` for
//! the channel's fixed unit, on the session bus. It never runs the install
//! command: that is copied to the clipboard.

use std::path::{Path, PathBuf};

use serde_json::Value;
use trace_commons_contributor::daemon::host_link::{self, LinkState};
use trace_commons_contributor::daemon::service::{
    DaemonAnswer, ServiceFacts, ServiceState, answer_from_hello,
};

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
    /// systemd to start the unit, attach. Once per dialog.
    SwitchOver,
    /// Keep waiting.
    Keep,
}

/// The not-installed and too-old dialogs watch for the service while open.
pub fn watch_step(open: Dialog, state: ServiceState, switched: bool) -> WatchStep {
    match (open, state) {
        (_, ServiceState::Running) => WatchStep::Close,
        (Dialog::Install, ServiceState::InstalledNotRunning) if !switched => WatchStep::SwitchOver,
        _ => WatchStep::Keep,
    }
}

/// How often an open dialog looks again.
pub const WATCH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// How long a switch-over waits for the unit to answer after `StartUnit`.
pub const SWITCH_WAIT: std::time::Duration = std::time::Duration::from_secs(20);

/// The systemd user manager's `StartUnit`, as one fixed call: destination,
/// object, interface, method, then the arguments. Only the channel's unit.
pub fn start_unit_call() -> (
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
        "StartUnit",
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

/// Ask the systemd user manager to start the unit. Blocking and bounded;
/// callers run it off the GTK thread. `Ok` means systemd queued the job, not
/// that the daemon came up: the probe says that.
pub fn start_unit() -> anyhow::Result<()> {
    bounded(|| {
        let (destination, path, interface, method, args) = start_unit_call();
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
            Ok(_) => Some(true),
            Err(zbus::Error::MethodError(..)) => Some(false),
            Err(_) => None,
        }
    })
    .flatten()
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

/// The daemon to ask `hello`: the one this window is attached to, else one
/// listening on the host link -- never this window's own daemon.
pub fn hello_target(
    hosting: bool,
    attached: Option<&Path>,
    link: LinkState,
    link_socket: Option<&Path>,
) -> Option<PathBuf> {
    if hosting {
        return None;
    }
    attached
        .or(link_socket.filter(|_| link == LinkState::Listening))
        .map(Path::to_path_buf)
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
    let answer = match hello_target(hosting, attached, link, link_socket.as_deref()) {
        Some(socket) => answer_from_hello(hello_at(&socket).as_ref()),
        None => DaemonAnswer::NoAnswer,
    };
    ServiceFacts {
        link,
        hosting,
        unit_installed,
        answer,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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

    #[test]
    fn an_open_dialog_closes_when_the_service_runs_and_switches_over_once() {
        for dialog in [Dialog::Install, Dialog::Start, Dialog::Update] {
            assert_eq!(
                watch_step(dialog, ServiceState::Running, false),
                WatchStep::Close
            );
            assert_eq!(
                watch_step(dialog, ServiceState::Running, true),
                WatchStep::Close
            );
        }
        assert_eq!(
            watch_step(Dialog::Install, ServiceState::InstalledNotRunning, false),
            WatchStep::SwitchOver
        );
        assert_eq!(
            watch_step(Dialog::Install, ServiceState::InstalledNotRunning, true),
            WatchStep::Keep
        );
        assert_eq!(
            watch_step(Dialog::Install, ServiceState::NotInstalled, false),
            WatchStep::Keep
        );
        assert_eq!(
            watch_step(Dialog::Update, ServiceState::TooOld, false),
            WatchStep::Keep
        );
    }

    #[test]
    fn first_setup_asks_on_linux_unless_a_headless_driver_runs() {
        assert!(first_setup_asks(true, false));
        assert!(!first_setup_asks(false, false));
        // The container smoke run expects the roots screen.
        assert!(!first_setup_asks(true, true));
    }

    /// Every way a dialog can come up, named. There is no app-start
    /// trigger: an ordinary start never shows one (owner, 2026-11-17). A new
    /// variant has to be added here, on purpose.
    #[test]
    fn no_trigger_is_an_ordinary_app_start() {
        for trigger in [Trigger::FirstSetup, Trigger::Launch, Trigger::SetUp] {
            let named = match trigger {
                Trigger::FirstSetup => "first setup, before the folders are declared",
                Trigger::Launch => "Launch pressed",
                Trigger::SetUp => "Set up pressed",
            };
            assert!(!named.contains("start of the app"));
        }
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
    fn start_unit_asks_for_the_channels_fixed_unit_only() {
        let (destination, path, interface, method, (unit, mode)) = start_unit_call();
        assert_eq!(destination, "org.freedesktop.systemd1");
        assert_eq!(path, "/org/freedesktop/systemd1");
        assert_eq!(interface, "org.freedesktop.systemd1.Manager");
        assert_eq!(method, "StartUnit");
        assert_eq!(unit, trace_commons_contributor::channel::SYSTEMD_UNIT_NAME);
        assert_eq!(mode, "replace");
    }

    #[test]
    fn hello_goes_to_the_attached_daemon_then_a_listening_link_never_our_own() {
        let attached = Path::new("/state/daemon.sock");
        let link = Path::new("/run/user/1/trace-commons/daemon.sock");
        assert_eq!(
            hello_target(false, Some(attached), LinkState::Listening, Some(link)),
            Some(attached.to_path_buf())
        );
        assert_eq!(
            hello_target(false, None, LinkState::Listening, Some(link)),
            Some(link.to_path_buf())
        );
        assert_eq!(hello_target(false, None, LinkState::Down, Some(link)), None);
        assert_eq!(
            hello_target(false, None, LinkState::Absent, Some(link)),
            None
        );
        // A hosting window's host link is its own daemon.
        assert_eq!(
            hello_target(true, Some(attached), LinkState::Listening, Some(link)),
            None
        );
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
