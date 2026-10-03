import XCTest
@testable import TCShellCore

/// #1208: `SampleDaemonClient` answers the pill's override writes the way
/// the daemon does, refusals included, and `status` follows them.
final class ContributionOverrideSampleTests: XCTestCase {
    func testAutoContributeWithoutConfirmIsRefusedAndChangesNothing() async throws {
        let client = SampleDaemonClient(.normalDay)
        let before = try await client.status()
        do {
            _ = try await client.setContributionOverride(mode: .autoUpload, confirm: false)
            XCTFail("an unconfirmed Auto contribute answered")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .daemon(code: "bad_params", message: "confirm-required"))
        }
        let after = try await client.status()
        XCTAssertEqual(after.contributionOverride, nil)
        XCTAssertEqual(after.contributionMode, before.contributionMode)
    }

    /// The fail-closed path: a store with no grant terms (not enrolled)
    /// refuses Auto contribute even when confirmed, and nothing changes.
    func testAutoContributeWithoutGrantTermsIsRefused() async throws {
        let client = SampleDaemonClient(.empty)
        let enrolled = try await client.status()
        XCTAssertEqual(enrolled.loggedIn, false, "the empty set is the unenrolled one")
        do {
            _ = try await client.setContributionOverride(mode: .autoUpload, confirm: true)
            XCTFail("Auto contribute without terms answered")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .daemon(code: "unavailable", message: "arming-terms-unavailable"))
        }
        let afterRefusal = try await client.status()
        XCTAssertNil(afterRefusal.contributionOverride)
        // The stopping overrides need no terms.
        let never = try await client.setContributionOverride(mode: .ignore, confirm: false)
        XCTAssertTrue(never.changed)
    }

    func testSettingAModeUpdatesStatusAndClearRestoresIt() async throws {
        let client = SampleDaemonClient(.armedFolder)
        let recorded = try await client.status()
        XCTAssertEqual(recorded.contributionMode, "auto_upload")

        let never = try await client.setContributionOverride(mode: .ignore, confirm: false)
        XCTAssertEqual(never.changed, true)
        XCTAssertEqual(never.contributionOverride?.mode, "ignore")
        XCTAssertEqual(never.returned, 0)
        var status = try await client.status()
        XCTAssertEqual(status.contributionMode, "ignore")
        XCTAssertEqual(status.contributionOverride?.mode, "ignore")
        XCTAssertNotNil(status.contributionOverride?.since)
        XCTAssertEqual(status.contributionModePartial, false)

        // The same mode again is not a change.
        let again = try await client.setContributionOverride(mode: .ignore, confirm: false)
        XCTAssertEqual(again.changed, false)
        XCTAssertEqual(again.contributionOverride, never.contributionOverride)

        let auto = try await client.setContributionOverride(mode: .autoUpload, confirm: true)
        XCTAssertEqual(auto.changed, true)
        status = try await client.status()
        XCTAssertEqual(status.contributionMode, "auto_upload")
        XCTAssertEqual(status.contributionOverride?.mode, "auto_upload")

        let cleared = try await client.clearContributionOverride()
        XCTAssertEqual(cleared, DaemonData.ContributionOverrideClearResult(cleared: true, returned: 0))
        let restored = try await client.status()
        XCTAssertEqual(restored, recorded, "clear restores the recorded status exactly")
        let none = try await client.clearContributionOverride()
        XCTAssertEqual(none.cleared, false)
    }

    /// The stream's opening snapshot carries the override too, as `status`
    /// does.
    func testTheSnapshotAgreesWithStatus() async throws {
        let client = SampleDaemonClient(.normalDay)
        _ = try await client.setContributionOverride(mode: .ask, confirm: false)
        var iterator = client.events().makeAsyncIterator()
        guard case .snapshot(_, let status)? = await iterator.next() else {
            return XCTFail("first event is not a snapshot")
        }
        XCTAssertEqual(status?.contributionOverride?.mode, "notify_only")
        XCTAssertEqual(status?.contributionMode, "notify_only")
    }

    func testCoreDownIsUnreachable() async {
        let client = SampleDaemonClient(.coreDown)
        for call in [
            { _ = try await client.setContributionOverride(mode: .ask, confirm: false) },
            { _ = try await client.clearContributionOverride() },
        ] as [() async throws -> Void] {
            do {
                try await call()
                XCTFail("a core-down set answered")
            } catch {
                XCTAssertEqual(error as? DaemonDataError, .unreachable)
            }
        }
        XCTAssertEqual(client.overrideCalls, [], "nothing reached a core that is down")
    }

    /// Decoding the core's confirmation: Auto contribute is never accepted
    /// without its arming disclosure.
    func testAnAutoConfirmationWithoutTheArmingDisclosureDoesNotDecode() {
        let ask = #"{"mode":"notify_only","title":"T","body":"B","confirm":"C","cancel":"X","arming":null}"#
        XCTAssertNotNil(ContributionOverrideConfirmCopy.decode(fromJSON: ask))
        let bare = #"{"mode":"auto_upload","title":"T","body":"B","confirm":"C","cancel":"X","arming":null}"#
        XCTAssertNil(ContributionOverrideConfirmCopy.decode(fromJSON: bare))
        let armed = #"{"mode":"auto_upload","title":"T","body":"One.\n\nTwo.","confirm":"C","cancel":"X","arming":{"disclosure":"patterns_only","patterns_only":{"scope":"S","limit":"L"},"model_scrubbed":null,"no_review":"N"}}"#
        let copy = ContributionOverrideConfirmCopy.decode(fromJSON: armed)
        XCTAssertEqual(copy?.paragraphs, ["One.", "Two.", "S", "L", "N"])
        let empty = #"{"mode":"ignore","title":"","body":"B","confirm":"C","cancel":"X","arming":null}"#
        XCTAssertNil(ContributionOverrideConfirmCopy.decode(fromJSON: empty))
    }
}
