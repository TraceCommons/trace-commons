# Linux platform integration — second pass

Date: 2026-08-09
Branch: `linux-shell`
Crate: `crates/trace-commons-contributor-gtk`
Reads with: `docs/superpowers/plans/linux-shell-report.md` (the first pass:
queue, preview, history, settings, and the process-model split) and
`docs/superpowers/specs/2026-08-08-contributor-shell-linux-design.md` (the
spec this pass builds against).

## What this pass built

The three Linux-specific integration points the first pass explicitly left
undone, in the order the brief gave them, plus a Flatpak manifest.

### 1. `org.freedesktop.portal.Background` registration — `src/portal.rs`

`portal::spawn_request()` is called once, from `ui::App::build`, on its own
thread. It calls `RequestBackground` on
`org.freedesktop.portal.Desktop` → `org.freedesktop.portal.Background` with
`reason` (the fixed string `copy::PORTAL_BACKGROUND_REASON`) and
`autostart: false`, then waits on the returned request object's `Response`
signal so the round trip is genuinely exercised rather than declared done
the moment the dialog opens.

`autostart` is deliberately `false`. The portal *can* register its own
autostart entry, but this app already has an answer to "how does this start
at login" — the systemd-unit-or-XDG-entry choice below, with its own
"never both" rule — and turning the portal's autostart option on too would
be a third mechanism. (A Flatpak build is the one place this tradeoff
inverts, because a confined app cannot write `~/.config/autostart` itself;
noted as an open question under the Flatpak section below, not resolved
here.)

Every failure path — no portal at all, a portal without `Background`
installed, a contributor declining the dialog — is caught in one place and
logged with a fixed label (`"background portal unavailable"`), never the
underlying D-Bus error text, which is not guaranteed to be free of detail
this repo's logging conventions don't allow. Nothing here can block or fail
startup: `spawn_request` returns immediately and the window opens on the
same tick it always did.

### 2. Autostart detection — `src/autostart.rs`, wired into Settings

`autostart::detect()` reads two well-known paths and returns one of:

- `Mechanism::SystemdUnit` — `~/.config/systemd/user/trace-commons-contributor.service`
  exists, meaning `trace-commons-contributor daemon install` has been run.
- `Mechanism::XdgEntry { enabled }` — no unit was found, and `enabled`
  reflects whether this app's own
  `~/.config/autostart/ai.tracecommons.Contributor.desktop` currently
  exists.

Settings (`ui/settings.rs`) renders exactly one of two things, never both:
a static sentence naming the service when the unit is detected (with the
switch row hidden entirely, not disabled — there is nothing for this app to
toggle), or a working switch that calls
`autostart::enable_xdg_entry()` / `disable_xdg_entry()` when no unit is
present. This was screenshotted in both states in the container (see
Verification) to confirm the branch, not just the code path, is real.

`detect()` never writes anything, and this module never touches the
systemd unit file at all — install/uninstall stay
`trace-commons-contributor daemon install|uninstall`, so there remains
exactly one writer for that path. The unit's filename is duplicated here as
a constant (`SYSTEMD_UNIT_FILE_NAME`) rather than imported, because
`daemon::install` keeps both the filename and a presence check private —
see Contract note below.

### 3. `StatusNotifierItem` tray — `src/tray.rs`, bonus only

`tray::spawn()` exports a minimal `org.kde.StatusNotifierItem` object at
`/StatusNotifierItem` (`Category`, `Id`, `Title`, `Status`, `IconName` by
name, never a path) and calls
`org.kde.StatusNotifierWatcher.RegisterStatusNotifierItem`. Its entire
vocabulary is one signal: `Activate`, `SecondaryActivate`, and
`ContextMenu` (no menu is exported, so a host asking for one gets nothing)
all raise the window at the queue and do nothing else — the same
one-capability rule `notify.rs` already holds for notification actions, for
the same reason: a surface reachable when nobody is looking at the window
must have the smallest possible vocabulary.

Absence of a watcher (plain GNOME, the majority case) is caught, logged
with the fixed label `"tray unavailable"`, and changes nothing else. There
is no code path anywhere that tells a contributor to install a shell
extension to get it back.

### 4. Flatpak manifest — `crates/trace-commons-contributor-gtk/flatpak/`

`ai.tracecommons.Contributor.yml` and the launcher `.desktop` file. The
`finish-args` grant exactly `~/.claude/projects:ro` and
`~/.codex/sessions:ro` — the daemon's actual default session roots
(`trace-commons-contributor/src/source/mod.rs`) — plus display, IPC, and
session-bus sockets. No `--filesystem=home`. The design spec's exact
onboarding wording for why is pinned as
`copy::FLATPAK_SESSION_ROOTS_EXPLANATION`, unused today because onboarding
does not exist yet (unchanged from the first pass's finding).

**This manifest is unbuilt**, and says so in its own header comment. It was
not run through `flatpak-builder` anywhere: the container this crate
otherwise builds and runs in has no `flatpak-builder`, no Flatpak runtimes,
and no `flatpak` command; the macOS host cannot build Flatpaks at all. What
would actually validate it, on a real Linux machine:

```
flatpak install org.gnome.Platform//46 org.gnome.Sdk//46 \
  org.freedesktop.Sdk.Extension.rust-stable//46
pip install aiohttp toml
python3 flatpak-cargo-generator.py \
  crates/trace-commons-contributor-gtk/Cargo.lock \
  -o crates/trace-commons-contributor-gtk/flatpak/cargo-sources.json
flatpak-builder --user --install build-dir \
  crates/trace-commons-contributor-gtk/flatpak/ai.tracecommons.Contributor.yml
```

`cargo-sources.json` does not exist in this pass — generating it needs
network access to resolve checksums for the full dependency tree, which
this environment does not have for that specific tool, and fabricating one
by hand would risk silently wrong checksums, which is worse than the gap
being visible. The manifest's `sources` list references it with
`only-arches: []`, so a `flatpak-builder` run today fails immediately and
legibly at that missing file rather than attempting a network build that
would defeat the point of a reproducible Flatpak.

A known, unresolved limitation of the Flatpak build specifically (not the
native one): a contributor who points the daemon at a non-default
`claude_root` / `codex_root` (Settings already shows these as
configured-or-not, never as a path, per the no-paths rule) would need a
matching custom `--filesystem` grant this manifest cannot predict. The
tarball distribution the design spec also asks for has no such limit,
because it is not confined; this pass did not build the tarball packaging
either — see Not done.

## New dependencies

Both confined to this crate, and both were already resolved transitively
before this pass (`dirs` via `trace-commons-contributor`, `zbus` via
`notify-rust`), so declaring them as direct dependencies added **zero new
packages** to `Cargo.lock` — only two new direct edges. Verified by diffing
`Cargo.lock` package counts before and after: unchanged.

- `dirs` 6 — the same crate the daemon uses to resolve `~/.config`, reused
  here to find the systemd unit and the XDG autostart directory the same
  way.
- `zbus` 5 (default features, `zbus::blocking`) — the only raw D-Bus surface
  needed for the portal call and the tray's `StatusNotifierItem` object
  server; there was no higher-level binding for either already in the tree.

The contributor crate gained nothing; `Cargo.toml`/`Cargo.lock` under
`crates/trace-commons-contributor` and `crates/trace-commons-contributor-ffi`
are untouched.

## Contract note (not a gap; reported per the brief anyway)

`trace-commons-contributor::daemon::install` keeps `UNIT_FILE_NAME` and
`unit_path()` private, and there is no public "is a unit installed"
query. `autostart.rs` duplicates the filename as a constant with a comment
pointing at the source of truth, rather than adding one to the frozen
crate. This is a minor seam, not a contract gap in the sense the first
pass's four findings were: detecting a file at a documented, stable path
convention does not need the writer's cooperation, and nothing about the
detection is speculative. Flagged here only because the brief asked for
anything found to be reported rather than silently worked around.

## Verification

Everything below ran in the container via `scripts/linux-build.sh`; nothing
was reasoned about instead of run.

| Check | Command | Result |
|---|---|---|
| Builds | `scripts/linux-build.sh "cargo build"` | clean |
| Builds, warnings-as-errors | `scripts/linux-build.sh "RUSTFLAGS='-D warnings' cargo build --all-targets"` | clean |
| Formatting | `scripts/linux-build.sh "cargo fmt --check"` | clean |
| Lints | `scripts/linux-build.sh "RUSTFLAGS='-D warnings' cargo clippy --all-targets"` | clean, no allow-list |
| Unit tests | `scripts/linux-build.sh "RUSTFLAGS='-D warnings' cargo test"` | 16 passed (11 before this pass, 5 new — all in `autostart::tests`) |
| Attached run, portal + tray both absent | `scripts/linux-build.sh --run-headless` | started, realized, both logged unavailable and continued, screenshot taken |
| Hosting run, portal + tray both absent | `scripts/linux-build.sh "bash .../hosted-run.sh"` | same |
| Settings, no systemd unit | headless run with `--start-page settings` | switch row visible, off, correct copy |
| Settings, systemd unit present | same, with a unit file planted at the container's `~/.config/systemd/user/trace-commons-contributor.service` first | switch row hidden, static sentence shown instead |
| Host workspace unaffected | `RUSTFLAGS='-D warnings' cargo check -p trace-commons-contributor --bins` on macOS | clean, exit 0 |

### What this evidence actually proves, and what it does not

The headless runs prove three concrete things about the portal and tray
code, not just that it compiles: under `dbus-run-session` (a real, private
session bus with no portal service and no `StatusNotifierWatcher` on it),
both `RequestBackground` and `RegisterStatusNotifierItem` were actually
called, both actually failed the way absence looks (`ServiceUnknown`-style
D-Bus errors), both were caught in exactly the places this report says, and
the process kept running and quit cleanly afterward — the console lines
`"trace-commons-shell: background portal unavailable"` and
`"trace-commons-shell: tray unavailable"` are direct evidence of the catch
path executing, not an assumption about it. That is real coverage of "does
this fail to start when the desktop has neither," which was the literal
acceptance bar in the brief for the portal.

It does **not** prove three things a real desktop is needed for:

- That the portal's own permission dialog renders correctly, that
  `RequestBackground` returns the answer GNOME's Background Apps panel then
  actually reflects, or that the "quit from the panel" affordance the
  design spec describes works end to end. This build does not change
  `adw::Application`'s flags from `NON_UNIQUE`, so this application is not
  D-Bus-activatable; GNOME Shell's own "Quit" button in that panel is
  understood to rely on D-Bus activation in at least some
  `xdg-desktop-portal-gnome` versions. Whether the portal grants the
  "pause/quit surface" framing full credit without that is genuinely
  untested and not verifiable from this container. Flagging this now
  rather than presenting the portal call as the whole of that promise.
- That a tray icon actually renders, actually looks like the app, or that
  clicking it in a live KDE, Cinnamon, or XFCE panel raises the window.
  Only the D-Bus registration call and its failure path were exercised;
  nothing here has run against a real `StatusNotifierWatcher`.
- That the autostart switch, once flipped on a real desktop, actually
  starts the app at the next login. Only the file-write/detect round trip
  was verified, in the container and via unit tests with an injected
  directory (`autostart::tests`, five new tests) — never a real login
  session.

Whoever picks this up next needs a real GNOME session to check the
Background Apps panel entry and its quit button, a real KDE or XFCE session
to check the tray icon actually appears and is clickable, and a real login
cycle to check the XDG autostart entry actually fires — none of which a
container can stand in for.

## Not done, and knowingly so

- **Onboarding** still does not exist, so `FLATPAK_SESSION_ROOTS_EXPLANATION`
  and the portal's own dialog text are the only places the "why we're
  asking" wording appears; nothing in this window explains the background
  portal request before it happens. Same finding as the first pass, just
  now with one more string waiting on it.
- **The tarball distribution** the design spec asks for alongside Flatpak
  ("a plain tarball with the binary and unit file for people who want
  neither Flatpak nor a desktop") was not built this pass.
- **Making the app D-Bus-activatable**, which the GNOME Background Apps
  panel's quit affordance may depend on — see the verification caveat
  above. Changing `ApplicationFlags` away from `NON_UNIQUE` is a real
  behavior change (single-instance enforcement) that deserves its own
  decision, not a silent side effect of wiring the portal.
- **A context menu on the tray icon.** `ContextMenu` currently just raises
  the window, the same as a click. A real `com.canonical.dbusmenu` menu
  (Pause, Quit) is a separate, larger D-Bus surface and was judged not
  worth it for a bonus affordance whose whole job, per the spec, is to not
  be load-bearing.
- **`cargo-sources.json`** for the Flatpak build — see above.
- The four contract gaps the first pass found (preview body unreachable
  when attached, per-entry preview cost on the queue row, "what's in it"
  fields missing, no withdraw method) are unchanged; nothing in this pass
  touched `trace-commons-contributor` or `-ffi`, per the brief.
