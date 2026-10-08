import SwiftUI

// MARK: - Actions

/// One button in a `GlassModal`'s footer. Every title is the caller's, from
/// the core's copy.
public struct GlassModalAction: Identifiable {
    public enum Role: Sendable, Equatable {
        /// Leaves without doing anything: the left-most button, and Escape.
        case cancel
        /// An ordinary action.
        case standard
        /// The action that cannot be undone: the right-most button, in the
        /// destructive kind, and never the default.
        case destructive
    }

    public let id: String
    public let title: String
    public let role: Role
    public let isDefault: Bool
    public let isEnabled: Bool
    /// Drawn as the primary button without being the default: an action
    /// that must never be one Return away (an irreversible send).
    public let isProminent: Bool
    public let action: () -> Void

    /// `isDefault` makes this the action Return takes, drawn as the primary
    /// button; a destructive action is never the default, whatever it asks.
    /// `isProminent` draws a standard action as the primary with no key.
    /// `id` defaults to the title.
    public init(
        _ title: String, role: Role = .standard, isDefault: Bool = false, isEnabled: Bool = true,
        isProminent: Bool = false, id: String? = nil, action: @escaping () -> Void
    ) {
        self.id = id ?? title
        self.title = title
        self.role = role
        self.isDefault = isDefault
        self.isEnabled = isEnabled
        self.isProminent = isProminent
        self.action = action
    }

    /// A cancel action.
    public static func cancel(_ title: String, action: @escaping () -> Void) -> GlassModalAction {
        GlassModalAction(title, role: .cancel, action: action)
    }

    /// A destructive action.
    public static func destructive(_ title: String, isEnabled: Bool = true, action: @escaping () -> Void) -> GlassModalAction {
        GlassModalAction(title, role: .destructive, isEnabled: isEnabled, action: action)
    }

    /// The footer's order, left to right: cancel, then the others as given,
    /// then the destructive action right-most.
    static func ordered(_ actions: [GlassModalAction]) -> [GlassModalAction] {
        actions.filter { $0.role == .cancel } + actions.filter { $0.role == .standard }
            + actions.filter { $0.role == .destructive }
    }

    /// The action Return takes: the first non-destructive action marked
    /// default. Never a destructive one, and never a cancel.
    static func defaultAction(in actions: [GlassModalAction]) -> GlassModalAction? {
        actions.first { $0.isDefault && $0.role == .standard }
    }

    /// The button kind an action draws in.
    static func kind(_ action: GlassModalAction, isDefault: Bool) -> GlassButtonKind {
        switch action.role {
        case .destructive: .destructive
        case .cancel: .glass
        case .standard: isDefault || action.isProminent ? .primary : .glass
        }
    }
}

/// A modal's width: `size.modalWidth` (780), or `size.modalNarrowWidth`
/// (450) for a confirmation or a short form.
public enum GlassModalWidth: Sendable, Equatable {
    case regular, narrow

    public var points: CGFloat {
        switch self {
        case .regular: GlassTokens.Size.modalWidth
        case .narrow: GlassTokens.Size.modalNarrowWidth
        }
    }
}

// MARK: - Modal

/// A modal over the whole window (#1146 `Modal`): title, an optional
/// subtitle and close button, the caller's body, and a footer of actions.
/// The replacement for a stock `.sheet`. Present it with `.glassModal`, and
/// put `.glassModalHost()` at the window's root so it covers the window.
///
/// The modal is one more pane on a solid backing (no glass on glass), over
/// a blurred, dimmed scrim. What it covers cannot be clicked, focused or
/// read by VoiceOver while it is up; the modal itself is a modal container
/// to assistive tech. Only the topmost modal answers the keyboard: Escape
/// calls `onCancel`, Return takes the default action (never a destructive
/// one). A click on the scrim calls `onCancel` too, as #1146's does.
///
/// Every word is the caller's: `title`, `subtitle`, `closeLabel` (which
/// names the close button) and the action titles. An empty `closeLabel`
/// takes the host's (`glassModalCloseLabel`), so every modal under a host
/// that names one has #1146's close button; with neither, none is drawn.
public struct GlassModal<Content: View>: View {
    private let title: String
    private let subtitle: String?
    private let width: GlassModalWidth
    private let closeLabel: String
    private let actions: [GlassModalAction]
    private let onCancel: () -> Void
    private let content: Content
    @Environment(\.glassModalIsTopmost) private var isTopmost
    @Environment(\.glassModalCloseLabel) private var hostCloseLabel

    public init(
        title: String, subtitle: String? = nil, width: GlassModalWidth = .regular, closeLabel: String = "",
        actions: [GlassModalAction] = [], onCancel: @escaping () -> Void,
        @ViewBuilder content: () -> Content
    ) {
        self.title = title
        self.subtitle = subtitle
        self.width = width
        self.closeLabel = closeLabel
        self.actions = actions
        self.onCancel = onCancel
        self.content = content()
    }

    public var body: some View {
        let shape = RoundedRectangle(cornerRadius: GlassTokens.Radius.pane, style: .continuous)
        let defaultAction = GlassModalAction.defaultAction(in: actions)
        let closeLabel = Self.closeLabel(own: closeLabel, host: hostCloseLabel)
        VStack(alignment: .leading, spacing: 0) {
            // The title block and the close button centre on each other
            // (#1146 `.tc-modal__header`, `align-items: center`).
            HStack(alignment: .center, spacing: GlassTokens.Space.s6) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                    Text(title)
                        .glassType(GlassTokens.TypeScale.title.weight(.bold))
                        .foregroundStyle(GlassColor.textPrimary)
                        .accessibilityAddTraits(.isHeader)
                    if let subtitle {
                        Text(subtitle)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textTertiary)
                    }
                }
                .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 0)
                if !closeLabel.isEmpty {
                    GlassRoundButton(closeLabel, systemImage: "xmark", small: true, action: onCancel)
                }
            }
            .padding(.top, 14)
            .padding(.bottom, 10)
            .padding(.leading, 18)
            .padding(.trailing, 16)
            .overlay(alignment: .bottom) { GlassModalRule() }

            content
                .frame(maxWidth: .infinity, alignment: .leading)

            if !actions.isEmpty {
                HStack(spacing: GlassTokens.Space.s4) {
                    Spacer(minLength: 0)
                    ForEach(GlassModalAction.ordered(actions)) { action in
                        let isDefault = action.id == defaultAction?.id
                        Button(action.title, action: action.action)
                            .buttonStyle(GlassButtonStyle(GlassModalAction.kind(action, isDefault: isDefault), small: true))
                            .disabled(!action.isEnabled)
                            .keyboardShortcut(Self.shortcut(for: action, isDefault: isDefault, isTopmost: isTopmost))
                    }
                }
                .padding(.top, 10)
                .padding(.bottom, 14)
                .padding(.leading, 18)
                .padding(.trailing, 16)
                .overlay(alignment: .top) { GlassModalRule() }
            }
        }
        .frame(maxWidth: width.points)
        .background {
            ZStack {
                shape.fill(GlassTokens.Color.paneBase.color)
                shape.fill(GlassTokens.Gradient.paneFill.linear)
            }
        }
        .clipShape(shape)
        .glassEdge(GlassTokens.Shadow.paneEdge + GlassTokens.Shadow.modal, in: shape)
        .focusSection()
        .onExitCommand(perform: isTopmost ? onCancel : nil)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(title)
        .accessibilityAddTraits(.isModal)
        // The scrim around this modal closes it as Escape does (#1146
        // `Modal`: `onClick={onClose}` on the scrim).
        .preference(key: GlassModalScrimAction.self, value: GlassModalScrimAction.Action(run: onCancel))
    }

    /// The close button's name: the modal's own, else its host's; empty
    /// draws none.
    static func closeLabel(own: String, host: String) -> String {
        own.isEmpty ? host : own
    }

    /// The key an action answers: Return for the default action and Escape
    /// for the cancel action, on the topmost modal only; none otherwise.
    static func shortcut(for action: GlassModalAction, isDefault: Bool, isTopmost: Bool) -> KeyboardShortcut? {
        guard isTopmost else { return nil }
        if isDefault && action.role == .standard { return .defaultAction }
        if action.role == .cancel { return .cancelAction }
        return nil
    }
}

/// A confirmation (#1146's viewport modal, `responsive-overlay.tsx`): the
/// regular modal raised over the whole window, an optional short
/// `subtitle` under its title, its `message` (a disclosure, a consequence)
/// in the scrolling body at body size in secondary ink, as #1146 keeps it
/// in the overlay's children, and its actions the footer. The replacement
/// for a stock `.alert` or `.confirmationDialog`.
public struct GlassConfirmation: View {
    private let title: String
    private let subtitle: String?
    private let message: String?
    private let actions: [GlassModalAction]
    private let onCancel: () -> Void

    public init(
        title: String, subtitle: String? = nil, message: String? = nil, actions: [GlassModalAction],
        onCancel: @escaping () -> Void
    ) {
        self.title = title
        self.subtitle = subtitle
        self.message = message
        self.actions = actions
        self.onCancel = onCancel
    }

    /// #1146's width: the regular modal, never the narrow one.
    public static let width: GlassModalWidth = .regular

    /// The paragraphs the body draws: the message split at its blank
    /// lines, empty ones dropped.
    static func paragraphs(_ message: String?) -> [String] {
        (message ?? "").components(separatedBy: "\n\n")
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
    }

    public var body: some View {
        GlassModal(title: title, subtitle: subtitle, width: Self.width, actions: actions, onCancel: onCancel) {
            let paragraphs = Self.paragraphs(message)
            if paragraphs.isEmpty {
                // #1146's body (`px-[18px] py-3`), empty.
                Color.clear
                    .frame(height: 0)
                    .padding(.vertical, GlassTokens.Space.s6)
                    .accessibilityHidden(true)
            } else {
                GlassModalBody {
                    ForEach(Array(paragraphs.enumerated()), id: \.offset) { _, paragraph in
                        Text(paragraph)
                            .glassType(GlassTokens.TypeScale.body)
                            .foregroundStyle(GlassColor.textSecondary)
                            .fixedSize(horizontal: false, vertical: true)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
            }
        }
    }
}

/// A modal's body at the modal's own insets: as tall as its content while
/// that fits the window, and scrolling once it does not, so a short body
/// never stretches the modal to the window's height.
public struct GlassModalBody<Content: View>: View {
    private let spacing: CGFloat
    private let content: Content

    public init(spacing: CGFloat = GlassTokens.Space.s6, @ViewBuilder content: () -> Content) {
        self.spacing = spacing
        self.content = content()
    }

    public var body: some View {
        ViewThatFits(in: .vertical) {
            padded
            ScrollView { padded }
        }
    }

    private var padded: some View {
        VStack(alignment: .leading, spacing: spacing) {
            content
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.vertical, GlassTokens.Space.s7)
        .padding(.horizontal, 18)
    }
}

/// The hairline between a modal's header, body and footer.
private struct GlassModalRule: View {
    var body: some View {
        GlassHairline(GlassTokens.Color.rule.color)
    }
}

// MARK: - Presentation

public extension EnvironmentValues {
    /// False on a modal with another raised over it: it does not answer the
    /// keyboard. True everywhere else.
    @Entry var glassModalIsTopmost: Bool = true
    /// True under a `glassModalHost()`: a presented modal goes to the host.
    @Entry var glassModalHostIsPresent: Bool = false
    /// The close button's name for every modal raised under this view that
    /// names none of its own (#1146 draws one on every modal). Set once at
    /// the window's root, from the core's copy; empty draws none.
    @Entry var glassModalCloseLabel: String = ""
}

/// A presented modal, on its way to the host.
struct GlassModalRequest: Identifiable {
    let id: UUID
    let view: AnyView
}

/// The modals raised inside a host's content, innermost first.
struct GlassModalRequests: PreferenceKey {
    // Computed: `AnyView` is not `Sendable`, so a stored default would be
    // shared mutable state under strict concurrency.
    static var defaultValue: [GlassModalRequest] { [] }

    static func reduce(value: inout [GlassModalRequest], nextValue: () -> [GlassModalRequest]) {
        value += nextValue()
    }
}

/// Which modals are up, by id: what the host disables its content on.
struct GlassModalPresence: PreferenceKey {
    static var defaultValue: [UUID] { [] }

    static func reduce(value: inout [UUID], nextValue: () -> [UUID]) {
        value += nextValue()
    }
}

public extension View {
    /// The layer modals are raised in: put it at a window's root, around
    /// everything a modal covers. Modals presented anywhere inside cover
    /// the whole of it; nested modals stack, and only the topmost answers
    /// the keyboard.
    func glassModalHost() -> some View {
        modifier(GlassModalHost())
    }

    /// Present `modal` (a `GlassModal` or `GlassConfirmation`) while
    /// `isPresented` is true. Under a `glassModalHost()` it covers the
    /// window; without one, it covers this view.
    func glassModal<M: View>(isPresented: Binding<Bool>, @ViewBuilder modal: @escaping () -> M) -> some View {
        modifier(GlassModalPresenter(isPresented: isPresented.wrappedValue, modal: modal))
    }

    /// Present `modal` for `item` while it is not nil.
    func glassModal<Item: Identifiable, M: View>(
        item: Binding<Item?>, @ViewBuilder modal: @escaping (Item) -> M
    ) -> some View {
        let current = item.wrappedValue
        return modifier(GlassModalPresenter(isPresented: current != nil, modal: { current.map(modal) }))
    }
}

/// Which layer of a stack answers the keyboard: the last one raised, and
/// only while nothing is raised over it.
enum GlassModalStack {
    static func isTopmost(index: Int, count: Int, parentIsTopmost: Bool) -> Bool {
        parentIsTopmost && index == count - 1
    }
}

private struct GlassModalPresenter<M: View>: ViewModifier {
    let isPresented: Bool
    let modal: () -> M
    @State private var id = UUID()
    @Environment(\.glassModalHostIsPresent) private var hosted

    func body(content: Content) -> some View {
        if hosted {
            content
                .transformPreference(GlassModalRequests.self) { requests in
                    if isPresented { requests.append(GlassModalRequest(id: id, view: AnyView(modal()))) }
                }
                .transformPreference(GlassModalPresence.self) { ids in
                    if isPresented { ids.append(id) }
                }
        } else {
            // No host: the modal covers this view alone.
            content
                .disabled(isPresented)
                .accessibilityHidden(isPresented)
                .overlay {
                    if isPresented {
                        GlassModalLayer(isTopmost: true) { modal() }
                    }
                }
        }
    }
}

private struct GlassModalHost: ViewModifier {
    @State private var presented: [UUID] = []
    @Environment(\.glassModalIsTopmost) private var parentIsTopmost
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    func body(content: Content) -> some View {
        let covered = !presented.isEmpty
        content
            .environment(\.glassModalHostIsPresent, true)
            // A modal raised over this content takes the keyboard from it.
            .environment(\.glassModalIsTopmost, parentIsTopmost && !covered)
            .disabled(covered)
            .accessibilityHidden(covered)
            // #1146's `.tc-scrim` backdrop: the window behind blurred by
            // `size.modalScrimBlur` (8) under the 40% dim, so what the
            // modal is raised over (Join, under a passkey sheet) stays
            // faintly legible. Never a system material, which hides it.
            .blur(radius: covered ? GlassTokens.Size.modalScrimBlur : 0)
            .onPreferenceChange(GlassModalPresence.self) { ids in
                presented = ids
            }
            .overlayPreferenceValue(GlassModalRequests.self) { requests in
                ZStack {
                    ForEach(Array(requests.enumerated()), id: \.element.id) { index, request in
                        GlassModalLayer(
                            isTopmost: GlassModalStack.isTopmost(
                                index: index, count: requests.count, parentIsTopmost: parentIsTopmost)
                        ) {
                            request.view
                        }
                        .transition(.opacity)
                    }
                }
                // #1146's `tc-fade` on its scrim, at `--tc-dur`.
                .animation(GlassMotion.standard(reduceMotion), value: requests.map(\.id))
            }
            // This host presents what was raised inside it; nothing goes on
            // to a host further out, which would raise it a second time.
            .transformPreference(GlassModalRequests.self) { $0.removeAll() }
            .transformPreference(GlassModalPresence.self) { $0.removeAll() }
    }
}

/// One raised modal: the scrim over the whole host, the modal centred on
/// it inside the window's insets. The layer is itself a host, so a modal
/// raised from inside this one stacks over it.
private struct GlassModalLayer<M: View>: View {
    let isTopmost: Bool
    let modal: () -> M

    var body: some View {
        modal()
            .padding(.top, GlassTokens.Space.modalInsetTop)
            .padding([.horizontal, .bottom], GlassTokens.Space.modalInset)
            // The layer covers the whole host, the modal centred on it.
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            // The modal's own cancel, read here and kept from any layer
            // further out, so the scrim closes this modal and no other.
            .backgroundPreferenceValue(GlassModalScrimAction.self) { action in
                GlassModalScrim(onTap: isTopmost ? action?.run : nil)
            }
            .transformPreference(GlassModalScrimAction.self) { $0 = nil }
        // The host reads the layer's standing and hands the modal less
        // than it when one is raised over it.
        .glassModalHost()
        .environment(\.glassModalIsTopmost, isTopmost)
    }
}

/// The cancel a `GlassModal` hands to the scrim around it.
struct GlassModalScrimAction: PreferenceKey {
    struct Action {
        let run: () -> Void
    }

    static var defaultValue: Action? { nil }

    /// The modal itself, which sets it first; nothing inside it sets one.
    static func reduce(value: inout Action?, nextValue: () -> Action?) {
        if value == nil { value = nextValue() }
    }
}

/// The scrim: `modalScrim` over the window behind, which the host blurs
/// by `size.modalScrimBlur`. It takes every click, so nothing behind can
/// be used; a click on it calls `onTap`, the topmost modal's cancel (#1146).
private struct GlassModalScrim: View {
    let onTap: (() -> Void)?

    var body: some View {
        ZStack {
            Rectangle().fill(GlassTokens.Color.modalScrim.color)
        }
        .ignoresSafeArea()
        .contentShape(Rectangle())
        .onTapGesture { onTap?() }
        .accessibilityHidden(true)
    }
}
