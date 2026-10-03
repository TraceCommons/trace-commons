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
/// row's optional one, and does not wait for the detail read. Without the
/// core's public-run copy (static, not a daemon answer) the session detail
/// draws nothing, and nothing is offered.
struct HistoryDetailInspector: View {
    let row: DaemonData.HistoryRow
    @EnvironmentObject private var model: AppModel

    private var record: HistoryRecord? {
        HistorySelection.record(for: row.submissionId, in: model.history)
    }

    // The always-present scroll view reads the detail and the installed
    // skill for the row, again when its record first resolves or its status
    // moves. The session detail reads it again when the app comes back to
    // the front.
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                summary
                defects
                // A new row is a new detail: its editor draft, withdrawal
                // confirmation and Skills panel are the record's, and the
                // pane outlives the selection.
                if let record {
                    explanations(record)
                    SessionDetailView(record: record)
                        .id(record.submissionID)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .scrollIndicators(.never)
        .task(id: [row.submissionId, record?.status ?? ""]) { load() }
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
                    .glassType(GlassTokens.TypeScale.label.weight(.semibold))
                    .foregroundStyle(GlassColor.textPrimary)
                Text(HomeFormat.meta(row, compact: false))
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textTertiary)
            }
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

    /// Whether the inspector's own tag says the status: until the session
    /// detail's overview, which says it from the same History table, is
    /// drawn, that is while no record resolves or its detail is unread.
    static func tagsStatus(record: HistoryRecord?, detail: SessionDetail?) -> Bool {
        record == nil || detail == nil
    }
}
