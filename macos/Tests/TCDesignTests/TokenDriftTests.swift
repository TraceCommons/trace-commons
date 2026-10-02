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
        }
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
            XCTAssertEqual(String(describing: made.weight), entry["weight"] as? String, "type.\(name) weight")
            XCTAssertTrue(Double(made.lineHeight) == Self.number(entry["lineHeight"]), "type.\(name) lineHeight")
            XCTAssertTrue(Double(made.tracking) == Self.number(entry["tracking"]), "type.\(name) tracking")
            XCTAssertTrue(made.uppercase == ((entry["uppercase"] as? Bool) ?? false), "type.\(name) uppercase")
            XCTAssertTrue(made.tabular == ((entry["tabular"] as? Bool) ?? false), "type.\(name) tabular")
            XCTAssertTrue((made.design == .monospaced) == ((entry["design"] as? String) == "monospaced"), "type.\(name) design")
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
