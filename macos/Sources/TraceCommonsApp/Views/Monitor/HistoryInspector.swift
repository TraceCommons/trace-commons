#if DEBUG
import AppKit
import SwiftUI
import TCDesign
import TCShellCore

/// The selected History row's details (D-7): its status in the core's
/// words, then the session detail the legacy History opens (`SessionDetailView`:
/// reading and read-failure with Retry, the contribution overview with
/// Withdraw and its confirmation, Skills behind `SkillLearningGate`, and
/// the public-run editor), composed rather than drawn a second time.
///
/// Everything below the status runs on the app's own record of the
/// contribution (`AppModel.history`). A row the record does not hold yet
/// draws its status alone: no Withdraw, no editor, no Skills (fail closed).
/// Withdraw asks the detail's status, else the record's, never the list
/// row's optional one.
struct HistoryDetailInspector: View {
    let row: DaemonData.HistoryRow
    @EnvironmentObject private var model: AppModel

    private var record: HistoryRecord? {
        HistorySelection.record(for: row.submissionId, in: model.history)
    }

    // The always-present scroll view reads the detail and the installed
    // skill for the row, again when its record first resolves or its status
    // moves, and again when the app comes back to the front, as the session
    // detail screen does.
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                summary
                defects
                // A new row is a new detail: its editor draft, withdrawal
                // confirmation and Skills panel are the record's, and the
                // pane outlives the selection.
                if let record {
                    SessionDetailView(record: record)
                        .id(record.submissionID)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .scrollIndicators(.never)
        .task(id: [row.submissionId, record?.status ?? ""]) { load() }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            if let record { model.loadSessionDetail(record) }
        }
    }

    /// The row's folder, day and tool, and its status from the core's one
    /// table, the same word its list row shows. An unknown status reads the
    /// core's unavailable word; with no core copy there is no tag at all.
    private var summary: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            Text(row.projectLabel ?? "—")
                .glassType(GlassTokens.TypeScale.label.weight(.semibold))
                .foregroundStyle(GlassColor.textPrimary)
            Text(HomeFormat.meta(row, compact: false))
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textTertiary)
            if let tag = HomeFormat.statusWord(row.status, label: { HomeFormat.historyStatusLabel(copy: model.publicRunCopy, $0) }) {
                GlassTag(tag, tone: HomeFormat.tone(row.status))
                    .fixedSize()
            }
        }
        .accessibilityElement(children: .combine)
    }

    /// The withdrawal wording's own checks, where Withdraw is offered: a
    /// screen that admits its withdrawal wording has stopped being
    /// trustworthy, rather than one that quietly keeps showing it. Empty in
    /// every healthy build.
    @ViewBuilder
    private var defects: some View {
        let failures = WithdrawalCopyCheck.failures()
        if !failures.isEmpty {
            GlassNotice(tone: .outside, title: HistoryLegacyWords.withdrawalWordingDefect) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                    ForEach(failures, id: \.self) { Text($0).fixedSize(horizontal: false, vertical: true) }
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
