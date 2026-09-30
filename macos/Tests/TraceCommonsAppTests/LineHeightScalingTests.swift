import AppKit
import Foundation
import SwiftUI
import XCTest
@testable import TraceCommonsApp

/// Leading has to be derived from the size the type is drawn at, never
/// written down beside it.
///
/// `LineHeight.spacing(for:_:)` takes points, and after PR #636 every SF face
/// in this shell is a text style, which grows with the system text size. A
/// call site that passed the style's BASE size as a literal --
/// `spacing(for: 11, ...)` beside a `Font.subheadline` face -- was correct at
/// the default text size and proportionally too tight at every larger one.
///
/// Correcting today's numbers would not have fixed the class, because nothing
/// kept the literal and the face agreeing. `Face`/`tcType(_:)` carry the two
/// together and resolve the size, and the scan below is what keeps a literal
/// from coming back.
final class LineHeightScalingTests: XCTestCase {
    private static let faces: [(name: String, face: TC.Font_.Face, appKit: NSFont.TextStyle)] = [
        ("bodyText", TC.Font_.bodyText, .body),
        ("captionText", TC.Font_.captionText, .subheadline),
        ("footnoteText", TC.Font_.footnoteText, .caption1),
        ("monoCodeText", TC.Font_.monoCodeText, .subheadline),
        ("monoTranscriptText", TC.Font_.monoTranscriptText, .subheadline),
    ]

    /// No call site names a point size. The bug class is a literal that has
    /// to agree with something declared elsewhere and has nothing keeping it
    /// honest, so the literal itself is what this forbids.
    func testNoCallSitePassesALiteralPointSizeToSpacing() throws {
        let sources = try Self.shellSources()
        XCTAssertGreaterThanOrEqual(
            sources.count, 60,
            "only \(sources.count) sources were scanned; the whole shell is expected")

        var calls = 0
        var failures: [String] = []
        for (path, text) in sources.sorted(by: { $0.key < $1.key }) {
            for (index, line) in text.components(separatedBy: "\n").enumerated() {
                guard let range = line.range(of: "LineHeight.spacing(for: ") else { continue }
                calls += 1
                let argument = line[range.upperBound...].prefix { $0 != "," && $0 != ")" }
                if argument.first?.isNumber == true {
                    failures.append(
                        "\(path):\(index + 1) passes the literal \(argument) as a point size. "
                            + "Derive it -- a face knows its own size; see TC.Font_.Face.")
                }
            }
        }

        // Without this the scan would pass over a shell that had renamed the
        // helper out from under it and checked nothing at all.
        XCTAssertGreaterThan(calls, 0, "no spacing(for:) call was found -- this scan proved nothing")
        XCTAssertTrue(failures.isEmpty, failures.joined(separator: "\n"))
    }

    /// Each face resolves its size from its own text style, which is what
    /// makes the leading follow the face.
    func testEachFaceResolvesItsSizeFromItsTextStyle() {
        for (name, face, appKit) in Self.faces {
            XCTAssertEqual(
                face.resolvedSize, NSFont.preferredFont(forTextStyle: appKit).pointSize,
                accuracy: 0.001, "\(name) resolves to the wrong text style's size")
            XCTAssertGreaterThan(face.resolvedSize, 0, "\(name) resolved to nothing")
        }
    }

    /// The style-to-AppKit mapping is not degenerate.
    ///
    /// A `switch` that answered `.body` for everything would satisfy the test
    /// above for `bodyText` and silently give every other face body leading.
    /// Body, caption and caption2 are three different sizes on macOS and must
    /// stay three different answers.
    func testTheTextStyleMappingIsNotDegenerate() {
        let body = TC.Font_.pointSize(of: .body)
        let subheadline = TC.Font_.pointSize(of: .subheadline)
        let caption = TC.Font_.pointSize(of: .caption)
        XCTAssertGreaterThan(body, subheadline)
        XCTAssertGreaterThan(subheadline, caption)
        XCTAssertGreaterThan(TC.Font_.pointSize(of: .largeTitle), body)
    }

    /// Leading is proportional to the size, which is the whole reason a
    /// frozen literal was wrong: at twice the size it must be twice the gap.
    func testSpacingIsProportionalToTheSizeItIsResolvedAgainst() {
        let small = TC.Font_.LineHeight.spacing(for: 11, TC.Font_.LineHeight.caption)
        let large = TC.Font_.LineHeight.spacing(for: 22, TC.Font_.LineHeight.caption)
        XCTAssertEqual(large, small * 2, accuracy: 0.001)
        XCTAssertGreaterThan(small, 0, "a multiple above 1.2 must add a gap")
        // A multiple at or below the 1.2 default line box adds nothing rather
        // than a negative gap.
        XCTAssertEqual(TC.Font_.LineHeight.spacing(for: 11, 1.0), 0)
    }

    /// The brand faces are `.custom`, so they do not scale -- but their size
    /// is still stated once and read from there by face, tracking and
    /// leading alike.
    func testTheBrandLeadingIsDerivedFromTheBrandSize() {
        XCTAssertEqual(
            CommunityBrand.Font_.ledeLineSpacing,
            TC.Font_.LineHeight.spacing(
                for: CommunityBrand.Font_.ledeSize, CommunityBrand.Font_.ledeLineHeight),
            accuracy: 0.001)
        XCTAssertEqual(
            CommunityBrand.Font_.bodyLineSpacing,
            TC.Font_.LineHeight.spacing(
                for: CommunityBrand.Font_.bodySize, CommunityBrand.Font_.bodyLineHeight),
            accuracy: 0.001)
    }

    /// `.../macos/Tests/TraceCommonsAppTests/<this file>` ->
    /// `.../macos/Sources`, located from this file's own path the way
    /// `ShellWordingTests` does.
    private static func shellSources() throws -> [String: String] {
        let base = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()  // TraceCommonsAppTests
            .deletingLastPathComponent()  // Tests
            .deletingLastPathComponent()  // macos
            .appendingPathComponent("Sources")
        let walker = try XCTUnwrap(
            FileManager.default.enumerator(at: base, includingPropertiesForKeys: nil))
        var scanned: [String: String] = [:]
        for case let url as URL in walker where url.pathExtension == "swift" {
            let relative = url.path.replacingOccurrences(of: base.path + "/", with: "")
            scanned[relative] = try String(contentsOf: url, encoding: .utf8)
        }
        return scanned
    }
}
