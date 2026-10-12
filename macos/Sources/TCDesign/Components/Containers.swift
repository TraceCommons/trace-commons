import AppKit
import SwiftUI

/// A rule one device pixel thick, as a browser draws #1146's 0.5px border:
/// 0.5pt on a Retina display and a whole point at 1x, where a 0.5pt fill
/// would blend into the glass and drop out.
public struct GlassHairline: View {
    public enum Axis: Sendable { case horizontal, vertical }

    private let color: Color
    private let axis: Axis
    @Environment(\.displayScale) private var displayScale

    public init(_ color: Color = GlassColor.hairline, axis: Axis = .horizontal) {
        self.color = color
        self.axis = axis
    }

    /// The rule's thickness in points for a display of this scale: one
    /// device pixel, never thinner than half a point.
    public static func thickness(displayScale: CGFloat) -> CGFloat {
        max(0.5, 1 / max(displayScale, 1))
    }

    public var body: some View {
        let thickness = Self.thickness(displayScale: displayScale)
        Rectangle().fill(color)
            .frame(width: axis == .vertical ? thickness : nil, height: axis == .horizontal ? thickness : nil)
            .accessibilityHidden(true)
    }
}

/// A tint layer on a pane. Never blurred. `quiet` is the 12pt-radius tier.
///
/// `interactive` is a card that acts (it sits in a button): it lifts under
/// the pointer (#1146 `.tc-card--interactive:hover`).
public struct GlassCard<Content: View>: View {
    private let quiet: Bool
    private let flush: Bool
    private let interactive: Bool
    private let content: Content

    public init(quiet: Bool = false, flush: Bool = false, interactive: Bool = false, @ViewBuilder content: () -> Content) {
        self.quiet = quiet
        self.flush = flush
        self.interactive = interactive
        self.content = content()
    }

    /// The hover fill: `cardHover` on an interactive card, none otherwise.
    /// It replaces the card's gradient (#1146 `.tc-card--interactive:hover`).
    static func hoverFill(interactive: Bool) -> GlassRGBA? {
        interactive ? GlassTokens.Color.cardHover : nil
    }

    public var body: some View {
        content
            .padding(.vertical, flush ? 0 : (quiet ? 10 : GlassTokens.Space.cardPaddingVertical))
            .padding(.horizontal, flush ? 0 : (quiet ? 12 : GlassTokens.Space.cardPaddingHorizontal))
            .frame(maxWidth: .infinity, alignment: .leading)
            .glassTier(quiet ? .cardQuiet : .card, hover: Self.hoverFill(interactive: interactive))
    }
}

/// A bare refresh glyph, no pill around it (owner, 2026-10-10), named for
/// assistive tech and in its tooltip.
public struct GlassRefreshButton: View {
    private let label: String
    private let size: CGFloat
    private let action: () -> Void

    public init(_ label: String, size: CGFloat = 13, action: @escaping () -> Void) {
        self.label = label
        self.size = size
        self.action = action
    }

    public var body: some View {
        Button(action: action) {
            Image(systemName: "arrow.clockwise")
                .glassGlyph(size, weight: .medium)
                .foregroundStyle(GlassColor.textSecondary)
                .frame(width: size + 7, height: size + 7)
                .contentShape(Rectangle())
        }
        .buttonStyle(GlassPressStyle())
        .accessibilityLabel(label)
        .help(label)
    }
}

/// An icon card's re-read control: its name and what it does.
public struct GlassCardRefresh {
    let label: String
    let isDisabled: Bool
    let action: () -> Void

    public init(_ label: String, isDisabled: Bool = false, action: @escaping () -> Void) {
        self.label = label
        self.isDisabled = isDisabled
        self.action = action
    }
}

extension VerticalAlignment {
    /// An icon card's head: the icon's top on the title's cap height.
    private enum IconTitleTop: AlignmentID {
        static func defaultValue(in context: ViewDimensions) -> CGFloat { context[.top] }
    }

    static let iconTitleTop = VerticalAlignment(IconTitleTop.self)
}

/// A card headed by an icon (owner, 2026-10-10), laid out the same on
/// every card:
/// - the icon in a fixed column at the top left, level with the title;
/// - the title, with the card's re-read icon right after it on its line;
/// - the subtitle under it, with an info icon after its last line for the
///   card's fine print (shown on hover and in a popover);
/// - the card's actions on the right, centred on the head;
/// - anything else the card holds under the head, aligned with the title.
public struct GlassIconCard<Accessory: View, Content: View>: View {
    /// The card's re-read control.
    public typealias Refresh = GlassCardRefresh

    private let systemImage: String
    private let title: String
    private let subtitle: [String]
    private let info: String?
    private let refresh: Refresh?
    private let accessory: Accessory
    private let content: Content
    /// Whether the whole card is a control (its caller wraps it in one): it
    /// then lifts under the pointer, as an interactive `GlassCard` does.
    private let interactive: Bool
    @State private var infoOpen = false

    /// The icon's point size, and the column it sits in.
    public static var iconSize: CGFloat { 22 }
    static var iconColumn: CGFloat { 26 }
    /// The clear space an SF Symbol at `iconSize` keeps above its drawn
    /// top, measured from a render: the drawn top, not the frame, meets the
    /// title's capitals.
    static var iconTopInset: CGFloat { 2.25 }
    /// The title face's cap height: from its baseline to the top of its
    /// capitals.
    static var titleCapHeight: CGFloat {
        NSFont.systemFont(ofSize: GlassTokens.TypeScale.bodyStrong.size, weight: .semibold).capHeight
    }

    /// `subtitle` is drawn line by line under the title, empty lines left
    /// out; `info` goes behind the info icon after it.
    public init(
        systemImage: String, title: String, subtitle: [String] = [], info: String? = nil, refresh: Refresh? = nil,
        interactive: Bool = false,
        @ViewBuilder accessory: () -> Accessory, @ViewBuilder content: () -> Content
    ) {
        self.interactive = interactive
        self.systemImage = systemImage
        self.title = title
        self.subtitle = subtitle.filter { !$0.isEmpty }
        self.info = info.flatMap { $0.isEmpty ? nil : $0 }
        self.refresh = refresh
        self.accessory = accessory()
        self.content = content()
    }

    public var body: some View {
        GlassCard(interactive: interactive) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                HStack(alignment: .center, spacing: GlassTokens.Space.s5) {
                    HStack(alignment: .iconTitleTop, spacing: GlassTokens.Space.s5) {
                        // The icon's top on the title's cap height, the top
                        // of its letters (owner, 2026-10-10), on every card.
                        Image(systemName: systemImage)
                            .glassGlyph(Self.iconSize, weight: .regular)
                            .foregroundStyle(GlassColor.textSecondary)
                            .frame(width: Self.iconColumn)
                            .alignmentGuide(.iconTitleTop) { $0[.top] + Self.iconTopInset }
                            .accessibilityHidden(true)
                        VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                            HStack(alignment: .center, spacing: GlassTokens.Space.s2) {
                                Text(title)
                                    .glassType(GlassTokens.TypeScale.bodyStrong)
                                    .foregroundStyle(GlassColor.textPrimary)
                                    .fixedSize(horizontal: false, vertical: true)
                                    .alignmentGuide(.iconTitleTop) { $0[.firstTextBaseline] - Self.titleCapHeight }
                                    .accessibilityAddTraits(.isHeader)
                                if let refresh {
                                    GlassRefreshButton(refresh.label, size: 12, action: refresh.action)
                                        .disabled(refresh.isDisabled)
                                }
                                if subtitle.isEmpty { infoButton }
                            }
                            ForEach(Array(subtitle.enumerated()), id: \.offset) { index, line in
                                if index == subtitle.count - 1, info != nil {
                                    infoLine(line)
                                } else {
                                    Text(line)
                                        .glassType(GlassTokens.TypeScale.caption)
                                        .foregroundStyle(GlassColor.textSecondary)
                                        .fixedSize(horizontal: false, vertical: true)
                                }
                            }
                        }
                    }
                    Spacer(minLength: GlassTokens.Space.s4)
                    accessory
                }
                if Content.self != EmptyView.self {
                    content
                        .padding(.leading, Self.iconColumn + GlassTokens.Space.s5)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }
    }

    /// The info icon on its own, after the title when there is no subtitle.
    @ViewBuilder
    private var infoButton: some View {
        if let info {
            Button { infoOpen.toggle() } label: {
                Self.infoGlyph
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textTertiary)
                    .contentShape(Rectangle())
            }
            .buttonStyle(GlassPressStyle())
            .accessibilityLabel(info)
            .help(info)
            .popover(isPresented: $infoOpen, arrowEdge: .bottom) { infoPopover(info) }
        }
    }

    /// The subtitle's last line with the info icon set in its text, right
    /// after the last word wherever the line wraps. The line opens the
    /// popover and shows the fine print on hover.
    @ViewBuilder
    private func infoLine(_ line: String) -> some View {
        if let info {
            Button { infoOpen.toggle() } label: {
                (Text(line).foregroundColor(GlassColor.textSecondary)
                    + Text("\u{00a0}\u{00a0}")
                    + Self.infoGlyph.foregroundColor(GlassColor.textTertiary))
                    .glassType(GlassTokens.TypeScale.caption)
                    .multilineTextAlignment(.leading)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .contentShape(Rectangle())
            }
            .buttonStyle(GlassPressStyle())
            .accessibilityLabel(line)
            .accessibilityHint(info)
            .help(info)
            .popover(isPresented: $infoOpen, arrowEdge: .bottom) { infoPopover(info) }
        }
    }

    private static var infoGlyph: Text { Text(Image(systemName: "info.circle")) }

    private func infoPopover(_ info: String) -> some View {
        Text(info)
            .glassType(GlassTokens.TypeScale.caption)
            .foregroundStyle(GlassColor.textSecondary)
            .fixedSize(horizontal: false, vertical: true)
            .frame(width: 260, alignment: .leading)
            .padding(GlassTokens.Space.s5)
    }
}

public extension GlassIconCard where Content == EmptyView {
    /// A head-only card: `trailing` is the control on its right.
    init(
        systemImage: String, title: String, subtitle: [String] = [], info: String? = nil, refresh: Refresh? = nil,
        interactive: Bool = false, @ViewBuilder trailing: () -> Accessory
    ) {
        self.init(systemImage: systemImage, title: title, subtitle: subtitle, info: info, refresh: refresh,
                  interactive: interactive, accessory: trailing) { EmptyView() }
    }
}

public extension GlassIconCard where Accessory == EmptyView {
    init(
        systemImage: String, title: String, subtitle: [String] = [], info: String? = nil, refresh: Refresh? = nil,
        @ViewBuilder content: () -> Content
    ) {
        self.init(systemImage: systemImage, title: title, subtitle: subtitle, info: info, refresh: refresh,
                  accessory: { EmptyView() }, content: content)
    }
}

/// A card with an eyebrow heading and an optional accessory on the right.
/// With `title`, #1146's two-level head: the eyebrow over an h2, the
/// accessory (a chip, a re-read link) top-right. With `action` the whole
/// card is a button (Home's Missions and History).
public struct GlassEyebrowCard<Accessory: View, Content: View>: View {
    private let eyebrow: String
    private let title: String?
    private let accessory: Accessory
    private let content: Content
    private let action: (() -> Void)?
    private let chevron: Bool

    /// `chevron` draws a chevron at the card's trailing edge, centred on the
    /// whole card rather than on its heading (owner, 2026-10-10), for a card
    /// that is a way into somewhere; `accessory` stays beside the heading.
    public init(
        _ eyebrow: String,
        title: String? = nil,
        action: (() -> Void)? = nil,
        chevron: Bool = false,
        @ViewBuilder accessory: () -> Accessory = { EmptyView() },
        @ViewBuilder content: () -> Content
    ) {
        self.eyebrow = eyebrow
        self.title = title
        self.action = action
        self.chevron = chevron
        self.accessory = accessory()
        self.content = content()
    }

    private var card: some View {
        GlassCard(interactive: action != nil) {
            HStack(alignment: .center, spacing: GlassTokens.Space.s6) {
                stack
                if chevron { GlassChevron() }
            }
        }
    }

    /// The heading, its accessory, and the content under them.
    private var stack: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            HStack(alignment: title == nil ? .center : .top, spacing: GlassTokens.Space.s6) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                    Text(eyebrow).glassType(GlassTokens.TypeScale.eyebrow).foregroundStyle(GlassColor.textTertiary)
                    if let title {
                        Text(title)
                            .glassType(GlassTokens.TypeScale.title)
                            .foregroundStyle(GlassColor.textPrimary)
                            .fixedSize(horizontal: false, vertical: true)
                            .accessibilityAddTraits(.isHeader)
                    }
                }
                Spacer(minLength: GlassTokens.Space.s4)
                accessory
            }
            content
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

/// The chevron on a card that is a way into somewhere. Decorative: the
/// card says where it goes.
public struct GlassChevron: View {
    public init() {}

    public var body: some View {
        Image(systemName: "chevron.right")
            .glassGlyph(10, weight: .semibold)
            .foregroundStyle(GlassColor.textTertiary)
            .accessibilityHidden(true)
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

/// One datum in a well: dot with a halo, label, tabular value. #1146's
/// `.tc-legend-cell`: a fixed 28pt pill on one line, so two cells side by
/// side are always the same height; a label too long for its half shrinks a
/// little, then truncates, and never wraps.
public struct GlassLegendCell: View {
    private let status: GlassStatus
    private let label: String
    private let value: String

    public init(_ label: String, value: String, status: GlassStatus) {
        self.label = label
        self.value = value
        self.status = status
    }

    /// How far a long label may shrink before it truncates.
    public static let labelShrink: CGFloat = 0.85

    public var body: some View {
        HStack(spacing: GlassTokens.Space.s4) {
            HStack(spacing: 7) {
                GlassStatusDot(status, size: GlassTokens.Size.dotLarge, halo: true)
                Text(label).foregroundStyle(GlassColor.textSecondary)
                    .lineLimit(1)
                    .minimumScaleFactor(Self.labelShrink)
                    .truncationMode(.tail)
            }
            Spacer(minLength: GlassTokens.Space.s4)
            Text(value).monospacedDigit().fontWeight(.semibold).foregroundStyle(GlassColor.textPrimary)
                .lineLimit(1)
                .fixedSize()
        }
        .glassType(GlassTokens.TypeScale.label.weight(.regular))
        .padding(.horizontal, 10)
        .frame(height: GlassTokens.Size.controlLarge)
        .glassTier(.well)
        .accessibilityElement(children: .combine)
    }
}

/// Label and value pairs, labels right-aligned in a 70pt column (the
/// inspector's Path, Sessions, User), inset 6pt on each side (#1146
/// `.tc-kv`).
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
        .padding(.horizontal, Self.inset)
        .glassType(GlassTokens.TypeScale.label.weight(.regular))
    }

    static let inset: CGFloat = 6
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
                    GlassHairline(GlassColor.hairline)
                }
            }
    }
}

/// The heading row over a flush card's `GlassTableRow`s (#1146
/// `TableHead`): eyebrow type in tertiary text, on the rows' insets, read as
/// a header. The caller lays out the columns to match its rows.
public struct GlassTableHead<Content: View>: View {
    private let content: Content

    public init(@ViewBuilder content: () -> Content) {
        self.content = content()
    }

    public var body: some View {
        content
            .glassType(GlassTokens.TypeScale.eyebrow)
            .foregroundStyle(GlassColor.textTertiary)
            .padding(.vertical, 8)
            .padding(.horizontal, 12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .accessibilityElement(children: .combine)
            .accessibilityAddTraits(.isHeader)
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
            GlassHairline(GlassColor.ink(0.14))
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

    static var titleGap: CGFloat { GlassTokens.Space.s3 }

    public var body: some View {
        GlassCard(quiet: true) {
            // Title to body: space-3 (#1146 `.tc-notice`); the title stays
            // secondary text, its tone carried by the dot.
            VStack(alignment: .leading, spacing: Self.titleGap) {
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
