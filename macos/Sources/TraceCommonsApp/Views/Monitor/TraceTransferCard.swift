import SwiftUI
import TCDesign
import TCShellCore

/// The trace being contributed, kept or dismissed (owner, 2026-10-19): a
/// popover notice (`GlassPopoverNotice`) floating over the top of the
/// Traces tab while that tab is shown, and over the foot of the Traces map
/// above its legend and zoom while another tab is (owner, 2026-10-19). It
/// shows while the trace moves and a few seconds after
/// (`MapTransferPlayer.shown`), and its X closes it sooner: the tool, what
/// is happening to the trace, the trace's folder and time, and, behind
/// Details, the tool's memory files, which stay on this Mac, and the note.
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
            // A batch from several tools names none: its count is the title.
            GlassPopoverNotice(
                title: transfer.mixedTools ? words.traces(transfer.count) : name, tag: state.tag, tagTone: state.tone,
                detailsLabel: MonitorWords.noticeDetails, closeLabel: MonitorWords.close, onClose: onClose
            ) {
                coin(glass)
            } summary: {
                HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s3) {
                    // A batch says how many traces it is (owner,
                    // 2026-10-19), and names its folder only when they
                    // share one.
                    Text((transfer.count > 1 ? words.traces(transfer.count) : words.transferTrace).uppercased())
                        .glassType(GlassTokens.TypeScale.eyebrow)
                        .foregroundStyle(GlassColor.textTertiary)
                        .fixedSize()
                    // The folder gives way, never the time: a long name is
                    // cut in its middle and the date stays whole.
                    if !transfer.mixedFolders {
                        Text(folder)
                            .glassType(GlassTokens.TypeScale.label.weight(.regular))
                            .foregroundStyle(GlassColor.textPrimary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                    if let time {
                        Text("\u{00B7} " + time)
                            .glassType(GlassTokens.TypeScale.label.weight(.regular))
                            .foregroundStyle(GlassColor.textSecondary)
                            .lineLimit(1)
                            .fixedSize()
                    }
                }
                .accessibilityElement(children: .combine)
            } details: {
                if !transfer.mixedTools { memory(words) }
                Text(state.note)
            } actions: {
                EmptyView()
            }
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

    /// The trace's folder: the core's name, else the map's.
    var folder: String { transfer.folderLabel ?? folderLabel ?? transfer.folder }

    /// When the trace was recorded (`MapTransfer.at`), if it was.
    var time: String? { transfer.at?.formatted(date: .abbreviated, time: .shortened) }

    /// The trace's folder and, when recorded, its time.
    var traceLine: String {
        guard let time else { return folder }
        return folder + " \u{00B7} " + time
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
        .frame(width: GlassNoticeMark.side, height: GlassNoticeMark.side)
    }

    /// The tool's memory files, which stay on this Mac.
    private func memory(_ words: MonitorFlowMapCopy) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s3) {
            Image(systemName: "lock")
                .font(.system(size: 10))
                .foregroundStyle(GlassColor.textPrimary)
            VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                if let files = words.memoryFiles(transfer.tool) {
                    Text(files).glassType(GlassTokens.TypeScale.mono).foregroundStyle(GlassColor.textPrimary)
                }
                Text(words.memoryStays)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
            }
            Spacer(minLength: 0)
        }
        .padding(.vertical, GlassTokens.Space.s2)
        .padding(.horizontal, GlassTokens.Space.s4)
        .background(RoundedRectangle(cornerRadius: 9, style: .continuous).fill(GlassTokens.Color.wellFill.color))
    }

    /// The cards for the traces moving now, oldest first
    /// (`MapTransferPlayer.cards`), as popover items: each closes by the
    /// trace's id.
    static func items(
        _ cards: [MapTransfer], folderLabel: @escaping (MapTransfer) -> String? = { _ in nil },
        onClose: ((String) -> Void)?
    ) -> [GlassPopoverItem] {
        cards.map { transfer in
            GlassPopoverItem(id: "transfer:" + transfer.id) {
                TraceTransferCard(
                    transfer: transfer, folderLabel: folderLabel(transfer),
                    onClose: onClose.map { close in { close(transfer.id) } })
            }
        }
    }
}
