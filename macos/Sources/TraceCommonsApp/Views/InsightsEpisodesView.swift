import SwiftUI
import TCDesign
import TCBridge

struct InsightsEpisodesView: View {
    @Bindable var model: InsightsModel
    @State private var clearConfirmation: InsightsModel.EpisodeConfirmation?
    @State private var deleteConfirmation: InsightsModel.EpisodeConfirmation?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text(model.text("episode_title")).insightsTitle()
                Spacer()
                Button(model.text("refresh")) { model.refreshEpisodes() }
                    .accessibilityLabel(model.text("episode_refresh_accessibility"))
                if model.episodeBusy { GlassSpinner() }
            }
            Text(model.text("episode_scope")).insightsNote()
            if let notice = model.episodeNotice { Text(notice).foregroundStyle(GlassStatus.on.textColor) }
            if let error = model.episodeError { Text(error).foregroundStyle(GlassStatus.outside.textColor) }

            if let detail = model.episodeDetail {
                detailView(detail)
            } else {
                createView
                InsightsRule()
                Text(model.text("episode_saved")).insightsHeading()
                if model.episodes.isEmpty { Text(model.text("episode_empty")) }
                ForEach(model.episodes) { entry in
                    Button { model.openEpisode(entry.id) } label: {
                        VStack(alignment: .leading, spacing: 3) {
                            Text(entry.id).insightsMono().lineLimit(1)
                            Text("\(model.text("episode_members")): \(entry.episode.members.count)")
                                .glassType(GlassTokens.TypeScale.caption)
                            Text(entry.overlapping_episode_ids.isEmpty
                                 ? model.text("episode_no_overlap")
                                 : "\(model.text("episode_overlaps")): \(entry.overlapping_episode_ids.count)")
                                .insightsCaption()
                        }.frame(maxWidth: .infinity, alignment: .leading).insightsRowCard()
                    }
                    .buttonStyle(GlassPressStyle())
                }
            }
        }
        .disabled(model.episodeBusy)
        .insightsSurface()
        // Both are destructive: right-most, never on Return.
        .glassModal(isPresented: confirmationBinding($clearConfirmation)) {
            GlassConfirmation(
                title: model.text("episode_clear_assessment_confirm"),
                actions: [
                    .cancel(model.text("cancel")) { clearConfirmation = nil },
                    .destructive(model.text("episode_clear_assessment")) {
                        if let confirmation = clearConfirmation { model.clearEpisodeAssessment(confirmation) }
                        clearConfirmation = nil
                    },
                ],
                onCancel: { clearConfirmation = nil })
        }
        .glassModal(isPresented: confirmationBinding($deleteConfirmation)) {
            GlassConfirmation(
                title: model.text("episode_delete_confirm"),
                actions: [
                    .cancel(model.text("cancel")) { deleteConfirmation = nil },
                    .destructive(model.text("episode_delete")) {
                        if let confirmation = deleteConfirmation { model.deleteEpisode(confirmation) }
                        deleteConfirmation = nil
                    },
                ],
                onCancel: { deleteConfirmation = nil })
        }
    }

    private var createView: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(model.text("episode_create")).insightsHeading()
            Text(model.text("episode_create_notice")).insightsCaption()
            snapshotChooser(selection: $model.episodeCreateSelection)
            Button(model.text("episode_create")) { model.createEpisode() }
                .disabled(model.episodeCreateSelection.isEmpty)
        }
    }

    private func detailView(_ detail: EpisodeDetail) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Button(model.text("episode_saved")) { model.closeEpisode() }
                Spacer()
                Button(model.text("episode_delete"), role: .destructive) {
                    deleteConfirmation = model.episodeConfirmation()
                }
                .buttonStyle(GlassButtonStyle(.destructive, small: true))
            }
            Group {
                Text("\(model.text("episode_id")): \(detail.episode.id)")
                Text("\(model.text("episode_revision")): \(detail.episode.revision)")
                Text("\(model.text("episode_membership_revision")): \(detail.episode.membership_revision)")
                Text("\(model.text("episode_created_at")): \(InsightsDate.label(detail.episode.created_at))")
                Text("\(model.text("episode_updated_at")): \(InsightsDate.label(detail.episode.updated_at))")
                Text("\(model.text("episode_resolved")): \(InsightsDate.label(detail.resolved_at))")
            }.glassType(GlassTokens.TypeScale.caption).textSelection(.enabled)

            Text(model.text("episode_overlap_notice")).insightsCaption()
            if detail.episode.members.flatMap({ member in
                detail.overlap.first(where: { $0.snapshot_id == member.snapshot_id })?.episode_ids ?? []
            }).isEmpty {
                Text(model.text("episode_no_overlap"))
            }

            Text(model.text("episode_member_evidence")).insightsHeading()
            ForEach(detail.members) { member in
                VStack(alignment: .leading, spacing: 5) {
                    Button { model.explain(member.id) } label: {
                        Text(member.id).insightsMono().lineLimit(1)
                    }
                    .buttonStyle(GlassButtonStyle(.link))
                    ForEach(member.report.evidence) { evidence in
                        Text("\(evidence.id) · \(evidence.source_digest)")
                            .insightsMono().lineLimit(1).textSelection(.enabled)
                    }
                    let overlaps = detail.overlap.first { $0.snapshot_id == member.id }?.episode_ids ?? []
                    if !overlaps.isEmpty {
                        Text("\(model.text("episode_overlaps")): \(overlaps.joined(separator: ", "))")
                            .glassType(GlassTokens.TypeScale.caption).textSelection(.enabled)
                    }
                }.insightsCard()
            }

            if !model.episodeEditingMembers {
                Button(model.text("episode_edit_members")) { model.beginEpisodeMemberEdit() }
            } else {
                Text(model.text("episode_edit_members_notice")).insightsCaption()
                snapshotChooser(selection: $model.episodeEditSelection)
                Button(model.text("episode_save_members")) { model.saveEpisodeMembers() }
            }

            VStack(alignment: .leading, spacing: 8) {
                Text(model.text("episode_assessment")).insightsHeading()
                Text(model.text("episode_assessment_notice_short")).insightsCaption()
                if detail.episode.manual_assessment == nil { Text(model.text("episode_unassessed")) }
                HStack {
                    GlassSelect(model.text("category"), selection: $model.episodeCategory,
                                options: InsightsChoices.categories.map { GlassPickerOption(model.text("category_" + $0), value: $0) })
                    GlassSelect(model.text("outcome"), selection: $model.episodeOutcome,
                                options: InsightsChoices.outcomes.map { GlassPickerOption(model.text("outcome_" + $0), value: $0) })
                }
                HStack {
                    Button(model.text("episode_save_assessment")) { model.saveEpisodeAssessment() }
                        .accessibilityLabel(model.text("episode_save_assessment_accessibility"))
                    if detail.episode.manual_assessment != nil {
                        Button(model.text("episode_clear_assessment")) {
                            clearConfirmation = model.episodeConfirmation()
                        }
                    }
                }
                Text(model.text("episode_assessment_notice")).insightsCaption()
            }
        }
    }

    private func confirmationBinding(_ confirmation: Binding<InsightsModel.EpisodeConfirmation?>) -> Binding<Bool> {
        Binding(get: { confirmation.wrappedValue != nil },
                set: { if !$0 { confirmation.wrappedValue = nil } })
    }

    private func snapshotChooser(selection: Binding<Set<String>>) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            ForEach(model.snapshots) { snapshot in
                Toggle(isOn: Binding(
                    get: { selection.wrappedValue.contains(snapshot.id) },
                    set: { selected in
                        if selected { selection.wrappedValue.insert(snapshot.id) }
                        else { selection.wrappedValue.remove(snapshot.id) }
                    }
                )) {
                    VStack(alignment: .leading) {
                        Text(model.text(snapshot.source_format))
                        Text(snapshot.id).insightsMono().lineLimit(1)
                    }
                }.toggleStyle(GlassCheckboxStyle())
            }
        }
    }
}
