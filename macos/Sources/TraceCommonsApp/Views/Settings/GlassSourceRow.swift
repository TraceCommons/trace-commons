import AppKit
import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// What one source row says, as data. Every sentence is the core's; this
/// only chooses which of them applies.
struct SourceRowAnswer: Equatable {
    let line: String?
    let secondaryLine: String?
    let candidatePath: String?
    let evidence: String?
    let selectedPath: String?
}

/// The old source row's branching, pure. A reported MODE is
/// authoritative: `get_settings` says that a folder is watched and never
/// which, so a watched or declined source shows the core's mode sentence and
/// no path, and candidate evidence never replaces it.
enum SourceRowState {
    static func answer(
        copy: SourceSettingsCopy, kind: SourceKind, reportedMode: String?,
        choice: SourceChoice, candidate: SourceCandidate?, now: Date = Date()
    ) -> SourceRowAnswer {
        guard let tool = copy.tools[kind.rawValue] else {
            return SourceRowAnswer(line: nil, secondaryLine: nil, candidatePath: nil, evidence: nil, selectedPath: nil)
        }
        func candidateAnswer(line: String?) -> SourceRowAnswer {
            if let candidate {
                return SourceRowAnswer(
                    line: line, secondaryLine: nil, candidatePath: candidate.path,
                    evidence: candidate.evidence(now: now), selectedPath: nil)
            }
            let none = tool.explanation == nil ? copy.noCandidate : nil
            return SourceRowAnswer(
                line: line ?? none, secondaryLine: line == nil ? nil : none,
                candidatePath: nil, evidence: nil, selectedPath: nil)
        }
        if let reportedMode {
            let line = TCSourceChecks.checkLine(tool: tool.key, sourceMode: reportedMode)
            // A conventional unset source is already read, so candidate
            // evidence must not replace that authoritative answer.
            if reportedMode == "unset", !tool.unsetScansConventional {
                return candidateAnswer(line: line)
            }
            return SourceRowAnswer(line: line, secondaryLine: nil, candidatePath: nil, evidence: nil, selectedPath: nil)
        }
        switch choice {
        case .off:
            return SourceRowAnswer(
                line: TCSourceChecks.checkLine(tool: tool.key, sourceMode: "off"),
                secondaryLine: nil, candidatePath: nil, evidence: nil, selectedPath: nil)
        case .watch(let path):
            return SourceRowAnswer(
                line: copy.selectedFolder, secondaryLine: nil, candidatePath: nil, evidence: nil,
                selectedPath: path.isEmpty ? nil : path)
        case .undecided:
            return candidateAnswer(line: nil)
        }
    }
}

/// The declared folder of exported traces (`trajectory_source`), pure. It
/// has a row while the daemon reports one watched or turned off, never a
/// path; the off switch is offered only while it is watched.
enum TrajectoryRowState {
    struct Answer: Equatable {
        let line: String
        let canTurnOff: Bool
    }

    static func answer(copy: SourceSettingsCopy.Trajectory, mode: String?) -> Answer? {
        switch mode {
        case "watch": return Answer(line: copy.watching, canTurnOff: true)
        case "off": return Answer(line: copy.off, canTurnOff: false)
        default: return nil
        }
    }

    /// What the off switch writes.
    static var offParams: [String: Any] {
        [SessionRoots.trajectorySettingsKey: ["mode": "off"]]
    }
}

/// The row for a declared folder of exported traces: the core's words and
/// one off switch. Presentational, as `GlassSourceRow` is.
struct GlassTrajectoryRow: View {
    let copy: SourceSettingsCopy.Trajectory
    let answer: TrajectoryRowState.Answer
    var onTurnOff: () -> Void

    var body: some View {
        GlassCard {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                Text(copy.title).glassType(GlassTokens.TypeScale.bodyStrong)
                Text(copy.explanation)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                Text(answer.line).glassType(GlassTokens.TypeScale.body)
                    .fixedSize(horizontal: false, vertical: true)
                HStack(spacing: GlassTokens.Space.s3) {
                    Button(copy.decline) { onTurnOff() }
                        .buttonStyle(GlassButtonStyle(.glass))
                        .disabled(!answer.canTurnOff)
                    Spacer(minLength: 0)
                }
            }
        }
        .accessibilityElement(children: .contain)
    }
}

/// One agent's session store: what is known about it, what the contributor
/// has said, and the three ways to answer. Shared by Settings' watched
/// folders and the Folders onboarding step. Presentational: it holds no
/// state and writes nothing; each button reports the choice to its owner.
struct GlassSourceRow: View {
    let kind: SourceKind
    let candidate: SourceCandidate?
    let choice: SourceChoice
    var reportedMode: String? = nil
    private static let copy = TCSourceChecks.settingsCopy()

    var onWatchCandidate: (SourceCandidate) -> Void
    var onChoose: (String) -> Void
    var onDecline: () -> Void

    var body: some View {
        if let copy = Self.copy, let tool = copy.tools[kind.rawValue] {
            let answer = SourceRowState.answer(
                copy: copy, kind: kind, reportedMode: reportedMode, choice: choice, candidate: candidate)
            GlassCard {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                    Text(kind.displayName).glassType(GlassTokens.TypeScale.bodyStrong)
                    if let explanation = tool.explanation {
                        Text(explanation)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textSecondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    if let line = answer.line {
                        Text(line).glassType(GlassTokens.TypeScale.body)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    if let path = answer.candidatePath ?? answer.selectedPath {
                        Text(path)
                            .glassType(GlassTokens.TypeScale.mono)
                            .foregroundStyle(GlassColor.textSecondary)
                            .lineLimit(1).truncationMode(.head)
                    }
                    if let evidence = answer.evidence {
                        Text(evidence).glassType(GlassTokens.TypeScale.body)
                            .foregroundStyle(GlassColor.textSecondary)
                    }
                    if let secondary = answer.secondaryLine {
                        Text(secondary).glassType(GlassTokens.TypeScale.body)
                            .foregroundStyle(GlassColor.textSecondary)
                    }
                    // The actions first, the decline last as a link (Ron,
                    // 2026-10-09).
                    HStack(spacing: GlassTokens.Space.s3) {
                        if let candidate, candidate.exists {
                            Button(copy.watchCandidate) { onWatchCandidate(candidate) }
                                .buttonStyle(GlassButtonStyle(.glass))
                                .disabled(choice == .watch(path: candidate.path))
                        }
                        GlassFolderButton(tool.chooseFolder ?? copy.chooseFolder) {
                            if let path = FolderPanel.choose() { onChoose(path) }
                        }
                        Button(tool.decline) { onDecline() }
                            .buttonStyle(GlassButtonStyle(.link))
                            .disabled(choice == .off)
                        Spacer(minLength: 0)
                    }
                }
            }
            .accessibilityElement(children: .contain)
        }
    }
}
