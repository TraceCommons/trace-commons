import XCTest

@testable import TCShellCore

/// Spec 4.3: a staging bundle around a production dylib would show one
/// channel and talk to the other. The app starts nothing unless the bundle's
/// `TCChannel` and the dylib's channel are the same string.
final class ChannelAgreementTests: XCTestCase {
    func testAgreeingChannelsStart() {
        XCTAssertEqual(
            ChannelAgreement.verdict(plistChannel: "production", dylib: BuildChannelFixture.production),
            .agreed(BuildChannelFixture.production))
        XCTAssertEqual(
            ChannelAgreement.verdict(plistChannel: "staging", dylib: BuildChannelFixture.staging),
            .agreed(BuildChannelFixture.staging))
    }

    func testAChannelMismatchRefusesToStart() {
        XCTAssertEqual(
            ChannelAgreement.verdict(plistChannel: "staging", dylib: BuildChannelFixture.production),
            .refused)
        XCTAssertEqual(
            ChannelAgreement.verdict(plistChannel: "production", dylib: BuildChannelFixture.staging),
            .refused)
        XCTAssertEqual(
            ChannelAgreement.verdict(plistChannel: "Production", dylib: BuildChannelFixture.production),
            .refused, "an exact match only")
    }

    func testAMissingPlistChannelRefusesToStart() {
        XCTAssertEqual(
            ChannelAgreement.verdict(plistChannel: nil, dylib: BuildChannelFixture.production), .refused)
        XCTAssertEqual(
            ChannelAgreement.verdict(plistChannel: "", dylib: BuildChannelFixture.production), .refused)
    }

    func testAnUnreadableDylibChannelRefusesToStart() {
        XCTAssertEqual(ChannelAgreement.verdict(plistChannel: "production", dylib: nil), .refused)
    }

    func testTheDylibJSONDecodes() throws {
        let json = #"{"channel":"staging","state_dir_name":"trace-commons-staging","state_dir_env":"TRACE_COMMONS_STAGING_CONTRIBUTOR_DIR","url_scheme":"tracecommons-staging","near_ai_dashboard_origin":"https://cloud-stg.near.ai","near_ai_sign_in_providers":["google"]}"#
        XCTAssertEqual(BuildChannel.decode(fromJSON: json), BuildChannelFixture.staging)
        XCTAssertNil(BuildChannel.decode(fromJSON: nil))
        XCTAssertNil(BuildChannel.decode(fromJSON: "{}"))
        XCTAssertNil(
            BuildChannel.decode(fromJSON: json.replacingOccurrences(of: #""channel":"staging""#, with: #""channel":"""#)),
            "an empty field is unreadable, not a channel")
        XCTAssertNil(
            BuildChannel.decode(fromJSON: json.replacingOccurrences(
                of: #","near_ai_dashboard_origin":"https://cloud-stg.near.ai""#, with: "")),
            "a missing dashboard origin is unreadable, not a default")
        let providers = #","near_ai_sign_in_providers":["google"]"#
        XCTAssertNil(
            BuildChannel.decode(fromJSON: json.replacingOccurrences(of: providers, with: "")),
            "a missing provider list is unreadable, not a default")
        XCTAssertNil(
            BuildChannel.decode(fromJSON: json.replacingOccurrences(
                of: providers, with: #","near_ai_sign_in_providers":[]"#)),
            "an empty provider list is unreadable: a build offers at least one sign-in")
    }
}
