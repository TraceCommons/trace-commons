# Omarchy palettes, vendored as test data

These `colors.toml` files are copied byte for byte from Omarchy
(https://github.com/omacom/omarchy), which is MIT-licensed; its licence
text is `LICENSE` in this directory. They are test inputs for the palette
adapter in `src/ui/omarchy.rs` and are not shipped in the app.

| File | Upstream path | Commit |
| --- | --- | --- |
| `tokyo-night.colors.toml` | `themes/tokyo-night/colors.toml` | `quattro` @ `3437cfb482d603a34c1292db2d425cacd0f80281` (2026-11-17) |
| `catppuccin-latte.colors.toml` | `themes/catppuccin-latte/colors.toml` | same |
| `rose-pine.colors.toml` | `themes/rose-pine/colors.toml` | same |
| `gruvbox.colors.toml` | `themes/gruvbox/colors.toml` | same |
| `flexoki-light.colors.toml` | `themes/flexoki-light/colors.toml` | same |
| `legacy-tokyo-night.colors.toml` | `themes/tokyo-night/colors.toml` | `master` @ `f4378f0de5b44d331ee943746a97872b718a6c18` (2026-08-14), the pre-4.x ANSI `colorN` format |

`LICENSE` is `LICENSE` from the same `quattro` commit. Refresh by
re-downloading from a named commit, never by hand edits.
