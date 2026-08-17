import SwiftUI

/// The community brand: the second design system this app embeds a piece of.
///
/// ## Why this is not in `TC`
///
/// `TC` is the private tool's language -- warm ground, hairlines, SF, 6-8pt
/// radii, two appearances. This is the commons': pure white paper, 2px black
/// frames, Helvetica, mint, and no corner radius anywhere. The two are meant
/// to read as foreign to each other, because the black frame is the exact
/// boundary of what becomes public (spec §7.3). A person scrolling History
/// should be able to see where their private record stops without reading a
/// word.
///
/// That is why these values stay out of `TC` even now that they are shared.
/// Folding them into the native token set would make them look like
/// alternatives to the native tokens, and the first time somebody reached for
/// `TC.brandInk` to warm up an ordinary card the seam these surfaces exist to
/// draw would be gone. The namespace is deliberately verbose for the same
/// reason: `CommunityBrand.ink` on a native surface reads as a mistake, which
/// is the point. Use it only inside a surface that is drawing the commons.
///
/// ## Light-only, on purpose
///
/// The community site declares `color-scheme: light` and has no dark palette
/// (§2.2). Every colour below is therefore a flat sRGB literal rather than an
/// appearance-aware pair: a brand panel keeps its own appearance when the rest
/// of the window goes dark, which is exactly what makes it read as an embedded
/// piece of somewhere else. Surfaces that fill the frame also pin
/// `.environment(\.colorScheme, .light)` so system-drawn parts inside them --
/// the mark, links, focus rings -- follow the panel rather than the window.
///
/// ## The one yellow
///
/// `yellow` (`#f5c91f`) is used exactly once in the product, at the manifesto
/// headline in `CreditRecordView`, and exactly once on the website. It is
/// defined here because that is where the palette lives, not because it is
/// available. A second use is a bug.
enum CommunityBrand {
    // MARK: - Palette (§2.2)

    /// `brand.ink` `#000000`. 2px frames, internal rules, all text inside a
    /// brand panel.
    static let ink = hex(0x000000)
    /// `brand.paper` `#ffffff`. The panel ground.
    static let paper = hex(0xFFFFFF)
    /// `brand.accent` `#00d4aa`, the mint. Primary button fill, headline
    /// highlight, mark square, coin face.
    static let accent = hex(0x00D4AA)
    /// `brand.rim` `#00b894`. The coin's offset rim disc; the globe's dashed
    /// arc and its one filled node.
    static let rim = hex(0x00B894)
    /// `brand.tint` `#eafaf5`. Fill behind acknowledgement rows and the
    /// withheld-analytics notice.
    static let tint = hex(0xEAFAF5)
    /// `brand.yellow` `#f5c91f`. THE one yellow -- see the type's own note.
    /// The manifesto headline, and nothing else, ever.
    static let yellow = hex(0xF5C91F)
    /// `brand.muted` `#6b6b6b`. Mono uppercase micro-labels on white.
    static let muted = hex(0x6B6B6B)
    /// `brand.muted.onblack` `#8a8a8a`. The same role on the inverted
    /// manifesto screen.
    static let mutedOnBlack = hex(0x8A8A8A)

    private static func hex(_ value: UInt32) -> Color {
        Color(
            .sRGB,
            red: Double((value >> 16) & 0xFF) / 255,
            green: Double((value >> 8) & 0xFF) / 255,
            blue: Double(value & 0xFF) / 255
        )
    }

    // MARK: - Type (§3.3)

    /// The community type scale. Helvetica rather than the system face, and
    /// stated sizes rather than text styles: this is landing type, and its
    /// tracking and line-height ratios are quoted against a fixed size.
    ///
    /// Tracking is given in `em` in the spec and in points by SwiftUI, so every
    /// tracking token below is the spec's own em value multiplied by the size
    /// it applies to -- `tracking(_:at:)` does the conversion, and doing it
    /// there rather than at a call site is what keeps a size and its tracking
    /// from drifting apart.
    ///
    /// Sizes are given to `Font.custom(_:size:)` rather than
    /// `Font.custom(_:fixedSize:)`, so the brand surfaces still answer to the
    /// accessibility text sizes. The ratios survive that: tracking is
    /// expressed as a fraction of the nominal size either way.
    enum Font_ {
        /// Helvetica, the brand face. Not bundled -- it ships with macOS.
        static let face = "Helvetica Neue"

        /// Display type at any size: 700 Helvetica, always set UPPERCASE.
        static func display(_ size: CGFloat) -> Font {
            .custom(face, size: size).weight(.bold)
        }

        /// Brand prose: 500 Helvetica.
        static func sans(_ size: CGFloat, _ weight: Font.Weight = .medium) -> Font {
            .custom(face, size: size).weight(weight)
        }

        /// The brand's mono, for micro-labels, buttons and figures.
        static func mono(_ size: CGFloat, _ weight: Font.Weight = .bold) -> Font {
            .system(size: size, weight: weight, design: .monospaced)
        }

        /// The spec's `em` tracking, in the points SwiftUI wants.
        static func tracking(_ em: CGFloat, at size: CGFloat) -> CGFloat {
            em * size
        }

        /// `display.hero`, 700/50/UPPERCASE, `-.04em`, line-height `.88`.
        /// The onboarding welcome headline drives this from a `@ScaledMetric`,
        /// so the size and its tracking are given as parts rather than as a
        /// finished font.
        static let heroSize: CGFloat = 50
        /// See `heroSize`. Multiply by the size actually being drawn.
        static let heroTrackingEm: CGFloat = -0.04
        /// See `heroSize`. The line height, as a multiple of the size being
        /// drawn. It is applied as an explicit frame height on each line
        /// rather than as stack spacing, because these lines carry the mint
        /// highlight as a background and a negative gap would let one line's
        /// block paint over the line above it -- see `OnboardingWelcomeContent`
        /// for the measured arithmetic.
        static let heroLineHeightEm: CGFloat = 0.88
        /// `display.manifesto`, 700/44/UPPERCASE, `-.035em`.
        static let displayManifesto = display(44)
        /// See `displayManifesto`.
        static let displayManifestoTracking = tracking(-0.035, at: 44)
        /// `display.dialog`, 700/27/UPPERCASE, `-.035em`.
        static let displayDialog = display(27)
        /// See `displayDialog`.
        static let displayDialogTracking = tracking(-0.035, at: 27)
        /// `display.panel`, 700/24/UPPERCASE, `-.035em`. Brand panel headings.
        static let displayPanel = display(24)
        /// See `displayPanel`.
        static let displayPanelTracking = tracking(-0.035, at: 24)
        /// `lede`, 500/18, line-height 1.3, `-.01em`.
        static let lede = sans(18)
        /// See `lede`.
        static let ledeTracking = tracking(-0.01, at: 18)
        /// `body.brand`, 500/13, line-height 1.4-1.45, `-.01em`.
        static let body = sans(13)
        /// See `body`.
        static let bodyTracking = tracking(-0.01, at: 13)
        /// `field.value`, 500/15 sans -- the bio.
        static let fieldValue = sans(15)
        /// `field.value`, 500/15 mono -- the handle.
        static let fieldValueMono = mono(15, .medium)
        /// See `fieldValue`.
        static let fieldValueTracking = tracking(-0.01, at: 15)
        /// `figure.brand`, 700/26 mono, tabular, `-.03em`. Community stats.
        static let figure = mono(26)
        /// See `figure`.
        static let figureTracking = tracking(-0.03, at: 26)
        /// `label.mono`, 700/11 mono/UPPERCASE, `.02em`. Every brand
        /// micro-label, meta row and page counter.
        static let labelMono = mono(11)
        /// See `labelMono`. The same `.02em` carries the 12px mono labels.
        static let monoTracking = tracking(0.02, at: 11)
        /// `button.brand`, 700/12 mono/UPPERCASE.
        static let button = mono(12)
        /// `button.brand` at the onboarding size, 700/13 mono/UPPERCASE.
        static let buttonLarge = mono(13)
        /// `chrome.mono`, 700/12 mono/UPPERCASE. The onboarding wordmark.
        static let chromeMono = mono(12)
        /// `link.mono`, 500/13 mono, underlined at the call site.
        static let linkMono = mono(13, .medium)
        /// The dialog and manifesto footnote, 500/11.
        static let footnote = sans(11)
    }

    // MARK: - Metrics (§4.2, §4.3, §4.6)

    /// The brand's frame geometry. Radius is zero everywhere inside a
    /// black-framed panel (§4.2, stated as `TC.Radius.brand`), which is why
    /// every edge these surfaces draw is a `Rectangle` rather than a rounded
    /// one.
    enum Metric {
        /// §4.3: outer frames are 2px solid `#000`.
        static let frame: CGFloat = 2
        /// §4.3: internal cell dividers and field boxes are 1px.
        static let rule: CGFloat = 1
        /// §6.5: the brand panel's padding and stack gap, both 14.
        static let panelGap: CGFloat = 14
        /// §5.8's onboarding frame: `padding:12px`, `gap:26px`, 860px canvas,
        /// a 14pt rule under the header and a 30pt hero gap.
        static let pagePadding: CGFloat = 12
        /// See `pagePadding`.
        static let pageGap: CGFloat = 26
        /// See `pagePadding`.
        static let pageWidth: CGFloat = 860
        /// See `pagePadding`.
        static let headerRule: CGFloat = 14
        /// See `pagePadding`.
        static let heroGap: CGFloat = 30
        /// §5.9.2's manifesto frame: `padding:18px 34px 30px`, `gap:22px`, on
        /// a 640px canvas.
        static let stanzaPadding: CGFloat = 34
        /// See `stanzaPadding`.
        static let stanzaGap: CGFloat = 22
    }
}

// MARK: - Brand button

/// Spec §6.1: brand primary is `#00d4aa` fill with a 2px `#000` border; brand
/// secondary is the same box in white; on the black manifesto screen both flip
/// their ink and border. All of them carry 700 mono uppercase type and none of
/// them is ever rounded -- no radius appears anywhere inside a black-framed
/// panel. Disabled is a flat `opacity:.4`, which the spec draws on "Go public".
///
/// The native `TCPrimaryButtonStyle` is the wrong control on these surfaces --
/// it is rounded, SF, and tinted from `TC`, which is exactly the language the
/// brand is not speaking.
struct CommunityBrandButtonStyle: ButtonStyle {
    /// §6.1 gives the brand button two sizes: `10x16` at 12pt inside a panel
    /// or dialog, and `12x18` at 13pt on the onboarding screens, where the
    /// type is at landing scale and the button has to hold its own beside it.
    enum Size {
        case panel
        case onboarding

        var font: Font {
            switch self {
            case .panel: return CommunityBrand.Font_.button
            case .onboarding: return CommunityBrand.Font_.buttonLarge
            }
        }

        var paddingVertical: CGFloat {
            switch self {
            case .panel: return 10
            case .onboarding: return 12
            }
        }

        var paddingHorizontal: CGFloat {
            switch self {
            case .panel: return 16
            case .onboarding: return 18
            }
        }
    }

    @Environment(\.isEnabled) private var isEnabled

    var fill: Color
    var ink: Color = CommunityBrand.ink
    var border: Color = CommunityBrand.ink
    var size: Size = .panel

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(size.font)
            .textCase(.uppercase)
            .tracking(CommunityBrand.Font_.monoTracking)
            .foregroundStyle(ink)
            .padding(.vertical, size.paddingVertical)
            .padding(.horizontal, size.paddingHorizontal)
            .background(fill)
            .overlay {
                Rectangle().strokeBorder(border, lineWidth: CommunityBrand.Metric.frame)
            }
            .opacity(isEnabled ? (configuration.isPressed ? 0.82 : 1) : 0.4)
            .contentShape(Rectangle())
    }
}

// MARK: - Brand panel

/// Spec §6.5's brand panel: a 2px `#000` frame on white paper, `padding:14`,
/// `gap:14`, radius 0.
func communityBrandPanel<Content: View>(
    @ViewBuilder _ content: () -> Content
) -> some View {
    VStack(alignment: .leading, spacing: CommunityBrand.Metric.panelGap) {
        content()
    }
    .padding(CommunityBrand.Metric.panelGap)
    .frame(maxWidth: .infinity, alignment: .leading)
    .background(CommunityBrand.paper)
    .overlay {
        Rectangle().strokeBorder(
            CommunityBrand.ink,
            lineWidth: CommunityBrand.Metric.frame
        )
    }
}
