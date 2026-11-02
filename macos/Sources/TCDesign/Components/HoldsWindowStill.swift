import AppKit
import SwiftUI

extension View {
    /// Holds the window still while the pointer is over this view, so a
    /// press here reaches the view rather than moving the window, which
    /// otherwise moves by a drag anywhere in its background (owner,
    /// 2026-11-02: dragging a Traces folder moved the window, and a click
    /// on a running Private AI session did not show its terminal).
    public func holdsWindowStill() -> some View {
        background(GlassHoldWindow())
    }
}

private struct GlassHoldWindow: NSViewRepresentable {
    func makeNSView(context: Context) -> GlassHoldWindowView { GlassHoldWindowView() }
    func updateNSView(_ view: GlassHoldWindowView, context: Context) {}
}

/// Watched by a tracking area, so it takes no clicks of its own.
///
/// Several of these can sit in one window side by side (one per running
/// session row), so the window is held while any of them has the pointer:
/// holds are counted per window, and the window moves by its background
/// again only when the last one lets go. AppKit does not promise the order
/// of one view's exit and the next one's entry.
///
/// A view whose tracking area is made again, because it appeared under a
/// resting pointer or scrolled out from under one, gets no enter or exit
/// for that, so it asks where the pointer is then.
final class GlassHoldWindowView: NSView {
    private var tracking: NSTrackingArea?
    /// The window this view holds still, to let go of when the pointer
    /// leaves or the view does.
    private weak var held: NSWindow?

    /// How many of these views hold each window still.
    @MainActor private static var holds: [ObjectIdentifier: Int] = [:]

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(
            rect: .zero, options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect], owner: self)
        addTrackingArea(area)
        tracking = area
        guard let window else { return }
        let pointer = convert(window.mouseLocationOutsideOfEventStream, from: nil)
        settle(pointerInside: visibleRect.contains(pointer))
    }

    override func mouseEntered(with event: NSEvent) { hold() }

    override func mouseExited(with event: NSEvent) { release() }

    override func viewWillMove(toWindow newWindow: NSWindow?) {
        if newWindow !== held { release() }
        super.viewWillMove(toWindow: newWindow)
    }

    /// Hold or let go to match where the pointer is, when no enter or exit
    /// says so.
    func settle(pointerInside: Bool) {
        if pointerInside { hold() } else { release() }
    }

    private func hold() {
        guard held == nil, let window else { return }
        let key = ObjectIdentifier(window)
        // A count while the window moves again is a gone window's whose
        // identifier this one now has: this window starts its own.
        if window.isMovableByWindowBackground { Self.holds[key] = nil }
        if Self.holds[key] == nil {
            // Held still by something else: not ours to let go of.
            guard window.isMovableByWindowBackground else { return }
            window.isMovableByWindowBackground = false
        }
        Self.holds[key, default: 0] += 1
        held = window
    }

    private func release() {
        guard let window = held else { return }
        held = nil
        let key = ObjectIdentifier(window)
        let left = (Self.holds[key] ?? 1) - 1
        if left > 0 {
            Self.holds[key] = left
        } else {
            Self.holds[key] = nil
            window.isMovableByWindowBackground = true
        }
    }
}
