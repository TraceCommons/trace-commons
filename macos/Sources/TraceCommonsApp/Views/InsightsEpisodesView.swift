import SwiftUI
import TCBridge

struct InsightsEpisodesView: View {
    @Bindable var model: InsightsModel
    @State private var clearConfirmation: InsightsModel.EpisodeConfirmation?
    @State private var deleteConfirmation: InsightsModel.EpisodeConfirmation?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text(model.text("episode_title")).font(.title2)
                Spacer()
                Button(model.text("refresh")) { model.refreshEpisodes() }
                if model.episodeBusy { ProgressView().controlSize(.small) }
            }
            Text(model.text("episode_scope")).font(.callout).foregroundStyle(.secondary)
            if let notice = model.episodeNotice { Text(notice).foregroundStyle(.green) }
            if let error = model.episodeError { Text(error).foregroundStyle(.red) }

            if let detail = model.episodeDetail {
                detailView(detail)
            } else {
                createView
                Divider()
                Text(model.text("episode_saved")).font(.headline)
                if model.episodes.isEmpty { Text(model.text("episode_empty")) }
                ForEach(model.episodes) { entry in
                    Button { model.openEpisode(entry.id) } label: {
                        VStack(alignment: .leading, spacing: 3) {
                            Text(entry.id).font(.caption.monospaced()).lineLimit(1)
                            Text("\(model.text("episode_members")): \(entry.episode.members.count)")
                                .font(.caption)
                            Text(entry.overlapping_episode_ids.isEmpty
                                 ? model.text("episode_no_overlap")
                                 : "\(model.text("episode_overlaps")): \(entry.overlapping_episode_ids.count)")
                                .font(.caption).foregroundStyle(.secondary)
                        }.frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
            }
        }
        .disabled(model.episodeBusy)
        .confirmationDialog(model.text("episode_clear_assessment_confirm"),
                            isPresented: confirmationBinding($clearConfirmation)) {
            Button(model.text("episode_clear_assessment"), role: .destructive) {
                if let confirmation = clearConfirmation { model.clearEpisodeAssessment(confirmation) }
                clearConfirmation = nil
            }
        }
        .confirmationDialog(model.text("episode_delete_confirm"),
                            isPresented: confirmationBinding($deleteConfirmation)) {
            Button(model.text("episode_delete"), role: .destructive) {
                if let confirmation = deleteConfirmation { model.deleteEpisode(confirmation) }
                deleteConfirmation = nil
            }
        }
    }

    private var createView: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(model.text("episode_create")).font(.headline)
            Text(model.text("episode_create_notice")).font(.caption).foregroundStyle(.secondary)
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
            }
            Group {
                Text("\(model.text("episode_id")): \(detail.episode.id)")
                Text("\(model.text("episode_revision")): \(detail.episode.revision)")
                Text("\(model.text("episode_membership_revision")): \(detail.episode.membership_revision)")
                Text("\(model.text("episode_created_at")): \(InsightsDate.label(detail.episode.created_at))")
                Text("\(model.text("episode_updated_at")): \(InsightsDate.label(detail.episode.updated_at))")
                Text("\(model.text("episode_resolved")): \(InsightsDate.label(detail.resolved_at))")
            }.font(.caption).textSelection(.enabled)

            Text(model.text("episode_overlap_notice")).font(.caption).foregroundStyle(.secondary)
            if detail.episode.members.flatMap({ member in
                detail.overlap.first(where: { $0.snapshot_id == member.snapshot_id })?.episode_ids ?? []
            }).isEmpty {
                Text(model.text("episode_no_overlap"))
            }

            Text(model.text("episode_member_evidence")).font(.headline)
            ForEach(detail.members) { member in
                VStack(alignment: .leading, spacing: 5) {
                    Button { model.explain(member.id) } label: {
                        Text(member.id).font(.caption.monospaced()).lineLimit(1)
                    }
                    ForEach(member.report.evidence) { evidence in
                        Text("\(evidence.id) · \(evidence.source_digest)")
                            .font(.caption.monospaced()).lineLimit(1).textSelection(.enabled)
                    }
                    let overlaps = detail.overlap.first { $0.snapshot_id == member.id }?.episode_ids ?? []
                    if !overlaps.isEmpty {
                        Text("\(model.text("episode_overlaps")): \(overlaps.joined(separator: ", "))")
                            .font(.caption).textSelection(.enabled)
                    }
                }.padding(8).background(.quaternary, in: RoundedRectangle(cornerRadius: 8))
            }

            if !model.episodeEditingMembers {
                Button(model.text("episode_edit_members")) { model.beginEpisodeMemberEdit() }
            } else {
                Text(model.text("episode_edit_members_notice")).font(.caption).foregroundStyle(.secondary)
                snapshotChooser(selection: $model.episodeEditSelection)
                Button(model.text("episode_save_members")) { model.saveEpisodeMembers() }
            }

            VStack(alignment: .leading, spacing: 8) {
                Text(model.text("episode_assessment")).font(.headline)
                Text(model.text("episode_assessment_notice_short")).font(.caption).foregroundStyle(.secondary)
                if detail.episode.manual_assessment == nil { Text(model.text("episode_unassessed")) }
                HStack {
                    Picker(model.text("category"), selection: $model.episodeCategory) {
                        ForEach(["unknown", "refactor", "tests", "docs", "debugging", "other"], id: \.self) {
                            Text(model.text("category_" + $0)).tag($0)
                        }
                    }
                    Picker(model.text("outcome"), selection: $model.episodeOutcome) {
                        ForEach(["unknown", "accepted", "partial", "rejected"], id: \.self) {
                            Text(model.text("outcome_" + $0)).tag($0)
                        }
                    }
                }
                HStack {
                    Button(model.text("episode_save_assessment")) { model.saveEpisodeAssessment() }
                    if detail.episode.manual_assessment != nil {
                        Button(model.text("episode_clear_assessment")) {
                            clearConfirmation = model.episodeConfirmation()
                        }
                    }
                }
                Text(model.text("episode_assessment_notice")).font(.caption).foregroundStyle(.secondary)
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
                        Text(snapshot.id).font(.caption.monospaced()).lineLimit(1)
                    }
                }.toggleStyle(.checkbox)
            }
        }
    }
}
