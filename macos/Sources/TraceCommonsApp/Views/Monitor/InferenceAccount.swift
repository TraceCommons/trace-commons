import SwiftUI
import TCDesign
import TCShellCore

/// The Inference tab's Private AI page, in #1146's order
/// (`private-ai-page.tsx`): the subtitle, the Inference access / Runtime
/// stat pair, then the tools, the connection (the switch, then sign-in), the
/// balance and the funding panels. Sign-in and the tools keep their live
/// `AppModel` paths; the switch reads and writes through the data contract
/// (`InferenceStore`).
///
/// Nothing is drawn until the core's Private AI words arrive: a destination
/// missing the sentence on what turning the switch on exposes is worse than
/// none (`AppModel.privateInferenceCopy`).
struct InferenceAccountSection: View {
    let store: InferenceStore
    @EnvironmentObject private var model: AppModel

    var body: some View {
        if let copy = model.privateInferenceCopy {
            VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                Text(copy.subtitle)
                    .glassType(GlassTokens.TypeScale.label.weight(.regular))
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                HStack(spacing: GlassTokens.Space.s3) {
                    PrivateAIStatCard(
                        label: copy.statInferenceAccess,
                        value: CredentialSurface.stateLine(model.credentialStatus, copy: copy, calls: model.credentialCalls))
                    PrivateAIStatCard(
                        label: copy.statRuntime,
                        value: Self.runtimeWord(store.privateAI?.state, copy: copy))
                }
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
                    state: Self.surfaceState(store.privateAI?.state),
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
                    onRefresh: { refreshConnection() })
                GlassCard { CredentialSection(copy: copy, prominent: true) }
                PrivateAIBalanceCard(copy: copy)
                GlassCard { FundingRow(copy: copy) }
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

    /// The daemon's listener report as the card's state. Unreported is the
    /// empty label, which the core answers with the sentence that claims
    /// nothing; a port that is not a port is no port, never another one.
    static func surfaceState(_ state: DaemonData.PrivateInferenceState?) -> PrivateInferenceState {
        PrivateInferenceState(label: state?.state ?? "", port: state?.port.flatMap { UInt16(exactly: $0) })
    }

    /// The Runtime tile's word for the listener's report, as the core's
    /// `runtime_word` picks it: every running label is on, a stopped one is
    /// off, and an unreported or unfamiliar label is unknown, never off.
    static func runtimeWord(_ state: DaemonData.PrivateInferenceState?, copy: PrivateInferenceCopy) -> String {
        switch state?.state ?? "" {
        case "off": copy.runtimeOff
        case "stopping": copy.runtimeStopping
        case "running", "running_no_backends", "running_answered_elsewhere", "running_destination_unknown":
            copy.runtimeOn
        case "running_elsewhere": copy.runtimeElsewhere
        case "port_in_use", "start_failed", "crashed": copy.runtimeNotRunning
        default: copy.runtimeUnknown
        }
    }
}

/// A Private AI panel's header, as #1146's panels draw it: an eyebrow over
/// the panel's heading, and a re-read link on the right that keeps to one
/// line at its own width.
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
            Button(action: onRefresh) { Text(refresh).lineLimit(1) }
                .buttonStyle(GlassButtonStyle(.link))
                .fixedSize()
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
