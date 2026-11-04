import TCBridge
import TCShellCore
import XCTest

/// The build channel, read through the real ABI: the one place Swift learns
/// which channel it is and its state directory (spec 4.1, 4.3). The TCShellCore tests inject fixtures; this checks the
/// dylib's answer decodes into the same shape.
final class BuildChannelBridgeTests: XCTestCase {
    func test_theDylibReportsADecodableChannel() throws {
        let channel = try XCTUnwrap(BuildChannel.decode(fromJSON: TCBuildChannel.json()))
        XCTAssertTrue(["production", "staging"].contains(channel.channel))
        XCTAssertTrue(channel.stateDirName.hasPrefix("trace-commons"))
        XCTAssertTrue(channel.stateDirEnv.hasSuffix("CONTRIBUTOR_DIR"))
        XCTAssertTrue(channel.urlScheme.hasPrefix("tracecommons"))
        XCTAssertTrue(channel.nearAiDashboardOrigin.hasPrefix("https://"))
        // Production offers the NEAR wallet; staging signs in through NEAR
        // AI's Google login only.
        if channel.channel == "staging" {
            XCTAssertEqual(channel.nearAiSignInProviders, ["google"])
        } else {
            XCTAssertEqual(channel.nearAiSignInProviders, ["near", "github", "google"])
        }
        XCTAssertEqual(
            ChannelAgreement.verdict(plistChannel: channel.channel, dylib: channel), .agreed(channel))
    }

    func test_theMismatchLineIsASentence() throws {
        let line = try XCTUnwrap(TCBuildChannel.mismatchLine())
        XCTAssertTrue(line.hasSuffix("."))
    }

    /// Spec S7, 4.8: a staging dylib gives the main window its label, a
    /// production one gives none. CI links only a production dylib, so there
    /// this kills "the bridge returns an empty string instead of nil"; that
    /// a staging build keeps its label is pinned by the staging channel
    /// tests in ci.yml (`a_staging_channel_shows_the_label` and the FFI
    /// `build_channel` suite run as staging), and here only against a
    /// locally built staging dylib.
    func test_onlyAStagingDylibGivesAWindowLabel() throws {
        let channel = try XCTUnwrap(BuildChannel.decode(fromJSON: TCBuildChannel.json()))
        let label = TCBuildChannel.windowLabel()
        XCTAssertEqual(label != nil, channel.channel == "staging")
        if let label { XCTAssertFalse(label.isEmpty) }
    }
}
