import TCBridge
import TCShellCore
import XCTest

/// The build channel, read through the real ABI: the one place Swift learns
/// which channel it is, its state directory and its passkey relying party
/// (spec 4.1, 4.3). The TCShellCore tests inject fixtures; this checks the
/// dylib's answer decodes into the same shape.
final class BuildChannelBridgeTests: XCTestCase {
    func test_theDylibReportsADecodableChannel() throws {
        let channel = try XCTUnwrap(BuildChannel.decode(fromJSON: TCBuildChannel.json()))
        XCTAssertTrue(["production", "staging"].contains(channel.channel))
        XCTAssertTrue(channel.stateDirName.hasPrefix("trace-commons"))
        XCTAssertTrue(channel.stateDirEnv.hasSuffix("CONTRIBUTOR_DIR"))
        XCTAssertFalse(channel.passkeyRelyingParty.isEmpty)
        XCTAssertTrue(channel.urlScheme.hasPrefix("tracecommons"))
        XCTAssertTrue(channel.nearAiDashboardOrigin.hasPrefix("https://"))
        XCTAssertEqual(
            ChannelAgreement.verdict(plistChannel: channel.channel, dylib: channel), .agreed(channel))
    }

    func test_theMismatchLineIsASentence() throws {
        let line = try XCTUnwrap(TCBuildChannel.mismatchLine())
        XCTAssertTrue(line.hasSuffix("."))
    }
}
