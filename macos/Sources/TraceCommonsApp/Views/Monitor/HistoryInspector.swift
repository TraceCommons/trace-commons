#if DEBUG
import SwiftUI
import TCDesign
import TCShellCore

/// The selected History row's details (D-7). Skills first; Phase 4 adds
/// Withdraw and the public-run editor beside it.
///
/// Skills runs on the app's own record of the contribution
/// (`AppModel.history`). A row that record does not hold yet, or Skills
/// copy the core has not given, draws no panel rather than a broken one.
struct HistoryDetailInspector: View {
    let row: DaemonData.HistoryRow
    @EnvironmentObject private var model: AppModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                if let record = HistorySelection.record(for: row.submissionId, in: model.history),
                   let copy = model.skillLearningCopy {
                    // A new row is a new panel: its draft and install status
                    // are the record's, and the pane outlives the selection.
                    SkillLearningView(record: record, copy: copy)
                        .id(record.submissionID)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .scrollIndicators(.never)
    }
}

/// The legacy record behind a History row, by its submission id.
enum HistorySelection {
    static func record(for submissionId: String, in history: [HistoryRecord]) -> HistoryRecord? {
        history.first { $0.submissionID == submissionId }
    }
}
#endif
