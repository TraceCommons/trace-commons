import SwiftUI
import TCDesign
import TCShellCore

/// The trace being contributed, kept or dismissed (owner, 2026-10-19): one
/// card, floating over the foot of the Traces map above its legend and
/// zoom, and over the top of the Traces tab under the tab bar, pushing
/// nothing aside. It shows while the trace moves and a few seconds after
/// (`MapTransferPlayer.shown`), never under the pointer, and its X closes
/// it sooner: the tool, what is happening to the trace, the trace's folder
/// and time, and the tool's memory files, which stay on this Mac.
struct TraceTransferCard: View {
    let transfer: MapTransfer
    /// The folder's name when the transfer carries none.
    var folderLabel: String? = nil
    /// Closes the card: the X in its top right corner, named with the
    /// core's close word; none without one.
    var onClose: (() -> Void)? = nil

    private static var words: MonitorFlowMapCopy? { FlowMapScene.words }

    var body: some View {
        if let words = Self.words {
            let tool = SourceKind(rawValue: transfer.tool)
            let name = tool?.displayName ?? transfer.tool
            let glass = tool.map(TracesTreeView.glassTool) ?? .other(initials: FlowMapScene.initials(transfer.tool))
            let state = Self.state(transfer.action, tool: name, words: words)
            HStack(alignment: .top, spacing: GlassTokens.Space.s5) {
                coin(glass)
                VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                    HStack(spacing: GlassTokens.Space.s4) {
                        Text(name).glassType(GlassTokens.TypeScale.bodyStrong).foregroundStyle(GlassColor.textPrimary)
                        GlassTag(state.tag, tone: state.tone)
                        Spacer(minLength: 0)
                    }
                    HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s3) {
                        Text(words.transferTrace.uppercased())
                            .glassType(GlassTokens.TypeScale.eyebrow)
                            .foregroundStyle(GlassColor.textTertiary)
                        Text(traceLine)
                            .glassType(GlassTokens.TypeScale.label.weight(.regular))
                            .foregroundStyle(GlassColor.textPrimary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                    memory(words)
                    Text(state.note)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                if let onClose, let close = MonitorWords.table?.close {
                    GlassRoundButton(close, systemImage: "xmark", small: true, action: onClose)
                }
            }
            .padding(GlassTokens.Space.s6)
            .frame(maxWidth: .infinity, alignment: .leading)
            .glassSurface(.nodeCard, floating: true)
            // Opaque under its glass: a card stacked behind, or the map and
            // list beneath, never shows through it.
            .background(RoundedRectangle(cornerRadius: GlassTokens.Radius.card, style: .continuous)
                .fill(GlassTokens.Color.paneOpaque.color))
            .accessibilityElement(children: .contain)
        }
    }

    /// The card's tag, its tone, and its note, by what is happening.
    static func state(_ action: MapTransfer.Action, tool: String, words: MonitorFlowMapCopy)
        -> (tag: String, tone: GlassTag.Tone, note: String)
    {
        switch action {
        case .contribute: (words.transferContributing, .on, words.transferNote(contributing: true, tool: tool))
        case .keep: (words.transferKeeping, .neutral, words.transferKeepNote)
        case .dismiss: (words.transferDismissed, .neutral, words.transferDismissNote)
        }
    }

    /// The trace's folder and, when recorded, its time (`MapTransfer.at`).
    var traceLine: String {
        let folder = transfer.folderLabel ?? folderLabel ?? transfer.folder
        guard let at = transfer.at else { return folder }
        return folder + " \u{00B7} " + at.formatted(date: .abbreviated, time: .shortened)
    }

    private func coin(_ glass: GlassTool) -> some View {
        ZStack {
            Circle().fill(GlassTokens.Color.paneOpaque.color)
            Circle().strokeBorder(glass.tint.color, lineWidth: 2)
            if let logo = glass.logo {
                GlassToolLogoShape(logo).fill(glass.tint.color).frame(width: 14, height: 14)
            } else {
                Text(glass.initials).font(.system(size: 10, weight: .bold, design: .monospaced))
                    .foregroundStyle(glass.tint.color)
            }
        }
        .frame(width: 30, height: 30)
    }

    /// The tool's memory files, which stay on this Mac.
    private func memory(_ words: MonitorFlowMapCopy) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s3) {
            Image(systemName: "lock")
                .font(.system(size: 10))
                .foregroundStyle(GlassColor.textPrimary)
            if let files = words.memoryFiles(transfer.tool) {
                Text(files).glassType(GlassTokens.TypeScale.mono).foregroundStyle(GlassColor.textPrimary)
            }
            Text(words.memoryStays)
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textSecondary)
        }
        .padding(.vertical, GlassTokens.Space.s2)
        .padding(.horizontal, GlassTokens.Space.s4)
        .background(RoundedRectangle(cornerRadius: 9, style: .continuous).fill(GlassTokens.Color.wellFill.color))
    }
}

/// The trace cards, stacked (owner, 2026-10-19): when traces follow one
/// another the cards pile up rather than replace each other. The newest is
/// in front and up to two earlier ones peek out behind it, on the side the
/// stack opens toward: above it at the map's foot, below it at the top of
/// the Traces tab. Each card's X closes that card, and the one behind comes
/// forward; each also goes on its own when its time is up.
struct TraceTransferStack: View {
    /// The side the stack hangs from.
    enum Anchor { case top, bottom }

    /// Oldest first, as `MapTransferPlayer.cards`.
    let cards: [MapTransfer]
    let anchor: Anchor
    /// A trace's folder name when the transfer carries none.
    var folderLabel: (MapTransfer) -> String? = { _ in nil }
    /// Closes the card for the trace with this id.
    var onClose: ((String) -> Void)? = nil
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    /// How far each card behind shows past the one in front, and how many
    /// are drawn.
    static let peek: CGFloat = 8
    static let drawn = 3

    /// The cards drawn, the front one first.
    static func visible(_ cards: [MapTransfer]) -> [MapTransfer] {
        Array(cards.suffix(drawn).reversed())
    }

    var body: some View {
        let visible = Self.visible(cards)
        let up: CGFloat = anchor == .bottom ? -1 : 1
        let behind = CGFloat(max(0, visible.count - 1))
        ZStack(alignment: anchor == .bottom ? .bottom : .top) {
            ForEach(Array(visible.enumerated()), id: \.element.id) { depth, transfer in
                let level = CGFloat(depth)
                let fade: Double = depth == 0 ? 1 : 0.75 - 0.2 * Double(depth - 1)
                TraceTransferCard(
                    transfer: transfer, folderLabel: folderLabel(transfer),
                    onClose: onClose.map { close in { close(transfer.id) } })
                    .scaleEffect(1 - 0.04 * level, anchor: anchor == .bottom ? .top : .bottom)
                    .offset(y: up * Self.peek * level)
                    .opacity(fade)
                    .allowsHitTesting(depth == 0)
                    .accessibilityHidden(depth > 0)
                    .zIndex(-Double(depth))
                    .transition(.move(edge: anchor == .bottom ? .bottom : .top).combined(with: .opacity))
            }
        }
        .padding(anchor == .bottom ? .top : .bottom, Self.peek * behind)
        .animation(reduceMotion ? nil : .spring(response: 0.36, dampingFraction: 0.86), value: cards)
    }
}
