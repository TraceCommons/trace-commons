import Foundation
import TCBridge
import TCShellCore
import XCTest

/// The other-channel refusal through the real ABI (`tc_resolve_state_dir`,
/// staging design 4.4). The app opens its directory with
/// `ConfigStore::open`, which checks nothing, so this check is the only
/// thing that stops an override from pointing one channel's app at the
/// other channel's enrollment.
final class StateDirectoryBridgeTests: XCTestCase {
    private func channel() throws -> BuildChannel {
        try XCTUnwrap(BuildChannel.decode(fromJSON: TCBuildChannel.json()))
    }

    /// The other channel's default directory. The channel JSON deliberately
    /// carries only this build's names, so the other channel's name is
    /// pinned here, for this test only.
    private func otherChannelDefault(_ channel: BuildChannel) -> String {
        let other = channel.channel == "production" ? "trace-commons-staging" : "trace-commons"
        return NSHomeDirectory() + "/Library/Application Support/" + other
    }

    private let check = StateDirectory.OverrideCheck { dir in
        TCBuildChannel.overrideRefusal(dir).map {
            .init(label: $0, sentence: TCBuildChannel.overrideRefusedLine() ?? $0)
        }
    }

    func test_anOverrideNamingTheOtherChannelDirectoryIsRefused() throws {
        let channel = try channel()
        let other = otherChannelDefault(channel)
        XCTAssertEqual(TCBuildChannel.overrideRefusal(other), "state-dir-other-channel")
        XCTAssertEqual(
            TCBuildChannel.overrideRefusal(other + "/nested"), "state-dir-other-channel")
        // APFS is case-insensitive by default, whether or not it exists yet.
        let parent = (other as NSString).deletingLastPathComponent
        let upper = parent + "/" + (other as NSString).lastPathComponent.uppercased()
        XCTAssertEqual(TCBuildChannel.overrideRefusal(upper), "state-dir-other-channel")

        for resolve in [StateDirectory.resolve, StateDirectory.resolveCompute] {
            XCTAssertThrowsError(
                try resolve(
                    channel, nil, [channel.stateDirEnv: other], NSHomeDirectory(),
                    .init { _ in .absent }, check)
            ) { error in
                guard case StateDirectory.Refusal.otherChannel = error else {
                    return XCTFail("expected otherChannel, got \(error)")
                }
                XCTAssertFalse("\(error)".contains(other), "the sentence names no path")
            }
        }
    }

    func test_anOrdinaryOverrideIsAllowed() throws {
        let channel = try channel()
        let own = NSTemporaryDirectory() + "tc-state-dir-bridge-test"
        XCTAssertNil(TCBuildChannel.overrideRefusal(own))
        XCTAssertNil(
            TCBuildChannel.overrideRefusal(
                NSHomeDirectory() + "/Library/Application Support/" + channel.stateDirName))
        let resolved = try StateDirectory.resolve(
            channel: channel, explicit: nil, environment: [channel.stateDirEnv: own],
            probe: .init { _ in .absent }, overrideCheck: check)
        XCTAssertEqual(resolved.path, own)
    }

    func test_theOverrideRefusedLineIsASentence() throws {
        let line = try XCTUnwrap(TCBuildChannel.overrideRefusedLine())
        XCTAssertTrue(line.hasSuffix("."))
        XCTAssertTrue(line.contains("Nothing is being watched or sent"))
    }
}
