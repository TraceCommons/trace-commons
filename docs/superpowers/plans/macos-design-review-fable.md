# Design review: macOS shell design pass (a990a4c0)

Reviewed against the before/after screenshots in
`docs/superpowers/plans/macos-design-pass/`, the designer's report, the
source in `macos/Sources/TraceCommonsApp/Views/`, and the community site
(`community/public/styles.css`, `index.html`). Contrast figures below are
computed WCAG ratios, not estimates. Advisory only; no code was changed.

## Verdict

The direction is right and most of the judgement calls hold. "A customs
declaration, not a feed" is the correct metaphor for this product and the
manifest strip earns its place as the one bold move. But the pass has a
blind spot exactly where the brief says the design must be judged: the
moment of consent. The Contribute button is bound to the Return key, is
enabled before the payload has ever been shown, chains straight into the
next session, and in Dark Mode its label measures 2.3:1 — the worst
contrast in the product, on the one irreversible control, introduced by
the same pass that correctly refused to set a warning sentence at 2.9:1.

---

## 1. Does the hierarchy match the stakes?

Mostly yes, and the delta from "before" is large. The before-card set four
things at one weight and threw the two actions to opposite ends of the
window; the after-card reads project → prompt → manifest → decision, which
is the order of a reader's actual questions. The monospaced-figures /
never-monospaced-prose rule is the best small decision in the pass: a
payload size is findable on any card without reading a word. Protect it.

Three hierarchy problems remain:

- **The redaction summary leaks detector taxonomy and double-reads.**
  "2 secret · 1 secret:aws access key · 1 secret:github token" makes a
  reader do set arithmetic: is the total 2 or 4? The code
  (`QueueRow.removedSummary`) prints raw category keys sorted by count, and
  the tab badge sums all values, so the badge says 4 while the strip reads
  like it might say 2. The most consequential figure on the card — how much
  scrubbing fired — is the one figure the card renders ambiguously.
  Internal keys like `secret:aws access key` are detector namespace, not
  contributor language.
- **Size is under-weighted relative to its risk.** "3 KB" in footnote-size
  monospace is fine for 3 KB. A 400 KB session — a hundred files of source —
  renders at the same visual weight. The manifest treats every payload as
  the same size of decision. The rhythm-break treatment given to
  "nothing matched" (gold border, pill, different sentence) is exactly the
  right mechanism; a large-payload card deserves the same break and does
  not get one.
- **The queue binds Return on every row.** Each `QueueRow` attaches
  `.keyboardShortcut(.defaultAction)` to its "Look inside" button, so a
  two-row queue registers Return twice. Which row fires is whatever SwiftUI
  happens to resolve. Harmless today only because the action is the safe
  one; still a defect.

## 2. The repeated caveat

The three-placement split is two-thirds successful.

- **Placement 1 (per-card, varying) works.** This is the best idea in the
  pass. The sentence now differs by what scrubbing did, so it must be read
  to be parsed, and the more dangerous case ("Nothing matched a pattern.
  That is not the same as nothing being there.") is the louder one. The
  copy is honest and the tone mapping (attention only when nothing matched)
  is exactly right. Keep verbatim.
- **Placement 2 (once per screen) is acceptable wallpaper.** Tertiary info
  glyph, secondary text, foot of the list. It will not be read after the
  first day, but a mechanism statement attached to the list is the correct
  home for it, and it costs nothing.
- **Placement 3 (at commit) fails as executed, in three ways.** First, it
  is not "at the moment of commitment" — it is a permanent fixture of the
  sheet's footer, visible from the instant the sheet opens, on every tab,
  for every session, whether or not Contribute is even plausible yet. That
  is the definition of wallpaper; the sheet is the room a repeat
  contributor spends the most time in. Second, it is not where the report
  says it is: the footer is 820pt wide, the caveat sits at the leading
  edge, and Contribute sits at the trailing edge — the caveat is roughly
  600pt from the cursor at the moment it matters. Third, it is not "in
  gold": `ScrubbingCaveatAtCommit` sets the text in `.secondary` gray;
  only the small glyph is gold. It is the quietest text in the footer.

  How I would have solved it: make the caveat part of the action instead
  of part of the room. Either (a) right-align it directly above the
  Contribute button, appearing only once the summary has loaded — same
  copy, but its appearance is an event, not furniture; or (b) fold it into
  a two-stage control: first click turns the button into
  "Send it — scrubbing may have missed things", second click sends. Option
  (b) also fixes most of question 3 below and replaces the caveat's
  repetition with a mechanism that cannot be tuned out.

## 3. Is the moment of consent well built?

The skeleton is genuinely good and worth protecting: no approve action on
the queue row (preview-then-approve only, stricter than the shared spec's
sketch); Search as the first tab, focused, answering the one question a
contributor can answer in five seconds; "Not this one" untinted so it
cannot read as a second approval; permissions restated at the moment of
consent; a real undo backed by `cancel`; a quit dialog that tells the
truth about the watcher. The consent-scopes screen's
"Continue with 1 permission" button label — the count in the button — is
the strongest consent-affordance in the app.

But the sheet's mechanics then undermine the skeleton:

- **Return is Contribute.** The only irreversible action in the product is
  the default keyboard action of the sheet (`.keyboardShortcut(.defaultAction)`).
  While the search field is focused, Return is swallowed by `onSubmit`;
  the moment focus moves — click a tab, click anywhere — Return sends a
  transcript. macOS convention makes the default button the confirming
  action of an *alert*; this sheet is a review surface, and giving the
  irreversible act the cheapest keystroke in it optimizes for exactly the
  fast-approval behavior the brief says a good screen must not produce.
- **Contribute is enabled before the payload has been seen.** The gate is
  `summary != nil` — metadata loaded — not "the contributor looked at
  anything". A user can open the sheet and click Contribute without ever
  visiting "Exactly what would be sent", scrolling a transcript, or running
  a search. The design's whole thesis is "see exactly what leaves"; the
  control does not require the seeing.
- **`advance()` builds an approval rhythm.** After Contribute, the sheet
  loads the next session with the Contribute button in the same pixels and
  "2 more after this" reading as a progress meter. Three sessions are
  "three deliberate clicks" only in count; in practice they are the same
  click three times, accelerating. And while the sheet is showing session
  two, the undo bar for session one is rendered in `QueueView` — behind
  the sheet, invisible until the batch is done and probably expired. If
  chained review stays, the undo affordance must live in the sheet.
- Minor: "2 more after this" sits beside "Not this one", where it can be
  misread as describing the dismissal.

## 4. Alignment vs. nativeness

Where it reads as one family, correctly: the warm ground, hairline-only
card separation, radii, pills, the uppercase heavy field labels over data,
the KPI band, and the brand mark as transcribed geometry. The four calls
the designer defended are all right, and I checked them rather than took
them: dropping the shadow (`0 18px 48px` verified in styles.css — a web
idiom, hairlines are correct here), not bundling Inter, skipping violet
(verified: the site uses it only as a meter fill; it has no semantic), and
stopping the brand at the chrome. The menu-bar mark instead of `tray.full`
is the single best alignment decision — findability among twenty SF-Symbol
neighbours is a real argument, and the monochrome reduction (solid wedge,
washed field, seam) is careful work.

Where it is a website wearing a Mac costume:

- **The hand-rolled tab bar in the preview sheet.** Buttons styled as
  segments, with a green selection border. The stated reasons are badges
  and `ImageRenderer` screenshotability — but the report itself calls
  replacing controls to satisfy the screenshot tool "the tail wagging the
  dog", and this is that, half-admitted. The cost is native segmented
  behavior: arrow-key traversal as a radio group, Full Keyboard Access
  focus appearance, correct vibrancy. If the badge justifies it, add the
  keyboard behavior it dropped; otherwise revert to `Picker(.segmented)`
  and accept the yellow placeholder in captures as documented.
- **Green-filled prominent buttons are an app invention, not alignment.**
  The site's buttons are ink (`--ink` fill, white text); nowhere does the
  site set white text on green. The app's green CTAs are where the contrast
  failures below live. The site's own idiom — ink buttons, green reserved
  for accents and standing — was the more native *and* more legible choice.
- **Overriding the user's accent colour**: defensible and I would keep it
  (green carries meaning here; the two surfaces are one product), but note
  it is the departure users can see as a taken preference. The report's
  candour about it is appropriate.
- Minor: `TCTag` sets pill text in monospace (`ledger`); the site's pills
  are heavy sans. Fine for "nothing matched" (it is data), slightly odd for
  "nothing sent yet" (it is a sentence).

## 5. Dark Mode

Structurally it holds up — this is not the light design with the lights
off. The warm cast survives (ground resolves to #23251D, an olive black,
not blue-black); ground/surface/inset keep their order and spacing; and
the text twins are strong: measured 7.35:1 (goldText on inset), 8.53:1
(greenText on surface), 7.78:1 (blueText), 7.57:1 (coralText). The derived
palette is better than most copied ones.

Two real problems:

- **The prominent-button label.** In Dark Mode the "Look inside" /
  "Contribute" / "Get started" buttons render a mint fill (#3FBE9A) with a
  white label — confirmed by cropping the after-dark capture. White on
  #3FBE9A is **2.32:1**. Dark text on the same fill would be 9.04:1. This
  is the identical failure mode the pass diagnosed in gold-on-ground
  (2.9:1) and refused to ship for a warning sentence — shipped instead on
  the primary action of every screen. SwiftUI chose the label colour, not
  the designer, but the tint was raised for dark without checking what the
  system would set on top of it. The fix is a darker dark-mode tint for
  fills (keeping #3FBE9A for text/glyphs) or forcing a dark label.
- **The brand mark changes identity in dark.** Its interior square follows
  `TC.surface`, so the site's white triangle becomes near-black and both
  accents are the lifted variants. The geometry is preserved but the
  colour identity is not — acceptable as a necessity, but the report's
  "reproduced exactly" holds only in Light. The welcome screen's dark mark
  reads noticeably heavier.

Also noted: Light Mode's white-on-#178F70 buttons measure 4.04:1 — passes
3:1 large-text but not 4.5:1, and button labels at ~13pt semibold are
normal-size text. The pass held warning copy to 4.5:1; its own CTAs sit
at 4.0 in light and 2.3 in dark.

## 6. Designer's calls, checked

Verified true:

- **No animation on the site**: zero `transition` / `@keyframes` /
  `animation` occurrences in `styles.css` and `app.js`. The claim stands.
- **Gold at ~2.9:1**: #B9821F on #EEF2F0 computes to 2.96:1. The text
  twins all clear 4.5:1 on every surface they are used on (4.85–6.04:1).
  Good, verified work.
- The `ImageRenderer` NSView-placeholder diagnosis is consistent with
  every placeholder position in the captures (Menu in the popover,
  TextField in connect and search, segmented Picker in the old sheet).
- Shadow, radii, pill, eyebrow, brand-mark gradient-stop values all match
  the CSS as reported.

Claimed and not true:

- **"No view writes a raw padding number, a font size, or a colour literal
  any more."** False. Roughly eighty raw spacing/font literals remain
  across twelve view files (`spacing: 12`, `padding(8)`,
  `cornerRadius: 6`, `.font(.callout)` and kin) — and they are densest in
  `PreviewSheet.swift` (~30), the consent surface itself. The design
  system exists; the most important screen only half-adopted it.
- **The commit caveat is "in gold with a glyph."** The glyph is gold; the
  sentence is `.secondary` gray. As shipped it is the least emphatic text
  in the footer.
- Pedantic: the site "declares `color-scheme: light`" via a `<meta>` tag
  in `index.html`, not in the stylesheet. Substantively true.

On motion: the one-shot mark assembly is fine — 0.85s, once, first screen
only, Reduce Motion honoured — and the instinct that a file-watching app
must not fidget is correct and should be written down somewhere more
durable than a report. I would not add a frame of motion beyond it, and I
would accept an argument for deleting even this one.

## What to protect from future edits

- The manifest strip and the monospace-figures rule.
- Preview-then-approve with no approve on the row, and no select-all.
- The varying per-card caveat line and its attention tone for zero matches.
- The untinted dismissive buttons.
- The menu-bar mark, the monochrome reduction, and the inert (non-actionable)
  session lines in the menu.
- The "Armed: N projects — contributed without asking" persistent
  disclosure in the menu.
- "Continue with 1 permission."

## Priority of fixes

1. Dark-mode prominent-button label contrast (2.3:1 on the consent action).
2. Remove `.defaultAction` from Contribute; gate it on the payload having
   been shown, or make it two-stage; move undo into the sheet if chained
   review stays.
3. Deduplicate Return on queue rows.
4. Finish design-system adoption inside `PreviewSheet.swift`; correct the
   report's absolute claims (raw literals, "in gold").
5. Clarify the redaction-summary arithmetic and translate detector keys.
