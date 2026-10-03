import SwiftUI
import TCDesign
import TCShellCore

/// The Inference inspector's account: sign-in, the tools and the Private AI
/// switch, as the legacy destination draws them (`PrivateInferenceContent`).
/// Sign-in and the tools keep their live `AppModel` paths; the switch reads
/// and writes through the data contract (`InferenceStore`).
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
                GlassCard { CredentialSection(copy: copy, prominent: true) }
                GlassCard { HarnessListSection(copy: copy) }
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
                    onDismiss: { store.dismissPrivateAIRefusal() })
            }
        }
    }

    /// The daemon's listener report as the card's state. Unreported is the
    /// empty label, which the core answers with the sentence that claims
    /// nothing; a port that is not a port is no port, never another one.
    static func surfaceState(_ state: DaemonData.PrivateInferenceState?) -> PrivateInferenceState {
        PrivateInferenceState(label: state?.state ?? "", port: state?.port.flatMap { UInt16(exactly: $0) })
    }
}
