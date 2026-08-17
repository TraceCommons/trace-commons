//! Updating this application, on the one platform where the application is
//! forbidden to do it itself.
//!
//! A Flatpak-confined process must not replace its own bytes. Flatpak added
//! `org.freedesktop.portal.Flatpak.UpdateMonitor` for precisely this
//! reason -- "homegrown methods of doing so are unreliable at best, and
//! insecure at worst" -- and the portal is scoped so an application can
//! only ever update *itself*, nothing else on the system. So this module
//! asks; flatpak does the work.
//!
//! **Nothing here reads an update manifest.** The release pipeline's
//! `updates/latest.json` and `updates/appcast.xml` exist for the Windows/CLI
//! self-update path and for macOS Sparkle. The portal learns what version
//! exists from the flatpak remote the app was installed from, so there is
//! no fetch, no signature check and no sha256 check on this path -- ostree
//! and flatpak do that on the far side of the portal, where the bytes
//! actually are. Wiring a manifest in here would add a network dependency
//! that verifies nothing.
//!
//! Outside a flatpak -- a build from source -- there is no portal and no
//! remote, so this module reports [`InstallKind::Unconfined`] and the window
//! says so plainly. It never installs anything in that case, and never
//! checks anything either: there is nothing for a source build to check
//! against.

use std::path::Path;

use std::collections::HashMap;

use zbus::zvariant::{OwnedValue, Value};

/// Where this running copy came from, decided by a local path check and
/// nothing else. No network, in keeping with the design spec's rule that
/// install-source detection is a filesystem question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallKind {
    /// Running inside a Flatpak sandbox. The portal is the update path.
    Flatpak,
    /// Built from source and run directly. Nothing here updates it.
    Unconfined,
}

/// The file every Flatpak sandbox has and nothing outside one does. Flatpak
/// writes it into the sandbox root and it carries the instance's app id,
/// branch and commit -- which is also what the portal reads to decide what
/// this caller is allowed to update.
pub const FLATPAK_INFO_PATH: &str = "/.flatpak-info";

/// The detection, with the path injected so it is testable without a real
/// sandbox and without the test ever touching `/`.
pub fn detect_install_kind_at(flatpak_info: &Path) -> InstallKind {
    if flatpak_info.exists() {
        InstallKind::Flatpak
    } else {
        InstallKind::Unconfined
    }
}

/// The detection as production runs it.
pub fn detect_install_kind() -> InstallKind {
    detect_install_kind_at(Path::new(FLATPAK_INFO_PATH))
}

/// The `status` values the portal puts in a `Progress` signal, from
/// flatpak's own `UpdateStatus` enum in `portal/flatpak-portal.c`.
pub const PROGRESS_STATUS_RUNNING: u32 = 0;
/// The transaction turned out to have nothing in it.
pub const PROGRESS_STATUS_EMPTY: u32 = 1;
/// Installed. The new bytes are on disk; this process is still the old one.
pub const PROGRESS_STATUS_DONE: u32 = 2;
/// The transaction failed. The installed application is unchanged.
pub const PROGRESS_STATUS_ERROR: u32 = 3;

/// The one label an update failure is ever recorded or rendered under.
/// Repo convention: fixed labels, never the D-Bus error name or message,
/// which can carry more detail than a journal line should.
pub const FAILED_LABEL: &str = "flatpak-update-refused";

/// One portal signal, already lifted out of its `a{sv}` payload, plus the
/// one locally-generated event that is not a signal at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortalSignal {
    /// No monitor could be created: no session bus, no flatpak portal on
    /// it, a portal older than version 2, or a caller that is not a flatpak
    /// app. All of them mean the same thing to this application -- there is
    /// no update path here -- so they are one variant, not four.
    MonitorUnavailable,
    /// `org.freedesktop.portal.Flatpak.UpdateMonitor.UpdateAvailable`.
    UpdateAvailable {
        /// The commit this process is running.
        running_commit: String,
        /// The commit an ordinary restart would pick up.
        local_commit: String,
        /// The commit available from the remote.
        remote_commit: String,
    },
    /// `org.freedesktop.portal.Flatpak.UpdateMonitor.Progress`.
    Progress {
        /// How many operations the transaction has in total.
        n_ops: u32,
        /// Which one is running, zero-based.
        op: u32,
        /// 0-100 within the current operation, not across the transaction.
        progress: u32,
        /// One of the `PROGRESS_STATUS_*` constants above.
        status: u32,
        /// The portal's error *name*, kept only so the client layer can
        /// decide it saw a failure. It is never rendered and never logged.
        error: Option<String>,
    },
}

/// What the window currently knows about updating itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateState {
    /// Not running under flatpak -- a build from source. Nothing updates
    /// this copy and nothing here pretends to.
    Unmanaged,
    /// Under flatpak, but no monitor: no portal answered, or it is too old.
    Unavailable,
    /// Monitored, nothing offered.
    Idle,
    /// The portal has offered a newer commit. Waiting on a person.
    Available {
        /// Truncated by [`short_commit`] before it ever gets here.
        remote_commit: String,
    },
    /// The contributor said yes and the portal is working. Percent spans
    /// the whole transaction, not the current operation.
    Installing { percent: u32 },
    /// Installed. This process is still the old build; a restart finishes.
    Ready,
    /// The portal refused or failed. The installed app is unchanged.
    Failed { label: &'static str },
}

/// The leading 12 characters of an ostree commit, which is what a
/// contributor is shown. A full 64-character hash is not more informative
/// to a person and is harder to read back.
pub fn short_commit(commit: &str) -> String {
    commit.chars().take(12).collect()
}

/// Turn the portal's per-operation progress into one number for the whole
/// transaction. `op` is zero-based and `progress` is 0-100 *within* that
/// operation, so a four-operation update reports 0-100 four times; showing
/// that directly would look like it finished and restarted three times.
pub fn overall_percent(n_ops: u32, op: u32, progress: u32) -> u32 {
    if n_ops == 0 {
        return 0;
    }
    let op = u64::from(op.min(n_ops - 1));
    let progress = u64::from(progress.min(100));
    let percent = (op * 100 + progress) / u64::from(n_ops);
    percent.min(100) as u32
}

/// The whole state machine, as a pure function of the current state and one
/// signal.
///
/// Two transitions are load-bearing for the fail-closed rule and are the
/// reason this is a function rather than scattered `match` arms in a
/// callback:
///
/// * `Installing` is reachable only through [`begin_install`], never from a
///   signal. A `Progress` signal arriving in any other state is dropped.
/// * `Ready` -- the state that tells a contributor to restart into new
///   bytes -- is reachable only from `Installing`.
///
/// Together those mean no sequence of portal traffic can make this window
/// claim an install happened that a person did not confirm.
pub fn next_state(current: &UpdateState, signal: &PortalSignal) -> UpdateState {
    match signal {
        PortalSignal::MonitorUnavailable => UpdateState::Unavailable,
        PortalSignal::UpdateAvailable { remote_commit, .. } => match current {
            UpdateState::Installing { .. } => current.clone(),
            _ => UpdateState::Available {
                remote_commit: short_commit(remote_commit),
            },
        },
        PortalSignal::Progress {
            n_ops,
            op,
            progress,
            status,
            ..
        } => match *status {
            PROGRESS_STATUS_RUNNING => match current {
                UpdateState::Installing { .. } => UpdateState::Installing {
                    percent: overall_percent(*n_ops, *op, *progress),
                },
                _ => current.clone(),
            },
            PROGRESS_STATUS_EMPTY => UpdateState::Idle,
            PROGRESS_STATUS_DONE => match current {
                UpdateState::Installing { .. } => UpdateState::Ready,
                _ => current.clone(),
            },
            PROGRESS_STATUS_ERROR => UpdateState::Failed {
                label: FAILED_LABEL,
            },
            _ => current.clone(),
        },
    }
}

/// The one transition a person causes rather than the portal. Called when
/// the confirmation dialog is accepted, immediately before the `Update`
/// call goes out, so the window shows work starting rather than waiting for
/// the first `Progress` signal to arrive.
pub fn begin_install(current: &UpdateState) -> UpdateState {
    match current {
        UpdateState::Available { .. } => UpdateState::Installing { percent: 0 },
        _ => current.clone(),
    }
}

/// The flatpak portal. Note this is **not** `org.freedesktop.portal.Desktop`
/// -- that is xdg-desktop-portal, which `src/portal.rs` talks to for
/// Background. The update monitor lives on a separate service shipped by
/// flatpak itself and D-Bus-activated on demand.
pub const FLATPAK_PORTAL_BUS_NAME: &str = "org.freedesktop.portal.Flatpak";
/// The flatpak portal's own object.
pub const FLATPAK_PORTAL_OBJECT_PATH: &str = "/org/freedesktop/portal/Flatpak";
/// The interface carrying `CreateUpdateMonitor` and the `version` property.
pub const FLATPAK_PORTAL_INTERFACE: &str = "org.freedesktop.portal.Flatpak";
/// The interface on the object `CreateUpdateMonitor` hands back. Its path
/// is `/org/freedesktop/portal/Flatpak/update_monitor/SENDER/TOKEN` and is
/// never constructed here -- it is whatever the portal returned.
pub const UPDATE_MONITOR_INTERFACE: &str = "org.freedesktop.portal.Flatpak.UpdateMonitor";
/// The offer signal.
pub const SIGNAL_UPDATE_AVAILABLE: &str = "UpdateAvailable";
/// The install-progress signal.
pub const SIGNAL_PROGRESS: &str = "Progress";
/// `CreateUpdateMonitor` was added in version 2 of the flatpak portal
/// interface (flatpak 1.5.0). Current flatpak reports 8. An older portal
/// answers on the bus but has no such method, so the version is read first
/// rather than discovering it as an `UnknownMethod` error.
pub const MINIMUM_PORTAL_VERSION: u32 = 2;

/// Pull a string out of an `a{sv}`, or `None` if it is absent or not a
/// string. Matching the `Value` variant directly rather than going through
/// a `TryFrom` conversion keeps this independent of which zvariant
/// conversion impls are available in a given point release.
fn dict_str(dict: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    match dict.get(key).map(|value| &**value) {
        Some(Value::Str(text)) => Some(text.to_string()),
        _ => None,
    }
}

/// Pull a `u` out of an `a{sv}`, or `None` if it is absent or not a `u`.
fn dict_u32(dict: &HashMap<String, OwnedValue>, key: &str) -> Option<u32> {
    match dict.get(key).map(|value| &**value) {
        Some(Value::U32(number)) => Some(*number),
        _ => None,
    }
}

/// Turn one received signal into a [`PortalSignal`], or drop it.
///
/// Two fields are required and everything else has a default, and that
/// split is the fail-closed rule expressed in a parser: without a
/// `remote-commit` there is no update to offer, and without a `status` a
/// `Progress` signal cannot be classified -- and the wrong guess would be
/// "done".
pub fn parse_signal(name: &str, dict: &HashMap<String, OwnedValue>) -> Option<PortalSignal> {
    match name {
        SIGNAL_UPDATE_AVAILABLE => Some(PortalSignal::UpdateAvailable {
            running_commit: dict_str(dict, "running-commit").unwrap_or_default(),
            local_commit: dict_str(dict, "local-commit").unwrap_or_default(),
            remote_commit: dict_str(dict, "remote-commit")?,
        }),
        SIGNAL_PROGRESS => Some(PortalSignal::Progress {
            // The portal omits these on terminal signals; zero is the
            // honest reading of "no operation is in flight", and
            // `overall_percent` already treats n_ops == 0 as 0%.
            n_ops: dict_u32(dict, "n_ops").unwrap_or(0),
            op: dict_u32(dict, "op").unwrap_or(0),
            progress: dict_u32(dict, "progress").unwrap_or(0),
            status: dict_u32(dict, "status")?,
            error: dict_str(dict, "error"),
        }),
        _ => None,
    }
}

/// What the UI can ask the monitor thread to do. One variant today; an
/// enum rather than a unit type because `Close` is the obvious next one.
enum MonitorCommand {
    /// Call `UpdateMonitor.Update`. Only ever sent after a contributor has
    /// confirmed -- see `ui::update`.
    Install,
}

/// A live update monitor: signals coming out, install requests going in.
///
/// The D-Bus side runs on its own threads and talks to the GTK main loop
/// only through `async_channel`, exactly as `portal::spawn_request` does.
/// A portal round trip can include however long a permission dialog sits on
/// screen, and the window must never wait on that.
pub struct UpdateMonitor {
    /// Everything the portal said, plus a single `MonitorUnavailable` if it
    /// never said anything because there was nothing to talk to.
    pub signals: async_channel::Receiver<PortalSignal>,
    commands: std::sync::mpsc::Sender<MonitorCommand>,
}

impl UpdateMonitor {
    /// Ask the portal to install the offered update.
    ///
    /// Fire-and-forget: the answer arrives as `Progress` signals on
    /// `signals`, never as a return value. A send failure means the monitor
    /// thread is gone, which is already reported through the channel, so
    /// there is nothing further to say here.
    pub fn request_install(&self) {
        let _ = self.commands.send(MonitorCommand::Install);
    }
}

/// Create the monitor and start reading it.
///
/// Outside a flatpak this makes no D-Bus call at all: there is no portal to
/// call, and `CreateUpdateMonitor` would answer `NotSupported`. The caller
/// is expected to check [`detect_install_kind`] first and never reach here;
/// the check is repeated anyway so a future caller cannot make this module
/// talk to the flatpak portal from an unconfined process by accident.
pub fn spawn_monitor() -> UpdateMonitor {
    let (signal_tx, signal_rx) = async_channel::bounded(32);
    let (command_tx, command_rx) = std::sync::mpsc::channel::<MonitorCommand>();

    std::thread::spawn(move || {
        if detect_install_kind() != InstallKind::Flatpak {
            let _ = signal_tx.send_blocking(PortalSignal::MonitorUnavailable);
            return;
        }

        let (connection, monitor_path) = match create_monitor() {
            Ok(pair) => pair,
            Err(_) => {
                // Fixed label only. No session bus, no flatpak portal, a
                // portal older than version 2, and a portal that declined
                // all reach here, and none of them should look like a
                // crash -- never the D-Bus error text.
                eprintln!("trace-commons-shell: flatpak update portal unavailable");
                let _ = signal_tx.send_blocking(PortalSignal::MonitorUnavailable);
                return;
            }
        };

        // A shadow copy of `UpdateState`, kept only so the command loop
        // below can decide whether `Update` is worth calling at all. It is
        // driven by the same pure `next_state`/`begin_install` the UI's own
        // copy uses (see `ui::update`); this thread never renders it and
        // never sends it anywhere.
        let mirrored_state = std::sync::Arc::new(std::sync::Mutex::new(UpdateState::Idle));

        // One reader thread per signal name: a blocking signal iterator
        // owns its thread for as long as the monitor lives, and there are
        // two signals to read from the same object.
        for name in [SIGNAL_UPDATE_AVAILABLE, SIGNAL_PROGRESS] {
            let connection = connection.clone();
            let path = monitor_path.clone();
            let tx = signal_tx.clone();
            let mirrored_state = std::sync::Arc::clone(&mirrored_state);
            std::thread::spawn(move || {
                read_signals(&connection, &path, name, &tx, &mirrored_state)
            });
        }

        // Commands run on this thread, sharing the same connection. The
        // loop ends when the UI side is dropped, which is process exit.
        while command_rx.recv().is_ok() {
            let mut state = mirrored_state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(signal) =
                apply_install(&mut state, || call_update(&connection, &monitor_path))
            {
                let _ = signal_tx.send_blocking(signal);
            }
        }
    });

    UpdateMonitor {
        signals: signal_rx,
        commands: command_tx,
    }
}

/// Open the session bus, check the portal is new enough, and create the
/// monitor object.
fn create_monitor() -> anyhow::Result<(zbus::blocking::Connection, zbus::zvariant::OwnedObjectPath)>
{
    let connection = zbus::blocking::Connection::session()?;
    let portal = zbus::blocking::Proxy::new(
        &connection,
        FLATPAK_PORTAL_BUS_NAME,
        FLATPAK_PORTAL_OBJECT_PATH,
        FLATPAK_PORTAL_INTERFACE,
    )?;

    // Read the version before calling. An older portal is on the bus and
    // answers, so the alternative is discovering the gap as an
    // UnknownMethod error, which is indistinguishable from the portal not
    // being there at all.
    let version: u32 = portal.get_property("version")?;
    anyhow::ensure!(version >= MINIMUM_PORTAL_VERSION, "portal too old");

    let handle_token = format!("tracecommons{}", std::process::id());
    let mut options: HashMap<&str, Value> = HashMap::new();
    options.insert("handle_token", Value::from(handle_token.as_str()));
    let path: zbus::zvariant::OwnedObjectPath = portal.call("CreateUpdateMonitor", &(options,))?;

    Ok((connection, path))
}

/// Read one signal name off the monitor object until the channel closes or
/// the bus goes away. Runs on its own thread; the iterator blocks.
///
/// Every signal that reaches a contributor also updates `mirrored_state`
/// first, via the same [`next_state`] the UI uses -- so the command loop's
/// re-entrancy guard (see [`apply_install`]) sees a real `Installing` ->
/// `Ready`/`Failed`/`Idle` transition land, not just the one it makes
/// itself.
fn read_signals(
    connection: &zbus::blocking::Connection,
    path: &zbus::zvariant::OwnedObjectPath,
    name: &str,
    tx: &async_channel::Sender<PortalSignal>,
    mirrored_state: &std::sync::Mutex<UpdateState>,
) {
    let proxy = match zbus::blocking::Proxy::new(
        connection,
        FLATPAK_PORTAL_BUS_NAME,
        path,
        UPDATE_MONITOR_INTERFACE,
    ) {
        Ok(proxy) => proxy,
        Err(_) => {
            eprintln!("trace-commons-shell: flatpak update monitor unreadable");
            return;
        }
    };
    let stream = match proxy.receive_signal(name) {
        Ok(stream) => stream,
        Err(_) => {
            eprintln!("trace-commons-shell: flatpak update monitor unreadable");
            return;
        }
    };

    for message in stream {
        let body = message.body();
        let Ok((dict,)) = body.deserialize::<(HashMap<String, OwnedValue>,)>() else {
            // A payload this application cannot read is dropped, never
            // guessed at.
            continue;
        };
        let Some(signal) = parse_signal(name, &dict) else {
            continue;
        };
        {
            let mut state = mirrored_state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *state = next_state(&state, &signal);
        }
        if tx.send_blocking(signal).is_err() {
            return;
        }
    }
}

/// Call `UpdateMonitor.Update`. Returns as soon as the portal accepts the
/// request; the work is reported through `Progress`.
///
/// `parent_window` is empty: this application has no way to produce an X11
/// or Wayland window handle string for the portal, and the portal treats an
/// empty handle as "parent it yourself", which is the correct behaviour
/// rather than a degraded one.
fn call_update(
    connection: &zbus::blocking::Connection,
    path: &zbus::zvariant::OwnedObjectPath,
) -> anyhow::Result<()> {
    let proxy = zbus::blocking::Proxy::new(
        connection,
        FLATPAK_PORTAL_BUS_NAME,
        path,
        UPDATE_MONITOR_INTERFACE,
    )?;
    let options: HashMap<&str, Value> = HashMap::new();
    let _: () = proxy.call("Update", &("", options))?;
    Ok(())
}

/// Apply one `Install` request to the shadow `UpdateState` the command loop
/// keeps, calling `update` only when doing so is meaningful, and return the
/// synthesized failure signal to forward to the UI if it was and `update`
/// failed.
///
/// This is the whole re-entrancy guard, and it is deliberately not built on
/// the portal's own error semantics: matching on
/// `org.freedesktop.DBus.Error.Failed` would also swallow genuine failures
/// that share that generic name, and matching on the portal's "Already
/// installing" message text would mean reasoning about a string this
/// codebase otherwise refuses to log or interpret. Instead this reuses
/// [`begin_install`], the same pure function [`ui::update`] uses to gate the
/// confirmation button: a request is meaningful exactly when `begin_install`
/// would actually move `state`, which is true only from `Available`. A
/// second request arriving while already `Installing` -- or from any other
/// state -- leaves `state` unchanged and never calls `update` at all, which
/// makes the double call unrepresentable rather than merely handled.
///
/// `update` is injected so this can be exercised without a live portal: a
/// closure stands in for [`call_update`], and a test can assert both the
/// resulting state and whether the closure ran.
fn apply_install(
    state: &mut UpdateState,
    update: impl FnOnce() -> anyhow::Result<()>,
) -> Option<PortalSignal> {
    let next = begin_install(state);
    if next == *state {
        // Not meaningful right now -- most notably a re-entrant request
        // while an install is already in flight.
        return None;
    }
    *state = next;

    match update() {
        Ok(()) => None,
        Err(_) => {
            eprintln!("trace-commons-shell: flatpak update portal refused the install");
            // Synthesised so the window leaves `Installing` instead of
            // showing a progress bar that will never move again. It is
            // shaped exactly like the portal's own terminal error so the
            // state machine has one path, not two.
            let signal = PortalSignal::Progress {
                n_ops: 0,
                op: 0,
                progress: 0,
                status: PROGRESS_STATUS_ERROR,
                error: None,
            };
            *state = next_state(state, &signal);
            Some(signal)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A private directory for one test, under the system temp dir, named
    /// so two tests in the same process never collide. Deliberately not
    /// `tempfile`: that would be a new dependency for four lines.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("tc-update-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_present_flatpak_info_means_confined() {
        let dir = scratch("present");
        let marker = dir.join("flatpak-info");
        std::fs::write(
            &marker,
            b"[Application]\nname=ai.tracecommons.Contributor\n",
        )
        .unwrap();
        assert_eq!(detect_install_kind_at(&marker), InstallKind::Flatpak);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_absent_flatpak_info_means_a_source_build() {
        let dir = scratch("absent");
        let marker = dir.join("flatpak-info");
        assert_eq!(detect_install_kind_at(&marker), InstallKind::Unconfined);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_production_path_is_the_sandbox_root_marker() {
        // The constant is the contract with the flatpak runtime; a typo in
        // it would make every confined run silently report Unconfined and
        // no other test would notice.
        assert_eq!(FLATPAK_INFO_PATH, "/.flatpak-info");
    }

    fn update_available(remote: &str) -> PortalSignal {
        PortalSignal::UpdateAvailable {
            running_commit: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            local_commit: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            remote_commit: remote.to_string(),
        }
    }

    fn progress(status: u32, n_ops: u32, op: u32, percent: u32) -> PortalSignal {
        PortalSignal::Progress {
            n_ops,
            op,
            progress: percent,
            status,
            error: None,
        }
    }

    #[test]
    fn a_commit_is_shown_truncated_and_never_whole() {
        assert_eq!(
            short_commit("0123456789abcdef0123456789abcdef"),
            "0123456789ab"
        );
        // Shorter than the window is returned as-is rather than padded.
        assert_eq!(short_commit("abc"), "abc");
        assert_eq!(short_commit(""), "");
    }

    #[test]
    fn overall_percent_spans_every_operation_not_just_the_current_one() {
        // The portal reports `op` 0-based within `n_ops` operations, and
        // `progress` 0-100 within the current operation. Rendering the
        // per-operation number alone would make a four-operation update
        // appear to finish four times.
        assert_eq!(overall_percent(1, 0, 50), 50);
        assert_eq!(overall_percent(4, 0, 0), 0);
        assert_eq!(overall_percent(4, 2, 0), 50);
        assert_eq!(overall_percent(4, 3, 100), 100);
    }

    #[test]
    fn overall_percent_never_divides_by_zero_or_exceeds_one_hundred() {
        assert_eq!(overall_percent(0, 0, 0), 0);
        assert_eq!(overall_percent(0, 7, 90), 0);
        // A portal that reported nonsense must still produce a drawable bar.
        assert_eq!(overall_percent(2, 9, 200), 100);
    }

    #[test]
    fn an_offer_moves_an_idle_app_to_available() {
        let state = next_state(
            &UpdateState::Idle,
            &update_available("beefbeefbeefbeefbeefbeefbeefbeef"),
        );
        assert_eq!(
            state,
            UpdateState::Available {
                remote_commit: "beefbeefbeef".to_string()
            }
        );
    }

    #[test]
    fn a_repeat_offer_does_not_restart_an_install_already_in_flight() {
        // The portal re-checks every 30 minutes and will re-announce the
        // same commit while it is being fetched. Resetting to Available
        // would replace a progress bar with a confirmation button for an
        // install the contributor already confirmed.
        let installing = UpdateState::Installing { percent: 40 };
        assert_eq!(
            next_state(&installing, &update_available("beefbeefbeefbeef")),
            installing
        );
    }

    #[test]
    fn nothing_reaches_installing_without_the_contributor_confirming() {
        // Fail closed: a Running progress signal arriving in any state this
        // application did not put itself into is dropped, not rendered. A
        // window that showed "installing" for something nobody agreed to
        // would be lying about what is happening to the machine.
        for state in [
            UpdateState::Idle,
            UpdateState::Unmanaged,
            UpdateState::Unavailable,
            UpdateState::Available {
                remote_commit: "beefbeefbeef".to_string(),
            },
        ] {
            assert_eq!(
                next_state(&state, &progress(PROGRESS_STATUS_RUNNING, 2, 0, 50)),
                state,
                "{state:?} must not be advanced by an unsolicited progress signal"
            );
        }
    }

    #[test]
    fn progress_is_rendered_once_an_install_is_underway() {
        assert_eq!(
            next_state(
                &UpdateState::Installing { percent: 0 },
                &progress(PROGRESS_STATUS_RUNNING, 2, 1, 50)
            ),
            UpdateState::Installing { percent: 75 }
        );
    }

    #[test]
    fn done_is_only_ever_reached_from_installing() {
        // The other half of fail-closed: "restart to finish" must never be
        // shown for an install that never ran here.
        assert_eq!(
            next_state(
                &UpdateState::Installing { percent: 90 },
                &progress(PROGRESS_STATUS_DONE, 1, 0, 100)
            ),
            UpdateState::Ready
        );
        assert_eq!(
            next_state(
                &UpdateState::Idle,
                &progress(PROGRESS_STATUS_DONE, 1, 0, 100)
            ),
            UpdateState::Idle
        );
    }

    #[test]
    fn an_empty_transaction_returns_the_app_to_idle() {
        // status 1 means the portal found nothing to do after all.
        assert_eq!(
            next_state(
                &UpdateState::Installing { percent: 10 },
                &progress(PROGRESS_STATUS_EMPTY, 0, 0, 0)
            ),
            UpdateState::Idle
        );
    }

    #[test]
    fn a_portal_error_becomes_a_fixed_label_and_never_the_portal_text() {
        let state = next_state(
            &UpdateState::Installing { percent: 10 },
            &PortalSignal::Progress {
                n_ops: 1,
                op: 0,
                progress: 10,
                status: PROGRESS_STATUS_ERROR,
                error: Some("org.freedesktop.DBus.Error.AccessDenied".to_string()),
            },
        );
        assert_eq!(
            state,
            UpdateState::Failed {
                label: FAILED_LABEL
            }
        );
        // Whatever the portal said is not carried into anything renderable.
        assert_eq!(FAILED_LABEL, "flatpak-update-refused");
    }

    #[test]
    fn losing_the_monitor_is_its_own_state_and_overrides_any_other() {
        for state in [
            UpdateState::Idle,
            UpdateState::Installing { percent: 50 },
            UpdateState::Ready,
        ] {
            assert_eq!(
                next_state(&state, &PortalSignal::MonitorUnavailable),
                UpdateState::Unavailable
            );
        }
    }

    #[test]
    fn an_install_can_only_begin_from_an_offer() {
        assert_eq!(
            begin_install(&UpdateState::Available {
                remote_commit: "beefbeefbeef".to_string()
            }),
            UpdateState::Installing { percent: 0 }
        );
        for state in [
            UpdateState::Idle,
            UpdateState::Unmanaged,
            UpdateState::Unavailable,
            UpdateState::Ready,
            UpdateState::Failed {
                label: FAILED_LABEL,
            },
        ] {
            assert_eq!(
                begin_install(&state),
                state,
                "{state:?} is not an offer and must not start an install"
            );
        }
    }

    #[test]
    fn a_reentrant_install_request_while_installing_calls_nothing_and_stays_installing() {
        // The bug this guards against: a contributor who double-clicks
        // Update (or a portal re-announcing the same offer mid-install --
        // see `a_repeat_offer_does_not_restart_an_install_already_in_flight`)
        // must not turn a perfectly healthy in-flight install into a
        // synthesized Failed state.
        let mut state = UpdateState::Installing { percent: 40 };
        let mut calls = 0;
        let signal = apply_install(&mut state, || {
            calls += 1;
            Ok(())
        });
        assert_eq!(calls, 0, "Update must not be called a second time");
        assert_eq!(signal, None, "a no-op must never synthesize a failure");
        assert_eq!(
            state,
            UpdateState::Installing { percent: 40 },
            "a dropped re-entrant request must leave the state untouched"
        );
    }

    #[test]
    fn an_install_request_only_calls_the_portal_from_an_offer() {
        // Same guard, the rest of the state space: `apply_install` must
        // never call `update` from anywhere `begin_install` itself refuses
        // to leave.
        for state in [
            UpdateState::Idle,
            UpdateState::Unmanaged,
            UpdateState::Unavailable,
            UpdateState::Ready,
            UpdateState::Failed {
                label: FAILED_LABEL,
            },
        ] {
            let mut state = state;
            let before = state.clone();
            let mut calls = 0;
            let signal = apply_install(&mut state, || {
                calls += 1;
                Ok(())
            });
            assert_eq!(calls, 0, "{before:?} must not call Update");
            assert_eq!(signal, None);
            assert_eq!(state, before, "{before:?} must be left untouched");
        }
    }

    #[test]
    fn an_install_request_from_an_offer_calls_the_portal_exactly_once_and_moves_to_installing() {
        let mut state = UpdateState::Available {
            remote_commit: "beefbeefbeef".to_string(),
        };
        let mut calls = 0;
        let signal = apply_install(&mut state, || {
            calls += 1;
            Ok(())
        });
        assert_eq!(calls, 1);
        assert_eq!(signal, None);
        assert_eq!(state, UpdateState::Installing { percent: 0 });
    }

    #[test]
    fn a_failed_portal_call_still_only_happens_once_and_reports_the_fixed_label() {
        // The failure path this guard leaves untouched: a *first* call that
        // genuinely fails still becomes the fixed-label Failed state, same
        // as before this change -- the guard only ever prevents a *second*
        // call, never masks a real one failing.
        let mut state = UpdateState::Available {
            remote_commit: "beefbeefbeef".to_string(),
        };
        let mut calls = 0;
        let signal = apply_install(&mut state, || {
            calls += 1;
            Err(anyhow::anyhow!("org.freedesktop.DBus.Error.Failed"))
        });
        assert_eq!(calls, 1);
        assert_eq!(
            signal,
            Some(PortalSignal::Progress {
                n_ops: 0,
                op: 0,
                progress: 0,
                status: PROGRESS_STATUS_ERROR,
                error: None,
            })
        );
        assert_eq!(
            state,
            UpdateState::Failed {
                label: FAILED_LABEL
            }
        );
    }

    use std::collections::HashMap;
    use zbus::zvariant::{OwnedValue, Value};

    /// A real `a{sv}` payload, built the way the portal builds one, so
    /// parsing is exercised through the actual zvariant types rather than a
    /// stand-in for them.
    fn vardict(pairs: Vec<(&str, Value<'static>)>) -> HashMap<String, OwnedValue> {
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_string(), OwnedValue::try_from(value).unwrap()))
            .collect()
    }

    #[test]
    fn an_update_available_payload_parses_into_its_three_commits() {
        let dict = vardict(vec![
            ("running-commit", Value::from("1111111111111111")),
            ("local-commit", Value::from("2222222222222222")),
            ("remote-commit", Value::from("3333333333333333")),
        ]);
        assert_eq!(
            parse_signal(SIGNAL_UPDATE_AVAILABLE, &dict),
            Some(PortalSignal::UpdateAvailable {
                running_commit: "1111111111111111".to_string(),
                local_commit: "2222222222222222".to_string(),
                remote_commit: "3333333333333333".to_string(),
            })
        );
    }

    #[test]
    fn an_update_available_payload_missing_the_remote_commit_is_dropped() {
        // Fail closed: without a remote commit there is nothing to offer,
        // and offering an update whose target is unknown is worse than
        // staying quiet.
        let dict = vardict(vec![
            ("running-commit", Value::from("1111111111111111")),
            ("local-commit", Value::from("2222222222222222")),
        ]);
        assert_eq!(parse_signal(SIGNAL_UPDATE_AVAILABLE, &dict), None);
    }

    #[test]
    fn a_progress_payload_parses_every_numeric_field() {
        let dict = vardict(vec![
            ("n_ops", Value::from(4u32)),
            ("op", Value::from(1u32)),
            ("progress", Value::from(50u32)),
            ("status", Value::from(PROGRESS_STATUS_RUNNING)),
        ]);
        assert_eq!(
            parse_signal(SIGNAL_PROGRESS, &dict),
            Some(PortalSignal::Progress {
                n_ops: 4,
                op: 1,
                progress: 50,
                status: PROGRESS_STATUS_RUNNING,
                error: None,
            })
        );
    }

    #[test]
    fn a_progress_payload_without_a_status_is_dropped() {
        // The portal omits op/n_ops/progress on some terminal signals, but
        // never status. A payload with no status could not be classified as
        // anything, and defaulting it would risk defaulting it to Done.
        let dict = vardict(vec![("n_ops", Value::from(1u32))]);
        assert_eq!(parse_signal(SIGNAL_PROGRESS, &dict), None);
    }

    #[test]
    fn a_terminal_error_payload_parses_with_its_counters_absent() {
        let dict = vardict(vec![
            ("status", Value::from(PROGRESS_STATUS_ERROR)),
            (
                "error",
                Value::from("org.freedesktop.DBus.Error.AccessDenied"),
            ),
            ("error_message", Value::from("nope")),
        ]);
        assert_eq!(
            parse_signal(SIGNAL_PROGRESS, &dict),
            Some(PortalSignal::Progress {
                n_ops: 0,
                op: 0,
                progress: 0,
                status: PROGRESS_STATUS_ERROR,
                error: Some("org.freedesktop.DBus.Error.AccessDenied".to_string()),
            })
        );
    }

    #[test]
    fn a_signal_this_application_did_not_subscribe_to_is_ignored() {
        let dict = vardict(vec![("status", Value::from(PROGRESS_STATUS_DONE))]);
        assert_eq!(parse_signal("SomethingElse", &dict), None);
    }

    #[test]
    fn the_portal_addresses_are_the_flatpak_portal_not_the_desktop_portal() {
        // These are a different service from org.freedesktop.portal.Desktop,
        // which is what src/portal.rs talks to. Getting them confused
        // produces an UnknownMethod at runtime and nothing at compile time.
        assert_eq!(FLATPAK_PORTAL_BUS_NAME, "org.freedesktop.portal.Flatpak");
        assert_eq!(
            FLATPAK_PORTAL_OBJECT_PATH,
            "/org/freedesktop/portal/Flatpak"
        );
        assert_eq!(
            UPDATE_MONITOR_INTERFACE,
            "org.freedesktop.portal.Flatpak.UpdateMonitor"
        );
        // CreateUpdateMonitor landed in interface version 2 (flatpak 1.5.0).
        assert_eq!(MINIMUM_PORTAL_VERSION, 2);
    }
}
