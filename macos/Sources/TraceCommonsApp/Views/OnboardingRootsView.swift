import AppKit
import SwiftUI
import TCBridge
import TCShellCore

/// The roots screen: which session folders this app may watch.
///
/// It runs BEFORE the daemon starts, and therefore before any IPC exists --
/// which is why it cannot be part of the daemon-backed onboarding flow and
/// why it is not the "What to watch" screen. That screen lists projects the
/// daemon has already discovered and sets a per-project mode; it is
/// downstream of a scan. This one decides whether a scan may happen at all.
///
/// The product had a fully designed six-screen onboarding flow with no
/// screen that asked this question, which is why nothing in any client could
/// write `daemon-settings.json` and why the macOS shell's roots refusal was
/// a dead end: the only screens that could clear it lived behind the daemon
/// it was blocking.
///
/// ## Discovery is not consent
///
/// `tc_discover_sources` finds the conventional stores and describes them --
/// where, whether they are there, how many sessions, how recently touched --
/// so the contributor is answering about "953 sessions, most recent 2 hours
/// ago" instead of filling an empty field from memory. That is a better
/// question, not a substitute for asking it: **nothing is pre-selected**.
/// Every row opens undecided and needs one deliberate action.
///
/// A pre-ticked row plus a habitual Continue records consent nobody gave --
/// the same objection that rules out migrating existing installs by writing
/// the roots they were implicitly watching.
///
/// ## Why declining has its own button
///
/// "I don't use this" writes `{"mode":"off"}`. Leaving a row blank would be
/// read by the daemon as the conventional location -- the contributor's real
/// `~/.codex/sessions` -- so the one answer a blank field cannot give is the
/// one a contributor who does not use that agent needs to give.
struct OnboardingRootsView: View {
    @EnvironmentObject private var model: AppModel

    /// Where the daemon will be started once the roots are declared. Passed
    /// in rather than re-resolved so the screen and the start agree even if
    /// the environment changes underneath them.
    let configDirectory: String

    @State private var roots = SessionRoots()
    @State private var candidates: [SourceCandidate] = []
    @State private var failure: String?

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.xl) {
            header
            explanation
            rows
            if let failure {
                Text(failure).font(TC.Font_.body).foregroundStyle(.red)
            }
            actions
        }
        .padding(TC.Space.xxl)
        .tcColumn(TC.Measure.prose)
        .tcScreen()
        .onAppear(perform: discover)
    }

    private var header: some View {
        Text("Which folders may this app watch?").font(TC.Font_.sectionTitle)
    }

    private var explanation: some View {
        VStack(alignment: .leading, spacing: TC.Space.m) {
            Text("""
                This app reads coding-session transcripts. It will not guess where they \
                are, and it will not watch anything until you say.
                """)
                .font(.body)

            Text("""
                Answer for both. Declining is an answer — leaving one blank is not, and \
                the watcher would fall back to the standard location for it, which is \
                probably your real work.
                """)
                .font(.body)
        }
    }

    // MARK: - Rows

    private var rows: some View {
        VStack(alignment: .leading, spacing: TC.Space.m) {
            row(for: .claudeCode)
            row(for: .codex)
        }
    }

    private func row(for kind: SourceKind) -> some View {
        let candidate = candidates.first { $0.source == kind }
        return VStack(alignment: .leading, spacing: TC.Space.xs) {
            Text(kind.displayName).font(TC.Font_.body.weight(.semibold))
            VStack(alignment: .leading, spacing: TC.Space.s) {
                answerLine(for: kind, candidate: candidate)
                choiceButtons(for: kind, candidate: candidate)
            }
            .padding(TC.Space.m)
            .frame(maxWidth: .infinity, alignment: .leading)
            .tcCard()
        }
    }

    /// What this row currently says: the discovered store and its evidence
    /// while undecided, or the answer once one has been given.
    @ViewBuilder
    private func answerLine(for kind: SourceKind, candidate: SourceCandidate?) -> some View {
        switch roots[kind] {
        case .off:
            Text("Not used on this machine — nothing will be watched")
                .font(TC.Font_.body)
        case .watch(let path):
            VStack(alignment: .leading, spacing: TC.Space.xxs) {
                Text("Watching").font(TC.Font_.body)
                Text(path)
                    .font(TC.Font_.ledger)
                    .lineLimit(1)
                    .truncationMode(.head)
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

    @ViewBuilder
    private func choiceButtons(for kind: SourceKind, candidate: SourceCandidate?) -> some View {
        HStack(spacing: TC.Space.m) {
            if let candidate, candidate.exists {
                Button("Watch this folder") { roots.watch(candidate) }
                    .disabled(roots[kind] == .watch(path: candidate.path))
            }
            Button("Choose a different folder…") { choose(for: kind) }
            Button("I don't use \(kind.displayName)") { roots[kind] = .off }
                .disabled(roots[kind] == .off)
            Spacer(minLength: 0)
        }
    }

    private var actions: some View {
        HStack(spacing: TC.Space.m) {
            Spacer(minLength: 0)
            Button("Continue") { start() }
                .tcPrimaryAction()
                .disabled(!roots.isComplete)
        }
    }

    // MARK: - Actions

    /// Describing the machine is best-effort. If it fails the screen still
    /// works -- every row can be answered by hand -- because a contributor
    /// who cannot get past this screen cannot start the daemon at all.
    private func discover() {
        guard candidates.isEmpty, let json = TCDiscovery.sourcesJSON() else { return }
        candidates = (try? SourceCandidate.decodeList(from: json)) ?? []
    }

    private func choose(for kind: SourceKind) {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.canCreateDirectories = false
        guard panel.runModal() == .OK, let url = panel.url else { return }
        roots[kind] = .watch(path: url.path)
    }

    private func start() {
        guard let settingsJSON = roots.settingsJSON() else {
            failure = "Answer for both before continuing."
            return
        }
        failure = nil
        model.startDaemon(at: configDirectory, settingsJSON: settingsJSON)
        // A refusal here is not the roots refusal -- that one cannot recur,
        // the settings were just persisted -- so it is something else and
        // belongs on this screen rather than silently returning to it.
        if case .refused(let reason) = model.startup {
            failure = reason
        }
    }
}
