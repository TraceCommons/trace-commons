import SwiftUI

/// "About credit." -- shown on first run and again in History, per the
/// shared design spec
/// (`docs/superpowers/specs/2026-08-08-contributor-shell-shared-design.md`,
/// "## Credit, framed honestly"). Copy is verbatim.
///
/// Credit is a **record**, not a currency: no currency symbol, no fiat
/// estimate, no projection, no date, no streaks, no leaderboards, no
/// progress rings. The audience is developers giving away work product;
/// gamifying it insults them and makes the speculative framing look like
/// manipulation. `lastRefreshedAt == nil` renders as "Not synced yet", never
/// a confident `0.0`.
///
/// A single reusable view rather than two copies, since the two call sites
/// (onboarding and `HistoryView`) must never drift on this wording.
struct CreditRecordView: View {
    let creditFinal: Double
    let creditPending: Double
    let lastRefreshedAt: Date?

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.s) {
            Text("About credit.").font(TC.Font_.cardTitle)
            Text("""
            Contributions earn credit points, scored on how novel and \
            information-rich a trace is. Today credit is a record, not a currency: \
            there is no payout, no token, no exchange rate, and no date. The intent \
            is that credit eventually settles to something real, and if it does it \
            will settle from this record. Contribute because you want the commons to \
            exist.
            """)
            .font(TC.Font_.body)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: 560, alignment: .leading)

            if lastRefreshedAt == nil {
                // Never a confident 0.0 for a number that was never fetched.
                TCTag(text: "Not synced yet", tone: .neutral, symbol: "arrow.triangle.2.circlepath")
            } else {
                // Same label-over-figure shape as a queue card's manifest.
                // No currency symbol, no ring, no streak: it is a record.
                HStack(alignment: .top, spacing: TC.Space.xxl) {
                    figure("Final", creditFinal)
                    figure("Still being scored", creditPending)
                }
            }
        }
        .padding(TC.Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .tcCard()
    }

    private func figure(_ label: String, _ value: Double) -> some View {
        VStack(alignment: .leading, spacing: TC.Space.xxs) {
            TCFieldLabel(label)
            Text(String(format: "%.1f", value))
                .font(.title3.weight(.bold))
                .monospacedDigit()
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(label): \(String(format: "%.1f", value))")
    }
}
