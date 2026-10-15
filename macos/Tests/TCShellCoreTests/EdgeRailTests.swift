import Foundation
import TCShellCore
import XCTest

/// The edge rail's switch.
final class EdgeRailTests: XCTestCase {
    private var defaults: UserDefaults!
    private let suite = "EdgeRailTests"

    override func setUp() {
        super.setUp()
        defaults = UserDefaults(suiteName: suite)
        defaults.removePersistentDomain(forName: suite)
    }

    override func tearDown() {
        defaults.removePersistentDomain(forName: suite)
        super.tearDown()
    }

    /// Off until the contributor turns it on, and remembered after.
    func testTheRailIsOffUntilTurnedOn() {
        XCTAssertFalse(EdgeRailPreference.isEnabled(defaults))
        EdgeRailPreference.set(true, defaults)
        XCTAssertTrue(EdgeRailPreference.isEnabled(defaults))
        EdgeRailPreference.set(false, defaults)
        XCTAssertFalse(EdgeRailPreference.isEnabled(defaults))
    }
}
