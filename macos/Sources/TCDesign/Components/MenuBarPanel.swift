import SwiftUI

// The menu-bar popover's parts (R13 of #1173), from the "Menu bar item and
// popover" handoff: the state pills, the sub-list, the day graph, the
// menu-bar item and the activity rows. Every word is the caller's.

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
            HStack(spacing: GlassTokens.Space.s5) {
                Image(systemName: "chevron.left")
                    .glassGlyph(14, weight: .medium)
                    .foregroundStyle(GlassColor.textSecondary)
                Text(title)
                    .glassType(GlassTokens.TypeScale.title.weight(.bold))
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
                    .fill(hovering && isEnabled ? GlassColor.ink(0.1) : .clear)
                    .glassPressedFill()
            )
            .contentShape(Rectangle())
        }
        // Disabled, the row dims by the shared disabled opacity, in
        // GlassPressStyle, and by nothing more.
        .buttonStyle(GlassPressStyle())
        .onHover { hovering = $0 }
        .accessibilityAddTraits(checked ? .isSelected : [])
    }
}

// MARK: - Day graph

/// One column of `GlassDayGraph`, or one step of a pair of
/// `GlassMenuBarStrip`'s lines.
public struct GlassDayColumn: Identifiable, Sendable, Equatable {
    public let id: String
    /// Rises from the centre line: shared, or in the strip's Private AI
    /// pair, verified calls.
    public let up: Int
    /// Falls below it: kept, or in the Private AI pair, all calls.
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
                    Rectangle().fill(GlassColor.ink(0.14)).frame(height: 1)
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

/// The status item (owner, 2026-10-21): the NEAR AI mark, then the last 24
/// hours as bars on a centre line like the popover's graph, both centred in
/// the menu bar's height and drawn on the bar itself with nothing behind
/// them. Each step is a pair of bars side by side: Private AI on the left,
/// verified calls (green) up and all calls (amber) down; traces on the
/// right, shared (purple) up and kept (blue) down. A badge counting
/// decisions owed, 99+ past 99, sits against the bars' bottom-right
/// corner, clear of the newest step; hidden at zero or unknown.
///
/// The mark and the bars take the menu bar's appearance from
/// `colorScheme`: the logo kit's white mark on a dark bar, its black mark on
/// a light one. Each colour is chosen per scheme here rather than resolved
/// by the system, so a caller can draw both and pick by the bar.
public struct GlassMenuBarStrip: View {
    /// What the strip can say about the data behind it. A strip never
    /// draws a dead or unknown core as healthy.
    public enum Condition: Equatable, Sendable {
        /// Current data, watching.
        case live
        /// Current data, watching paused: grey trace bars.
        case paused
        /// The core reports trouble, or its decision count is unknown: the
        /// bars as recorded, with the attention mark.
        case attention
        /// The core is down or not running, or the strip's data could not
        /// be read: no bars are drawn from it, no badge, and the attention
        /// mark.
        case unavailable
    }

    /// The re-engagement mark the core lit. It never replaces or hides a
    /// higher state: the news ring sits in the badge's slot only while that
    /// slot is empty on a live strip, and the halo only rings a badge that
    /// is drawn.
    public enum Mark: Equatable, Sendable {
        case none
        /// Something new to look at: a small hollow ring.
        case news
        /// Some decisions owed are idle sessions: a halo around the badge.
        case ready
    }

    /// One step's two bars, each segment in points from the centre line
    /// outward: the Private AI bar's verified (up) and calls (down), the
    /// trace bar's shared (up) and kept (down).
    struct Step: Equatable {
        var shared: CGFloat = 0
        var verified: CGFloat = 0
        var kept: CGFloat = 0
        var calls: CGFloat = 0
    }

    private let traces: [GlassDayColumn]
    private let privateAI: [GlassDayColumn]?
    private let condition: Condition
    private let badge: Int?
    private let mark: Mark
    @Environment(\.colorScheme) private var colorScheme

    /// `traces` is shared (up) over kept (down) per step, oldest first.
    /// `privateAI` is verified calls (up) over all calls (down) per step, or
    /// nil while unknown, which draws no Private AI segments rather than
    /// empty ones. A zero draws no segment either, as a trace zero does, so
    /// known steps with no calls and unknown ones draw the same; the
    /// popover has the counts.
    public init(
        traces: [GlassDayColumn], privateAI: [GlassDayColumn]?, condition: Condition, badge: Int?,
        mark: Mark = .none
    ) {
        self.traces = traces
        self.privateAI = privateAI
        self.condition = condition
        self.badge = badge
        self.mark = mark
    }

    /// The mark drawn: the ring only with no badge on a live strip (no
    /// attention mark, not paused, not unavailable), the halo only around
    /// a badge that is drawn.
    nonisolated public static func shownMark(_ mark: Mark, badge: Int?, condition: Condition) -> Mark {
        let badgeShown = shownBadge(badge, condition: condition) != nil
        switch mark {
        case .news: return !badgeShown && condition == .live ? .news : .none
        case .ready: return badgeShown ? .ready : .none
        case .none: return .none
        }
    }

    /// Each step's bars. Each kind of bar has its own scale, its busiest
    /// step reaching `reach`: traces their busiest count, Private AI its
    /// busiest call count, so verified never outreaches the calls it is
    /// counted among. Unavailable is flat: data that could not be read is
    /// never drawn as activity.
    static func steps(
        traces: [GlassDayColumn], privateAI: [GlassDayColumn]?, condition: Condition
    ) -> [Step] {
        guard condition != .unavailable else { return traces.map { _ in Step() } }
        let ai = shownPrivateAI(privateAI, condition: condition) ?? []
        let traceMax = traces.map { Swift.max($0.up, $0.down) }.max() ?? 0
        let aiMax = ai.map { Swift.max($0.up, $0.down) }.max() ?? 0
        return traces.enumerated().map { index, column in
            let calls = index < ai.count ? ai[index] : nil
            return Step(
                shared: offset(column.up, max: traceMax), verified: offset(calls?.up ?? 0, max: aiMax),
                kept: offset(column.down, max: traceMax), calls: offset(calls?.down ?? 0, max: aiMax))
        }
    }

    /// A segment's reach for a count: nothing at zero, then at least
    /// `least`, up to `reach` for the busiest step. Whole points, so the
    /// segment's end is crisp on a 1x display.
    static func offset(_ count: Int, max: Int) -> CGFloat {
        guard count > 0, max > 0 else { return 0 }
        return Swift.max(least, (reach * CGFloat(count) / CGFloat(max)).rounded())
    }

    /// The Private AI steps drawn: none while unknown or unavailable.
    static func shownPrivateAI(_ privateAI: [GlassDayColumn]?, condition: Condition) -> [GlassDayColumn]? {
        condition == .unavailable ? nil : privateAI
    }

    /// The badge drawn, if any: none when unavailable, since an unknown
    /// count is never a number.
    nonisolated static func shownBadge(_ badge: Int?, condition: Condition) -> Int? {
        guard condition != .unavailable, let badge, badge > 0 else { return nil }
        return badge
    }

    /// The badge's figure: two digits at most, 99+ past them (owner,
    /// 2026-10-21). The item's accessibility label still says the count.
    static func badgeText(_ badge: Int) -> String {
        badge > 99 ? "99+" : String(badge)
    }

    /// Whether the attention mark is drawn.
    static func showsAttention(_ condition: Condition) -> Bool {
        condition == .attention || condition == .unavailable
    }

    /// Whether the trace segments are drawn grey. Pausing stops watching
    /// for traces, not Private AI, so its segments keep their colours.
    static func grey(_ condition: Condition) -> Bool {
        condition == .paused || condition == .unavailable
    }

    /// The badge's height (and least width), its figure's size and inset,
    /// how far it sits above the item's bottom edge (room for the halo),
    /// and how far it reaches back over the bars: not at all, since the
    /// newest step's bars are the last ones. Sized to other menu-bar items'
    /// badges (owner, 2026-10-21).
    static let badgeSize: CGFloat = 11
    static let badgeGlyph: CGFloat = 7
    static let badgeInset: CGFloat = 2.5
    static let badgeLift: CGFloat = haloReach
    static let badgeOverlap: CGFloat = 0
    /// How far the halo reaches past the badge's edge: its inset and half
    /// its line. The badge sits this far above the item's bottom edge and
    /// this far after the bars while the halo is drawn, and the image is
    /// padded this far past it, so the halo is never cut off and never
    /// covers the newest step.
    public static let haloReach: CGFloat = haloInset + haloLine / 2
    static let haloInset: CGFloat = 2.5
    static let haloLine: CGFloat = 2

    /// The item's height, the mark's side and the gap after it.
    static let height: CGFloat = 22
    static let markSize: CGFloat = 14
    static let markGap: CGFloat = 4
    /// A bar's width; a step's two bars touch, and steps are `stepGap`
    /// apart. Whole points, so a 1x display draws them crisp too.
    nonisolated static let barWidth: CGFloat = 1
    static let stepGap: CGFloat = 1
    /// The graph's height and its centre line's, and its top edge: centred
    /// in the item's height, as the mark is, but half a point high: a 1pt
    /// line cannot be centred exactly in 22pt, and on a half point it and
    /// the bars' ends draw blurred on a 1x display.
    static let graphHeight: CGFloat = 17
    static let centreLine: CGFloat = 1
    static let graphTop: CGFloat = ((height - graphHeight) / 2).rounded(.down)
    /// The furthest a bar reaches from the centre line.
    static let reach: CGFloat = (graphHeight - centreLine) / 2
    /// The least a segment with any count reaches, so one trace beside a
    /// busy step still shows.
    static let least: CGFloat = 1

    /// The graph's width for `count` steps of two bars.
    static func graphWidth(steps count: Int) -> CGFloat {
        guard count > 0 else { return 0 }
        return CGFloat(count) * 2 * barWidth + CGFloat(count - 1) * stepGap
    }

    /// A token in the bar's scheme, never left for the system to resolve.
    private func ink(_ rgba: GlassRGBA) -> GlassRGBA {
        colorScheme == .dark ? rgba.dark : rgba.light
    }

    public var body: some View {
        let steps = Self.steps(traces: traces, privateAI: privateAI, condition: condition)
        let grey = Self.grey(condition)
        let markInk = ink(GlassTokens.Color.menuItemMark)
        let paused = markInk.opacity(0.45)
        let shared = grey ? paused : ink(GlassTokens.Color.menuStripShared)
        let kept = grey ? paused : ink(GlassTokens.Color.menuStripKept)
        let verified = ink(GlassTokens.Color.menuStripVerified)
        let calls = ink(GlassTokens.Color.menuStripCalls)
        let width = Self.markSize + Self.markGap + Self.graphWidth(steps: steps.count)
        let halo = Self.shownMark(mark, badge: badge, condition: condition) == .ready
        HStack(alignment: .bottom, spacing: halo ? Self.haloReach : -Self.badgeOverlap) {
            Canvas { context, size in
                if let logo = GlassBrandMark.path(
                    at: CGPoint(x: 0, y: (size.height - Self.markSize) / 2), size: Self.markSize)
                {
                    context.fill(logo, with: .color(markInk.color))
                }
                let left = Self.markSize + Self.markGap
                let above = Self.graphTop + Self.reach
                let below = above + Self.centreLine
                context.fill(
                    Path(CGRect(x: left, y: above, width: Self.graphWidth(steps: steps.count), height: Self.centreLine)),
                    with: .color(markInk.opacity(0.3).color))
                func segment(_ x: CGFloat, from edge: CGFloat, by length: CGFloat, up: Bool, _ colour: GlassRGBA) {
                    guard length > 0 else { return }
                    let rect = CGRect(x: x, y: up ? edge - length : edge, width: Self.barWidth, height: length)
                    context.fill(Path(rect), with: .color(colour.color))
                }
                for (index, step) in steps.enumerated() {
                    let x = left + CGFloat(index) * (2 * Self.barWidth + Self.stepGap)
                    segment(x, from: above, by: step.verified, up: true, verified)
                    segment(x, from: below, by: step.calls, up: false, calls)
                    segment(x + Self.barWidth, from: above, by: step.shared, up: true, shared)
                    segment(x + Self.barWidth, from: below, by: step.kept, up: false, kept)
                }
            }
            .frame(width: width, height: Self.height)
            slot
        }
    }

    /// What sits in the badge's slot past the bars: the count, the attention
    /// mark, the news ring, or nothing.
    @ViewBuilder
    private var slot: some View {
        let shownMark = Self.shownMark(mark, badge: badge, condition: condition)
        if let badge = Self.shownBadge(badge, condition: condition) {
            // Smaller, with no edge (owner, 2026-10-11).
            Text(Self.badgeText(badge))
                .glassGlyph(Self.badgeGlyph, weight: .bold)
                .foregroundStyle(Color.white)
                .padding(.horizontal, Self.badgeInset)
                .frame(minWidth: Self.badgeSize, minHeight: Self.badgeSize)
                .background(Capsule().fill(GlassTokens.Color.menuModeNever.color))
                // The halo: the accent line colour, outside the badge's
                // own edge, as `GlassStatusDot`'s ring sits outside it.
                .overlay {
                    if shownMark == .ready {
                        Capsule().stroke(ink(GlassTokens.Color.purpleText).color, lineWidth: Self.haloLine)
                            .padding(-Self.haloInset)
                    }
                }
                .padding(.bottom, Self.badgeLift)
        } else if Self.showsAttention(condition) {
            // The shipping mark's attention state, in the badge's place.
            Text("!")
                .glassGlyph(Self.badgeGlyph, weight: .bold)
                .foregroundStyle(Color.white)
                .frame(minWidth: Self.badgeSize, minHeight: Self.badgeSize)
                .background(Capsule().fill(GlassTokens.Color.menuModeNever.color))
                .padding(.bottom, Self.badgeLift)
        } else if shownMark == .news {
            // The news ring, hollow, in the badge's slot.
            Circle()
                .stroke(ink(GlassTokens.Color.purpleText).color, lineWidth: 1.5)
                .frame(width: 9, height: 9)
                .frame(width: 16, height: 16)
                .padding(.bottom, Self.badgeLift)
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
                    RoundedRectangle(cornerRadius: 5, style: .continuous).fill(GlassColor.ink(0.12))
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
