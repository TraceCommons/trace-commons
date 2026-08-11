# Contributor shell — Linux

Date: 2026-08-08
Status: approved for planning
Scope: sub-project 4. Platform mechanics only.
Reads with: `2026-08-08-contributor-shell-shared-design.md`.

## Shape — and why Linux genuinely differs

**Do not build this tray-first.** GNOME, the majority Linux desktop, has no
system tray without a user-installed extension. A tray-first design would be
invisible to most of the people it shipped to. This is the one place where
painting the same design three times would be actively wrong.

So on Linux:

- **The window is the primary surface**, and **notification actions do the
  work the tray menu does elsewhere**. libnotify actions are reliable across
  desktops in a way the tray is not.
- **The tray is a bonus**, via `StatusNotifierItem` where it is real (KDE,
  Cinnamon, XFCE, GNOME with the extension installed). The app must be fully
  usable when it is absent, and must not tell the user to install an extension.
- **Register with the XDG Background portal**
  (`org.freedesktop.portal.Background`) so the app appears in GNOME's
  Background Apps menu. That is where a GNOME user looks for something like
  this, and it doubles as the pause and quit surface for users who never see a
  tray.

## Process model — the inverse of macOS

Linux is where headless and SSH-only contributors actually are, and where the
systemd user unit already exists. So here the **separate daemon is the primary
deployment**, not the fallback:

- `trace-commons-contributor daemon run` under the existing systemd user unit
  is the normal way to run. `daemon install` already writes it.
- The GTK application is an **optional client** over the control socket. It
  may also host the loop itself (`tc_daemon_start`) for a user who wants only
  the app, and the existing exclusive lock arbitrates: whoever gets the lock
  runs the loop, the other connects.
- **CLI parity matters most on this platform.** Any capability reachable only
  through the GUI is a capability a headless contributor does not have. The
  shared spec's flows must all have CLI equivalents, which they do.

## Technology

- **GTK 4** with libadwaita, in Rust via `gtk4-rs`, linking the core crate
  directly rather than through the C ABI — on this platform the shell and the
  core are both Rust, so the FFI layer buys nothing. (The C ABI remains what
  macOS and Windows use.)
- Notifications through `libnotify`, or `Gio.Notification` where the portal is
  available.

## Autostart

Two paths, and the app should prefer the first:

1. The systemd user unit, if `daemon install` has been run — the app detects
   it and does not offer a second mechanism.
2. Otherwise an XDG autostart desktop entry for the app itself, offered at the
   end of onboarding with the same wording as the other platforms.

Never both. Two autostart mechanisms for one product is how people end up with
two running copies and a confusing lock error.

## Packaging

Flatpak is the realistic distribution channel, and it constrains the design:
a Flatpak-confined app cannot read `~/.claude/projects` without a filesystem
permission. The manifest requests read access to the session roots
specifically, not `home`, and the app explains why at onboarding:

> Trace Commons needs to read your Claude Code and Codex session files. It
> asks for access to those folders only.

Also ship a plain tarball with the binary and unit file for people who want
neither Flatpak nor a desktop.

## Acceptance

The shared checklist, plus: the app is fully usable on stock GNOME with no
tray present; notification actions Review and Not now both work under
GNOME and KDE; the app detects an already-running systemd daemon and connects
rather than failing on the lock; a headless machine can do everything the GUI
can from the CLI; the Flatpak build can actually read the session roots.
