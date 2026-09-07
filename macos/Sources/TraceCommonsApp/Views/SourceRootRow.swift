import AppKit
import SwiftUI
import TCShellCore
import TCBridge

/// One agent's session store: what is known about it, what the contributor
/// has said, and the three ways to answer.
///
/// Shared between the roots screen and Settings' "Watched folders" so an
/// answer given at first run and one changed later look and behave the
/// same. The row is presentational: it holds no state and writes nothing,
/// and each button reports the choice to its owner, who either collects it
/// (the roots screen, before the daemon exists) or writes it through
/// `set_settings` (Settings, against a running daemon).
///
/// A `watch` whose path is empty is a real state here: `get_settings`
/// reports each source's MODE and never its path, so Settings knows a
/// folder is being watched without knowing which. The row uses the core mode sentence
/// and shows no path rather than inventing one.
struct SourceRootRow: View {
    let kind: SourceKind
    let candidate: SourceCandidate?
    let choice: SourceChoice
    var reportedMode: String? = nil
    private static let copy = TCSourceChecks.settingsCopy()

    var onWatchCandidate: (SourceCandidate) -> Void
    var onChoose: (String) -> Void
    var onDecline: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.xs) {
            Text(kind.displayName).font(TC.Font_.body.weight(.semibold))
            if let explanation = Self.copy?.tools[kind.rawValue]?.explanation {
                Text(explanation).font(TC.Font_.meta).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            VStack(alignment: .leading, spacing: TC.Space.s) {
                answerLine
                choiceButtons
            }
            .padding(TC.Space.m)
            .frame(maxWidth: .infinity, alignment: .leading)
            .tcCard()
        }
    }

    /// What this row currently says: the discovered store and its evidence
    /// while undecided, or the answer once one has been given.
    @ViewBuilder
    private var answerLine: some View {
        if let copy = Self.copy, let tool = copy.tools[kind.rawValue] {
            if let reportedMode {
                if let line = TCSourceChecks.checkLine(tool: tool.key, sourceMode: reportedMode) {
                    Text(line).font(TC.Font_.body)
                }
                // A conventional unset source is already read. Candidate
                // evidence must not replace that authoritative answer.
                if reportedMode == "unset", !tool.unsetScansConventional {
                    candidateLine(copy)
                }
            } else {
                switch choice {
                case .off:
                    if let line = TCSourceChecks.checkLine(tool: tool.key, sourceMode: "off") {
                        Text(line).font(TC.Font_.body)
                    }
                case .watch(let path):
                    Text(copy.selectedFolder).font(TC.Font_.body)
                    if !path.isEmpty {
                        Text(path).font(TC.Font_.ledger).lineLimit(1).truncationMode(.head)
                    }
                case .undecided:
                    candidateLine(copy)
                }
            }
        }
    }

    @ViewBuilder
    private func candidateLine(_ copy: SourceSettingsCopy) -> some View {
        if let candidate {
            Text(candidate.path).font(TC.Font_.ledger).foregroundStyle(.secondary)
                .lineLimit(1).truncationMode(.head)
            Text(candidate.evidence(now: Date())).font(TC.Font_.body).foregroundStyle(.secondary)
        } else if copy.tools[kind.rawValue]?.explanation == nil {
            Text(copy.noCandidate).font(TC.Font_.body).foregroundStyle(.secondary)
        }
    }

    @ViewBuilder
    private var choiceButtons: some View {
        if let copy = Self.copy, let tool = copy.tools[kind.rawValue] {
            HStack(spacing: TC.Space.m) {
                if let candidate, candidate.exists {
                    Button(copy.watchCandidate) { onWatchCandidate(candidate) }
                        .disabled(choice == .watch(path: candidate.path))
                }
                Button(tool.chooseFolder ?? copy.chooseFolder) {
                    if let path = Self.chooseFolder() { onChoose(path) }
                }
                Button(tool.decline) { onDecline() }.disabled(choice == .off)
                Spacer(minLength: 0)
            }
        }
    }

    /// The folder panel. Nil when dismissed.
    static func chooseFolder() -> String? {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.canCreateDirectories = false
        guard panel.runModal() == .OK, let url = panel.url else { return nil }
        return url.path
    }
}
