import SwiftUI
import TCBridge
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
    /// Only for the queue's configured limit, which the queue-full banner
    /// names (the store's data contract does not carry it).
    @EnvironmentObject private var model: AppModel
    @Binding var selection: String
    /// A session's Review pill: select it and show the inspector its review
    /// lives in, so Review is never a press that does nothing visible.
    var onReview: (String) -> Void = { _ in }

    @State private var collapsed: Set<String> = []
    /// A mode change waiting on the core's confirmation: arming always,
    /// ignoring when the folder has sessions waiting. It carries the core's
    /// words for that folder, decoded when the change was asked for.
    @State private var confirming: Confirmation?
    /// A tool switch change waiting on the core's explanation of what the
    /// source declaration does.
    @State private var sourceChange: SourceChange?
    /// The folder whose "Submit all as" menu is open.
    @State private var verdictMenu: String?
    /// VoiceOver's focus, which follows the selection the arrow keys move,
    /// so a person hearing the tree hears where it went.
    @AccessibilityFocusState private var spoken: String?

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
            if let sample = store.sample, let words = store.words {
                // Debug builds draw sample data here; say so, and say when
                // the set asked for was not one.
                GlassChip("\(words.sample) · \(sample)", status: store.sampleUnknown ? .ask : .off)
            }
            if let notice = store.folderNotice {
                GlassNotice(tone: .ask) { Text(notice) }
            }
            // Why approved sessions are not moving, or that the core is not
            // answering, beside the tree and before Contribute is reached.
            // The last good tree stays below a failed refresh. An unread
            // status is never drawn as healthy.
            ForEach(TracesHealth.banners(
                phase: store.phase, status: store.status, words: store.words, coreDown: TracesHealth.coreDownLine,
                maxQueueEntries: model.daemonSettings?.maxQueueEntries)
            ) { GlassHealthBanner(banner: $0) }
            // Undo and the consent offers, here rather than in the
            // inspector, so they are on screen with the inspector hidden.
            TracesOffersBar(store: store)
            if store.phase == .loading && isEmpty {
                ProgressView().controlSize(.small).frame(maxWidth: .infinity)
            } else if isEmpty && store.phase == .loaded {
                VStack(spacing: GlassTokens.Space.s2) {
                    Image(systemName: "tray")
                        .glassGlyph(22)
                        .foregroundStyle(GlassColor.textTertiary)
                        .accessibilityLabel(MonitorWindowView.Tab.traces.title)
                    Text(QueueLegacyWords.nothingWaiting)
                        .glassType(GlassTokens.TypeScale.bodyStrong)
                        .foregroundStyle(GlassColor.textPrimary)
                    Text(QueueLegacyWords.nothingWaitingDetail)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .multilineTextAlignment(.center)
                .frame(maxWidth: .infinity)
                .padding(.top, GlassTokens.Space.s10)
            } else {
                ScrollViewReader { proxy in
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
                    // The keyboard moves the selection; keep it on screen.
                    .onChange(of: selection) { _, selected in
                        guard !selected.isEmpty else { return }
                        withAnimation(GlassMotion.fast(GlassMotion.systemReducesMotion)) {
                            proxy.scrollTo(selected)
                        }
                        spoken = selected
                    }
                }
                .scrollIndicators(.never)
                // Full Keyboard Access: focus the tree, then the arrow keys
                // move the selection through the sessions as drawn.
                .focusable()
                .onMoveCommand(perform: move)
                // Return opens the selected session's review, so the pill
                // need not be its own tab stop on every row.
                .onKeyPress(.return) {
                    guard !selection.isEmpty else { return .ignored }
                    onReview(selection)
                    return .handled
                }
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
            guard let path = FolderPanel.choose() else { return }
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
        case .left, .right:
            disclose(selection, open: direction == .right)
            return
        @unknown default: return
        }
        selection = sessions[next].entryId
    }

    /// Left collapses the selected session's folder; right expands it and
    /// its tool. The selection stays, so the two undo each other and the
    /// inspector keeps the session.
    private func disclose(_ entryId: String, open: Bool) {
        guard let path = TracesTreeView.path(to: entryId, in: store.tree) else { return }
        if open {
            if let tool = path.tool { collapsed.remove(tool) }
            collapsed.remove(path.folder)
        } else {
            collapsed.insert(path.folder)
        }
    }

    /// The tool (if placed under one) and folder holding a session.
    static func path(to entryId: String, in tree: TracesTree) -> (tool: String?, folder: String)? {
        for tool in tree.tools {
            for folder in tool.folders where folder.sessions.contains(where: { $0.entryId == entryId }) {
                return (tool.id, folder.id)
            }
        }
        for folder in tree.unplaced where folder.sessions.contains(where: { $0.entryId == entryId }) {
            return (nil, folder.id)
        }
        return nil
    }

    private var isEmpty: Bool { store.tree.tools.isEmpty && store.tree.unplaced.isEmpty }

    private func isOpen(_ id: String) -> Bool { !collapsed.contains(id) }

    private func toggle(_ id: String) {
        if collapsed.contains(id) { collapsed.remove(id) } else { collapsed.insert(id) }
    }

    @ViewBuilder
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
        notes(refusal(tool.id).map { [$0] } ?? [], depth: .tool)
    }

    /// The core's line for a write it refused on this row, if one is held.
    private func refusal(_ id: String) -> String? {
        store.writeErrors[id].flatMap { store.words?.line(for: $0) }
    }

    /// What a folder says under its row, whether open or not: a refused
    /// write; the core's disclosure for an armed folder, chosen by the
    /// daemon; and why the bucket can never be armed.
    func folderNotes(_ folder: TracesTree.FolderNode) -> [String] {
        var lines: [String] = []
        if let refused = refusal(folder.id) { lines.append(refused) }
        lines += store.disclosureLines(folder.disclosure)
        if folder.isBucket { lines.append(ProjectCopy.unresolvedBucketNote) }
        return lines
    }

    /// Caption lines under a row, indented to its content.
    @ViewBuilder
    private func notes(_ lines: [String], depth: GlassListRow.Depth) -> some View {
        if !lines.isEmpty {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                ForEach(lines, id: \.self) { line in
                    Text(line)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.leading, 8 + 16 + GlassTokens.Space.s4 + CGFloat(depth.rawValue) * 18)
            .padding(.trailing, 6)
            .padding(.bottom, GlassTokens.Space.s2)
        }
    }

    @ViewBuilder
    private func folderRows(_ folder: TracesTree.FolderNode) -> some View {
        // Submit all and Submit all as are a consent action: drawn only when
        // the shared table offers Contribute for the daemon's counts, never
        // for a folder that is not sent, and removed rather than disabled.
        let offer = store.groupOffer(folder)
        let submits = offer.offersContribute && store.mayContributeFolder(folder)
        GlassListRow(
            depth: .folder,
            tile: .folder,
            title: folder.label,
            // The mode is the picker's; the sub-line counts what is waiting.
            sub: folder.sessions.isEmpty ? nil : String(folder.sessions.count),
            expanded: folder.sessions.isEmpty ? nil : isOpen(folder.id),
            submitTitle: submits ? QueueFolderWords.submitAll(offer.count) : nil,
            // The folder's mode: the three-way choice, with the core's
            // confirmations. A folder the core has not listed has no mode.
            accessory: modePicker(folder),
            expandLabel: folder.label,
            menuLabel: submits ? VerdictCopy.submitAllAs : "",
            menuOpen: verdictMenu == folder.id,
            onToggleExpand: { toggle(folder.id) },
            onSubmit: submits ? { Task { await store.contributeFolder(folder, verdict: nil) } } : nil,
            onMenu: submits ? { verdictMenu = verdictMenu == folder.id ? nil : folder.id } : nil
        )
        .help(submits ? QueueFolderWords.submitAllHelp(folder.label) : "")
        .disabled(store.writing.contains(folder.id))
        if submits, verdictMenu == folder.id {
            GlassMenu(onDismiss: { verdictMenu = nil }) {
                ForEach(ContributorVerdict.allCases, id: \.rawValue) { option in
                    GlassMenuItem(option.label) {
                        verdictMenu = nil
                        Task { await store.contributeFolder(folder, verdict: option) }
                    }
                }
            }
            .help(VerdictCopy.submitAllAsTooltip)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.leading, 8 + 16 + GlassTokens.Space.s4 + CGFloat(GlassListRow.Depth.folder.rawValue) * 18)
            .padding(.bottom, GlassTokens.Space.s2)
        }
        // The withheld line is drawn once: the tab's notice may already say it.
        let withheld: [String] = offer.withheldLine.flatMap { $0 == store.folderNotice ? nil : [$0] } ?? []
        notes(folderNotes(folder) + withheld, depth: .folder)
        if isOpen(folder.id) {
            ForEach(folder.sessions) { sessionRow($0) }
        }
    }

    private func sessionRow(_ entry: DaemonData.QueueEntry) -> some View {
        GlassListRow(
            depth: .session,
            tile: .session,
            title: Self.when(entry),
            sub: Self.sub(entry, held: store.words?.held, ineligible: store.ineligibleLine(entry)),
            flag: Self.flag(entry, ineligible: store.ineligibleLine(entry) != nil),
            selected: selection == entry.entryId,
            // D10 default: a session's pill opens its review. Focus roves:
            // only the selected row's pill is a tab stop; Return opens it.
            submitTitle: store.words?.review,
            submitFocusable: selection == entry.entryId,
            onSelect: { selection = entry.entryId },
            onSubmit: { onReview(entry.entryId) }
        )
        .id(entry.entryId)
        .accessibilityFocused($spoken, equals: entry.entryId)
    }


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

    /// Held when a person has to look before it can go: a second-look hold,
    /// Manual Scrub check, or back from Keep.
    static func isHeld(_ entry: DaemonData.QueueEntry) -> Bool {
        entry.heldForSecondLook || entry.heldByManualScrubCheck || entry.returnedFromKeep
    }

    /// Amber when held, or when the core says the session cannot go as it
    /// stands.
    static func flag(_ entry: DaemonData.QueueEntry, ineligible: Bool = false) -> GlassListRow.Flag? {
        isHeld(entry) || ineligible ? .ask : nil
    }

    /// A session's sub-line. The amber is never the only signal: a held
    /// session says the core's word for it, and one that cannot be
    /// contributed says the core's sentence, before its measures.
    static func sub(_ entry: DaemonData.QueueEntry, held: String?, ineligible: String?) -> String? {
        let parts = [isHeld(entry) ? held : nil, ineligible, measures(entry)].compactMap { $0 }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }
}

/// The inspector for the selected session: what the core reports about it
/// (R6), and its review (R7): what would leave this Mac, the consent gate in
/// the core's words, and Contribute, Keep or Dismiss.
struct SessionInspectorView: View {
    let store: TracesStore
    let entry: DaemonData.QueueEntry?
    /// The tab's words, from the core, decoded once by the store.
    private var words: MonitorTracesCopy? { store.words }
    /// The legacy queue's entries, which the preview sheet still takes.
    @EnvironmentObject private var model: AppModel
    /// The session whose preview sheet is open, as the legacy queue holds it.
    @State private var previewing: QueueEntry?

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
    /// The consent gate, from the Rust core, decoded once by the store.
    private var consent: ConsentCopy? { store.consent }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            if let entry {
                ScrollView {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                        Text(summary?.title ?? TracesTreeView.when(entry))
                            .glassType(GlassTokens.TypeScale.title)
                            .foregroundStyle(GlassColor.textPrimary)
                            .lineLimit(3)
                        if let failure, let line = words?.line(for: failure) {
                            GlassNotice(tone: .outside, title: line) { EmptyView() }
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
                        if let words {
                            GlassKeyValueList(Self.rows(
                                entry, summary, words: words,
                                eligibility: store.eligibilityValue(entry), attestation: store.attestationValue(entry)))
                        }
                        if let summary { redactions(summary) }
                        queueCardFacts(entry)
                        if let reasons = entry.secondLook, !reasons.isEmpty {
                            // The core's fixed reason labels until K4 gives
                            // them words.
                            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                                ForEach(reasons, id: \.self) { GlassChip($0, status: .ask) }
                            }
                        }
                        // The full review, on the legacy queue's entry for
                        // this same session. Absent, not disabled, when the
                        // legacy queue does not hold it: no sheet for another.
                        if let legacy = QueueEntryBridge.legacyEntry(for: entry.entryId, in: model.awaitingDecision) {
                            Button(QueueLegacyWords.lookInside) { previewing = legacy }
                                .buttonStyle(GlassButtonStyle(.glass))
                        }
                        review(entry)
                    }
                }
                .scrollIndicators(.never)
                .task(id: entry.entryId) { await load(entry.entryId) }
            } else {
                Spacer(minLength: 0)
            }
        }
        .sheet(item: $previewing) { PreviewSheet(entry: $0).environmentObject(model) }
        .onChange(of: model.awaitingDecision.count) { _, _ in
            // Development hook, as the legacy queue's: opens the first
            // preview so the sheet can be captured. Never on by default.
            if ProcessInfo.processInfo.environment["TRACE_COMMONS_DEMO_PREVIEW"] == "1",
                previewing == nil,
                let first = model.awaitingDecision.first
            {
                previewing = first
            }
        }
    }

    /// What the queue card said about this session that the rows above do
    /// not: a secret the scan found and left in, what scrubbing did and
    /// what that does not prove, and whether delegated subagent transcripts
    /// were trimmed to fit. Each line carries its words beside its dot.
    @ViewBuilder
    private func queueCardFacts(_ entry: DaemonData.QueueEntry) -> some View {
        if let survivor = TracesStore.survivorLine(summary) {
            GlassStatusLabel(survivor, status: .ask)
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityLabel(survivor)
        }
        // Only once the preview has counted: an unread count is never
        // read as "nothing matched".
        if let redactions = summary?.redactions {
            let removed = RedactionLabels.removedTotal(redactions)
            GlassStatusLabel(
                ScrubbingCaveat.rowLine(redactionCount: removed),
                status: ScrubbingCaveat.status(redactionCount: removed))
                .fixedSize(horizontal: false, vertical: true)
        }
        // A load-time fact on the entry, so it is said before the preview
        // is in: a trimmed conversation never reaches a decision unsaid.
        if let line = SubagentCopy.line(count: entry.subagentCount ?? 0, dropped: entry.subagentsDropped ?? 0) {
            if (entry.subagentsDropped ?? 0) > 0 {
                GlassStatusLabel(line, status: .ask)
                    .fixedSize(horizontal: false, vertical: true)
            } else {
                Text(line)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    /// What scrubbing removed, and what it found but left in, as the core
    /// groups, splits and words it (`tc_redaction_summary_json`), the way
    /// the review sheet renders it. No counts, or an answer that will not
    /// parse, lists nothing rather than claiming nothing matched. Distinct
    /// counts are sent only with a full summary; absent, the core reads them
    /// as none.
    @ViewBuilder
    private func redactions(_ summary: DaemonData.PreviewSummary) -> some View {
        if let occurrences = summary.redactions,
            let rows = RedactionSummary.rows(fromJSON: TCCoreCopy.redactionSummaryJSON(
                occurrences: occurrences, distinct: summary.redactionsDistinct ?? [:]))
        {
            redactionRows(rows)
        }
    }

    @ViewBuilder
    private func redactionRows(
        _ rows: (removed: [RedactionSummaryRow], stillPresent: [RedactionSummaryRow])
    ) -> some View {
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
        // The scrubbing caveat, repeated at the commit as the review sheet
        // repeats it, then the gate statement, at reading weight: they sit
        // directly above an irreversible button.
        ScrubbingCaveatAtCommit()
        if let consent {
            Text(consent.gateStatement)
                .glassType(GlassTokens.TypeScale.label)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        // Why this session cannot be contributed, beside the disarmed
        // button, in the shared table's words.
        if let eligibility = TracesStore.eligibility(entry),
            !EligibilitySurface.offersContribute(eligibility, calls: TracesStore.eligibilityCalls),
            let reason = eligibility.reason,
            let line = TracesStore.eligibilityCalls.reasonLine(reason)
        {
            Text(line)
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        TracesRefusal(store: store, entryId: entry.entryId)
        if let words {
            HStack(spacing: GlassTokens.Space.s4) {
                Button(words.dismiss) { act(.dismiss, entry) }
                    .buttonStyle(GlassButtonStyle(.glass))
                Button(words.keep) { act(.keep, entry) }
                    .buttonStyle(GlassButtonStyle(.glass))
                Spacer(minLength: 0)
                Button(words.contribute) { act(.contribute, entry) }
                    .buttonStyle(GlassButtonStyle(.primary, small: true))
                    .disabled(!armed(entry))
                    // Why it is armed or not, in the core's words, as the
                    // review sheet's Contribute says it.
                    .help(consent == nil ? "" : TCConsentCopy.gateHelp(pinned: armed(entry)) ?? "")
            }
            .disabled(busy)
        }
    }

    /// Contribute's gate for `entry`, on the preview asked for it.
    private func armed(_ entry: DaemonData.QueueEntry) -> Bool {
        TracesStore.contributeArmed(
            enrolled: slot.summary(for: entry.entryId)?.enrolled, consent: consent,
            eligibility: TracesStore.eligibility(entry), calls: TracesStore.eligibilityCalls)
    }

    private func act(_ action: TracesStore.ReviewAction, _ entry: DaemonData.QueueEntry) {
        if action == .contribute {
            // Asked again at the press, on the session as the tree now has
            // it: the summary or the eligibility may have moved since the
            // button was drawn.
            guard let live = store.tree.allSessions.first(where: { $0.entryId == entry.entryId }),
                armed(live)
            else { return }
        }
        Task { await store.perform(action, on: entry.entryId) }
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
            result = .success(try await store.attached().preview(entryId: entryId))
        } catch {
            result = .failure(error as? DaemonDataError ?? .undecodable(method: "preview"))
        }
        // A late answer for a session no longer selected is dropped.
        guard !Task.isCancelled else { return }
        slot.accept(entryId, result)
    }

    /// One word a row, and only what the daemon reported. Unknown is a dash.
    static func rows(
        _ entry: DaemonData.QueueEntry, _ summary: DaemonData.PreviewSummary?, words: MonitorTracesCopy,
        eligibility: String? = nil, attestation: String? = nil
    ) -> [GlassKeyValueList.Item] {
        let dash = "—"
        func bytes(_ value: Int?) -> String {
            value.map { ByteCountFormatter.string(fromByteCount: Int64($0), countStyle: .file) } ?? dash
        }
        func number(_ value: Int?) -> String { value.map(String.init) ?? dash }
        let tool = SourceKind(rawValue: entry.declaredSource ?? entry.source)?.displayName ?? entry.source
        var rows: [GlassKeyValueList.Item] = [
            .init(words.tool, tool),
            .init(words.folder, entry.projectLabel),
            .init(words.started, entry.startedAt.map { $0.formatted(date: .abbreviated, time: .shortened) } ?? dash),
            .init(words.length, entry.durationSecs.map {
                Duration.seconds($0).formatted(.units(allowed: [.hours, .minutes], width: .abbreviated))
            } ?? dash),
            .init(words.prompts, number(entry.userTurns)),
            .init(words.size, bytes(entry.sizeBytes)),
            .init(words.sends, bytes(summary?.wouldSendBytes)),
            // Absent until scrubbed: a dash, never zero.
            .init(words.marks, number(entry.marks)),
            .init(words.unsure, number(entry.unsureSpans)),
        ]
        // Whether it may go, and what the witness attested, in the core's
        // sentences; left out when the core has nothing to say.
        if let eligibility { rows.append(.init(words.eligibility, eligibility)) }
        if let attestation { rows.append(.init(words.attestation, attestation)) }
        // Only the full preview carries these. Categories only: the matched
        // text is never reported. The risk is the core's label.
        if let labels = summary?.piiLabelsPresent, !labels.isEmpty {
            rows.append(.init(words.personalInformation, labels.joined(separator: ", ")))
        }
        if let risk = summary?.residualRisk, !risk.isEmpty {
            rows.append(.init(words.residualRisk, risk.replacingOccurrences(of: "_", with: " ")))
        }
        return rows
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

/// The preview sheet still takes the legacy queue's entry. The inspector's
/// session is found there by id, and only by id: a session the legacy
/// queue does not hold opens no sheet, never a sheet for another session.
enum QueueEntryBridge {
    static func legacyEntry(for entryId: String, in awaiting: [QueueEntry]) -> QueueEntry? {
        awaiting.first { $0.entryID == entryId }
    }
}

