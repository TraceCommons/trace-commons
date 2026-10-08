import SwiftUI
import TCDesign
import TCShellCore

/// The opened History row's details (D-7), drawn in the inspector when a
/// History row is opened (Ron's inspector auto-open; #1146's History
/// detail): its status in the core's words, the row's tool, day, size and
/// how it was approved, then the session detail the legacy History opens
/// (`SessionDetailView`: reading and read-failure with Retry, the
/// contribution overview, Skills behind `SkillLearningGate`, and the
/// public-run editor), composed rather than drawn a second time. Withdraw
/// is not drawn here: it is on the row (Ron's `HistoryRow`), so History has
/// one.
///
/// Everything below the status runs on the app's own record of the
/// contribution (`AppModel.history`). A row the record does not hold yet
/// draws its status alone: no editor, no Skills (fail closed). Without the
/// core's public-run copy (static, not a daemon answer) the session detail
/// draws nothing, and nothing is offered.
struct HistoryDetailInspector: View {
    let row: DaemonData.HistoryRow
    @EnvironmentObject private var model: AppModel

    private var record: HistoryRecord? {
        HistorySelection.record(for: row.submissionId, in: model.history)
    }

    // Every read is the composed session detail's, on legacy's triggers:
    // the detail when it appears (a new record, or one that first resolves)
    // and when the app comes back to the front; the installed skill only for
    // a terminal or unaccepted row. The stack adds no read of its own, so a
    // status move does not reload the detail and an ineligible row costs no
    // install-status read. It scrolls inside the inspector's pane
    // (`HistoryInspectorPane`), so it has no scroll of its own.
    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            summary
            defects
            // A new row is a new detail: its editor draft, withdrawal
            // confirmation and Skills panel are the record's, and the
            // page outlives the selection.
            if let record {
                explanations(record)
                SessionDetailView(record: record, offersWithdrawal: false)
                    .id(record.submissionID)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// The row's status from the core's one table, the same word its list
    /// row shows. An unknown status reads the core's unavailable word; with
    /// no core copy there is no tag at all. The tag only until the session
    /// detail's overview, which says the status from the same table, is
    /// drawn (`HistorySelection.tagsStatus`), so the status is said once.
    /// The folder and day only while no record resolves: the session
    /// detail's heading carries them otherwise.
    private var summary: some View {
        // The resolved record's status when there is one: `model.withdraw`
        // refreshes the record, so the tag agrees with the Withdraw outcome
        // below it rather than waiting for the list's next poll.
        let status = record?.status ?? row.status
        return VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            if record == nil {
                Text(row.projectLabel ?? "—")
                    .glassType(GlassTokens.TypeScale.heading)
                    .foregroundStyle(GlassColor.textPrimary)
            }
            // The row's full line and how it was approved: the list row
            // says only its tool and day (Ron's `HistoryRow`).
            Text(HomeFormat.meta(row, compact: false))
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textTertiary)
                .fixedSize(horizontal: false, vertical: true)
            Text(HomeFormat.provenance(row.provenance))
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textTertiary)
                .fixedSize(horizontal: false, vertical: true)
            if HistorySelection.tagsStatus(record: record, detail: record.flatMap { model.sessionDetails[$0.submissionID] }),
               let tag = HomeFormat.statusWord(status, label: { HomeFormat.historyStatusLabel(copy: model.publicRunCopy, $0) }) {
                GlassTag(tag, tone: HomeFormat.tone(status))
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

    /// The server's own reasons for this record, of any status (why it was
    /// rejected, why it is held), without opaque digests. A held record the
    /// server said nothing about reads the core's held sentence instead
    /// (`MonitorScreensCopy.heldExplanation`); without the core's words it
    /// reads nothing, never a blank line.
    @ViewBuilder
    private func explanations(_ record: HistoryRecord) -> some View {
        let lines = HeldExplanations.lines(in: [record.explanations])
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            ForEach(lines, id: \.self) { Text($0).fixedSize(horizontal: false, vertical: true) }
            if lines.isEmpty, record.status == "quarantined", !MonitorWords.heldExplanation.isEmpty {
                Text(MonitorWords.heldExplanation).fixedSize(horizontal: false, vertical: true)
            }
        }
        .glassType(GlassTokens.TypeScale.caption)
        .foregroundStyle(GlassColor.textSecondary)
    }
}

/// The inspector while a History row is open: the row's details, in a
/// scroll of the inspector's own (Ron's inspector shows the selection
/// alone).
struct HistoryInspectorPane: View {
    let row: DaemonData.HistoryRow

    var body: some View {
        ScrollView {
            HistoryDetailInspector(row: row)
        }
        .scrollIndicators(.never)
    }
}

/// The legacy record behind a History row, by its submission id.
enum HistorySelection {
    /// The History row the inspector shows: only on History's page, and
    /// only while the opened row is still in the list. Nil otherwise, and
    /// then the inspector is the Traces selection's.
    static func opened(
        _ submissionId: String, onHistory: Bool, in rows: [DaemonData.HistoryRow]?
    ) -> DaemonData.HistoryRow? {
        guard onHistory, !submissionId.isEmpty else { return nil }
        return rows?.first { $0.submissionId == submissionId }
    }

    static func record(for submissionId: String, in history: [HistoryRecord]) -> HistoryRecord? {
        history.first { $0.submissionID == submissionId }
    }

    /// Whether the inspector's own tag says the status: until the session
    /// detail's overview, which says it from the same History table, is
    /// drawn, that is while no record resolves or its detail is unread.
    static func tagsStatus(record: HistoryRecord?, detail: SessionDetail?) -> Bool {
        record == nil || detail == nil
    }
}
