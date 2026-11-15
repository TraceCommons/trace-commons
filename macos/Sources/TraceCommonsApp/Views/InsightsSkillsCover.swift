import SwiftUI
import TCDesign

/// A catalog skill's cover, drawn here from its id and its publisher's id.
/// Nothing is fetched: the catalog carries no artwork.
///
/// The publisher picks the colours, so one publisher's skills look related;
/// the skill picks the pattern and where it sits. Both come from FNV-1a, not
/// `Hasher`, whose seed changes at every launch: the same skill has the same
/// cover on every run and every Mac.
enum SkillCoverArt {
    /// FNV-1a, 64 bit, over the string's UTF-8 bytes.
    static func fnv1a(_ value: String) -> UInt64 {
        var hash: UInt64 = 0xcbf2_9ce4_8422_2325
        for byte in value.utf8 {
            hash ^= UInt64(byte)
            hash = hash &* 0x0000_0100_0000_01b3
        }
        return hash
    }

    /// The hash a cover is read from: FNV-1a, then the 64-bit finalizer
    /// from MurmurHash3. FNV-1a alone changes few bits when two ids differ
    /// only in their last character ("skill-1", "skill-2"), and such
    /// neighbours would share a pattern and a colour.
    static func seed(_ value: String) -> UInt64 {
        var hash = fnv1a(value)
        hash ^= hash >> 33
        hash = hash &* 0xff51_afd7_ed55_8ccd
        hash ^= hash >> 33
        hash = hash &* 0xc4ce_b9fe_1a85_ec53
        hash ^= hash >> 33
        return hash
    }

    enum Pattern: Int, CaseIterable { case rings, bars, dots, arcs }

    struct RGB: Equatable {
        let red: Double, green: Double, blue: Double

        init(_ hex: UInt32) {
            red = Double((hex >> 16) & 0xFF) / 255
            green = Double((hex >> 8) & 0xFF) / 255
            blue = Double(hex & 0xFF) / 255
        }

        init(red: Double, green: Double, blue: Double) {
            self.red = red
            self.green = green
            self.blue = blue
        }

        var color: Color { Color(.sRGB, red: red, green: green, blue: blue) }

        /// WCAG relative luminance.
        var luminance: Double {
            func linear(_ channel: Double) -> Double {
                channel <= 0.03928 ? channel / 12.92 : pow((channel + 0.055) / 1.055, 2.4)
            }
            return 0.2126 * linear(red) + 0.7152 * linear(green) + 0.0722 * linear(blue)
        }

        /// WCAG contrast of white text on this colour.
        var contrastWithWhite: Double { 1.05 / (luminance + 0.05) }
    }

    /// The gradients a publisher can take. A cover is the same in light and
    /// in dark, as artwork is, and every stop holds white at 4.5:1 or more
    /// (SkillCoverTests), so the publisher's initials read on any of them.
    static let palettes: [(from: RGB, to: RGB)] = [
        (RGB(0x2F3FA8), RGB(0x6B3FB8)),
        (RGB(0x0E5C8F), RGB(0x1D4FB0)),
        (RGB(0x0F6B5C), RGB(0x1B5E86)),
        (RGB(0x8A3A12), RGB(0xA8324A)),
        (RGB(0x7A2E8C), RGB(0xB0306A)),
        (RGB(0x4A5A12), RGB(0x1F6B3A)),
        (RGB(0x6E5008), RGB(0x9A3C16)),
        (RGB(0x35404F), RGB(0x4A3F6B)),
    ]

    /// Everything a cover is drawn from.
    struct Spec: Equatable {
        let palette: Int
        let pattern: Pattern
        /// Where the pattern is centred or starts, each 0...1 of the cover.
        let anchorX: Double, anchorY: Double
        /// The pattern's spacing, as a share of the cover's shorter side.
        let step: Double
        /// The bars' lean, in degrees.
        let angle: Double
        /// Whether the gradient runs the other diagonal.
        let flipped: Bool

        init(skillID: String, publisherID: String) {
            palette = Int(SkillCoverArt.seed(publisherID) % UInt64(SkillCoverArt.palettes.count))
            // The separator keeps ("ab", "c") and ("a", "bc") apart.
            let hash = SkillCoverArt.seed(publisherID + "\u{0}" + skillID)
            pattern = Pattern(rawValue: Int(hash & 3)) ?? .rings
            anchorX = Double((hash >> 8) & 0xFF) / 255
            anchorY = Double((hash >> 16) & 0xFF) / 255
            step = 0.12 + Double((hash >> 24) & 0x3F) / 63 * 0.12
            angle = Double((hash >> 32) & 0xFF) / 255 * 70 - 35
            flipped = (hash >> 40) & 1 == 1
        }
    }

    /// Up to two letters for a publisher: the first letters of its first
    /// two words, or the first two letters of a single word.
    static func initials(_ name: String) -> String {
        let words = name.split { !$0.isLetter && !$0.isNumber }
        let letters = words.count > 1 ? words.prefix(2).compactMap(\.first).map(String.init).joined() : String(words.first?.prefix(2) ?? "")
        return letters.uppercased()
    }

    /// The pattern, in white at a low opacity over the gradient.
    static func draw(_ spec: SkillCoverArt.Spec, in context: inout GraphicsContext, size: CGSize) {
        let unit = min(size.width, size.height)
        let reach = max(size.width, size.height) * 1.6
        let step = max(4, unit * spec.step)
        let ink = GraphicsContext.Shading.color(.white.opacity(0.16))
        let anchor = CGPoint(x: size.width * spec.anchorX, y: size.height * spec.anchorY)
        switch spec.pattern {
        case .rings:
            var radius = step
            while radius < reach {
                context.stroke(Path(ellipseIn: CGRect(x: anchor.x - radius, y: anchor.y - radius, width: radius * 2, height: radius * 2)),
                               with: ink, lineWidth: max(1, step * 0.3))
                radius += step
            }
        case .bars:
            context.translateBy(x: size.width / 2, y: size.height / 2)
            context.rotate(by: .degrees(spec.angle))
            var x = -reach + step * spec.anchorX
            while x < reach {
                context.fill(Path(CGRect(x: x, y: -reach, width: step * 0.45, height: reach * 2)), with: ink)
                x += step
            }
        case .dots:
            var y = step * (spec.anchorY - 1)
            while y < size.height + step {
                var x = step * (spec.anchorX - 1)
                while x < size.width + step {
                    context.fill(Path(ellipseIn: CGRect(x: x, y: y, width: step * 0.45, height: step * 0.45)), with: ink)
                    x += step
                }
                y += step
            }
        case .arcs:
            let corner = CGPoint(x: spec.anchorX < 0.5 ? 0 : size.width, y: spec.anchorY < 0.5 ? 0 : size.height)
            var radius = step * (1 + spec.anchorX)
            while radius < reach {
                context.stroke(Path(ellipseIn: CGRect(x: corner.x - radius, y: corner.y - radius, width: radius * 2, height: radius * 2)),
                               with: ink, lineWidth: max(1, step * 0.45))
                radius += step * 1.5
            }
        }
    }
}

/// The cover itself, filling whatever frame it is given. Decorative: the
/// story beside it names the skill and the publisher in words, so it is
/// hidden from assistive technology.
struct SkillCover: View {
    let spec: SkillCoverArt.Spec
    /// The publisher's initials, or empty for none.
    var initials = ""
    /// Draw the initials as a small mark in the bottom-left corner (a
    /// band), not centred (a tile).
    var cornerMark = false
    var radius: CGFloat = 9

    var body: some View {
        let palette = SkillCoverArt.palettes[spec.palette]
        Rectangle()
            .fill(LinearGradient(colors: [palette.from.color, palette.to.color],
                                 startPoint: spec.flipped ? .topTrailing : .topLeading,
                                 endPoint: spec.flipped ? .bottomLeading : .bottomTrailing))
            .overlay {
                Canvas { context, size in SkillCoverArt.draw(spec, in: &context, size: size) }
            }
            .overlay(alignment: cornerMark ? .bottomLeading : .center) {
                if !initials.isEmpty { mark }
            }
            .clipShape(RoundedRectangle(cornerRadius: radius, style: .continuous))
            .accessibilityHidden(true)
    }

    @ViewBuilder private var mark: some View {
        if cornerMark {
            Text(initials).glassType(GlassTokens.TypeScale.caption.weight(.semibold))
                .foregroundStyle(.white)
                .padding(.horizontal, GlassTokens.Space.s3)
                .padding(.vertical, GlassTokens.Space.s1)
                .background(Capsule().fill(.black.opacity(0.28)))
                .padding(GlassTokens.Space.s4)
        } else {
            Text(initials).glassType(GlassTokens.TypeScale.label.weight(.semibold))
                .foregroundStyle(.white)
                .shadow(color: .black.opacity(0.35), radius: 1, y: 0.5)
        }
    }
}
