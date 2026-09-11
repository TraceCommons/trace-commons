// INTEGRATION: add FundingRow(copy:) inside CredentialSection. AppModel supplies
// nearAiFunding(expected: FundingDestination?) async -> FundingStatus? and must
// return nil after awaiting if its captured client is no longer current or the
// task was cancelled. PrivateInferenceCopy supplies fundingTitle, fundingWhat,
// fundingManage, fundingRefresh, and fundingUnavailable. The model read must not
// set credentialBusy; this row owns its request and observes credential writes.
// Only ready responses can authorize a browser destination. observed_at is an
// unused RFC3339 observation timestamp, not an authorization or freshness token.

import Foundation
import SwiftUI
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

    var browserURL: URL? {
        URL(string: "https://cloud.near.ai/dashboard/organizations/\(organizationID)/credits")
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

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.sm) {
            TCSectionHeader(title: copy.fundingTitle)
            Text(status?.view.message ?? copy.fundingUnavailable)
                .font(TC.Font_.body)
                .fixedSize(horizontal: false, vertical: true)
            Text(copy.fundingWhat)
                .font(TC.Font_.meta)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            Button {
                load(opening: status?.destination)
            } label: {
                Text(status?.destination == nil ? copy.fundingRefresh : copy.fundingManage)
                    .frame(minHeight: 44)
            }
            .buttonStyle(.bordered)
            .disabled(model.credentialBusy || request != nil)
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
        .onReceive(model.$credentialBusy.removeDuplicates().dropFirst()) { _ in
            invalidate()
        }
        .onReceive(model.$credentialStatus.map(\.sessionState).removeDuplicates().dropFirst()) { _ in
            invalidate()
        }
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
                let accepted = await withCheckedContinuation { continuation in
                    openURL(url) { continuation.resume(returning: $0) }
                }
                guard !Task.isCancelled, isCurrent(ticket, sessionState: sessionState) else { return }
                if !accepted { status = nil }
            } else {
                status = result
            }
        }
    }
}
