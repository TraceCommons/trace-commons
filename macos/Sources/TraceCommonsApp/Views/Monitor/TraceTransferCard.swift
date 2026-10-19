import SwiftUI
import TCDesign
import TCShellCore

/// The trace being contributed, kept or dismissed (owner, 2026-10-19): one
/// card, drawn at the foot of the Traces map above its legend and zoom,
/// and at the top of the Traces tab between the tab bar and the folders.
/// It shows while the trace moves and a few seconds after
/// (`MapTransferPlayer.shown`), never under the pointer: the tool, what is
/// happening to the trace, the trace's folder and time, and the tool's
/// memory files, which stay on this Mac.
struct TraceTransferCard: View {
    let transfer: MapTransfer
    /// The folder's name when the transfer carries none.
    var folderLabel: String? = nil

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
            }
            .padding(GlassTokens.Space.s6)
            .frame(maxWidth: .infinity, alignment: .leading)
            .glassSurface(.nodeCard, floating: true)
            .accessibilityElement(children: .combine)
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
