import SwiftUI

/// A status dot. Status is shown as a dot and a label, never a fill.
public struct GlassStatusDot: View {
    private let status: GlassStatus
    private let size: CGFloat
    private let halo: Bool
    private let ring: Bool

    public init(_ status: GlassStatus, size: CGFloat = GlassTokens.Size.dotLarge, halo: Bool = false, ring: Bool = false) {
        self.status = status
        self.size = size
        self.halo = halo
        self.ring = ring
    }

    public var body: some View {
        Circle()
            .fill(status.color)
            .frame(width: size, height: size)
            .overlay {
                if halo {
                    Circle().stroke(Color.white.opacity(0.08), lineWidth: 2).padding(-1)
                }
                if ring {
                    Circle().stroke(status.color.opacity(0.25), lineWidth: 3).padding(-1.5)
                }
            }
            .accessibilityHidden(true)
    }
}

/// A dot and its label: the only way status is shown.
public struct GlassStatusLabel: View {
    private let title: String
    private let status: GlassStatus

    public init(_ title: String, status: GlassStatus) {
        self.title = title
        self.status = status
    }

    public var body: some View {
        HStack(spacing: 7) {
            GlassStatusDot(status)
            Text(title)
        }
        .font(.system(size: 12, weight: .medium))
        .foregroundStyle(GlassColor.textSecondary)
        .accessibilityElement(children: .combine)
    }
}

/// A mono chip with a hairline ring at 60% of its colour.
public struct GlassChip: View {
    private let title: String
    private let status: GlassStatus?

    public init(_ title: String, status: GlassStatus? = nil) {
        self.title = title
        self.status = status
    }

    public var body: some View {
        let ink = status?.color ?? GlassColor.textSecondary
        HStack(spacing: 5) {
            if let status { GlassStatusDot(status, size: 6) }
            Text(title)
        }
        .glassType(GlassTokens.TypeScale.mono)
        .foregroundStyle(ink)
        .padding(.vertical, 3)
        .padding(.horizontal, 9)
        .overlay(Capsule().strokeBorder(ink.opacity(0.6), lineWidth: 0.5))
        .accessibilityElement(children: .combine)
    }
}

/// A tinted verdict pill (Contributed, Under review, Taken back, Drafts).
public struct GlassTag: View {
    public enum Tone: Sendable, Equatable { case neutral, on, ask, outside, accent }

    private let title: String
    private let tone: Tone

    public init(_ title: String, tone: Tone = .neutral) {
        self.title = title
        self.tone = tone
    }

    public var body: some View {
        let (fill, ink): (GlassRGBA, Color) = switch tone {
        case .neutral: (GlassTokens.Color.tintNeutral, GlassColor.textSecondary)
        case .on: (GlassTokens.Color.tintOn, GlassTokens.Color.statusOn.color)
        case .ask: (GlassTokens.Color.tintAsk, GlassTokens.Color.statusAsk.color)
        case .outside: (GlassTokens.Color.tintOutside, GlassTokens.Color.statusOutside.color)
        case .accent: (GlassTokens.Color.tintAccent, GlassColor.accentText)
        }
        Text(title)
            .font(.system(size: 10, weight: .bold))
            .foregroundStyle(ink)
            .padding(.vertical, 3)
            .padding(.horizontal, 8)
            .background(Capsule().fill(fill.color))
    }
}

/// Decisions owed. On the menu bar and the Traces tab only; never queue
/// depth or credit. `subtle` is the white count pill inside a tab.
public struct GlassBadge: View {
    private let count: Int
    private let subtle: Bool
    private let label: String?

    /// `label` is what VoiceOver reads, from the core's copy; without it the
    /// count is read alone.
    public init(count: Int, subtle: Bool = false, label: String? = nil) {
        self.count = count
        self.subtle = subtle
        self.label = label
    }

    public var body: some View {
        Text("\(count)")
            .font(.system(size: 10, weight: .bold))
            .monospacedDigit()
            .foregroundStyle(subtle ? GlassColor.textPrimary : GlassTokens.Color.textOnStatus.color)
            .padding(.horizontal, 5)
            .frame(minWidth: subtle ? nil : 16, minHeight: subtle ? 14 : 16)
            .background(
                Capsule().fill(subtle ? Color.white.opacity(0.16) : GlassTokens.Color.statusOutside.color)
            )
            .accessibilityLabel(label ?? "\(count)")
    }
}

/// The tools the shell knows. A tool with artwork shows its logo; one
/// without (Gemini CLI, Cline, any other) shows its initials.
/// Display names are not here: the caller shows the core's name for a tool.
public enum GlassTool: Sendable, Equatable {
    case claudeCode, codex, antigravity, geminiCLI, cline, openCode, theia
    /// Any other tool, by its two-letter initials.
    case other(initials: String)

    /// The tint the tool's logo is drawn in.
    public var tint: GlassRGBA {
        switch self {
        case .claudeCode: GlassTokens.Color.toolClaude
        case .codex: GlassTokens.Color.toolCodex
        case .antigravity, .geminiCLI: GlassTokens.Color.toolAntigravity
        case .openCode: GlassTokens.Color.toolOpenCode
        case .theia: GlassTokens.Color.toolTheia
        case .cline, .other: GlassTokens.Color.textPrimary
        }
    }

    /// The logo artwork, if the tool has any. Gemini CLI and Cline have
    /// none yet, in either shell.
    public var logo: GlassToolLogoID? {
        switch self {
        case .claudeCode: .claude
        case .codex: .codex
        case .antigravity: .antigravity
        case .openCode: .openCode
        case .theia: .theia
        case .geminiCLI, .cline, .other: nil
        }
    }

    var initials: String {
        switch self {
        case .claudeCode: "CC"
        case .codex: "Cx"
        case .antigravity: "Ag"
        case .geminiCLI: "Gm"
        case .cline: "Cl"
        case .openCode: "OC"
        case .theia: "Th"
        case let .other(initials): String(initials.prefix(2))
        }
    }
}

/// The 22pt tile in a tree row: a tool's mark, a folder or a session.
/// A tool is drawn as its logo in its tint, or as its tinted initials when
/// it has no logo.
public struct GlassToolTile: View {
    public enum Kind: Sendable, Equatable {
        case tool(GlassTool), folder, session
    }

    private let kind: Kind
    private let large: Bool

    public init(_ kind: Kind, large: Bool = false) {
        self.kind = kind
        self.large = large
    }

    public var body: some View {
        let side = large ? GlassTokens.Size.toolTileLarge : GlassTokens.Size.toolTile
        Group {
            switch kind {
            case let .tool(tool):
                Group {
                    if let logo = tool.logo {
                        let mark = large ? GlassTokens.Size.toolLogoLarge : GlassTokens.Size.toolLogo
                        GlassToolLogoShape(logo)
                            .fill(tool.tint.color)
                            .frame(width: mark, height: mark)
                    } else {
                        Text(tool.initials)
                            .font(.system(size: large ? 11 : 9, weight: .bold))
                            .foregroundStyle(tool.tint.color)
                    }
                }
                    .frame(width: side, height: side)
                    .background(RoundedRectangle(cornerRadius: GlassTokens.Radius.tile, style: .continuous).fill(Color.white.opacity(0.1)))
            case .folder:
                Image(systemName: "folder.fill")
                    .font(.system(size: large ? 12 : 10, weight: .semibold))
                    .foregroundStyle(GlassTokens.Color.tileFolderInk.color)
                    .frame(width: side, height: side)
                    .background(RoundedRectangle(cornerRadius: GlassTokens.Radius.tile, style: .continuous).fill(GlassTokens.Color.tileFolder.color))
            case .session:
                Image(systemName: "doc.plaintext")
                    .font(.system(size: large ? 13 : 10))
                    .foregroundStyle(GlassTokens.Color.statusOff.color)
                    .frame(width: side, height: side)
                    .background(RoundedRectangle(cornerRadius: GlassTokens.Radius.tile, style: .continuous).fill(GlassTokens.Color.tileFolder.color))
            }
        }
        .accessibilityHidden(true)
    }
}

/// One bucket of `GlassBarGraph`.
public struct GlassBarBucket: Identifiable, Sendable {
    public let id: String
    public let label: String
    /// Shared: rises from the axis in purple.
    public let up: Double
    /// Kept: falls from the axis in blue.
    public let down: Double
    /// What VoiceOver reads for the bucket.
    public let description: String

    public init(id: String, label: String, up: Double, down: Double, description: String) {
        self.id = id
        self.label = label
        self.up = up
        self.down = down
        self.description = description
    }
}

/// Shared over kept, per period. Values scale to the largest bucket, never
/// below `scaleFloor`, so a count of one stays short.
public struct GlassBarGraph: View {
    private let buckets: [GlassBarBucket]
    private let scaleFloor: Double
    @Binding private var hovered: String?

    public init(_ buckets: [GlassBarBucket], scaleFloor: Double = 1, hovered: Binding<String?> = .constant(nil)) {
        self.buckets = buckets
        self.scaleFloor = scaleFloor
        self._hovered = hovered
    }

    public var body: some View {
        let maximum = max(scaleFloor, buckets.map { max($0.up, $0.down) }.max() ?? 0)
        let thin = buckets.count > 14
        HStack(alignment: .bottom, spacing: thin ? 2 : 6) {
            ForEach(buckets) { bucket in
                VStack(spacing: GlassTokens.Space.s2) {
                    ZStack {
                        RoundedRectangle(cornerRadius: thin ? 3 : GlassTokens.Radius.control, style: .continuous)
                            .fill(Color.white.opacity(hovered == bucket.id ? 0.18 : 0.08))
                        VStack(spacing: 0) {
                            Spacer(minLength: 0)
                            bar(bucket.up, of: maximum, color: GlassTokens.Color.dataShared.color, top: true)
                            Rectangle().fill(Color.white.opacity(0.18)).frame(height: 1)
                            bar(bucket.down, of: maximum, color: GlassTokens.Color.dataKept.color, top: false)
                            Spacer(minLength: 0)
                        }
                        .padding(.horizontal, thin ? 2 : 6)
                    }
                    .frame(height: 96)
                    Text(bucket.label)
                        .font(.system(size: thin ? 9 : 11))
                        .foregroundStyle(hovered == bucket.id ? GlassColor.textPrimary : GlassTokens.Color.statusOff.color)
                        .frame(height: 12)
                }
                .onHover { hovered = $0 ? bucket.id : nil }
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(bucket.description)
            }
        }
    }

    private func bar(_ value: Double, of maximum: Double, color: Color, top: Bool) -> some View {
        let height = value <= 0 ? 0 : max(3, value / maximum * 46)
        return VStack(spacing: 0) {
            if !top { Spacer(minLength: 0) }
            UnevenRoundedRectangle(
                topLeadingRadius: top ? 99 : 0,
                bottomLeadingRadius: top ? 0 : 99,
                bottomTrailingRadius: top ? 0 : 99,
                topTrailingRadius: top ? 99 : 0
            )
            .fill(color)
            .frame(height: height)
            if top { EmptyView() }
        }
        .frame(height: 47.5, alignment: top ? .bottom : .top)
    }
}

/// A node card on the map, shown on hover and pinned on click.
public struct GlassNodeCard: View {
    private let title: String
    private let detail: String
    private let hint: String?

    public init(_ title: String, detail: String, hint: String? = nil) {
        self.title = title
        self.detail = detail
        self.hint = hint
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(title).font(.system(size: 13, weight: .semibold)).foregroundStyle(GlassColor.textPrimary)
            Text(detail).font(.system(size: 12)).foregroundStyle(GlassColor.textSecondary)
            if let hint {
                Text(hint).font(.system(size: 10, design: .monospaced)).foregroundStyle(GlassColor.textTertiary).padding(.top, 3)
            }
        }
        .padding(.vertical, 12)
        .padding(.horizontal, 14)
        .frame(width: GlassTokens.Size.nodeCardWidth, alignment: .leading)
        .glassTier(.nodeCard)
        .accessibilityElement(children: .combine)
    }
}

/// A map node's look, for the map's `Canvas` (R8): fill, ring and radius
/// by state. The map draws; this states what each node looks like.
public struct GlassMapNodeStyle: Sendable, Equatable {
    public enum State: Sendable, Equatable { case active, idle, outside, hub }

    public let fill: GlassRGBA
    public let ring: Bool
    public let radius: CGFloat

    public init(_ state: State) {
        switch state {
        case .active:
            fill = GlassTokens.Color.blue
            ring = true
            radius = 11
        case .idle:
            fill = GlassTokens.Color.mapNodeOff
            ring = false
            radius = 11
        case .outside:
            fill = GlassTokens.Color.statusOutside
            ring = true
            radius = 11
        case .hub:
            fill = GlassTokens.Color.mapHub
            ring = true
            radius = 16
        }
    }
}
