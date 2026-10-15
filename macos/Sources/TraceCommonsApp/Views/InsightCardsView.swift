import SwiftUI
import TCDesign
import TCBridge

struct InsightCardsView: View {
    @Bindable var model: InsightsModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(model.text("card_title")).insightsTitle()
            Text(model.text("card_selection_notice"))
                .insightsNote()
            selection
            HStack {
                Button(model.text("card_update")) { model.generateCards() }
                if model.cardBusy { GlassSpinner() }
            }
            if let error = model.cardError { Text(error).foregroundStyle(GlassStatus.outside.textColor) }
            if let result = model.cardResult {
                Text("\(model.text("provider")): \(result.provider.id) · \(result.provider.rubric_version)")
                    .insightsCaption()
                ForEach(result.cards) { card in cardView(card) }
            }
        }
    }

    private var selection: some View {
        InsightsDisclosure(model.text("card_choose_evidence")) {
            VStack(alignment: .leading, spacing: 6) {
                Text(model.text("saved")).insightsHeading()
                ForEach(model.snapshots) { snapshot in
                    Toggle(isOn: Binding(
                        get: { model.cardSnapshotSelection.contains(snapshot.id) },
                        set: { model.setCardSnapshot(snapshot.id, selected: $0) }
                    )) { Text(snapshot.id).insightsMono().lineLimit(1) }
                    .toggleStyle(GlassCheckboxStyle())
                }
                Text(model.text("episode_saved")).insightsHeading().padding(.top, 4)
                ForEach(model.episodes) { episode in
                    Toggle(isOn: Binding(
                        get: { model.cardEpisodeSelection.contains(episode.id) },
                        set: { model.setCardEpisode(episode.id, selected: $0) }
                    )) { Text(episode.id).insightsMono().lineLimit(1) }
                    .toggleStyle(GlassCheckboxStyle())
                }
            }.padding(.top, 6)
        }
    }

    private func cardView(_ card: InsightQuestionCard) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(model.text(card.question.copyKey)).insightsHeading()
                Spacer()
                Text(model.text("card_state_\(card.state.rawValue)"))
                    .insightsCaption()
            }
            ForEach(card.rows) { row in
                HStack(alignment: .firstTextBaseline) {
                    Text(rowTitle(row))
                    Spacer()
                    Text(rowValue(row)).monospacedDigit().multilineTextAlignment(.trailing)
                }
            }
            if !card.coverage.isEmpty {
                Text(model.text("card_coverage")).insightsCaption()
                ForEach(card.coverage) { coverage in
                    Text("\(model.text("card_coverage_\(coverage.unit)")): \(coverage.observed) / \(coverage.eligible)")
                        .insightsCaption()
                }
            }
            ForEach(card.limitations, id: \.self) { limitation in
                Text(model.text("card_limitation_\(limitation)"))
                    .insightsCaption()
            }
            if card.rows_omitted {
                Text(model.text("card_more_models")).insightsCaption()
            }
            evidence(card)
        }
        .insightsCard()
    }

    @ViewBuilder private func evidence(_ card: InsightQuestionCard) -> some View {
        if !card.evidence_ids.isEmpty {
            Text(model.text("card_evidence")).glassType(GlassTokens.TypeScale.caption.weight(.semibold))
            VStack(alignment: .leading, spacing: 4) {
                ForEach(card.evidence_ids, id: \.self) { id in
                    Button(id) { model.explain(id) }.buttonStyle(GlassButtonStyle(.link)).insightsMono().lineLimit(1)
                }
            }
        }
        if !card.episode_ids.isEmpty {
            Text(model.text("card_episodes")).glassType(GlassTokens.TypeScale.caption.weight(.semibold))
            VStack(alignment: .leading, spacing: 4) {
                ForEach(card.episode_ids, id: \.self) { id in
                    Button(id) { model.openEpisode(id) }.buttonStyle(GlassButtonStyle(.link)).insightsMono().lineLimit(1)
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
