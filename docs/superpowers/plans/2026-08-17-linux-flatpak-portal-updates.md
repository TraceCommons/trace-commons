# Linux Flatpak Portal Updates Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The GTK contributor shell detects that it is running under Flatpak, asks `org.freedesktop.portal.Flatpak.UpdateMonitor` to watch for a newer build of itself, and — only after a contributor says yes — asks the portal to install it and shows progress until a restart finishes the job.

**Architecture:** One new module, `src/update.rs`, holds three separable layers: a local-path install-source check (`/.flatpak-info`), a pure state machine over the portal's two signals, and a threaded `zbus::blocking` client that owns the D-Bus connection. One new UI module, `src/ui/update.rs`, holds a pure state-to-copy presentation function plus the libadwaita banner and dialog that render it. The application never replaces its own bytes; flatpak does the replacement and the app only asks. The threading shape (own thread, blocking zbus, `async_channel` back to the GTK main loop) is copied verbatim from the existing `src/portal.rs`, which does the same thing for `org.freedesktop.portal.Background`.

**Tech Stack:** Rust edition 2024, GTK 4 (`gtk4` 0.7), libadwaita (`libadwaita` 0.5, `v1_2`), `zbus` 5 (blocking API), `async-channel` 2, `anyhow` 1. Flatpak runtime `org.gnome.Platform//46`.

---

## Dependency decision: none required

**No new dependency is needed and none is proposed.** `zbus = "5"` is already a
*direct* dependency in `crates/trace-commons-contributor-gtk/Cargo.toml`, added
for the Background portal and the StatusNotifierItem tray, and
`src/portal.rs` already calls a portal through `zbus::blocking`. This plan adds
D-Bus calls only; it touches neither `Cargo.toml` nor `Cargo.lock`.

Two consequences worth stating, because they remove work other people might
otherwise plan for:

- `crates/trace-commons-contributor-gtk/flatpak/cargo-sources.json` does **not**
  need regenerating. It is derived from `Cargo.lock`, and `Cargo.lock` does not
  change. The network-sandboxed flatpak build therefore keeps working untouched.
- `ashpd` (the usual Rust portal wrapper) is deliberately **not** used. It would
  be a new direct dependency requiring a written workup and explicit approval,
  and it would buy nothing here: `ashpd::flatpak::UpdateMonitor` wraps exactly
  the four D-Bus members this plan calls directly.

## This subsystem consumes no update manifest

`updates/latest.json` and `updates/appcast.xml` from the design spec are for the
Windows/CLI self-update path and for macOS Sparkle respectively. **Neither is
read here, and nothing in this plan should be wired to either.** The Flatpak
portal learns what version exists by consulting the flatpak remote the app was
installed from — the app never fetches, never parses, and never verifies a
manifest, because it never obtains the bytes. Verification is ostree's and
flatpak's, on the far side of the portal. Any task that adds an HTTP fetch to
this subsystem is wrong.

## Recorded deviation from the brief

The brief asks that a non-flatpak (built-from-source) run degrade to "a
check-and-notify banner that installs nothing". The *notify* half is
implemented; the *check* half is not, because this subsystem deliberately has no
manifest to check against (see above) and a source build has no remote to
consult. The unconfined state therefore renders an honest, dismissible line
saying updates are not managed for a source build, and performs no network
access at all. Adding a version check for source builds would mean introducing
manifest fetching into the one platform path the design spec explicitly exempts
from it.

## Global Constraints

- **Separate cargo workspace.** `crates/trace-commons-contributor-gtk` declares its own `[workspace]` and is excluded from the root one. Never verify it with `-p` from the repository root. Use `--manifest-path crates/trace-commons-contributor-gtk/Cargo.toml`, or `scripts/linux-build.sh <command>`, which runs inside the crate directory in a container.
- **Does not build on macOS.** The crate links GTK 4 and libadwaita. Every compile or test command in this plan runs through `scripts/linux-build.sh` (or the `linux-build.Dockerfile` image directly).
- **Rust edition 2024; `rust-version = "1.92"`.** Do not raise the floor. The flatpak manifest pins rust `1.92.0` as a build source, and `release-apps.yml`'s "Check the manifest's pinned rust against the crate's requirement" step fails the release if the pin drops below `rust-version`. If you change one you must change both.
- **The flatpak build is network-sandboxed.** Nothing may be fetched at build time. This is why `Cargo.lock` must not change.
- **Fail closed.** The application never replaces its own bytes inside the sandbox. `UpdateState::Ready` is reachable only from `UpdateState::Installing`, which is reachable only through `begin_install`, which is reachable only from `UpdateState::Available`. A stray or malformed portal signal must never advance the state past a confirmation the contributor did not give.
- **Hash-only logging.** Log fixed labels, never D-Bus error text, never a portal error message. Commit ids may be shown to a contributor truncated to 12 characters via `short_commit`; nothing else about the update is logged.
- **No emojis** in commits, PRs, code, or comments. Commit subjects are short and imperative, with no `feat:` / `fix:` prefix.
- **Copy rules** (from `src/copy.rs`): never name an internal mechanism; always state the data consequence ("nothing in your queue is touched"). "Flatpak" is the contributor's own package manager and is fine to name.

## Confirmed D-Bus interface

Verified against `flatpak/data/org.freedesktop.portal.Flatpak.xml` and
`flatpak/portal/flatpak-portal.c` on `main`, not from memory:

| Thing | Value |
|---|---|
| Bus name | `org.freedesktop.portal.Flatpak` (the flatpak portal — a *different* service from `org.freedesktop.portal.Desktop`) |
| Object path | `/org/freedesktop/portal/Flatpak` |
| Interface | `org.freedesktop.portal.Flatpak` |
| Interface `version` property | currently `8`; `CreateUpdateMonitor` was added in version **2** (flatpak 1.5.0) |
| `CreateUpdateMonitor` | `(IN a{sv} options, OUT o handle)`; `options` accepts `handle_token` (`s`) |
| Monitor object path | `/org/freedesktop/portal/Flatpak/update_monitor/SENDER/TOKEN` |
| Monitor interface | `org.freedesktop.portal.Flatpak.UpdateMonitor` |
| `Update` | `(IN s parent_window, IN a{sv} options)`; returns immediately, work continues asynchronously. `options` is accepted and ignored by the portal. A second call while installing returns `org.freedesktop.DBus.Error.Failed` "Already installing". |
| `Close` | `()` |
| `UpdateAvailable` signal | `(a{sv} update_info)`; keys `running-commit` (`s`), `local-commit` (`s`), `remote-commit` (`s`) |
| `Progress` signal | `(a{sv} info)`; keys `n_ops` (`u`), `op` (`u`, **0-based**), `progress` (`u`, 0–100 within the current op), `status` (`u`), `error` (`s`), `error_message` (`s`) |
| `status` values | `0` Running, `1` Empty, `2` Done, `3` Error |
| Signal delivery | **unicast** — `flatpak-portal.c` emits with destination `m->sender`, so these are directed messages, not broadcasts |
| Poll interval | the portal re-checks every 30 minutes by default (`DEFAULT_UPDATE_POLL_TIMEOUT_SEC`) |
| Error when not a flatpak | `CreateUpdateMonitor` returns `org.freedesktop.DBus.Error.NotSupported` "Updates only supported by flatpak apps" |

---

### Task 1: Install-source detection

**Files:**
- Create: `crates/trace-commons-contributor-gtk/src/update.rs`
- Modify: `crates/trace-commons-contributor-gtk/src/lib.rs` (add `pub mod update;` to the module list, alphabetically after `pub mod tray;`)

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub enum InstallKind { Flatpak, Unconfined }` — derives `Debug, Clone, Copy, PartialEq, Eq`
  - `pub const FLATPAK_INFO_PATH: &str = "/.flatpak-info"`
  - `pub fn detect_install_kind_at(flatpak_info: &std::path::Path) -> InstallKind`
  - `pub fn detect_install_kind() -> InstallKind`

- [ ] **Step 1: Create the module with its header and the failing test**

Create `crates/trace-commons-contributor-gtk/src/update.rs` with exactly this content:

```rust
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
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `scripts/linux-build.sh cargo test update::tests`

Expected: FAIL to compile, with `error[E0583]: file not found for module` or `unresolved module` — `src/update.rs` exists but is not declared in `lib.rs`, so its tests are not part of the crate and none of them run. The reported test count is `0 tests`, which is the failure.

- [ ] **Step 3: Declare the module**

In `crates/trace-commons-contributor-gtk/src/lib.rs`, add the module declaration so the list reads:

```rust
pub mod autostart;
pub mod backend;
pub mod copy;
pub mod model;
pub mod notify;
pub mod portal;
pub mod tray;
pub mod ui;
pub mod update;
pub mod worker;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `scripts/linux-build.sh cargo test update::tests`

Expected: PASS — `test result: ok. 3 passed`.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor-gtk/src/update.rs crates/trace-commons-contributor-gtk/src/lib.rs
git commit -m "Detect whether the Linux shell is running under flatpak"
```

---

### Task 2: The update state machine

**Files:**
- Modify: `crates/trace-commons-contributor-gtk/src/update.rs`

**Interfaces:**
- Consumes: `InstallKind` from Task 1 (same file).
- Produces:
  - `pub const PROGRESS_STATUS_RUNNING: u32 = 0` / `PROGRESS_STATUS_EMPTY: u32 = 1` / `PROGRESS_STATUS_DONE: u32 = 2` / `PROGRESS_STATUS_ERROR: u32 = 3`
  - `pub const FAILED_LABEL: &str = "flatpak-update-refused"`
  - `pub enum PortalSignal { MonitorUnavailable, UpdateAvailable { running_commit: String, local_commit: String, remote_commit: String }, Progress { n_ops: u32, op: u32, progress: u32, status: u32, error: Option<String> } }` — derives `Debug, Clone, PartialEq, Eq`
  - `pub enum UpdateState { Unmanaged, Unavailable, Idle, Available { remote_commit: String }, Installing { percent: u32 }, Ready, Failed { label: &'static str } }` — derives `Debug, Clone, PartialEq, Eq`
  - `pub fn short_commit(commit: &str) -> String`
  - `pub fn overall_percent(n_ops: u32, op: u32, progress: u32) -> u32`
  - `pub fn next_state(current: &UpdateState, signal: &PortalSignal) -> UpdateState`
  - `pub fn begin_install(current: &UpdateState) -> UpdateState`

- [ ] **Step 1: Write the failing tests**

Append to the `mod tests` block in `crates/trace-commons-contributor-gtk/src/update.rs`, inside the existing braces:

```rust
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
            next_state(&UpdateState::Installing { percent: 90 }, &progress(PROGRESS_STATUS_DONE, 1, 0, 100)),
            UpdateState::Ready
        );
        assert_eq!(
            next_state(&UpdateState::Idle, &progress(PROGRESS_STATUS_DONE, 1, 0, 100)),
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `scripts/linux-build.sh cargo test update::tests`

Expected: FAIL to compile with `error[E0433]: failed to resolve: use of undeclared type PortalSignal` and `cannot find function short_commit in this scope`, `... overall_percent ...`, `... next_state ...`, `... begin_install ...`.

- [ ] **Step 3: Write the implementation**

Insert into `crates/trace-commons-contributor-gtk/src/update.rs`, after `detect_install_kind` and before `#[cfg(test)] mod tests`:

```rust
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
            PROGRESS_STATUS_ERROR => UpdateState::Failed { label: FAILED_LABEL },
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
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `scripts/linux-build.sh cargo test update::tests`

Expected: PASS — `test result: ok. 14 passed`.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor-gtk/src/update.rs
git commit -m "Add the flatpak update state machine"
```

---

### Task 3: Parse the portal's signal payloads

**Files:**
- Modify: `crates/trace-commons-contributor-gtk/src/update.rs`

**Interfaces:**
- Consumes: `PortalSignal`, `PROGRESS_STATUS_*` from Task 2 (same file).
- Produces:
  - `pub const FLATPAK_PORTAL_BUS_NAME: &str = "org.freedesktop.portal.Flatpak"`
  - `pub const FLATPAK_PORTAL_OBJECT_PATH: &str = "/org/freedesktop/portal/Flatpak"`
  - `pub const FLATPAK_PORTAL_INTERFACE: &str = "org.freedesktop.portal.Flatpak"`
  - `pub const UPDATE_MONITOR_INTERFACE: &str = "org.freedesktop.portal.Flatpak.UpdateMonitor"`
  - `pub const SIGNAL_UPDATE_AVAILABLE: &str = "UpdateAvailable"`
  - `pub const SIGNAL_PROGRESS: &str = "Progress"`
  - `pub const MINIMUM_PORTAL_VERSION: u32 = 2`
  - `pub fn parse_signal(name: &str, dict: &std::collections::HashMap<String, zbus::zvariant::OwnedValue>) -> Option<PortalSignal>`

- [ ] **Step 1: Write the failing tests**

Append inside the existing `mod tests` block in `crates/trace-commons-contributor-gtk/src/update.rs`:

```rust
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
            ("error", Value::from("org.freedesktop.DBus.Error.AccessDenied")),
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
        assert_eq!(FLATPAK_PORTAL_OBJECT_PATH, "/org/freedesktop/portal/Flatpak");
        assert_eq!(
            UPDATE_MONITOR_INTERFACE,
            "org.freedesktop.portal.Flatpak.UpdateMonitor"
        );
        // CreateUpdateMonitor landed in interface version 2 (flatpak 1.5.0).
        assert_eq!(MINIMUM_PORTAL_VERSION, 2);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `scripts/linux-build.sh cargo test update::tests`

Expected: FAIL to compile with `error[E0425]: cannot find function parse_signal in this scope` and `cannot find value SIGNAL_UPDATE_AVAILABLE in this scope`.

- [ ] **Step 3: Write the implementation**

Add this `use` line at the top of `crates/trace-commons-contributor-gtk/src/update.rs`, directly under `use std::path::Path;`:

```rust
use std::collections::HashMap;

use zbus::zvariant::{OwnedValue, Value};
```

Then insert this block after `begin_install` and before `#[cfg(test)] mod tests`:

```rust
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
```

Note on the required-commit test: `parse_signal` uses `unwrap_or_default()` for
`running-commit` and `local-commit` because the state machine never reads them,
and `?` for `remote-commit` because it does.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `scripts/linux-build.sh cargo test update::tests`

Expected: PASS — `test result: ok. 21 passed`.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor-gtk/src/update.rs
git commit -m "Parse the flatpak update portal's signal payloads"
```

---

### Task 4: The live portal client

**Files:**
- Modify: `crates/trace-commons-contributor-gtk/src/update.rs`

**Interfaces:**
- Consumes: `PortalSignal`, `parse_signal`, `FLATPAK_PORTAL_*`, `UPDATE_MONITOR_INTERFACE`, `SIGNAL_*`, `MINIMUM_PORTAL_VERSION`, `PROGRESS_STATUS_ERROR`, `detect_install_kind`, `InstallKind` (all Tasks 1–3, same file).
- Produces:
  - `pub struct UpdateMonitor { pub signals: async_channel::Receiver<PortalSignal>, commands: std::sync::mpsc::Sender<MonitorCommand> }`
  - `impl UpdateMonitor { pub fn request_install(&self) }`
  - `pub fn spawn_monitor() -> UpdateMonitor`

- [ ] **Step 1: Write the implementation**

This task has no unit-test cycle: every line of it needs a live session bus with
a running flatpak portal and a real flatpak installation, which no test in this
crate can provide (see the manual verification in Step 3 and the live procedure
in Task 7). The testable parts — detection, the state machine, and payload
parsing — were separated out into Tasks 1–3 precisely so this layer contains
nothing but plumbing.

Insert into `crates/trace-commons-contributor-gtk/src/update.rs`, after
`parse_signal` and before `#[cfg(test)] mod tests`:

```rust
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

        // One reader thread per signal name: a blocking signal iterator
        // owns its thread for as long as the monitor lives, and there are
        // two signals to read from the same object.
        for name in [SIGNAL_UPDATE_AVAILABLE, SIGNAL_PROGRESS] {
            let connection = connection.clone();
            let path = monitor_path.clone();
            let tx = signal_tx.clone();
            std::thread::spawn(move || read_signals(&connection, &path, name, &tx));
        }

        // Commands run on this thread, sharing the same connection. The
        // loop ends when the UI side is dropped, which is process exit.
        while let Ok(MonitorCommand::Install) = command_rx.recv() {
            if call_update(&connection, &monitor_path).is_err() {
                eprintln!("trace-commons-shell: flatpak update portal refused the install");
                // Synthesised so the window leaves `Installing` instead of
                // showing a progress bar that will never move again. It is
                // shaped exactly like the portal's own terminal error so
                // the state machine has one path, not two.
                let _ = signal_tx.send_blocking(PortalSignal::Progress {
                    n_ops: 0,
                    op: 0,
                    progress: 0,
                    status: PROGRESS_STATUS_ERROR,
                    error: None,
                });
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
fn create_monitor() -> anyhow::Result<(
    zbus::blocking::Connection,
    zbus::zvariant::OwnedObjectPath,
)> {
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
fn read_signals(
    connection: &zbus::blocking::Connection,
    path: &zbus::zvariant::OwnedObjectPath,
    name: &str,
    tx: &async_channel::Sender<PortalSignal>,
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
```

- [ ] **Step 2: Compile and lint**

Run: `scripts/linux-build.sh cargo clippy --all-targets -- -D warnings`

Expected: PASS, no warnings. If clippy objects to `while let Ok(MonitorCommand::Install) = command_rx.recv()` as an irrefutable-pattern-in-let, replace that line with:

```rust
        while command_rx.recv().is_ok() {
```

which is equivalent while `MonitorCommand` has one variant.

- [ ] **Step 3: Manual verification — the no-portal path against a live bus**

This proves the fail-closed branch reaches a real D-Bus session that has no
flatpak portal on it, which is what an unconfined Linux desktop looks like.

Run:

```bash
scripts/linux-build.sh "dbus-run-session -- cargo run --bin trace-commons-shell-probe 2>&1 | head -40"
```

That container has no `/.flatpak-info`, so `spawn_monitor` short-circuits before
any bus call. To exercise the *bus* branch instead, run this one-off inside the
container shell (`scripts/linux-build.sh --shell`):

```bash
touch /.flatpak-info
dbus-run-session -- bash -c 'busctl --user call org.freedesktop.portal.Flatpak /org/freedesktop/portal/Flatpak org.freedesktop.portal.Flatpak CreateUpdateMonitor "a{sv}" 0'
```

Expected: `Failed to call method: The name org.freedesktop.portal.Flatpak was
not provided by any .service files` — i.e. `ServiceUnknown`. That is the exact
error `create_monitor` converts into `PortalSignal::MonitorUnavailable`, and
confirms the bus name in `FLATPAK_PORTAL_BUS_NAME` is the one D-Bus actually
looks up. Then `rm /.flatpak-info`.

- [ ] **Step 4: Commit**

```bash
git add crates/trace-commons-contributor-gtk/src/update.rs
git commit -m "Talk to the flatpak update portal over zbus"
```

---

### Task 5: The copy, and turning state into what a person reads

**Files:**
- Modify: `crates/trace-commons-contributor-gtk/src/copy.rs`
- Create: `crates/trace-commons-contributor-gtk/src/ui/update.rs`
- Modify: `crates/trace-commons-contributor-gtk/src/ui/mod.rs` (add `pub mod update;` to the module list at the top, after `pub mod style;`)

**Interfaces:**
- Consumes: `UpdateState`, `FAILED_LABEL` from Task 2; `crate::ui::style::Tone` (existing, `#[derive(Clone, Copy, PartialEq, Eq)]`, no `Debug`).
- Produces:
  - `copy::UPDATE_AVAILABLE_BODY`, `UPDATE_AVAILABLE_ACTION`, `UPDATE_CONFIRM_HEADING`, `UPDATE_CONFIRM_BODY`, `UPDATE_CONFIRM_ACCEPT`, `UPDATE_CONFIRM_CANCEL`, `UPDATE_READY_BODY`, `UPDATE_READY_ACTION`, `UPDATE_FAILED_BODY`, `UPDATE_UNMANAGED_BODY`, `UPDATE_UNAVAILABLE_BODY` — all `&'static str`
  - `pub fn copy::update_installing_line(percent: u32) -> String`
  - `pub fn copy::update_offer_line(short_commit: &str) -> String`
  - `pub enum ui::update::BannerAction { Confirm, Restart }` — derives `Debug, Clone, Copy, PartialEq, Eq`
  - `pub struct ui::update::Banner { pub tone: Tone, pub body: String, pub action: Option<(&'static str, BannerAction)> }`
  - `pub fn ui::update::banner_for(state: &UpdateState) -> Option<Banner>`

- [ ] **Step 1: Write the failing tests**

Create `crates/trace-commons-contributor-gtk/src/ui/update.rs` with exactly this content:

```rust
//! The update surface: one banner under the header bar, one confirmation
//! dialog, and the restart prompt that finishes the job.
//!
//! The decision of *what* to say for a given state is a pure function
//! ([`banner_for`]) so it can be tested without a display; the widgets
//! below are a direct mapping of its output onto libadwaita and carry no
//! logic of their own. Nothing here installs anything: the confirmation
//! hands off to `update::UpdateMonitor::request_install`, which asks the
//! portal, which does the work.

use super::style::Tone;
use crate::copy;
use crate::update::UpdateState;

/// What the banner's one button does, if it has one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BannerAction {
    /// Open the confirmation dialog. Never installs directly: the banner is
    /// something a person notices, not something they act on by reflex.
    Confirm,
    /// Close the window so the next start runs the installed build.
    Restart,
}

/// Everything the banner renders, decided without touching a widget.
pub struct Banner {
    pub tone: Tone,
    pub body: String,
    /// Button label and what pressing it means, or `None` for a banner that
    /// is only telling you something.
    pub action: Option<(&'static str, BannerAction)>,
}

/// What to show for a state, or `None` to show nothing at all.
///
/// `Idle` is `None` on purpose: "you are up to date" is a sentence that
/// occupies the top of the window permanently and tells a contributor
/// nothing they can act on. `Installing` and `Ready` have no dismissal
/// because they describe work that is happening to their machine.
pub fn banner_for(state: &UpdateState) -> Option<Banner> {
    match state {
        UpdateState::Idle => None,
        UpdateState::Unmanaged => Some(Banner {
            tone: Tone::Neutral,
            body: copy::UPDATE_UNMANAGED_BODY.to_string(),
            action: None,
        }),
        UpdateState::Unavailable => Some(Banner {
            tone: Tone::Attention,
            body: copy::UPDATE_UNAVAILABLE_BODY.to_string(),
            action: None,
        }),
        UpdateState::Available { remote_commit } => Some(Banner {
            tone: Tone::Clear,
            body: copy::update_offer_line(remote_commit),
            action: Some((copy::UPDATE_AVAILABLE_ACTION, BannerAction::Confirm)),
        }),
        UpdateState::Installing { percent } => Some(Banner {
            tone: Tone::Held,
            body: copy::update_installing_line(*percent),
            action: None,
        }),
        UpdateState::Ready => Some(Banner {
            tone: Tone::Clear,
            body: copy::UPDATE_READY_BODY.to_string(),
            action: Some((copy::UPDATE_READY_ACTION, BannerAction::Restart)),
        }),
        UpdateState::Failed { .. } => Some(Banner {
            tone: Tone::Attention,
            body: copy::UPDATE_FAILED_BODY.to_string(),
            action: None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::FAILED_LABEL;

    #[test]
    fn being_up_to_date_puts_nothing_on_screen() {
        assert!(banner_for(&UpdateState::Idle).is_none());
    }

    #[test]
    fn an_offer_is_the_only_state_with_a_confirm_button() {
        let banner = banner_for(&UpdateState::Available {
            remote_commit: "beefbeefbeef".to_string(),
        })
        .expect("an offer must be shown");
        assert_eq!(banner.action, Some(("Install", BannerAction::Confirm)));
        // The commit a person is being offered is named, truncated.
        assert!(banner.body.contains("beefbeefbeef"), "{}", banner.body);

        for state in [
            UpdateState::Unmanaged,
            UpdateState::Unavailable,
            UpdateState::Installing { percent: 50 },
            UpdateState::Ready,
            UpdateState::Failed {
                label: FAILED_LABEL,
            },
        ] {
            let action = banner_for(&state).and_then(|banner| banner.action);
            assert_ne!(
                action.map(|(_, what)| what),
                Some(BannerAction::Confirm),
                "{state:?} must not offer an install"
            );
        }
    }

    #[test]
    fn an_install_in_flight_cannot_be_confirmed_again() {
        let banner = banner_for(&UpdateState::Installing { percent: 40 })
            .expect("work underway must be visible");
        assert!(banner.action.is_none());
        assert!(banner.body.contains("40"), "{}", banner.body);
    }

    #[test]
    fn a_finished_install_asks_for_a_restart_and_says_the_queue_is_safe() {
        let banner = banner_for(&UpdateState::Ready).expect("a finished install must be visible");
        assert_eq!(banner.action, Some(("Quit now", BannerAction::Restart)));
        assert!(banner.body.contains("queue"), "{}", banner.body);
    }

    #[test]
    fn a_failure_states_the_data_consequence_and_never_the_portal_text() {
        let banner = banner_for(&UpdateState::Failed {
            label: FAILED_LABEL,
        })
        .expect("a failure must be visible");
        assert!(banner.tone == Tone::Attention);
        assert!(banner.body.contains("unchanged"), "{}", banner.body);
        // The internal label is for logs, not for a window.
        assert!(!banner.body.contains(FAILED_LABEL), "{}", banner.body);
    }

    #[test]
    fn a_source_build_is_told_plainly_that_nothing_updates_it() {
        let banner = banner_for(&UpdateState::Unmanaged).expect("a source build must be told");
        assert!(banner.action.is_none());
        assert!(banner.tone == Tone::Neutral);
        assert!(banner.body.contains("built from source"), "{}", banner.body);
    }

    #[test]
    fn a_missing_portal_is_told_where_to_go_instead() {
        let banner = banner_for(&UpdateState::Unavailable).expect("a missing portal must be told");
        assert!(banner.action.is_none());
        assert!(banner.body.contains("flatpak update"), "{}", banner.body);
    }
}
```

Then add `pub mod update;` to the module list at the top of
`crates/trace-commons-contributor-gtk/src/ui/mod.rs`, so it reads:

```rust
pub mod history;
pub mod preview;
pub mod queue;
pub mod settings;
pub mod style;
pub mod update;
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `scripts/linux-build.sh cargo test ui::update::tests`

Expected: FAIL to compile with `error[E0425]: cannot find value UPDATE_UNMANAGED_BODY in module crate::copy` and the same for each of the other copy constants and the two copy functions.

- [ ] **Step 3: Write the copy**

Append to `crates/trace-commons-contributor-gtk/src/copy.rs`:

```rust
// --- Updating ----------------------------------------------------------

/// The offer, with the commit a person is being moved to.
///
/// The commit is named because "an update is available" with nothing else
/// is unfalsifiable -- there is no way for a contributor to check they got
/// what they were shown. Twelve characters of an ostree commit is enough to
/// compare against `flatpak info ai.tracecommons.Contributor` and short
/// enough to read.
pub fn update_offer_line(short_commit: &str) -> String {
    format!(
        "A newer Trace Commons is available ({short_commit}). Installing it replaces this app; \
         your queue and everything already waiting in it are untouched."
    )
}

/// The banner's button while an update is merely offered.
pub const UPDATE_AVAILABLE_ACTION: &str = "Install";

/// Kept as a constant so the banner body and the dialog body cannot drift
/// apart, since the dialog is the second time a person reads the same fact.
pub const UPDATE_AVAILABLE_BODY: &str = "A newer Trace Commons is available.";

/// The confirmation, which is where the actual decision is made.
pub const UPDATE_CONFIRM_HEADING: &str = "Install the newer version?";
pub const UPDATE_CONFIRM_BODY: &str =
    "Flatpak installs it. This app does not change while it is open -- you keep running this \
     version until you quit and reopen. Nothing in your queue is sent, removed or re-scanned.";
pub const UPDATE_CONFIRM_ACCEPT: &str = "Install";
pub const UPDATE_CONFIRM_CANCEL: &str = "Not now";

/// Progress. One sentence, because a progress bar carries the rest.
pub fn update_installing_line(percent: u32) -> String {
    format!("Installing the update -- {percent}% done. You can keep using this window.")
}

/// Installed but not yet running.
pub const UPDATE_READY_BODY: &str =
    "The update is installed. Quit and reopen Trace Commons to start using it. Your queue stays \
     exactly where it is.";
pub const UPDATE_READY_ACTION: &str = "Quit now";

/// Refused or failed. States the data consequence, names no mechanism, and
/// does not ask anyone to retry -- the portal re-checks on its own.
pub const UPDATE_FAILED_BODY: &str =
    "The update did not install. This copy is unchanged and nothing in your queue was affected. \
     It will be offered again.";

/// Built from source, so nothing here manages it. Honest about the fact
/// that this app is not checking anything in that case.
pub const UPDATE_UNMANAGED_BODY: &str =
    "This copy was built from source, so updates are not managed here and nothing is being \
     checked. Rebuild from the repository to move to a newer version.";

/// Under flatpak, but nothing answered.
pub const UPDATE_UNAVAILABLE_BODY: &str =
    "Updates cannot be offered here: this desktop's Flatpak service did not answer. Use your \
     software centre, or run flatpak update, to move to a newer version.";
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `scripts/linux-build.sh cargo test ui::update::tests`

Expected: PASS — `test result: ok. 7 passed`.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor-gtk/src/copy.rs crates/trace-commons-contributor-gtk/src/ui/update.rs crates/trace-commons-contributor-gtk/src/ui/mod.rs
git commit -m "Say what each update state means to a contributor"
```

---

### Task 6: The GTK surface and its wiring

**Files:**
- Modify: `crates/trace-commons-contributor-gtk/src/ui/update.rs`
- Modify: `crates/trace-commons-contributor-gtk/src/ui/mod.rs` (add the `update` field to `App`, build the view, append it to `content`, call `update::wire`)

**Interfaces:**
- Consumes: `Banner`, `BannerAction`, `banner_for` from Task 5; `UpdateState`, `PortalSignal`, `next_state`, `begin_install`, `spawn_monitor`, `UpdateMonitor`, `detect_install_kind`, `InstallKind` from Tasks 1–4; `style::card`, `style::Tone::glyph`, `style::space` (existing).
- Produces:
  - `pub struct ui::update::UpdateView { pub root: gtk::Box, ... }` with `pub fn new() -> Self` and `impl Default`
  - `pub fn ui::update::wire(app: &Rc<App>)`
  - `App` gains the public field `pub update: update::UpdateView`

- [ ] **Step 1: Write the widget layer**

First, the widgets need imports Task 5 deliberately left out (an unused import
would have failed that task's own lint gate). Replace the import block at the
top of `crates/trace-commons-contributor-gtk/src/ui/update.rs` — currently:

```rust
use super::style::Tone;
use crate::copy;
use crate::update::UpdateState;
```

with:

```rust
use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

use super::App;
use super::style::{Tone, space};
use crate::copy;
use crate::update::{self, UpdateState};
```

Then append to the same file, between `banner_for` and `#[cfg(test)] mod tests`:

```rust
/// The banner itself. Built in the same shape as the health banner in
/// `ui::mod` -- glyph, wrapping body, one optional button -- because a
/// contributor should not have to learn two kinds of notice bar.
pub struct UpdateView {
    pub root: gtk::Box,
    glyph: gtk::Label,
    body: gtk::Label,
    button: gtk::Button,
    /// The live state, owned by the UI thread. The D-Bus threads never
    /// touch it; they send signals and this applies them.
    state: RefCell<UpdateState>,
    /// What the button currently means, so one handler serves both actions.
    action: RefCell<Option<BannerAction>>,
    /// `None` outside a flatpak, where no monitor is ever created.
    monitor: RefCell<Option<update::UpdateMonitor>>,
}

impl Default for UpdateView {
    fn default() -> Self {
        Self::new()
    }
}

impl UpdateView {
    pub fn new() -> Self {
        let glyph = gtk::Label::new(Some(Tone::Neutral.glyph()));
        glyph.add_css_class("tc-card-title");
        glyph.set_valign(gtk::Align::Start);

        let body = gtk::Label::builder()
            .wrap(true)
            .xalign(0.0)
            .hexpand(true)
            .build();
        body.add_css_class("tc-body");

        let button = gtk::Button::builder().visible(false).build();
        button.add_css_class("tc-quiet");
        button.set_valign(gtk::Align::Center);

        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(space::M)
            .visible(false)
            .margin_top(space::M)
            .margin_start(space::L)
            .margin_end(space::L)
            .build();
        root.append(&glyph);
        root.append(&body);
        root.append(&button);
        root.add_css_class("tc-banner");

        Self {
            root,
            glyph,
            body,
            button,
            state: RefCell::new(UpdateState::Idle),
            action: RefCell::new(None),
            monitor: RefCell::new(None),
        }
    }
}

/// Draw the current state. Called once at startup and after every signal.
fn render(app: &Rc<App>) {
    let view = &app.update;
    let banner = banner_for(&view.state.borrow());

    let Some(banner) = banner else {
        view.root.set_visible(false);
        *view.action.borrow_mut() = None;
        return;
    };

    view.glyph.set_text(banner.tone.glyph());
    // One tone class at a time, so a state change does not leave the
    // previous state's colour behind.
    for tone in [
        Tone::Neutral,
        Tone::Clear,
        Tone::Attention,
        Tone::Held,
        Tone::Refused,
    ] {
        view.glyph.remove_css_class(tone.css());
    }
    view.glyph.add_css_class(banner.tone.css());
    view.body.set_text(&banner.body);

    match banner.action {
        Some((label, action)) => {
            view.button.set_label(label);
            view.button.set_visible(true);
            *view.action.borrow_mut() = Some(action);
        }
        None => {
            view.button.set_visible(false);
            *view.action.borrow_mut() = None;
        }
    }
    view.root.set_visible(true);
}

/// Start the monitor, pump its signals onto the main loop, and connect the
/// one button.
///
/// Outside a flatpak nothing is started at all: the state goes straight to
/// `Unmanaged` and the banner says so. Under flatpak the monitor runs on
/// its own threads and the window never blocks on it, so a portal that
/// never answers costs nothing but a missing banner.
pub fn wire(app: &Rc<App>) {
    if update::detect_install_kind() != update::InstallKind::Flatpak {
        *app.update.state.borrow_mut() = UpdateState::Unmanaged;
        render(app);
        return;
    }

    let monitor = update::spawn_monitor();
    let signals = monitor.signals.clone();
    *app.update.monitor.borrow_mut() = Some(monitor);
    render(app);

    let pump = Rc::clone(app);
    gtk::glib::spawn_future_local(async move {
        while let Ok(signal) = signals.recv().await {
            let next = {
                let current = pump.update.state.borrow();
                update::next_state(&current, &signal)
            };
            *pump.update.state.borrow_mut() = next;
            render(&pump);
        }
    });

    let pressed = Rc::clone(app);
    app.update.button.connect_clicked(move |_| {
        let action = *pressed.update.action.borrow();
        match action {
            Some(BannerAction::Confirm) => confirm_install(&pressed),
            // The existing close-request handler runs, so quitting still
            // says what keeps running afterwards. This does not relaunch:
            // a confined process cannot start itself, and the honest
            // instruction is to reopen it.
            Some(BannerAction::Restart) => pressed.window.close(),
            None => {}
        }
    });
}

/// The confirmation. Nothing about the installed application changes
/// without a person pressing the accept response here.
fn confirm_install(app: &Rc<App>) {
    let dialog = adw::MessageDialog::new(
        Some(&app.window),
        Some(copy::UPDATE_CONFIRM_HEADING),
        Some(copy::UPDATE_CONFIRM_BODY),
    );
    dialog.add_responses(&[
        ("cancel", copy::UPDATE_CONFIRM_CANCEL),
        ("install", copy::UPDATE_CONFIRM_ACCEPT),
    ]);
    dialog.set_close_response("cancel");

    let app = Rc::clone(app);
    dialog.connect_response(None, move |dialog, response| {
        dialog.close();
        if response != "install" {
            return;
        }
        // Move into Installing before the call goes out, so the window
        // shows work starting rather than sitting on a stale offer until
        // the first Progress signal arrives. `begin_install` is a no-op
        // from any state that is not an offer.
        let next = {
            let current = app.update.state.borrow();
            update::begin_install(&current)
        };
        *app.update.state.borrow_mut() = next;
        render(&app);

        if let Some(monitor) = app.update.monitor.borrow().as_ref() {
            monitor.request_install();
        }
    });
    dialog.present();
}
```

- [ ] **Step 2: Wire it into the window**

Four edits in `crates/trace-commons-contributor-gtk/src/ui/mod.rs`:

1. Add the field to `struct App`, immediately after the `pub settings: settings::SettingsView,` line:

```rust
    /// The update banner. Above the health banner is deliberate: an update
    /// is a standing fact about this machine, while health is about the
    /// current run.
    pub update: update::UpdateView,
```

2. In `App::build`, immediately after `let settings = settings::SettingsView::new();`:

```rust
        let update = update::UpdateView::new();
```

3. In `App::build`, change the `content` assembly so the update banner sits between the header and the health banner:

```rust
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("tc-root");
        content.append(&header);
        content.append(&update.root);
        content.append(&health_banner);
        content.append(&stack);
        stack.set_vexpand(true);
```

4. In the `Rc::new(Self { ... })` literal, add `update,` immediately after `settings,`; and in the wiring block, add `update::wire(&app);` immediately after `settings::wire(&app);` so it reads:

```rust
        app.wire_result_pump();
        app.wire_event_pump();
        app.wire_quit();
        app.wire_tray();
        queue::wire(&app);
        history::wire(&app);
        settings::wire(&app);
        update::wire(&app);
        app.refresh();
```

- [ ] **Step 3: Compile, test and lint**

Run: `scripts/linux-build.sh cargo test`

Expected: PASS — every test in the crate, including the 21 from Tasks 1–3 and the 7 from Task 5.

Run: `scripts/linux-build.sh cargo clippy --all-targets -- -D warnings`

Expected: PASS, no warnings.

- [ ] **Step 4: Manual verification — the banner renders and installs nothing**

Run: `scripts/linux-build.sh --run-headless`

Expected: the existing headless script starts the app under Xvfb with a private
session bus and prints `trace-commons-shell: realized, quitting
(--exit-after-realize)`. The container has no `/.flatpak-info`, so `wire` takes
the `Unmanaged` branch, no D-Bus call is made, and the log contains **no**
`flatpak update portal unavailable` line. Confirm that line is absent — its
presence would mean the unconfined branch is talking to the bus when it should
not be.

Then run the confined branch's rendering, still with no portal on the bus:

```bash
scripts/linux-build.sh "touch /.flatpak-info && bash crates/trace-commons-contributor-gtk/scripts/headless-run.sh 2>&1 | tail -20; rm -f /.flatpak-info"
```

Expected: the log now contains exactly one
`trace-commons-shell: flatpak update portal unavailable` line, the app still
reaches `realized, quitting`, and nothing crashes. That is the fail-closed path
end to end: no portal, a stated reason, and a window that still works.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor-gtk/src/ui/update.rs crates/trace-commons-contributor-gtk/src/ui/mod.rs
git commit -m "Show the flatpak update banner and its confirmation"
```

---

### Task 7: Manifest permission, CI coverage, and live flatpak verification

**Files:**
- Modify: `crates/trace-commons-contributor-gtk/flatpak/ai.tracecommons.Contributor.yml`
- Modify: `.github/workflows/ci.yml` (the `linux-contributor-shell` job, after the "Build the shell" step)

**Interfaces:**
- Consumes: everything from Tasks 1–6.
- Produces: no Rust interface. Produces the `--talk-name=org.freedesktop.portal.Flatpak` grant and a CI step that runs this crate's tests.

- [ ] **Step 1: Grant the D-Bus permission**

In `crates/trace-commons-contributor-gtk/flatpak/ai.tracecommons.Contributor.yml`,
replace the `--socket=session-bus` line and its comment with exactly this:

```yaml
  # Talks to org.freedesktop.Notifications (libnotify), the
  # org.freedesktop.portal.* surfaces (background, and file-chooser if
  # onboarding ever needs one), and org.kde.StatusNotifierWatcher for the
  # bonus tray. All of these are session-bus services, not portals with
  # their own finish-arg, except Notifications and the SNI watcher, which
  # need this socket explicitly.
  - --socket=session-bus
  # The update path (src/update.rs). org.freedesktop.portal.Flatpak is the
  # *flatpak* portal -- a different service from org.freedesktop.portal.Desktop
  # -- and it is what CreateUpdateMonitor and UpdateMonitor.Update live on.
  #
  # Strictly, --socket=session-bus above already covers this: it takes
  # flatpak's `unrestricted` branch (common/flatpak-run-dbus.c), which binds
  # the real bus socket into the sandbox and bypasses xdg-dbus-proxy
  # entirely. And even under the filtered proxy, the default rules include
  # `--call=org.freedesktop.portal.*=*`, and the UpdateMonitor's signals are
  # emitted unicast to the caller (portal/flatpak-portal.c emits with
  # destination = the requesting sender), not as broadcasts, so they are not
  # subject to the `--broadcast=...` path filter either.
  #
  # It is declared anyway, because a permission that the app genuinely needs
  # should be visible where a reviewer looks for it, and because narrowing
  # --socket=session-bus later must not silently break updating.
  - --talk-name=org.freedesktop.portal.Flatpak
```

- [ ] **Step 2: Verify the manifest still parses**

Run:

```bash
python3 -c "import yaml,sys; d=yaml.safe_load(open('crates/trace-commons-contributor-gtk/flatpak/ai.tracecommons.Contributor.yml')); print('\n'.join(d['finish-args']))"
```

Expected output, exactly these seven lines:

```
--socket=wayland
--socket=fallback-x11
--share=ipc
--socket=session-bus
--talk-name=org.freedesktop.portal.Flatpak
--filesystem=~/.claude/projects:ro
--filesystem=~/.codex/sessions:ro
```

- [ ] **Step 3: Give the crate's tests a CI job**

CI currently builds this crate but never runs `cargo test` for it — everything in
Tasks 1, 2, 3 and 5 would otherwise be uncovered on every future PR. In
`.github/workflows/ci.yml`, in the `linux-contributor-shell` job, insert this
step immediately after the `- name: Build the shell` step and before
`- name: Run weston + portal verification`:

```yaml
      # The GTK crate is its own workspace, so the root workspace's test job
      # never touches it. Its update, portal-parsing and copy logic is all
      # pure and needs no display -- but without this step nothing runs it.
      - name: Test the shell
        run: cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
```

- [ ] **Step 4: Verify the CI step locally**

Run: `cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml` inside the container:

```bash
scripts/linux-build.sh "cargo test --manifest-path /work/crates/trace-commons-contributor-gtk/Cargo.toml"
```

Expected: PASS, with the update and ui::update test counts from Tasks 1–5 present in the summary.

- [ ] **Step 5: Manual verification — a real flatpak, a real portal, a real update**

This is the only step that proves the subsystem works, and it cannot be
automated in this repository: it needs a Linux machine with `flatpak` and
`flatpak-builder`, a real session bus, and two builds published to a local
remote so there is something to update *to*. Run all of it on a Linux host.

```bash
# 1. Build and install version one into a local repo.
flatpak-builder --force-clean --repo=/tmp/tc-repo build-dir \
  crates/trace-commons-contributor-gtk/flatpak/ai.tracecommons.Contributor.yml
flatpak remote-add --user --no-gpg-verify tc-local /tmp/tc-repo
flatpak install --user -y tc-local ai.tracecommons.Contributor

# 2. Confirm the app can reach the flatpak portal at all.
flatpak run --command=busctl ai.tracecommons.Contributor --user \
  call org.freedesktop.portal.Flatpak /org/freedesktop/portal/Flatpak \
  org.freedesktop.DBus.Properties Get ss org.freedesktop.portal.Flatpak version
```

Expected from step 2: `v u 8` (or any `u` value >= 2). A `ServiceUnknown` here
means `flatpak-portal` is not installed or not activatable, and nothing further
in this procedure will work.

```bash
# 3. Start the app. It should show no update banner.
flatpak run ai.tracecommons.Contributor
```

Expected: the window opens with no banner under the header bar. `Idle` renders
nothing, so an update banner appearing at this point is a bug.

```bash
# 4. Publish a second build, then leave the app running.
#    Bump anything that changes the binary -- a comment in src/main.rs is enough.
flatpak-builder --force-clean --repo=/tmp/tc-repo build-dir \
  crates/trace-commons-contributor-gtk/flatpak/ai.tracecommons.Contributor.yml
flatpak build-update-repo /tmp/tc-repo
```

Expected within 30 minutes (the portal's default poll interval — to see it
sooner, restart the app, which creates a fresh monitor and triggers an immediate
first check): a `Clear`-toned banner reading "A newer Trace Commons is available
(<12 hex characters>)…" with an **Install** button.

Cross-check the commit shown against the remote's actual commit:

```bash
flatpak remote-info --user tc-local ai.tracecommons.Contributor | grep Commit
```

Expected: the banner's 12 characters are the leading 12 of that commit. If they
are not, `short_commit` or the `remote-commit` key is wrong.

```bash
# 5. Press Install. Accept the dialog.
```

Expected, in order: the banner turns `Held`-toned and reads "Installing the
update -- N% done", the percentage climbs monotonically to 100 across the whole
transaction (it must not reset to 0 and climb again per operation — that would
mean `overall_percent` is being fed the wrong `op`/`n_ops`), then the banner
turns `Clear` and reads "The update is installed. Quit and reopen Trace Commons
to start using it." with a **Quit now** button.

```bash
# 6. Confirm flatpak, not this app, did the replacing, and that the running
#    process is still the old build.
flatpak info ai.tracecommons.Contributor | grep Commit
```

Expected: the installed commit now matches the remote's. The running window is
still the previous build — that is correct and is exactly why the copy asks for
a restart rather than claiming the update is live.

```bash
# 7. Quit and reopen.
flatpak run ai.tracecommons.Contributor
```

Expected: the app starts on the new commit and shows no banner.

Finally, verify the fail-closed branch on the same machine:

```bash
cargo build --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml --bin trace-commons-shell
./crates/trace-commons-contributor-gtk/target/debug/trace-commons-shell
```

Expected: a `Neutral`-toned banner reading "This copy was built from source, so
updates are not managed here and nothing is being checked…", with no button, and
no `flatpak update portal unavailable` line in the terminal — the unconfined
branch must make no D-Bus call at all.

- [ ] **Step 6: Commit**

```bash
git add crates/trace-commons-contributor-gtk/flatpak/ai.tracecommons.Contributor.yml .github/workflows/ci.yml
git commit -m "Grant the flatpak update portal permission and run the shell's tests in CI"
```

---

## Verification

All of these must pass before the branch is considered done. Run them from the
repository root.

Formatting (runs on the macOS host; no compilation):

```bash
cargo fmt --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml -- --check
```

Expected: no output, exit 0.

Compilation, tests and lint (run in the Linux container):

```bash
scripts/linux-build.sh "cargo build --bins"
scripts/linux-build.sh "cargo test"
scripts/linux-build.sh "cargo clippy --all-targets -- -D warnings"
```

Expected: build succeeds; `cargo test` reports `test result: ok.` with at least
28 passing tests in the crate (21 from `update::tests`, 7 from
`ui::update::tests`); clippy emits no warnings.

The lockfile must be untouched, because the flatpak build is network-sandboxed
and `cargo-sources.json` is derived from it:

```bash
git diff --exit-code crates/trace-commons-contributor-gtk/Cargo.lock crates/trace-commons-contributor-gtk/Cargo.toml crates/trace-commons-contributor-gtk/flatpak/cargo-sources.json
```

Expected: no output, exit 0. Any diff here means a dependency was added and the
plan's zero-dependency premise is broken.

The manifest still parses and carries the grant:

```bash
python3 -c "import yaml; d=yaml.safe_load(open('crates/trace-commons-contributor-gtk/flatpak/ai.tracecommons.Contributor.yml')); assert '--talk-name=org.freedesktop.portal.Flatpak' in d['finish-args']; print('ok')"
```

Expected: `ok`.

The rust pin the release job enforces is unchanged:

```bash
grep -m1 '^rust-version' crates/trace-commons-contributor-gtk/Cargo.toml
grep -m1 -oE 'rust-[0-9]+\.[0-9]+\.[0-9]+-' crates/trace-commons-contributor-gtk/flatpak/ai.tracecommons.Contributor.yml
```

Expected: `rust-version = "1.92"` and `rust-1.92.0-`.

Headless run, unconfined branch:

```bash
scripts/linux-build.sh --run-headless
```

Expected: `trace-commons-shell: realized, quitting (--exit-after-realize)`, and
**no** `flatpak update portal unavailable` line.

Live flatpak verification: Task 7 Step 5 in full, on a Linux host with flatpak
and flatpak-builder. Its expected output is written out there step by step. This
is the only evidence that the portal path actually works; the automated checks
above prove the pure logic and the fail-closed branches only, and must not be
reported as proof that updating works.
