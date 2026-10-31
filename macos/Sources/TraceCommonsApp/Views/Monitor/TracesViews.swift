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
struct TracesTreeView: View, Equatable {
    let store: TracesStore
    /// The folder or session selected; nil for none.
    @Binding var selection: MonitorSelection?
    /// `selection` when this view was made. The tree draws from it, and it
    /// is what `==` compares: the binding reads the live value, so two
    /// views would always compare equal through it.
    private let selected: MonitorSelection?
    /// The missions switch, which a row's tags follow. A value, not the
    /// app model: every change the model published redrew the whole tree.
    let missionsShown: Bool
    /// A session's Review pill: select it and show the inspector its review
    /// lives in, so Review is never a press that does nothing visible.
    var onReview: (String) -> Void
    /// Opens History on the Traces tab, from the button beside the order
    /// control (owner, 2026-10-12).
    var openHistory: (() -> Void)?

    init(
        store: TracesStore, selection: Binding<MonitorSelection?>, missionsShown: Bool = false,
        onReview: @escaping (String) -> Void = { _ in }, openHistory: (() -> Void)? = nil
    ) {
        self.store = store
        _selection = selection
        selected = selection.wrappedValue
        self.missionsShown = missionsShown
        self.onReview = onReview
        self.openHistory = openHistory
    }

    /// Whether a new tree from the window draws anything this one does not.
    /// The window makes a new one, with a new binding and new closures, on
    /// every pass of its own; without this each pass rebuilt every row.
    /// The closures it ignores act through the window's stored state, so an
    /// older copy of them does the same.
    nonisolated static func == (lhs: TracesTreeView, rhs: TracesTreeView) -> Bool {
        MainActor.assumeIsolated {
            lhs.store === rhs.store && lhs.selected == rhs.selected && lhs.missionsShown == rhs.missionsShown
                && (lhs.openHistory == nil) == (rhs.openHistory == nil)
        }
    }

    /// The folders opened: every folder starts collapsed (#1146
    /// `traces-workspace.tsx`, `expanded = []`), and one opens when a
    /// session in it is selected from elsewhere (#1146 `reveal`).
    @State private var expanded: Set<String> = []
    /// The folders the search opened that were not open already: they
    /// close again when the search is cleared. A folder the person opens or
    /// closes by hand leaves this set and stays as they left it.
    @State private var searchOpened: Set<String> = []
    /// The selection last revealed, so a folder the person closes over a
    /// selected session stays closed until the selection moves. Held in a
    /// box SwiftUI does not watch: writing it is bookkeeping, and as plain
    /// state it redrew the whole tree a second time on every click.
    @State private var revealed = Revealed()

    final class Revealed {
        var selection: MonitorSelection?
        /// A row to scroll to once the folder just opened for it is drawn:
        /// the lazy stack has no such row until then.
        var scrollTarget: String?
    }
    /// A mode change waiting on the core's confirmation: arming always,
    /// ignoring when the folder has sessions waiting. It carries the core's
    /// words for that folder, decoded when the change was asked for.
    @State private var confirming: Confirmation?
    /// The one row menu open, if any: a folder's "..." menu, a folder's
    /// Submit split button's menu, or a session's menu. One value, so
    /// opening a menu closes the others and every open or switch changes it.
    @State private var rowMenu: RowMenu?
    /// The folder whose Submit all as is open, from its row's split
    /// button (owner, 2026-10-19).
    @State private var verdictFolder: TracesTree.FolderNode?
    /// The session whose Dismiss confirmation is open (Ron's
    /// `SessionRow`): dismissing asks first, and only its button dismisses.
    @State private var dismissing: DaemonData.QueueEntry?
    /// VoiceOver's focus, which follows the selection the arrow keys move,
    /// so a person hearing the tree hears where it went.
    @AccessibilityFocusState private var spoken: String?
    /// VoiceOver's focus on an open row menu. The menu floats over the
    /// stack rather than inside its row, so it is read after the rows on
    /// screen; focus goes to it when it opens, as it went to the row.
    /// Keyed on the menu, not a Bool: a switch from one menu to another is
    /// a new value, so focus moves again.
    @AccessibilityFocusState private var menuSpoken: RowMenu?
    /// Whether keeping the selection on screen animates. Off only where no
    /// animation can run: a window that is never shown (the render tests).
    @Environment(\.tracesScrollAnimates) private var scrollAnimates

    /// A row menu, tagged with its kind and its row's id.
    enum RowMenu: Hashable {
        /// A folder's "..." menu: its contribution rule, and Ignore.
        case folder(String)
        /// A folder's Submit split button's menu: Submit all as.
        case submit(String)
        /// A session's menu: Dismiss session.
        case session(String)
    }

    /// The menu open after a control for `menu` is pressed: that menu,
    /// closing whichever was open, or none when it was the one open.
    static func toggled(_ menu: RowMenu, current: RowMenu?) -> RowMenu? {
        current == menu ? nil : menu
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
            // The notices that headed the tree (offers, undos, health, the
            // first-contribution note) are popover notices over the tab now
            // (owner, 2026-10-19, `MonitorNotices`): the controls and the
            // tree start at the tab's top, and nothing pushes them down.
            if store.phase == .loading && isEmpty {
                // #1146: "Reading local queue…", not a spinner.
                Text(store.words?.tree.readingQueue ?? "")
                    .glassType(GlassTokens.TypeScale.body)
                    .foregroundStyle(GlassColor.textTertiary)
                    .padding(GlassTokens.Space.s6)
            } else if isEmpty {
                ScrollView {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                        // A narrowed list that came back empty still says
                        // so, with its way out: the idle filter's Clear, or
                        // Show ignored folders when it hides every folder.
                        if Self.keepsControlsWhenEmpty(idleOnly: store.idleOnly, hiddenIgnored: store.tree.hiddenIgnored) {
                            TracesListControls(store: store, openHistory: openHistory)
                        }
                        if store.phase == .loaded { emptyTree }
                    }
                }
                .scrollIndicators(.never)
            } else {
                VStack(spacing: 0) {
                    TracesListControls(store: store, openHistory: openHistory)
                        .padding(.bottom, GlassTokens.Space.s2)
                    if let automatic = automaticSwitch {
                        SharingChoiceCard(asSwitch: automatic)
                            .padding(.bottom, GlassTokens.Space.s2)
                    }
                    if automaticSwitch != nil, let tree = store.words?.tree,
                        let folders = tree.columnFolders, let submit = tree.columnSubmit, let auto = tree.columnAutomatic {
                        TracesColumnTitles(folders: folders, submit: submit, automatic: auto)
                    }
                    tree
                }
            }
        }
        // Whole-window confirmations: the destructive action right-most and
        // never on Return; Escape cancels.
        .glassModal(isPresented: Binding(get: { confirming != nil }, set: { if !$0 { confirming = nil } })) {
            if let pending = confirming { confirmation(pending) }
        }
        .glassModal(item: $dismissing) { dismissModal($0) }
        .glassModal(item: $verdictFolder) { folder in
            if let words = store.words {
                SubmitAllAsModal(folder: folder, store: store, words: words) { verdictFolder = nil }
            }
        }
    }

    /// What a search does to the open folders. A search opens every folder
    /// it matched and remembers which of those were closed before it; an
    /// empty search closes exactly those, and leaves the rest as they were.
    /// Before this, every folder a search ever matched stayed open, and a
    /// one-letter search matches nearly all of them.
    ///
    /// `keep` is the selected session's folder: clearing never closes it,
    /// so the selection is never hidden by the search going.
    static func searchExpansion(
        query: String, matched: [String], expanded: Set<String>, searchOpened: Set<String>,
        keep: String? = nil
    ) -> (expanded: Set<String>, searchOpened: Set<String>) {
        guard !query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return (expanded.subtracting(searchOpened.subtracting([keep].compactMap { $0 })), [])
        }
        let opened = Set(matched).subtracting(expanded)
        return (expanded.union(matched), searchOpened.union(opened))
    }

    /// Whether an empty list still draws its controls: when a filter in
    /// them emptied it, so the way out stays on screen.
    static func keepsControlsWhenEmpty(idleOnly: Bool, hiddenIgnored: Int) -> Bool {
        idleOnly || hiddenIgnored > 0
    }

    /// The folders the idle filter opens: every folder it lists.
    static func idleFolders(_ tree: TracesTree) -> Set<String> {
        Set(tree.folders.filter { !$0.sessions.isEmpty }.map(\.id))
    }

    /// One row of the tree as drawn: a folder, or a session under an open
    /// folder. Flat, so the lazy stack knows every row by the id
    /// `scrollTo` and VoiceOver focus use, built or not.
    enum DrawnRow: Identifiable, Equatable {
        case folder(TracesTree.FolderNode)
        /// `off`: under an ignored folder, so drawn off too (#1146).
        case session(DaemonData.QueueEntry, off: Bool)

        var id: String {
            switch self {
            case .folder(let folder): TracesTreeView.rowID(.folder(projectID: folder.id))
            case .session(let entry, _): TracesTreeView.rowID(.session(entryID: entry.entryId))
            }
        }
    }

    /// The rows in drawing order: every folder, and the sessions of the
    /// folders in `expanded`. The same order as `visibleRows`.
    static func drawnRows(in tree: TracesTree, expanded: Set<String>) -> [DrawnRow] {
        var rows: [DrawnRow] = []
        for folder in tree.folders {
            rows.append(.folder(folder))
            if expanded.contains(folder.id) {
                let off = folder.mode == .ignore
                rows += folder.sessions.map { .session($0, off: off) }
            }
        }
        return rows
    }

    /// The folders and sessions, in their own scroll.
    ///
    /// Lazy: with a few hundred watched folders, building and laying out
    /// every row took a fifth of a second each time the tab opened, and the
    /// rows on screen are a few dozen. Each row is its own view of plain
    /// values (`TracesFolderRow`, `TracesSessionRow`), so a click or a read
    /// that changes one session rebuilds only the rows it changed. The row
    /// menus float over the stack (`openMenus`), not inside a row, so they
    /// draw over the rows below without a `zIndex` between lazy children.
    private var tree: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(spacing: 2) {
                    // The search narrows what is drawn, never the read.
                    ForEach(Self.drawnRows(in: store.shownTree, expanded: expanded)) { row in
                        switch row {
                        case .folder(let folder): folderRow(folder)
                        case .session(let entry, let off): sessionRow(entry, off: off)
                        }
                    }
                    if store.shownTree.folders.isEmpty, let none = store.words?.tree.searchNone {
                        Text(none)
                            .glassType(GlassTokens.TypeScale.body)
                            .foregroundStyle(GlassColor.textTertiary)
                            .frame(maxWidth: .infinity)
                            .padding(.top, GlassTokens.Space.s6)
                    }
                }
                // Folders are the top level: no tool level to indent past.
                .environment(\.glassTreeRoot, .folder)
                .overlayPreferenceValue(TracesMenuAnchors.self, alignment: .topLeading) { anchors in
                    GeometryReader { space in openMenus(anchors, in: space) }
                }
            }
            // The keyboard moves the selection; keep it on screen,
            // opening a session's folder if it was closed.
            .onChange(of: selection, initial: true) { _, selected in
                let opened = reveal(selected)
                guard let id = selected.map(Self.rowID) else { return }
                if opened {
                    revealed.scrollTarget = id
                } else {
                    scroll(to: id, proxy)
                }
            }
            .onChange(of: expanded) { _, _ in
                guard let id = revealed.scrollTarget else { return }
                revealed.scrollTarget = nil
                scroll(to: id, proxy)
            }
        }
        .scrollIndicators(.never)
        .onChange(of: rowMenu) { _, open in
            guard open != nil else { return }
            Task { @MainActor in menuSpoken = open }
        }
        // A restored selection whose session arrives with a later
        // read is revealed then.
        .onChange(of: store.tree) { _, _ in
            reveal(selection)
            // The idle card's Review lists the sessions it named: their
            // folders open, so the list is the sessions, not closed folders.
            if store.idleOnly {
                let open = expanded.union(Self.idleFolders(store.tree))
                if open != expanded { expanded = open }
            }
        }
        // A search opens the folders it matched, so the matches are rows,
        // not closed folders; clearing it closes what it opened. On the
        // applied search, not each keystroke.
        .onChange(of: store.appliedSearch) { _, query in
            (expanded, searchOpened) = Self.searchExpansion(
                query: query, matched: store.shownTree.folders.map(\.id),
                expanded: expanded, searchOpened: searchOpened,
                keep: selection.flatMap { Self.folder(of: $0, in: store.tree) })
        }
        // #1146's tree name.
        .accessibilityLabel(store.words?.tree.treeLabel ?? "")
        // Full Keyboard Access: focus the tree, then the arrow keys
        // move the selection through the folders and sessions as drawn.
        .focusable()
        // No focus ring round the whole tree (owner, 2026-10-09): the
        // selected row's fill already shows where the arrow keys are.
        .focusEffectDisabled()
        .onMoveCommand(perform: move)
        // Return opens the selected session's review, so the pill
        // need not be its own tab stop on every row.
        .onKeyPress(.return) {
            guard case .session(let entryID) = selection else { return .ignored }
            onReview(entryID)
            return .handled
        }
    }

    /// Automatic for every folder, over the tree (owner, 2026-10-30): the
    /// Settings choice as one switch, with what it does. Nil without the
    /// core's words.
    private var automaticSwitch: SharingChoiceCard.Switch? {
        guard let tree = store.words?.tree, let name = tree.automaticAll,
            let title = tree.automaticTitle, let line = tree.automaticLine else {
            return nil
        }
        let store = store
        return SharingChoiceCard.Switch(
            accessibleName: name,
            title: title,
            line: line,
            saved: { await store.load() })
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
    /// sessions, with their count; Ask is a direct call. With no words
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

    /// A folder row's Automatic switch (owner, 2026-10-30): on at
    /// Automatic, off at Ask, when the core offers the other; nil at Never
    /// or for a folder the core lists no mode for. Turning it on arms the
    /// folder through `modeChange`, so the core's arming question is asked.
    nonisolated static func automaticSwitch(_ folder: TracesTree.FolderNode) -> Bool? {
        // Traces with no project: a switch that stays off (owner,
        // 2026-10-30); pressing it says why.
        if folder.isBucket { return folder.mode == nil ? nil : false }
        return switch folder.mode {
        case .autoUpload: folder.offerableModes.contains(.ask) ? true : nil
        case .ask: folder.offerableModes.contains(.autoUpload) ? false : nil
        case .ignore, nil: nil
        }
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

    /// Opens the folder of a selected session; true when that opened one.
    @discardableResult
    private func reveal(_ selected: MonitorSelection?) -> Bool {
        guard selected != revealed.selection else { return false }
        // Wait for a session the tree does not hold yet.
        if case .session(let entryID) = selected, Self.folder(of: entryID, in: store.shownTree) == nil { return false }
        revealed.selection = selected
        // Written only when a folder opens: a write of the same set still
        // redraws the tree.
        let open = Self.revealing(selected, in: store.shownTree, expanded: expanded)
        guard open != expanded else { return false }
        expanded = open
        return true
    }

    /// Keeps a row on screen. VoiceOver follows once the scroll has built
    /// the row: a lazy row off screen does not exist until then.
    private func scroll(to id: String, _ proxy: ScrollViewProxy) {
        withAnimation(scrollAnimates ? GlassMotion.fast(GlassMotion.systemReducesMotion) : nil) {
            proxy.scrollTo(id)
        }
        Task { @MainActor in spoken = id }
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
            let rows = Self.visibleRows(in: store.shownTree, expanded: expanded)
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
        guard let folder = TracesTreeView.folder(of: selected, in: store.shownTree) else { return }
        searchOpened.remove(folder)
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
    nonisolated static func rowID(_ selected: MonitorSelection) -> String { selected.rawValue }

    /// The folder holding a session.
    static func folder(of entryId: String, in tree: TracesTree) -> String? {
        tree.folders.first { $0.sessions.contains { $0.entryId == entryId } }?.id
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
        searchOpened.remove(id)
        if expanded.contains(id) { expanded.remove(id) } else { expanded.insert(id) }
    }

    /// The core's line for a write it refused on this row, if one is held.
    private func refusal(_ id: String) -> String? {
        store.writeErrors[id].flatMap { store.words?.line(for: $0) }
    }

    /// What a folder says under its row, whether open or not: a refused
    /// write, and why the bucket can never be armed. An armed folder's
    /// disclosure is not drawn on the row (owner, 2026-10-12: too much
    /// text over the list); its inspector card says it in full.
    func folderNotes(_ folder: TracesTree.FolderNode) -> [String] {
        var lines: [String] = []
        if let refused = refusal(folder.id) { lines.append(refused) }
        if folder.isBucket, let note = ProjectCopy.unresolvedBucketNote { lines.append(note) }
        return lines
    }

    /// #1146's inset for the lines under a folder row (`px-14`).
    static let noteInset: CGFloat = 38

    /// A folder row's values, worked out here and handed to its own view,
    /// which is rebuilt only when one of them changes.
    @ViewBuilder
    private func folderRow(_ folder: TracesTree.FolderNode) -> some View {
        let words = store.words
        let ignored = folder.mode == .ignore
        // Submit all is a consent action: drawn only when the shared table
        // offers Contribute for the daemon's counts, never for a folder that
        // is not sent, and removed rather than disabled. Submit all as is
        // the Folder inspector's modal. The offer was worked out with the
        // read, off the main thread.
        let offer = store.groupOffer(folder)
        let submits = offer.offersContribute && store.mayContributeFolder(folder)
        let busy = store.submittingFolder == folder.id
        // Ignore remains available when a global Automatic default removes
        // the per-folder Ask choice.
        let switchable = folder.mode != nil && folder.offerableModes.contains(.ignore)
        let ignorable = switchable && !ignored
        // The row menu holds the contribution rule (owner, 2026-10-19), the
        // Folder inspector's picker, whenever the core lists a mode and
        // offers others; and Ignore.
        let rulable = folder.mode != nil && !folder.offerableModes.isEmpty
        let menu = rulable || ignorable
        let automatic = words?.tree.automaticFolder == nil ? nil : Self.automaticSwitch(folder)
        // Submit is the Folder inspector's split button (owner, 2026-10-19):
        // the action, and a chevron to Submit all as, when the core's
        // outcome words are read; else the plain pill.
        let outcome = store.disclosure?.outcome
        let split = submits && outcome != nil
        let submitWord: String? = words.map { busy ? $0.tree.submitting : Self.submitTitle(offer.count, words: $0) }
        // The withheld line is drawn once: the tab's notice may already say
        // it. #1146 puts the count Submit sends before it.
        let withheld: [String] = ignored
            ? [] : offer.withheldLine.flatMap {
                $0 == store.folderNotice ? nil : [Self.withheldNote($0, eligible: offer.count, words: words)]
            } ?? []
        // A write to this folder in flight: the row dims and turns its
        // controls off at once, and says nothing else until the daemon
        // answers and the read redraws it.
        let writing = store.writing.contains(folder.id)
        TracesFolderRow(
            folder: folder,
            // The mode's word and the count waiting; an ignored folder says so.
            sub: words.flatMap {
                Self.folderSub(folder, words: $0, modeLabels: store.disclosure?.folderModeLabels ?? [:])
            },
            selected: selected == .folder(projectID: folder.id),
            off: folder.mode == .ignore,
            open: isOpen(folder.id),
            submitTitle: submits && !split ? submitWord : nil,
            split: split ? TracesFolderRow.Split(
                title: submitWord ?? "", menuLabel: outcome?.submitAllAs ?? "",
                menuOpen: rowMenu == .submit(folder.id), busy: busy) : nil,
            menuLabel: menu ? words?.tree.folderMenu ?? "" : "",
            menuOpen: rowMenu == .folder(folder.id),
            automatic: automatic,
            automaticLabel: words?.tree.automaticFolder.map { $0.replacingOccurrences(of: "{label}", with: folder.label) } ?? "",
            floats: (menu && rowMenu == .folder(folder.id)) || (split && rowMenu == .submit(folder.id)),
            help: submits ? Self.submitHelp(offer.withheldLine, words: words) : "",
            writing: writing,
            // The pill already says Submitting for a Submit all.
            spins: writing && !busy,
            notes: folderNotes(folder) + withheld,
            onToggleExpand: { toggle(folder.id) },
            onSelect: { selection = Self.toggled(.folder(projectID: folder.id), current: selection) },
            onSubmit: submits && !busy ? { Task { await store.contributeFolder(folder, verdict: nil) } } : nil,
            onSplitSubmit: { Task { await store.contributeFolder(folder, verdict: nil) } },
            onSplitMenu: { rowMenu = Self.toggled(.submit(folder.id), current: rowMenu) },
            onMenu: menu ? { rowMenu = Self.toggled(.folder(folder.id), current: rowMenu) } : nil,
            onAutomatic: { on in
                if folder.isBucket {
                    if on, let note = ProjectCopy.unresolvedBucketNote { store.say(note) }
                } else {
                    request(folder, on ? .autoUpload : .ask)
                }
            },
            spoken: $spoken
        )
        .equatable()
        .id(Self.rowID(.folder(projectID: folder.id)))
    }

    /// A session row's values, worked out here and handed to its own view.
    @ViewBuilder
    private func sessionRow(_ entry: DaemonData.QueueEntry, off: Bool) -> some View {
        let words = store.words
        let selected = self.selected == .session(entryID: entry.entryId)
        let lines = store.lines(entry)
        TracesSessionRow(
            entry: entry,
            tile: Self.sessionTile(entry),
            title: Self.when(entry),
            sub: Self.sub(entry, held: words?.held, ineligible: lines.ineligible,
                          attestation: lines.attestation, words: words),
            flag: Self.flag(entry, ineligible: lines.ineligible != nil, attestation: lines.tone),
            selected: selected,
            off: off,
            // A review action on it in flight (Contribute, Keep, Dismiss):
            // dimmed, its pill off and turning, until the daemon answers.
            acting: store.acting.contains(entry.entryId),
            // A session's pill is Submit (owner, 2026-10-22, in place of
            // D10's Review): it sends the trace at once, as a folder's
            // Submit does, and is drawn turned off while it cannot be sent.
            // Focus roves: only the selected row's pill is a tab stop;
            // Return still opens the session's review.
            pill: words?.tree.submit,
            menuLabel: words?.tree.dismissSession ?? "",
            menuOpen: rowMenu == .session(entry.entryId),
            help: words?.tree.submitTraceTip ?? "",
            // The tool's name for VoiceOver: the tile shows it, the sub line
            // (#1146's size and state) does not.
            toolName: Self.toolName(entry) ?? "",
            // The core's tags for the row: a mission it fits, and its
            // estimate only while the daemon says to draw it. The mission
            // tag is blanked while missions are hidden; the estimate keeps
            // its own switch.
            tags: store.rowTags[entry.entryId].map { $0.showing(missions: missionsShown) },
            onSelect: { selection = Self.toggled(.session(entryID: entry.entryId), current: selection) },
            onSubmit: store.maySubmitSession(entry, folderIgnored: off)
                ? { Task { await store.submitSession(entry, folderIgnored: off) } } : nil,
            onMenu: { rowMenu = Self.toggled(.session(entry.entryId), current: rowMenu) },
            spoken: $spoken
        )
        .equatable()
        .id(Self.rowID(.session(entryID: entry.entryId)))
    }

    /// The open row menus, floated over the tree at their rows (#1146
    /// `RowMenuPill`: under the row's kebab, over the rows below it, never
    /// pushing them down). A row publishes its bounds only while its menu
    /// is open (`TracesMenuAnchors`).
    @ViewBuilder
    private func openMenus(_ anchors: [String: Anchor<CGRect>], in space: GeometryProxy) -> some View {
        if let open = rowMenu, let id = Self.folderID(of: open),
           let anchor = anchors[Self.rowID(.folder(projectID: id))],
           let folder = store.shownTree.folders.first(where: { $0.id == id }) {
            let rect = space[anchor]
            folderMenus(folder)
                .accessibilityElement(children: .contain)
                .accessibilityFocused($menuSpoken, equals: open)
                .frame(width: rect.width, height: rect.height, alignment: .topTrailing)
                .offset(x: rect.minX, y: rect.minY)
        }
        if case .session(let id) = rowMenu,
           let anchor = anchors[Self.rowID(.session(entryID: id))],
           let label = store.words?.tree.dismissSession,
           let entry = store.shownTree.allSessions.first(where: { $0.entryId == id }) {
            let rect = space[anchor]
            TracesRowMenuPill(label, onDismiss: { rowMenu = nil }) {
                rowMenu = nil
                dismissing = entry
            }
            .accessibilityFocused($menuSpoken, equals: .session(id))
            .frame(width: rect.width, height: rect.height, alignment: .topTrailing)
            .offset(x: rect.minX, y: rect.minY)
        }
    }

    /// The folder whose menu is open, for either of a folder row's menus.
    static func folderID(of menu: RowMenu) -> String? {
        switch menu {
        case .folder(let id), .submit(let id): id
        case .session: nil
        }
    }

    /// A folder row's open menu: its rule and Ignore, or Submit all as.
    @ViewBuilder
    private func folderMenus(_ folder: TracesTree.FolderNode) -> some View {
        let ignorable = folder.mode != nil && folder.offerableModes.contains(.ignore) && folder.mode != .ignore
        if rowMenu == .folder(folder.id), let words = store.words {
            TracesFolderMenu(
                folder: folder, ruleTitle: words.inspector.contributionRule,
                ignoreLabel: ignorable ? words.tree.ignoreFolder : nil,
                onRule: { mode in
                    rowMenu = nil
                    if mode != folder.mode { request(folder, mode) }
                },
                onIgnore: {
                    rowMenu = nil
                    request(folder, .ignore)
                },
                onDismiss: { rowMenu = nil })
        } else if rowMenu == .submit(folder.id), let outcome = store.disclosure?.outcome {
            TracesSubmitMenu(
                label: outcome.submitAllAs, tip: outcome.submitAllAsTooltip,
                onPick: {
                    rowMenu = nil
                    verdictFolder = folder
                },
                onDismiss: { rowMenu = nil })
                .offset(TracesSubmitMenu.offset)
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

    nonisolated static func glassTool(_ kind: SourceKind) -> GlassTool {
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

extension EnvironmentValues {
    /// See `TracesTreeView.scrollAnimates`.
    @Entry var tracesScrollAnimates = true
}

/// The bounds of the rows whose menu is open, by row id, so the tree
/// floats the menu over the rows below at its row (`openMenus`).
struct TracesMenuAnchors: PreferenceKey {
    static var defaultValue: [String: Anchor<CGRect>] { [:] }

    static func reduce(value: inout [String: Anchor<CGRect>], nextValue: () -> [String: Anchor<CGRect>]) {
        value.merge(nextValue()) { $1 }
    }
}

/// How much a row with a write or review action in flight is dimmed.
/// Not `rowOff`, which says a folder is ignored.
enum TracesRowBusy {
    static let opacity = GlassTokens.Opacity.disabledCheck
}

/// A folder row of the Traces tree, and the lines under it. Every value it
/// draws is a field, and `==` compares them all, so SwiftUI rebuilds it
/// only when one changes; the actions are left out of `==`, since each
/// acts on the folder (compared) through the tree's stored state.
struct TracesFolderRow: View, Equatable {
    /// The split button's state (owner, 2026-10-19).
    struct Split: Equatable {
        let title: String
        let menuLabel: String
        let menuOpen: Bool
        let busy: Bool
    }

    let folder: TracesTree.FolderNode
    let sub: String?
    let selected: Bool
    let off: Bool
    let open: Bool
    let submitTitle: String?
    let split: Split?
    let menuLabel: String
    let menuOpen: Bool
    /// The folder's Automatic switch beside Submit (owner, 2026-10-30): on
    /// for Automatic, off for Ask; nil for a folder it cannot switch
    /// (Never, or a mode the core does not offer). Its accessible name.
    let automatic: Bool?
    let automaticLabel: String
    /// One of this row's menus is open: it publishes its bounds for it.
    let floats: Bool
    let help: String
    let writing: Bool
    let spins: Bool
    let notes: [String]
    let onToggleExpand: () -> Void
    let onSelect: () -> Void
    let onSubmit: (() -> Void)?
    let onSplitSubmit: () -> Void
    let onSplitMenu: () -> Void
    let onMenu: (() -> Void)?
    let onAutomatic: (Bool) -> Void
    /// The tree's VoiceOver focus, put on the `GlassListRow`: the element
    /// VoiceOver lands on, an accessibility container. Left out of `==`:
    /// it is the same focus state for every row.
    let spoken: AccessibilityFocusState<String?>.Binding

    /// SwiftUI compares views on the main thread.
    nonisolated static func == (lhs: TracesFolderRow, rhs: TracesFolderRow) -> Bool {
        MainActor.assumeIsolated {
            lhs.folder == rhs.folder && lhs.sub == rhs.sub && lhs.selected == rhs.selected && lhs.off == rhs.off
                && lhs.open == rhs.open && lhs.submitTitle == rhs.submitTitle && lhs.split == rhs.split
                && lhs.menuLabel == rhs.menuLabel && lhs.menuOpen == rhs.menuOpen && lhs.automatic == rhs.automatic
                && lhs.automaticLabel == rhs.automaticLabel && lhs.floats == rhs.floats
                && lhs.help == rhs.help && lhs.writing == rhs.writing && lhs.spins == rhs.spins && lhs.notes == rhs.notes
                && (lhs.onSubmit == nil) == (rhs.onSubmit == nil) && (lhs.onMenu == nil) == (rhs.onMenu == nil)
        }
    }

    var body: some View {
        #if DEBUG
        let _ = TracesRowBodies.folders += 1
        let _ = TracesRowBodies.built.insert(TracesTreeView.rowID(.folder(projectID: folder.id)))
        #endif
        VStack(spacing: 2) {
            GlassListRow(
                depth: .folder,
                tile: .folder,
                title: folder.label,
                sub: sub,
                selected: selected,
                off: off,
                expanded: folder.sessions.isEmpty ? nil : open,
                submitTitle: submitTitle,
                submitHelp: help,
                accessory: accessory,
                // The Automatic switch is on the row again (owner,
                // 2026-10-30), beside Submit; Never stays in the row menu.
                expandLabel: folder.label,
                menuLabel: menuLabel,
                menuOpen: menuOpen,
                onToggleExpand: onToggleExpand,
                onSelect: onSelect,
                onSubmit: onSubmit,
                onMenu: onMenu
            )
            .accessibilityFocused(spoken, equals: TracesTreeView.rowID(.folder(projectID: folder.id)))
            // Spoken, not a hover tip: the pill carries that, and VoiceOver
            // lands on the row, so the core's withheld sentence is heard here.
            .accessibilityHint(help)
            .disabled(writing)
            .opacity(writing ? TracesRowBusy.opacity : 1)
            .anchorPreference(key: TracesMenuAnchors.self, value: .bounds) {
                floats ? [TracesTreeView.rowID(.folder(projectID: folder.id)): $0] : [:]
            }
            if !notes.isEmpty {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                    ForEach(notes, id: \.self) { line in
                        Text(line)
                            .glassType(GlassTokens.TypeScale.caption)
                            // #1146: tertiary, as a row's own sub line.
                            .foregroundStyle(GlassColor.textTertiary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                // #1146 `px-14`: 56 in from both sides.
                .padding(.horizontal, TracesTreeView.noteInset)
                .padding(.bottom, GlassTokens.Space.s2)
            }
        }
    }

    /// The row's Submit as the Folder inspector's split button (owner,
    /// 2026-10-19): Submit all, and a chevron to Submit all as; the
    /// spinner while a mode change is in flight; and the Automatic switch
    /// after them (owner, 2026-10-30).
    private var accessory: AnyView? {
        guard split != nil || spins || automatic != nil else { return nil }
        return AnyView(HStack(spacing: GlassTokens.Space.s3) {
            if spins { GlassSpinner() }
            if let split {
                GlassSplitButton(split.title, menuLabel: split.menuLabel, menuOpen: split.menuOpen, onMenu: onSplitMenu) {
                    onSplitSubmit()
                }
                .actionHelp(help)
                .disabled(split.busy)
            }
            if let automatic {
                Toggle(automaticLabel, isOn: Binding(get: { automatic }, set: { onAutomatic($0) }))
                    .toggleStyle(GlassToggleStyle(.watch, showsLabel: false))
                    .help(automaticLabel)
            } else {
                // A row with no switch (Traces with no project) keeps its
                // place, so every Submit lines up.
                Color.clear
                    .frame(width: GlassTokens.Size.watchSwitchWidth, height: 1)
                    .accessibilityHidden(true)
            }
        })
    }
}

/// A session row of the Traces tree, and the core's tags under it. As
/// `TracesFolderRow`: every value drawn is a field `==` compares.
struct TracesSessionRow: View, Equatable {
    let entry: DaemonData.QueueEntry
    let tile: GlassToolTile.Kind
    let title: String
    let sub: String?
    let flag: GlassListRow.Flag?
    let selected: Bool
    let off: Bool
    let acting: Bool
    let pill: String?
    let menuLabel: String
    let menuOpen: Bool
    let help: String
    let toolName: String
    let tags: NudgeEntryTags?
    let onSelect: () -> Void
    /// Nil while the session cannot be sent: the pill is drawn turned off.
    let onSubmit: (() -> Void)?
    let onMenu: () -> Void
    /// As `TracesFolderRow.spoken`.
    let spoken: AccessibilityFocusState<String?>.Binding

    nonisolated static func == (lhs: TracesSessionRow, rhs: TracesSessionRow) -> Bool {
        MainActor.assumeIsolated {
            lhs.entry == rhs.entry && lhs.tile == rhs.tile && lhs.title == rhs.title && lhs.sub == rhs.sub
                && lhs.flag == rhs.flag && lhs.selected == rhs.selected && lhs.off == rhs.off && lhs.acting == rhs.acting
                && lhs.pill == rhs.pill && lhs.menuLabel == rhs.menuLabel && lhs.menuOpen == rhs.menuOpen
                && lhs.help == rhs.help && lhs.toolName == rhs.toolName && lhs.tags == rhs.tags
                && (lhs.onSubmit == nil) == (rhs.onSubmit == nil)
        }
    }

    var body: some View {
        #if DEBUG
        let _ = TracesRowBodies.sessions += 1
        let _ = TracesRowBodies.built.insert(TracesTreeView.rowID(.session(entryID: entry.entryId)))
        #endif
        VStack(spacing: 2) {
            GlassListRow(
                depth: .session,
                tile: tile,
                title: title,
                sub: sub,
                flag: flag,
                selected: selected,
                off: off,
                submitTitle: pill,
                submitHelp: help,
                submitFocusable: selected && !acting,
                accessory: acting ? AnyView(GlassSpinner()) : nil,
                menuLabel: menuLabel,
                menuOpen: menuOpen,
                onSelect: onSelect,
                onSubmit: acting ? nil : onSubmit,
                onMenu: acting ? nil : onMenu
            )
            .accessibilityFocused(spoken, equals: TracesTreeView.rowID(.session(entryID: entry.entryId)))
            .opacity(acting ? TracesRowBusy.opacity : 1)
            .accessibilityHint(toolName)
            .anchorPreference(key: TracesMenuAnchors.self, value: .bounds) {
                menuOpen ? [TracesTreeView.rowID(.session(entryID: entry.entryId)): $0] : [:]
            }
            if let tags {
                NudgeRowTags(tags: tags)
            }
        }
    }
}

#if DEBUG
/// How many times the tree's rows were built. For tests: a selection
/// change or a reload that changes one session rebuilds only the rows it
/// changed, and the rows off screen are not built at all.
@MainActor
enum TracesRowBodies {
    static var folders = 0
    static var sessions = 0
    /// The rows built since the last reset, by row id.
    static var built: Set<String> = []
    static var total: Int { folders + sessions }

    static func reset() {
        folders = 0
        sessions = 0
        built = []
    }
}
#endif

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
/// The glass menu panel the Traces tab's own menus share (owner,
/// 2026-10-19): a folder row's "..." menu and the Submit split buttons'
/// chevron, so they look alike. Opaque under its glass, so the rows it
/// floats over never show through it; Escape closes it.
struct TracesMenuPanel<Content: View>: View {
    let onDismiss: () -> Void
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) { content }
            .padding(GlassTokens.Space.s5)
            .glassSurface(.menu, floating: true)
            .background(RoundedRectangle(cornerRadius: GlassTier.menu.radius, style: .continuous)
                .fill(GlassTokens.Color.paneOpaque.color))
            .fixedSize()
            .onExitCommand(perform: onDismiss)
    }
}

/// One item in a `TracesMenuPanel`: its glyph and the core's words. Under
/// the pointer it takes the menu's own highlight, the accent with the
/// menu's hover ink, as every menu row does (owner, 2026-10-22: the control
/// hover, a dark well, all but vanished on the dark menu).
struct TracesMenuItem: View {
    let systemImage: String
    let label: String
    let action: () -> Void
    @State private var hovering = false
    @Environment(\.isEnabled) private var isEnabled

    var body: some View {
        let lit = GlassHover.shows(hovering: hovering, enabled: isEnabled)
        Button(action: action) {
            HStack(spacing: GlassTokens.Space.s3) {
                Image(systemName: systemImage)
                    .glassGlyph(11, weight: .semibold)
                    .accessibilityHidden(true)
                Text(label)
                    .glassType(GlassTokens.TypeScale.label)
                    .lineLimit(1)
            }
            .foregroundStyle(TracesMenuHighlight.ink(lit: lit).color)
            .padding(.horizontal, GlassTokens.Space.s4)
            .frame(height: GlassTokens.Size.control)
            .glassHover(TracesMenuHighlight.fill, in: Capsule())
            .onHover { hovering = $0 }
        }
        .buttonStyle(GlassPressStyle())
    }
}

/// A Traces menu row's highlight under the pointer: the menu's own, as
/// `GlassMenuItem` draws it.
enum TracesMenuHighlight {
    static let fill = GlassTokens.Color.menuHover

    static func ink(lit: Bool) -> GlassRGBA {
        lit ? GlassTokens.Color.menuHoverText : GlassTokens.Color.textPrimary
    }
}

/// A folder row's menu (owner, 2026-10-19): its contribution rule, as the
/// Folder inspector's picker in the core's names, then Ignore. It floats
/// over the rows below the row, as the session row's menu pill does.
struct TracesFolderMenu: View {
    let folder: TracesTree.FolderNode
    /// The core's "Contribution rule": the picker's heading and its name.
    let ruleTitle: String
    /// The core's Ignore word; nil when the folder cannot be ignored.
    let ignoreLabel: String?
    let onRule: (ProjectMode) -> Void
    let onIgnore: () -> Void
    let onDismiss: () -> Void

    var body: some View {
        TracesMenuPanel(onDismiss: onDismiss) {
            if let mode = folder.mode, !folder.offerableModes.isEmpty {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                    Text(ruleTitle)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .accessibilityHidden(true)
                    GlassPicker(
                        ruleTitle,
                        selection: Binding(get: { mode }, set: { wanted in if let wanted { onRule(wanted) } }),
                        options: folder.offerableModes.map {
                            GlassPickerOption(FolderInspector.ruleLabel($0) ?? "\u{2014}", value: $0,
                                              dot: TracesTreeView.status($0))
                        },
                        placeholder: "\u{2014}"
                    )
                }
            }
            if let ignoreLabel {
                TracesMenuItem(systemImage: "nosign", label: ignoreLabel, action: onIgnore)
            }
        }
        .offset(TracesRowMenuPill.offset)
    }
}

/// The Submit split button's menu (owner, 2026-10-19), in the "..." menu's
/// look: Submit all as, which opens the outcome modal.
struct TracesSubmitMenu: View {
    /// The core's "Submit all as" and its tip.
    let label: String
    let tip: String
    let onPick: () -> Void
    let onDismiss: () -> Void

    /// Where it sits under a folder row: under the split button's chevron,
    /// which ends left of the row menu's column.
    static let offset = CGSize(width: -36, height: 38)

    var body: some View {
        TracesMenuPanel(onDismiss: onDismiss) {
            TracesMenuItem(systemImage: "checklist", label: label, action: onPick)
                .help(tip)
        }
    }
}

struct TracesRowMenuPill: View {
    private let label: String
    private let onDismiss: () -> Void
    private let action: () -> Void
    @State private var hovering = false
    @Environment(\.isEnabled) private var isEnabled

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
            .foregroundStyle(TracesMenuHighlight.ink(lit: GlassHover.shows(hovering: hovering, enabled: isEnabled)).color)
            .padding(.horizontal, GlassTokens.Space.s6)
            .frame(height: GlassTokens.Size.control)
            .glassHover(TracesMenuHighlight.fill, in: Capsule())
            .onHover { hovering = $0 }
            .glassSurface(.menu, radius: GlassTokens.Radius.pill, floating: true)
            .fixedSize()
        }
        .buttonStyle(GlassPressStyle())
        .onExitCommand(perform: onDismiss)
        .accessibilityAddTraits(.isButton)
        .offset(Self.offset)
    }
}

/// The titles over the tree's columns while the Automatic switches show
/// (owner, 2026-10-30, Proposal A): the folders at the left, Submit, and
/// Auto over the rows' switches. Decorative: each control has its own
/// accessible name, so the titles are hidden from assistive tech.
struct TracesColumnTitles: View {
    let folders: String
    let submit: String
    let automatic: String

    var body: some View {
        HStack(spacing: GlassTokens.Space.s4) {
            Text(folders)
                .frame(maxWidth: .infinity, alignment: .leading)
            // Submit and the switch sit 6pt apart in the row's accessory;
            // the switch ends it, before the row menu's 22pt column and the
            // row's 6pt trailing inset. Submit's title is centred over the
            // usual split button, "Submit all (n)" and its chevron.
            HStack(spacing: GlassTokens.Space.s3) {
                Text(submit)
                    .frame(minWidth: 128)
                Text(automatic)
                    .frame(width: GlassTokens.Size.watchSwitchWidth)
            }
            Color.clear.frame(width: 22, height: 0)
        }
        .glassType(GlassTokens.TypeScale.eyebrow)
        .foregroundStyle(GlassColor.textTertiary)
        .lineLimit(1)
        .padding(.leading, 8)
        .padding(.trailing, 6)
        .padding(.bottom, GlassTokens.Space.s2)
        .accessibilityHidden(true)
    }
}
