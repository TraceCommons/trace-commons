import Foundation
import os
import XCTest
@testable import TraceCommonsApp

final class WindowNotificationObserversTests: XCTestCase {
    func testReleasingTheOwnerRemovesEveryRegistration() {
        let center = NotificationCenter()
        let calls = OSAllocatedUnfairLock(initialState: [String]())
        let names = [Notification.Name("synthetic-resize"), .init("synthetic-move")]
        let tokens = names.map { name in
            center.addObserver(forName: name, object: nil, queue: nil) { _ in
                calls.withLock { $0.append(name.rawValue) }
            }
        }
        var owner: WindowNotificationObservers? = WindowNotificationObservers(center: center, tokens: tokens)
        withExtendedLifetime(owner) {
            for name in names { center.post(name: name, object: nil) }
        }
        XCTAssertEqual(calls.withLock { $0 }, names.map(\.rawValue))

        owner = nil
        for name in names { center.post(name: name, object: nil) }
        XCTAssertEqual(calls.withLock { $0 }, names.map(\.rawValue), "Released window observers must stop receiving notifications")
    }

    func testReplacingTheOwnerRemovesOldTokensAndKeepsTheNewOnes() {
        let center = NotificationCenter()
        let name = Notification.Name("synthetic-window-event")
        let calls = OSAllocatedUnfairLock(initialState: [String]())
        func registration(_ label: String) -> NSObjectProtocol {
            center.addObserver(forName: name, object: nil, queue: nil) { _ in
                calls.withLock { $0.append(label) }
            }
        }
        var owner: WindowNotificationObservers? = WindowNotificationObservers(center: center, tokens: [registration("old")])
        withExtendedLifetime(owner) { center.post(name: name, object: nil) }
        XCTAssertEqual(calls.withLock { $0 }, ["old"])

        owner = WindowNotificationObservers(center: center, tokens: [registration("new")])
        withExtendedLifetime(owner) { center.post(name: name, object: nil) }
        XCTAssertEqual(calls.withLock { $0 }, ["old", "new"])

        owner = nil
        center.post(name: name, object: nil)
        XCTAssertEqual(calls.withLock { $0 }, ["old", "new"], "Replacing the window must not retain either registration group")
    }
}
