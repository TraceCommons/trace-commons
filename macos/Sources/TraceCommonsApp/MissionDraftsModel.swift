import Foundation
import Observation
import TCBridge

@Observable @MainActor
final class MissionDraftsModel {
    typealias Service = @Sendable (MissionDraftRequest) async throws -> MissionDraftResponse

    struct DeleteConfirmation: Equatable {
        fileprivate let id: String
        fileprivate let presentation: UUID
    }

    private enum MutationEffect {
        case imported(inserted: Bool)
        case deleted(id: String)
    }

    private let service: Service
    private var active = false
    private var generation = UUID()
    private var listPresentation = UUID()
    private var detailPresentation = UUID()
    private var mutationPresentation = UUID()
    private var copyTask: Task<Void, Never>?
    private var listTask: Task<Void, Never>?
    private var detailTask: Task<Void, Never>?
    private var mutationTask: Task<Void, Never>?

    private(set) var copy: [String: String] = [:]
    private(set) var drafts: [MissionDraftSummary] = []
    private(set) var selectedID: String?
    private(set) var detail: StoredMissionDraft?
    private(set) var loading = false
    private(set) var detailBusy = false
    private(set) var mutationBusy = false
    private(set) var error: String?
    private(set) var notice: String?

    init(service: @escaping Service = { request in
        try await Task.detached {
            let file = request.operation.file.map { URL(fileURLWithPath: $0) }
            let scoped = file?.startAccessingSecurityScopedResource() ?? false
            defer { if scoped { file?.stopAccessingSecurityScopedResource() } }
            return try TCMissionDrafts.call(request)
        }.value
    }) {
        self.service = service
    }

    func text(_ key: String) -> String { copy[key] ?? "" }

    func loadCopy() {
        let screen = generation
        let service = service
        copyTask?.cancel()
        copyTask = Task { [weak self] in
            do {
                let response = try await service(.init(operation: .init("copy")))
                guard let self, self.generation == screen, !Task.isCancelled,
                      case .copy(let copy) = response else { return }
                self.copy = copy
            } catch {
                // A missing copy payload leaves authored UI words empty.
            }
        }
    }

    func open() {
        active = true
        loadCopy()
        if !mutationBusy { refresh(showNotice: false) }
    }

    func close() {
        active = false
        generation = UUID()
        listPresentation = UUID()
        detailPresentation = UUID()
        mutationPresentation = UUID()
        copyTask?.cancel(); copyTask = nil
        listTask?.cancel(); listTask = nil
        detailTask?.cancel(); detailTask = nil
        // A handle-free FFI mutation runs on a detached task and cannot be
        // cancelled. Keep its wrapper alive so a reopened screen refreshes
        // only after the native operation has actually finished.
        loading = false; detailBusy = false
        selectedID = nil; detail = nil
    }

    func refresh() { refresh(showNotice: true) }

    private func refresh(showNotice: Bool) {
        guard active else { return }
        listPresentation = UUID()
        let presentation = listPresentation
        let screen = generation
        loading = true
        error = nil
        let service = service
        listTask?.cancel()
        listTask = Task { [weak self] in
            do {
                let response = try await service(.init(operation: .init("list")))
                guard let self, self.active, self.generation == screen,
                      self.listPresentation == presentation, !Task.isCancelled,
                      case .list(let drafts) = response else { return }
                self.drafts = drafts
                self.loading = false
                if showNotice { self.notice = self.text("refreshed") }
                if let id = self.selectedID, !drafts.contains(where: { $0.id == id }) {
                    self.invalidateDetail()
                }
            } catch {
                guard let self, self.active, self.generation == screen,
                      self.listPresentation == presentation, !Task.isCancelled else { return }
                self.loading = false
                self.error = self.errorText()
            }
        }
    }

    func show(_ id: String) {
        guard active, !mutationBusy else { return }
        detailPresentation = UUID()
        let presentation = detailPresentation
        let screen = generation
        detailBusy = true
        error = nil
        let service = service
        detailTask?.cancel()
        detailTask = Task { [weak self] in
            do {
                let response = try await service(.init(operation: .init("show", id: id)))
                guard let self, self.active, self.generation == screen,
                      self.detailPresentation == presentation,
                      !Task.isCancelled, case .show(let detail) = response,
                      detail.id == id else { return }
                self.selectedID = id
                self.detail = detail
                self.detailBusy = false
            } catch {
                guard let self, self.active, self.generation == screen,
                      self.detailPresentation == presentation,
                      !Task.isCancelled else { return }
                self.detailBusy = false
                self.error = self.errorText()
            }
        }
    }

    func closeDetail() { invalidateDetail() }

    func deleteConfirmation() -> DeleteConfirmation? {
        guard let selectedID, detail?.id == selectedID else { return nil }
        return DeleteConfirmation(id: selectedID, presentation: detailPresentation)
    }

    func delete(_ confirmation: DeleteConfirmation) {
        guard active, !mutationBusy, confirmation.id == selectedID,
              confirmation.presentation == detailPresentation else { return }
        let id = confirmation.id
        detailPresentation = UUID()
        detailTask?.cancel(); detailTask = nil; detailBusy = false
        beginMutation(.init("delete", id: id)) { response in
            guard case .deleted(let deleted) = response, deleted.id == id, deleted.deleted else { return nil }
            return .deleted(id: id)
        }
    }

    func importFile(_ file: URL) {
        guard active, !mutationBusy else { return }
        beginMutation(.init("import", file: file.path)) { response in
            guard case .imported(let imported) = response else { return nil }
            return .imported(inserted: imported.inserted)
        }
    }

    private func beginMutation(
        _ operation: MissionDraftRequest.Operation,
        accept: @escaping @MainActor (MissionDraftResponse) -> MutationEffect?
    ) {
        mutationPresentation = UUID()
        let presentation = mutationPresentation
        let screen = generation
        mutationBusy = true
        error = nil
        notice = nil
        let service = service
        mutationTask = Task { [weak self] in
            do {
                let response = try await service(.init(operation: operation))
                guard let self else { return }
                self.mutationTask = nil
                self.mutationBusy = false
                guard let effect = accept(response) else {
                    if self.active { self.error = self.errorText() }
                    if self.active, self.generation != screen { self.refresh(showNotice: false) }
                    return
                }
                guard self.active, self.generation == screen,
                      self.mutationPresentation == presentation else {
                    if self.active {
                        self.notice = nil
                        self.error = nil
                        self.invalidateDetail()
                        self.refresh(showNotice: false)
                    }
                    return
                }
                switch effect {
                case .imported(let inserted):
                    self.notice = self.text(inserted ? "added" : "duplicate")
                case .deleted(let id):
                    self.invalidateDetail()
                    self.drafts.removeAll { $0.id == id }
                    self.notice = self.text("deleted")
                }
                self.refresh(showNotice: false)
            } catch {
                guard let self else { return }
                self.mutationTask = nil
                self.mutationBusy = false
                guard self.active else { return }
                if self.generation == screen, self.mutationPresentation == presentation {
                    self.error = self.errorText()
                } else {
                    self.notice = nil
                    self.mutationBusy = false
                    self.error = self.errorText()
                    self.invalidateDetail()
                    self.refresh(showNotice: false)
                }
            }
        }
    }

    private func invalidateDetail() {
        detailPresentation = UUID()
        detailTask?.cancel(); detailTask = nil
        selectedID = nil; detail = nil; detailBusy = false
    }

    private func errorText() -> String { text("error") }
}
