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
- `placement.js`: where each window goes. No GNOME imports, so it runs
  under plain node.

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

Packaging it for extensions.gnome.org, or alongside the Flatpak, is not
done here.
