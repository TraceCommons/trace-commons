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
/// Every write goes through `FirstRunState.add(_:)` or, on a row,
/// `ToolAnswerRowLayout.select`, so one answer per kind is kept.
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

    /// Apply an outcome. An added folder is written; a question writes
    /// nothing until it is answered. False for a refusal.
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

    /// The person's answer to an ambiguous folder.
    static func choose(_ kind: AddedFolder.Kind, path: String, in state: inout FirstRunState) {
        state.add(AddedFolder(kind: kind, path: path))
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

    /// The trajectory row's picker. "I don't use it" withdraws the folder;
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
    /// snapshot, or the screen may have moved on.
    static func acceptsFolder(step: FirstRunStep, isCommitting: Bool) -> Bool {
        step == .tools && !isCommitting
    }

    /// Discovery's rows, with each folder the person gave shown as a found
    /// tool at that folder (Ron's custom tools are found). A folder is the
    /// person's when they added it with the tile, or when the state watches
    /// a kind discovery did not find -- its row's own folder button writes
    /// through `answer`, which drops the added folder, and the row must not
    /// vanish while the watch stays declared.
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

    /// Ron's compact meta: "Added by you" for a folder the person gave, the
    /// session count alone for a found tool, discovery's evidence otherwise.
    static func meta(
        for candidate: SourceCandidate, in state: FirstRunState, discovered: [SourceCandidate],
        copy: FirstRunCopy, now: Date
    ) -> String {
        if personalPath(for: candidate.source, discovered: discovered, in: state) != nil {
            return copy.tools.addedByYou
        }
        if candidate.exists {
            return FirstRunCopy.fill(copy.frame.sessionCount, ["count": String(candidate.sessionCount)])
        }
        return candidate.evidence(now: now)
    }

    /// The added trajectory folder, which has no tool row of its own.
    static func trajectoryFolder(in state: FirstRunState) -> AddedFolder? {
        state.addedFolders.first { $0.kind == .trajectory }
    }

    /// The folder the person gave for `kind`: one added with the tile, or a
    /// watch of a kind discovery did not find. Nil for discovery's own row.
    private static func personalPath(
        for kind: SourceKind, discovered: [SourceCandidate], in state: FirstRunState
    ) -> String? {
        if let folder = state.addedFolders.first(where: { $0.kind == .source(kind) }) {
            return folder.path
        }
        let found = discovered.contains { $0.source == kind && $0.exists }
        if !found, case .watch(let path) = state.sessionRoots[kind] { return path }
        return nil
    }

    private static func added(_ kind: SourceKind, at path: String) -> SourceCandidate {
        SourceCandidate(
            source: kind, path: path, exists: true, sessionCount: 0, mostRecent: nil, relocatedByEnv: false)
    }
}

/// Ron's Tools screen (#1030 `tool-screens.tsx` W-4), Custom setup's tool
/// list: the Folders rows with compact meta, then the "add your tool" tile.
/// A click opens a folder panel; a drop takes a folder. Continue commits
/// `.leaveRoots`, which starts the daemon.
struct ToolsScreen: View {
    let copy: FirstRunCopy
    @ObservedObject var runner: FirstRunRunner
    /// Where each tool's "Get {tool}" leads: the core's install pages
    /// (`FirstRunCopy.Folders.installURL(for:)`), passed by the host.
    var installURL: (SourceKind) -> URL? = { _ in nil }
    /// Whether Back to Join is offered (`FoldersScreenLayout.backAction`).
    var offersJoin = true

    @State private var discovery: DiscoveredRows = .loading
    @State private var onboarding = TCOnboardingCopy.load()
    /// A folder that matched more than one kind, waiting for the person.
    @State private var pending: (path: String, kinds: [AddedFolder.Kind])?
    @State private var refused = false
    @State private var dragging = false

    var body: some View {
        FirstRunFrame(
            copy: copy,
            state: $runner.state,
            onBack: FoldersScreenLayout.backAction(isCommitting: runner.isCommitting, offersJoin: offersJoin) {
                runner.state = FirstRunNavigation.back(runner.state)
            },
            isCommitting: runner.isCommitting,
            notice: FoldersScreenLayout.notice(for: runner.failure, copy: copy, onboarding: onboarding),
            footer: FirstRunFooter(
                title: copy.frame.continueButton,
                isEnabled: canContinue,
                action: { Task { await runner.commit(.leaveRoots) } }
            )
        ) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
                title
                switch discovery {
                case .found(let discovered):
                    ScrollView {
                        VStack(spacing: GlassTokens.Space.s4) {
                            ForEach(ToolsScreenLayout.rows(discovered, state: runner.state), id: \.source) {
                                candidate in
                                ToolAnswerRow(
                                    copy: copy.folders,
                                    choose: copy.frame.choose,
                                    candidate: candidate,
                                    meta: ToolsScreenLayout.meta(
                                        for: candidate, in: runner.state, discovered: discovered,
                                        copy: copy, now: Date()),
                                    state: $runner.state,
                                    installURL: installURL(candidate.source)
                                )
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
                                }
                            }
                            if let pending {
                                let question = ToolsScreenLayout.question(path: pending.path, copy: copy.tools)
                                folderCard(name: ToolsScreenLayout.folderName(pending.path), path: pending.path, meta: nil) {
                                    GlassPicker(
                                        question,
                                        selection: pendingChoice,
                                        options: ToolsScreenLayout.pendingOptions(pending.kinds, copy: copy.tools),
                                        placeholder: copy.frame.choose
                                    )
                                }
                            }
                            addTile
                        }
                    }
                    .disabled(!FoldersScreenLayout.rowsEnabled(isCommitting: runner.isCommitting))
                case .failed:
                    HStack(spacing: GlassTokens.Space.s4) {
                        Text(discovery.failureLine(copy.folders) ?? "")
                            .glassType(GlassTokens.TypeScale.body)
                            .foregroundStyle(GlassColor.textSecondary)
                            .fixedSize(horizontal: false, vertical: true)
                        Button(copy.folders.retry) { refreshDiscovery() }
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
        .task {
            if discovery == .loading { refreshDiscovery() }
        }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            refreshDiscovery()
        }
    }

    /// Ron's add tile: click to pick a folder, or drop one on it.
    private var addTile: some View {
        Button {
            if let path = FolderPanel.choose() { describe(path) }
        } label: {
            GlassCard {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                    HStack(spacing: GlassTokens.Space.s6) {
                        GlassToolTile(.folder, large: true)
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
                    if refused {
                        Text(ToolsScreenLayout.refusal(copy.tools))
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textSecondary)
                    }
                }
            }
            .opacity(dragging ? 0.7 : 1)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onDrop(of: [UTType.fileURL], isTargeted: $dragging) { providers in
            guard let provider = providers.first else { return false }
            _ = provider.loadObject(ofClass: URL.self) { url, _ in
                guard let url else { return }
                let isFolder = (try? url.resourceValues(forKeys: [.isDirectoryKey]))?.isDirectory == true
                let path = url.path
                Task { @MainActor in
                    guard accepting else { return }
                    if isFolder { describe(path) } else { refused = true }
                }
            }
            return true
        }
    }

    private func folderCard<Trailing: View>(
        name: String, path: String, meta: String?, @ViewBuilder trailing: () -> Trailing
    ) -> some View {
        GlassCard {
            HStack(spacing: GlassTokens.Space.s6) {
                GlassToolTile(.folder, large: true)
                VStack(alignment: .leading, spacing: 0) {
                    Text(name)
                        .glassType(GlassTokens.TypeScale.bodyStrong)
                        .foregroundStyle(GlassColor.textPrimary)
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
    }

    /// Describe a picked folder and act on what the core reports. The core
    /// walks the folder off the main actor; its answer lands only if the
    /// screen is still Tools and nothing is committing.
    private func describe(_ path: String) {
        guard accepting else { return }
        Task.detached(priority: .userInitiated) {
            let json = TCDiscovery.describeFolderJSON(path)
            await MainActor.run { land(ToolsScreenLayout.outcome(path: path, json: json)) }
        }
    }

    @MainActor
    private func land(_ outcome: AddToolOutcome) {
        guard accepting else { return }
        refused = !ToolsScreenLayout.apply(outcome, to: &runner.state)
        if case .ask(let path, let kinds) = outcome {
            pending = (path, kinds)
        } else {
            pending = nil
        }
    }

    private var accepting: Bool {
        ToolsScreenLayout.acceptsFolder(step: runner.state.step, isCommitting: runner.isCommitting)
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
                guard let answer, let pending,
                    ToolsScreenLayout.answer(answer, path: pending.path, kinds: pending.kinds, in: &runner.state)
                else { return }
                self.pending = nil
            }
        )
    }

    private func refreshDiscovery() {
        discovery = FoldersScreenLayout.discovered(TCDiscovery.sourcesJSON(), keeping: discovery)
    }

    private var canContinue: Bool {
        ToolsScreenLayout.canContinue(
            discovered: discovery.rows, state: runner.state, pending: pending != nil,
            isCommitting: runner.isCommitting)
    }

    private var title: some View {
        FirstRunTitle(light: copy.tools.titleLight, bold: copy.tools.titleBold)
    }
}
