import Foundation
import Observation
import TCShellCore

/// A confirmed daemon snapshot. Reads and follow writes share one serial lane,
/// so an older read cannot overwrite a newer follow confirmation.
@Observable @MainActor
final class InsightsSkillsModel {
    typealias Reader = @Sendable (Bool) async throws -> DaemonData.SkillsSnapshot
    typealias Writer = @Sendable (String, Bool) async throws -> DaemonData.SkillsSnapshot
    typealias DefinitionReader = @Sendable (String) async throws -> DaemonData.SkillDefinition
    typealias EvidenceReader = @Sendable (String) async throws -> DaemonData.SkillEvidence
    private let definitionReader: DefinitionReader?
    private let evidenceReader: EvidenceReader?
    private(set) var definition: DaemonData.SkillDefinition?
    private(set) var evidence: DaemonData.SkillEvidence?
    private(set) var definitionFailure = false
    private(set) var evidenceFailure = false
    private(set) var detailEpoch = UUID()
    private var definitionGeneration = UUID()
    private var evidenceGeneration = UUID()
    private var selectedDefinition: String?
    private var selectedEvidence: String?
    private let reader: Reader?
    private let writer: Writer?
    private let fallbackCopy: [String: String]
    private let idlePollInterval: Duration
    private(set) var shown: DaemonData.SkillsSnapshot?
    private(set) var busy = false
    private(set) var failureKey: String?
    var canFollow: Bool { writer != nil && !busy }

    init(reader: Reader? = nil, writer: Writer? = nil, definitionReader: DefinitionReader? = nil, evidenceReader: EvidenceReader? = nil, copy: [String: String] = [:],
         idlePollInterval: Duration = .seconds(5 * 60)) {
        self.definitionReader = definitionReader; self.evidenceReader = evidenceReader
        self.reader = reader; self.writer = writer; self.fallbackCopy = copy
        self.idlePollInterval = idlePollInterval
    }

    func invalidateDetails() {
        definitionGeneration = UUID(); evidenceGeneration = UUID()
        selectedDefinition = nil; selectedEvidence = nil
        definition = nil; evidence = nil
        definitionFailure = false; evidenceFailure = true
        detailEpoch = UUID()
    }

    func loadDefinition(_ id: String) async {
        let generation = UUID(); definitionGeneration = generation
        selectedDefinition = id; definition = nil; definitionFailure = false
        guard let definitionReader else { definitionFailure = true; return }
        do {
            let result = try await definitionReader(id)
            guard !Task.isCancelled, selectedDefinition == id, definitionGeneration == generation else { return }
            definition = result; definitionFailure = result.status != "available"
        } catch { if !Task.isCancelled, selectedDefinition == id, definitionGeneration == generation { definitionFailure = true } }
    }
    func loadEvidence(_ reference: String) async {
        let generation = UUID(); evidenceGeneration = generation
        selectedEvidence = reference; evidence = nil; evidenceFailure = false
        guard let evidenceReader else { evidenceFailure = true; return }
        do {
            let result = try await evidenceReader(reference)
            guard !Task.isCancelled, selectedEvidence == reference, evidenceGeneration == generation else { return }
            evidence = result; evidenceFailure = !result.enabled || result.readable != true || result.session == nil
        } catch { if !Task.isCancelled, selectedEvidence == reference, evidenceGeneration == generation { evidenceFailure = true } }
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

    func inventorySkills(in snapshot: DaemonData.SkillsSnapshot, matching query: String,
                         agent: String = "", availability: String = "", sort: String = "name",
                         days: Int = 30) -> [DaemonData.InstalledSkill] {
        let query = query.trimmingCharacters(in: .whitespacesAndNewlines)
        return snapshot.inventory.skills.filter { skill in
            (query.isEmpty || [skill.name, skill.description].contains { $0.localizedStandardContains(query) })
                && (agent.isEmpty || skill.agent == agent)
                && (availability.isEmpty || (snapshot.inventory.details[skill.id]?.availability ?? "unknown") == availability)
        }.sorted { lhs, rhs in
            let left = usage(for: lhs, in: snapshot), right = usage(for: rhs, in: snapshot)
            if sort == "sessions" {
                let a = left?.windows.first { $0.days == days }?.sessions ?? 0
                let b = right?.windows.first { $0.days == days }?.sessions ?? 0
                if a != b { return a > b }
            }
            if sort == "recent" {
                let a = usageRows(for: lhs, in: snapshot).compactMap(\.lastSeen).max() ?? ""
                let b = usageRows(for: rhs, in: snapshot).compactMap(\.lastSeen).max() ?? ""
                if a != b { return a > b }
            }
            return lhs.name.localizedStandardCompare(rhs.name) == .orderedAscending
        }
    }

    /// Shared roots can be consumed by several agents. Keep their observations
    /// separate: capped all-time references cannot deduplicate window totals.
    func usageRows(for skill: DaemonData.InstalledSkill, in snapshot: DaemonData.SkillsSnapshot) -> [DaemonData.SkillsUsageRow] {
        guard !hasUsageCollision(skill, in: snapshot) else { return [] }
        if skill.agent == "shared" {
            let supported = Set(snapshot.usage.coverage.filter {
                ["codex", "claude_code", "opencode"].contains($0.agent) && $0.status != "unsupported"
            }.map(\.agent))
            return snapshot.usage.skills.filter { $0.name == skill.name && supported.contains($0.agent) }
                .sorted { $0.agent < $1.agent }
        }
        return snapshot.usage.skills.filter { $0.agent == skill.agent && $0.name == skill.name }
    }

    private func hasUsageCollision(_ skill: DaemonData.InstalledSkill, in snapshot: DaemonData.SkillsSnapshot) -> Bool {
        snapshot.inventory.skills.filter {
            $0.name == skill.name && (skill.agent == "shared" || $0.agent == "shared" || $0.agent == skill.agent)
        }.count != 1
    }

    /// Only expose a single-row total when its meaning is unambiguous.
    func usage(for skill: DaemonData.InstalledSkill, in snapshot: DaemonData.SkillsSnapshot) -> DaemonData.SkillsUsageRow? {
        let rows = usageRows(for: skill, in: snapshot)
        return rows.count == 1 ? rows.first : nil
    }

    func usageState(for skill: DaemonData.InstalledSkill, in snapshot: DaemonData.SkillsSnapshot) -> String {
        guard snapshot.usage.enabled else { return "skills_usage_unavailable" }
        let coverage = snapshot.usage.coverage.filter {
            skill.agent == "shared" ? ["codex", "claude_code", "opencode"].contains($0.agent) : $0.agent == skill.agent
        }
        guard coverage.contains(where: { $0.status != "unsupported" }) else { return "skills_usage_unavailable" }
        if hasUsageCollision(skill, in: snapshot) { return "skills_usage_unattributed" }
        if snapshot.usage.partial || snapshot.usage.lastAnalysisAt == nil { return "skills_usage_pending" }
        return "skills_no_usage"
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
            invalidateDetails()
            if snapshot.readable {
                shown = snapshot
                failureKey = snapshot.error == nil ? nil : "skills_refresh_failed"
            } else {
                failureKey = shown == nil ? "skills_unavailable" : "skills_refresh_failed"
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
            invalidateDetails()
            guard snapshot.readable else { failureKey = "skills_follow_failed"; return }
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
