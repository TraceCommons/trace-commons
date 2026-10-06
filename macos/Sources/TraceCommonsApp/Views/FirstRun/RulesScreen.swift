import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// Where the Rules screen reads its folders and their past sessions. Nil is
/// a refusal or no daemon, never an empty answer.
@MainActor
protocol FirstRunRulesSource: AnyObject {
    func rulesProjects() async -> [ProjectRow]?
    func pastSessions(projectID: String) async -> PastSessionList?
}

extension AppModel: FirstRunRulesSource {}

/// The Rules screen's decisions, apart from the view so they can be tested
/// (#1030 `ftux-model.ts`: `groupState`, `toggleGroup`, `applyRule`,
/// `pastSessionSummary`).
///
/// A selection is the person's own approval: nothing here ticks a session
/// the person did not tick, by the folder box or one by one.
enum RulesScreenLayout {
    enum GroupState: Equatable {
        case none, some, all
    }

    /// Whether the picker may tick a row: the daemon must allow it, and a
    /// session still being written never can be, whatever the wire said.
    static func isTickable(_ session: PastSession) -> Bool {
        session.selectable && session.state != .stillActive
    }

    /// The folder's rule as shown: the person's answer, else the daemon's
    /// mode. Only a change is written to the state, so an untouched rule is
    /// not sent again.
    static func rule(_ state: FirstRunState, for project: ProjectRow) -> ProjectMode {
        state.rules[project.projectId] ?? project.mode
    }

    /// What a pick on a folder's picker did.
    enum PickOutcome: Equatable {
        /// Written, or cleared because it is the daemon's own mode.
        case applied
        /// Automatic: nothing is written until the core's arming
        /// confirmation is accepted (`confirmArming`).
        case needsConfirmation
        /// Not for this person or this folder; nothing is written.
        case refused
    }

    /// The modes a folder's picker offers. Automatic needs an account, so a
    /// watch-only person is not offered it; a folder the daemon already
    /// arms keeps it, so the picker can show what is in force, and picking
    /// it again is not a change.
    static func offeredModes(_ project: ProjectRow, account: AccountAnswer) -> [ProjectMode] {
        let modes = project.offerableModes
        guard !FirstRunNavigation.canChooseAutomatic(account), project.mode != .autoUpload else { return modes }
        return modes.filter { $0 != .autoUpload }
    }

    /// A pick on a folder's picker. The daemon's own mode clears the
    /// folder's answer, so it is not sent again. Automatic is a grant, so
    /// it is never silent: it waits for the arming confirmation, and a
    /// watch-only person is refused it.
    static func pick(_ state: inout FirstRunState, project: ProjectRow, wanted: ProjectMode) -> PickOutcome {
        let id = project.projectId
        if wanted == project.mode {
            state.rules[id] = nil
            if wanted == .ignore { state.pastSelections[id] = nil }
            return .applied
        }
        guard project.offerableModes.contains(wanted) else { return .refused }
        if wanted == .autoUpload {
            return FirstRunNavigation.canChooseAutomatic(state.account) ? .needsConfirmation : .refused
        }
        setRule(&state, projectID: id, mode: wanted)
        return .applied
    }

    /// The arming confirmation was accepted: the only writer of Automatic.
    /// Refused for a watch-only person and for a folder the daemon will
    /// not arm, whatever the view asked.
    static func confirmArming(_ state: inout FirstRunState, project: ProjectRow) -> Bool {
        guard FirstRunNavigation.canChooseAutomatic(state.account),
            project.offerableModes.contains(.autoUpload)
        else { return false }
        state.rules[project.projectId] = .autoUpload
        return true
    }

    /// Set a folder's rule other than Automatic, which only
    /// `confirmArming` writes. Never clears the folder's selection: a Never
    /// folder contributes nothing, its past sessions included.
    static func setRule(_ state: inout FirstRunState, projectID: String, mode: ProjectMode) {
        guard mode != .autoUpload else { return }
        state.rules[projectID] = mode
        if mode == .ignore {
            state.pastSelections[projectID] = nil
        }
    }

    /// The past-session card's note. Watching only has no enrolment, so the
    /// sessions picked here are queued on this Mac and none is sent; the
    /// core's line says they wait there. Nil for every other account.
    static func pastSessionsNote(_ state: FirstRunState, copy: FirstRunCopy.Rules) -> String? {
        state.account == .watchOnly ? copy.pastSessionsWatchOnly : nil
    }

    static func groupState(_ state: FirstRunState, projectID: String, sessions: [PastSession]) -> GroupState {
        let tickable = sessions.filter(isTickable).map(\.id)
        let selected = state.pastSelections[projectID] ?? []
        let on = tickable.filter(selected.contains).count
        if !tickable.isEmpty, on == tickable.count { return .all }
        return on == 0 ? .none : .some
    }

    /// "Include every past session in {folder}": on selects every tickable
    /// session, off clears the folder. A value, not a flip, so the box's
    /// write lands the same however many of its rows it reaches; which value
    /// a press writes is the native group toggle's (on from mixed, as Ron's
    /// `toggleGroup`).
    static func includeEvery(_ state: inout FirstRunState, projectID: String, sessions: [PastSession], on: Bool) {
        guard state.rules[projectID] != .ignore else { return }
        let tickable = Set(sessions.filter(isTickable).map(\.id))
        state.pastSelections[projectID] = on && !tickable.isEmpty ? tickable : nil
    }

    /// Tick or untick one row. Sets, never toggles, so a group write that
    /// reaches every row lands every row on the same value.
    static func setTicked(_ state: inout FirstRunState, projectID: String, session: PastSession, on: Bool) {
        guard state.rules[projectID] != .ignore, isTickable(session) else { return }
        var selected = state.pastSelections[projectID] ?? []
        if on {
            selected.insert(session.id)
        } else {
            selected.remove(session.id)
        }
        state.pastSelections[projectID] = selected.isEmpty ? nil : selected
    }

    /// "{selected} of {total} selected" over the folders that are not
    /// Never. The total is the rows the picker lists, what a person could
    /// select, not the folder's whole count.
    static func summary(
        _ state: FirstRunState,
        projects: [ProjectRow],
        sessions: [String: [PastSession]]
    ) -> (selected: Int, total: Int) {
        var selected = 0
        var total = 0
        for project in projects where rule(state, for: project) != .ignore {
            let rows = sessions[project.projectId] ?? []
            total += rows.count
            let chosen = state.pastSelections[project.projectId] ?? []
            selected += rows.filter { chosen.contains($0.id) }.count
        }
        return (selected, total)
    }

    /// A row's words: date, then title and duration when the session was
    /// opened; a `not_queued` row has neither and shows its size instead.
    /// Every part is a system formatter's, in the person's locale.
    static func labelParts(_ session: PastSession) -> [String] {
        var parts: [String] = []
        if let started = session.startedAt {
            parts.append(dateText(started))
        }
        if let title = session.title, !title.isEmpty {
            parts.append(title)
        }
        if let seconds = session.durationSecs {
            parts.append(durationText(seconds))
        }
        if session.title == nil, session.durationSecs == nil {
            parts.append(sizeText(session.sizeBytes))
        }
        return parts
    }

    static func dateText(_ date: Date) -> String {
        date.formatted(.dateTime.weekday(.abbreviated).day().month(.abbreviated))
    }

    static func durationText(_ seconds: Int) -> String {
        let formatter = DateComponentsFormatter()
        formatter.allowedUnits = seconds >= 3_600 ? [.hour, .minute] : [.minute]
        formatter.unitsStyle = .abbreviated
        return formatter.string(from: TimeInterval(max(seconds, 0))) ?? ""
    }

    static func sizeText(_ bytes: Int) -> String {
        ByteCountFormatter.string(fromByteCount: Int64(bytes), countStyle: .file)
    }

    /// The watched tools' names for "{tools}", joined the person's way.
    static func toolNames(_ state: FirstRunState) -> String {
        let roots = state.sessionRoots
        let watched = SourceKind.allCases.filter {
            if case .watch = roots[$0] { return true }
            return false
        }
        return ListFormatter.localizedString(byJoining: watched.map(\.displayName))
    }

    /// The folder as a row names it: its path, or its label without one.
    static func folder(_ project: ProjectRow) -> String {
        project.projectPath.isEmpty ? project.displayLabel : project.projectPath
    }
}

/// Ron's Rules screen (#1030 `rules-screen.tsx`, Custom setup's W-5) in
/// glass: a rule per folder found in the watched tools' sessions, then the
/// past sessions of each folder to include as the person's own approval.
struct RulesScreen: View {
    private static let collapsedSessions = 2

    let copy: FirstRunCopy
    @ObservedObject var runner: FirstRunRunner
    /// Where the folders and their past sessions are read (the app model).
    let source: any FirstRunRulesSource

    @State private var projects: [ProjectRow]?
    @State private var loadFailed = false
    @State private var sessions: [String: [PastSession]] = [:]
    /// Folders whose past sessions were refused: drawn as unavailable, not
    /// as an empty list beside card 1's count.
    @State private var refused: Set<String> = []
    @State private var armingCandidate: ProjectRow?
    @State private var totals: [String: Int] = [:]
    @State private var open: Set<String> = []
    @State private var showingAll: Set<String> = []

    private var modeCopy: ContributionModeCopy? { ProjectModeWords.table }

    var body: some View {
        FirstRunFrame(
            copy: copy,
            state: $runner.state,
            onBack: { runner.state = FirstRunNavigation.back(runner.state) },
            footer: FirstRunFooter(
                title: copy.frame.continueButton,
                isEnabled: projects != nil,
                action: { runner.state = FirstRunNavigation.next(runner.state) })
        ) {
            ScrollView {
                VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                    FirstRunTitle(light: copy.rules.titleLight, bold: copy.rules.titleBold)
                    content
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .task { await load() }
        // One dialog for the list, named by whichever folder is being armed.
        // Not presented without the core's words: arming is never confirmed
        // against a sentence this shell wrote.
        .glassModal(isPresented: Binding(
            get: { armingCandidate.flatMap(armingCopy) != nil },
            set: { if !$0 { armingCandidate = nil } })
        ) {
            if let project = armingCandidate, let words = armingCopy(project) {
                GlassConfirmation(
                    title: words.question, message: words.body,
                    actions: [
                        .cancel(words.decline) { armingCandidate = nil },
                        GlassModalAction(words.confirm, isDefault: true) {
                            _ = RulesScreenLayout.confirmArming(&runner.state, project: project)
                            armingCandidate = nil
                        },
                    ],
                    onCancel: { armingCandidate = nil })
            }
        }
    }

    /// The arming confirmation's words, from the core, as Settings asks
    /// them. No count is in hand here, so the evidence line is not drawn.
    private func armingCopy(_ project: ProjectRow) -> ProjectArmingCopy? {
        ProjectArmingCopy.decode(
            fromJSON: TCCoreCopy.armingOfferCopyJSON(
                project: project.displayLabel,
                count: 0))
    }

    @ViewBuilder private var content: some View {
        if let projects {
            if projects.isEmpty {
                GlassCard(quiet: true) {
                    Text(copy.rules.empty)
                        .glassType(GlassTokens.TypeScale.body)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            } else {
                rulesCard(projects)
                pastSessionsCard(projects)
            }
        } else if loadFailed {
            GlassNotice(tone: .outside) {
                Text(copy.rules.unavailable)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        } else {
            HStack(spacing: GlassTokens.Space.s4) {
                GlassSpinner()
                Text(copy.rules.loading)
                    .glassType(GlassTokens.TypeScale.body)
                    .foregroundStyle(GlassColor.textSecondary)
            }
        }
    }

    // MARK: Card 1: a rule per folder

    private func rulesCard(_ projects: [ProjectRow]) -> some View {
        GlassEyebrowCard(
            FirstRunCopy.fill(copy.rules.reposFound, ["tools": RulesScreenLayout.toolNames(runner.state)])
        ) {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(Array(projects.enumerated()), id: \.element.id) { index, project in
                    GlassTableRow(first: index == 0) { ruleRow(project) }
                }
            }
        }
    }

    private func ruleRow(_ project: ProjectRow) -> some View {
        let folder = RulesScreenLayout.folder(project)
        return HStack(alignment: .center, spacing: GlassTokens.Space.s4) {
            VStack(alignment: .leading, spacing: 0) {
                Text(folder)
                    .glassType(GlassTokens.TypeScale.mono)
                    .foregroundStyle(GlassColor.textPrimary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                if let count = sessionCount(project) {
                    Text(FirstRunCopy.fill(copy.frame.sessionCount, ["count": String(count)]))
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textTertiary)
                }
            }
            Spacer(minLength: 0)
            if let modeCopy {
                GlassPicker(
                    FirstRunCopy.fill(copy.rules.ruleFor, ["folder": folder]),
                    selection: Binding<ProjectMode?>(
                        get: { RulesScreenLayout.rule(runner.state, for: project) },
                        set: { wanted in
                            guard let wanted else { return }
                            // Arming is a grant, so it is never silent.
                            if RulesScreenLayout.pick(&runner.state, project: project, wanted: wanted) == .needsConfirmation {
                                armingCandidate = project
                            }
                        }),
                    options: ProjectModeChoices.options(
                        for: RulesScreenLayout.offeredModes(project, account: runner.state.account),
                        copy: modeCopy),
                    placeholder: modeCopy.title)
            }
        }
    }

    /// A folder's session count as card 1 and a Never folder show it: the
    /// daemon's total for the folder, which can exceed the rows it lists.
    /// The selection counts ("{selected} of {total}") are over listed rows,
    /// what a person could tick.
    private func sessionCount(_ project: ProjectRow) -> Int? {
        totals[project.projectId] ?? project.sessionCount
    }

    // MARK: Card 2: past sessions, by folder

    private func pastSessionsCard(_ projects: [ProjectRow]) -> some View {
        let summary = RulesScreenLayout.summary(runner.state, projects: projects, sessions: sessions)
        return GlassCard {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
                HStack {
                    Text(copy.rules.pastSessions)
                        .glassType(GlassTokens.TypeScale.bodyStrong)
                        .foregroundStyle(GlassColor.textPrimary)
                    Spacer(minLength: GlassTokens.Space.s4)
                    Text(
                        FirstRunCopy.fill(
                            copy.rules.selectedSummary,
                            ["selected": String(summary.selected), "total": String(summary.total)])
                    )
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textTertiary)
                }
                if let note = RulesScreenLayout.pastSessionsNote(runner.state, copy: copy.rules) {
                    Text(note)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                ForEach(projects) { project in
                    folderSessions(project)
                }
            }
        }
    }

    @ViewBuilder private func folderSessions(_ project: ProjectRow) -> some View {
        let id = project.projectId
        let folder = RulesScreenLayout.folder(project)
        let rows = sessions[id] ?? []
        if RulesScreenLayout.rule(runner.state, for: project) == .ignore {
            HStack(spacing: GlassTokens.Space.s4) {
                GlassCheckMark(checked: false)
                Text(folder)
                    .glassType(GlassTokens.TypeScale.mono)
                    .foregroundStyle(GlassColor.textTertiary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer(minLength: 0)
                Text(FirstRunCopy.fill(copy.rules.neverCount, ["count": String(sessionCount(project) ?? rows.count)]))
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textTertiary)
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(FirstRunCopy.fill(copy.rules.neverLabel, ["folder": folder]))
        } else if refused.contains(id) {
            HStack(spacing: GlassTokens.Space.s4) {
                Text(folder)
                    .glassType(GlassTokens.TypeScale.mono)
                    .foregroundStyle(GlassColor.textTertiary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer(minLength: 0)
                Text(copy.rules.sessionsUnavailable)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textTertiary)
            }
        } else {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                HStack(spacing: GlassTokens.Space.s4) {
                    includeEvery(project, rows: rows, folder: folder)
                    GlassExpander(folder, isOpen: openBinding(id))
                    Text(
                        FirstRunCopy.fill(
                            copy.rules.folderSelected,
                            [
                                "selected": String(rows.filter { runner.state.pastSelections[id]?.contains($0.id) == true }.count),
                                "total": String(rows.count),
                            ])
                    )
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textTertiary)
                }
                if open.contains(id) {
                    sessionList(project, rows: rows)
                        .padding(.leading, GlassTokens.Space.s8)
                }
            }
        }
    }

    /// The folder's mixed-state box. Each tickable row is one source, so the
    /// style reads mixed from the rows; every source writes the whole folder
    /// through `includeEvery`. The folder is named once, by the expander
    /// beside the box, so the box's sentence is its accessibility label and
    /// not drawn. With nothing to tick the box is off and disabled.
    @ViewBuilder private func includeEvery(_ project: ProjectRow, rows: [PastSession], folder: String) -> some View {
        let label = FirstRunCopy.fill(copy.rules.includeEvery, ["folder": folder])
        let tickable = rows.filter(RulesScreenLayout.isTickable)
        if tickable.isEmpty {
            Toggle(isOn: .constant(false)) { EmptyView() }
                .toggleStyle(GlassCheckboxStyle())
                .disabled(true)
                .accessibilityLabel(label)
        } else {
            Toggle(sources: tickable.map { groupBinding(project.projectId, rows: rows, $0) }, isOn: \.self) {
                EmptyView()
            }
            .toggleStyle(GlassCheckboxStyle())
            .accessibilityLabel(label)
        }
    }

    private func sessionList(_ project: ProjectRow, rows: [PastSession]) -> some View {
        let id = project.projectId
        let all = showingAll.contains(id)
        let visible = all ? rows : Array(rows.prefix(Self.collapsedSessions))
        return VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            ForEach(visible) { session in
                Toggle(isOn: tickBinding(id, session)) {
                    Text(RulesScreenLayout.labelParts(session).joined(separator: " · "))
                }
                .toggleStyle(GlassCheckboxStyle())
                .disabled(!RulesScreenLayout.isTickable(session))
            }
            if rows.count > Self.collapsedSessions {
                Button(all ? copy.rules.showFewer : FirstRunCopy.fill(copy.rules.showAll, ["count": String(rows.count)])) {
                    withAnimation(GlassMotion.fast(GlassMotion.systemReducesMotion)) {
                        if all { showingAll.remove(id) } else { showingAll.insert(id) }
                    }
                }
                .buttonStyle(GlassButtonStyle(.link))
            }
        }
    }

    private func tickBinding(_ projectID: String, _ session: PastSession) -> Binding<Bool> {
        Binding(
            get: { runner.state.pastSelections[projectID]?.contains(session.id) == true },
            set: { on in RulesScreenLayout.setTicked(&runner.state, projectID: projectID, session: session, on: on) })
    }

    /// One row as a source of the folder's box: read per row, written for
    /// the whole folder.
    private func groupBinding(_ projectID: String, rows: [PastSession], _ session: PastSession) -> Binding<Bool> {
        Binding(
            get: { runner.state.pastSelections[projectID]?.contains(session.id) == true },
            set: { on in RulesScreenLayout.includeEvery(&runner.state, projectID: projectID, sessions: rows, on: on) })
    }

    private func openBinding(_ id: String) -> Binding<Bool> {
        Binding(
            get: { open.contains(id) },
            set: { isOpen in
                if isOpen { open.insert(id) } else { open.remove(id) }
            })
    }

    // MARK: Loading

    /// The folders, then each folder's past sessions. The first folder opens,
    /// as Ron's does. Folders that cannot be read say so, and Continue stays
    /// disabled; a folder whose sessions are refused is drawn as unavailable
    /// and lists nothing to tick.
    private func load() async {
        guard projects == nil else { return }
        guard let loaded = await source.rulesProjects() else {
            loadFailed = true
            return
        }
        loadFailed = false
        if let first = loaded.first { open.insert(first.projectId) }
        projects = loaded
        for project in loaded {
            guard let list = await source.pastSessions(projectID: project.projectId) else {
                refused.insert(project.projectId)
                continue
            }
            sessions[project.projectId] = list.sessions
            totals[project.projectId] = list.total
        }
    }
}
