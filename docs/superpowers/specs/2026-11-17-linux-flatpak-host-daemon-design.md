# Linux Flatpak: host-daemon mode

Status: decided by the owner on 2026-11-17 ("Host daemon (Recommended)").
Implemented on branch `linux-flatpak-host-daemon`. Nothing here has run on
Linux yet; the checklist at the end is what a Linux box must confirm.

## Problem

The Flatpak build of the Linux shell runs its daemon inside the sandbox, and
inside the sandbox almost nothing the daemon needs is reachable:

- **No network.** `finish-args` in
  `crates/trace-commons-contributor-gtk/flatpak/ai.tracecommons.Contributor.yml`
  has no `--share=network`, and it never had one (`git log -S'share=network'`
  on the manifest finds nothing; `release-apps.yml` passes no finish-arg
  overrides to `flatpak-builder`). A Flatpak without that permission gets its
  own network namespace with loopback only. From code reading, then: the
  in-sandbox daemon cannot reach the issuer, ingest, or NEAR AI, so it cannot
  enroll, upload, sign in, or run a Private AI session today. Not yet
  confirmed on a Linux box (checklist item 1).
- **No host terminal.** `managed::terminal::sandboxed()` turns Launch off
  under `/.flatpak-info`; the sandbox cannot exec a host terminal.
- **No host tools or folders.** `claude`, `codex` and `opencode` on the host
  `PATH`, and the person's project folders, are outside the sandbox.

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
3. In a Flatpak, and the host link socket file exists but nothing listens:
   the host daemon is installed and down (crashed, restarting, or stopped).
   Attach to it anyway. Calls fail with the existing `daemon-not-running`
   label and the window shows its existing not-running state; no new text.
   The host daemon deliberately leaves its host-link socket file in place on
   a clean stop so that this state is recognisable. `/run/user/<uid>` is
   cleared at logout, so the marker never outlives the login.
4. In a Flatpak, and no host link socket file: the host part is not
   installed. Today's in-sandbox path runs unchanged (roots screen, hosting).

### Never two daemons over the same sessions

The existing `daemon.lock` cannot arbitrate here: the sandbox's state
directory is `~/.var/app/ai.tracecommons.Contributor/config/trace-commons`,
the host's is `~/.config/trace-commons`, and they hold two different locks.

So a second lock, `watcher.lock` in the same runtime directory, is taken by
every daemon on its default state directory, on the host and in the sandbox,
for its whole life, after `daemon.lock`. Whoever holds it watches; a second
start fails with the existing `StartFailure::AlreadyRunning`:

- Window hosting in the sandbox, then the host daemon starts: the host start
  fails, systemd retries every 30 s (`Restart=on-failure`, `RestartSec=30`),
  and it comes up when the window quits. The next window attaches to it.
- Host daemon running, then a window starts: case 2 above; if the window raced
  past the probe and tried to host, its start fails on `watcher.lock` and it
  re-probes and attaches to the host link instead.

`flock` on one inode reached through a bind mount is the same lock on both
sides (checklist item 4).

Existing in-sandbox enrollments: without network the sandbox could not
enroll, so none should exist. If one did, it stays in the sandbox directory,
unused while the host daemon runs; nothing migrates it.

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

## No host daemon installed

Launch stays off in the sandbox with the existing "unavailable" line, as
today. Offering to install the host part is a new notice, and the repo rule
forbids one without owner approval of both the notice and its words. The
mechanism is therefore not built into the UI; proposed words are in shared
copy, marked DRAFT, NEEDS APPROVAL, and not shown anywhere
(`managed::copy::host_daemon_offer_draft`). The intended action, once approved,
is to open the documented install command in the person's browser or a
portal-launched terminal; this change does not run anything on the host.

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
- **Folder chooser paths.** `gtk::FileChooserNative` in a sandbox goes through
  the document portal and can return a `/run/user/<uid>/doc/...` path rather
  than the host path. The host daemon would then launch in the FUSE view of
  the folder. A follow-up should map the result through
  `org.freedesktop.portal.Documents.GetHostPaths`. A typed path is passed as
  typed.
- **Staging Flatpak.** The manifest is production-only; a staging Flatpak
  would need `xdg-run/trace-commons-staging:create`.

## Permissions that can then be dropped (proposal only)

Not changed in this branch, because the in-sandbox fallback (case 4) still
needs them:

- `--filesystem=~/.claude/projects:ro` and `~/.codex/sessions:ro`: only the
  in-sandbox daemon reads sessions. If the in-sandbox daemon is retired (it
  cannot upload without network anyway), both go.
- `--socket=session-bus` could narrow to `--talk-name` entries for
  Notifications, the StatusNotifierWatcher and `portal.Flatpak`, plus
  `org.freedesktop.secrets` only while the sandbox still hosts. Risky without
  a Linux run: the tray and update monitor must be re-tested.
- `--share=network` should not be added; the host daemon does the network.

## Owner review points

- `crates/trace-commons-contributor/tests/release_pipeline.rs` pinned the
  manifest at exactly two filesystem grants as a security assertion. It now
  pins three, naming the new one exactly and refusing any `xdg-config`
  grant. This is a deliberate widening and needs the owner's sign-off.
- `FLATPAK_SESSION_ROOTS_EXPLANATION` in the GTK crate's `src/copy.rs`
  ("It asks for access to those folders only.") is no longer exact: the app
  also gets the runtime directory, which holds no session data. Not edited;
  approved words.

## Residual risks

- At login the runtime directory is empty, so which daemon wins is start
  order: if the window starts and hosts before the systemd unit runs, the
  unit fails on `watcher.lock` and retries until the window quits. The
  window does not hand over to the host part while it runs.
- A sandboxed daemon arbitrates only when the runtime directory is mounted
  in the sandbox (the manifest grant). An older Flatpak build without the
  grant hosts without taking `watcher.lock`, so a host daemon cannot see it.
- After `close`, the host-link listener stops when the runtime retires its
  aborted task; `daemon run` exits right after, so in practice at once.

## Linux checklist

1. In the installed Flatpak, `flatpak run --command=sh ai.tracecommons.Contributor`
   then `curl -sS https://tracecommons.ai` (or `getent hosts tracecommons.ai`)
   fails: confirms there is no network in the sandbox today.
2. Host: install the CLI, `daemon install`, `systemctl --user enable --now`.
   `ls -ld $XDG_RUNTIME_DIR/trace-commons` is `drwx------` owned by you, and
   `daemon.sock` and `watcher.lock` are in it.
3. Start the Flatpak: it shows the host daemon's queue, not a roots screen.
   `ls $XDG_RUNTIME_DIR/trace-commons` inside `flatpak run --command=sh`
   lists the same socket.
4. Stop the unit, start the Flatpak with no host-link socket file present
   (log out and in first, so the runtime directory is empty) so the window
   hosts its own daemon, then `systemctl --user start` the unit: the unit
   fails with "already running" and retries every 30 s; quitting the window
   lets it start, and the next window attaches to it.
5. `systemctl --user stop` the unit with the window open: the window shows
   its not-running state; start it again: calls succeed.
6. Private AI: Launch is enabled, the terminal picker lists host terminals,
   a session opens in a host terminal, in the folder chosen.
7. Pick the folder with the chooser: record whether the path the daemon gets
   is the host path or a `/run/user/<uid>/doc/` path.
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
