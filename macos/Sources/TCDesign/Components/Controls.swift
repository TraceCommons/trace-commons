import SwiftUI

// MARK: - Buttons

/// The button kinds of the glass system.
public enum GlassButtonKind: Sendable, Equatable {
    /// The one purple action on a surface.
    case primary
    /// Dark purple (Download).
    case secondary
    /// Glass pill (Check Now, Review permission).
    case glass
    /// The 24pt pill on tree rows ("Submit · 3"); `done` reads "Sent".
    case submit(done: Bool)
    /// Accent text, no container.
    case link
    /// A glass pill whose label reads in the outside red: the action that
    /// cannot be undone (delete, sign out). Never the default action.
    case destructive
}

/// `Button("Start watching") {}.buttonStyle(GlassButtonStyle(.primary))`.
public struct GlassButtonStyle: ButtonStyle {
    private let kind: GlassButtonKind
    private let small: Bool
    private let selected: Bool

    /// `selected` is a glass button that is the chosen one of a set (#1146's
    /// `aria-pressed`): it reads purple, the CTA fill and white label, and
    /// says selected to assistive tech. Other kinds ignore it.
    public init(_ kind: GlassButtonKind, small: Bool = false, selected: Bool = false) {
        self.kind = kind
        self.small = small
        self.selected = selected
    }

    public func makeBody(configuration: Configuration) -> some View {
        GlassButtonBody(kind: kind, small: small, selected: selected, configuration: configuration)
    }

    /// The fill a kind gets under the pointer, in place of its own fill
    /// (#1146 `.tc-btn--glass:hover`): the glass pill's control hover. The
    /// CTAs, the submit pill and the link have none (the link underlines
    /// instead); a selected glass button keeps its purple.
    static func hoverFill(_ kind: GlassButtonKind, selected: Bool = false) -> GlassRGBA? {
        switch kind {
        case .glass where !selected, .destructive: GlassTokens.Color.controlHover
        default: nil
        }
    }

    /// The destructive pill's label ink: the outside red, lifted for the
    /// control fill.
    static let destructiveInk = GlassTokens.Color.destructiveText

    /// The submit pill's label ink: green once sent, `statusOff` while it
    /// cannot be used (on top of the shared disabled dimming), text otherwise.
    static func submitInk(done: Bool, enabled: Bool) -> GlassRGBA {
        if done { return GlassTokens.Color.statusOnText }
        return enabled ? GlassTokens.Color.textPrimary : GlassTokens.Color.statusOff
    }

    /// Every kind dims by the one shared disabled opacity, except the submit
    /// pill, which #1146 dims less (`.tc-btn--submit:disabled`, 0.55): its
    /// `statusOff` ink already says it cannot be used.
    static func disabledOpacity(_ kind: GlassButtonKind) -> Double {
        if case .submit = kind { return GlassTokens.Opacity.disabledSubmit }
        return GlassTokens.Opacity.disabled
    }
}

private struct GlassButtonBody: View {
    let kind: GlassButtonKind
    let small: Bool
    let selected: Bool
    let configuration: ButtonStyleConfiguration
    @Environment(\.isEnabled) private var isEnabled
    @State private var hovering = false

    var body: some View {
        label
            // Pressed is the fill 8% darker (`GlassPress`), not a fade: a
            // faded consent button reads as disabled. The label keeps its
            // contrast; the fill reads `glassPressed`.
            .environment(\.glassPressed, configuration.isPressed && isEnabled)
            .opacity(isEnabled ? 1 : GlassButtonStyle.disabledOpacity(kind))
            .contentShape(Capsule())
            .onHover { hovering = $0 }
            .accessibilityAddTraits(kind == .glass && selected ? .isSelected : [])
    }

    @ViewBuilder
    private var label: some View {
        switch kind {
        case .primary:
            configuration.label
                .glassType(small ? GlassTokens.TypeScale.label.weight(.bold) : GlassTokens.TypeScale.bodyStrong.weight(.bold))
                .foregroundStyle(GlassTokens.Color.textOnAccent.color)
                .padding(.horizontal, small ? 14 : 16)
                .frame(minHeight: small ? 30 : GlassTokens.Size.cta)
                .background(Capsule().fill(GlassTokens.Gradient.ctaFill.linear).glassPressedFill())
                .glassEdge(isEnabled ? GlassTokens.Shadow.ctaEdge : Array(GlassTokens.Shadow.ctaEdge.prefix(2)), in: Capsule())
        case .secondary:
            configuration.label
                .glassType(small ? GlassTokens.TypeScale.label.weight(.bold) : GlassTokens.TypeScale.bodyStrong.weight(.bold))
                .foregroundStyle(GlassTokens.Color.textOnAccent.color)
                .padding(.horizontal, 14)
                .frame(minHeight: small ? 30 : GlassTokens.Size.cta)
                .background(Capsule().fill(GlassTokens.Gradient.ctaSecondaryFill.linear).glassPressedFill())
                .glassEdge(GlassTokens.Shadow.ctaSecondaryEdge, in: Capsule())
        case .glass where selected:
            // One of a set, chosen: the CTA's purple (#1146 glass.css,
            // `.tc-btn--glass[aria-pressed="true"]`).
            configuration.label
                .glassType(GlassTokens.TypeScale.label.weight(.semibold))
                .foregroundStyle(GlassTokens.Color.textOnAccent.color)
                .padding(.horizontal, 12)
                .frame(minHeight: GlassTokens.Size.controlLarge)
                .background(Capsule().fill(GlassTokens.Gradient.ctaFill.linear).glassPressedFill())
                .glassEdge(GlassTokens.Shadow.ctaEdge, in: Capsule())
        case .glass:
            configuration.label
                .glassType(GlassTokens.TypeScale.label.weight(.semibold))
                .foregroundStyle(GlassColor.textPrimary)
                .padding(.horizontal, 12)
                .frame(minHeight: GlassTokens.Size.controlLarge)
                .glassSurface(.control, hover: GlassButtonStyle.hoverFill(kind, selected: selected))
        case let .submit(done):
            configuration.label
                .glassType(GlassTokens.TypeScale.caption.weight(.bold))
                .foregroundStyle(GlassButtonStyle.submitInk(done: done, enabled: isEnabled).color)
                .padding(.horizontal, 10)
                .frame(minHeight: GlassTokens.Size.submitPill)
                .glassSurface(.control)
        case .destructive:
            configuration.label
                .glassType(GlassTokens.TypeScale.label.weight(.semibold))
                .foregroundStyle(GlassButtonStyle.destructiveInk.color)
                .padding(.horizontal, 12)
                .frame(minHeight: GlassTokens.Size.controlLarge)
                .glassSurface(.control, hover: GlassButtonStyle.hoverFill(kind, selected: selected))
        case .link:
            // No fill to darken: the text takes the press instead, and the
            // pointer underlines it.
            configuration.label
                .glassType(GlassTokens.TypeScale.label.weight(.semibold))
                .foregroundStyle(GlassColor.accentText)
                .underline(hovering && isEnabled)
                .glassPressedFill()
        }
    }
}

/// What an icon-only control draws: an SF Symbol, or one of #1146's own
/// glyphs at its own size.
public enum GlassIcon: Sendable, Equatable {
    case symbol(String)
    case glyph(GlassGlyph)
}

/// The icon inside an icon-only control: an SF Symbol at `symbolSize`, or a
/// glyph at #1146's size.
private struct GlassIconView: View {
    let icon: GlassIcon
    let symbolSize: CGFloat
    var weight: GlassWeight = .regular

    var body: some View {
        switch icon {
        case let .symbol(name): Image(systemName: name).glassGlyph(symbolSize, weight: weight)
        case let .glyph(glyph): GlassGlyphView(glyph)
        }
    }
}

/// A round glass button with an icon. Icon-only, so `label` names it.
/// Under the pointer the control hover replaces its fill (#1146
/// `.tc-btn--round:hover`).
public struct GlassRoundButton: View {
    private let label: String
    private let icon: GlassIcon
    private let small: Bool
    private let action: () -> Void

    public init(_ label: String, systemImage: String, small: Bool = false, action: @escaping () -> Void) {
        self.init(label, icon: .symbol(systemImage), small: small, action: action)
    }

    public init(_ label: String, icon: GlassIcon, small: Bool = false, action: @escaping () -> Void) {
        self.label = label
        self.icon = icon
        self.small = small
        self.action = action
    }

    public var body: some View {
        let side = small ? GlassTokens.Size.control : GlassTokens.Size.controlLarge
        Button(action: action) {
            GlassIconView(icon: icon, symbolSize: small ? 11 : 13, weight: .medium)
                .foregroundStyle(GlassColor.textPrimary)
                .frame(width: side, height: side)
                .glassSurface(.control, radius: side / 2, hover: GlassTokens.Color.controlHover)
        }
        .buttonStyle(GlassPressStyle())
        .accessibilityLabel(label)
        .help(label)
    }
}

/// A 30×26 pill with an icon (the graph's period controls). Under the
/// pointer the control hover replaces its fill (#1146 `.tc-btn--pill-icon`).
public struct GlassPillIconButton: View {
    private let label: String
    private let systemImage: String?
    private let glyph: String?
    private let ink: GlassRGBA?
    private let action: () -> Void

    public init(_ label: String, systemImage: String, ink: GlassRGBA? = nil, action: @escaping () -> Void) {
        self.label = label
        self.systemImage = systemImage
        self.glyph = nil
        self.ink = ink
        self.action = action
    }

    /// A pill whose icon is a text glyph (#1146's graph controls: ‹ − + › ›|).
    /// `label` names it; the glyph is never read.
    public init(_ label: String, glyph: String, ink: GlassRGBA? = nil, action: @escaping () -> Void) {
        self.label = label
        self.systemImage = nil
        self.glyph = glyph
        self.ink = ink
        self.action = action
    }

    @ViewBuilder
    private var icon: some View {
        if let systemImage {
            Image(systemName: systemImage).glassGlyph(11, weight: .semibold)
        } else {
            Text(glyph ?? "").glassGlyph(14, weight: .medium)
        }
    }

    public var body: some View {
        Button(action: action) {
            icon
                .foregroundStyle(ink?.color ?? GlassColor.textPrimary)
                .frame(width: 30, height: GlassTokens.Size.control)
                .glassSurface(.control, hover: GlassTokens.Color.controlHover)
        }
        .buttonStyle(GlassPressStyle())
        .accessibilityLabel(label)
        .help(label)
    }
}

/// A toolbar icon inside a grouped glass capsule (view, graph, map,
/// inspector). `pressed` dims it when its panel is hidden; `expanded` is a
/// button whose menu is open (the View menu), drawn on a solid fill.
public struct GlassToolbarButton: View {
    private let label: String
    private let icon: GlassIcon
    private let pressed: Bool?
    private let expanded: Bool?
    private let action: () -> Void

    public init(
        _ label: String, systemImage: String, pressed: Bool? = nil, expanded: Bool? = nil,
        action: @escaping () -> Void
    ) {
        self.init(label, icon: .symbol(systemImage), pressed: pressed, expanded: expanded, action: action)
    }

    /// `icon` is one of #1146's toolbar glyphs (`.glyph(.viewMenu)`), or a
    /// symbol.
    public init(
        _ label: String, icon: GlassIcon, pressed: Bool? = nil, expanded: Bool? = nil,
        action: @escaping () -> Void
    ) {
        self.label = label
        self.icon = icon
        self.pressed = pressed
        self.expanded = expanded
        self.action = action
    }

    public var body: some View {
        Button(action: action) {
            GlassIconView(icon: icon, symbolSize: 13)
                .foregroundStyle(Self.glyph(pressed: pressed).color)
                .frame(width: 28, height: 24)
                // A glyph has no fill: the press darkens a wash behind it,
                // never the glyph, which keeps its contrast.
                .glassPressedWash(Capsule())
                .background(Capsule().fill(Self.fill(expanded: expanded)?.color ?? .clear))
                // No fill of its own, so the hover is the whole fill
                // (#1146 `.tc-btn--icon:hover`).
                .glassHover(GlassTokens.Color.controlHover, in: Capsule())
                .contentShape(Capsule())
        }
        .buttonStyle(GlassPressStyle())
        .accessibilityLabel(label)
        .accessibilityAddTraits(pressed == true ? .isSelected : [])
        .help(label)
    }

    /// The glyph's ink, as #1146 draws it: `toolbarGlyph`, dimmed to
    /// `toolbarGlyphHidden` while its pane is hidden (owner ruling,
    /// 2026-10-07: #1146 wins for theming). Dimmed, it still clears the
    /// 3:1 glyph floor on the control fill (ToolbarGlyphContrastTests).
    static func glyph(pressed: Bool?) -> GlassRGBA {
        pressed == false ? GlassTokens.Color.toolbarGlyphHidden : GlassTokens.Color.toolbarGlyph
    }

    /// The fill behind the glyph: `toolbarExpanded` while its menu is open
    /// (#1146 `.tc-btn--icon[aria-expanded="true"]`), none otherwise.
    static func fill(expanded: Bool?) -> GlassRGBA? {
        expanded == true ? GlassTokens.Color.toolbarExpanded : nil
    }
}

/// The capsule that groups toolbar buttons.
public struct GlassToolbarGroup<Content: View>: View {
    private let content: Content

    public init(@ViewBuilder content: () -> Content) {
        self.content = content()
    }

    public var body: some View {
        HStack(spacing: 2) { content }
            .padding(2)
            .glassSurface(.control)
    }
}

/// The folder chooser (#1146 `FolderButton`): #1146's 13pt folder outline
/// and "…" on a control pill, 10pt sides, bold; the help text carries the
/// full label. Under the pointer the control hover replaces its fill.
public struct GlassFolderButton: View {
    private let label: String
    private let action: () -> Void

    public init(_ label: String, action: @escaping () -> Void) {
        self.label = label
        self.action = action
    }

    /// The pill's sides and the label's type (#1146 `.tc-btn--folder`).
    static let horizontalPadding: CGFloat = 10
    static let type = GlassTokens.TypeScale.label.weight(.bold)

    public var body: some View {
        Button(action: action) {
            HStack(spacing: GlassTokens.Space.s2) {
                GlassGlyphView(.folder)
                Text("…").accessibilityHidden(true)
            }
            .glassType(Self.type)
            .foregroundStyle(GlassColor.textPrimary)
            .padding(.horizontal, Self.horizontalPadding)
            .frame(minHeight: GlassTokens.Size.controlLarge)
            .glassSurface(.control, hover: GlassTokens.Color.controlHover)
        }
        .buttonStyle(GlassPressStyle())
        .accessibilityLabel(label)
        .help(label)
    }
}

/// The "⋮" row-menu trigger.
public struct GlassKebab: View {
    private let label: String
    private let open: Bool
    private let action: () -> Void
    @State private var hovering = false
    @Environment(\.isEnabled) private var isEnabled

    /// `label` names the button for VoiceOver and the help tag, from the
    /// core's copy.
    public init(_ label: String, open: Bool = false, action: @escaping () -> Void) {
        self.label = label
        self.open = open
        self.action = action
    }

    /// Open or under the pointer, the kebab lights: the glyph in the text
    /// colour on a faint fill (#1146 `.tc-btn--kebab:hover`); otherwise a
    /// `statusOff` glyph with no fill.
    static func lit(open: Bool, hovering: Bool) -> Bool {
        open || hovering
    }

    public var body: some View {
        let lit = Self.lit(open: open, hovering: hovering && isEnabled)
        Button(action: action) {
            // #1146's three 1.6pt dots in a 4×14 box.
            GlassGlyphView(.kebab)
                .foregroundStyle(lit ? GlassColor.textPrimary : GlassTokens.Color.statusOff.color)
                .frame(width: 22, height: 24)
                .background(
                    RoundedRectangle(cornerRadius: 6, style: .continuous)
                        .fill(lit ? GlassColor.ink(0.14) : .clear)
                )
                .glassPressedFill()
                .contentShape(Rectangle())
        }
        .buttonStyle(GlassPressStyle())
        .onHover { hovering = $0 }
        .accessibilityLabel(label)
        .help(label)
    }
}

/// "›" that turns 90° when open, then a label. No container.
public struct GlassExpander: View {
    private let title: String
    @Binding private var isOpen: Bool

    public init(_ title: String, isOpen: Binding<Bool>) {
        self.title = title
        self._isOpen = isOpen
    }

    /// #1146's `.tc-expander` padding: 6 above, 2 on the other sides.
    static let padding = EdgeInsets(top: 6, leading: 2, bottom: 2, trailing: 2)

    public var body: some View {
        Button {
            withAnimation(GlassMotion.fast(GlassMotion.systemReducesMotion)) { isOpen.toggle() }
        } label: {
            HStack(spacing: GlassTokens.Space.s4) {
                Text("›")
                    .glassGlyph(14)
                    .foregroundStyle(GlassColor.textTertiary)
                    .rotationEffect(.degrees(isOpen ? 90 : 0))
                    .glassPressedFill()
                    .frame(width: 12)
                Text(title)
                    .glassType(GlassTokens.TypeScale.label.weight(.semibold))
                    .foregroundStyle(GlassColor.textPrimary)
                Spacer(minLength: 0)
            }
            .padding(Self.padding)
            .contentShape(Rectangle())
        }
        .buttonStyle(GlassPressStyle())
        // A native disclosure for assistive tech, so the system says
        // expanded or collapsed in the person's language.
        .accessibilityRepresentation {
            DisclosureGroup(isExpanded: $isOpen) { EmptyView() } label: { Text(title) }
        }
    }
}

// MARK: - Picker

/// An option for `GlassPicker`.
public struct GlassPickerOption<Value: Hashable>: Identifiable {
    public let value: Value
    public let title: String
    public let dot: GlassStatus?
    public var id: Value { value }

    public init(_ title: String, value: Value, dot: GlassStatus? = nil) {
        self.title = title
        self.value = value
        self.dot = dot
    }
}

/// A native menu dressed as a glass pill: dot, label, chevron. With no
/// value it shows the caller's `placeholder` and no dot.
public struct GlassPicker<Value: Hashable>: View {
    private let label: String
    @Binding private var selection: Value?
    private let options: [GlassPickerOption<Value>]
    private let placeholder: String

    /// `placeholder` is shown while nothing is chosen, from the core's copy.
    public init(_ label: String, selection: Binding<Value?>, options: [GlassPickerOption<Value>], placeholder: String) {
        self.label = label
        self._selection = selection
        self.options = options
        self.placeholder = placeholder
    }

    private var current: GlassPickerOption<Value>? {
        options.first { $0.value == selection }
    }

    public var body: some View {
        Menu {
            ForEach(options) { option in
                Button(option.title) { selection = option.value }
            }
        } label: {
            GlassPickerPill(title: current?.title ?? placeholder, dot: current?.dot)
        }
        // A button-style menu draws the label as given (the borderless
        // style keeps only its text, tinted, and drops the pill), and
        // GlassPressStyle darkens the pill's fill while it is pressed.
        .menuStyle(.button)
        .buttonStyle(GlassPressStyle())
        .menuIndicator(.hidden)
        .fixedSize()
        .accessibilityLabel(label)
        .accessibilityValue(current?.title ?? placeholder)
    }
}

/// The picker's pill: dot, label, #1146's 10pt chevron, on the control
/// tier, with the control hover in place of its fill. `invalid` adds the
/// outside-red ring (`GlassSelect`). `trailing` is the inset after the
/// chevron: 8 on the picker (#1146 `.tc-picker`), 10 on the select, whose
/// chevron #1146 pins 10 from the pill's end (`.tc-select`).
struct GlassPickerPill: View {
    let title: String
    let dot: GlassStatus?
    var invalid = false
    var trailing: CGFloat = 8

    var body: some View {
        HStack(spacing: GlassTokens.Space.inlineGap) {
            if let dot {
                GlassStatusDot(dot, size: GlassTokens.Size.dot)
            }
            Text(title)
            GlassGlyphView(.chevronDown)
                .foregroundStyle(GlassColor.textTertiary)
        }
        .glassType(GlassTokens.TypeScale.label.weight(.semibold))
        .foregroundStyle(GlassColor.textPrimary)
        .padding(.leading, 10)
        .padding(.trailing, trailing)
        .frame(minHeight: GlassTokens.Size.controlLarge)
        .overlay {
            if let ring = GlassTextField.ring(invalid: invalid) {
                Capsule().strokeBorder(ring.color, lineWidth: 1)
            }
        }
        .glassSurface(.control, hover: GlassTokens.Color.controlHover)
    }
}

// MARK: - Toggles and checkboxes

/// The switch variants: the 40×24 toggle (blue on, purple in Settings) and
/// the 38×22 watch switch on tree rows (green when watching).
public enum GlassSwitchKind: Sendable, Equatable {
    case standard, settings, watch
}

/// `Toggle("Start at login", isOn: $on).toggleStyle(GlassToggleStyle(.settings))`.
public struct GlassToggleStyle: ToggleStyle {
    private let kind: GlassSwitchKind
    private let showsLabel: Bool

    /// `showsLabel: false` draws the switch alone; the label still names it
    /// for VoiceOver. `.labelsHidden()` does not reach a custom style before
    /// macOS 15, so this is the way to hide it.
    public init(_ kind: GlassSwitchKind = .standard, showsLabel: Bool = true) {
        self.kind = kind
        self.showsLabel = showsLabel
    }

    /// The track colour when on. The watch switch takes the Settings
    /// purple, as every switch matches (owner, 2026-10-08): its knob lost
    /// the dark ring that let a white knob read on the bright watch green,
    /// and white on that green is about 1.8:1, under the 3:1 floor.
    static func onColor(_ kind: GlassSwitchKind) -> GlassRGBA {
        switch kind {
        case .standard: GlassTokens.Color.toggleOn
        case .settings, .watch: GlassTokens.Color.toggleOnSettings
        }
    }

    /// The knob: plain white on every track, with no ring (owner,
    /// 2026-10-08), as #1146 draws it.
    static func knob(_ kind: GlassSwitchKind, isOn: Bool) -> GlassRGBA {
        GlassTokens.Color.textOnAccent
    }

    /// How long the knob takes to slide: #1146's `--tc-dur` (220ms) for a
    /// toggle, 150ms for the watch switch (`glass.css` .tc-watch).
    static func duration(_ kind: GlassSwitchKind) -> Double {
        kind == .watch ? GlassTokens.Motion.fast : GlassTokens.Motion.standard
    }

    public func makeBody(configuration: Configuration) -> some View {
        let watch = kind == .watch
        let width = watch ? GlassTokens.Size.watchSwitchWidth : GlassTokens.Size.toggleWidth
        let height = watch ? GlassTokens.Size.watchSwitchHeight : GlassTokens.Size.toggleHeight
        let inset: CGFloat = watch ? 2 : 3
        let onColor = Self.onColor(kind)
        return HStack(spacing: showsLabel ? GlassTokens.Space.s6 : 0) {
            // Hidden or not, the native toggle below carries the label to
            // VoiceOver.
            if showsLabel {
                configuration.label
            }
            Button {
                withAnimation(GlassMotion.systemReducesMotion ? nil : .easeOut(duration: Self.duration(kind))) {
                    configuration.isOn.toggle()
                }
            } label: {
                ZStack(alignment: configuration.isOn ? .trailing : .leading) {
                    Capsule()
                        .fill(configuration.isOn ? onColor.color : GlassTokens.Color.toggleOff.color)
                        .glassPressedFill()
                    Circle()
                        .fill(Self.knob(kind, isOn: configuration.isOn).color)
                        .frame(width: 18, height: 18)
                        .padding(inset)
                }
                .frame(width: width, height: height)
            }
            .buttonStyle(GlassPressStyle())
        }
        // A native toggle for assistive tech, so the system states its
        // value in the person's language.
        .accessibilityRepresentation {
            Toggle(isOn: configuration.$isOn) { configuration.label }
        }
    }
}

/// The 15pt rounded checkbox: purple gradient and a white check when on, a
/// dark well when off. Use as a `ToggleStyle`.
///
/// A group checkbox is a native `Toggle(sources:isOn:)`: the style reads its
/// mixed state from `configuration.isMixed`, so callers pass no flag and no
/// wording, and VoiceOver says the system's own "mixed".
public struct GlassCheckboxStyle: ToggleStyle {
    public init() {}

    public func makeBody(configuration: Configuration) -> some View {
        let mixed = configuration.isMixed
        return Button {
            configuration.isOn.toggle()
        } label: {
            HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s4) {
                GlassCheckMark(checked: configuration.isOn, mixed: mixed)
                    .glassPressedFill()
                configuration.label
                    .glassType(GlassTokens.TypeScale.label.weight(.regular))
                    .foregroundStyle(GlassColor.textPrimary)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(GlassPressStyle(disabledOpacity: GlassTokens.Opacity.disabledCheck))
        .accessibilityRepresentation {
            // A native toggle, so the system states the value -- checked,
            // unchecked or mixed -- in the person's language. A mixed box is
            // represented by two sources that disagree; both write through to
            // the caller's binding, which sets every child.
            Toggle(sources: Self.sources(configuration.$isOn, mixed: mixed), isOn: \.self) {
                configuration.label
            }
        }
    }

    /// The representation's sources: the caller's binding alone, or with its
    /// negation beside it when the group is mixed.
    static func sources(_ isOn: Binding<Bool>, mixed: Bool) -> [Binding<Bool>] {
        guard mixed else { return [isOn] }
        return [isOn, Binding(get: { !isOn.wrappedValue }, set: { isOn.wrappedValue = $0 })]
    }
}

/// The checkbox's look for a state the person reads but does not change
/// here (a folder is set, a scan is configured). Hidden from assistive
/// tech: the row's text must say the state.
public struct GlassCheckMark: View {
    private let checked: Bool
    private let mixed: Bool

    public init(checked: Bool, mixed: Bool = false) {
        self.checked = checked
        self.mixed = mixed
    }

    public var body: some View {
        let shape = RoundedRectangle(cornerRadius: GlassTokens.Radius.checkbox, style: .continuous)
        ZStack {
            if checked || mixed {
                shape.fill(GlassTokens.Gradient.checkboxOnFill.linear)
            } else {
                shape.fill(GlassTokens.Color.wellFill.color)
            }
            if mixed {
                Image(systemName: "minus").glassGlyph(8, weight: .heavy).foregroundStyle(.white)
            } else if checked {
                Image(systemName: "checkmark").glassGlyph(8, weight: .heavy).foregroundStyle(.white)
            }
        }
        .frame(width: GlassTokens.Size.checkbox, height: GlassTokens.Size.checkbox)
        .glassEdge(GlassTokens.Shadow.checkboxEdge, in: shape)
        .accessibilityHidden(true)
    }
}

/// A labelled field: eyebrow label over a dark inset field (#1146
/// `.tc-input`): the field fill with the well's inner edge, the prompt in
/// the placeholder ink, and the shared disabled dimming. Focus is the system's
/// ring.
///
/// `secure` hides what is typed (a passphrase). `invalid` draws a 1pt ring
/// in the outside red inside the field; say why in words beside it, from
/// the core's copy. `showsLabel: false` drops the eyebrow for a field whose
/// label is drawn elsewhere; `label` still names it for VoiceOver.
public struct GlassTextField: View {
    private let label: String
    private let prompt: String?
    private let secure: Bool
    private let invalid: Bool
    private let showsLabel: Bool
    private let focus: FocusState<Bool>.Binding?
    @Binding private var text: String
    @Environment(\.isEnabled) private var isEnabled

    /// `focus` binds the field's keyboard focus, for a caller that moves
    /// focus into it (a search field taking Command-F).
    public init(
        _ label: String, text: Binding<String>, prompt: String? = nil, secure: Bool = false,
        invalid: Bool = false, showsLabel: Bool = true, focus: FocusState<Bool>.Binding? = nil
    ) {
        self.label = label
        self._text = text
        self.prompt = prompt
        self.secure = secure
        self.invalid = invalid
        self.showsLabel = showsLabel
        self.focus = focus
    }

    /// The ring an invalid entry draws inside the field, or none.
    static func ring(invalid: Bool) -> GlassRGBA? {
        invalid ? GlassTokens.Color.statusOutside : nil
    }

    /// The prompt's ink: #1146's white placeholder, raised from 30% so it
    /// clears 4.5:1 on the field fill (owner ruling, 2026-10-07: hold the
    /// WCAG floors; FormControlTests).
    static let promptInk = GlassTokens.Color.placeholder

    public var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            if showsLabel {
                Text(label).glassType(GlassTokens.TypeScale.eyebrow).foregroundStyle(GlassColor.textTertiary)
            }
            focused(field)
                .textFieldStyle(.plain)
                .glassType(GlassTokens.TypeScale.label.weight(.regular))
                .foregroundStyle(GlassColor.textPrimary)
                .padding(.horizontal, 10)
                .frame(minHeight: GlassTokens.Size.controlLarge)
                .glassFieldWell(invalid: invalid)
                .labelsHidden()
                .opacity(isEnabled ? 1 : GlassTokens.Opacity.disabled)
        }
    }

    @ViewBuilder
    private func focused(_ field: some View) -> some View {
        if let focus {
            field.focused(focus)
        } else {
            field
        }
    }

    @ViewBuilder
    private var field: some View {
        let shownPrompt = prompt.map { Text($0).foregroundStyle(Self.promptInk.color) }
        if secure {
            SecureField(label, text: $text, prompt: shownPrompt)
        } else {
            TextField(label, text: $text, prompt: shownPrompt)
        }
    }
}

public extension View {
    /// The field well under a text field or text area: the field fill, the
    /// well's inner edge, and the invalid ring when there is one.
    func glassFieldWell(invalid: Bool) -> some View {
        let shape = RoundedRectangle(cornerRadius: GlassTokens.Radius.control, style: .continuous)
        return background(shape.fill(GlassTokens.Color.fieldFill.color))
            .glassEdge(GlassTokens.Shadow.wellEdge, in: shape)
            .overlay {
                if let ring = GlassTextField.ring(invalid: invalid) {
                    shape.strokeBorder(ring.color, lineWidth: 1)
                }
            }
    }
}
