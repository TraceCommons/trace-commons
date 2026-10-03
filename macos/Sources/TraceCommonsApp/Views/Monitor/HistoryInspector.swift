#if DEBUG
import AppKit
import SwiftUI
import TCDesign
import TCShellCore

/// The selected History row's details (D-7). Skills first; Phase 4 adds
/// Withdraw and the public-run editor beside it.
///
/// Skills runs on the app's own record of the contribution
/// (`AppModel.history`) and its session detail, under the same rule the
/// session detail screen uses (`SkillLearningGate`): nothing is offered
/// until the detail answers, and only an active, corrected contribution
/// offers learning. An installed skill shows outside that rule. A row the
/// record does not hold yet, or Skills copy the core has not given, draws
/// no panel rather than a broken one.
struct HistoryDetailInspector: View {
    let row: DaemonData.HistoryRow
    @EnvironmentObject private var model: AppModel

    private var record: HistoryRecord? {
        HistorySelection.record(for: row.submissionId, in: model.history)
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                if let record, let copy = model.skillLearningCopy {
                    skills(record, copy: copy)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .scrollIndicators(.never)
        // Read the detail and the installed skill for the row, again when
        // its record first resolves or its status moves, and again when the
        // app comes back to the front, as the session detail screen does.
        .task(id: [row.submissionId, record?.status ?? ""]) { load() }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            if let record { model.loadSessionDetail(record) }
        }
    }

    @ViewBuilder
    private func skills(_ record: HistoryRecord, copy: SkillLearningCopy) -> some View {
        let id = record.submissionID
        let detail = model.sessionDetails[id]
        let state = model.skillLearningState(for: id)
        let withdrawn = SkillLearningGate.withdrawalCompleted(
            recordStatus: record.status, withdrawal: model.withdrawals[id])
        if SkillLearningGate.offersLearning(detail, recordStatus: record.status, withdrawalCompleted: withdrawn)
            || (SkillLearningGate.showsInstalledSurface(detail, withdrawalCompleted: withdrawn)
                && state.installedSkill != nil)
        {
            // A new row is a new panel: its draft and install status are the
            // record's, and the pane outlives the selection.
            SkillLearningView(record: record, copy: copy)
                .id(record.submissionID)
        } else if SkillLearningGate.showsInstalledSurface(detail, withdrawalCompleted: withdrawn),
                  SkillLearningGate.offersInstallStatusRetry(state, detail: detail, recordStatus: record.status),
                  let message = state.failure,
                  let retry = model.publicRunCopy?.retryRead
        {
            GlassNotice(tone: .outside) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                    Text(message).fixedSize(horizontal: false, vertical: true)
                    Button(retry) { model.ensureLocalInstalledSkillStatus(for: record) }
                        .buttonStyle(GlassButtonStyle(.glass))
                        .frame(minHeight: 44)
                        .disabled(state.isWorking)
                }
            }
        }
    }

    private func load() {
        guard let record else { return }
        model.loadSessionDetail(record)
        model.ensureLocalInstalledSkillStatus(for: record)
    }
}

/// The legacy record behind a History row, by its submission id.
enum HistorySelection {
    static func record(for submissionId: String, in history: [HistoryRecord]) -> HistoryRecord? {
        history.first { $0.submissionID == submissionId }
    }
}
#endif
