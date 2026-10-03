import SwiftUI
import TCDesign

struct AdmissionPreparationView: View {
    @EnvironmentObject private var model: AppModel
    @State private var backend = ""
    @State private var working = false
    @State private var message = ""
    @State private var refused = false
    let entryID: String

    var body: some View {
        if let copy = model.witnessCopy?.admission {
            GlassCard {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                    Text(copy.heading)
                        .glassType(GlassTokens.TypeScale.label.weight(.semibold))
                        .foregroundStyle(GlassColor.textPrimary)
                    Text(copy.disclosure)
                        .glassType(GlassTokens.TypeScale.body)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                    Text(copy.prerequisite)
                        .glassType(GlassTokens.TypeScale.label.weight(.regular))
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                    HStack(alignment: .bottom, spacing: GlassTokens.Space.s4) {
                        GlassTextField(copy.backend, text: $backend).disabled(working)
                        Button(copy.confirm, action: prepare)
                            .buttonStyle(GlassButtonStyle(.primary))
                            .disabled(working || backend.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || model.daemonSettings?.inferenceEvidenceEnabled != true)
                    }
                    if model.daemonSettings?.inferenceEvidenceEnabled != true {
                        SettingsLink { Text(copy.permission) }
                    }
                    if working { ProgressView().controlSize(.small) }
                    if refused { NativeFlowNotice(message: message, glyph: copy.refusedGlyph, tone: copy.refusedTone) }
                    else if !message.isEmpty {
                        Text(message)
                            .glassType(GlassTokens.TypeScale.body)
                            .foregroundStyle(GlassColor.textSecondary)
                    }
                }
            }
        }
    }

    private func prepare() {
        working = true
        message = ""
        Task {
            let outcome = await model.prepareAdmissionSession(
                entryID: entryID,
                backend: backend.trimmingCharacters(in: .whitespacesAndNewlines)
            )
            working = false
            refused = !outcome.succeeded
            // The daemon classifies the cause and picks the words; this
            // rendered one sentence for every refusal, so a missing receipt
            // service read exactly like an agent this build cannot read.
            // `admission.failed` stays as the fallback for a response
            // carrying no sentence: a transport failure, or a daemon older
            // than this shell.
            message = outcome.sentence ?? model.witnessCopy?.admission?.failed ?? ""
        }
    }
}

struct AdmissionPreparation: Decodable, Sendable {
    let status: String
    let view: AdmissionReadyView?
}
struct AdmissionReadyView: Decodable, Sendable {
    let ready: Bool
    let message: String
    let tone: String
    let glyph: String
}
