import AppKit
import SwiftUI
import XCTest
@testable import TraceCommonsApp

@MainActor
final class InsightsSkillsCoverTests: XCTestCase {
    func testHashIsFNV1aNotTheLaunchSeededHasher() {
        // The published 64-bit FNV-1a test vectors.
        XCTAssertEqual(SkillCoverArt.fnv1a(""), 0xcbf2_9ce4_8422_2325)
        XCTAssertEqual(SkillCoverArt.fnv1a("a"), 0xaf63_dc4c_8601_ec8c)
        XCTAssertEqual(SkillCoverArt.fnv1a("foobar"), 0x8594_4171_f739_67e8)
    }

    func testTheSameSkillHasTheSameCoverOnEveryRun() {
        // Worked out outside the app from the hash, not read back from it: a
        // cover that changed between launches would fail here.
        XCTAssertEqual(SkillCoverArt.seed(""), 0xefd0_1f60_ba99_2926)
        XCTAssertEqual(SkillCoverArt.seed("a"), 0x82a2_a958_a9be_ce5b)
        let review = SkillCoverArt.Spec(skillID: "catalog-review", publisherID: "creator")
        XCTAssertEqual(review.palette, 5)
        XCTAssertEqual(review.pattern, .dots)
        XCTAssertEqual(review.anchorX, 253.0 / 255, accuracy: 1e-12)
        XCTAssertEqual(review.anchorY, 140.0 / 255, accuracy: 1e-12)
        XCTAssertEqual(review.step, 0.12 + 56.0 / 63 * 0.12, accuracy: 1e-12)
        XCTAssertEqual(review.angle, 143.0 / 255 * 70 - 35, accuracy: 1e-12)
        XCTAssertFalse(review.flipped)
        XCTAssertEqual(review, SkillCoverArt.Spec(skillID: "catalog-review", publisherID: "creator"))

        let brainstorming = SkillCoverArt.Spec(skillID: "catalog-brainstorming", publisherID: "creator")
        XCTAssertEqual(brainstorming.pattern, .rings)
        XCTAssertEqual(brainstorming.palette, review.palette, "One publisher's skills share its colours")
        XCTAssertNotEqual(brainstorming, review)

        let elsewhere = SkillCoverArt.Spec(skillID: "catalog-review", publisherID: "other")
        XCTAssertEqual(elsewhere.palette, 0)
        XCTAssertEqual(elsewhere.pattern, .rings)
        // Ids that differ only in a last digit still get different covers.
        let neighbours = (0 ..< 8).map { SkillCoverArt.Spec(skillID: "skill-\($0)", publisherID: "publisher-skill-\($0)") }
        XCTAssertGreaterThan(Set(neighbours.map(\.pattern)).count, 2)
        XCTAssertGreaterThan(Set(neighbours.map(\.palette)).count, 3)
        XCTAssertNotEqual(SkillCoverArt.Spec(skillID: "ab", publisherID: "c"), SkillCoverArt.Spec(skillID: "a", publisherID: "bc"),
                          "The ids are hashed apart, not run together")
    }

    func testEveryCoverIsInRange() {
        for index in 0 ..< 500 {
            let spec = SkillCoverArt.Spec(skillID: "skill-\(index)", publisherID: "publisher-\(index % 17)")
            XCTAssertTrue(SkillCoverArt.palettes.indices.contains(spec.palette))
            XCTAssertTrue((0 ... 1).contains(spec.anchorX) && (0 ... 1).contains(spec.anchorY))
            XCTAssertTrue((0.12 ... 0.24 + 1e-9).contains(spec.step))
            XCTAssertTrue((-35.0 ... 35).contains(spec.angle))
        }
        let patterns = Set((0 ..< 200).map { SkillCoverArt.Spec(skillID: "skill-\($0)", publisherID: "p").pattern })
        XCTAssertEqual(patterns, Set(SkillCoverArt.Pattern.allCases))
        let palettes = Set((0 ..< 200).map { SkillCoverArt.Spec(skillID: "s", publisherID: "publisher-\($0)").palette })
        XCTAssertEqual(palettes.count, SkillCoverArt.palettes.count)
    }

    func testWhiteInitialsHoldContrastOnEveryPaletteInBothAppearances() {
        // A cover is not re-coloured for light or dark, so one check covers
        // both. The pattern adds white at 16%, which only lightens: the
        // corner mark sits on its own dark capsule, and the tile's initials
        // are checked against that lightened ground too.
        for (index, palette) in SkillCoverArt.palettes.enumerated() {
            for stop in [palette.from, palette.to] {
                XCTAssertGreaterThanOrEqual(stop.contrastWithWhite, 4.5, "palette \(index)")
                let lightened = SkillCoverArt.RGB(red: stop.red + (1 - stop.red) * 0.16, green: stop.green + (1 - stop.green) * 0.16,
                                                  blue: stop.blue + (1 - stop.blue) * 0.16)
                XCTAssertGreaterThanOrEqual(lightened.contrastWithWhite, 4.5, "palette \(index) under the pattern")
            }
        }
        XCTAssertEqual(SkillCoverArt.RGB(0xFFFFFF).contrastWithWhite, 1, accuracy: 1e-9)
        XCTAssertEqual(SkillCoverArt.RGB(0x000000).contrastWithWhite, 21, accuracy: 1e-9)
    }

    func testInitialsComeFromThePublishersWords() {
        XCTAssertEqual(SkillCoverArt.initials("Obra"), "OB")
        XCTAssertEqual(SkillCoverArt.initials("Trace Commons Team"), "TC")
        XCTAssertEqual(SkillCoverArt.initials("creator/skills"), "CS")
        XCTAssertEqual(SkillCoverArt.initials("x"), "X")
        XCTAssertEqual(SkillCoverArt.initials(" -- "), "")
    }

    private func png(_ view: some View, scheme: ColorScheme) throws -> Data {
        let renderer = ImageRenderer(content: view.environment(\.colorScheme, scheme))
        renderer.scale = 2
        let image = try XCTUnwrap(renderer.cgImage)
        return try XCTUnwrap(NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]))
    }

    func testACoverDrawsTheSamePixelsEachTimeAndInBothAppearances() throws {
        func sheet(_ ids: [String], initials: String = "OB") -> some View {
            HStack(spacing: 8) {
                ForEach(ids, id: \.self) { id in
                    SkillCover(spec: .init(skillID: id, publisherID: "publisher-" + id), initials: initials).frame(width: 96, height: 64)
                }
            }.padding(8)
        }
        let ids = (0 ..< 8).map { "skill-\($0)" }
        let first = try png(sheet(ids), scheme: .dark)
        XCTAssertEqual(first, try png(sheet(ids), scheme: .dark))
        XCTAssertNotEqual(first, try png(sheet(ids.reversed()), scheme: .dark))
        // The gradient and the pattern are compared without the initials:
        // the system smooths text differently in the two appearances.
        XCTAssertEqual(try png(sheet(ids, initials: ""), scheme: .dark), try png(sheet(ids, initials: ""), scheme: .light),
                       "Artwork does not change with the appearance")
        XCTAssertGreaterThan(first.count, 5_000)
        if let directory = ProcessInfo.processInfo.environment["TC_SKILLS_COVER_RENDER_DIR"] {
            try FileManager.default.createDirectory(atPath: directory, withIntermediateDirectories: true)
            try first.write(to: URL(fileURLWithPath: directory).appendingPathComponent("skills-covers.png"))
        }
    }
}
