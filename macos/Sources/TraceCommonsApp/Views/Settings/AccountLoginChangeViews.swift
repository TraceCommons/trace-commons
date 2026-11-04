import SwiftUI
import TCDesign
import TCShellCore

// Changing the NEAR AI login an account uses (F10, spec section 2): the
// three entry points and the three-step sheet. Every sentence is the core's
// approved copy (`AccountChangeCopy`); the daemon runs the ceremony.

/// Raises the change sheet over this view. Put it on each section that
/// shows an entry point, inside the window's `glassModalHost()`.
struct AccountLoginChangePresenter: ViewModifier {
    @EnvironmentObject private var app: AppModel
    @ObservedObject private var model = AccountLoginChangeModel.shared

    func body(content: Content) -> some View {
        content
            .glassModal(isPresented: Binding(
                get: { model.presented && model.page != nil },
                set: { if !$0 { model.cancel() } })
            ) {
                if let page = model.page {
                    AccountLoginChangeSheet(model: model, page: page)
                }
            }
            .task(id: app.payoutLinkDaemonClient.map(ObjectIdentifier.init)) {
                model.attach(
                    app.payoutLinkDaemonClient,
                    openBrowser: { [weak app] url in app?.openBrowser(url) },
                    onFinished: { [weak app] in app?.refreshStatus() })
            }
    }
}

/// One page of the sheet: its step's heading and lines, the wallet field in
/// step 1 of a wallet-origin account, the step's refusal, and Cancel beside
/// the step's button.
struct AccountLoginChangeSheet: View {
    @EnvironmentObject private var app: AppModel
    @ObservedObject var model: AccountLoginChangeModel
    let page: AccountLoginChangePage

    private var working: Bool { page.busy || model.calling }

    var body: some View {
        GlassModal(
            title: page.title, width: .narrow,
            actions: [
                .cancel(page.cancel) { model.cancel() },
                GlassModalAction(
                    page.primary, isDefault: page.action != .confirm,
                    isEnabled: !working && (!page.asksWalletAccount || !walletAccountEmpty),
                    // Switch is never one Return away: it moves the account.
                    isProminent: page.action == .confirm,
                    accessibilityName: page.primaryAccessibilityName
                ) {
                    Task { await model.primary() }
                },
            ],
            busy: model.calling,
            onCancel: { model.cancel() }
        ) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                if let heading = page.heading {
                    Text(heading)
                        .glassType(GlassTokens.TypeScale.bodyStrong)
                        .foregroundStyle(GlassColor.textPrimary)
                }
                ForEach(page.lines, id: \.self) { line in
                    Text(line)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                // The NEAR account name, typed as at sign-up; its label is
                // the wallet join's own.
                if page.asksWalletAccount, let label = app.witnessCopy?.wallet?.account {
                    GlassTextField(label, text: $model.walletAccount).disabled(working)
                }
                if working { GlassSpinner(standalone: true) }
                if let error = page.error {
                    Text(error)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
    }

    private var walletAccountEmpty: Bool {
        model.walletAccount.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }
}

/// Entry point 1 (spec 2.1): after a NEAR AI sign-in the account refused as
/// another account. The approved mismatch body and Sign in (with the
/// account's own login), and -- only while that login is held aside for a
/// change -- Use a different account (an approved notice, spec 2.6). A
/// status in the account section, never a banner or notification.
struct AccountLoginMismatchBlock: View {
    @EnvironmentObject private var app: AppModel
    @ObservedObject private var model = AccountLoginChangeModel.shared

    var body: some View {
        if app.status.loginChange.mismatch, let copy = model.copy {
            GlassCard {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                    Text(copy.mismatchBody)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                    HStack(spacing: GlassTokens.Space.s3) {
                        Button(copy.mismatchSignIn) {
                            Task {
                                if let url = await app.startNearAiCredential() { app.openBrowser(url) }
                            }
                        }
                        .buttonStyle(GlassButtonStyle(.glass))
                        if app.status.loginChange.offersChange {
                            Button(copy.mismatchChange) { Task { await model.begin() } }
                                .buttonStyle(GlassButtonStyle(.glass))
                                .disabled(model.calling)
                        }
                    }
                    if model.startFailed {
                        Text(copy.errorUnavailable)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textSecondary)
                    }
                }
            }
        }
    }
}

/// Entry point 2 (spec 2.1): deliberately, from the account section and
/// from the Private AI section under its connection card, by one rule
/// (`AccountLoginChangeStatus.entry`). Drawn while the change is offered
/// and no refused login is held; with one held, the account section's
/// mismatch block offers it, and Private AI, which has no such block, draws
/// the mismatch's button here instead (`offersMismatchChange`).
struct AccountLoginChangeButton: View {
    @EnvironmentObject private var app: AppModel
    @ObservedObject private var model = AccountLoginChangeModel.shared
    /// Also draw the mismatch's "Use a different account" while a refused
    /// login is held. Only where no mismatch block is drawn.
    var offersMismatchChange = false
    /// Drawn as its own card, where it stands between cards.
    var carded = false

    var body: some View {
        if let entry = app.status.loginChange.entry(loggedIn: app.status.loggedIn),
            offersMismatchChange || entry == .change, let copy = model.copy
        {
            if carded {
                GlassCard { control(entry, copy: copy) }
            } else {
                control(entry, copy: copy)
            }
        }
    }

    private func control(_ entry: AccountLoginChangeEntry, copy: AccountChangeCopy) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            Button(copy.label(for: entry)) { Task { await model.begin() } }
                .buttonStyle(GlassButtonStyle(.glass))
                .disabled(model.calling)
            if model.startFailed {
                Text(copy.errorUnavailable)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
            }
        }
    }
}

/// Entry point 3 (spec 2.1): a wallet-origin account's payouts section,
/// where the link status would be. A status line and its button, an
/// approved notice (spec 2.6), cleared once the account records a NEAR AI
/// login.
struct PayoutsNeedLoginLine: View {
    @EnvironmentObject private var app: AppModel
    @ObservedObject private var model = AccountLoginChangeModel.shared

    var body: some View {
        if app.status.loginChange.showsPayoutsLine, let copy = model.copy {
            GlassCard {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                    Text(copy.payoutsNeedsLoginBody)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                    Button(copy.payoutsNeedsLoginAdd) { Task { await model.begin() } }
                        .buttonStyle(GlassButtonStyle(.glass))
                        .disabled(model.calling)
                    // It adds a login: its own failure line, not "switch
                    // accounts" (rule 7).
                    if model.startFailed {
                        Text(copy.payoutsNeedsLoginAddFailed)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textSecondary)
                    }
                }
            }
        }
    }
}
