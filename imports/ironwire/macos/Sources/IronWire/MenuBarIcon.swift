//! Drawing the four icon states.
//
// The icon has to carry state without being read, which rules out anything that
// needs a glance longer than the one a person gives a menu bar. So: one glyph,
// and a dot that is absent, filled, or coloured.
//
// Healthy and degraded are template images, so AppKit tints them with whatever
// the menu bar is doing and they blend in the way every other system icon does.
// Attention cannot be — a coloured dot is the point — so it is drawn against the
// resolved appearance and regenerated on each status poll, which picks up a
// light/dark switch within one tick.

import AppKit
import IronWireKit

enum MenuBarIcon {
    /// The glyph. A branch, because what this app reports on is which way
    /// traffic went.
    private static let symbolName = "arrow.triangle.branch"

    @MainActor
    static func image(for state: IconState) -> NSImage {
        let size = NSSize(width: 18, height: 18)
        let dot: NSColor? = {
            switch state {
            case .attention: return .systemRed
            case .healthy, .degraded, .unreachable: return nil
            }
        }()
        // Resolved out here, on the main actor, because the drawing handler runs
        // without one and `effectiveAppearance` is main-actor state. Carried as
        // components rather than an `NSColor` so nothing non-`Sendable` crosses
        // into the closure.
        let tint = labelColour()

        let image = NSImage(size: size, flipped: false) { _ in
            guard let glyph = symbol() else { return false }

            // A daemon that is not running is a normal state, so it is said
            // quietly: the same icon, faded, rather than a warning.
            let alpha: CGFloat = state == .unreachable ? 0.4 : 1
            let glyphRect = NSRect(
                x: (size.width - glyph.size.width) / 2,
                y: (size.height - glyph.size.height) / 2,
                width: glyph.size.width,
                height: glyph.size.height
            )

            if dot == nil {
                // Template: draw the shape and let AppKit colour it.
                glyph.draw(in: glyphRect, from: .zero, operation: .sourceOver, fraction: alpha)
            } else {
                glyph.tinted(tint.colour)
                    .draw(in: glyphRect, from: .zero, operation: .sourceOver, fraction: alpha)
            }

            // Degraded gets the same dot in the glyph's own colour: a change in
            // shape, not in urgency. Rungs 1 and 2 are not worth alarming
            // anyone over (`docs/DESIGN.md` §3) but are worth being able to see.
            if state == .degraded || dot != nil {
                let diameter: CGFloat = 6
                let rect = NSRect(
                    x: size.width - diameter,
                    y: 0,
                    width: diameter,
                    height: diameter
                )
                (dot ?? .labelColor).setFill()
                NSBezierPath(ovalIn: rect).fill()
            }
            return true
        }

        // Only the states with no colour of their own may be tinted by AppKit.
        image.isTemplate = dot == nil
        image.accessibilityDescription = "IronWire — \(state.summary)"
        return image
    }

    private static func symbol() -> NSImage? {
        NSImage(systemSymbolName: symbolName, accessibilityDescription: nil)?
            .withSymbolConfiguration(.init(pointSize: 14, weight: .regular))
    }

    /// An appearance-resolved colour, reduced to numbers so it can be handed to
    /// a drawing handler that runs off the main actor.
    private struct Components: Sendable {
        let red: CGFloat, green: CGFloat, blue: CGFloat, alpha: CGFloat
        var colour: NSColor { NSColor(srgbRed: red, green: green, blue: blue, alpha: alpha) }
    }

    /// `labelColor` resolved against the appearance the menu bar is actually
    /// using, rather than whatever the drawing context happens to inherit.
    ///
    /// Resolved fresh on every status poll, so a light/dark switch is picked up
    /// within one tick without this app having to watch for one.
    @MainActor
    private static func labelColour() -> Components {
        var resolved = NSColor.labelColor
        NSApp?.effectiveAppearance.performAsCurrentDrawingAppearance {
            resolved = NSColor.labelColor
        }
        let srgb = resolved.usingColorSpace(.sRGB) ?? .black
        return Components(
            red: srgb.redComponent,
            green: srgb.greenComponent,
            blue: srgb.blueComponent,
            alpha: srgb.alphaComponent
        )
    }
}

private extension NSImage {
    /// A copy of this image painted in one colour, for the cases where a
    /// template image would lose the distinction we are drawing.
    func tinted(_ colour: NSColor) -> NSImage {
        guard let copy = self.copy() as? NSImage else { return self }
        copy.lockFocus()
        colour.set()
        NSRect(origin: .zero, size: copy.size).fill(using: .sourceAtop)
        copy.unlockFocus()
        copy.isTemplate = false
        return copy
    }
}
