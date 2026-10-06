#if DEBUG
import SwiftUI
import TCBridge
import TCDesign
import TCBridge
import TCShellCore

/// The Traces tab (R6 of #1173): Ron's #1146 tree without the tool level
/// (owner, 2026-10-05). Folders at the top, sessions under each, and every
/// session row showing its own tool, since a folder can hold sessions from
/// several tools. Watching a tool is Settings', not the tree's.
///
/// Every label is a single word, a formatted number or date, a name the
/// core reports, or the core's Monitor words (`MonitorTracesCopy`, the
/// disclosure bundle's folder mode names) (`ShellWordingTests`).
struct TracesTreeView: View {
    let store: TracesStore
    /// The folder or session selected; nil for none.
    @Binding var selection: MonitorSelection?
    /// A session's Review pill: select it and show the inspector its review
    /// lives in, so Review is never a press that does nothing visible.
    var onReview: (String) -> Void = { _ in }

    @State private var collapsed: Set<String> = []
    /// A mode change waiting on the core's confirmation: arming always,
    /// ignoring when the folder has sessions waiting. It carries the core's
    /// words for that folder, decoded when the change was asked for.
    @State private var confirming: Confirmation?
    /// The folder whose row menu (Ignore folder) is open.
    @State private var folderMenu: String?
    /// The session whose row menu (Dismiss session) is open.
    @State private var sessionMenu: String?
    /// The session whose Dismiss confirmation is open (Ron's
    /// `SessionRow`): dismissing asks first, and only its button dismisses.
    @State private var dismissing: DaemonData.QueueEntry?
    /// VoiceOver's focus, which follows the selection the arrow keys move,
    /// so a person hearing the tree hears where it went.
    @AccessibilityFocusState private var spoken: String?

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

    /// What asking for a folder mode does: nothing for the mode it has; a
    /// direct write; a confirmation in the core's words first; or nothing
    /// at all when the core's words for that confirmation are missing.
    enum ModeChange {
        case noop
        case apply
        case confirm(Confirmation)
        case unavailable
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
            // A core that does not answer, or a read it refused, is said
            // beside the tree too, so the pane never reads as current over a
            // last good tree with the inspector hidden. The safeguards and
            // the prompts are the inspector's (`TracesInspectorHost`).
            if case .failed(let error) = store.phase, let line = store.words?.line(for: error) {
                GlassNotice(tone: .outside, title: line) { EmptyView() }
            }
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
                            ForEach(store.tree.folders) { folderRows($0) }
                        }
                    }
                    // The keyboard moves the selection; keep it on screen.
                    .onChange(of: selection) { _, selected in
                        guard let id = selected.map(Self.rowID) else { return }
                        withAnimation(GlassMotion.fast(GlassMotion.systemReducesMotion)) {
                            proxy.scrollTo(id)
                        }
                        spoken = id
                    }
                }
                .scrollIndicators(.never)
                // Full Keyboard Access: focus the tree, then the arrow keys
                // move the selection through the folders and sessions as drawn.
                .focusable()
                .onMoveCommand(perform: move)
                // Return opens the selected session's review, so the pill
                // need not be its own tab stop on every row.
                .onKeyPress(.return) {
                    guard case .session(let entryID) = selection else { return .ignored }
                    onReview(entryID)
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
        .sheet(item: $dismissing) { dismissSheet($0) }
    }

    // MARK: Folder modes

    /// The route every folder mode change takes, from the tree's switch and
    /// menu and from the Folder inspector's picker alike. Arming always asks
    /// first, in the core's words; ignoring asks when it would clear waiting
    /// sessions, with their count; Ask me is a direct call. With no words
    /// from the core a change that needs them is not made: the confirmation
    /// is never shown without what it says.
    static func modeChange(_ folder: TracesTree.FolderNode, _ mode: ProjectMode) -> ModeChange {
        guard mode != folder.mode else { return .noop }
        if mode == .autoUpload {
            guard let copy = ProjectArmingCopy.decode(fromJSON: TCCoreCopy.armingOfferCopyJSON(
                project: folder.label, count: 0)) else { return .unavailable }
            return .confirm(Confirmation(folder: folder, mode: mode, words: .arm(copy)))
        }
        if mode == .ignore && !folder.sessions.isEmpty {
            guard let copy = ProjectIgnoreCopy.decode(fromJSON: TCCoreCopy.projectIgnoreCopyJSON(
                project: folder.label, pending: folder.sessions.count)) else { return .unavailable }
            return .confirm(Confirmation(folder: folder, mode: mode, words: .ignore(copy)))
        }
        return .apply
    }

    private func request(_ folder: TracesTree.FolderNode, _ mode: ProjectMode) {
        switch Self.modeChange(folder, mode) {
        case .noop, .unavailable: return
        case .apply: apply(folder, mode)
        case .confirm(let pending): confirming = pending
        }
    }

    private func apply(_ folder: TracesTree.FolderNode, _ mode: ProjectMode) {
        confirming = nil
        Task { await store.setFolderMode(folder, mode, promised: folder.sessions.count) }
    }

    /// A folder row's switch (Ron's watch switch): on is Ask me, off is
    /// Never, which asks first when sessions are waiting. An ignored folder
    /// is turned back on here.
    static func watchChoice(_ on: Bool) -> ProjectMode {
        on ? .ask : .ignore
    }

    static func status(_ mode: ProjectMode) -> GlassStatus {
        switch mode {
        case .ask: .ask
        case .autoUpload: .on
        case .ignore: .off
        }
    }

    // MARK: Keyboard

    /// The selectable rows in drawing order, folders and sessions both,
    /// skipping what a collapsed folder hides. A collapsed folder keeps its
    /// own row.
    static func visibleRows(in tree: TracesTree, collapsed: Set<String>) -> [MonitorSelection] {
        var rows: [MonitorSelection] = []
        for folder in tree.folders {
            rows.append(.folder(projectID: folder.id))
            if !collapsed.contains(folder.id) {
                rows += folder.sessions.map { .session(entryID: $0.entryId) }
            }
        }
        return rows
    }

    /// One step up or down from the current row; the top row when nothing
    /// drawn is selected, and the ends hold.
    static func moved(from current: MonitorSelection?, down: Bool, through rows: [MonitorSelection]) -> MonitorSelection? {
        guard !rows.isEmpty else { return nil }
        guard let at = current.flatMap({ rows.firstIndex(of: $0) }) else { return rows[0] }
        return rows[down ? min(at + 1, rows.count - 1) : max(at - 1, 0)]
    }

    /// A row click, as Ron's tree: the selected row again selects nothing
    /// (the Summary), any other row selects that row.
    static func toggled(_ clicked: MonitorSelection, current: MonitorSelection?) -> MonitorSelection? {
        clicked == current ? nil : clicked
    }

    private func move(_ direction: MoveCommandDirection) {
        switch direction {
        case .down, .up:
            let rows = Self.visibleRows(in: store.tree, collapsed: collapsed)
            if let next = Self.moved(from: selection, down: direction == .down, through: rows), next != selection {
                selection = next
            }
        case .left, .right:
            if let selection { disclose(selection, open: direction == .right) }
        @unknown default: return
        }
    }

    /// Left collapses the selected session's folder, or the selected
    /// folder; right expands it. The selection stays, so the two undo each
    /// other and the inspector keeps what it shows.
    private func disclose(_ selected: MonitorSelection, open: Bool) {
        guard let folder = TracesTreeView.folder(of: selected, in: store.tree) else { return }
        if open {
            collapsed.remove(folder)
        } else {
            collapsed.insert(folder)
        }
    }

    /// The folder holding a selection: the folder itself, or the session's
    /// folder; nil when it is not in the tree.
    static func folder(of selected: MonitorSelection, in tree: TracesTree) -> String? {
        switch selected {
        case .session(let entryID):
            return folder(of: entryID, in: tree)
        case .folder(let projectID):
            return tree.folders.contains { $0.id == projectID } ? projectID : nil
        }
    }

    /// The row id a selection scrolls to and VoiceOver focuses.
    static func rowID(_ selected: MonitorSelection) -> String { selected.rawValue }

    /// The folder holding a session.
    static func folder(of entryId: String, in tree: TracesTree) -> String? {
        tree.folders.first { $0.sessions.contains { $0.entryId == entryId } }?.id
    }

    private var isEmpty: Bool { store.tree.folders.isEmpty }

    private func isOpen(_ id: String) -> Bool { !collapsed.contains(id) }

    private func toggle(_ id: String) {
        if collapsed.contains(id) { collapsed.remove(id) } else { collapsed.insert(id) }
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
            .padding(.leading, Self.indent(depth))
            .padding(.trailing, 6)
            .padding(.bottom, GlassTokens.Space.s2)
        }
    }

    /// Where a row's content starts, for the lines and menus under it.
    private static func indent(_ depth: GlassListRow.Depth) -> CGFloat {
        8 + 16 + GlassTokens.Space.s4 + CGFloat(depth.rawValue) * 18
    }

    @ViewBuilder
    private func folderRows(_ folder: TracesTree.FolderNode) -> some View {
        let words = store.words
        let ignored = folder.mode == .ignore
        // Submit all is a consent action: drawn only when the shared table
        // offers Contribute for the daemon's counts, never for a folder that
        // is not sent, and removed rather than disabled. Submit all as is
        // the Folder inspector's modal.
        let offer = store.groupOffer(folder)
        let submits = offer.offersContribute && store.mayContributeFolder(folder)
        let busy = store.submittingFolder == folder.id
        // The switch and the Ignore menu need a mode the core listed and
        // both ends of the switch among the modes it accepts.
        let switchable = folder.mode != nil && folder.offerableModes.contains(.ask)
            && folder.offerableModes.contains(.ignore)
        let ignorable = switchable && !ignored
        GlassListRow(
            depth: .folder,
            tile: .folder,
            title: folder.label,
            // The mode's word and the count waiting; an ignored folder says so.
            sub: words.flatMap {
                Self.folderSub(folder, words: $0, modeLabels: store.disclosure?.folderModeLabels ?? [:])
            },
            selected: selection == .folder(projectID: folder.id),
            off: folder.mode == .ignore,
            expanded: folder.sessions.isEmpty ? nil : isOpen(folder.id),
            submitTitle: submits ? words.map { busy ? $0.tree.submitting : Self.submitTitle(offer.count, words: $0) } : nil,
            watched: switchable
                ? Binding(get: { folder.mode != .ignore }, set: { request(folder, Self.watchChoice($0)) })
                : nil,
            watchDisabled: store.writing.contains(folder.id),
            watchLabel: words?.tree.watchFolder ?? "",
            expandLabel: folder.label,
            menuLabel: ignorable ? words?.tree.ignoreFolder ?? "" : "",
            menuOpen: folderMenu == folder.id,
            onToggleExpand: { toggle(folder.id) },
            onSelect: { selection = Self.toggled(.folder(projectID: folder.id), current: selection) },
            onSubmit: submits && !busy ? { Task { await store.contributeFolder(folder, verdict: nil) } } : nil,
            onMenu: ignorable ? { folderMenu = folderMenu == folder.id ? nil : folder.id } : nil
        )
        .help(submits ? Self.submitHelp(offer.withheldLine, words: words) : "")
        .disabled(store.writing.contains(folder.id))
        .id(Self.rowID(.folder(projectID: folder.id)))
        .accessibilityFocused($spoken, equals: Self.rowID(.folder(projectID: folder.id)))
        if ignorable, folderMenu == folder.id, let label = words?.tree.ignoreFolder {
            GlassMenu(onDismiss: { folderMenu = nil }) {
                GlassMenuItem(label) {
                    folderMenu = nil
                    request(folder, .ignore)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.leading, Self.indent(.folder))
            .padding(.bottom, GlassTokens.Space.s2)
        }
        // The withheld line is drawn once: the tab's notice may already say it.
        let withheld: [String] = ignored
            ? [] : offer.withheldLine.flatMap { $0 == store.folderNotice ? nil : [$0] } ?? []
        notes(folderNotes(folder) + withheld, depth: .folder)
        if isOpen(folder.id) {
            ForEach(folder.sessions) { sessionRow($0) }
        }
    }

    @ViewBuilder
    private func sessionRow(_ entry: DaemonData.QueueEntry) -> some View {
        let words = store.words
        let selected = selection == .session(entryID: entry.entryId)
        GlassListRow(
            depth: .session,
            tile: Self.sessionTile(entry),
            title: Self.when(entry),
            sub: Self.sub(
                entry, held: words?.held, ineligible: store.ineligibleLine(entry), tool: Self.toolName(entry)),
            flag: Self.flag(entry, ineligible: store.ineligibleLine(entry) != nil),
            selected: selected,
            // D10 default: a session's pill opens its review. Focus roves:
            // only the selected row's pill is a tab stop; Return opens it.
            submitTitle: selected ? words?.tree.reviewing : words?.review,
            submitFocusable: selected,
            menuLabel: words?.tree.dismissSession ?? "",
            menuOpen: sessionMenu == entry.entryId,
            onSelect: { selection = Self.toggled(.session(entryID: entry.entryId), current: selection) },
            onSubmit: { onReview(entry.entryId) },
            onMenu: { sessionMenu = sessionMenu == entry.entryId ? nil : entry.entryId }
        )
        .help(words?.tree.reviewTip ?? "")
        .id(Self.rowID(.session(entryID: entry.entryId)))
        .accessibilityFocused($spoken, equals: Self.rowID(.session(entryID: entry.entryId)))
        if sessionMenu == entry.entryId, let label = words?.tree.dismissSession {
            GlassMenu(onDismiss: { sessionMenu = nil }) {
                GlassMenuItem(label) {
                    sessionMenu = nil
                    dismissing = entry
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.leading, Self.indent(.session))
            .padding(.bottom, GlassTokens.Space.s2)
        }
    }

    /// Ron's Dismiss-session confirmation: the session named by when and
    /// size, Keep it, and Dismiss, which is the only dismiss the tree has.
    /// A refusal is said here, and the confirmation stays open over it.
    @ViewBuilder
    private func dismissSheet(_ entry: DaemonData.QueueEntry) -> some View {
        if let words = store.words {
            let busy = store.acting.contains(entry.entryId)
            GlassSheet(title: words.tree.dismissSessionTitle, subtitle: Self.dismissBody(entry, words: words)) {
                if let refused = store.actionError, refused.entryId == entry.entryId {
                    GlassNotice(tone: .outside, title: words.tree.dismissSessionFailed) {
                        Text(words.line(for: refused.error))
                    }
                }
                HStack(spacing: GlassTokens.Space.s4) {
                    Spacer(minLength: 0)
                    Button(words.tree.dismissSessionKeep) { dismissing = nil }
                        .buttonStyle(GlassButtonStyle(.glass))
                        .keyboardShortcut(.cancelAction)
                    Button(busy ? words.tree.dismissing : words.dismissAction) {
                        Task {
                            await store.perform(.dismiss, on: entry.entryId)
                            guard store.actionError?.entryId != entry.entryId else { return }
                            dismissing = nil
                            if selection == .session(entryID: entry.entryId) { selection = nil }
                        }
                    }
                    .buttonStyle(GlassButtonStyle(.glass))
                }
                .disabled(busy)
            }
            .frame(minWidth: 360)
        }
    }

    // MARK: Formatting

    static func glassTool(_ kind: SourceKind) -> GlassTool {
        kind.glassTool
    }

    /// The tool a session came from, as the tree's majority rule reads it:
    /// the declared source first, then the adapter's.
    static func tool(_ entry: DaemonData.QueueEntry) -> SourceKind? {
        SourceKind(rawValue: entry.declaredSource ?? entry.source) ?? SourceKind(rawValue: entry.source)
    }

    /// A session row's tile: its tool's, or the plain session tile for a
    /// source this build does not know.
    static func sessionTile(_ entry: DaemonData.QueueEntry) -> GlassToolTile.Kind {
        tool(entry).map { .tool(glassTool($0)) } ?? .session
    }

    /// The tool's name for a session's sub-line, or nil for an unknown one.
    static func toolName(_ entry: DaemonData.QueueEntry) -> String? {
        tool(entry)?.displayName
    }

    /// A folder row's sub-line (Ron's `FolderBranch`): the mode's word and
    /// the count waiting, or the core's line for an ignored folder. A
    /// folder the core does not list has no mode word.
    static func folderSub(
        _ folder: TracesTree.FolderNode, words: MonitorTracesCopy, modeLabels: [String: String]
    ) -> String? {
        if folder.mode == .ignore { return words.tree.ignoredFolder }
        let count = folder.sessions.count == 1
            ? words.counts.sessionsWaitingOne
            : FirstRunCopy.fill(words.counts.sessionsWaiting, ["count": String(folder.sessions.count)])
        let parts = [folder.mode.flatMap { modeLabels[$0.rawValue] }, count].compactMap { $0 }
        return parts.joined(separator: " · ")
    }

    /// The folder's Submit pill, with the count the daemon says it sends.
    static func submitTitle(_ count: Int, words: MonitorTracesCopy) -> String {
        FirstRunCopy.fill(words.tree.submitCount, ["count": String(count)])
    }

    /// The pill's help: what it sends, then what it leaves behind, both the
    /// core's.
    static func submitHelp(_ withheld: String?, words: MonitorTracesCopy?) -> String {
        [words?.tree.submitTip, withheld].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " ")
    }

    /// The Dismiss confirmation's body: the core's sentence with the
    /// session's time and size filled in.
    static func dismissBody(_ entry: DaemonData.QueueEntry, words: MonitorTracesCopy) -> String {
        let size = entry.sizeBytes.map { ByteCountFormatter.string(fromByteCount: Int64($0), countStyle: .file) } ?? "—"
        return FirstRunCopy.fill(words.tree.dismissSessionBody, ["when": when(entry), "size": size])
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

    /// A session's sub-line: its tool's name first, since a folder holds
    /// several tools' sessions. The amber is never the only signal: a held
    /// session says the core's word for it, and one that cannot be
    /// contributed says the core's sentence, before its measures.
    static func sub(_ entry: DaemonData.QueueEntry, held: String?, ineligible: String?, tool: String? = nil) -> String? {
        let parts = [tool, isHeld(entry) ? held : nil, ineligible, measures(entry)].compactMap { $0 }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
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

#endif
