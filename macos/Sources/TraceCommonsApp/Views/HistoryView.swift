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
            VStack(alignment: .leading, spacing: 16) {
                if let rollup = model.rollup {
                    groups(rollup)
                    if rollup.quarantined > 0 {
                        quarantine(rollup)
                    }
                    credit(rollup)
                } else {
                    Text("Nothing yet.").foregroundStyle(.secondary)
                }

                if !model.history.isEmpty {
                    Divider()
                    Text("Everything you've contributed").font(.headline)
                    ForEach(model.history) { record in
                        HistoryRow(record: record)
                    }
                }
            }
            .padding(18)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func groups(_ rollup: HistoryRollup) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            row("checkmark.circle", "In the commons", rollup.allTime.accepted)
            row("clock", "Being reviewed for privacy", rollup.quarantined)
            row("circle.dotted", "Waiting to be scored", rollup.allTime.submitted)
        }
    }

    private func row(_ symbol: String, _ title: String, _ count: Int) -> some View {
        HStack(spacing: 10) {
            Image(systemName: symbol).foregroundStyle(.secondary)
            Text(title)
            Spacer()
            Text("\(count)").monospacedDigit()
        }
        .font(.callout)
        .frame(maxWidth: 420, alignment: .leading)
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
            Text("Held for privacy review — ^[\(rollup.quarantined) trace](inflect: true)")
                .font(.headline)
        }
    }

    private func credit(_ rollup: HistoryRollup) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Divider()
            Text("About credit.").font(.headline)
            Text("""
            Contributions earn credit points, scored on how novel and \
            information-rich a trace is. Today credit is a record, not a currency: \
            there is no payout, no token, no exchange rate, and no date. The intent \
            is that credit eventually settles to something real, and if it does it \
            will settle from this record. Contribute because you want the commons to \
            exist.
            """)
            .font(.callout)
            .foregroundStyle(.secondary)
            .frame(maxWidth: 560, alignment: .leading)

            if rollup.lastRefreshedAt == nil {
                // Never a confident 0.0 for a number that was never fetched.
                Text("Not synced yet").font(.callout)
            } else {
                HStack(spacing: 24) {
                    VStack(alignment: .leading) {
                        Text("Final").font(.caption).foregroundStyle(.secondary)
                        Text(String(format: "%.1f", rollup.creditFinal)).monospacedDigit()
                    }
                    VStack(alignment: .leading) {
                        Text("Still being scored").font(.caption).foregroundStyle(.secondary)
                        Text(String(format: "%.1f", rollup.creditPending)).monospacedDigit()
                    }
                }
            }
        }
    }
}

struct HistoryRow: View {
    let record: HistoryRecord

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 8) {
                Text(record.projectLabel).font(.callout.weight(.semibold))
                Text(Format.when(record.submittedAt)).foregroundStyle(.secondary)
                Spacer()
                Text(statusSentence).foregroundStyle(.secondary)
            }
            .font(.callout)
            ForEach(Array(record.explanations.enumerated()), id: \.offset) { _, text in
                Text(text).font(.caption).foregroundStyle(.secondary)
            }
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.quaternary.opacity(0.3), in: RoundedRectangle(cornerRadius: 8))
    }

    private var statusSentence: String {
        switch record.status {
        case "accepted": return "In the commons"
        case "quarantined": return "Held for privacy review"
        case "submitted": return "Waiting to be scored"
        default: return "Not in the commons"
        }
    }
}
