import TCShellCore
import XCTest

/// Decoding and prettification for the removed-list screen.
///
/// The list itself is generated from the scrubber's table, so what is worth
/// testing here is everything AROUND that: that a malformed answer still
/// renders a screen, that order is preserved, and that an unrecognized
/// detector is de-slugged rather than dropped.
/// Locates `tests/fixtures/scrub-detectors/` from this source file's own
/// path, the way `ConformanceFixtures` locates the update fixtures. The file
/// lives outside the SwiftPM package because the GTK and Windows shells read
/// the same one; copying it into a resource bundle would produce two files
/// that drift, which is the drift it exists to prevent.
private enum ScrubLabelFixture {
    static func labels() throws -> [String: String] {
        // .../macos/Tests/TCShellCoreTests/ScrubDetectorsTests.swift
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()   // TCShellCoreTests
            .deletingLastPathComponent()   // Tests
            .deletingLastPathComponent()   // macos
            .deletingLastPathComponent()   // <repo root>
            .appendingPathComponent("tests/fixtures/scrub-detectors/labels.json")
        let data = try Data(contentsOf: url)
        guard
            let object = try JSONSerialization.jsonObject(with: data) as? [String: Any],
            let labels = object["labels"] as? [String: String]
        else {
            throw NSError(
                domain: "ScrubLabelFixture", code: 1,
                userInfo: [NSLocalizedDescriptionKey: "labels.json has no `labels` object"]
            )
        }
        return labels
    }
}

final class ScrubDetectorsTests: XCTestCase {
    /// The words themselves, against the copy the other two shells read.
    ///
    /// `testEveryDetectorHasAHumanLabel` in `TCBridgeTests` proves a label
    /// EXISTS for every detector the real scrubber reports. It cannot see
    /// what the label says, and each shell hardcodes its own nine strings, so
    /// all three could satisfy their coverage guards while telling
    /// contributors three different things about the same detector.
    ///
    /// Iterates the fixture rather than this shell's table, so a detector
    /// this shell forgot fails here too, not only upstream.
    func testScrubDetectorLabelsMatchTheSharedFixture() throws {
        let labels = try ScrubLabelFixture.labels()

        // An empty or misread fixture would make the loop below assert
        // nothing at all.
        XCTAssertGreaterThanOrEqual(
            labels.count, 9,
            "the shared fixture lists only \(labels.count) detectors; a short list silently weakens this test"
        )

        for (slug, want) in labels {
            XCTAssertEqual(
                ScrubDetectors.label(for: slug), want,
                "this shell words \(slug) differently from the shared fixture, which the GTK and Windows shells also read"
            )
        }
    }

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
        // Cursor is its own detector rather than a fold into the line above,
        // so a Cursor user can find their own key in the list.
        XCTAssertEqual(ScrubDetectors.label(for: "cursor_api_key"), "Cursor API keys")
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
