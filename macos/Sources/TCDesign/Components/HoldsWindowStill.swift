import AppKit
import SwiftUI

extension View {
    /// Holds the window still while the pointer is over this view, so a
    /// press here reaches the view rather than moving the window, which
    /// otherwise moves by a drag anywhere in its background (owner,
    /// 2026-11-02: dragging a Traces folder moved the window, and a click
    /// on a running Private AI session did not show its terminal).
    ///
    /// `enabled` turns the hold on and off in place, so the view it is
    /// attached to keeps its identity and state when it does.
    public func holdsWindowStill(enabled: Bool = true) -> some View {
        background(GlassHoldWindow(enabled: enabled))
    }
}

private struct GlassHoldWindow: NSViewRepresentable {
    let enabled: Bool
    func makeNSView(context: Context) -> GlassHoldWindowView {
        let view = GlassHoldWindowView()
        view.isEnabled = enabled
        return view
    }
    func updateNSView(_ view: GlassHoldWindowView, context: Context) { view.isEnabled = enabled }
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

    /// How many of these views hold each window still, by window, held
    /// weakly: a window that goes takes its count with it.
    @MainActor private static let holds = NSMapTable<NSWindow, NSNumber>.weakToStrongObjects()

    /// Off, the view holds nothing and lets go of what it held; turned on,
    /// it holds when the pointer is already over it.
    var isEnabled = true {
        didSet {
            guard isEnabled != oldValue else { return }
            if isEnabled { settleByPointer() } else { release() }
        }
    }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(
            rect: .zero, options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect], owner: self)
        addTrackingArea(area)
        tracking = area
        settleByPointer()
    }

    private func settleByPointer() {
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
        guard isEnabled, held == nil, let window else { return }
        let count = Self.holds.object(forKey: window)?.intValue ?? 0
        if count == 0 {
            // Held still by something else: not ours to let go of.
            guard window.isMovableByWindowBackground else { return }
        }
        // Set on every hold, not only the first: anything that made the
        // window movable again while held is overruled while a view holds.
        window.isMovableByWindowBackground = false
        Self.holds.setObject(NSNumber(value: count + 1), forKey: window)
        held = window
    }

    private func release() {
        guard let window = held else { return }
        held = nil
        let left = (Self.holds.object(forKey: window)?.intValue ?? 1) - 1
        if left > 0 {
            Self.holds.setObject(NSNumber(value: left), forKey: window)
        } else {
            Self.holds.removeObject(forKey: window)
            window.isMovableByWindowBackground = true
        }
    }
}
