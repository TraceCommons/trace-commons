import SwiftUI
import TCBridge
import UniformTypeIdentifiers

struct MissionDraftsView: View {
    let model: MissionDraftsModel
    @State private var choosingFile = false
    @State private var selectedFile: URL?
    @State private var deleteConfirmation: MissionDraftsModel.DeleteConfirmation?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Text(model.text("intro"))
                controls
                if let selectedFile {
                    Text("\(model.text("file_selected")): \(selectedFile.lastPathComponent)")
                        .font(.callout)
                        .textSelection(.enabled)
                }
                if model.loading || model.mutationBusy {
                    ProgressView(model.text("working"))
                        .controlSize(.small)
                }
                if let error = model.error { Text(error).foregroundStyle(.red) }
                if let notice = model.notice { Text(notice).foregroundStyle(.secondary) }
                Text(model.text("review_notice"))
                    .font(.callout)
                    .foregroundStyle(.secondary)
                Text(model.text("authority_notice"))
                    .font(.callout)
                    .foregroundStyle(.secondary)
                Divider()
                draftList
                if model.detailBusy { ProgressView().controlSize(.small) }
                if let detail = model.detail {
                    Divider()
                    MissionDraftDetailView(detail: detail, copy: model.copy)
                    HStack {
                        Button(model.text("close")) { model.closeDetail() }
                        Button(model.text("delete"), role: .destructive) {
                            deleteConfirmation = model.deleteConfirmation()
                        }
                    }
                    .disabled(model.mutationBusy)
                }
            }
            .padding(24)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .fileImporter(
            isPresented: $choosingFile,
            allowedContentTypes: [.json, .data],
            allowsMultipleSelection: false
        ) { result in
            if case .success(let files) = result { selectedFile = files.first }
        }
        .confirmationDialog(
            model.text("delete_confirm_title"),
            isPresented: Binding(
                get: { deleteConfirmation != nil },
                set: { if !$0 { deleteConfirmation = nil } }
            ),
            titleVisibility: .visible
        ) {
            Button(model.text("delete"), role: .destructive) {
                guard let confirmation = deleteConfirmation else { return }
                deleteConfirmation = nil
                model.delete(confirmation)
            }
            Button(model.text("cancel"), role: .cancel) { deleteConfirmation = nil }
        } message: {
            Text(model.text("delete_confirm"))
        }
        .onAppear { model.open() }
        .onDisappear { model.close() }
    }

    private var controls: some View {
        HStack {
            Button(model.text("choose_file")) { choosingFile = true }
            if selectedFile != nil {
                Button(model.text("import")) {
                    guard let selectedFile else { return }
                    model.importFile(selectedFile)
                    self.selectedFile = nil
                }
            }
            Button(model.text("refresh")) { model.refresh() }
            Spacer(minLength: 0)
        }
        .disabled(model.mutationBusy)
    }

    @ViewBuilder
    private var draftList: some View {
        if model.drafts.isEmpty, !model.loading {
            Text(model.text("empty"))
        } else {
            LazyVStack(alignment: .leading, spacing: 8) {
                ForEach(model.drafts) { draft in
                    Button { model.show(draft.id) } label: {
                        VStack(alignment: .leading, spacing: 4) {
                            Text(model.text(draft.status))
                                .font(.headline)
                            Text("\(model.text("source_count")): \(draft.source_count.formatted())")
                                .font(.caption)
                            Text(draft.id)
                                .font(.caption.monospaced())
                                .lineLimit(1)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .buttonStyle(.plain)
                    .disabled(model.mutationBusy)
                    .accessibilityLabel("\(model.text(draft.status)), \(draft.id)")
                }
            }
        }
    }
}

struct MissionDraftDetailView: View {
    let detail: StoredMissionDraft
    let copy: [String: String]

    private func text(_ key: String) -> String { copy[key] ?? "" }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(detail.proposal.title).font(.title2)
            field("proposal_sha256", detail.id, monospaced: true)
            field("proposal_title", detail.proposal.title)
            field("source_claim", detail.proposal.claim_to_test)
            field("task", detail.proposal.task)
            field("starting_artifact", detail.proposal.starting_artifact.url)
            field("starting_artifact_digest", detail.proposal.starting_artifact.sha256, monospaced: true)
            values("source_urls", detail.proposal.source_urls)
            values("success_criteria", detail.proposal.success_criteria)
            values("required_evidence", detail.proposal.required_evidence)
            values("allowed_models", detail.proposal.allowed_models)
            values("allowed_tools", detail.proposal.allowed_tools)
            VStack(alignment: .leading, spacing: 4) {
                Text(text("proposed_budget")).font(.headline)
                Text("\(text("duration_seconds")): \(detail.proposal.budget.max_duration_seconds.formatted())")
                Text("\(text("input_tokens")): \(detail.proposal.budget.max_input_tokens.formatted())")
                Text("\(text("output_tokens")): \(detail.proposal.budget.max_output_tokens.formatted())")
            }
            field("author_unverified", detail.proposal.author_id)
            field("evaluator_unverified", detail.proposal.evaluator_id)
            field("rubric_version", detail.proposal.rubric_version)
            Text(text(detail.review.status)).font(.headline)
            Text(text("display_notice")).font(.callout).foregroundStyle(.secondary)
            Text(text("review_notice")).font(.callout).foregroundStyle(.secondary)
            Text(text("authority_notice")).font(.callout).foregroundStyle(.secondary)
        }
        .textSelection(.enabled)
        .accessibilityElement(children: .contain)
    }

    private func field(_ key: String, _ value: String, monospaced: Bool = false) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(text(key)).font(.headline)
            Text(value).font(monospaced ? .body.monospaced() : .body)
        }
    }

    private func values(_ key: String, _ values: [String]) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(text(key)).font(.headline)
            ForEach(Array(values.enumerated()), id: \.offset) { _, value in
                Text("• \(value)")
            }
        }
    }
}
