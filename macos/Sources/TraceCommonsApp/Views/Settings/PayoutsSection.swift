import SwiftUI
import TCDesign
import TCShellCore

/// Settings > Payouts (Settings IA, owner 2026-11-04, D2): where NEAR AI
/// credit is paid. The payout link, then, for a wallet-origin account, the
/// approved "Payouts need a NEAR AI login" line (F10 entry point 3), "where
/// the link status would be". Both moved here from Private AI unchanged.
/// The section tells the window whether anything in it applies, and the
/// window lists its row only while something does.
struct PayoutsSection: View {
    @EnvironmentObject private var model: AppModel
    /// Whether anything is drawn: the window lists the row only then.
    var onPresence: (Bool) -> Void = { _ in }
    @State private var linkVisible = false

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            PayoutLinkView(onVisibility: { linkVisible = $0 })
            // A wallet-origin account's payouts need a NEAR AI login (F10):
            // the line and its button, where the link status would be.
            PayoutsNeedLoginLine()
        }
        .onChange(of: Self.present(linkVisible: linkVisible, loginLine: model.status.loginChange.showsPayoutsLine),
                  initial: true) { _, present in onPresence(present) }
    }

    /// Whether the section has anything to show: the payout link's card, or
    /// the payouts-need-a-login line.
    static func present(linkVisible: Bool, loginLine: Bool) -> Bool {
        linkVisible || loginLine
    }
}
