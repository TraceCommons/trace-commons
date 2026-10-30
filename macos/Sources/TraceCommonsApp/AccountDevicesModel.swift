import Foundation
import SwiftUI
import TCBridge
import TCShellCore

/// A request, from the Traces map's +, to start linking a server the next
/// time the Devices section is on screen (owner, 2026-10-27). One request
/// starts one link: taking it clears it.
@MainActor
final class AccountDeviceLinkRequest: ObservableObject {
    static let shared = AccountDeviceLinkRequest()
    @Published private(set) var pending = false

    func ask() { pending = true }

    /// Whether linking was asked for, clearing the request.
    func take() -> Bool {
        defer { pending = false }
        return pending
    }
}

/// The Devices section's state: the devices linked to the account, the
/// one-time code for linking a server while it is on screen, and the one
/// refusal line. All sentences come from the core's copy; the daemon does
/// every network call.
@MainActor
final class AccountDevicesModel: ObservableObject {
    /// The core's words, nil if the export did not arrive (the section then
    /// draws nothing).
    let surface: AccountDevicesSurface? =
        TCConsentCopy.accountDevicesCopyJSON()
        .flatMap(AccountDevicesCopy.decode(fromJSON:))
        .map(AccountDevicesSurface.init(copy:))

    /// Nil until the first answer, and after a refused read: the section
    /// hides rather than show a stale list, except for an ended session.
    @Published private(set) var devices: [AccountDevice]?
    @Published private(set) var listRefusal: AccountDevicesRefusal?
    @Published private(set) var link: DeviceLinkPhase = .idle
    @Published private(set) var refusal: AccountDevicesRefusal?
    @Published private(set) var busy = false
    /// The device whose Unlink was clicked, waiting for the confirmation.
    @Published var confirming: AccountDevice?
    @Published private(set) var copiedCode: String?

    /// How often the shell asks whether the code on screen was used.
    static let pollEvery: Duration = .seconds(3)

    private var client: DaemonClient?
    private var generation: UInt64 = 0

    func attach(_ client: DaemonClient?) {
        generation &+= 1
        self.client = client
        link = .idle
    }

    /// The Traces map's + asked to link a server: start it, once -- but
    /// only once this model has a client to link with. Taken earlier, the
    /// request would be spent on a link that returns at once (Kristi's
    /// #1681 review): a section opened fresh by the + attaches its client
    /// in `.task`, after its first update. Returns whether it started.
    @discardableResult
    func linkIfRequested(_ request: AccountDeviceLinkRequest = .shared) async -> Bool {
        guard client != nil, request.take() else { return false }
        await linkServer()
        return true
    }

    func refresh() async {
        generation &+= 1
        let current = generation
        guard surface != nil, let client else {
            devices = nil
            listRefusal = nil
            return
        }
        let result = await DaemonWork.run { Result { try client.accountDevices.list() } }
        guard current == generation else { return }
        switch result {
        case .success(let listed):
            devices = AccountDevicesSurface.ordered(listed)
            listRefusal = nil
        case .failure(let error):
            devices = nil
            let kind = (error as? AccountDevicesClient.Failure)?.refusal
            listRefusal = kind == .sessionEnded ? .sessionEnded : nil
        }
    }

    /// Link a server: mint a code, show it, and ask about it until it is
    /// used or expires. Cancelled when the view goes away.
    func linkServer() async {
        guard let client, !busy, link.code == nil else { return }
        busy = true
        refusal = nil
        copiedCode = nil
        link = .minting
        let minted = await DaemonWork.run { Result { try client.accountDevices.mint(label: nil) } }
        busy = false
        switch minted {
        case .failure(let error):
            let kind = (error as? AccountDevicesClient.Failure)?.refusal ?? .failed
            link = .refused(kind)
            return
        case .success(let code):
            link = .waiting(code)
            await follow(code, client: client)
        }
    }

    private func follow(_ code: DeviceCode, client: DaemonClient) async {
        let codeId = code.codeId
        var missed = 0
        while link.code?.codeId == codeId {
            try? await Task.sleep(for: Self.pollEvery)
            if Task.isCancelled { return }
            // Past its expiry the code is spent whatever the daemon says.
            link = link.checked(at: Date(), for: codeId)
            guard link.code?.codeId == codeId else { break }
            // A missed answer is not the end, the code may still be used,
            // but a run of them is: the shell stops asking.
            guard case .success(let status) = await DaemonWork.run({
                Result { try client.accountDevices.status(codeId: codeId) }
            }) else {
                missed += 1
                link = link.afterMissed(missed, for: codeId)
                continue
            }
            missed = 0
            link = link.after(status, for: codeId)
        }
        if case .linked = link { await refresh() }
    }

    /// Done: put the code away. A code still on screen stays valid on the
    /// server until it expires; it is simply no longer shown.
    func dismissLink() {
        link = .idle
        copiedCode = nil
    }

    func copyCommand(_ command: String, for code: DeviceCode) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(command, forType: .string)
        copiedCode = code.codeId
    }

    func unlink(_ device: AccountDevice) async {
        guard let client, !busy else { return }
        confirming = nil
        busy = true
        let id = device.deviceKeyId
        let result = await DaemonWork.run { Result { try client.accountDevices.revoke(deviceKeyId: id) } }
        busy = false
        switch result {
        case .success:
            refusal = nil
        case .failure(let error):
            refusal = (error as? AccountDevicesClient.Failure)?.refusal ?? .failed
        }
        await refresh()
    }
}
