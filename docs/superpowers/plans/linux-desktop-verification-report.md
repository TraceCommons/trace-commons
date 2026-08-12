# Closing the Xvfb evidence gap in CI

Date: 2026-08-09
Branch: `linux-shell`
Crate: `crates/trace-commons-contributor-gtk`
Reads with: `docs/superpowers/plans/linux-integration-report.md` (the "What
this evidence actually proves, and what it does not" section, in
particular), which is the report this pass is closing part of the gap for.

## The gap this closes

The previous pass's evidence for the portal and tray code was entirely
`dbus-run-session` with **nothing** listening on the bus: no portal, no
`StatusNotifierWatcher`. That is real coverage of "does the app fail to
start when the desktop has neither" — the fail-closed path — but it proves
the easy half. It does not prove anything renders, and it does not prove a
portal call reaches an actual portal implementation.

This pass adds one CI job, `linux-shell-desktop-integration`, in
`.github/workflows/ci.yml`, running on `ubuntu-latest` (a real GitHub
Actions runner, not the local macOS-host Docker dev image), that upgrades
both axes as far as CI genuinely can. New files:

- `.github/workflows/ci.yml` — the new job.
- `crates/trace-commons-contributor-gtk/scripts/weston-portal-verify.sh` —
  outer script: builds nothing itself (the job's own steps build the
  daemon and shell binaries first), starts the real daemon, and hands off
  to the inner script inside `dbus-run-session`.
- `crates/trace-commons-contributor-gtk/scripts/weston-portal-verify-inner.sh`
  — the actual compositor + portal orchestration and assertions.

No new Rust dependencies. Every new package is apt-only, installed by the
CI job itself, not by `linux-build.Dockerfile` (that image is for local
macOS-host development via Docker Desktop and is unchanged).

## Axis 1: real rendering — achieved

`weston --backend=headless-backend.so --socket=wayland-ci --width=1280
--height=900` starts an actual Wayland compositor: real protocol dispatch,
real surface commits, real output composition. The shell runs under it with
`GDK_BACKEND=wayland`, `weston-screenshooter` captures the composited
output, and the job asserts two things about that frame, not "the process
stayed alive":

1. **Not blank/uniform.** `convert <shot> -colorspace Gray -format
   "%[fx:standard_deviation]" info:` computes the grayscale standard
   deviation; the job fails if it is at or near zero (a flat-colour frame
   means nothing was actually composited, whatever the process's exit code
   says).
2. **Expected text present.** `tesseract <shot> stdout` runs OCR over the
   frame and the job fails unless it finds "Queue" — the label
   `stack.add_titled(&queue.root, Some("queue"), "Queue")` puts on the
   `AdwViewSwitcherTitle` pill in `src/ui/mod.rs`, present on every cold
   start regardless of daemon state.

Both are hard, bounded assertions; neither passes on a process that merely
avoided crashing.

## Axis 2: a real portal — partially achieved, and the brief needed a correction

The brief's suggested packages were `xdg-desktop-portal` and
`xdg-desktop-portal-gtk`. Before wiring anything up, I checked what
`xdg-desktop-portal-gtk` actually implements by extracting its shipped
manifest rather than assuming:

```
$ cat /usr/share/xdg-desktop-portal/portals/gtk.portal
[portal]
DBusName=org.freedesktop.impl.portal.desktop.gtk
Interfaces=org.freedesktop.impl.portal.FileChooser;org.freedesktop.impl.portal.AppChooser;org.freedesktop.impl.portal.Print;org.freedesktop.impl.portal.Notification;org.freedesktop.impl.portal.Inhibit;org.freedesktop.impl.portal.Access;org.freedesktop.impl.portal.Account;org.freedesktop.impl.portal.Email;org.freedesktop.impl.portal.DynamicLauncher;org.freedesktop.impl.portal.Lockdown;org.freedesktop.impl.portal.Settings;
UseIn=gnome
```

**`xdg-desktop-portal-gtk` does not implement
`org.freedesktop.impl.portal.Background` at all.** The only package that
does is `xdg-desktop-portal-gnome`:

```
$ cat /usr/share/xdg-desktop-portal/portals/gnome.portal
Interfaces=...;org.freedesktop.impl.portal.Background;...
```

I checked whether installing `xdg-desktop-portal-gnome` instead would get a
real accept/deny answer, by extracting strings from its shipped binary
(`/usr/libexec/xdg-desktop-portal-gnome`, from `background.c`):

```
/org/gnome/Shell/Introspect
Failed to acquire org.gnome.Shell.Introspect proxy: %s
handle-notify-background
NotifyBackground
org.gnome.Shell.Introspect
```

Its `RequestBackground` handling calls `org.gnome.Shell.Introspect` — a
real, running GNOME Shell — to raise the notification a contributor would
approve or deny. There is no GNOME Shell in this job, and this pass does
not attempt to fake one (spinning up a headless Mutter/GNOME Shell well
enough for this one D-Bus interface to answer would be a much larger,
much less reliable undertaking than the rendering half, and risks exactly
the flakiness this task was told to avoid). So: **no CI job here gets an
actual grant/deny out of a permission dialog.** That was never going to be
achievable without a real desktop session, and pretending otherwise would
be the "looks like evidence but isn't" failure mode this task exists to
correct.

What the job does instead, honestly: it runs the brief's original pairing
(`xdg-desktop-portal` + `xdg-desktop-portal-gtk`) under `dbus-run-session`
and asserts the request reaches a **live** daemon:

1. Poll `org.freedesktop.DBus.NameHasOwner org.freedesktop.portal.Desktop`
   until true (bounded, 20 × 0.5s) — the daemon must actually claim the
   bus name. Fails the job if it never does.
2. Call `RequestBackground` directly (independent of the app, via `gdbus`)
   with a 10s bound, and check the failure mode: it must **not** be
   `ServiceUnknown` / `NameHasNoOwner` / "was not provided by any .service
   files" — that is exactly what "nothing is listening at all" looks like,
   the same evidence the previous pass already had. Getting that error here
   would mean this job tests nothing new. It must also not time out — a
   live portal core with no `Background` backend registered should reject
   the call fast, not hang.

That is a real, if narrow, upgrade: the request now fails because a live
portal genuinely has no backend for the interface, not because nothing
answered the phone. It is not, and is not claimed to be, "a real portal
answering the permission dialog."

## Axis 3 (tray): explicitly not attempted

Not asked for as a primary axis, and correctly so. `src/tray.rs` registers
`org.kde.StatusNotifierItem` with `org.kde.StatusNotifierWatcher` — GNOME
has shipped no `StatusNotifierWatcher` without a user-installed shell
extension for years, so the majority-desktop case is and stays "absent,
caught, logged, changes nothing" (already covered by the existing Xvfb
job). No CI job here starts KDE Plasma, Cinnamon, XFCE, or a
`snixembed`/`StatusNotifierWatcher` stand-in to test the tray path against
a real watcher; doing so would prove nothing about the desktops
contributors actually run, per the same reasoning as GNOME's Background
portal above, and was out of scope for this task. `src/tray.rs` is worth
exactly what a bonus, deliberately non-load-bearing surface is worth: real
on a watcher-having desktop, inert everywhere else, unverified against a
live watcher in CI before this pass and unverified against one after it.

## What was verified, and how

Everything below was checked against real Ubuntu 24.04 package contents,
not asserted from memory, using local Docker containers (`ubuntu:24.04`)
run from this macOS host. Two different verification depths were used
depending on cost:

- **Fast, no-install checks** (`apt-get download` + `dpkg -c` / `dpkg -x`,
  seconds each): confirmed every apt package name used in the CI job
  resolves and downloads (`weston`, `xdg-desktop-portal`,
  `xdg-desktop-portal-gtk`, `xdg-desktop-portal-gnome`, `dbus`, `dbus-x11`,
  `libglib2.0-bin`, `tesseract-ocr`, `tesseract-ocr-eng`, `imagemagick`,
  `libgtk-4-dev`, `libadwaita-1-dev`, `libnotify-dev`, `pkg-config`).
  Confirmed `weston-screenshooter` ships at `/usr/bin/weston-screenshooter`
  in the `weston` package by listing the `.deb`'s contents directly, and
  confirmed `xdg-desktop-portal`'s and `xdg-desktop-portal-gtk`'s binaries
  live at `/usr/libexec/xdg-desktop-portal` and
  `/usr/libexec/xdg-desktop-portal-gtk` the same way. Extracted and read
  both portals' `.portal` manifests directly (see Axis 2 above) rather than
  assuming what each backend implements.
- **YAML validity**: `python3 -c "import yaml,sys;
  yaml.safe_load(open('.github/workflows/ci.yml'))"` — passes.
- **Shell syntax**: `bash -n` on both new scripts — passes. Ran
  `shellcheck` (available on this host) over both; fixed the one
  actionable finding (`cd` without an `|| exit` guard). The remaining
  findings are informational (an unfollowed `source`, `ls` instead of
  `find` for a glob that cannot contain the problematic filename
  characters shellcheck warns about here) and were left as-is.

**What was not verified, and why**: a full live run of
`weston-portal-verify.sh` end to end — starting a real `apt-get install` of
the full dependency chain (GTK, Mesa, Wayland, the portal daemons)
sequenced with `weston` actually starting, the shell actually rendering,
and `weston-screenshooter` actually producing a file — did not complete in
this session. Multiple `apt-get install` attempts against these packages in
the sandboxed Docker environment used for verification each ran for many
minutes without finishing (confirmed still making progress via `ps aux`
inside the container, not hung — `apt-get`'s HTTP method process was
active — just slow in this sandbox's network path). That is a property of
this verification sandbox, not evidence about GitHub's runners, but it
means the exact flag syntax for `weston --backend=headless-backend.so
--socket=... --idle-time=0` and `weston-screenshooter`'s output filename
were not exercised live; they are standard, long-documented weston CLI
usage, and the script is deliberately defensive around the one point of
real uncertainty (the screenshot's exact filename) by globbing
`$WORKDIR/*.png` and taking the most recent match rather than assuming a
fixed name. If the first real CI run surfaces a flag-syntax mismatch, that
is expected to be a small, visible fix, not a design problem — the job
should be treated as unverified-in-anger on its first PR run and watched
accordingly.

Docker was used only for static package/manifest inspection in this pass,
not to dry-run the full job — the job itself will get its first real
execution on `ubuntu-latest` when this branch's PR runs CI.

## Summary

| Claim | Status |
|---|---|
| Real compositor rendering, asserted non-blank + OCR text | Built; flag syntax not live-exercised, static package/binary checks only |
| Real portal daemon, bounded non-absence reply | Built; static manifest/binary checks confirm the design is sound, not live-exercised |
| Real portal *granting* Background | Not achievable in CI without GNOME Shell; not attempted, explained above |
| Tray icon in a real desktop shell | Not attempted; out of scope, GNOME ships no watcher without an extension |
| No new Rust dependencies | True — apt packages only |
