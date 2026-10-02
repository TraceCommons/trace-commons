import SwiftUI

/// Tree row: tool › folder › session. Indent grows 18pt a level; title
/// 13/600 (500 at a session), sub-line 11 tertiary, amber when flagged.
public struct GlassListRow: View {
    public enum Depth: Int, Sendable { case tool = 0, folder = 1, session = 2 }
    public enum Flag: Sendable, Equatable { case ask, on }

    private let depth: Depth
    private let tile: GlassToolTile.Kind
    private let title: String
    private let sub: String?
    private let flag: Flag?
    private let selected: Bool
    private let off: Bool
    private let expanded: Bool?
    private let submitTitle: String?
    private let submitDone: Bool
    private let watched: Binding<Bool>?
    private let watchLabel: String
    private let expandLabel: String?
    private let menuOpen: Bool
    private let onToggleExpand: (() -> Void)?
    private let onSelect: (() -> Void)?
    private let onSubmit: (() -> Void)?
    private let onMenu: (() -> Void)?

    /// `expanded` is nil for a row that cannot expand. `watched` is nil for
    /// a row with no switch of its own (a session). A nil `onSubmit` with a
    /// `submitTitle` shows the pill disabled. Every word is the caller's:
    /// `watchLabel` names the switch and `expandLabel` the chevron, from the
    /// core's copy; the components author no wording.
    public init(
        depth: Depth,
        tile: GlassToolTile.Kind,
        title: String,
        sub: String? = nil,
        flag: Flag? = nil,
        selected: Bool = false,
        off: Bool = false,
        expanded: Bool? = nil,
        submitTitle: String? = nil,
        submitDone: Bool = false,
        watched: Binding<Bool>? = nil,
        watchLabel: String = "",
        expandLabel: String? = nil,
        menuOpen: Bool = false,
        onToggleExpand: (() -> Void)? = nil,
        onSelect: (() -> Void)? = nil,
        onSubmit: (() -> Void)? = nil,
        onMenu: (() -> Void)? = nil
    ) {
        self.depth = depth
        self.tile = tile
        self.title = title
        self.sub = sub
        self.flag = flag
        self.selected = selected
        self.off = off
        self.expanded = expanded
        self.submitTitle = submitTitle
        self.submitDone = submitDone
        self.watched = watched
        self.watchLabel = watchLabel
        self.expandLabel = expandLabel
        self.menuOpen = menuOpen
        self.onToggleExpand = onToggleExpand
        self.onSelect = onSelect
        self.onSubmit = onSubmit
        self.onMenu = onMenu
    }

    public var body: some View {
        HStack(spacing: GlassTokens.Space.s4) {
            Group {
                if let expanded, let onToggleExpand {
                    Button(action: onToggleExpand) {
                        Text("›")
                            .font(.system(size: 14))
                            .foregroundStyle(selected ? Color.white : GlassTokens.Color.statusOff.color)
                            .rotationEffect(.degrees(expanded ? 90 : 0))
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(expandLabel ?? title)
                    .accessibilityValue(expanded ? "expanded" : "collapsed")
                } else {
                    Color.clear
                }
            }
            .frame(width: 16)

            GlassToolTile(tile)

            VStack(alignment: .leading, spacing: 0) {
                Text(title)
                    .font(.system(size: 13, weight: depth == .session ? .medium : .semibold))
                    .lineLimit(1)
                if let sub {
                    Text(sub)
                        .font(.system(size: 11))
                        .foregroundStyle(subColor)
                        .lineLimit(1)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            if let submitTitle {
                Button(submitTitle) { onSubmit?() }
                    .buttonStyle(GlassButtonStyle(.submit(done: submitDone)))
                    .disabled(onSubmit == nil)
            }

            if let watched {
                Toggle(watchLabel, isOn: watched)
                    .labelsHidden()
                    .toggleStyle(GlassToggleStyle(.watch))
            }

            Group {
                if let onMenu {
                    GlassKebab(open: menuOpen, action: onMenu)
                } else {
                    Color.clear
                }
            }
            .frame(width: 22)
        }
        .foregroundStyle(selected ? Color.white : GlassColor.textPrimary)
        .padding(.leading, 8 + CGFloat(depth.rawValue) * 18)
        .padding(.trailing, 6)
        .frame(height: GlassTokens.Size.listRow)
        .background(
            RoundedRectangle(cornerRadius: GlassTokens.Radius.control, style: .continuous)
                .fill(selected ? GlassTokens.Color.selection.color : Color.clear)
        )
        .opacity(off ? GlassTokens.Opacity.rowOff : 1)
        .contentShape(Rectangle())
        .onTapGesture { onSelect?() }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(title)
        .accessibilityValue(sub ?? "")
        .accessibilityAddTraits(selected ? [.isSelected, .isButton] : .isButton)
    }

    private var subColor: Color {
        if selected { return Color.white.opacity(0.8) }
        return switch flag {
        case .ask: GlassTokens.Color.statusAsk.color
        case .on: GlassTokens.Color.statusOn.color
        case nil: GlassColor.textTertiary
        }
    }
}
