import Foundation
import Observation
import TCBridge

struct ComparisonStratumOption: Identifiable, Equatable, Sendable {
    let stratum: ExactComparisonStratum
    let taskIDs: [String]
    let cohortCandidates: [String]
    let label: String
    let taskLabels: [String: String]
    var id: String { "\(stratum.project_id):\(stratum.language):\(stratum.configuration_fingerprint)" }
}

@Observable @MainActor
final class ComparisonSpecificationsModel {
    typealias Service = @Sendable (InsightsRequest) async throws -> InsightsResponse
    private let service: Service
    private var active = false
    private var generation = UUID()
    private var presentation = UUID()
    private var evidenceGeneration: UInt64 = 0
    private var readTask: Task<Void, Never>?
    private var saveTask: Task<Void, Never>?
    private var pendingRefresh = false
    private var previewInput: ComparisonSpecificationDraftInput?

    private(set) var specifications: [ComparisonSpecification] = []
    private(set) var selected: ComparisonSpecification?
    private(set) var result: DescriptiveComparisonResult?
    private(set) var previewSpecification: ComparisonSpecification?
    private(set) var previewResult: DescriptiveComparisonResult?
    private(set) var options: [ComparisonStratumOption] = []
    private(set) var busy = false
    private(set) var notice: String?
    private(set) var error: String?
    var selectedStratumID = ""
    var selectedCohorts = Set<String>()
    var dateStart = Date()
    var dateEnd = Date()
    var evidenceCutoff = Date()

    init(service: @escaping Service = { request in
        try await Task.detached { try TCInsights.call(request) }.value
    }) { self.service = service }

    func open() { active = true; if saveTask == nil { refresh() } }
    func close() {
        active = false; generation = UUID(); presentation = UUID(); readTask?.cancel(); readTask = nil
        if saveTask == nil { busy = false }
        selected = nil; result = nil; previewSpecification = nil; previewResult = nil; previewInput = nil
    }
    func updateSources(tasks: [ComparisonTaskDetail], snapshots: [LocalInsight]) {
        let snapshotByID = Dictionary(uniqueKeysWithValues: snapshots.map { ($0.id, $0) })
        let grouped = Dictionary(grouping: tasks.compactMap { detail -> (ExactComparisonStratum, String, Set<String>, String, String)? in
            guard let context = detail.task.context, context.isComplete,
                  case .known(let language) = context.language else { return nil }
            let stratum = ExactComparisonStratum(projectID: context.project_id, language: language,
                configurationFingerprint: context.configuration_fingerprint)
            let ids = detail.task.episodes.flatMap(\.members).map(\.snapshot_id)
            let labels = Set(ids.flatMap { snapshotByID[$0]?.model_observations?.declared_models ?? [] })
            let configuration = context.configuration
            let harness = Self.known(configuration.harness_id)
            let version = Self.known(configuration.harness_version)
            let policy = Self.known(configuration.tool_policy_id)
            let project = String(context.project_id.prefix(8))
            let stratumLabel = "\(project) · \(language) · \(harness) \(version) · \(configuration.reasoning_effort.rawValue) · \(policy)"
            let taskLabel = "\(context.task_date) · \(detail.task.episodes.count) episode\(detail.task.episodes.count == 1 ? "" : "s")"
            return (stratum, detail.id, labels, stratumLabel, taskLabel)
        }, by: { $0.0 })
        options = grouped.map { stratum, rows in
            ComparisonStratumOption(stratum: stratum, taskIDs: rows.map(\.1).sorted(),
                cohortCandidates: Set(rows.flatMap(\.2)).sorted(), label: rows[0].3,
                taskLabels: Dictionary(uniqueKeysWithValues: rows.map { ($0.1, $0.4) }))
        }.sorted { $0.id < $1.id }
        if !options.contains(where: { $0.id == selectedStratumID }) {
            selectedStratumID = options.first?.id ?? ""; selectedCohorts = []
        }
        reconcileCohorts()
    }
    func sourceEvidenceChanged(tasks: [ComparisonTaskDetail], snapshots: [LocalInsight]) {
        updateSources(tasks: tasks, snapshots: snapshots)
        upstreamEvidenceChanged()
    }
    func selectStratum(_ id: String) { selectedStratumID = id; selectedCohorts = []; invalidatePreview() }
    func setCohort(_ label: String, selected: Bool) {
        if selected { if selectedCohorts.count < 2 { selectedCohorts.insert(label) } }
        else { selectedCohorts.remove(label) }
        invalidatePreview()
    }
    var selectedOption: ComparisonStratumOption? { options.first { $0.id == selectedStratumID } }
    func taskLabel(_ id: String) -> String {
        options.lazy.compactMap { $0.taskLabels[id] }.first ?? String(id.prefix(8))
    }
    var canDraft: Bool { selectedOption != nil && selectedCohorts.count == 2 && dateStart <= dateEnd }
    func preview() { guard let input = draftInput() else { return }; runPreview(input) }
    func save() { guard let input = previewInput, saveTask == nil else { return }; runSave(input) }
    func draftChanged() { invalidatePreview() }
    func refresh() { runRead(.init("comparison_list_specs"), intent: .list) }
    func select(_ id: String) { runRead(.init("comparison_get_spec", id: id), intent: .specification(id)) }
    func evaluate() {
        guard let selected else { return }
        runRead(.init("comparison_evaluate", id: selected.id), intent: .evaluation(selected))
    }
    struct ResultConfirmation: Equatable, Sendable {
        fileprivate let specificationID, specificationDigest, savedRecordDigest, auditDigest: String
        fileprivate let presentation: UUID
        fileprivate let evidenceGeneration: UInt64
    }
    func resultConfirmation() -> ResultConfirmation? {
        guard let selected, let result, result.specification_id == selected.id else { return nil }
        return .init(specificationID: selected.id, specificationDigest: selected.specification_digest,
                     savedRecordDigest: selected.saved_record_digest, auditDigest: result.audit_digest,
                     presentation: presentation, evidenceGeneration: evidenceGeneration)
    }
    func explain(_ confirmation: ResultConfirmation) {
        guard confirmation.presentation == presentation, confirmation.specificationID == selected?.id,
              confirmation.evidenceGeneration == evidenceGeneration,
              confirmation.specificationDigest == selected?.specification_digest,
              confirmation.savedRecordDigest == selected?.saved_record_digest,
              confirmation.auditDigest == result?.audit_digest else { return }
        runRead(.init("comparison_explain_result", specificationID: confirmation.specificationID,
                      auditDigest: confirmation.auditDigest), intent: .evaluation(selected!))
    }
    func upstreamEvidenceChanged() {
        guard active else { return }
        evidenceGeneration &+= 1
        result = nil; previewSpecification = nil; previewResult = nil; previewInput = nil
        if busy { pendingRefresh = true }
        else if selected != nil { evaluate() } else { refresh() }
    }

    private enum ReadIntent: Sendable {
        case list, specification(String), evaluation(ComparisonSpecification)
    }
    private func runPreview(_ input: ComparisonSpecificationDraftInput) {
        guard active, !busy else { return }
        busy = true; error = nil; notice = nil; presentation = UUID(); let token = presentation
        let screen = generation; let evidence = evidenceGeneration; let service = service
        readTask = Task { [weak self] in
            do {
                let response = try await service(.init(operation: .init("comparison_preview_spec", input: input)))
                guard let self, self.active, self.generation == screen, self.presentation == token,
                      !Task.isCancelled else { return }
                guard self.evidenceGeneration == evidence else { self.finish(); return }
                guard response.type == "comparison_preview_spec",
                      let spec = response.specification, let result = response.comparisonResult
                else { throw InsightsError.invalidResponse }
                try spec.validateStructure(); try result.validateStructure(expectedSpecification: spec)
                self.previewSpecification = spec; self.previewResult = result; self.previewInput = input
                self.notice = "comparison_preview_notice"; self.finish()
            } catch {
                guard let self else { return }
                if self.active, self.generation == screen, self.evidenceGeneration != evidence {
                    self.finish()
                } else { self.fail(error, screen: screen) }
            }
        }
    }
    private func runSave(_ input: ComparisonSpecificationDraftInput) {
        busy = true; error = nil; notice = nil; let service = service
        saveTask = Task { [weak self] in
            do {
                let response = try await service(.init(operation: .init("comparison_save_spec", input: input)))
                guard let self else { return }; self.saveTask = nil; self.busy = false
                guard response.type == "comparison_specification", let spec = response.specification else {
                    throw InsightsError.invalidResponse
                }
                try spec.validateStructure()
                guard self.active else { return }
                self.notice = "comparison_specification_saved_notice"
                self.reconcileSaved(id: spec.id)
            } catch {
                guard let self else { return }; self.saveTask = nil; self.busy = false
                guard self.active else { return }; self.error = "comparison_specification_error"
                self.drain()
            }
        }
    }
    private func reconcileSaved(id: String) {
        let retained = notice; let screen = generation
        presentation = UUID(); selected = nil; result = nil; busy = true
        let service = service; let token = presentation
        readTask = Task { [weak self] in
            do {
                let list = try await service(.init(operation: .init("comparison_list_specs")))
                guard let self, self.active, self.generation == screen, self.presentation == token,
                      !Task.isCancelled else { return }
                guard list.type == "comparison_specification_list",
                      let specs = list.specifications else { throw InsightsError.invalidResponse }
                try specs.forEach { try $0.validateStructure() }; self.specifications = specs
                let response = try await service(.init(operation: .init("comparison_get_spec", id: id)))
                guard self.active, self.generation == screen, self.presentation == token,
                      !Task.isCancelled else { return }
                guard response.type == "comparison_specification",
                      let spec = response.specification else { throw InsightsError.invalidResponse }
                try spec.validateStructure(); self.selected = spec; self.notice = retained; self.finish()
            } catch {
                guard let self, self.active, self.generation == screen, self.presentation == token,
                      !Task.isCancelled else { return }
                self.notice = retained
                self.error = "comparison_specification_committed_reload_failed"; self.finish()
            }
        }
    }
    private func runRead(_ operation: InsightsRequest.Operation, intent: ReadIntent) {
        guard active, !busy else { pendingRefresh = true; return }
        busy = true; error = nil; presentation = UUID(); let token = presentation
        let screen = generation; let evidence = evidenceGeneration; let service = service
        readTask = Task { [weak self] in
            do {
                let response = try await service(.init(operation: operation))
                guard let self, self.active, self.generation == screen, self.presentation == token,
                      !Task.isCancelled else { return }
                switch intent {
                case .list:
                    guard response.type == "comparison_specification_list", let specs = response.specifications else { throw InsightsError.invalidResponse }
                    try specs.forEach { try $0.validateStructure() }; self.specifications = specs
                case .specification(let id):
                    guard response.type == "comparison_specification", let spec = response.specification,
                          spec.id == id else { throw InsightsError.invalidResponse }
                    try spec.validateStructure(); self.selected = spec; self.result = nil
                case .evaluation(let spec):
                    guard self.evidenceGeneration == evidence else { self.finish(); return }
                    guard response.type == "comparison_result",
                          let result = response.comparisonResult else { throw InsightsError.invalidResponse }
                    try result.validateStructure(expectedSpecification: spec); self.result = result
                }
                self.finish()
            } catch {
                guard let self else { return }
                if self.active, self.generation == screen, self.evidenceGeneration != evidence {
                    self.finish()
                } else { self.fail(error, screen: screen) }
            }
        }
    }
    private func finish() { busy = false; drain() }
    private func fail(_ failure: Error, screen: UUID) {
        guard active, generation == screen else { return }; error = "comparison_specification_error"; finish()
    }
    private func drain() {
        guard pendingRefresh, active, !busy else { return }; pendingRefresh = false
        let retainedNotice = notice; let retainedError = error
        if selected != nil { evaluate() } else { refresh() }
        notice = retainedNotice; error = retainedError
    }
    private func reconcileCohorts() {
        let available = Set(selectedOption?.cohortCandidates ?? [])
        selectedCohorts.formIntersection(available); invalidatePreview()
    }
    private func invalidatePreview() { previewSpecification = nil; previewResult = nil; previewInput = nil; notice = nil }
    private func draftInput() -> ComparisonSpecificationDraftInput? {
        guard canDraft, let option = selectedOption else { return nil }
        return .init(evidenceCutoff: evidenceCutoff.ISO8601Format(), cohortLabels: Array(selectedCohorts),
                     dateStart: Self.day(dateStart), dateEnd: Self.day(dateEnd), stratum: option.stratum)
    }
    static func day(_ date: Date, calendar: Calendar = .current) -> String {
        let parts = calendar.dateComponents([.year, .month, .day], from: date)
        guard let year = parts.year, let month = parts.month, let day = parts.day else { return "" }
        return String(format: "%04d-%02d-%02d", year, month, day)
    }
    private static func known(_ value: ComparisonContextString) -> String {
        if case .known(let text) = value { return text }
        return "?"
    }
}
