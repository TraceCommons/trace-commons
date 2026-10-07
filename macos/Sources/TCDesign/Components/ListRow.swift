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
    private let submitFocusable: Bool
    private let watched: Binding<Bool>?
    private let watchDisabled: Bool
    private let accessory: AnyView?
    private let watchLabel: String
    private let expandLabel: String?
    private let menuLabel: String
    private let menuOpen: Bool
    private let onToggleExpand: (() -> Void)?
    private let onSelect: (() -> Void)?
    private let onSubmit: (() -> Void)?
    private let onMenu: (() -> Void)?
    @State private var hovering = false

    /// `accessory` is a control drawn before the row menu (a folder's mode
    /// picker). `watchDisabled` shows the switch's state without letting it
    /// change. `expanded` is nil for a row that cannot expand. `watched` is nil for
    /// a row with no switch of its own (a session). A nil `onSubmit` with a
    /// `submitTitle` shows the pill disabled. Every word is the caller's:
    /// `watchLabel` names the switch, `expandLabel` the chevron and
    /// `menuLabel` the row menu, from the core's copy; the components author no
    /// wording. A row menu with an empty `menuLabel` is not drawn, so it can
    /// never borrow the row's own name. `submitFocusable` false keeps the
    /// pill out of the keyboard's tab order, for a list whose focus roves
    /// with its selection (only the selected row's pill is a stop).
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
        submitFocusable: Bool = true,
        watched: Binding<Bool>? = nil,
        watchDisabled: Bool = false,
        accessory: AnyView? = nil,
        watchLabel: String = "",
        expandLabel: String? = nil,
        menuLabel: String = "",
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
        self.submitFocusable = submitFocusable
        self.watched = watched
        self.watchDisabled = watchDisabled
        self.accessory = accessory
        self.watchLabel = watchLabel
        self.expandLabel = expandLabel
        self.menuLabel = menuLabel
        self.menuOpen = menuOpen
        self.onToggleExpand = onToggleExpand
        self.onSelect = onSelect
        self.onSubmit = onSubmit
        self.onMenu = onMenu
    }

    /// The ink a selected row sets its title, sub-line and chevron in, on
    /// `GlassTokens.Color.selection`. Solid, so the sub-line keeps text
    /// contrast too (`SelectionContrastTests`).
    static let selectedInk = GlassTokens.Color.textOnAccent
    /// The sub-line's ink when selected: the white ink at 80%, as #1146's
    /// `.tc-list-row[aria-selected] .tc-list-row__sub` (owner ruling,
    /// 2026-10-07: #1146 wins for theming).
    static let selectedSubInk = GlassRGBA(selectedInk.rgb, alpha: 0.8)

    /// The watch switch's column, kept on every row (#1146 `38px`).
    static let watchColumn: CGFloat = GlassTokens.Size.watchSwitchWidth

    /// The row's fill: the selection, or the faint hover fill under the
    /// pointer (#1146 `.tc-list-row:hover`), never over a selected row.
    static func fill(selected: Bool, hovering: Bool) -> GlassRGBA? {
        if selected { return GlassTokens.Color.selection }
        return hovering ? GlassTokens.Color.rowHover : nil
    }

    /// An unflagged sub-line's ink: tertiary, on an off row too, which then
    /// fades by `rowOff` with the rest of the row (#1146 `glass.css`
    /// `.tc-list-row[data-off]`).
    static func plainSubInk(off: Bool) -> GlassRGBA {
        GlassTokens.Color.textTertiary
    }

    public var body: some View {
        HStack(spacing: GlassTokens.Space.s4) {
            Group {
                if let expanded, let onToggleExpand {
                    Button(action: onToggleExpand) {
                        Text("›")
                            .glassGlyph(14)
                            .foregroundStyle(selected ? Self.selectedInk.color : GlassTokens.Color.statusOff.color)
                            .rotationEffect(.degrees(expanded ? 90 : 0))
                            .glassPressedFill()
                    }
                    .buttonStyle(GlassPressStyle())
                    // A native disclosure for assistive tech, so the system
                    // says expanded or collapsed in the person's language.
                    .accessibilityRepresentation {
                        DisclosureGroup(isExpanded: Binding(get: { expanded }, set: { _ in onToggleExpand() })) {
                            EmptyView()
                        } label: {
                            Text(expandLabel ?? title)
                        }
                    }
                } else {
                    Color.clear
                }
            }
            .frame(width: 16)

            GlassToolTile(tile)

            VStack(alignment: .leading, spacing: 0) {
                Text(title)
                    .glassType(GlassTokens.TypeScale.body.weight(depth == .session ? .medium : .semibold))
                    .lineLimit(1)
                if let sub {
                    Text(sub)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(subColor)
                        .lineLimit(1)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            if let submitTitle {
                Button(submitTitle) { onSubmit?() }
                    .buttonStyle(GlassButtonStyle(.submit(done: submitDone)))
                    // One line at its full width ("Submit all (12)" at the
                    // 360pt left pane): the title gives way, never the count.
                    .lineLimit(1)
                    .fixedSize()
                    .disabled(onSubmit == nil)
                    .focusable(submitFocusable)
            }

            if let accessory {
                accessory
            }

            // The switch's column is always kept (#1146 `.tc-list-row`'s
            // `38px` track, a placeholder where a row has no switch), so a
            // session's Review lines up with its folder's Submit.
            Group {
                if let watched {
                    Toggle(watchLabel, isOn: watched)
                        .toggleStyle(GlassToggleStyle(.watch, showsLabel: false))
                        // A switch the person cannot change here is disabled, so
                        // assistive tech does not offer a control that does nothing.
                        .disabled(watchDisabled)
                } else {
                    Color.clear.accessibilityHidden(true)
                }
            }
            .frame(width: Self.watchColumn)

            Group {
                // The kebab needs its own name: falling back to the row title
                // gave it the same VoiceOver name as the row it sits in.
                if let onMenu, !menuLabel.isEmpty {
                    GlassKebab(menuLabel, open: menuOpen, action: onMenu)
                } else {
                    Color.clear
                }
            }
            .frame(width: 22)
        }
        .foregroundStyle(selected ? Self.selectedInk.color : GlassColor.textPrimary)
        .padding(.leading, 8 + CGFloat(depth.rawValue) * 18)
        .padding(.trailing, 6)
        .frame(minHeight: GlassTokens.Size.listRow)
        .background(
            RoundedRectangle(cornerRadius: GlassTokens.Radius.control, style: .continuous)
                .fill(Self.fill(selected: selected, hovering: hovering)?.color ?? Color.clear)
        )
        .opacity(off ? GlassTokens.Opacity.rowOff : 1)
        .contentShape(Rectangle())
        .onHover { hovering = $0 }
        // A click selects the row. The keyboard does not stop on each row:
        // the list holding the rows is one tab stop and the arrow keys move
        // its selection (spec, "Components": one list, not a tab stop per
        // row), as the Traces tree does. VoiceOver selects with the row's
        // default action.
        .onTapGesture { onSelect?() }
        .accessibilityAction { onSelect?() }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(title)
        .accessibilityValue(sub ?? "")
        .accessibilityAddTraits(selected ? [.isSelected, .isButton] : .isButton)
        // The tree's depth, for assistive tech: a tool row heads its
        // folders, a folder row its sessions, so the rotor and heading
        // navigation walk the levels as they are drawn.
        .accessibilityAddTraits(depth == .session ? [] : .isHeader)
        .accessibilityHeading(Self.heading(depth))
    }

    static func heading(_ depth: Depth) -> AccessibilityHeadingLevel {
        switch depth {
        case .tool: .h1
        case .folder: .h2
        case .session: .unspecified
        }
    }

    private var subColor: Color {
        if selected { return Self.selectedSubInk.color }
        return switch flag {
        case .ask: GlassTokens.Color.statusAsk.color
        case .on: GlassTokens.Color.statusOn.color
        case nil: Self.plainSubInk(off: off).color
        }
    }
}
