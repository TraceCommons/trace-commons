import SwiftUI

/// Three groups, never one column of mixed semantics.
///
/// Quarantine reads as held, not rejected, and never states a turnaround
/// time. Credit is a record: no currency symbol, no fiat estimate, no
/// projection, no date, no streaks, no leaderboards.
struct HistoryView: View {
    @EnvironmentObject private var model: AppModel
    @State private var quarantineExpanded = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: TC.Space.xl) {
                if let rollup = model.rollup {
                    groups(rollup)
                    if rollup.quarantined > 0 {
                        quarantine(rollup)
                    }
                    credit(rollup)
                } else {
                    Text("Nothing yet.")
                        .font(TC.Font_.meta)
                        .foregroundStyle(.secondary)
                }

                if !model.history.isEmpty {
                    VStack(alignment: .leading, spacing: TC.Space.m) {
                        TCSectionHeader(
                            title: "Everything you've contributed",
                            trailing: "\(model.history.count)"
                        )
                        ForEach(model.history) { record in
                            HistoryRow(record: record)
                        }
                    }
                }
            }
            .padding(.horizontal, TC.Space.xxl)
            .padding(.vertical, TC.Space.xl)
            .tcColumn()
        }
        .tcScreen()
    }

    /// Three states, three tones, three glyphs, three words. The counts are
    /// the same shape as a queue card's manifest -- uppercase label over a
    /// monospaced figure -- so the two screens read as one system.
    private func groups(_ rollup: HistoryRollup) -> some View {
        HStack(alignment: .top, spacing: TC.Space.m) {
            tally("In the commons", rollup.allTime.accepted, .clear, "checkmark.circle")
            tally("Held for privacy review", rollup.quarantined, .held, "clock")
            tally("Waiting to be scored", rollup.allTime.submitted, .neutral, "circle.dotted")
        }
    }

    private func tally(_ title: String, _ count: Int, _ tone: TC.Tone, _ symbol: String) -> some View {
        VStack(alignment: .leading, spacing: TC.Space.s) {
            HStack(spacing: TC.Space.xs) {
                Image(systemName: symbol)
                    .imageScale(.small)
                    .foregroundStyle(tone.textColor)
                    .accessibilityHidden(true)
                TCFieldLabel(title)
            }
            Text("\(count)")
                .font(.title2.weight(.bold))
                .monospacedDigit()
        }
        .padding(TC.Space.m)
        .frame(maxWidth: .infinity, alignment: .leading)
        .tcCard()
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(title): \(count)")
    }

    private func quarantine(_ rollup: HistoryRollup) -> some View {
        DisclosureGroup(isExpanded: $quarantineExpanded) {
            VStack(alignment: .leading, spacing: 8) {
                Text("""
                A person at Trace Commons reads these before they enter the commons. \
                It happens when automated checks see something that might be personal \
                or sensitive and can't decide on its own.
                """)
                Text("""
                These have not been rejected, and they have not been shared with \
                anyone but the reviewer. They are sitting still.
                """)
                .fontWeight(.semibold)
                // Never state a turnaround time that cannot be honoured.
                Text("Typical wait: we don't have a reliable number yet.")
                    .foregroundStyle(.secondary)

                let explanations = model.history
                    .filter { $0.status == "quarantined" }
                    .flatMap(\.explanations)
                if !explanations.isEmpty {
                    // Rendered verbatim: the server's own prose beats a
                    // status word every time.
                    ForEach(Array(explanations.enumerated()), id: \.offset) { _, text in
                        Text(text).font(.callout).foregroundStyle(.secondary)
                    }
                }

                Button("Withdraw these traces") {}
                    .disabled(true)
                    .help("Withdrawal is not wired up in this build.")
                Text("""
                Withdraw is not available in this build yet. It is the next thing \
                this screen needs, and it is not being hidden from you.
                """)
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            .font(.callout)
            .frame(maxWidth: 560, alignment: .leading)
            .padding(.top, 8)
        } label: {
            // Held is a state, not a colour. It says "held", it carries a
            // clock, and it is tinted -- in that order of importance.
            HStack(spacing: TC.Space.s) {
                Image(systemName: TC.Tone.held.symbol)
                    .foregroundStyle(TC.Tone.held.textColor)
                    .accessibilityHidden(true)
                Text("Held for privacy review — ^[\(rollup.quarantined) trace](inflect: true)")
                    .font(TC.Font_.cardTitle)
            }
        }
    }

    private func credit(_ rollup: HistoryRollup) -> some View {
        VStack(alignment: .leading, spacing: TC.Space.m) {
            TCSectionHeader(title: "Credit")
            CreditRecordView(
                creditFinal: rollup.creditFinal,
                creditPending: rollup.creditPending,
                lastRefreshedAt: rollup.lastRefreshedAt
            )
        }
    }
}

struct HistoryRow: View {
    let record: HistoryRecord

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.xs) {
            HStack(spacing: TC.Space.s) {
                Text(record.projectLabel).font(TC.Font_.body.weight(.semibold))
                Text(Format.when(record.submittedAt))
                    .font(TC.Font_.footnote)
                    .foregroundStyle(.tertiary)
                Spacer(minLength: TC.Space.m)
                TCTag(text: statusSentence, tone: statusTone)
            }
            ForEach(Array(record.explanations.enumerated()), id: \.offset) { _, text in
                Text(text)
                    .font(TC.Font_.footnote)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(TC.Space.m)
        .frame(maxWidth: .infinity, alignment: .leading)
        .tcCard()
    }

    private var statusSentence: String {
        switch record.status {
        case "accepted": return "In the commons"
        case "quarantined": return "Held for privacy review"
        case "submitted": return "Waiting to be scored"
        default: return "Not in the commons"
        }
    }

    private var statusTone: TC.Tone {
        switch record.status {
        case "accepted": return .clear
        case "quarantined": return .held
        case "submitted": return .neutral
        default: return .refused
        }
    }
}
