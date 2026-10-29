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
    /// Asks the daemon to analyze and save one watched trace by its
    /// Sessions reference (`insights_analyze_session`). `nil` without a
    /// daemon, and for a custom store, which the daemon never writes.
    typealias SessionAnalyzer = @Sendable (String) async throws -> DaemonData.InsightsAnalyzeSessionAnswer
    private let sessionAnalyzer: SessionAnalyzer?
    /// A watched trace can be analyzed and saved from its Sessions row.
    var canAnalyzeSessions: Bool { sessionAnalyzer != nil }
    /// The watched trace being analyzed and saved, from the moment it is
    /// asked for until the daemon answers; `nil` otherwise.
    private(set) var analyzingSession: String?
    /// The watched trace whose last Analyze & save failed, with the core's
    /// copy key that says why; cleared by the next one asked for.
    private(set) var sessionAnalyzeFailure: (reference: String, key: String)?
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
    }, weekReader: WeekReader? = nil, sessionAnalyzer: SessionAnalyzer? = nil) {
        self.service = service
        self.weekReader = weekReader
        self.sessionAnalyzer = sessionAnalyzer
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
        analyzingSession = nil; sessionAnalyzeFailure = nil
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
    /// Analyze & save on a watched trace's Sessions row (Insights PM,
    /// 2026-10-21): the daemon reads the file its watcher found, so the
    /// person never picks one, and saves the snapshot. The model is busy
    /// from the moment it is asked; on success the saved snapshot is the
    /// detail on screen and the saved list, summary and episodes are read
    /// again, as after any save. `true` when the snapshot is on screen, so
    /// the caller can open Analyze; a failure leaves the detail as it was
    /// and names the row with the core's line.
    @discardableResult
    func analyzeAndSave(session reference: String) async -> Bool {
        guard active, !busy, analyzingSession == nil, let sessionAnalyzer else { return false }
        busy = true; error = nil; analyzingSession = reference; sessionAnalyzeFailure = nil
        let token = generation
        let outcome: Result<InsightsResponse, Error>
        do {
            let answer = try await sessionAnalyzer(reference)
            guard answer.enabled, answer.readable == true, let analysis = answer.analysis else {
                throw InsightsError.invalidResponse
            }
            outcome = .success(try TCInsights.analysis(analysis.data))
        } catch { outcome = .failure(error) }
        guard active, generation == token else { return false }
        analyzingSession = nil
        guard case .success(let response) = outcome, response.type == "analyze",
              let insight = response.insight else {
            busy = false
            sessionAnalyzeFailure = (reference, Self.sessionAnalyzeKey(outcome))
            return false
        }
        selected = insight; selectedIsSaved = true; selectedFile = nil
        snapshots.removeAll { $0.id == insight.id }
        snapshots.insert(insight, at: 0)
        invalidateCards()
        invalidatedEpisodeIDs = response.invalidatedEpisodeIDs
        recordComparisonEffects(response)
        if let episodeID = episodeDetail?.episode.id, response.invalidatedEpisodeIDs.contains(episodeID) {
            closeEpisode()
        }
        refreshEpisodesAfterSnapshotChain = true
        busy = false
        perform(.init("list"), preservingMutationEffects: true)
        return true
    }
    /// The line for a row no longer listed under the reference asked for.
    static let sessionUnknownKey = "analytics_analyze_unknown"

    /// The core's line for a failed Analyze & save: the daemon's fixed
    /// labels for a file that has gone, a row that is no longer listed, a
    /// file over the core's size limit and a Codex file without exactly one
    /// session, and one line for everything else.
    static func sessionAnalyzeKey(_ outcome: Result<InsightsResponse, Error>) -> String {
        guard case .failure(let error) = outcome, case .daemon(_, let message) = error as? DaemonDataError else {
            return "analytics_analyze_failed"
        }
        switch message {
        case "insights-session-file-gone": return "analytics_analyze_file_gone"
        case "insights-session-unknown": return sessionUnknownKey
        case "insights-session-too-large": return "analytics_analyze_too_large"
        case "insights-session-not-one-session": return "analytics_analyze_codex_not_one"
        default: return "analytics_analyze_failed"
        }
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
    /// The open figure's drill-down: tokens or cache share.
    private(set) var inputs: InsightsCardInputs?
    /// The Sessions figure's breakdown under feed S, read with every week
    /// and always shown under the Sessions card (owner, 2026-10-22: never
    /// behind a chevron). Nil while unread, after a failed read, and under
    /// feed T.
    private(set) var sessionsInputs: InsightsCardInputs?
    private var sessionsTask: Task<Void, Never>?
    /// Feed T's counted sessions, the Sessions figure's breakdown while the
    /// daemon's week shows: the daemon's rows, not the core's, since they
    /// carry no reference the core could look up.
    private(set) var counterSessions: InsightsCounterSessions?
    /// The week asked for; `nil` is the current week.
    private(set) var requestedWeek: String?

    init(service: @escaping InsightsModel.Service) { self.service = service }

    /// The shell's current UTC offset, which buckets the week's days.
    static var offset: Int32 { Int32(TimeZone.current.secondsFromGMT()) }

    /// The week on screen: feed T when the daemon sent one, feed S otherwise.
    var shown: InsightsWeekOverview? { counter ?? overview }

    /// Show the daemon's week and its counted sessions, or `nil` to go back
    /// to the saved week. Any open drill-down closes either way.
    func showCounter(_ week: InsightsWeekOverview?, sessions: InsightsCounterSessions? = nil) {
        counter = week; counterSessions = week == nil ? nil : sessions
        inputs = nil
    }

    /// Whether the tokens or cache share figure opens a drill-down in the
    /// week on screen. Feed T rows carry no session reference the core
    /// could look up, so under feed T neither does.
    func drills(_ card: String) -> Bool { counter == nil && card != "sessions" }

    /// The figure whose drill-down is open.
    var openCard: String? { inputs?.card }

    func open() { active = true; load() }
    func close() {
        active = false; token = UUID(); task?.cancel(); task = nil; busy = false
        sessionsTask?.cancel(); sessionsTask = nil
        inputs = nil; sessionsInputs = nil
    }
    func reload() { load() }
    func selectWeek(_ weekStart: String) { requestedWeek = weekStart; load() }

    /// Read the drill-down for `card` (`tokens` or `cache_share`) in the
    /// saved week on screen.
    func showInputs(_ card: String) {
        guard active, counter == nil, card != "sessions", let week = overview?.week_start else { return }
        run(.init("card_inputs", weekStart: week, tz: Self.offset, card: card)) { model, response in
            guard let inputs = response.inputs, inputs.card == card else { throw InsightsError.invalidResponse }
            model.inputs = inputs
        }
    }
    func hideInputs() { inputs = nil }

    private func load() {
        guard active else { return }
        inputs = nil; sessionsInputs = nil
        sessionsTask?.cancel(); sessionsTask = nil
        run(.init("week_overview", weekStart: requestedWeek, tz: Self.offset)) { model, response in
            guard let overview = response.overview else { throw InsightsError.invalidResponse }
            model.overview = overview
            model.readSessions(week: overview.week_start)
        }
    }

    /// Read the Sessions figure's breakdown for `week`, on its own task so
    /// a figure's drill-down never cancels it. A failed read draws none.
    private func readSessions(week: String) {
        sessionsTask?.cancel()
        let service = service
        sessionsTask = Task { [weak self] in
            let response = try? await service(
                .init(operation: .init("card_inputs", weekStart: week, tz: Self.offset, card: "sessions")))
            guard let self, self.active, !Task.isCancelled, self.overview?.week_start == week else { return }
            let inputs = response?.type == "card_inputs" ? response?.inputs : nil
            self.sessionsInputs = inputs?.card == "sessions" ? inputs : nil
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

/// Feed T's counted sessions for the Sessions card's drill-down: the
/// daemon's rows in its order, and whether their route column is drawn.
/// Local counter references identify the displayed sessions. Row identity
/// retains its place for compatibility with older daemon payloads.
struct InsightsCounterSessions: Equatable, Sendable {
    struct Row: Identifiable, Equatable, Sendable {
        let id: Int
        let session: DaemonData.InsightsWeekSession
    }

    let rows: [Row]
    let column: InsightsRouteWords.Column

    /// `nil` for an answer with no rollup.
    init?(week: DaemonData.InsightsWeek) {
        guard let sessions = week.rollup?.sessions else { return nil }
        self.init(sessions: sessions, column: InsightsRouteWords.column(week))
    }

    init(sessions: [DaemonData.InsightsWeekSession], column: InsightsRouteWords.Column) {
        rows = sessions.enumerated().map { Row(id: $0.offset, session: $0.element) }
        self.column = column
    }
}

/// One day picked on "Tokens by day": its place in the week, Monday first,
/// as each session's `by_day` counts it, and its weekday for the title.
struct InsightsDayPick: Equatable {
    let index: Int
    let weekday: String

    /// The day dated `date` among the week's `days`; `nil` when none is.
    /// The weekday is in `locale`, the person's own outside the tests.
    static func pick(_ date: String?, days: [InsightsDayTokens]?,
                     locale: Locale = .autoupdatingCurrent) -> InsightsDayPick? {
        guard let date, let index = days?.firstIndex(where: { $0.date == date }) else { return nil }
        let parse = Date.ISO8601FormatStyle().year().month().day()
        let wide = Date.FormatStyle(locale: locale, calendar: Calendar(identifier: .gregorian),
                                    timeZone: TimeZone(secondsFromGMT: 0) ?? .current).weekday(.wide)
        return InsightsDayPick(index: index, weekday: (try? Date(date, strategy: parse))?.formatted(wide) ?? date)
    }
}

/// The Sessions breakdown for a picked day: the sessions with tokens that
/// day, most first, each with that day's tokens from its own `by_day`. A
/// session with no split (Codex, or an unknown figure) is in no day.
enum InsightsDayBreakdown {
    struct Row<Session: Identifiable>: Identifiable {
        let session: Session
        let tokens: UInt64
        var id: Session.ID { session.id }
    }

    /// `nil` when no session carries a split, as from a daemon or core
    /// that predates it: the week shows, never an empty day.
    static func rows<Session: Identifiable>(_ sessions: [Session], day: Int,
                                            split: (Session) -> [UInt64]?) -> [Row<Session>]? {
        guard sessions.contains(where: { split($0) != nil }) else { return nil }
        let counted = sessions.enumerated().compactMap { offset, session -> (offset: Int, row: Row<Session>)? in
            guard let days = split(session), days.indices.contains(day), days[day] > 0 else { return nil }
            return (offset, Row(session: session, tokens: days[day]))
        }
        // Ties keep the breakdown's own order.
        return counted.sorted { $0.row.tokens != $1.row.tokens ? $0.row.tokens > $1.row.tokens : $0.offset < $1.offset }
            .map(\.row)
    }

    /// A feed T row's split, in the core's unsigned figures.
    static func split(_ session: DaemonData.InsightsWeekSession) -> [UInt64]? {
        session.byDay?.map { UInt64(max(0, $0)) }
    }
}

/// Where a counted session's calls went, in the core's words. Every label
/// comes from the proxy's proof, never from a backend's name; only
/// verified proof reads as verified, and no proxy record is never "not
/// private". The tokens here are the proxy's per-call count, drawn beside
/// the transcript's and never added to it.
enum InsightsRouteWords {
    /// The Private AI column: drawn, replaced by the feed-off line or the
    /// no-ledger line, or left out with nothing said.
    enum Column: Equatable, Sendable { case shown, feedOff, noLedger, hidden }

    private static func text(_ key: String, _ copy: [String: String]) -> String { copy[key] ?? "" }
    private static func dash(_ copy: [String: String]) -> String { text("analytics_unavailable", copy) }

    /// Drawn only when the daemon says the rows carry a route. The feed
    /// turned off in Settings and no proxy ledger each get their line; an
    /// older daemon or a reason this build does not know draw nothing.
    static func column(_ week: DaemonData.InsightsWeek) -> Column {
        if week.routingAvailable == true { return .shown }
        switch week.routingUnavailable {
        case "ledger_feed_off": return .feedOff
        case "no_ledger": return .noLedger
        default: return .hidden
        }
    }

    /// The one line shown in place of the column: the feed is off, or no
    /// Private AI call is recorded yet. `nil` when the column is drawn or
    /// left out with nothing said.
    static func unavailableLine(_ column: Column, copy: [String: String]) -> String? {
        switch column {
        case .feedOff: return text("analytics_route_feed_off", copy)
        case .noLedger: return text("analytics_route_no_ledger", copy)
        case .shown, .hidden: return nil
        }
    }

    /// The category's word; the dash for a missing route or a category this
    /// build does not know, so nothing is claimed for it.
    static func category(_ routing: DaemonData.InsightsWeekRouting?, copy: [String: String]) -> String {
        let key: String
        switch routing?.category {
        case "unobserved": key = "analytics_route_unobserved"
        case "unrecorded": key = "analytics_route_unrecorded"
        case "outside": key = "analytics_route_outside"
        case "mixed": key = "analytics_route_mixed"
        case "check_failed": key = "analytics_route_check_failed"
        case "routed_verified": key = "analytics_route_verified"
        case "routed_unverified": key = "analytics_route_unverified"
        default: return dash(copy)
        }
        return text(key, copy)
    }

    /// The route's qualifiers that have words; one this build does not
    /// know is left out rather than shown as its wire label.
    static func reasons(_ routing: DaemonData.InsightsWeekRouting?, copy: [String: String]) -> [String] {
        (routing?.reasons ?? []).compactMap { reason in
            reason == "some_calls_unrecorded" ? text("analytics_reason_" + reason, copy) : nil
        }
    }

    /// The split in proxy tokens, each of the five buckets the daemon's
    /// figure or the dash, the failed-check one included. `nil` only with
    /// no figures (no proxy record).
    static func split(_ routing: DaemonData.InsightsWeekRouting?, copy: [String: String]) -> String? {
        guard let tokens = routing?.tokens else { return nil }
        let figure = { (value: Int64?) in
            InsightsOverviewWords.figure(value.flatMap { UInt64(exactly: $0) }, copy: copy)
        }
        return InsightsOverviewWords.fill(text("analytics_route_split", copy), [
            "v": figure(tokens.verified),
            "u": figure(tokens.routedUnverified),
            "f": figure(tokens.checkFailed),
            "o": figure(tokens.outside),
            "n": figure(tokens.unrecorded),
        ])
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

    /// Whether the core says the card leads with its count: its estimate
    /// rounds to 0 while it found something (owner decision 2026-10-20).
    /// An older core, or a word this build does not know, leads with tokens.
    static func leadsWithCount(_ card: InsightsPatternCard) -> Bool {
        card.headline == "count" && card.count != nil
    }

    /// Each card's hole for its count in the headline words.
    private static let countHole = [
        "retried_calls": "c",
    ]

    /// The card's headline: the token figure, or the count with its unit
    /// ("2 reads", "1 call") when the core leads with the count.
    static func headline(_ card: InsightsPatternCard, copy: [String: String]) -> String {
        guard leadsWithCount(card), let found = card.count, let hole = countHole[card.kind] else {
            return InsightsOverviewWords.figure(card.tokens, copy: copy)
        }
        let key = "analytics_pattern_" + card.kind + (found == 1 ? "_headline_one" : "_headline_other")
        return InsightsOverviewWords.fill(text(key, copy), [hole: String(found)])
    }

    /// The line under the headline: the core's count sentence. Under a count headline the sentence
    /// without the count, so the count is not said twice.
    static func countLine(_ card: InsightsPatternCard, copy: [String: String]) -> String {
        let fill = InsightsOverviewWords.fill
        let underCount = leadsWithCount(card)
        switch card.kind {
        case "retried_calls" where underCount:
            return text("analytics_pattern_retried_calls_count_under_count", copy)
        case "retried_calls":
            return fill(text("analytics_pattern_retried_calls_count", copy), ["c": count(card.count, copy)])
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

    /// The figure above the current week's bar. A card that leads with its
    /// count draws none: its estimate rounds to 0 and a "0" would contradict
    /// the headline. `nil` also when the week has no figure.
    static func currentWeekFigure(_ card: InsightsPatternCard, copy: [String: String]) -> String? {
        guard !leadsWithCount(card), let tokens = bars(card).last?.tokens else { return nil }
        return InsightsOverviewWords.figure(tokens, copy: copy)
    }

    /// The bars to draw: weeks with a figure only.
    static func drawnBars(_ card: InsightsPatternCard) -> [Bar] {
        bars(card).filter { $0.tokens != nil }
    }

    static func seeSessions(_ card: InsightsPatternCard, copy: [String: String]) -> String {
        InsightsOverviewWords.fill(text("analytics_see_sessions", copy), ["n": String(card.sessions)])
    }

    /// The cards Breakdown shows. Repeated reads, edit after a failed
    /// command and long context left Insights (product owner, 2026-10-29).
    static let shownKinds: Set<String> = ["retried_calls"]

    static func shown(_ cards: [InsightsPatternCard]) -> [InsightsPatternCard] {
        cards.filter { shownKinds.contains($0.kind) }
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

/// The Sessions tab (drill-in). While the daemon's watched-folder counting
/// (feed T) shows, the tab is a feed of every watched trace, newest first,
/// read a page at a time as its last rows are drawn (owner request
/// 2026-10-20); a row expands its drill in place, one at a time, through the
/// daemon (`insights_sessions`, `insights_session_drill`; owner decision
/// 2026-10-20: feeds are never mixed). Otherwise it is one saved snapshot
/// (feed S), picked as before: the selection follows the saved list, the
/// newest session until another is picked, and the newest left when the
/// picked one is deleted.
@Observable @MainActor
final class InsightsSessionsModel {
    /// Reads one page of `insights_sessions` through the daemon client:
    /// `nil` is the first page, otherwise the `next` cursor of the page
    /// before.
    typealias WatchedReader = @Sendable (String?) async throws -> DaemonData.InsightsSessions
    /// Reads `insights_session_drill` for one listed reference.
    typealias WatchedDrillReader = @Sendable (String) async throws -> DaemonData.InsightsSessionDrillAnswer
    /// Writes `insights_session_label` for one listed reference: its task,
    /// then its outcome.
    typealias WatchedLabelWriter = @Sendable (String, DaemonData.InsightsLabelChange,
                                              DaemonData.InsightsLabelChange) async throws
        -> DaemonData.InsightsSessionLabelAnswer
    /// The most rows one page of the feed asks for.
    static let pageSize = 50
    private let service: InsightsModel.Service
    private let watchedReader: WatchedReader?
    private let watchedDrill: WatchedDrillReader?
    private let watchedLabel: WatchedLabelWriter?
    /// A row's own labels, as the daemon last said them or as just chosen.
    struct RowLabels: Equatable {
        var task: String?
        var outcome: String?
    }
    /// Label writes the daemon has not answered, by reference: the newest
    /// one's number, and the row's labels as the daemon last said them, to
    /// put back if that newest write fails.
    private var labelWrites: [String: (latest: Int, base: RowLabels)] = [:]
    private var labelCount = 0
    /// Each row's labels as last shown here (chosen, answered or put back),
    /// with the `labelClock` tick they were shown at. A list read that
    /// started before that tick, or while a write to the row is
    /// unanswered, is older than them, so they are put over its rows.
    private var labelsShown: [String: (labels: RowLabels, at: Int)] = [:]
    private var labelClock = 0
    /// The last label write asked for: each waits for the one before, so
    /// the daemon applies one row's writes in the order they were made.
    private var labelTail: Task<Void, Never>?
    /// Rows whose newest label could not be saved: each shows its labels as
    /// they were, with the core's line, until it is labelled again or the
    /// list no longer lists it.
    private(set) var labelFailed: Set<String> = []
    /// A label write is waiting for the daemon.
    var savingLabels: Bool { !labelWrites.isEmpty }
    /// The watched rows can be labelled: a daemon client is wired.
    var canLabel: Bool { watchedLabel != nil }
    /// Called each time the daemon saves a label, so what reads the labels
    /// (Details' NEAR AI spend by task) can read again.
    var labelSaved: (@MainActor () -> Void)?
    private var task: Task<Void, Never>?
    private var listTask: Task<Void, Never>?
    private var pageTask: Task<Void, Never>?
    private var token = UUID()
    private var listToken = UUID()
    private var active = false
    private var drillBusy = false
    private var listBusy = false
    /// The next page is being read.
    private(set) var loadingMore = false
    /// A drill, the watched list or its next page is being read.
    var busy: Bool { drillBusy || listBusy || loadingMore }
    private(set) var failed = false
    private(set) var drill: InsightsSessionDrill?
    /// The session on screen: a saved snapshot ID, or the expanded watched
    /// trace's opaque reference while `watched` is set (`nil`: none open).
    private(set) var selected: String?
    /// The watched traces read so far, newest first, only while feed T
    /// shows; `nil` is the saved snapshots.
    private(set) var watched: [DaemonData.InsightsWatchedSession]?
    /// The cursor for the feed's next page; `nil` at the end of the list.
    private var nextCursor: String?
    /// The last next-page read failed. The feed's last rows appearing again
    /// retry once on their own after a short wait; after that only
    /// `retry()` (the foot's control) or reading the list again asks.
    private(set) var pageFailed = false
    /// The automatic retry has been spent for this failure.
    private var autoRetried = false
    /// The store's `sessions_stored` and `revision` as the first page last
    /// said them (`updated_at` from a daemon without `revision`). A page
    /// that says otherwise means rows came or went since, and the loaded
    /// range is read again from the first page.
    private var stamp: String?
    private let retryBackoff: Duration
    /// Set when feed T is on but its traces could not be read, shown with
    /// the saved feed's line, as Overview shows it; `nil` when it is off or
    /// the daemon predates it.
    private(set) var noticeKey: String?
    /// The saved list as last synced, newest first, kept while watched
    /// traces show so going back needs no second read.
    private var savedIDs: [String] = []
    private var wantsWatched = false

    /// Every watched trace is listed: the last page said there is no next.
    var atEnd: Bool { watched != nil && nextCursor == nil }

    init(service: @escaping InsightsModel.Service,
         watchedReader: WatchedReader? = nil,
         watchedDrill: WatchedDrillReader? = nil,
         watchedLabel: WatchedLabelWriter? = nil,
         retryBackoff: Duration = .seconds(2)) {
        self.retryBackoff = retryBackoff
        self.service = service
        self.watchedReader = watchedReader
        self.watchedDrill = watchedDrill
        self.watchedLabel = watchedLabel
    }

    // MARK: The person's own labels

    /// Set a watched row's task to one of the five task words, or clear it
    /// (`nil`) so the inferred task shows again.
    func setTask(_ reference: String, to task: String?) {
        writeLabel(reference, task: task.map { .set($0) } ?? .clear, outcome: .keep)
    }

    /// Take the inferred task as the person's own: the one click that
    /// confirms it. Nothing when the row shows the person's task already or
    /// none was inferred.
    func confirmTask(_ reference: String) {
        guard let row = watched?.first(where: { $0.sessionRef == reference }),
              let task = InsightsSessionsWords.confirmableTask(row) else { return }
        setTask(reference, to: task)
    }

    /// Set a watched row's outcome to one of the three outcome words, or
    /// clear it (`nil`).
    func setOutcome(_ reference: String, to outcome: String?) {
        writeLabel(reference, task: .keep, outcome: outcome.map { .set($0) } ?? .clear)
    }

    /// Shows the change at once, then writes it, after any write asked for
    /// before it. The daemon's answer is the row as it now lists it; a
    /// write that fails puts the row's labels back as the daemon last said
    /// them. Only the newest write to a row decides what it shows; an
    /// older one's answer only moves what is put back.
    private func writeLabel(_ reference: String, task: DaemonData.InsightsLabelChange,
                            outcome: DaemonData.InsightsLabelChange) {
        guard let watchedLabel, let index = watched?.firstIndex(where: { $0.sessionRef == reference }),
              var row = watched?[index] else { return }
        let shown = RowLabels(task: row.userTask, outcome: row.userOutcome)
        labelCount += 1
        let number = labelCount
        labelWrites[reference] = (number, labelWrites[reference]?.base ?? shown)
        labelFailed.remove(reference)
        Self.apply(task, to: &row.userTask)
        Self.apply(outcome, to: &row.userOutcome)
        watched?[index] = row
        showLabels(of: row)
        let previous = labelTail
        labelTail = Task { [weak self] in
            await previous?.value
            let result: Result<DaemonData.InsightsSessionLabelAnswer, Error>
            do { result = .success(try await watchedLabel(reference, task, outcome)) } catch { result = .failure(error) }
            self?.answered(reference, number: number, result)
        }
    }

    private func answered(_ reference: String, number: Int,
                          _ outcome: Result<DaemonData.InsightsSessionLabelAnswer, Error>) {
        guard let pending = labelWrites[reference] else { return }
        var saved: DaemonData.InsightsWatchedSession?
        if case .success(let answer) = outcome, answer.enabled, answer.readable == true,
           let row = answer.session, row.sessionRef == reference {
            saved = row
        }
        if saved != nil { labelSaved?() }
        guard pending.latest == number else {
            // A newer write to this row is under way and decides what it
            // shows; this one only moves what a failure would put back.
            if let saved {
                labelWrites[reference]?.base = RowLabels(task: saved.userTask, outcome: saved.userOutcome)
            }
            return
        }
        labelWrites[reference] = nil
        if let saved {
            replaceRow(reference) { $0 = saved }
            showLabels(of: saved)
            return
        }
        replaceRow(reference) { row in
            row.userTask = pending.base.task
            row.userOutcome = pending.base.outcome
        }
        labelsShown[reference] = (pending.base, tickLabels())
        labelFailed.insert(reference)
        if case .failure(let error) = outcome, Self.isUnknownReference(error) {
            // Made again since its page was read (its file grew), or
            // dropped: read the loaded range again, as a drill does.
            refresh(dropping: nil)
        }
    }

    private func tickLabels() -> Int { labelClock += 1; return labelClock }

    private func showLabels(of row: DaemonData.InsightsWatchedSession) {
        labelsShown[row.sessionRef] = (RowLabels(task: row.userTask, outcome: row.userOutcome), tickLabels())
    }

    /// `rows` from a list read that started at label tick `since`, with the
    /// labels shown here since then, or still being written, put over
    /// them: a read under way while a label was saved would otherwise put
    /// the old label back.
    private func withShownLabels(_ rows: [DaemonData.InsightsWatchedSession],
                                 since: Int) -> [DaemonData.InsightsWatchedSession] {
        rows.map { row in
            guard let shown = labelsShown[row.sessionRef],
                  shown.at > since || labelWrites[row.sessionRef] != nil else { return row }
            var row = row
            row.userTask = shown.labels.task
            row.userOutcome = shown.labels.outcome
            return row
        }
    }

    /// After the list is replaced by a read that started at tick `since`:
    /// its rows are the daemon's word for every label shown before then,
    /// and a row it no longer lists keeps no mark.
    private func settleLabels(listed rows: [DaemonData.InsightsWatchedSession], since: Int) {
        let listed = Set(rows.map(\.sessionRef))
        labelsShown = labelsShown.filter { reference, shown in
            labelWrites[reference] != nil || (shown.at > since && listed.contains(reference))
        }
        labelFailed.formIntersection(listed)
    }

    private func replaceRow(_ reference: String, _ change: (inout DaemonData.InsightsWatchedSession) -> Void) {
        guard let index = watched?.firstIndex(where: { $0.sessionRef == reference }),
              var row = watched?[index] else { return }
        change(&row)
        watched?[index] = row
    }

    private static func apply(_ change: DaemonData.InsightsLabelChange, to slot: inout String?) {
        switch change {
        case .keep: break
        case .clear: slot = nil
        case .set(let word): slot = word
        }
    }

    func open() {
        active = true
        if wantsWatched { readWatched() } else { load() }
    }
    func close() {
        active = false; token = UUID(); listToken = UUID()
        task?.cancel(); task = nil; listTask?.cancel(); listTask = nil; pageTask?.cancel(); pageTask = nil
        drillBusy = false; listBusy = false; loadingMore = false
    }
    func reload() { if wantsWatched { readWatched() } else { load() } }
    /// Something else asked the daemon about a listed row and was told it
    /// no longer lists it (Analyze & save): read the loaded range again, as
    /// a drill or a label does.
    func rowNoLongerListed() { refresh(dropping: nil) }
    func select(_ reference: String) { selected = reference; load() }

    /// A watched row clicked: open it (closing any other), or close it when
    /// it is the open one.
    func toggle(_ reference: String) {
        guard selected == reference else { select(reference); return }
        collapse()
    }

    private func collapse() {
        token = UUID(); task?.cancel(); task = nil
        selected = nil; drill = nil; failed = false; drillBusy = false
    }

    /// Read the feed's next page, when there is one and none is being read.
    /// The tab asks as the last rows are drawn. After a failed read, the
    /// last rows drawn again retry once, after a short wait.
    func loadMore() {
        if pageFailed {
            guard !autoRetried else { return }
            // Spent only by a read that starts: asked while the list is
            // busy, it is kept for the next time the last rows appear.
            if readNext(after: retryBackoff) { autoRetried = true }
        } else {
            readNext(after: nil)
        }
    }

    /// The foot's control: read the page that failed again, from the same
    /// cursor, at once. The rows read so far are kept.
    func retry() {
        guard pageFailed else { return }
        readNext(after: nil)
    }

    /// Starts reading the next page; `false` when none was started (no
    /// next page, or a read already under way).
    @discardableResult
    private func readNext(after delay: Duration?) -> Bool {
        guard active, watched != nil, let cursor = nextCursor, !loadingMore, !listBusy,
              let watchedReader else { return false }
        let current = listToken
        let labelsFrom = labelClock
        loadingMore = true
        pageFailed = false
        pageTask = Task { [weak self] in
            if let delay { try? await Task.sleep(for: delay) }
            let outcome: Result<DaemonData.InsightsSessions, Error>
            if Task.isCancelled { return }
            do { outcome = .success(try await watchedReader(cursor)) } catch { outcome = .failure(error) }
            guard let self, self.active, self.listToken == current, !Task.isCancelled else { return }
            self.loadingMore = false
            switch outcome {
            case .success(let page) where page.showsCounterPass:
                let known = Set((self.watched ?? []).map(\.sessionRef))
                let fresh = self.withShownLabels((page.sessions ?? []).filter { !known.contains($0.sessionRef) },
                                                 since: labelsFrom)
                self.watched = (self.watched ?? []) + fresh
                self.nextCursor = page.next
                self.autoRetried = false
                if Self.stamp(page) != self.stamp {
                    // Rows came or went since the first page was read.
                    self.refresh(dropping: nil)
                } else if fresh.isEmpty, let next = page.next, next != cursor {
                    // Nothing new to draw, so no last row will appear
                    // again: ask for the following page now. Each read moves
                    // the cursor, so this ends.
                    self.readNext(after: nil)
                }
            case .success(let page):
                self.showSaved(notice: page.enabled)
            case .failure(let error) where Self.isCursorRefused(error):
                // The store started again: these references name other
                // rows now, so the list is read from the first page.
                self.readWatched()
            case .failure:
                // The rows read so far stay; the foot says the rest could
                // not be read and offers to try again.
                self.pageFailed = true
            }
        }
        return true
    }

    private static func stamp(_ page: DaemonData.InsightsSessions) -> String {
        let changed = page.revision.map(String.init) ?? page.updatedAt ?? ""
        return "\(page.sessionsStored.map(String.init) ?? "")|\(changed)"
    }

    /// Read the loaded range again from the first page, as many rows as
    /// are loaded, and put the answer in place of the list. The daemon's
    /// answer is the truth for that range: a row made again (its file grew)
    /// shows once, under its new reference and at its place, wherever it
    /// was in the list; a row gone is gone. `stale` is a row the daemon no
    /// longer knows: it is taken out at once, before the read. A read that
    /// fails leaves the list as it was; a refused cursor reads from the
    /// first page alone. A failed next page past the range stays failed.
    private func refresh(dropping stale: String?) {
        guard active, let watchedReader, let loaded = watched else { return }
        if let stale { watched?.removeAll { $0.sessionRef == stale } }
        let wanted = max(loaded.count, 1)
        listTask?.cancel(); pageTask?.cancel(); pageTask = nil; loadingMore = false
        listToken = UUID(); let current = listToken
        let labelsFrom = labelClock
        listBusy = true
        listTask = Task { [weak self] in
            var rows: [DaemonData.InsightsWatchedSession] = []
            var seen = Set<String>()
            var cursor: String?
            var first: DaemonData.InsightsSessions?
            var next: String?
            var outcome: Result<Void, Error> = .success(())
            // Each page moves the cursor; the bound only stops a daemon that
            // would not.
            for _ in 0..<1_000 {
                let page: DaemonData.InsightsSessions
                do { page = try await watchedReader(cursor) } catch { outcome = .failure(error); break }
                if first == nil { first = page }
                guard page.showsCounterPass else { break }
                for row in page.sessions ?? [] where seen.insert(row.sessionRef).inserted { rows.append(row) }
                next = page.next
                guard let following = page.next, following != cursor, rows.count < wanted,
                      !Task.isCancelled else { break }
                cursor = following
            }
            guard let self, self.active, self.listToken == current, !Task.isCancelled else { return }
            self.listBusy = false
            switch outcome {
            case .failure(let error) where Self.isCursorRefused(error):
                self.readWatched()
            case .failure:
                break
            case .success:
                guard let first, first.showsCounterPass else {
                    if let first { self.showSaved(notice: first.enabled) }
                    return
                }
                self.watched = self.withShownLabels(rows, since: labelsFrom)
                self.settleLabels(listed: rows, since: labelsFrom)
                self.nextCursor = next
                self.stamp = Self.stamp(first)
                if next == nil { self.pageFailed = false; self.autoRetried = false }
                if let selected = self.selected, !rows.contains(where: { $0.sessionRef == selected }) {
                    self.collapse()
                }
            }
        }
    }

    /// Keep the selection inside the saved list, newest first. Reads only
    /// when the selection changes, and nothing while watched traces show.
    func sync(snapshotIDs: [String]) {
        savedIDs = snapshotIDs
        guard watched == nil else { return }
        if let selected, snapshotIDs.contains(selected) { return }
        guard let newest = snapshotIDs.first else {
            token = UUID(); task?.cancel(); task = nil
            selected = nil; drill = nil; failed = false; drillBusy = false
            return
        }
        select(newest)
    }

    /// Show the watched traces while feed T shows (`true`), or go back to the
    /// saved snapshots (`false`). A watched list that cannot be read falls
    /// back to the saved snapshots, with a notice only when counting is on.
    func showWatched(_ on: Bool) {
        wantsWatched = on && watchedReader != nil
        if wantsWatched { readWatched() } else { showSaved(notice: false) }
    }

    private func showSaved(notice: Bool) {
        listToken = UUID(); listTask?.cancel(); listTask = nil; listBusy = false
        pageTask?.cancel(); pageTask = nil; loadingMore = false
        noticeKey = notice ? InsightsModel.FeedLineKey.counterPassUnavailable : nil
        if watched != nil {
            // A watched trace's figures never stay on the saved feed.
            watched = nil; nextCursor = nil; pageFailed = false; autoRetried = false; labelFailed = []
            token = UUID(); task?.cancel(); task = nil
            selected = nil; drill = nil; failed = false; drillBusy = false
        }
        sync(snapshotIDs: savedIDs)
    }

    /// The feed's first page, in place of every page read so far.
    private func readWatched() {
        guard active, let watchedReader else { return }
        listTask?.cancel(); pageTask?.cancel(); pageTask = nil; loadingMore = false
        listToken = UUID(); let current = listToken
        let labelsFrom = labelClock
        listBusy = true
        listTask = Task { [weak self] in
            let outcome: Result<DaemonData.InsightsSessions, Error>
            do { outcome = .success(try await watchedReader(nil)) } catch { outcome = .failure(error) }
            guard let self, self.active, self.listToken == current, !Task.isCancelled else { return }
            self.listBusy = false
            switch outcome {
            case .success(let list) where list.showsCounterPass:
                let rows = self.withShownLabels(list.sessions ?? [], since: labelsFrom)
                self.settleLabels(listed: rows, since: labelsFrom)
                let wasWatched = self.watched != nil
                self.watched = rows; self.nextCursor = list.next; self.pageFailed = false; self.autoRetried = false; self.stamp = Self.stamp(list); self.noticeKey = nil
                if !wasWatched {
                    // Coming from the saved feed: no row is open until one
                    // is clicked.
                    self.collapse()
                } else if let selected = self.selected, rows.contains(where: { $0.sessionRef == selected }) {
                    if self.drill?.session_ref != selected || self.drill?.feed != "counter_pass" { self.load() }
                } else {
                    self.collapse()
                }
            case .success(let list):
                self.showSaved(notice: list.enabled)
            case .failure(let error):
                self.showSaved(notice: !((error as? DaemonDataError)?.isNotServed ?? false))
            }
        }
    }

    private func load() {
        guard active, let reference = selected else { return }
        task?.cancel()
        token = UUID(); let current = token
        drillBusy = true; failed = false
        let watchedRead: WatchedDrillReader? = watched == nil ? nil : watchedDrill
        if watched != nil && watchedRead == nil { drillBusy = false; failed = true; drill = nil; return }
        let service = service
        let operation = InsightsRequest.Operation("session_drill", tz: InsightsOverviewModel.offset,
                                                  snapshotID: reference)
        task = Task { [weak self] in
            do {
                let drill: InsightsSessionDrill
                if let watchedRead {
                    let answer = try await watchedRead(reference)
                    guard answer.enabled, answer.readable == true, let core = answer.coreDrill,
                          core.feed == "counter_pass" else { throw InsightsError.invalidResponse }
                    drill = core
                } else {
                    let response = try await service(.init(operation: operation))
                    guard response.type == operation.type, let read = response.session
                    else { throw InsightsError.invalidResponse }
                    drill = read
                }
                guard let self, self.active, self.token == current, !Task.isCancelled else { return }
                guard drill.session_ref == reference else { throw InsightsError.invalidResponse }
                self.drill = drill; self.drillBusy = false
            } catch {
                guard let self, self.active, self.token == current, !Task.isCancelled else { return }
                if watchedRead != nil, Self.isUnknownReference(error) {
                    // Made again since its page was read (its file grew), or
                    // dropped: read the loaded range again, so the session
                    // is back at its place under its new reference and the
                    // rest of the list stays.
                    self.collapse()
                    self.refresh(dropping: reference)
                    return
                }
                // A failed read never keeps another session's figures.
                self.drill = nil; self.failed = true; self.drillBusy = false
            }
        }
    }

    /// The daemon refused the cursor: it was answered from another store
    /// (`insights-sessions-cursor-invalid`).
    private static func isCursorRefused(_ error: Error) -> Bool {
        if case .daemon(_, let message)? = error as? DaemonDataError {
            return message == "insights-sessions-cursor-invalid"
        }
        return false
    }

    /// The daemon no longer lists that reference (`insights-session-unknown`).
    private static func isUnknownReference(_ error: Error) -> Bool {
        if case .daemon(_, let message)? = error as? DaemonDataError {
            return message == "insights-session-unknown"
        }
        return false
    }
}

extension DaemonData.InsightsSessionDrillAnswer {
    /// The drill in the core's own shape, the type the saved drill decodes
    /// into, so one view draws either feed.
    var coreDrill: InsightsSessionDrill? {
        session.flatMap { try? JSONDecoder().decode(InsightsSessionDrill.self, from: $0.data) }
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

    /// A saved session's label: its first recorded event's local date and
    /// time, its harness and `tokens`, the core's figure when the caller
    /// holds one and the dash otherwise. Never the snapshot ID. The time is
    /// read in the fixed offset the app sends the core
    /// (`InsightsOverviewModel.offset`), so the date is the one the drill
    /// header names.
    static func label(_ insight: LocalInsight, tokens: UInt64? = nil, copy: [String: String],
                      timeZone: TimeZone = TimeZone(secondsFromGMT: TimeZone.current.secondsFromGMT()) ?? .current)
        -> String {
        label(first: firstEvent(insight), source: insight.source_format, tokens: tokens, copy: copy,
              timeZone: timeZone)
    }

    /// The same folder label and opaque short reference used by Sessions.
    /// Older daemon payloads omit both; do not invent a session identifier.
    static func counterTitle(_ row: DaemonData.InsightsWeekSession, copy: [String: String]) -> String {
        let folder = row.folder.flatMap { $0.isEmpty ? nil : $0 }
            ?? text("analytics_folder_unknown", copy)
        guard let reference = row.sessionRef, !reference.isEmpty else { return folder }
        return folder + " · " + reference
    }

    /// A counted session's label (feed T): its recorded activity range,
    /// harness and transcript tokens, beneath its folder and reference.
    /// `tokens`, when given, stands in for the week's (a picked day's).
    /// A missing or invalid last event retains the first-event-only label.
    static func counterLabel(_ row: DaemonData.InsightsWeekSession, tokens: UInt64? = nil, copy: [String: String],
                             timeZone: TimeZone = .current) -> String {
        let tokens = tokens ?? row.tokens.flatMap { UInt64(exactly: $0) }
        let first = row.startedAt.flatMap(parseEvent)
        if let first, let last = row.lastEventAt.flatMap(parseEvent), last > first,
           let template = copy["analytics_session_label_range"] {
            var calendar = Calendar(identifier: .gregorian)
            calendar.timeZone = timeZone
            let startStyle = Date.FormatStyle(timeZone: timeZone).month(.abbreviated).day().hour().minute()
            let end: String
            if calendar.isDate(first, inSameDayAs: last) {
                end = last.formatted(Date.FormatStyle(timeZone: timeZone).hour().minute())
            } else if calendar.component(.year, from: first) == calendar.component(.year, from: last) {
                end = last.formatted(startStyle)
            } else {
                end = last.formatted(startStyle.year())
            }
            return InsightsOverviewWords.fill(template, [
                "span": first.formatted(startStyle) + " – " + end,
                "harness": InsightsOverviewWords.harness(row.source, copy: copy),
                "t": InsightsOverviewWords.figure(tokens, copy: copy),
            ])
        }
        return label(first: first, source: row.source, tokens: tokens, copy: copy, timeZone: timeZone)
    }

    private static func label(first: Date?, source: String?, tokens: UInt64?, copy: [String: String],
                              timeZone: TimeZone) -> String {
        let harness = InsightsOverviewWords.harness(source, copy: copy)
        let figure = InsightsOverviewWords.figure(tokens, copy: copy)
        guard let first else {
            return InsightsOverviewWords.fill(text("analytics_session_label_undated", copy),
                                              ["harness": harness, "t": figure])
        }
        return InsightsOverviewWords.fill(text("analytics_session_label", copy), [
            "date": first.formatted(Date.FormatStyle(timeZone: timeZone).month(.abbreviated).day()),
            "time": first.formatted(Date.FormatStyle(timeZone: timeZone).hour().minute()),
            "harness": harness,
            "t": figure,
        ])
    }

    /// The first recorded event. The core writes it with or without
    /// fractional seconds; the tests pin that both read.
    private static func firstEvent(_ insight: LocalInsight) -> Date? {
        guard let wire = insight.time_evidence?.earliest?.recorded_at else { return nil }
        return parseEvent(wire)
    }

    private static func parseEvent(_ wire: String) -> Date? {
        try? Date(wire, strategy: Date.ISO8601FormatStyle(includingFractionalSeconds: true))
    }

    /// A watched trace's label (feed T, owner decision 2026-10-20): its
    /// folder's label, its first recorded event's local day and time, its
    /// harness and its turns. Codex records no turns, so its label has none;
    /// a folder no longer known reads the core's word. Never the reference.
    static func watchedLabel(_ row: DaemonData.InsightsWatchedSession, copy: [String: String],
                             timeZone: TimeZone = TimeZone(secondsFromGMT: TimeZone.current.secondsFromGMT())
                                ?? .current) -> String {
        let values = [
            "folder": row.folder ?? text("analytics_folder_unknown", copy),
            "harness": InsightsOverviewWords.harness(row.source, copy: copy),
        ]
        guard let first = row.startedAt.flatMap(parseEvent) else {
            return InsightsOverviewWords.fill(text("analytics_watched_label_undated", copy), values)
        }
        var filled = values
        filled["date"] = first.formatted(Date.FormatStyle(timeZone: timeZone)
            .weekday(.abbreviated).month(.abbreviated).day())
        filled["time"] = first.formatted(Date.FormatStyle(timeZone: timeZone).hour().minute())
        guard let turns = row.turns else {
            return InsightsOverviewWords.fill(text("analytics_watched_label_no_turns", copy), filled)
        }
        filled["n"] = String(turns)
        return InsightsOverviewWords.fill(text("analytics_watched_label", copy), filled)
    }

    /// A watched row's tokens in the core's words; a row with no counts
    /// says so once (the core's "no token counts" reason).
    static func rowTokens(_ row: DaemonData.InsightsWatchedSession, copy: [String: String]) -> String {
        guard let tokens = row.tokens else { return text("analytics_reason_no_usage_counters", copy) }
        return InsightsOverviewWords.fill(text("analytics_session_row_tokens", copy),
                                          ["t": InsightsOverviewWords.figure(tokens, copy: copy)])
    }

    /// The row's sparkline in words, for VoiceOver: its largest turn. `nil`
    /// when the row has no sparkline.
    static func sparklineSummary(_ row: DaemonData.InsightsWatchedSession, copy: [String: String]) -> String? {
        guard let sparkline = row.sparkline else { return nil }
        return InsightsOverviewWords.fill(text("analytics_sparkline_peak", copy), [
            "t": InsightsOverviewWords.figure(sparkline.peakTokens, copy: copy),
            "n": String(sparkline.peakTurn),
        ])
    }

    /// A watched row as VoiceOver reads it, one element: its label, its
    /// tokens and its sparkline in words. Never the reference.
    static func rowAccessibilityLabel(_ row: DaemonData.InsightsWatchedSession, copy: [String: String],
                                      timeZone: TimeZone = TimeZone(secondsFromGMT: TimeZone.current.secondsFromGMT())
                                         ?? .current) -> String {
        [watchedLabel(row, copy: copy, timeZone: timeZone), rowTokens(row, copy: copy),
         sparklineSummary(row, copy: copy)].compactMap { $0 }.joined(separator: ", ")
    }

    // MARK: A watched row's own labels

    /// The task words a row's task control offers, in the core's order
    /// (`session_labels::LabelledTask`). No "no edits": that is not a task.
    static let taskWords = ["bug_fix", "refactor", "tests", "docs", "other"]
    /// The outcome words a row's outcome control offers.
    static let outcomeWords = ["kept", "partly_kept", "thrown_away"]

    static func taskChoices(copy: [String: String]) -> [(value: String, title: String)] {
        taskWords.map { (value: $0, title: text("analytics_task_" + $0, copy)) }
    }

    static func outcomeChoices(copy: [String: String]) -> [(value: String, title: String)] {
        outcomeWords.map { (value: $0, title: text("analytics_outcome_" + $0, copy)) }
    }

    /// The task a row's control shows: the person's own, else the inferred
    /// one when it is a task word (`inferred: true`); `nil` when neither,
    /// as for a trace inferred as no edits.
    static func shownTask(_ row: DaemonData.InsightsWatchedSession) -> (word: String, inferred: Bool)? {
        if let task = row.userTask, taskWords.contains(task) { return (task, false) }
        if let task = row.inferredTask, taskWords.contains(task) { return (task, true) }
        return nil
    }

    /// The inferred task a click takes as the person's own; `nil` once the
    /// person has set one, or when none was inferred.
    static func confirmableTask(_ row: DaemonData.InsightsWatchedSession) -> String? {
        guard row.userTask == nil, let shown = shownTask(row), shown.inferred else { return nil }
        return shown.word
    }

    /// The task control's words: the person's task, the inferred one marked
    /// as inferred, or the control's name when there is neither.
    static func taskTitle(_ row: DaemonData.InsightsWatchedSession, copy: [String: String]) -> String {
        guard let shown = shownTask(row) else { return text("analytics_label_task", copy) }
        let word = text("analytics_task_" + shown.word, copy)
        guard shown.inferred else { return word }
        return InsightsOverviewWords.fill(text("analytics_task_inferred", copy), ["label": word])
    }

    /// The outcome control's words: the person's outcome, or the control's
    /// name while unset.
    static func outcomeTitle(_ row: DaemonData.InsightsWatchedSession, copy: [String: String]) -> String {
        guard let outcome = row.userOutcome, outcomeWords.contains(outcome) else {
            return text("analytics_label_outcome", copy)
        }
        return text("analytics_outcome_" + outcome, copy)
    }

    /// The row's controls as VoiceOver names them, each with the row's own
    /// label, so one row's controls are told from another's.
    static func labelAccessibility(_ key: String, _ row: DaemonData.InsightsWatchedSession,
                                   copy: [String: String],
                                   timeZone: TimeZone = TimeZone(secondsFromGMT: TimeZone.current.secondsFromGMT())
                                      ?? .current) -> String {
        InsightsOverviewWords.fill(text(key, copy), ["label": watchedLabel(row, copy: copy, timeZone: timeZone)])
    }

    /// The row the up (`-1`) or down (`1`) key moves to, held to the list's
    /// ends; with none focused (or one no longer listed), the first.
    static func moved(_ current: String?, by delta: Int, in references: [String]) -> String? {
        guard !references.isEmpty else { return nil }
        guard let current, let index = references.firstIndex(of: current) else { return references.first }
        return references[min(max(index + delta, 0), references.count - 1)]
    }

    /// Which feed the drill on screen is from, behind the header's "i": the
    /// watched-folder line, or the saved line with the unavailable notice
    /// when counting is on but could not be read, as Overview says it.
    static func feedLines(_ feed: String, notice: String?, copy: [String: String]) -> [String] {
        if feed == "counter_pass" { return [text("analytics_feed_counter_pass", copy)] }
        return [text("analytics_feed_saved", copy)] + (notice.map { [$0] } ?? [])
    }

    /// The session picker: each saved session by its label, valued by its ID.
    /// The shell holds no core token figure per saved session, so `{t}` is
    /// the dash here.
    static func pickerChoices(_ snapshots: [LocalInsight],
                              copy: [String: String]) -> [(value: String, title: String)] {
        snapshots.map { (value: $0.id, title: label($0, copy: copy)) }
    }

    /// A drill-down row's session: its saved snapshot's label with the row's
    /// own figure, or the dash when no saved snapshot has that ID.
    static func rowLabel(_ sessionRef: String, tokens: UInt64?, snapshots: [LocalInsight],
                         copy: [String: String]) -> String {
        guard let insight = snapshots.first(where: { $0.id == sessionRef }) else { return dash(copy) }
        return label(insight, tokens: tokens, copy: copy)
    }

    /// The reasons under the drill's title, in the core's words, less the
    /// one the body line already says in place of the chart.
    static func headerReasons(_ drill: InsightsSessionDrill, copy: [String: String]) -> [String] {
        drill.reasons.filter { $0 != drill.series_unavailable }.map { InsightsOverviewWords.reason($0, copy: copy) }
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

    /// The markers the drill shows. A turn that crossed the long-context
    /// line and a file read again left Insights (product owner, 2026-10-29).
    static let shownMarkerKinds: Set<String> = ["cache_written_again", "context_shrank"]

    static func shownMarkers(_ drill: InsightsSessionDrill) -> [InsightsDrillMarker] {
        drill.markers.filter { shownMarkerKinds.contains($0.kind) }
    }

    /// A marker card: its title, its detail, and last the derivation label.
    static func markerLines(_ marker: InsightsDrillMarker, copy: [String: String]) -> [String] {
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

/// The lever of the week and the weekly summary card (feed T only). The
/// core reads the daemon's weekly figures, which this model passes through
/// unchanged; nothing is compared here.
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

/// Goals: the goals set with `/goal` in Claude and Codex, as the daemon's
/// week answers them (product owner, 2026-10-29), in the core's words. Only
/// status, tokens, time and the folder: a goal's text never reaches the app.
enum InsightsGoalsWords {
    private static func text(_ key: String, _ copy: [String: String]) -> String { copy[key] ?? "" }
    private static func dash(_ copy: [String: String]) -> String { text("analytics_unavailable", copy) }

    /// The statuses the core has words for; any other reads "Status
    /// unavailable".
    static let statuses = [
        "met", "not_met", "open", "active", "paused", "blocked", "usage_limited", "budget_limited", "complete",
    ]

    /// "Goals met {n} of {m}".
    static func metOf(_ goals: DaemonData.InsightsWeekGoals, copy: [String: String]) -> String {
        InsightsOverviewWords.fill(text("analytics_goals_met_of", copy), [
            "n": String(goals.met), "m": String(goals.total),
        ])
    }

    /// "Claude" or "Codex".
    static func tool(_ tool: String, copy: [String: String]) -> String {
        let words = text("analytics_credit_tool_" + tool, copy)
        return words.isEmpty ? dash(copy) : words
    }

    /// The tool's own status, in the core's words.
    static func status(_ status: String, copy: [String: String]) -> String {
        statuses.contains(status) ? text("analytics_goal_" + status, copy) : text("analytics_goal_status_unknown", copy)
    }

    /// "{t} tokens · {span}"; an unknown figure is the dash, never 0.
    static func line(_ goal: DaemonData.InsightsWeekGoal, copy: [String: String]) -> String {
        InsightsOverviewWords.fill(text("analytics_goal_line", copy), [
            "t": InsightsOverviewWords.figure(goal.tokens.map { UInt64(clamping: $0) }, copy: copy),
            "span": InsightsSessionsWords.span(goal.timeSecs.map { UInt64(clamping: $0) }, copy: copy),
        ])
    }
}

/// The core's lever and summary-card words, filled with the core's
/// figures. Nothing here composes a sentence or compares a figure.
enum InsightsComparisonsWords {
    private static func text(_ key: String, _ copy: [String: String]) -> String { copy[key] ?? "" }
    private static func dash(_ copy: [String: String]) -> String { text("analytics_unavailable", copy) }
    private static let fill = InsightsOverviewWords.fill

    /// The lever's observation: its card's name and figure. No advice (owner
    /// decision D2, open).
    static func leverLines(_ pick: InsightsLeverView, copy: [String: String]) -> [String] {
        let tokens = InsightsOverviewWords.figure(pick.tokens, copy: copy)
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
        case "pattern_up":
            guard let pattern = item.pattern, InsightsPatternsWords.shownKinds.contains(pattern),
                  let up = item.up_permille else { return nil }
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
