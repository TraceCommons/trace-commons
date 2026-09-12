import Foundation
import Observation
import TCBridge

@Observable @MainActor
final class InsightsModel {
    typealias Service = @Sendable (InsightsRequest) async throws -> InsightsResponse
    private let service: Service
    private var task: Task<Void, Never>?
    private var generation = UUID()
    private var active = false
    private var selectionRevision = UUID()
    private(set) var busy = false
    private(set) var copy: [String: String] = [:]
    func text(_ key: String) -> String { copy[key] ?? "" }
    private(set) var error: String?
    private(set) var invalidatedEpisodeIDs: [String] = []
    private(set) var summary: SavedInsightsSummary?
    private(set) var summaryError: String?
    private(set) var loadingSummary = false
    private(set) var snapshots: [LocalInsight] = []
    var assessmentCategory = "unknown"
    var assessmentOutcome = "unknown"
    private(set) var selected: LocalInsight? {
        didSet {
            selectionRevision = UUID()
            assessmentCategory = selected?.manual_annotation?.category ?? "unknown"
            assessmentOutcome = selected?.manual_annotation?.outcome ?? "unknown"
        }
    }
    private(set) var selectedIsSaved = false
    private(set) var selectedFile: URL?
    private var selectedSource = "codex"

    init(service: @escaping Service = { request in
        try await Task.detached {
            let file = (request.operation.file ?? request.operation.repository).map { URL(fileURLWithPath: $0) }
            let scoped = file?.startAccessingSecurityScopedResource() ?? false
            defer { if scoped { file?.stopAccessingSecurityScopedResource() } }
            return try TCInsights.call(request)
        }.value
    }) { self.service = service }

    func open() { active = true; perform(.init("copy")) }
    func close() {
        active = false; generation = UUID(); task?.cancel(); task = nil; busy = false
        loadingSummary = false
        invalidatedEpisodeIDs = []
    }
    func refresh() { perform(.init("list")) }
    func analyze(file: URL, source: String) {
        guard active, !busy else { return }
        selected = nil; selectedIsSaved = false
        selectedFile = file; selectedSource = source
        perform(.init("analyze", source: source, file: file.path, save: false))
    }
    /// Save re-reads the selected file. The displayed result becomes the new
    /// dated snapshot; it is never represented as saving old analyzed bytes.
    func save() {
        guard let selectedFile, selected != nil, !selectedIsSaved else { return }
        perform(.init("analyze", source: selectedSource, file: selectedFile.path, save: true))
    }
    func explain(_ id: String) {
        guard active, !busy else { return }
        selected = nil; selectedIsSaved = false; selectedFile = nil
        perform(.init("explain", id: id))
    }
    func delete() {
        guard selectedIsSaved, let selected else { return }
        perform(.init("delete", id: selected.id))
    }
    func annotate(category: String, outcome: String) {
        guard selectedIsSaved, let selected else { return }
        perform(.init("annotate", id: selected.id, category: category, outcome: outcome))
    }
    func clearAnnotation() {
        guard selectedIsSaved, let selected else { return }
        perform(.init("clear_annotation", id: selected.id))
    }
    struct EvidenceSelection: Equatable, Sendable {
        let snapshotID: String
        fileprivate let generation: UUID
        fileprivate let revision: UUID
    }
    func evidenceSelection() -> EvidenceSelection? {
        guard active, !busy, selectedIsSaved, let selected else { return nil }
        return EvidenceSelection(snapshotID: selected.id, generation: generation, revision: selectionRevision)
    }
    func linkGit(selection: EvidenceSelection, repository: URL, commit: String) {
        guard acceptsEvidenceSelection(selection) else { return }
        performEvidence(.init("link_git", id: selection.snapshotID, repository: repository.path, commit: commit))
    }
    func linkTestReport(selection: EvidenceSelection, file: URL) {
        guard acceptsEvidenceSelection(selection) else { return }
        performEvidence(.init("link_test_report", file: file.path, id: selection.snapshotID))
    }
    func unlinkEvidence(selection: EvidenceSelection, evidenceID: String) {
        guard acceptsEvidenceSelection(selection),
              selected?.outcome_links?.contains(where: { $0.id == evidenceID }) == true else { return }
        performEvidence(.init("unlink_evidence", id: selection.snapshotID, evidenceID: evidenceID))
    }
    private func acceptsEvidenceSelection(_ selection: EvidenceSelection) -> Bool {
        guard active, !busy else { return false }
        guard evidenceSelection() == selection else {
            error = text("link_changed_selection")
            return false
        }
        return true
    }
    private func performEvidence(_ operation: InsightsRequest.Operation) {
        // The picker token has been checked; clear the old detail so failure
        // cannot masquerade as a successful association to another snapshot.
        selected = nil; selectedIsSaved = false; selectedFile = nil
        perform(operation)
    }
    private func isEvidenceMutation(_ operation: InsightsRequest.Operation) -> Bool {
        ["link_git", "link_test_report", "unlink_evidence"].contains(operation.type)
    }
    private func perform(_ operation: InsightsRequest.Operation, preservingMutationEffects: Bool = false) {
        guard active, !busy else { return }
        busy = true; error = nil
        if !preservingMutationEffects { invalidatedEpisodeIDs = [] }
        if operation.type == "copy" || operation.type == "list" || operation.type == "summary"
            || operation.type == "delete" || operation.type == "annotate"
            || operation.type == "clear_annotation" || operation.save == true || isEvidenceMutation(operation) {
            summary = nil; summaryError = nil; loadingSummary = true
        }
        let token = generation
        let service = service
        task = Task { [weak self] in
            do {
                let response = try await service(.init(operation: operation))
                guard let self, self.active, self.generation == token, !Task.isCancelled else { return }
                guard response.type == operation.type else { throw InsightsError.invalidResponse }
                switch response.type {
                case "copy":
                    guard let copy = response.copy else { throw InsightsError.invalidResponse }
                    self.copy = copy
                case "list":
                    guard let values = response.insights else { throw InsightsError.invalidResponse }
                    self.snapshots = values
                    if self.selectedIsSaved, let id = self.selected?.id {
                        self.selected = values.first { $0.id == id }
                        if self.selected == nil {
                            self.selectedFile = nil; self.selectedIsSaved = false
                        }
                    }
                case "summary":
                    guard let summary = response.summary else { throw InsightsError.invalidResponse }
                    try summary.validateSupportedSchema()
                    self.summary = summary
                    self.loadingSummary = false
                case "delete":
                    guard response.deleted != nil else { throw InsightsError.invalidResponse }
                    self.snapshots.removeAll { $0.id == operation.id }
                    self.selected = nil; self.selectedFile = nil; self.selectedIsSaved = false
                default:
                    guard let value = response.insight else { throw InsightsError.invalidResponse }
                    if self.isEvidenceMutation(operation), value.id != operation.id {
                        throw InsightsError.invalidResponse
                    }
                    self.selected = value
                    self.selectedIsSaved = operation.type != "analyze" || operation.save == true
                    if self.selectedIsSaved {
                        self.snapshots.removeAll { $0.id == value.id }
                        self.snapshots.insert(value, at: 0)
                    }
                }
                if operation.type == "analyze" || operation.type == "delete" {
                    self.invalidatedEpisodeIDs = response.invalidatedEpisodeIDs
                }
                self.busy = false
                if operation.type == "copy" || operation.save == true || operation.type == "delete"
                    || operation.type == "annotate" || operation.type == "clear_annotation" || self.isEvidenceMutation(operation) {
                    self.perform(.init("list"), preservingMutationEffects: true)
                } else if operation.type == "list" {
                    self.perform(.init("summary"), preservingMutationEffects: true)
                }
            } catch {
                guard let self, self.active, self.generation == token, !Task.isCancelled else { return }
                if !preservingMutationEffects { self.invalidatedEpisodeIDs = [] }
                self.error = self.copy["error"] ?? "insights-operation-failed"
                if self.loadingSummary {
                    self.summary = nil
                    self.summaryError = self.copy["summary_unavailable"] ?? self.error
                    self.loadingSummary = false
                }
                self.busy = false
            }
        }
    }
}
