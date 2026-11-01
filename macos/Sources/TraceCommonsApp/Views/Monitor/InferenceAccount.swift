import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// The Private AI tab's link to Settings. Balance stays on the tab; billing,
/// the standard tool settings, and the connection controls live in Settings.
/// The standard settings, the local tools and the connection (the switch,
/// then sign-in) are in that section (owner, 2026-10-10:
/// `PrivateAISettingsPanels`).
///
/// Nothing is drawn until the core's Private AI words arrive: a destination
/// missing the sentence on what turning the switch on exposes is worse than
/// none (`AppModel.privateInferenceCopy`).
struct InferenceAccountSection: View {
    let store: InferenceStore
    /// Opens the Private AI section of Settings.
    var onOpenSettings: () -> Void = {}
    @EnvironmentObject private var model: AppModel

    var body: some View {
        if let copy = model.privateInferenceCopy {
            PrivateAISettingsLinkCard(copy: copy, onOpen: onOpenSettings)
        }
    }

    /// The daemon's listener report as the card's state. Unreported is the
    /// empty label, which the core answers with the sentence that claims
    /// nothing; a port that is not a port is no port, never another one.
    static func surfaceState(_ state: DaemonData.PrivateInferenceState?) -> PrivateInferenceState {
        PrivateInferenceState(label: state?.state ?? "", port: state?.port.flatMap { UInt16(exactly: $0) })
    }

    /// The Runtime tile's word for the listener's report, read from the
    /// core's `runtime_word` (`tc_private_inference_runtime_word`), never
    /// re-implemented here: every running label is on, a stopped one is
    /// off, and an unreported or unfamiliar label is unknown, never off.
    /// A caught panic is the core's unknown word too.
    static func runtimeWord(_ state: DaemonData.PrivateInferenceState?, copy: PrivateInferenceCopy) -> String {
        TCPrivateInference.runtimeWord(state: state?.state ?? "") ?? copy.runtimeUnknown
    }
}

/// The NEAR AI balance near the top of Private AI. Billing lives in Settings.
/// Nothing is drawn until the core's Private AI words arrive.
struct InferenceBalanceSection: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        if let copy = model.privateInferenceCopy {
            PrivateAIBalanceCard(copy: copy)
        }
    }
}

/// The Private AI section of Settings (owner, 2026-10-10; they were the
/// Private AI tab's): the standard-settings heading, the local tools and
/// the connection (the switch, then sign-in). The managed cards are not
/// here: the heading says everything under it changes standard sessions,
/// and the managed cards never do. Sign-in and the tools keep their live
/// `AppModel` paths; the switch reads and writes through the data contract
/// (`InferenceStore`).
struct PrivateAISettingsPanels: View {
    let store: InferenceStore
    @EnvironmentObject private var model: AppModel

    var body: some View {
        if let copy = model.privateInferenceCopy {
            VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                // The standard tool settings below are global; managed
                // launches never edit them.
                ManagedGlobalSettingsHeader()
                GlassCard {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
                        PrivateAIPanelHeader(
                            eyebrow: copy.panelToolsEyebrow, title: copy.harnessesTitle,
                            refresh: copy.panelToolsRefreshAccessibility, onRefresh: { refreshTools() })
                        HarnessListSection(copy: copy, titled: false)
                    }
                }
                PrivateAISwitchCard(
                    copy: copy,
                    isOn: store.privateAI?.on,
                    state: InferenceAccountSection.surfaceState(store.privateAI?.state),
                    calls: model.privateInferenceCalls,
                    busy: store.privateAIBusy,
                    refusal: store.privateAIRefusal,
                    onSet: { on in
                        Task { @MainActor in
                            await store.setPrivateAI(on: on, unconfirmed: copy.writeUnconfirmed)
                            model.refreshSettings()
                        }
                    },
                    onDismiss: { store.dismissPrivateAIRefusal() },
                    onRefresh: { refreshConnection() },
                    // #1146: the sign-in is part of the connection panel.
                    credential: AnyView(CredentialSection(copy: copy, prominent: true, titled: false)))
            }
        }
    }

    private func refreshTools() {
        model.refreshHarnesses()
        Task { @MainActor in await store.load() }
    }

    private func refreshConnection() {
        model.refreshSettings()
        model.refreshNearAiCredential()
        Task { @MainActor in await store.load() }
    }
}

/// The card at the foot of the Private AI tab that opens the Private AI
/// section of Settings, where the standard settings, the local tools and
/// the connection are (owner, 2026-10-10): a medium gear on the left, the
/// title and a chevron on the right, centred on the
/// card's height. The whole card opens the section, as Home's cards do
/// (owner, 2026-10-10: a chevron, no Open button); VoiceOver hears the
/// core's name for where it goes. Every word is the core's.
struct PrivateAISettingsLinkCard: View {
    let copy: PrivateInferenceCopy
    let onOpen: () -> Void

    var body: some View {
        Button(action: onOpen) {
            GlassIconCard(
                systemImage: "gearshape", title: copy.panelSettingsTitle, subtitle: [],
                interactive: true,
                trailing: {
                    // Home's chevron (`HomeChevron`), drawn here too.
                    Image(systemName: "chevron.right")
                        .glassGlyph(10, weight: .semibold)
                        .foregroundStyle(GlassColor.textTertiary)
                        .accessibilityHidden(true)
                })
        }
        .buttonStyle(GlassPressStyle())
        .accessibilityHint(copy.panelSettingsOpenAccessibility)
    }
}

/// A Private AI panel's single heading and a re-read icon beside it (owner,
/// 2026-10-10: an icon, not a link), named for assistive tech in the
/// core's words for that panel ("Refresh your tools") so the panels' icons
/// are told apart.
struct PrivateAIPanelHeader: View {
    let eyebrow: String
    var title: String?
    /// The refresh icon's accessible name, from the core.
    let refresh: String
    var disabled = false
    let onRefresh: () -> Void

    var body: some View {
        HStack(alignment: .center, spacing: GlassTokens.Space.s4) {
            Text(title ?? eyebrow)
                .glassType(GlassTokens.TypeScale.title)
                .foregroundStyle(GlassColor.textPrimary)
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityAddTraits(.isHeader)
            Spacer(minLength: GlassTokens.Space.s4)
            GlassRefreshButton(refresh, action: onRefresh)
                .disabled(disabled)
        }
    }
}

/// #1146's `StatCard`: an eyebrow over one bold tabular value on one line.
struct PrivateAIStatCard: View {
    let label: String
    let value: String

    var body: some View {
        GlassCard(flush: true) {
            VStack(alignment: .leading, spacing: 0) {
                Text(label)
                    .glassType(GlassTokens.TypeScale.eyebrow)
                    .foregroundStyle(GlassColor.textTertiary)
                    .lineLimit(1)
                Text(value)
                    .glassType(GlassTokens.TypeScale.heading.weight(.bold))
                    .monospacedDigit()
                    .foregroundStyle(GlassColor.textPrimary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .help(value)
            }
            .padding(.vertical, 10)
            .padding(.horizontal, 12)
        }
        .frame(maxWidth: .infinity)
        .accessibilityElement(children: .combine)
    }
}

/// #1146's `PrivateAiBalancePanel`, drawn in the main pane and in the
/// inspector: the balance eyebrow and heading with its own re-read link, then
/// the balance row's sentence or figures. When the NEAR AI login is not
/// usable it also draws the Private AI tab's Sign in, which opens the browser
/// (the Settings credential card keeps its own, with the provider chooser).
struct PrivateAIBalanceCard: View {
    @EnvironmentObject private var model: AppModel
    let copy: PrivateInferenceCopy

    /// Under its icon (owner, 2026-10-10): the heading with its re-read
    /// icon and the state or the figures under it.
    var body: some View {
        GlassIconCard(
            systemImage: "dollarsign.circle", title: copy.balanceTitle,
            refresh: .init(
                copy.panelBalanceRefreshAccessibility,
                isDisabled: model.credentialBusy, isRefreshing: model.balanceRefreshing,
                action: { model.refreshNearAiBalanceFromButton() }),
            content: {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                    BalanceRow(copy: copy)
                    // To billing, verified, as Settings' Manage billing
                    // (owner, 2026-10-27); hidden while a sign-in is needed.
                    FundingRow(copy: copy, style: .addFunds)
                    NearAiSignInControl(copy: copy)
                }
            })
    }
}
