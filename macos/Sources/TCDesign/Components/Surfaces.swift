import SwiftUI

/// One glass pane: the app's outermost containment. Panes float with a gap
/// between them; there is no window chrome around them.
public struct GlassPane<Content: View>: View {
    private let padding: CGFloat?
    private let isContent: Bool
    private let content: Content

    /// `padding` defaults to the pane padding; pass 0 for edge-to-edge
    /// content such as the Traces tree. `isContent` puts the pane in the
    /// content layer (the map): the opaque base, never Liquid Glass, so the
    /// glass controls floating on it are not glass on glass.
    public init(
        padding: CGFloat? = GlassTokens.Space.panePadding, isContent: Bool = false,
        @ViewBuilder content: () -> Content
    ) {
        self.padding = padding
        self.isContent = isContent
        self.content = content()
    }

    public var body: some View {
        content
            .padding(padding ?? 0)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .glassTier(.pane)
            .environment(\.glassPaneIsContent, isContent)
    }
}

/// Popover tier: the menu-bar panel, floating menus.
///
/// One container for VoiceOver. Escape calls `onDismiss`; whoever presents
/// the popover closes it there and puts focus back on the control that
/// opened it (a SwiftUI `.popover` does both itself).
public struct GlassPopover<Content: View>: View {
    private let onDismiss: (() -> Void)?
    private let content: Content

    public init(onDismiss: (() -> Void)? = nil, @ViewBuilder content: () -> Content) {
        self.onDismiss = onDismiss
        self.content = content()
    }

    public var body: some View {
        content
            .padding(GlassTokens.Space.s5)
            .glassSurface(.popover, floating: true)
            .focusSection()
            .onExitCommand { onDismiss?() }
            .accessibilityElement(children: .contain)
    }
}

/// A sheet's body: "Exactly what would be sent" and the passkey steps.
public struct GlassSheet<Content: View>: View {
    private let title: String
    private let subtitle: String?
    private let content: Content

    public init(title: String, subtitle: String? = nil, @ViewBuilder content: () -> Content) {
        self.title = title
        self.subtitle = subtitle
        self.content = content()
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                Text(title).glassType(GlassTokens.TypeScale.title).foregroundStyle(GlassColor.textPrimary)
                if let subtitle {
                    Text(subtitle).glassType(GlassTokens.TypeScale.caption).foregroundStyle(GlassColor.textTertiary)
                }
            }
            content
        }
        .padding(GlassTokens.Space.panePadding)
        .frame(maxWidth: .infinity, alignment: .leading)
        .glassTier(.pane)
    }
}

/// A floating menu (view options, a row's menu). Items are `GlassMenuItem`.
///
/// One container for VoiceOver, and one focus section, so Tab and Full
/// Keyboard Access move through its items and the system focus ring shows
/// on each; Space or Return activates the focused item (it is a button).
/// Escape calls `onDismiss`; the presenter closes the menu there and puts
/// focus back on the control that opened it.
public struct GlassMenu<Content: View>: View {
    private let onDismiss: (() -> Void)?
    private let content: Content

    public init(onDismiss: (() -> Void)? = nil, @ViewBuilder content: () -> Content) {
        self.onDismiss = onDismiss
        self.content = content()
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 0) { content }
            .padding(5)
            .frame(minWidth: 220, alignment: .leading)
            .glassSurface(.menu, floating: true)
            .focusSection()
            .onExitCommand { onDismiss?() }
            .accessibilityElement(children: .contain)
    }
}

/// A menu row. `checked` makes it a checkbox item with a leading check.
public struct GlassMenuItem: View {
    private let title: String
    private let checked: Bool?
    private let action: () -> Void
    @State private var hovering = false
    @Environment(\.isEnabled) private var isEnabled

    public init(_ title: String, checked: Bool? = nil, action: @escaping () -> Void) {
        self.title = title
        self.checked = checked
        self.action = action
    }

    public var body: some View {
        Button(action: action) {
            HStack(spacing: GlassTokens.Space.s4) {
                if let checked {
                    Text(checked ? "✓" : "").frame(width: 12).accessibilityHidden(true)
                }
                Text(title)
                Spacer(minLength: 0)
            }
            .glassType(GlassTokens.TypeScale.body)
            .foregroundStyle(isEnabled ? GlassColor.textPrimary : GlassColor.textTertiary)
            .padding(.horizontal, GlassTokens.Space.s5)
            .padding(.vertical, GlassTokens.Space.s2)
            .background(
                RoundedRectangle(cornerRadius: 5, style: .continuous)
                    .fill(hovering && isEnabled ? GlassTokens.Color.menuHover.color : .clear)
                    .glassPressedFill()
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(GlassPressStyle())
        .onHover { hovering = $0 }
        .accessibilityAddTraits(checked == true ? .isSelected : [])
    }
}

public struct GlassMenuSeparator: View {
    public init() {}

    public var body: some View {
        Rectangle()
            .fill(GlassTokens.Color.menuSeparator.color)
            .frame(height: 1)
            .padding(.horizontal, GlassTokens.Space.s4)
            .padding(.vertical, GlassTokens.Space.s2)
            .accessibilityHidden(true)
    }
}

/// A row in a menu-like panel whose rows are ordinary buttons (the
/// menu-bar panel): full width, the menu hover fill, and the pressed fill
/// 8% darker. The button's own label is drawn; this style authors no words.
public struct GlassMenuRowStyle: ButtonStyle {
    public init() {}

    public func makeBody(configuration: Configuration) -> some View {
        GlassMenuRowBody(configuration: configuration)
    }
}

private struct GlassMenuRowBody: View {
    let configuration: ButtonStyleConfiguration
    @State private var hovering = false
    @Environment(\.isEnabled) private var isEnabled

    var body: some View {
        configuration.label
            .glassType(GlassTokens.TypeScale.body)
            .foregroundStyle(hovering && isEnabled ? Color.white : (isEnabled ? GlassColor.textPrimary : GlassColor.textTertiary))
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, GlassTokens.Space.s4)
            .padding(.vertical, GlassTokens.Space.s2)
            // The macOS menu selection: blue, with white text.
            .background(
                RoundedRectangle(cornerRadius: 6, style: .continuous)
                    .fill(hovering && isEnabled ? GlassTokens.Color.blue.color : .clear)
                    .glassPressedFill()
            )
            .environment(\.glassPressed, configuration.isPressed && isEnabled)
            .contentShape(Rectangle())
            .onHover { hovering = $0 }
    }
}
