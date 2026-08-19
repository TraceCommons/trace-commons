import TCShellCore
import XCTest

/// Decoding and prettification for the removed-list screen.
///
/// The list itself is generated from the scrubber's table, so what is worth
/// testing here is everything AROUND that: that a malformed answer still
/// renders a screen, that order is preserved, and that an unrecognized
/// detector is de-slugged rather than dropped.
final class ScrubDetectorsTests: XCTestCase {
    func testSlugsDecodeInTheOrderTheScrubberReportsThem() {
        let json = #"["openai_api_key","github_token","jwt"]"#
        XCTAssertEqual(
            ScrubDetectors.slugs(fromJSON: json),
            ["openai_api_key", "github_token", "jwt"]
        )
    }

    func testKnownDetectorsGetHumanLabels() {
        XCTAssertEqual(ScrubDetectors.label(for: "openai_api_key"), "OpenAI API keys")
        XCTAssertEqual(ScrubDetectors.label(for: "pem_header_orphan"), "Private keys in PEM blocks")
        // Named rather than called "provider tokens", which tells a
        // contributor nothing about whether their own provider is covered.
        XCTAssertEqual(
            ScrubDetectors.label(for: "provider_token"),
            "Stripe, GitLab and Slack tokens"
        )
    }

    /// The property that matters most: a detector added upstream must still
    /// reach the contributor. Dropping it would leave the screen quietly
    /// describing an older build.
    func testAnUnknownDetectorIsDeslggedRatherThanDropped() {
        let json = #"["some_new_vendor_key"]"#
        XCTAssertEqual(ScrubDetectors.labels(fromJSON: json), ["some new vendor key"])
    }

    func testMalformedJSONYieldsNoRowsRatherThanThrowing() {
        // The caller is the first screen a contributor sees. It still has the
        // residual-risk concession to show, which is the honest half anyway.
        XCTAssertEqual(ScrubDetectors.slugs(fromJSON: "not json"), [])
        XCTAssertEqual(ScrubDetectors.slugs(fromJSON: "{}"), [])
        XCTAssertEqual(ScrubDetectors.slugs(fromJSON: ""), [])
    }

    func testLabelsPreserveOrder() {
        let json = #"["jwt","openai_api_key"]"#
        XCTAssertEqual(
            ScrubDetectors.labels(fromJSON: json),
            ["JSON Web Tokens", "OpenAI API keys"]
        )
    }
}
