import SwiftUI
import TCBridge
import UniformTypeIdentifiers

struct InsightModelSection: View {
    let observations: InsightModelObservations?
    let copy: [String: String]
    private func text(_ key: String) -> String { copy[key] ?? "" }
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(text("model_title")).font(.headline)
            if let observations {
                Text(observations.declared_models.isEmpty ? text("model_no_labels") : observations.declared_models.joined(separator: ", "))
                    .textSelection(.enabled)
                Text(text(observations.mixed_declared_models ? "model_mixed" : "model_not_proven_mixed"))
                    .font(.callout)
                DisclosureGroup(text("model_references")) {
                    InsightModelDetails(observations: observations, copy: copy)
                }
            } else {
                Text(text("model_legacy"))
            }
            Text(text("model_notice")).font(.caption)
        }
    }
}

struct InsightModelDetails: View {
    let observations: InsightModelObservations
    let copy: [String: String]
    private func text(_ key: String) -> String { copy[key] ?? "" }
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("\(text("model_record_count")): \(observations.record_count.formatted())")
            Text("\(text("model_candidates")): \(observations.candidate_records.formatted())")
            Text("\(text("model_valid")): \(observations.valid_declarations.formatted())")
            Text("\(text("model_missing")): \(observations.missing_declarations.formatted())")
            Text("\(text("model_invalid")): \(observations.invalid_declarations.formatted())")
            Text("\(text("model_omitted")): \(observations.omitted_declarations.formatted())")
            if observations.model_labels_omitted { Text(text("model_labels_omitted")) }
            Text(text("model_coordinates_" + observations.coordinates.rawValue))
            Text("\(text("source_digest")): \(observations.source_digest)").font(.caption.monospaced())
            ForEach(observations.declarations) { declaration in
                Text("\(declaration.model) · \(text("model_kind_" + declaration.kind.rawValue)) · \(text("model_record_index")) \(declaration.record_index.formatted())")
                    .font(.caption)
            }
        }.textSelection(.enabled)
    }
}

struct InsightEvidenceControls: View {
    let model: InsightsModel
    let insight: LocalInsight
    @State private var commit = ""
    @State private var choosingRepository = false
    @State private var choosingReport = false
    @State private var pendingGit: GitSelection?
    @State private var pendingReport: InsightsModel.EvidenceSelection?
    private struct GitSelection {
        let target: InsightsModel.EvidenceSelection
        let commit: String
    }
    private func text(_ key: String) -> String { model.text(key) }
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(text("linked_evidence_title")).font(.headline)
            Text(text("link_notice")).font(.callout)
            DisclosureGroup(text("link_git")) {
                VStack(alignment: .leading, spacing: 8) {
                    Text(text("link_git_notice")).font(.caption)
                    TextField(text("link_commit"), text: $commit)
                        .textFieldStyle(.roundedBorder).font(.body.monospaced()).autocorrectionDisabled()
                    Button(text("choose_repository"), action: chooseRepository)
                        .disabled(!InsightEvidenceInput.isFullCommit(commit) || choosingRepository || choosingReport)
                }
            }
            DisclosureGroup(text("link_test_report")) {
                VStack(alignment: .leading, spacing: 8) {
                    Text(text("link_test_notice")).font(.caption)
                    Text(text("test_report_format")).font(.caption).textSelection(.enabled)
                    Button(text("choose_test_report"), action: chooseReport)
                        .disabled(choosingRepository || choosingReport)
                }
            }
            if (insight.outcome_links ?? []).isEmpty { Text(text("link_empty")) }
            ForEach(insight.outcome_links ?? []) { link in
                DisclosureGroup(title(link)) {
                    InsightOutcomeDetails(link: link, copy: model.copy)
                    Button(text("unlink_evidence"), role: .destructive) {
                        guard let selection = selectionForView() else { return }
                        model.unlinkEvidence(selection: selection, evidenceID: link.id)
                    }
                }
            }
        }
        .disabled(model.busy)
        .fileImporter(isPresented: $choosingRepository, allowedContentTypes: [.folder]) { result in
            let request = pendingGit
            pendingGit = nil
            guard let request, case .success(let directory) = result else { return }
            model.linkGit(selection: request.target, repository: directory, commit: request.commit)
        }
        .fileImporter(isPresented: $choosingReport, allowedContentTypes: [.json]) { result in
            let selection = pendingReport
            pendingReport = nil
            guard let selection, case .success(let file) = result else { return }
            model.linkTestReport(selection: selection, file: file)
        }
    }
    private func selectionForView() -> InsightsModel.EvidenceSelection? {
        guard let selection = model.evidenceSelection(), selection.snapshotID == insight.id else { return nil }
        return selection
    }
    private func chooseRepository() {
        guard !choosingRepository, !choosingReport, pendingGit == nil, pendingReport == nil,
              let selection = selectionForView(), InsightEvidenceInput.isFullCommit(commit) else { return }
        pendingGit = GitSelection(target: selection, commit: commit)
        choosingRepository = true
    }
    private func chooseReport() {
        guard !choosingRepository, !choosingReport, pendingGit == nil, pendingReport == nil,
              let selection = selectionForView() else { return }
        pendingReport = selection
        choosingReport = true
    }
    private func title(_ link: InsightOutcomeLink) -> String {
        switch link.evidence {
        case .gitCommit: return text("link_git_provenance")
        case .testReport: return text("link_test_provenance")
        }
    }
}

struct InsightOutcomeDetails: View {
    let link: InsightOutcomeLink
    let copy: [String: String]
    private func text(_ key: String) -> String { copy[key] ?? "" }
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(text("link_user_provenance"))
            Text("\(text("link_id")): \(link.id)").font(.caption.monospaced())
            Text("\(text("source_digest")): \(link.source_digest)").font(.caption.monospaced())
            Text("\(text("link_recorded_at")): \(InsightsDate.label(link.linked_at))")
            switch link.evidence {
            case .gitCommit(let evidence):
                Text(text("link_git_provenance")).font(.headline)
                Text(text("link_git_notice")).font(.callout)
                Text("\(text("git_object")): \(evidence.object_id)").font(.caption.monospaced())
                Text("\(text("git_tree")): \(evidence.tree_id)").font(.caption.monospaced())
                Text("\(text("git_repository_digest")): \(evidence.repository_path_digest)").font(.caption.monospaced())
                Text("\(text("git_inspected_at")): \(InsightsDate.label(evidence.inspected_at))")
                Text(text("git_parents"))
                if evidence.parent_ids.isEmpty { Text(text("git_no_parents")) }
                ForEach(evidence.parent_ids, id: \.self) { Text($0).font(.caption.monospaced()) }
            case .testReport(let evidence):
                Text(text("link_test_provenance")).font(.headline)
                Text(text("link_test_notice")).font(.callout)
                Text("\(text("test_runner")): \(evidence.runner)")
                Text("\(text("test_passed")): \(evidence.passed.formatted())")
                Text("\(text("test_failed")): \(evidence.failed.formatted())")
                Text("\(text("test_skipped")): \(evidence.skipped.formatted())")
                Text("\(text("test_observed_at")): \(InsightsDate.label(evidence.observed_at))")
                Text("\(text("test_imported_at")): \(InsightsDate.label(evidence.imported_at))")
                Text("\(text("test_claimed_commit")): \(evidence.commit_id ?? text("unknown"))").font(.caption.monospaced())
                Text("\(text("artifact_digest")): \(evidence.artifact_digest)").font(.caption.monospaced())
            }
        }.textSelection(.enabled)
    }
}

enum InsightEvidenceInput {
    static func isFullCommit(_ value: String) -> Bool {
        (value.utf8.count == 40 || value.utf8.count == 64)
            && value.utf8.allSatisfy { (48...57).contains($0) || (97...102).contains($0) }
    }
}
