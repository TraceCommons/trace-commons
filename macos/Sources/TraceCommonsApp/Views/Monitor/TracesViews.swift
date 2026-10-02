#if DEBUG
import SwiftUI
import TCDesign
import TCBridge
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

/// The inspector for the selected session: what the core reports about it
/// (R6), and its review (R7): what would leave this Mac, the consent gate in
/// the core's words, and Contribute, Keep or Dismiss.
struct SessionInspectorView: View {
    let store: TracesStore
    let entry: DaemonData.QueueEntry?

    @State private var summary: DaemonData.PreviewSummary?
    @State private var failure: DaemonDataError?
    /// The consent gate, from the Rust core. Without it Contribute stays
    /// disabled: the shell never words consent itself.
    private let consent = TCConsentCopy.copyJSON().flatMap(ConsentCopy.decode(fromJSON:))

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
                        // The opening prompt in full, unless it is no more
                        // than the title above it (the title is its first line).
                        if let prompt = summary?.openingPrompt, !prompt.isEmpty, prompt != summary?.title {
                            GlassCard(quiet: true) {
                                Text(prompt)
                                    .glassType(GlassTokens.TypeScale.body)
                                    .foregroundStyle(GlassColor.textPrimary)
                                    .lineLimit(8)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                            }
                        }
                        GlassKeyValueList(Self.rows(entry, summary))
                        if let summary { redactions(summary) }
                        if let reasons = entry.secondLook, !reasons.isEmpty {
                            // The core's fixed reason labels until K4 gives
                            // them words.
                            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                                ForEach(reasons, id: \.self) { GlassChip($0, status: .ask) }
                            }
                        }
                        review(entry)
                    }
                }
                .scrollIndicators(.never)
                .task(id: entry.entryId) { await load(entry.entryId) }
            } else if let kept = store.lastKept {
                undoKeep(kept)
            } else {
                Color.clear
            }
        }
    }

    /// What scrubbing removed, and what it found but left in, in the
    /// core's redaction labels (`RedactionSummary`).
    @ViewBuilder
    private func redactions(_ summary: DaemonData.PreviewSummary) -> some View {
        // A label counted zero times removed nothing: it is not a row.
        let rows = RedactionSummary.rows(occurrences: (summary.redactions ?? [:]).filter { $0.value > 0 }, distinct: [:])
        if !rows.removed.isEmpty {
            GlassCard(quiet: true) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                    ForEach(rows.removed, id: \.family) { row in
                        Text(row.countLine)
                            .glassType(GlassTokens.TypeScale.label)
                            .foregroundStyle(GlassColor.textSecondary)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        if !rows.stillPresent.isEmpty {
            GlassNotice(tone: .outside) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                    ForEach(rows.stillPresent, id: \.family) { row in
                        Text(row.countLine)
                    }
                }
            }
        }
    }

    /// The consent gate and the three actions. Contribute needs the
    /// summary (what would leave) and the core's consent words in front of
    /// the contributor.
    @ViewBuilder
    private func review(_ entry: DaemonData.QueueEntry) -> some View {
        let busy = store.acting.contains(entry.entryId)
        if let consent {
            Text(consent.gateStatement)
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textTertiary)
                .fixedSize(horizontal: false, vertical: true)
        }
        if let refused = store.actionError, refused.entryId == entry.entryId {
            GlassNotice(tone: .outside, title: refused.error.description) { EmptyView() }
        }
        HStack(spacing: GlassTokens.Space.s4) {
            Button(Self.dismissTitle) { act(.dismiss, entry) }
                .buttonStyle(GlassButtonStyle(.glass))
            Button(Self.keepTitle) { act(.keep, entry) }
                .buttonStyle(GlassButtonStyle(.glass))
            Spacer(minLength: 0)
            Button(Self.contributeTitle) { act(.contribute, entry) }
                .buttonStyle(GlassButtonStyle(.primary, small: true))
                .disabled(summary == nil || consent == nil)
        }
        .disabled(busy)
    }

    private func undoKeep(_ entryId: String) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            Button(Self.undoTitle) { Task { await store.perform(.undoKeep, on: entryId) } }
                .buttonStyle(GlassButtonStyle(.glass))
                .disabled(store.acting.contains(entryId))
            Spacer(minLength: 0)
        }
    }

    private func act(_ action: TracesStore.ReviewAction, _ entry: DaemonData.QueueEntry) {
        Task { await store.perform(action, on: entry.entryId) }
    }

    static let contributeTitle = "Contribute"
    static let keepTitle = "Keep"
    static let dismissTitle = "Dismiss"
    static let undoTitle = "Undo"

    private func load(_ entryId: String) async {
        summary = nil
        failure = nil
        do {
            summary = try await store.client.preview(entryId: entryId)
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
