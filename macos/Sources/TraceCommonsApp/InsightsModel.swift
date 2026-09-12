import Foundation
import Observation
import TCBridge

@Observable @MainActor
final class InsightsModel {
    typealias Service = @Sendable (InsightsRequest) async throws -> InsightsResponse
    private let service: Service
    private var task: Task<Void, Never>?
    private var episodeTask: Task<Void, Never>?
    private var cardTask: Task<Void, Never>?
    private var generation = UUID()
    private var active = false
    private var selectionRevision = UUID()
    private(set) var busy = false
    private(set) var copy: [String: String] = [:]
    func text(_ key: String) -> String { copy[key] ?? "" }
    private(set) var error: String?
    private(set) var invalidatedEpisodeIDs: [String] = []
    private(set) var comparisonInvalidationGeneration = UUID()
    private(set) var summary: SavedInsightsSummary?
    private(set) var summaryError: String?
    private(set) var loadingSummary = false
    private(set) var snapshots: [LocalInsight] = []
    private(set) var episodes: [EpisodeListEntry] = []
    private(set) var episodeDetail: EpisodeDetail?
    private(set) var episodeBusy = false
    private(set) var episodeError: String?
    private(set) var episodeNotice: String?
    private(set) var cardResult: InsightCardResult?
    private(set) var cardText: String?
    private(set) var cardError: String?
    private(set) var cardBusy = false
    private(set) var cardSnapshotSelection = Set<String>()
    private(set) var cardEpisodeSelection = Set<String>()
    private var cardPresentation = UUID()
    var episodeCreateSelection = Set<String>()
    var episodeEditSelection = Set<String>()
    private(set) var episodeEditingMembers = false
    var episodeCategory = "unknown"
    var episodeOutcome = "unknown"
    private var episodePresentation = UUID()
    private var refreshEpisodesAfterSnapshotChain = false
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

    func open() {
        active = true
        perform(.init("copy"))
        refreshEpisodes()
    }
    func close() {
        active = false; generation = UUID(); task?.cancel(); task = nil; busy = false
        episodePresentation = UUID(); episodeTask?.cancel(); episodeTask = nil; episodeBusy = false
        cardPresentation = UUID(); cardTask?.cancel(); cardTask = nil; cardBusy = false
        cardResult = nil; cardText = nil; cardError = nil
        cardSnapshotSelection = []; cardEpisodeSelection = []
        refreshEpisodesAfterSnapshotChain = false
        episodeDetail = nil; episodeEditSelection = []; episodeEditingMembers = false; episodeCreateSelection = []
        loadingSummary = false
        invalidatedEpisodeIDs = []
    }
    func refresh() { invalidateCards(); perform(.init("list")) }
    func setCardSnapshot(_ id: String, selected: Bool) {
        if selected { cardSnapshotSelection.insert(id) } else { cardSnapshotSelection.remove(id) }
        invalidateCards()
    }
    func setCardEpisode(_ id: String, selected: Bool) {
        if selected { cardEpisodeSelection.insert(id) } else { cardEpisodeSelection.remove(id) }
        invalidateCards()
    }
    func generateCards() {
        guard active, !cardBusy else { return }
        let questions = InsightQuestion.allCases
        let snapshotIDs = cardSnapshotSelection.sorted()
        let episodeIDs = cardEpisodeSelection.sorted()
        cardPresentation = UUID(); let presentation = cardPresentation
        let screenGeneration = generation
        cardBusy = true; cardError = nil; cardResult = nil; cardText = nil
        let service = service
        cardTask = Task { [weak self] in
            do {
                let response = try await service(.init(operation: .init(
                    "question_cards", snapshotIDs: snapshotIDs, episodeIDs: episodeIDs,
                    questions: questions)))
                guard let self, self.active, self.generation == screenGeneration,
                      self.cardPresentation == presentation, !Task.isCancelled,
                      self.cardSnapshotSelection.sorted() == snapshotIDs,
                      self.cardEpisodeSelection.sorted() == episodeIDs,
                      response.type == "question_cards", let result = response.result,
                      let rendered = response.text else { throw InsightsError.invalidResponse }
                try result.validateSupportedSchema(expectedQuestions: questions)
                self.cardResult = result; self.cardText = rendered; self.cardBusy = false
            } catch {
                guard let self, self.active, self.generation == screenGeneration,
                      self.cardPresentation == presentation, !Task.isCancelled else { return }
                self.cardResult = nil; self.cardText = nil; self.cardBusy = false
                self.cardError = self.text("error")
            }
        }
    }
    private func invalidateCards() {
        cardPresentation = UUID(); cardTask?.cancel(); cardTask = nil; cardBusy = false
        cardResult = nil; cardText = nil; cardError = nil
    }
    private func reconcileCardSelections() {
        let snapshotsNow = Set(snapshots.map(\.id)); let episodesNow = Set(episodes.map(\.id))
        let newSnapshots = cardSnapshotSelection.intersection(snapshotsNow)
        let newEpisodes = cardEpisodeSelection.intersection(episodesNow)
        if newSnapshots != cardSnapshotSelection || newEpisodes != cardEpisodeSelection {
            cardSnapshotSelection = newSnapshots; cardEpisodeSelection = newEpisodes
            invalidateCards()
        }
    }
    func refreshEpisodes() {
        invalidateCards()
        episodeTask?.cancel(); episodeBusy = false
        episodePresentation = UUID(); episodeEditSelection = []; episodeEditingMembers = false
        episodeDetail = nil
        runEpisode(.init("episode_list"), intent: .list)
    }
    func openEpisode(_ id: String) {
        episodeTask?.cancel(); episodeBusy = false
        episodePresentation = UUID(); episodeDetail = nil; episodeEditSelection = []; episodeEditingMembers = false
        runEpisode(.init("episode_explain", id: id), intent: .detail(id, episodePresentation))
    }
    func closeEpisode() {
        episodeTask?.cancel(); episodeTask = nil; episodeBusy = false
        episodePresentation = UUID(); episodeDetail = nil; episodeEditSelection = []; episodeEditingMembers = false
        episodeError = nil; episodeNotice = nil
    }
    func createEpisode() {
        let ids = episodeCreateSelection.sorted()
        guard !ids.isEmpty else { episodeError = text("episode_selection_empty"); return }
        runEpisode(.init("episode_create", snapshotIDs: ids), intent: .create)
    }
    func beginEpisodeMemberEdit() {
        guard let detail = episodeDetail else { return }
        episodeEditSelection = Set(detail.episode.members.map(\.snapshot_id))
        episodeEditingMembers = true
        episodePresentation = UUID()
    }
    func saveEpisodeMembers() {
        guard let presented = frozenEpisode(), !episodeEditSelection.isEmpty else {
            episodeError = text("episode_selection_empty"); return
        }
        runEpisode(.init("episode_replace_members", id: presented.id,
                         snapshotIDs: episodeEditSelection.sorted(), expectedRevision: presented.revision),
                   intent: .mutate(presented, text("episode_members_saved")))
    }
    func saveEpisodeAssessment() {
        guard let presented = frozenEpisode() else { return }
        runEpisode(.init("episode_annotate", id: presented.id, category: episodeCategory,
                         outcome: episodeOutcome, expectedRevision: presented.revision),
                   intent: .mutate(presented, text("episode_assessment_saved")))
    }
    struct EpisodeConfirmation: Equatable, Sendable { fileprivate let presented: PresentedEpisode }
    func episodeConfirmation() -> EpisodeConfirmation? { frozenEpisode().map(EpisodeConfirmation.init) }
    func clearEpisodeAssessment(_ confirmation: EpisodeConfirmation) {
        let presented = confirmation.presented
        guard accepts(presented), !episodeBusy else { return }
        runEpisode(.init("episode_clear_assessment", id: presented.id,
                         expectedRevision: presented.revision),
                   intent: .mutate(presented, text("episode_assessment_cleared")))
    }
    func deleteEpisode(_ confirmation: EpisodeConfirmation) {
        let presented = confirmation.presented
        guard accepts(presented), !episodeBusy else { return }
        runEpisode(.init("episode_delete", id: presented.id, expectedRevision: presented.revision),
                   intent: .delete(presented))
    }
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
                    self.reconcileCardSelections()
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
                    if operation.type == "delete" || operation.save == true { self.invalidateCards() }
                    self.invalidatedEpisodeIDs = response.invalidatedEpisodeIDs
                    self.recordComparisonEffects(response)
                    if let episodeID = self.episodeDetail?.episode.id,
                       response.invalidatedEpisodeIDs.contains(episodeID) {
                        self.closeEpisode()
                    }
                    if operation.type == "delete" || operation.save == true {
                        self.refreshEpisodesAfterSnapshotChain = true
                    }
                }
                self.busy = false
                if operation.type == "copy" || operation.save == true || operation.type == "delete"
                    || operation.type == "annotate" || operation.type == "clear_annotation" || self.isEvidenceMutation(operation) {
                    if operation.type != "copy" { self.invalidateCards() }
                    self.perform(.init("list"), preservingMutationEffects: true)
                } else if operation.type == "list" {
                    self.perform(.init("summary"), preservingMutationEffects: true)
                } else if operation.type == "summary", self.refreshEpisodesAfterSnapshotChain {
                    self.refreshEpisodesAfterSnapshotChain = false
                    self.refreshEpisodes()
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
                if operation.type == "summary", self.refreshEpisodesAfterSnapshotChain {
                    self.refreshEpisodesAfterSnapshotChain = false
                    self.refreshEpisodes()
                }
            }
        }
    }

    fileprivate struct PresentedEpisode: Equatable, Sendable {
        let id: String
        let revision: UInt64
        let membershipRevision: UInt64
        let token: UUID
    }
    private enum EpisodeIntent: Sendable {
        case list, create, detail(String, UUID), mutate(PresentedEpisode, String), delete(PresentedEpisode)
    }
    private func frozenEpisode() -> PresentedEpisode? {
        guard active, !episodeBusy, let episode = episodeDetail?.episode else { return nil }
        return .init(id: episode.id, revision: episode.revision,
                     membershipRevision: episode.membership_revision, token: episodePresentation)
    }
    private func accepts(_ value: PresentedEpisode) -> Bool {
        episodeDetail?.episode.id == value.id && episodeDetail?.episode.revision == value.revision
            && episodePresentation == value.token
    }
    private func runEpisode(_ operation: InsightsRequest.Operation, intent: EpisodeIntent) {
        guard active, !episodeBusy else { return }
        episodeBusy = true; episodeError = nil; episodeNotice = nil
        let screenGeneration = generation
        let service = service
        episodeTask = Task { [weak self] in
            do {
                let response = try await service(.init(operation: operation))
                guard let self, self.active, self.generation == screenGeneration, !Task.isCancelled else { return }
                guard response.type == operation.type else { throw InsightsError.invalidResponse }
                switch intent {
                case .list:
                    guard let episodes = response.episodes else { throw InsightsError.invalidResponse }
                    try episodes.forEach { try $0.episode.validateSupportedSchema() }
                    self.episodes = episodes
                    self.reconcileCardSelections()
                    if let id = self.episodeDetail?.episode.id,
                       !episodes.contains(where: { $0.id == id }) {
                        self.episodePresentation = UUID()
                        self.episodeDetail = nil; self.episodeEditSelection = []; self.episodeEditingMembers = false
                    }
                case .create:
                    guard let episode = response.episode else { throw InsightsError.invalidResponse }
                    try episode.validateSupportedSchema()
                    self.recordComparisonEffects(response)
                    self.invalidateCards()
                    self.episodeCreateSelection = []
                    self.episodeNotice = self.text("episode_create_success")
                    self.episodeBusy = false
                    self.refreshEpisodeState(opening: episode.id, preservingNotice: true)
                    return
                case let .detail(id, token):
                    guard token == self.episodePresentation, let detail = response.detail else {
                        throw InsightsError.invalidResponse
                    }
                    try detail.validateSupportedSchema(expectedID: id)
                    self.installEpisodeDetail(detail)
                case let .mutate(presented, notice):
                    guard self.accepts(presented), response.episode?.id == presented.id else {
                        self.episodeBusy = false; return
                    }
                    try response.episode?.validateSupportedSchema()
                    self.recordComparisonEffects(response)
                    self.invalidateCards()
                    var appliedNotice = notice
                    if operation.type == "episode_replace_members",
                       response.episode?.membership_revision != presented.membershipRevision {
                        appliedNotice += " " + self.text("episode_membership_changed")
                    }
                    self.episodeNotice = appliedNotice
                    self.episodeEditSelection = []; self.episodeEditingMembers = false
                    self.episodeBusy = false
                    self.refreshEpisodeState(opening: presented.id, preservingNotice: true)
                    return
                case let .delete(presented):
                    guard self.accepts(presented), response.episode?.id == presented.id else {
                        self.episodeBusy = false; return
                    }
                    try response.episode?.validateSupportedSchema()
                    self.recordComparisonEffects(response)
                    self.invalidateCards()
                    self.closeEpisode()
                    self.episodeNotice = self.text("episode_deleted")
                    self.episodeBusy = false
                    self.refreshEpisodeState(opening: nil, preservingNotice: true)
                    return
                }
                self.episodeBusy = false
            } catch {
                guard let self, self.active, self.generation == screenGeneration, !Task.isCancelled else { return }
                self.episodeBusy = false
                if case InsightsError.service("insights_episode_revision_conflict") = error,
                   case let .mutate(presented, _) = intent {
                    self.handleEpisodeConflict(presented)
                } else if case InsightsError.service("insights_episode_revision_conflict") = error,
                          case let .delete(presented) = intent {
                    self.handleEpisodeConflict(presented)
                } else if case InsightsError.service("insights_episode_not_found") = error {
                    self.closeEpisode(); self.episodeError = self.text("episode_missing")
                    self.refreshEpisodeState(opening: nil, preservingError: true)
                } else {
                    self.episodeError = self.episodeMessage(for: error, list: operation.type == "episode_list")
                }
            }
        }
    }
    private func installEpisodeDetail(_ detail: EpisodeDetail) {
        episodeDetail = detail
        episodeEditSelection = []; episodeEditingMembers = false
        episodeCategory = detail.episode.manual_assessment?.category ?? "unknown"
        episodeOutcome = detail.episode.manual_assessment?.outcome ?? "unknown"
    }
    private func recordComparisonEffects(_ response: InsightsResponse) {
        if !response.staleComparisonTaskIDs.isEmpty { comparisonInvalidationGeneration = UUID() }
    }
    private func episodeMessage(for error: Error, list: Bool) -> String {
        if case let InsightsError.service(code) = error {
            let key: String? = switch code {
            case "insights_episode_member_limit": "episode_member_limit"
            case "insights_episode_limit_exceeded": "episode_limit"
            case "insights_episode_missing_members": "episode_missing_members"
            case "insights_episode_invalid": "episode_invalid"
            case "insights_response_too_large": "episode_response_too_large"
            default: nil
            }
            if let key { return text(key) }
        }
        return text(list ? "episode_list_unavailable" : "episode_detail_unavailable")
    }
    private func handleEpisodeConflict(_ presented: PresentedEpisode) {
        guard accepts(presented) else { return }
        episodeEditSelection = []; episodeEditingMembers = false
        episodeError = text("episode_revision_conflict")
        refreshEpisodeState(opening: presented.id, preservingError: true)
    }
    private func refreshEpisodeState(opening id: String?, preservingNotice: Bool = false,
                                     preservingError: Bool = false) {
        guard active else { return }
        let notice = preservingNotice ? episodeNotice : nil
        let retainedError = preservingError ? episodeError : nil
        episodePresentation = UUID()
        episodeDetail = nil; episodeEditSelection = []; episodeEditingMembers = false
        episodeBusy = true
        let screenGeneration = generation
        let token = episodePresentation
        let service = service
        episodeTask = Task { [weak self] in
            var resolvingDetail = false
            do {
                let list = try await service(.init(operation: .init("episode_list")))
                guard let self, self.active, self.generation == screenGeneration, !Task.isCancelled,
                      list.type == "episode_list", let episodes = list.episodes else {
                    throw InsightsError.invalidResponse
                }
                try episodes.forEach { try $0.episode.validateSupportedSchema() }
                self.episodes = episodes
                if let id {
                    guard episodes.contains(where: { $0.id == id }) else {
                        self.episodeError = self.text("episode_missing")
                        self.episodeNotice = notice; self.episodeBusy = false
                        return
                    }
                    resolvingDetail = true
                    let result = try await service(.init(operation: .init("episode_explain", id: id)))
                    guard self.active, self.generation == screenGeneration, self.episodePresentation == token,
                          !Task.isCancelled, result.type == "episode_explain", let detail = result.detail else {
                        throw InsightsError.invalidResponse
                    }
                    try detail.validateSupportedSchema(expectedID: id)
                    self.installEpisodeDetail(detail)
                }
                self.episodeNotice = notice; self.episodeError = retainedError; self.episodeBusy = false
            } catch {
                guard let self, self.active, self.generation == screenGeneration, !Task.isCancelled else { return }
                self.episodePresentation = UUID()
                self.episodeDetail = nil; self.episodeEditSelection = []; self.episodeEditingMembers = false
                self.episodeNotice = notice
                self.episodeError = retainedError ?? self.text(resolvingDetail
                    ? "episode_detail_unavailable" : "episode_list_unavailable")
                self.episodeBusy = false
            }
        }
    }
}
