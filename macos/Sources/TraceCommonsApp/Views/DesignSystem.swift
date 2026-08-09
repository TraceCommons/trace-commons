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
///   the scale (a warm near-black, `#15170F`-family) rather than the
///   blue-black that a naive inversion produces.
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
        static let xxs: CGFloat = 4
        static let xs: CGFloat = 6
        static let s: CGFloat = 8
        static let m: CGFloat = 12
        static let l: CGFloat = 16
        static let xl: CGFloat = 20
        static let xxl: CGFloat = 28
        static let xxxl: CGFloat = 36
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

    /// The site's radii: 6 and 8, with 999 pills for badges.
    enum Radius {
        static let card: CGFloat = 8
        static let inset: CGFloat = 6
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
    enum Font_ {
        /// Screen titles. "2 sessions waiting for your decision".
        static let screenTitle = Font.title3.weight(.bold)
        /// Onboarding headlines. The site's `h1` is very heavy; this is the
        /// nearest honest equivalent that still sits inside a window.
        static let display = Font.title.weight(.heavy)
        static let sectionTitle = Font.title3.weight(.bold)
        /// The name of the thing a card is about.
        static let cardTitle = Font.headline
        /// The opening prompt -- the text that actually identifies a session
        /// to the person who wrote it.
        static let body = Font.callout
        /// Attribution, timestamps, supporting sentences.
        static let meta = Font.callout
        /// Field labels on the manifest strip, and the site's eyebrows.
        static let fieldLabel = Font.caption2.weight(.heavy)
        /// Figures on the manifest strip, and anything else countable.
        static let ledger = Font.system(.footnote, design: .monospaced)
            .weight(.medium)
        /// Footnotes and disclosure text.
        static let footnote = Font.caption
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

    private static func hex(_ value: UInt32) -> NSColor {
        NSColor(
            srgbRed: Double((value >> 16) & 0xFF) / 255,
            green: Double((value >> 8) & 0xFF) / 255,
            blue: Double(value & 0xFF) / 255,
            alpha: 1
        )
    }

    /// Site `--bg`. The ground a person reads on. Warm, never neutral grey.
    static let ground = dynamic(hex(0xF6F7F4), hex(0x15170F).blended(withFraction: 0.06, of: .white)!)
    /// Site `--surface`. Card faces.
    static let surface = dynamic(hex(0xFFFFFF), hex(0x21241E))
    /// Site `--surface-2`. Recessed strips inside a card.
    static let surfaceInset = dynamic(hex(0xEEF2F0), hex(0x2A2E27))
    /// Site `--line`.
    static let line = dynamic(hex(0xD9DFDC), hex(0x3B4038))

    /// Site `--green`. Primary. Good standing, and the app's accent.
    static let green = dynamic(hex(0x178F70), hex(0x3FBE9A))
    /// Site `--blue`. Secondary. Held, ranked, in progress.
    static let blue = dynamic(hex(0x315FBA), hex(0x7FA0EC))
    /// Site `--gold`. Weigh this before deciding.
    static let gold = dynamic(hex(0xB9821F), hex(0xDCAA43))
    /// Site `--coral`. Refused, withdrawn, cannot proceed.
    static let coral = dynamic(hex(0xD65D4F), hex(0xF2887A))

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
    static let greenText = dynamic(hex(0x0F7256), hex(0x5CD3AF))
    static let goldText = dynamic(hex(0x8A5F12), hex(0xE2B75C))
    static let coralText = dynamic(hex(0xB8483B), hex(0xF79C8F))
    static let blueText = dynamic(hex(0x315FBA), hex(0x9DB6F1))

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

// MARK: - The mark

/// The TraceCommons mark, as pure geometry.
///
/// A direct transcription of `.brand-mark` in `community/public/styles.css`:
/// a square on the surface colour with a hairline border, a green wedge cut
/// from the top-left corner at 38% along the leading diagonal, and a blue
/// field beyond 45% along the other. No asset, no bundled image, and it
/// stays crisp at every size the app uses it at.
///
/// `monochrome` is the menu-bar variant, and it is a reduction rather than a
/// desaturation. Filled flat in one tint the two wedges touch, merge, and
/// cover most of the square -- a heavy dark blob in a menu bar full of light
/// line art. So the monochrome build keeps the green wedge solid, drops the
/// blue field to a wash, opens a hairline seam between them, and draws the
/// square's border. The two-shape relationship survives; the weight does
/// not. Everything is expressed in `.primary`, so the system tints it for a
/// light or dark menu bar and inverts it while the menu is open.
/// `reveal` drives the one piece of motion in the app -- see `BrandMarkIntro`.
/// At `1` the mark is exactly the site's. Below that, each wedge is drawn
/// back along its own diagonal, so the mark assembles from its own geometry
/// rather than fading or sliding as a picture would.
struct BrandMark: View {
    var size: CGFloat = 28
    var monochrome: Bool = false
    var reveal: Double = 1

    var body: some View {
        ZStack {
            BlueField(reveal: reveal)
                .fill(monochrome ? Color.primary.opacity(0.35) : TC.blue)
            GreenWedge(reveal: reveal, seam: monochrome ? 0.10 : 0)
                .fill(monochrome ? Color.primary : TC.green)
        }
        .frame(width: size, height: size)
        .background(monochrome ? Color.clear : TC.surface)
        .overlay {
            Rectangle().strokeBorder(
                monochrome ? Color.primary.opacity(0.55) : TC.line,
                lineWidth: TC.Space.hairline
            )
        }
        .accessibilityHidden(true)
    }
}

/// Site: `linear-gradient(135deg, var(--green) 0 38%, transparent 38% 100%)`.
/// The 38% stop along the leading diagonal puts the cut at `x + y = 0.76`.
private struct GreenWedge: Shape {
    var reveal: Double
    var seam: CGFloat

    var animatableData: Double {
        get { reveal }
        set { reveal = newValue }
    }

    func path(in rect: CGRect) -> Path {
        let t = (0.76 - seam) * max(0, min(1, reveal))
        var path = Path()
        path.move(to: rect.origin)
        path.addLine(to: CGPoint(x: rect.minX + t * rect.width, y: rect.minY))
        path.addLine(to: CGPoint(x: rect.minX, y: rect.minY + t * rect.height))
        path.closeSubpath()
        return path
    }
}

/// Site: `linear-gradient(45deg, transparent 0 45%, var(--blue) 45% 100%)`.
/// The 45% stop along the other diagonal puts the cut at `y = x + 0.1`.
/// Revealing sweeps that cut in from the top-right corner.
private struct BlueField: Shape {
    var reveal: Double

    var animatableData: Double {
        get { reveal }
        set { reveal = newValue }
    }

    func path(in rect: CGRect) -> Path {
        let d = -1 + 1.1 * max(0, min(1, reveal))
        func point(_ x: CGFloat, _ y: CGFloat) -> CGPoint {
            CGPoint(x: rect.minX + x * rect.width, y: rect.minY + y * rect.height)
        }
        var path = Path()
        if d <= 0 {
            // The cut crosses the top and right edges.
            path.move(to: point(-d, 0))
            path.addLine(to: point(1, 0))
            path.addLine(to: point(1, 1 + d))
        } else {
            // The cut has passed the corner and now crosses left and bottom.
            path.move(to: point(0, d))
            path.addLine(to: point(0, 0))
            path.addLine(to: point(1, 0))
            path.addLine(to: point(1, 1))
            path.addLine(to: point(1 - d, 1))
        }
        path.closeSubpath()
        return path
    }
}

/// The mark, assembling itself once, on the first screen a contributor ever
/// sees.
///
/// The community site has no motion at all -- there is not one `transition`,
/// `@keyframes` or `requestAnimationFrame` in it -- so there was nothing to
/// port. This is the one moment of motion in the app, and it is built from
/// the only thing the brand actually owns: the mark's two diagonals. Each
/// wedge draws in along its own axis and stops. There is no loop, no
/// bounce, no ambient drift, and no second animated thing anywhere else in
/// the product, because an app that asks to be trusted with somebody's
/// source code should not fidget.
///
/// It respects Reduce Motion by rendering the finished mark immediately.
struct BrandMarkIntro: View {
    var size: CGFloat
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var reveal: Double = 0

    var body: some View {
        BrandMark(size: size, reveal: reduceMotion ? 1 : reveal)
            .onAppear {
                guard !reduceMotion else { return }
                withAnimation(.easeOut(duration: 0.85).delay(0.15)) { reveal = 1 }
            }
    }
}

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

extension View {
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
        .font(TC.Font_.ledger)
        .foregroundStyle(tone.textColor)
        .padding(.horizontal, TC.Space.s)
        .padding(.vertical, 3)
        .overlay {
            Capsule().strokeBorder(tone.color.opacity(0.45), lineWidth: TC.Space.hairline)
        }
        .accessibilityElement(children: .combine)
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
