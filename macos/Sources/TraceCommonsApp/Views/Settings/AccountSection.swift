import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// Settings > Account (Settings IA, owner 2026-11-04): who you are on this
/// computer. It was #1146's Connection section (D1). In order: the account's
/// state (the attention block, the mismatch block and the connected card, as
/// before: D11 keeps both blocks able to show), the NEAR AI login (moved
/// here from Private AI), the account's contributions, its devices, the
/// legacy move, and last the ways to leave: Sign out (D3), then Set up
/// again. The source-mode lines are gone (D7): Watching holds the switches.
struct AccountSection: View {
    @EnvironmentObject private var model: AppModel
    @ObservedObject private var change = AccountLoginChangeModel.shared

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            // Where the account's state is said: a status here, never a
            // banner or notification (#1836 F1).
            AccountAttentionCard()
            // A NEAR AI sign-in the account refused as another account,
            // held aside for a change of login (F10).
            AccountLoginMismatchBlock()
            connection
            // The NEAR AI login, moved from Private AI (D8): its state and
            // Sign in, Cancel or move-from-the-keychain. Never Forget: Sign
            // out below replaces it (D3).
            if let copy = model.privateInferenceCopy {
                GlassEyebrowCard(copy.credentialTitle) {
                    CredentialSection(copy: copy, titled: false, placement: .account)
                }
            }
            ContributionAccountCard()
            // The devices linked to this account, and Link a server.
            AccountDevicesView()
            LegacyMigrationSettingsCard()
            AccountLeaveCard()
        }
    }

    private var connection: some View {
        GlassEyebrowCard(Self.eyebrow, title: connectionTitle, accessory: {
            if model.statusRead == .answered {
                GlassChip(glass: model.status.loggedIn ? SettingsLegacyWords.connectionReady
                                                       : SettingsLegacyWords.connectionLocalOnly,
                          muted: !model.status.loggedIn)
            }
        }) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                if model.statusRead != .answered {
                    SettingsReadNotice(model.statusRead, retry: model.refreshStatus)
                } else if !model.status.loggedIn {
                    Text(SettingsLegacyWords.queuedNothingSent)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textTertiary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                // Change the NEAR AI login this account uses (F10), beside
                // the connected line.
                AccountLoginChangeButton()
            }
        }
    }

    /// The card's eyebrow: the section's own name, from the core.
    static var eyebrow: String { MonitorWords.table?.settingsNav.account ?? "" }

    /// The connected line. Before the core's first answer, no title and no
    /// chip: "Not connected" would be an answer nothing gave.
    private var connectionTitle: String? {
        guard model.statusRead == .answered else { return nil }
        return Self.connectedTitle(
            loggedIn: model.status.loggedIn, accountLogin: model.status.accountLogin,
            needsAttention: model.accountAttention != nil, copy: change.copy)
    }

    /// "Connected with {provider}" (D6) while the account's login is known
    /// and the account needs no attention (F1: a login stored without the
    /// check is never presented as the account's); otherwise "Connected", or
    /// "Not connected" for an app that is not enrolled.
    static func connectedTitle(
        loggedIn: Bool, accountLogin: String?, needsAttention: Bool, copy: AccountChangeCopy?
    ) -> String {
        guard loggedIn else { return SettingsLegacyWords.notConnected }
        if !needsAttention, let copy, let line = copy.render(copy.connected, provider: accountLogin) {
            return line
        }
        return SettingsLegacyWords.connected
    }
}

/// The account section's foot: leaving this computer's account. "Sign out"
/// (D3) with the line that says what it does above it, as Forget's was, then
/// #1912's "Set up again" behind its confirmation. Sign out is drawn while
/// there is something to sign out of: an account session or a NEAR AI login.
struct AccountLeaveCard: View {
    @EnvironmentObject private var model: AppModel
    @ObservedObject private var change = AccountLoginChangeModel.shared

    private var signedInSomewhere: Bool {
        model.accountSession == .signedIn
            || CredentialSurface.action(model.credentialStatus, calls: model.credentialCalls) == .forget
    }

    private var showsSetUpAgain: Bool {
        SetUpAgainSurface.showsInAccountSection(
            loggedIn: model.status.loggedIn, needsAttention: model.accountAttention != nil)
            && model.setUpAgainCopy != nil
    }

    var body: some View {
        let signOut = signedInSomewhere ? change.copy : nil
        if signOut != nil || showsSetUpAgain {
            GlassCard {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                    if let copy = signOut {
                        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                            Text(copy.signOutBody)
                                .glassType(GlassTokens.TypeScale.body)
                                .foregroundStyle(GlassColor.textPrimary)
                                .fixedSize(horizontal: false, vertical: true)
                            Button(copy.signOut) { Task { await model.signOut() } }
                                .buttonStyle(GlassButtonStyle(.glass))
                                .disabled(model.signingOut)
                            // The account section's approved refusal ("Nothing
                            // changed. Try again."), as Set up again says it.
                            if model.signOutFailed, let failed = model.setUpAgainCopy?.failed {
                                Text(failed)
                                    .glassType(GlassTokens.TypeScale.caption)
                                    .foregroundStyle(GlassColor.textSecondary)
                            }
                        }
                    }
                    // Clear this computer's sign-in and start over (owner,
                    // 2026-11-04), behind the attention block's confirmation.
                    SetUpAgainControl()
                }
            }
            .task { if model.accountSession == .unread { model.refreshAccountSession() } }
        }
    }
}

/// The offer to move a legacy invite identity to the contributor's NEAR AI
/// account, for as long as the daemon makes it, whether or not the card
/// above the tabs was dismissed. Every word is the core's
/// (`AppModel.legacyMigrationOffer`); nothing is drawn without it.
struct LegacyMigrationSettingsCard: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        if model.isLegacyMigrationOfferDrawn, let offer = model.legacyMigrationOffer {
            GlassCard {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                    Text(offer.title)
                        .glassType(GlassTokens.TypeScale.title)
                        .foregroundStyle(GlassColor.textPrimary)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityAddTraits(.isHeader)
                    Text(offer.body)
                        .glassType(GlassTokens.TypeScale.body)
                        .foregroundStyle(GlassColor.textPrimary)
                        .fixedSize(horizontal: false, vertical: true)
                    LegacyMigrationMoveControls(offer: offer, placement: .settings)
                }
            }
        }
    }
}

/// Account contribution readiness and invite redemption. Every word is the
/// core's (`account_contribution` in the contributor crate, carried in the
/// Private AI copy); the card draws nothing until that copy has arrived.
struct ContributionAccountCard: View {
    @EnvironmentObject private var model: AppModel
    @State private var inviteCode = ""

    var body: some View {
        if let copy = model.privateInferenceCopy,
           let heading = copy.accountContributionHeading,
           let refresh = copy.accountContributionRefreshAction,
           let inviteLabel = copy.accountContributionInviteCode,
           let redeem = copy.accountContributionRedeemAction {
            GlassEyebrowCard(heading) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                    Text(model.contributionLine)
                        .glassType(GlassTokens.TypeScale.caption)
                        .accessibilityAddTraits(.updatesFrequently)
                    Button(refresh) { Task { await model.updateContributionAccount() } }
                        .buttonStyle(GlassButtonStyle(.glass))
                    GlassTextField(inviteLabel, text: $inviteCode, secure: true)
                        .accessibilityLabel(inviteLabel)
                    Button(redeem) {
                        Task {
                            if await model.updateContributionAccount(inviteCode: inviteCode) {
                                inviteCode = ""
                            }
                        }
                    }
                    .buttonStyle(GlassButtonStyle(.glass))
                    .disabled(inviteCode.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    if let pending = copy.accountContributionPendingCredit {
                        Text(pending)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textSecondary)
                    }
                }
            }
            .disabled(model.contributionBusy)
        }
    }
}

/// This app's account needs attention (#1836 F1): its record cannot be read,
/// or the address it was set up with is refused. Every word is the Rust's
/// (`AppModel.accountAttention`). The button asks first; only the
/// confirmation's own button unenrolls, then the first run window opens.
struct AccountAttentionCard: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        if let attention = model.accountAttention {
            AccountAttentionBlock(
                attention: attention, busy: model.settingUpAgain, failed: model.setUpAgainFailed
            ) {
                Task {
                    if await model.setUpAgain() {
                        openWindow(id: WindowID.firstRun)
                    }
                }
            }
        }
    }
}

/// The block itself, separate from the model so a test can lay it out.
struct AccountAttentionBlock: View {
    let attention: AccountAttention
    let busy: Bool
    let failed: Bool
    /// Runs only from the confirmation's own button.
    let setUpAgain: () -> Void

    /// The lines under the title, in order: the reason, then Private AI,
    /// then the refusal while the last attempt failed.
    static func lines(_ attention: AccountAttention, failed: Bool = false) -> [String] {
        [attention.body, attention.privateAI, failed ? attention.failed : nil].compactMap { $0 }
    }

    var body: some View {
        GlassEyebrowCard(attention.title, title: attention.title) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                ForEach(Self.lines(attention, failed: failed), id: \.self) { line in
                    Text(line)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                SetUpAgainButton(
                    action: attention.action, confirmTitle: attention.confirmTitle,
                    confirmBody: attention.confirmBody, confirmAction: attention.confirmAction,
                    confirmCancel: attention.confirmCancel, busy: busy, setUpAgain: setUpAgain)
            }
        }
    }
}

/// "Set up again" and the confirmation it opens, one control wherever it is
/// drawn, so the attention block and the account section ask the same way.
/// Only the confirmation's own button runs `setUpAgain`.
struct SetUpAgainButton: View {
    let action: String
    let confirmTitle: String
    let confirmBody: String
    let confirmAction: String
    let confirmCancel: String
    let busy: Bool
    let setUpAgain: () -> Void
    @State private var confirming = false

    var body: some View {
        Button(action) { confirming = true }
            .buttonStyle(GlassButtonStyle(.glass))
            .disabled(busy)
            .alert(confirmTitle, isPresented: $confirming) {
                Button(confirmAction, role: .destructive, action: setUpAgain)
                Button(confirmCancel, role: .cancel) {}
            } message: {
                Text(confirmBody)
            }
    }
}

/// The account section's own "Set up again" (owner, 2026-11-04): clears
/// this computer's sign-in -- the enrollment, the account session, the NEAR
/// AI login and any change of login in progress -- then opens the first
/// run. Drawn on an enrolled app whose account needs no attention
/// (`SetUpAgainSurface`); every word is the Rust's.
struct SetUpAgainControl: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        if SetUpAgainSurface.showsInAccountSection(
            loggedIn: model.status.loggedIn, needsAttention: model.accountAttention != nil),
            let copy = model.setUpAgainCopy
        {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                SetUpAgainButton(
                    action: copy.action, confirmTitle: copy.confirmTitle, confirmBody: copy.confirmBody,
                    confirmAction: copy.confirmAction, confirmCancel: copy.confirmCancel,
                    busy: model.settingUpAgain
                ) {
                    Task {
                        if await model.setUpAgain() {
                            openWindow(id: WindowID.firstRun)
                        }
                    }
                }
                if model.setUpAgainFailed {
                    Text(copy.failed)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                }
            }
        }
    }
}
