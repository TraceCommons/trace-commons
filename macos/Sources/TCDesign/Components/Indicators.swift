import SwiftUI

/// A status dot. Status is shown as a dot and a label, never a fill.
public struct GlassStatusDot: View {
    private let status: GlassStatus
    private let size: CGFloat
    private let halo: Bool
    private let ring: Bool

    public init(_ status: GlassStatus, size: CGFloat = GlassTokens.Size.dotLarge, halo: Bool = false, ring: Bool = false) {
        self.status = status
        self.size = size
        self.halo = halo
        self.ring = ring
    }

    public var body: some View {
        Circle()
            .fill(status.color)
            .frame(width: size, height: size)
            .overlay {
                if halo {
                    Circle().stroke(GlassColor.ink(0.08), lineWidth: 2).padding(-1)
                }
                if ring {
                    Circle().stroke(status.color.opacity(0.25), lineWidth: 3).padding(-1.5)
                }
            }
            .accessibilityHidden(true)
    }
}

/// A dot and its label: the only way status is shown.
public struct GlassStatusLabel: View {
    private let title: String
    private let status: GlassStatus

    public init(_ title: String, status: GlassStatus) {
        self.title = title
        self.status = status
    }

    public var body: some View {
        HStack(spacing: 7) {
            GlassStatusDot(status)
            Text(title)
        }
        .glassType(GlassTokens.TypeScale.label)
        .foregroundStyle(GlassColor.textSecondary)
        .accessibilityElement(children: .combine)
    }
}

/// A mono chip with a hairline ring at 60% of its colour.
public struct GlassChip: View {
    private let title: String
    private let status: GlassStatus?

    public init(_ title: String, status: GlassStatus? = nil) {
        self.title = title
        self.status = status
    }

    public var body: some View {
        let ink = status?.textColor ?? GlassColor.textSecondary
        HStack(spacing: 5) {
            if let status { GlassStatusDot(status, size: 6) }
            Text(title)
        }
        .glassType(GlassTokens.TypeScale.mono)
        .foregroundStyle(ink)
        .padding(.vertical, 3)
        .padding(.horizontal, 9)
        .overlay(Capsule().strokeBorder(ink.opacity(0.6), lineWidth: 0.5))
        .accessibilityElement(children: .combine)
    }
}

/// A tinted verdict pill (Contributed, Under review, Taken back, Drafts).
public struct GlassTag: View {
    /// `failed` is a check that failed: the outside tint, outlined, so it
    /// is never the same pill as `outside` and never colour only.
    public enum Tone: Sendable, Equatable { case neutral, on, ask, outside, failed, accent }

    private let title: String
    private let tone: Tone

    public init(_ title: String, tone: Tone = .neutral) {
        self.title = title
        self.tone = tone
    }

    public var body: some View {
        let (fill, ink): (GlassRGBA, Color) = switch tone {
        case .neutral: (GlassTokens.Color.tintNeutral, GlassColor.textSecondary)
        // Text: the text-safe status colours (4.5:1 in light).
        case .on: (GlassTokens.Color.tintOn, GlassTokens.Color.statusOnText.color)
        case .ask: (GlassTokens.Color.tintAsk, GlassTokens.Color.statusAskText.color)
        case .outside, .failed: (GlassTokens.Color.tintOutside, GlassTokens.Color.statusOutsideText.color)
        case .accent: (GlassTokens.Color.tintAccent, GlassColor.accentText)
        }
        Text(title)
            .glassType(GlassTokens.TypeScale.micro)
            .foregroundStyle(ink)
            .padding(.vertical, 3)
            .padding(.horizontal, 8)
            .background(Capsule().fill(fill.color))
            .overlay(Capsule().strokeBorder(ink, lineWidth: tone == .failed ? 1 : 0))
    }
}

/// Decisions owed. On the menu bar and the Traces tab only; never queue
/// depth or credit. `subtle` is the white count pill inside a tab.
public struct GlassBadge: View {
    private let count: Int?
    private let subtle: Bool
    private let label: String?

    /// `label` is what VoiceOver reads, from the core's copy; without it the
    /// count is read alone. A nil `count` is unknown and draws a dash, never
    /// a number.
    public init(count: Int?, subtle: Bool = false, label: String? = nil) {
        self.count = count
        self.subtle = subtle
        self.label = label
    }

    /// The most the badge states; past it, `99+`, as the menu bar does
    /// (`MenuBarStatus.badgeCap`).
    public static let cap = 99

    /// What the badge draws: the count up to `cap`, then `99+`; a dash for
    /// unknown, never a number.
    public static func text(for count: Int?) -> String {
        guard let count else { return "—" }
        return count > cap ? "\(cap)+" : String(count)
    }

    public var body: some View {
        Text(Self.text(for: count))
            .glassType(GlassTokens.TypeScale.micro)
            .monospacedDigit()
            .foregroundStyle(subtle ? GlassColor.textPrimary : GlassTokens.Color.textOnStatus.color)
            .padding(.horizontal, 5)
            .frame(minWidth: subtle ? nil : 16, minHeight: subtle ? 14 : 16)
            .background(
                Capsule().fill(subtle ? GlassColor.ink(0.16) : GlassTokens.Color.badgeFill.color)
            )
            .accessibilityLabel(label ?? Self.text(for: count))
    }
}

/// The tools the shell knows. A tool with artwork shows its logo; one
/// without (Gemini CLI, Cline, any other) shows its initials.
/// Display names are not here: the caller shows the core's name for a tool.
public enum GlassTool: Sendable, Equatable {
    case claudeCode, codex, antigravity, geminiCLI, cline, openCode, theia
    /// Any other tool, by its two-letter initials.
    case other(initials: String)

    /// The tint the tool's logo is drawn in.
    public var tint: GlassRGBA {
        switch self {
        case .claudeCode: GlassTokens.Color.toolClaude
        case .codex: GlassTokens.Color.toolCodex
        case .antigravity, .geminiCLI: GlassTokens.Color.toolAntigravity
        case .openCode: GlassTokens.Color.toolOpenCode
        case .theia: GlassTokens.Color.toolTheia
        case .cline, .other: GlassTokens.Color.textPrimary
        }
    }

    /// The logo artwork, if the tool has any. Gemini CLI and Cline have
    /// none yet, in either shell.
    public var logo: GlassToolLogoID? {
        switch self {
        case .claudeCode: .claude
        case .codex: .codex
        case .antigravity: .antigravity
        case .openCode: .openCode
        case .theia: .theia
        case .geminiCLI, .cline, .other: nil
        }
    }

    /// The tool's two-letter mark, for a surface with no logo artwork.
    public var initials: String {
        switch self {
        case .claudeCode: "CC"
        case .codex: "Cx"
        case .antigravity: "Ag"
        case .geminiCLI: "Gm"
        case .cline: "Cl"
        case .openCode: "OC"
        case .theia: "Th"
        case let .other(initials): String(initials.prefix(2))
        }
    }
}

/// The 22pt tile in a tree row: a tool's mark, a folder or a session.
/// A tool is drawn as its logo in its tint, or as its tinted initials when
/// it has no logo.
public struct GlassToolTile: View {
    public enum Kind: Sendable, Equatable {
        case tool(GlassTool), folder, session
        /// The "+" of an add box (the first run's "add your tool").
        case add
        /// A folder tile marked with its name's first letter (#1146
        /// `HistoryRow`: `project_label.slice(0, 1).toUpperCase()`).
        case folderInitial(String)
    }

    /// A name's first letter, upper-cased; the folder mark for none.
    public static func initial(_ name: String) -> String {
        name.first.map { String($0).uppercased() } ?? folderMark
    }

    private let kind: Kind
    private let large: Bool

    public init(_ kind: Kind, large: Bool = false) {
        self.kind = kind
        self.large = large
    }

    /// The folder and session tiles' marks, as #1146 draws them.
    public static let folderMark = "dir"
    static let sessionMark = "▤"

    public var body: some View {
        let side = large ? GlassTokens.Size.toolTileLarge : GlassTokens.Size.toolTile
        // The large tile rounds like a control (#1146 `.tc-tool-tile--lg`).
        let radius = large ? GlassTokens.Radius.control : GlassTokens.Radius.tile
        Group {
            switch kind {
            case let .tool(tool):
                Group {
                    if let logo = tool.logo {
                        let mark = large ? GlassTokens.Size.toolLogoLarge : GlassTokens.Size.toolLogo
                        GlassToolLogoShape(logo)
                            .fill(tool.tint.color)
                            .frame(width: mark, height: mark)
                    } else {
                        Text(tool.initials)
                            .glassGlyph(large ? 11 : 9, weight: .bold)
                            .foregroundStyle(tool.tint.color)
                    }
                }
                    .frame(width: side, height: side)
                    .background(RoundedRectangle(cornerRadius: radius, style: .continuous).fill(GlassColor.ink(0.1)))
            // #1146 `ToolTile` marks a folder "dir" and a session "▤": a
            // glyph, hidden from assistive tech with the rest of the tile.
            case .folder:
                Text(Self.folderMark)
                    .glassGlyph(large ? 11 : 9, weight: .bold)
                    .foregroundStyle(GlassTokens.Color.tileFolderInk.color)
                    .frame(width: side, height: side)
                    .background(RoundedRectangle(cornerRadius: radius, style: .continuous).fill(GlassTokens.Color.tileFolder.color))
            case let .folderInitial(name):
                Text(Self.initial(name))
                    .glassGlyph(large ? 11 : 9, weight: .bold)
                    .foregroundStyle(GlassTokens.Color.tileFolderInk.color)
                    .frame(width: side, height: side)
                    .background(RoundedRectangle(cornerRadius: radius, style: .continuous).fill(GlassTokens.Color.tileFolder.color))
            case .add:
                Image(systemName: "plus")
                    .glassGlyph(large ? 13 : 10, weight: .semibold)
                    .foregroundStyle(GlassColor.textSecondary)
                    .frame(width: side, height: side)
                    .background(RoundedRectangle(cornerRadius: radius, style: .continuous).fill(GlassColor.ink(0.1)))
            case .session:
                Text(Self.sessionMark)
                    .glassGlyph(11, weight: .bold)
                    .foregroundStyle(GlassTokens.Color.statusOff.color)
                    .frame(width: side, height: side)
                    .background(RoundedRectangle(cornerRadius: radius, style: .continuous).fill(GlassTokens.Color.tileFolder.color))
            }
        }
        .accessibilityHidden(true)
    }
}

/// One bucket of `GlassBarGraph`.
public struct GlassBarBucket: Identifiable, Sendable {
    public let id: String
    public let label: String
    /// Shared: rises from the axis in purple.
    public let up: Double
    /// Kept: falls from the axis in blue.
    public let down: Double
    /// What VoiceOver reads for the bucket.
    public let description: String

    public init(id: String, label: String, up: Double, down: Double, description: String) {
        self.id = id
        self.label = label
        self.up = up
        self.down = down
        self.description = description
    }
}

/// Shared over kept, per period. Values scale to the largest bucket, never
/// below `scaleFloor`, so a count of one stays short.
public struct GlassBarGraph: View {
    private let buckets: [GlassBarBucket]
    private let scaleFloor: Double
    private let period: Int
    @Binding private var hovered: String?
    /// The window last drawn, so a change knows which side it came from.
    @State private var shownPeriod: Int?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    /// `period` names the window the buckets cover (0 is now, -1 the window
    /// before it): when it changes the bars slide in from the side the new
    /// window lies on (#1146 `tc-slide-l` / `tc-slide-r`).
    public init(
        _ buckets: [GlassBarBucket], scaleFloor: Double = 1, period: Int = 0,
        hovered: Binding<String?> = .constant(nil)
    ) {
        self.buckets = buckets
        self.scaleFloor = scaleFloor
        self.period = period
        self._hovered = hovered
    }

    /// The bar labels' ink: `statusOff`, primary under the pointer (#1146
    /// `.tc-bar-graph__label`).
    static func labelInk(hovered: Bool) -> GlassRGBA {
        hovered ? GlassTokens.Color.textPrimary : GlassTokens.Color.statusOff
    }

    public var body: some View {
        ZStack {
            bars
                .id(period)
                .transition(GlassBarSlide(from: shownPeriod ?? period, to: period).transition)
        }
        .animation(reduceMotion ? nil : GlassBarSlide.animation, value: period)
        .clipped()
        .onAppear { shownPeriod = period }
        .onChange(of: period) { _, new in shownPeriod = new }
    }

    private var bars: some View {
        let maximum = max(scaleFloor, buckets.map { max($0.up, $0.down) }.max() ?? 0)
        let thin = buckets.count > 14
        return HStack(alignment: .bottom, spacing: thin ? 2 : 6) {
            ForEach(buckets) { bucket in
                VStack(spacing: GlassTokens.Space.s2) {
                    ZStack {
                        let track = RoundedRectangle(cornerRadius: thin ? 3 : GlassTokens.Radius.control, style: .continuous)
                        track
                            .fill(GlassColor.ink(hovered == bucket.id ? 0.18 : 0.08))
                            .glassEdge(GlassTokens.Shadow.barTrackEdge, in: track)
                        VStack(spacing: 0) {
                            Spacer(minLength: 0)
                            bar(bucket.up, of: maximum, color: GlassTokens.Color.dataShared.color, top: true)
                            Rectangle().fill(GlassColor.ink(0.18)).frame(height: 1)
                            bar(bucket.down, of: maximum, color: GlassTokens.Color.dataKept.color, top: false)
                            Spacer(minLength: 0)
                        }
                        .padding(.horizontal, thin ? 2 : 6)
                    }
                    .frame(height: 96)
                    Text(bucket.label)
                        .glassType(thin ? GlassTokens.TypeScale.micro.weight(.regular) : GlassTokens.TypeScale.caption)
                        .foregroundStyle(Self.labelInk(hovered: hovered == bucket.id).color)
                        // Never wrapped or cut ("Sat", "14:00"): centred on
                        // its bar and free to run past it, as #1146's
                        // `white-space: nowrap`, without widening the bar.
                        .lineLimit(1)
                        .fixedSize()
                        .frame(width: 0)
                        .frame(minHeight: 12)
                }
                .onHover { hovered = $0 ? bucket.id : nil }
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(bucket.description)
            }
        }
    }

    private func bar(_ value: Double, of maximum: Double, color: Color, top: Bool) -> some View {
        let height = value <= 0 ? 0 : max(3, value / maximum * 46)
        // Shared stands on the axis and kept hangs from it (#1146
        // `.tc-bar-graph__down { top: 50% }`), never from the track's foot.
        return VStack(spacing: 0) {
            UnevenRoundedRectangle(
                topLeadingRadius: top ? 99 : 0,
                bottomLeadingRadius: top ? 0 : 99,
                bottomTrailingRadius: top ? 0 : 99,
                topTrailingRadius: top ? 99 : 0
            )
            .fill(color)
            .frame(height: height)
            if top { EmptyView() }
        }
        .frame(height: 47.5, alignment: top ? .bottom : .top)
    }
}

/// How a bar graph's new window comes in (#1146 `tc-slide-l` / `tc-slide-r`
/// over .45s): a later window from the trailing side, an earlier one from
/// the leading side, the window it replaces fading out.
public enum GlassBarSlide: Sendable, Equatable {
    case none, fromTrailing, fromLeading

    public init(from old: Int, to new: Int) {
        self = new > old ? .fromTrailing : new < old ? .fromLeading : .none
    }

    /// #1146's `cubic-bezier(.4,0,.2,1)` at .45s.
    static let animation = Animation.timingCurve(0.4, 0, 0.2, 1, duration: 0.45)

    var transition: AnyTransition {
        switch self {
        case .none: .opacity
        case .fromTrailing: .asymmetric(insertion: .move(edge: .trailing).combined(with: .opacity), removal: .opacity)
        case .fromLeading: .asymmetric(insertion: .move(edge: .leading).combined(with: .opacity), removal: .opacity)
        }
    }
}

/// The map's field (#1146 `--tc-map-fill`): `radial-gradient(80% 60% at
/// 50% 55%, mapFieldInner, mapFieldOuter)`, an ellipse 80% of the field's
/// width and 60% of its height across each radius, its centre a little
/// below the middle.
public struct GlassMapField: View {
    public init() {}

    /// The ellipse's radii as fractions of the field, and its centre.
    static let radii = CGSize(width: 0.8, height: 0.6)
    static let center = UnitPoint(x: 0.5, y: 0.55)

    public var body: some View {
        GeometryReader { proxy in
            let size = proxy.size
            EllipticalGradient(
                colors: [GlassTokens.Color.mapFieldInner.color, GlassTokens.Color.mapFieldOuter.color],
                center: .center, startRadiusFraction: 0, endRadiusFraction: 0.5)
                .frame(width: size.width * Self.radii.width * 2, height: size.height * Self.radii.height * 2)
                .position(x: size.width * Self.center.x, y: size.height * Self.center.y)
        }
        .background(GlassTokens.Color.mapFieldOuter.color)
        .clipped()
        .accessibilityHidden(true)
    }
}

/// A node card on the map, shown on hover and pinned on click.
public struct GlassNodeCard: View {
    private let title: String
    private let detail: String
    private let hint: String?

    public init(_ title: String, detail: String, hint: String? = nil) {
        self.title = title
        self.detail = detail
        self.hint = hint
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(title).glassType(GlassTokens.TypeScale.bodyStrong).foregroundStyle(GlassColor.textPrimary)
            Text(detail).glassType(GlassTokens.TypeScale.label.weight(.regular)).foregroundStyle(GlassColor.textSecondary)
            if let hint {
                Text(hint).glassType(GlassTokens.TypeScale.micro.weight(.regular).monospaced).foregroundStyle(GlassColor.textTertiary).padding(.top, 3)
            }
        }
        .padding(.vertical, 12)
        .padding(.horizontal, 14)
        .frame(width: GlassTokens.Size.nodeCardWidth, alignment: .leading)
        .glassSurface(.nodeCard, floating: true)
        .accessibilityElement(children: .combine)
    }
}

/// A map node's look, for the map's `Canvas` (R8): fill, ring and radius
/// by state. The map draws; this states what each node looks like.
public struct GlassMapNodeStyle: Sendable, Equatable {
    public enum State: Sendable, Equatable { case active, idle, outside, hub }

    public let fill: GlassRGBA
    public let ring: Bool
    public let radius: CGFloat

    public init(_ state: State) {
        switch state {
        case .active:
            fill = GlassTokens.Color.blue
            ring = true
            radius = 11
        case .idle:
            fill = GlassTokens.Color.mapNodeOff
            ring = false
            radius = 11
        case .outside:
            fill = GlassTokens.Color.statusOutside
            ring = true
            radius = 11
        case .hub:
            fill = GlassTokens.Color.mapHub
            ring = true
            radius = 16
        }
    }
}
