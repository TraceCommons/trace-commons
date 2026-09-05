import SwiftUI

struct AdmissionPreparationView: View {
    @EnvironmentObject private var model: AppModel
    @State private var backend = ""
    @State private var working = false
    @State private var message = ""
    let entryID: String

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.m) {
            Text("Prepare next NEAR inference").font(TC.Font_.cardTitle)
            Text("This adds an account-bound challenge to the next inference request in this session. Use your own funded NEAR AI backend, then continue the agent task and return here to review.")
                .font(.callout).foregroundStyle(.secondary)
            Text("IronWire must already route this agent to that backend and capture request bodies. Inference-body evidence also needs your separate permission in Settings.")
                .font(.caption).foregroundStyle(.secondary)
            HStack {
                TextField("NEAR AI backend name", text: $backend).textFieldStyle(.roundedBorder).disabled(working)
                Button("Prepare next request", action: prepare)
                    .disabled(working || backend.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || model.daemonSettings?.inferenceEvidenceEnabled != true)
            }
            if model.daemonSettings?.inferenceEvidenceEnabled != true {
                SettingsLink { Text("Review inference-body permission") }
            }
            if working { ProgressView().controlSize(.small) }
            if !message.isEmpty { Text(message).font(.callout).foregroundStyle(.secondary) }
        }
    }

    private func prepare() {
        working = true
        message = ""
        Task {
            let result = await model.prepareAdmissionSession(entryID: entryID, backend: backend.trimmingCharacters(in: .whitespacesAndNewlines))
            working = false
            if result?.status == "ready_for_next_inference" {
                message = "Ready. Continue this session in your agent, then review the updated session."
            } else {
                message = "Setup did not complete. Check your funded backend, routing, capture permissions, and proxy support, then try again."
            }
        }
    }
}

struct AdmissionPreparation: Decodable, Sendable {
    let status: String
}
