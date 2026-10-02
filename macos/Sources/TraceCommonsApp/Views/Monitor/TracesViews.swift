#if DEBUG
import SwiftUI
import TCDesign
import TCShellCore

/// The Traces tab (R6 of #1173): the tool › folder › session tree.
///
/// Every label is a single word, a formatted number or date, a name the
/// core reports, or `ProjectCopy` / `SourceKind` wording the shell already
/// carries (`ShellWordingTests`).
struct TracesTreeView: View {
    let store: TracesStore
    @Binding var selection: String

    @State private var collapsed: Set<String> = []

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            if case .failed(let error) = store.phase {
                // The core's own fixed label until K4 gives this state its
                // words. The last good tree stays below it.
                GlassNotice(tone: .outside, title: error.description) { EmptyView() }
            }
            if store.phase == .loading && isEmpty {
                ProgressView().controlSize(.small).frame(maxWidth: .infinity)
            } else if isEmpty && store.phase == .loaded {
                Image(systemName: "tray")
                    .glassGlyph(22)
                    .foregroundStyle(GlassColor.textTertiary)
                    .frame(maxWidth: .infinity)
                    .padding(.top, GlassTokens.Space.s10)
                    .accessibilityLabel(MonitorWindowView.Tab.traces.rawValue)
            } else {
                ScrollView {
                    VStack(spacing: 2) {
                        ForEach(store.tree.tools) { tool in
                            toolRow(tool)
                            if isOpen(tool.id) {
                                ForEach(tool.folders) { folderRows($0) }
                            }
                        }
                        ForEach(store.tree.unplaced) { folderRows($0) }
                    }
                }
                .scrollIndicators(.never)
            }
        }
    }

    private var isEmpty: Bool { store.tree.tools.isEmpty && store.tree.unplaced.isEmpty }

    private func isOpen(_ id: String) -> Bool { !collapsed.contains(id) }

    private func toggle(_ id: String) {
        if collapsed.contains(id) { collapsed.remove(id) } else { collapsed.insert(id) }
    }

    private func toolRow(_ tool: TracesTree.ToolNode) -> some View {
        GlassListRow(
            depth: .tool,
            tile: .tool(Self.glassTool(tool.kind)),
            title: tool.kind.displayName,
            sub: tool.waiting > 0 ? String(tool.waiting) : nil,
            off: tool.mode == .off,
            expanded: tool.folders.isEmpty ? nil : isOpen(tool.id),
            // The source declaration. Unset draws no switch: never off.
            // Read-only until the contract carries a source-mode write.
            watched: tool.mode == .unset ? nil : .constant(tool.mode == .watch),
            watchLabel: tool.kind.displayName,
            expandLabel: tool.kind.displayName,
            onToggleExpand: { toggle(tool.id) }
        )
    }

    @ViewBuilder
    private func folderRows(_ folder: TracesTree.FolderNode) -> some View {
        GlassListRow(
            depth: .folder,
            tile: .folder,
            title: folder.label,
            sub: folder.mode.map(ProjectCopy.modeChoiceLabel),
            expanded: folder.sessions.isEmpty ? nil : isOpen(folder.id),
            // On is offered (Ask me first); off is Never offer this one. A
            // folder the core has not listed has no mode, so no switch.
            watched: folder.mode.map { mode in
                Binding(
                    get: { mode != .ignore },
                    set: { offered in Task { await store.setFolderOffered(folder.id, offered) } })
            },
            watchLabel: folder.label,
            expandLabel: folder.label,
            onToggleExpand: { toggle(folder.id) }
        )
        .disabled(store.writing.contains(folder.id))
        if isOpen(folder.id) {
            ForEach(folder.sessions) { sessionRow($0) }
        }
    }

    private func sessionRow(_ entry: DaemonData.QueueEntry) -> some View {
        GlassListRow(
            depth: .session,
            tile: .session,
            title: Self.when(entry),
            sub: Self.measures(entry),
            flag: Self.flag(entry),
            selected: selection == entry.entryId,
            // D10 default: a session's pill opens its review.
            submitTitle: Self.reviewTitle,
            onSelect: { selection = entry.entryId },
            onSubmit: { selection = entry.entryId }
        )
    }

    static let reviewTitle = "Review"

    // MARK: Formatting

    static func glassTool(_ kind: SourceKind) -> GlassTool {
        switch kind {
        case .claudeCode: .claudeCode
        case .codex: .codex
        case .geminiCli: .geminiCLI
        case .cline: .cline
        case .opencode: .openCode
        }
    }

    /// When the session started, or when it was queued if the daemon did not
    /// record a start.
    static func when(_ entry: DaemonData.QueueEntry) -> String {
        guard let date = entry.startedAt ?? entry.discoveredAt else { return "—" }
        return date.formatted(.dateTime.month(.abbreviated).day().hour().minute())
    }

    /// Length and size, each only when the daemon reported it.
    static func measures(_ entry: DaemonData.QueueEntry) -> String? {
        var parts: [String] = []
        if let seconds = entry.durationSecs {
            parts.append(Duration.seconds(seconds).formatted(.units(allowed: [.hours, .minutes], width: .abbreviated)))
        }
        if let bytes = entry.sizeBytes {
            parts.append(ByteCountFormatter.string(fromByteCount: Int64(bytes), countStyle: .file))
        }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    /// Amber when a person has to look before it can go: a second-look hold,
    /// Manual Scrub check, or back from Keep.
    static func flag(_ entry: DaemonData.QueueEntry) -> GlassListRow.Flag? {
        if entry.heldForSecondLook || entry.heldByManualScrubCheck || entry.returnedFromKeep {
            return .ask
        }
        return nil
    }
}

/// The inspector for the selected session (R6): what the core reports
/// about it, from the `preview` summary.
struct SessionInspectorView: View {
    let client: any DaemonDataClient
    let entry: DaemonData.QueueEntry?

    @State private var summary: DaemonData.PreviewSummary?
    @State private var failure: DaemonDataError?

    var body: some View {
        Group {
            if let entry {
                ScrollView {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                        Text(summary?.title ?? TracesTreeView.when(entry))
                            .glassType(GlassTokens.TypeScale.title)
                            .foregroundStyle(GlassColor.textPrimary)
                            .lineLimit(3)
                        if let failure {
                            GlassNotice(tone: .outside, title: failure.description) { EmptyView() }
                        }
                        GlassKeyValueList(Self.rows(entry, summary))
                        if let reasons = entry.secondLook, !reasons.isEmpty {
                            // The core's fixed reason labels until K4 gives
                            // them words.
                            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                                ForEach(reasons, id: \.self) { GlassChip($0, status: .ask) }
                            }
                        }
                    }
                }
                .scrollIndicators(.never)
                .task(id: entry.entryId) { await load(entry.entryId) }
            } else {
                Color.clear
            }
        }
    }

    private func load(_ entryId: String) async {
        summary = nil
        failure = nil
        do {
            summary = try await client.preview(entryId: entryId)
        } catch {
            failure = error as? DaemonDataError ?? .undecodable(method: "preview")
        }
    }

    /// One word a row, and only what the daemon reported. Unknown is a dash.
    static func rows(_ entry: DaemonData.QueueEntry, _ summary: DaemonData.PreviewSummary?) -> [GlassKeyValueList.Item] {
        let dash = "—"
        func bytes(_ value: Int?) -> String {
            value.map { ByteCountFormatter.string(fromByteCount: Int64($0), countStyle: .file) } ?? dash
        }
        func number(_ value: Int?) -> String { value.map(String.init) ?? dash }
        let tool = SourceKind(rawValue: entry.declaredSource ?? entry.source)?.displayName ?? entry.source
        return [
            .init("Tool", tool),
            .init("Folder", entry.projectLabel),
            .init("Started", entry.startedAt.map { $0.formatted(date: .abbreviated, time: .shortened) } ?? dash),
            .init("Length", entry.durationSecs.map {
                Duration.seconds($0).formatted(.units(allowed: [.hours, .minutes], width: .abbreviated))
            } ?? dash),
            .init("Prompts", number(entry.userTurns)),
            .init("Size", bytes(entry.sizeBytes)),
            .init("Sends", bytes(summary?.wouldSendBytes)),
            // Absent until scrubbed: a dash, never zero.
            .init("Marks", number(entry.marks)),
            .init("Unsure", number(entry.unsureSpans)),
        ]
    }
}
#endif
