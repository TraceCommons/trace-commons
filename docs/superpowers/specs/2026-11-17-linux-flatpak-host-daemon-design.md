# Linux Flatpak: host-daemon mode

Status: decided by the owner on 2026-11-17 ("Host daemon (Recommended)").
Implemented on branch `linux-flatpak-host-daemon`. Nothing here has run on
Linux yet; the checklist at the end is what a Linux box must confirm.

## Problem

The Flatpak build of the Linux shell runs its daemon inside the sandbox, and
from inside the sandbox two things the product needs are out of reach:

- **Network is not the reason.** The manifest had no `--share=network`;
  #2172 adds it (owner, 2026-11-17), so the in-sandbox daemon can reach the
  issuer, ingest and NEAR AI. The host daemon is not a network workaround.
- **No host terminal.** `managed::terminal::sandboxed()` turns Launch off
  under `/.flatpak-info`; the sandbox cannot exec a host terminal.
- **No host tools or folders.** `claude`, `codex` and `opencode` on the host
  `PATH`, and the person's project folders, are outside the sandbox.
- **No watching once the window closes.** A daemon in the sandbox lives in
  the window's process; the host daemon keeps watching for traces when the
  window is closed, and is the single watcher on the login.

## Decision

The Flatpak becomes the window only. The contributor daemon runs on the host,
installed by the existing CLI installer (`scripts/install.sh`, then
`daemon install` and `systemctl --user enable --now`), and the sandboxed
window talks to it over a socket. The host daemon launches terminals with the
code it already has, because from the host `sandboxed()` is false.

## How the window finds the host daemon

The host daemon's normal socket is `~/.config/trace-commons/daemon.sock`,
inside the 0700 state directory that also holds the device key. Granting the
sandbox that directory would expose the key, so it is not granted.

Instead the daemon serves a second listener, the *host link*, at

```
$XDG_RUNTIME_DIR/<state-dir-name>/daemon.sock
```

where `<state-dir-name>` is `crate::channel::STATE_DIR_NAME`
(`trace-commons`, or `trace-commons-staging` for a staging build). The
manifest grants exactly that directory:

```
--filesystem=xdg-run/trace-commons:create
```

`xdg-run/...` is bind-mounted at the same path inside the sandbox, and
`XDG_RUNTIME_DIR` is the same value on both sides, so both compute one path.
`:create` makes Flatpak create the directory if the host daemon has not run
yet this login, so a window started first still sees the socket when the
daemon arrives. The daemon re-asserts 0700 on a directory it owns (Flatpak
may have created it with a wider mode), and refuses to serve from one it
does not own.

The host link is served only:

- on Linux, by a daemon that is **not** itself in a Flatpak sandbox;
- when the daemon's state directory is the platform default (the one the
  installer's unit names). A daemon run with a custom
  `TRACE_COMMONS_CONTRIBUTOR_DIR` serves no host link, so two daemons on two
  directories never fight over one runtime path;
- when `XDG_RUNTIME_DIR` is set and absolute.

A failure to bind the host link is logged with a fixed label and does not stop
the daemon: the primary socket is the contract every other client uses.

Besides the socket, the directory holds `watcher.lock` and, only while the
systemd unit is installed, the empty marker `host-unit`; see "Which daemon
runs" below.

## Peer checks across the boundary

The primary socket's only control is its 0700 directory. The host link keeps
that rule (its directory is 0700, inside `/run/user/<uid>`, itself 0700) and
adds one more: each accepted connection's `SO_PEERCRED` uid must equal the
daemon's own uid, or the connection is closed before a byte is read. A Flatpak
app runs under bubblewrap with the same uid mapped through, so the window
passes and nothing else on the machine can. Every request then runs through
`serve_connection`, the same dispatcher as the primary socket, so no method
is added, widened or skipped.

## Which daemon runs: preference order

`Backend::open` in the GTK shell decides, in this order:

1. Not in a Flatpak (`/.flatpak-info` absent): exactly today's logic. Attach
   to the state directory's socket if a daemon is listening, otherwise host
   one in-process.
2. In a Flatpak, and the host link socket accepts a connection: attach to the
   host daemon.
3. In a Flatpak, nothing listens, and the marker file `host-unit` is in the
   runtime directory: the host daemon is installed and down (crashed,
   restarting, or stopped). Attach to it anyway. Calls fail with the existing
   `daemon-not-running` label and the window shows its existing not-running
   state; no new text. Its event stream is reopened every 2 s while the
   daemon is down, and the first events after it returns start with
   `resync_required`.
4. In a Flatpak, nothing listens and no marker: the host part is not
   installed, or a daemon that is not the unit left a socket. Today's
   in-sandbox path runs unchanged (roots screen, hosting).

Only the systemd user unit's daemon writes `host-unit`, at start, and it
leaves it when it stops. It knows it is the unit by `INVOCATION_ID` together
with `/proc/self/cgroup` naming the unit (`.../trace-commons-contributor.service`;
the staging unit for a staging build): `INVOCATION_ID` alone is inherited by
anything a service spawns. Every daemon -- the unit too -- removes its
host-link socket on close, so a one-off `daemon run`, or a GTK window on the
host hosting its own, leaves nothing that reads as "installed". A daemon that
dies without closing leaves a dead socket, which without the marker reads as
absent. `daemon uninstall` removes the marker; `/run/user/<uid>` is cleared at
logout, so it never outlives the login either way. The marker is written by
unlinking the name and creating it with `O_EXCL`, so a symlink planted there
(the sandbox can write the directory) is replaced, never followed.

### Never two daemons over the same sessions

The existing `daemon.lock` cannot arbitrate here: the sandbox's state
directory is `~/.var/app/ai.tracecommons.Contributor/config/trace-commons`,
the host's is `~/.config/trace-commons`, and they hold two different locks.

So a second lock, `watcher.lock` in the same runtime directory, is taken by
every daemon on its default state directory, on the host and in the sandbox,
for its whole life, after `daemon.lock`. Whoever holds it watches; a second
start fails with the existing `StartFailure::AlreadyRunning` (so the FFI label
stays `already-running`), whose message says that another daemon on this login
watches the session folders, not that the state directory's lock is held.

Only `WouldBlock` from the lock means "held". Any other lock error, or a
`watcher.lock` that is not a regular file this daemon opened itself (a
symlink, a directory, a file swapped in between the checks), refuses the start
with an unnamed error (the FFI's generic `daemon-start-failed`) rather than
running unarbitrated. With no `libc` in the crate there is no `O_NOFOLLOW`:
the file is made with `create_new` (`O_EXCL`, which refuses any existing name,
a dangling symlink included), or an existing name is opened without `create`
and its `fstat` dev/inode must equal the name's `lstat` and be a regular file.

On the host, a window whose start loses the watcher lock attaches to
whichever socket answers: its own state directory's, then the host link. A
daemon in the sandbox serves no host link and nothing on the host can reach
it, so the window then shows its existing not-running state.

- Window hosting in the sandbox, then the host daemon starts: the host start
  fails, systemd retries every 30 s (`Restart=on-failure`, `RestartSec=30`),
  and it comes up when the window quits. The next window attaches to it.
- Host daemon running, then a window starts: case 2 above; if the window raced
  past the probe and tried to host, its start fails on `watcher.lock` and it
  re-probes and attaches to the host link instead.

`flock` on one inode reached through a bind mount is the same lock on both
sides (checklist item 4).

Existing in-sandbox enrollments: with #2172's network grant the sandbox can
enroll. Such an enrollment, its declared folders and its sign-ins stay in the
sandbox directory, unused while the host daemon runs; nothing migrates them,
and the host daemon starts with its own state (owner review point below).

## Version skew

The Flatpak updates through the portal; the host daemon through its own
updater. Either can be newer. The window already attaches to systemd daemons
of other versions outside Flatpak, and the shells already read the
`managed_snapshot` `capabilities` object, where an absent key reads as off.
This change adds one key, `capabilities.daemon_on_host` (true when the daemon
is not in a Flatpak sandbox). A daemon older than this change omits it, which
reads as false; Launch still follows `terminal_launch`, which every daemon
already computes from its own environment. `SUPPORTED_VERSIONS` is unchanged:
the host link speaks `trace_commons.daemon.v1_1` like the primary socket.

An older host daemon serves no host link at all, so a new window sees case 4
and runs in the sandbox as today until the host part is updated.

## No host daemon installed: the background-service dialogs

Flow and words approved by the owner on 2026-11-17 ("suggested words are
approved", "yes approved"). They replace the earlier unshown draft offer,
which is removed.

**One answer for every shell.** `daemon::service::classify` (contributor
crate) turns four facts into `Running`, `NotInstalled`,
`InstalledNotRunning` or `TooOld`: the host link's probe
(Listening/Down/Absent), whether the window runs a daemon in its own process,
whether the unit's file is installed (systemd's `GetUnitFileState` on the
session bus; on the host, also the file `daemon install` writes), and
`hello`'s new `service_level` from a daemon outside the window, compared with
the shell's `MIN_SERVICE_LEVEL` (absent reads as too old), and whether systemd
reports the unit `active` (`GetUnit`, then the unit's `ActiveState`) and
since when (`ActiveEnterTimestamp`). Too old needs positive evidence of an
old daemon: one that answers `hello` without `service_level`, or a unit
active for at least `TOO_OLD_GRACE` (15 s) with nobody on the host link and
no `host-unit` marker. A current unit daemon writes the marker as soon as it
takes `watcher.lock` (so one that could not bind the link reads `Down`, not
too old), and one that loses the lock -- right after the install command,
while the window's own daemon runs -- is `active` (`Type=simple`) only until
it exits, well inside the grace. A unit held off by the window's own lock is
restarting (`activating`), so it reads as installed but not running. The daemon reports
no build version on the wire and none is added. Only the Linux GTK shell draws
it in this change; the Mac dialog for its rare case is a later follow-up.

**When a dialog shows.** During first setup, as one step before the folders
screen, only when the service is not running; and when the person presses
Launch in the Private AI start area of a Flatpak whose daemon is not the host
daemon. Never on an ordinary start. "Not now" quiets both for the rest of the
run; the start area then keeps the existing unavailable line and a "Set up"
button that reopens the dialog. A headless driver run (`--exit-after-realize`,
`--start-page`, `--onboarding-page`) skips the first-setup step.

**The dialogs.**

- Not installed: the install command with "Copy command" (clipboard only;
  the app never runs it), "Open install guide", "Not now". While open the
  window looks again every 2 s. When the unit's file appears it switches
  over: stops a daemon it runs itself (`Backend::stop`: shutdown, wait for
  the pass, then let the runtime end -- its `spawn_blocking` work included --
  before closing, so the state directory's lock and `watcher.lock` are
  released only once nothing of that daemon can still write), asks systemd
  to `RestartUnit` the fixed unit, waits up to 20 s for it to answer, and
  opens again by the usual order, which attaches. Restart, not start (owner,
  2026-11-17): the unit's first start failed on the window's lock, so it
  sits in `auto-restart` waiting out `RestartSec=30`, and before systemd
  v257 a `StartUnit` job queued then waits for that timer (per the review:
  `service_start` returns `-EAGAIN` in that state); a restart job starts it
  at once, so there is no 30 s wait. 20 s covers only the daemon's own start
  (`Type=simple`, no `ExecStartPre`). If the stop does not finish within its
  bound, the locks stay held until the old runtime ends and the restart is
  skipped. A switch-over that does not bring the service up is tried again
  after 30 s (longer than the 20 s wait, so a unit still starting is not
  restarted under itself), at most three times in all; the dialog keeps its
  waiting line meanwhile. Once the service answers, the dialog closes with
  nothing else shown. When the third try has not brought it up (owner,
  2026-11-17), the dialog shows, in place of its waiting line, the
  not-running dialog's "It didn't start. To see why, run this in a
  terminal:" and the status command -- no other words, no new notice --
  and keeps watching, so a service that comes up later still closes it.
  The too-old dialog keeps waiting; the line was approved for this one.
- Installed but not running: "Start it" does the same switch-over (the same
  `RestartUnit`; the button still says "Start it"). If the service still does
  not answer, the dialog shows "It didn't start. To see why, run this in a
  terminal:" and the status command. When the window is attached over the
  host link to a stopped unit, `managed_snapshot` fails with
  `daemon-not-running`; the Private AI start area still draws its
  unavailable line and "Set up", so this dialog stays reachable.
- Too old: the same install command. Its last step is `systemctl --user
  restart`, so running it again replaces an older daemon that is running.
  While open it watches and switches over the same way when the updated unit
  is held off by the window's own daemon.

**After switching over** (owner, 2026-11-17): when the daemon the window
now attaches to -- after a switch-over, or any attach to the background
service -- has no session folders declared (`get_settings`: `claude_source`
or `codex_source` null), the window shows the existing folders screen (the
Tools step first run uses), answering to that daemon over `set_settings`.
Once per attachment, never while the window hosts its own daemon, and not
while first run is open, whose own Tools step asks the same; sign-in and
enrollment stay with their existing flows. No new words. The managed-session
revision the window last drew is forgotten on every reopen and on
`resync_required`: the sandbox's and the host's daemons count separately.
A folder picked in the Flatpak's chooser, or dropped, outside the app's
grants is a document-portal path (`$XDG_RUNTIME_DIR/doc/<id>/<name>`). For
the host daemon it is mapped to the host folder through the document
portal's `GetHostPaths`, bounded and off the GTK thread, as Launch does. The
mapping sits in the one path every folder declaration takes to a running
daemon (`ui::roots::declare_to_daemon`): the folders screen above and the
first run's Roots commit (`Commit::Roots`, `set_settings`), whichever way
the folder came in. The daemon's answer is checked against what was sent.
If a folder cannot be mapped, its host path is not UTF-8, or anything sent
would still be in the portal's mount, nothing is sent and the screen shows
its existing "couldn't be saved" line. The first run before any daemon runs
writes the sandbox's own settings file, where the portal path is right; if
that start then attaches to the host daemon instead, the first run opens at
Tools rather than taking those answers as sent.

Nothing is declared while a switch-over is under way: a reopen queued ahead
of a declaration replaces the backend first, so until the window has
recorded the reopen's attachment (`Worker::declaration_target`), where a
declaration would land is unknown and the screen shows the same "couldn't
be saved" line.

A document portal below interface version 5 (GNOME 46's) has no
`GetHostPaths`, so no chooser folder outside the grants can be mapped (owner,
2026-11-17: "Explain + CLI fallback"). The folder is still refused with the
existing line. Under it the Tools screen adds the owner's approved line
(2026-11-17, `managed::copy::host_folder`: "To add this folder anyway, open
a terminal in it and run:") and a fixed command (`daemon::service::host_folder_command`,
`daemon settings --set "claude_root=$PWD"` or `codex_root`) to run in a
terminal opened in that folder, with "Copy command" only; the window never
runs it and does not know the host path to put in it. Only Claude Code and
Codex take a bare path there, and only one refused folder at a time gets
the command. Any other refusal (a portal new enough to map, or one whose
version is unknown) shows the failure line alone.

**Falling back.** A Flatpak window attached over the host link checks every
5 s; when the link reads `Absent` (no socket answers, no unit marker: the
host part was uninstalled, or the login's runtime directory was cleared) it
quietly reopens, which runs the daemon in the sandbox again, and Private AI
shows its unavailable line again. If the sandbox cannot start its own daemon
(no folders declared there) the window stays attached, showing its existing
not-running state, and does not retry until the link changes.

**The command the dialogs show** (production; a staging build shows the
staging installer flag, command and unit, from `channel`):

```
curl -fsSL https://raw.githubusercontent.com/TraceCommons/trace-commons/main/scripts/install.sh | sh && "$HOME/.local/bin/near-ai" daemon install && systemctl --user daemon-reload && systemctl --user enable trace-commons-contributor.service && systemctl --user restart trace-commons-contributor.service
```

`scripts/install.sh` installs only the CLI binaries; it writes no unit and
enables nothing, so the command adds the documented `daemon install` and the
`systemctl --user` steps. The status command is `systemctl --user status
trace-commons-contributor.service`. Both are fixed strings in `channel.rs`,
never read from settings or IPC.

## Updating the host daemon

The host part is the CLI install, so it updates the way the CLI updates:
`trace-commons-contributor update`, or a staged update applied at the next
daemon start (`update::run::apply_staged`, already in `start_embedded`). The
Flatpak does not update it and cannot.

## Gaps this does not close

- **State read outside IPC.** The shell reads two things from its own state
  directory rather than from the daemon: the witness config
  (`ui/settings.rs` `witness_read`/`witness_write`) and the missions switch
  (`main.rs`, `ui::read_missions_shown`). Attached to the host daemon from the
  sandbox, these read and write the sandbox's copy, which the host daemon
  never reads. Fixing it needs IPC methods for both; granting the host state
  directory would expose the device key and is rejected.
- **Folder chooser paths, outside Launch.** The Private AI launch maps a
  `$XDG_RUNTIME_DIR/doc/<id>/<name>` folder through
  `org.freedesktop.portal.Documents.GetHostPaths` (interface version 5)
  before sending it to a host daemon, and refuses the launch with the
  existing "action failed" toast when it cannot (`document_portal.rs`). It
  checks the path being sent, so a typed portal path is mapped too. The
  Insights evidence chooser (`ui/insights.rs`) is not mapped yet. Folder
  declarations are mapped too (see "After switching over").
- **Staging Flatpak.** The manifest is production-only; a staging Flatpak
  would need `xdg-run/trace-commons-staging:create`.

## Permissions that can then be dropped (proposal only)

Not changed in this branch, because the in-sandbox fallback (case 4) still
needs them:

- `--filesystem=~/.claude/projects:ro` and `~/.codex/sessions:ro`: only the
  in-sandbox daemon reads sessions. If the in-sandbox daemon is retired,
  both go.
- `--socket=session-bus` could narrow to `--talk-name` entries for
  Notifications, the StatusNotifierWatcher, `portal.Flatpak` and
  `org.freedesktop.systemd1` (the background-service dialogs' `RestartUnit`,
  `GetUnitFileState`, `GetUnit`, `ActiveState` and `ActiveEnterTimestamp`), plus
  `org.freedesktop.secrets` only while the sandbox still hosts. Risky without
  a Linux run: the tray and update monitor must be re-tested.
- `--share=network` is added by its own PR (owner, 2026-11-17), not here.

## Owner review points

- `crates/trace-commons-contributor/tests/release_pipeline.rs` pinned the
  manifest at exactly two filesystem grants as a security assertion. It now
  pins three, naming the new one exactly and refusing any `xdg-config`
  grant. This is a deliberate widening and needs the owner's sign-off.
- `FLATPAK_SESSION_ROOTS_EXPLANATION` in the GTK crate's `src/copy.rs` now
  names the private folder (owner-approved 2026-11-17).
- The host daemon starts with its own state directory: a person who set up
  the sandbox (folders, sign-in, enrollment) sets up again through the
  window once it attaches to the host daemon. Nothing migrates.
- The install command is composed from the documented steps (installer,
  `daemon install`, `daemon-reload`, `enable`, `restart`); see "No host
  daemon installed".

## Residual risks

- At login the runtime directory is empty, so which daemon wins is start
  order: if the window starts and hosts before the systemd unit runs, the
  unit fails on `watcher.lock` and retries until the window quits. The
  window hands over only through the background-service dialog's
  switch-over (Launch, "Set up", or first setup), never by itself.
- Any `watcher.lock` error, not only an unsafe lock, refuses the daemon's
  start (the FFI's generic `daemon-start-failed`), so a broken runtime
  directory stops the daemon rather than letting it run unarbitrated. The
  `open_lock` check-then-open (an `lstat`, then an open without `create`
  whose `fstat` must match) has a residual race: a FIFO swapped in between
  the two makes the open for writing wait for a reader, so the start hangs
  instead of refusing; the checks after the open refuse it only once the
  open returns. The sandbox can write the directory, so a sandboxed process
  of the same uid could do this; it gains nothing but a stalled start.
- A sandboxed daemon arbitrates only when the runtime directory is mounted
  in the sandbox (the manifest grant). An older Flatpak build without the
  grant hosts without taking `watcher.lock`, so a host daemon cannot see it.
- After `close`, the host-link listener stops when the runtime retires its
  aborted task; `daemon run` exits right after, so in practice at once.
- A crashed one-off host daemon leaves a dead socket and no marker; a
  Flatpak window started then runs its own daemon. That is the intended
  fallback, not a second watcher: the dead daemon holds no lock.

## Linux checklist

1. In the installed Flatpak, `flatpak run --command=sh ai.tracecommons.Contributor`
   then `getent hosts tracecommons.ai`: record whether the sandbox resolves
   names (it does once #2172's `--share=network` ships). Informational; the
   host daemon does not depend on it.
2. Host: install the CLI, `daemon install`, `systemctl --user enable --now`.
   `ls -ld $XDG_RUNTIME_DIR/trace-commons` is `drwx------` owned by you, and
   `daemon.sock` and `watcher.lock` are in it.
3. Start the Flatpak: it shows the host daemon's queue, not a roots screen.
   `ls $XDG_RUNTIME_DIR/trace-commons` inside `flatpak run --command=sh`
   lists the same socket.
4. Stop the unit, start the Flatpak with no `host-unit` marker present
   (log out and in first, so the runtime directory is empty) so the window
   hosts its own daemon, then `systemctl --user start` the unit: the unit
   fails with "already running" and retries every 30 s; quitting the window
   lets it start, and the next window attaches to it.
5. `systemctl --user stop` the unit with the window open: the window shows
   its not-running state, `daemon.sock` is gone and `host-unit` stays; start
   it again: calls succeed and the queue refreshes by itself within a few
   seconds (the event stream reopened).
6. Private AI: Launch is enabled, the terminal picker lists host terminals,
   a session opens in a host terminal, in the folder chosen.
7. Pick a folder outside the app's grants with the chooser (one that comes
   back as `/run/user/<uid>/doc/<id>/<name>`), launch: the session opens in
   the host folder, not under `/run/user/<uid>/doc/`. Then revoke the
   document (`flatpak document-unexport`) and launch with the same path
   typed in: the launch is refused with the "action failed" toast. Record
   the portal's version (`gdbus introspect --session --dest
   org.freedesktop.portal.Documents --object-path
   /org/freedesktop/portal/documents | grep version`): below 5 there is no
   `GetHostPaths` and every portal-path launch is refused.
8. Uninstall the host part, log out and in, start the app: it runs in the
   sandbox as before (roots screen), Launch off with the existing line.
9. Another local user (or `sudo -u nobody socat - UNIX-CONNECT:...`) cannot
   connect to the host link.
10. Confirm `:create` works on an `xdg-run/...` path: with no
    `$XDG_RUNTIME_DIR/trace-commons` on the host, `flatpak run` creates it,
    and the host daemon then tightens it to 0700.
11. Confirm `flock` on `watcher.lock` through the bind mount excludes the
    host daemon (item 4).
12. Run `scripts/flatpak/check-host-link.sh`; it covers items 1-3 and
    the `daemon_on_host` capability from both sides of the sandbox.
13. `cat /proc/<unit daemon pid>/cgroup` ends with
    `/trace-commons-contributor.service`, and `host-unit` exists while the
    unit runs. Run `trace-commons-contributor daemon run` by hand instead
    (unit stopped): no `host-unit` is written, and after Ctrl-C no
    `daemon.sock` is left.
14. `daemon uninstall` removes `host-unit`; a Flatpak window started next
    runs its own daemon.
15. With the unit stopped and no window open, replace `watcher.lock` with a
    symlink (`ln -sf /tmp/x $XDG_RUNTIME_DIR/trace-commons/watcher.lock`):
    starting the unit fails and `/tmp/x` is not created.
