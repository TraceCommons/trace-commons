# Contributor shell — Windows

Date: 2026-08-08
Status: approved for planning
Scope: sub-project 5. Platform mechanics only.
Reads with: `2026-08-08-contributor-shell-shared-design.md`.

**Sequence this last.** It has a hard prerequisite the other two do not, and
that prerequisite is security-critical rather than plumbing.

## The blocker, stated first

The daemon's control socket is unix-only. Windows named-pipe support is
specified in the v1 contract but deliberately not implemented, because a
per-user-restricted pipe requires building a `SECURITY_DESCRIPTOR`, and tokio
only exposes `ServerOptions::create_with_security_attributes_raw`. That means
a **new `windows-sys` dependency, which needs explicit approval** under the
repo's dependency policy and has not been granted.

More importantly: on Unix the 0700 state directory is what protects the
socket, and Windows has no equivalent doing that work. **The pipe's ACL is the
only access control there is.** It must be written as security work — reviewed,
tested against an unprivileged second user — not as a porting detail.

Until that lands, the Windows app can still be built in the hosting
configuration below, because a hosting app calls the library directly and does
not need the pipe at all. The pipe is required only for CLI control while the
app runs. Ship without it if necessary, and say so.

## Shape

**One signed executable, like macOS.** The app links the C-ABI library, calls
`tc_daemon_start`, and hosts the loops in-process. The tray is real on Windows
and is the primary ambient surface, with a normal window behind it.

## Legibility is the platform-specific requirement

Windows users audit background software in Task Manager → Startup and in
Settings → Apps → Startup. **Software that appears in neither, but runs at
login, reads as malware.** So all of the following are mandatory, not optional
polish:

- A registered startup entry with a visible, correct publisher name.
- A proper entry in Installed Apps with a working uninstaller that offers to
  remove the contributor state directory (asking first — that directory holds
  the device key and the contribution history).
- A real window with a taskbar identity, not a trayless background process.
- An Authenticode signature. An unsigned binary that launches at login and
  reads your source directories will be quarantined by users' own instincts,
  correctly.

## Technology

- **WinUI 3**, C#, via P/Invoke to the C ABI. The interop layer is one file;
  strings marshalled from `char*` are copied and then freed with
  `tc_string_free`, never held.
- `tc_subscribe` invokes its callback from a Rust thread; the interop layer
  marshals onto the UI dispatcher queue. The delegate must be rooted for the
  lifetime of the subscription or the GC will collect it and the callback will
  crash — this is the classic P/Invoke callback bug and is worth a comment in
  the code.
- Toast notifications via `AppNotificationBuilder`, with exactly two buttons:
  `Review` and `Not now`. **No toast button uploads anything.**

## Autostart

A Startup entry registered through the packaged app's `StartupTask`, so it
appears in Settings → Apps → Startup where a user can disable it and expect
that to work. Offered at the end of onboarding, never silent.

## Packaging

MSIX, signed. Unpackaged deployment is out of scope: `StartupTask`, toast
identity, and the Installed Apps entry all work better packaged, and all three
are on the mandatory list above.

## Acceptance

The shared checklist, plus: the app appears in Task Manager → Startup with the
right publisher; uninstalling offers to remove the state directory and honours
the answer; toast actions are exactly Review and Not now; the app runs without
the named pipe and says clearly that CLI control is unavailable in that
configuration; and, once the pipe lands, a second unprivileged user on the same
machine cannot connect to it.
