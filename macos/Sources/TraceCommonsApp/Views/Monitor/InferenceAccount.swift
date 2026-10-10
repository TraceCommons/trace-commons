import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// The Private AI tab's account cards, after the Private AI summary and the
/// calls: saved model accounts and managed sessions, the balance and the
/// funding, then the card that opens the Private AI section of Settings.
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
            VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                // Saved model accounts and managed sessions, which #1146
                // has no panel for (owner ruling O3: in the main pane).
                ManagedSessionsSection()
                PrivateAIBalanceCard(copy: copy)
                GlassCard { FundingRow(copy: copy) }
                PrivateAISettingsLinkCard(copy: copy, onOpen: onOpenSettings)
            }
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
                            refresh: copy.panelRefresh, onRefresh: { refreshTools() })
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
/// title over its sentence, and the button on the right, centred on the
/// card's height. Every word is the core's.
struct PrivateAISettingsLinkCard: View {
    let copy: PrivateInferenceCopy
    let onOpen: () -> Void

    /// The gear's size: medium, between a row glyph and a tile.
    static let iconSize: CGFloat = 22

    var body: some View {
        GlassCard {
            HStack(alignment: .center, spacing: GlassTokens.Space.s5) {
                Image(systemName: "gearshape")
                    .glassGlyph(Self.iconSize, weight: .regular)
                    .foregroundStyle(GlassColor.textSecondary)
                    .frame(width: Self.iconSize + 4)
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                    Text(copy.panelSettingsTitle)
                        .glassType(GlassTokens.TypeScale.bodyStrong)
                        .foregroundStyle(GlassColor.textPrimary)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityAddTraits(.isHeader)
                    Text(copy.panelSettingsBody)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: GlassTokens.Space.s4)
                Button(copy.panelSettingsOpen, action: onOpen)
                    .buttonStyle(GlassButtonStyle(.glass, small: true))
                    .lineLimit(1)
                    .fixedSize()
                    .accessibilityLabel(copy.panelSettingsOpenAccessibility)
            }
        }
    }
}

/// A Private AI panel's header, as #1146's panels draw it: an eyebrow over
/// the panel's heading, and a re-read icon on the right (owner,
/// 2026-10-10: an icon, not a link), named "Refresh <heading>" for
/// assistive tech so the panels' icons are told apart.
struct PrivateAIPanelHeader: View {
    let eyebrow: String
    var title: String?
    let refresh: String
    var disabled = false
    let onRefresh: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: GlassTokens.Space.s4) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                Text(eyebrow)
                    .glassType(GlassTokens.TypeScale.eyebrow)
                    .foregroundStyle(GlassColor.textTertiary)
                if let title {
                    Text(title)
                        .glassType(GlassTokens.TypeScale.title)
                        .foregroundStyle(GlassColor.textPrimary)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityAddTraits(.isHeader)
                }
            }
            Spacer(minLength: GlassTokens.Space.s4)
            GlassRoundButton(Self.refreshName(refresh, title: title ?? eyebrow), systemImage: "arrow.clockwise",
                             small: true, action: onRefresh)
                .disabled(disabled)
        }
    }
}

extension PrivateAIPanelHeader {
    /// The refresh icon's name: the core's "Refresh" and the panel it
    /// re-reads.
    static func refreshName(_ refresh: String, title: String) -> String {
        title.isEmpty ? refresh : refresh + " " + title
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
/// the balance row's sentence or figures. Its sign-in button is the
/// credential card's, never a second one here.
struct PrivateAIBalanceCard: View {
    @EnvironmentObject private var model: AppModel
    let copy: PrivateInferenceCopy

    var body: some View {
        GlassCard {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
                PrivateAIPanelHeader(
                    eyebrow: copy.panelBalanceEyebrow, title: copy.balanceTitle,
                    refresh: copy.panelBalanceRefresh, disabled: model.credentialBusy,
                    onRefresh: { model.refreshNearAiBalance() })
                BalanceRow(copy: copy)
            }
        }
    }
}
