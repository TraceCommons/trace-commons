import SwiftUI
import XCTest

@testable import TCDesign

/// The components, layout metrics and states brought to #1146 for #1268
/// (owner ruling, 2026-10-07: #1146 wins for theming, layout, components
/// and states, except the accepted differences).
final class ComponentParityTests: XCTestCase {
    // MARK: Glyphs

    /// Every glyph parses, and each is drawn at #1146's size and stroke.
    func test_everyGlyphParsesAtTheReferenceSize() throws {
        for glyph in GlassGlyph.allCases {
            let path = try XCTUnwrap(glyph.path, "\(glyph) does not parse")
            XCTAssertFalse(path.isEmpty, "\(glyph)")
            // The artwork stays inside its viewBox, give or take a stroke.
            let box = glyph.artwork.viewBox.insetBy(dx: -1, dy: -1)
            XCTAssertTrue(box.contains(path.boundingRect), "\(glyph) runs outside its viewBox")
        }
        XCTAssertEqual(GlassGlyph.viewMenu.artwork.size, CGSize(width: 14, height: 12))
        XCTAssertEqual(GlassGlyph.gear.artwork.size, CGSize(width: 15, height: 15))
        XCTAssertEqual(GlassGlyph.chevronDown.artwork.size, CGSize(width: 10, height: 10))
        XCTAssertEqual(GlassGlyph.folder.artwork.size, CGSize(width: 13, height: 13))
        XCTAssertEqual(GlassGlyph.kebab.artwork.size, CGSize(width: 4, height: 14))
        // A stroke in viewBox units scales with the glyph: the 1.8 chevron
        // stroke in a 16 box drawn at 10 is 1.125pt.
        XCTAssertEqual(try XCTUnwrap(GlassGlyphView.lineWidth(.chevronDown)), 1.125, accuracy: 0.0001)
        XCTAssertEqual(try XCTUnwrap(GlassGlyphView.lineWidth(.viewMenu)), 1.4, accuracy: 0.0001)
        XCTAssertNil(GlassGlyphView.lineWidth(.kebab), "the kebab's dots are filled")
    }

    // MARK: Components

    /// The folder button: 10pt sides, bold (#1146 `.tc-btn--folder`).
    func test_theFolderButtonIsTheReferenceButton() {
        XCTAssertEqual(GlassFolderButton.horizontalPadding, 10)
        XCTAssertEqual(GlassFolderButton.type.weight, .bold)
    }

    /// The expander's 6 / 2 / 2 padding (#1146 `.tc-expander`).
    func test_theExpanderPadding() {
        XCTAssertEqual(GlassExpander.padding, EdgeInsets(top: 6, leading: 2, bottom: 2, trailing: 2))
    }

    /// A key-value list sits 6pt in from each side (#1146 `.tc-kv`).
    func test_theKeyValueListInset() {
        XCTAssertEqual(GlassKeyValueList.inset, 6)
    }

    /// The tree's folder and session tiles carry #1146's marks.
    func test_theTileMarks() {
        XCTAssertEqual(GlassToolTile.folderMark, "dir")
        XCTAssertEqual(GlassToolTile.sessionMark, "▤")
    }

    /// A row reserves the watch switch's column when it has no switch, so a
    /// session's pill lines up with its folder's (#1146's row grid).
    func test_aRowReservesTheWatchColumn() throws {
        XCTAssertEqual(GlassListRow.watchColumn, 38)
        let rows = try XCTUnwrap(Dictionary(uniqueKeysWithValues: try DesignSources.components())["ListRow.swift"])
        XCTAssertTrue(rows.contains("Color.clear.accessibilityHidden(true)"))
        XCTAssertTrue(rows.contains(".frame(width: Self.watchColumn)"))
    }

    /// The floating segmented control's chosen segment: 18% white, no edge.
    func test_theFloatingSelection() {
        XCTAssertEqual(GlassTokens.Color.controlSelectedFloating.alpha, 0.18, accuracy: 0.0001)
        XCTAssertNotNil(GlassTokens.Color.controlSelectedFloating.lightRGB)
    }

    /// The bar graph's labels are `statusOff`, primary under the pointer,
    /// and its track has #1146's inner top highlight.
    func test_theBarGraphTrackAndLabels() {
        XCTAssertEqual(GlassBarGraph.labelInk(hovered: false), GlassTokens.Color.statusOff)
        XCTAssertEqual(GlassBarGraph.labelInk(hovered: true), GlassTokens.Color.textPrimary)
        let edge = GlassTokens.Shadow.barTrackEdge
        XCTAssertEqual(edge.count, 1)
        XCTAssertTrue(edge[0].inset)
        XCTAssertEqual(edge[0].y, 1)
    }

    /// A later window slides in from the trailing side, an earlier one from
    /// the leading side; the same window does not slide.
    func test_theBarGraphSlidesFromTheSideTheWindowLiesOn() {
        XCTAssertEqual(GlassBarSlide(from: -1, to: 0), .fromTrailing)
        XCTAssertEqual(GlassBarSlide(from: 0, to: -1), .fromLeading)
        XCTAssertEqual(GlassBarSlide(from: 0, to: 0), .none)
    }

    /// The map field is #1146's ellipse: 80% by 60%, centred at 50% / 55%.
    func test_theMapFieldGeometry() {
        XCTAssertEqual(GlassMapField.radii, CGSize(width: 0.8, height: 0.6))
        XCTAssertEqual(GlassMapField.center, UnitPoint(x: 0.5, y: 0.55))
    }

    /// A sheet is a padded column with no tier of its own.
    func test_aSheetIsNotAPaneOnAPane() throws {
        let surfaces = try XCTUnwrap(Dictionary(uniqueKeysWithValues: try DesignSources.components())["Surfaces.swift"])
        let sheet = try XCTUnwrap(surfaces.range(of: "public struct GlassSheet").map { String(surfaces[$0.lowerBound...].prefix(1200)) })
        XCTAssertFalse(sheet.contains(".glassTier("))
    }

    // MARK: Modal

    /// The header centres its title on the close button, the scrim closes
    /// the topmost modal, and the modal fades over #1146's `--tc-dur`.
    func test_theModalHeaderScrimAndFade() throws {
        let modal = try XCTUnwrap(Dictionary(uniqueKeysWithValues: try DesignSources.components())["Modal.swift"])
        XCTAssertTrue(modal.contains("HStack(alignment: .center, spacing: GlassTokens.Space.s6)"))
        XCTAssertTrue(modal.contains(".preference(key: GlassModalScrimAction.self, value: GlassModalScrimAction.Action(run: onCancel))"))
        XCTAssertTrue(modal.contains("GlassModalScrim(onTap: isTopmost ? action?.run : nil)"))
        XCTAssertTrue(modal.contains(".onTapGesture { onTap?() }"))
        XCTAssertTrue(modal.contains(".animation(GlassMotion.standard(reduceMotion), value: requests.map(\\.id))"))
    }

    /// The scrim takes the innermost modal's cancel: the modal sets it, and
    /// nothing a layer further out holds replaces it.
    func test_theScrimTakesTheModalsOwnCancel() {
        var first = 0
        var value: GlassModalScrimAction.Action? = GlassModalScrimAction.Action(run: { first += 1 })
        GlassModalScrimAction.reduce(value: &value) { GlassModalScrimAction.Action(run: { first += 10 }) }
        value?.run()
        XCTAssertEqual(first, 1)
        var empty: GlassModalScrimAction.Action?
        GlassModalScrimAction.reduce(value: &empty) { GlassModalScrimAction.Action(run: { first += 100 }) }
        empty?.run()
        XCTAssertEqual(first, 101)
    }

    // MARK: Layout

    /// The inspector's insets, the tree's inset and the map's overlay inset
    /// (#1146 `monitor-shell.tsx`, `flow-map.tsx`).
    func test_layoutMetrics() {
        XCTAssertEqual(GlassPaneInsets.inspector, EdgeInsets(top: 18, leading: 16, bottom: 18, trailing: 16))
        XCTAssertEqual(GlassTokens.Space.treeInset, 8)
        XCTAssertEqual(GlassTokens.Space.mapOverlayInset, 14)
        XCTAssertEqual(GlassTokens.Radius.menuPanel, GlassTokens.Radius.card, "the menu-bar panel is #1146's popover")
        // The panes sit at the window's edge, 10pt apart.
        XCTAssertEqual(GlassTokens.Space.windowPadding, 0)
        XCTAssertEqual(GlassTokens.Space.paneGap, 10)
    }
}
