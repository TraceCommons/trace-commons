import SwiftUI
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
enum FirstRunRulesLayout {
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

    /// Set a folder's rule. Never clears the folder's selection: a Never
    /// folder contributes nothing, its past sessions included.
    static func setRule(_ state: inout FirstRunState, projectID: String, mode: ProjectMode) {
        state.rules[projectID] = mode
        if mode == .ignore {
            state.pastSelections[projectID] = nil
        }
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

    /// Fill a core sentence's `{name}` placeholders.
    static func fill(_ template: String, _ values: [String: String]) -> String {
        values.reduce(template) { text, pair in
            text.replacingOccurrences(of: "{" + pair.key + "}", with: pair.value)
        }
    }
}

/// Ron's Rules screen (#1030 `rules-screen.tsx`, Custom setup's W-5) in
/// glass: a rule per folder found in the watched tools' sessions, then the
/// past sessions of each folder to include as the person's own approval.
struct RulesScreen: View {
    private static let collapsedSessions = 2

    let copy: FirstRunCopy
    @Binding var state: FirstRunState
    let source: any FirstRunRulesSource
    var notice: String?
    var onBack: (() -> Void)?
    let onContinue: () -> Void

    @State private var projects: [ProjectRow]?
    @State private var sessions: [String: [PastSession]] = [:]
    @State private var totals: [String: Int] = [:]
    @State private var open: Set<String> = []
    @State private var showingAll: Set<String> = []

    private var modeCopy: ContributionModeCopy? { ProjectModeWords.table }

    var body: some View {
        FirstRunFrame(
            copy: copy,
            state: $state,
            onBack: onBack,
            notice: notice,
            footer: FirstRunFooter(
                title: copy.frame.continueButton,
                isEnabled: projects != nil,
                action: onContinue)
        ) {
            ScrollView {
                VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                    Text("\(copy.rules.titleLight)\(Text(copy.rules.titleBold).bold())")
                        .glassType(GlassTokens.TypeScale.title)
                        .foregroundStyle(GlassColor.textPrimary)
                    content
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .task { await load() }
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
        } else {
            HStack(spacing: GlassTokens.Space.s4) {
                ProgressView().controlSize(.small)
                Text(copy.rules.loading)
                    .glassType(GlassTokens.TypeScale.body)
                    .foregroundStyle(GlassColor.textSecondary)
            }
        }
    }

    // MARK: Card 1: a rule per folder

    private func rulesCard(_ projects: [ProjectRow]) -> some View {
        GlassEyebrowCard(
            FirstRunRulesLayout.fill(copy.rules.reposFound, ["tools": FirstRunRulesLayout.toolNames(state)])
        ) {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(Array(projects.enumerated()), id: \.element.id) { index, project in
                    GlassTableRow(first: index == 0) { ruleRow(project) }
                }
            }
        }
    }

    private func ruleRow(_ project: ProjectRow) -> some View {
        let folder = FirstRunRulesLayout.folder(project)
        return HStack(alignment: .center, spacing: GlassTokens.Space.s4) {
            VStack(alignment: .leading, spacing: 0) {
                Text(folder)
                    .glassType(GlassTokens.TypeScale.mono)
                    .foregroundStyle(GlassColor.textPrimary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                if let count = sessionCount(project) {
                    Text(FirstRunRulesLayout.fill(copy.rules.sessionCount, ["count": String(count)]))
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textTertiary)
                }
            }
            Spacer(minLength: 0)
            if let modeCopy {
                GlassPicker(
                    FirstRunRulesLayout.fill(copy.rules.ruleFor, ["folder": folder]),
                    selection: Binding<ProjectMode?>(
                        get: { FirstRunRulesLayout.rule(state, for: project) },
                        set: { wanted in
                            guard let wanted else { return }
                            FirstRunRulesLayout.setRule(&state, projectID: project.projectId, mode: wanted)
                        }),
                    options: ProjectModeChoices.options(for: project.offerableModes, copy: modeCopy),
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
        let summary = FirstRunRulesLayout.summary(state, projects: projects, sessions: sessions)
        return GlassCard {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
                HStack {
                    Text(copy.rules.pastSessions)
                        .glassType(GlassTokens.TypeScale.bodyStrong)
                        .foregroundStyle(GlassColor.textPrimary)
                    Spacer(minLength: GlassTokens.Space.s4)
                    Text(
                        FirstRunRulesLayout.fill(
                            copy.rules.selectedSummary,
                            ["selected": String(summary.selected), "total": String(summary.total)])
                    )
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textTertiary)
                }
                ForEach(projects) { project in
                    folderSessions(project)
                }
            }
        }
    }

    @ViewBuilder private func folderSessions(_ project: ProjectRow) -> some View {
        let id = project.projectId
        let folder = FirstRunRulesLayout.folder(project)
        let rows = sessions[id] ?? []
        if FirstRunRulesLayout.rule(state, for: project) == .ignore {
            HStack(spacing: GlassTokens.Space.s4) {
                GlassCheckMark(checked: false)
                Text(folder)
                    .glassType(GlassTokens.TypeScale.mono)
                    .foregroundStyle(GlassColor.textTertiary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer(minLength: 0)
                Text(FirstRunRulesLayout.fill(copy.rules.neverCount, ["count": String(sessionCount(project) ?? rows.count)]))
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textTertiary)
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(FirstRunRulesLayout.fill(copy.rules.neverLabel, ["folder": folder]))
        } else {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                HStack(spacing: GlassTokens.Space.s4) {
                    includeEvery(project, rows: rows, folder: folder)
                    GlassExpander(folder, isOpen: openBinding(id))
                    Text(
                        FirstRunRulesLayout.fill(
                            copy.rules.folderSelected,
                            [
                                "selected": String(rows.filter { state.pastSelections[id]?.contains($0.id) == true }.count),
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
        let label = FirstRunRulesLayout.fill(copy.rules.includeEvery, ["folder": folder])
        let tickable = rows.filter(FirstRunRulesLayout.isTickable)
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
                    Text(FirstRunRulesLayout.labelParts(session).joined(separator: " · "))
                }
                .toggleStyle(GlassCheckboxStyle())
                .disabled(!FirstRunRulesLayout.isTickable(session))
            }
            if rows.count > Self.collapsedSessions {
                Button(all ? copy.rules.showFewer : FirstRunRulesLayout.fill(copy.rules.showAll, ["count": String(rows.count)])) {
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
            get: { state.pastSelections[projectID]?.contains(session.id) == true },
            set: { on in FirstRunRulesLayout.setTicked(&state, projectID: projectID, session: session, on: on) })
    }

    /// One row as a source of the folder's box: read per row, written for
    /// the whole folder.
    private func groupBinding(_ projectID: String, rows: [PastSession], _ session: PastSession) -> Binding<Bool> {
        Binding(
            get: { state.pastSelections[projectID]?.contains(session.id) == true },
            set: { on in FirstRunRulesLayout.includeEvery(&state, projectID: projectID, sessions: rows, on: on) })
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
    /// as Ron's does; a folder whose sessions are refused lists none.
    private func load() async {
        guard projects == nil, let loaded = await source.rulesProjects() else { return }
        if let first = loaded.first { open.insert(first.projectId) }
        projects = loaded
        for project in loaded {
            guard let list = await source.pastSessions(projectID: project.projectId) else { continue }
            sessions[project.projectId] = list.sessions
            totals[project.projectId] = list.total
        }
    }
}
