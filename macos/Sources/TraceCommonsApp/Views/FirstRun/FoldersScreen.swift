import AppKit
import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// What discovery has given the Folders screen so far.
enum DiscoveredRows: Equatable {
    /// Discovery has not answered yet.
    case loading
    /// Discovery answered with no row the shell could read: nil JSON (a
    /// caught panic), an undecodable array, or an empty one. Claude Code and
    /// Codex are always emitted, so an empty list is a failure too.
    case failed
    case found([SourceCandidate])

    var rows: [SourceCandidate]? {
        if case .found(let rows) = self { return rows }
        return nil
    }

    /// The core's line for a failed discovery, shown with a retry.
    func failureLine(_ copy: FirstRunCopy.Folders) -> String? {
        self == .failed ? copy.discoveryFailed : nil
    }
}

/// The Folders screen's decisions, apart from the view so they can be tested.
enum FoldersScreenLayout {
    /// Discovery's rows, read without a daemon. A refresh that fails keeps
    /// the rows already shown; answers live in the run's state, keyed by
    /// kind, so a refresh that succeeds keeps them too.
    static func discovered(_ json: String?, keeping previous: DiscoveredRows) -> DiscoveredRows {
        if let json, let rows = try? SourceCandidate.decodeList(from: json), !rows.isEmpty {
            return .found(rows)
        }
        if case .found = previous { return previous }
        return .failed
    }

    /// Back, withdrawn while a commit runs: the runner moves the step on
    /// from wherever the state is when its calls finish.
    static func backAction(isCommitting: Bool, back: @escaping () -> Void) -> (() -> Void)? {
        isCommitting ? nil : back
    }

    /// The rows take no answer while a commit runs: `.start` sends no roots,
    /// so a row changed mid-commit would show a declaration the daemon never
    /// received.
    static func rowsEnabled(isCommitting: Bool) -> Bool {
        !isCommitting
    }

    /// The core's sentence for a failure this step can show. A failed start
    /// reads `watcher_start_failed` and the step stays. A failed enroll reads
    /// `enroll_refused`: lookup has already accepted the invite, so Join's
    /// `invite_error` ("not an invite link") would be false, and the daemon
    /// never says why enroll refused. A dead invite has already returned the
    /// person to Join, which shows its own line.
    static func notice(
        for failure: FirstRunFailure?, copy: FirstRunCopy, onboarding: TCOnboardingCopy?
    ) -> String? {
        switch failure {
        case .startFailed?: return onboarding?.watcherStartFailed
        case .enrollFailed?: return copy.folders.enrollRefused
        // Silent by decision: the step stays and Continue reopens near.ai's
        // sheet, and a cancelled sheet is the person's own act. No core line
        // tells a cancel from a refusal.
        case .signInFailed?: return nil
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

    @State private var discovery: DiscoveredRows = .loading
    @State private var onboarding = TCOnboardingCopy.load()

    var body: some View {
        FirstRunFrame(
            copy: copy,
            state: $runner.state,
            // Withdrawn while a commit runs: the runner moves the step on
            // from wherever the state is when its calls finish.
            onBack: FoldersScreenLayout.backAction(isCommitting: runner.isCommitting) {
                runner.state = FirstRunNavigation.back(runner.state)
            },
            isCommitting: runner.isCommitting,
            notice: FoldersScreenLayout.notice(for: runner.failure, copy: copy, onboarding: onboarding),
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
                switch discovery {
                case .found(let candidates):
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
                    .disabled(!FoldersScreenLayout.rowsEnabled(isCommitting: runner.isCommitting))
                case .failed:
                    HStack(spacing: GlassTokens.Space.s4) {
                        Text(discovery.failureLine(copy.folders) ?? "")
                            .glassType(GlassTokens.TypeScale.body)
                            .foregroundStyle(GlassColor.textSecondary)
                            .fixedSize(horizontal: false, vertical: true)
                        Button(copy.folders.retry) { refreshDiscovery() }
                            .buttonStyle(GlassButtonStyle(.secondary))
                    }
                case .loading:
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
            if discovery == .loading { refreshDiscovery() }
        }
        // A tool installed while the app was in the background shows when the
        // person comes back, as the missing row's line promises.
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            refreshDiscovery()
        }
    }

    private func refreshDiscovery() {
        discovery = FoldersScreenLayout.discovered(TCDiscovery.sourcesJSON(), keeping: discovery)
    }

    private var canContinue: Bool {
        guard let candidates = discovery.rows, !runner.isCommitting else { return false }
        return FirstRunNavigation.canContinue(runner.state, candidates: candidates, requiredScope: nil)
    }

    private var title: some View {
        Text("\(Text(copy.folders.titleLight))\(Text(copy.folders.titleBold).fontWeight(.bold))")
            .glassType(GlassTokens.TypeScale.display.weight(.regular))
            .foregroundStyle(GlassColor.textPrimary)
    }
}
