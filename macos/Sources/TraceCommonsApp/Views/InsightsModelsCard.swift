import SwiftUI
import TCDesign
import TCShellCore

/// The Patterns "Models" card (spec 2026-10-20 section 8): which model did
/// what in the contributor's own traces, six questions in the daemon's
/// order. Each row is a scoped fact, never a ranking: the question, the
/// leading model and the next one with their figures and traces (or the
/// row's state), what the figures rest on, and the scope on the row itself
/// so a screenshot keeps it. How each figure is measured and what was left
/// out are behind the row's "i". Drawn only from answers the daemon gave;
/// otherwise the card is not drawn at all.
struct InsightsModelsCard: View {
    let answers: DaemonData.InsightsModelAnswers
    let copy: [String: String]

    private func text(_ key: String) -> String { copy[key] ?? "" }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            InsightsSectionTitle(text("analytics_models_title"), info: InsightsModelsWords.cardInfo(copy: copy))
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                if let window = InsightsModelsWords.window(answers, copy: copy) {
                    Text(window).insightsNote()
                }
                VStack(spacing: 0) {
                    ForEach(Array(InsightsModelsWords.rows(answers).enumerated()), id: \.element.id) { index, row in
                        GlassTableRow(first: index == 0) {
                            InsightsModelsRow(row: row, copy: copy)
                        }
                    }
                }
            }
            .insightsCardBody()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .insightsSection()
    }
}

/// One question's answer. The text is one VoiceOver element carrying the
/// row's whole sentence; the "i" beside it is a control of its own.
struct InsightsModelsRow: View {
    let row: DaemonData.InsightsModelAnswerRow
    let copy: [String: String]

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s4) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                Text(InsightsModelsWords.title(row, copy: copy)).insightsHeading()
                    .fixedSize(horizontal: false, vertical: true)
                ForEach(Array(InsightsModelsWords.lines(row, copy: copy).enumerated()), id: \.offset) { index, line in
                    Text(line)
                        .foregroundStyle(index == 0 ? GlassColor.textPrimary : GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                if let basis = InsightsModelsWords.basisLine(row, copy: copy) {
                    Text(basis).insightsCaption()
                        .fixedSize(horizontal: false, vertical: true)
                }
                Text(InsightsModelsWords.scope(copy: copy)).insightsCaption()
                    .fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(InsightsModelsWords.accessibilityLabel(row, copy: copy))
            InsightsInfo(InsightsModelsWords.title(row, copy: copy),
                         InsightsModelsWords.info(row, copy: copy))
        }
    }
}
