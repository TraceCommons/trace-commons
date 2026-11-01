import TCDesign
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// Owner, 2026-10-31: a favorite stays among the favorites, ahead of every
/// group, and shows its privacy tier with a dot and the tier's heading as
/// its accessibility value, the heading the tier's group already carries.
@MainActor
final class ManagedModelPickerTierTests: XCTestCase {
    private static let copy = [
        "model_default": "Default: {model}", "model_default_unknown": "Default",
        "model_group_incognito": "Incognito models", "model_group_near_ai": "NEAR AI private models",
        "model_group_external": "External private models",
    ]

    func testAFavoriteShowsItsTiersDotAndHeading() throws {
        let catalogue = try JSONDecoder().decode(ManagedModelCatalogue.self, from: Data(#"""
        {"models":[
          {"id":"i/star","tier":"incognito"},
          {"id":"n/star","tier":"near_ai"},
          {"id":"e/star","tier":"external"},
          {"id":"x/star"},
          {"id":"n/plain","tier":"near_ai"}
        ],"tier_order":["near_ai","external","incognito"],"default":null,
        "favorites":["i/star","n/star","e/star","x/star"]}
        """#.utf8))
        let options = ManagedFolderOptions.pickerOptions(catalogue, chosen: nil, copy: { Self.copy[$0] ?? "" })
        XCTAssertEqual(options.map(\.value), ["", "i/star", "n/star", "e/star", "x/star", "n/plain"])
        XCTAssertEqual(options.map(\.dot), [nil, .outside, .on, .ask, nil, nil])
        XCTAssertEqual(options.map(\.detail), [
            nil, "Incognito models", "NEAR AI private models", "External private models", nil, nil,
        ])
        // The grouped model keeps its heading and gets no dot.
        XCTAssertEqual(options.last?.section, "NEAR AI private models")
    }

    func testAChosenModelTheCatalogueLacksIsKeptWithoutATier() throws {
        let catalogue = try JSONDecoder().decode(ManagedModelCatalogue.self, from: Data(
            #"{"models":[{"id":"a/b"}],"default":null}"#.utf8))
        let options = ManagedFolderOptions.pickerOptions(catalogue, chosen: "gone/model", copy: { Self.copy[$0] ?? "" })
        XCTAssertEqual(options.map(\.value), ["", "a/b", "gone/model"])
        XCTAssertNil(options.last?.dot)
    }
}
