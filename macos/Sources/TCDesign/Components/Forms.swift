import SwiftUI

// MARK: - Select

/// A choice that always has a value, dressed as the glass picker pill
/// (#1146 `Select`): the replacement for a stock `Picker`. The menu is a
/// native inline picker, so the chosen option carries the system's check
/// and VoiceOver reads a native menu. `GlassPicker` is the variant with no
/// value yet and a placeholder.
///
/// Every word is the caller's: `label` names it for VoiceOver, the options
/// title themselves. `invalid` rings the pill in the outside red.
public struct GlassSelect<Value: Hashable>: View {
    private let label: String
    @Binding private var selection: Value
    private let options: [GlassPickerOption<Value>]
    private let invalid: Bool

    public init(_ label: String, selection: Binding<Value>, options: [GlassPickerOption<Value>], invalid: Bool = false) {
        self.label = label
        self._selection = selection
        self.options = options
        self.invalid = invalid
    }

    /// The option the pill shows: the selection's, or none when the caller
    /// passed a value no option has.
    static func current(_ selection: Value, in options: [GlassPickerOption<Value>]) -> GlassPickerOption<Value>? {
        options.first { $0.value == selection }
    }

    public var body: some View {
        let current = Self.current(selection, in: options)
        Menu {
            Picker(label, selection: $selection) {
                ForEach(options) { option in
                    Text(option.title).tag(option.value)
                }
            }
            .pickerStyle(.inline)
            .labelsHidden()
        } label: {
            GlassPickerPill(title: current?.title ?? "", dot: current?.dot, invalid: invalid)
        }
        .menuStyle(.button)
        .buttonStyle(GlassPressStyle())
        .menuIndicator(.hidden)
        .fixedSize()
        .accessibilityLabel(label)
        .accessibilityValue(current?.title ?? "")
    }
}

// MARK: - Radio

/// One choice of a `GlassRadioGroup`.
public struct GlassRadioOption<Value: Hashable>: Identifiable {
    public let value: Value
    public let title: String
    public let sub: String?
    public let isEnabled: Bool
    public var id: Value { value }

    public init(_ title: String, value: Value, sub: String? = nil, isEnabled: Bool = true) {
        self.title = title
        self.value = value
        self.sub = sub
        self.isEnabled = isEnabled
    }
}

/// The radio's look: a 15pt round well, the checkbox's purple with a white
/// dot when chosen (#1146 `.tc-radio`). Hidden from assistive tech; the
/// group speaks for it.
public struct GlassRadio: View {
    private let checked: Bool

    public init(checked: Bool) {
        self.checked = checked
    }

    public var body: some View {
        ZStack {
            if checked {
                Circle().fill(GlassTokens.Gradient.checkboxOnFill.linear)
                Circle().fill(GlassTokens.Color.textOnAccent.color)
                    .frame(width: GlassTokens.Size.radioDot, height: GlassTokens.Size.radioDot)
            } else {
                Circle().fill(GlassTokens.Color.wellFill.color)
            }
        }
        .frame(width: GlassTokens.Size.radio, height: GlassTokens.Size.radio)
        .glassEdge(GlassTokens.Shadow.checkboxEdge, in: Circle())
        .accessibilityHidden(true)
    }
}

/// One choice of several, as a column of radios with their words (#1146
/// `RadioGroup`). The group is one keyboard stop and the arrow keys move
/// the choice between the enabled options, as a native radio group does.
/// To assistive tech it is a native radio-group picker, so the system says
/// the choice and the count in the person's language.
public struct GlassRadioGroup<Value: Hashable>: View {
    private let label: String
    @Binding private var selection: Value
    private let options: [GlassRadioOption<Value>]

    public init(_ label: String, selection: Binding<Value>, options: [GlassRadioOption<Value>]) {
        self.label = label
        self._selection = selection
        self.options = options
    }

    /// The value an arrow key moves to: the next enabled option in
    /// `step`'s direction, wrapping, or the selection when none is enabled.
    static func moved(_ selection: Value, by step: Int, in options: [GlassRadioOption<Value>]) -> Value {
        let enabled = options.filter(\.isEnabled)
        guard !enabled.isEmpty else { return selection }
        guard let at = enabled.firstIndex(where: { $0.value == selection }) else {
            return (step < 0 ? enabled.last : enabled.first)!.value
        }
        return enabled[(at + step + enabled.count) % enabled.count].value
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            ForEach(options) { option in
                Button {
                    selection = option.value
                } label: {
                    HStack(alignment: .top, spacing: GlassTokens.Space.s4) {
                        GlassRadio(checked: option.value == selection)
                            .glassPressedFill()
                            .padding(.top, 1)
                        VStack(alignment: .leading, spacing: 1) {
                            Text(option.title)
                                .glassType(GlassTokens.TypeScale.label.weight(.regular))
                                .foregroundStyle(GlassColor.textPrimary)
                            if let sub = option.sub {
                                Text(sub)
                                    .glassType(GlassTokens.TypeScale.caption)
                                    .foregroundStyle(GlassColor.textTertiary)
                            }
                        }
                        .fixedSize(horizontal: false, vertical: true)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(GlassPressStyle())
                .focusable(false)
                .disabled(!option.isEnabled)
            }
        }
        .focusable()
        .onMoveCommand { direction in
            switch direction {
            case .down, .right: selection = Self.moved(selection, by: 1, in: options)
            case .up, .left: selection = Self.moved(selection, by: -1, in: options)
            @unknown default: break
            }
        }
        .accessibilityRepresentation {
            Picker(label, selection: $selection) {
                ForEach(options) { option in
                    Text(option.title).tag(option.value)
                }
            }
            .pickerStyle(.radioGroup)
        }
    }
}

// MARK: - Check row

/// A checkbox with its sentence beside it, wrapping under its own width
/// (#1146 `CheckRow`): a consent or an option written out in full. The
/// sentence names the checkbox for VoiceOver.
public struct GlassCheckRow: View {
    private let title: String
    @Binding private var isOn: Bool

    public init(_ title: String, isOn: Binding<Bool>) {
        self.title = title
        self._isOn = isOn
    }

    public var body: some View {
        Toggle(isOn: $isOn) {
            Text(title)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .toggleStyle(GlassCheckboxStyle())
    }
}

// MARK: - Text area

/// A multi-line field: the text field's well, at least
/// `size.textAreaMinHeight` tall, growing with its text (#1146
/// `textarea.tc-input`). The replacement for a stock `TextEditor`.
///
/// `prompt` shows while it is empty, in tertiary text. `invalid` rings it in
/// the outside red. `showsLabel: false` drops the eyebrow; `label` still
/// names it for VoiceOver.
public struct GlassTextArea: View {
    private let label: String
    private let prompt: String?
    private let invalid: Bool
    private let showsLabel: Bool
    @Binding private var text: String
    @Environment(\.isEnabled) private var isEnabled

    public init(
        _ label: String, text: Binding<String>, prompt: String? = nil, invalid: Bool = false,
        showsLabel: Bool = true
    ) {
        self.label = label
        self._text = text
        self.prompt = prompt
        self.invalid = invalid
        self.showsLabel = showsLabel
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            if showsLabel {
                Text(label).glassType(GlassTokens.TypeScale.eyebrow).foregroundStyle(GlassColor.textTertiary)
            }
            TextEditor(text: $text)
                .scrollContentBackground(.hidden)
                .glassType(GlassTokens.TypeScale.label.weight(.regular))
                .foregroundStyle(GlassColor.textPrimary)
                .overlay(alignment: .topLeading) {
                    if text.isEmpty, let prompt {
                        // The editor insets its text by the line fragment
                        // padding; the prompt sits where the text will.
                        Text(prompt)
                            .glassType(GlassTokens.TypeScale.label.weight(.regular))
                            .foregroundStyle(GlassColor.textTertiary)
                            .padding(.leading, 5)
                            .allowsHitTesting(false)
                            .accessibilityHidden(true)
                    }
                }
                .padding(.vertical, 8)
                .padding(.horizontal, 5)
                .frame(minHeight: GlassTokens.Size.textAreaMinHeight, alignment: .topLeading)
                .glassFieldWell(invalid: invalid)
                .accessibilityLabel(label)
                .opacity(isEnabled ? 1 : GlassTokens.Opacity.disabled)
        }
    }
}
