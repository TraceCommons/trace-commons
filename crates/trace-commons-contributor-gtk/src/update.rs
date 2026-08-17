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
}
