import Foundation

public enum SingleOrganizationOnboardingOutcome: Equatable, Sendable {
    case ready
    case needsChoice
    case unavailable
}

/// Prepares both uses of the only cloud organization. All writes remain
/// guarded by the daemon's organization revision and link version checks.
@MainActor
public struct SingleOrganizationOnboarding {
    public var organizations: @MainActor () async throws -> SpendOrganizations
    public var payoutOrganizations: @MainActor () async throws -> PayoutOrganizationListing
    public var status: @MainActor () async throws -> [PayoutRailStatus]
    public var selectSpend: @MainActor (String) async throws -> Void
    public var startLink: @MainActor (String, String, Int64) async throws -> Void
    public var confirmLink: @MainActor (String) async throws -> Void
    public var isCurrent: @MainActor () -> Bool

    public init(
        organizations: @escaping @MainActor () async throws -> SpendOrganizations,
        payoutOrganizations: @escaping @MainActor () async throws -> PayoutOrganizationListing,
        status: @escaping @MainActor () async throws -> [PayoutRailStatus],
        selectSpend: @escaping @MainActor (String) async throws -> Void,
        startLink: @escaping @MainActor (String, String, Int64) async throws -> Void,
        confirmLink: @escaping @MainActor (String) async throws -> Void,
        isCurrent: @escaping @MainActor () -> Bool
    ) {
        self.organizations = organizations
        self.payoutOrganizations = payoutOrganizations
        self.status = status
        self.selectSpend = selectSpend
        self.startLink = startLink
        self.confirmLink = confirmLink
        self.isCurrent = isCurrent
    }

    public func prepare() async -> SingleOrganizationOnboardingOutcome {
        do {
            guard isCurrent(), !Task.isCancelled else { return .unavailable }
            let cloud = try await organizations()
            guard isCurrent(), !Task.isCancelled,
                  cloud.state == SpendOrganizations.stateListed, !cloud.revokePending
            else { return .unavailable }
            guard cloud.organizations.count == 1 else { return .needsChoice }
            let id = cloud.organizations[0].id
            let owned = try await payoutOrganizations()
            guard isCurrent(), !Task.isCancelled else { return .unavailable }
            guard owned.organizations.count == 1, owned.organizations[0].id == id,
                  !owned.orgsRevision.isEmpty else { return .needsChoice }
            let rails = try await status()
            guard isCurrent(), !Task.isCancelled else { return .unavailable }
            guard let rail = PayoutLinkSurface.nearAiRail(in: rails) else { return .unavailable }
            let alreadyLinked = Self.matches(rail, id: id)
            // A historical, pending, closing or unverified link needs the
            // existing chooser. Never silently redirect another recipient.
            guard alreadyLinked || (!rail.linked && rail.state == nil && rail.version == 0
                && rail.requestState == nil && !rail.unlinkRequested
                && !rail.recipientUnavailable && rail.recipientRefCheck == nil)
            else { return .needsChoice }
            try await selectSpend(id)
            guard isCurrent(), !Task.isCancelled else { return .unavailable }
            let selected = try await organizations()
            guard isCurrent(), !Task.isCancelled,
                  selected.state == SpendOrganizations.stateListed, !selected.revokePending,
                  selected.organizations.count == 1, selected.organizations[0].id == id,
                  selected.currentOrganizationID == id else { return .unavailable }
            if !alreadyLinked {
                // Selecting inference can refresh the credential used to
                // prove cloud membership. Obtain its new payout revision.
                let refreshed = try await payoutOrganizations()
                guard isCurrent(), !Task.isCancelled else { return .unavailable }
                guard refreshed.organizations.count == 1, refreshed.organizations[0].id == id,
                      !refreshed.orgsRevision.isEmpty else { return .needsChoice }
                try await startLink(id, refreshed.orgsRevision, rail.version)
                guard isCurrent(), !Task.isCancelled else { return .unavailable }
                try await confirmLink(id)
                guard isCurrent(), !Task.isCancelled else { return .unavailable }
            }
            let confirmed = try await status()
            guard isCurrent(), !Task.isCancelled else { return .unavailable }
            guard let linked = PayoutLinkSurface.nearAiRail(in: confirmed), Self.matches(linked, id: id)
            else { return .unavailable }
            return .ready
        } catch {
            return .unavailable
        }
    }

    private static func matches(_ rail: PayoutRailStatus, id: String) -> Bool {
        rail.linked && rail.state == "active" && rail.recipientRefCheck == "match"
            && rail.organizationID == id && rail.requestState == nil
            && !rail.unlinkRequested && !rail.recipientUnavailable
    }
}
