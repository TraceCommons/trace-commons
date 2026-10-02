import SwiftUI

/// One segment of `GlassSegmentedTabs`.
public struct GlassSegment<Value: Hashable>: Identifiable {
    public let value: Value
    public let title: String
    /// Decisions owed, as a count pill. Never queue depth or credit.
    public let badge: Int?
    /// A status dot after the title (Inference: Private AI on or off).
    public let dot: GlassStatus?

    public var id: Value { value }

    public init(_ title: String, value: Value, badge: Int? = nil, dot: GlassStatus? = nil) {
        self.title = title
        self.value = value
        self.badge = badge
        self.dot = dot
    }
}

/// Segmented tabs in a well: Home · Inference · Traces. `floating` is the
/// variant that sits on the map (Traces · Private AI).
public struct GlassSegmentedTabs<Value: Hashable>: View {
    private let label: String
    private let segments: [GlassSegment<Value>]
    @Binding private var selection: Value
    private let floating: Bool

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
                        if let badge = segment.badge {
                            GlassBadge(count: badge, subtle: true)
                        }
                    }
                    .glassType(GlassTokens.TypeScale.label.weight(selected ? .semibold : .medium))
                    .foregroundStyle(selected ? GlassColor.textPrimary : (floating ? GlassColor.textSecondary : GlassColor.textTertiary))
                    .padding(.horizontal, floating ? 12 : 8)
                    .frame(maxWidth: floating ? nil : .infinity)
                    .frame(minHeight: floating ? 24 : GlassTokens.Size.tab)
                    .background {
                        if selected {
                            if floating {
                                Capsule().fill(Color.white.opacity(0.18))
                            } else {
                                Capsule().fill(GlassTokens.Color.controlSelected.color)
                                    .glassEdge(GlassTokens.Shadow.controlSelectedEdge, in: Capsule())
                            }
                        }
                    }
                    .contentShape(Capsule())
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(selected ? [.isSelected, .isButton] : .isButton)
            }
        }
        .padding(2)
        .frame(minHeight: floating ? nil : GlassTokens.Size.segmentedTrack)
        .glassTier(floating ? .control : .well)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(label)
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

/// A round back button, then crumbs: secondary › tertiary › current.
public struct GlassBreadcrumb: View {
    private let trail: [GlassCrumb]
    private let backLabel: String
    private let onBack: (() -> Void)?

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
                GlassRoundButton(backLabel.isEmpty ? (trail.first?.title ?? "") : backLabel, systemImage: "chevron.left", small: true, action: onBack)
            }
            ForEach(Array(trail.enumerated()), id: \.element.id) { index, crumb in
                if index > 0 {
                    Text("›").foregroundStyle(GlassColor.textTertiary).accessibilityHidden(true)
                }
                if index == trail.count - 1 {
                    Text(crumb.title).foregroundStyle(GlassColor.textPrimary)
                        .accessibilityAddTraits(.isHeader)
                } else {
                    Button(crumb.title) { crumb.action?() }
                        .buttonStyle(.plain)
                        .foregroundStyle(GlassColor.textSecondary)
                }
            }
        }
        .glassType(GlassTokens.TypeScale.label.weight(.semibold))
    }
}

/// Sequential glass nodes joined by lines, for the first run. Not a tab
/// control: the steps are not chosen, they are reached.
public struct GlassStepProgress: View {
    private let labels: [String]
    private let current: Int

    public init(labels: [String], current: Int) {
        self.labels = labels
        self.current = current
    }

    public var body: some View {
        HStack(alignment: .top, spacing: 0) {
            ForEach(Array(labels.enumerated()), id: \.offset) { index, label in
                if index > 0 {
                    Capsule()
                        .fill(index <= current ? GlassTokens.Color.purpleSoft.color : Color.white.opacity(0.14))
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
                        .glassType(GlassTokens.TypeScale.eyebrow)
                        .foregroundStyle(index == current ? GlassColor.textPrimary : GlassColor.textTertiary)
                }
                .fixedSize()
                .accessibilityElement(children: .combine)
                .accessibilityValue(index < current ? "done" : index == current ? "current" : "pending")
                .accessibilityAddTraits(index == current ? .isSelected : [])
            }
        }
    }
}
