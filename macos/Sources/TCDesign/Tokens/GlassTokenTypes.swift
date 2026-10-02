import CoreGraphics
import SwiftUI

/// An sRGB colour with alpha, as the token source states it.
public struct GlassRGBA: Sendable, Equatable {
    public let rgb: UInt32
    public let alpha: Double

    public init(_ rgb: UInt32, alpha: Double = 1) {
        self.rgb = rgb
        self.alpha = alpha
    }

    public var red: Double { Double((rgb >> 16) & 0xFF) / 255 }
    public var green: Double { Double((rgb >> 8) & 0xFF) / 255 }
    public var blue: Double { Double(rgb & 0xFF) / 255 }

    public var color: Color {
        Color(.sRGB, red: red, green: green, blue: blue, opacity: alpha)
    }

    /// The same colour at a different alpha (tints, rings at 60%).
    public func opacity(_ alpha: Double) -> GlassRGBA {
        GlassRGBA(rgb, alpha: alpha)
    }
}

/// One stop of a linear gradient.
public struct GlassStop: Sendable, Equatable {
    public let color: GlassRGBA
    public let location: CGFloat

    public init(_ rgb: UInt32, alpha: Double, at location: CGFloat) {
        self.color = GlassRGBA(rgb, alpha: alpha)
        self.location = location
    }
}

/// A linear gradient at a CSS angle: 180 runs top to bottom, 160 leans the
/// light toward the top left, the way the panes are lit.
public struct GlassGradient: Sendable, Equatable {
    public let angle: Double
    public let stops: [GlassStop]

    public init(angle: Double, stops: [GlassStop]) {
        self.angle = angle
        self.stops = stops
    }

    /// Start and end points for a CSS angle over a unit square.
    public var unitPoints: (start: UnitPoint, end: UnitPoint) {
        let radians = angle * .pi / 180
        let dx = sin(radians) / 2
        let dy = -cos(radians) / 2
        return (
            UnitPoint(x: 0.5 - dx, y: 0.5 - dy),
            UnitPoint(x: 0.5 + dx, y: 0.5 + dy)
        )
    }

    public var linear: LinearGradient {
        let points = unitPoints
        return LinearGradient(
            stops: stops.map { .init(color: $0.color.color, location: $0.location) },
            startPoint: points.start,
            endPoint: points.end
        )
    }
}

/// One layer of an edge: an inset layer is a specular highlight or shade
/// inside the shape, an outer layer is a drop shadow.
public struct GlassShadow: Sendable, Equatable {
    public let x: CGFloat
    public let y: CGFloat
    public let blur: CGFloat
    public let color: GlassRGBA
    public let inset: Bool

    public init(x: CGFloat, y: CGFloat, blur: CGFloat, color: GlassRGBA, inset: Bool) {
        self.x = x
        self.y = y
        self.blur = blur
        self.color = color
        self.inset = inset
    }
}

public enum GlassWeight: Sendable, Equatable {
    case regular, medium, semibold, bold, heavy

    public var font: Font.Weight {
        switch self {
        case .regular: .regular
        case .medium: .medium
        case .semibold: .semibold
        case .bold: .bold
        case .heavy: .heavy
        }
    }
}

public enum GlassFontDesign: Sendable, Equatable {
    case `default`, monospaced

    public var font: Font.Design {
        switch self {
        case .default: .default
        case .monospaced: .monospaced
        }
    }
}

/// A step of the type scale, in SF Pro (or SF Mono for `.monospaced`).
public struct GlassTypeStyle: Sendable, Equatable {
    public let size: CGFloat
    public let weight: GlassWeight
    public let lineHeight: CGFloat
    /// Letter spacing in points.
    public let tracking: CGFloat
    public let design: GlassFontDesign
    public let uppercase: Bool
    public let tabular: Bool

    public init(
        size: CGFloat,
        weight: GlassWeight,
        lineHeight: CGFloat,
        tracking: CGFloat,
        design: GlassFontDesign,
        uppercase: Bool,
        tabular: Bool
    ) {
        self.size = size
        self.weight = weight
        self.lineHeight = lineHeight
        self.tracking = tracking
        self.design = design
        self.uppercase = uppercase
        self.tabular = tabular
    }

    public var font: Font {
        let base = Font.system(size: size, weight: weight.font, design: design.font)
        return tabular ? base.monospacedDigit() : base
    }

    /// Extra spacing between lines to reach the stated line height.
    public var lineSpacing: CGFloat {
        max(0, lineHeight - size * 1.2)
    }
}
