import SwiftUI
import TCDesign
import TCShellCore

/// The tool a Start button is explaining the folder choice for (#1408).
struct ManagedFolderChoice: Identifiable, Equatable {
    let tool: String
    var id: String { tool }
}

/// Before the folder picker, until a session has started on this computer
/// (#1408, Ron's rulings 2026-10-14): the Start button's own label as the
/// title, a folder sliding into a terminal, one line, then Cancel and
/// Choose folder. Every word is the core's (`managed/copy.rs`).
struct ManagedFolderExplainerSheet: View {
    let tool: String
    let onCancel: () -> Void
    let onChoose: () -> Void
    @EnvironmentObject private var model: AppModel

    /// The line, with the tool's name and the terminal the daemon reports.
    @MainActor
    static func line(tool: String, model: AppModel) -> String {
        ManagedSurface.folderChoiceLine(
            model.managedText("folder_choice"),
            tool: ManagedWords.tool(tool, model: model) ?? "",
            destination: model.managedSnapshot?.capabilities.terminalDestination,
            fallback: model.managedText("terminal"))
    }

    var body: some View {
        GlassModal(
            title: ManagedSurface.startKey(tool).map(model.managedText) ?? "", width: .narrow,
            actions: [
                .cancel(model.managedText("cancel"), action: onCancel),
                GlassModalAction(model.managedText("choose_folder"), isDefault: true, id: "choose", action: onChoose),
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
            }
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
