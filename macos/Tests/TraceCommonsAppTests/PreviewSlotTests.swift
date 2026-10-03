#if DEBUG
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// The inspector's preview is keyed by the session it was asked for. Select
/// A, then B: A's late answer must not be shown, or reviewed, as B's.
@MainActor
final class PreviewSlotTests: XCTestCase {
    private func summary() async throws -> DaemonData.PreviewSummary {
        let client = SampleDaemonClient(.normalDay)
        let pending = try await client.listPending(projectId: nil)
        let entry = try XCTUnwrap(pending.first)
        return try await client.preview(entryId: entry.entryId)
    }

    func test_aLateAnswerForAnotherSessionIsDropped() async throws {
        let late = try await summary()
        var slot = PreviewSlot()
        slot.begin("A")
        slot.begin("B")
        XCTAssertFalse(slot.accept("A", .success(late)))
        XCTAssertNil(slot.summary(for: "B"))
        XCTAssertNil(slot.summary(for: "A"))
    }

    func test_theAnswerForTheSelectedSessionIsShownOnlyForIt() async throws {
        let answer = try await summary()
        var slot = PreviewSlot()
        slot.begin("B")
        XCTAssertTrue(slot.accept("B", .success(answer)))
        XCTAssertNotNil(slot.summary(for: "B"))
        XCTAssertNil(slot.summary(for: "A"))
    }

    func test_aFailureIsKeyedToo() {
        var slot = PreviewSlot()
        slot.begin("B")
        XCTAssertFalse(slot.accept("A", .failure(.unreachable)))
        XCTAssertNil(slot.failure(for: "B"))
        XCTAssertTrue(slot.accept("B", .failure(.unreachable)))
        XCTAssertEqual(slot.failure(for: "B"), .unreachable)
    }
}
#endif
