import Foundation
import Observation
import TCShellCore

/// A confirmed daemon snapshot. Reads and follow writes share one serial lane,
/// so an older read cannot overwrite a newer follow confirmation.
@Observable @MainActor
final class InsightsSkillsModel {
    typealias Reader = @Sendable (Bool) async throws -> DaemonData.SkillsSnapshot
    typealias Writer = @Sendable (String, Bool) async throws -> DaemonData.SkillsSnapshot
    private let reader: Reader?
    private let writer: Writer?
    private let fallbackCopy: [String: String]
    private let idlePollInterval: Duration
    private(set) var shown: DaemonData.SkillsSnapshot?
    private(set) var busy = false
    private(set) var failureKey: String?
    var canFollow: Bool { writer != nil && !busy }

    init(reader: Reader? = nil, writer: Writer? = nil, copy: [String: String] = [:],
         idlePollInterval: Duration = .seconds(5 * 60)) {
        self.reader = reader; self.writer = writer; self.fallbackCopy = copy
        self.idlePollInterval = idlePollInterval
    }

    func text(_ key: String, values: [String: String] = [:]) -> String {
        var result = shown?.copy[key] ?? fallbackCopy[key] ?? ""
        for (key, value) in values { result = result.replacingOccurrences(of: "{\(key)}", with: value) }
        return result
    }

    func timestamp(_ value: String) -> String {
        let date = (try? Date(value, strategy: .iso8601))
            ?? (try? Date(value, strategy: Date.ISO8601FormatStyle(includingFractionalSeconds: true)))
        return date?.formatted(date: .abbreviated, time: .shortened) ?? value
    }

    func inventorySkills(in snapshot: DaemonData.SkillsSnapshot, matching query: String) -> [DaemonData.InstalledSkill] {
        let query = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty else { return snapshot.inventory.skills }
        return snapshot.inventory.skills.filter { skill in
            [skill.name, skill.description, agentLabel(skill.agent), scopeLabel(skill.scope), skill.location]
                .contains { $0.localizedStandardContains(query) }
        }
    }

    func catalogSkills(in snapshot: DaemonData.SkillsSnapshot, matching query: String) -> [DaemonData.CatalogSkill] {
        let query = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty else { return snapshot.catalog.skills }
        let publishers = Dictionary(snapshot.catalog.publishers.map { ($0.id, $0.name) }, uniquingKeysWith: { first, _ in first })
        return snapshot.catalog.skills.filter { skill in
            [skill.name, skill.description, skill.repository, publishers[skill.publisherID] ?? ""]
                .contains { $0.localizedStandardContains(query) }
        }
    }

    func agentLabel(_ agent: String) -> String {
        let label = text("skills_agent_\(agent)")
        return label.isEmpty ? agent : label
    }

    func scopeLabel(_ scope: String) -> String {
        let label = text("skills_scope_\(scope)")
        return label.isEmpty ? scope : label
    }

    func usageSummary(_ row: DaemonData.SkillsUsageRow, coverage: [DaemonData.SkillsUsageCoverage]) -> String {
        let observation = coverage.first { $0.agent == row.agent }?.status
        var figures: [String] = []
        if observation == "explicit_invocations" || row.invocations > 0 {
            figures.append(row.invocations > 0
                ? text("skills_invocations", values: ["count": String(row.invocations)])
                : text("skills_no_invocations"))
        }
        if observation == "loads_only" || row.loads > 0 {
            figures.append(text("skills_loads", values: ["count": String(row.loads)]))
        }
        figures.append(text("skills_sessions", values: ["count": String(row.sessions)]))
        return figures.filter { !$0.isEmpty }.joined(separator: " · ")
    }

    func load(refresh: Bool = false) async {
        guard !busy, !Task.isCancelled else { return }
        guard let reader else { failureKey = "skills_unavailable"; return }
        busy = true
        defer { busy = false }
        do {
            let snapshot = try await reader(refresh)
            guard !Task.isCancelled else { return }
            if snapshot.readable {
                shown = snapshot
                failureKey = snapshot.error == nil ? nil : "skills_refresh_failed"
            } else {
                shown = nil
                failureKey = "skills_unavailable"
            }
        } catch {
            guard !Task.isCancelled else { return }
            failureKey = shown == nil ? "skills_unavailable" : "skills_refresh_failed"
        }
    }

    func setFollow(publisherID: String, followed: Bool) async {
        guard !busy, let writer, !Task.isCancelled else { return }
        busy = true
        defer { busy = false }
        do {
            let snapshot = try await writer(publisherID, followed)
            guard !Task.isCancelled else { return }
            guard snapshot.readable else { shown = nil; failureKey = "skills_follow_failed"; return }
            shown = snapshot
            failureKey = snapshot.error == nil ? nil : "skills_follow_failed"
        } catch {
            guard !Task.isCancelled else { return }
            failureKey = "skills_follow_failed"
        }
    }

    /// Automatic reads let the backend refresh a due catalog under its
    /// six-hour throttle. Only the explicit Refresh action forces a refresh.
    func observe() async {
        await readWhenReady()
        while !Task.isCancelled, reader != nil {
            let refreshing = shown?.refreshing == true
            do { try await Task.sleep(for: refreshing ? .seconds(2) : idlePollInterval) }
            catch { return }
            await readWhenReady()
        }
    }

    /// An inactive view may leave a noncancellable IPC read finishing.
    /// Reactivation waits for that lane instead of sleeping a whole poll
    /// interval after a busy read was skipped.
    private func readWhenReady() async {
        while busy, !Task.isCancelled {
            do { try await Task.sleep(for: .milliseconds(10)) }
            catch { return }
        }
        guard !Task.isCancelled else { return }
        await load(refresh: false)
    }

    /// A shared display name never establishes installed source identity.
    func catalogMatch(for installed: DaemonData.InstalledSkill,
                      in snapshot: DaemonData.SkillsSnapshot) -> DaemonData.CatalogSkill? {
        guard let source = installed.source else { return nil }
        return snapshot.catalog.skills.first {
            $0.repository == source.repository && $0.path == source.path
        }
    }
}
