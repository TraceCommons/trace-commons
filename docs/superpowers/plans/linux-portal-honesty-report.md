# Linux background-portal honesty: detect and say so

## The problem

`portal.rs` requested `org.freedesktop.portal.Background` at startup and
dropped the outcome on the floor except for a fixed `eprintln!`. On a
desktop with no `Background` portal backend (XFCE, Cinnamon, MATE, Budgie,
any wlroots compositor -- see below), the request is a silent no-op and the
contributor is never told. Silent no-op is the failure mode this product
rejects everywhere else.

## What actually determines persistence

The portal is not what keeps the process alive on any desktop; `systemd
--user` is (with `loginctl enable-linger` needed to survive logout, which
no portal can do either). The portal's real job on GNOME/Plasma is
registering in their own "Background Apps" UI and not being flagged as a
rogue background process. So the honest message on a backend-less desktop
depends on **both** signals, not just the portal:

| Backend `state` | Systemd unit installed | Message says |
|---|---|---|
| Present | true | Registered; systemd is what actually keeps it running |
| Present | false | Registered; that alone doesn't persist past login -- use the switch above |
| Absent | true | No backend to register with, but **nothing is wrong** -- systemd is doing the real work |
| Absent | false | No backend and no systemd unit -- genuinely weaker; told how to fix it |
| Unknown | true | Couldn't tell; systemd is doing the real work either way |
| Unknown | false | Couldn't tell; no systemd unit, so it only runs while the window is open |

`Unknown` never asserts `Present` or `Absent` -- see `copy::portal_status_line`
and its tests in `src/copy.rs`.

## How the probe works

`portal::spawn_request()` (renamed from a fire-and-forget `void` function to
one that returns `async_channel::Receiver<BackendState>`) reuses the
**one** real `RequestBackground` call the app already makes at startup --
it does not issue a second D-Bus round trip, so a backend-less desktop is
never asked twice for the same thing and a backend that exists never shows
a second permission dialog just to be probed.

The outcome is classified in `portal::classify`:

- `Ok(())` -- some backend fielded the call -> `Present`.
- `Err` whose `zbus::Error` downcasts to `MethodError` with a name in the
  "nothing here answers for this" family -> `Absent`.
- Any other error (no session bus, a non-D-Bus failure, etc.) -> `Unknown`.

The classification is bounded: the outer thread waits at most
`PROBE_TIMEOUT` (5s) on a `std::sync::mpsc::channel` fed by an inner thread
doing the actual blocking zbus call, and falls back to `Unknown` if nothing
arrives in time. If the real request is still sitting on a permission
dialog past that bound, the inner thread is simply left running (same as
this module already did before this change) -- the UI never blocks on it.

### The `Absent` error-name list, verified empirically

The task briefing named `ServiceUnknown`/`NameHasNoOwner` as the
"no backend" class. I did not take that on faith -- I built a real
`xdg-desktop-portal` + `xdg-desktop-portal-gtk` (no `Background` impl) in a
`debian:bookworm` container over a session D-Bus with no display, and called
`RequestBackground` directly:

```
--- calling RequestBackground ---
Error: GDBus.Error:org.freedesktop.DBus.Error.UnknownMethod: No such interface
"org.freedesktop.portal.Background" on object at path /org/freedesktop/portal/desktop
```

That is **not** `ServiceUnknown` or `NameHasNoOwner` -- it's `UnknownMethod`
(the frontend itself is running; it just never exported the interface
because no installed backend implements it). So `is_absent_backend_error`
in `portal.rs` classifies four names as "no backend", not two:

- `org.freedesktop.DBus.Error.UnknownMethod`
- `org.freedesktop.DBus.Error.UnknownInterface` (the same family; not
  independently observed, but the same "no such thing to answer this"
  reasoning applies if the frontend exports the interface without the
  method for some portal version)
- `org.freedesktop.DBus.Error.ServiceUnknown`
- `org.freedesktop.DBus.Error.NameHasNoOwner`

The latter two cover the rarer case where `xdg-desktop-portal` itself isn't
running at all (no auto-activation available), which the empirical test
above did not exercise directly but which is the well-documented D-Bus
behavior for a missing well-known name.

I also attempted the `Present` case (real GNOME backend, `xdg-desktop-portal
+ xdg-desktop-portal-gnome`, no display) in the same style of container. It
did not produce a conclusive transcript in the time available -- the GNOME
backend appears to hang waiting on machinery this headless container
doesn't have (no display, no shell), rather than returning quickly one way
or the other. I did not chase this further because the `Present` branch of
`classify` is the trivial one (`Ok(())` -> `Present`, no error-name
parsing involved) and is exercised directly by
`portal::tests::a_successful_call_is_present`.

## Where it's surfaced

`ui/settings.rs`, in the existing "Starting automatically" card, right
below the systemd/XDG-autostart explanation that was already there --
same topic (what keeps this process going), same card, no new section
heading. `App::build` spawns the probe once and hands the receiver to the
new `settings::wire_background_probe`, which renders
`copy::PORTAL_STATUS_CHECKING` immediately and swaps in the real sentence
once the classification lands. `render_background` reads the systemd
signal live from `autostart::detect()` (the same function `render_autostart`
already uses -- no duplicated detection) and the portal signal from a
`RefCell<Option<BackendState>>` on `SettingsView`.

## Files touched

- `crates/trace-commons-contributor-gtk/src/portal.rs` -- `BackendState`,
  `classify`, `is_absent_backend_error`, bounded `spawn_request`, 4 new
  tests.
- `crates/trace-commons-contributor-gtk/src/copy.rs` -- `PORTAL_STATUS_CHECKING`,
  `portal_status_line` (the 6-cell matrix), 2 new tests.
- `crates/trace-commons-contributor-gtk/src/ui/settings.rs` -- new
  `background_body` label + `background_state` field on `SettingsView`,
  `wire_background_probe`, `render_background`.
- `crates/trace-commons-contributor-gtk/src/ui/mod.rs` -- `App::build` now
  captures `spawn_request()`'s receiver and wires it through
  `settings::wire_background_probe`.

No new dependencies -- `zbus` and `async-channel` were already direct
dependencies of this crate.

## Verification

This crate cannot build on this macOS host (`gtk4`/`glib-sys` need
`pkg-config` + system GTK, not present here). Verified instead in a
`rust:1-bookworm` container with `libgtk-4-dev libadwaita-1-dev
libdbus-1-dev pkg-config` installed (GTK 4.8.3), mounting this worktree at
`/work`:

```
cargo build                                 -> Finished, clean
cargo clippy --all-targets                  -> clean, zero warnings
cargo fmt --check                           -> clean, no diff
cargo test                                  -> 23 passed; 0 failed
```

Baseline before this change was 17 tests; this change adds 6 (4 in
`portal.rs`, 2 in `copy.rs`) for 23, no regressions.

## What I could not verify

- The literal `Present` transcript for a real GNOME/KDE backend replying
  to `RequestBackground` (attempted in a headless GNOME-backend container;
  it hung rather than answering, and I did not have budget to build a full
  display + shell to chase it further). The `Present` branch of `classify`
  itself needs no error-name parsing, so this gap is about the container
  setup, not about untested classification logic.
- Real end-to-end behavior of the Settings row against a live GNOME or
  Plasma session (no windowed desktop available in this environment) --
  the build/clippy/fmt/test loop above is what's verified; the row was not
  visually inspected running.
