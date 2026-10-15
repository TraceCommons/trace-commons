import Foundation

/// Where a window's frame is set. A frame set inside a SwiftUI update, or
/// inside AppKit's resize of a window SwiftUI also sizes, re-enters the
/// layout pass that is already running; the two sizings fight until AppKit
/// throws `_postWindowNeedsUpdateConstraints` and the app aborts. That is how
/// the edge rail crashed in 0.16.0 (#1402).
public enum GlassWindowFrame {
    /// Runs `body` on the next turn of the main run loop, after the update
    /// that asked for it has finished. Calls run in the order they were made.
    @MainActor
    public static func onNextTurn(_ body: @escaping @MainActor () -> Void) {
        DispatchQueue.main.async { MainActor.assumeIsolated { body() } }
    }
}
