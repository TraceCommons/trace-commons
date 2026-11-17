import XCTest
import TCShellCore
@testable import TraceCommonsApp

@MainActor
final class InsightsSkillsFollowTests: XCTestCase {
    func testPendingBelongsToOneCreatorAndConfirmedStateWaitsForReply() async {
        let replies = FollowReplyGate()
        let model = InsightsSkillsModel(reader: { _ in .init(enabled: true, followedPublishers: ["old"]) },
            writer: { publisher, followed in await replies.write(publisher, followed: followed) })
        await model.load()
        let operation = Task { await model.setFollow(publisherID: "new", followed: true) }
        await replies.waitUntilStarted()
        XCTAssertEqual(model.pendingFollowPublisherID, "new")
        XCTAssertFalse(model.canFollow)
        XCTAssertEqual(model.shown?.followedPublishers, ["old"], "Pending is never a saved follow")
        await model.setFollow(publisherID: "other", followed: true)
        await model.load(refresh: true)
        await replies.finish(.init(enabled: true, followedPublishers: ["old", "new"]))
        await operation.value
        XCTAssertEqual(model.shown?.followedPublishers, ["old", "new"])
        XCTAssertNil(model.pendingFollowPublisherID)
        XCTAssertNil(model.followFailurePublisherID)
        XCTAssertTrue(model.canFollow)
        let writes = await replies.publishers
        XCTAssertEqual(writes, ["new"], "Reads and writes share a serial lane")
    }

    func testTransportFailureKeepsConfirmedFollowAndIdentifiesCreatorForRetry() async {
        let model = InsightsSkillsModel(reader: { _ in .init(enabled: true, followedPublishers: ["old"]) },
            writer: { _, _ in throw FollowFailure.offline })
        await model.load()
        await model.setFollow(publisherID: "old", followed: false)
        XCTAssertEqual(model.shown?.followedPublishers, ["old"])
        XCTAssertEqual(model.followFailurePublisherID, "old")
        XCTAssertEqual(model.failureKey, "skills_follow_failed")
        XCTAssertNil(model.pendingFollowPublisherID)
        XCTAssertTrue(model.canFollow)
    }

    func testConfirmedFollowSucceedsDespiteUnrelatedCatalogRefreshError() async {
        let model = InsightsSkillsModel(reader: { _ in .init(enabled: true) },
            writer: { _, _ in .init(enabled: true, error: "insights-skills-refresh-failed", followedPublishers: ["new"]) })
        await model.load()
        await model.setFollow(publisherID: "new", followed: true)
        XCTAssertEqual(model.shown?.followedPublishers, ["new"])
        XCTAssertNil(model.followFailurePublisherID, "Catalog refresh failure does not undo a persisted follow")
        XCTAssertEqual(model.failureKey, "skills_refresh_failed")
        XCTAssertNil(model.pendingFollowPublisherID)
    }

    func testUnconfirmedReplyIsFailureEvenWithoutDaemonError() async {
        let model = InsightsSkillsModel(reader: { _ in .init(enabled: true) },
            writer: { _, _ in .init(enabled: true) })
        await model.load()
        await model.setFollow(publisherID: "new", followed: true)
        XCTAssertEqual(model.shown?.followedPublishers, [])
        XCTAssertEqual(model.followFailurePublisherID, "new")
        XCTAssertEqual(model.failureKey, "skills_follow_failed")
    }

    func testRetryClearsOnlyFailureAfterDaemonConfirmation() async {
        let replies = FollowReplyGate()
        let model = InsightsSkillsModel(reader: { _ in .init(enabled: true) },
            writer: { publisher, followed in await replies.write(publisher, followed: followed) })
        await model.load()
        let failed = Task { await model.setFollow(publisherID: "new", followed: true) }
        await replies.waitUntilStarted()
        await replies.finish(.init(enabled: true, error: "failed"))
        await failed.value
        XCTAssertEqual(model.followFailurePublisherID, "new")
        let retry = Task { await model.setFollow(publisherID: "new", followed: true) }
        await replies.waitUntilStarted()
        XCTAssertNil(model.followFailurePublisherID)
        XCTAssertEqual(model.pendingFollowPublisherID, "new")
        await replies.finish(.init(enabled: true, followedPublishers: ["new"]))
        await retry.value
        XCTAssertEqual(model.shown?.followedPublishers, ["new"])
        XCTAssertNil(model.followFailurePublisherID)
        XCTAssertNil(model.failureKey)
    }

    func testCancelledNoncancellableReplyClearsPendingWithoutChangingConfirmedState() async {
        let replies = FollowReplyGate()
        let model = InsightsSkillsModel(reader: { _ in .init(enabled: true, followedPublishers: ["old"]) },
            writer: { publisher, followed in await replies.write(publisher, followed: followed) })
        await model.load()
        let operation = Task { await model.setFollow(publisherID: "new", followed: true) }
        await replies.waitUntilStarted()
        XCTAssertEqual(model.pendingFollowPublisherID, "new")
        operation.cancel()
        await replies.finish(.init(enabled: true, followedPublishers: ["old", "new"]))
        await operation.value
        XCTAssertEqual(model.shown?.followedPublishers, ["old"])
        XCTAssertNil(model.pendingFollowPublisherID)
        XCTAssertNil(model.followFailurePublisherID)
        XCTAssertNil(model.failureKey)
        XCTAssertFalse(model.busy)
    }

    func testUnavailableReplyRevokesFollowControls() async {
        for reply in [DaemonData.SkillsSnapshot(enabled: true, readable: false), .init(enabled: false)] {
            let model = InsightsSkillsModel(reader: { _ in .init(enabled: true, followedPublishers: ["old"]) },
                writer: { _, _ in reply })
            await model.load()
            await model.setFollow(publisherID: "new", followed: true)
            XCTAssertFalse(model.canFollow)
            XCTAssertNil(model.pendingFollowPublisherID)
            XCTAssertTrue(model.shown == nil || model.shown?.enabled == false)
        }
    }

    func testDisabledAndMissingSnapshotsRejectFollowWrites() async {
        for snapshot in [DaemonData.SkillsSnapshot(enabled: false), .init(enabled: true, readable: false)] {
            let model = InsightsSkillsModel(reader: { _ in snapshot }, writer: { _, _ in
                XCTFail("Unavailable Skills must not issue a follow write")
                return .init(enabled: true)
            })
            await model.load()
            XCTAssertFalse(model.canFollow)
            await model.setFollow(publisherID: "new", followed: true)
            XCTAssertNil(model.pendingFollowPublisherID)
        }
        let unloaded = InsightsSkillsModel(writer: { _, _ in
            XCTFail("A confirmed snapshot is required before following")
            return .init(enabled: true)
        })
        XCTAssertFalse(unloaded.canFollow)
        await unloaded.setFollow(publisherID: "new", followed: true)
    }
}

private enum FollowFailure: Error { case offline }

/// Only the IPC boundary is delayed; assertions exercise the real observable model.
private actor FollowReplyGate {
    private(set) var publishers: [String] = []
    private var reply: CheckedContinuation<DaemonData.SkillsSnapshot, Never>?
    func write(_ publisher: String, followed: Bool) async -> DaemonData.SkillsSnapshot {
        publishers.append(publisher)
        return await withCheckedContinuation { reply = $0 }
    }
    func waitUntilStarted() async {
        while reply == nil { await Task.yield() }
    }
    func finish(_ snapshot: DaemonData.SkillsSnapshot) {
        reply?.resume(returning: snapshot)
        reply = nil
    }
}
