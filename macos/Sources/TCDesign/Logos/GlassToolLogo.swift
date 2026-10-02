import SwiftUI

/// The tools that have logo artwork. A tool without one is drawn as its
/// initials (`GlassTool.logo` is nil for it).
public enum GlassToolLogoID: String, CaseIterable, Sendable {
    case claude, codex, antigravity, openCode, theia

    var artwork: GlassLogoArtwork {
        switch self {
        case .claude: GlassLogoPaths.claude
        case .codex: GlassLogoPaths.codex
        case .antigravity: GlassLogoPaths.antigravity
        case .openCode: GlassLogoPaths.openCode
        case .theia: GlassLogoPaths.theia
        }
    }

    /// The logo's paths parsed into one path in viewBox units, once per
    /// process. Nil only if the artwork is malformed, which
    /// GlassToolLogoTests rules out.
    var path: Path? { GlassToolLogoID.parsed[self] }

    // `Path`, not `CGPath`: it is Sendable, so the cache can be a static.
    private static let parsed: [GlassToolLogoID: Path] = allCases.reduce(into: [:]) { result, id in
        let combined = CGMutablePath()
        for data in id.artwork.paths {
            guard let path = SVGPathData.parse(data) else { return }
            combined.addPath(path)
        }
        result[id] = Path(combined)
    }
}

/// A tool's logo, scaled to fit its frame with the aspect ratio kept and
/// filled non-zero, as SVG fills it. Decorative: the caller labels the row.
public struct GlassToolLogoShape: Shape {
    private let id: GlassToolLogoID

    public init(_ id: GlassToolLogoID) {
        self.id = id
    }

    public func path(in rect: CGRect) -> Path {
        guard let path = id.path else { return Path() }
        let box = id.artwork.viewBox
        let scale = min(rect.width / box.width, rect.height / box.height)
        let transform = CGAffineTransform(translationX: rect.midX, y: rect.midY)
            .scaledBy(x: scale, y: scale)
            .translatedBy(x: -box.midX, y: -box.midY)
        return path.applying(transform)
    }
}
