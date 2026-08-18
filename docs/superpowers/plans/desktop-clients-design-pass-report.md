# Implementation notes — desktop clients design pass

Running record of what the design pass changed, what it deliberately did not,
and what needs a decision. Compiled from the per-surface implementation agents.

## Source truncation

The design document `Desktop Clients.dc.html` is 266.5 KB and the import tool
caps a read at 256 KiB, so the tail never arrived. Recovered: turns 2-6 in full
and `1a`-`1c` in full. Lost: the back half of `1d` Settings, and `1e`
(onboarding), `1f` (menu bar / tray), `1g` (empty and error states) entirely.

`1g` is the costliest gap: no surviving frame shows an empty queue, an empty
history, a failed daemon connection, a network error, or a loading state.

To recover the rest, split the file in the design project so each half is under
256 KiB, then re-import.

## Verification

| Surface | How it was checked | Result |
|---|---|---|
| macOS | `swift build` on this host | builds clean |
| Linux | Debian container mirroring the CI job's package set, real `gtk4 0.7.3` + `libadwaita 0.5.3`, `RUSTFLAGS=-Dwarnings` | builds clean; 38 unit tests pass |
| Windows | not compiled here (WinUI is Windows-only); XAML well-formedness via `xmllint`, every resource key proven against the pinned SDK's `generic.xaml`, and the cross-platform Interop project built | CI confirms at `.github/workflows/ci.yml:490` |

The Linux container is the meaningful check: a Homebrew GTK build on macOS type-checks
the Rust but says nothing about the `cfg(linux)` paths or the zbus/StatusNotifierItem
tray code.

Still unverified anywhere: **how any of this looks**. Nobody has run the apps and
looked at them. GTK in particular skips a CSS declaration it cannot parse and merely
logs it, so a stylesheet mistake surfaces as an unstyled widget rather than a failure.
The Linux header switcher is the specific risk — `AdwViewSwitcherTitle` was replaced
with a hand-built track of grouped `GtkToggleButton`s, and its failure mode is quiet:
three plain buttons in a row instead of one segmented control. That, the Queue count
badge, and the selected-tab icon recolour are the first things to look at on a real
session.

## Consistency gaps closed along the way

These were not design problems. They were found because implementing one spec
across three clients meant reading what each of them actually did, and the
clients had drifted apart.

### The read gate is now enforced identically on every client

§7.3 specifies the gate as two conditions — the transcript tab opened AND an
explicit acknowledgement — and the clients did not agree on enforcing both.
They do now.

Each client arms Contribute through a single path that requires every
condition at once, so no code path can arm the button while forgetting one.
On Linux that path is:

```rust
fn sync_contribute(&self) {
    let ready = self.pinned.get()
        && self.gate_opened.is_active()
        && self.gate_acknowledged.is_active();
    self.contribute.set_sensitive(ready);
}
```

### "Withdrawn" read as still in flight (Linux history)

A withdrawn record fell through to "Waiting to be scored", violating §7.3's rule
that withdrawn traces stay on the list reading as withdrawn. It is now a
first-class status with the coral chip, pinned by a unit test.

### Windows nearly labelled disk size as "Would send"

Following the mockup's label literally would have made the queue card state
something false about what leaves the machine. A queue entry's `size_bytes` is
the session file on disk; `PreviewSummary::would_send_bytes` is what would be
sent, and is usually larger. `model.rs:57-60` says so in terms: "Never label
this one 'would send'." Windows has no preview loaded and cannot compute the
real figure, so the card reads "Session on disk", which is what macOS already
calls that field.

Checked on the other two clients: macOS labels "Would send" but binds
`summary.wouldSendBytes`, which is correct. Linux does not reference
`size_bytes` on the card at all.

### The Linux desktop entry has never had an icon

See section 8 below.

### The Windows header truncated the app's central promise

Found by rendering the built app on a Windows VM. The content header's subtitle
rendered as "Nothing is sent unless" — the status pill was winning the space
contest on the same row and clipping "you say so." at exactly the word that
makes the sentence mean something.

## Deviations forced by reality

### The hero cannot be 50pt (§3.3 `display.hero`)

Measured against Helvetica Neue Bold at the spec's -.04em tracking, the
headline "YOU DECIDE WHAT GETS CONTRIBUTED." is **948pt wide at 50pt**. The hero
column maxes out at 780pt inside the spec's own 860pt canvas, after the
margins, the 2px frame and the 12px padding. 50pt does not fit in any window
this app can open, with or without the globe.

It now resolves through a measured `ViewThatFits` ladder that states each rung's
column width as the measured width of the longest line at that size, so the
choice is exact and resolves in one pass.

The first attempt put the headline in a column beside the globe and landed at
30pt, dropping the globe entirely at narrow widths. That was a trade, not a
solution: the copy fixes the line breaks, so a narrower column cannot make a
line need less width — it can only make the type smaller. Sharing the measure
with the globe was costing the headline about a third of its size.

The headline now spans the full measure and the two-column split starts beneath
it, lede and button left, globe right. Two independent ladders, both measured.
Result: **36pt with the globe at the shipping 940pt window**, 36pt with the
globe at the 900pt capture, and 30pt with a 145pt globe at 660pt. The no-globe
rung survives as a floor for a genuinely tiny window; nothing normal reaches it.

The related leading bug is worth recording because the diagnosis was initially
wrong. Reaching a .88 line height by subtracting from stack spacing needs
-.341em, since the real line box is 1.221em. At that spacing each line's mint
background — sized to the 1.221em line box, not the .88em line — overlapped the
line above by more than its empty descender space, so line 2's mint block
painted over the bottom of line 1's capitals. The glyphs were not colliding; the
background was covering them. Fixed by setting an explicit .88em frame per line
with zero stack spacing, which is what a browser does with `line-height: .88`.

### Capture-path divergence in the macOS screenshot harness

`ImageRenderer` rasterizes a `ScrollView` as blank and paints a `TextField`'s
caret as a solid "no entry" glyph, both already documented in this codebase
(`ConsentScopesView.swift:40`, `OnboardingConnectView.swift:96-104`). Two
"defects" in the preview sheet turned out to be these artifacts rather than app
bugs — proven by finding the identical yellow bar in a file the pass had never
touched.

The captures are how these screens get reviewed, and a capture showing a gold
block where the search field is says the opposite of what the app does, so the
capture path was fixed rather than the app. The divergence is bounded:

```swift
static let isRendering = DebugScreenshot.directory != nil
```

and `DebugScreenshot.directory` is nil unless `TRACE_COMMONS_SCREENSHOT_DIR` is
set to a non-empty value at launch. Two readers, both presentational. A
contributor's build cannot reach either path.

## Open decisions

### 1. Community section has no data behind it (macOS)

`HistoryView` now renders the Community section per `2a`, but every figure in it
is model-less: `Models.swift` has no rank, accept rate, novelty credit in
window, public-since, snapshot time, profile URL, or roster-membership flag, and
`AppModel` has no roster publisher.

Rather than hardcode the mockup's fixtures, the section takes an optional
`RosterSnapshot` whose fields each omit their own cell, and it renders nothing
when the roster is nil — which is always, today. Wiring a daemon roster call is
a feature, not a design pass.

**Needs:** a daemon roster call, or a decision to drop the section.

### 2. The held explanation is now said twice (macOS History)

The disclosure group above the held rows already explains what "held" means, in
copy with a documented rationale. `2a` also specifies a per-row held sentence,
which the app had no copy for. Both are now present, so an expanded group reads
the assurance once at the group and once per row.

Options: keep both (current); suppress the row sentence while the group is
expanded (needs expansion state passed into the row); or drop the row sentence
and let the group carry it, leaving the mockup's held row without a body.

### 3. Shared macOS components drifted from the spec — RESOLVED

`DesignSystem.swift` was off-limits to the screen agents so they could run in
parallel, which left `TCTag` and `TCPrimaryButtonStyle` off-spec and three
screens each carrying a private copy of the §6.9 checkbox. A consolidation pass
has since fixed all of it: `TCTag` moved to `mono.chip` (11/500) and 2x8
padding, `TCPrimaryButtonStyle` to 12/600 at 5pt vertical, and the checkbox was
promoted to `TCReadGateCheckbox`. Both corrections shrink the component, and all
seventeen call sites were checked for a fixed frame — none has one.

The four private copies of the community brand were folded into
`CommunityBrand.swift`. Two of them disagreed in ways that mattered:

- **Dynamic Type.** One file built every Helvetica step with
  `Font.custom(_:fixedSize:)`, opting the brand panels out of Dynamic Type
  entirely; the other three used `size:`, which scales. The scaling form won —
  it is the majority and the accessible one, and the fixed-size rationale
  (tracking quoted against a nominal size) survives because every tracking token
  is now the spec's em value times that nominal size.
- **Black and white.** Two files used SwiftUI's `Color.black` / `Color.white`;
  two used sRGB literals. The literals won, and this is the one disagreement
  that could actually have rendered differently: a system colour is entitled to
  resolve per appearance, which is exactly what a light-only palette must never
  do.

### 4. Public profile and go-public are drawn but inert (macOS Settings)

Same shape as the Community problem. `DaemonStatus` carries no handle, bio or
roster date, there is no profile-write call, and there is no consent-write path
in this build — so the §5.6 panel renders against empty values with Save and
Leave disabled, and §5.7's "Go public" stays off. The mockup's fixtures
("manian", the bio sentence, "74/280") appear nowhere in the code.

The agent wrote two sentences of its own to explain this to a contributor, in
the register of the neighbouring disclaimer, so the empty fields read as an
honest account rather than as a bug. **These are new copy, not from the spec,
and want a decision:** keep them, or render nothing at all until the contract
carries a profile.

### 5. Two deliberate deviations from the spec in the preview sheet

- §5.10 says the transcript tab's footer drops the scrubbing line. It was kept
  on every tab: dropping it on the tab where Contribute is most likely to be
  clicked weakens the sentence the whole read gate rests on.
- Zero-match search stays green rather than gold. The spec never depicts a
  zero-match search; the existing code treats "0 matches" as a clean answer to
  the question the tab exists for. The gold "nothing matched" treatment went to
  the case that means "no pattern fired", which is the one worth a second look.

Also presentational, and easily reverted: "Residual risk: <value>" became a
section header reading "Residual risk" with the value beneath. Same words.

### 6. Sidebar width still not pinned

§7.2 records the mockups disagreeing (184px in `1a`/`1c`, 160px in `1d`); the
decision was 184. `MainWindowView.swift` still declares
`navigationSplitViewColumnWidth(min: 180, ideal: 200)`.

### 7. The spec's `ink.tertiary` fails WCAG and was refused on all three clients

NEEDS SIGN-OFF — this is a deliberate deviation from the approved mockups.

§2.1 states `ink.tertiary` as `#8A9086` light and `#82887C` dark, and assigns it
entirely to small text: timestamps, eyebrow labels, footnotes. Measured against
the grounds it actually sits on, it clears 4.5:1 nowhere:

| pair | ratio |
|---|---|
| `#8A9086` on `#F6F7F4` (light ground) | 3.04:1 |
| `#8A9086` on `#FFFFFF` (light surface) | 3.27:1 |
| `#82887C` on `#23251D` (dark ground) | 4.26:1 |
| `#82887C` on `#21241E` (dark surface) | 4.31:1 |

All three clients now ship the nearest accessible twin on the same hue and
saturation instead: `#6D7269` light, `#878D81` dark, measuring 4.58:1 / 4.93:1
and 4.55:1 / 4.61:1. The refusal is recorded at each definition site rather than
substituted silently.

This is the move the palette already makes for `green.text`, `gold.text` and
`coral.text`, with the reasoning already written into both codebases: the
palette is tuned for fills and borders where 3:1 is the bar, and small type
needs a darkened twin.

One further constraint: the dark twin measures 4.05:1 on the inset surface, so
it is unusable there. Small type inside a manifest strip stays on
`ink.secondary`.

### 8. The Linux desktop entry has never had an icon (pre-existing bug)

Found while wiring The Turn into the tray. `flatpak/ai.tracecommons.Contributor.desktop`
declares `Icon=ai.tracecommons.Contributor`, and `flatpak/ai.tracecommons.Contributor.yml`
installs the binary and the desktop file and nothing else. No icon asset exists
anywhere in the crate, and there is no `.metainfo.xml`. That `Icon=` key has
never resolved to anything.

This is not a stale reference to the superseded circuit mark — a search found no
reference to that mark anywhere in the crate or its packaging. It is a reference
to an asset that was never shipped at all.

The tray now works around it at runtime by writing a private freedesktop icon
theme under `dirs::data_dir()/trace-commons-shell/icons` and handing the host an
`IconThemePath`. Packaging still needs a real install of a framed-Turn SVG at
`/app/share/icons/hicolor/scalable/apps/ai.tracecommons.Contributor.svg`. Since
the mark is generated rather than shipped, that is either a checked-in SVG kept
in step with `mark.rs` or a build step that emits it. Packaging was out of scope
for this pass.

### 9. Two Linux follow-ups the tray pass could not take

- `notify.rs` now calls `tray::icons()` for the icon path, so notify depends on
  tray. That is backwards. The icon-theme writer belongs in a shared `ui::icons`
  module, or absorbed into `ui::mark`, which already documents its SVG emitters
  as existing for surfaces that can only take a serialised icon.
- `IconThemePath` is a KDE extension and some hosts ignore it. Serving those
  needs `IconPixmap`, which needs an ARGB32 rasteriser. Writing one inside
  `tray.rs` would duplicate the bracket geometry that `mark.rs` deliberately
  owns, so it was not done. `mark.rs` should absorb an
  `argb32(ink, size) -> Vec<u8>` helper and `tray.rs` should export from it.

### 10. The spec contradicts itself on the credit coin

§5.9.1 draws a `$` glyph at 700/34px in the coin face. §7.3 states that credit
is a record: no currency symbol in the native UI, and the `$` appears only on
the website's coin. The `$` was left off, following §7.3, and the disclaimer
beside it reads correctly about the website's coin either way. Yours to
overturn.

## Resolved in passing

- `#315FBA` vs `#315FBB` — two blues one digit apart in the mockups, used
  interchangeably for the mark and borders versus icons and text. Standardised
  on `#315FBA` across all three clients.
- Sidebar width — 184px in `1a`/`1c`, 160px in `1d`. Standardised on 184px.
- The circuit/solder-dot mark on mint is superseded as the client mark. It
  survives as the community website mark only. All three clients now carry
  The Turn.

## Copy policy applied

Where the app already had copy carrying a documented rationale, that copy won
over the mockup's. The clearest case is the Linux client's `residual_risk_line`,
which varies the sentence by redaction count because an identical warning
repeated down a column becomes wallpaper; the mockup's fixed sentence is the
worse version. Where the design introduced an element the app had no copy for,
the spec's copy was taken verbatim.
