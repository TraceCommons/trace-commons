import AppKit
import SwiftUI
import TCDesign
import TCShellCore

/// A row's answer as its picker offers it. `nil` is the placeholder: nobody
/// has answered, and nothing is written for the person.
enum ToolAnswer: Hashable {
    case watch
    case dontUse
}

/// The tool row's decisions, apart from the view so they can be tested.
/// Every write goes through `FirstRunState.answer(_:_:)`, which keeps one
/// answer per kind.
enum ToolAnswerRowLayout {
    /// A found tool offers both answers. A missing one still asks, but can
    /// only be answered "I don't use it" (owner, 2026-10-04): Continue waits
    /// for it, and its `off` is the person's, never written unasked.
    static func options(for candidate: SourceCandidate) -> [ToolAnswer] {
        candidate.exists ? [.watch, .dontUse] : [.dontUse]
    }

    /// Ron's folder button sits on a found tool's row only.
    static func offersFolderChoice(_ candidate: SourceCandidate) -> Bool {
        candidate.exists
    }

    /// "Get {tool}" sits on a missing tool's row only.
    static func offersGetTool(_ candidate: SourceCandidate) -> Bool {
        !candidate.exists
    }

    /// What the row shows as answered, read from the same declaration
    /// Continue reads, so the two cannot disagree.
    static func answer(in state: FirstRunState, for candidate: SourceCandidate) -> ToolAnswer? {
        switch state.sessionRoots[candidate.source] {
        case .undecided: return nil
        case .watch: return .watch
        case .off: return .dontUse
        }
    }

    /// The folder the row would watch: a chosen one, else discovery's.
    static func shownPath(in state: FirstRunState, for candidate: SourceCandidate) -> String {
        if case .watch(let path) = state.sessionRoots[candidate.source] { return path }
        return candidate.path
    }

    static func select(_ answer: ToolAnswer?, for candidate: SourceCandidate, in state: inout FirstRunState) {
        switch answer {
        case .watch?:
            state.answer(candidate.source, .watch(path: shownPath(in: state, for: candidate)))
        case .dontUse?:
            state.answer(candidate.source, .off)
        case nil:
            state.answer(candidate.source, .undecided)
        }
    }

    /// A folder picked with "Choose a different folder" is watched.
    static func choose(folder path: String, for candidate: SourceCandidate, in state: inout FirstRunState) {
        state.answer(candidate.source, .watch(path: path))
    }

    /// Fill the core's `{tool}` placeholder with the tool's name.
    static func fill(_ template: String, tool: SourceKind) -> String {
        template.replacingOccurrences(of: "{tool}", with: tool.displayName)
    }
}

/// Ron's tool row (#1030 `tool-row.tsx`) in glass: the tool's tile, name,
/// the folder it would watch and discovery's evidence line, then the
/// answer. Every string is the core's or discovery's.
///
/// `meta` is the trailing line; Folders passes the evidence line, and the
/// Tools screen may pass a shorter one. `installURL` names where "Get
/// {tool}" leads; with none the button is shown but cannot be pressed.
struct ToolAnswerRow: View {
    let copy: FirstRunCopy.Folders
    let candidate: SourceCandidate
    let meta: String
    @Binding var state: FirstRunState
    var installURL: URL? = nil

    var body: some View {
        GlassCard {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                HStack(spacing: GlassTokens.Space.s6) {
                    GlassToolTile(.tool(TracesTreeView.glassTool(candidate.source)), large: true)
                    VStack(alignment: .leading, spacing: 0) {
                        Text(candidate.source.displayName)
                            .glassType(GlassTokens.TypeScale.bodyStrong)
                            .foregroundStyle(GlassColor.textPrimary)
                        Text(ToolAnswerRowLayout.shownPath(in: state, for: candidate))
                            .glassType(GlassTokens.TypeScale.mono)
                            .foregroundStyle(GlassColor.textTertiary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                            .help(ToolAnswerRowLayout.shownPath(in: state, for: candidate))
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    Text(meta)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textTertiary)
                        .lineLimit(1)
                }
                HStack(spacing: GlassTokens.Space.s4) {
                    if ToolAnswerRowLayout.offersGetTool(candidate) {
                        Text(copy.notInstalled)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textTertiary)
                            .frame(maxWidth: .infinity, alignment: .leading)
                        Button(ToolAnswerRowLayout.fill(copy.getTool, tool: candidate.source)) {
                            if let installURL { NSWorkspace.shared.open(installURL) }
                        }
                        .buttonStyle(GlassButtonStyle(.secondary))
                        .disabled(installURL == nil)
                        .help(ToolAnswerRowLayout.fill(copy.downloadTool, tool: candidate.source))
                    } else {
                        Spacer(minLength: 0)
                    }
                    GlassPicker(
                        ToolAnswerRowLayout.fill(copy.watchQuestion, tool: candidate.source),
                        selection: answer,
                        options: options,
                        placeholder: ToolAnswerRowLayout.fill(copy.watchQuestion, tool: candidate.source)
                    )
                    if ToolAnswerRowLayout.offersFolderChoice(candidate) {
                        GlassFolderButton(ToolAnswerRowLayout.fill(copy.chooseFolder, tool: candidate.source)) {
                            if let path = Self.pickFolder() {
                                ToolAnswerRowLayout.choose(folder: path, for: candidate, in: &state)
                            }
                        }
                    }
                }
            }
        }
    }

    private var answer: Binding<ToolAnswer?> {
        Binding(
            get: { ToolAnswerRowLayout.answer(in: state, for: candidate) },
            set: { ToolAnswerRowLayout.select($0, for: candidate, in: &state) }
        )
    }

    private var options: [GlassPickerOption<ToolAnswer>] {
        ToolAnswerRowLayout.options(for: candidate).map { option in
            switch option {
            case .watch: return GlassPickerOption(copy.watch, value: .watch, dot: .on)
            case .dontUse: return GlassPickerOption(copy.dontUse, value: .dontUse, dot: .off)
            }
        }
    }

    private static func pickFolder() -> String? {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.canCreateDirectories = false
        guard panel.runModal() == .OK, let url = panel.url else { return nil }
        return url.path
    }
}
