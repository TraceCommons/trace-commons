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
        VStack(alignment: .leading, spacing: 6) {
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

            if lastRefreshedAt == nil {
                // Never a confident 0.0 for a number that was never fetched.
                Text("Not synced yet").font(.callout)
            } else {
                HStack(spacing: 24) {
                    VStack(alignment: .leading) {
                        Text("Final").font(.caption).foregroundStyle(.secondary)
                        Text(String(format: "%.1f", creditFinal)).monospacedDigit()
                    }
                    VStack(alignment: .leading) {
                        Text("Still being scored").font(.caption).foregroundStyle(.secondary)
                        Text(String(format: "%.1f", creditPending)).monospacedDigit()
                    }
                }
            }
        }
    }
}
