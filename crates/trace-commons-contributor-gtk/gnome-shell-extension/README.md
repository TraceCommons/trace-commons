# GNOME Shell extension: edge surfaces

GNOME has no wlr-layer-shell, and a GTK 4 window cannot place itself. This
extension lets the Linux shell show the Mac's edge rail and menu-bar panel
on GNOME: the app draws both as ordinary GTK windows, and the extension only
moves them, keeps them above other windows and shows them on every
workspace. It draws nothing and holds no words of its own.

- `metadata.json`: GNOME Shell 46 (Ubuntu 24.04) to 51. Its name and
  description are the app's own words; `src/edge/gnome.rs` tests that.
- `extension.js`: owns `ai.tracecommons.Contributor.EdgeSurfaces` on the
  session bus and exports `ai.tracecommons.Contributor.EdgeSurfaces1` at
  `/ai/tracecommons/Contributor/EdgeSurfaces`: `Place(s title, s role, i x,
  i y)`, `Release(s title)` and a `Version` property. A window is matched by
  the calling process's id and its title, and never a GtkApplicationWindow,
  so a caller can place only its own rail and panel.
- `placement.js`: where each window goes, and whether a rail that took
  focus gives it back. No GNOME imports, so `tests/placement.test.js` runs
  it under Node's own test runner in CI (`package.json` only tells Node
  the files are ES modules; it has no dependencies and is not packed).

What the extension does to a window it places:

- keeps it above other windows and on every workspace;
- for the rail, puts it against the primary monitor's right work-area
  edge, centred (the Mac's `EdgeRailGeometry.frame`); for the panel, puts
  it near the press, inside an 8-pixel margin;
- places it again when Mutter first shows it (whose own first placement
  would otherwise move it), when it changes size, and when the monitors or
  work areas change;
- lets the open rail keep keyboard focus once it is given, so Escape and
  Tab work, and never lets a closed rail keep it, as the Mac's
  non-activating panel never takes it (owner, 2026-11-18, "Keyboard only
  while open"): focus goes back only to a window on the active workspace
  and not minimised, or else to nothing;
- undoes all of this on `Release`, when the caller leaves the bus, and on
  `disable()`. GNOME Shell disables every extension on the lock screen and
  enables it again on unlock; the app's client (`src/edge/gnome.rs`)
  remembers what it last asked for each window and asks again when the
  name gets a new owner.

The app works without the extension: no rail, and the panel as a plain
window (`src/edge/capability.rs`). The app never asks anyone to install it.

## Licence

MIT OR Apache-2.0, like the rest of the shipped client. An extension runs
inside GNOME Shell, which is GPL-2.0-or-later; MIT is GPL-compatible, and
this directory imports only GNOME Shell's and GJS's public module APIs at
run time. No GPL code is copied into this repository.

## Trying it

    ln -s "$PWD/edge-surfaces@tracecommons.ai" \
      ~/.local/share/gnome-shell/extensions/edge-surfaces@tracecommons.ai
    # log out and in (Wayland), then
    gnome-extensions enable edge-surfaces@tracecommons.ai

## Packing

    crates/trace-commons-contributor-gtk/scripts/pack-gnome-shell-extension.sh [OUT_DIR]

writes `edge-surfaces@tracecommons.ai.shell-extension.zip` with exactly the
files the shell loads (a test in `src/edge/gnome.rs` holds the list to the
imports): the file extensions.gnome.org takes and `gnome-extensions
install` reads. CI packs it on every run. A Flatpak cannot install an
extension into the host's shell, and neither the Flatpak nor the installer
offers it: offering it needs words, and an offer is a notice, so that is
the owner's call.

## Against a real shell

    crates/trace-commons-contributor-gtk/scripts/gnome-shell-extension-verify.sh

starts a headless GNOME Shell (46 or later) on a private bus, with its own
runtime, data and settings directories, enables this extension and a
throwaway one that turns on unsafe mode so `org.gnome.Shell.Eval` can read
Mutter's answer back, and runs the ignored
`edge::gnome::live_tests::a_running_shell_holds_the_rail_and_the_panel`
with the app's real client: the rail closed, open, and taller than the work
area; the panel near a press; focus given back from the rail; disable
undoing everything; re-enable and the client's replay; `Release`. Run on
GNOME Shell 51.1 (Arch), and on GNOME Shell 46.0 (Ubuntu 24.04), where it
failed until the extension placed a window again once shown. Shell 47 to
50 are listed in `metadata.json` on the strength of the APIs used, all
present since 45.
