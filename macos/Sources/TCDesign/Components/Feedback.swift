import SwiftUI

// MARK: - Spinner

/// Work under way with no measure of how far along (#1146 `Spinner`): a
/// 12pt ring whose quarter arc turns once every `motion.spin` seconds. The
/// replacement for an indeterminate stock `ProgressView`.
///
/// Under Reduce Motion the arc stands still. To assistive tech it is a
/// native indeterminate progress view named by `label`, the caller's word;
/// with no label it is hidden, for a spinner beside words that already say
/// what is happening. `standalone` keeps an unlabelled spinner that stands
/// alone, as the stock one did, so VoiceOver still hears that something is
/// in progress.
public struct GlassSpinner: View {
    private let label: String
    private let size: CGFloat
    private let standalone: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    public init(_ label: String = "", size: CGFloat = GlassTokens.Size.spinner, standalone: Bool = false) {
        self.label = label
        self.size = size
        self.standalone = standalone
    }

    /// Whether assistive tech skips it: only an unlabelled spinner beside
    /// its words.
    static func isHidden(label: String, standalone: Bool) -> Bool {
        label.isEmpty && !standalone
    }

    /// The arc's turn, in degrees, at `time`: a full turn per
    /// `motion.spin`, and none under Reduce Motion.
    static func angle(at time: TimeInterval, reduceMotion: Bool) -> Double {
        guard !reduceMotion else { return 0 }
        let period = GlassTokens.Motion.spin
        return time.truncatingRemainder(dividingBy: period) / period * 360
    }

    public var body: some View {
        TimelineView(.animation(paused: reduceMotion)) { context in
            ZStack {
                Circle()
                    .stroke(GlassColor.ink(0.18), lineWidth: GlassTokens.Size.spinnerStroke)
                Circle()
                    .trim(from: 0, to: 0.25)
                    .stroke(GlassColor.textSecondary, style: StrokeStyle(lineWidth: GlassTokens.Size.spinnerStroke, lineCap: .round))
                    .rotationEffect(.degrees(Self.angle(at: context.date.timeIntervalSinceReferenceDate, reduceMotion: reduceMotion)))
            }
            .padding(GlassTokens.Size.spinnerStroke / 2)
        }
        .frame(width: size, height: size)
        .accessibilityRepresentation {
            ProgressView { Text(label) }
        }
        .accessibilityHidden(Self.isHidden(label: label, standalone: standalone))
    }
}

// MARK: - Skeleton

/// A placeholder bar where content is still loading (#1146 `Skeleton`): a
/// faint rounded bar whose opacity breathes down to `opacity.skeletonDim`
/// over `motion.pulse` seconds, on the one motion curve. Still under Reduce
/// Motion. Hidden from assistive tech: a spinner or the words beside it
/// say that something is loading.
public struct GlassSkeleton: View {
    private let width: CGFloat?
    private let height: CGFloat
    @State private var dim = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    /// `width` nil fills the width offered.
    public init(width: CGFloat? = nil, height: CGFloat = GlassTokens.Size.skeletonHeight) {
        self.width = width
        self.height = height
    }

    /// The pulse: half a `motion.pulse` each way, on the token curve,
    /// repeating; none under Reduce Motion.
    static func pulse(_ reduceMotion: Bool) -> Animation? {
        reduceMotion ? nil : GlassMotion.curve(GlassTokens.Motion.pulse / 2).repeatForever(autoreverses: true)
    }

    /// The bar's opacity at either end of the pulse.
    static func opacity(dim: Bool) -> Double {
        dim ? GlassTokens.Opacity.skeletonDim : 1
    }

    public var body: some View {
        RoundedRectangle(cornerRadius: GlassTokens.Radius.control, style: .continuous)
            .fill(GlassColor.ink(0.08))
            .frame(width: width, height: height)
            .frame(maxWidth: width == nil ? .infinity : nil, alignment: .leading)
            .opacity(Self.opacity(dim: dim && !reduceMotion))
            .animation(Self.pulse(reduceMotion), value: dim)
            .onAppear { dim = true }
            .accessibilityHidden(true)
    }
}

// MARK: - Warning glyph

/// The warning triangle (#1146 `WarningGlyph`), drawn from its 16-unit
/// artwork in the foreground style: the replacement for the SF warning
/// symbol, so it matches the other shells. Hidden from assistive tech
/// unless `label`, the caller's word, names it.
public struct GlassWarningGlyph: View {
    private let label: String
    private let size: CGFloat

    public init(_ label: String = "", size: CGFloat = 12) {
        self.label = label
        self.size = size
    }

    public var body: some View {
        let scale = size / 16
        let line = StrokeStyle(lineWidth: 1.4 * scale, lineCap: .round, lineJoin: .round)
        ZStack {
            GlassWarningShape.triangle.stroke(style: line)
            GlassWarningShape.bar.stroke(style: line)
            GlassWarningShape.dot.fill()
        }
        .frame(width: size, height: size)
        .accessibilityElement()
        .accessibilityLabel(label)
        .accessibilityAddTraits(.isImage)
        .accessibilityHidden(label.isEmpty)
    }
}

/// The warning glyph's three parts in a 16×16 box, scaled to the frame.
enum GlassWarningShape: Shape {
    case triangle, bar, dot

    func path(in rect: CGRect) -> Path {
        let s = min(rect.width, rect.height) / 16
        func p(_ x: CGFloat, _ y: CGFloat) -> CGPoint { CGPoint(x: rect.minX + x * s, y: rect.minY + y * s) }
        var path = Path()
        switch self {
        case .triangle:
            path.move(to: p(8, 1.75))
            path.addLine(to: p(15, 14.25))
            path.addLine(to: p(1, 14.25))
            path.closeSubpath()
        case .bar:
            path.move(to: p(8, 6.25))
            path.addLine(to: p(8, 9.75))
        case .dot:
            let r = 0.85 * s
            let c = p(8, 11.9)
            path.addEllipse(in: CGRect(x: c.x - r, y: c.y - r, width: 2 * r, height: 2 * r))
        }
        return path
    }
}
