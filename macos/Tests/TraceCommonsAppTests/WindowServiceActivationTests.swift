import XCTest
@testable import TraceCommonsApp

/// The Settings window (⌘,) and the monitor read and write the daemon, so
/// opening either starts services. Before, Settings opened with nothing
/// loaded and an edit went nowhere. No section defers services (D-11).
final class WindowServiceActivationTests: XCTestCase {
    @MainActor
    func test_aServiceWindowStartsServices() {
        let navigation = MainWindowNavigation()
        var starts = 0
        navigation.registerServiceStart { starts += 1 }

        navigation.activateServicesForWindow()
        XCTAssertEqual(starts, 1)
        XCTAssertTrue(navigation.servicesActivated)

        // Once, however many windows open, and the launch path does not
        // start them a second time.
        navigation.activateServicesForWindow()
        navigation.activateServicesIfNeeded { starts += 1 }
        XCTAssertEqual(starts, 1)
    }

    /// A window that opens before launch registered the start work starts
    /// nothing, and is not counted as started.
    @MainActor
    func test_nothingRegisteredStartsNothing() {
        let navigation = MainWindowNavigation()
        navigation.activateServicesForWindow()
        XCTAssertFalse(navigation.servicesActivated)
    }
}
