# Linux blur spike (S1, S2)

Spike for `docs/superpowers/specs/2026-11-16-linux-mac-window-and-map-design.md`
(draft PR #2159), spikes S1 (translucency and KDE blur) and S2 (growing a mapped
window at its right edge). Branch `linux-blur-spike`. **Not for merge.** It
exists to record what was tried, what was observed and on which machine, and
which crates the blur request needs.

## What is on the branch

- `crates/trace-commons-contributor-gtk/examples/blur_spike.rs`: a self-contained
  GTK 4 window with a sidebar (veil fill), three cards (tier fill) and a hidden
  640 px pane. It contains no product wording; its only literal is the title
  `blur_spike`.
- The cargo feature `blur-spike`, off by default. It turns on five Linux-only
  optional crates and is the `required-features` of the example. Default builds,
  CI and the flatpak build do not compile any of it. `Cargo.lock` gains 18
  packages (listed below) because a lockfile records optional dependencies;
  `flatpak/cargo-sources.json` was **not** regenerated, since the branch is
  not for merge.
- No `unsafe`: the crate keeps `unsafe_code = "forbid"`. Every FFI step is
  inside `gdk4-wayland`, `gdk4-x11`, `wayland-client` and `x11rb`.

Environment knobs (developer-only, read by the example):

| Variable | Values | Effect |
| --- | --- | --- |
| `TC_BLUR_SPIKE_MODE` | `auto` (default), `blur-off`, `painted` | `auto` is translucent only where blur was bound, else painted. `blur-off` is translucent with no blur request, for comparison. |
| `TC_BLUR_SPIKE_GROW` | `default-size`, `size-request` | S2: after map, grow by `set_default_size(w + 320)`, or show the 640 px pane so the minimum width rises past the current width. Then shrink back to 760. |
| `TC_BLUR_SPIKE_STEP_MS` | ms, default 2000 | Interval between the S2 phases. |
| `TC_BLUR_SPIKE_MAXIMIZE` | any | Maximize before presenting. |
| `TC_BLUR_SPIKE_QUIT_MS` | ms | Quit after this long. |

Diagnostics go to stderr as `key=value` lines (`backend=`, `blur_available=`,
`wayland_globals_matching=`, `x11_root_announces_blur=`, `request rect=... sent=`,
`s2 phase=... widget=WxH surface=WxH`).

## The blur request: names verified from source

- **Wayland:** `org_kde_kwin_blur_manager` v1, with requests
  `create(new_id org_kde_kwin_blur, wl_surface)` and `unset(wl_surface)`.
  `org_kde_kwin_blur` v1 has `commit`, `set_region(wl_region, allow-null)` and
  `release` (destructor). Source: `plasma-wayland-protocols/src/protocols/blur.xml`
  as vendored in `wayland-protocols-plasma` 0.3.12. The state is
  double-buffered: it applies on the next `wl_surface.commit`, so the
  prototype calls `queue_draw()` after `blur.commit()`.
- **X11:** the window property `_KDE_NET_WM_BLUR_BEHIND_REGION`, type
  `CARDINAL`, format 32, holding `x y w h` per rectangle in device pixels.
  An empty property, or one with a single value, means the whole window.
  Sources: KWindowSystem `src/platforms/xcb/kwindoweffects.cpp`
  (`enableBlurBehind`, which floors x and y and ceils w and h after scaling)
  and KWin `src/plugins/blur/blur.cpp` (`s_blurAtomName`, `updateBlurRegion`),
  both at `master` on invent.kde.org, fetched 2026-11-16.
- **Availability:** KWin creates the Wayland global only while its blur
  effect is loaded, and on X11 it announces the atom on the root window
  (`announceSupportProperty`). The prototype therefore decides by what it can
  actually bind (the global, or the atom on the root window), not by desktop
  name.
- **Region:** GTK 4 draws client-side decorations on KWin too, so the
  `wl_surface` is larger than the window by the shadow margin. The prototype
  sends a region equal to the window widget's box
  (`surface_transform()` offset, `width() x height()`), so blur does not
  reach under the shadow. The rounded corners are not cut out, so a few
  pixels of blur would show outside each corner. Never observed, because the
  request never ran on a KWin that offered blur (see Observations).
- **Connection:** `gdk4_wayland::WaylandDisplay::wl_display()` (safe, with
  feature `wayland_crate`) gives a `WlDisplay` on GTK's own `wl_display`. The
  prototype wraps its backend in a `wayland_client::Connection` and runs
  `registry_queue_init` on a private event queue. That roundtrip ran on both
  compositors used here.

## Observations

### Machines and what they could show

- **tc-linux-kde** (Debian 13, kernel `6.12.116+deb13-cloud-amd64`, KWin 6.3.6,
  GTK 4.18.6, libadwaita 1.7.6):
  - **No real Plasma session.** First boot's bootstrap aborted on
    `Unable to locate package spectacle`, because Debian names the package
    `kde-spectacle`.
  - I installed the bootstrap's package list with `kde-spectacle`, plus
    `kwin-x11`, over ssh. Someone else then rebooted the box into the
    corrected bootstrap, and `sddm` became active.
  - The `tc` Plasma session still never started. The cloud kernel ships no
    DRM drivers: `find /lib/modules/$(uname -r) -path '*gpu/drm*'` finds 0
    files, and `/dev/dri` does not exist. `lspci` shows a VGA controller
    (`Google, Inc. Device a002`), but nothing drives it.
  - Fixing that needs a kernel with DRM drivers (for example
    `linux-image-amd64` instead of `-cloud-amd64`). That changes the VM, so
    it is left to the owner.
- **tc-linux-gnome** (Ubuntu 24.04): its bootstrap failed on archive 404s
  (`xbrlapi`, `libpam-fprintd`), and later on `held broken packages`. The
  display manager was inactive and there was no `tc` session. My attempt to
  re-run the package install was refused by the session's permission policy,
  and I did not retry it. **Nothing was run on GNOME.**

### S1, blur

- **Not verified on a real KDE desktop.** Neither run below could load
  KWin's blur effect, so `create`, `set_region`, `commit` and the X11
  `change_property` were compiled but **never executed against a compositor
  that offered blur**. Treat them as untested code.
- **Nested KWin, run 1.** `kwin_wayland --virtual --xwayland` 6.3.6 under
  `dbus-run-session`, run as an ordinary user over ssh:
  - KWin reported `Compositing Type: QPainter`. Its loaded effects did not
    include `blur`, and `isEffectLoaded blur` returned `false`.
    `KWIN_COMPOSE=O2` was refused with "Configured compositor not supported
    by Platform", because the virtual backend has no OpenGL without a render
    node.
  - The example logged `wayland_globals_matching=[]` and
    `blur_available=false`. Under `GDK_BACKEND=x11` (XWayland inside that
    KWin, not an X11 session) it logged `x11_root_announces_blur=false`.
  - `auto` therefore chose `Painted` on both paths, which is the intended
    fallback.
  - Spectacle refused to capture ("On Wayland, Spectacle requires KDE
    Plasma's KWin compositor"), presumably because KWin's screenshot
    interface needs OpenGL compositing. There are no KWin screenshots.
- **Nested KWin, run 2.** `kwin_wayland` windowed on a headless weston with
  the GL renderer crashed at start with `zwp_linux_dmabuf_v1 v4 or newer is
  needed` (exit 139).
- **X11 session:** not tested. `kwin-x11` is installed, but there is no
  X11 Plasma session and no display device for one. The XWayland check
  above is the only X11-backend run.

### S1, translucency and the painted fallback

- **GTK 4 translucency works.** The run was weston 14.0.2, headless with the
  GL renderer (llvmpipe), desktop-shell, and a patterned background. In
  `blur-off` mode the window background, headerbar, veil and cards
  composite over the pattern; in `painted` mode they are opaque. See
  `kde-weston/blur-off.png` and `painted.png`.
  - This was weston, not KWin and not Mutter.
  - The `blur-off` capture also shows the contrast problem the spec warns
    about: with no blur, the pattern stays sharp behind the content.
- **Fallback:** weston advertises no blur global. `auto` logged
  `blur_available=false glass=Painted`, and the window drew opaque
  (`auto.png`). That is the path GNOME is expected to take, but it was not
  observed on GNOME.

### S2, growing a mapped window

`set_default_size` was called on a mapped window, then the window was shrunk
back. Widths are the window widget's (GTK's `width()`); "surface" includes the
CSD shadow.

| Compositor | Method | Before | Grown | Shrunk |
| --- | --- | --- | --- | --- |
| KWin 6.3.6 `--virtual` (nested) | `default-size` | 760 (surface 882) | 1080 (1202) | 760 (882) |
| KWin 6.3.6 `--virtual` (nested) | `size-request` (640 px pane) | 760 | 872 (the new minimum) | 760 |
| KWin 6.3.6 `--virtual`, maximized | `default-size` | 1280 | 1280 (default 1600, ignored) | 1280 |
| weston 14 headless | `default-size` | 760 | 1080 | 760 |
| weston 14 headless | `size-request` | 760 | 872 | 760 |

- On weston, the screenshots were compared with the bare background in each
  run. **The left edge stayed put and the window grew at its right edge.**
  - `default-size`: x 119-962, then 119-1279 (clipped at the screen edge),
    then 119-962.
  - `size-request`: x 238-1081, then 238-1193, then 238-1081.
- KWin has no screenshots (see above), so its edge was not observed. Only
  the widths are measured there.
- **Reading.** A mapped GTK 4.18 window on Wayland can grow and shrink on
  both compositors, by either method, and a pane that raises the minimum
  width forces growth. A maximized window ignores it, as expected.
- **Still open:** what GNOME Mutter and a real Plasma session do. Neither
  was available here.

## Dependency write-up

Every crate below is used only with `--features blur-spike`, on Linux only
(`[target.'cfg(target_os = "linux")'.dependencies]`, all `optional = true`).
All five direct crates are permissive, so they are compatible with this crate's
`MIT OR Apache-2.0` and combinable into the AGPL-3.0 server workspace. None
touches the AGPL crates; this is a separate Cargo workspace.

`cargo deny` was run against the repository `deny.toml` from the GTK crate
directory, with `--features blur-spike` and with `--all-features`. The
results were `licenses ok`, `advisories ok`, `bans ok` and `sources ok`. The
only warnings are pre-existing unmatched exceptions and ignores.

Downloads and the last publish date are from the crates.io API on 2026-11-16.

### Direct crates

| Crate | Version locked | Purpose | Total downloads (90-day) | Transitive crates (new to lock) | Last publish of any version | License |
| --- | --- | --- | --- | --- | --- | --- |
| `gdk4-wayland` | 0.9.6 (feature `wayland_crate`) | `WaylandDisplay::wl_display()`, `WaylandSurface::wl_surface()` as `wayland-client` proxies | 656k (189k) | 66 (10) | 2026-09-20 (0.11.5) | MIT |
| `gdk4-x11` | 0.9.6 (no features; `xlib` would need unsafe) | `X11Display` detection, `X11Surface::xid()` | 1.26M (810k) | 49 (1) | 2026-09-20 (0.11.5) | MIT |
| `wayland-client` | 0.31.15 | `Connection`, registry, event queue | 90.4M (30.6M) | 24 (8) | 2026-07-22 | MIT |
| `wayland-protocols-plasma` | 0.3.12 (feature `client`) | generated `org_kde_kwin_blur*` bindings | 41.8M (15.0M) | 26 (10) | 2026-03-31 | MIT |
| `x11rb` | 0.13.2 (`default-features = false`) | Pure-Rust X connection: intern the atom, check the root window, `change_property32` | 90.7M (35.0M) | 5 (2) | 2026-07-16 (0.14.0) | MIT OR Apache-2.0 |

"Transitive crates" counts unique crates under the dependency in `cargo tree
--target x86_64-unknown-linux-gnu -e normal,build --features blur-spike`. Most
are the gtk-rs and glib stack already in the lock. "New to lock" is how many of
those this branch adds.

Maintenance:

- **gtk-rs** (`gdk4-*`) and **Smithay** (`wayland-*`) are active
  organisations. Both publish regularly, and `gdk4-wayland` and `gdk4-x11`
  release in lock-step with `gtk4`.
- The 0.9 line pinned here matches `gtk4` 0.9 / `gdk4` 0.9.6. Moving to 0.11
  means moving the whole gtk-rs stack.
- **x11rb** is maintained by psychon, last release 2026-07-16.

### Packages new to `Cargo.lock` (18)

| Crate | Version | Pulled by | Downloads | Last publish | License |
| --- | --- | --- | --- | --- | --- |
| `gdk4-wayland-sys` | 0.9.6 | gdk4-wayland | 666k | 2026-09-20 | MIT |
| `gdk4-x11-sys` | 0.9.6 | gdk4-x11 | 1.27M | 2026-09-20 | MIT |
| `wayland-backend` | 0.3.17 | wayland-client | 69.2M | 2026-08-14 | MIT |
| `wayland-sys` | 0.31.11 | wayland-backend | 101M | 2026-03-31 | MIT |
| `wayland-scanner` | 0.31.11 | wayland-client, -protocols (proc-macro) | 90.7M | 2026-07-22 | MIT |
| `wayland-protocols` | 0.32.13 | wayland-protocols-plasma | 96.6M | 2026-06-19 | MIT |
| `quick-xml` | 0.41.0 | wayland-scanner (build time) | 553M | 2026-08-22 | MIT |
| `dlib` | 0.5.3 | wayland-sys | 78.8M | 2026-02-26 | MIT |
| `libloading` | 0.8.9 | dlib | 644M | 2026-11-13 | ISC |
| `downcast-rs` | 1.2.1 | wayland-backend | 213M | 2025-09-01 | MIT OR Apache-2.0 |
| `scoped-tls` | 1.0.1 | wayland-backend | 229M | 2022-10-31 (stable, small) | MIT OR Apache-2.0 |
| `x11rb-protocol` | 0.13.2 | x11rb | 88.6M | 2026-07-16 | MIT OR Apache-2.0 |
| `gethostname` | 1.1.0 | x11rb (Xauthority lookup) | 157M | 2025-10-14 | Apache-2.0 |

Together with the five direct crates, that is 18. Every license above is in
`deny.toml`'s allow list.

Advisories: `cargo deny check advisories` against the RustSec database, with
the feature on, reports none for any of these crates.

### Smallest crate set

- **Wayland:** `gdk4-wayland` (with `wayland_crate`), `wayland-client` and
  `wayland-protocols-plasma`. `wayland-client` is already a dependency of
  `gdk4-wayland` with that feature, but it has to be named to use
  `Connection` and the globals API.
- **X11:** `gdk4-x11` for the xid, and `x11rb` for the property.
- **A single path:** Plasma 6 defaults to Wayland and Debian 13 ships X11
  Plasma separately (`kwin-x11`). If only Wayland matters, the X11 pair can
  be dropped, removing 5 of the 18 new packages (`gdk4-x11`, `gdk4-x11-sys`,
  `x11rb`, `x11rb-protocol`, `gethostname`).
- **Without these crates:** the alternative, calling
  `gdk_wayland_surface_get_wl_surface` directly, needs `unsafe`. The crate
  forbids that.

## Not done, and why

- No real KDE Plasma session (the cloud kernel has no DRM) and no GNOME
  session (its bootstrap failed). Blur itself, the corner overspill and
  GNOME's behaviour are all unobserved.
- No X11 Plasma session.
- `cargo` 1.100 prints `warning: unused dependency` for the five crates on a
  `clippy --all-targets` run with the feature on, because only the example
  uses them. It is a warning, not an error. If the feature were kept, they
  would move into a module of the library.
