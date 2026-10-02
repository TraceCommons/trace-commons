import SwiftUI

// The menu-bar popover's parts (R13 of #1173), from the "Menu bar item and
// popover" handoff: the state pills, the sub-list, the day graph, the
// menu-bar strip and the activity rows. Every word is the caller's.

// MARK: - State pill

/// What fills a state pill's circle: one colour, or the three mode colours
/// together for a mixed set of folder modes.
public enum GlassPillFill: Sendable, Equatable {
    case solid(GlassRGBA)
    case mixed

    @ViewBuilder
    var shape: some View {
        switch self {
        case let .solid(rgba):
            Circle().fill(rgba.color)
        case .mixed:
            Circle().fill(AngularGradient(
                colors: [
                    GlassTokens.Color.menuModeAsk.color, GlassTokens.Color.menuModeArmed.color,
                    GlassTokens.Color.menuModeNever.color, GlassTokens.Color.menuModeAsk.color,
                ],
                center: .center))
        }
    }
}

/// A 44pt glass pill: a 31pt filled circle with a white glyph, and a
/// caption over a value that shows only while the pill is expanded. The
/// row expands the hovered pill and collapses the others to circles.
public struct GlassStatePill: View {
    private let caption: String
    private let value: String
    private let systemImage: String
    private let fill: GlassPillFill
    private let expanded: Bool
    private let action: () -> Void

    public init(
        caption: String, value: String, systemImage: String, fill: GlassPillFill,
        expanded: Bool, action: @escaping () -> Void
    ) {
        self.caption = caption
        self.value = value
        self.systemImage = systemImage
        self.fill = fill
        self.expanded = expanded
        self.action = action
    }

    public var body: some View {
        Button(action: action) {
            HStack(spacing: 0) {
                ZStack {
                    fill.shape
                    Image(systemName: systemImage)
                        .glassGlyph(14, weight: .semibold)
                        .foregroundStyle(Color.white)
                }
                .frame(width: 31, height: 31)
                if expanded {
                    VStack(alignment: .leading, spacing: 0) {
                        Text(caption)
                            .glassType(GlassTokens.TypeScale.micro.weight(.regular))
                            .foregroundStyle(GlassColor.textSecondary)
                        Text(value)
                            .glassType(GlassTokens.TypeScale.bodyStrong)
                            .foregroundStyle(GlassColor.textPrimary)
                    }
                    .lineLimit(1)
                    .padding(.leading, GlassTokens.Space.s4)
                    .transition(.opacity)
                    Spacer(minLength: 0)
                }
            }
            .padding(.horizontal, GlassTokens.Space.s3)
            .frame(minWidth: 44, maxWidth: expanded ? .infinity : 44, minHeight: 44)
            .glassSurface(.control, radius: GlassTokens.Radius.menuStatePill)
        }
        .buttonStyle(GlassPressStyle())
        .accessibilityLabel(caption)
        .accessibilityValue(value)
    }
}

// MARK: - Sub-list

/// The header that replaces the pills while a sub-list is open: a back
/// chevron and the list's title. Pressing it closes the list.
public struct GlassSubListHeader: View {
    private let title: String
    private let closeLabel: String
    private let onClose: () -> Void

    /// `closeLabel` names the control for VoiceOver, from the core's copy.
    public init(_ title: String, closeLabel: String, onClose: @escaping () -> Void) {
        self.title = title
        self.closeLabel = closeLabel
        self.onClose = onClose
    }

    public var body: some View {
        Button(action: onClose) {
            HStack(spacing: GlassTokens.Space.s4) {
                Image(systemName: "chevron.left")
                    .glassGlyph(13, weight: .semibold)
                    .foregroundStyle(GlassColor.textSecondary)
                Text(title)
                    .glassType(GlassTokens.TypeScale.title)
                    .foregroundStyle(GlassColor.textPrimary)
                Spacer(minLength: 0)
            }
            .padding(.horizontal, GlassTokens.Space.s7)
            .frame(minHeight: 44)
            .glassSurface(.control, radius: GlassTokens.Radius.menuStatePill)
        }
        .buttonStyle(GlassPressStyle())
        .accessibilityLabel(closeLabel)
        .accessibilityValue(title)
    }
}

/// One option in a sub-list: a check column, a 24pt colour circle, a label
/// and an optional sub-line. A disabled row shows its state and does not
/// act (the mode overrides, until the core can apply them).
public struct GlassOptionRow: View {
    private let title: String
    private let sub: String?
    private let fill: GlassPillFill
    private let checked: Bool
    private let action: () -> Void
    @State private var hovering = false
    @Environment(\.isEnabled) private var isEnabled

    public init(_ title: String, sub: String? = nil, fill: GlassPillFill, checked: Bool, action: @escaping () -> Void) {
        self.title = title
        self.sub = sub
        self.fill = fill
        self.checked = checked
        self.action = action
    }

    public var body: some View {
        Button(action: action) {
            HStack(spacing: GlassTokens.Space.s5) {
                Image(systemName: "checkmark")
                    .glassGlyph(11, weight: .bold)
                    .foregroundStyle(GlassColor.textPrimary)
                    .opacity(checked ? 1 : 0)
                    .frame(width: 16)
                    .accessibilityHidden(true)
                fill.shape.frame(width: 24, height: 24)
                VStack(alignment: .leading, spacing: 1) {
                    Text(title)
                        .glassType(GlassTokens.TypeScale.body.weight(.medium))
                        .foregroundStyle(GlassColor.textPrimary)
                    if let sub {
                        Text(sub)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textSecondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                Spacer(minLength: 0)
            }
            .padding(.vertical, 7)
            .padding(.horizontal, GlassTokens.Space.s5)
            .background(
                RoundedRectangle(cornerRadius: GlassTokens.Radius.control, style: .continuous)
                    .fill(hovering && isEnabled ? Color.white.opacity(0.1) : .clear)
                    .glassPressedFill()
            )
            .contentShape(Rectangle())
            .opacity(isEnabled ? 1 : 0.55)
        }
        .buttonStyle(GlassPressStyle())
        .onHover { hovering = $0 }
        .accessibilityAddTraits(checked ? .isSelected : [])
    }
}

// MARK: - Day graph

/// One column of `GlassDayGraph` and `GlassMenuBarStrip`.
public struct GlassDayColumn: Identifiable, Sendable, Equatable {
    public let id: String
    /// Shared: rises from the centre line in purple.
    public let up: Int
    /// Kept: falls below it in blue.
    public let down: Int

    public init(id: String, up: Int, down: Int) {
        self.id = id
        self.up = up
        self.down = down
    }
}

/// Shared over kept, one column per period, on a centre line, with the
/// two totals as chips and the range under it. Grey while paused.
public struct GlassDayGraph: View {
    private let columns: [GlassDayColumn]
    private let paused: Bool
    private let summary: String
    private let sharedChip: String
    private let keptChip: String
    private let leading: String
    private let trailing: String

    /// `summary` is what VoiceOver reads for the plot, from the caller: the
    /// bars are a picture, and a picture needs its words (R14).
    public init(
        columns: [GlassDayColumn], paused: Bool, summary: String,
        sharedChip: String, keptChip: String, leading: String, trailing: String
    ) {
        self.columns = columns
        self.paused = paused
        self.summary = summary
        self.sharedChip = sharedChip
        self.keptChip = keptChip
        self.leading = leading
        self.trailing = trailing
    }

    static let plotHeight: CGFloat = 104

    /// A bar's height for a count: nothing at zero, then at least 6pt, up to
    /// the half-plot less the chip room, scaled to the busiest column.
    static func height(_ count: Int, max: Int) -> CGFloat {
        guard count > 0, max > 0 else { return 0 }
        let room = plotHeight / 2 - 10
        return Swift.max(6, room * CGFloat(count) / CGFloat(max))
    }

    public var body: some View {
        let maximum = columns.map { Swift.max($0.up, $0.down) }.max() ?? 0
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            ZStack(alignment: .topLeading) {
                VStack(spacing: 0) {
                    HStack(alignment: .bottom, spacing: 2) {
                        ForEach(columns) { column in
                            UnevenRoundedRectangle(topLeadingRadius: 2, topTrailingRadius: 2)
                                .fill(paused ? GlassTokens.Color.menuBarsPaused.color : GlassTokens.Color.dataShared.color)
                                .frame(height: Self.height(column.up, max: maximum))
                                .frame(maxWidth: .infinity)
                        }
                    }
                    .frame(height: Self.plotHeight / 2, alignment: .bottom)
                    Rectangle().fill(Color.white.opacity(0.14)).frame(height: 1)
                    HStack(alignment: .top, spacing: 2) {
                        ForEach(columns) { column in
                            UnevenRoundedRectangle(bottomLeadingRadius: 2, bottomTrailingRadius: 2)
                                .fill(paused ? GlassTokens.Color.menuBarsPaused.color : GlassTokens.Color.dataKept.color)
                                .frame(height: Self.height(column.down, max: maximum))
                                .frame(maxWidth: .infinity)
                        }
                    }
                    .frame(height: Self.plotHeight / 2, alignment: .top)
                }
                chip(sharedChip, GlassTokens.Color.menuChipShared)
                    .frame(maxHeight: .infinity, alignment: .top)
                chip(keptChip, GlassTokens.Color.menuChipKept)
                    .frame(maxHeight: .infinity, alignment: .bottom)
            }
            .frame(height: Self.plotHeight)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(summary)
            HStack {
                Text(leading)
                Spacer(minLength: 0)
                Text(trailing)
            }
            .glassType(GlassTokens.TypeScale.caption)
            .foregroundStyle(GlassColor.textTertiary)
        }
        .padding(GlassTokens.Space.s5)
        .glassTier(.card)
    }

    private func chip(_ text: String, _ fill: GlassRGBA) -> some View {
        Text(text)
            .glassType(GlassTokens.TypeScale.caption.weight(.semibold))
            .foregroundStyle(GlassColor.textPrimary)
            .padding(.horizontal, GlassTokens.Space.s4)
            .padding(.vertical, GlassTokens.Space.s1)
            .background(RoundedRectangle(cornerRadius: 6, style: .continuous).fill(fill.color))
    }
}

// MARK: - Menu-bar strip

/// The status item: the last seven columns as a mini strip (2.5pt bars,
/// heights halved), with a badge counting decisions owed over its
/// bottom-right corner. Hidden badge at zero or unknown; grey while paused.
public struct GlassMenuBarStrip: View {
    private let columns: [GlassDayColumn]
    private let paused: Bool
    private let badge: Int?

    public init(columns: [GlassDayColumn], paused: Bool, badge: Int?) {
        self.columns = Array(columns.suffix(7))
        self.paused = paused
        self.badge = badge
    }

    /// A strip bar: 2pt at least, 9pt at most, for a column's count.
    static func height(_ count: Int, max: Int) -> CGFloat {
        guard max > 0 else { return 2 }
        return 2 + 7 * CGFloat(count) / CGFloat(max)
    }

    public var body: some View {
        let maximum = columns.map { Swift.max($0.up, $0.down) }.max() ?? 0
        HStack(spacing: 2) {
            ForEach(columns) { column in
                VStack(spacing: 0) {
                    RoundedRectangle(cornerRadius: 1)
                        .fill(paused ? GlassTokens.Color.menuBarsPaused.color : GlassTokens.Color.dataShared.color)
                        .frame(width: 2.5, height: Self.height(column.up, max: maximum))
                    RoundedRectangle(cornerRadius: 1)
                        .fill(paused ? GlassTokens.Color.menuBarsPaused.color : GlassTokens.Color.dataKept.color)
                        .frame(width: 2.5, height: Self.height(column.down, max: maximum))
                }
                .frame(height: 18, alignment: .center)
            }
        }
        .padding(.horizontal, 4)
        .frame(height: 22)
        .overlay(alignment: .bottomTrailing) {
            if let badge, badge > 0 {
                Text("\(badge)")
                    .glassGlyph(10, weight: .bold)
                    .foregroundStyle(Color.white)
                    .padding(.horizontal, 4)
                    .frame(minWidth: 16, minHeight: 16)
                    .background(Capsule().fill(GlassTokens.Color.menuModeNever.color))
                    .overlay(Capsule().stroke(GlassTokens.Color.menuBadgeEdge.color, lineWidth: 1.5))
                    .offset(x: 6, y: 2)
            }
        }
    }
}

// MARK: - Activity row

/// A recent-activity row: an 18pt tool tile with its logo, the text, and an
/// optional trailing word in the no-proof colour. Hover is the macOS menu
/// selection: blue fill, white text.
public struct GlassActivityRow: View {
    private let tool: GlassTool?
    private let text: String
    private let trailing: String?
    private let action: () -> Void
    @State private var hovering = false

    public init(tool: GlassTool?, text: String, trailing: String? = nil, action: @escaping () -> Void) {
        self.tool = tool
        self.text = text
        self.trailing = trailing
        self.action = action
    }

    public var body: some View {
        Button(action: action) {
            HStack(spacing: GlassTokens.Space.s5) {
                ZStack {
                    RoundedRectangle(cornerRadius: 5, style: .continuous).fill(Color.white.opacity(0.12))
                    if let logo = tool?.logo, let tool {
                        GlassToolLogoShape(logo).fill(hovering ? Color.white : tool.tint.color).frame(width: 12, height: 12)
                    } else if let tool {
                        Text(tool.initials).glassGlyph(8, weight: .bold).foregroundStyle(tool.tint.color)
                    }
                }
                .frame(width: 18, height: 18)
                .accessibilityHidden(true)
                Text(text)
                    .glassType(GlassTokens.TypeScale.body)
                    .foregroundStyle(hovering ? Color.white : GlassColor.textPrimary)
                    .lineLimit(1)
                Spacer(minLength: GlassTokens.Space.s4)
                if let trailing {
                    Text(trailing)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(hovering ? Color.white : GlassTokens.Color.menuNoProof.color)
                }
            }
            .padding(.vertical, 5)
            .padding(.horizontal, GlassTokens.Space.s4)
            .background(
                RoundedRectangle(cornerRadius: 6, style: .continuous)
                    .fill(hovering ? GlassTokens.Color.blue.color : .clear)
                    .glassPressedFill()
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(GlassPressStyle())
        .onHover { hovering = $0 }
    }
}
