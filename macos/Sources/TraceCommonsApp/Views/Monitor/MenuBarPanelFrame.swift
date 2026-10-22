import AppKit
import ObjectiveC

/// Keeps the `MenuBarExtra` window at the sizes `MenuBarTopEdgePin` gives
/// it: the one way that window's size changes, as the edge rail's panel
/// takes its frames from its controller alone (#1517).
///
/// Taking the sizing away from the panel's hosting view
/// (`MenuBarTopEdgePin.takeOverSizing`) is not enough in the app as built
/// against the macOS 26 SDK. Whenever the panel's content changed height
/// while it was open, `NSHostingView.updateAnimatedWindowSize` still resized
/// the window from inside `windowDidLayout`, animating it toward the
/// content's new height one layout pass at a time, and the pin set it back
/// on the next turn: two writers of one window's size, the shape AppKit
/// aborts on, and the "window resizing repeatedly: menu-bar panel" line in
/// the log.
///
/// The window is SwiftUI's (`MenuBarExtraWindow`), not a class this app
/// can subclass, so its class's three frame setters go through `keeps`.
/// For a window held here, a write that changes its size is dropped unless
/// `place(_:in:)` makes it; one that only moves it still lands, so SwiftUI
/// still puts the window under its menu-bar item. A window that is not
/// held, of that class or any other, takes every frame as before.
@MainActor
enum MenuBarPanelFrame {
    /// The windows whose size only `place(_:in:)` changes.
    private static let held = NSHashTable<NSWindow>.weakObjects()
    /// The window classes whose frame setters ask `keeps` first.
    private static var guarded: Set<ObjectIdentifier> = []
    /// Set only inside `place(_:in:)`.
    private static var placing = false

    /// From now on, `window`'s size changes only through `place(_:in:)`.
    static func hold(_ window: NSWindow) {
        guard !held.contains(window) else { return }
        guardFrameSetters(of: ownClass(of: window))
        held.add(window)
    }

    /// Moves and sizes a held window.
    static func place(_ frame: CGRect, in window: NSWindow) {
        placing = true
        defer { placing = false }
        window.setFrame(frame, display: true)
    }

    /// Whether a write that would give a window of size `current` the size
    /// `proposed` is kept: always for a window that is not held, or while
    /// `place(_:in:)` is setting it, and otherwise only when it leaves the
    /// size as it is. Pure, so tested.
    nonisolated static func keeps(proposed: CGSize, current: CGSize, held: Bool, placing: Bool) -> Bool {
        placing || !held || proposed == current
    }

    /// The class `window` was made as, which its frame setters belong to:
    /// a key-value observer's subclass, which stands in front of it while
    /// the window is observed, is passed over.
    static func ownClass(of window: NSWindow) -> AnyClass {
        var cls: AnyClass = object_getClass(window) ?? NSWindow.self
        while NSStringFromClass(cls).hasPrefix("NSKVONotifying_"), let parent = class_getSuperclass(cls) {
            cls = parent
        }
        return cls
    }

    /// Whether `window` takes a write that would give it a frame of the
    /// size `proposed` works out.
    private nonisolated static func accepts(_ window: NSWindow, _ proposed: @MainActor (NSWindow) -> CGSize) -> Bool {
        // AppKit sets frames on the main thread; anywhere else, leave it be.
        guard Thread.isMainThread else { return true }
        return MainActor.assumeIsolated {
            keeps(proposed: proposed(window), current: window.frame.size, held: held.contains(window), placing: placing)
        }
    }

    private typealias SetFrame = @convention(c) (NSWindow, Selector, NSRect, ObjCBool) -> Void
    private typealias SetFrameAnimated = @convention(c) (NSWindow, Selector, NSRect, ObjCBool, ObjCBool) -> Void
    private typealias SetContentSize = @convention(c) (NSWindow, Selector, NSSize) -> Void

    /// Sends `cls`'s `setFrame(_:display:)`, `setFrame(_:display:animate:)`
    /// and `setContentSize(_:)` through `accepts`, once per class: the
    /// three setters the edge rail's panel guards (#1517).
    private static func guardFrameSetters(of cls: AnyClass) {
        guard guarded.insert(ObjectIdentifier(cls)).inserted else { return }

        let setFrame = #selector(NSWindow.setFrame(_:display:))
        replace(setFrame, in: cls) { imp in
            let original = unsafeBitCast(imp, to: SetFrame.self)
            let gated: @convention(block) (NSWindow, NSRect, ObjCBool) -> Void = { window, frame, display in
                if accepts(window, { _ in frame.size }) { original(window, setFrame, frame, display) }
            }
            return imp_implementationWithBlock(unsafeBitCast(gated, to: AnyObject.self))
        }

        let setFrameAnimated = #selector(NSWindow.setFrame(_:display:animate:))
        replace(setFrameAnimated, in: cls) { imp in
            let original = unsafeBitCast(imp, to: SetFrameAnimated.self)
            let gated: @convention(block) (NSWindow, NSRect, ObjCBool, ObjCBool) -> Void = { window, frame, display, animate in
                if accepts(window, { _ in frame.size }) { original(window, setFrameAnimated, frame, display, animate) }
            }
            return imp_implementationWithBlock(unsafeBitCast(gated, to: AnyObject.self))
        }

        let setContentSize = #selector(NSWindow.setContentSize(_:))
        replace(setContentSize, in: cls) { imp in
            let original = unsafeBitCast(imp, to: SetContentSize.self)
            let gated: @convention(block) (NSWindow, NSSize) -> Void = { window, size in
                let content = NSRect(origin: .zero, size: size)
                if accepts(window, { $0.frameRect(forContentRect: content).size }) {
                    original(window, setContentSize, size)
                }
            }
            return imp_implementationWithBlock(unsafeBitCast(gated, to: AnyObject.self))
        }
    }

    /// Routes `selector` on `cls` through the implementation `wrap` makes
    /// of the one it has: its own, replaced, or the one it inherits, which
    /// the class then overrides.
    private static func replace(_ selector: Selector, in cls: AnyClass, _ wrap: (IMP) -> IMP) {
        guard let method = class_getInstanceMethod(cls, selector) else { return }
        let wrapped = wrap(method_getImplementation(method))
        if !class_addMethod(cls, selector, wrapped, method_getTypeEncoding(method)) {
            method_setImplementation(method, wrapped)
        }
    }
}
