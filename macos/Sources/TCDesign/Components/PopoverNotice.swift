import SwiftUI

/// The popover notice (owner, 2026-10-19): the Traces trace card's shape,
/// since 2026-10-19 every notice's. One card floating over the top of its
/// tab, under the tab bar, pushing nothing down: a mark on the left, the
/// title and its tag, a short line, whatever more there is to say behind a
/// Details expander, the card's own buttons, and an X that closes it.
///
/// Opaque under its glass, so a card stacked behind it, or the list it
/// floats over, never shows through. Every word is the caller's.
public struct GlassPopoverNotice<Mark: View, Summary: View, Details: View, Actions: View>: View {
    private let title: String
    private let tag: String?
    private let tagTone: GlassTag.Tone
    private let detailsLabel: String?
    private let closeLabel: String?
    private let onClose: (() -> Void)?
    private let refusal: String?
    private let mark: Mark
    private let summary: Summary
    private let details: Details
    private let actions: Actions
    @State private var open = false

    /// `detailsLabel` names the expander the details fold behind (owner,
    /// 2026-10-19: info strings collapse into a toggle); without it the
    /// details are drawn open. `onClose` with `closeLabel` draws the X; a
    /// notice that goes only when its cause does has neither. `refusal` is
    /// a failed request's line, under the buttons it is about, as plain red
    /// caption text (owner, 2026-10-09).
    public init(
        title: String, tag: String? = nil, tagTone: GlassTag.Tone = .neutral,
        detailsLabel: String? = nil, closeLabel: String? = nil, onClose: (() -> Void)? = nil,
        refusal: String? = nil,
        @ViewBuilder mark: () -> Mark,
        @ViewBuilder summary: () -> Summary,
        @ViewBuilder details: () -> Details,
        @ViewBuilder actions: () -> Actions
    ) {
        self.title = title
        self.tag = tag
        self.tagTone = tagTone
        self.detailsLabel = detailsLabel
        self.closeLabel = closeLabel
        self.onClose = onClose
        self.refusal = refusal
        self.mark = mark()
        self.summary = summary()
        self.details = details()
        self.actions = actions()
    }

    /// Whether a part was given at all: a part left as `EmptyView` takes no
    /// row and no gap.
    private var hasSummary: Bool { Summary.self != EmptyView.self }
    private var hasDetails: Bool { Details.self != EmptyView.self }
    private var hasActions: Bool { Actions.self != EmptyView.self }

    public var body: some View {
        HStack(alignment: .top, spacing: GlassTokens.Space.s5) {
            mark
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s4) {
                    Text(title)
                        .glassType(GlassTokens.TypeScale.bodyStrong)
                        .foregroundStyle(GlassColor.textPrimary)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityAddTraits(.isHeader)
                    if let tag { GlassTag(tag, tone: tagTone) }
                    Spacer(minLength: 0)
                }
                if hasSummary {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s2) { summary }
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                if hasDetails {
                    if let detailsLabel, !detailsLabel.isEmpty {
                        GlassExpander(detailsLabel, isOpen: $open)
                            .padding(.leading, -GlassExpander.padding.leading)
                        if open { detailsBody }
                    } else {
                        detailsBody
                    }
                }
                if hasActions {
                    HStack(spacing: GlassTokens.Space.s4) { actions }
                        .padding(.top, GlassTokens.Space.s2)
                }
                if let refusal, !refusal.isEmpty { GlassAlert(refusal) }
            }
            if let onClose, let closeLabel, !closeLabel.isEmpty {
                GlassRoundButton(closeLabel, systemImage: "xmark", small: true, action: onClose)
            }
        }
        .padding(GlassTokens.Space.s6)
        .frame(maxWidth: .infinity, alignment: .leading)
        .glassSurface(.nodeCard, floating: true)
        .background(RoundedRectangle(cornerRadius: GlassTokens.Radius.card, style: .continuous)
            .fill(GlassTokens.Color.paneOpaque.color))
        .accessibilityElement(children: .contain)
    }

    private var detailsBody: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) { details }
            .glassType(GlassTokens.TypeScale.caption)
            .foregroundStyle(GlassColor.textSecondary)
            .fixedSize(horizontal: false, vertical: true)
            .transition(.opacity)
    }
}

/// A popover notice's mark when it has no tool to show: the coin the trace
/// card draws a tool's logo in, holding a symbol in the notice's tint.
public struct GlassNoticeMark: View {
    private let systemImage: String
    private let tint: Color

    public init(systemImage: String, tint: Color) {
        self.systemImage = systemImage
        self.tint = tint
    }

    /// The mark for a status tone: the tone's own icon (`GlassNotice`) in
    /// its colour.
    public init(_ tone: GlassStatus) {
        self.init(systemImage: Self.symbol(tone), tint: tone.color)
    }

    /// The tone's symbol: a warning for outside, an exclamation for ask, a
    /// check for on, information otherwise.
    public static func symbol(_ tone: GlassStatus) -> String {
        switch tone {
        case .outside: "exclamationmark.triangle"
        case .ask: "exclamationmark"
        case .on: "checkmark"
        default: "info"
        }
    }

    /// The coin's side, as the trace card's tool coin.
    public static let side: CGFloat = 30

    public var body: some View {
        ZStack {
            Circle().fill(GlassTokens.Color.paneOpaque.color)
            Circle().strokeBorder(tint, lineWidth: 2)
            Image(systemName: systemImage)
                .glassGlyph(12, weight: .semibold)
                .foregroundStyle(tint)
        }
        .frame(width: Self.side, height: Self.side)
        .accessibilityHidden(true)
    }
}

/// One card in a popover stack: its identity, which outlives a redraw so a
/// card is not animated in again, and the card.
public struct GlassPopoverItem: Identifiable {
    public let id: String
    public let view: AnyView

    public init(id: String, @ViewBuilder _ view: () -> some View) {
        self.id = id
        self.view = AnyView(view())
    }
}

/// Popover notices stacked (owner, 2026-10-19): when notices follow one
/// another they pile up rather than replace each other. The last is in
/// front and up to two earlier ones peek out behind it, on the side the
/// stack flows toward: down from the top of a tab, up from the foot of the
/// map. Each card's X closes it and the one behind comes forward.
///
/// The cards behind are drawn as plates the front card's own size, so each
/// always shows the same sliver whatever the cards say.
public struct GlassPopoverStack: View {
    /// The side the stack hangs from; it flows away from it.
    public enum Anchor: Sendable { case top, bottom }

    /// Back first, front last.
    private let items: [GlassPopoverItem]
    private let anchor: Anchor
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    public init(_ items: [GlassPopoverItem], anchor: Anchor) {
        self.items = items
        self.anchor = anchor
    }

    /// How far each card behind shows past the one in front, and how many
    /// cards are drawn.
    public static let peek: CGFloat = 8
    public static let drawn = 3

    /// The ids of the cards drawn, the front one first.
    public static func visible(_ ids: [String]) -> [String] {
        Array(ids.suffix(drawn).reversed())
    }

    /// Which way a card behind moves from the front one: down from the top,
    /// up from the foot.
    public static func direction(_ anchor: Anchor) -> CGFloat { anchor == .bottom ? -1 : 1 }

    public var body: some View {
        let ids = items.map(\.id)
        let behind = max(0, Self.visible(ids).count - 1)
        if let front = items.last {
            front.view
                .background {
                    ZStack {
                        ForEach(Array((0 ..< behind).reversed()), id: \.self) { index in
                            plate(level: CGFloat(index + 1))
                        }
                    }
                    .accessibilityHidden(true)
                }
                .id(front.id)
                .transition(.move(edge: anchor == .bottom ? .bottom : .top).combined(with: .opacity))
                .padding(anchor == .bottom ? .top : .bottom, Self.peek * CGFloat(behind))
                .animation(reduceMotion ? nil : .spring(response: 0.36, dampingFraction: 0.86), value: ids)
        }
    }

    /// A card behind the front one, `level` back: the front card's size, a
    /// little narrower, moved out by `peek` a level.
    private func plate(level: CGFloat) -> some View {
        Color.clear
            .glassSurface(.nodeCard, floating: true)
            .background(RoundedRectangle(cornerRadius: GlassTokens.Radius.card, style: .continuous)
                .fill(GlassTokens.Color.paneOpaque.color))
            .scaleEffect(x: 1 - 0.04 * level, y: 1, anchor: .center)
            .offset(y: Self.direction(anchor) * Self.peek * level)
            .opacity(level == 1 ? 0.9 : 0.7)
    }
}
