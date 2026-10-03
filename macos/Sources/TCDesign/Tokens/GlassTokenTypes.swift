import AppKit
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

/// A macOS text style. Every step of the type scale is one, so the scale
/// grows and shrinks with the system text size like the rest of the shell.
public enum GlassTextStyle: Sendable, Equatable {
    case largeTitle, title, title2, title3, headline, body, callout, subheadline, footnote, caption, caption2

    public var font: Font.TextStyle {
        switch self {
        case .largeTitle: .largeTitle
        case .title: .title
        case .title2: .title2
        case .title3: .title3
        case .headline: .headline
        case .body: .body
        case .callout: .callout
        case .subheadline: .subheadline
        case .footnote: .footnote
        case .caption: .caption
        case .caption2: .caption2
        }
    }

    public var appKit: NSFont.TextStyle {
        switch self {
        case .largeTitle: .largeTitle
        case .title: .title1
        case .title2: .title2
        case .title3: .title3
        case .headline: .headline
        case .body: .body
        case .callout: .callout
        case .subheadline: .subheadline
        case .footnote: .footnote
        case .caption: .caption1
        case .caption2: .caption2
        }
    }

    /// The points this style is drawn at under the current system text size.
    public var resolvedSize: CGFloat {
        NSFont.preferredFont(forTextStyle: appKit).pointSize
    }
}

/// A step of the type scale, in SF Pro (or SF Mono for `.monospaced`).
///
/// `size`, `lineHeight` and `tracking` are stated at the default system text
/// size, where `size` is the text style's own size (the generator refuses
/// anything else). At any other text size the font follows the style, and
/// leading and tracking are scaled by the same ratio, so the three never
/// come apart.
public struct GlassTypeStyle: Sendable, Equatable {
    public let textStyle: GlassTextStyle
    public let size: CGFloat
    public let weight: GlassWeight
    public let lineHeight: CGFloat
    /// Letter spacing in points, at `size`.
    public let tracking: CGFloat
    public let design: GlassFontDesign
    public let uppercase: Bool
    public let tabular: Bool

    public init(
        textStyle: GlassTextStyle,
        size: CGFloat,
        weight: GlassWeight,
        lineHeight: CGFloat,
        tracking: CGFloat,
        design: GlassFontDesign,
        uppercase: Bool,
        tabular: Bool
    ) {
        self.textStyle = textStyle
        self.size = size
        self.weight = weight
        self.lineHeight = lineHeight
        self.tracking = tracking
        self.design = design
        self.uppercase = uppercase
        self.tabular = tabular
    }

    /// The same step at another weight (a selected tab, a bold count).
    public func weight(_ weight: GlassWeight) -> GlassTypeStyle {
        GlassTypeStyle(
            textStyle: textStyle, size: size, weight: weight, lineHeight: lineHeight,
            tracking: tracking, design: design, uppercase: uppercase, tabular: tabular)
    }

    /// The same step in SF Mono, for a figure or path inside it.
    public var monospaced: GlassTypeStyle {
        GlassTypeStyle(
            textStyle: textStyle, size: size, weight: weight, lineHeight: lineHeight,
            tracking: tracking, design: .monospaced, uppercase: uppercase, tabular: tabular)
    }

    public var font: Font {
        let base = Font.system(textStyle.font, design: design.font).weight(weight.font)
        return tabular ? base.monospacedDigit() : base
    }

    /// How far the system text size has moved this step from `size`.
    public var scale: CGFloat {
        size > 0 ? textStyle.resolvedSize / size : 1
    }

    /// Tracking at the size the type is drawn at now.
    public var resolvedTracking: CGFloat {
        tracking * scale
    }

    /// Extra spacing between lines to reach the stated line height, at the
    /// size the type is drawn at now. Approximates the default line box as
    /// 1.2x, as `TC.Font_.LineHeight` does.
    public var lineSpacing: CGFloat {
        max(0, (lineHeight - size * 1.2) * scale)
    }
}
