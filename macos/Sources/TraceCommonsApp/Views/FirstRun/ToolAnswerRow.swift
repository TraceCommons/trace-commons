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
    /// Whether the row asks at all. A tool not on this Mac is not asked
    /// (spec rule 1, the owner's reversal in Ron's review of #1235); a
    /// missing one the state already watches -- a restored state -- still
    /// asks, so the picker can show the answer Continue counts.
    static func asks(_ candidate: SourceCandidate, in state: FirstRunState) -> Bool {
        if candidate.exists { return true }
        if case .watch = state.sessionRoots[candidate.source] { return true }
        return false
    }

    /// The answers a row offers: both on a row that asks, none otherwise.
    static func options(for candidate: SourceCandidate, in state: FirstRunState) -> [ToolAnswer] {
        asks(candidate, in: state) ? [.watch, .dontUse] : []
    }

    /// Ron's folder button sits on a row that asks.
    static func offersFolderChoice(_ candidate: SourceCandidate, in state: FirstRunState) -> Bool {
        asks(candidate, in: state)
    }

    /// "Get {tool}" sits on a row that does not ask, and only when the core
    /// gave the tool an install page (Ron's `tool.installUrl ? ... : null`).
    static func offersGetTool(_ candidate: SourceCandidate, installURL: URL?, in state: FirstRunState) -> Bool {
        !asks(candidate, in: state) && installURL != nil
    }

    /// What the row shows as answered, read from the same declaration
    /// Continue reads. The picker can show it because
    /// `options(for:in:)` offers every answer this can return.
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

    /// `chosenFolder` is the row's last "Choose a different folder", kept
    /// so "I don't use it" and back to Watch returns to it, not discovery's.
    static func select(
        _ answer: ToolAnswer?, for candidate: SourceCandidate, in state: inout FirstRunState,
        chosenFolder: String? = nil
    ) {
        switch answer {
        case .watch?:
            state.answer(candidate.source, .watch(path: chosenFolder ?? shownPath(in: state, for: candidate)))
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
        FirstRunCopy.fill(template, ["tool": tool.displayName])
    }
}

/// Ron's tool row (#1030 `tool-row.tsx`) in glass: the tool's tile, name,
/// the folder it would watch and discovery's evidence line, then the
/// answer. Every string is the core's or discovery's.
///
/// `meta` is the trailing line; Folders passes the evidence line, and the
/// Tools screen may pass a shorter one. `installURL` names where "Get
/// {tool}" leads; with none there is no button.
struct ToolAnswerRow: View {
    let copy: FirstRunCopy.Folders
    /// What the picker reads while unanswered: the core's "Choose…".
    let choose: String
    let candidate: SourceCandidate
    let meta: String
    @Binding var state: FirstRunState
    var installURL: URL? = nil

    @State private var chosenFolder: String?

    var body: some View {
        GlassCard {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                HStack(spacing: GlassTokens.Space.s6) {
                    GlassToolTile(.tool(candidate.source.glassTool), large: true)
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
                    if ToolAnswerRowLayout.asks(candidate, in: state) {
                        Spacer(minLength: 0)
                        GlassPicker(
                            ToolAnswerRowLayout.fill(copy.watchQuestion, tool: candidate.source),
                            selection: answer,
                            options: options,
                            placeholder: choose
                        )
                        if ToolAnswerRowLayout.offersFolderChoice(candidate, in: state) {
                            GlassFolderButton(ToolAnswerRowLayout.fill(copy.chooseFolder, tool: candidate.source)) {
                                if let path = FolderPanel.choose() {
                                    chosenFolder = path
                                    ToolAnswerRowLayout.choose(folder: path, for: candidate, in: &state)
                                }
                            }
                        }
                    } else {
                        // Not on this Mac: not asked. Ron's install line, and
                        // "Get {tool}" only with an install page.
                        Text(copy.notInstalled)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textTertiary)
                            .frame(maxWidth: .infinity, alignment: .leading)
                        if ToolAnswerRowLayout.offersGetTool(candidate, installURL: installURL, in: state),
                            let installURL
                        {
                            Button {
                                NSWorkspace.shared.open(installURL)
                            } label: {
                                Label(
                                    ToolAnswerRowLayout.fill(copy.getTool, tool: candidate.source),
                                    systemImage: "arrow.down.to.line")
                            }
                            .buttonStyle(GlassButtonStyle(.secondary, small: true))
                            .help(ToolAnswerRowLayout.fill(copy.downloadTool, tool: candidate.source))
                        }
                    }
                }
            }
        }
    }

    private var answer: Binding<ToolAnswer?> {
        Binding(
            get: { ToolAnswerRowLayout.answer(in: state, for: candidate) },
            set: { ToolAnswerRowLayout.select($0, for: candidate, in: &state, chosenFolder: chosenFolder) }
        )
    }

    private var options: [GlassPickerOption<ToolAnswer>] {
        ToolAnswerRowLayout.options(for: candidate, in: state).map { option in
            switch option {
            case .watch: return GlassPickerOption(copy.watch, value: .watch, dot: .on)
            case .dontUse: return GlassPickerOption(copy.dontUse, value: .dontUse, dot: .off)
            }
        }
    }
}
