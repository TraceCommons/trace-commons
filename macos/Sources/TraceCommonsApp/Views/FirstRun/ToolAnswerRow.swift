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

/// A tool row's decisions, which Tools' checks follow, apart from the view
/// so they can be tested. Every write goes through `FirstRunState.answer(_:_:)`,
/// which keeps one answer per kind.
enum ToolAnswerRowLayout {
    /// A path under the home folder as `~/...` (#1030's tool row); any
    /// other path as it is.
    static func homeRelative(_ path: String, home: String = NSHomeDirectory()) -> String {
        guard !home.isEmpty, path == home || path.hasPrefix(home + "/") else { return path }
        return "~" + path.dropFirst(home.count)
    }

    /// What the row shows as answered: the row's own answer, never a folder
    /// added for the same tool, which has a row of its own (Ron's review of
    /// #1235, item 3).
    static func answer(in state: FirstRunState, for candidate: SourceCandidate) -> ToolAnswer? {
        switch state.rowAnswer(candidate.source) {
        case .undecided: return nil
        case .watch: return .watch
        case .off: return .dontUse
        }
    }

    /// The folder the row would watch: a chosen one, else discovery's.
    static func shownPath(in state: FirstRunState, for candidate: SourceCandidate) -> String {
        if case .watch(let path) = state.rowAnswer(candidate.source) { return path }
        return candidate.path
    }

    /// `chosenFolder` is the row's last "Choose a different folder", kept
    /// so "Not used" and back to Watch returns to it, not discovery's.
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
