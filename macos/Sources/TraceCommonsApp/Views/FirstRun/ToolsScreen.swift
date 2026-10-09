import AppKit
import SwiftUI
import TCBridge
import TCDesign
import TCShellCore
import UniformTypeIdentifiers

/// What a folder picked with "add your tool" turned out to be.
enum AddToolOutcome: Equatable {
    /// One kind matched; the folder is added as that kind.
    case added(AddedFolder)
    /// More than one kind matched (a flat folder of `.json` files is both an
    /// OpenCode export and a trajectory export): the person picks which.
    /// `kinds` holds only the matches this build can offer.
    case ask(path: String, kinds: [AddedFolder.Kind])
    /// Nothing this build can declare matched, or the core could not
    /// describe the folder. Nothing is added.
    case refused
}

/// The Tools screen's decisions, apart from the view so they can be tested.
/// Every write goes through `FirstRunState.add(_:)`, `answerAdded`, or, on
/// a tool row, `ToolAnswerRowLayout.select`.
enum ToolsScreenLayout {
    /// The `tc_describe_folder` JSON for `path` as an outcome. Decoded with
    /// `FolderMatch.decodeList`, which keeps every row: a dropped trajectory
    /// row would turn a question into a certainty. A nil or undecodable
    /// answer is a refusal, so a folder the core could not read is never
    /// added on a guess.
    static func outcome(path: String, json: String?) -> AddToolOutcome {
        guard let json, let matches = try? FolderMatch.decodeList(from: json) else { return .refused }
        return outcome(path: path, matches: matches)
    }

    static func outcome(path: String, matches: [FolderMatch]) -> AddToolOutcome {
        let offerable = matches.compactMap { match -> AddedFolder.Kind? in
            switch match.kind {
            case .source(let kind): return .source(kind)
            case .trajectory: return .trajectory
            case .unrecognised: return nil
            }
        }
        if matches.count == 1, let kind = offerable.first {
            return .added(AddedFolder(kind: kind, path: matches[0].path))
        }
        // More than one match is a question even when only one of them can
        // be offered: the core said the folder is not certainly that kind.
        if matches.count > 1, !offerable.isEmpty {
            return .ask(path: path, kinds: offerable)
        }
        return .refused
    }

    /// Apply an outcome. An added folder is written as a row of its own,
    /// unanswered (Ron's review of #1235, item 3); a question writes nothing
    /// until it is answered. False for a refusal.
    @discardableResult
    static func apply(_ outcome: AddToolOutcome, to state: inout FirstRunState) -> Bool {
        switch outcome {
        case .added(let folder):
            state.add(folder)
            return true
        case .ask:
            return true
        case .refused:
            return false
        }
    }

    /// The person's answer to an ambiguous folder: which kind it is. Its
    /// row is then added, unanswered like any other.
    static func choose(_ kind: AddedFolder.Kind, path: String, in state: inout FirstRunState) {
        state.add(AddedFolder(kind: kind, path: path))
    }

    /// The folders added for a tool, each a row of its own. A folder of
    /// exported traces has its own row (`trajectoryFolder`).
    static func addedRows(_ state: FirstRunState) -> [AddedFolder] {
        state.addedFolders.filter { $0.kind != .trajectory }
    }

    /// An added row's answer as its picker shows it; nil is "Choose…".
    static func addedAnswer(_ folder: AddedFolder) -> ToolAnswer? {
        folder.watched.map { $0 ? .watch : .dontUse }
    }

    /// Answer an added row.
    static func selectAdded(_ answer: ToolAnswer?, path: String, in state: inout FirstRunState) {
        state.answerAdded(path: path, watched: answer.map { $0 == .watch })
    }

    /// The core's line beside an added folder that reads Watch while its
    /// tool is watched in another row too, naming the tool; nil otherwise.
    static func conflict(_ folder: AddedFolder, in state: FirstRunState, copy: FirstRunCopy.Tools) -> String? {
        guard folder.watched == true, case .source(let kind) = folder.kind, state.watchedTwice.contains(kind)
        else { return nil }
        return FirstRunCopy.fill(copy.oneFolderPerTool, ["tool": kind.displayName])
    }

    /// An added row's folder button: the newly picked folder, described,
    /// replaces the row. Picking a folder for a row watches it, as on every
    /// other row; a question replaces the row with the question, and a
    /// refusal leaves the row as it was. False for a refusal.
    @discardableResult
    static func replace(path: String, with outcome: AddToolOutcome, in state: inout FirstRunState) -> Bool {
        switch outcome {
        case .refused:
            return false
        case .ask:
            state.addedFolders.removeAll { $0.path == path }
            return true
        case .added(let folder):
            state.addedFolders.removeAll { $0.path == path }
            state.add(folder)
            state.answerAdded(path: folder.path, watched: true)
            return true
        }
    }

    /// One option on an ambiguous folder's picker: a kind it matched, by its
    /// index in the question's kinds, or the core's "Neither".
    enum PendingAnswer: Hashable {
        case kind(Int)
        case neither
    }

    /// The ambiguous folder's options: each kind it matched, then the
    /// core's `neither`, which closes the question with nothing added.
    static func pendingOptions(
        _ kinds: [AddedFolder.Kind], copy: FirstRunCopy.Tools
    ) -> [GlassPickerOption<PendingAnswer>] {
        kinds.indices.map { GlassPickerOption(name(kinds[$0], copy: copy), value: .kind($0)) }
            + [GlassPickerOption(copy.neither, value: .neither, dot: .off)]
    }

    /// Answer an ambiguous folder. A kind is added as `choose` adds it;
    /// Neither leaves the state as it is, so the folder is attributed to no
    /// tool. True when the question is closed; an index the picker never
    /// offered keeps it open and writes nothing.
    static func answer(
        _ answer: PendingAnswer, path: String, kinds: [AddedFolder.Kind], in state: inout FirstRunState
    ) -> Bool {
        switch answer {
        case .neither:
            return true
        case .kind(let index):
            guard kinds.indices.contains(index) else { return false }
            choose(kinds[index], path: path, in: &state)
            return true
        }
    }

    /// The core's line for a refused folder.
    static func refusal(_ copy: FirstRunCopy.Tools) -> String {
        copy.addToolRefused
    }

    /// An option's name: a tool's display name, or the core's label for a
    /// folder of exported traces.
    static func name(_ kind: AddedFolder.Kind, copy: FirstRunCopy.Tools) -> String {
        switch kind {
        case .source(let source): return source.displayName
        case .trajectory: return copy.trajectoryLabel
        }
    }

    /// The core's question for an ambiguous folder, naming it as its card
    /// does (`folderName`).
    static func question(path: String, copy: FirstRunCopy.Tools) -> String {
        FirstRunCopy.fill(copy.whichKind, ["folder": folderName(path)])
    }

    /// A picked folder's name: its last path component.
    static func folderName(_ path: String) -> String {
        URL(fileURLWithPath: path).lastPathComponent
    }

    /// The trajectory row's picker. "Not used" withdraws the folder;
    /// the placeholder and "Watch this folder" leave it as it is.
    static func selectTrajectory(_ answer: ToolAnswer?, in state: inout FirstRunState) {
        if answer == .dontUse { state.withdrawTrajectory() }
    }

    /// Continue: discovery read, nothing committing, no question open, and
    /// every row answered.
    static func canContinue(
        discovered: [SourceCandidate]?, state: FirstRunState, pending: Bool, isCommitting: Bool
    ) -> Bool {
        guard let discovered, !isCommitting, !pending else { return false }
        return FirstRunNavigation.canContinue(
            state, candidates: rows(discovered, state: state), requiredScope: nil)
    }

    /// Whether a described folder may still change the state. A drop and an
    /// off-main describe answer later; by then a commit may hold the start
    /// snapshot, or the screen may have moved on. Both tool screens take
    /// folders: Quick's Folders and Custom's Tools (owner, 2026-10-08).
    static func acceptsFolder(step: FirstRunStep, isCommitting: Bool) -> Bool {
        (step == .tools || step == .folders) && !isCommitting
    }

    /// Discovery's rows. A kind discovery did not find that the state
    /// watches through its own row (a restored state) is shown found at that
    /// folder, so the row does not vanish while the watch stays declared.
    /// Folders added with the tile are rows of their own (`addedRows`).
    static func rows(_ discovered: [SourceCandidate], state: FirstRunState) -> [SourceCandidate] {
        var rows = discovered.map { candidate in
            personalPath(for: candidate.source, discovered: discovered, in: state)
                .map { added(candidate.source, at: $0) } ?? candidate
        }
        for kind in SourceKind.allCases where !rows.contains(where: { $0.source == kind }) {
            if let path = personalPath(for: kind, discovered: discovered, in: state) {
                rows.append(added(kind, at: path))
            }
        }
        return rows
    }

    /// A tool card's caption: "Added by you" for a folder the person gave
    /// through a missing tool's row, nothing otherwise. Tool cards show no
    /// session count and no "not found" line (owner, 2026-10-08).
    static func meta(
        for candidate: SourceCandidate, in state: FirstRunState, discovered: [SourceCandidate],
        copy: FirstRunCopy
    ) -> String? {
        personalPath(for: candidate.source, discovered: discovered, in: state) != nil ? copy.tools.addedByYou : nil
    }

    /// The added trajectory folder, which has no tool row of its own.
    static func trajectoryFolder(in state: FirstRunState) -> AddedFolder? {
        state.addedFolders.first { $0.kind == .trajectory }
    }

    /// The folder the person gave for `kind` through its row: a watch of a
    /// kind discovery did not find. Nil for discovery's own row.
    private static func personalPath(
        for kind: SourceKind, discovered: [SourceCandidate], in state: FirstRunState
    ) -> String? {
        let found = discovered.contains { $0.source == kind && $0.exists }
        if !found, case .watch(let path) = state.rowAnswer(kind) { return path }
        return nil
    }

    private static func added(_ kind: SourceKind, at path: String) -> SourceCandidate {
        SourceCandidate(
            source: kind, path: path, exists: true, sessionCount: 0, mostRecent: nil, relocatedByEnv: false)
    }
}

/// A folder that matched more than one kind, waiting for the person.
struct PendingFolder: Equatable {
    let path: String
    let kinds: [AddedFolder.Kind]
}

/// The add-tool flow the two tool screens share (owner, 2026-10-08: Quick's
/// Folders and Custom's Tools both offer it, with one component and one
/// behaviour): a picked or dropped folder is described by the core off the
/// main actor, and its answer lands only while a tool screen is up and
/// nothing is committing (`ToolsScreenLayout.acceptsFolder`).
@MainActor
final class AddToolModel: ObservableObject {
    @Published var pending: PendingFolder?
    @Published var refused = false

    /// Describe a picked folder and act on what the core reports.
    /// `replacing` names an added row whose folder button picked `path`.
    func describe(_ path: String, replacing: String? = nil, runner: FirstRunRunner) {
        guard Self.accepting(runner) else { return }
        Task.detached(priority: .userInitiated) {
            let json = TCDiscovery.describeFolderJSON(path)
            await MainActor.run {
                self.land(ToolsScreenLayout.outcome(path: path, json: json), replacing: replacing, runner: runner)
            }
        }
    }

    func land(_ outcome: AddToolOutcome, replacing: String? = nil, runner: FirstRunRunner) {
        guard Self.accepting(runner) else { return }
        if let replacing {
            refused = !ToolsScreenLayout.replace(path: replacing, with: outcome, in: &runner.state)
        } else {
            refused = !ToolsScreenLayout.apply(outcome, to: &runner.state)
        }
        if case .ask(let path, let kinds) = outcome {
            pending = PendingFolder(path: path, kinds: kinds)
        } else {
            pending = nil
        }
    }

    static func accepting(_ runner: FirstRunRunner) -> Bool {
        ToolsScreenLayout.acceptsFolder(step: runner.state.step, isCommitting: runner.isCommitting)
    }
}

/// The tool cards both tool screens show, in the frame's scrolling body:
/// discovery's rows, the folders added with the tile, the trajectory folder
/// and an open question. The add tile is not among them: it is the frame's
/// pinned area, always in view (`AddToolTile`).
struct ToolList: View {
    let copy: FirstRunCopy
    @ObservedObject var runner: FirstRunRunner
    @ObservedObject var adding: AddToolModel
    let discovery: DiscoveredRows
    let installURL: (SourceKind) -> URL?
    let retry: () -> Void

    var body: some View {
        Group {
            switch discovery {
            case .found(let discovered):
                VStack(spacing: GlassTokens.Space.s4) {
                    ForEach(ToolsScreenLayout.rows(discovered, state: runner.state), id: \.source) { candidate in
                        ToolAnswerRow(
                            copy: copy.folders,
                            choose: copy.frame.choose,
                            candidate: candidate,
                            meta: ToolsScreenLayout.meta(
                                for: candidate, in: runner.state, discovered: discovered, copy: copy),
                            state: $runner.state,
                            installURL: installURL(candidate.source)
                        )
                    }
                    ForEach(ToolsScreenLayout.addedRows(runner.state), id: \.path) { folder in
                        addedRow(folder)
                    }
                    if let trajectory = ToolsScreenLayout.trajectoryFolder(in: runner.state) {
                        let name = ToolsScreenLayout.name(.trajectory, copy: copy.tools)
                        let question = FirstRunCopy.fill(copy.folders.watchQuestion, ["tool": name])
                        folderCard(name: name, path: trajectory.path, meta: copy.tools.addedByYou) {
                            GlassPicker(
                                question,
                                selection: trajectoryChoice,
                                options: [
                                    GlassPickerOption(copy.folders.watch, value: ToolAnswer.watch, dot: .on),
                                    GlassPickerOption(copy.folders.dontUse, value: ToolAnswer.dontUse, dot: .off),
                                ],
                                placeholder: copy.frame.choose
                            )
                            .fixedSize()
                        }
                    }
                    if let pending = adding.pending {
                        let question = ToolsScreenLayout.question(path: pending.path, copy: copy.tools)
                        folderCard(name: ToolsScreenLayout.folderName(pending.path), path: pending.path, meta: nil) {
                            GlassPicker(
                                question,
                                selection: pendingChoice,
                                options: ToolsScreenLayout.pendingOptions(pending.kinds, copy: copy.tools),
                                placeholder: copy.frame.choose
                            )
                            .fixedSize()
                        }
                    }
                }
                .disabled(!FoldersScreenLayout.rowsEnabled(isCommitting: runner.isCommitting))
            case .failed:
                HStack(spacing: GlassTokens.Space.s4) {
                    Text(discovery.failureLine(copy.folders) ?? "")
                        .glassType(GlassTokens.TypeScale.body)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                    Button(copy.folders.retry, action: retry)
                        .buttonStyle(GlassButtonStyle(.secondary))
                }
            case .loading:
                HStack(spacing: GlassTokens.Space.s4) {
                    GlassSpinner()
                    Text(copy.folders.loading)
                        .glassType(GlassTokens.TypeScale.body)
                        .foregroundStyle(GlassColor.textSecondary)
                }
            }
        }
    }

    /// A folder added with the tile (Ron's custom tool row): named after
    /// the folder, a folder tile, "Added by you", and the same picker and
    /// folder button as every other row, centred beside the name, unanswered
    /// until the person answers.
    private func addedRow(_ folder: AddedFolder) -> some View {
        let name = ToolsScreenLayout.folderName(folder.path)
        let question = FirstRunCopy.fill(copy.folders.watchQuestion, ["tool": name])
        return GlassCard {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                folderRow(name: name, path: folder.path, meta: copy.tools.addedByYou) {
                    GlassPicker(
                        question,
                        selection: Binding(
                            get: { ToolsScreenLayout.addedAnswer(folder) },
                            set: { ToolsScreenLayout.selectAdded($0, path: folder.path, in: &runner.state) }),
                        options: [
                            GlassPickerOption(copy.folders.watch, value: ToolAnswer.watch, dot: .on),
                            GlassPickerOption(copy.folders.dontUse, value: ToolAnswer.dontUse, dot: .off),
                        ],
                        placeholder: copy.frame.choose
                    )
                    .fixedSize()
                    GlassFolderButton(FirstRunCopy.fill(copy.folders.chooseFolder, ["tool": name])) {
                        if let path = FolderPanel.choose() { adding.describe(path, replacing: folder.path, runner: runner) }
                    }
                    .fixedSize()
                }
                if let line = ToolsScreenLayout.conflict(folder, in: runner.state, copy: copy.tools) {
                    Text(line)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
    }

    private func folderCard<Trailing: View>(
        name: String, path: String, meta: String?, @ViewBuilder trailing: () -> Trailing
    ) -> some View {
        GlassCard {
            folderRow(name: name, path: path, meta: meta, trailing: trailing)
        }
    }

    /// One compact row: the folder tile, its name, path and caption, and
    /// the answer centred beside them.
    private func folderRow<Trailing: View>(
        name: String, path: String, meta: String?, @ViewBuilder trailing: () -> Trailing
    ) -> some View {
        HStack(alignment: .center, spacing: GlassTokens.Space.s6) {
            GlassToolTile(.folder, large: true)
            VStack(alignment: .leading, spacing: 0) {
                Text(name)
                    .glassType(GlassTokens.TypeScale.bodyStrong)
                    .foregroundStyle(GlassColor.textPrimary)
                    .lineLimit(1)
                Text(path)
                    .glassType(GlassTokens.TypeScale.mono)
                    .foregroundStyle(GlassColor.textTertiary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .help(path)
                if let meta {
                    Text(meta)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textTertiary)
                        .lineLimit(1)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            trailing()
        }
    }

    private var trajectoryChoice: Binding<ToolAnswer?> {
        Binding(
            get: { .watch },
            set: { ToolsScreenLayout.selectTrajectory($0, in: &runner.state) }
        )
    }

    private var pendingChoice: Binding<ToolsScreenLayout.PendingAnswer?> {
        Binding(
            get: { nil },
            set: { answer in
                guard let answer, let pending = adding.pending,
                    ToolsScreenLayout.answer(answer, path: pending.path, kinds: pending.kinds, in: &runner.state)
                else { return }
                adding.pending = nil
            }
        )
    }
}

/// Ron's add box (`ftux-add-tool`): a dashed border, a "+" tile, and the
/// purple border over a faint fill while a folder is dragged over it.
/// Click to pick a folder, or drop one on it. Both tool screens pin it under
/// their cards, so it stays in view while the cards scroll.
struct AddToolTile: View {
    let copy: FirstRunCopy
    @ObservedObject var runner: FirstRunRunner
    @ObservedObject var adding: AddToolModel
    @State private var dragging = false

    var body: some View {
        Button {
            if let path = FolderPanel.choose() { adding.describe(path, runner: runner) }
        } label: {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                HStack(spacing: GlassTokens.Space.s6) {
                    GlassToolTile(.add, large: true)
                    VStack(alignment: .leading, spacing: 0) {
                        Text(copy.tools.addTool)
                            .glassType(GlassTokens.TypeScale.bodyStrong)
                            .foregroundStyle(GlassColor.textPrimary)
                        Text(copy.tools.addToolCaption)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textTertiary)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                if adding.refused {
                    Text(ToolsScreenLayout.refusal(copy.tools))
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                }
            }
            .padding(.vertical, GlassTokens.Space.cardPaddingVertical)
            .padding(.horizontal, GlassTokens.Space.cardPaddingHorizontal)
            .background(
                RoundedRectangle(cornerRadius: GlassTokens.Radius.card, style: .continuous)
                    .fill(dragging ? GlassTokens.Color.tintNeutral.color : .clear))
            .overlay(
                RoundedRectangle(cornerRadius: GlassTokens.Radius.card, style: .continuous)
                    .strokeBorder(
                        dragging ? GlassTokens.Color.purpleText.color : GlassColor.ink(0.28),
                        style: StrokeStyle(lineWidth: 1, dash: [4, 3])))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!FoldersScreenLayout.rowsEnabled(isCommitting: runner.isCommitting))
        .onDrop(of: [UTType.fileURL], isTargeted: $dragging) { providers in
            guard let provider = providers.first else { return false }
            _ = provider.loadObject(ofClass: URL.self) { url, _ in
                guard let url else { return }
                let isFolder = (try? url.resourceValues(forKeys: [.isDirectoryKey]))?.isDirectory == true
                let path = url.path
                Task { @MainActor in
                    guard AddToolModel.accepting(runner) else { return }
                    if isFolder { adding.describe(path, runner: runner) } else { adding.refused = true }
                }
            }
            return true
        }
    }
}

/// Ron's Tools screen (#1030 `tool-screens.tsx` W-4), Custom setup's tool
/// list: the tool cards, scrolling, and the "add your tool" tile pinned
/// under them. A click opens a folder panel; a drop takes a folder.
/// Continue commits `.leaveRoots`, which starts the daemon.
struct ToolsScreen: View {
    let copy: FirstRunCopy
    @ObservedObject var runner: FirstRunRunner
    /// Where each tool's "Get {tool}" leads: the core's install pages
    /// (`FirstRunCopy.Folders.installURL(for:)`), passed by the host.
    var installURL: (SourceKind) -> URL? = { _ in nil }

    @State private var discovery: DiscoveredRows = .loading
    @State private var onboarding = TCOnboardingCopy.load()
    @StateObject private var adding = AddToolModel()

    var body: some View {
        FirstRunFrame(
            copy: copy,
            state: $runner.state,
            isCommitting: runner.isCommitting,
            notice: FoldersScreenLayout.notice(for: runner.failure, copy: copy, onboarding: onboarding),
            footer: FirstRunFooter(
                title: copy.frame.continueButton,
                isEnabled: canContinue,
                busy: runner.isCommitting,
                cancel: FoldersScreenLayout.signInCancel(waiting: runner.signInWaiting, copy: copy) {
                    Task { await runner.cancelSignIn() }
                },
                action: { Task { await runner.commit(.leaveRoots) } }
            )
        ) {
            title
        } content: {
            ToolList(
                copy: copy, runner: runner, adding: adding, discovery: discovery, installURL: installURL,
                retry: refreshDiscovery)
        } pinned: {
            if discovery.rows != nil {
                AddToolTile(copy: copy, runner: runner, adding: adding)
            }
        }
        .task {
            if discovery == .loading { refreshDiscovery() }
        }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            refreshDiscovery()
        }
    }

    /// Discovery's rows, and what they say is not on this Mac: such a tool
    /// is not asked (`FirstRunState.recordDiscovery`). Not recorded while a
    /// commit holds the state it started from.
    private func refreshDiscovery() {
        discovery = FoldersScreenLayout.discovered(TCDiscovery.sourcesJSON(), keeping: discovery)
        if let rows = discovery.rows, !runner.isCommitting { runner.state.recordDiscovery(rows) }
    }

    private var canContinue: Bool {
        ToolsScreenLayout.canContinue(
            discovered: discovery.rows, state: runner.state, pending: adding.pending != nil,
            isCommitting: runner.isCommitting)
    }

    private var title: some View {
        FirstRunTitle(light: copy.tools.titleLight, bold: copy.tools.titleBold)
    }
}
