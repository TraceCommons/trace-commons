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
}

/// `Button("Start watching") {}.buttonStyle(GlassButtonStyle(.primary))`.
public struct GlassButtonStyle: ButtonStyle {
    private let kind: GlassButtonKind
    private let small: Bool

    public init(_ kind: GlassButtonKind, small: Bool = false) {
        self.kind = kind
        self.small = small
    }

    public func makeBody(configuration: Configuration) -> some View {
        GlassButtonBody(kind: kind, small: small, configuration: configuration)
    }
}

private struct GlassButtonBody: View {
    let kind: GlassButtonKind
    let small: Bool
    let configuration: ButtonStyleConfiguration
    @Environment(\.isEnabled) private var isEnabled

    var body: some View {
        label
            // Pressed is the fill 8% darker (`GlassPress`), not a fade: a
            // faded consent button reads as disabled. The label keeps its
            // contrast; the fill reads `glassPressed`.
            .environment(\.glassPressed, configuration.isPressed && isEnabled)
            .opacity(isEnabled ? 1 : GlassTokens.Opacity.disabled)
            .contentShape(Capsule())
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
        case .glass:
            configuration.label
                .glassType(GlassTokens.TypeScale.label.weight(.semibold))
                .foregroundStyle(GlassColor.textPrimary)
                .padding(.horizontal, 12)
                .frame(minHeight: GlassTokens.Size.controlLarge)
                .glassSurface(.control)
        case let .submit(done):
            configuration.label
                .glassType(GlassTokens.TypeScale.caption.weight(.bold))
                .foregroundStyle(done ? GlassTokens.Color.statusOn.color : GlassColor.textPrimary)
                .padding(.horizontal, 10)
                .frame(minHeight: GlassTokens.Size.submitPill)
                .glassSurface(.control)
        case .link:
            // No fill to darken: the text takes the press instead.
            configuration.label
                .glassType(GlassTokens.TypeScale.label.weight(.semibold))
                .foregroundStyle(GlassColor.accentText)
                .glassPressedFill()
        }
    }
}

/// A round glass button with an SF Symbol. Icon-only, so `label` names it.
public struct GlassRoundButton: View {
    private let label: String
    private let systemImage: String
    private let small: Bool
    private let action: () -> Void

    public init(_ label: String, systemImage: String, small: Bool = false, action: @escaping () -> Void) {
        self.label = label
        self.systemImage = systemImage
        self.small = small
        self.action = action
    }

    public var body: some View {
        let side = small ? GlassTokens.Size.control : GlassTokens.Size.controlLarge
        Button(action: action) {
            Image(systemName: systemImage)
                .glassGlyph(small ? 11 : 13, weight: .medium)
                .foregroundStyle(GlassColor.textPrimary)
                .frame(width: side, height: side)
                .glassSurface(.control, radius: side / 2)
        }
        .buttonStyle(GlassPressStyle())
        .accessibilityLabel(label)
        .help(label)
    }
}

/// A 30×26 pill with an icon (the graph's period controls).
public struct GlassPillIconButton: View {
    private let label: String
    private let systemImage: String
    private let action: () -> Void

    public init(_ label: String, systemImage: String, action: @escaping () -> Void) {
        self.label = label
        self.systemImage = systemImage
        self.action = action
    }

    public var body: some View {
        Button(action: action) {
            Image(systemName: systemImage)
                .glassGlyph(11, weight: .semibold)
                .foregroundStyle(GlassColor.textPrimary)
                .frame(width: 30, height: GlassTokens.Size.control)
                .glassSurface(.control)
        }
        .buttonStyle(GlassPressStyle())
        .accessibilityLabel(label)
        .help(label)
    }
}

/// A toolbar icon inside a grouped glass capsule (view, graph, map,
/// inspector). `pressed` dims it when its panel is hidden.
public struct GlassToolbarButton: View {
    private let label: String
    private let systemImage: String
    private let pressed: Bool?
    private let action: () -> Void

    public init(_ label: String, systemImage: String, pressed: Bool? = nil, action: @escaping () -> Void) {
        self.label = label
        self.systemImage = systemImage
        self.pressed = pressed
        self.action = action
    }

    public var body: some View {
        Button(action: action) {
            Image(systemName: systemImage)
                .glassGlyph(13)
                .foregroundStyle(pressed == false ? Color(white: 0.49) : Color(white: 0.9))
                .glassPressedFill()
                .frame(width: 28, height: 24)
                .contentShape(Capsule())
        }
        .buttonStyle(GlassPressStyle())
        .accessibilityLabel(label)
        .accessibilityAddTraits(pressed == true ? .isSelected : [])
        .help(label)
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

/// The folder chooser: a folder icon and "…"; the help text carries the
/// full label.
public struct GlassFolderButton: View {
    private let label: String
    private let action: () -> Void

    public init(_ label: String, action: @escaping () -> Void) {
        self.label = label
        self.action = action
    }

    public var body: some View {
        Button(action: action) {
            HStack(spacing: GlassTokens.Space.s2) {
                Image(systemName: "folder")
                Text("…")
            }
        }
        .buttonStyle(GlassButtonStyle(.glass))
        .accessibilityLabel(label)
        .help(label)
    }
}

/// The "⋮" row-menu trigger.
public struct GlassKebab: View {
    private let label: String
    private let open: Bool
    private let action: () -> Void

    /// `label` names the button for VoiceOver and the help tag, from the
    /// core's copy.
    public init(_ label: String, open: Bool = false, action: @escaping () -> Void) {
        self.label = label
        self.open = open
        self.action = action
    }

    public var body: some View {
        Button(action: action) {
            Image(systemName: "ellipsis")
                .rotationEffect(.degrees(90))
                .glassGlyph(12, weight: .bold)
                .foregroundStyle(open ? GlassColor.textPrimary : GlassTokens.Color.statusOff.color)
                .frame(width: 22, height: 24)
                .background(
                    RoundedRectangle(cornerRadius: 6, style: .continuous)
                        .fill(open ? Color.white.opacity(0.14) : .clear)
                )
                .glassPressedFill()
                .contentShape(Rectangle())
        }
        .buttonStyle(GlassPressStyle())
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
            HStack(spacing: GlassTokens.Space.inlineGap) {
                if let dot = current?.dot {
                    GlassStatusDot(dot, size: GlassTokens.Size.dot)
                }
                Text(current?.title ?? placeholder)
                Image(systemName: "chevron.down")
                    .glassGlyph(8, weight: .bold)
                    .foregroundStyle(GlassColor.textTertiary)
            }
            .glassType(GlassTokens.TypeScale.label.weight(.semibold))
            .foregroundStyle(GlassColor.textPrimary)
            .padding(.leading, 10)
            .padding(.trailing, 8)
            .frame(minHeight: GlassTokens.Size.controlLarge)
            .glassSurface(.control)
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
        .accessibilityLabel(label)
        .accessibilityValue(current?.title ?? placeholder)
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

    public init(_ kind: GlassSwitchKind = .standard) {
        self.kind = kind
    }

    /// The track colour when on.
    static func onColor(_ kind: GlassSwitchKind) -> GlassRGBA {
        switch kind {
        case .standard: GlassTokens.Color.toggleOn
        case .settings: GlassTokens.Color.toggleOnSettings
        case .watch: GlassTokens.Color.watchOn
        }
    }

    /// The knob: white, except on the bright watch green, where white is
    /// about 1.8:1 and the knob takes the dark ink instead (over 9:1). Every
    /// knob clears the 3:1 glyph floor against its track (SwitchContrastTests).
    static func knob(_ kind: GlassSwitchKind, isOn: Bool) -> GlassRGBA {
        kind == .watch && isOn ? GlassTokens.Color.textOnStatus : GlassTokens.Color.textOnAccent
    }

    public func makeBody(configuration: Configuration) -> some View {
        let watch = kind == .watch
        let width = watch ? GlassTokens.Size.watchSwitchWidth : GlassTokens.Size.toggleWidth
        let height = watch ? GlassTokens.Size.watchSwitchHeight : GlassTokens.Size.toggleHeight
        let inset: CGFloat = watch ? 2 : 3
        let onColor = Self.onColor(kind)
        return HStack(spacing: GlassTokens.Space.s6) {
            configuration.label
            Button {
                withAnimation(GlassMotion.fast(GlassMotion.systemReducesMotion)) { configuration.isOn.toggle() }
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
public struct GlassCheckboxStyle: ToggleStyle {
    /// A group checkbox whose children differ.
    private let mixed: Bool
    /// What VoiceOver says for the mixed state, from the core's copy. A
    /// native toggle has no mixed value of its own.
    private let mixedValue: String?

    public init(mixed: Bool = false, mixedValue: String? = nil) {
        self.mixed = mixed
        self.mixedValue = mixedValue
    }

    public func makeBody(configuration: Configuration) -> some View {
        Button {
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
        .buttonStyle(GlassPressStyle())
        .accessibilityRepresentation {
            Toggle(isOn: configuration.$isOn) { configuration.label }
                .accessibilityValue(mixed ? (mixedValue ?? "") : "")
        }
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

/// A labelled field: eyebrow label over a dark inset field.
public struct GlassTextField: View {
    private let label: String
    private let prompt: String?
    @Binding private var text: String

    public init(_ label: String, text: Binding<String>, prompt: String? = nil) {
        self.label = label
        self._text = text
        self.prompt = prompt
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            Text(label).glassType(GlassTokens.TypeScale.eyebrow).foregroundStyle(GlassColor.textTertiary)
            TextField(label, text: $text, prompt: prompt.map { Text($0) })
                .textFieldStyle(.plain)
                .glassType(GlassTokens.TypeScale.label.weight(.regular))
                .foregroundStyle(GlassColor.textPrimary)
                .padding(.horizontal, 10)
                .frame(minHeight: GlassTokens.Size.controlLarge)
                .background(
                    RoundedRectangle(cornerRadius: GlassTokens.Radius.control, style: .continuous)
                        .fill(GlassTokens.Color.fieldFill.color)
                )
                .labelsHidden()
        }
    }
}
