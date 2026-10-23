import Foundation
import SwiftUI
import TCBridge
import TCShellCore

/// The NEAR AI payout section's state: the rail's status from the commons,
/// the picker, and the one refusal line. All sentences come from the core's
/// copy; the daemon does every network call.
@MainActor
final class PayoutLinkModel: ObservableObject {
    /// The core's words, nil if the export did not arrive (the section then
    /// draws nothing).
    let surface: PayoutLinkSurface? =
        TCConsentCopy.payoutLinkCopyJSON()
        .flatMap(PayoutLinkCopy.decode(fromJSON:))
        .map(PayoutLinkSurface.init(copy:))

    @Published private(set) var rail: PayoutRailStatus?
    @Published private(set) var picker = PayoutPicker()
    @Published private(set) var busy = false
    @Published private(set) var refusal: PayoutLinkRefusal?

    private var client: DaemonClient?

    func attach(_ client: DaemonClient?) {
        self.client = client
    }

    /// Reads the rail's status. A failed read keeps nothing stale on screen:
    /// the section hides rather than show an old link state.
    func refresh() async {
        guard surface != nil, let client else {
            rail = nil
            return
        }
        let result = await Task.detached(priority: .userInitiated) {
            Result { try client.payoutLink.status() }
        }.value
        switch result {
        case .success(let rails):
            rail = PayoutLinkSurface.nearAiRail(in: rails)
        case .failure:
            rail = nil
        }
        if let rail, let surface {
            // The name only comes from the org list. Load it when the status
            // line needs it or when the person may choose.
            if surface.needsOrgName(for: rail) || surface.canChoose(for: rail) {
                await loadOrganizations()
                await reloadStatusForNames()
            }
        }
    }

    private func reloadStatusForNames() async {
        guard let client else { return }
        let result = await Task.detached(priority: .userInitiated) {
            Result { try client.payoutLink.status() }
        }.value
        if case .success(let rails) = result { rail = PayoutLinkSurface.nearAiRail(in: rails) }
    }

    func loadOrganizations() async {
        guard let client, !busy else { return }
        picker.beginLoading()
        let result = await Task.detached(priority: .userInitiated) {
            Result { try client.payoutLink.organizations() }
        }.value
        switch result {
        case .success(let listing):
            refusal = nil
            picker.listed(listing)
        case .failure(let error):
            let kind = (error as? PayoutLinkClient.Failure)?.refusal ?? .failed
            refusal = kind
            picker.refused(kind)
        }
    }

    func select(_ id: String?) {
        picker.select(id)
    }

    func notNow() {
        picker.dismissSelection()
    }

    /// The person's click on Choose, after the consent was shown for the
    /// selected org: start, then confirm. Nothing is saved if start fails.
    func choose() async {
        guard let client, let rail, let org = picker.selected,
            let revision = picker.listing?.orgsRevision, !busy
        else { return }
        picker.beginWorking()
        busy = true
        defer { busy = false }
        let version = rail.version
        let result = await Task.detached(priority: .userInitiated) {
            Result {
                let link = client.payoutLink
                try link.startLink(
                    railId: PayoutLinkSurface.nearAiRailId, organizationId: org.id,
                    orgsRevision: revision, expectedVersion: version)
                try link.confirmLink(railId: PayoutLinkSurface.nearAiRailId, organizationId: org.id)
            }
        }.value
        await finish(result)
    }

    /// Stop sending credit: start and confirm an unlink in one click, as the
    /// button names the whole action.
    func stop() async {
        guard let client, let rail, !busy else { return }
        busy = true
        defer { busy = false }
        let version = rail.version
        let result = await Task.detached(priority: .userInitiated) {
            Result {
                let link = client.payoutLink
                try link.startUnlink(railId: PayoutLinkSurface.nearAiRailId, expectedVersion: version)
                try link.confirmUnlink(railId: PayoutLinkSurface.nearAiRailId)
            }
        }.value
        await finish(result)
    }

    private func finish(_ result: Result<Void, Error>) async {
        switch result {
        case .success:
            refusal = nil
            picker.finished()
        case .failure(let error):
            let kind = (error as? PayoutLinkClient.Failure)?.refusal ?? .failed
            refusal = kind
            picker.refused(kind)
        }
        // Re-read: a refused change may mean the link moved underneath.
        busy = false
        await refresh()
    }
}
