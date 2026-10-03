import SwiftUI
import TCDesign
import TCShellCore

/// The indicator every surface on this destination is painted from.
///
/// One place, deliberately. The sidebar row, the menu-bar section and this
/// screen's own status line all want the same answer, and the dangerous way
/// to get it is to reach for `daemonSettings?.privateInferenceOn` -- the
/// switch, which says what was ASKED FOR. A listener that refused to start
/// leaves that switch on. Everything here derives from the tone the shared
/// table answers with, so "on" and "working" can never be confused.
///
/// It holds no words. Every sentence on this destination comes from
/// `PrivateInferenceCopy`, which is composed in the Rust contributor crate.
enum PrivateInferenceIndicator {
    /// Whether an indicator may be painted as working. `Clear` alone; see
    /// `PrivateInferenceTone.readsAsWorking`.
    static func readsAsWorking(
        _ state: PrivateInferenceState, calls: PrivateInferenceCalls
    ) -> Bool {
        PrivateInferenceSurface.tone(state, calls: calls).readsAsWorking
    }

    /// The tone onto a glass status, as the window's Inference dot maps
    /// it (`MonitorWindowView.inferenceDot`): only clear is on; held,
    /// attention and refused all ask; neutral is off. A glass status is a dot,
    /// so whoever draws it draws the core's sentence beside it.
    static func status(_ tone: PrivateInferenceTone) -> GlassStatus {
        switch tone {
        case .clear: return .on
        case .held, .attention, .refused: return .ask
        case .neutral: return .off
        }
    }
}

/// Answering model calls on this computer: a destination of its own rather
/// than a card near the bottom of Settings.
///
/// Renders nothing at all if the words did not arrive, for the reason
/// `AppModel.privateInferenceCopy` gives: a screen missing the sentence
/// about what turning the switch on exposes is worse than no screen.
struct PrivateInferenceView: View {
    var body: some View {
        ScrollView {
            PrivateInferenceContent()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }
}

/// The screen's content, split out of its `ScrollView` for the same reason
/// `QueueContent` is: `ImageRenderer` renders a
/// `ScrollView` as blank, so the screenshot hook can only rasterize what
/// lives outside one.
struct PrivateInferenceContent: View {
    @EnvironmentObject private var model: AppModel

    /// The same narrow prose column Settings uses. This screen is three
    /// paragraphs and a switch; the full window width would set them at a
    /// measure nobody reads.
    private static let proseColumn: CGFloat = 640

    var body: some View {
        if let copy = model.privateInferenceCopy {
            content(copy)
        }
    }

    private func content(_ copy: PrivateInferenceCopy) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            // Sign-in comes first so setup starts with the account needed
            // to connect tools. The section shows account controls once signed in.
            GlassCard { CredentialSection(copy: copy, prominent: true) }
            GlassCard { HarnessListSection(copy: copy) }
            // The switch, below the list and unchanged: a kill switch, which
            // is what it always was. A switch the daemon has not reported
            // (`privateInference == nil`) is nil here, and the card draws it
            // off and disabled.
            PrivateAISwitchCard(
                copy: copy,
                isOn: model.daemonSettings?.privateInference,
                state: model.privateInferenceState,
                calls: model.privateInferenceCalls,
                busy: model.privateInferenceBusy,
                refusal: model.lastActionError,
                onSet: model.applyPrivateInference,
                onDismiss: { model.lastActionError = nil })
        }
        .padding(GlassTokens.Space.s10)
        .frame(maxWidth: Self.proseColumn, alignment: .leading)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// The Private AI switch, with the core's sentence on what turning it on
/// exposes beside it, drawn by the legacy destination on `AppModel` and by
/// the glass Inference tab on `InferenceStore`.
///
/// The switch says what was asked for; the state line says what happened,
/// and it is drawn from the core's tone -- never from `isOn`, which stays on
/// over a listener that refused to start. `isOn` nil is a switch nobody
/// could read: drawn off and disabled, never on. A write that was not
/// confirmed is `refusal`, in the core's words, outside the expander so a
/// collapsed card still shows it.
struct PrivateAISwitchCard: View {
    let copy: PrivateInferenceCopy
    let isOn: Bool?
    let state: PrivateInferenceState
    let calls: PrivateInferenceCalls
    let busy: Bool
    let refusal: String?
    let onSet: (Bool) -> Void
    let onDismiss: () -> Void

    @State private var isOpen = false

    /// The state line and its dot, from the listener's report alone. It
    /// takes no switch: what was asked for never says what happened.
    static func stateLabel(
        state: PrivateInferenceState, copy: PrivateInferenceCopy, calls: PrivateInferenceCalls
    ) -> (line: String, status: GlassStatus) {
        (PrivateInferenceSurface.stateLine(state, copy: copy, calls: calls),
         PrivateInferenceIndicator.status(PrivateInferenceSurface.tone(state, calls: calls)))
    }

    var body: some View {
        let label = Self.stateLabel(state: state, copy: copy, calls: calls)
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            GlassCard {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
                    GlassExpander(copy.settingsTitle, isOpen: $isOpen)
                    GlassStatusLabel(label.line, status: label.status)
                        .fixedSize(horizontal: false, vertical: true)
                    if isOpen {
                        Text(copy.offerWhat)
                            .glassType(GlassTokens.TypeScale.body)
                            .foregroundStyle(GlassColor.textPrimary)
                            .fixedSize(horizontal: false, vertical: true)
                        // The exposure paragraph in full, on the destination as well as
                        // in the offer. A contributor who declined and came back months
                        // later is making the same decision and is owed the same words.
                        Text(copy.offerExposure)
                            .glassType(GlassTokens.TypeScale.body)
                            .foregroundStyle(GlassColor.textPrimary)
                            .fixedSize(horizontal: false, vertical: true)
                        Toggle(copy.settingsToggle, isOn: Binding(get: { isOn ?? false }, set: onSet))
                            .toggleStyle(GlassToggleStyle(.settings))
                            .disabled(busy || isOn == nil)
                        if let serving = PrivateInferenceSurface.servingLine(state, calls: calls) {
                            Text(serving)
                                .glassType(GlassTokens.TypeScale.caption)
                                .foregroundStyle(GlassColor.textSecondary)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        Text(copy.settingsAppliesAtOnce)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textSecondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
            if let refusal {
                GlassNotice(tone: .outside, title: refusal) {
                    Button(ActionMessageBanner.coreDismissWord ?? ActionMessageBanner.dismissWord, action: onDismiss)
                        .buttonStyle(GlassButtonStyle(.glass))
                }
            }
        }
    }
}
