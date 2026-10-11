import SwiftUI

/// A badge's value: a count, or unknown (a dash).
public enum GlassBadgeValue: Sendable, Equatable {
    case count(Int)
    case unknown
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
                        if let dot = segment.dot {
                            GlassStatusDot(dot, size: 6)
                        }
                        // With a text equivalent, the badge is not read on
                        // its own: the segment's value says it in words.
                        Group {
                            switch segment.badge {
                            case .count(let count): GlassBadge(count: count, subtle: true)
                            case .unknown: GlassBadge(count: nil, subtle: true)
                            case nil: EmptyView()
                            }
                        }
                        .accessibilityHidden(segment.accessibilityValue != nil)
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
                        // The press darkens the selected fill, or a wash
                        // behind an unselected label; never the label.
                        if selected {
                            if floating {
                                // #1146's floating selection: a brighter fill, no edge.
                                Capsule().fill(GlassTokens.Color.controlSelectedFloating.color).glassPressedFill()
                            } else {
                                Capsule().fill(GlassTokens.Color.controlSelected.color)
                                    .glassPressedFill()
                                    .glassEdge(GlassTokens.Shadow.controlSelectedEdge, in: Capsule())
                            }
                        }
                    }
                    .glassPressedWash(Capsule())
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
public struct GlassRuleTabs<Value: Hashable>: View {
    private let label: String
    private let segments: [GlassSegment<Value>]
    @Binding private var selection: Value
    /// How far the rule runs past the tabs on each side, so it reaches the
    /// pane's edges from inside its padding. `toPaneEdge` outruns any pane
    /// inset; the pane clips the rule, and the ember, at its own edge.
    private let bleed: CGFloat
    @State private var hovered: Value?
    @Namespace private var ember
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    public init(_ label: String, selection: Binding<Value>, segments: [GlassSegment<Value>], bleed: CGFloat = 0) {
        self.label = label
        self._selection = selection
        self.segments = segments
        self.bleed = bleed
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
                    tab(segment)
                }
            }
        }
        .animation(reduceMotion ? nil : .spring(response: 0.35, dampingFraction: 0.85), value: selection)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(label)
    }

    private func tab(_ segment: GlassSegment<Value>) -> some View {
        let selected = segment.value == selection
        return Button {
            selection = segment.value
        } label: {
            HStack(spacing: GlassTokens.Space.inlineGap) {
                Text(segment.title)
                if let dot = segment.dot {
                    GlassStatusDot(dot, size: 6)
                }
                // With a text equivalent, the badge is not read on its own:
                // the segment's value says it in words.
                Group {
                    switch segment.badge {
                    case .count(let count): GlassBadge(count: count, subtle: true)
                    case .unknown: GlassBadge(count: nil, subtle: true)
                    case nil: EmptyView()
                    }
                }
                .accessibilityHidden(segment.accessibilityValue != nil)
            }
            .glassType(GlassTokens.TypeScale.body.weight(selected ? .semibold : .medium))
            .foregroundStyle(Self.ink(selected: selected, hovering: hovered == segment.value))
            .frame(maxWidth: .infinity)
            .frame(minHeight: GlassTokens.Size.segmentedTrack + GlassTokens.Space.s4)
            .background(alignment: .bottom) {
                if selected { emberGlow.matchedGeometryEffect(id: "ember", in: ember) }
            }
            // The press is a wash behind the title and the ember, never the
            // title, as on the segmented tabs.
            .glassPressedWash(RoundedRectangle(cornerRadius: GlassTokens.Radius.control, style: .continuous))
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

/// A back chevron, then crumbs: secondary › tertiary › current.
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
                    Text("›").foregroundStyle(GlassColor.textTertiary).accessibilityHidden(true)
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
                    Circle()
                        .fill(index <= current ? GlassTokens.Color.purpleSoft.color : GlassTokens.Color.wellFill.color)
                        .frame(width: 14, height: 14)
                        .overlay {
                            if index <= current {
                                Circle().stroke(GlassTokens.Color.purpleSoft.color.opacity(0.25), lineWidth: 3).padding(-1.5)
                            }
                        }
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
