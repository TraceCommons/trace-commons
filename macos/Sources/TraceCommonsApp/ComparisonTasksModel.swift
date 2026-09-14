import Foundation
import Observation
import TCBridge

@Observable @MainActor
final class ComparisonTasksModel {
    typealias Service = @Sendable (InsightsRequest) async throws -> InsightsResponse
    private let service: Service
    private let calendar: Calendar
    private let now: @Sendable () -> Date
    private var active = false
    private var generation = UUID()
    private var presentation = UUID()
    private var task: Task<Void, Never>?
    private var mutationTask: Task<Void, Never>?
    private var refreshAfterMutation = false

    private(set) var tasks: [ComparisonTaskDetail] = []
    private(set) var detail: ComparisonTaskDetail?
    private(set) var busy = false
    private(set) var error: String?
    private(set) var notice: String?
    var createSelection = Set<String>()
    var editSelection = Set<String>()
    private(set) var editingEpisodes = false
    var projectID = UUID().uuidString.lowercased()
    var taskDate = ""
    var language = ""
    var harnessID = ""
    var harnessVersion = ""
    var reasoningEffort = ComparisonReasoningEffort.unknown
    var toolPolicyID = ""
    var toolPolicyVersion = ""
    var promptTemplateDigest = ""
    var outcome = ComparisonTaskOutcome.pending

    init(calendar: Calendar = .current, now: @escaping @Sendable () -> Date = { Date() },
         service: @escaping Service = { request in
        try await Task.detached { try TCInsights.call(request) }.value
    }) {
        self.calendar = calendar; self.now = now; self.service = service
        taskDate = ComparisonLocalCalendar.day(now(), calendar: calendar)
    }

    func open() { active = true; if mutationTask == nil { refresh() } }
    func close() {
        active = false; generation = UUID(); presentation = UUID(); task?.cancel(); task = nil
        if mutationTask == nil { busy = false }
        detail = nil; editSelection = []; editingEpisodes = false
    }
    func upstreamEvidenceChanged() {
        guard active else { return }
        if busy { refreshAfterMutation = true } else { refresh() }
    }
    func refresh() {
        presentation = UUID(); detail = nil; editSelection = []; editingEpisodes = false
        run(.init("comparison_task_list"), intent: .list)
    }
    func create() {
        let ids = createSelection.sorted()
        guard !ids.isEmpty else { return }
        run(.init("comparison_task_create", episodeIDs: ids), intent: .create)
    }
    func select(_ id: String) {
        presentation = UUID(); detail = nil; editSelection = []; editingEpisodes = false
        run(.init("comparison_task_explain", id: id), intent: .detail(id, presentation))
    }
    func closeDetail() {
        presentation = UUID(); detail = nil; editSelection = []; editingEpisodes = false
        error = nil; notice = nil
    }
    var knownProjectIDs: [String] {
        Array(Set(tasks.compactMap { $0.task.context?.project_id })).sorted()
    }
    func startNewProject() { projectID = UUID().uuidString.lowercased() }
    func beginEpisodeEdit() {
        guard let detail else { return }
        editSelection = Set(detail.task.episodes.map(\.episode_id)); editingEpisodes = true
        presentation = UUID()
    }
    func replaceEpisodes() {
        guard let bound = binding(), !editSelection.isEmpty else { return }
        run(.init("comparison_task_replace_episodes", id: bound.id,
                  episodeIDs: editSelection.sorted(), expectedRevision: bound.revision),
            intent: .mutation(bound, "comparison_task_replace_episodes"))
    }
    func saveContext() {
        guard let bound = binding() else { return }
        let context = ComparisonTaskContextInput(
            projectID: projectID, taskDate: taskDate, language: contextString(language),
            configuration: .init(harnessID: contextString(harnessID),
                                 harnessVersion: contextString(harnessVersion),
                                 reasoningEffort: reasoningEffort,
                                 toolPolicyID: contextString(toolPolicyID),
                                 toolPolicyVersion: contextString(toolPolicyVersion),
                                 promptTemplateDigest: promptTemplateDigest.isEmpty
                                    ? .unknown : .known(promptTemplateDigest)))
        run(.init("comparison_task_set_context", id: bound.id, expectedRevision: bound.revision,
                  context: context), intent: .mutation(bound, "comparison_task_set_context"))
    }
    func setOutcome() {
        guard let bound = binding() else { return }
        run(.init("comparison_task_set_outcome", id: bound.id, outcome: outcome.rawValue,
                  expectedRevision: bound.revision), intent: .mutation(bound, "comparison_task_set_outcome"))
    }
    func clearOutcome() {
        guard let bound = binding() else { return }
        run(.init("comparison_task_clear_outcome", id: bound.id, expectedRevision: bound.revision),
            intent: .mutation(bound, "comparison_task_clear_outcome"))
    }
    struct Confirmation: Equatable, Sendable { fileprivate let binding: PresentedTask }
    func reconfirmation() -> Confirmation? { binding().map(Confirmation.init) }
    func deletion() -> Confirmation? { binding().map(Confirmation.init) }
    func reconfirm(_ confirmation: Confirmation) {
        let bound = confirmation.binding
        guard accepts(bound) else { return }
        run(.init("comparison_task_reconfirm", id: bound.id, expectedRevision: bound.revision,
                  displayedMaterialDigest: bound.digest),
            intent: .mutation(bound, "comparison_task_reconfirm"))
    }
    func delete(_ confirmation: Confirmation) {
        let bound = confirmation.binding
        guard accepts(bound) else { return }
        run(.init("comparison_task_delete", id: bound.id, expectedRevision: bound.revision),
            intent: .delete(bound))
    }

    private func contextString(_ value: String) -> ComparisonContextString {
        value.isEmpty ? .unknown : .known(value)
    }
    fileprivate struct PresentedTask: Equatable, Sendable {
        let id: String; let revision: UInt64; let digest: String; let token: UUID
    }
    private func binding() -> PresentedTask? {
        guard active, !busy, let value = detail?.task else { return nil }
        return .init(id: value.id, revision: value.revision,
                     digest: value.material_digest, token: presentation)
    }
    private func accepts(_ value: PresentedTask) -> Bool {
        !busy && value.token == presentation && value.id == detail?.task.id
            && value.revision == detail?.task.revision && value.digest == detail?.task.material_digest
    }
    private enum Intent: Sendable {
        case list, create, detail(String, UUID), mutation(PresentedTask, String), delete(PresentedTask)
        var isMutation: Bool {
            switch self { case .create, .mutation, .delete: true; case .list, .detail: false }
        }
    }
    private func run(_ operation: InsightsRequest.Operation, intent: Intent) {
        guard active, !busy else { return }
        busy = true; error = nil; notice = nil
        let screen = generation; let service = service
        let writes = intent.isMutation
        let operationTask = Task { [weak self] in
            do {
                let response = try await service(.init(operation: operation))
                guard let self else { return }
                if writes {
                    self.mutationTask = nil; self.busy = false
                    if self.generation != screen || !self.active {
                        let effect = try self.detachedEffect(response, intent: intent)
                        guard self.active else { return }
                        self.reconcile(id: effect.id, noticeKey: effect.notice)
                        return
                    }
                }
                guard self.active, self.generation == screen, !Task.isCancelled else { return }
                switch intent {
                case .list:
                    guard response.type == "comparison_task_list", let tasks = response.tasks else {
                        throw InsightsError.invalidResponse
                    }
                    try tasks.forEach { try $0.validateSupportedSchema() }
                    self.tasks = tasks; self.finishAndDrainRefresh()
                case .detail(let id, let token):
                    guard self.presentation == token, response.type == "comparison_task_explain",
                          let detail = response.comparisonTaskDetail else { throw InsightsError.invalidResponse }
                    try detail.validateSupportedSchema(expectedID: id)
                    self.install(detail); self.finishAndDrainRefresh()
                case .create:
                    guard response.type == "comparison_task", let value = response.task else {
                        throw InsightsError.invalidResponse
                    }
                    try value.validateSupportedSchema(); self.createSelection = []
                    self.busy = false; self.reconcile(id: value.id, noticeKey: "comparison_task_create")
                case .mutation(let bound, let noticeKey):
                    guard self.acceptsWhileBusy(bound), response.type == "comparison_task",
                          let value = response.task, value.id == bound.id, value.revision >= bound.revision else {
                        throw InsightsError.invalidResponse
                    }
                    try value.validateSupportedSchema(); self.busy = false
                    self.reconcile(id: value.id, noticeKey: noticeKey)
                case .delete(let bound):
                    guard self.acceptsWhileBusy(bound), response.type == "comparison_task_delete",
                          response.task?.id == bound.id else { throw InsightsError.invalidResponse }
                    self.busy = false; self.reconcile(id: nil, noticeKey: "comparison_task_deleted")
                }
            } catch {
                guard let self else { return }
                if writes { self.mutationTask = nil; self.busy = false }
                guard self.active, !Task.isCancelled else { return }
                if self.generation != screen {
                    let retainedError = self.copyKey(for: error)
                    self.discardEditableDetail(); self.refresh(); self.error = retainedError
                    return
                }
                self.busy = false
                self.error = self.copyKey(for: error)
                if case .mutation = intent { self.discardEditableDetail() }
                if case .delete = intent { self.discardEditableDetail() }
                self.drainRefreshPreservingMessages()
            }
        }
        if writes { mutationTask = operationTask } else { task = operationTask }
    }
    private func detachedEffect(_ response: InsightsResponse, intent: Intent) throws -> (id: String?, notice: String) {
        switch intent {
        case .create:
            guard response.type == "comparison_task", let task = response.task else { throw InsightsError.invalidResponse }
            try task.validateSupportedSchema(); return (task.id, "comparison_task_create")
        case .mutation(let bound, let notice):
            guard response.type == "comparison_task", let task = response.task, task.id == bound.id,
                  task.revision >= bound.revision else { throw InsightsError.invalidResponse }
            try task.validateSupportedSchema(); return (task.id, notice)
        case .delete(let bound):
            guard response.type == "comparison_task_delete", response.task?.id == bound.id else {
                throw InsightsError.invalidResponse
            }
            return (nil, "comparison_task_deleted")
        case .list, .detail: throw InsightsError.invalidResponse
        }
    }
    private func acceptsWhileBusy(_ value: PresentedTask) -> Bool {
        value.token == presentation && value.id == detail?.task.id
            && value.revision == detail?.task.revision && value.digest == detail?.task.material_digest
    }
    private func reconcile(id: String?, noticeKey: String) {
        notice = noticeKey; let retainedNotice = notice; let screen = generation
        presentation = UUID(); detail = nil; editSelection = []; editingEpisodes = false; busy = true
        let token = presentation; let service = service
        task = Task { [weak self] in
            do {
                let list = try await service(.init(operation: .init("comparison_task_list")))
                guard let self, self.active, self.generation == screen, !Task.isCancelled,
                      list.type == "comparison_task_list", let values = list.tasks else {
                    throw InsightsError.invalidResponse
                }
                try values.forEach { try $0.validateSupportedSchema() }; self.tasks = values
                if let id {
                    guard values.contains(where: { $0.id == id }) else { throw InsightsError.invalidResponse }
                    let explained = try await service(.init(operation: .init("comparison_task_explain", id: id)))
                    guard self.presentation == token, explained.type == "comparison_task_explain",
                          let detail = explained.comparisonTaskDetail else { throw InsightsError.invalidResponse }
                    try detail.validateSupportedSchema(expectedID: id); self.install(detail)
                }
                self.notice = retainedNotice; self.finishAndDrainRefresh()
            } catch {
                guard let self, self.active, self.generation == screen, !Task.isCancelled else { return }
                self.discardEditableDetail(); self.notice = retainedNotice
                self.error = "comparison_task_committed_reload_failed"
                self.finishAndDrainRefresh()
            }
        }
    }
    private func install(_ value: ComparisonTaskDetail) {
        detail = value; presentation = UUID(); editingEpisodes = false; editSelection = []
        outcome = value.task.outcome?.value ?? .pending
        projectID = UUID().uuidString.lowercased()
        taskDate = ComparisonLocalCalendar.day(now(), calendar: calendar)
        language = ""; harnessID = ""; harnessVersion = ""; reasoningEffort = .unknown
        toolPolicyID = ""; toolPolicyVersion = ""; promptTemplateDigest = ""
        if let context = value.task.context {
            projectID = context.project_id; taskDate = context.task_date
            language = string(context.language); harnessID = string(context.configuration.harness_id)
            harnessVersion = string(context.configuration.harness_version)
            reasoningEffort = context.configuration.reasoning_effort
            toolPolicyID = string(context.configuration.tool_policy_id)
            toolPolicyVersion = string(context.configuration.tool_policy_version)
            promptTemplateDigest = digest(context.configuration.prompt_template_digest)
        }
    }
    private func string(_ value: ComparisonContextString) -> String {
        if case .known(let value) = value { return value }; return ""
    }
    private func digest(_ value: ComparisonContextDigest) -> String {
        if case .known(let value) = value { return value }; return ""
    }
    private func discardEditableDetail() {
        presentation = UUID(); detail = nil; editSelection = []; editingEpisodes = false
    }
    private func finishAndDrainRefresh() {
        busy = false
        drainRefreshPreservingMessages()
    }
    private func drainRefreshPreservingMessages() {
        guard active, !busy, refreshAfterMutation else { return }
        refreshAfterMutation = false
        let retainedNotice = notice; let retainedError = error
        refresh()
        notice = retainedNotice; error = retainedError
    }
    private func copyKey(for error: Error) -> String {
        guard case .service(let code) = error as? InsightsError else { return "error" }
        switch code {
        case "insights_comparison_task_revision_conflict": return "comparison_task_revision_conflict"
        case "insights_comparison_task_material_digest_conflict": return "comparison_task_digest_conflict"
        case "insights_comparison_task_stale_evidence": return "comparison_task_stale_evidence"
        default: return "error"
        }
    }
}
