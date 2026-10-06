import XCTest
@testable import TraceCommonsApp

#if DEBUG
/// K2 (#1173): the app shows its dry-run notice from the daemon's own
/// `status.dev_dry_run`, never from the environment, so the notice matches
/// whatever the daemon decided.
final class DevDryRunStatusTests: XCTestCase {
    func testOnOnlyWhenTheDaemonReportsTrue() {
        XCTAssertTrue(DaemonDataWiring.devDryRun(fromStatus: Data(#"{"dev_dry_run":true}"#.utf8)))
        XCTAssertFalse(DaemonDataWiring.devDryRun(fromStatus: Data(#"{"dev_dry_run":false}"#.utf8)))
        // A release daemon, or one that predates the field, names nothing.
        XCTAssertFalse(DaemonDataWiring.devDryRun(fromStatus: Data(#"{"paused":false}"#.utf8)))
        XCTAssertFalse(DaemonDataWiring.devDryRun(fromStatus: Data(#"{"dev_dry_run":"1"}"#.utf8)))
        XCTAssertFalse(DaemonDataWiring.devDryRun(fromStatus: Data("not json".utf8)))
    }
}
#endif
