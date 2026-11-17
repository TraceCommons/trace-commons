import SwiftUI

/// A badge's value: a count, unknown (a dash), or still loading (a spinner,
/// until the first answer; never a dash, which says the core did not give one).
public enum GlassBadgeValue: Sendable, Equatable {
    case count(Int)
    case unknown
    case loading
}

/// One segment of `GlassSegmentedTabs`.
public struct GlassSegment<Value: Hashable>: Identifiable {
    public let value: Value
    public let title: String
    /// Decisions owed, as a count pill. Never queue depth or credit.
    public let badge: GlassBadgeValue?
    /// A status dot after the title (Inference: whether Private AI is
    /// answering). Colour alone, so pair it with `accessibilityValue`.
    public let dot: GlassStatus?
    /// What the dot and the badge say, in words, for VoiceOver: the dot is
    /// hidden from assistive tech, so a tab with one must carry its text
    /// equivalent here, from the core's copy.
    public let accessibilityValue: String?

    public var id: Value { value }

    public init(_ title: String, value: Value, badge: Int? = nil, dot: GlassStatus? = nil, accessibilityValue: String? = nil) {
        self.init(title, value: value, badgeValue: badge.map(GlassBadgeValue.count), dot: dot,
                  accessibilityValue: accessibilityValue)
    }

    /// `badgeValue: .unknown` draws a dash: a count the core did not give is
    /// never shown as a number. Pass nil, or `.count(0)`, for no badge.
    public init(
        _ title: String, value: Value, badgeValue: GlassBadgeValue?, dot: GlassStatus? = nil,
        accessibilityValue: String? = nil
    ) {
        self.title = title
        self.value = value
        self.badge = badgeValue == .count(0) ? nil : badgeValue
        self.dot = dot
        self.accessibilityValue = accessibilityValue
    }

    /// What the segment draws after its title (`GlassSegmentMark`).
    public var mark: GlassSegmentMark? {
        if let badge { return .count(badge, status: dot) }
        return dot.map(GlassSegmentMark.dot)
    }
}

/// What a tab draws after its title: one element, never a dot beside a
/// count (owner, 2026-10-16). A tab with a count draws it in its dot's tint
/// (`GlassBadge.tinted(_:)`); one with only a dot draws the dot.
public enum GlassSegmentMark: Equatable, Sendable {
    case dot(GlassStatus)
    case count(GlassBadgeValue, status: GlassStatus?)
}

/// A segment's mark, drawn. With a text equivalent, a count is not read on
/// its own: the segment's value says it, and its state, in words. The dot
/// is never read.
private struct GlassSegmentMarkView: View {
    let mark: GlassSegmentMark?
    let hidesCount: Bool

    var body: some View {
        switch mark {
        case .dot(let status):
            GlassStatusDot(status, size: 6)
        case .count(.count(let count), let status):
            GlassBadge(count: count, subtle: true, status: status).accessibilityHidden(hidesCount)
        case .count(.unknown, let status):
            GlassBadge(count: nil, subtle: true, status: status).accessibilityHidden(hidesCount)
        case .count(.loading, _):
            GlassSpinner(size: 10)
        case nil:
            EmptyView()
        }
    }
}

/// Segmented tabs in a well: Home · Inference · Traces. `floating` is the
/// variant that sits on the map (Traces · Private AI).
public struct GlassSegmentedTabs<Value: Hashable>: View {
    private let label: String
    private let segments: [GlassSegment<Value>]
    @Binding private var selection: Value
    private let floating: Bool
    @State private var hovered: Value?

    public init(_ label: String, selection: Binding<Value>, segments: [GlassSegment<Value>], floating: Bool = false) {
        self.label = label
        self._selection = selection
        self.segments = segments
        self.floating = floating
    }

    public var body: some View {
        HStack(spacing: 2) {
            ForEach(segments) { segment in
                let selected = segment.value == selection
                Button {
                    selection = segment.value
                } label: {
                    HStack(spacing: GlassTokens.Space.inlineGap) {
                        Text(segment.title)
                        GlassSegmentMarkView(mark: segment.mark, hidesCount: segment.accessibilityValue != nil)
                    }
                    .glassType(GlassTokens.TypeScale.label.weight(selected ? .semibold : .medium))
                    .foregroundStyle(Self.ink(selected: selected, hovering: hovered == segment.value, floating: floating))
                    .padding(.horizontal, floating ? 12 : 8)
                    // The floating item is its label and 5pt above and
                    // below (#1146 `.tc-segmented--floating` item).
                    .padding(.vertical, floating ? 5 : 0)
                    .frame(maxWidth: floating ? nil : .infinity)
                    .frame(minHeight: floating ? nil : GlassTokens.Size.tab)
                    .background {
                        // No press state (owner, 2026-11-02: no click fill
                        // on any tab): the selection moving is the answer.
                        if selected {
                            if floating {
                                // #1146's floating selection: a brighter fill, no edge.
                                Capsule().fill(GlassTokens.Color.controlSelectedFloating.color)
                            } else {
                                Capsule().fill(GlassTokens.Color.controlSelected.color)
                                    .glassEdge(GlassTokens.Shadow.controlSelectedEdge, in: Capsule())
                            }
                        }
                    }
                    .contentShape(Capsule())
                }
                .buttonStyle(GlassPressStyle())
                .onHover { inside in
                    if inside {
                        hovered = segment.value
                    } else if hovered == segment.value {
                        hovered = nil
                    }
                }
                .accessibilityValue(segment.accessibilityValue ?? "")
                .accessibilityAddTraits(selected ? [.isSelected, .isButton] : .isButton)
            }
        }
        .padding(2)
        .frame(minHeight: floating ? nil : GlassTokens.Size.segmentedTrack)
        .glassSurface(floating ? .control : .well, floating: floating)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(label)
    }

    /// A segment's label ink: primary when selected or under the pointer
    /// (#1146 `.tc-segmented__item:hover`), otherwise secondary on the
    /// floating variant and tertiary in the well.
    static func ink(selected: Bool, hovering: Bool, floating: Bool) -> Color {
        if selected { return GlassTokens.Color.textOnSelected.color }
        if hovering { return GlassColor.textPrimary }
        return floating ? GlassColor.textSecondary : GlassColor.textTertiary
    }
}

/// The main window's tabs (owner choice, 2026-10-10, the Ember Rule): the
/// titles over a rule that spans the pane edge to edge, the selected one in
/// primary ink with an ember laid on the rule under it, a 2pt purple line
/// fading out 30% past the tab each side, and a faint halo rising behind
/// its title. The ember glides to a newly selected tab; under Reduce Motion
/// it moves at once. Segments, badges and dots are `GlassSegment`'s.
///
/// Given `onMove`, a movable tab can be dragged along the strip (owner,
/// 2026-10-30): it follows the pointer, and each time it passes the middle
/// of a neighbouring movable tab the two trade places (`onMove` with the
/// dragged value and the one whose place it takes). A movable tab lifts
/// under the pointer (owner, 2026-11-05; it waited for a held press
/// before): it pops out of the strip, a little larger and raised on a
/// control's surface, saying it can be picked up, and a drag moves it
/// straight away. A click opens a tab, as before; a tab let go where it
/// was opens too, and one that was moved never does. No tab draws a press
/// fill (owner, 2026-11-02).
public struct GlassRuleTabs<Value: Hashable>: View {
    private let label: String
    private let segments: [GlassSegment<Value>]
    @Binding private var selection: Value
    /// How far the rule runs past the tabs on each side, so it reaches the
    /// pane's edges from inside its padding. `toPaneEdge` outruns any pane
    /// inset; the pane clips the rule, and the ember, at its own edge.
    private let bleed: CGFloat
    private let movable: (Value) -> Bool
    private let onMove: ((Value, Value) -> Void)?
    @State private var hovered: Value?
    /// The tab lifted by a press, while the press lasts. SwiftUI
    /// clears it however the gesture ends, so a lifted tab always drops.
    @GestureState private var lifted: Value?
    /// The tab being dragged, and how far it sits from its slot.
    @State private var dragging: Value?
    /// The lifted tab has moved, so letting go does not open it.
    @State private var moved = false
    @State private var dragOffset: CGFloat = 0
    /// The dragged tab's slot when the drag began.
    @State private var dragOrigin = 0
    /// The strip's width, which with the count gives one tab's width: the
    /// tabs share it equally.
    @State private var stripWidth: CGFloat = 0
    @Namespace private var ember
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    public init(
        _ label: String, selection: Binding<Value>, segments: [GlassSegment<Value>], bleed: CGFloat = 0,
        movable: @escaping (Value) -> Bool = { _ in false }, onMove: ((Value, Value) -> Void)? = nil
    ) {
        self.label = label
        self._selection = selection
        self.segments = segments
        self.bleed = bleed
        self.movable = movable
        self.onMove = onMove
    }

    /// A bleed past any pane inset, for tabs inside a pane that clips its
    /// content (`glassTier`), so the rule meets the pane's edges whatever
    /// the inset is.
    public static var toPaneEdge: CGFloat { 48 }

    /// How far the ember line reaches past its tab, as a share of the
    /// tab's width, each side.
    static var emberReach: CGFloat { 0.3 }
    /// The ember line's thickness and the halo's height.
    static var emberLine: CGFloat { 2 }
    static var haloHeight: CGFloat { 26 }

    public var body: some View {
        ZStack(alignment: .bottom) {
            Rectangle()
                .fill(GlassTokens.Color.rule.color)
                .frame(height: 1)
                .padding(.horizontal, -bleed)
            HStack(spacing: 0) {
                ForEach(segments) { segment in
                    if onMove != nil, movable(segment.value) {
                        // Under the pointer, pressed or dragged, a movable
                        // tab is lifted (owner, 2026-11-05).
                        let up = hovered == segment.value || lifted == segment.value
                            || dragging == segment.value
                        tab(segment, lifted: up)
                            .scaleEffect(up ? Self.liftScale : 1)
                            .offset(x: dragging == segment.value ? dragOffset : 0, y: up ? -Self.liftRise : 0)
                            .zIndex(up ? 1 : 0)
                            .animation(reduceMotion ? nil : .spring(response: 0.25, dampingFraction: 0.8), value: up)
                            .simultaneousGesture(drag(segment.value))
                    } else {
                        tab(segment, lifted: false)
                    }
                }
            }
            // However the press ends, a lifted tab drops.
            .onChange(of: lifted) { _, now in
                if now == nil, dragging != nil { drop() }
            }
            .background {
                GeometryReader { proxy in
                    Color.clear
                        .onAppear { stripWidth = proxy.size.width }
                        .onChange(of: proxy.size.width) { _, width in stripWidth = width }
                }
            }
        }
        .animation(reduceMotion ? nil : .spring(response: 0.35, dampingFraction: 0.85), value: selection)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(label)
    }

    private func tab(_ segment: GlassSegment<Value>, lifted: Bool) -> some View {
        let selected = segment.value == selection
        return Button {
            // A drag ends with the pointer still on the tab; a tab that
            // moved is not opened by it. One held and let go in place is.
            guard !moved else { return }
            selection = segment.value
        } label: {
            HStack(spacing: GlassTokens.Space.inlineGap) {
                Text(segment.title)
                GlassSegmentMarkView(mark: segment.mark, hidesCount: segment.accessibilityValue != nil)
            }
            .glassType(GlassTokens.TypeScale.body.weight(selected ? .semibold : .medium))
            .foregroundStyle(Self.ink(selected: selected, hovering: hovered == segment.value))
            .frame(maxWidth: .infinity)
            .frame(minHeight: GlassTokens.Size.segmentedTrack + GlassTokens.Space.s4)
            .background(alignment: .bottom) {
                if selected { emberGlow.matchedGeometryEffect(id: "ember", in: ember) }
            }
            // Lifted, the tab stands on a control's surface with a soft
            // shadow under it, behind its title and ember.
            .background {
                if lifted {
                    Color.clear
                        .glassSurface(.control, radius: GlassTokens.Radius.control)
                        .shadow(color: .black.opacity(Self.liftShadowOpacity), radius: 8, y: 3)
                        .padding(.vertical, GlassTokens.Space.s1)
                        .allowsHitTesting(false)
                        .accessibilityHidden(true)
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(GlassPressStyle())
        .onHover { inside in
            if inside {
                hovered = segment.value
            } else if hovered == segment.value {
                hovered = nil
            }
        }
        .accessibilityValue(segment.accessibilityValue ?? "")
        .accessibilityAddTraits(selected ? [.isSelected, .isButton] : .isButton)
    }

    /// The halo behind the title and the ember line on the rule, sized to
    /// the tab they sit under.
    private var emberGlow: some View {
        GeometryReader { proxy in
            let width = proxy.size.width
            let ember = GlassTokens.Color.tabEmber.color
            let halo = GlassTokens.Color.tabEmberHalo.color
            ZStack(alignment: .bottom) {
                // Brightest at the rule under the title's middle, gone by
                // `haloHeight` up and by the tab's sides: the ellipse is
                // centred on the frame's foot and reaches half its height and
                // width, so it is clear before any edge.
                EllipticalGradient(
                    colors: [halo, halo.opacity(0)],
                    center: .bottom, startRadiusFraction: 0, endRadiusFraction: 0.5)
                    .frame(width: width * 1.1, height: Self.haloHeight * 2)
                LinearGradient(
                    colors: [ember.opacity(0), ember, ember.opacity(0)],
                    startPoint: .leading, endPoint: .trailing)
                    .frame(width: width * (1 + Self.emberReach * 2), height: Self.emberLine)
                    // Centred on the 1pt rule.
                    .offset(y: (Self.emberLine - 1) / 2)
            }
            .frame(width: width, height: proxy.size.height, alignment: .bottom)
        }
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }

    /// A title's ink: primary when selected or under the pointer, otherwise
    /// secondary (which deepens under Increase Contrast).
    static func ink(selected: Bool, hovering: Bool) -> Color {
        inksPrimary(selected: selected, hovering: hovering) ? GlassColor.textPrimary : GlassColor.textSecondary
    }

    static func inksPrimary(selected: Bool, hovering: Bool) -> Bool {
        selected || hovering
    }
}

/// Dragging a rule tab to move it (owner, 2026-10-30), lifted under the
/// pointer and moved by a plain drag (owner, 2026-11-02, 2026-11-05).
extension GlassRuleTabs {
    /// How much larger a lifted tab is, how far it rises, and its shadow.
    static var liftScale: CGFloat { 1.06 }
    static var liftRise: CGFloat { 2 }
    static var liftShadowOpacity: Double { 0.18 }
    /// How far a lifted tab travels before letting go no longer opens it.
    static var dragThreshold: CGFloat { 6 }

    /// Where a tab dragged from `from` by `offset` lands, as an index: the
    /// slot whose middle it has passed, kept to the movable tabs (`allowed`)
    /// and never past the strip's ends.
    static func landing(from: Int, offset: CGFloat, tabWidth: CGFloat, allowed: ClosedRange<Int>) -> Int {
        guard tabWidth > 0 else { return from }
        let slot = from + Int((offset / tabWidth).rounded())
        return min(max(slot, allowed.lowerBound), allowed.upperBound)
    }

    /// Whether a lifted tab has travelled far enough that letting go moves
    /// it rather than opening it.
    static func hasMoved(_ translation: CGFloat) -> Bool {
        abs(translation) >= dragThreshold
    }

    /// A press holds a movable tab lifted (owner, 2026-11-05); a drag then
    /// moves it with the pointer, and it trades places with a movable
    /// neighbour once past that neighbour's middle, keeping its place under
    /// the pointer as the strip reorders.
    private func drag(_ value: Value) -> some Gesture {
        DragGesture(minimumDistance: 0, coordinateSpace: .global)
            .updating($lifted) { _, state, _ in state = value }
            .onChanged { gesture in follow(value, translation: gesture.translation.width) }
            .onEnded { _ in drop() }
    }

    private func follow(_ value: Value, translation: CGFloat) {
        guard let onMove, let from = segments.firstIndex(where: { $0.value == value }),
              let first = segments.firstIndex(where: { movable($0.value) }),
              let last = segments.lastIndex(where: { movable($0.value) }) else { return }
        let tabWidth = stripWidth / CGFloat(segments.count)
        if dragging != value {
            dragging = value
            dragOrigin = from
        }
        if Self.hasMoved(translation) { moved = true }
        // The pointer's travel is from the slot the drag began in; the tab
        // sits that far from it, so its offset from the slot it holds now
        // is the travel less the slots it moved.
        let offset = translation - CGFloat(from - dragOrigin) * tabWidth
        let to = Self.landing(from: from, offset: offset, tabWidth: tabWidth, allowed: first...last)
        if to != from {
            onMove(value, segments[to].value)
        }
        dragOffset = offset - CGFloat(to - from) * tabWidth
    }

    /// Lets the tab down into its slot. Twice is harmless: the gesture's end
    /// and the lift clearing both call it.
    private func drop() {
        withAnimation(reduceMotion ? nil : .spring(response: 0.3, dampingFraction: 0.85)) {
            dragOffset = 0
        }
        // Cleared after the click the release delivers.
        DispatchQueue.main.async {
            dragging = nil
            moved = false
        }
    }
}

/// A crumb in `GlassBreadcrumb`.
public struct GlassCrumb: Identifiable {
    public let title: String
    public let action: (() -> Void)?
    public var id: String { title }

    public init(_ title: String, action: (() -> Void)? = nil) {
        self.title = title
        self.action = action
    }
}

/// A back chevron, then crumbs: secondary / tertiary / current.
public struct GlassBreadcrumb: View {
    private let trail: [GlassCrumb]
    private let backLabel: String
    private let onBack: (() -> Void)?
    @State private var hovered: Int?
    /// `hovered` for the back chevron, which is not a crumb.
    private static var backIndex: Int { -1 }

    /// `backLabel` names the back button (it is icon-only), from the core's
    /// copy.
    public init(_ trail: [GlassCrumb], backLabel: String = "", onBack: (() -> Void)? = nil) {
        self.trail = trail
        self.backLabel = backLabel
        self.onBack = onBack
    }

    public var body: some View {
        HStack(spacing: GlassTokens.Space.s4) {
            if let onBack {
                // The chevron alone, no round container (owner,
                // 2026-10-08); it reads primary under the pointer, as the
                // crumbs do, and keeps a control-sized target.
                let label = backLabel.isEmpty ? (trail.first?.title ?? "") : backLabel
                Button(action: onBack) {
                    Image(systemName: "chevron.left")
                        .glassGlyph(12, weight: .semibold)
                        .frame(width: GlassTokens.Size.control, height: GlassTokens.Size.control)
                        .contentShape(Rectangle())
                }
                .buttonStyle(GlassPressStyle())
                .foregroundStyle(hovered == Self.backIndex ? GlassColor.textPrimary : GlassColor.textSecondary)
                .onHover { inside in
                    if inside {
                        hovered = Self.backIndex
                    } else if hovered == Self.backIndex {
                        hovered = nil
                    }
                }
                .accessibilityLabel(label)
                .help(label)
                // The chevron's own inset stands in for the container's
                // edge, so the first crumb keeps its place.
                .padding(.trailing, -GlassTokens.Space.s4)
            }
            ForEach(Array(trail.enumerated()), id: \.element.id) { index, crumb in
                if index > 0 {
                    Text("/").foregroundStyle(GlassColor.textTertiary).accessibilityHidden(true)
                }
                if index == trail.count - 1 {
                    Text(crumb.title).foregroundStyle(GlassColor.textPrimary)
                        .accessibilityAddTraits(.isHeader)
                } else {
                    // A text link: no fill, so the text takes the press.
                    // Under the pointer it reads primary (#1146
                    // `.tc-breadcrumb__crumb:hover`).
                    Button { crumb.action?() } label: { Text(crumb.title).glassPressedFill() }
                        .buttonStyle(GlassPressStyle())
                        .foregroundStyle(hovered == index ? GlassColor.textPrimary : GlassColor.textSecondary)
                        .onHover { inside in
                            if inside {
                                hovered = index
                            } else if hovered == index {
                                hovered = nil
                            }
                        }
                }
            }
        }
        .glassType(GlassTokens.TypeScale.label.weight(.semibold))
    }
}

/// Sequential glass nodes joined by lines, for the first run. Not a tab
/// control: the steps are not chosen, they are reached.
public struct GlassStepProgress: View {
    /// What VoiceOver says for a step that is behind, at or ahead of the
    /// current one, from the core's copy.
    public struct StateValues: Sendable, Equatable {
        public let done: String
        public let current: String
        public let pending: String

        public init(done: String, current: String, pending: String) {
            self.done = done
            self.current = current
            self.pending = pending
        }
    }

    private let labels: [String]
    private let current: Int
    private let stateValues: StateValues?

    /// With no `stateValues` a step speaks only its label, and the current
    /// step is marked selected; the component authors no state words.
    public init(labels: [String], current: Int, stateValues: StateValues? = nil) {
        self.labels = labels
        self.current = current
        self.stateValues = stateValues
    }

    /// The step dot's side, and its halo's: 3pt of ring all round.
    static let dotSide: CGFloat = 14
    static let haloSide: CGFloat = 20

    /// The accessibility value of the step at `index`.
    func value(at index: Int) -> String {
        guard let stateValues else { return "" }
        return index < current ? stateValues.done : index == current ? stateValues.current : stateValues.pending
    }

    public var body: some View {
        HStack(alignment: .top, spacing: 0) {
            ForEach(Array(labels.enumerated()), id: \.offset) { index, label in
                if index > 0 {
                    Capsule()
                        .fill(index <= current ? GlassTokens.Color.purpleSoft.color : GlassColor.ink(0.14))
                        .frame(height: 2)
                        .padding(.horizontal, 6)
                        .padding(.top, 6)
                        .accessibilityHidden(true)
                }
                VStack(spacing: GlassTokens.Space.s3) {
                    // The halo is a filled 20pt disc behind the 14pt dot, both
                    // centred by the ZStack, so the ring is concentric at any
                    // scale; a stroke inset by a negative padding was not
                    // (owner, 2026-10-12). The 14pt frame keeps the layout:
                    // the halo overflows it by 3pt on each side.
                    ZStack {
                        if index <= current {
                            Circle()
                                .fill(GlassTokens.Color.purpleSoft.color.opacity(0.25))
                                .frame(width: Self.haloSide, height: Self.haloSide)
                        }
                        Circle()
                            .fill(index <= current ? GlassTokens.Color.purpleSoft.color : GlassTokens.Color.wellFill.color)
                            .frame(width: Self.dotSide, height: Self.dotSide)
                    }
                    .frame(width: Self.dotSide, height: Self.dotSide)
                    Text(label)
                        // #1146's step label tracks 0.06em, tighter than an eyebrow.
                        .glassType(GlassTokens.TypeScale.eyebrow.tracking(GlassTokens.TypeScale.eyebrow.size * 0.06))
                        .foregroundStyle(index == current ? GlassColor.textPrimary : GlassColor.textTertiary)
                }
                .fixedSize()
                .accessibilityElement(children: .combine)
                .accessibilityValue(value(at: index))
                .accessibilityAddTraits(index == current ? .isSelected : [])
            }
        }
    }
}
