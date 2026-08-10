# Linux contributor shell: design pass

A visual pass over `crates/trace-commons-contributor-gtk`, aligning the GTK 4
/ libadwaita window with `community/public/styles.css` and with the macOS
shell that was aligned to the same site.

Before this pass the shell used stock libadwaita classes only -- `dim-label`,
`title-4`, `card`, `heading`, `suggested-action` -- with no `CssProvider` and
no colour definitions anywhere. It inherited whatever the user's theme gave
it, including the accent.

## The direction

The window stands between a developer's private transcripts and a public
research pool, and the only question it has to answer is "what exactly is
about to leave this machine, and can I stop it". So it is built like a
**customs declaration, not a feed**: every session is one card, every card
carries the same fields in the same order, and each card ends in a fixed
manifest strip set in monospaced type.

```
TURNS   WOULD SEND   SCRUBBED     PERSONAL INFO
5       3 KB         3 removed    private_email
```

The repetition is the point. When every card's outbound facts land in the
same place, a person stops reading and starts scanning, and the row that is
different -- a large payload, a session where scrubbing matched nothing -- is
a break in a rhythm rather than a sentence they have to notice. The preview
sheet builds the identical strip from the identical function
(`queue::manifest_for`), so a person who scanned a row and then opened it is
looking at the same four numbers in the same four places.

That is the one deliberately bold move. Everything else is kept quiet so it
can carry.

## What is carried across from the site

The palette, and more importantly the **roles** each colour plays: green is
primary and means good standing, gold means "weigh this", coral means
refused, blue means held or ranked. Also the warm off-white ground rather
than a neutral grey, 6/8px radii with 999px pills, hairlines instead of
shadows, and heavy uppercase micro labels over data.

Not carried across: **Inter is not bundled**, per the brief. No new
dependencies of any kind. The site's 680/760/800 weights are approximated on
the system face, which is what a Linux desktop's user has already calibrated
their eye against. The site's `0 18px 48px` card shadow is also dropped --
inside an application window it reads as a floating dialog, and a hairline
does the same separating work natively.

## The GNOME accent, overridden on purpose

GNOME convention is that applications follow the user's chosen system accent.
This one does not: it pins the Trace Commons green, as ruled. The reason is
written into `src/ui/style.rs` in a paragraph that says explicitly that this
is not a bug and asks a future contributor not to "fix" it back to
`AdwStyleManager`'s accent -- and that if it is ever reverted it should be
reverted as a product decision, not as a theming cleanup.

## Contrast, measured

Every figure below is a computed WCAG 2.1 relative-luminance ratio. None of
them were eyeballed.

The hazard flagged in the brief applies here directly, because libadwaita
derives `.suggested-action` from `accent_bg_color` / `accent_fg_color`. The
site's `--green` is tuned to be read *on* the ground, not to be a fill with a
label on top of it:

| Rejected pairing | Ratio | Floor |
| --- | --- | --- |
| `#ffffff` on site green `#178f70` | **4.04:1** | 4.5:1 normal text |
| `#ffffff` on dark mint `#3FBE9A` | **2.32:1** | 3:1 even for large text |

So the filled primary action carries its own measured pair, and because it is
set on the libadwaita accent tokens rather than on individual buttons, it
fixes every suggested action in the window at once:

| Chosen pairing | Ratio |
| --- | --- |
| light: `#ffffff` label on fill `#137C61` | **5.14:1** |
| dark: `#0B1F19` label on fill `#3FBE9A` | **7.39:1** |

Both were then confirmed against the rendered frames by sampling the pixels:
the light fill renders `srgb(19,124,97)` = `#137C61` and the dark fill
`srgb(63,190,154)` = `#3FBE9A`, with the label ink sampled at `#0B1F19`.

Everything else in the palette:

| Light | Ratio | Dark | Ratio |
| --- | --- | --- | --- |
| ink `#202426` on bg `#f6f7f4` | 14.55:1 | ink `#E9ECE2` on bg `#16180F` | 14.99:1 |
| ink on surface `#ffffff` | 15.65:1 | ink on surface `#1F221A` | 13.49:1 |
| muted `#5e6668` on bg | 5.46:1 | muted `#A6AE9F` on bg | 7.83:1 |
| muted on surface-2 `#eef2f0` | 5.20:1 | muted on surface-2 `#282C23` | 6.22:1 |
| green text `#0F7256` on surface | 5.90:1 | green text `#5CD3AF` on surface | 8.75:1 |
| gold text `#8A5F12` on surface | 5.64:1 | gold text `#E2B75C` on surface | 8.57:1 |
| gold text on surface-2 | 4.99:1 | gold text on surface-2 | 7.57:1 |
| coral text `#B8483B` on surface | 5.21:1 | coral text `#F79C8F` on surface | 7.76:1 |
| blue text `#315FBA` on surface | 6.04:1 | blue text `#9DB6F1` on surface | 7.98:1 |
| gold rule `#b9821f` on surface (non-text) | 3.35:1 | gold rule `#DCAA43` on surface | 7.58:1 |
| redaction `#202426` on wash `#f3e3c0` | 12.34:1 | redaction `#F0EBDD` on wash `#4A3C18` | 9.04:1 |

Every text pairing clears 4.5:1 in both schemes. The one non-text pairing
(the gold card rule that marks a card worth weighing) clears the 3:1 UI
component floor.

The site's accents are tuned for fills, meter bars and borders, where 3:1 is
the bar. As small type on white several of them do not clear 4.5:1, so each
accent has a darkened light-mode twin used **only for type**; fills, glyph
strokes and borders keep the site's exact values. Only the lightness moves,
so the family resemblance survives.

### A contrast bug found and fixed

`highlight_redactions` hard-coded `background: #f6d32d` (the GNOME palette
yellow) and set **no foreground**. Under a dark theme that put the theme's
near-white text on a bright yellow field: the marks that most need reading
were the least readable thing on the screen. It now carries a brand gold wash
in the "weigh this" role with both halves stated per scheme, at 12.34:1 and
9.04:1.

## Dark, derived rather than inverted

The site has no `prefers-color-scheme` block anywhere, so there was no dark
palette to copy -- and on this platform dark is not optional. The derivation
preserves the site's relations:

- The site's ground `#f6f7f4` is warm, with a faint green cast, not a neutral
  grey. The dark ground `#16180F` keeps that cast at the other end of the
  scale rather than the blue-black a naive inversion produces.
- ground / surface / inset keep the same order and roughly the same
  perceptual spacing as `--bg` / `--surface` / `--surface-2`.
- Every accent keeps its hue and its role and is lifted in lightness until it
  clears text contrast on the dark ground.
- Dark flips the primary *label* rather than dulling the mint. The mint is
  what makes the dark scheme feel like the same product; a duller green that
  could carry white would not.

## The repeated-caveat problem

`RESIDUAL_RISK` -- "Scrubbing is pattern-based. It misses things it hasn't
seen before." -- was printed verbatim on every queue card. Repeated down a
column, identical each time, it stops being read. That is how a warning
becomes wallpaper, and it is the warning this product most needs someone to
take seriously. A fable review of the macOS pass found that splitting it
across three placements produced three pieces of wallpaper, and that the
strongest idea was a per-card line that varies with what scrubbing actually
did. That is what is built here.

The row now carries `copy::residual_risk_line(total)`:

- **0 removed** -- "Scrubbing matched nothing here. That is not the same as
  there being nothing to find -- it only recognises patterns it has seen
  before." Set in the attention tone, with a gold rule on the card, a
  `! Nothing matched` pill, and the strip's `SCRUBBED` column reading
  `nothing` in gold.
- **1 removed** -- "Scrubbing removed 1 thing it recognised. It works from
  patterns, so it misses what it hasn't seen before."
- **n removed** -- the same, pluralised, in the muted tone.

A person reads the second card's line because it is not the line they read on
the first. The zero case gets the attention treatment because it is the case
worth weighing: a session that obviously touched a `.env` and reports nothing
removed is a signal.

The constant is **not deleted**. It is restated in full, verbatim, under
"Residual risk" in the preview sheet -- the screen a person is actually
reading on when they decide, rather than the one they are scanning.

A unit test in `copy.rs` asserts the two forms differ, that both concede the
limit whatever the count, and that "1 thing" is not written "1 things".

## Accessibility

- Contrast as tabulated above; measured, not estimated.
- **Meaning is never in colour alone.** Every tone carries a glyph and words
  as well as a colour (`Tone::glyph` -- checkmark, `!`, clock, cross,
  middot), restricted to characters DejaVu carries. The "nothing matched"
  state is signalled four ways at once: pill text, glyph, gold rule, and a
  differently-worded sentence.
- Uppercasing for eyebrows is done in Rust rather than via `text-transform`,
  which older GTK 4 releases do not implement, so the labels never silently
  fall back to mixed case.
- Sizes are in px, which GTK scales with the desktop's text-scaling factor,
  so a person who has asked for larger text still gets it.
- Controls that sit beside a separate label -- the autostart switch, the
  per-project mode dropdown -- now carry their own accessible label, so they
  say what they control without depending on reading order.
- The pill in `style::tag` is exposed as one accessible object rather than as
  two unrelated labels.
- Focus rings come from `accent_color`, which is set to the text-safe green
  (5.90:1 light, 6.95:1 dark), so keyboard focus is visible in both schemes.
  No control was removed from the tab order; the two card actions are plain
  buttons.

## Structure

- **One `CssProvider`** for the whole application, installed by
  `ui::style::install()` before any widget is built. It carries a
  `@define-color` token block plus `include_str!("style.css")`. There are no
  colour literals in the `ui` modules; the only exception is the transcript
  redaction tag, which is a `TextTag` and cannot reference a CSS named colour
  -- that is called out in a comment beside the two pairs.
- The tokens are chosen at **load** time and the provider is reloaded on
  `AdwStyleManager::dark`, because GTK CSS has no `prefers-color-scheme` and
  `@define-color` cannot be scoped to a selector.
- The libadwaita palette is recoloured **wholesale** -- `window_bg_color`,
  `view_bg_color`, `headerbar_*`, `card_*`, `popover_*`, `dialog_*`,
  `accent_*`, `success_*`, `warning_*`, `error_*`, `destructive_*`, plus the
  GTK-level `theme_*` and `borders`. A partial override produces a hybrid
  that looks broken in a way neither pure theme nor pure brand does. Because
  Adwaita's own stylesheet is written against these names, redefining them
  recolours the widgets this pass never mentions.
- Spacing is a 4px rhythm in `style::space`; widget code no longer writes raw
  numbers.
- Queue, history and settings are clamped to an 840px reading column
  (`adw::Clamp`), so a maximised window does not produce sentences nobody
  finishes and keeps a card's two actions within one eye movement.
- The brand mark is drawn as **two CSS gradients**, a direct transcription of
  `.brand-mark`. No asset file, no image, crisp at any scale factor. It sits
  at the start of both header bars.

## What was rendered, and how

Rendered for real, on this macOS host, via Docker with a Linux image
(`rust:1-bookworm` plus `libgtk-4-dev` / `libadwaita-1-dev`, which resolve to
GTK **4.8.3** and libadwaita **1.2.2**), running the real binary against a
real daemon under Xvfb and capturing the X root window with ImageMagick.
Every frame below was opened and looked at, in both schemes:

- queue, one card
- queue, two cards -- one with redactions and one with none, side by side,
  which is the frame that shows the varying caveat doing its job
- history
- settings
- preview sheet: Search (with a term typed and `0 matches` returned), What's
  in it, Exactly what would be sent

The fills were additionally verified by sampling pixels out of the captured
frames rather than by looking, which is how the `#137C61` / `#3FBE9A` figures
above are confirmed as what actually reaches a screen.

`TRACE_COMMONS_APPEARANCE=dark|light` was added to `ui::style::install()` as
a development hook so a capture run can photograph both schemes on a machine
pinned to one; unset -- the normal case -- the application follows the system
exactly. `--preview-tab <name>` was added to `main.rs` alongside the existing
`--open-preview` / `--search` debug drivers, so a capture run can photograph
the redacted transcript rather than only the tab a person lands on. Neither
approves anything.

To reproduce:

```bash
docker build -t tc-gtk - <<'EOF'
FROM rust:1-bookworm
RUN apt-get update -qq && apt-get install -y --no-install-recommends \
      libgtk-4-dev libadwaita-1-dev pkg-config build-essential \
      xvfb dbus-x11 imagemagick x11-apps xauth fonts-dejavu adwaita-icon-theme
ENV CARGO_TARGET_DIR=/target
EOF
docker run --rm -v "$PWD:/work" -w /work/crates/trace-commons-contributor-gtk \
  tc-gtk bash scripts/headless-run.sh
```

The captured PNGs are not committed; they are build output, and the recipe
above regenerates them.

## Verification

Run inside the container above:

- `cargo build` -- clean.
- `cargo clippy --all-targets` -- clean, no warnings.
- `cargo fmt -- --check` -- clean.
- `cargo test` -- 17 passed, 0 failed (up from 16; one added).

The crate remains its own workspace and was not added to the repository root
workspace.

## What was deliberately not done

- **Violet is unused.** The site defines `--violet`, but there is no fifth
  meaning in this window, and inventing one to spend the colour would make
  the role mapping less legible rather than more complete.
- **No motion.** The community site has no `transition`, `@keyframes` or
  animation of any kind, so there was nothing to port. An application asking
  to be trusted with somebody's source code should not fidget, and a
  reduce-motion path is a thing that has to be maintained.
- **Window chrome is left system-drawn.** The palette is applied to the
  content area and to the header's ground; the close/maximise controls, the
  view switcher's own button geometry, dropdowns and switches stay the
  platform's. A GTK application that reimplements its chrome is a worse GTK
  application.
- **No custom font.** Per the brief.
- **`private_email` and friends are still rendered raw.** The daemon's PII
  labels reach the strip and the sheet as the daemon writes them. Humanising
  them is a copy change with a contract question behind it (which labels
  exist, and who owns their wording), not a design change, so it is recorded
  here rather than guessed at.
- **`scripts/` was not touched**, per the brief, including
  `scripts/fixture.sh`. The second, redaction-free fixture session used for
  the two-card frame was created inside the throwaway container run instead.
- **`Not sent` rows and the history records are styled but not restructured.**
  Regrouping them is a product question about what the queue is for, and this
  pass is a design pass.

## Files

- `src/ui/style.rs` -- new. Tokens, both palettes with their measured tables,
  the `Tone` role enum, provider installation and scheme following, and the
  shared parts (brand mark, eyebrow, tag, manifest strip, section header,
  card).
- `src/ui/style.css` -- new. Every rule, referencing the tokens above.
- `src/ui/{mod,queue,preview,history,settings}.rs` -- restyled.
- `src/copy.rs` -- `residual_risk_line` added, `RESIDUAL_RISK` kept verbatim,
  one test added. This is the only copy change and it is the caveat fix.
- `src/main.rs` -- `--preview-tab` capture driver.

`src/backend.rs`, `src/portal.rs`, `src/tray.rs`, `src/autostart.rs`,
`src/worker.rs` and `scripts/` were not touched.
