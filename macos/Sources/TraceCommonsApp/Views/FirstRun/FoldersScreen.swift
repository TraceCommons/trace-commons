import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// The Folders screen's decisions, apart from the view so they can be tested.
enum FoldersScreenLayout {
    /// Discovery's rows, read without a daemon. Nil JSON (a caught panic) or
    /// an undecodable array shows no row; Continue then stays closed, since
    /// Claude Code and Codex are still unanswered.
    static func candidates(fromDiscovery json: String?) -> [SourceCandidate] {
        guard let json, let rows = try? SourceCandidate.decodeList(from: json) else { return [] }
        return rows
    }

    /// The core's sentence for a failure this step can show. A failed start
    /// reads `watcher_start_failed` and the step stays; a dead invite has
    /// already returned the person to Join, which shows its own line.
    static func notice(for failure: FirstRunFailure?, onboarding: TCOnboardingCopy?) -> String? {
        switch failure {
        case .startFailed?: return onboarding?.watcherStartFailed
        default: return nil
        }
    }
}

/// Ron's Folders screen (#1030 `tool-screens.tsx` W-2), Quick setup's tool
/// list: one `ToolAnswerRow` per discovered store, and Continue once every
/// row is answered, missing tools included. Continue commits `.leaveRoots`,
/// which starts the daemon.
struct FoldersScreen: View {
    let copy: FirstRunCopy
    @ObservedObject var runner: FirstRunRunner
    /// Where each tool's "Get {tool}" leads; none is known yet.
    var installURL: (SourceKind) -> URL? = { _ in nil }

    @State private var candidates: [SourceCandidate]?
    @State private var onboarding = TCOnboardingCopy.load()

    var body: some View {
        FirstRunFrame(
            copy: copy,
            state: $runner.state,
            onBack: { runner.state = FirstRunNavigation.back(runner.state) },
            notice: FoldersScreenLayout.notice(for: runner.failure, onboarding: onboarding),
            footer: FirstRunFooter(
                title: copy.frame.continueButton,
                isEnabled: canContinue,
                action: { Task { await runner.commit(.leaveRoots) } }
            )
        ) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
                title
                Text(copy.folders.body)
                    .glassType(GlassTokens.TypeScale.body)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                if let candidates {
                    ScrollView {
                        VStack(spacing: GlassTokens.Space.s4) {
                            ForEach(candidates, id: \.source) { candidate in
                                ToolAnswerRow(
                                    copy: copy.folders,
                                    candidate: candidate,
                                    meta: candidate.evidence(now: Date()),
                                    state: $runner.state,
                                    installURL: installURL(candidate.source)
                                )
                            }
                        }
                    }
                } else {
                    HStack(spacing: GlassTokens.Space.s4) {
                        ProgressView().controlSize(.small)
                        Text(copy.folders.loading)
                            .glassType(GlassTokens.TypeScale.body)
                            .foregroundStyle(GlassColor.textSecondary)
                    }
                }
            }
        }
        .task {
            if candidates == nil {
                candidates = FoldersScreenLayout.candidates(fromDiscovery: TCDiscovery.sourcesJSON())
            }
        }
    }

    private var canContinue: Bool {
        guard let candidates, !runner.isCommitting else { return false }
        return FirstRunNavigation.canContinue(runner.state, candidates: candidates, requiredScope: nil)
    }

    private var title: some View {
        Text("\(Text(copy.folders.titleLight))\(Text(copy.folders.titleBold).fontWeight(.bold))")
            .glassType(GlassTokens.TypeScale.display.weight(.regular))
            .foregroundStyle(GlassColor.textPrimary)
    }
}
