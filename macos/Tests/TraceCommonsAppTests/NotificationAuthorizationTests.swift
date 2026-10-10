import UserNotifications
import XCTest
@testable import TraceCommonsApp

final class NotificationAuthorizationTests: XCTestCase {
    func testDigestRequiresKnownAuthorization() {
        for status in [UNAuthorizationStatus.authorized, .provisional] {
            XCTAssertTrue(Notifier.canPostDigest(status))
        }
        XCTAssertFalse(Notifier.canPostDigest(.notDetermined))
        XCTAssertFalse(Notifier.canPostDigest(.denied))
        XCTAssertFalse(Notifier.canPostDigest(nil))
        XCTAssertFalse(Notifier.canPostDigest(UNAuthorizationStatus(rawValue: 999)))
    }

    func testLaunchConfigurationNeverRequestsPermissionAndPostingChecksFreshStatus() throws {
        let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
        let source = try String(contentsOf: root.appendingPathComponent("Sources/TraceCommonsApp/Notifier.swift"), encoding: .utf8)
        let start = try XCTUnwrap(source.range(of: "func configure() {"))
        let end = try XCTUnwrap(source.range(of: "func authorizationStatus()", range: start.upperBound..<source.endIndex))
        XCTAssertFalse(source[start.upperBound..<end.lowerBound].contains(".requestAuthorization("))
        // Launch registers every category at once (`launchCategories`):
        // setting the digest's alone would drop the re-engagement buttons
        // from notifications delivered before a relaunch.
        XCTAssertTrue(source[start.upperBound..<end.lowerBound].contains("Self.launchCategories(Self.nudgeCopy)"))
        let posting = try XCTUnwrap(source.range(of: "func postDigest("))
        let add = try XCTUnwrap(source.range(of: ".add(request)", range: posting.upperBound..<source.endIndex))
        XCTAssertTrue(source[posting.upperBound..<add.lowerBound].contains("Self.canPostDigest(await authorizationStatus())"))
    }
}
