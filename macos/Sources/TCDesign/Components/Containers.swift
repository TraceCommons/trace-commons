import SwiftUI

/// A tint layer on a pane. Never blurred. `quiet` is the 12pt-radius tier.
public struct GlassCard<Content: View>: View {
    private let quiet: Bool
    private let flush: Bool
    private let content: Content

    public init(quiet: Bool = false, flush: Bool = false, @ViewBuilder content: () -> Content) {
        self.quiet = quiet
        self.flush = flush
        self.content = content()
    }

    public var body: some View {
        content
            .padding(.vertical, flush ? 0 : (quiet ? 10 : GlassTokens.Space.cardPaddingVertical))
            .padding(.horizontal, flush ? 0 : (quiet ? 12 : GlassTokens.Space.cardPaddingHorizontal))
            .frame(maxWidth: .infinity, alignment: .leading)
            .glassTier(quiet ? .cardQuiet : .card)
    }
}

/// A card with an eyebrow heading and an optional accessory on the right.
/// With `action` the whole card is a button (Home's Missions and History).
public struct GlassEyebrowCard<Accessory: View, Content: View>: View {
    private let eyebrow: String
    private let accessory: Accessory
    private let content: Content
    private let action: (() -> Void)?

    public init(
        _ eyebrow: String,
        action: (() -> Void)? = nil,
        @ViewBuilder accessory: () -> Accessory = { EmptyView() },
        @ViewBuilder content: () -> Content
    ) {
        self.eyebrow = eyebrow
        self.action = action
        self.accessory = accessory()
        self.content = content()
    }

    private var card: some View {
        GlassCard {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                HStack {
                    Text(eyebrow).glassType(GlassTokens.TypeScale.eyebrow).foregroundStyle(GlassColor.textTertiary)
                    Spacer(minLength: GlassTokens.Space.s4)
                    accessory
                }
                content
            }
        }
    }

    public var body: some View {
        if let action {
            // The card's tier fill darkens while pressed (`GlassPressStyle`).
            Button(action: action) { card }.buttonStyle(GlassPressStyle())
        } else {
            card
        }
    }
}

/// An inset track: legend cells, segmented tabs, unchecked boxes.
public struct GlassWell<Content: View>: View {
    private let content: Content

    public init(@ViewBuilder content: () -> Content) {
        self.content = content()
    }

    public var body: some View {
        content.glassTier(.well)
    }
}

/// The consent sentence, verbatim, directly above Submit. The text comes
/// from the Rust core; this only lays it out.
public struct GlassConsentBlock: View {
    private let text: String

    public init(_ text: String) {
        self.text = text
    }

    public var body: some View {
        Text(text)
            .glassType(GlassTokens.TypeScale.label.weight(.regular))
            .foregroundStyle(GlassColor.textPrimary)
            .padding(.vertical, 10)
            .padding(.horizontal, 12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                RoundedRectangle(cornerRadius: GlassTokens.Radius.cardQuiet, style: .continuous)
                    .fill(GlassTokens.Color.consentFill.color)
            )
            .glassEdge(GlassTokens.Shadow.consentEdge, in: RoundedRectangle(cornerRadius: GlassTokens.Radius.cardQuiet, style: .continuous))
    }
}

/// One datum in a well: dot with a halo, label, tabular value.
public struct GlassLegendCell: View {
    private let status: GlassStatus
    private let label: String
    private let value: String

    public init(_ label: String, value: String, status: GlassStatus) {
        self.label = label
        self.value = value
        self.status = status
    }

    public var body: some View {
        HStack(spacing: GlassTokens.Space.s4) {
            HStack(spacing: 7) {
                GlassStatusDot(status, size: GlassTokens.Size.dotLarge, halo: true)
                Text(label).foregroundStyle(GlassColor.textSecondary)
            }
            Spacer(minLength: GlassTokens.Space.s4)
            Text(value).monospacedDigit().fontWeight(.semibold).foregroundStyle(GlassColor.textPrimary)
        }
        .glassType(GlassTokens.TypeScale.label.weight(.regular))
        .padding(.horizontal, 10)
        .frame(minHeight: GlassTokens.Size.controlLarge)
        .glassTier(.well)
        .accessibilityElement(children: .combine)
    }
}

/// Label and value pairs, labels right-aligned in a 70pt column (the
/// inspector's Path, Sessions, User).
public struct GlassKeyValueList: View {
    public struct Item: Identifiable {
        public let label: String
        public let value: String
        public let mono: Bool
        public var id: String { label }

        public init(_ label: String, _ value: String, mono: Bool = false) {
            self.label = label
            self.value = value
            self.mono = mono
        }
    }

    private let items: [Item]

    public init(_ items: [Item]) {
        self.items = items
    }

    public var body: some View {
        Grid(alignment: .topLeading, horizontalSpacing: 10, verticalSpacing: 6) {
            ForEach(items) { item in
                GridRow {
                    Text(item.label)
                        .foregroundStyle(GlassColor.textTertiary)
                        .frame(width: 70, alignment: .trailing)
                    Group {
                        if item.mono {
                            Text(item.value).glassType(GlassTokens.TypeScale.mono)
                        } else {
                            Text(item.value)
                        }
                    }
                    .foregroundStyle(GlassColor.textPrimary)
                    .textSelection(.enabled)
                }
                .accessibilityElement(children: .combine)
            }
        }
        .glassType(GlassTokens.TypeScale.label.weight(.regular))
    }
}

/// A grid row inside a flush card: hairline above, never around.
public struct GlassTableRow<Content: View>: View {
    private let first: Bool
    private let content: Content

    public init(first: Bool = false, @ViewBuilder content: () -> Content) {
        self.first = first
        self.content = content()
    }

    public var body: some View {
        content
            .glassType(GlassTokens.TypeScale.label.weight(.regular))
            .padding(.vertical, 9)
            .padding(.horizontal, 12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .overlay(alignment: .top) {
                if !first {
                    Rectangle().fill(GlassColor.hairline).frame(height: 0.5)
                }
            }
    }
}

/// A Settings section heading: purple eyebrow and a rule to the right.
public struct GlassSectionRule: View {
    private let title: String

    public init(_ title: String) {
        self.title = title
    }

    public var body: some View {
        HStack(spacing: GlassTokens.Space.s6) {
            Text(title)
                .glassType(GlassTokens.TypeScale.eyebrow)
                .tracking(1)
                .foregroundStyle(GlassColor.accentText)
                .fixedSize()
                .accessibilityAddTraits(.isHeader)
            Rectangle().fill(Color.white.opacity(0.14)).frame(height: 0.5)
        }
        .padding(.top, GlassTokens.Space.s3)
    }
}

/// A notice on a quiet card: the title carries the tone as a status dot;
/// the body stays in the text colours. Tone never tints the card.
public struct GlassNotice<Content: View>: View {
    private let tone: GlassStatus
    private let title: String?
    private let content: Content

    public init(tone: GlassStatus = .ask, title: String? = nil, @ViewBuilder content: () -> Content) {
        self.tone = tone
        self.title = title
        self.content = content()
    }

    public var body: some View {
        GlassCard(quiet: true) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                if let title {
                    GlassStatusLabel(title, status: tone).fontWeight(.semibold)
                }
                content
                    .glassType(GlassTokens.TypeScale.label.weight(.regular))
                    .foregroundStyle(GlassColor.textSecondary)
            }
        }
        .accessibilityElement(children: .contain)
    }
}
