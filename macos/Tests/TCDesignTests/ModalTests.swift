import AppKit
import SwiftUI
import XCTest

@testable import TCDesign

/// `GlassModal`: footer order, the keys it answers, and its presentation.
@MainActor
final class ModalTests: XCTestCase {
    private func action(_ title: String, _ role: GlassModalAction.Role, isDefault: Bool = false) -> GlassModalAction {
        GlassModalAction(title, role: role, isDefault: isDefault) {}
    }

    /// Cancel left-most, the others as given, the destructive action
    /// right-most.
    func test_theFooterOrder() {
        let given = [
            action("delete", .destructive), action("one", .standard), action("cancel", .cancel),
            action("two", .standard),
        ]
        XCTAssertEqual(GlassModalAction.ordered(given).map(\.title), ["cancel", "one", "two", "delete"])
    }

    /// Return takes the default action, never a destructive one or a cancel.
    func test_returnNeverTakesADestructiveAction() {
        XCTAssertNil(GlassModalAction.defaultAction(in: [action("delete", .destructive, isDefault: true)]))
        XCTAssertNil(GlassModalAction.defaultAction(in: [action("cancel", .cancel, isDefault: true)]))
        XCTAssertEqual(
            GlassModalAction.defaultAction(in: [action("delete", .destructive, isDefault: true), action("save", .standard, isDefault: true)])?.title,
            "save")
        XCTAssertNil(GlassModalAction.defaultAction(in: [action("save", .standard)]))
    }

    func test_theKindEachActionDrawsIn() {
        XCTAssertEqual(GlassModalAction.kind(action("d", .destructive), isDefault: false), .destructive)
        XCTAssertEqual(GlassModalAction.kind(action("c", .cancel), isDefault: false), .glass)
        XCTAssertEqual(GlassModalAction.kind(action("s", .standard), isDefault: true), .primary)
        XCTAssertEqual(GlassModalAction.kind(action("s", .standard), isDefault: false), .glass)
    }

    /// A prominent action draws as the primary and takes no key.
    func test_aProminentActionIsPrimaryWithNoKey() {
        let send = GlassModalAction("send", isProminent: true) {}
        XCTAssertEqual(GlassModalAction.kind(send, isDefault: false), .primary)
        XCTAssertNil(GlassModalAction.defaultAction(in: [send]))
        XCTAssertNil(GlassModal<EmptyView>.shortcut(for: send, isDefault: false, isTopmost: true))
    }

    /// A body that fits lays out no taller than its content.
    func test_aShortBodyDoesNotFillTheWindow() {
        let modal = GlassModal(title: "t", onCancel: {}) { GlassModalBody { Text("b") } }
        let host = NSHostingView(rootView: modal.frame(maxHeight: 700))
        XCTAssertLessThan(host.fittingSize.height, 300)
    }

    /// Only the topmost modal answers Return and Escape.
    func test_onlyTheTopmostModalAnswersTheKeyboard() {
        let save = action("save", .standard, isDefault: true)
        let cancel = action("cancel", .cancel)
        let delete = action("delete", .destructive, isDefault: true)
        XCTAssertEqual(GlassModal<EmptyView>.shortcut(for: save, isDefault: true, isTopmost: true), .defaultAction)
        XCTAssertEqual(GlassModal<EmptyView>.shortcut(for: cancel, isDefault: false, isTopmost: true), .cancelAction)
        XCTAssertNil(GlassModal<EmptyView>.shortcut(for: delete, isDefault: true, isTopmost: true))
        XCTAssertNil(GlassModal<EmptyView>.shortcut(for: save, isDefault: true, isTopmost: false))
        XCTAssertNil(GlassModal<EmptyView>.shortcut(for: cancel, isDefault: false, isTopmost: false))
    }

    func test_theTopmostLayerIsTheLastRaised() {
        XCTAssertTrue(GlassModalStack.isTopmost(index: 1, count: 2, parentIsTopmost: true))
        XCTAssertFalse(GlassModalStack.isTopmost(index: 0, count: 2, parentIsTopmost: true))
        XCTAssertFalse(GlassModalStack.isTopmost(index: 1, count: 2, parentIsTopmost: false))
    }

    func test_widths() {
        XCTAssertEqual(GlassModalWidth.regular.points, GlassTokens.Size.modalWidth)
        XCTAssertEqual(GlassModalWidth.narrow.points, GlassTokens.Size.modalNarrowWidth)
    }

    /// A modal is no wider than its width, however wide the window.
    func test_aModalIsNoWiderThanItsWidth() {
        let modal = GlassModal(title: "t", width: .narrow, onCancel: {}) { Text("m") }
        let wide = NSHostingView(rootView: modal).fittingSize
        XCTAssertLessThanOrEqual(wide.width, GlassTokens.Size.modalNarrowWidth + 0.5)
    }

    /// A confirmation is #1146's regular modal (`responsive-overlay.tsx`),
    /// 780 wide, never the narrow one. Its short subtitle is the header's;
    /// its message (a disclosure) is the scrolling body at body size in
    /// secondary ink, as #1146 keeps it in the overlay's children (P31).
    func test_aConfirmationIsTheRegularModalWithItsMessageInTheBody() throws {
        XCTAssertEqual(GlassConfirmation.width, .regular)
        XCTAssertEqual(GlassConfirmation.width.points, 780)
        let long = String(repeating: "a long confirmation message ", count: 40)
        let modal = GlassConfirmation(title: "t", message: long, actions: [.cancel("c") {}], onCancel: {})
        let size = NSHostingView(rootView: modal.frame(maxWidth: 1400)).fittingSize
        XCTAssertGreaterThan(size.width, GlassTokens.Size.modalNarrowWidth + 0.5)
        XCTAssertLessThanOrEqual(size.width, GlassTokens.Size.modalWidth + 0.5)
        let sources = Dictionary(uniqueKeysWithValues: try DesignSources.components())
        let source = try XCTUnwrap(sources["Modal.swift"])
        XCTAssertTrue(source.contains("GlassModal(title: title, subtitle: subtitle, width: Self.width"))
        XCTAssertFalse(source.contains("subtitle: message"), "the disclosure is demoted to the caption subtitle")
        XCTAssertEqual(GlassConfirmation.paragraphs("a\n\n b \n\n\n\nc"), ["a", "b", "c"])
        XCTAssertEqual(GlassConfirmation.paragraphs(nil), [])
        // A tall disclosure grows the modal, up to the body's scroll.
        let short = NSHostingView(rootView: GlassConfirmation(title: "t", actions: [.cancel("c") {}], onCancel: {})
            .frame(maxWidth: 1400)).fittingSize
        XCTAssertGreaterThan(size.height, short.height + 10, "the message is not drawn in the body")
    }

    /// #1146 draws a close button on every modal: a modal that names none
    /// of its own takes its host's name, and with neither none is drawn.
    func test_theCloseButtonTakesTheHostsName() {
        XCTAssertEqual(GlassModal<EmptyView>.closeLabel(own: "", host: "Close"), "Close")
        XCTAssertEqual(GlassModal<EmptyView>.closeLabel(own: "Done", host: "Close"), "Done")
        XCTAssertEqual(GlassModal<EmptyView>.closeLabel(own: "", host: ""), "")
    }

    /// A click on the scrim cancels the modal over it (#1146 `surfaces.tsx`
    /// `onClick={onClose}`; the mechanism is pinned in ComponentParityTests).
    /// The header centres its title block and close button, and the fade is
    /// 220ms.
    func test_aClickOnTheScrimCancels() throws {
        let sources = Dictionary(uniqueKeysWithValues: try DesignSources.components())
        let modal = try XCTUnwrap(sources["Modal.swift"])
        XCTAssertFalse(modal.contains(".onTapGesture {}"), "the scrim swallows the click")
        XCTAssertTrue(modal.contains(".onTapGesture { onTap?() }"))
        XCTAssertTrue(modal.contains("HStack(alignment: .center, spacing: GlassTokens.Space.s6)"))
        XCTAssertTrue(modal.contains("GlassMotion.standard(reduceMotion)"), "the fade is not #1146's 220ms")
    }

    /// Hosted and unhosted presentations lay out, raised and not.
    func test_presentationLaysOut() {
        for presented in [true, false] {
            let raised = Color.clear.frame(width: 900, height: 600)
                .glassModal(isPresented: .constant(presented)) {
                    GlassModal(title: "t", onCancel: {}) { Text("b") }
                }
            for view in [AnyView(raised.glassModalHost()), AnyView(raised)] {
                let host = NSHostingView(rootView: view)
                host.frame = CGRect(x: 0, y: 0, width: 900, height: 600)
                host.layoutSubtreeIfNeeded()
                XCTAssertEqual(host.fittingSize.width, 900, accuracy: 0.5)
            }
        }
    }

    /// Escape is wired once, on the modal, and only while it is topmost;
    /// Surfaces keeps its two.
    func test_escapeIsGatedOnTheTopmostModal() throws {
        let sources = Dictionary(uniqueKeysWithValues: try DesignSources.components())
        let modal = try XCTUnwrap(sources["Modal.swift"])
        XCTAssertEqual(modal.components(separatedBy: ".onExitCommand").count - 1, 1)
        // And not while the modal is busy (#1273 review).
        XCTAssertTrue(modal.contains(".onExitCommand(perform: isTopmost && cancellable ? onCancel : nil)"))
    }
}
