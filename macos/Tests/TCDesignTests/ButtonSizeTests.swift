import SwiftUI
import XCTest

@testable import TCDesign

/// Owner ruling, 2026-10-08: in a window's action bar every button is the
/// primary CTA's size; only inline buttons take the smaller size.
final class ButtonSizeTests: XCTestCase {
    private let barKinds: [GlassButtonKind] = [.secondary, .glass, .destructive, .link]

    func test_aBarButtonOfEveryKindIsTheCTAsSize() {
        let cta = GlassButtonStyle.metrics(.primary, size: .inline)
        XCTAssertEqual(cta.minHeight, GlassTokens.Size.cta)
        XCTAssertEqual(cta.type.size, GlassTokens.TypeScale.bodyStrong.size)
        for kind in barKinds {
            let bar = GlassButtonStyle.metrics(kind, size: .bar)
            XCTAssertEqual(bar.minHeight, GlassTokens.Size.cta, "\(kind)")
            XCTAssertEqual(bar.horizontalPadding, cta.horizontalPadding, "\(kind)")
            XCTAssertEqual(bar.type.size, cta.type.size, "\(kind)")
        }
        // The primary reads the same at either size.
        XCTAssertEqual(GlassButtonStyle.metrics(.primary, size: .bar), cta)
    }

    func test_inlineGlassAndDestructiveAreControlLarge() {
        for kind in [GlassButtonKind.glass, .destructive] {
            let inline = GlassButtonStyle.metrics(kind, size: .inline)
            XCTAssertEqual(inline.minHeight, GlassTokens.Size.controlLarge, "\(kind)")
            XCTAssertEqual(inline.type.size, GlassTokens.TypeScale.label.size, "\(kind)")
            XCTAssertEqual(inline.horizontalPadding, GlassButtonStyle.inlinePadding, "\(kind)")
        }
        XCTAssertNil(GlassButtonStyle.metrics(.link, size: .inline).minHeight, "the inline link has no container")
    }

    /// A bar beside the small CTA (a modal's) matches the small CTA.
    func test_aSmallBarMatchesTheSmallCTA() {
        let small = GlassButtonStyle.metrics(.primary, size: .inline, small: true)
        for kind in barKinds {
            let bar = GlassButtonStyle.metrics(kind, size: .bar, small: true)
            XCTAssertEqual(bar.minHeight, small.minHeight, "\(kind)")
            XCTAssertEqual(bar.horizontalPadding, small.horizontalPadding, "\(kind)")
            XCTAssertEqual(bar.type.size, small.type.size, "\(kind)")
        }
    }

    /// Bar keeps each kind's own weight: the glass pill is not bolded into
    /// a second CTA.
    func test_aBarButtonKeepsItsKindsWeight() {
        XCTAssertEqual(GlassButtonStyle.metrics(.glass, size: .bar).type.weight, .semibold)
        XCTAssertEqual(GlassButtonStyle.metrics(.secondary, size: .bar).type.weight, .bold)
        XCTAssertEqual(GlassButtonStyle.metrics(.primary, size: .bar).type.weight, .bold)
    }

    /// The modal's action bar sizes every action against its small CTA.
    @MainActor
    func test_theModalsActionBarIsBarSized() throws {
        let cancel = GlassModalAction.cancel("Cancel") {}
        let confirm = GlassModalAction("Confirm", isDefault: true) {}
        let cancelBox = GlassModal<EmptyView>.actionMetrics(cancel, isDefault: false)
        let confirmBox = GlassModal<EmptyView>.actionMetrics(confirm, isDefault: true)
        XCTAssertEqual(cancelBox.minHeight, confirmBox.minHeight)
        XCTAssertEqual(cancelBox.horizontalPadding, confirmBox.horizontalPadding)
        XCTAssertEqual(cancelBox.type.size, confirmBox.type.size)
        let sources = Dictionary(uniqueKeysWithValues: try DesignSources.components())
        let modal = try XCTUnwrap(sources["Modal.swift"])
        XCTAssertTrue(modal.contains(
            ".buttonStyle(GlassButtonStyle(GlassModalAction.kind(action, isDefault: isDefault), size: .bar, small: true))"))
    }
}
