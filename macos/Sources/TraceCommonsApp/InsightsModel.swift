import Foundation
import Observation
import TCBridge
import TCShellCore

@Observable @MainActor
final class InsightsModel {
    typealias Service = @Sendable (InsightsRequest) async throws -> InsightsResponse
    /// Shared with the sibling tab models, so every tab reads the same store.
    let service: Service
    /// Reads `insights_week` through the daemon client (Insights feed T).
    typealias WeekReader = @Sendable (String?) async throws -> DaemonData.InsightsWeek
    /// Which counter rows the window's week figures come from. One at a time;
    /// the two are never mixed.
    enum WeekFeed: Equatable, Sendable { case saved, counterPass }
    /// The core's feed-line keys (`service::INSIGHTS_FEED_LINE_KEYS`). The
    /// window names the feed it shows with these and nothing else.
    enum FeedLineKey {
        static let saved = "insights_feed_saved"
        static let counterPass = "insights_feed_counter_pass"
        static let counterPassUnavailable = "insights_feed_counter_pass_unavailable"
    }
    private let weekReader: WeekReader?
    private var weekGeneration = UUID()
    /// Feed T only when the daemon answered enabled and readable; feed S
    /// (saved imports) otherwise.
    private(set) var weekFeed: WeekFeed = .saved
    /// The daemon's week, held only while `weekFeed` is `.counterPass`.
    private(set) var counterWeek: DaemonData.InsightsWeek?
    /// Set when feed T is switched on but could not be read, shown with the
    /// saved-imports line; `nil` when it is off or the daemon predates it.
    private(set) var counterPassNoticeKey: String?
    var feedLineKey: String { weekFeed == .counterPass ? FeedLineKey.counterPass : FeedLineKey.saved }
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
    }, weekReader: WeekReader? = nil) {
        self.service = service
        self.weekReader = weekReader
    }

    /// Ask the daemon for one week of feed T (`nil` is the current week).
    /// Shows it only when it is switched on and readable; otherwise the
    /// saved-imports feed, with a notice only when it is on but unreadable.
    /// A later failure drops an earlier T week whole.
    func loadWeek(isoWeek: String? = nil) async {
        let generation = UUID(); weekGeneration = generation
        guard let weekReader else { showSavedFeed(notice: false); return }
        let outcome: Result<DaemonData.InsightsWeek, Error>
        do { outcome = .success(try await weekReader(isoWeek)) } catch { outcome = .failure(error) }
        guard weekGeneration == generation else { return }
        switch outcome {
        case .success(let week) where week.showsCounterPass:
            weekFeed = .counterPass; counterWeek = week; counterPassNoticeKey = nil
        case .success(let week):
            showSavedFeed(notice: week.enabled)
        case .failure(let error):
            showSavedFeed(notice: !((error as? DaemonDataError)?.isNotServed ?? false))
        }
    }
    private func showSavedFeed(notice: Bool) {
        weekFeed = .saved; counterWeek = nil
        counterPassNoticeKey = notice ? FeedLineKey.counterPassUnavailable : nil
    }

    func open() {
        active = true
        perform(.init("copy"))
        refreshEpisodes()
        if weekReader != nil { Task { [weak self] in await self?.loadWeek() } }
    }
    func close() {
        active = false; generation = UUID(); task?.cancel(); task = nil; busy = false
        weekGeneration = UUID()
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
                self.cardError = self.cardMessage(for: error)
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
    private func cardMessage(for error: Error) -> String {
        if case let InsightsError.service(code) = error {
            let key: String? = switch code {
            case "insights_card_snapshot_limit": "card_snapshot_limit"
            case "insights_card_episode_limit": "card_episode_limit"
            default: nil
            }
            if let key { return text(key) }
        }
        return text("error")
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

/// The Overview tab's state: one saved week (feed S) and, when asked, what
/// makes up one card's figure. Every figure is the core's; a failed read
/// clears the figures rather than keep stale ones.
@Observable @MainActor
final class InsightsOverviewModel {
    private let service: InsightsModel.Service
    private var task: Task<Void, Never>?
    private var token = UUID()
    private var active = false
    private(set) var busy = false
    private(set) var failed = false
    private(set) var overview: InsightsWeekOverview?
    /// The daemon's week (feed T), in the same shape. While it is set it is
    /// the only week shown; the saved week is never mixed in.
    private(set) var counter: InsightsWeekOverview?
    private(set) var inputs: InsightsCardInputs?
    /// The week asked for; `nil` is the current week.
    private(set) var requestedWeek: String?

    init(service: @escaping InsightsModel.Service) { self.service = service }

    /// The shell's current UTC offset, which buckets the week's days.
    static var offset: Int32 { Int32(TimeZone.current.secondsFromGMT()) }

    /// The week on screen: feed T when the daemon sent one, feed S otherwise.
    var shown: InsightsWeekOverview? { counter ?? overview }

    /// Show the daemon's week, or `nil` to go back to the saved week. The
    /// saved week's drill-down closes either way.
    func showCounter(_ week: InsightsWeekOverview?) {
        counter = week; inputs = nil
    }

    func open() { active = true; load() }
    func close() { active = false; token = UUID(); task?.cancel(); task = nil; busy = false; inputs = nil }
    func reload() { load() }
    func selectWeek(_ weekStart: String) { requestedWeek = weekStart; load() }

    /// Read the drill-down for `card` (`tokens`, `cache_share` or `sessions`)
    /// in the week on screen.
    func showInputs(_ card: String) {
        // Feed T rows carry no session reference, so only the saved week
        // drills down.
        guard active, counter == nil, let week = overview?.week_start else { return }
        run(.init("card_inputs", weekStart: week, tz: Self.offset, card: card)) { model, response in
            guard let inputs = response.inputs, inputs.card == card else { throw InsightsError.invalidResponse }
            model.inputs = inputs
        }
    }
    func hideInputs() { inputs = nil }

    private func load() {
        guard active else { return }
        inputs = nil
        run(.init("week_overview", weekStart: requestedWeek, tz: Self.offset)) { model, response in
            guard let overview = response.overview else { throw InsightsError.invalidResponse }
            model.overview = overview
        }
    }

    private func run(_ operation: InsightsRequest.Operation,
                     apply: @escaping @MainActor (InsightsOverviewModel, InsightsResponse) throws -> Void) {
        task?.cancel()
        token = UUID(); let current = token
        busy = true
        if operation.type != "card_inputs" { failed = false }
        let service = service
        task = Task { [weak self] in
            do {
                let response = try await service(.init(operation: operation))
                guard let self, self.active, self.token == current, !Task.isCancelled else { return }
                guard response.type == operation.type else { throw InsightsError.invalidResponse }
                try apply(self, response)
                self.busy = false
            } catch {
                guard let self, self.active, self.token == current, !Task.isCancelled else { return }
                // A failed drill-down closes only itself; a failed week read
                // never keeps the old figures.
                if operation.type != "card_inputs" { self.overview = nil; self.failed = true }
                self.inputs = nil; self.busy = false
            }
        }
    }
}

/// The core's analytics words, filled. Nothing here composes a sentence: a
/// template's `{name}` holes take numbers the core computed.
enum InsightsOverviewWords {
    static func text(_ key: String, _ copy: [String: String]) -> String { copy[key] ?? "" }

    static func fill(_ template: String, _ values: [String: String]) -> String {
        values.reduce(template) { result, hole in
            result.replacingOccurrences(of: "{" + hole.key + "}", with: hole.value)
        }
    }

    /// The figure, or the dash for unknown. A measured zero stays zero.
    static func figure(_ value: UInt64?, copy: [String: String]) -> String {
        guard let value else { return text("analytics_unavailable", copy) }
        return value.formatted(.number.notation(.compactName))
    }

    /// Whole percent from the core's per mille, rounded half up.
    static func share(_ share: InsightsShareFigure?, copy: [String: String]) -> String {
        guard let share else { return text("analytics_unavailable", copy) }
        return String((share.permille + 5) / 10)
    }

    /// Whole percent from a per mille, rounded half up.
    static func percent(_ permille: UInt64) -> String { String((permille + 5) / 10) }

    /// "vs last week". The core sends a figure only under feed T; any
    /// unavailable reason is the dash.
    static func change(_ source: InsightsWeekSource, copy: [String: String]) -> String {
        changeLine(source.change_permille, copy: copy)
    }

    /// "▼ {p}% vs last week" / "▲ {p}% vs last week", or the dash.
    static func changeLine(_ permille: Int64?, copy: [String: String]) -> String {
        guard let permille else { return text("analytics_unavailable", copy) }
        let key = permille < 0 ? "analytics_change_down" : "analytics_change_up"
        return fill(text(key, copy), ["p": percent(permille.magnitude)])
    }

    /// "Your best week · previous best {q}%", or the dash.
    static func bestWeek(_ source: InsightsWeekSource, copy: [String: String]) -> String {
        guard let best = source.best else { return text("analytics_unavailable", copy) }
        return fill(text("analytics_best_week", copy), ["q": percent(best.previous_best_permille)])
    }

    /// Where the previous best sits on the share bar; `nil` without one.
    static func bestTick(_ source: InsightsWeekSource) -> UInt64? { source.best?.previous_best_permille }

    static func sourceLine(_ source: String, copy: [String: String]) -> String {
        text("analytics_source_" + source, copy)
    }

    static func harness(_ source: String?, copy: [String: String]) -> String {
        guard let source else { return text("analytics_unavailable", copy) }
        return text(source, copy)
    }

    static func coverageLine(_ coverage: InsightsWeekCoverage, copy: [String: String]) -> String {
        fill(text("analytics_coverage_line", copy), [
            "k": String(coverage.known), "n": String(coverage.sessions),
            "p": String(coverage.partial), "u": String(coverage.unknown),
        ])
    }

    /// Which feed is showing. Feed S also says that weeks are not compared.
    static func feedLines(_ feed: String, copy: [String: String]) -> [String] {
        switch feed {
        case "saved":
            return [text("analytics_feed_saved", copy), text("analytics_feed_comparisons_need_counter_pass", copy)]
        case "counter_pass":
            return [text("analytics_feed_counter_pass", copy)]
        case "ledger":
            return [text("analytics_feed_ledger", copy)]
        default:
            return []
        }
    }

    /// `yyyy-MM-dd` (a Monday) as the ISO week `insights_week` takes.
    static func isoWeek(_ weekStart: String) -> String? {
        let parse = Date.ISO8601FormatStyle().year().month().day()
        guard let day = try? Date(weekStart, strategy: parse) else { return nil }
        var calendar = Calendar(identifier: .iso8601)
        calendar.timeZone = TimeZone(secondsFromGMT: 0) ?? .current
        let parts = calendar.dateComponents([.yearForWeekOfYear, .weekOfYear], from: day)
        guard let year = parts.yearForWeekOfYear, let week = parts.weekOfYear else { return nil }
        return String(format: "%04d-W%02d", year, week)
    }

    static func reason(_ wire: String, copy: [String: String]) -> String { text("analytics_reason_" + wire, copy) }
    static func state(_ wire: String, copy: [String: String]) -> String { text("analytics_state_" + wire, copy) }

    struct ModelRow: Identifiable, Equatable {
        let id: Int
        let label: String
        let tokens: UInt64
    }

    /// By model, in the core's fixed order. Never sorted by value here.
    static func modelRows(_ overview: InsightsWeekOverview, copy: [String: String]) -> [ModelRow] {
        overview.by_model.enumerated().map { index, row in
            ModelRow(id: index, label: row.label ?? text("analytics_unknown_label", copy), tokens: row.tokens)
        }
    }

    /// The week's dates, Monday to Sunday, in the user's locale.
    static func weekRange(start: String, end: String) -> String {
        let parse = Date.ISO8601FormatStyle().year().month().day()
        guard let first = try? Date(start, strategy: parse), let last = try? Date(end, strategy: parse) else {
            return start
        }
        let style = Date.FormatStyle(timeZone: TimeZone(secondsFromGMT: 0) ?? .current).month(.abbreviated).day()
        return first.formatted(style) + " \u{2013} " + last.formatted(style)
    }
}

/// The Patterns tab ("Where tokens went") over the saved snapshots (feed S).
@Observable @MainActor
final class InsightsPatternsModel {
    private let service: InsightsModel.Service
    private var task: Task<Void, Never>?
    private var token = UUID()
    private var active = false
    private(set) var busy = false
    private(set) var failed = false
    private(set) var patterns: InsightsWeekPatterns?
    /// The daemon's week (feed T), in the same shape; while it is set it is
    /// the only week shown.
    private(set) var counter: InsightsWeekPatterns?
    /// The sessions behind one card, while its list is open.
    private(set) var sessions: InsightsPatternSessions?

    /// The week on screen: feed T when the daemon sent one, feed S otherwise.
    var shown: InsightsWeekPatterns? { counter ?? patterns }

    func showCounter(_ week: InsightsWeekPatterns?) {
        counter = week; sessions = nil
    }
    /// The week asked for; `nil` is the current week.
    private(set) var requestedWeek: String?

    init(service: @escaping InsightsModel.Service) { self.service = service }

    func open() { active = true; load() }
    func close() { active = false; token = UUID(); task?.cancel(); task = nil; busy = false; sessions = nil }
    func reload() { load() }
    func selectWeek(_ weekStart: String) { requestedWeek = weekStart; load() }

    /// Read the sessions behind the card of `kind` in the week on screen.
    func showSessions(_ kind: String) {
        guard active, counter == nil, let week = patterns?.week_start else { return }
        run(.init("pattern_sessions", weekStart: week, tz: InsightsOverviewModel.offset, pattern: kind)) { model, response in
            guard let found = response.pattern_sessions, found.pattern == kind else { throw InsightsError.invalidResponse }
            model.sessions = found
        }
    }
    func hideSessions() { sessions = nil }

    private func load() {
        guard active else { return }
        sessions = nil
        run(.init("patterns", weekStart: requestedWeek, tz: InsightsOverviewModel.offset)) { model, response in
            guard let patterns = response.patterns else { throw InsightsError.invalidResponse }
            model.patterns = patterns
        }
    }

    private func run(_ operation: InsightsRequest.Operation,
                     apply: @escaping @MainActor (InsightsPatternsModel, InsightsResponse) throws -> Void) {
        task?.cancel()
        token = UUID(); let current = token
        busy = true
        let listing = operation.type == "pattern_sessions"
        if !listing { failed = false }
        let service = service
        task = Task { [weak self] in
            do {
                let response = try await service(.init(operation: operation))
                guard let self, self.active, self.token == current, !Task.isCancelled else { return }
                guard response.type == operation.type else { throw InsightsError.invalidResponse }
                try apply(self, response)
                self.busy = false
            } catch {
                guard let self, self.active, self.token == current, !Task.isCancelled else { return }
                // A failed session list closes only itself; a failed week
                // read never keeps the old figures.
                if !listing { self.patterns = nil; self.failed = true }
                self.sessions = nil; self.busy = false
            }
        }
    }
}

/// The core's Patterns words, filled with the core's figures. Nothing here
/// composes a sentence.
enum InsightsPatternsWords {
    private static func text(_ key: String, _ copy: [String: String]) -> String { copy[key] ?? "" }
    private static func dash(_ copy: [String: String]) -> String { text("analytics_unavailable", copy) }
    private static func count(_ value: UInt32?, _ copy: [String: String]) -> String {
        value.map(String.init) ?? dash(copy)
    }

    /// The card's name, by its wire kind.
    static func title(_ kind: String, copy: [String: String]) -> String {
        text("analytics_pattern_" + kind, copy)
    }

    /// The line under the headline: the core's count sentence, or the
    /// long-context threshold line.
    static func countLine(_ card: InsightsPatternCard, threshold: UInt64, copy: [String: String]) -> String {
        let fill = InsightsOverviewWords.fill
        switch card.kind {
        case "repeated_reads":
            return fill(text("analytics_pattern_repeated_reads_count", copy),
                        ["r": count(card.count, copy), "f": count(card.files, copy)])
        case "retried_calls":
            return fill(text("analytics_pattern_retried_calls_count", copy), ["c": count(card.count, copy)])
        case "edit_fail_edit":
            return fill(text("analytics_pattern_edit_fail_edit_count", copy), ["l": count(card.count, copy)])
        case "long_context":
            return fill(text("analytics_pattern_long_context_line", copy),
                        ["threshold": InsightsOverviewWords.figure(threshold, copy: copy)])
        default:
            return dash(copy)
        }
    }

    /// How the figure was arrived at: the inferred label first, then the basis.
    static func basisLines(_ card: InsightsPatternCard, copy: [String: String]) -> [String] {
        var lines: [String] = []
        if card.inferred { lines.append(text("analytics_inferred_from_order", copy)) }
        switch card.basis {
        case "estimate_from_result_size": lines.append(text("analytics_estimate_from_result_size", copy))
        case "from_counters": lines.append(text("analytics_from_counters", copy))
        default: break
        }
        return lines
    }

    /// "vs last week". Only feed T sends a figure; anything else is the dash.
    static func change(_ card: InsightsPatternCard, copy: [String: String]) -> String {
        guard let permille = card.change else { return dash(copy) }
        let percent = String((abs(permille) + 5) / 10)
        let key = permille < 0 ? "analytics_change_down" : "analytics_change_up"
        return InsightsOverviewWords.fill(text(key, copy), ["p": percent])
    }

    struct Bar: Identifiable, Equatable {
        let week: String
        /// `nil` is a gap: no bar is drawn, never a zero.
        let tokens: UInt64?
        var id: String { week }
    }

    /// Every week the card covers, oldest first, gaps included.
    static func bars(_ card: InsightsPatternCard) -> [Bar] {
        card.weeks.map { Bar(week: $0.week_start, tokens: $0.tokens) }
    }

    /// The bars to draw: weeks with a figure only.
    static func drawnBars(_ card: InsightsPatternCard) -> [Bar] {
        bars(card).filter { $0.tokens != nil }
    }

    static func seeSessions(_ card: InsightsPatternCard, copy: [String: String]) -> String {
        InsightsOverviewWords.fill(text("analytics_see_sessions", copy), ["n": String(card.sessions)])
    }

    /// "File B · .rs", or "File B" with no extension.
    static func fileLabel(_ row: InsightsRereadRow, copy: [String: String]) -> String {
        guard let ext = row.ext else {
            return InsightsOverviewWords.fill(text("analytics_file_label_no_ext", copy), ["letter": row.letter])
        }
        return InsightsOverviewWords.fill(text("analytics_file_label", copy), ["letter": row.letter, "ext": ext])
    }

    /// "Claude Code sessions only: {k} of {n}." when another harness is in
    /// the week; `nil` otherwise.
    static func claudeOnlyLine(_ patterns: InsightsWeekPatterns, copy: [String: String]) -> String? {
        guard patterns.claude_only else { return nil }
        return InsightsOverviewWords.fill(text("analytics_claude_sessions_only", copy), [
            "k": String(patterns.claude_sessions), "n": String(patterns.sessions),
        ])
    }
}

/// The Sessions tab (drill-in) over one saved snapshot (feed S). The
/// selection follows the saved list: the newest session until another is
/// picked, and the newest left when the picked one is deleted.
@Observable @MainActor
final class InsightsSessionsModel {
    private let service: InsightsModel.Service
    private var task: Task<Void, Never>?
    private var token = UUID()
    private var active = false
    private(set) var busy = false
    private(set) var failed = false
    private(set) var drill: InsightsSessionDrill?
    /// The saved snapshot on screen.
    private(set) var selected: String?

    init(service: @escaping InsightsModel.Service) { self.service = service }

    func open() { active = true; load() }
    func close() { active = false; token = UUID(); task?.cancel(); task = nil; busy = false }
    func reload() { load() }
    func select(_ snapshotID: String) { selected = snapshotID; load() }

    /// Keep the selection inside the saved list, newest first. Reads only
    /// when the selection changes.
    func sync(snapshotIDs: [String]) {
        if let selected, snapshotIDs.contains(selected) { return }
        guard let newest = snapshotIDs.first else {
            token = UUID(); task?.cancel(); task = nil
            selected = nil; drill = nil; failed = false; busy = false
            return
        }
        select(newest)
    }

    private func load() {
        guard active, let snapshotID = selected else { return }
        task?.cancel()
        token = UUID(); let current = token
        busy = true; failed = false
        let service = service
        let operation = InsightsRequest.Operation("session_drill", tz: InsightsOverviewModel.offset,
                                                  snapshotID: snapshotID)
        task = Task { [weak self] in
            do {
                let response = try await service(.init(operation: operation))
                guard let self, self.active, self.token == current, !Task.isCancelled else { return }
                guard response.type == operation.type, let drill = response.session,
                      drill.session_ref == snapshotID else { throw InsightsError.invalidResponse }
                self.drill = drill; self.busy = false
            } catch {
                guard let self, self.active, self.token == current, !Task.isCancelled else { return }
                // A failed read never keeps another session's figures.
                self.drill = nil; self.failed = true; self.busy = false
            }
        }
    }
}

/// The core's Sessions words, filled with the core's figures. Nothing here
/// composes a sentence.
enum InsightsSessionsWords {
    private static func text(_ key: String, _ copy: [String: String]) -> String { copy[key] ?? "" }
    private static func dash(_ copy: [String: String]) -> String { text("analytics_unavailable", copy) }

    /// The local date of the first event, in the user's locale.
    static func date(_ wire: String?, copy: [String: String]) -> String {
        let parse = Date.ISO8601FormatStyle().year().month().day()
        guard let wire, let day = try? Date(wire, strategy: parse) else { return dash(copy) }
        let style = Date.FormatStyle(timeZone: TimeZone(secondsFromGMT: 0) ?? .current).month(.abbreviated).day()
        return day.formatted(style)
    }

    /// Hours and minutes between the first and last event.
    static func span(_ seconds: UInt64?, copy: [String: String]) -> String {
        guard let seconds else { return dash(copy) }
        return Duration.seconds(Int64(clamping: seconds))
            .formatted(.units(allowed: [.hours, .minutes], width: .abbreviated))
    }

    static func header(_ drill: InsightsSessionDrill, copy: [String: String]) -> String {
        InsightsOverviewWords.fill(text("analytics_session_header", copy), [
            "date": date(drill.date, copy: copy),
            "harness": InsightsOverviewWords.harness(drill.source, copy: copy),
            "n": drill.turns.map(String.init) ?? dash(copy),
            "t": InsightsOverviewWords.figure(drill.tokens, copy: copy),
            "span": span(drill.span_secs, copy: copy),
        ])
    }

    /// Why there is no chart; `nil` when there is one.
    static func unavailableLine(_ drill: InsightsSessionDrill, copy: [String: String]) -> String? {
        switch drill.series_unavailable {
        case nil: return nil
        case "not_recorded": return text("analytics_codex_not_recorded", copy)
        case let reason?: return InsightsOverviewWords.reason(reason, copy: copy)
        }
    }

    static func turnLabel(_ ordinal: UInt32, copy: [String: String]) -> String {
        InsightsOverviewWords.fill(text("analytics_turn", copy), ["n": String(ordinal)])
    }

    struct Segment: Identifiable, Equatable {
        let turn: UInt32
        let series: String
        let tokens: UInt64
        var id: String { String(turn) + "/" + series }
    }

    /// The stacked bars, cache read at the bottom, then uncached, then cache
    /// write. A turn with an unknown counter draws nothing, never a zero bar.
    static func segments(_ drill: InsightsSessionDrill, copy: [String: String]) -> [Segment] {
        (drill.series ?? []).flatMap { turn -> [Segment] in
            guard let read = turn.cache_read, let uncached = turn.uncached, let write = turn.cache_write else {
                return []
            }
            return [
                Segment(turn: turn.ordinal, series: text("analytics_series_cache_read", copy), tokens: UInt64(read)),
                Segment(turn: turn.ordinal, series: text("analytics_series_uncached", copy), tokens: UInt64(uncached)),
                Segment(turn: turn.ordinal, series: text("analytics_series_cache_write", copy), tokens: write),
            ]
        }
    }

    /// A marker card: its title, its detail, a re-read's file label, and
    /// last the derivation label.
    static func markerLines(_ marker: InsightsDrillMarker, threshold: UInt64,
                            copy: [String: String]) -> [String] {
        let fill = InsightsOverviewWords.fill
        let figure = { (value: UInt64?) in InsightsOverviewWords.figure(value, copy: copy) }
        var lines: [String]
        switch marker.kind {
        case "cache_written_again":
            lines = [
                fill(text("analytics_marker_cache_rewrite", copy),
                     ["m": marker.pause_minutes.map(String.init) ?? dash(copy)]),
                fill(text("analytics_marker_cache_rewrite_detail", copy),
                     ["t": String(marker.turn_ordinal), "x": figure(marker.cache_write)]),
            ]
        case "context_shrank":
            lines = [fill(text("analytics_marker_shrank", copy), ["t": String(marker.turn_ordinal)])]
        case "crossed_long_context":
            lines = [fill(text("analytics_marker_crossed", copy), ["threshold": figure(threshold)]),
                     text("analytics_marker_crossed_detail", copy)]
        case "re_read":
            let letter = marker.file_letter ?? dash(copy)
            let label = marker.file_ext.map {
                fill(text("analytics_file_label", copy), ["letter": letter, "ext": $0])
            } ?? fill(text("analytics_file_label_no_ext", copy), ["letter": letter])
            lines = [fill(text("analytics_marker_reread", copy), ["letter": letter]),
                     text("analytics_marker_reread_detail", copy), label]
        default:
            return [dash(copy)]
        }
        switch marker.basis {
        case "inferred_from_counters": lines.append(text("analytics_marker_inferred", copy))
        case "from_counters": lines.append(text("analytics_from_counters", copy))
        case "from_tool_calls": lines.append(text("analytics_marker_from_tool_calls", copy))
        default: break
        }
        return lines
    }
}

/// The daemon's week (feed T) in the core's own shapes: the same `TCBridge`
/// types the saved week decodes into, so one view draws either feed.
extension DaemonData.InsightsWeek {
    private func core<T: Decodable>(_ part: DaemonData.CoreJSON?, as type: T.Type) -> T? {
        part.flatMap { try? JSONDecoder().decode(T.self, from: $0.data) }
    }
    var coreOverview: InsightsWeekOverview? { core(overview, as: InsightsWeekOverview.self) }
    var corePatterns: InsightsWeekPatterns? { core(patterns, as: InsightsWeekPatterns.self) }
    /// The kept weeks, passed to the core's `comparisons` unchanged.
    var coreHistory: [InsightsWeekFigures]? { core(history, as: [InsightsWeekFigures].self) }
}

/// Goals, the lever of the week and the weekly summary card (feed T only).
/// The core marks the goals it stores against the daemon's weekly figures,
/// which this model passes through unchanged; nothing is compared here.
@Observable @MainActor
final class InsightsComparisonsModel {
    private let service: InsightsModel.Service
    private var task: Task<Void, Never>?
    private var token = UUID()
    private(set) var busy = false
    private(set) var failed = false
    private(set) var comparisons: InsightsComparisons?
    private var counterWeeks: [InsightsWeekFigures]?
    private var weekStart: String?
    private var recapCardEnabled = false

    init(service: @escaping InsightsModel.Service) { self.service = service }

    /// Read for the daemon's week; `nil` weeks (feed S) clears everything,
    /// because nothing is compared under feed S.
    func load(counterWeeks: [InsightsWeekFigures]?, weekStart: String?, recapCardEnabled: Bool) {
        self.counterWeeks = counterWeeks; self.weekStart = weekStart
        self.recapCardEnabled = recapCardEnabled
        guard counterWeeks != nil else { clear(); return }
        reload()
    }

    func clear() {
        token = UUID(); task?.cancel(); task = nil
        comparisons = nil; busy = false; failed = false
    }

    func reload() {
        guard let counterWeeks else { return }
        run(.init("comparisons", weekStart: weekStart, tz: InsightsOverviewModel.offset,
                  counterWeeks: counterWeeks, recapCardEnabled: recapCardEnabled))
    }

    func addGoal(_ goal: InsightsGoal) { write(.init("goal_set", goal: goal)) }
    func deleteGoal(_ id: String) { write(.init("goal_delete", id: id)) }

    /// "Not useful" on the lever shown, for the week shown.
    func notUseful() {
        guard let found = comparisons, let pick = found.lever.pick else { return }
        write(.init("lever_feedback", weekStart: found.week_start, kind: pick.kind, action: "not_useful"))
    }

    /// "Open recap": the card is not shown again. Returns the closed week to
    /// put on screen.
    func openRecap() -> String? {
        guard let week = comparisons?.recap?.week_start else { return nil }
        write(.init("recap_opened", weekStart: week))
        return week
    }

    private func write(_ operation: InsightsRequest.Operation) {
        guard counterWeeks != nil else { return }
        let service = service
        let current = UUID(); token = current
        task?.cancel(); busy = true
        task = Task { [weak self] in
            do {
                let response = try await service(.init(operation: operation))
                guard let self, self.token == current, !Task.isCancelled else { return }
                guard response.type == operation.type, response.state != nil else {
                    throw InsightsError.invalidResponse
                }
                self.busy = false
                self.reload()
            } catch {
                guard let self, self.token == current, !Task.isCancelled else { return }
                self.busy = false; self.failed = true
            }
        }
    }

    private func run(_ operation: InsightsRequest.Operation) {
        let service = service
        let current = UUID(); token = current
        task?.cancel(); busy = true; failed = false
        task = Task { [weak self] in
            do {
                let response = try await service(.init(operation: operation))
                guard let self, self.token == current, !Task.isCancelled else { return }
                guard response.type == operation.type, let found = response.comparisons else {
                    throw InsightsError.invalidResponse
                }
                self.comparisons = found; self.busy = false
            } catch {
                guard let self, self.token == current, !Task.isCancelled else { return }
                // A failed read never keeps an earlier week's marks.
                self.comparisons = nil; self.failed = true; self.busy = false
            }
        }
    }
}

/// The core's goal, lever and summary-card words, filled with the core's
/// figures. Nothing here composes a sentence or compares a figure.
enum InsightsComparisonsWords {
    private static func text(_ key: String, _ copy: [String: String]) -> String { copy[key] ?? "" }
    private static func dash(_ copy: [String: String]) -> String { text("analytics_unavailable", copy) }
    private static let fill = InsightsOverviewWords.fill

    /// The goal kinds, in the core's order, and their words.
    static let goalKinds = ["cache_share_at_least", "repeated_reads_under", "long_context_under", "weekly_tokens_under"]
    private static let goalKeys = [
        "cache_share_at_least": "analytics_goal_cache_share",
        "repeated_reads_under": "analytics_goal_repeated_reads",
        "long_context_under": "analytics_goal_long_context",
        "weekly_tokens_under": "analytics_goal_weekly_tokens",
    ]

    /// Whether a kind names one harness.
    static func goalNeedsSource(_ kind: String) -> Bool {
        kind == "cache_share_at_least" || kind == "weekly_tokens_under"
    }

    /// The goal's line, its harness first when it names one.
    static func goal(_ goal: InsightsGoal, copy: [String: String]) -> String {
        let line = fill(text(goalKeys[goal.kind] ?? "", copy), [
            "p": goal.permille.map(InsightsOverviewWords.percent) ?? dash(copy),
            "t": InsightsOverviewWords.figure(goal.tokens, copy: copy),
        ])
        guard let source = goal.source else { return line }
        return InsightsOverviewWords.harness(source, copy: copy) + " \u{b7} " + line
    }

    /// A goal from what the user typed: a whole percent for a share, a token
    /// count otherwise. `nil` when the number is not one.
    static func newGoal(kind: String, source: String, number: String) -> InsightsGoal? {
        guard let value = UInt64(number.trimmingCharacters(in: .whitespaces)), value > 0 else { return nil }
        let harness = goalNeedsSource(kind) ? source : nil
        if kind == "cache_share_at_least" {
            guard value <= 100 else { return nil }
            return InsightsGoal(kind: kind, source: harness, permille: value * 10)
        }
        return InsightsGoal(kind: kind, source: harness, tokens: value)
    }

    /// One weekly mark: met, not met, or the dash for a week with no figure.
    static func mark(_ wire: String, copy: [String: String]) -> String {
        switch wire {
        case "met": return text("analytics_goal_met", copy)
        case "not_met": return text("analytics_goal_not_met", copy)
        default: return dash(copy)
        }
    }

    /// The figure the goal is judged on, in its own unit.
    static func goalFigure(_ view: InsightsGoalView, copy: [String: String]) -> String {
        guard let figure = view.figure else { return dash(copy) }
        return view.goal.kind == "cache_share_at_least"
            ? InsightsOverviewWords.percent(figure) + "%" : InsightsOverviewWords.figure(figure, copy: copy)
    }

    /// "Down from {x} last week." / "Up from {x} last week."; `nil` when the
    /// same, or when either week is not comparable.
    static func goalChange(_ view: InsightsGoalView, copy: [String: String]) -> String? {
        guard let change = view.marks?.change else { return nil }
        let from = view.goal.kind == "cache_share_at_least"
            ? InsightsOverviewWords.percent(change.from) + "%" : InsightsOverviewWords.figure(change.from, copy: copy)
        switch change.direction {
        case "down": return fill(text("analytics_goal_down_from", copy), ["x": from])
        case "up": return fill(text("analytics_goal_up_from", copy), ["x": from])
        default: return nil
        }
    }

    /// The lever's observation. Repeated reads have their own line; any
    /// other kind shows its card's name and figure. No advice (owner
    /// decision D2, open).
    static func leverLines(_ pick: InsightsLeverView, copy: [String: String]) -> [String] {
        let tokens = InsightsOverviewWords.figure(pick.tokens, copy: copy)
        if pick.kind == "repeated_reads" {
            return [fill(text("analytics_lever_line", copy), [
                "f": pick.files.map(String.init) ?? dash(copy),
                "r": pick.count.map(String.init) ?? dash(copy),
                "t": tokens,
            ])]
        }
        return [InsightsPatternsWords.title(pick.kind, copy: copy), tokens]
    }

    /// The summary card's title.
    static func recapTitle(_ recap: InsightsRecap, copy: [String: String]) -> String {
        fill(text("analytics_your_week", copy), [
            "range": InsightsOverviewWords.weekRange(start: recap.week_start, end: recap.week_end),
        ])
    }

    /// "{n} sessions · {p}% fewer|more tokens than last week", per harness;
    /// `nil` when last week cannot be compared.
    static func recapChange(_ source: InsightsRecapSource, sessions: UInt32, copy: [String: String]) -> String? {
        guard let permille = source.change_permille else { return nil }
        let key = permille < 0 ? "analytics_recap_fewer" : "analytics_recap_more"
        return fill(text(key, copy), ["n": String(sessions), "p": InsightsOverviewWords.percent(permille.magnitude)])
    }

    /// One item's line, by its rule; `nil` for a kind this build does not know.
    static func recapItem(_ item: InsightsRecapItem, copy: [String: String]) -> String? {
        switch item.kind {
        case "best_cache_week":
            guard let now = item.permille, let before = item.previous_best_permille else { return nil }
            return fill(text("analytics_recap_best_cache", copy), [
                "p": InsightsOverviewWords.percent(now), "q": InsightsOverviewWords.percent(before),
            ])
        case "goal":
            guard let goal = item.goal, let met = item.met else { return nil }
            return fill(text(met ? "analytics_recap_goal_met" : "analytics_recap_goal_not_met", copy),
                        ["goal": Self.goal(goal, copy: copy)])
        case "pattern_up":
            guard let pattern = item.pattern, let up = item.up_permille else { return nil }
            return fill(text("analytics_recap_pattern_up", copy), [
                "pattern": InsightsPatternsWords.title(pattern, copy: copy), "p": InsightsOverviewWords.percent(up),
            ])
        default:
            return nil
        }
    }

    /// "{n} sessions passed your {threshold} threshold.", only when the user
    /// has set a threshold.
    static func recapThreshold(_ recap: InsightsRecap, copy: [String: String]) -> String? {
        guard let past = recap.past_threshold else { return nil }
        return fill(text("analytics_recap_threshold", copy), [
            "n": String(past.sessions), "threshold": InsightsOverviewWords.figure(past.threshold, copy: copy),
        ])
    }
}
