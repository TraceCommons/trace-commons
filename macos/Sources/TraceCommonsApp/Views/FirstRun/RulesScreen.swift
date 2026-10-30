import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// Where Customize on Tools reads the repos and their past sessions (Ron's
/// #1030 Rules screen, folded into Tools on 2026-10-21). Nil is a refusal or
/// no daemon, never an empty answer.
@MainActor
protocol FirstRunRulesSource: AnyObject {
    func rulesProjects() async -> [ProjectRow]?
    func pastSessions(projectID: String) async -> PastSessionList?
}

extension AppModel: FirstRunRulesSource {}

/// The per-repo rules' decisions, which Customize on Tools draws, apart from
/// the view so they can be tested
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

    /// The tools the person chose to watch, by the daemon's source name
    /// (`ProjectTool.source`, `PastSession.source`): each watched session
    /// store, and the exported-traces folder when one is declared.
    static func watchedSources(_ state: FirstRunState) -> Set<String> {
        let roots = state.sessionRoots
        var watched = Set(
            SourceKind.allCases.filter {
                if case .watch = roots[$0] { return true }
                return false
            }.map(\.rawValue))
        if case .watch = roots.trajectory { watched.insert(trajectorySource) }
        return watched
    }

    /// The daemon's source name for exported traces (`SOURCE_TRAJECTORY`).
    static let trajectorySource = "trajectory"

    /// Spec rule 9: only repos found in the sessions of a watched tool. A
    /// repo whose tools the daemon has not named is not known to be one, so
    /// it is left out rather than offered on a guess.
    static func offered(_ projects: [ProjectRow], state: FirstRunState) -> [ProjectRow] {
        let watched = watchedSources(state)
        return projects.filter { project in project.tools.contains { watched.contains($0.source) } }
    }

    /// A repo's session count over its watched tools only.
    static func watchedSessionCount(_ project: ProjectRow, state: FirstRunState) -> Int {
        let watched = watchedSources(state)
        return project.tools.filter { watched.contains($0.source) }.reduce(0) { $0 + $1.sessionCount }
    }

    /// A repo's past sessions from watched tools only.
    static func offered(_ sessions: [PastSession], state: FirstRunState) -> [PastSession] {
        let watched = watchedSources(state)
        return sessions.filter { watched.contains($0.source) }
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
        /// Not for this folder; nothing is written.
        case refused
    }

    /// The modes a folder's picker offers: every mode the daemon would set
    /// for it. Automatic is offered before sign-in, as on Uses (Ron,
    /// 2026-10-22): it is confirmed when picked and armed at Start, once the
    /// enrollment it needs is held (`FirstRunPlan`), and until then the row
    /// says it turns on once the person signs in (`armsAfterSignIn`).
    static func offeredModes(_ project: ProjectRow, state: FirstRunState) -> [ProjectMode] {
        project.offerableModes
    }

    /// A pick on a folder's picker. The daemon's own Never or Automatic
    /// clears the folder's answer, so it is not sent again. Ask is kept
    /// even when the daemon already reads Ask: an Ask the person picked
    /// stays an exception to "Automatic, including existing history" chosen
    /// later on Uses (owner decision 2026-10-29, #1747), and `FirstRunPlan`
    /// sends it after the grant. Automatic is a grant, so it is never
    /// silent: it waits for the arming confirmation.
    static func pick(_ state: inout FirstRunState, project: ProjectRow, wanted: ProjectMode) -> PickOutcome {
        let id = project.projectId
        if wanted == project.mode, wanted != .ask {
            state.rules[id] = nil
            if wanted == .ignore { state.pastSelections[id] = nil }
            return .applied
        }
        guard project.offerableModes.contains(wanted) else { return .refused }
        if wanted == .autoUpload { return .needsConfirmation }
        setRule(&state, projectID: id, mode: wanted)
        return .applied
    }

    /// The arming confirmation was accepted: the only writer of Automatic.
    /// Refused for a folder the daemon will not arm, whatever the view
    /// asked. Without an enrollment it is recorded, not sent: Start arms it
    /// after sign-in.
    static func confirmArming(_ state: inout FirstRunState, project: ProjectRow) -> Bool {
        guard project.offerableModes.contains(.autoUpload) else { return false }
        state.rules[project.projectId] = .autoUpload
        return true
    }

    /// A folder set to Automatic in this first run that cannot be armed yet,
    /// for want of an enrollment: its row says it turns on once the person
    /// signs in, as Uses' Automatic does.
    static func armsAfterSignIn(_ state: FirstRunState, project: ProjectRow) -> Bool {
        state.rules[project.projectId] == .autoUpload && !FirstRunNavigation.canChooseAutomatic(state)
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

    /// A folder's older sessions beyond the newest the picker lists, in the
    /// core's words; nil when there are none.
    static func notListedNote(_ count: Int, copy: FirstRunCopy.Rules) -> String? {
        count > 0 ? FirstRunCopy.fill(copy.notListed, ["count": String(count)]) : nil
    }

    /// The past-session card's note. Watching only has no enrollment, so the
    /// sessions picked here are queued on this Mac and none is sent; the
    /// core's line says they wait there. Nil for every other account, and
    /// for watching only while the daemon still holds an enrollment, when
    /// Start sends nothing (`FirstRunPlan`) and nothing waits.
    static func pastSessionsNote(_ state: FirstRunState, copy: FirstRunCopy.Rules) -> String? {
        state.account == .watchOnly && !state.daemonHoldsEnrolment ? copy.pastSessionsWatchOnly : nil
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
    /// The date and duration are Ron's formats in the core's words.
    static func labelParts(_ session: PastSession, copy: FirstRunCopy.Rules) -> [String] {
        var parts: [String] = []
        if let started = session.startedAt {
            parts.append(dateText(started, copy: copy))
        }
        if let title = session.title, !title.isEmpty {
            parts.append(title)
        }
        if let seconds = session.durationSecs {
            parts.append(durationText(seconds, copy: copy))
        }
        if session.title == nil, session.durationSecs == nil {
            parts.append(sizeText(session.sizeBytes))
        }
        return parts
    }

    /// "Sat 12 Sep" (Ron's `formatSessionDate`), read in UTC so a session
    /// never moves to another day with the time zone. Every word is the
    /// core's; only the calendar arithmetic is here.
    static func dateText(_ date: Date, copy: FirstRunCopy.Rules) -> String {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC") ?? TimeZone(secondsFromGMT: 0)!
        let parts = calendar.dateComponents([.weekday, .day, .month], from: date)
        guard let weekday = parts.weekday, let day = parts.day, let month = parts.month,
            copy.weekdays.indices.contains(weekday - 1), copy.months.indices.contains(month - 1)
        else { return "" }
        return FirstRunCopy.fill(
            copy.sessionDate,
            ["weekday": copy.weekdays[weekday - 1], "day": String(day), "month": copy.months[month - 1]])
    }

    /// "52 min", "1 h 18 min", "2 h 04 min" (Ron's `formatDuration`): whole
    /// minutes, rounded; the minutes are padded to two digits from two hours
    /// up, as his are.
    static func durationText(_ seconds: Int, copy: FirstRunCopy.Rules) -> String {
        let total = max(0, Int((Double(seconds) / 60).rounded()))
        guard total >= 60 else {
            return FirstRunCopy.fill(copy.durationMinutes, ["minutes": String(total)])
        }
        let hours = total / 60
        let rest = total % 60
        let minutes = hours >= 2 && rest < 10 ? "0\(rest)" : String(rest)
        return FirstRunCopy.fill(copy.durationHours, ["hours": String(hours), "minutes": minutes])
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

    /// The retry beside `rules.unavailable`: Rules' own word, `rules.retry`
    /// (owner ruling, 2026-10-06), not the one Folders and Tools offer when
    /// discovery fails.
    static func retryTitle(_ copy: FirstRunCopy) -> String {
        copy.rules.retry
    }

    /// What the screen draws: the folders once read; while a read runs,
    /// the loading line, even when the last read failed, so a retry never
    /// leaves the old failure and a live retry up beside it (Kristi's review
    /// of #1261); the failure only once no read is running.
    static func phase(projects: [ProjectRow]?, loadFailed: Bool, loading: Bool) -> RulesLoadPhase {
        if let projects { return .loaded(projects) }
        return loadFailed && !loading ? .failed : .loading
    }

    /// The folder as a row names it: its path, or its label without one.
    static func folder(_ project: ProjectRow) -> String {
        project.projectPath.isEmpty ? project.displayLabel : project.projectPath
    }
}

/// What Customize draws in place of its repos (`RulesScreenLayout.phase`).
enum RulesLoadPhase: Equatable {
    case loading
    case failed
    case loaded([ProjectRow])
}
