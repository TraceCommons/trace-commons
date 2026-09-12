import SwiftUI
import TCBridge

struct ComparisonTasksView: View {
    @Bindable var model: ComparisonTasksModel
    let episodes: [EpisodeListEntry]
    let copy: [String: String]
    @State private var reconfirmation: ComparisonTasksModel.Confirmation?
    @State private var deletion: ComparisonTasksModel.Confirmation?

    private func text(_ key: String) -> String { copy[key] ?? "" }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text(text("comparison_task_title")).font(.title2)
                Spacer()
                Button(text("comparison_task_list")) { model.refresh() }
                if model.busy { ProgressView().controlSize(.small) }
            }
            if let notice = model.notice { Text(text(notice)).foregroundStyle(.green) }
            if let error = model.error { Text(text(error)).foregroundStyle(.red) }
            if let detail = model.detail { detailView(detail) } else { listView }
        }
        .disabled(model.busy)
        .confirmationDialog(text("comparison_task_reconfirm_notice"),
                            isPresented: confirmationBinding($reconfirmation)) {
            Button(text("comparison_task_reconfirm")) {
                if let reconfirmation { model.reconfirm(reconfirmation) }
                reconfirmation = nil
            }
        } message: {
            Text(model.detail?.task.material_digest ?? "")
        }
        .confirmationDialog(text("comparison_task_delete_confirm"),
                            isPresented: confirmationBinding($deletion)) {
            Button(text("comparison_task_delete"), role: .destructive) {
                if let deletion { model.delete(deletion) }
                deletion = nil
            }
        }
    }

    private var listView: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(text("comparison_task_create")).font(.headline)
            episodeChooser(selection: $model.createSelection)
            Button(text("comparison_task_create")) { model.create() }
                .disabled(model.createSelection.isEmpty)
            Divider()
            if model.tasks.isEmpty { Text(text("comparison_task_empty")) }
            ForEach(model.tasks) { item in
                Button { model.select(item.id) } label: {
                    VStack(alignment: .leading, spacing: 3) {
                        Text(item.id).font(.caption.monospaced()).lineLimit(1)
                        Text("\(text("comparison_task_revision")): \(item.task.revision)").font(.caption)
                        status(item).font(.caption).foregroundStyle(.secondary)
                    }.frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }
    }

    private func detailView(_ detail: ComparisonTaskDetail) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Button(text("comparison_task_back")) { model.closeDetail() }
                Spacer()
                Button(text("comparison_task_delete"), role: .destructive) {
                    deletion = model.deletion()
                }
            }
            Group {
                Text(detail.task.id)
                Text("\(text("comparison_task_revision")): \(detail.task.revision)")
                Text("\(text("comparison_task_material_digest")): \(detail.task.material_digest)")
                Text("\(text("comparison_task_resolved")): \(InsightsDate.label(detail.resolved_at))")
            }.font(.caption.monospaced()).textSelection(.enabled)
            status(detail)
            staleReview(detail)
            frozenEvidence(detail)
            episodeEditor(detail)
            contextEditor(detail)
            outcomeEditor(detail)
            Button(text("comparison_task_reconfirm")) {
                reconfirmation = model.reconfirmation()
            }
        }
    }

    private func status(_ detail: ComparisonTaskDetail) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(text(detail.task.context?.isComplete == true
                      ? "comparison_task_context_complete" : "comparison_task_context_incomplete"))
            Text(text(detail.task.outcome == nil
                      ? "comparison_task_outcome_unassessed" : "outcome_\(detail.task.outcome!.value.rawValue)"))
            let current = detail.task.independence_confirmation?.material_revision == detail.task.material_revision
                && detail.task.independence_confirmation?.material_digest == detail.task.material_digest
            Text(text(current ? "comparison_task_confirmation_current" : "comparison_task_confirmation_missing"))
            if detail.stale_reasons.contains(.attributionPendingQualification) {
                Text(text("comparison_task_attribution_pending"))
            }
        }.font(.callout)
    }

    private func staleReview(_ detail: ComparisonTaskDetail) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(text("comparison_task_stale_reasons")).font(.headline)
            if detail.stale_reasons.isEmpty { Text(text("comparison_task_stale_none")) }
            ForEach(detail.stale_reasons, id: \.self) { reason in
                Text(text("comparison_task_stale_\(reason.rawValue)"))
            }
            if !detail.overlapping_task_ids.isEmpty {
                Text(text("comparison_task_overlapping_tasks")).font(.headline)
                ForEach(detail.overlapping_task_ids, id: \.self) { Text($0).font(.caption.monospaced()) }
            }
        }
    }

    private func frozenEvidence(_ detail: ComparisonTaskDetail) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(text("comparison_task_frozen_evidence")).font(.headline)
            ForEach(detail.task.episodes) { episode in
                VStack(alignment: .leading, spacing: 3) {
                    Text(episode.episode_id).font(.caption.monospaced())
                    Text("\(text("comparison_task_revision")): \(episode.revision) · \(text("episode_membership_revision")): \(episode.membership_revision)")
                    Text(episode.members_digest).font(.caption.monospaced()).textSelection(.enabled)
                    ForEach(episode.members, id: \.snapshot_id) { member in
                        Text(member.snapshot_id).font(.caption.monospaced()).lineLimit(1)
                    }
                }.padding(8).background(.quaternary, in: RoundedRectangle(cornerRadius: 8))
            }
        }
    }

    private func episodeEditor(_ detail: ComparisonTaskDetail) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            if model.editingEpisodes {
                episodeChooser(selection: $model.editSelection)
                Button(text("comparison_task_replace_episodes")) { model.replaceEpisodes() }
                    .disabled(model.editSelection.isEmpty)
            } else {
                Button(text("comparison_task_replace_episodes")) { model.beginEpisodeEdit() }
            }
        }
    }

    private func contextEditor(_ detail: ComparisonTaskDetail) -> some View {
        VStack(alignment: .leading, spacing: 7) {
            Text(text("comparison_task_set_context")).font(.headline)
            TextField(text("comparison_task_project_id"), text: $model.projectID)
            TextField(text("comparison_task_task_date"), text: $model.taskDate)
            TextField(text("comparison_task_language"), text: $model.language)
            TextField(text("comparison_task_harness_id"), text: $model.harnessID)
            TextField(text("comparison_task_harness_version"), text: $model.harnessVersion)
            Picker(text("comparison_task_reasoning_effort"), selection: $model.reasoningEffort) {
                ForEach(ComparisonReasoningEffort.allCases, id: \.self) {
                    Text(text("comparison_task_reasoning_\($0.rawValue)")).tag($0)
                }
            }
            TextField(text("comparison_task_tool_policy_id"), text: $model.toolPolicyID)
            TextField(text("comparison_task_tool_policy_version"), text: $model.toolPolicyVersion)
            TextField(text("comparison_task_prompt_template_digest"), text: $model.promptTemplateDigest)
            Text(text("comparison_task_checkout_unavailable")).font(.caption).foregroundStyle(.secondary)
            Button(text("comparison_task_set_context")) { model.saveContext() }
        }
    }

    private func outcomeEditor(_ detail: ComparisonTaskDetail) -> some View {
        HStack {
            Picker(text("comparison_task_set_outcome"), selection: $model.outcome) {
                ForEach(ComparisonTaskOutcome.allCases, id: \.self) {
                    Text(text("outcome_\($0.rawValue)")).tag($0)
                }
            }
            Button(text("comparison_task_set_outcome")) { model.setOutcome() }
            if detail.task.outcome != nil {
                Button(text("comparison_task_clear_outcome")) { model.clearOutcome() }
            }
        }
    }

    private func episodeChooser(selection: Binding<Set<String>>) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            ForEach(episodes) { entry in
                Toggle(isOn: Binding(get: { selection.wrappedValue.contains(entry.id) }, set: { selected in
                    if selected { selection.wrappedValue.insert(entry.id) }
                    else { selection.wrappedValue.remove(entry.id) }
                })) {
                    Text(entry.id).font(.caption.monospaced()).lineLimit(1)
                }.toggleStyle(.checkbox)
            }
        }
    }
    private func confirmationBinding(_ value: Binding<ComparisonTasksModel.Confirmation?>) -> Binding<Bool> {
        Binding(get: { value.wrappedValue != nil }, set: { if !$0 { value.wrappedValue = nil } })
    }
}
