import SwiftUI
import TCBridge

struct InsightCardsView: View {
    @Bindable var model: InsightsModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Questions about saved evidence").font(.title2)
            Text("Choose saved snapshots and episode groups. Empty selections cover all saved evidence.")
                .font(.callout).foregroundStyle(.secondary)
            selection
            HStack {
                Button("Update cards") { model.generateCards() }
                if model.cardBusy { ProgressView().controlSize(.small) }
            }
            if let error = model.cardError { Text(error).foregroundStyle(.red) }
            if let result = model.cardResult {
                Text("\(model.text("provider")): \(result.provider.id) · \(result.provider.rubric_version)")
                    .font(.caption).foregroundStyle(.secondary)
                ForEach(result.cards) { card in cardView(card) }
            }
        }
    }

    private var selection: some View {
        DisclosureGroup("Choose evidence") {
            VStack(alignment: .leading, spacing: 6) {
                Text(model.text("saved")).font(.headline)
                ForEach(model.snapshots) { snapshot in
                    Toggle(isOn: Binding(
                        get: { model.cardSnapshotSelection.contains(snapshot.id) },
                        set: { model.setCardSnapshot(snapshot.id, selected: $0) }
                    )) { Text(snapshot.id).font(.caption.monospaced()).lineLimit(1) }
                    .toggleStyle(.checkbox)
                }
                Text(model.text("episode_saved")).font(.headline).padding(.top, 4)
                ForEach(model.episodes) { episode in
                    Toggle(isOn: Binding(
                        get: { model.cardEpisodeSelection.contains(episode.id) },
                        set: { model.setCardEpisode(episode.id, selected: $0) }
                    )) { Text(episode.id).font(.caption.monospaced()).lineLimit(1) }
                    .toggleStyle(.checkbox)
                }
            }.padding(.top, 6)
        }
    }

    private func cardView(_ card: InsightQuestionCard) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(model.text(card.question.copyKey)).font(.headline)
                Spacer()
                Text(model.text("card_state_\(card.state.rawValue)"))
                    .font(.caption).foregroundStyle(.secondary)
            }
            ForEach(card.rows) { row in
                HStack(alignment: .firstTextBaseline) {
                    Text(rowTitle(row))
                    Spacer()
                    Text(rowValue(row)).monospacedDigit().multilineTextAlignment(.trailing)
                }
            }
            if !card.coverage.isEmpty {
                Text(model.text("card_coverage")).font(.caption).foregroundStyle(.secondary)
                ForEach(card.coverage) { coverage in
                    Text("\(model.text("card_coverage_\(coverage.unit)")): \(coverage.observed) / \(coverage.eligible)")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
            ForEach(card.limitations, id: \.self) { limitation in
                Text(model.text("card_limitation_\(limitation)"))
                    .font(.caption).foregroundStyle(.secondary)
            }
            if card.rows_omitted {
                Text(model.text("card_more_models")).font(.caption).foregroundStyle(.secondary)
            }
            evidence(card)
        }
        .padding(12).background(.quaternary, in: RoundedRectangle(cornerRadius: 10))
    }

    @ViewBuilder private func evidence(_ card: InsightQuestionCard) -> some View {
        if !card.evidence_ids.isEmpty {
            Text(model.text("card_evidence")).font(.caption.bold())
            VStack(alignment: .leading, spacing: 4) {
                ForEach(card.evidence_ids, id: \.self) { id in
                    Button(id) { model.explain(id) }.font(.caption.monospaced()).lineLimit(1)
                }
            }
        }
        if !card.episode_ids.isEmpty {
            Text(model.text("card_episodes")).font(.caption.bold())
            VStack(alignment: .leading, spacing: 4) {
                ForEach(card.episode_ids, id: \.self) { id in
                    Button(id) { model.openEpisode(id) }.font(.caption.monospaced()).lineLimit(1)
                }
            }
        }
    }

    private func rowTitle(_ row: InsightCardRow) -> String {
        let title = model.text("card_row_\(row.id)")
        return row.label.map { "\(title) (\($0))" } ?? title
    }
    private func rowValue(_ row: InsightCardRow) -> String {
        guard let value = row.value else {
            return row.missing_reason.map { model.text("card_missing_\($0)") } ?? ""
        }
        switch value {
        case .count(let count): return String(count)
        case .milliseconds(let milliseconds): return "\(milliseconds) ms"
        case .unixMilliseconds(let milliseconds):
            return Date(timeIntervalSince1970: Double(milliseconds) / 1_000)
                .formatted(date: .abbreviated, time: .standard)
        }
    }
}
