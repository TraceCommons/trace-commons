import CTraceCommons
import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// Joining a commons with the NEAR AI login a contributor already has.
///
/// **The way in that needs no wallet.** A contributor cannot produce an
/// admissible receipt without a NEAR AI account in the first place, because
/// the receipt comes from their own inference calls, so requiring a wallet as
/// well is a second onboarding for an identity they already hold. This is
/// drawn beside the wallet ceremony and replaces none of it.
///
/// **Nothing here authors a sentence and nothing branches on a control
/// name.** All ten refusals, the offer, the working line and the done line
/// come from the shared crate through `TCNearAiEnroll` and
/// `PrivateInferenceCopy`. A `switch` here would be an eleventh table.
///
/// **Not signed in is not a refusal.** The daemon checks the session before
/// it reaches out, so a contributor who never signed in is told to sign in
/// rather than that the commons is unreachable. This view keeps that apart:
/// with no session it withholds the control and says the step, because a
/// button whose only outcome is `near_ai_enroll_no_session` teaches somebody
/// the feature is broken.
struct NearAiJoinView: View {
    @EnvironmentObject private var model: AppModel
    @State private var commons = ""
    @State private var pending = false
    @State private var refusal: String?
    @State private var joined = false
    var onEnrolled: () -> Void

    /// Whether the daemon reports a usable NEAR AI sign-in.
    ///
    /// `credentialStatus` is seeded `.unreported`, so this is false until the
    /// first poll answers -- the reading that claims less. A card that has
    /// not been told yet must not offer a control that can only refuse.
    private var signedIn: Bool {
        model.credentialStatus.sessionState == CredentialSurface.statePresent
    }

    var body: some View {
        if let copy = model.privateInferenceCopy {
            GlassCard {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                    Text(copy.nearAiEnrollTitle)
                        .glassType(GlassTokens.TypeScale.label.weight(.semibold))
                        .foregroundStyle(GlassColor.textPrimary)
                    Text(copy.nearAiEnrollWhat)
                        .glassType(GlassTokens.TypeScale.body)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                    GlassTextField(model.witnessCopy?.wallet?.commons ?? "", text: $commons)
                        .disabled(pending)

                    if joined {
                        Text(copy.nearAiEnrollDone)
                            .glassType(GlassTokens.TypeScale.body)
                            .foregroundStyle(GlassColor.textSecondary)
                    } else if signedIn {
                        Button(copy.nearAiEnrollAction) { join() }
                            .buttonStyle(GlassButtonStyle(.primary))
                            .disabled(pending || commons.trimmingCharacters(in: .whitespaces).isEmpty)
                    } else {
                        Text(copy.nearAiEnrollNeedsLogin)
                            .glassType(GlassTokens.TypeScale.body)
                            .foregroundStyle(GlassColor.textSecondary)
                        CredentialSection(copy: copy, requiresSession: true)
                    }

                    if pending {
                        ProgressView().controlSize(.small)
                        Text(copy.nearAiEnrollWorking)
                            .glassType(GlassTokens.TypeScale.body)
                            .foregroundStyle(GlassColor.textSecondary)
                    }
                    if let refusal, let line = TCNearAiEnroll.line(label: refusal) {
                        NativeFlowNotice(
                            message: line,
                            glyph: model.witnessCopy?.wallet?.refusedGlyph ?? "",
                            tone: TCNearAiEnroll.tone(label: refusal)
                                == TC_PRIVATE_INFERENCE_TONE_REFUSED ? "refused" : "neutral")
                    }
                }
            }
        }
    }

    private func join() {
        pending = true
        refusal = nil
        Task {
            let outcome = await model.nearAiAccountEnroll(
                commons: commons.trimmingCharacters(in: .whitespaces))
            pending = false
            switch outcome {
            case .joined(let enrollment):
                // `enrolled` is read rather than assumed from the absence of
                // a refusal: a response that did not say so is not a join.
                joined = enrollment.enrolled
                if enrollment.enrolled { onEnrolled() }
            case .refused(let label):
                refusal = label
            }
        }
    }
}
