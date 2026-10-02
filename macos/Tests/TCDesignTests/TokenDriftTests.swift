import AppKit
import Foundation
import XCTest

@testable import TCDesign

/// The JSON source and the generated Swift must agree, value for value. A
/// failure here means one was edited without the other: edit
/// design-tokens/glass.tokens.json and run scripts/design-tokens/generate.py.
final class TokenDriftTests: XCTestCase {
    // Read-only after first use; JSON objects are not Sendable.
    nonisolated(unsafe) private static let source: [String: Any] = {
        let repo = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent() // TCDesignTests
            .deletingLastPathComponent() // Tests
            .deletingLastPathComponent() // macos
            .deletingLastPathComponent() // repo
        let url = repo.appending(path: "design-tokens/glass.tokens.json")
        guard
            let data = try? Data(contentsOf: url),
            let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            fatalError("cannot read \(url.path)")
        }
        return object
    }()

    private static func group(_ name: String) -> [String: Any] {
        source[name] as? [String: Any] ?? [:]
    }

    private static func rgb(_ hex: Any?) -> UInt32? {
        guard let hex = hex as? String, hex.hasPrefix("#") else { return nil }
        return UInt32(hex.dropFirst(), radix: 16)
    }

    private static func number(_ value: Any?, default fallback: Double = 0) -> Double {
        (value as? NSNumber)?.doubleValue ?? fallback
    }

    func test_colorsMatchTheSource() {
        let colors = Self.group("color")
        XCTAssertTrue(Set(colors.keys) == Set(GlassTokens.Color.all.keys))
        for (name, raw) in colors {
            let entry = raw as? [String: Any] ?? [:]
            let generated = GlassTokens.Color.all[name]
            XCTAssertTrue(generated?.rgb == Self.rgb(entry["hex"]), "color.\(name)")
            XCTAssertTrue(generated?.alpha == Self.number(entry["alpha"], default: 1), "color.\(name) alpha")
            Self.checkLight(generated, entry, "color.\(name)")
        }
    }

    /// A token's light appearance value matches the JSON's `light`, or is
    /// absent when the JSON has none.
    private static func checkLight(_ generated: GlassRGBA?, _ entry: [String: Any], _ name: String) {
        guard let light = entry["light"] as? [String: Any] else {
            XCTAssertTrue(generated?.lightRGB == nil && generated?.lightAlpha == nil, "\(name) has no light value")
            return
        }
        XCTAssertTrue(generated?.lightRGB == rgb(light["hex"]), "\(name).light")
        XCTAssertTrue(generated?.lightAlpha == number(light["alpha"], default: 1), "\(name).light alpha")
    }

    func test_gradientsMatchTheSource() {
        let gradients = Self.group("gradient")
        XCTAssertTrue(Set(gradients.keys) == Set(GlassTokens.Gradient.all.keys))
        for (name, raw) in gradients {
            let entry = raw as? [String: Any] ?? [:]
            let stops = entry["stops"] as? [[String: Any]] ?? []
            let generated = GlassTokens.Gradient.all[name]
            XCTAssertTrue(generated?.angle == Self.number(entry["angle"]), "gradient.\(name)")
            XCTAssertTrue(generated?.stops.count == stops.count, "gradient.\(name) stops")
            for (index, stop) in stops.enumerated() where index < (generated?.stops.count ?? 0) {
                let made = generated!.stops[index]
                XCTAssertTrue(made.color.rgb == Self.rgb(stop["hex"]), "gradient.\(name)[\(index)]")
                XCTAssertTrue(made.color.alpha == Self.number(stop["alpha"], default: 1), "gradient.\(name)[\(index)] alpha")
                Self.checkLight(made.color, stop, "gradient.\(name)[\(index)]")
                XCTAssertTrue(Double(made.location) == Self.number(stop["at"]), "gradient.\(name)[\(index)] at")
            }
        }
    }

    func test_shadowsMatchTheSource() {
        let shadows = Self.group("shadow")
        XCTAssertTrue(Set(shadows.keys) == Set(GlassTokens.Shadow.all.keys))
        for (name, raw) in shadows {
            let layers = raw as? [[String: Any]] ?? []
            let generated = GlassTokens.Shadow.all[name] ?? []
            XCTAssertTrue(generated.count == layers.count, "shadow.\(name)")
            for (index, layer) in layers.enumerated() where index < generated.count {
                let made = generated[index]
                XCTAssertTrue(Double(made.x) == Self.number(layer["x"]), "shadow.\(name)[\(index)] x")
                XCTAssertTrue(Double(made.y) == Self.number(layer["y"]), "shadow.\(name)[\(index)] y")
                XCTAssertTrue(Double(made.blur) == Self.number(layer["blur"]), "shadow.\(name)[\(index)] blur")
                XCTAssertTrue(made.color.rgb == Self.rgb(layer["hex"]), "shadow.\(name)[\(index)] colour")
                XCTAssertTrue(made.color.alpha == Self.number(layer["alpha"], default: 1), "shadow.\(name)[\(index)] alpha")
                Self.checkLight(made.color, layer, "shadow.\(name)[\(index)]")
                XCTAssertTrue(made.inset == ((layer["inset"] as? Bool) ?? false), "shadow.\(name)[\(index)] inset")
            }
        }
    }

    func test_scalarsMatchTheSource() {
        let tables: [(String, [String: Double])] = [
            ("radius", GlassTokens.Radius.all.mapValues(Double.init)),
            ("space", GlassTokens.Space.all.mapValues(Double.init)),
            ("size", GlassTokens.Size.all.mapValues(Double.init)),
            ("opacity", GlassTokens.Opacity.all),
            ("motion", GlassTokens.Motion.all),
        ]
        for (name, generated) in tables {
            let source = Self.group(name)
            XCTAssertTrue(Set(source.keys) == Set(generated.keys), "\(name) keys")
            for (key, value) in source {
                XCTAssertTrue(generated[key] == Self.number(value), "\(name).\(key)")
            }
        }
    }

    func test_typeScaleMatchesTheSource() {
        let types = Self.group("type")
        XCTAssertTrue(Set(types.keys) == Set(GlassTokens.TypeScale.all.keys))
        for (name, raw) in types {
            let entry = raw as? [String: Any] ?? [:]
            guard let made = GlassTokens.TypeScale.all[name] else { continue }
            XCTAssertTrue(Double(made.size) == Self.number(entry["size"]), "type.\(name) size")
            XCTAssertTrue(Double(made.lineHeight) == Self.number(entry["lineHeight"]), "type.\(name) lineHeight")
            XCTAssertTrue(Double(made.tracking) == Self.number(entry["tracking"]), "type.\(name) tracking")
            XCTAssertTrue(made.uppercase == ((entry["uppercase"] as? Bool) ?? false), "type.\(name) uppercase")
            XCTAssertTrue(made.tabular == ((entry["tabular"] as? Bool) ?? false), "type.\(name) tabular")
            XCTAssertTrue((made.design == .monospaced) == ((entry["design"] as? String) == "monospaced"), "type.\(name) design")
            XCTAssertTrue(String(describing: made.textStyle) == (entry["textStyle"] as? String), "type.\(name) textStyle")
        }
    }

    /// The brand is purple (D3); the community green is gone from the
    /// glass palette.
    func test_theBrandIsPurpleAndTheCommunityGreenIsGone() {
        XCTAssertTrue(GlassTokens.Color.purple.rgb == 0x6D14F3)
        let communityGreens: Set<UInt32> = [0x178F70, 0x3FBE9A, 0x137C61, 0x0F7256, 0x5CD3AF]
        XCTAssertTrue(GlassTokens.Color.all.values.allSatisfy { !communityGreens.contains($0.rgb) })
    }
}

final class GradientGeometryTests: XCTestCase {
    func test_cssAngleOneEightyRunsTopToBottom() {
        let points = GlassGradient(angle: 180, stops: []).unitPoints
        XCTAssertTrue(abs(points.start.x - 0.5) < 1e-9)
        XCTAssertTrue(abs(points.start.y - 0) < 1e-9)
        XCTAssertTrue(abs(points.end.y - 1) < 1e-9)
    }

    func test_cssAngleNinetyRunsLeftToRight() {
        let points = GlassGradient(angle: 90, stops: []).unitPoints
        XCTAssertTrue(abs(points.start.x - 0) < 1e-9)
        XCTAssertTrue(abs(points.end.x - 1) < 1e-9)
        XCTAssertTrue(abs(points.start.y - 0.5) < 1e-9)
    }
}

/// R2: the type scale is SF Pro and SF Mono set as macOS text styles, so it
/// follows the system text size instead of sitting at fixed points.
final class TypeScaleTests: XCTestCase {
    private static let styles: [GlassTextStyle] = [
        .largeTitle, .title, .title2, .title3, .headline, .body, .callout, .subheadline, .footnote, .caption, .caption2,
    ]

    /// Each step resolves its size from its own text style.
    func test_eachStepResolvesItsSizeFromItsTextStyle() {
        for (name, step) in GlassTokens.TypeScale.all {
            XCTAssertEqual(
                step.textStyle.resolvedSize,
                NSFont.preferredFont(forTextStyle: step.textStyle.appKit).pointSize,
                accuracy: 0.001, "type.\(name)")
            XCTAssertGreaterThan(step.textStyle.resolvedSize, 0, "type.\(name) resolved to nothing")
        }
    }

    /// A mapping that answered `.body` for everything would pass the test
    /// above and flatten the scale.
    func test_theTextStyleMappingIsNotDegenerate() {
        let appKit = Self.styles.map { $0.appKit.rawValue }
        XCTAssertEqual(Set(appKit).count, Self.styles.count)
        XCTAssertGreaterThan(GlassTextStyle.largeTitle.resolvedSize, GlassTextStyle.title.resolvedSize)
        XCTAssertGreaterThan(GlassTextStyle.title.resolvedSize, GlassTextStyle.body.resolvedSize)
        XCTAssertGreaterThan(GlassTextStyle.body.resolvedSize, GlassTextStyle.subheadline.resolvedSize)
    }

    /// The scale climbs: no step is drawn smaller than a step below it.
    func test_theScaleIsOrdered() {
        let order = ["micro", "caption", "label", "body", "title", "heading", "display", "number"]
        let sizes = order.compactMap { GlassTokens.TypeScale.all[$0]?.size }
        XCTAssertEqual(sizes.count, order.count)
        XCTAssertEqual(sizes, sizes.sorted())
    }

    /// Leading and tracking scale with the drawn size, not apart from it.
    func test_leadingAndTrackingFollowTheDrawnSize() {
        for (name, step) in GlassTokens.TypeScale.all {
            XCTAssertEqual(step.scale, step.textStyle.resolvedSize / step.size, accuracy: 0.0001, "type.\(name)")
            XCTAssertEqual(step.resolvedTracking, step.tracking * step.scale, accuracy: 0.0001, "type.\(name)")
            XCTAssertGreaterThanOrEqual(step.lineSpacing, 0, "type.\(name)")
        }
    }

    /// Only the mono step is SF Mono; prose is never monospaced.
    func test_onlyMonoIsMonospaced() {
        for (name, step) in GlassTokens.TypeScale.all {
            XCTAssertEqual(step.design == .monospaced, name == "mono", "type.\(name)")
        }
    }
}

/// Words in TCDesign are set with `glassType(_:)`, never at fixed points.
/// The one fixed-size font is `glassGlyph(_:weight:)`, for marks inside a
/// control of fixed size.
final class FixedPointTypeTests: XCTestCase {
    func test_noComponentSetsWordsAtFixedPoints() throws {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()  // TCDesignTests
            .deletingLastPathComponent()  // Tests
            .deletingLastPathComponent()  // macos
            .appendingPathComponent("Sources/TCDesign")
        let files = try XCTUnwrap(FileManager.default.enumerator(at: root, includingPropertiesForKeys: nil))
            .compactMap { $0 as? URL }
            .filter { $0.pathExtension == "swift" }
        XCTAssertGreaterThanOrEqual(files.count, 8, "the TCDesign sources were not found")

        var allowed = 0
        var failures: [String] = []
        for file in files {
            let lines = try String(contentsOf: file, encoding: .utf8).components(separatedBy: "\n")
            for (index, line) in lines.enumerated() where line.contains("system(size:") {
                if line.contains("font(.system(size: size, weight: weight.font))") {
                    allowed += 1
                } else {
                    failures.append("\(file.lastPathComponent):\(index + 1) sets a fixed point size; use glassType or glassGlyph")
                }
            }
        }
        XCTAssertEqual(allowed, 1, "glassGlyph's own font was not found; this scan proved nothing")
        XCTAssertTrue(failures.isEmpty, failures.joined(separator: "\n"))
    }
}
