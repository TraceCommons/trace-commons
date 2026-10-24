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
    /// Why the status read was refused, when the section shows that instead
    /// of hiding (`PayoutLinkSurface.shownStatusRefusal`).
    @Published private(set) var statusRefusal: PayoutLinkRefusal?

    private var client: DaemonClient?
    // A refresh supersedes earlier refreshes, while an explicit Change's
    // org read stays valid when a refresh deliberately leaves it alone.
    private var refreshGeneration: UInt64 = 0
    private var organizationsGeneration: UInt64 = 0

    func attach(_ client: DaemonClient?) {
        refreshGeneration &+= 1
        organizationsGeneration &+= 1
        self.client = client
    }

    /// Reads the rail's status. A failed read keeps nothing stale on screen:
    /// the section hides rather than show an old link state, except for a
    /// missing account session, which it says.
    func refresh() async {
        refreshGeneration &+= 1
        let generation = refreshGeneration
        guard surface != nil, let client else {
            rail = nil
            statusRefusal = nil
            return
        }
        let result = await DaemonWork.run {
            Result { try client.payoutLink.status() }
        }
        guard generation == refreshGeneration else { return }
        switch result {
        case .success(let rails):
            rail = PayoutLinkSurface.nearAiRail(in: rails)
            statusRefusal = nil
        case .failure(let error):
            rail = nil
            statusRefusal = PayoutLinkSurface.shownStatusRefusal(error)
        }
        if let rail, let surface, !picker.changing {
            // The daemon has no name for the link (neither listed nor
            // saved), or the person may choose: list the orgs.
            if surface.needsOrgName(for: rail) || surface.canChoose(for: rail) {
                await loadOrganizations(refresh: generation)
                guard generation == refreshGeneration else { return }
                await reloadStatusForNames(refresh: generation)
            }
        }
    }

    private func reloadStatusForNames(refresh generation: UInt64) async {
        guard let client else { return }
        let result = await DaemonWork.run {
            Result { try client.payoutLink.status() }
        }
        guard generation == refreshGeneration else { return }
        if case .success(let rails) = result { rail = PayoutLinkSurface.nearAiRail(in: rails) }
    }

    func loadOrganizations() async {
        await loadOrganizations(refresh: nil)
    }

    private func loadOrganizations(refresh expectedRefresh: UInt64?) async {
        guard let client, !busy else { return }
        organizationsGeneration &+= 1
        let generation = organizationsGeneration
        picker.beginLoading()
        let result = await DaemonWork.run {
            Result { try client.payoutLink.organizations() }
        }
        guard generation == organizationsGeneration,
              expectedRefresh.map({ $0 == refreshGeneration }) ?? true
        else { return }
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

    /// Change on an active link: the org list, then the same consent and
    /// `choose()` -- start, then confirm against the version shown.
    func change() async {
        guard !busy else { return }
        picker.beginChange()
        await loadOrganizations()
    }

    func select(_ id: String?) {
        picker.select(id)
    }

    func notNow() {
        picker.dismissSelection()
    }

    /// The person's click on Choose, after the consent was shown for the
    /// selected org: start, then confirm. Nothing is saved if start fails.
    /// The same for a first link and a change: the start carries the
    /// `version` the person was shown, so a link that moved since refuses.
    func choose() async {
        guard let client, let rail, let org = picker.selected,
            let revision = picker.listing?.orgsRevision, !busy
        else { return }
        picker.beginWorking()
        busy = true
        defer { busy = false }
        let version = rail.version
        let result = await DaemonWork.run {
            Result {
                let link = client.payoutLink
                try link.startLink(
                    railId: PayoutLinkSurface.nearAiRailId, organizationId: org.id,
                    orgsRevision: revision, expectedVersion: version)
                try link.confirmLink(railId: PayoutLinkSurface.nearAiRailId, organizationId: org.id)
            }
        }
        await finish(result)
    }

    /// Stop sending credit: start and confirm an unlink in one click, as the
    /// button names the whole action.
    func stop() async {
        guard let client, let rail, !busy else { return }
        busy = true
        defer { busy = false }
        let version = rail.version
        let result = await DaemonWork.run {
            Result {
                let link = client.payoutLink
                try link.startUnlink(railId: PayoutLinkSurface.nearAiRailId, expectedVersion: version)
                try link.confirmUnlink(railId: PayoutLinkSurface.nearAiRailId)
            }
        }
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
