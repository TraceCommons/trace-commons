import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// Ron's Folder inspector (#1146, `waiting-page.tsx` `FolderInspector`):
/// the folder's path, what is waiting and its size; its contribution rule
/// with the mode picker; and, while sessions wait, Submit all, Submit all
/// as (a modal with one outcome for every eligible session) and Ignore.
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

    /// A mode change waiting on its confirmation.
    @State private var confirming: TracesTreeView.Confirmation?
    /// Whether Submit all as's modal is open.
    @State private var choosingVerdict = false

    var body: some View {
        if let words = store.words {
            VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                header(words)
                GlassEyebrowCard(words.inspector.project) {
                    GlassKeyValueList(Self.rows(folder, words: words))
                }
                GlassEyebrowCard(words.inspector.contributionRule) {
                    rule(words)
                }
                if !folder.sessions.isEmpty && folder.mode != .ignore {
                    GlassEyebrowCard(words.inspector.decisions) {
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

    private func header(_ words: MonitorTracesCopy) -> some View {
        HStack(spacing: GlassTokens.Space.s4) {
            GlassToolTile(.folder, large: true)
            VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                Text(folder.label)
                    .glassType(GlassTokens.TypeScale.title)
                    .foregroundStyle(GlassColor.textPrimary)
                    .lineLimit(2)
                Text(Self.headerSub(folder, words: words))
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textTertiary)
            }
        }
    }

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
            ForEach(store.disclosureLines(folder.disclosure), id: \.self) { caption($0) }
            if folder.isBucket { caption(ProjectCopy.unresolvedBucketNote) }
        } else {
            caption(words.inspector.noRule)
        }
    }

    /// What is waiting, and what to do with all of it at once.
    @ViewBuilder
    private func decisions(_ words: MonitorTracesCopy) -> some View {
        let offer = store.groupOffer(folder)
        let busy = store.writing.contains(folder.id)
        caption(FirstRunCopy.fill(words.counts.waitingCount, ["count": String(folder.sessions.count)]))
        if let withheld = offer.withheldLine, !withheld.isEmpty {
            caption(withheld)
        }
        if let refused = store.writeErrors[folder.id] {
            GlassNotice(tone: .outside, title: words.line(for: refused)) { EmptyView() }
        }
        HStack(spacing: GlassTokens.Space.s4) {
            if Self.offersSubmitAll(folder, store: store) {
                Button(store.submittingFolder == folder.id
                    ? words.tree.submitting : TracesTreeView.submitTitle(offer.count, words: words))
                {
                    Task { await store.contributeFolder(folder, verdict: nil) }
                }
                .buttonStyle(GlassButtonStyle(.glass))
                .help(TracesTreeView.submitHelp(offer.withheldLine, words: words))
                if let outcome = store.disclosure?.outcome {
                    Button(outcome.submitAllAs) { choosingVerdict = true }
                        .buttonStyle(GlassButtonStyle(.glass))
                        .help(outcome.submitAllAsTooltip)
                }
            }
            if let copy = ignoreCopy {
                Button(copy.button) { route(TracesTreeView.modeChange(folder, .ignore), .ignore) }
                    .buttonStyle(GlassButtonStyle(.glass))
                    .help(copy.tooltip)
            }
        }
        .disabled(busy)
    }

    /// Ron's `SubmitAllAsControl`: one outcome for every eligible session,
    /// in the core's words, or Cancel.
    @ViewBuilder
    private func submitAllAsModal(_ words: MonitorTracesCopy) -> some View {
        if let outcome = store.disclosure?.outcome {
            let busy = store.writing.contains(folder.id)
            GlassModal(
                title: outcome.submitAllAs, subtitle: outcome.submitAllAsTooltip, width: .narrow,
                actions: [.cancel(words.inspector.cancel) { choosingVerdict = false }],
                onCancel: { choosingVerdict = false }
            ) {
                GlassModalBody {
                    Text(Self.applyLine(store.groupOffer(folder).count, words: words))
                        .glassType(GlassTokens.TypeScale.label)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                    HStack(spacing: GlassTokens.Space.s4) {
                        Button(outcome.worked) {
                            choosingVerdict = false
                            Task { await Self.submitAllAs(.worked, folder: folder, store: store) }
                        }
                        .buttonStyle(GlassButtonStyle(.primary, small: true))
                        Button(outcome.partly) {
                            choosingVerdict = false
                            Task { await Self.submitAllAs(.partly, folder: folder, store: store) }
                        }
                        .buttonStyle(GlassButtonStyle(.glass))
                        Button(outcome.failed) {
                            choosingVerdict = false
                            Task { await Self.submitAllAs(.failed, folder: folder, store: store) }
                        }
                        .buttonStyle(GlassButtonStyle(.glass))
                    }
                    .disabled(busy)
                }
            }
        }
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
            : ByteCountFormatter.string(fromByteCount: Int64(sizes.compactMap { $0 }.reduce(0, +)), countStyle: .file)
        return [
            .init(words.inspector.path, folder.path.flatMap { $0.isEmpty ? nil : $0 } ?? "—", mono: true),
            .init(MonitorWords.waiting, String(folder.sessions.count)),
            .init(words.size, size),
        ]
    }
}
