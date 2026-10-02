#if DEBUG
import SwiftUI
import TCBridge
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
    /// A mode change waiting on the core's confirmation: arming always,
    /// ignoring when the folder has sessions waiting. It carries the core's
    /// words for that folder, decoded when the change was asked for.
    @State private var confirming: Confirmation?
    /// A tool switch change waiting on the core's explanation of what the
    /// source declaration does.
    @State private var sourceChange: SourceChange?

    struct SourceChange {
        let kind: SourceKind
        /// True to watch a folder, false for "I do not use this tool".
        let watch: Bool
        let explanation: String
        let action: String
    }

    struct Confirmation {
        let folder: TracesTree.FolderNode
        let mode: ProjectMode
        let words: Words

        enum Words {
            case ignore(ProjectIgnoreCopy)
            case arm(ProjectArmingCopy)
        }

        var title: String {
            switch words {
            case .ignore(let copy): copy.title
            case .arm(let copy): copy.question
            }
        }

        var body: String {
            switch words {
            case .ignore(let copy): copy.body
            case .arm(let copy): copy.body
            }
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            if let notice = store.folderNotice {
                GlassNotice(tone: .ask) { Text(notice) }
            }
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
                // Full Keyboard Access: focus the tree, then the arrow keys
                // move the selection through the sessions as drawn.
                .focusable()
                .onMoveCommand(perform: move)
            }
        }
        .confirmationDialog(
            confirming?.title ?? "",
            isPresented: Binding(get: { confirming != nil }, set: { if !$0 { confirming = nil } }),
            titleVisibility: .visible,
            presenting: confirming
        ) { pending in
            switch pending.words {
            case .ignore(let copy):
                Button(copy.button, role: .destructive) { apply(pending.folder, pending.mode) }
                // The system's word, as the Waiting screen's ignore uses.
                Button("Cancel", role: .cancel) { confirming = nil }
            case .arm(let copy):
                Button(copy.confirm) { apply(pending.folder, pending.mode) }
                Button(copy.decline, role: .cancel) { confirming = nil }
            }
        } message: { pending in
            Text(pending.body)
        }
        .confirmationDialog(
            sourceChange?.kind.displayName ?? "",
            isPresented: Binding(get: { sourceChange != nil }, set: { if !$0 { sourceChange = nil } }),
            titleVisibility: .visible,
            presenting: sourceChange
        ) { change in
            Button(change.action) { applySource(change) }
            Button("Cancel", role: .cancel) { sourceChange = nil }
        } message: { change in
            Text(change.explanation)
        }
    }

    // MARK: Tool sources

    /// A tool switch flipped. The declaration is written only after the
    /// core's explanation of it is shown (`SourceSettingsCopy`), as Settings
    /// shows it beside the same choice; with no copy nothing is written.
    private func requestSource(_ tool: TracesTree.ToolNode, watch: Bool) {
        guard let copy = TCSourceChecks.settingsCopy(), let entry = copy.tools[tool.kind.rawValue] else { return }
        sourceChange = SourceChange(
            kind: tool.kind,
            watch: watch,
            explanation: entry.explanation ?? copy.explanation,
            action: watch ? (entry.chooseFolder ?? copy.chooseFolder) : entry.decline)
    }

    private func applySource(_ change: SourceChange) {
        sourceChange = nil
        if change.watch {
            // `get_settings` never reports a path, so watching asks which
            // folder, as Settings does.
            guard let path = SourceRootRow.chooseFolder() else { return }
            Task { await store.setSource(change.kind, .watch(path: path)) }
        } else {
            Task { await store.setSource(change.kind, .off) }
        }
    }

    // MARK: Folder modes

    /// The three modes, as Settings offers them. Arming always asks first,
    /// in the core's words; ignoring asks when it would clear waiting
    /// sessions, with their count. Asking first is a direct call. With no
    /// words from the core a change that needs them is not made: the
    /// confirmation is never shown without what it says.
    private func request(_ folder: TracesTree.FolderNode, _ mode: ProjectMode) {
        guard mode != folder.mode else { return }
        if mode == .autoUpload {
            guard let copy = ProjectArmingCopy.decode(fromJSON: TCCoreCopy.armingOfferCopyJSON(
                project: folder.label, count: 0)) else { return }
            confirming = Confirmation(folder: folder, mode: mode, words: .arm(copy))
        } else if mode == .ignore && !folder.sessions.isEmpty {
            guard let copy = ProjectIgnoreCopy.decode(fromJSON: TCCoreCopy.projectIgnoreCopyJSON(
                project: folder.label, pending: folder.sessions.count)) else { return }
            confirming = Confirmation(folder: folder, mode: mode, words: .ignore(copy))
        } else {
            apply(folder, mode)
        }
    }

    private func apply(_ folder: TracesTree.FolderNode, _ mode: ProjectMode) {
        confirming = nil
        Task { await store.setFolderMode(folder, mode, promised: folder.sessions.count) }
    }

    private func modePicker(_ folder: TracesTree.FolderNode) -> AnyView? {
        guard let mode = folder.mode, !folder.offerableModes.isEmpty else { return nil }
        return AnyView(
            GlassPicker(
                folder.label,
                selection: Binding(get: { mode }, set: { wanted in if let wanted { request(folder, wanted) } }),
                options: folder.offerableModes.map {
                    GlassPickerOption(ProjectCopy.modeChoiceLabel($0), value: $0, dot: Self.status($0))
                },
                placeholder: "—"
            )
            .disabled(store.writing.contains(folder.id)))
    }

    private static func status(_ mode: ProjectMode) -> GlassStatus {
        switch mode {
        case .ask: .ask
        case .autoUpload: .on
        case .ignore: .off
        }
    }

    // MARK: Keyboard

    /// The sessions in drawing order, skipping collapsed tools and folders.
    private var visibleSessions: [DaemonData.QueueEntry] {
        var sessions: [DaemonData.QueueEntry] = []
        for tool in store.tree.tools where isOpen(tool.id) {
            for folder in tool.folders where isOpen(folder.id) { sessions += folder.sessions }
        }
        for folder in store.tree.unplaced where isOpen(folder.id) { sessions += folder.sessions }
        return sessions
    }

    private func move(_ direction: MoveCommandDirection) {
        let sessions = visibleSessions
        guard !sessions.isEmpty else { return }
        let current = sessions.firstIndex { $0.entryId == selection }
        let next: Int
        switch direction {
        case .down: next = current.map { min($0 + 1, sessions.count - 1) } ?? 0
        case .up: next = current.map { max($0 - 1, 0) } ?? 0
        default: return
        }
        selection = sessions[next].entryId
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
            // The core's sentence for the declaration (an unset Claude Code
            // is read from its usual folder; unreadable settings say so),
            // after the count waiting.
            sub: Self.toolSub(tool),
            off: tool.mode == .off,
            expanded: tool.folders.isEmpty ? nil : isOpen(tool.id),
            // The source declaration, written through `setSource` after the
            // core's explanation. Unset and unknown draw no switch: never off.
            watched: tool.mode == .watch || tool.mode == .off
                ? Binding(get: { tool.mode == .watch }, set: { requestSource(tool, watch: $0) })
                : nil,
            watchDisabled: store.writing.contains(tool.kind.rawValue),
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
            // The mode is the picker's; the sub-line counts what is waiting.
            sub: folder.sessions.isEmpty ? nil : String(folder.sessions.count),
            expanded: folder.sessions.isEmpty ? nil : isOpen(folder.id),
            // The folder's mode: the three-way choice, with the core's
            // confirmations. A folder the core has not listed has no mode.
            accessory: modePicker(folder),
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

    static let reviewTitle = MonitorWords.review

    static func toolSub(_ tool: TracesTree.ToolNode) -> String? {
        let parts = [tool.waiting > 0 ? String(tool.waiting) : nil, TracesStore.sourceLine(tool)].compactMap { $0 }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

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

    /// The preview, keyed by the session it was asked for. Only the
    /// selected session's answer is ever read out of it.
    @State private var slot = PreviewSlot()
    private var summary: DaemonData.PreviewSummary? { slot.summary(for: entry?.entryId) }
    private var failure: DaemonDataError? { slot.failure(for: entry?.entryId) }

    /// How long a selection must rest before its preview is asked for. Each
    /// arrow-key step cancels the wait, so stepping through the tree starts
    /// no preview until it stops; `preview` is a full read-parse-redact pass
    /// the daemon cannot cancel.
    static let previewSettle: Duration = .milliseconds(300)

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
        slot.begin(entryId)
        do {
            try await Task.sleep(for: Self.previewSettle)
        } catch {
            return
        }
        let result: Result<DaemonData.PreviewSummary, DaemonDataError>
        do {
            result = .success(try await client.preview(entryId: entryId))
        } catch {
            result = .failure(error as? DaemonDataError ?? .undecodable(method: "preview"))
        }
        // A late answer for a session no longer selected is dropped.
        guard !Task.isCancelled else { return }
        slot.accept(entryId, result)
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
            .init(MonitorWords.tool, tool),
            .init(MonitorWords.folder, entry.projectLabel),
            .init(MonitorWords.started, entry.startedAt.map { $0.formatted(date: .abbreviated, time: .shortened) } ?? dash),
            .init(MonitorWords.length, entry.durationSecs.map {
                Duration.seconds($0).formatted(.units(allowed: [.hours, .minutes], width: .abbreviated))
            } ?? dash),
            .init(MonitorWords.prompts, number(entry.userTurns)),
            .init(MonitorWords.size, bytes(entry.sizeBytes)),
            .init(MonitorWords.sends, bytes(summary?.wouldSendBytes)),
            // Absent until scrubbed: a dash, never zero.
            .init(MonitorWords.marks, number(entry.marks)),
            .init(MonitorWords.unsure, number(entry.unsureSpans)),
        ]
    }
}
/// The inspector's one preview, keyed by the session it was asked for. A
/// result for any other session -- one that was selected, then left before
/// its blocking `preview` returned -- is refused, so it can never be shown,
/// or reviewed, as the selected session's.
struct PreviewSlot: Equatable {
    private(set) var entryId: String?
    private var summary: DaemonData.PreviewSummary?
    private var failure: DaemonDataError?

    mutating func begin(_ entryId: String) {
        self.entryId = entryId
        summary = nil
        failure = nil
    }

    /// Takes `result` if it is for the session asked for last.
    @discardableResult
    mutating func accept(_ entryId: String, _ result: Result<DaemonData.PreviewSummary, DaemonDataError>) -> Bool {
        guard entryId == self.entryId else { return false }
        switch result {
        case .success(let value): summary = value
        case .failure(let error): failure = error
        }
        return true
    }

    func summary(for entryId: String?) -> DaemonData.PreviewSummary? {
        entryId != nil && entryId == self.entryId ? summary : nil
    }

    func failure(for entryId: String?) -> DaemonDataError? {
        entryId != nil && entryId == self.entryId ? failure : nil
    }
}

/// The monitor's own words, in one place, while the core has none for
/// them. TODO(K4): every one of these moves to the Rust core's copy and is
/// read across the ABI, like the consent words already are; this enum is
/// the list to move, and nothing outside it may add a label. Debug-only,
/// with the monitor.
enum MonitorWords {
    static let review = "Review"
    static let tool = "Tool"
    static let folder = "Folder"
    static let started = "Started"
    static let length = "Length"
    static let prompts = "Prompts"
    static let size = "Size"
    static let sends = "Sends"
    static let marks = "Marks"
    static let unsure = "Unsure"
}
#endif
