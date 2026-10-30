import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// Ron's Folder inspector (#1146, `waiting-page.tsx` `FolderInspector`):
/// the header, the shared/kept legend, then three sections, each an
/// expander heading over bare content: its contribution rule with the mode
/// picker; the folder's details (its path, what is waiting and its size);
/// and, while
/// sessions wait, Ron's Decisions card (`waiting-project-folder.tsx`) with
/// Submit all eligible, Submit all as (a modal with one outcome for every
/// eligible session) and Ignore.
///
/// There is no Tool inspector: the tree has no tool level (owner,
/// 2026-10-05). Every mode change takes the tree's route
/// (`TracesTreeView.modeChange`), so Automatic always asks first in the
/// core's words, and Never asks first when sessions are waiting.
///
/// Every word is the core's: the Monitor tables (`MonitorTracesCopy`), the
/// disclosure bundle's outcome words and folder mode names, and the
/// project ignore and arming confirmations.
struct FolderInspector: View {
    let store: TracesStore
    let folder: TracesTree.FolderNode
    /// History as of its last good read, for the shared count; nil when
    /// unread, and the count is then a dash.
    var history: [DaemonData.HistoryRow]? = nil

    /// A mode change waiting on its confirmation.
    @State private var confirming: TracesTreeView.Confirmation?
    /// Whether Submit all as's modal is open.
    @State private var choosingVerdict = false
    /// Whether the Submit split button's menu is open (owner, 2026-10-19:
    /// the Traces tab's menu panel, not a system menu).
    @State private var submitMenuOpen = false

    var body: some View {
        if let words = store.words {
            // No scroll of its own: the inspector column scrolls as one
            // (`MonitorWindowView`), and a scroll nested in it was sized
            // short of its content and cut off the Decisions card's Submit
            // (owner, 2026-10-19).
            VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                InspectorHeader(tile: .folder, title: folder.label, sub: Self.headerSub(folder, words: words))
                // The legend says its words or is not drawn, as the Summary's.
                if let table = MonitorWords.table {
                    HStack(spacing: GlassTokens.Space.s3) {
                        GlassLegendCell(table.shared, value: Self.shared(folder, history: history), status: .shared)
                        GlassLegendCell(table.kept, value: String(folder.sessions.count), status: .kept)
                    }
                }
                // The rule first, under the legend (owner, 2026-10-19), then
                // the folder's details, then what waits on a decision.
                InspectorSection(words.inspector.contributionRule) {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s3) { rule(words) }
                }
                InspectorSection(words.inspector.projectDetails) {
                    GlassKeyValueList(Self.rows(folder, words: words))
                }
                if !folder.sessions.isEmpty && folder.mode != .ignore {
                    InspectorSection(words.inspector.decisions) {
                        decisions(words)
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            // Whole-window confirmations, as the tree's: the destructive
            // action right-most and never on Return; Escape cancels.
            .glassModal(isPresented: Binding(get: { confirming != nil }, set: { if !$0 { confirming = nil } })) {
                if let pending = confirming { confirmation(pending, words: words) }
            }
            .glassModal(isPresented: $choosingVerdict) { submitAllAsModal(words) }
        }
    }

    // MARK: Parts

    /// The rule: the three-way picker with the core's names, or the core's
    /// line for a folder it does not list. A Never folder shows Never here
    /// and nothing to submit below.
    @ViewBuilder
    private func rule(_ words: MonitorTracesCopy) -> some View {
        if let mode = folder.mode, !folder.offerableModes.isEmpty {
            GlassPicker(
                folder.label,
                selection: Binding(get: { mode }, set: { wanted in if let wanted { request(wanted) } }),
                options: folder.offerableModes.map {
                    GlassPickerOption(Self.ruleLabel($0) ?? "—", value: $0, dot: TracesTreeView.status($0))
                },
                placeholder: "—"
            )
            .disabled(store.writing.contains(folder.id))
            disclosure(words)
            if folder.isBucket, let note = ProjectCopy.unresolvedBucketNote { caption(note) }
        } else {
            caption(words.inspector.noRule)
        }
    }

    /// What an armed folder is told: the one sentence for its route (owner,
    /// 2026-10-12), with the long, precise disclosure behind an info button
    /// beside it. If the core's "More about" words are unavailable the long
    /// form is drawn instead, so it is never unreachable.
    @ViewBuilder
    private func disclosure(_ words: MonitorTracesCopy) -> some View {
        if let copy = store.armedDisclosure(folder.disclosure) {
            if let label = Self.moreAboutLabel(words.inspector.contributionRule) {
                HStack(alignment: .top, spacing: GlassTokens.Space.s2) {
                    caption(copy.summary)
                    GlassInfoButton(label, text: copy.lines.joined(separator: "\n\n"))
                }
            } else {
                ForEach(copy.lines, id: \.self) { caption($0) }
            }
        }
    }

    /// The core's "More about {title}" for the rule section, nil if unread.
    static func moreAboutLabel(_ title: String) -> String? {
        TCCoreCopy.firstRunCopyJSON().flatMap(FirstRunCopy.decode).map {
            FirstRunCopy.fill($0.uses.moreAbout, ["title": title])
        }
    }

    /// Ron's Decisions card (`waiting-project-folder.tsx`): the folder,
    /// its path, what is waiting, what is eligible and what was withheld,
    /// then Submit all eligible, Submit all as and Ignore, each on one line.
    @ViewBuilder
    private func decisions(_ words: MonitorTracesCopy) -> some View {
        let offer = store.groupOffer(folder)
        let busy = store.writing.contains(folder.id)
        // Its section is the card (`InspectorSection`): no card inside it.
        VStack(alignment: .leading, spacing: GlassTokens.Space.s5) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                Text(folder.label)
                    .glassType(GlassTokens.TypeScale.bodyStrong)
                    .foregroundStyle(GlassColor.textPrimary)
                if let path = folder.path, !path.isEmpty {
                    caption(path)
                }
                caption(Self.waitingLine(folder.sessions.count, words: words))
                if let eligible = Self.eligibleLine(folder, offer: offer, words: words) {
                    caption(eligible)
                }
            }
            // Submit all eligible and Submit all as are one split
            // button (owner, 2026-10-10): the action, and a chevron
            // that opens Submit all as. Ignore, the decline, is a link
            // after it (Ron, 2026-10-09).
            HStack(spacing: GlassTokens.Space.s4) {
                let submits = Self.offersSubmitAll(folder, store: store)
                let title = store.submittingFolder == folder.id
                    ? words.tree.submitting : Self.submitAllTitle(offer.count, words: words)
                if submits, let outcome = store.disclosure?.outcome {
                    GlassSplitButton(
                        title, menuLabel: outcome.submitAllAs, menuOpen: submitMenuOpen,
                        onMenu: { submitMenuOpen.toggle() }
                    ) {
                        Task { await store.contributeFolder(folder, verdict: nil) }
                    }
                    .actionHelp(TracesTreeView.submitHelp(offer.withheldLine, words: words))
                    .overlay(alignment: .topTrailing) {
                        if submitMenuOpen {
                            TracesSubmitMenu(
                                label: outcome.submitAllAs, tip: outcome.submitAllAsTooltip,
                                onPick: {
                                    submitMenuOpen = false
                                    choosingVerdict = true
                                },
                                onDismiss: { submitMenuOpen = false })
                                .offset(y: GlassTokens.Size.controlLarge + GlassTokens.Space.s2)
                        }
                    }
                    .zIndex(submitMenuOpen ? 1 : 0)
                } else if submits {
                    Button(title) { Task { await store.contributeFolder(folder, verdict: nil) } }
                        .buttonStyle(GlassButtonStyle(.glass))
                        .help(TracesTreeView.submitHelp(offer.withheldLine, words: words))
                }
                if let copy = ignoreCopy {
                    Button(copy.button) { route(TracesTreeView.modeChange(folder, .ignore), .ignore) }
                        .buttonStyle(GlassButtonStyle(.link))
                        .help(copy.tooltip)
                }
            }
            .lineLimit(1)
            .disabled(busy)
            // Its menu floats over what follows the buttons.
            .zIndex(submitMenuOpen ? 1 : 0)
            // A refused write, under the buttons it is about.
            if let refused = store.writeErrors[folder.id] {
                GlassAlert(words.line(for: refused))
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// Ron's `SubmitAllAsControl` (`SubmitAllAsModal`), shared with the
    /// folder row's Submit split button.
    @ViewBuilder
    private func submitAllAsModal(_ words: MonitorTracesCopy) -> some View {
        SubmitAllAsModal(folder: folder, store: store, words: words) { choosingVerdict = false }
    }

    /// A rule change's confirmation, in the core's words for this folder.
    private func confirmation(_ pending: TracesTreeView.Confirmation, words: MonitorTracesCopy) -> GlassConfirmation {
        let cancel = { confirming = nil }
        switch pending.words {
        case .ignore(let copy):
            return GlassConfirmation(
                title: pending.title, message: pending.body,
                actions: [
                    .cancel(copy.keep, action: cancel),
                    .destructive(copy.button) { apply(pending.mode) },
                ],
                onCancel: cancel)
        case .arm(let copy):
            return GlassConfirmation(
                title: pending.title, message: pending.body,
                actions: [
                    .cancel(copy.decline, action: cancel),
                    GlassModalAction(copy.confirm, isDefault: true) { apply(pending.mode) },
                ],
                onCancel: cancel)
        }
    }

    private func caption(_ line: String) -> some View {
        Text(line)
            .glassType(GlassTokens.TypeScale.caption)
            .foregroundStyle(GlassColor.textSecondary)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// The core's Ignore words for this folder and its waiting count; nil
    /// (no button) when the folder cannot be ignored or the words are
    /// missing.
    private var ignoreCopy: ProjectIgnoreCopy? {
        guard folder.mode != nil, folder.mode != .ignore, folder.offerableModes.contains(.ignore) else { return nil }
        return ProjectIgnoreCopy.decode(fromJSON: TCCoreCopy.projectIgnoreCopyJSON(
            project: folder.label, pending: folder.sessions.count))
    }

    // MARK: Mode changes

    private func request(_ wanted: ProjectMode) {
        route(TracesTreeView.modeChange(folder, wanted), wanted)
    }

    private func route(_ change: TracesTreeView.ModeChange, _ mode: ProjectMode) {
        switch change {
        case .noop, .unavailable: return
        case .apply: apply(mode)
        case .confirm(let pending): confirming = pending
        }
    }

    private func apply(_ mode: ProjectMode) {
        confirming = nil
        Task { await store.setFolderMode(folder, mode, promised: folder.sessions.count) }
    }

    // MARK: Rules

    /// Whether Submit all (and Submit all as) is offered: the shared table
    /// offers Contribute for the daemon's counts, the core is attached, and
    /// the folder is not one that is never sent. Review Focus 3: a Never
    /// folder shows its rule and nothing to submit.
    static func offersSubmitAll(_ folder: TracesTree.FolderNode, store: TracesStore) -> Bool {
        store.groupOffer(folder).offersContribute && store.mayContributeFolder(folder)
    }

    /// One outcome for every eligible session in the folder.
    static func submitAllAs(_ verdict: ContributorVerdict, folder: TracesTree.FolderNode, store: TracesStore) async {
        await store.contributeFolder(folder, verdict: verdict)
    }

    /// A mode's one name, from the disclosure bundle's `folder_mode_labels`
    /// (`TracesStore.disclosureCopy`). Nil when the bundle will not decode:
    /// then the picker names nothing and Submit all as is not offered.
    static func ruleLabel(_ mode: ProjectMode) -> String? {
        TracesStore.disclosureCopy?.folderModeLabels[mode.rawValue]
    }

    /// The modal's count line, filled.
    static func applyLine(_ count: Int, words: MonitorTracesCopy) -> String {
        count == 1
            ? words.inspector.applyOutcomeOne
            : FirstRunCopy.fill(words.inspector.applyOutcome, ["count": String(count)])
    }

    /// Ron's "Submit all eligible (n)", in the core's words.
    static func submitAllTitle(_ count: Int, words: MonitorTracesCopy) -> String {
        FirstRunCopy.fill(words.inspector.submitAllEligible, ["count": String(count)])
    }

    /// "n waiting sessions", the singular its own line.
    static func waitingLine(_ count: Int, words: MonitorTracesCopy) -> String {
        count == 1
            ? words.inspector.waitingSessionsOne
            : FirstRunCopy.fill(words.inspector.waitingSessions, ["count": String(count)])
    }

    /// "n eligible", then the core's withheld line after a middle dot, as
    /// #1146 draws it under every folder that offers Submit all; nil when
    /// nothing in the folder can be submitted.
    static func eligibleLine(
        _ folder: TracesTree.FolderNode, offer: GroupSubmitOffer, words: MonitorTracesCopy
    ) -> String? {
        guard folder.contributableCount != nil else {
            // The daemon asked no eligibility question: the folder submits
            // whole, and the line counts what Submit all eligible counts
            // (#1146 draws "{n} eligible" beside it).
            return offer.offersContribute ? FirstRunCopy.fill(words.tree.eligibleCount, ["count": String(offer.count)]) : nil
        }
        let eligible = FirstRunCopy.fill(words.tree.eligibleCount, ["count": String(offer.count)])
        guard let withheld = offer.withheldLine, !withheld.isEmpty else { return eligible }
        return "\(eligible) · \(withheld)"
    }

    /// The folder's shared count: the history records from this project
    /// that still stand as contributed, as the Summary's statistics count
    /// them. A dash when history is unread or capped: never a part count.
    static func shared(_ folder: TracesTree.FolderNode, history: [DaemonData.HistoryRow]?) -> String {
        guard let rows = SummaryFacts.wholeHistory(history) else { return "—" }
        return String(rows.filter {
            $0.projectId == folder.id && SummaryFacts.contributedStatuses.contains($0.status ?? "")
        }.count)
    }

    /// "Project · <tool>" for the tool most of its sessions came from, or
    /// the plain word when none is known.
    static func headerSub(_ folder: TracesTree.FolderNode, words: MonitorTracesCopy) -> String {
        guard let tool = TracesTree.majorityTool(folder.sessions) else { return words.inspector.project }
        return FirstRunCopy.fill(words.inspector.projectOf, ["tool": tool.displayName])
    }

    /// Path, waiting and size. An empty path (the unresolvable bucket's)
    /// is a dash. The size is a dash unless every session
    /// reported one: a partial sum is never shown as the folder's size.
    static func rows(_ folder: TracesTree.FolderNode, words: MonitorTracesCopy) -> [GlassKeyValueList.Item] {
        let sizes = folder.sessions.map(\.sizeBytes)
        let size = sizes.contains(where: { $0 == nil })
            ? "—"
            : ByteCountFormatter.string(fromByteCount: Int64(sizes.compactMap { $0 }.reduce(0, +)), countStyle: .memory)
        return [
            .init(words.inspector.path, folder.path.flatMap { $0.isEmpty ? nil : $0 } ?? "—", mono: true),
            .init(MonitorWords.waiting, String(folder.sessions.count)),
            .init(words.size, size),
        ]
    }
}

/// Ron's `SubmitAllAsControl`: one outcome for every eligible session, in
/// the core's words, or Cancel. Opened from Submit all as on the Folder
/// inspector's split button and on a folder row's (owner, 2026-10-19).
struct SubmitAllAsModal: View {
    let folder: TracesTree.FolderNode
    let store: TracesStore
    let words: MonitorTracesCopy
    /// Closes the modal; called before an outcome is sent too.
    let close: () -> Void

    var body: some View {
        if let outcome = store.disclosure?.outcome {
            let busy = store.writing.contains(folder.id)
            GlassModal(
                title: outcome.submitAllAs, subtitle: outcome.submitAllAsTooltip, width: .narrow,
                actions: [.cancel(words.inspector.cancel, action: close)],
                onCancel: close
            ) {
                GlassModalBody {
                    Text(FolderInspector.applyLine(store.groupOffer(folder).count, words: words))
                        .glassType(GlassTokens.TypeScale.label)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                    HStack(spacing: GlassTokens.Space.s4) {
                        Button(outcome.worked) { send(.worked) }
                            .buttonStyle(GlassButtonStyle(.primary, small: true))
                        Button(outcome.partly) { send(.partly) }
                            .buttonStyle(GlassButtonStyle(.glass))
                        Button(outcome.failed) { send(.failed) }
                            .buttonStyle(GlassButtonStyle(.glass))
                    }
                    .disabled(busy)
                }
            }
        }
    }

    private func send(_ verdict: ContributorVerdict) {
        close()
        Task { await FolderInspector.submitAllAs(verdict, folder: folder, store: store) }
    }
}
