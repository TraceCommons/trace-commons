import AppKit
import SwiftUI

/// The one place the shell's visual decisions live.
///
/// ## The direction: a customs declaration, not a feed
///
/// This app stands between a developer's private transcripts and a public
/// research pool. The only question its interface has to answer is "what
/// exactly is about to leave this machine, and can I stop it." So the
/// surfaces are built like a declaration form rather than a stream of
/// content: every session is one card, every card carries the SAME fields in
/// the SAME order, and each card ends in a fixed manifest strip set in
/// monospaced type.
///
/// That repetition is the point. When every card's outbound facts land in
/// the same place on the page, a person stops reading and starts scanning,
/// and the row that is different -- a large payload, a session where
/// scrubbing matched nothing -- is a break in a rhythm rather than a
/// sentence they have to notice. It is the one deliberately bold move in an
/// otherwise quiet interface, and everything else is kept plain so it can
/// carry.
///
/// ## Where these values come from
///
/// The palette began as a transcription of the community site and has since
/// been reconciled against the approved desktop mockups, recorded in
/// `design-import/DESIGN-SPEC.md`. That document is now the target: every
/// token below carries the spec's own name for it in its doc comment, so a
/// value can be traced back to the frame it was measured in. Where the two
/// sources disagreed, the mockups won, and the previous value is noted in a
/// trailing comment rather than deleted from the record.
///
/// The spec also carries a SECOND palette -- the community brand, used on the
/// public-facing surfaces (going public, the manifesto, the credit coin). That
/// one is deliberately foreign: black 2px frames, Helvetica, mint. It is not
/// defined here, because the seam between the two is the point. This file is
/// the private tool.
///
/// ## Family resemblance to the community site
///
/// `community/public/styles.css` is the other face of this product, and the
/// two are meant to read as the same organisation. What is carried across:
/// the palette and, more importantly, the ROLES each colour plays (green is
/// primary and means "good standing", gold means "weigh this", coral means
/// "refused", blue means "held / ranked"); the warm off-white ground rather
/// than a neutral grey; modest 6-8pt radii with pill-shaped badges; hairline
/// rules instead of shadows to separate things; and heavy uppercase micro
/// labels over data.
///
/// What is deliberately NOT carried across, because a Mac app that looks
/// like a web page is a worse Mac app:
///
/// - **Inter is not bundled.** A font file in a notarized bundle is a real
///   cost for a brand cue. The site's 680/760/800 headings are reproduced
///   with SF's `.semibold`/`.bold`/`.heavy`, which is what those weights are
///   for, and SF is the face a Mac user's eye already calibrates against.
/// - **No drop shadows.** The site's `0 18px 48px` card shadow is a web
///   idiom; inside a macOS window it reads as a floating dialog. Hairlines
///   do the same separating work natively.
/// - **Window chrome stays system-drawn.** Toolbar, sidebar, sheets, focus
///   rings, and the menu-bar popover use system materials and vibrancy. The
///   brand palette is applied to the CONTENT area -- the ground a person
///   reads on, the card faces, the accents -- and stops at the chrome.
///
/// ## Dark Mode: derived, not inverted
///
/// The site has no `prefers-color-scheme` block and declares
/// `color-scheme: light`; there is no dark palette to copy. Dark Mode is a
/// macOS requirement, so one is derived here by preserving the site's
/// *relations* rather than flipping its hex values:
///
/// - The site's ground (`#f6f7f4`) is not a neutral grey -- it is warm, with
///   a faint green cast. The dark ground keeps that cast at the other end of
///   the scale (`#23251D`, a warm near-black) rather than the blue-black that
///   a naive inversion produces.
/// - Ground / surface / inset keep the same ORDER and roughly the same
///   perceptual spacing as `--bg` / `--surface` / `--surface-2`, so the same
///   layering reads in both appearances.
/// - Every accent keeps its hue and its role and is lifted in lightness
///   until it clears text contrast against the dark ground. `--green`
///   (`#178f70`) is a good colour on white and an illegible one on near
///   black; the dark counterpart is the same green, raised, not a different
///   colour.
enum TC {
    // MARK: - Spacing

    /// A 4pt rhythm. Views should not write raw padding numbers; if a value
    /// is missing here, it is probably the wrong value.
    enum Space {
        static let hairline: CGFloat = 1
        /// Spec `space.0.5`. A sidebar row's internal gap; label to value.
        static let micro: CGFloat = 2
        /// A 3pt step. The vertical padding on a status chip (§6.2) and on a
        /// small secondary button (§6.1); the spec's own scale skips it, but
        /// both components state it.
        static let tiny: CGFloat = 3
        /// Spec `space.1`. Segmented-control padding, chip icon gap.
        static let xxs: CGFloat = 4
        /// A 5pt step, likewise absent from the spec's scale and likewise
        /// stated by the components that need it: the vertical padding on
        /// buttons (§6.1), tab items (§6.6) and search fields (§6.10).
        static let control: CGFloat = 5
        /// Spec `space.1.5`. Intra-group gaps, e.g. a checkbox stack.
        static let xs: CGFloat = 6
        /// Spec `space.2`. Button gaps, inline metadata, card inner stacks.
        static let s: CGFloat = 8
        /// Spec `space.2.5`. Section stacks and header gaps.
        static let sm: CGFloat = 10
        /// Spec `space.3`. Card grid gap, banner icon gap, compact padding.
        static let m: CGFloat = 12
        /// Spec `space.3.5`. Content gap inside a queue row or brand panel.
        static let md: CGFloat = 14
        /// Spec `space.4`. Card padding-x; content-block gap in History.
        static let l: CGFloat = 16
        /// Spec `space.4.5`. Sheet padding-x; gap between settings sections.
        static let lg: CGFloat = 18
        /// Spec `space.5`. Screen padding-x on brand panels and dialogs.
        static let xl: CGFloat = 20
        /// Spec `space.5.5`. macOS content padding-x.
        static let xlg: CGFloat = 22
        /// Spec `space.6`. Gaps between metrics in a queue card's manifest.
        static let xxlSmall: CGFloat = 24
        /// Spec `space.7`. Sheet header field gap.
        static let xxl: CGFloat = 28
        /// Spec `space.8`. Credit-card figure gap.
        static let figure: CGFloat = 32
        static let xxxl: CGFloat = 36

        /// The standard content region's padding on macOS: `18 22 22`.
        enum Content {
            static let top: CGFloat = 18
            static let horizontal: CGFloat = 22
            static let bottom: CGFloat = 22
        }

        /// A content header's padding: `9 20`.
        enum Header {
            static let vertical: CGFloat = 9
            static let horizontal: CGFloat = 20
        }
    }

    /// A reading column. Trust copy is read, not skimmed, and a sentence
    /// that runs the full width of a 1400pt window is a sentence nobody
    /// finishes. It also keeps a card's primary and secondary action within
    /// one eye movement of each other, which the previous full-bleed row
    /// did not.
    enum Measure {
        /// Lists and dashboards. The community site's `.app-shell` runs to
        /// 1180px and fills it by banding content across the full measure
        /// rather than by setting long lines, and this follows it: figures,
        /// tags and actions spread out to the edges so a wide window is used
        /// rather than left as margin.
        static let column: CGFloat = 980
        /// Prose that is actually read start to finish -- onboarding, the
        /// consent screen, settings. Kept narrow on purpose; the site sets
        /// its running text in a column too.
        static let prose: CGFloat = 660
    }

    /// The site's radii: 6 and 8, with 999 pills for badges. The macOS column
    /// of the spec's radius table; Windows tightens controls to 4 and GNOME
    /// widens the window to 12, neither of which this shell draws.
    enum Radius {
        /// Spec `radius.card`. Cards, banners, segmented tracks, the
        /// transcript panel.
        static let card: CGFloat = 8
        /// Spec `radius.control.mac`. Buttons, inputs, sidebar rows, tabs.
        static let inset: CGFloat = 6
        /// Spec `radius.control.mac`, under the spec's own name.
        static let control: CGFloat = 6
        /// Spec `radius.pill`. Status chips and count badges. `Capsule()` is
        /// the idiomatic way to draw these; the number is here for the cases
        /// that need a radius rather than a shape.
        static let pill: CGFloat = 999
        /// Spec `radius.chip.inline`. The search-term highlight.
        static let chipInline: CGFloat = 2
        /// Spec `radius.chip.redaction`. The redaction marker chip.
        static let redactionChip: CGFloat = 3
        /// Spec `radius.checkbox`. The read-gate checkbox, a 13x13 box.
        static let checkbox: CGFloat = 3
        /// Spec `radius.window.mac`. The window and its sheets.
        static let window: CGFloat = 10
        /// The last row of §4.2's table: zero, on every community brand
        /// surface. Nothing inside a black-framed panel is ever rounded, and
        /// the token exists so that rule is stated rather than implied by a
        /// bare `Rectangle`.
        static let brand: CGFloat = 0
    }

    /// Fixed control sizes the spacing scale has no step for, because they
    /// size an object rather than a gap.
    enum Control {
        /// Spec §6.9. The read-gate checkbox is a 13pt square.
        static let checkbox: CGFloat = 13
    }

    /// The status item. The menu bar is 22pt tall and the system's own
    /// glyphs sit at 15-16pt inside it, so there is no room to set a digit
    /// row under the mark; the count is a capsule over the mark's top
    /// right corner instead, and these are its measures.
    enum MenuBar {
        /// Spec §1.3: the mark at 15pt, template variant.
        static let mark: CGFloat = 15
        /// The badge's height, and the smallest a digit can be and still be
        /// read against the menu bar at 1x.
        static let badgeHeight: CGFloat = 10
        /// Side padding inside the capsule, beyond the digits' own width.
        static let badgeInset: CGFloat = 2.5
        /// The clear ring between the badge and the bracket it overlaps,
        /// so the capsule reads as a thing on the mark rather than a growth
        /// of it.
        static let badgeHalo: CGFloat = 1
        /// How far the badge rises above the mark's top edge. The label is
        /// padded by this on both edges so the mark stays centred, and
        /// 15 + 3 + 3 is the most that fits the 22pt bar.
        static let badgeOverhang: CGFloat = 3
        /// How far back over the mark the badge's leading edge sits. The
        /// user's bracket ends at 28/64 of the mark (6.5pt at 15) and the
        /// agent's begins at 36/64 (8.4pt) but only below the badge, so a
        /// leading edge at 15 - 6 = 9pt touches neither.
        static let badgeOverlap: CGFloat = 6
    }

    // MARK: - Type

    /// A fixed scale, all of it relative to system text styles.
    ///
    /// Two rules carry the identity. Figures that describe what leaves the
    /// machine are always monospaced and prose never is, so a person can
    /// find a payload size on any card without reading a word. And field
    /// labels are heavy, uppercase and tracked, which is the site's
    /// `.eyebrow` / `th` / `.kpi .label` treatment (12px, weight 800,
    /// uppercase) rendered in SF instead of Inter.
    /// Where a macOS text style's default size already equals the size the
    /// mockups state, the text style is used, so the scale still answers to
    /// Dynamic Type. macOS `title2` is 17, `title3` 15, `headline`/`body` 13,
    /// `callout` 12, `subheadline` 11, `footnote`/`caption`/`caption2` 10.
    /// Where the mockups ask for a size no text style carries -- 20, 18, 16,
    /// 12.5, 10.5 -- an exact size is given instead.
    enum Font_ {
        /// Spec `title.screen`, 15/700. Content-header titles: "Waiting",
        /// "History", "Settings".
        static let screenTitle = Font.title3.weight(.bold)
        /// Onboarding headlines. The community panels set display type at
        /// landing scale; this is the nearest honest equivalent that still
        /// sits inside a window.
        static let display = Font.title.weight(.heavy)
        /// Spec `title.section`, 17/700. "3 sessions waiting for your
        /// decision". Was `title3.bold` (15pt).
        static let sectionTitle = Font.title2.weight(.bold)
        /// Spec `title.card`, 13/600. The name of the thing a card is about.
        static let cardTitle = Font.headline
        /// Spec `metric.value`, 20/700. Stat-card numbers.
        static let metricValue = Font.system(size: 20, weight: .bold)
        /// Spec `metric.value.mono`, 18/700 mono. Credit figures.
        static let metricValueMono = Font.system(size: 18, weight: .bold, design: .monospaced)
        /// Spec `heading.alert`, 16/700. "2 matches".
        static let headingAlert = Font.system(size: 16, weight: .bold)
        /// Spec `body`, 13/400. The opening prompt -- the text that actually
        /// identifies a session to the person who wrote it. Was `callout`
        /// (12pt).
        static let body = Font.body
        /// Spec `body.dense`, 12.5/600. The undo bar's headline.
        static let bodyDense = Font.system(size: 12.5, weight: .semibold)
        /// Spec `body.dense` at 400. Disclosure rows.
        static let disclosure = Font.system(size: 12.5)
        /// Spec `label.control`, 12/500. Secondary buttons.
        static let labelControl = Font.system(size: 12, weight: .medium)
        /// Spec `label.control.primary`, 12/600. Filled buttons.
        static let labelControlPrimary = Font.system(size: 12, weight: .semibold)
        /// Spec `caption`, 11/400. Attribution, timestamps, agent names,
        /// supporting sentences. Was `callout` (12pt).
        static let meta = Font.subheadline
        /// Spec `caption`, 11/400, under the spec's own name.
        static let caption = Font.subheadline
        /// Spec `caption.small`, 10.5/400. The read-gate footnote.
        static let captionSmall = Font.system(size: 10.5)
        /// Spec `eyebrow`, 10/800 uppercase, tracked. Field labels on the
        /// manifest strip. See `Tracking.eyebrow`.
        static let fieldLabel = Font.caption2.weight(.heavy)
        /// Spec `mono.figure`, 12/500 mono. Figures on the manifest strip, and
        /// anything else countable. Was `footnote`-sized mono (10pt).
        static let ledger = Font.system(.callout, design: .monospaced)
            .weight(.medium)
        /// The status item's count, drawn against an opaque 10pt capsule. Rounded
        /// and bold because at this size a hairline digit disappears when
        /// it is drawn in reverse.
        static let menuBarBadge = Font.system(size: 8, weight: .bold, design: .rounded)
        /// Spec `mono.chip`, 11/500 mono. Status-pill text.
        static let monoChip = Font.system(.subheadline, design: .monospaced)
            .weight(.medium)
        /// Spec `mono.badge`, 10/500 mono. Tab counts.
        static let monoBadge = Font.system(.caption2, design: .monospaced)
            .weight(.medium)
        /// Spec `mono.code`, 11/400 mono. Search excerpts.
        static let monoCode = Font.system(.subheadline, design: .monospaced)
        /// Spec `mono.transcript`, 11/400 mono. The transcript renderer's
        /// body. Set it with `LineHeight.transcript`.
        static let monoTranscript = Font.system(.subheadline, design: .monospaced)
        /// Footnotes and disclosure text.
        static let footnote = Font.caption

        /// Letter-spacing, in points. Only the eyebrow is tracked; macOS
        /// widens it to 0.5, GNOME to 0.8.
        enum Tracking {
            static let eyebrow: CGFloat = 0.5
        }

        /// Line-height multiples from the mockups. SwiftUI's `lineSpacing` is
        /// the gap ADDED between lines, not the line box, so a multiple has to
        /// be converted against the size it applies to -- `spacing(for:_:)`
        /// does that.
        enum LineHeight {
            static let body: CGFloat = 1.45
            static let caption: CGFloat = 1.5
            static let transcript: CGFloat = 1.7

            /// Extra spacing to pass to `.lineSpacing()` for `size` type set
            /// at `multiple`. Approximates the default line box as 1.2x.
            static func spacing(for size: CGFloat, _ multiple: CGFloat) -> CGFloat {
                max(0, size * (multiple - 1.2))
            }
        }
    }

    // MARK: - Palette

    /// One brand colour, defined once for each appearance.
    ///
    /// Built on `NSColor(name:dynamicProvider:)` rather than an asset
    /// catalogue so the whole palette is readable in one file, and so it
    /// resolves live when the system appearance changes or a capture run
    /// pins one.
    private static func dynamic(_ light: NSColor, _ dark: NSColor) -> Color {
        Color(nsColor: NSColor(name: nil) { appearance in
            appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua ? dark : light
        })
    }

    /// A development hook, paired with `TRACE_COMMONS_APPEARANCE` in
    /// `TraceCommonsAppMain`. Setting `NSApp.appearance` pins the appearance
    /// of real windows but not of an offscreen `ImageRenderer`, which
    /// resolves colours from the SwiftUI environment rather than from the
    /// application object -- so a capture run asked for Dark and got Light,
    /// silently. `tcScreen()` pins the environment as well. Unset (the
    /// normal case) this is `nil` and every screen follows the system.
    static let forcedColorScheme: ColorScheme? = {
        switch ProcessInfo.processInfo.environment["TRACE_COMMONS_APPEARANCE"] {
        case "dark": return .dark
        case "light": return .light
        default: return nil
        }
    }()

    private static func hex(_ value: UInt32, alpha: Double = 1) -> NSColor {
        NSColor(
            srgbRed: Double((value >> 16) & 0xFF) / 255,
            green: Double((value >> 8) & 0xFF) / 255,
            blue: Double(value & 0xFF) / 255,
            alpha: alpha
        )
    }

    // MARK: Grounds and surfaces

    /// Spec `bg.window`, site `--bg`. The ground a person reads on. Warm,
    /// never neutral grey.
    ///
    /// The dark value used to be written as `#15170F` blended 6% toward white;
    /// that expression resolves to exactly `#23251D`, which is the value the
    /// mockups state, so it is now written literally.
    static let ground = dynamic(hex(0xF6F7F4), hex(0x23251D))
    /// Spec `bg.window` as the translucent content-header ground:
    /// `rgba(246,247,244,.9)` light, `rgba(35,37,29,.92)` dark.
    static let groundTranslucent = dynamic(hex(0xF6F7F4, alpha: 0.9), hex(0x23251D, alpha: 0.92))
    /// Spec `bg.sidebar.macos`. The `NavigationSplitView` sidebar ground, for
    /// the places the app draws its own sidebar fill instead of the system's.
    static let sidebarGround = dynamic(hex(0xECEEE8), hex(0x262922))
    /// Spec `bg.chrome.windows`. Recorded for palette completeness only -- the
    /// macOS shell never draws Windows title-bar chrome.
    static let chromeGroundWindows = dynamic(hex(0xF3F3F0), hex(0x2B2D28))
    /// Spec `surface.card`, site `--surface`. Card faces, popovers, inputs.
    static let surface = dynamic(hex(0xFFFFFF), hex(0x21241E))
    /// Spec `surface.inset`, site `--surface-2`. Recessed strips inside a
    /// card; segmented-control tracks.
    static let surfaceInset = dynamic(hex(0xEEF2F0), hex(0x2A2E27))
    /// Spec `surface.scrim`. A wash over whatever is behind it -- code blocks
    /// inside search results, segmented tracks.
    static let surfaceScrim = dynamic(hex(0x000000, alpha: 0.06), hex(0xFFFFFF, alpha: 0.08))
    /// Spec `surface.selected.macos`. The selected sidebar row.
    static let surfaceSelected = dynamic(hex(0x000000, alpha: 0.07), hex(0xFFFFFF, alpha: 0.10))

    // MARK: Hairlines

    /// Spec `hairline`, site `--line`. Card borders, input borders, section
    /// rules, sheet dividers.
    static let line = dynamic(hex(0xD9DFDC), hex(0x3B4038))
    /// Spec `hairline.divider`. Structural edges rather than object edges: the
    /// sidebar's right edge, the content header's bottom edge. A shade apart
    /// from `line` on purpose, and only distinguishable side by side.
    static let divider = dynamic(hex(0xDDDFD8), hex(0x373A33))

    // MARK: Ink

    /// Spec `ink.primary`. Body and title text.
    static let inkPrimary = dynamic(hex(0x20241F), hex(0xE8EAE3))
    /// Spec `ink.secondary`. Supporting prose, sub-labels, muted icons.
    static let inkSecondary = dynamic(hex(0x5C635B), hex(0xA6AC9F))
    /// Spec `ink.tertiary`. Timestamps, eyebrow labels, footnotes.
    ///
    /// The mockups state `#8A9086` light and `#82887C` dark, and both are
    /// refused here. Every role the spec assigns this token is small text, and
    /// the stated pair does not clear the 4.5:1 floor in any of the four
    /// combinations it has to survive: 3.04:1 on `ground` and 3.27:1 on
    /// `surface` in light, 4.26:1 and 4.31:1 in dark. What ships instead is the
    /// nearest accessible twin on the same hue and saturation -- 4.58:1 and
    /// 4.93:1 light, 4.55:1 and 4.61:1 dark -- which keeps the three-step ink
    /// ramp the spec is after without setting a timestamp in a grey a person
    /// cannot read.
    ///
    /// This is the same move the accents already make below, and for the same
    /// reason: the palette is tuned for fills and borders, where 3:1 is the
    /// bar, and small type needs a darkened twin. The Linux client refuses the
    /// identical pair with the identical substitutes, so the two clients agree.
    static let inkTertiary = dynamic(hex(0x6D7269), hex(0x878D81))

    // MARK: Accents

    /// Spec `green.brand`, site `--green`. Primary. Good standing, the app's
    /// accent, and the mark's top-left bracket.
    static let green = dynamic(hex(0x178F70), hex(0x3FBE9A))
    /// Spec `blue.brand`, site `--blue`. Secondary. Held, ranked, in progress,
    /// and the mark's bottom-right bracket.
    ///
    /// The mockups carry two blues one digit apart -- `#315FBA` on the mark and
    /// chip borders, `#315FBB` on icon strokes and chip text. They are the same
    /// intended colour; the whole app standardises on `#315FBA`, the mark blue.
    static let blue = dynamic(hex(0x315FBA), hex(0x7FA0EC))
    /// Spec `blue.icon`. The "held for privacy review" clock glyph and its
    /// chip text. Light is `#315FBA`, not the mockups' `#315FBB` -- see `blue`.
    static let blueIcon = dynamic(hex(0x315FBA), hex(0x9DB6F1))
    /// Spec `gold.brand`, site `--gold`. Weigh this before deciding.
    // MARK: The mark

    /// The mark's own palette, and deliberately not the semantic accents.
    ///
    /// The mark converged on the site's single accent over ink. `green` and
    /// `blue` did not move with it: they still mean "good standing" and "held
    /// or ranked" on every chip, row and badge in the app, and repainting the
    /// logo is not a reason to repaint status. The two palettes are related
    /// only by history, so they are separate tokens now.
    ///
    /// Mirrors `Scheme::bracket_open`, `bracket_close` and `surface` in the
    /// trace-commons-mark crate, which is the source of truth for the
    /// generated icon files. Change them together.
    static let markAccent = dynamic(hex(0x00D4AA), hex(0x00D4AA))
    /// The closing bracket, the frame, and the template variant's single ink.
    static let markInk = dynamic(hex(0x000000), hex(0xFFFFFF))
    /// Inside the frame.
    static let markField = dynamic(hex(0xFFFFFF), hex(0x000000))

    static let gold = dynamic(hex(0xB9821F), hex(0xDCAA43))
    /// Spec `gold.highlight`. The wash behind a matched search term inside
    /// excerpt text. A fill, never a text colour.
    static let goldHighlight = dynamic(hex(0xB9821F, alpha: 0.28), hex(0xDCAA43, alpha: 0.32))
    /// Spec `coral.brand`, site `--coral`. Refused, withdrawn, cannot proceed.
    /// The mockups never draw coral in dark; the dark value is derived here on
    /// the same rule as the other accents.
    static let coral = dynamic(hex(0xD65D4F), hex(0xF2887A))

    // MARK: Redaction chip

    /// Spec `redaction.chip.bg` / `redaction.chip.fg`. The marker that stands
    /// in the transcript where something was removed. Measured in the mockups
    /// at 12.3:1 light and 9:1 dark, which is why this pair is stated rather
    /// than composed from the gold ramp.
    static let redactionChipBackground = dynamic(hex(0xF3E3C0), hex(0x4A3C18))
    /// See `redactionChipBackground`.
    static let redactionChipForeground = dynamic(hex(0x202426), hex(0xF0EBDD))

    // Fill-safe counterparts for a FILLED primary action.
    //
    // `green` is tuned to be read *on* the ground, not to be a fill with a
    // label on top of it, and the difference is not cosmetic. A white label
    // on the dark-mode mint measures 2.32:1 -- below even the 3:1 large-text
    // floor -- and on the light green 4.04:1, below the 4.5:1 normal-text
    // floor. That is the same failure this file already refuses for gold
    // warning text, and it was sitting on Contribute: the one irreversible
    // control in the product, the button that moves a private transcript to
    // a public commons. A consent action nobody can read is not a consent
    // action.
    //
    // So the filled action carries its own pair, measured rather than
    // eyeballed:
    //   light  #137C61 fill + white label -> 5.14:1
    //   dark   #3FBE9A fill + #0B1F19 ink -> 7.39:1
    // Light darkens the fill (the hue survives; the site's green is still
    // recognisably the accent) and dark flips the label instead of dulling
    // the mint, because the mint is what makes the dark scheme feel like the
    // same product.
    /// Spec `green.fill`. Fill for a filled primary action. Not the same value
    /// as `green`.
    static let primaryFill = dynamic(hex(0x137C61), hex(0x3FBE9A))
    /// Spec `on.accent`. Label colour that sits on `primaryFill` at >= 4.5:1 in
    /// both schemes. Light was `#FFFFFF`; the mockups state `#FEFEFE`.
    static let primaryLabel = dynamic(hex(0xFEFEFE), hex(0x0B1F19))
    /// Spec `on.accent`, under the spec's own name. Text or glyph on any
    /// filled accent, not only the primary button.
    static let onAccent = primaryLabel

    // Text-safe counterparts.
    //
    // The site's accents are tuned for fills, meter bars and borders, where
    // 3:1 is the bar. As small text on a light surface several of them do
    // not clear 4.5:1 -- `--gold` on `--surface-2` lands near 2.9:1, which
    // is not a contrast a warning sentence may be set in. So each accent has
    // a darkened light-mode twin used ONLY for type, while fills, glyph
    // strokes and borders keep the site's exact value. The hue is preserved;
    // only the lightness moves, so the family resemblance survives and the
    // text is legible.
    /// Spec `green.text`.
    static let greenText = dynamic(hex(0x0F7256), hex(0x5CD3AF))
    /// Spec `gold.text`.
    static let goldText = dynamic(hex(0x8A5F12), hex(0xE2B75C))
    /// Spec `coral.text`. Dark is not drawn in the mockups; derived here.
    static let coralText = dynamic(hex(0xB8483B), hex(0xF79C8F))
    /// Spec `blue.icon` in its text role. Same pair as `blueIcon`.
    static let blueText = dynamic(hex(0x315FBA), hex(0x9DB6F1))

    // MARK: - Platform chrome

    /// The macOS traffic lights, for the frames that draw their own window
    /// chrome rather than taking the system's. 12 x 12pt circles, 8pt apart.
    enum TrafficLight {
        static let close = dynamic(hex(0xFF5F57), hex(0xFF5F57))
        static let minimise = dynamic(hex(0xFEBC2E), hex(0xFEBC2E))
        static let zoom = dynamic(hex(0x28C840), hex(0x28C840))
        static let diameter: CGFloat = 12
        static let gap: CGFloat = 8
    }

    // MARK: - Borders

    /// Hairline weights and the alphas the mockups draw borders at.
    ///
    /// A status chip's border is not its status hue at full strength -- it is a
    /// 45% wash of it, so the chip reads as a token rather than as a button.
    /// Attention borders on cards and banners are drawn stronger, because a
    /// card that wants weighing has to be findable down a scrolling column.
    ///
    /// Shadows are deliberately absent. Inside the window this app separates
    /// things with hairlines; the mockups' `0 18px 44px` shadows describe the
    /// elevation of the window frames in the design document, not in-app
    /// elevation.
    enum Border {
        /// Every native hairline: 1pt.
        static let hairline: CGFloat = 1
        /// 1.5pt. Heavier than a hairline, and used where a border is the only
        /// thing drawing an object rather than separating two of them: the
        /// unchecked read-gate box of §6.9, whose outline IS the control.
        static let medium: CGFloat = 1.5
        /// Status-chip borders.
        static let chipAlpha: Double = 0.45
        /// Cards and banners asking to be weighed.
        static let attentionAlpha: Double = 0.55
        static let attentionAlphaDark: Double = 0.6
        /// The selected tab in a segmented control.
        static let activeTabAlpha: Double = 0.55
        static let activeTabAlphaDark: Double = 0.6
    }

    // MARK: - Colour roles

    /// What a piece of information means, expressed as a colour AND a symbol
    /// AND (at the call site) words. Never the colour on its own.
    ///
    /// The mapping is the site's: green for good standing, gold for "weigh
    /// this", coral for refused, blue for held.
    enum Tone {
        /// Ordinary, nothing to weigh.
        case neutral
        /// Something was found, or something cannot be checked. Caution, not
        /// alarm: this app never shouts, because a product that shouts on
        /// every row teaches people to stop looking.
        case attention
        /// A question a person asked and got a clean answer to.
        case clear
        /// Held, waiting on somebody else. Not a failure.
        case held
        /// Refused, or unavailable.
        case refused

        /// For fills, borders and glyphs. The site's exact values.
        var color: Color {
            switch self {
            case .neutral: return .secondary
            case .attention: return TC.gold
            case .clear: return TC.green
            case .held: return TC.blue
            case .refused: return TC.coral
            }
        }

        /// For type. See the note beside `TC.goldText`.
        var textColor: Color {
            switch self {
            case .neutral: return .secondary
            case .attention: return TC.goldText
            case .clear: return TC.greenText
            case .held: return TC.blueText
            case .refused: return TC.coralText
            }
        }

        /// Every tone carries a glyph so the state survives greyscale,
        /// colour-blindness, and a screenshot printed in black and white.
        var symbol: String {
            switch self {
            case .neutral: return "circle"
            case .attention: return "exclamationmark.triangle"
            case .clear: return "checkmark.circle"
            case .held: return "clock"
            case .refused: return "xmark.circle"
            }
        }
    }
}

// The mark used to be transcribed here as the site's `.brand-mark` gradient
// square. That mark is superseded: the clients now carry "The Turn", drawn in
// `BrandMark.swift` from the tokens above.

// MARK: - Card

/// The one card treatment in the app: a face, a hairline, and nothing else.
///
/// The hairline is what makes a card read as a document rather than a grey
/// blob. Flat fills of the same value stacked down a window give the eye no
/// edge to catch, which is what made the previous queue read as a preview
/// canvas. The site separates its panels the same way, and for the same
/// reason its cards' shadows are dropped here: a hairline is native, a
/// 48px blur is not.
struct TCCard: ViewModifier {
    var emphasised: Bool = false

    func body(content: Content) -> some View {
        content
            .background(TC.surface, in: RoundedRectangle(cornerRadius: TC.Radius.card))
            .overlay {
                RoundedRectangle(cornerRadius: TC.Radius.card)
                    .strokeBorder(
                        emphasised ? TC.gold.opacity(0.55) : TC.line,
                        lineWidth: TC.Space.hairline
                    )
            }
    }
}

/// See `View.tcScreen()`.
private struct TCScreen: ViewModifier {
    @Environment(\.colorScheme) private var systemScheme

    func body(content: Content) -> some View {
        content
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .background(TC.ground)
            .tint(TC.green)
            .environment(\.colorScheme, TC.forcedColorScheme ?? systemScheme)
    }
}

/// A filled primary action whose label is legible on its fill in both
/// appearances.
///
/// This is a full `ButtonStyle` rather than a tint plus a `foregroundStyle`,
/// and that is not a stylistic preference. `.borderedProminent` derives its
/// own label colour from the tint and ignores an outer `.foregroundStyle`, so
/// the obvious version of this fix compiles, reads correctly, changes
/// nothing, and leaves white-on-mint at 2.32:1 exactly where it was. Drawing
/// the fill and the label here is what actually makes the pair hold.
struct TCPrimaryButtonStyle: ButtonStyle {
    @Environment(\.isEnabled) private var isEnabled

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            // Spec §6.1: primary is `label.control.primary` (12/600) padded
            // `5px 12-14px`. It was 13/600 at `12x8`, which drew the filled
            // action a step larger than the outlined buttons standing beside
            // it in the same row -- an emphasis the palette already carries.
            .font(TC.Font_.labelControlPrimary)
            .foregroundStyle(TC.primaryLabel)
            .padding(.horizontal, TC.Space.m)
            .padding(.vertical, TC.Space.control)
            .background(
                RoundedRectangle(cornerRadius: TC.Radius.inset, style: .continuous)
                    .fill(TC.primaryFill)
            )
            .opacity(isEnabled ? (configuration.isPressed ? 0.82 : 1) : 0.45)
            .contentShape(Rectangle())
    }
}

extension View {
    /// The filled primary action. Replaces `.buttonStyle(.borderedProminent)`
    /// rather than decorating it -- see `TCPrimaryButtonStyle` for why.
    func tcPrimaryAction() -> some View {
        buttonStyle(TCPrimaryButtonStyle())
    }

    func tcCard(emphasised: Bool = false) -> some View {
        modifier(TCCard(emphasised: emphasised))
    }

    /// Constrains a screen to its reading column and keeps it left-aligned
    /// inside a window that may be much wider.
    func tcColumn(_ width: CGFloat = TC.Measure.column) -> some View {
        frame(maxWidth: width, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .topLeading)
    }

    /// The content area's ground plus the brand accent.
    ///
    /// Applied per screen rather than once at the window, deliberately. The
    /// `Window` scene sets the same tint, but a screen also has to carry it
    /// on its own: the screenshot hook rasterizes these views detached from
    /// any scene, and a verification image that shows a different accent
    /// from the shipping app is worse than no image. Applying it in both
    /// places costs nothing -- the inner value simply wins -- and means what
    /// is captured is what runs.
    ///
    /// The brand stops here. Toolbar, sidebar, sheet chrome and the
    /// menu-bar popover stay system materials.
    func tcScreen() -> some View {
        modifier(TCScreen())
    }
}

// MARK: - Small parts

/// A field label on a manifest strip: the site's `.eyebrow`, in SF.
struct TCFieldLabel: View {
    let text: String
    var tone: TC.Tone?

    init(_ text: String, tone: TC.Tone? = nil) {
        self.text = text
        self.tone = tone
    }

    var body: some View {
        Text(text.uppercased())
            .font(TC.Font_.fieldLabel)
            .tracking(0.5)
            .foregroundStyle(tone.map { AnyShapeStyle($0.textColor) } ?? AnyShapeStyle(.tertiary))
    }
}

/// A short state token: symbol plus words, in a tone. The site's `.pill` --
/// fully rounded, hairline bordered, heavy small type.
///
/// Both halves are mandatory. The symbol is what keeps the state legible
/// without colour; the words are what keep it legible without the symbol.
struct TCTag: View {
    let text: String
    var tone: TC.Tone = .neutral
    /// Overrides the tone's default glyph where a more specific one exists.
    var symbol: String?

    var body: some View {
        HStack(spacing: TC.Space.xxs) {
            Image(systemName: symbol ?? tone.symbol)
                .imageScale(.small)
            Text(text)
        }
        // Spec §6.2: `mono.chip` at 11/500, padded `2px 8px`. It was set in
        // `ledger` (12pt) at `8x3`, which made a status token the same size as
        // the manifest figures it sits beside; the chip is an annotation on a
        // row, not one of the row's facts.
        .font(TC.Font_.monoChip)
        .foregroundStyle(tone.textColor)
        .padding(.horizontal, TC.Space.s)
        .padding(.vertical, TC.Space.micro)
        .overlay {
            Capsule().strokeBorder(tone.color.opacity(0.45), lineWidth: TC.Space.hairline)
        }
        .accessibilityElement(children: .combine)
    }
}

/// The read gate's box, spec §6.9: a 13pt square at `radius.checkbox`, filled
/// `green.brand` with a white tick when checked and outlined in `ink.tertiary`
/// at 1.5pt when not.
///
/// It is drawn rather than taken from `Toggle(.checkbox)` because the read gate
/// is not a control -- nothing here is clickable on its own. It is the app
/// REPORTING a condition it has observed (the transcript tab was opened; the
/// acknowledgement was given), and a system checkbox would invite a person to
/// click the report instead of doing the thing it reports. The tick is what
/// carries the state, not the fill, so it survives greyscale.
///
/// Accessibility-hidden on purpose: every call site combines it into a row
/// whose label already says the condition in words.
struct TCReadGateCheckbox: View {
    var checked: Bool

    var body: some View {
        RoundedRectangle(cornerRadius: TC.Radius.checkbox)
            .fill(checked ? TC.green : Color.clear)
            .overlay {
                if checked {
                    Image(systemName: "checkmark")
                        .font(.system(size: 9, weight: .bold))
                        .foregroundStyle(TC.onAccent)
                } else {
                    RoundedRectangle(cornerRadius: TC.Radius.checkbox)
                        .strokeBorder(TC.inkTertiary, lineWidth: TC.Border.medium)
                }
            }
            .frame(width: TC.Control.checkbox, height: TC.Control.checkbox)
            .accessibilityHidden(true)
    }
}

/// A section heading with a hairline rule running to the end of the column.
///
/// The rule is structural, not decorative: it is what tells a reader that
/// the group below it is a different kind of thing from the group above,
/// which is the whole job of a heading in a screen made of lists. The site
/// bands its sections with the same single `--line` rule.
struct TCSectionHeader: View {
    let title: String
    var trailing: String?

    var body: some View {
        // A long heading and a rule cannot share a line. Rather than let the
        // label wrap under its own rule -- which reads as a layout bug --
        // the rule drops to its own line when the words need the width.
        // This is also what keeps the header intact at accessibility text
        // sizes, where every heading is a long heading.
        ViewThatFits(in: .horizontal) {
            HStack(alignment: .center, spacing: TC.Space.m) {
                label.lineLimit(1).fixedSize()
                rule
                trailingFigure
            }
            VStack(alignment: .leading, spacing: TC.Space.xs) {
                HStack(alignment: .center, spacing: TC.Space.m) {
                    label
                    Spacer(minLength: TC.Space.m)
                    trailingFigure
                }
                rule
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(trailing.map { "\(title), \($0)" } ?? title)
    }

    private var label: some View {
        TCFieldLabel(title, tone: .clear)
    }

    private var rule: some View {
        Rectangle()
            .fill(TC.line)
            .frame(height: TC.Space.hairline)
    }

    @ViewBuilder
    private var trailingFigure: some View {
        if let trailing {
            Text(trailing)
                .font(TC.Font_.ledger)
                .foregroundStyle(.tertiary)
        }
    }
}
