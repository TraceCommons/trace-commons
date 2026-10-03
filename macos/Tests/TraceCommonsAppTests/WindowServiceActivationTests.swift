import XCTest
@testable import TraceCommonsApp

/// The Settings window (⌘,) and the monitor read and write the daemon, so
/// opening either starts services even while the main window rests on
/// Insights, which defers them. Before, Settings opened with nothing loaded
/// and an edit went nowhere.
final class WindowServiceActivationTests: XCTestCase {
    @MainActor
    func test_aServiceWindowStartsServicesWhileMainRestsOnInsights() {
        let navigation = MainWindowNavigation()
        var starts = 0
        navigation.registerServiceStart { starts += 1 }
        XCTAssertEqual(navigation.section, .insights)

        navigation.activateServicesForWindow()
        XCTAssertEqual(starts, 1)
        XCTAssertTrue(navigation.servicesActivated)

        // Once, however many windows open, and the main window's own path
        // does not start them a second time.
        navigation.activateServicesForWindow()
        navigation.section = .queue
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
