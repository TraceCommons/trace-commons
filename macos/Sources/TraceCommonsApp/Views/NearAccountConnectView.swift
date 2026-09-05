import SwiftUI

/// The daemon owns wallet verification, PKCE and the device key. This view only
/// presents readiness and opens the exact browser ceremony it returns.
struct NearAccountConnectView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.openURL) private var openURL
    @State private var active = true
    @State private var commons = ""
    @State private var account = ""
    @State private var ready = false
    @State private var checking = false
    @State private var attemptID: String?
    @State private var message = ""
    var onBusyChanged: (Bool) -> Void = { _ in }
    var onEnrolled: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.m) {
            Text("Join with a NEAR account").font(TC.Font_.cardTitle)
            Text("Check whether your commons accepts new accounts. Connecting proves control of your account and this device; it does not fund inference or enable capture.")
                .font(.callout).foregroundStyle(.secondary)
            HStack {
                TextField("Commons HTTPS address", text: $commons)
                    .textFieldStyle(.roundedBorder).disabled(checking || attemptID != nil)
                    .onChange(of: commons) { _, _ in ready = false }
                Button("Check availability", action: checkAvailability)
                    .disabled(commons.isEmpty || checking || attemptID != nil)
            }
            if ready {
                TextField("Your NEAR account", text: $account)
                    .textFieldStyle(.roundedBorder).disabled(attemptID != nil || checking)
                if attemptID == nil {
                    Button("Continue in wallet", action: start)
                        .disabled(account.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || checking)
                }
            }
            if checking || attemptID != nil { ProgressView().controlSize(.small) }
            if !message.isEmpty { Text(message).font(.callout).foregroundStyle(.secondary) }
            if attemptID != nil { Button("Cancel connection", role: .cancel, action: cancel) }
        }
        .task(id: attemptID) { await poll() }
        .onAppear { active = true }
        .onDisappear { active = false; cancel() }
    }

    private func checkAvailability() {
        checking = true
        ready = false
        message = ""
        Task {
            let capability = await model.nearAccountCapabilities(commons: commons)
            ready = capability?.ready == true
            message = ready ? "This commons supports wallet signup." : "Wallet signup is unavailable for this commons. You can still use an invite."
            checking = false
        }
    }

    private func start() {
        checking = true
        onBusyChanged(true)
        message = "Opening a wallet connection…"
        Task {
            let progress = await model.nearAccountStart(commons: commons, account: account.trimmingCharacters(in: .whitespacesAndNewlines))
            checking = false
            if !active {
                if let id = progress?.attemptID { await model.nearAccountCancel(attemptID: id) }
                onBusyChanged(false)
                return
            }
            guard let progress, let id = progress.attemptID,
                  let url = progress.browserURLFor(commons: commons) else {
                message = "The connection could not start. Check availability and try again."
                onBusyChanged(false)
                return
            }
            attemptID = id
            message = "Finish signing in your wallet. Keep this window open."
            openURL(url) { accepted in
                if !accepted { cancel() }
            }
        }
    }

    private func poll() async {
        guard let id = attemptID else { return }
        while !Task.isCancelled, attemptID == id {
            guard let progress = await model.nearAccountStatus(attemptID: id) else {
                message = "The connection status is unavailable. Cancel and try again."
                return
            }
            guard !Task.isCancelled, attemptID == id else { return }
            switch progress.status {
            case "complete":
                attemptID = nil
                account = ""
                onBusyChanged(false)
                onEnrolled()
                return
            case "failed", "cancelled", "expired":
                attemptID = nil
                message = "The wallet connection did not complete. You can try again."
                onBusyChanged(false)
                return
            case "starting", "waiting_for_wallet":
                do { try await Task.sleep(for: .seconds(2)) } catch { return }
            default:
                message = "The connection status is unavailable. Cancel and try again."
                return
            }
        }
    }

    private func cancel() {
        guard let id = attemptID else { return }
        attemptID = nil
        onBusyChanged(false)
        message = "Connection cancelled."
        Task { await model.nearAccountCancel(attemptID: id) }
    }
}

struct NearAccountCapability: Decodable, Sendable {
    let ready: Bool
}
struct NearAccountProgress: Decodable, Sendable {
    let status: String
    let attemptID: String?
    let browserURL: String?
    func browserURLFor(commons: String) -> URL? {
        guard let browserURL, let url = URL(string: browserURL), let origin = URL(string: commons),
              url.scheme == "https", origin.scheme == "https", url.host == origin.host,
              (url.port ?? 443) == (origin.port ?? 443), url.user == nil, url.password == nil else { return nil }
        return url
    }
    enum CodingKeys: String, CodingKey {
        case status
        case attemptID = "attempt_id"
        case browserURL = "browser_url"
    }
}
