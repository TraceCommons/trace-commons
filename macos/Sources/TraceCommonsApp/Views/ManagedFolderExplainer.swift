import SwiftUI
import TCDesign
import TCShellCore

/// The tool a Start button is explaining the folder choice for (#1408).
struct ManagedFolderChoice: Identifiable, Equatable {
    let tool: String
    var id: String { tool }
}

/// A tool's launch dialog (owner, 2026-10-19; #1408's explainer before
/// it): the Start button's own label as the title, a folder sliding into a
/// terminal, the line that says where the tool opens, the folder row with
/// its "Pick a folder as context" button, then Cancel and Open {tool}.
/// The folder is optional (owner, 2026-10-29): Open is enabled from the
/// start and, with none picked, the daemon opens the tool in the person's
/// home folder. A picked folder can be cleared back to that default. Every
/// word is the core's (`managed/copy.rs`).
struct ManagedFolderExplainerSheet: View {
    let tool: String
    let onCancel: () -> Void
    /// Open, with the chosen folder's path, or nil for the home folder.
    let onOpen: (String?) -> Void
    let onOpenWithSettings: ((String?, ManagedFolderDraft) -> Void)?
    @EnvironmentObject private var model: AppModel
    @State private var path: String?
    @State private var draft = ManagedFolderDraft()
    @State private var catalogue: ManagedModelCatalogue?
    private var supportsFolderSettings: Bool { model.managedSnapshot?.capabilities.folderSettings == true }
    private var canOpen: Bool {
        guard !model.managedBusy else { return false }
        return !supportsFolderSettings || (draft.matches(tool: tool, cwd: path)
            && ManagedFolderOptions.modelIsAvailable(draft, tool: tool, connection: "near_ai_signin", catalogue: catalogue))
    }

    init(tool: String, onCancel: @escaping () -> Void, onOpen: @escaping (String?) -> Void,
         onOpenWithSettings: ((String?, ManagedFolderDraft) -> Void)? = nil) {
        self.tool = tool
        self.onCancel = onCancel
        self.onOpen = onOpen
        self.onOpenWithSettings = onOpenWithSettings
    }

    /// The line, with the tool's name and the terminal the daemon reports.
    @MainActor
    static func line(tool: String, model: AppModel) -> String {
        ManagedSurface.folderChoiceLine(
            model.managedText("folder_choice"),
            tool: ManagedWords.tool(tool, model: model) ?? "",
            destination: model.managedSnapshot?.capabilities.terminalDestination,
            fallback: model.managedText("terminal"))
    }

    /// The dialog's Open button: "Open Claude Code".
    @MainActor
    static func openLabel(tool: String, model: AppModel) -> String {
        model.managedText("open_tool").replacingOccurrences(of: "{tool}", with: ManagedWords.tool(tool, model: model) ?? "")
    }

    var body: some View {
        GlassModal(
            title: ManagedSurface.startKey(tool).map(model.managedText) ?? "", width: .narrow,
            actions: [
                .cancel(model.managedText("cancel"), action: onCancel),
                GlassModalAction(Self.openLabel(tool: tool, model: model), isDefault: true, isEnabled: canOpen,
                                 id: "open", action: open),
            ],
            onCancel: onCancel
        ) {
            GlassModalBody {
                ManagedFolderHandoff(tool: ManagedWords.glassTool(tool))
                    .frame(maxWidth: .infinity)
                Text(Self.line(tool: tool, model: model))
                    .glassType(GlassTokens.TypeScale.label)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                // The folder, once picked (else the home default), the button
                // that picks it and, once picked, the one that clears it.
                HStack(spacing: GlassTokens.Space.s4) {
                    GlassToolTile(.folder)
                    if let path {
                        Text(path)
                            .glassType(GlassTokens.TypeScale.mono)
                            .foregroundStyle(GlassColor.textPrimary)
                            .lineLimit(1)
                            .truncationMode(.head)
                            .help(path)
                    } else {
                        Text(model.managedText("home_folder"))
                            .glassType(GlassTokens.TypeScale.label.weight(.regular))
                            .foregroundStyle(GlassColor.textTertiary)
                            .lineLimit(1)
                    }
                    Spacer(minLength: GlassTokens.Space.s4)
                    if path != nil {
                        Button(model.managedText("clear_folder")) { path = nil }
                            .buttonStyle(GlassButtonStyle(.glass, small: true))
                            .fixedSize()
                    }
                    Button(model.managedText("pick_folder")) {
                        if let chosen = FolderPanel.choose() { path = chosen }
                    }
                    .buttonStyle(GlassButtonStyle(.glass, small: true))
                    .fixedSize()
                }
                if supportsFolderSettings {
                    ManagedFolderOptions(tool: tool, connection: "near_ai_signin", draft: $draft,
                        catalogue: catalogue, supportsFolderSettings: true,
                        retry: { loadFolderSettings() }, reset: { loadFolderSettings(reset: true) })
                }
            }
        }
        .onAppear {
            loadFolderSettings()
            if supportsFolderSettings { model.managedModelCatalogue(tool: tool) { catalogue = $0 } }
        }
        .onChange(of: path) { _, _ in loadFolderSettings() }
    }

    private func open() {
        guard canOpen else { return }
        if supportsFolderSettings, let onOpenWithSettings { onOpenWithSettings(path, draft) }
        else { onOpen(path) }
    }

    private func loadFolderSettings(reset: Bool = false) {
        guard supportsFolderSettings else { return }
        let token = draft.begin(tool: tool, cwd: path)
        model.managedFolderSettings(tool: tool, cwd: path, reset: reset) { settings in
            if let settings { draft.complete(settings, token: token) }
            else { draft.fail(token: token) }
        }
    }
}

/// A tool's launch button (owner, 2026-10-19): its logo and its name on a
/// glass button, named for assistive tech by the core's Start label. On
/// the Private AI tab's NEAR AI card and in the menu bar's popover.
struct ManagedToolButton: View {
    let tool: String
    let action: () -> Void
    @EnvironmentObject private var model: AppModel

    var body: some View {
        Button(action: action) {
            HStack(spacing: GlassTokens.Space.s3) {
                logo(ManagedWords.glassTool(tool))
                Text(ManagedWords.tool(tool, model: model) ?? "").lineLimit(1)
            }
        }
        .buttonStyle(GlassButtonStyle(.glass, small: true))
        .fixedSize()
        .accessibilityLabel(ManagedSurface.startKey(tool).map(model.managedText) ?? "")
    }

    /// The tool's logo in its tint, or its initials when it has none.
    @ViewBuilder
    private func logo(_ glass: GlassTool) -> some View {
        if let logo = glass.logo {
            GlassToolLogoShape(logo).fill(glass.tint.color).frame(width: 14, height: 14)
        } else {
            Text(glass.initials).glassType(GlassTokens.TypeScale.micro).foregroundStyle(glass.tint.color)
        }
    }
}

/// A folder slides into a terminal window that shows the tool's logo,
/// once, for about 1.2 s, then rests there. Flat artwork on the glass
/// tokens: a faint rim, flat fills, no glow. Decorative, so hidden from
/// VoiceOver; under Reduce Motion it is the rest frame, still.
struct ManagedFolderHandoff: View {
    let tool: GlassTool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var atRest: Bool

    /// The ruling's duration. No motion token is this long; the token
    /// curve is used at it.
    static let duration: Double = 1.2

    /// Whether the artwork starts on its rest frame rather than moving.
    static func startsAtRest(reduceMotion: Bool) -> Bool { reduceMotion }

    /// `atRest` draws the rest frame from the start, as Reduce Motion does.
    init(tool: GlassTool, atRest: Bool = false) {
        self.tool = tool
        _atRest = State(initialValue: atRest)
    }

    // The terminal is centred, so the rest frame (and the Reduce Motion
    // still) is balanced; the folder starts outside its left edge and rests
    // inside it, left of the logo.
    private static let width: CGFloat = 260
    private static let height: CGFloat = 108
    private static let terminalSize = CGSize(width: 156, height: 100)
    private static let terminalX: CGFloat = 0
    private static let contentY: CGFloat = 9
    private static let folderStartX: CGFloat = -112
    private static let folderRestX: CGFloat = terminalX - 34
    private static let logoX: CGFloat = terminalX + 34

    var body: some View {
        ZStack {
            terminal.offset(x: Self.terminalX)
            GlassToolTile(.folder, large: true)
                .scaleEffect(atRest ? 0.9 : 1)
                .offset(x: atRest ? Self.folderRestX : Self.folderStartX, y: Self.contentY)
        }
        .frame(width: Self.width, height: Self.height)
        .accessibilityHidden(true)
        .onAppear {
            guard !atRest else { return }
            if Self.startsAtRest(reduceMotion: reduceMotion) {
                atRest = true
            } else {
                withAnimation(GlassMotion.curve(Self.duration)) { atRest = true }
            }
        }
    }

    /// A flat window: a title bar with three dots over a hairline, and the
    /// tool's logo in its tint.
    private var terminal: some View {
        let shape = RoundedRectangle(cornerRadius: GlassTokens.Radius.control, style: .continuous)
        return ZStack(alignment: .top) {
            shape.fill(GlassColor.ink(0.06))
            VStack(spacing: 0) {
                HStack(spacing: 4) {
                    ForEach(0..<3, id: \.self) { _ in
                        Circle().fill(GlassColor.ink(0.18)).frame(width: 6, height: 6)
                    }
                    Spacer(minLength: 0)
                }
                .padding(.horizontal, 9)
                .frame(height: 18)
                Rectangle().fill(GlassColor.hairline).frame(height: 1)
            }
            if let logo = tool.logo {
                GlassToolLogoShape(logo)
                    .fill(tool.tint.color)
                    .frame(width: 28, height: 28)
                    .offset(x: Self.logoX - Self.terminalX, y: Self.terminalSize.height / 2 + Self.contentY - 14)
            }
        }
        .frame(width: Self.terminalSize.width, height: Self.terminalSize.height)
        .overlay(shape.strokeBorder(GlassColor.hairline, lineWidth: 1))
    }
}
