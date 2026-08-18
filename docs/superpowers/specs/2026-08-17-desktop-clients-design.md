# Trace Commons desktop clients — design specification

Extracted from the Claude Design mockup document "Trace Commons desktop clients"
(`Desktop Clients.dc.html` in the Claude Design project "Trace Commons desktop
clients", project id `284b9ad8-9835-4ba4-9c2f-a67dd11b49d0`). The mockup file
itself is not checked in: it is a 266 KB inline-styled canvas export, and this
document is the implementable form of it. See §7.1 on the truncation and its
later recovery.

The implementation pass against this spec is recorded in
`docs/superpowers/plans/desktop-clients-design-pass-report.md`.

Every value below is transcribed verbatim from the inline styles of the mockup
frames. Where the mockups disagree between platforms or between turns, both
values are given and the disagreement is flagged.

**Source-of-truth ordering.** The document is organised as five conversational
turns, newest first. Section `6a` is the most recent and is authoritative for the
brand mark. Sections `4b`, `3b`, `5a` record earlier mark candidates and are
superseded. Sections `1a`–`1d` (turn 1) and `2a`–`2c` (turn 2) are the product
screens; they were drawn before the final mark was chosen but section `6a`
states that the new mark is rolled out across all of them.

**Truncation, and recovery.** The first import of the source file was cut at
256 KiB: section `1d` (Settings) stopped mid-attribute and sections `1e`, `1f`,
`1g` never arrived. All four have since been recovered in full by splitting the
source document into two files, each under the read cap, and §5.4–§5.7 below are
written from the complete frames. §7.1 records what the truncated pass got wrong.

**Platforms.** Three shells are drawn for most screens: macOS (SwiftUI,
`NavigationSplitView`), Windows (proposed, Fluent chrome — no client exists
yet), Linux (GTK4 / libadwaita). Copy is identical across all three in turns 1–4
*except* in `1d` Startup and `1f`, where the recovered frames diverge
per-platform on purpose; otherwise only chrome, type family, corner radii and
control weights differ. See §7.2 item 17.

---

## 1. Brand mark — "The Turn"

### 1.1 Status of candidates

| Mark | Where | Status |
|---|---|---|
| **The Turn** (two corner brackets, green + blue) | `6a`, candidate C in `5a` | **ADOPTED.** Authoritative. Used in every window chrome, tray flyout, menu bar, notification and title bar. |
| The Gate (green/blue fields with a white channel) | `5a` candidate A | Superseded (was the doc author's recommendation in `5a`, not chosen). |
| Confluence (two joining traces + dot) | `5a` candidate B | Superseded. |
| The Ledger (three transcript bars) | `5a` candidate D | Superseded. |
| The Drop (green trace into open blue vessel) | `5a` candidate E | Superseded. |
| Circuit / solder-dot mark on mint (`trace-mark.svg`) | `4b` candidate B, `3b`, used in `3a` onboarding header | **Superseded as the client mark.** It remains the *community website / docs-site* mark. `3a`'s onboarding frame still shows it because that frame predates the decision. |
| `.brand-mark` green/blue diagonal gradient square (site-v2 CSS) | `4b` candidate A | Superseded. Was the mark shipping in the clients at the time of the document. |

### 1.2 The Turn — exact geometry

`viewBox="0 0 64 64"`. Three variants.

**Light variant**

```
<rect x="1" y="1" width="62" height="62" fill="#FFFFFF" stroke="#D9DFDC" stroke-width="2"/>
<path d="M11 28V11h17" fill="none" stroke="#178F70" stroke-width="7"/>
<path d="M53 36v17H36" fill="none" stroke="#315FBA" stroke-width="7"/>
```

**Dark variant**

```
<rect x="1" y="1" width="62" height="62" fill="#21241E" stroke="#3B4038" stroke-width="2"/>
<path d="M11 28V11h17" fill="none" stroke="#3FBE9A" stroke-width="7"/>
<path d="M53 36v17H36" fill="none" stroke="#7FA0EC" stroke-width="7"/>
```

**Status-bar template variant** (macOS menu-bar rules: frameless, single ink)

```
<path d="M11 28V11h17" fill="none" stroke="#20241F" stroke-width="8"/>
<path d="M53 36v17H36" fill="none" stroke="#20241F" stroke-width="8"/>
```

Notes, verbatim from `6a`:

- Semantics: "user's bracket in green, agent's answer in blue, the session
  implied between them." Both brackets are corner brackets facing each other:
  the green one is the top-left corner, the blue one the bottom-right.
- Spec caption: `Green #178F70 · blue #315FBA · stroke 7/64 · hairline frame #D9DFDC`
- Template caption: "Template variant: frameless, ink-only brackets. The system
  recolors it in the menu bar's light, dark and selected states."
- No gradients. No fills other than the frame. Pure geometry.
- Stroke width in the framed variants is **7** at a 64-unit viewBox (≈11% of the
  mark); the template variant thickens to **8** to survive the loss of the frame.
- The frame rect is inset 1 unit with a 2-unit stroke, so it sits exactly on the
  64×64 edge.

### 1.3 Rendered sizes shown

Light row: 84, 40, 20, 14 px. Dark row: 40, 20, 14 px. Template row: 20, 14 px.
In-chrome sizes: **16 px** (Windows title bar), **20 px** (GNOME header bar),
**15 px** (macOS menu bar, template variant), **22 px** (inline palette swatch).

### 1.4 Superseded marks — geometry, for the record

Circuit / solder-dot mark (`3b`, `4b` B, still shown in `3a`):

```
<rect x="1" y="1" width="62" height="62" fill="#00d4aa" stroke="#000" stroke-width="2"/>
<path d="M13 18h17l8 14h13M13 32h14l8 14h16M13 46h9" fill="none" stroke="#000" stroke-width="4"/>
<circle cx="51" cy="18" r="3.5" fill="#000"/>
<circle cx="51" cy="32" r="3.5" fill="#000"/>
<circle cx="51" cy="46" r="3.5" fill="#000"/>
```
Its ink template variant: `<rect x="2" y="2" width="60" height="60" fill="#000"/>`
with the same paths/circles in `#fff`. Caption: "Mint #00d4aa · ink #000 · 2px
stroke at 64px, scaled with the mark."

`.brand-mark` gradient square (`4b` A) — CSS, not SVG:
`background-color:#FFFFFF; background-image:linear-gradient(135deg,#178F70 38%,transparent 38%),linear-gradient(45deg,transparent 45%,#315FBA 45%); border:1px solid #D9DFDC`

---

## 2. Color tokens

Two palettes coexist deliberately:

- **Native palette** — the private tool (screens 1a–1d, 4a). Derived from
  `DesignSystem.swift`. Hairlines not shadows, mono figures, muted ground.
- **Community brand palette** — the public surface (2a, 2b, 2c, and the
  landing-infused onboarding 3a/3c). 2px black hairlines, Helvetica, mint accent.
  The visual seam between the two is intentional: it marks where the private tool
  ends and the public surface begins. The community brand is **light-only** — the
  site declares `color-scheme: light`.

### 2.1 Native palette

| Token | Light | Dark | Used for |
|---|---|---|---|
| `bg.window` | `#F6F7F4` | `#23251D` | App ground / window background; GNOME header bar bg (light) |
| `bg.sidebar.macos` | `#ECEEE8` | `#262922` | macOS `NavigationSplitView` sidebar |
| `bg.chrome.windows` | `#F3F3F0` | `#2B2D28` | Windows title bar and nav rail |
| `surface.card` | `#FFFFFF` | `#21241E` | Cards, sheet header/footer bars, popovers, inputs |
| `surface.inset` | `#EEF2F0` | `#2A2E27` | Manifest strip inside a queue card; segmented-control track |
| `surface.scrim` | `rgba(0,0,0,.06)` | `rgba(255,255,255,.08)` | Search-result code blocks; GNOME segmented track; Windows nav selected row (`rgba(0,0,0,.06)` / `rgba(255,255,255,.08)`) |
| `surface.selected.macos` | `rgba(0,0,0,.07)` | `rgba(255,255,255,.1)` | macOS sidebar selected row; GNOME round close button |
| `hairline` | `#D9DFDC` | `#3B4038` | Card borders, input borders, section rules, sheet dividers |
| `hairline.divider` | `#DDDFD8` | `#373A33` | Sidebar right edge, content-header bottom edge |
| `ink.primary` | `#20241F` | `#E8EAE3` | Body and title text |
| `ink.secondary` | `#5C635B` | `#A6AC9F` | Supporting prose, sub-labels, muted icons |
| `ink.tertiary` | `#8A9086` | `#82887C` | Timestamps, eyebrow labels, footnotes |
| `on.accent` | `#FEFEFE` | `#0B1F19` | Text/glyph on a filled accent button |
| `green.brand` | `#178F70` | `#3FBE9A` | Mark bracket; checkbox fill; chip border (as `rgba(23,143,112,.45)` / `rgba(63,190,154,.5)`) |
| `green.fill` | `#137C61` | `#3FBE9A` | Primary button fill; Windows nav selection bar; text-cursor caret; GNOME count-badge fill |
| `green.text` | `#0F7256` | `#5CD3AF` | Section-header eyebrows, status-chip text/icon, "Recent:" search terms |
| `blue.brand` | `#315FBA` | `#7FA0EC` | Mark bracket; chip border (`rgba(49,95,186,.45)`) |
| `blue.icon` | `#315FBB` | `#9DB6F1` | "Held for privacy review" clock icon and chip text |
| `gold.brand` | `#B9821F` | `#DCAA43` | Warning banner border/icon, "nothing matched" card border |
| `gold.text` | `#8A5F12` | `#E2B75C` | Warning prose, "2 matches" heading, "nothing matched" chip text |
| `gold.highlight` | `rgba(185,130,31,.28)` | `rgba(220,170,67,.32)` | Search-term highlight inside excerpt text |
| `coral.brand` | `#D65D4F` | *(not drawn)* | "Withdrawn by you" chip border (light: also as `rgba(214,93,79,.45)`) |
| `coral.text` | `#B8483B` | *(not drawn)* | "Withdrawn by you" chip text |
| `redaction.chip.bg` | `#f3e3c0` | `#4A3C18` | Redaction marker chip in the transcript renderer |
| `redaction.chip.fg` | `#202426` | `#F0EBDD` | ditto. Measured contrast stated in the doc: **12.3:1** light, **9:1** dark |

Semantic legend given in turn 1's "Assumptions & sources" block:
`#F6F7F4` ground · `#178F70` green "good standing" · `#315FBA` blue "held" ·
`#B9821F` gold "weigh this" · `#D65D4F` coral "refused".
Rule stated: **tone = colour + glyph + words** (never colour alone).

**Near-duplicate hexes — flagged**

- `#315FBA` (mark, chip borders) vs `#315FBB` (icon strokes, chip text). One
  digit apart; almost certainly the same intended blue. Pick one.
- `#178F70` / `#137C61` / `#0F7256` are three greens in the light palette
  (brand / button fill / text). This is a deliberate 3-step ramp, not an error,
  but only `#178F70` is a "brand" green — the other two are darkened for
  contrast on fills and small text.
- `#D9DFDC` vs `#DDDFD8` — two light hairlines, one for card edges and one for
  structural dividers. Distinguishable only side by side.
- `#F6F7F4` vs `#F3F3F0` — window ground vs Windows chrome.
- `#23251D` vs `#21241E` vs `#262922` vs `#2B2D28` vs `#2A2E27` — five dark
  greys within 6 units of each other. They encode elevation; keep them but name
  them.
- `#3B4038` vs `#373A33` — the dark hairline pair.
- macOS content headers use `background:rgba(246,247,244,.9)` (translucent
  `bg.window`) in 1a/1c/1d light and `rgba(35,37,29,.92)` in 1a dark. Windows and
  Linux headers use the opaque value.

### 2.2 Community brand palette (light only)

| Token | Value | Used for |
|---|---|---|
| `brand.accent` | `#00d4aa` | Mint. Primary button fill, headline highlight, mark square, coin face |
| `brand.bright` | `#00ef8b` | Listed in the `4b` swatch row ("bright"). Not used in any product frame. |
| `brand.rim` | `#00b894` | Coin rim / offset disc; globe dashed arc + filled node |
| `brand.yellow` | `#f5c91f` | "The one yellow." Used **exactly once** on the site and exactly once in the app (3c manifesto headline). |
| `brand.tint` | `#eafaf5` | Fill behind acknowledgement rows and the withheld-analytics notice |
| `brand.ink` | `#000000` | 2px frames, all text, hairlines inside brand panels |
| `brand.paper` | `#ffffff` | Brand panel background |
| `brand.muted` | `#6b6b6b` | Mono uppercase micro-labels on white |
| `brand.muted.onblack` | `#8a8a8a` | Same role on the black manifesto screen |

### 2.3 Platform chrome colors

macOS traffic lights: close `#FF5F57`, minimise `#FEBC2E`, zoom `#28C840`;
12 × 12 px circles, 8 px gap.

### 2.4 Doc-viewer chrome (NOT product design — ignore)

`00-global-style.html` sets `body{background:#EBE9E2}`, link `#0F7256`,
hover `#178F70`, ID pills `#1a1a1a`. The only product-relevant leak is that the
viewer reuses the product greens for links.

---

## 3. Typography

### 3.1 Families

| Role | macOS | Windows | Linux | Community brand panels |
|---|---|---|---|---|
| UI sans | `-apple-system, BlinkMacSystemFont, system-ui, sans-serif` | `'Segoe UI Variable Text', 'Segoe UI', system-ui, sans-serif` (title bars and small controls use plain `'Segoe UI', system-ui`) | `Cantarell, Ubuntu, system-ui, sans-serif` | `'Helvetica Neue', Helvetica, Arial, sans-serif` |
| Mono | `ui-monospace, SFMono-Regular, Menlo, Consolas, monospace` (often abbreviated `ui-monospace, Menlo, monospace`) | `Consolas, ui-monospace, monospace` | `ui-monospace, monospace` | `ui-monospace, SFMono-Regular, Menlo, Consolas, monospace` |

Weight conventions differ per platform for the same element: macOS uses 600 for
card titles and 700 for screen titles; Windows drops each by one step (600 → 600,
700 → 600); Linux raises each (600 → 700). Where the mockups differ this is
noted inline in §5.

### 3.2 Native type scale

| Step | Size / weight / line-height | Letter-spacing | Use |
|---|---|---|---|
| `title.screen` | 15px / 700 (Win 600) | — | Content-header title ("Waiting", "History", "Settings") |
| `title.section` | 17px / 700 | — | "3 sessions waiting for your decision" |
| `metric.value` | 20px / 700 (Win 600; Linux 20–22px mono 700) | — | Stat-card numbers (9 / 2 / 31) |
| `metric.value.mono` | 18px / 700 mono | — | Credit figures (1,240 / 180) |
| `heading.alert` | 16px / 700 (Win 600) | — | "2 matches" search result count |
| `title.card` | 13px / 600 (Linux 14px / 700) | — | Session name, history row name |
| `body` | 13px / 400, line-height 1.45 | — | Session summary line, prompts, search field text |
| `body.dense` | 12.5px / 600 | — | Undo-bar headline; disclosure rows use 12.5px / 400 |
| `label.control` | 12px / 500 (mac), 400 (Win), 600 (Linux) | — | Secondary buttons |
| `label.control.primary` | 12px / 600 (Linux 700) | — | Filled buttons |
| `caption` | 11px / 400, line-height 1.45–1.5 | — | Sub-headers, footnotes, agent name, timestamps |
| `caption.small` | 10.5px / 400, line-height 1.5 | — | The read-gate footnote (mac/Win); Linux uses 11px |
| `eyebrow` | 10px / 800, UPPERCASE | `.5px` (mac/Win), `.8px` (Linux) | Field labels: "Would send", "Removed by pattern", "Recorded", "In the commons", "This week", "Credit" |
| `mono.figure` | 12px / 500 mono | — | "148 KB", "12 secrets · 4 file paths · 2 email addresses", "held 41s" |
| `mono.chip` | 11px / 500 mono (Linux 700) | — | Status pill text |
| `mono.badge` | 10px / 500 mono | — | Tab counts ("18", "3") |
| `mono.code` | 11px / 400 mono, line-height 1.5 | — | Search excerpts |
| `mono.transcript` | 11px / 400 mono, line-height **1.7** | — | Transcript renderer body |

Linux consistently widens eyebrow tracking to `.8px` and pushes weights one step
heavier; Windows lightens weights one step. Same sizes throughout.

### 3.3 Community brand type scale

| Step | Spec | Use |
|---|---|---|
| `display.hero` | 700, **50px**, line-height **.88**, letter-spacing **-.04em**, UPPERCASE | Onboarding welcome headline (3a) |
| `display.manifesto` | 700, **44px**, line-height **.92**, letter-spacing **-.035em**, UPPERCASE, `max-width:15ch` | Black manifesto stanza (3c) |
| `display.dialog` | 700, **27px**, line-height **.95**, letter-spacing **-.035em**, UPPERCASE, `max-width:16ch` | Go-public dialog headline (2c) |
| `display.panel` | 700, **24px**, line-height **.95**, letter-spacing **-.035em**, UPPERCASE | Brand panel headings ("Community", "Your public profile") |
| `lede` | 500, **18px**, line-height **1.3**, letter-spacing **-.01em**, `max-width:52ch` (3a) / `44ch` (3c) | Sub-headline paragraph |
| `body.brand` | 500, **13px**, line-height 1.4–1.45, letter-spacing `-.01em` | Brand panel prose, notice boxes |
| `field.value` | 500, **15px** (mono for handle, sans for bio), letter-spacing `-.01em` | Handle and bio field values |
| `figure.brand` | 700, **26px** mono, `font-variant-numeric: tabular-nums`, letter-spacing `-.03em` | Community stat figures (#14, 1,240, 9, 82%) |
| `label.mono` | 700, **11px** mono, UPPERCASE, letter-spacing `.02em`, color `#6b6b6b` | All brand micro-labels, meta rows, page counter |
| `button.brand` | 700, **12px** mono, UPPERCASE (13px in the onboarding screens 3a/3c) | Brand buttons |
| `chrome.mono` | 700, **12px** mono, UPPERCASE, letter-spacing `.02em` | Onboarding header wordmark |
| `link.mono` | 500, **13px** mono, `text-decoration: underline` | "What gets removed?" header link |

Rule stated in 3a: display type sits at "landing scale (uppercase, tight
tracking, .88 line height)".

---

## 4. Spacing, radii, borders, shadows

### 4.1 Spacing scale

Observed values: **2, 3, 4, 5, 6, 7, 8, 10, 12, 14, 16, 18, 20, 22, 24, 26, 28,
30, 32, 34px**. Practical scale:

| Step | px | Typical use |
|---|---|---|
| `space.0.5` | 2 | Sidebar row gap; label→value gap |
| `space.1` | 4 | Segmented-control padding; chip icon gap |
| `space.1.5` | 6 | Intra-group gaps (checkbox stack) |
| `space.2` | 8 | Button gaps, inline metadata gaps, card inner stacks |
| `space.2.5` | 10 | Section stacks, header gaps |
| `space.3` | 12 | Card grid gap, banner icon gap, card padding (compact) |
| `space.3.5` | 14 | Card content gap (queue rows, brand panels) |
| `space.4` | 16 | Card padding-x; content-block gap in History |
| `space.4.5` | 18 | Sheet padding-x; settings section gap |
| `space.5` | 20 | Screen padding-x (brand panels, dialogs) |
| `space.5.5` | 22 | macOS content padding-x |
| `space.6` | 24 | Metric-row gaps in queue manifest |
| `space.7` | 28 | Sheet header field gap |
| `space.8` | 32 | Credit-card figure gap |

Standard content-region padding: macOS `18px 22px 22px`, Windows `18px 22px 22px`,
Linux `16px 20px 22px`. Content headers: `9px 20px`.

### 4.2 Radii

| Token | Value | Use |
|---|---|---|
| `radius.pill` | `999px` | Status chips, count badges |
| `radius.control.mac` | `6px` | Buttons, inputs, sidebar rows, tab items |
| `radius.control.win` | `4px` | Buttons, inputs, nav rows, tab items |
| `radius.control.linux` | `6px` | Buttons, inputs, tab items |
| `radius.card` | `8px` | All cards, banners, segmented-control tracks, transcript panel |
| `radius.chip.inline` | `2px` | Search-term highlight |
| `radius.chip.redaction` | `3px` | Redaction marker chip |
| `radius.checkbox` | `3px` (on a 13×13 box drawn in a 16-unit viewBox as `rx="3"`) | Read-gate checkboxes |
| `radius.window.mac` | `10px` | macOS window and sheet |
| `radius.window.win` | `8px` | Windows window and dialog |
| `radius.window.linux` | `12px` | GNOME window and dialog |
| `radius.circle` | `50%` | Traffic lights, GNOME close button, credit coin |
| — | `0` | **All community brand panels.** No rounding anywhere inside a black-framed panel. |

### 4.3 Borders

- Native hairlines: **1px** solid `hairline` / `hairline.divider`.
- Status-chip borders: 1px, usually a 45–55% alpha of the status hue
  (`rgba(23,143,112,.45)`, `rgba(49,95,186,.45)`, `rgba(185,130,31,.45)`,
  `rgba(214,93,79,.45)`); **Linux uses the solid hue instead** (`#178F70`,
  `#315FBA`, `#B9821F`, `#D65D4F`) and pads chips `2px 10px` rather than `2px 8px`.
- Attention borders on cards/banners: `rgba(185,130,31,.55)` light,
  `rgba(220,170,67,.6)` dark; Linux uses solid `#B9821F` / `#DCAA43`.
- Active tab border: `rgba(23,143,112,.55)` light, `rgba(63,190,154,.6)` dark.
  Linux does not border the active tab — it uses a white/`#21241E` pill with
  `box-shadow:0 1px 2px rgba(0,0,0,.08)`.
- Windows nav selection: a 3px `#137C61` (dark `#3FBE9A`) bar, `border-radius:2px`,
  absolutely positioned `left:0; top:8px; bottom:8px`.
- Windows search field carries an accent underline: `border-bottom:2px solid #137C61`
  (dark `#3FBE9A`) in addition to the 1px box.
- Community brand: **2px solid `#000`** outer frames; **1px solid `#000`**
  internal cell dividers; 1px `#000` on field boxes.
- Section rules: a `1px` tall flex-filling `<span>` in `#D9DFDC` (dark `#3B4038`).

### 4.4 Shadows

Native UI uses **hairlines, not shadows**, inside the window. Shadows appear
only on the mockup window frames themselves (i.e. they describe window elevation
in the design doc, not in-app elevation) and on the GNOME raised tab pill:

| Context | Value |
|---|---|
| macOS window | `0 18px 44px rgba(0,0,0,.16), 0 0 0 1px rgba(0,0,0,.1)` |
| macOS sheet | `0 22px 50px rgba(0,0,0,.22), 0 0 0 1px rgba(0,0,0,.1)` (dark: 1px ring `rgba(0,0,0,.4)`) |
| Windows window | `0 14px 36px rgba(0,0,0,.16), 0 0 0 1px rgba(0,0,0,.18)` |
| Windows dialog | `0 22px 50px rgba(0,0,0,.22), 0 0 0 1px rgba(0,0,0,.18)` |
| Linux window | `0 14px 36px rgba(0,0,0,.18), 0 0 0 1px rgba(0,0,0,.14)` |
| Linux dialog | `0 22px 50px rgba(0,0,0,.24), 0 0 0 1px rgba(0,0,0,.14)` |
| Black manifesto window (3c) | `0 18px 44px rgba(0,0,0,.2), 0 0 0 1px rgba(0,0,0,.2)` |
| GNOME selected tab pill | `0 1px 2px rgba(0,0,0,.08)` |
| macOS menu-bar flyout strip (3b) | `0 0 0 1px rgba(0,0,0,.1)` |

### 4.5 Motion

Stated rule (3a, 3c): the desktop app keeps a **one-motion rule** — the only
animation is the mark's assembly. The globe is still, the coin is still ("Still,
always. The coin only turns on the website."). With reduced motion the manifesto
screen does not invert; it renders ink-on-white.

### 4.6 Frame widths used in the mockups

macOS main window 900px; macOS History 790px; macOS Settings 700px; macOS sheet
760px; macOS History-with-Community 660px; credit-coin frame 560px; manifesto
640px; brand onboarding (`3a`) 860px. Windows main 900px, History 790px, Settings
700px, dialog 760/560px. Linux main 820px, History 720px, Settings 640px, dialog
760/640/560px.

From the recovered sections: native onboarding (`1e`) is **840px** on all three
platforms; the menu-bar / tray frames (`1f`) are **400px** wide on macOS and
Windows (menu 328px, flyout 344px) and **440px** on Linux (notification bubble
inset `margin:14px 24px 0`); every empty/error frame (`1g`) is exactly
**520 × 320px**, and 320px is the only fixed *height* anywhere in the document —
it exists so the centred notice has something to centre in.

These are mockup canvas sizes, not minimum window sizes.

---

## 5. Per-screen specifications

### 5.1 — `1a` Main window / queue

> "One decision surface: health banner (gold, actionable), the undo bar
> (recovery lives on the queue, not behind the sheet), one card per waiting
> session with the fixed manifest strip, then the week band. The only forward
> action on a row is 'Look inside' — Contribute lives inside the preview."

**Variants drawn:** macOS Light, Windows Light, Linux Light, macOS Dark,
Windows Dark, Linux Dark. (6 frames.)

#### Layout

- **macOS:** `NavigationSplitView`. Sidebar 184px fixed, `bg.sidebar.macos`,
  1px right border `hairline.divider`. Traffic lights at `padding:18px 18px 24px`,
  gap 8. Nav list `padding:0 10px`, rows `gap:2px`.
  Content column: header bar (`padding:9px 20px`, bottom hairline
  `#DDDFD8`, translucent ground) then a scrolling stack
  (`padding:18px 22px 22px`, `gap:14px`).
- **Windows:** Title bar 34px (`bg.chrome.windows`) holding the 16px mark,
  "Trace Commons", then `–` `▢` `✕` each in a 44px-wide centred cell
  (font sizes 13 / 10 / 12). Below: 184px nav rail (`bg.chrome.windows`,
  `padding:10px 8px`, rows `padding:7px 10px`, `radius 4px`) + content column,
  same header and stack as macOS.
- **Linux:** No sidebar. 46px header bar with the 20px mark at left, a centred
  segmented **view switcher** (track `rgba(0,0,0,.06)`, `radius 8px`,
  `padding:3px`; items `padding:4px 12px`, `radius 6px`; selected item white with
  the 1px shadow), and a 24px round close button (`rgba(0,0,0,.07)`). The Queue
  item carries a count badge: `#137C61` fill, `#FEFEFE` text, 10px/700,
  `radius 999px`, `padding:1px 6px`. Content stack `padding:16px 20px 22px`.

#### Component inventory, top to bottom

1. **Nav items** — icon 13px + label + spacer + optional count.
   - Waiting: video/monitor glyph `<rect x="2" y="3.5" width="12" height="9.5" rx="1.5"/><path d="M2 9h3l1.5 2h3L11 9h3"/>`
   - History: clock `<circle cx="8" cy="8" r="5.7"/><path d="M8 4.8V8l2.3 1.4"/>`
   - Settings: gear `<circle cx="8" cy="8" r="2.2"/>` + 8 spokes
     `M8 1.6v2.2M8 12.2v2.2M1.6 8h2.2M12.2 8h2.2M3.5 3.5l1.6 1.6M10.9 10.9l1.6 1.6M12.5 3.5l-1.6 1.6M5.1 10.9l-1.6 1.6`
   - Selected row: `background:rgba(0,0,0,.07)` (mac) / `rgba(0,0,0,.06)` + left
     accent bar (Win) / white pill (Linux); icon switches to `green.text`;
     weight 500 (mac) / 600 (Win) / 700 (Linux).
   - Count "3" is 11px/600 `ink.secondary`.
2. **Content header** — title "Waiting", subtitle "Nothing is sent unless you say so.",
   right-aligned: a **Watching chip** and a **Pause split-button**.
   - Watching chip: 1px `hairline` border, `radius 999px`, `padding:2px 8px`,
     11px/500 mono, `ink.secondary`, eye glyph
     `<path d="M1.5 8C3 5.2 5.2 3.7 8 3.7s5 1.5 6.5 4.3C13 10.8 10.8 12.3 8 12.3S3 10.8 1.5 8Z"/><circle cx="8" cy="8" r="2.1"/>`
   - Pause: `surface.card` fill, 1px `hairline`, `padding:4px 10px` (Win `4px 11px`),
     pause bars `M5.8 4v8M10.2 4v8` + 9px chevron `m5 6.5 3 3 3-3`.
   - The Linux frame has **no content header** — its title lives in the switcher.
3. **Health banner (gold, actionable)** — `surface.card` fill, gold border,
   `radius 8px`, `padding:12px 14px`, `gap:12px`. 14px warning triangle
   `<path d="M8 2.2 14.6 13.4H1.4Z"/><path d="M8 6.6v3"/><circle cx="8" cy="11.7" r=".5"/>`.
   - Title: **"One thing to confirm."** (13px/600). *Linux omits the title line.*
   - Body: **"You chose the extra privacy scan, which sends message text to NEAR AI. Confirm you're OK with that and contributions resume."**
   - Action: secondary button **"Review and confirm"**, `white-space:nowrap`.
4. **Undo bar** — `surface.card`, `hairline` border, `radius 8px`,
   `padding:14px 16px` (Linux `14px`), `gap:8px`.
   - Row: 12px blue clock icon + **"Approved parts-catalog. Still on this machine."** (12.5px/600) + mono **"held 41s"**.
   - Body: **"The watcher sends approved sessions on its next sweep. This app cannot see when that lands, so it does not pretend to count it down: undo works until the sweep starts, and says so plainly if it is already too late."**
     *Linux shortens to:* "The watcher sends approved sessions on its next sweep. Undo works until the sweep starts, and says so plainly if it is already too late."
   - Buttons: primary **"Undo"**, secondary **"Let it send"**.
5. **Section title** — **"3 sessions waiting for your decision"** (17px/700).
6. **Session cards** ×3. macOS/Windows: `overflow:hidden` card with a bleeding
   manifest strip; Linux: padded card with an inset manifest block
   (`radius 6px`, `padding:8px 12px`).
   - Header row: name (13px/600; Linux 14px/700) · agent (11px `ink.secondary`) ·
     spacer · relative time (11px `ink.tertiary`).
   - Summary paragraph: 13px/1.45.
   - **Manifest strip**: `surface.inset` fill, `border-top:1px hairline`
     (mac/Win) or a rounded inset block (Linux), `padding:10px 16px`.
     Left: two eyebrow/value pairs at `gap:24px` — "Would send" and
     "Removed by pattern" — then a caption line. Right: **"Not this one"**
     (secondary) and **"Look inside"** (primary), both `white-space:nowrap`.
   - Card 1 — **northwind-billing** · Claude Code · 12 minutes ago.
     Summary: "Refactor the invoice PDF generator to use the new template layout, and fix the VAT rounding on line items — totals drift by a cent on invoices over 100 lines."
     Would send **148 KB**; Removed by pattern **"12 secrets · 4 file paths · 2 email addresses"**;
     caption "Pattern scrubbing removed 18 items. It can miss things — which is why you look first."
   - Card 2 — **dotfiles** · Codex · 1 hour ago.
     Summary: "Set up zsh completions for the deploy script and prune the aliases that shadow coreutils."
     Would send **9 KB**; Removed by pattern **"2 file paths"**;
     caption "Pattern scrubbing removed 2 items. It can miss things — which is why you look first."
   - Card 3 — **acme-support-bot** · Claude Code · 3 hours ago. **Attention variant:**
     the whole card takes the gold border. Summary: "Add retry with backoff to the ticket-sync worker and log the rate-limit headers we get back from the API."
     Would send **63 KB**; Removed by pattern is replaced by a **gold chip
     "nothing matched"** (triangle glyph without the dot, `stroke-width:1.6`);
     caption in `gold.text`: "Nothing matched. On a session that touched credentials, that is itself worth a second look."
     Its metric row uses `align-items:flex-start`.
7. **Standing disclaimer** — 11px/1.5 `ink.secondary`:
   **"Scrubbing is local and pattern-based. It is good and it is not perfect — which is why you look before anything is sent."**
8. **Collapsed disclosure** — 10px right-chevron `m6 4 4 4-4 4` (stroke 1.8,
   `ink.secondary`) + **"Sessions no longer waiting (7)"** at 12.5px/400.
9. **Week band** — section header eyebrow **"This week"** in `green.text` + a
   1px flex rule, then three equal **stat cards** (`gap:12px`).
   - "Contributed" — check-in-circle icon `green.text` — value **9**
   - "Held for privacy review" — clock icon `blue.icon` — value **2**
   - "In the commons" — a bank/columns glyph `M2 13.5h12M3.5 13.5V7.5M6.5 13.5V7.5M9.5 13.5V7.5M12.5 13.5V7.5M2 6.8 8 2.6l6 4.2z`, `ink.secondary` — value **31**
   - Stat card: `surface.card`, 1px `hairline`, `radius 8px`, `padding:12px`;
     icon 11px + eyebrow (`ink.tertiary`); value `margin-top:6px`, 20px/700
     (Win 20px/600 sans; Linux 22px/700 mono).

#### States depicted

Busy queue with three sessions; one card in the gold "nothing matched" attention
state; a health banner requiring confirmation; a pending-send undo bar; a
collapsed group of 7 non-waiting sessions. Dark mode is drawn for all three
platforms. No empty state, no loading state, no error state in this section.

#### Interaction affordances

Per-row: "Not this one" (dismiss) and "Look inside" (opens the preview sheet).
There is **no Contribute button on the queue** — it exists only inside the
preview sheet. Header: Pause (a split-button with a chevron, implying a menu).
The disclosure row expands "Sessions no longer waiting (7)". The health banner's
"Review and confirm" is the only path to resume contributions.

---

### 5.2 — `1b` Preview sheet ("Look inside")

> "Search is first and focused: 'does this mention my client's name?' is
> answerable in five seconds; judging redaction quality by eye is not.
> Contribute waits on the read gate — transcript opened + explicit
> acknowledgement — and has no keyboard shortcut. One sheet, one session, one
> decision."

**Variants drawn:** macOS sheet Light/Dark, Windows modal dialog Light/Dark,
Linux adw dialog Light/Dark. (6 frames, 760px wide.) The tab shown active here
is **Search**.

#### Structure

1. **Dialog chrome** — macOS: none (it is a sheet on the main window).
   Windows: 34px title bar, label **"Look inside — northwind-billing"**, `✕` in
   a 44px cell. Linux: 44px header bar, centred bold title
   **"Look inside — northwind-billing"**, round 24px `✕`.
2. **Sheet header** — `surface.card`, `padding:14px 18px` (Win/Linux `12px 18px`),
   bottom hairline.
   - Identity row: **northwind-billing** (13px/600) · **Claude Code** ·
     spacer · **12 minutes ago**.
   - Two field pairs, `gap:28px`: "Would send" → **148 KB**; "Status" → a green
     chip **"nothing sent yet"** with a padlock glyph
     `<rect x="4.2" y="7" width="7.6" height="6" rx="1.2"/><path d="M6 7V5.2a2 2 0 0 1 4 0V7"/>`.
   - Reassurance line: **"Nothing has been sent. This is what would be."**
3. **Tab bar** — segmented control, track `surface.inset` (Linux uses
   `rgba(0,0,0,.06)` / `rgba(255,255,255,.08)`), `radius 8px` (Win 6px),
   `padding:4px`, item gap 4. Items `padding:5px 12px`, 11px, icon 11px.
   Four tabs, in order:
   - **Search** (magnifier `<circle cx="7" cy="7" r="4"/><path d="m10 10 4 4"/>`)
   - **What's in it** (document-lines) with a mono count badge **18**
   - **Exactly what would be sent** (page-with-fold `M4 1.8h6l3 3v9.4H4z` + lines)
   - **Permissions** (two checks + two rules) with a mono count badge **3**
   - Active item: `surface.card` fill, green-tinted 1px border, weight 700
     (Win 600); inactive items are `ink.secondary`, weight 400, no fill.
4. **Search body** (`padding:14px 18px`, `gap:12px`)
   - Instruction: **"Search this trace for anything you need to be sure isn't in it."** (13px)
   - Search field: `surface.card`, 1px `hairline`, `radius 6px` (Win 4px + a 2px
     green bottom border), `padding:5px 10px` (Linux `6px 10px`), value **"Acme Corp"**.
     macOS draws a caret: a `1.5px × 14px` bar in `green.fill` (dark `#3FBE9A`).
     Trailing **"Search"** button.
   - Recents row: label **"Recent:"** then **staging-db**, **jane@northwind**,
     **AKIA**. macOS/Windows render them as bare `green.text` links; **Linux
     renders them as pill chips** (`#EEF2F0` fill, 1px `hairline`, `radius 999px`,
     `padding:1px 10px`, 11px/600, `ink.primary`).
   - Result count: **"2 matches"** at 16px/700 in `gold.text`, with the 15px
     warning triangle.
   - Two excerpt blocks: `surface.scrim` background, `radius 6px` (Win 4px),
     `padding:8px 10px`, 11px mono / 1.5, with the matched term wrapped in a
     `gold.highlight` span (`radius 2px`, `padding:0 2px`):
     - "…the client is **Acme Corp** — their invoice template still uses the legacy footer, so keep the fallback path until…"
     - "…rename the fixture from **Acme Corp** to a neutral name before we commit the golden files…"
5. **Footer** — `surface.card`, top hairline, `padding:14px 18px`, `gap:10px`.
   - Warning: **"Scrubbing is pattern-based and may have missed something. Look before you send."**
   - **Read gate**, two rows, `gap:6px`, each a 13px checkbox + 11px label:
     - ✅ checked, green fill, white tick `m5 8.2 2 2 4-4.6` (stroke 1.8) —
       label in `ink.secondary`: **"You have opened \"Exactly what would be sent\"."**
     - ☐ unchecked, 1.5px `ink.tertiary` outline — label in `ink.primary`:
       **"I have looked at what would be sent, and I understand scrubbing is pattern-based and may have missed something."**
     - Footnote (10.5px, `ink.tertiary`): **"Contribute stays off until both are done. Looking at the first screen is what this checks — it cannot check that you read all of it, and it does not claim to."**
   - Button row: **"Not this one"** (left), spacer, **"Close"**, **"Contribute"**.
     Contribute is `green.fill` / `on.accent` at **`opacity:.45`** — the disabled
     state, because the second gate box is unchecked.

#### States depicted

Search tab with 2 matches; read gate half-satisfied; Contribute disabled. Dark
mode for all three platforms. Not depicted: zero-match search, the "What's in
it" tab, the "Permissions" tab, the enabled Contribute state, any loading state.

#### Interaction affordances

Tab switching; search submit; recent-term recall; two gate checkboxes (the first
is system-set by opening tab 3, the second is user-set); "Not this one",
"Close", "Contribute". **Contribute has no keyboard shortcut** by design.

---

### 5.3 — `1c` History

> "History — three groups, never one column of mixed semantics. Quarantine reads
> as held, not rejected, and never states a turnaround time. Credit is a record:
> no currency symbol, no fiat estimate, no projections, no streaks. Withdrawn
> traces stay on the list, reading as withdrawn."

**Variants drawn:** macOS Light, Windows Light, Linux Light. (No dark frames.)

#### Structure

1. Same shell as 1a, with **History** selected in the nav.
2. Content header: title **"History"**, subtitle **"What you have contributed,
   and what is still being reviewed."** macOS also shows the Watching chip;
   Windows and Linux do not.
3. **Three stat cards** (`gap:12px`), identical construction to 1a's week band:
   - "In the commons" — green check-in-circle — **31**
   - "Held for privacy review" — blue clock — **2**
   - "Waiting to be scored" — a **dashed** circle
     `<circle cx="8" cy="8" r="5.7" stroke-dasharray="1.8 2.6"/>` — **4**
4. **Group disclosure row** — 10px chevron + 12px blue clock +
   **"Held for privacy review — 2 traces"** at 13px/600 (Linux 700).
5. **Credit section** — eyebrow **"Credit"** in `green.text` + rule; then a card:
   - Figures at `gap:32px`: "Recorded" → **1,240**; "Pending review" → **180**
     (the second value in `ink.secondary`); right-aligned, bottom-aligned
     **"Refreshed 2 hours ago"** (11px, `ink.tertiary`).
   - Disclaimer: **"A credit is a signed record that a contribution was accepted
     into the commons. It is not currency, it is non-transferable, and this app
     makes no projections about it."**
6. **"Everything you've contributed"** section — eyebrow + rule + a right-aligned
   mono count **37**. Then four rows, each a `surface.card` card with 1px
   `hairline`, `radius 8px`, `padding:12px 14px`:
   - **northwind-billing** · 2 days ago · green chip **"In the commons"**
     (check-in-circle). Second line: a **"Withdraw"** secondary button
     (11px, `padding:3px 10px`; Win `3px 11px` `radius 4px`; Linux `3px 12px`
     600 weight, transparent fill).
   - **acme-support-bot** · 5 days ago · blue chip **"Held for privacy review"**
     (clock). Explanatory body: **"Automated checks saw something that might be
     personal and couldn't decide on their own. It has not been rejected, and it
     has not been shared with anyone but the reviewer."**
   - **dotfiles** · 1 week ago · neutral chip **"Waiting to be scored"** (dashed
     circle, `hairline` border, `ink.secondary` text). No body, no action.
   - **client-scratch** · 3 weeks ago · coral chip **"Withdrawn by you"**
     (undo-arrow glyph `<path d="M12 12.5V7.8a3 3 0 0 0-3-3H4.5"/><path d="M6.8 2.5 4.5 4.8l2.3 2.3"/>`).
     No body, no action.

#### States depicted

Populated history with all four row states. No empty state, no error state, no
dark mode.

#### Interaction affordances

"Withdraw" on an accepted trace; the collapsible held-for-review group; nav.
Explicitly **absent**: any turnaround-time estimate on quarantined traces, any
currency symbol or projection on credit.

---

### 5.4 — `1d` Settings

> "Consent list comes from the daemon, never hardcoded; nothing optional is
> pre-checked. 'List my handle publicly' is visually separated because it grants
> no data use at all. Prose column, kept narrow on purpose."

**Variants drawn:** macOS Light (700px), Windows Light (700px), Linux Light
(640px). **No dark frame on any platform** — Settings still has no dark pass.

The sidebar / nav rail is **160px** on both macOS and Windows here, against the
184px of 1a and 1c. Both Settings frames are 700px wide where the queue frames
are 900px, so this may be canvas-proportional rather than intentional, but the
document never reconciles it. See §7.2 item 1.

#### Shell

- **macOS** — `NavigationSplitView`. 160px sidebar, `bg.sidebar.macos`,
  `border-right:1px solid #DDDFD8`; traffic lights `padding:18px 18px 24px`;
  nav list `padding:0 10px`, `gap:2px`, 13px, with **Settings** selected
  (`radius 6px`, `rgba(0,0,0,.07)`, weight 500, gear glyph in `#0F7256`) and
  Waiting still carrying its `3`.
- **Windows** — 34px title bar (16px framed mark, "Trace Commons",
  `–` `▢` `✕` in 44px cells), then a row of a 160px nav rail
  (`bg.chrome.windows`, `border-right:1px solid #DDDFD8`, `padding:10px 8px`,
  `gap:2px`, 12.5px) and the content column. Selected row: `radius 4px`,
  `rgba(0,0,0,.06)`, weight 600, the 3px `#137C61` left bar.
- **Linux** — no sidebar. 46px header bar: 20px mark (`margin-left:6px`),
  centred view switcher with **Settings** as the raised white pill
  (`0 1px 2px rgba(0,0,0,.08)`, weight 700, `#0F7256` gear) and Queue still
  badged `3`, then the 24px round `✕`.
- **Content header** (macOS and Windows only): `padding:9px 20px`,
  `border-bottom:1px solid #DDDFD8`; title **"Settings"** at 15px/700
  (Win 600); subtitle **"What this machine watches, and what your traces are
  allowed to do."** at 11px `ink.secondary`. macOS uses the translucent ground
  `rgba(246,247,244,.9)`; the Windows header declares no background at all.
  Linux has no content header — the switcher is the title.
- **Content column**: `gap:18px`, `padding:18px 22px 22px` (Linux
  `16px 20px 22px`), **`max-width:520px`** ("prose column, kept narrow on
  purpose"). Four sections, in this order: Connection, Startup, How may your
  traces be used?, Projects. Every section is a `gap:8px` column opening with a
  §6.4 eyebrow-plus-rule header in `green.text`.

#### 1. Connection

- **"Connected"** chip. macOS: `500 11px` mono, `padding:2px 8px`, border
  `rgba(23,143,112,.45)`, `radius 999px`, `#0F7256`, with the 10px link glyph
  `M6.5 9.5 9.5 6.5M5 11l-1.2 1.2a2.4 2.4 0 0 1-3.4-3.4L2.9 6.3a2.4 2.4 0 0 1 3.4 0M11 5l1.2-1.2a2.4 2.4 0 0 1 3.4 3.4l-2.5 2.5a2.4 2.4 0 0 1-3.4 0`
  (`transform="scale(.9) translate(1 1)"`). Linux: `700 11px` mono,
  `padding:2px 10px`, **solid `#178F70`** border. **Windows draws the chip with
  no glyph at all** — the only status chip in the document that is words and
  colour without a glyph, and a direct violation of the stated
  colour + glyph + words rule. Treat it as an omission in the frame, not a
  platform rule.
- Three check rows, `gap:7px`, 12px (Linux 12.5px), each with a 12px filled
  green disc `<circle cx="8" cy="8" r="6.2" fill="#178F70"/>` +
  `<path d="m5.2 8.3 1.9 1.9 3.6-4.3" fill="none" stroke="#FEFEFE" stroke-width="1.7"/>`:
  - **"Claude Code sessions folder set"**
  - **"Codex sessions folder set"**
  - **"Extra privacy scan configured"**

  These are status marks, not controls: no unchecked variant is drawn and they
  carry no hit target.

#### 2. Startup

One toggle row, `gap:10px`, label at 13px. **The three platforms disagree on
both the toggle geometry and the copy** — the first place in the document where
copy is not identical across platforms:

| | Track | Fill (on) | Knob | Label |
|---|---|---|---|---|
| macOS | 34 × 20px, `radius 999px` | `#178F70` | 16 × 16px `#FEFEFE`, `top:2px; right:2px` | **"Start Trace Commons when you log in"** |
| Windows | 38 × 19px | `#137C61` | 13 × 13px `#FEFEFE`, `top:3px; right:3px` | **"Start Trace Commons when you sign in"** |
| Linux | 40 × 22px | `#137C61` | 16 × 16px `#FEFEFE`, `top:3px; right:3px` | **"Run in the background at login"** |

Linux alone adds a footnote under the row, 11px `ink.tertiary`:
**"Asked through the background portal, so it works the same inside Flatpak."**

All three are drawn **on**. The off position is still never drawn (§7.2 item 8).

#### 3. "How may your traces be used?" — the consent list

This is the section the truncated import lost entirely, and it is not a list of
toggles: **every consent scope is a card with a leading checkbox**, and the
grouping is done with field labels, not with a different control.

- Section sub-caption, 12px `ink.secondary`:
  **"Applies to traces you send from now on."**
- Then three group labels — `eyebrow` (10px/800, UPPERCASE, `.5px`; Linux
  `.8px`) in **`ink.tertiary` `#8A9086`**, not the `green.text` used by the
  section headers — each followed by its cards:
  - **"Always included"**
  - **"Optional — each one lets your traces do more"**
  - **"Credit"**
- **Consent row card** (the new component): `surface.card`,
  `1px solid #D9DFDC`, `radius 8px`, `padding:11px 12px`, `display:flex`,
  `gap:10px`, `align-items:flex-start`.
  - Leading **14 × 14px** checkbox SVG, `viewBox="0 0 16 16"`, `flex:none`,
    `margin-top:1px`. Checked:
    `<rect x="1.5" y="1.5" width="13" height="13" rx="3" fill="#178F70"/>` +
    `<path d="m5 8.2 2 2 4-4.6" fill="none" stroke="#FEFEFE" stroke-width="1.8"/>`.
    Unchecked: the same rect at `fill="none" stroke="#8A9086" stroke-width="1.5"`,
    no tick. This is the read-gate checkbox of §6.9 at 14px instead of 13px.
  - Title 12.5px/600 (Linux 700); body 12px `ink.secondary`, `margin-top:2px`.
- The four cards, verbatim:

  | Group | State | Title | Body |
  |---|---|---|---|
  | Always included | checked, non-interactive | **"Finding bugs and measuring agents"** | "Your traces can be inspected to find failure modes and measure how agents perform." |
  | Optional | **checked** | **"Turn my traces into test cases"** | "Your traces become benchmark tasks that agents are tested against." |
  | Optional | **unchecked** | **"Train models that judge agent output"** | "Used to train reward and ranking models, not the coding models themselves." |
  | Credit | **unchecked** | **"List my handle publicly as a contributor"** | "Attribution only. This grants no data use at all." |

- The always-on card carries an inline chip beside its title, `gap:8px`:
  **"always on"** at `500 10px` mono, `padding:1px 7px`, border
  `rgba(23,143,112,.45)`, `radius 999px`, `#0F7256`, with the 9px padlock glyph
  `<rect x="4.2" y="7" width="7.6" height="6" rx="1.2"/><path d="M6 7V5.2a2 2 0 0 1 4 0V7"/>`.
  Linux: `700 10px`, `padding:1px 8px`, solid `#178F70` border, **no glyph**.
  Windows: **no glyph** either.
- Closing footnote, 11px/1.5 `ink.tertiary`:
  **"Nothing here is pre-selected on your behalf."**

**How "List my handle publicly" is separated.** Not by control type and not by a
rule or a panel: it is the same consent row card as the data-use scopes, placed
under its own **"Credit"** group label, with the last data-use scope above it and
the "nothing is pre-selected" footnote below. That is the whole of the
separation. Two consequences the truncated spec could not state:

- 2a and 2b both call it a **toggle** by name ("Shown only while 'List my handle
  publicly' is on"). In 1d it is a **checkbox**, and its full label is longer:
  **"List my handle publicly as a contributor"**. The prose in 2a/2b quotes a
  control that is not drawn with that name or that shape.
- It is drawn **unchecked**, consistent with "nothing optional is pre-checked" —
  so the 2a Community section and the 2b public-profile panel are both the
  opted-in variant of a screen whose default state is off.

The list is stated to come from the daemon, so the four cards above are the
*shape* of a scope row, not a fixed set: card count, titles and bodies are
daemon-supplied, and only the three group labels, their order, and the
always-on / optional / credit partition are design.

#### 4. Projects

- Three rows, 12.5px, `gap:8px`: project name · spacer · state in
  `ink.secondary` · a trailing button.

  | Project | State | Button |
  |---|---|---|
  | **northwind-billing** | "Asks you first" | **"Ignore"** |
  | **dotfiles** | "Asks you first" | **"Ignore"** |
  | **client-scratch** | "Never offered" | **"Ask again"** |

- Button: the §6.1 small secondary. macOS `500 11px`, `padding:3px 10px`,
  `#FFFFFF` fill, `1px #D9DFDC`, `radius 6px`. Windows 11px (weight unset →
  400), `padding:3px 11px`, `radius 4px`. Linux `600 11px`,
  `padding:3px 12px`, `radius 6px`, **no fill** (transparent).
- Footnote, 11px/1.5 `ink.tertiary`:
  **"Arming a project so it contributes without asking is a deliberate
  confirmation flow, and it is not built yet."**

  The design therefore ships exactly two project states and one two-way toggle
  between them; a third "arms without asking" state is named and deliberately
  left unbuilt.

#### States depicted

Connected daemon, all three folder checks satisfied, autostart on, one optional
scope granted and one not, public listing off, three known projects across two
states. Not depicted: disconnected, any folder check failing, autostart off, an
empty project list, a daemon that returns no scopes, dark mode on any platform.

#### Interaction affordances

The autostart toggle; three consent checkboxes (the always-on one is inert); the
per-project Ignore / Ask again two-way. No Save button anywhere — every control
in Settings commits on change.

---

### 5.5 — `1e` Onboarding welcome (native)

> "Onboarding — 'What this is', the only hero and the only motion in the app.
> The thesis is the headline. The scrubbing concession stays verbatim and is not
> demoted to small print — it is what makes every later claim credible. The mark
> assembles itself once from its own geometry; there is no other motion
> anywhere. Screens 2–6 (Connect, Consent, Extra privacy scan, What to watch,
> Done) follow the same prose column."

**Variants drawn:** macOS Light, Windows Light, Linux Light — all **840px**.
No dark frame.

**This is a second, different first-run welcome screen.** `1e` is drawn entirely
in the **native palette** — `bg.window` ground, system type, The Turn at 110px, a
1px `hairline` rule. `3a` (§5.11) draws what is recognisably the same screen —
same headline, same lede, word for word — in the **community brand**: 2px black
frame, Helvetica, 50px uppercase display type on a mint highlight, the wireframe
globe, a 6-step counter. They cannot both ship. See §7.2 item 14.

#### Structure

- **macOS** — traffic lights at `padding:16px 18px 4px`; body
  `padding:22px 34px 32px`, `gap:22px`. No sidebar, no content header.
- **Windows** — 34px title bar (16px framed mark, "Trace Commons",
  `–` `▢` `✕`); body `padding:26px 34px 32px`.
- **Linux** — 46px header bar: 20px mark (`margin-left:6px`), centred
  `700 13px` title **"Trace Commons"**, 24px round `✕`; body
  `padding:24px 34px 32px`.

1. **Hero row** — `display:flex`, `align-items:flex-end`, `gap:34px`.
   - Left column, `flex:1`, `gap:14px`:
     - Eyebrow: **"Trace Commons"**, 10px/800 UPPERCASE in `green.text`,
       letter-spacing `.6px` (Linux `.8px`). Note `.6px`, not the `.5px` every
       other macOS/Windows eyebrow uses.
     - **Headline**, two lines split by an explicit `<br>`:
       **"You decide what gets contributed."** /
       **"Nothing is sent unless you say so."**
       Type: **30px / 900 / line-height 1.12** on macOS; **30px / 800** on
       Windows and Linux. This is a new type step — call it
       `display.native.hero`; §3.2 tops out at 20px and has nothing like it.
     - **Lede**, 15px / line-height 1.45, `ink.secondary`: **"Coding agents get
       better when there are real transcripts to learn from. Almost all of that
       data is locked inside companies. Trace Commons is a shared pool that
       isn't."** Also a new step — `lede.native`, the only 15px body text in the
       native palette.
   - Right column: **The Turn, light variant, at 110 × 110px**, `flex:none`,
     bottom-aligned with the lede. All three platforms draw the *framed colour*
     variant here, not the template. This is the largest rendering of the mark in
     any product frame — §1.3 lists 84px as the largest sample.
2. **Rule** — a full-width `1px` `#D9DFDC` div. Not the §6.4 flex-filling span:
   this one spans the whole column with no label beside it.
3. **Two-column prose row** — `gap:34px`, 13px / line-height 1.5, each column
   `flex:1`:
   - **"This app watches for finished Claude Code and Codex sessions on this
     machine and shows them to you."**
   - **"Before anything leaves this machine it is scrubbed locally for secrets,
     keys, and tokens. That scrubbing is good and it is not perfect — which is
     why you get to look first."**

   The second column is the standing concession of §5.1 item 7 promoted to half
   the width of the first screen. The group note is explicit that it must not be
   demoted to small print.
4. **Button row** — `gap:10px`:
   - **"Get started"** — primary. 13px/600 (Linux 700), `padding:8px 16px`
     (Windows and Linux `8px 18px`), `#137C61` fill, `#FEFEFE`, `radius 6px`
     (Win 4px).
   - **"What gets removed?"** — secondary. 13px/500 (Windows weight unset →
     400; Linux 600), same padding, `#FFFFFF` fill + `1px #D9DFDC` (Linux: no
     fill), `radius 6px` (Win 4px).

   Both are a size larger than the §6.1 button scale (13px / `8px 16px` against
   12px / `5px 12px`) — onboarding buttons are their own step.

   In `3a` the same pair reads **"Get started"** and **"Not now"**, with "What
   gets removed?" living in the header as a link. `1e` has no header link and no
   "Not now": there is no way to decline from this screen.

#### The flow

Six screens. `1e` is screen 1, "What this is". Screens 2–6 are named in the group
note: **Connect**, **Consent**, **Extra privacy scan**, **What to watch**,
**Done** — and they "follow the same prose column", i.e. the same 840px frame,
rule and two-column body without the hero. None of the five is drawn.

`1e` draws **no step counter**. `3a` puts **"01 — 06"** in a footer and `3c`
**"03 — 06"**, and `3c`'s step 3 is "The promise", which is not one of the five
names `1e` gives. The two flows are six steps each and share no numbering.

#### Motion

The one-motion rule is restated here and sharpened: "The mark assembles itself
once from its own geometry; there is no other motion anywhere." The assembly is
the mark's own strokes drawing in — it needs no asset beyond the SVG in §1.2.

#### States depicted

First run, nothing configured. Not depicted: the five following screens, any
back navigation, dark mode, and — because "What gets removed?" is a button on
the same row as "Get started" — whatever that button opens.

---

### 5.6 — `1f` Menu bar and tray

> "Menu bar / tray — the mark, a count of decisions owed, and no approve
> buttons. The badge counts decisions owed, never unread anything. The only
> forward action is Review — waiting sessions are inert lines, so nothing
> irreversible is one click from a tray. GNOME has no system tray: on Linux the
> window is the primary surface and the 4-hour digest arrives as a portal
> notification."

**Variants drawn:** macOS Light/Dark, Windows Light/Dark, Linux Light/Dark
(6 frames). This is the only recovered section with a full dark pass.

§6a establishes the status-bar template variant of the mark and both shipping
clients already draw it; what follows is the rest of the surface. The one
exception is the macOS status item itself, where the recovered frame draws
something else — see "the status-item mark" below.

#### macOS — status item + menu

- **Menu-bar strip**: 26px tall, `rgba(246,247,244,.9)` (dark
  `rgba(35,37,29,.92)`), `border-radius:6px 6px 0 0`,
  `box-shadow:0 0 0 1px rgba(0,0,0,.1)`, `padding:0 12px`, `gap:14px`, items
  right-aligned. Our item sits left of the system's own wifi glyph and clock
  (**"Sat Aug 16&nbsp;&nbsp;10:42 PM"**, 12px) — those two are context, not ours.
- **Status item**: a pill, `rgba(0,0,0,.1)` (dark `rgba(255,255,255,.16)`),
  `radius 4px`, `padding:2px 6px`, `gap:4px`, holding the 14px mark and the count
  **"3"** at `500 12px` mono. The count is decisions owed — the same number as
  the Waiting nav badge — and never an unread count.
- **The status-item mark, as drawn.** Not the template variant of §1.2. The frame
  is kept and the two brackets are drawn at *different* inks:

  ```
  <rect x="1" y="1" width="62" height="62" fill="none" stroke="rgba(0,0,0,.55)" stroke-width="2"/>
  <path d="M11 28V11h17" fill="none" stroke="#20241F" stroke-width="7"/>
  <path d="M53 36v17H36" fill="none" stroke="rgba(32,36,31,.35)" stroke-width="7"/>
  ```

  Dark: frame `rgba(255,255,255,.6)`, first bracket `#E8EAE3`, second
  `rgba(232,234,227,.4)`. Stroke stays **7**, not the template's 8.

  This contradicts §1.2 and §6a, and it contradicts what both clients shipped
  (macOS renders `BrandMark(size: 15, variant: .template)`; the GTK tray writes a
  template-variant symbolic icon). It is also the weaker choice on macOS: a
  multi-alpha, framed image cannot be a template image, so the system cannot
  recolour it for the menu bar's light, dark and selected states — precisely the
  property §1.2's template caption asks for. **Keep the template variant; treat
  this frame as superseded by §6a.** Recorded because it is the only place the
  mark is drawn with an asymmetric ink weighting, which may be worth borrowing
  elsewhere.
- **Menu**: 328px, `#FFFFFF` (dark `#21241E`), `radius 6px`,
  `box-shadow:0 12px 34px rgba(0,0,0,.22), 0 0 0 1px rgba(0,0,0,.1)`,
  `padding:5px`, `margin:2px 8px 0 0`, base 13px. Dividers are `1px` `#D9DFDC`
  (dark `#3B4038`) at `margin:4px 10px`. Rows, top to bottom:

  1. **"3 waiting for your decision"** — 13px/600, `padding:4px 10px`.
  2. Three **inert** session lines: `400 12px` mono, `ink.secondary` (dark
     `#A6AC9F`), `padding:1px 10px` (the last `1px 10px 4px`), each indented by
     two `&nbsp;`. Format `<project> — <count> · <size>`:
     **"northwind-billing — 1 · 148 KB"**, **"dotfiles — 1 · 9 KB"**,
     **"acme-support-bot — 1 · 63 KB"**. No hover, no target, no per-row action.
  3. divider.
  4. **"One thing to confirm."** — 13px/600, `padding:3px 10px`.
  5. **"You chose the extra privacy scan, which sends message text to NEAR
     AI."** — 12px `ink.secondary`, `padding:0 10px 4px`. This is the queue
     banner's sentence with its second half — "Confirm you're OK with that and
     contributions resume." — dropped: the menu states the fact and lets Review
     carry the action.
  6. divider.
  7. **"This week: 9 contributed, 2 held for privacy review"** — 12px
     `ink.secondary`, `padding:3px 10px`. The 1a week band collapsed to one
     sentence; the "In the commons: 31" figure is not carried.
  8. divider.
  9. **"Review waiting sessions…"** — the only filled row in the menu:
     `#137C61` fill (dark `#3FBE9A`), `#FEFEFE` text (dark `#0B1F19`),
     `radius 4px`, `margin:0 5px`, `padding:4px 10px`, `gap:8px`, with the 12px
     monitor glyph from the Waiting nav item.
  10. **"Pause"** — `padding:4px 10px`, 12px pause glyph `M5.8 4v8M10.2 4v8`
      (stroke 1.7), spacer, trailing 10px right chevron `m6 4 4 4-4 4`
      (stroke 1.6) marking a submenu. The submenu's items are not drawn.
  11. divider.
  12. **"Open Trace Commons"** — 12px window glyph
      `<rect x="2" y="2.5" width="12" height="11" rx="1.5"/><path d="M2 5.5h12"/>`.
  13. **"Quit…"** — `padding:4px 10px 5px`, no glyph.

#### Windows — tray flyout

- **Flyout**: 344px, sitting above the taskbar with `margin-bottom:8px`.
  `#FFFFFF` (dark `#21241E`), `radius 8px`,
  `box-shadow:0 12px 34px rgba(0,0,0,.22), 0 0 0 1px rgba(0,0,0,.14)`,
  `padding:8px`, base 13px. Dividers `margin:5px 10px`.
- Same heading and three inert mono lines (Consolas), then **no health block** —
  Windows drops "One thing to confirm." entirely — then the week sentence, then
  four action rows at `padding:6px 10px`, `radius 4px`, `margin:0 4px` (Review
  takes `margin:2px 4px`): **"Review waiting sessions"** (filled, and **without**
  the macOS ellipsis), **"Pause"** + chevron, **"Open Trace Commons"**,
  **"Quit"** (also without an ellipsis).
- **Taskbar strip**: 40px tall, `rgba(243,243,240,.95)` (dark
  `rgba(43,45,40,.95)`), `radius 8px`, `0 0 0 1px rgba(0,0,0,.1)`,
  `padding:0 14px`, `gap:12px`, right-aligned: an 11px up-chevron `m4 10 4-4 4 4`
  (stroke 1.6) in `ink.secondary` (dark `#A6AC9F`) for the overflow, then our
  tray item, then the clock at 11px, right-aligned, line-height 1.3, two lines:
  **"10:42 PM"** / **"8/16/2026"**.
- **Windows tray item**: a `rgba(0,0,0,.06)` (dark `rgba(255,255,255,.08)`)
  `radius 4px` pill, `padding:3px 5px`, `gap:3px`, holding **the full-colour
  framed mark at 14px** (light variant; the dark frame uses the dark variant)
  plus a separate count badge — `600 10px` mono, `#137C61` fill / `#FEFEFE`
  (dark `#3FBE9A` / `#0B1F19`), `radius 999px`, `padding:0 4px`, reading
  **"3"**. Windows keeps the colour mark and puts the count in a coloured pill;
  macOS de-saturates the mark and leaves the count as plain mono. That divergence
  is correct — Windows has no template-image contract.

#### Linux — no tray, a portal notification

The design position is explicit: **GNOME has no system tray. On Linux the window
is the primary surface**, and the periodic digest arrives as a portal
notification instead. The frame draws the notification, not a tray.

**DECIDED: read this as GNOME-specific, not Linux-wide.** The GTK client ships
an `org.kde.StatusNotifierItem` (`crates/trace-commons-contributor-gtk/src/tray.rs`)
and keeps it: an SNI item is the right surface on KDE, and on GNOME with an
extension, and it already works. What the frame settles is that GNOME-without-an-
extension must not be left with no ambient surface at all — so the portal
notification is an addition, not a replacement, and it is implementation work
this section specifies. The rule at the end of this section still binds both
surfaces equally: no approve or contribute action in either.

- GNOME top bar: 28px, `#1A1A1A`, `border-radius:8px 8px 0 0`, centred
  `700 11.5px` `#EDEDED`: **"Aug 16&nbsp;&nbsp;22:42"**.
- **Notification bubble**: `#303030`, `radius 12px`,
  `box-shadow:0 12px 34px rgba(0,0,0,.3)`, `padding:14px`, `margin:14px 24px 0`,
  `gap:12px`, `color:#EDEDED`.
  - Leading **34px** mark, `flex:none`. The light frame draws the light
    (white-framed) variant and the dark frame the dark variant — but the bubble
    ground is `#303030` in both, so on a light desktop the notification carries a
    white-framed mark on dark grey. Prefer the dark variant in both.
  - Title `700 13px`: **"Trace Commons"**.
  - Body 12px / line-height 1.45, `#C9C9C9`: **"3 sessions are waiting for your
    decision — northwind-billing, dotfiles, acme-support-bot. Nothing is sent
    unless you say so."** The project names are inlined into the sentence rather
    than listed, and the promise is repeated inside the notification.
  - Two action pills, `margin-top:6px`, `gap:8px`, `700 12px`,
    `padding:4px 14px`, `#454545`, `radius 999px`: **"Review"** and
    **"Dismiss"**. GNOME styles both identically; there is no primary.
- Annotation beneath the frame: **"At most one notification every 4 hours, and
  none when nothing is waiting. Review opens the window at the queue — that is
  the whole of what a notification can do."** Both halves are requirements: a
  4-hour floor between digests, silence at zero, and Review as the only action
  that leads anywhere.

The light and dark Linux frames are otherwise identical — GNOME notifications are
dark in both desktop themes.

#### The rule the section exists to state

**No approve button anywhere in a tray, menu or notification.** Waiting sessions
are inert text; the only forward action is Review, which opens the window at the
queue, from which the only path to sending is the preview sheet and its read
gate. Nothing irreversible is one click from a status item.

#### States depicted

Three waiting, one health item pending (macOS only), a week rollup. Not
depicted: the zero-waiting menu (the badge and the digest are both stated to
disappear, but no frame shows what the menu then says), paused,
disconnected / watcher-not-running, the Pause submenu, and any hover or highlight
state on a menu row.

---

### 5.7 — `1g` Empty and error states

> "Empty & error states — not-running is a first-class state, not a spinner.
> Every failure sentence states the data consequence and never names the
> mechanism. Four of the queue states mean nothing left the machine, and each
> says so in words."

**Variants drawn:** three frames, each **520 × 320px**, **light only**, one per
platform — and each platform carries a *different* state, so no state is drawn
twice and no state is drawn on more than one platform. The states are not
platform-specific; the pairing is a drawing economy.

All three are the same component.

#### The centred notice

The one new component in the section, and the whole of its layout:

```
flex:1; display:flex; flex-direction:column;
align-items:center; justify-content:center;
gap:8px; padding:0 60px; text-align:center
```

- **Title** — 17px / 700, `ink.primary`. Same step as `title.section`. Always a
  complete sentence, always ending in a full stop.
- **Body** — 13px / line-height 1.5, `ink.secondary`. One or two sentences.
- **Optional glyph** — carried *inside* the title row, not above it:
  `display:flex; align-items:center; gap:7px`, glyph at 15px. Only the failure
  state has one.

It fills whatever region it occupies — the content column, or the sheet body —
below that surface's own chrome. There is no illustration, no button, and no
spinner in any of the three frames.

#### State 1 — empty queue

**Frame:** macOS. 520 × 320px, `radius 10px`, the macOS window shadow,
`bg.window`, traffic lights at `padding:16px 18px 0`. Neither the sidebar nor the
content header is drawn — the notice fills the whole window.

- **Glyph:** none.
- **Title:** **"Nothing is waiting."**
- **Body:** **"When a session finishes and goes quiet, it shows up here. Nothing
  is sent unless you say so."**
- **Tone:** no colour at all. This is not a failure, and the absence of any hue
  or glyph is the signal. The promise line is repeated rather than assumed.

#### State 2 — the watcher is not running

**Frame:** Windows. 520 × 320px, `radius 8px`, the Windows window shadow,
`bg.window`, 34px title bar (`bg.chrome.windows`, 16px full-colour framed mark,
"Trace Commons", `–` `▢` `✕` in 44px cells). No nav rail is drawn.

- **Glyph:** none.
- **Title:** **"The watcher isn't running."**
- **Body:** **"It didn't answer. Nothing is being noticed or sent while it's
  stopped, and sessions already waiting stay on this machine."**
- **Tone:** neutral ink. **No coral, no gold, no warning triangle.** Deliberate,
  and the point of the section title: a disconnected daemon is a *state the app
  renders*, not an error it apologises for and not a spinner that never resolves.
  Three consequences in order: it didn't answer / nothing is being noticed or
  sent / what is already queued stays local.
- **Mechanism is never named.** No socket path, no PID, no exit status, no error
  code, no "connection refused". "It didn't answer" is the whole of the
  diagnosis the user is given.
- **No retry button is drawn.** The frame offers no reconnect, no "start it", no
  link to instructions. See §7.2 item 16.

#### State 3 — the preview cannot be shown

**Frame:** Linux, and it is **inside the sheet**, not the main window.
520 × 320px, `radius 12px`, the Linux dialog shadow, `bg.window`; 44px adw header
bar (`#F6F7F4`, `border-bottom:1px solid #D9DFDC`) with a centred `700 13px`
title **"Look inside — client-scratch"** and the 24px round `✕`
(`rgba(0,0,0,.07)`). The sheet header, tab bar and footer of §5.2 are all
replaced — the failure takes the whole body, and the read gate and the Contribute
button are simply not present.

- **Glyph:** 15px, `viewBox="0 0 16 16"`, `fill="none"`, `stroke="#B8483B"`,
  `stroke-width="1.5"`:
  `<circle cx="8" cy="8" r="5.7"/><path d="m5.8 5.8 4.4 4.4M10.2 5.8l-4.4 4.4"/>`
  — an ✕ inside a circle, at the same `r="5.7"` as the History clock and
  dashed-circle glyphs.
- **Title:** **"This one can't be shown."**
- **Body:** **"The session file changed while it was being read. Nothing has been
  sent, and nothing will be until it can be shown to you."**
- **Tone:** coral + ✕-in-circle + words. `#B8483B` is `coral.text`, and this is
  its **first use outside the "Withdrawn by you" chip** — the section promotes
  coral from a chip colour to the error ink. It still has no dark value
  (§7.2 item 3).
- The body names a cause in plain language ("the session file changed while it
  was being read") without naming a mechanism, then closes on the invariant: a
  trace that cannot be shown cannot be sent.

#### Copy rules the section establishes

1. **Every failure sentence states the data consequence.** All three bodies end
   on where the data is: "nothing is sent unless you say so", "stay on this
   machine", "nothing has been sent, and nothing will be".
2. **Never name the mechanism.** No paths, codes, PIDs, or transport words.
3. **Not-running is a state, not a spinner.** It gets a title and a sentence, and
   the app renders it indefinitely without pretending to be busy.
4. **Colour only where this app has failed at its job.** Empty queue and stopped
   watcher take none; only the unreadable session takes coral.
5. **Titles are full sentences with terminal punctuation** — "Nothing is
   waiting.", not "Nothing waiting".

#### States the section does NOT draw

The group note claims "four of the queue states mean nothing left the machine",
but only three frames exist and only two of them are queue-level. Unspecified
even after recovery:

- **Empty History** — no frame shows History with nothing contributed yet; §5.3's
  three stat cards, credit card and 37-row list all carry populated values only.
- **Network / server-unreachable error** — nothing anywhere in the document draws
  a failure to reach the ingest server. The queue and preview are local, so the
  only surfaces that could show one are the credit figures ("Refreshed 2 hours
  ago"), the Community snapshot, and the go-public save.
- **Any loading or in-flight state** — no spinner, progress bar or skeleton is
  drawn in any frame in the document. The section title argues against a spinner
  for *not-running* specifically; it does not say what the app shows while a
  preview is being scrubbed.
- **Zero-match search** inside the preview sheet (§7.2 item 7 already notes the
  Search tab only ever shows "2 matches").
- **Dark variants** of all three states.
- **The other six state × platform combinations.**
- **Any recovery affordance** — none of the three frames carries a button.

The document's own closing "try next" line names what its author knew was still
missing: dark rows for History and Settings, onboarding screens 2–6, the "Exactly
what would be sent" tab with inline `[SECRET]` chips, and the withdrawal
confirmation per tier.

---

### 5.8 — `2a` History → Community section

> "When you're on the roster, History gains a Community section rendered in the
> site's brand — 2px hairlines, Helvetica, mono figures — so the public surface
> reads as visually foreign to the private tool around it. Numbers mirror the
> snapshot payload (rank, novelty credit, accept rate, public-since), and the
> withheld-analytics state is stated in words, never an empty chart."

**Variants drawn:** macOS, Windows, Linux (light only — the brand is light-only).
Placed **below the Credit card in History**.

#### Structure

A **brand panel**: `border:2px solid #000`, `background:#ffffff`, `color:#000`,
Helvetica Neue, `padding:14px`, `gap:14px`, **no border-radius**.

1. Header row (`justify-content:space-between`, `align-items:baseline`):
   - **"Community"** in `display.panel` (700 / 24px / UPPERCASE / `-.035em` / .95).
   - **"View public profile ↗"** — 11px/700 mono, UPPERCASE, underlined.
2. **Metric strip** — a single `border:2px solid #000` box divided into four
   equal cells by `border-right:1px solid #000` (last cell `border-right:none`),
   each `padding:12px 14px`:

   | Label | Value |
   |---|---|
   | Rank | **#14** |
   | Novelty credit | **1,240** |
   | Accepted · 7d | **9** |
   | Accept rate | **82%** |

   Labels are `label.mono` (`#6b6b6b`); values are `figure.brand` (700 / 26px /
   mono / tabular-nums / `-.03em`, `margin-top:6px`).
3. **Meta row** — wrapping, `gap:6px 18px`, all `label.mono`:
   **"Window 7d"**, **"Public since May 12, 2026"**, **"Snapshot 2h old"**.
4. **Withheld-analytics notice** — `border:2px solid #000`, `background:#eafaf5`,
   `padding:12px 14px`, `body.brand`:
   **"Corpus analytics are withheld. The server publishes the roster on consent,
   but will not publish aggregates without an approved noise mechanism — so
   nothing is charted here either."**
5. **Footnote outside the panel**, in native type (11px/1.5, `ink.secondary`;
   Linux uses `ink.tertiary`):
   **"Shown only while "List my handle publicly" is on. Turn it off in Settings
   and this section disappears with it."**

#### States depicted

Roster member, analytics withheld. Not depicted: non-roster (the section simply
does not render), stale-snapshot, or error states.

#### Interaction affordances

"View public profile ↗" opens the public web profile (external).

---

### 5.9 — `2b` Settings → public profile management

> "The 'List my handle publicly' toggle grows into profile management: handle, a
> 280-byte plaintext bio, and leaving the roster. The brand-styled panel draws
> the exact boundary of what is public — everything outside the black frame stays
> private."

**Variants drawn:** macOS, Windows, Linux (light only). It **replaces the Credit
card in Settings once opted in.**

#### Structure

Same brand panel construction as 2a.

1. Header: **"Your public profile"** (`display.panel`) · right-aligned
   `label.mono` **"On the roster since May 12, 2026"**.
2. **Handle field** — `label.mono` **"Handle"**, then a value box:
   `border:1px solid #000`, `padding:8px 12px`, 500 / 15px / mono, value **"manian"**.
3. **Bio field** — `label.mono` **"Bio — 280 bytes, plaintext, no HTML"**, then a
   value box: `border:1px solid #000`, `padding:8px 12px`, 500 / 15px sans,
   `letter-spacing:-.01em`, `line-height:1.4`, **`min-height:56px`**, value
   **"Ships billing systems by day. Contributes the traces that survive review."**
   Below it, right-aligned `label.mono` counter **"74/280"**.
4. **Buttons** (`gap:10px`, wrapping): **"Save profile"** — brand primary
   (`#00d4aa` fill, 2px `#000` border, 700 / 12px mono UPPERCASE,
   `padding:10px 16px`); **"Leave the roster"** — brand secondary (`#fff` fill,
   same border and type).
5. **Footnote outside the panel** (native 11px/1.5):
   **"Attribution only — being listed grants no data use at all. Leaving the
   roster removes you from future snapshots."**

#### States depicted

Opted-in with a bio written and 74 of 280 bytes used. Not depicted: empty bio,
over-limit counter, in-flight save, handle-taken error.

---

### 5.10 — `2c` Go public — the opt-in dialog

> "Going public is a deliberate consent dialog, not a toggle flip: what gets
> published and what never does sit side by side, nothing is pre-checked, and Go
> public stays disabled until the acknowledgement is checked. Real handles only —
> the roster takes no pseudonyms."

**Variants drawn:** macOS sheet, Windows dialog (title **"Go public — Trace
Commons"**), Linux adw dialog (header title **"Go public?"**). 560px wide, light only.

#### Structure

Body is a pure brand surface: `background:#ffffff`, `color:#000`, Helvetica,
`padding:20px`, `gap:16px`.

1. Headline (`display.dialog`, `max-width:16ch`):
   **"Put your handle on the public roster?"**
2. **Two-column consent box** — `border:2px solid #000`, split by
   `border-right:1px solid #000`, each column `padding:12px 14px`, `gap:8px`:
   - **"What gets published"** — "Your handle — real handles only, no pseudonyms.
     Aggregate counts: accepted, novelty credit, accept rate. The date you went
     public. Your bio, if you write one."
   - **"What never does"** — "Your traces or anything in them. Per-trace data of
     any kind. Anything about sessions you didn't send."
3. **Acknowledgement row** — `border:2px solid #000`, `background:#eafaf5`,
   `padding:12px 14px`, `gap:10px`, `align-items:flex-start`. Checkbox is a bare
   **14 × 14px square with a 2px `#000` border, unfilled** (`margin-top:1px`).
   Label: **"I understand my handle and aggregate counts become public. Leaving
   the roster removes me from future snapshots."**
4. **Buttons**, right-aligned, `gap:10px`: **"Not now"** (brand secondary) and
   **"Go public"** (brand primary at **`opacity:.4`** — disabled).
5. Footnote (11px/500, `#6b6b6b`): **"Nothing is pre-checked, and Go public stays
   off until the acknowledgement is on. This changes attribution only — it grants
   no data use."**

#### States depicted

Unchecked acknowledgement, disabled primary. The checked/enabled state is not
drawn.

---

### 5.11 — `3a` Onboarding welcome (landing-infused)

> "The site's hero language moved into the first-run window: the 2px ink frame,
> display type at landing scale (uppercase, tight tracking, .88 line height) with
> the promise line carried on the mint highlight the site uses for live nav, mono
> uppercase labels, and the wireframe globe with its minority mint edges and
> dashed signal arcs. The globe is still — the desktop app keeps its one-motion
> rule (the mark's assembly), so the globe only turns on the website."

**Variants drawn:** macOS only, 860px, light. Caption states: "the other five
screens keep the frame + type, drop the globe" — so this is screen 1 of a 6-step
flow.

#### Structure

macOS window (traffic lights only, no title text) containing a **framed brand
page**: `margin:0 18px 18px`, `border:2px solid #000`, `background:#fff`,
Helvetica, `padding:12px`, `gap:26px`.

1. **Header bar** — `border-bottom:2px solid #000`, `padding-bottom:14px`,
   space-between:
   - Left: the mark at 26px (**the circuit mark in this frame — superseded; per
     6a this becomes The Turn**) + **"Trace Commons — Contributor"** in
     `chrome.mono`.
   - Right: **"What gets removed?"** in `link.mono` (500 / 13px mono, underlined).
2. **Hero row** (`gap:30px`, `align-items:center`, `padding:0 8px`):
   - Left column (`flex:1`, `gap:20px`):
     - Headline in `display.hero`, three lines, the last two carried on a mint
       highlight (`background:#00d4aa; padding:0 4px`):
       **"You decide what gets contributed."** / **"Nothing is sent"** /
       **"unless you say so."**
     - Lede: **"Coding agents get better when there are real transcripts to learn
       from. Almost all of that data is locked inside companies. Trace Commons is
       a shared pool that isn't."**
     - Buttons (`gap:12px`, 700 / 13px mono UPPERCASE, `padding:12px 18px`,
       2px `#000` border): **"Get started"** (`#00d4aa`) and **"Not now"** (`#fff`).
   - Right column: the **wireframe globe**, 230 × 230px, `viewBox="0 0 200 200"`:
     ```
     circle  cx=100 cy=100 r=86              stroke #000  1.5
     ellipse cx=100 cy=100 rx=86 ry=30       stroke #000  1.2
     ellipse cx=100 cy=100 rx=86 ry=62       stroke #00d4aa 1.5
     ellipse cx=100 cy=100 rx=30 ry=86       stroke #000  1.2
     ellipse cx=100 cy=100 rx=62 ry=86       stroke #000  1.2
     path M40 68 Q 100 8 162 82              stroke #000    1.5  dash 4 3
     path M52 148 Q 120 190 168 118          stroke #00b894 1.5  dash 4 3
     rect x=36  y=64  7×7  stroke #000 1.5 (no fill)
     rect x=158 y=78  7×7  stroke #000 1.5 (no fill)
     rect x=48  y=144 7×7  fill #00b894
     rect x=164 y=114 7×7  stroke #000 1.5 (no fill)
     ```
     "Minority mint edges": exactly one of five ellipses, one of two arcs and one
     of four nodes are mint. The globe is **static**.
3. **Footer** — `border-top:1px solid #000`, `padding-top:12px`, space-between,
   both in `label.mono`:
   - **"Scrubbed locally · shown to you · sent only on your word"**
   - **"01 — 06"** (step counter)

---

### 5.12 — `3c` Credit coin + manifesto takeover

> "Two louder borrowings. The coin (coin.js: mint faces, ink linework, #00b894
> rim) becomes the Credit card's emblem in History — held still on desktop. The
> manifesto inversion (transcript.js: page flips to black, headline in the site's
> one yellow) becomes onboarding's privacy stanza: the single black screen in the
> flow, with the stanza counter in the corner."

Two frames, macOS only.

#### 5.12.1 Credit card with the coin (560px)

Native History surface (`bg.window`, macOS traffic lights, `padding:4px 22px 22px`).

- Section header: eyebrow **"Credit"** in `green.text` + 1px rule.
- Card: `surface.card`, 1px `hairline`, `radius 8px`, `padding:14px 16px`,
  `gap:16px`, `align-items:center`.
- **The coin** — a 64 × 64px positioning box holding two stacked discs:
  - Rim/shadow disc: `position:absolute; left:3px; top:2px; 58 × 58px;
    border-radius:50%; background:#00b894`
  - Face disc: `left:0; top:0; 58 × 58px; border-radius:50%;
    background:#00d4aa; border:2px solid #000`, centring a **"$"** glyph at
    700 / 34px Helvetica Neue in `#000`.
- Right column: figure pair at `gap:32px` — "Recorded" **1,240**, "Pending review"
  **180** (value in `ink.secondary`) — both `label`+`mono.figure` at 18px/700.
- Disclaimer: **"A credit is a signed record that a contribution was accepted. It
  is not currency — the $ on the coin is the website's joke, and the app keeps
  the disclaimer."**
- Footnote below the card (11px, `ink.tertiary`): **"Still, always. The coin only
  turns on the website."**

#### 5.12.2 Onboarding privacy stanza — the one black screen (640px)

Window `background:#000`, Helvetica Neue, `color:#fff`, macOS traffic lights.
Body `padding:18px 34px 30px`, `gap:22px`.

1. Top row (space-between), both `label.mono` in `#8a8a8a`:
   **"The promise"** · **"03 — 06"**.
2. Headline in `display.manifesto`, color **`#f5c91f`**, `max-width:15ch`:
   **"Nothing is sent unless you say so."**
3. Lede (`lede`, `#fff`, `max-width:44ch`): **"Every trace is scrubbed on this
   machine, shown to you first, and sent only when you press the one button that
   sends it. There is no other path out."**
4. Buttons (`gap:12px`, `margin-top:6px`, 700 / 13px mono UPPERCASE,
   `padding:12px 18px`, **2px `#fff` border**): **"Continue"** (`#00d4aa` fill,
   `#000` text) and **"Back"** (`#000` fill, `#fff` text).
5. Footnote (11px/500, `#8a8a8a`): **"The site's yellow (#f5c91f) appears exactly
   once there and exactly once here. With reduced motion the site never inverts;
   this screen would follow the same setting and render ink-on-white."**

**Accessibility requirement stated:** under `prefers-reduced-motion`, this screen
renders ink-on-white instead of inverting.

---

### 5.13 — `4a` Transcript renderer ("Exactly what would be sent")

> "Clicking Look inside re-previews against the daemon, which pins the entry to
> the exact envelope shown; the third tab renders the redacted transcript as flat
> monospace text (deliberately not chat bubbles — these are the literal bytes an
> approval covers), and highlight_redactions() turns the pipeline's
> `<PRIVATE_*>` / `[REDACTED*]` markers into bold gold chips so you see where
> scrubbing fired, not just how often. Opening this tab satisfies the first half
> of the read gate; Contribute stays off until the acknowledgement."

**Variants drawn:** macOS Light and macOS Dark, 760px. This is **tab 3 of the
preview sheet (1b)**, so the shell is identical to 1b with these differences:

- Identity row adds a chunk indicator: **"Claude Code — 1 of 3"**.
- "Would send" reads **"148 KB"** followed, in `ink.secondary`, by
  **"(the session file on disk is 412 KB)"**.
- The second header field is **"Scrubbing found"** → **"12 secrets · 4 file paths · 2 email addresses"** (replacing 1b's "Status" chip).
- The active tab is **"Exactly what would be sent"**.
- The Search-tab body is replaced by the renderer.

#### Renderer

1. Caption (11px/1.5, `ink.secondary`), containing a live chip sample:
   **"These are the exact bytes an approval covers. Marks like `<PRIVATE_SECRET_1>`
   show where scrubbing fired — legible as chips, not holes."**
2. **Transcript panel** — `surface.card`, 1px `hairline`, `radius 8px`,
   `padding:12px 14px`, `white-space:normal`, `word-break:break-word`,
   type `mono.transcript` (400 / 11px / **1.7**).
   - **Turn separators** — 700 weight, `ink.tertiary`, `margin-top:8px` on all but
     the first: `— user · turn 1 —`, `— assistant · turn 2 —`,
     `— tool: bash · turn 3 —`, `— assistant · turn 4 —`.
   - **Redaction chip** — inline `<span>`: `font-weight:700`,
     `background:#f3e3c0`, `color:#202426`, `border-radius:3px`, `padding:0 4px`
     (dark: `#4A3C18` / `#F0EBDD`). Measured contrast: 12.3:1 light, 9:1 dark.
   - **Truncation footer**: `⌄ 144 more turns` in `ink.tertiary`, `margin-top:8px`.
   - Verbatim body text:
     - turn 1: "Refactor the invoice PDF generator to use the new template
       layout, and fix the VAT rounding on line items. Config is in
       `<PRIVATE_PATH_1>`/billing.toml if you need it."
     - turn 2: "I'll look at the current generator first, then the template.
       Reading billing.toml for the layout constants."
     - turn 3: "$ grep -r \"vat_rate\" src/ · src/invoice/totals.rs:41: let vat =
       line.subtotal * vat_rate; · .env:3: STRIPE_SECRET_KEY=`<PRIVATE_SECRET_1>`
       · .env:4: SMTP_LOGIN=`[REDACTED_EMAIL_1]`"
     - turn 4: "The rounding drifts because VAT is computed per line and then
       summed. I'll switch to summing subtotals first, rounding once at the
       total, matching the fixture in `<PRIVATE_PATH_2>`/golden/vat_100_lines.json…"
3. Footer is 1b's footer minus the "Scrubbing is pattern-based…" line — the read
   gate rows and the four buttons only. The first gate box is checked (this tab
   is open); Contribute remains at `opacity:.45`.

Explicit design constraints stated: **flat monospace text, deliberately not chat
bubbles**; redaction markers render as legible chips, **not holes**.

---

## 6. Component inventory

### 6.1 Buttons

| Variant | Light | Dark | Notes |
|---|---|---|---|
| Primary (filled) | `#137C61` fill, `#FEFEFE` text, 12px/600, `padding:5px 12px`–`5px 14px`, `radius 6px` | `#3FBE9A` fill, `#0B1F19` text | Win `radius 4px`, `padding:5px 13px`; Linux 700 weight, `padding:5px 14px`. Instances: "Look inside", "Undo", "Contribute". |
| Primary disabled | as above + **`opacity:.45`** | same | "Contribute" in 1b/4a |
| Secondary (outlined) | `#FFFFFF` fill, 1px `#D9DFDC`, `ink.primary`, 12px/500 | `#21241E` fill, 1px `#3B4038` | Linux uses `background:transparent` and 600 weight. Instances: "Not this one", "Close", "Let it send", "Review and confirm", "Search", "Pause". |
| Small secondary | 11px/500, `padding:3px 10px`, `radius 6px` | — | "Withdraw" in History (Win `3px 11px` `radius 4px`; Linux `3px 12px` 600) |
| Split-button | secondary + 9px chevron `m5 6.5 3 3 3-3` | — | "Pause" |
| Brand primary | `#00d4aa` fill, 2px `#000` border, `#000` text, 700 / 12px mono UPPERCASE, `padding:10px 16px` | n/a | "Save profile", "Go public". Onboarding uses 13px / `padding:12px 18px`. |
| Brand secondary | `#fff` fill, 2px `#000`, `#000` text, same type | n/a | "Not now", "Leave the roster", "Back" (on black: `#000` fill, 2px `#fff`) |
| Brand primary disabled | as brand primary + **`opacity:.4`** | n/a | "Go public" |
| Brand text link | 700 / 11px mono UPPERCASE, underlined | n/a | "View public profile ↗" |

### 6.2 Status chips (pills)

Base: `display:inline-flex; align-items:center; gap:4px; padding:2px 8px;
border:1px solid <hue @45–55% alpha>; border-radius:999px; font:500 11px mono`.
Linux override: `padding:2px 10px`, `font-weight:700`, solid hue border.
Each carries a 10px glyph — colour + glyph + words, never colour alone.

| Chip | Text | Border | Text color | Glyph |
|---|---|---|---|---|
| Good standing | "In the commons" | `rgba(23,143,112,.45)` | `#0F7256` | check in circle |
| Held | "Held for privacy review" | `rgba(49,95,186,.45)` | `#315FBB` | clock |
| Unscored | "Waiting to be scored" | `#D9DFDC` | `#5C635B` | dashed circle |
| Refused | "Withdrawn by you" | `rgba(214,93,79,.45)` | `#B8483B` | undo arrow |
| Attention | "nothing matched" | `rgba(185,130,31,.45)` | `#8A5F12` | warning triangle (no dot) |
| Locked | "nothing sent yet" | `rgba(23,143,112,.45)` | `#0F7256` | padlock |
| Watching | "Watching" | `#D9DFDC` | `#5C635B` | eye |
| Connected | "Connected" | `rgba(23,143,112,.45)` | `#0F7256` | link |

Dark equivalents: green → border `rgba(63,190,154,.5)`, text `#5CD3AF`; blue →
text `#9DB6F1`; gold → border `rgba(220,170,67,.5)`, text `#E2B75C`; neutral →
border `#3B4038`, text `#A6AC9F`.

### 6.3 Count badges

- **Nav count** (macOS/Windows): plain text, 11px/600, `ink.secondary`.
- **Switcher count** (Linux): `#137C61` fill, `#FEFEFE` text, 10px/700,
  `radius 999px`, `padding:1px 6px`. Dark: `#3FBE9A` on `#0B1F19`.
- **Tab count** (preview sheet): plain mono, 10px/500, inline after the label.
- **Section count** (History): mono 11px/500, `ink.tertiary`, right-aligned in
  the section header.

### 6.4 Section header (eyebrow + rule)

`display:flex; align-items:center; gap:12px` — an `eyebrow` label in
`green.text` (dark `#5CD3AF`), then `<span style="flex:1;height:1px;background:#D9DFDC">`
(dark `#3B4038`), optionally followed by a right-aligned mono count.
Used for: "This week", "Credit", "Everything you've contributed", "Connection",
"Startup".

### 6.5 Cards

| Card | Spec |
|---|---|
| **Stat card** | `surface.card`, 1px `hairline`, `radius 8px`, `padding:12px`. 11px icon + `eyebrow` in `ink.tertiary` at `gap:5px`; value `margin-top:6px` at 20px/700 (Linux 22px mono/700, Win 20px/600). |
| **Session card** | `surface.card`, 1px `hairline` (gold when flagged), `radius 8px`, `overflow:hidden`. Header `padding:11px 16px 4px`; summary `padding:0 16px 12px`; manifest strip as its footer. Linux instead: `padding:14px`, `gap:8px`, manifest as an inset rounded block. |
| **Manifest strip** | `surface.inset`, `border-top:1px hairline`, `padding:10px 16px`, `align-items:flex-end`, `gap:16px`. Left: metric pairs at `gap:24px` + caption; right: button pair. Linux: `radius 6px`, `padding:8px 12px`, no border. |
| **Banner (attention)** | `surface.card`, 1px gold, `radius 8px`, `padding:12px 14px`, `gap:12px`, 14px triangle icon `margin-top:1px`, flexible body, trailing nowrap action. |
| **Undo bar** | `surface.card`, 1px `hairline`, `radius 8px`, `padding:14px 16px`, `gap:8px`; icon + title + mono elapsed; body; button pair. |
| **History row card** | `surface.card`, 1px `hairline`, `radius 8px`, `padding:12px 14px`; optional `gap:8px` column for a body line or an action. |
| **Credit card** | as History row card, with a 32px-gap figure pair, right-aligned "Refreshed …", and a disclaimer line. |
| **Brand panel** | `border:2px solid #000`, `background:#fff`, `color:#000`, Helvetica, `padding:14px`, `gap:14px`, **radius 0**. |
| **Brand metric strip** | `border:2px solid #000` containing N equal cells split by `1px solid #000`, each `padding:12px 14px`. |
| **Brand notice** | `border:2px solid #000`, `background:#eafaf5`, `padding:12px 14px`. |

### 6.6 Segmented controls

- **Preview tab bar**: track `surface.inset`, `radius 8px` (Win 6px),
  `padding:4px`, `gap:4px`. Item `padding:5px 12px`, `radius 6px` (Win 4px),
  11px, icon 11px + label + optional mono count. Selected: `surface.card` fill +
  green-tinted 1px border + weight 700 (Win 600). Linux selected: white/`#21241E`
  pill with `0 1px 2px rgba(0,0,0,.08)`, no border.
- **Linux view switcher** (header): track `rgba(0,0,0,.06)` / `rgba(255,255,255,.08)`,
  `radius 8px`, `padding:3px`; item `padding:4px 12px`, `radius 6px`, 12px;
  selected white pill, 700 weight, green icon.

### 6.7 Nav / sidebar rows

macOS: `padding:5px 8px`, `radius 6px`, `gap:8px`, 13px. Selected:
`rgba(0,0,0,.07)` + weight 500 + green icon.
Windows: `padding:7px 10px`, `radius 4px`, `gap:10px`, 12.5px. Selected:
`rgba(0,0,0,.06)` + weight 600 + green icon + a 3px `#137C61` left bar
(`top:8px; bottom:8px; radius 2px`).

### 6.8 Toggle (1d)

`position:relative`, `radius 999px`, knob `radius 50%` in `#FEFEFE`, label at
13px with `gap:10px`. The three platforms disagree on both size and fill:

| | Track | Fill (on) | Knob | Knob offset |
|---|---|---|---|---|
| macOS | 34 × 20px | `#178F70` | 16 × 16px | `top:2px; right:2px` |
| Windows | 38 × 19px | `#137C61` | 13 × 13px | `top:3px; right:3px` |
| Linux | 40 × 22px | `#137C61` | 16 × 16px | `top:3px; right:3px` |

The fill split is unexplained: macOS uses `green.brand`, the other two use
`green.fill` — the token every other filled control uses. Prefer `green.fill`
everywhere. The off state is still not drawn on any platform; mirroring the knob
to the matching `left` offset is the obvious reading.

### 6.9 Checkboxes

- **Native read gate** — 13 × 13px SVG, `viewBox="0 0 16 16"`,
  `<rect x="1.5" y="1.5" width="13" height="13" rx="3">`.
  Checked: `fill:#178F70` (dark `#3FBE9A`) + tick `m5 8.2 2 2 4-4.6`,
  stroke `#FEFEFE` (dark `#0B1F19`), `stroke-width:1.8`.
  Unchecked: `fill:none`, `stroke:#8A9086` (dark `#82887C`), `stroke-width:1.5`.
- **Consent checkbox** (1d) — the read-gate checkbox at **14 × 14px** instead of
  13, `flex:none`, `margin-top:1px`, same `viewBox`, same rect, same tick.
  Unchecked: `fill:none`, `stroke:#8A9086`, `stroke-width:1.5`. This is the only
  place in the document where the unchecked native box appears at 14px.
- **Settings check-marks** (1d Connection, not interactive) — 12px,
  `<circle r="6.2" fill="#178F70">` + tick `m5.2 8.3 1.9 1.9 3.6-4.3`, stroke
  `#FEFEFE`, `1.7`.
- **Brand checkbox** (2c) — a bare 14 × 14px `<span>` with `border:2px solid #000`,
  no fill, `flex:none`, `margin-top:1px`.

### 6.10 Text inputs

- Native search field: `surface.card`, 1px `hairline`, `radius 6px`,
  `padding:5px 10px` (Linux `6px 10px`), 13px. macOS shows a `1.5 × 14px`
  `green.fill` caret; Windows adds `border-bottom:2px solid #137C61`.
- Brand field box: `border:1px solid #000`, `padding:8px 12px`, no radius; bio
  box `min-height:56px` with a right-aligned `label.mono` byte counter beneath.

### 6.11 Disclosure row

10px chevron `m6 4 4 4-4 4`, `stroke-width:1.8`, `ink.secondary`, then a 12.5px
label; optionally a second 12px status icon between them (History's
"Held for privacy review — 2 traces" uses chevron + clock + 13px/600 label).

### 6.12 Code / excerpt blocks

`surface.scrim` background, `radius 6px` (Win 4px), `padding:8px 10px`,
`mono.code` (11px/1.5). Matched terms wrapped in `gold.highlight` with
`radius 2px; padding:0 2px`.

### 6.13 Redaction chip

`font-weight:700`, `background:#f3e3c0`, `color:#202426`, `border-radius:3px`,
`padding:0 4px`. Dark: `#4A3C18` / `#F0EBDD`.

### 6.14 Window chrome

- **macOS**: three 12px circles (`#FF5F57`, `#FEBC2E`, `#28C840`), `gap:8px`,
  `padding:18px 18px 24px` (sidebar) or `16px 18px 12px` (full-width frames).
- **Windows**: 34px bar, 16px mark at `margin:0 8px 0 12px`, 12px title, then
  three 44px-wide centred cells `–` (13px) `▢` (10px) `✕` (12px).
- **GNOME**: 46px header bar (44px on dialogs), 20px mark at `margin-left:6px`,
  centred 13px/700 title or the view switcher, and a 24px round `✕`
  (`rgba(0,0,0,.07)` light, `rgba(255,255,255,.1)` dark).
- **macOS menu bar** (3b): 26px-tall strip, `rgba(246,247,244,.9)`,
  `radius 6px`, `0 0 0 1px rgba(0,0,0,.1)`; the template mark at 15px inside a
  `rgba(0,0,0,.1)` `radius 4px` pill with a mono count, then the clock
  ("Sat Aug 16  10:42 PM").

### 6.15 Illustrations

- **Wireframe globe** — §5.11, 230px, `viewBox 0 0 200 200`, static.
- **Credit coin** — §5.12.1, 58px discs with a 3px/2px offset rim, static.

### 6.16 Centred notice (1g)

The empty / error / not-running component. `flex:1`, column,
`align-items:center`, `justify-content:center`, `gap:8px`, `padding:0 60px`,
`text-align:center`. Title 17px/700 `ink.primary` (a full sentence, terminal
punctuation); body 13px/1.5 `ink.secondary`; optional 15px glyph carried inside
the title row at `gap:7px`. No button, no illustration, no spinner. Error glyph:
`<circle cx="8" cy="8" r="5.7"/><path d="m5.8 5.8 4.4 4.4M10.2 5.8l-4.4 4.4"/>`
at `stroke:#B8483B`, `stroke-width:1.5`.

### 6.17 Consent row card (1d)

`surface.card`, `1px hairline`, `radius 8px`, `padding:11px 12px`,
`display:flex`, `gap:10px`, `align-items:flex-start`. Leading 14px §6.9 consent
checkbox; title 12.5px/600 (Linux 700) with an optional inline `always on` chip
at `gap:8px`; body 12px `ink.secondary`, `margin-top:2px`. Grouped by `eyebrow`
labels in `ink.tertiary` — **not** the `green.text` used for section headers.

### 6.18 Menu / flyout rows (1f)

- **Heading row** — 13px/600, `padding:4px 10px`.
- **Inert list line** — `400 12px` mono, `ink.secondary`, `padding:1px 10px`,
  indented two `&nbsp;`, no hit target. Format `<project> — <count> · <size>`.
- **Action row** — `gap:8px`, 12px leading glyph, `padding:4px 10px` (Win
  `6px 10px`), `radius 4px`; a trailing 10px right chevron marks a submenu.
- **Primary action row** — the same, filled `green.fill` / `on.accent`,
  `margin:0 5px` (Win `2px 4px`). Exactly one per menu.
- **Divider** — `1px` `hairline`, `margin:4px 10px` (Win `5px 10px`).

---

## 7. Open questions and gaps

### 7.1 Truncation, and recovery — resolved

**This section is history, not an open gap.** It is kept because the first pass of
this spec — and the implementation built from it — were written against a
truncated source, so anything that looks unaccountably thin in `1d`–`1g` in the
shipped clients traces back to here.

**What happened.** The mockup document was first imported whole and the read was
cut at 256 KiB, mid-attribute, inside section `1d`. The first version of this spec
recorded the consequences accurately: `1d` was specified only as far as its
Startup toggle, and `1e`, `1f`, `1g` were listed as absent entirely, with `1g`
called "the single largest gap".

**Recovery.** The source document was later split into two files, each under the
read cap, and all four sections were read in full. §5.4 is now written from the
complete `1d` frame — which supersedes the 5 KB fragment, adds the consent list
and the Projects section, and adds Windows and Linux variants the fragment did
not contain — and §5.5, §5.6, §5.7 are written from `1e`, `1f`, `1g`. Nothing in
the mockup document is now unread.

**What the truncated pass got wrong, and is corrected above:**

- `1d` was described as "macOS Light only". It has macOS, Windows **and** Linux
  light frames. It still has no dark frame — that part was right for the wrong
  reason.
- The consent list and "List my handle publicly" were inferred from 2a/2b's
  prose, which calls the latter a **toggle**. It is a **checkbox** in a card,
  under a "Credit" group label, and its label is longer than 2a/2b quote. See
  §5.4 §3 and §7.2 item 15.
- The spec's front matter asserted "Copy is identical across all three
  [platforms]". `1d`'s Startup section falsifies that: three platforms, three
  different sentences. See §7.2 item 17.
- `1g` was called the single largest gap. It is now specified — but only three
  frames were ever drawn, so empty History, any network error and any loading
  state remain unspecified. That residue is tracked as §7.2 items 16 and 18
  rather than as a truncation problem.

### 7.2 Unresolved within what survives

1. **Sidebar width disagrees**: 184px in 1a and 1c, **160px** in 1d. **Still
   open after recovery, and now better characterised** — the complete 1d frame
   shows 160px on macOS *and* on the Windows nav rail, so it is consistent within
   1d rather than a one-off. Both Settings frames are 700px wide against 900px
   for the queue, so the narrowing may be canvas-proportional. Nothing in the
   document states a rule. Pick 184px: it is used by two screens to Settings'
   one, and a sidebar that resizes when you open Settings is worse than a wide
   one.
2. **`#315FBA` vs `#315FBB`** — two blues one digit apart, used interchangeably
   for the mark/borders vs icons/text. Almost certainly one colour.
3. **No dark palette for coral** (`#D65D4F` / `#B8483B`). **Still open, and now
   more urgent**: 1g promotes `#B8483B` from a chip border to the error glyph
   ink (§5.7 state 3), and 1g is drawn light-only, so the app's error colour has
   no dark value at all.
4. **No dark values for 1c History, 1d Settings, 2a/2b/2c, 3a, 3c, 4b, 5a.**
   The community brand is explicitly light-only (`color-scheme: light`), which
   resolves 2a/2b/2c/3a/3c. **Partly settled by recovery:** 1f is drawn in full
   dark on all three platforms — the only recovered section that is. 1d, 1e and
   1g are light-only, so **1c History, 1d Settings, 1e onboarding and 1g's three
   states all still need a dark pass.** The document's own closing "try next"
   line asks for "dark rows for History and Settings too", confirming they were
   never drawn rather than lost.
5. **Section 6a says the new mark rolls out across every frame in turns 1–2**,
   but the turn-1/2 frames in the file still contain the older mark geometry in
   some places (3a's onboarding header uses the circuit mark; the queue/History
   title bars already use The Turn). Treat The Turn as correct everywhere and the
   circuit mark as website-only.
6. **The onboarding flow is 6 steps, and there are now two of them.** 3a is step
   1 of a brand-styled flow ("01 — 06") with 3c as its step 3 ("The promise"),
   and 1e is step 1 of a *native* flow whose steps 2–6 the recovered group note
   names outright: **Connect, Consent, Extra privacy scan, What to watch, Done**.
   "The promise" is not among them. **Recovery settles the step names and leaves
   the flow's visual language open** — see item 14. Either way only two screens
   of six are drawn in the brand flow and one of six in the native flow; the rest
   are unspecified beyond "same prose column" / "keep the frame + type, drop the
   globe".
7. **Preview sheet tabs 2 and 4** — "What's in it" (badge 18) and "Permissions"
   (badge 3) — are never shown. Only "Search" (1b) and "Exactly what would be
   sent" (4a) have bodies.
8. **Enabled Contribute state is never drawn**, only the `.45`-opacity disabled
   state. Same for the enabled "Go public" (`.4` disabled only). **Partly settled
   by recovery:** 1d's consent list draws the **unchecked** native checkbox at
   14px (`fill:none`, `stroke:#8A9086`, `stroke-width:1.5`), which the truncated
   pass had only in the read gate. The **Startup toggle is still drawn on in all
   three platforms**, so the off knob position remains unspecified.
9. **Windows has no client.** All Windows frames are proposals mirroring the Mac
   layout under Fluent chrome.
10. **Focus, hover, pressed and keyboard-focus rings are never depicted** for any
    control on any platform. Recovery does not help: 1f draws menu and flyout
    rows with no hover state either, which is the one surface where a hover
    treatment is normally mandatory.
11. **Scroll behaviour, minimum window sizes and responsive breakpoints** are not
    specified; the widths in §4.6 are mockup canvas sizes.
12. **Bio validation behaviour** (what happens at 280/280 or above) is not drawn;
    only the 74/280 counter.
13. **"View public profile ↗"** target is unspecified — presumably the community
    website.

Opened by the recovered sections:

14. **Two incompatible onboarding welcome screens.** `1e` (§5.5) and `3a`
    (§5.11) are the same screen — same headline, same lede, verbatim — drawn in
    two different visual languages: native palette, system type, The Turn at
    110px, native buttons; versus 2px ink frame, Helvetica, 50px uppercase
    display type on a mint highlight, the wireframe globe, and a step counter.
    They differ on the secondary button too ("What gets removed?" versus "Not
    now"), on whether declining is possible at all, and on the six steps' names.
    `1e` is `1d`'s neighbour in turn 1; `3a` is turn 3 and is described as
    "landing-infused". Nothing in the document says which supersedes which. One
    of them has to go, and the choice sets whether first-run reads as the private
    tool or as the public brand.

    **DECIDED: `3a` ships; `1e` is superseded.** First-run reads as the public
    brand. `3a` is the later turn, it is what is implemented and rendered today
    (`OnboardingWelcomeView.swift`), and it carries the seam the rest of the
    design rests on — the community language marks what becomes public, and the
    first screen is where a contributor meets the commons rather than the tool.
    `1e` stays specified in §5.5 as the record of the alternative; it is not an
    implementation target. The step names `1e` gives (Connect, Consent, Extra
    privacy scan, What to watch, Done) are still the useful part of it, since
    `3a` never names its six.
15. **"List my handle publicly" is a checkbox, not a toggle.** 2a and 2b both
    name it as a toggle in user-visible copy; 1d draws it as a consent checkbox
    card labelled **"List my handle publicly as a contributor"**. Either the
    footnotes in 2a/2b need rewording to match the control, or the control needs
    to become a toggle. The shipped clients follow 1d's shape and 2a/2b's words.
16. **No recovery affordance on any failure state.** 1g's "The watcher isn't
    running." carries no retry, no "start it", and no route to instructions; the
    unreadable-preview state carries no re-try either. The design is deliberate
    about not naming the mechanism, but it does not say what the user is meant to
    *do*, or whether the app retries on its own and simply re-renders when the
    daemon answers.
17. **Copy is not identical across platforms after all.** The front matter of this
    document asserts it is. 1d's Startup section draws three different sentences
    ("…when you log in" / "…when you sign in" / "Run in the background at
    login"), and Linux alone adds a Flatpak-portal footnote. 1f diverges too:
    macOS shows the health item in the menu and Windows drops it; macOS uses
    ellipses on "Review waiting sessions…" and "Quit…" where Windows does not.
    The Windows and Linux deltas are platform-idiomatic and probably right —
    but the blanket claim needs retiring, and the Linux portal sentence is a
    functional difference, not a wording one.
18. **Empty History, network errors and loading states are still unspecified.**
    1g draws three states; its own note implies four; neither an empty History
    nor any server-unreachable condition nor any in-flight indicator appears
    anywhere in the document. The credit figures ("Refreshed 2 hours ago"), the
    Community snapshot ("Snapshot 2h old") and the go-public save are all
    server-dependent and all drawn only in their success state.
19. **The macOS status-item mark contradicts §1.2 and §6a.** 1f draws a framed,
    two-alpha mark at stroke 7 where §1.2 specifies the frameless, single-ink
    template variant at stroke 8, and both shipping clients implement the
    template. §6a is authoritative for the mark, and a multi-alpha framed image
    cannot be a macOS template image — keep the template. Recorded in §5.6
    because it is the only asymmetric ink treatment of the mark in the document.

### 7.3 Stated design rules to preserve

- Nothing is sent unless the user says so; Contribute exists only inside the
  preview sheet and has **no keyboard shortcut**.
- The read gate is two conditions: transcript tab opened **and** an explicit
  acknowledgement. The UI states plainly that it cannot verify the user read
  everything.
- Tone = colour + glyph + words. Never colour alone.
- Hairlines, not shadows, inside the window.
- Quarantine reads as *held*, never *rejected*, and never states a turnaround time.
- Credit is a record: no currency symbol in the native UI, no fiat estimate, no
  projections, no streaks. (The `$` appears only on the website's coin, and the
  app keeps the disclaimer next to it.)
- Withdrawn traces stay on the list, reading as withdrawn.
- Nothing optional is pre-checked; the consent list comes from the daemon.
- Community/public surfaces are rendered in a deliberately foreign visual
  language; the black frame is the exact boundary of what becomes public.
- Analytics that are withheld are stated in words, never as an empty chart.
- One-motion rule: only the mark animates. Globe and coin are static on desktop.
- `#f5c91f` is used exactly once in the product.
- Reduced motion: the manifesto screen renders ink-on-white instead of inverting.
- No approve or contribute action in any tray, menu bar or notification. Waiting
  sessions are inert lines; the only forward action is Review, which opens the
  window at the queue. Nothing irreversible is one click from a status item.
- A tray badge counts **decisions owed**, never unread anything.
- Digest notifications are rate-limited to at most one every four hours and are
  silent when nothing is waiting.
- Not-running is a first-class state with a title and a sentence, never a spinner.
- Every failure sentence states the **data consequence** and never names the
  mechanism — no paths, codes, PIDs or transport words.
- Failure colour is used only where the app has failed at its job: an empty queue
  and a stopped watcher take none.
- Nothing in the consent list is pre-selected, and the card that grants no data
  use is separated from the ones that do by its own group label.
