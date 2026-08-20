# Icon pipeline — one geometry, generated per platform

Date: 2026-08-19
Status: draft for review
Scope: sub-project B. Icon artwork and its generation. No runtime behaviour change.
Companions: sub-project A (fail-closed roots parity), sub-project C (macOS Dock icon and menu-bar mark).
Depends on: nothing. Ships independently.

## Problem

The product has no icon. Not a weak icon, not an outdated icon — none of the
three clients ships artwork a user would recognise as belonging to Trace
Commons.

- **macOS.** `macos/scripts/make-app-bundle.sh:86` creates
  `Contents/Resources` and nothing is ever copied into it.
  `macos/scripts/info-plist.sh` sets no `CFBundleIconFile`; its only
  occurrence of the word is the comment at line 65 explaining that
  `LSUIElement` means there is no Dock icon. Nobody noticed the missing
  artwork because a menu-bar-only app never displays one.
- **Windows.** `windows/packaging/Assets/` holds the three PNGs
  `Package.appxmanifest:34,78,79` references. All three are **solid squares
  of `#315FBA`** — one unique pixel each, no alpha channel, at 150x150,
  44x44 and 50x50. `#315FBA` is `TC.blue` in its light-mode value
  (`macos/Sources/TraceCommonsApp/Views/DesignSystem.swift:409`), so these
  are deliberate brand-coloured placeholders rather than an accident, but a
  flat rectangle is what ships on the Start menu and the taskbar today.
- **Linux.** `crates/trace-commons-contributor-gtk/flatpak/ai.tracecommons.Contributor.desktop:6`
  declares `Icon=ai.tracecommons.Contributor`. The manifest's build-commands
  install the binary and the desktop entry
  (`ai.tracecommons.Contributor.yml`, the two `install -Dm644`/`-Dm755`
  lines) and never install an icon file under that name. The entry points at
  an icon that does not exist in the image. There is also no AppStream
  metainfo anywhere in the repository, which every Linux software centre
  needs before it will show the app at all.

This is the gap sub-project C runs into: a macOS Dock icon cannot be added
without artwork, and the fastest fix — render one PNG, run `iconutil`, move
on — would leave Windows and Linux exactly as broken as they are now while
adding a fourth hand-maintained description of the mark.

## What is already right, and must not be thrown away

The mark is not undefined. It is defined three times, precisely, and two of
those definitions already argue the correct principle at length.

`crates/trace-commons-contributor-gtk/src/ui/mark.rs:22-30`, under the
heading "Why it is drawn, not shipped as a file":

> It has to be legible at 14px in a tray and at 84px on an onboarding screen,
> on displays at 1x, 1.5x, 2x and fractional scales in between. A
> `DrawingArea` is handed the real device scale by GTK and redraws into it, so
> there is one description of the mark and no size ladder of PNGs to keep in
> step with it.

`windows/src/TraceCommons.Interop/MarkRaster.cs:20-24`, under "Why not ship a
.ico":

> The tray asks for whatever size the current DPI calls for — 16, 20, 24, 32 —
> and the mark is geometry, not an image. A size ladder of PNGs baked into an
> .ico is a second description of the mark that has to be kept in step with
> the XAML by hand. Rendering from the same numbers keeps one description.

Both are right, and this design does not overturn them. What it observes is
that the principle was applied to *UI surfaces* and never to *packaging
surfaces*, and packaging is exactly where an OS demands a file on disk.

More usefully: **the SVG emitter already exists.**
`crates/trace-commons-contributor-gtk/src/ui/mark.rs:125` is
`pub fn svg(scheme: Scheme, size: u32) -> String`, and line 147 is
`pub fn template_svg(ink: &str, size: u32) -> String`. Both are pure
`format!` calls over the canonical numbers, with no GTK, cairo or GLib in
their bodies. They emit precisely the geometry this design needs. The problem
is only where they live — see "Where the code goes".

### The geometry

Transcribed identically in all three clients, on a 64-unit coordinate space:

| Element | Path | Stroke |
| --- | --- | --- |
| Frame | `rect x=1 y=1 w=62 h=62` | 2 |
| Green bracket | `M11 28 V11 H28` | 7 framed, 8 template |
| Blue bracket | `M53 36 v17 H36` | 7 framed, 8 template |

Sources: `macos/Sources/TraceCommonsApp/Views/BrandMark.swift:117-126`,
`crates/trace-commons-contributor-gtk/src/ui/mark.rs:280-289`,
`windows/src/TraceCommons.App/Controls/BrandMark.xaml.cs`.

The frame is inset one unit under a two-unit stroke so its outer edge lands
exactly on the 64x64 boundary. The template variant thickens to 8 because it
loses the frame that was holding the brackets apart.

**The cited authority does not exist in this repository.** All three files
name `design-import/DESIGN-SPEC.md` §1.2 as the source; there is no
`design-import/` directory and no file matching `DESIGN-SPEC*` anywhere in
the tree. Until it is located or imported, `BrandMark.swift` is the de facto
authority — it is the most complete of the three, carrying the reasoning for
the stroke change and the reveal animation alongside the numbers. Finding the
real document is worth doing before this work lands, because an app icon is
the one artifact a designer is most likely to have opinions about that the
code has never captured.

### Two treatments, not one

They are different artifacts and the spec keeps them apart throughout:

- **App icon** — full colour, framed, filled. Dock, Finder, Start menu, MSIX
  tiles, software centres, the About window. The `.auto`/framed variant.
- **Template stencil** — frameless, single ink, stroke 8. Menu bar,
  notification area, SNI tray. Recoloured by the host, never by us.

## Decision 1: the source of truth is the geometry, in one crate

Create `crates/trace-commons-mark` — a small, platform-neutral library with no
GUI dependencies. It holds:

1. the geometry constants (view box, the two bracket paths, the three stroke
   widths, the frame rect),
2. the palette values for both schemes, and
3. `svg()` and `template_svg()`, moved verbatim from
   `crates/trace-commons-contributor-gtk/src/ui/mark.rs:125-158`.

`mark.rs` then re-exports them and keeps only its cairo drawing and GTK widget
construction. That is a lift-and-shift of working, reviewed code, not a
rewrite.

The reason it cannot stay where it is: `Cargo.toml:17` excludes
`crates/trace-commons-contributor-gtk` from the root workspace, and that crate
links GTK 4, so it does not build on macOS at all. An export tool that has to
run on `macos-26` and `windows-latest` runners cannot depend on it. The new
crate joins the root workspace and builds everywhere.

The committed SVG files are generated from this crate, not authored. Nobody
edits an SVG by hand; the numbers live in Rust where the tests can reach them.

## Decision 2: the three in-code implementations stay

This is the open question the design has to answer, and the answer is that
live drawing stays for UI and generated assets are for packaging only.

Replacing `BrandMark.swift`, `mark.rs` and `BrandMark.xaml.cs` with a loaded
asset would discard exactly what their authors argued for: one description,
redrawn at the real device scale, correct at 14px and 84px and at fractional
scales in between. It would also make every in-window mark theme-static,
where all three currently follow the system appearance live —
`BrandMark.swift:74` sets the colour scheme through the environment, and
`mark.rs:211` has `follow_scheme`.

So the generated assets are consumed only where an operating system insists on
a file: the macOS bundle icon, the MSIX tiles, the hicolor icon the `.desktop`
entry names, and the metainfo screenshots. Four packaging surfaces, zero UI
surfaces.

**What this means for sub-project C.** C's dependency on B is the *app icon*
only. Its other half — the menu-bar mark rendering nothing — should not be
fixed by loading a template PNG from the bundle. If the root cause turns out
to be that a `MenuBarExtra` label made only of `Color.primary` strokes masks
out to nothing, the fix that stays consistent with everything above is to
rasterize the existing SwiftUI view at runtime through `ImageRenderer` into an
`NSImage` with `isTemplate = true` — one description, still redrawn at the
current scale, no asset on disk. C must not be blocked on B for the menu bar.

The template treatment, in fact, needs no packaged asset on any platform.
Windows renders it through `MarkRaster` into an `HICON`. Linux does need it
as a file, because `StatusNotifierItem` names an icon rather than carrying
one — but `crates/trace-commons-contributor-gtk/src/tray.rs:27-34` already
handles that by writing a private two-SVG icon theme under the application's
data directory at runtime and pointing the host at it through
`IconThemePath`, explicitly to stay out of the system icon directories that
"belong to the packaging and to the contributor, not to a running process."
So `mark-export` emits the template SVG — it is part of the geometry and it
is worth having under test — and no packaging step installs it. If that ever
changes, the file is already there.

## Decision 3: per-platform export targets

### macOS — Icon Composer `.icon`, with `.icns` as the fallback

Recommend the Icon Composer format. Three reasons, in order of weight:

1. **The mark already has a light drawing and a dark drawing.** Icon Composer
   is built around exactly that: light, dark and tinted appearances in one
   document. Every other route forces a single flattened appearance and
   throws away half of what `DesignSystem.swift:354,368,402,409` already
   defines.
2. On macOS 26 a classic `.icns` renders as a flat legacy icon beside
   Liquid Glass system icons. It works — it is not deprecated — but the app
   looks like it predates the OS it is shipping on.
3. Icon Composer takes vector input directly, so the committed SVG is the
   input rather than a raster ladder.

The cost is that this package is built with `swift build`, not `xcodebuild`
(`macos/scripts/make-app-bundle.sh` calls `swift build --configuration
"$CONFIG" --arch arm64 --arch x86_64`), and SwiftPM does not run the asset
compiler. `make-app-bundle.sh` must therefore invoke `actool` itself —
`/usr/bin/actool`, version 26.6 on this machine, whose `--compile` and
`--app-icon` flags are documented in `man actool` — writing into
`Contents/Resources`, and `info-plist.sh` must gain the `CFBundleIconFile` /
`CFBundleIconName` keys `actool`'s partial plist output names.

The fallback, if `actool` against a `.icon` from a non-Xcode build turns out
not to work: render the framed SVG to the standard size ladder, `iconutil
-c icns`, set `CFBundleIconFile`. `/usr/bin/iconutil` is present. This is the
cheap path and it is not the recommendation, but it is a known-good escape
hatch and the plan should not discover that on release day.

### Windows — replace the placeholders, add the scale variants

`Package.appxmanifest` references three assets today. MSIX expects
scale-qualified variants (`.scale-100`, `-125`, `-150`, `-200`, `-400`) and,
for `Square44x44Logo`, the `.targetsize-*` set the taskbar and Start use;
without them Windows scales one bitmap and the result is visibly soft. The
export produces the full set the manifest declares, and the manifest gains
whatever additional logo elements the generated set supports.

Rasterization reuses `MarkRaster` rather than introducing a second Windows
rasterizer. It already renders the mark to a BGRA buffer with 4x4 coverage
sampling, in `TraceCommons.Interop` — an assembly deliberately built to run
off-Windows so the geometry is exercised by tests on a developer machine
(`MarkRaster.cs:14-17`). It currently renders the single-ink template
variant, so it needs a framed, two-colour path added alongside for the app
icon.

### Linux — the icon the desktop entry already promises, plus metainfo

The `.desktop` entry names `ai.tracecommons.Contributor` and nothing installs
it. Three additions to the flatpak module's build-commands:

- `install -Dm644` the framed SVG to
  `/app/share/icons/hicolor/scalable/apps/ai.tracecommons.Contributor.svg`.
  Scalable, so no raster ladder and no rasterizer on the Linux path at all.
- Write and install `ai.tracecommons.Contributor.metainfo.xml` to
  `/app/share/metainfo/`. Flathub requires it; GNOME Software and KDE
  Discover will not display the app without it. This is new content — name,
  summary, description, licence, screenshots — not generated from geometry.

## Where the code goes

| Piece | Path |
| --- | --- |
| Geometry, palette, SVG emitters | `crates/trace-commons-mark/` |
| Export binary | `crates/trace-commons-mark/src/bin/mark-export.rs` |
| Generated SVG | `assets/mark/` |
| macOS icon document | `macos/Assets.xcassets/` (or `macos/AppIcon.icon`) |
| macOS bundle wiring | `macos/scripts/make-app-bundle.sh`, `macos/scripts/info-plist.sh` |
| Windows tile generation | `windows/scripts/make-icons.ps1` |
| Windows tiles | `windows/packaging/Assets/` |
| Linux icon and metainfo | `crates/trace-commons-contributor-gtk/flatpak/` |

`mark-export` writes SVG only. Each platform's raster step lives with that
platform's packaging, because each uses that platform's own toolchain and
runs on that platform's runner.

## Drift control

Hand-committed rasters that nobody could regenerate are what produced three
solid blue squares. The countermeasure is two checks, and the second matters
more than the first.

**1. Generated artwork matches the source.** A CI step runs `mark-export`
into the working tree and then `git diff --exit-code assets/mark`. Committing
the SVG keeps the tree buildable without running Rust first; the check makes
the commit non-authoritative. This is cheap and belongs on `ubuntu-latest`
alongside the existing `cargo-fmt-check` job in `.github/workflows/ci.yml:42`.

The same check cannot cover the platform rasters — `.icns`/`.icon` needs
macOS, MSIX tiles need Windows. Those regenerate in their release jobs
(`release-apps.yml:121` `macos-26`, `:197` `windows-latest`) and the job fails
if the result differs from what is committed.

**2. The three in-code implementations still agree with the canonical
numbers.** This is the drift that actually hurts, and it is invisible to any
asset check: `mark.rs` even warns that its colour literals are duplicated and
that `style.rs` wins if they diverge. A unit test in
`crates/trace-commons-mark` asserts the constants, and each client gets a test
asserting its own transcription equals them —
`crates/trace-commons-contributor-gtk` in its own workspace,
`windows/src/TraceCommons.Interop` in its existing test project, and macOS via
the SwiftPM test target. Three small tests that would have caught a
transcription error in any client.

Linux gains two validations in the existing
`linux-shell-desktop-integration` job (`ci.yml:424`): `desktop-file-validate`
on the entry and `appstreamcli validate` on the metainfo. Both tools are
standard on the runner image and neither is a new dependency.

## Dependencies

**None proposed.** This is deliberate and it constrained the design.

The obvious approach — render SVG to PNG in Rust — would mean `resvg`,
`usvg` and `tiny-skia`, none of which appear in `Cargo.lock`. The design
avoids all three by never rasterizing in Rust: Linux takes the SVG directly,
macOS rasterizes through `actool`/`iconutil`, and Windows reuses the
`MarkRaster` code that already exists and is already tested.

One near-miss worth stating so it is not mistaken for an easy option later.
`image` 0.25.10 and `png` 0.18.1 **are** in the root `Cargo.lock`, which makes
them look free. They are not: they arrive only through `fastembed` and
`mistralrs`, the feature-gated ML stack behind the server's scorer paths.
Adding either as a direct dependency of a contributor-side crate is a new
direct dependency under this repository's policy and would need approval on
its own terms. This design does not ask for it.

If review prefers a Rust rasterizer anyway, that is a dependency proposal
this spec has not made, and it should be raised explicitly with adoption,
maintenance, licence and transitive-count detail rather than folded in.

## Verification

- **macOS.** Build the bundle, confirm `Contents/Resources` is no longer
  empty, and confirm the icon appears in Finder, in Get Info at 512pt, and in
  the Dock once sub-project C removes `LSUIElement`. Check light, dark and
  tinted appearances if the `.icon` route holds.
- **Windows.** Build the MSIX, install it, and look at the Start tile, the
  taskbar, and the Apps list at 100% and 200% scaling. The current
  placeholders pass every automated check there is, so a human has to look.
- **Linux.** Build the flatpak, confirm the icon installs under `hicolor`,
  and confirm GNOME Software renders name, icon and summary from the
  metainfo. Per the manifest's own header the flatpak is `UNBUILT`, so this
  is a first build as much as a verification.
- **Drift.** Change one geometry constant, confirm the export check and all
  three transcription tests fail.

## What is unverified

Stated plainly, in the manner `docs/superpowers/specs/2026-08-16-signed-app-distribution-design.md`
and the flatpak manifest already use.

- ~~No `.icon` document has been produced or compiled.~~ **RESOLVED, and the
  premise was wrong.** See "The `.icon` route works from a build script"
  below.
- The full set of MSIX scale and targetsize qualifiers the manifest should
  declare has not been enumerated against Microsoft's current requirements.
- `MarkRaster` renders the template variant only. The framed two-colour path
  is new code, not a reuse.
- The flatpak has never been built by anything, so the icon and metainfo
  install steps are unexercised along with everything else in that manifest.

## Risk

Low, and bounded by the fact that nothing here changes runtime behaviour. The
worst outcome is that the macOS `.icon` route does not work outside Xcode and
the work falls back to `.icns`, losing the Liquid Glass treatment and the
light/dark appearances but still shipping a real icon on all three platforms.

The one thing that could expand scope is the missing `design-import/DESIGN-SPEC.md`.
If it turns up and specifies an app-icon treatment different from the framed
mark — a different lockup, padding, or background — the artwork changes but
the pipeline does not.

## Out of scope

- Sub-project A: fail-closed roots parity.
- Sub-project C: removing `LSUIElement`, the Dock icon wiring, and the
  invisible menu-bar mark. C consumes this work; it is not part of it.
- Any change to how the mark is drawn in any UI surface.
- Store submission of any kind — Microsoft Store, Flathub, Mac App Store.
  Each imposes its own icon requirements and each is an owner decision.
- Marketing or community-site artwork. `community/` keeps its own.

## Addendum: the `.icon` route works from a build script

Recorded 2026-08-19, after implementation. The open question above assumed an
Icon Composer document could not be produced outside Xcode. It can, and the
belief was never tested — it sat in `macos/scripts/make-icons.sh` as a
statement of fact, which is why it went unchallenged.

What was established, each by doing it rather than by reading about it:

- A `.icon` is a **directory**. Icon Composer's `Info.plist` exports the UTI
  `com.apple.iconcomposer.icon` with `UTTypeConformsTo` = `com.apple.package`.
- It holds `icon.json` plus an `Assets` directory of **SVGs**.
  `IconComposerFoundation` carries the strings `Assets should be a directory`
  and a diagnostic rejecting SVG assets that contain text elements, which is
  only meaningful if SVG is the asset format. The mark's SVGs are pure paths.
- `actool` compiles such a directory with no Xcode project involved, emitting
  `Assets.car`, a fallback `AppIcon.icns`, and a partial `Info.plist`
  declaring **both** `CFBundleIconFile` and `CFBundleIconName`.
- The compiled result renders with the genuine macOS 26 Liquid Glass
  treatment — system tile, shape, shadow and specular highlight.

Two properties of `actool` that the implementation had to be built around,
both verified deliberately:

1. **It validates nothing.** An `icon.json` naming an asset file that does not
   exist compiles without a warning, as does a key whose value is the wrong
   JSON type. A typo therefore yields a different icon, not a failed build.
   This is why `make-icon-document.sh` ends by decoding what came out.
2. **Its output is not reproducible.** Two compilations of byte-identical
   input produce different `Assets.car` bytes; rendition names embed fresh
   UUIDs each run. So this artifact cannot join the byte-level drift check
   that covers the SVG and PNG artwork. It is checked semantically instead.



### The dark drawing is carried, and the shape that carries it

Dark mode draws the mark's own palette. The layer names both glyphs through
an `image-name-specializations` array:

```json
"layers" : [
  {
    "image-name-specializations" : [
      { "value" : "mark-glyph-light.svg" },
      { "appearance" : "dark", "value" : "mark-glyph-dark.svg" }
    ]
  }
]
```

**The base entry is the whole trick.** The first element carries no
`appearance` key: it is what every appearance starts from, and entries with an
`appearance` override it. An array holding only the dark entry -- an override
with nothing to override -- is structurally invalid and, because `actool`
validates nothing, is discarded in silence. That silence is indistinguishable
from the feature not existing, which is what two earlier passes concluded.
The same construct works at the top level for `fill-specializations`.

This shape was taken from an `icon.json` written by Icon Composer 1.6, not
inferred. Two earlier attempts failed on structure, not vocabulary: the key
name `image-name-specializations` and the slot name `dark` were both right
from the start.

Evidence it works, read back from the compiled catalogue with
`assetutil --info`:

| Appearance | Layer | Digest |
| --- | --- | --- |
| `NSAppearanceNameDarkAqua` | `AppIcon_Assets/mark-glyph-dark` | `CFEFF50C…` |
| `NSAppearanceNameAqua` | `AppIcon_Assets/mark-glyph-light` | `A79E9F76…` |
| `ISAppearanceTintable` | `AppIcon_Assets/mark-glyph-light` | `A79E9F76…` |

Two distinct `Vector` assets, where before there was one shared by all three.

### Why the dark rendition is checked separately

`verify-icon.swift` inspects the fallback `.icns`, which carries only the light
drawing; the dark artwork exists solely inside `Assets.car`. And asserting
merely that a dark *composition* exists is not enough — this build previously
emitted three appearance compositions all referencing one vector, so it drew
the light inks in dark mode while every structural check passed. The icon
rendered. It rendered wrong, and nothing said so.

So `macos/scripts/verify-icon-appearances.py` asserts both halves: the dark
appearance draws the dark glyph, and its artwork digest differs from the
light one. It is a separate file rather than inline so it can be run against
any catalogue, which is how its teeth were proven.

Three failure modes, each demonstrated to fail the build:

| Broken input | Caught by |
| --- | --- |
| Blank dark glyph | bracket path check in `make-icon-document.sh` |
| Dark glyph byte-identical to light | `cmp` check in `make-icon-document.sh` |
| Specialization dropped from `icon.json` | `verify-icon-appearances.py`, exit 1 |

The third is the one that matters most: it is precisely the state this build
shipped in before the shape was understood.
