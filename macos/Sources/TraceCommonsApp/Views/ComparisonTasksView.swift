import SwiftUI
import TCDesign
import TCBridge

struct ComparisonTasksView: View {
    @Bindable var model: ComparisonTasksModel
    let episodes: [EpisodeListEntry]
    let copy: [String: String]
    let openEpisode: (String) -> Void
    let openSnapshot: (String) -> Void
    @State private var reconfirmation: ComparisonTasksModel.Confirmation?
    @State private var deletion: ComparisonTasksModel.Confirmation?

    private func text(_ key: String) -> String { copy[key] ?? "" }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text(text("comparison_task_title")).insightsTitle()
                Spacer()
                Button(text("comparison_task_list")) { model.refresh() }
                if model.busy { GlassSpinner() }
            }
            if let notice = model.notice { Text(text(notice)).foregroundStyle(GlassStatus.on.textColor) }
            if let error = model.error { Text(text(error)).foregroundStyle(GlassStatus.outside.textColor) }
            if let detail = model.detail { detailView(detail) } else { listView }
        }
        .disabled(model.busy)
        .insightsSurface()
        .glassModal(isPresented: confirmationBinding($reconfirmation)) {
            GlassConfirmation(
                title: text("comparison_task_reconfirm_notice"), message: model.detail?.task.material_digest ?? "",
                actions: [
                    .cancel(text("cancel")) { reconfirmation = nil },
                    GlassModalAction(text("comparison_task_reconfirm"), isDefault: true) {
                        if let reconfirmation { model.reconfirm(reconfirmation) }
                        reconfirmation = nil
                    },
                ],
                onCancel: { reconfirmation = nil })
        }
        // Delete is destructive: right-most, never on Return.
        .glassModal(isPresented: confirmationBinding($deletion)) {
            GlassConfirmation(
                title: text("comparison_task_delete_confirm"),
                actions: [
                    .cancel(text("cancel")) { deletion = nil },
                    .destructive(text("comparison_task_delete")) {
                        if let deletion { model.delete(deletion) }
                        deletion = nil
                    },
                ],
                onCancel: { deletion = nil })
        }
    }

    private var listView: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(text("comparison_task_create")).insightsHeading()
            episodeChooser(selection: $model.createSelection)
            Button(text("comparison_task_create")) { model.create() }
                .disabled(model.createSelection.isEmpty)
            InsightsRule()
            if model.tasks.isEmpty { Text(text("comparison_task_empty")) }
            ForEach(model.tasks) { item in
                Button { model.select(item.id) } label: {
                    VStack(alignment: .leading, spacing: 3) {
                        Text(taskLabel(item)).insightsHeading()
                        Text("\(item.task.episodes.count) \(text("comparison_task_frozen_evidence"))")
                            .glassType(GlassTokens.TypeScale.caption)
                        status(item).insightsCaption()
                    }.frame(maxWidth: .infinity, alignment: .leading).insightsRowCard()
                }
                .buttonStyle(GlassPressStyle())
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
                .buttonStyle(GlassButtonStyle(.destructive, small: true))
            }
            Text(taskLabel(detail)).insightsHeading()
            InsightsDisclosure(text("comparison_task_advanced_evidence")) {
                Group {
                    Text(detail.task.id)
                    Text("\(text("comparison_task_revision")): \(detail.task.revision)")
                    Text("\(text("comparison_task_material_digest")): \(detail.task.material_digest)")
                    Text("\(text("comparison_task_resolved")): \(InsightsDate.label(detail.resolved_at))")
                }.insightsMono().textSelection(.enabled)
            }
            status(detail)
            if let context = detail.task.context {
                VStack(alignment: .leading, spacing: 3) {
                    Text("\(text("comparison_task_project_id")): \(context.project_id)")
                    Text("\(text("comparison_task_category")): \(text("category_\(context.category)"))")
                    Text("\(text("comparison_task_task_date")): \(context.task_date)")
                    Text("\(text("comparison_task_configuration_fingerprint")): \(context.configuration_fingerprint)")
                }.glassType(GlassTokens.TypeScale.caption).textSelection(.enabled)
            }
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
            let current = confirmationIsCurrent(detail)
            Text(text(current ? "comparison_task_confirmation_current" : "comparison_task_confirmation_missing"))
            if detail.stale_reasons.contains(.attributionPendingQualification) {
                Text(text("comparison_task_attribution_pending"))
            }
        }.glassType(GlassTokens.TypeScale.label.weight(.regular))
    }

    private func staleReview(_ detail: ComparisonTaskDetail) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(text("comparison_task_stale_reasons")).insightsHeading()
            if detail.stale_reasons.isEmpty { Text(text("comparison_task_stale_none")) }
            ForEach(detail.stale_reasons, id: \.self) { reason in
                Text(text("comparison_task_stale_\(reason.rawValue)"))
            }
            if !detail.overlapping_task_ids.isEmpty {
                Text(text("comparison_task_overlap")).insightsHeading()
                ForEach(detail.overlapping_task_ids, id: \.self) { Text($0).insightsMono() }
            }
        }
    }

    private func frozenEvidence(_ detail: ComparisonTaskDetail) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(text("comparison_task_frozen_evidence")).insightsHeading()
            ForEach(detail.task.episodes) { episode in
                VStack(alignment: .leading, spacing: 3) {
                    let current = episodes.first { $0.id == episode.episode_id }?.episode
                    Text(current == nil ? text("comparison_task_current_missing")
                         : current?.revision == episode.revision && current?.membership_revision == episode.membership_revision
                            ? text("comparison_task_current_matches_frozen")
                            : text("comparison_task_current_changed"))
                    Text("\(episode.members.count) \(text("episode_members"))").glassType(GlassTokens.TypeScale.caption)
                    if current != nil {
                        Button(text("comparison_task_open_current_episode")) { openEpisode(episode.episode_id) }
                    }
                    ForEach(episode.members, id: \.snapshot_id) { member in
                        Button(text("comparison_task_open_frozen_snapshot")) { openSnapshot(member.snapshot_id) }
                    }
                    InsightsDisclosure(text("comparison_task_advanced_evidence")) {
                        Text(episode.episode_id).insightsMono()
                        Text("\(text("comparison_task_revision")): \(episode.revision) · \(text("episode_membership_revision")): \(episode.membership_revision)")
                        Text(episode.members_digest).insightsMono().textSelection(.enabled)
                        ForEach(episode.members, id: \.snapshot_id) { member in
                            Text(member.snapshot_id).insightsMono().lineLimit(1)
                        }
                    }
                }.insightsCard()
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
            Text(text("comparison_task_set_context")).insightsHeading()
            GlassSelect(text("comparison_task_project"), selection: $model.projectID,
                        options: model.knownProjectIDs.map { GlassPickerOption($0, value: $0) }
                            + (model.knownProjectIDs.contains(model.projectID)
                               ? [] : [GlassPickerOption(text("comparison_task_new_project"), value: model.projectID)]))
            Button(text("comparison_task_new_project")) { model.startNewProject() }
            InsightsDisclosure(text("comparison_task_advanced_evidence")) {
                GlassTextField(text("comparison_task_project_id"), text: $model.projectID)
            }
            GlassTextField(text("comparison_task_task_date"), text: $model.taskDate)
            GlassTextField(text("comparison_task_language"), text: $model.language)
            GlassTextField(text("comparison_task_harness_id"), text: $model.harnessID)
            GlassTextField(text("comparison_task_harness_version"), text: $model.harnessVersion)
            GlassSelect(text("comparison_task_reasoning_effort"), selection: $model.reasoningEffort,
                        options: ComparisonReasoningEffort.allCases.map {
                            GlassPickerOption(text("comparison_task_reasoning_\($0.rawValue)"), value: $0)
                        })
            GlassTextField(text("comparison_task_tool_policy_id"), text: $model.toolPolicyID)
            GlassTextField(text("comparison_task_tool_policy_version"), text: $model.toolPolicyVersion)
            GlassTextField(text("comparison_task_prompt_template_digest"), text: $model.promptTemplateDigest)
            Text(text("comparison_task_checkout_unavailable")).insightsCaption()
            Button(text("comparison_task_set_context")) { model.saveContext() }
        }
    }

    private func outcomeEditor(_ detail: ComparisonTaskDetail) -> some View {
        HStack {
            GlassSelect(text("comparison_task_set_outcome"), selection: $model.outcome,
                        options: ComparisonTaskOutcome.allCases.map {
                            GlassPickerOption(text("outcome_\($0.rawValue)"), value: $0)
                        })
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
                    VStack(alignment: .leading) {
                        Text("\(entry.episode.members.count) \(text("episode_members")) · \(InsightsDate.label(entry.episode.updated_at))")
                        Text(entry.episode.manual_assessment.map { text("outcome_\($0.outcome)") }
                             ?? text("episode_unassessed")).insightsCaption()
                    }
                }.toggleStyle(GlassCheckboxStyle())
            }
        }
    }
    private func confirmationBinding(_ value: Binding<ComparisonTasksModel.Confirmation?>) -> Binding<Bool> {
        Binding(get: { value.wrappedValue != nil }, set: { if !$0 { value.wrappedValue = nil } })
    }
    private func taskLabel(_ detail: ComparisonTaskDetail) -> String {
        let date = detail.task.context?.task_date ?? InsightsDate.label(detail.task.created_at)
        let outcome = detail.task.outcome.map { text("outcome_\($0.value.rawValue)") }
            ?? text("comparison_task_outcome_unassessed")
        return "\(date) · \(outcome)"
    }
    private func confirmationIsCurrent(_ detail: ComparisonTaskDetail) -> Bool {
        let upstream: Set<ComparisonTaskStaleReason> = [.episodeMissing, .episodeRevisionChanged,
            .episodeMembershipChanged, .snapshotMissingOrReplaced, .independenceMaterialChanged]
        return upstream.isDisjoint(with: detail.stale_reasons)
            && detail.task.independence_confirmation?.material_revision == detail.task.material_revision
            && detail.task.independence_confirmation?.material_digest == detail.task.material_digest
    }
}
