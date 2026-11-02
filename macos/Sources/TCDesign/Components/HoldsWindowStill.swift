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
public final class GlassHoldWindowView: NSView {
    private var tracking: NSTrackingArea?
    /// The window this view held still, to let go of when the pointer
    /// leaves or the view does.
    private weak var held: NSWindow?

    override public func hitTest(_ point: NSPoint) -> NSView? { nil }

    override public func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(
            rect: .zero, options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect], owner: self)
        addTrackingArea(area)
        tracking = area
    }

    override public func mouseEntered(with event: NSEvent) {
        guard let window, window.isMovableByWindowBackground else { return }
        window.isMovableByWindowBackground = false
        held = window
    }

    override public func mouseExited(with event: NSEvent) { release() }

    override public func viewWillMove(toWindow newWindow: NSWindow?) {
        if newWindow !== held { release() }
        super.viewWillMove(toWindow: newWindow)
    }

    private func release() {
        held?.isMovableByWindowBackground = true
        held = nil
    }
}
