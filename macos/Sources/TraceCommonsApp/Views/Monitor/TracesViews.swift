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
    /// The queue's configured limit, which the queue-full banner names,
    /// and the prompts drawn above the tree.
    @EnvironmentObject private var model: AppModel
    /// The folder or session selected; nil for none.
    @Binding var selection: MonitorSelection?
    /// A session's Review pill: select it and show the inspector its review
    /// lives in, so Review is never a press that does nothing visible.
    var onReview: (String) -> Void = { _ in }

    /// The folders opened: every folder starts collapsed (#1146
    /// `traces-workspace.tsx`, `expanded = []`), and one opens when a
    /// session in it is selected from elsewhere (#1146 `reveal`).
    @State private var expanded: Set<String> = []
    /// The selection last revealed, so a folder the person closes over a
    /// selected session stays closed until the selection moves.
    @State private var revealed: MonitorSelection?
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
            if store.phase == .loading && isEmpty {
                ScrollView {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                        prompts
                        // #1146: "Reading local queue…", not a spinner.
                        Text(store.words?.tree.readingQueue ?? "")
                            .glassType(GlassTokens.TypeScale.body)
                            .foregroundStyle(GlassColor.textTertiary)
                            .padding(GlassTokens.Space.s6)
                    }
                }
                .scrollIndicators(.never)
            } else if isEmpty {
                ScrollView {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                        prompts
                        // A narrowed list that came back empty still says
                        // so, with its way out.
                        if store.idleOnly { TracesListControls(store: store) }
                        if store.phase == .loaded { emptyTree }
                    }
                }
                .scrollIndicators(.never)
            } else {
                // The prompts sit above the tree on a shelf of their own,
                // never taller than `promptsShare` of the pane and scrolling
                // inside it past that: however many show, they never push
                // the graph footer or the panes beside it out of the window,
                // and the tree below them always keeps rows on screen
                // (R-LAYOUT-1, round 2).
                GeometryReader { pane in
                    VStack(spacing: 0) {
                        PromptsShelf(cap: Self.promptsCap(paneHeight: pane.size.height)) {
                            prompts
                        }
                        TracesListControls(store: store)
                            .padding(.vertical, GlassTokens.Space.s2)
                        tree
                    }
                }
            }
        }
        // Whole-window confirmations: the destructive action right-most and
        // never on Return; Escape cancels.
        .glassModal(isPresented: Binding(get: { confirming != nil }, set: { if !$0 { confirming = nil } })) {
            if let pending = confirming { confirmation(pending) }
        }
        .glassModal(item: $dismissing) { dismissModal($0) }
    }

    /// The folders the idle filter opens: every folder it lists.
    static func idleFolders(_ tree: TracesTree) -> Set<String> {
        Set(tree.folders.filter { !$0.sessions.isEmpty }.map(\.id))
    }

    /// The most of the Traces pane the prompts shelf may take; the rest is
    /// always the tree.
    static let promptsShare: CGFloat = 0.45

    /// The shelf's ceiling for a pane of this height.
    static func promptsCap(paneHeight: CGFloat) -> CGFloat {
        max(0, paneHeight * promptsShare)
    }

    /// The folders and sessions, in their own scroll.
    private var tree: some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(spacing: 2) {
                    ForEach(store.tree.folders) { folderRows($0) }
                }
            }
            // The keyboard moves the selection; keep it on screen,
            // opening a session's folder if it was closed.
            .onChange(of: selection, initial: true) { _, selected in
                reveal(selected)
                guard let id = selected.map(Self.rowID) else { return }
                withAnimation(GlassMotion.fast(GlassMotion.systemReducesMotion)) {
                    proxy.scrollTo(id)
                }
                spoken = id
            }
        }
        .scrollIndicators(.never)
        // A restored selection whose session arrives with a later
        // read is revealed then.
        .onChange(of: store.tree) { _, _ in
            reveal(selection)
            // The idle card's Review lists the sessions it named: their
            // folders open, so the list is the sessions, not closed folders.
            if store.idleOnly { expanded.formUnion(Self.idleFolders(store.tree)) }
        }
        // #1146's tree name.
        .accessibilityLabel(store.words?.tree.treeLabel ?? "")
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

    /// A mode change's confirmation, in the core's words for that folder.
    private func confirmation(_ pending: Confirmation) -> GlassConfirmation {
        let cancel = { confirming = nil }
        switch pending.words {
        case .ignore(let copy):
            return GlassConfirmation(
                title: pending.title, message: pending.body,
                actions: [
                    // #1146's "Keep project", from the core.
                    .cancel(copy.keep, action: cancel),
                    .destructive(copy.button) { apply(pending.folder, pending.mode) },
                ],
                onCancel: cancel)
        case .arm(let copy):
            return GlassConfirmation(
                title: pending.title, message: pending.body,
                actions: [
                    .cancel(copy.decline, action: cancel),
                    GlassModalAction(copy.confirm, isDefault: true) { apply(pending.folder, pending.mode) },
                ],
                onCancel: cancel)
        }
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
    /// skipping what a closed folder hides. A closed folder keeps its own
    /// row; only the folders in `expanded` show their sessions.
    static func visibleRows(in tree: TracesTree, expanded: Set<String>) -> [MonitorSelection] {
        var rows: [MonitorSelection] = []
        for folder in tree.folders {
            rows.append(.folder(projectID: folder.id))
            if expanded.contains(folder.id) {
                rows += folder.sessions.map { .session(entryID: $0.entryId) }
            }
        }
        return rows
    }

    /// The open folders once `selected` is shown: a session's folder opens
    /// so the session is drawn (#1146 `reveal`); a folder, or nothing,
    /// changes nothing.
    static func revealing(_ selected: MonitorSelection?, in tree: TracesTree, expanded: Set<String>) -> Set<String> {
        guard case .session(let entryID) = selected, let folder = folder(of: entryID, in: tree) else { return expanded }
        return expanded.union([folder])
    }

    private func reveal(_ selected: MonitorSelection?) {
        guard selected != revealed else { return }
        // Wait for a session the tree does not hold yet.
        if case .session(let entryID) = selected, Self.folder(of: entryID, in: store.tree) == nil { return }
        expanded = Self.revealing(selected, in: store.tree, expanded: expanded)
        revealed = selected
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
            let rows = Self.visibleRows(in: store.tree, expanded: expanded)
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
            expanded.insert(folder)
        } else {
            expanded.remove(folder)
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

    /// The folder notice, health banners and `InspectorPrompts`, drawn as
    /// the first rows of the tree's scroll, never above it: a tall offer or
    /// the first-contribution note scrolls away with the tree instead of
    /// pushing it, the graph footer and the panes beside it out of the
    /// window (R-LAYOUT-1).
    @ViewBuilder
    private var prompts: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            if let notice = store.folderNotice {
                GlassNotice(tone: .ask) { Text(notice) }
            }
            // Offers, undo and health above the tree (owner, 2026-10-07: an
            // accepted difference from #1146, which puts them in the
            // inspector), in the tree's scroll. Why approved sessions are not moving, or that the
            // core is not answering, first: the pane never reads as current
            // over a last good tree, and an unread status is never drawn as
            // healthy. Then the undos, the offers and the first-contribution
            // note (`InspectorPrompts`).
            ForEach(TracesHealth.banners(
                phase: store.phase, status: store.status, words: store.words, coreDown: TracesHealth.coreDownLine,
                maxQueueEntries: model.daemonSettings?.maxQueueEntries)
            ) { GlassHealthBanner(banner: $0) }
            InspectorPrompts(store: store)
            // The idle or backlog suggestion, in the daemon's words, after
            // anything owed: the daemon already holds it back while an
            // arming offer is up.
            if let card = store.nudgeCard {
                NudgeGlassCard(
                    card: card, busy: store.nudgeBusy,
                    refusal: store.nudgeError.flatMap { store.words?.line(for: $0) }
                ) { intent in Task { await store.perform(intent) } }
            }
        }
    }

    /// No folder, nothing waiting.
    private var emptyTree: some View {
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
    }

    private var isEmpty: Bool { store.tree.folders.isEmpty }

    private func isOpen(_ id: String) -> Bool { expanded.contains(id) }

    private func toggle(_ id: String) {
        if expanded.contains(id) { expanded.remove(id) } else { expanded.insert(id) }
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
        if folder.isBucket, let note = ProjectCopy.unresolvedBucketNote { lines.append(note) }
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
                        // #1146: tertiary, as a row's own sub line.
                        .foregroundStyle(GlassColor.textTertiary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            // #1146 `px-14`: 56 in from both sides.
            .padding(.horizontal, Self.noteInset)
            .padding(.bottom, GlassTokens.Space.s2)
        }
    }

    /// #1146's inset for the lines under a folder row (`px-14`).
    static let noteInset: CGFloat = 56

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
        // The row menu floats over the rows below it (#1146 `RowMenuPill`).
        .overlay(alignment: .topTrailing) {
            if ignorable, folderMenu == folder.id, let label = words?.tree.ignoreFolder {
                TracesRowMenuPill(label, onDismiss: { folderMenu = nil }) {
                    folderMenu = nil
                    request(folder, .ignore)
                }
            }
        }
        .zIndex(folderMenu == folder.id ? 1 : 0)
        .id(Self.rowID(.folder(projectID: folder.id)))
        .accessibilityFocused($spoken, equals: Self.rowID(.folder(projectID: folder.id)))
        // The withheld line is drawn once: the tab's notice may already say
        // it. #1146 puts the count Submit sends before it.
        let withheld: [String] = ignored
            ? [] : offer.withheldLine.flatMap {
                $0 == store.folderNotice ? nil : [Self.withheldNote($0, eligible: offer.count, words: words)]
            } ?? []
        notes(folderNotes(folder) + withheld, depth: .folder)
        if isOpen(folder.id) {
            // #1146: a session under an ignored folder is drawn off too.
            ForEach(folder.sessions) { sessionRow($0, off: ignored) }
        }
    }

    @ViewBuilder
    private func sessionRow(_ entry: DaemonData.QueueEntry, off: Bool) -> some View {
        let words = store.words
        let selected = selection == .session(entryID: entry.entryId)
        GlassListRow(
            depth: .session,
            tile: Self.sessionTile(entry),
            title: Self.when(entry),
            sub: Self.sub(entry, held: words?.held, ineligible: store.ineligibleLine(entry),
                          attestation: store.attestationLine(entry), words: words),
            flag: Self.flag(entry, ineligible: store.ineligibleLine(entry) != nil,
                            attestation: TracesStore.attestationTone(entry)),
            selected: selected,
            off: off,
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
        // The tool's name for VoiceOver: the tile shows it, the sub line
        // (#1146's size and state) does not.
        .accessibilityHint(Self.toolName(entry) ?? "")
        .overlay(alignment: .topTrailing) {
            if sessionMenu == entry.entryId, let label = words?.tree.dismissSession {
                TracesRowMenuPill(label, onDismiss: { sessionMenu = nil }) {
                    sessionMenu = nil
                    dismissing = entry
                }
            }
        }
        .zIndex(sessionMenu == entry.entryId ? 1 : 0)
        .id(Self.rowID(.session(entryID: entry.entryId)))
        .accessibilityFocused($spoken, equals: Self.rowID(.session(entryID: entry.entryId)))
        // The core's tags for the row: a mission it fits, and its estimate
        // only while the daemon says to draw it.
        if let tags = store.rowTags[entry.entryId] {
            NudgeRowTags(tags: tags)
        }
    }

    /// Ron's Dismiss-session confirmation, raised over the whole window: the
    /// session named by when and size, Keep it, and Dismiss, which is the
    /// only dismiss the tree has, right-most and never on Return. A refusal
    /// is said here, and the confirmation stays open over it.
    @ViewBuilder
    private func dismissModal(_ entry: DaemonData.QueueEntry) -> some View {
        if let words = store.words {
            let busy = store.acting.contains(entry.entryId)
            // #1146 ignores a close while dismissing: Keep, Escape and a
            // scrim click wait for the answer, so a refusal is never lost.
            let keep = { if !store.acting.contains(entry.entryId) { dismissing = nil } }
            GlassModal(
                title: words.tree.dismissSessionTitle, subtitle: Self.dismissBody(entry, words: words),
                actions: [
                    GlassModalAction(words.tree.dismissSessionKeep, role: .cancel, isEnabled: !busy, action: keep),
                    .destructive(busy ? words.tree.dismissing : words.dismissAction, isEnabled: !busy) {
                        Task {
                            await store.perform(.dismiss, on: entry.entryId)
                            guard store.actionError?.entryId != entry.entryId else { return }
                            dismissing = nil
                            if selection == .session(entryID: entry.entryId) { selection = nil }
                        }
                    },
                ],
                onCancel: keep
            ) {
                if let refused = store.actionError, refused.entryId == entry.entryId {
                    GlassModalBody {
                        GlassNotice(tone: .outside, title: words.tree.dismissSessionFailed) {
                            Text(words.line(for: refused.error))
                        }
                    }
                }
            }
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
        let size = entry.sizeBytes.map { ByteCountFormatter.string(fromByteCount: Int64($0), countStyle: .memory) } ?? "—"
        return FirstRunCopy.fill(words.tree.dismissSessionBody, ["when": when(entry), "size": size])
    }

    /// A session's title, as #1146's `sessionWhen`: the short weekday and
    /// time it was found ("Mon 3:04 PM"), or when it started if the daemon
    /// did not record that.
    static func when(_ entry: DaemonData.QueueEntry, timeZone: TimeZone = .current) -> String {
        guard let date = entry.discoveredAt ?? entry.startedAt else { return "—" }
        var style = Date.FormatStyle.dateTime.weekday(.abbreviated).hour().minute()
        style.timeZone = timeZone
        return date.formatted(style)
    }

    /// The line under a folder with sessions Submit leaves behind (#1146
    /// `FolderBranch`): how many it sends, then the core's line for the rest.
    static func withheldNote(_ withheld: String, eligible: Int, words: MonitorTracesCopy?) -> String {
        guard let words else { return withheld }
        return FirstRunCopy.fill(words.tree.eligibleCount, ["count": String(eligible)]) + " \u{00B7} " + withheld
    }

    /// Length and size, each only when the daemon reported it.
    static func measures(_ entry: DaemonData.QueueEntry) -> String? {
        var parts: [String] = []
        if let seconds = entry.durationSecs {
            parts.append(Duration.seconds(seconds).formatted(.units(allowed: [.hours, .minutes], width: .abbreviated)))
        }
        if let bytes = entry.sizeBytes {
            parts.append(ByteCountFormatter.string(fromByteCount: Int64(bytes), countStyle: .memory))
        }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    /// Held when a person has to look before it can go: a second-look hold,
    /// Manual Scrub check, or back from Keep.
    static func isHeld(_ entry: DaemonData.QueueEntry) -> Bool {
        entry.heldForSecondLook || entry.heldByManualScrubCheck || entry.returnedFromKeep
    }

    /// Amber when held, when the core says the session cannot go as it
    /// stands, or when its attestation asks for attention or was refused;
    /// green for a clear attestation (#1146 `SessionRow`), and nothing for
    /// any other tone, so a mark this build cannot read is never green.
    static func flag(
        _ entry: DaemonData.QueueEntry, ineligible: Bool = false, attestation: PrivateInferenceTone? = nil
    ) -> GlassListRow.Flag? {
        if isHeld(entry) || ineligible { return .ask }
        switch attestation {
        case .attention, .refused: return .ask
        case .clear: return .on
        default: return nil
        }
    }

    /// A session's sub-line, as #1146's `SessionRow`: its size, then its
    /// state. The amber is never the only signal: a held session says the
    /// core's word for it, and one that cannot be contributed says the
    /// core's sentence. Otherwise #1146's order: trimmed to fit when
    /// subagent transcripts were dropped, then the core's attestation
    /// sentence, then waiting when that sentence is unavailable. The tool
    /// is the row's tile (and its accessibility hint).
    static func sub(
        _ entry: DaemonData.QueueEntry, held: String?, ineligible: String?, attestation: String? = nil,
        words: MonitorTracesCopy?
    ) -> String? {
        let state: String?
        if isHeld(entry), let held {
            state = held
        } else if let ineligible {
            state = ineligible
        } else if (entry.subagentsDropped ?? 0) > 0 {
            state = words?.tree.sessionTrimmed
        } else if let attestation, !attestation.isEmpty {
            state = attestation
        } else {
            state = words?.tree.sessionWaiting
        }
        let parts = [size(entry), state].compactMap { $0 }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    /// The session's size, when the daemon reported it.
    static func size(_ entry: DaemonData.QueueEntry) -> String? {
        entry.sizeBytes.map { ByteCountFormatter.string(fromByteCount: Int64($0), countStyle: .memory) }
    }
}

/// A content-sized shelf over the Traces tree: as tall as what it holds up
/// to `cap`, then scrolling inside itself. With nothing in it, it takes no
/// room and adds no gap.
struct PromptsShelf<Content: View>: View {
    let cap: CGFloat
    @ViewBuilder let content: () -> Content
    @State private var height: CGFloat = 0

    var body: some View {
        ScrollView {
            content()
                .frame(maxWidth: .infinity, alignment: .leading)
                .onGeometryChange(for: CGFloat.self, of: \.size.height) { height = $0 }
        }
        .scrollIndicators(.automatic)
        .scrollBounceBehavior(.basedOnSize)
        .frame(height: Self.shelfHeight(content: height, cap: cap))
        .padding(.bottom, height > 0.5 ? GlassTokens.Space.cardGap : 0)
    }

    /// The shelf's height for content this tall under this ceiling.
    static func shelfHeight(content: CGFloat, cap: CGFloat) -> CGFloat {
        min(max(0, content), max(0, cap))
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

/// A tree row's menu, as #1146's `RowMenuPill`: one glass pill, a ⊘ glyph
/// and the core's label, floating under the row's kebab (38 down, 6 in from
/// the right) over the rows below it, never pushing them down. Escape
/// closes it, as the menus do.
struct TracesRowMenuPill: View {
    private let label: String
    private let onDismiss: () -> Void
    private let action: () -> Void
    @State private var hovering = false

    /// Where it sits under the row (#1146 `top-[38px] right-1.5`).
    static let offset = CGSize(width: -6, height: 38)

    init(_ label: String, onDismiss: @escaping () -> Void, action: @escaping () -> Void) {
        self.label = label
        self.onDismiss = onDismiss
        self.action = action
    }

    var body: some View {
        Button(action: action) {
            HStack(spacing: GlassTokens.Space.s3) {
                Image(systemName: "nosign")
                    .glassGlyph(11, weight: .semibold)
                    .accessibilityHidden(true)
                Text(label)
                    .glassType(GlassTokens.TypeScale.label)
                    .lineLimit(1)
            }
            .foregroundStyle(GlassColor.textPrimary)
            .padding(.horizontal, GlassTokens.Space.s6)
            .frame(height: GlassTokens.Size.control)
            .glassHover(GlassTokens.Color.controlHover, in: Capsule())
            .glassSurface(.menu, radius: GlassTokens.Radius.pill, floating: true)
            .fixedSize()
        }
        .buttonStyle(GlassPressStyle())
        .onExitCommand(perform: onDismiss)
        .accessibilityAddTraits(.isButton)
        .offset(Self.offset)
    }
}

