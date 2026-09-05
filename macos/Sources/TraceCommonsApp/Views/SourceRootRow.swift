import AppKit
import SwiftUI
import TCShellCore

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
/// folder is being watched without knowing which. The row says "Watching"
/// and shows no path rather than inventing one.
struct SourceRootRow: View {
    let kind: SourceKind
    let candidate: SourceCandidate?
    let choice: SourceChoice
    var onWatchCandidate: (SourceCandidate) -> Void
    var onChoose: (String) -> Void
    var onDecline: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.xs) {
            Text(kind.displayName).font(TC.Font_.body.weight(.semibold))
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
        switch choice {
        case .off:
            Text("Not used on this machine — nothing will be watched")
                .font(TC.Font_.body)
        case .watch(let path):
            VStack(alignment: .leading, spacing: TC.Space.xxs) {
                Text("Watching").font(TC.Font_.body)
                if !path.isEmpty {
                    Text(path)
                        .font(TC.Font_.ledger)
                        .lineLimit(1)
                        .truncationMode(.head)
                }
            }
        case .undecided:
            if let candidate {
                VStack(alignment: .leading, spacing: TC.Space.xxs) {
                    Text(candidate.path)
                        .font(TC.Font_.ledger)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.head)
                    Text(candidate.evidence(now: Date()))
                        .font(TC.Font_.body)
                        .foregroundStyle(.secondary)
                }
            } else {
                Text("No standard location found — choose a folder, or decline")
                    .font(TC.Font_.body)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private var choiceButtons: some View {
        HStack(spacing: TC.Space.m) {
            if let candidate, candidate.exists {
                Button("Watch this folder") { onWatchCandidate(candidate) }
                    .disabled(choice == .watch(path: candidate.path))
            }
            Button("Choose a different folder…") {
                if let path = Self.chooseFolder() { onChoose(path) }
            }
            Button("I don't use \(kind.displayName)") { onDecline() }
                .disabled(choice == .off)
            Spacer(minLength: 0)
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
