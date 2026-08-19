import AppKit
import SwiftUI
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
/// ## Why the fields start empty
///
/// The conventional locations are offered by a button, not pre-filled. A
/// pre-filled form that is one click from Continue records consent nobody
/// gave -- the same objection that rules out migrating existing installs by
/// writing the roots they were implicitly watching. The contributor has to
/// put the paths there, even if putting them there is one deliberate click.
struct OnboardingRootsView: View {
    @EnvironmentObject private var model: AppModel

    /// Where the daemon will be started once the roots are declared. Passed
    /// in rather than re-resolved so the screen and the start agree even if
    /// the environment changes underneath them.
    let configDirectory: String

    @State private var roots = SessionRoots()
    @State private var failure: String?

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.xl) {
            header
            explanation
            pickers
            if let failure {
                Text(failure).font(TC.Font_.body).foregroundStyle(.red)
            }
            actions
        }
        .padding(TC.Space.xxl)
        .tcColumn(TC.Measure.prose)
        .tcScreen()
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
                Name both folders. Leaving one blank does not mean "skip that one" — the \
                watcher would fall back to the standard location for it, which is \
                probably your real work.
                """)
                .font(.body)
        }
    }

    private var pickers: some View {
        VStack(alignment: .leading, spacing: TC.Space.m) {
            pickerRow(title: "Claude Code sessions", path: $roots.claude)
            pickerRow(title: "Codex sessions", path: $roots.codex)
        }
    }

    private func pickerRow(title: String, path: Binding<String>) -> some View {
        VStack(alignment: .leading, spacing: TC.Space.xs) {
            Text(title).font(TC.Font_.body.weight(.semibold))
            HStack(spacing: TC.Space.m) {
                Text(path.wrappedValue.isEmpty ? "Not set" : path.wrappedValue)
                    .font(TC.Font_.ledger)
                    .foregroundStyle(path.wrappedValue.isEmpty ? AnyShapeStyle(.tertiary) : AnyShapeStyle(.primary))
                    .lineLimit(1)
                    .truncationMode(.head)
                Spacer(minLength: 0)
                Button("Choose…") { choose(into: path) }
            }
            .padding(TC.Space.m)
            .frame(maxWidth: .infinity, alignment: .leading)
            .tcCard()
        }
    }

    private var actions: some View {
        HStack(spacing: TC.Space.m) {
            Button("Use the standard locations") {
                roots = SessionRoots(
                    claude: NSHomeDirectory() + "/.claude",
                    codex: NSHomeDirectory() + "/.codex"
                )
            }
            Spacer(minLength: 0)
            Button("Continue") { start() }
                .tcPrimaryAction()
                .disabled(!roots.isComplete)
        }
    }

    private func choose(into path: Binding<String>) {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.canCreateDirectories = false
        guard panel.runModal() == .OK, let url = panel.url else { return }
        path.wrappedValue = url.path
    }

    private func start() {
        guard let settingsJSON = roots.settingsJSON() else {
            failure = "Name both folders before continuing."
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
