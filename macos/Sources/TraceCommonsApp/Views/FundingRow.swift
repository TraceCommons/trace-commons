// INTEGRATION: FundingRow(copy:) is its own card on the Inference tab, after the
// balance panel (#1146's funding panel). AppModel supplies
// nearAiFunding(expected: FundingDestination?) async -> FundingStatus? and must
// return nil after awaiting if its captured client is no longer current or the
// task was cancelled. PrivateInferenceCopy supplies fundingTitle, fundingWhat,
// fundingManage, fundingRefresh, and fundingUnavailable. The model read must not
// set credentialBusy; this row owns its request and observes credential writes.
// Only ready responses can authorize a browser destination. observed_at is an
// unused RFC3339 observation timestamp, not an authorization or freshness token.

import Foundation
import SwiftUI
import TCDesign
import TCShellCore

/// The exact Cloud organization and credential revision displayed to the user.
struct FundingDestination: Decodable, Equatable, Sendable {
    let organizationID: String
    let connectionRevision: String

    init?(organizationID: String, connectionRevision: String) {
        guard !organizationID.isEmpty, organizationID.utf8.count <= 128,
            organizationID.utf8.allSatisfy({ byte in
                (65...90).contains(byte) || (97...122).contains(byte)
                    || (48...57).contains(byte) || byte == 45 || byte == 95
            }),
            connectionRevision.utf8.count == 64,
            connectionRevision.utf8.allSatisfy({ byte in
                (48...57).contains(byte) || (97...102).contains(byte)
            })
        else { return nil }
        self.organizationID = organizationID
        self.connectionRevision = connectionRevision
    }

    /// The credits page on the linked dylib's NEAR AI dashboard. The origin
    /// is the dylib's (staging design 4.1), never a Swift copy, so a staging
    /// build links to the staging dashboard. Nil when the dylib cannot say.
    var browserURL: URL? {
        browserURL(dashboardOrigin: BuildChannelGate.dylibChannel?.nearAiDashboardOrigin)
    }

    func browserURL(dashboardOrigin: String?) -> URL? {
        guard let dashboardOrigin, !dashboardOrigin.isEmpty else { return nil }
        return URL(string: "\(dashboardOrigin)/dashboard/organizations/\(organizationID)/credits")
    }

    private enum CodingKeys: String, CodingKey {
        case organizationID = "organization_id"
        case connectionRevision = "connection_revision"
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        let organizationID = try values.decode(String.self, forKey: .organizationID)
        let revision = try values.decode(String.self, forKey: .connectionRevision)
        guard let destination = Self(organizationID: organizationID, connectionRevision: revision) else {
            throw DecodingError.dataCorruptedError(
                forKey: .organizationID, in: values, debugDescription: "funding_destination_invalid")
        }
        self = destination
    }
}

/// Canonical presentation from the daemon, with a validated ready-only binding.
struct FundingStatus: Decodable, Sendable {
    struct Presentation: Decodable, Sendable {
        let message: String
    }

    let state: String
    let view: Presentation
    let organizationName: String?
    let destination: FundingDestination?

    private enum CodingKeys: String, CodingKey {
        case state, view
        case organizationName = "organization_name"
        case browserURL = "browser_url"
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        state = try values.decode(String.self, forKey: .state)
        view = try values.decode(Presentation.self, forKey: .view)
        guard !state.isEmpty, state.utf8.count <= 64,
            !view.message.isEmpty, view.message.utf8.count <= 4096
        else {
            throw DecodingError.dataCorruptedError(
                forKey: .view, in: values, debugDescription: "funding_status_invalid")
        }
        if state == "ready" {
            let binding = try FundingDestination(from: decoder)
            let name = try values.decode(String.self, forKey: .organizationName)
            let returnedURL = try values.decode(String.self, forKey: .browserURL)
            // Exact canonical comparison also refuses credentials, alternate
            // ports, escaping, query strings, fragments, and alternate hosts.
            guard !name.isEmpty, name.utf8.count <= 1024,
                let compiledURL = binding.browserURL,
                returnedURL == compiledURL.absoluteString
            else {
                throw DecodingError.dataCorruptedError(
                    forKey: .browserURL, in: values, debugDescription: "funding_destination_invalid")
            }
            organizationName = name
            destination = binding
        } else {
            organizationName = nil
            destination = nil
        }
    }
}

@MainActor
struct FundingRow: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.openURL) private var openURL
    let copy: PrivateInferenceCopy

    @State private var status: FundingStatus?
    @State private var request: Task<Void, Never>?
    @State private var generation = UUID()
    @State private var visible = false
    /// The Billing card in Settings, or the NEAR AI balance card's Add funds
    /// button alone (owner, 2026-10-27). Both make the same bound read and
    /// open the same verified page.
    let style: Style

    enum Style: Equatable { case card, addFunds }

    init(copy: PrivateInferenceCopy, style: Style = .card) {
        self.copy = copy
        self.style = style
    }

    /// Whether the balance card shows Add funds: only with a destination
    /// this read verified, and never in place of a sign-in, which the
    /// balance card offers itself.
    nonisolated static func showsAddFunds(destination: FundingDestination?, signInNeeded: Bool) -> Bool {
        destination != nil && !signInNeeded
    }

    var body: some View {
        Group {
            switch style {
            case .card: card
            case .addFunds: addFunds
            }
        }
        .onAppear {
            visible = true
            load(opening: nil)
        }
        .onDisappear {
            visible = false
            invalidate()
        }
        // Observe individual publications, including a busy true/false pair
        // within one render pass. Coalescing those would miss a credential write.
        //
        // Each also reloads a visible row once it has cleared it, so a sign-in
        // or a credential write leaves the account read again and not the
        // unavailable line. The reload waits one turn: `@Published` sends
        // before it stores, so the busy flag and the session state read here
        // are still the old ones.
        .onReceive(model.$credentialBusy.removeDuplicates().dropFirst()) { busy in
            invalidate()
            if !busy { reloadAfterPublication() }
        }
        // A Sign in that ends leaves the account read again.
        .onReceive(model.$nearAiSigningIn.removeDuplicates().dropFirst()) { signingIn in
            if !signingIn { reloadAfterPublication() }
        }
        .onReceive(model.$credentialStatus.map(\.sessionState).removeDuplicates().dropFirst()) { _ in
            invalidate()
            reloadAfterPublication()
        }
    }

    /// The balance card's Add funds: the button alone, while the read holds
    /// a verified destination and no sign-in is needed.
    @ViewBuilder
    private var addFunds: some View {
        let signIn = model.nearAiSigningIn || NearAiSignInOffer.billingOffers(
            state: status?.state, failed: model.nearAiSignInFailed)
        if Self.showsAddFunds(destination: status?.destination, signInNeeded: signIn) {
            Button {
                load(opening: status?.destination)
            } label: {
                Text(copy.balanceAddFunds).lineLimit(1)
            }
            .buttonStyle(GlassButtonStyle(.glass))
            .fixedSize()
            .disabled(model.credentialBusy || request != nil)
        } else {
            // Not nothing: an empty view never appears, and appearing is
            // what makes the first read.
            Color.clear.frame(width: 0, height: 0)
        }
    }

    private var card: some View {
        // #1146's funding panel, its own card after the balance, under its
        // icon (owner, 2026-10-10): the state in the head with the button
        // on the right, and what the button opens behind the info icon.
        GlassIconCard(
            systemImage: "creditcard", title: copy.fundingTitle,
            subtitle: [
                NearAiSignInOffer.progressLine(
                    copy, signingIn: model.nearAiSigningIn, failed: model.nearAiSignInFailed)
                    ?? status?.view.message ?? copy.fundingUnavailable
            ],
            // What the button opens, behind the info icon (owner, 2026-10-10).
            info: copy.fundingWhat,
            trailing: {
                // Signed out, the button is the balance card's Sign in (owner,
                // 2026-10-24), not Check account: checking cannot help.
                let signIn = model.nearAiSigningIn || NearAiSignInOffer.billingOffers(
                    state: status?.state, failed: model.nearAiSignInFailed)
                Group {
                    if signIn {
                        // Every login the build offers, as the balance card.
                        NearAiSignInButton(
                            title: NearAiSignInOffer.label(copy), choice: NearAiSignInChoice.of(copy),
                            style: GlassButtonStyle(.primary)
                        ) { provider in
                            Task { await model.signInToNearAI(provider: provider) }
                        }
                    } else {
                        Button {
                            load(opening: status?.destination)
                        } label: {
                            // The glass style's own 28pt, as every inline card button
                            // (owner, 2026-10-08); a 44pt label made it the odd one out.
                            Text(status?.destination == nil ? copy.fundingRefresh : copy.fundingManage)
                        }
                        .buttonStyle(GlassButtonStyle(.glass))
                    }
                }
                .lineLimit(1)
                .fixedSize()
                .disabled(model.credentialBusy || (model.nearAiSigningIn && !model.canRestartNearAiBrowserSignIn)
                          || (!signIn && request != nil))
            })
    }

    private func reloadAfterPublication() {
        Task { @MainActor in load(opening: nil) }
    }

    private func invalidate() {
        generation = UUID()
        request?.cancel()
        request = nil
        status = nil
    }

    private func isCurrent(_ ticket: UUID, sessionState: String) -> Bool {
        visible && generation == ticket && !model.credentialBusy
            && model.credentialStatus.sessionState == sessionState
    }

    private func load(opening expected: FundingDestination?) {
        guard visible, !model.credentialBusy, request == nil else { return }
        let ticket = UUID()
        generation = ticket
        let sessionState = model.credentialStatus.sessionState
        request = Task { @MainActor in
            defer {
                if generation == ticket { request = nil }
            }
            let result = await model.nearAiFunding(expected: expected)
            guard !Task.isCancelled, isCurrent(ticket, sessionState: sessionState) else { return }
            guard let result else {
                status = nil
                return
            }
            if let expected {
                guard result.destination == expected, let url = expected.browserURL else {
                    status = nil
                    return
                }
                status = result
                let accepted = await openURL.opened(url)
                // Before the staleness check below: the browser opened whether
                // or not this row is still on screen.
                if accepted { model.noteBillingOpened() }
                guard !Task.isCancelled, isCurrent(ticket, sessionState: sessionState) else { return }
                if !accepted { status = nil }
            } else {
                status = result
            }
        }
    }
}
