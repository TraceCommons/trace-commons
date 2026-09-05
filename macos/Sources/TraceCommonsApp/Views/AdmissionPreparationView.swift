import SwiftUI

struct AdmissionPreparationView: View {
    @EnvironmentObject private var model: AppModel
    @State private var backend = ""
    @State private var working = false
    @State private var message = ""
    @State private var refused = false
    let entryID: String

    var body: some View {
        if let copy = model.witnessCopy?.admission {
        VStack(alignment: .leading, spacing: TC.Space.m) {
            Text(copy.heading).font(TC.Font_.cardTitle)
            Text(copy.disclosure)
                .font(.callout).foregroundStyle(.secondary)
            Text(copy.prerequisite)
                .font(.caption).foregroundStyle(.secondary)
            HStack {
                TextField(copy.backend, text: $backend).textFieldStyle(.roundedBorder).disabled(working)
                Button(copy.confirm, action: prepare)
                    .disabled(working || backend.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || model.daemonSettings?.inferenceEvidenceEnabled != true)
            }
            if model.daemonSettings?.inferenceEvidenceEnabled != true {
                SettingsLink { Text(copy.permission) }
            }
            if working { ProgressView().controlSize(.small) }
            if refused { NativeFlowNotice(message: message, glyph: copy.refusedGlyph, tone: copy.refusedTone) }
            else if !message.isEmpty { Text(message).font(.callout).foregroundStyle(.secondary) }
        }
        }
    }

    private func prepare() {
        working = true
        message = ""
        Task {
            let result = await model.prepareAdmissionSession(entryID: entryID, backend: backend.trimmingCharacters(in: .whitespacesAndNewlines))
            working = false
            refused = result?.view?.ready != true
            message = result?.view?.message ?? model.witnessCopy?.admission?.failed ?? ""
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
