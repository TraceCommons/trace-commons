import SwiftUI

/// Onboarding screen 5, "What to watch" -- lists the projects the daemon has
/// discovered, every one starting at ask-first. Copy and rules are from the
/// shared design spec
/// (`docs/superpowers/specs/2026-08-08-contributor-shell-shared-design.md`,
/// "## Onboarding", "### 5. What to watch").
///
/// The project list itself comes from the daemon's `list_projects` call
/// (`AppModel.projects`, populated via `DaemonClient.listProjects()`), never
/// from a hardcoded array -- the same rule `ConsentScopesContent` follows for
/// scopes.
///
/// `Ignore` is offered here and `auto_upload` is deliberately not: excluding
/// a client repo is a live thought at this exact moment and never returns,
/// whereas arming automation before the contributor has seen a single
/// preview asks for trust they have no basis to give yet. There is no
/// `set_project_mode` daemon call yet (see `SettingsView.modeSentence`,
/// which is read-only for the same reason), so this screen -- like
/// `ConsentScopesContent` -- keeps the chosen ignore-set as local state and
/// hands it to the caller via `onContinue` rather than calling the daemon
/// itself.
struct OnboardingProjectsView: View {
    @EnvironmentObject private var model: AppModel
    var onContinue: (Set<String>) -> Void = { _ in }

    var body: some View {
        ScrollView {
            OnboardingProjectsContent(onContinue: onContinue)
                .environmentObject(model)
        }
    }
}

/// The screen's content, split out of its `ScrollView` for the same
/// `ImageRenderer` reason documented on `ConsentScopesContent`.
struct OnboardingProjectsContent: View {
    @EnvironmentObject private var model: AppModel

    /// Labels of projects the person has marked to skip. Nothing starts
    /// ignored -- every discovered project begins ask-first, per the spec.
    @State private var ignored: Set<String> = []

    var onContinue: (Set<String>) -> Void = { _ in }

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            header
            projectList
            unresolvedNote
            continueButton
        }
        .padding(24)
        .frame(maxWidth: 620, alignment: .leading)
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("What to watch").font(.title2.weight(.semibold))
            Text("Every project below asks you first, every time. You can turn one off entirely.")
                .font(.callout)
                .foregroundStyle(.secondary)
        }
    }

    private var projectList: some View {
        VStack(alignment: .leading, spacing: 10) {
            if model.projects.isEmpty {
                Text("No projects discovered yet.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            } else {
                ForEach(model.projects) { project in
                    projectRow(project)
                }
            }
        }
    }

    private func projectRow(_ project: ProjectRow) -> some View {
        let isIgnored = ignored.contains(project.projectLabel)
        return HStack(spacing: 8) {
            VStack(alignment: .leading, spacing: 2) {
                Text(project.projectLabel).font(.callout.weight(.semibold))
                Text(isIgnored ? "Never offered" : "Asks you first")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            Button(isIgnored ? "Ignored" : "Ignore") {
                if isIgnored {
                    ignored.remove(project.projectLabel)
                } else {
                    ignored.insert(project.projectLabel)
                }
            }
            .buttonStyle(.bordered)
        }
    }

    // Sessions the watcher cannot map to any project are not itemized here
    // (there is nothing to list -- no path, no label, per the "never render
    // a filesystem path" rule), just a permanent, plain-English note that
    // they can never be armed for automatic upload.
    private var unresolvedNote: some View {
        Text("""
        Sessions that don't resolve to a project are always ask-first. They can \
        never be set to upload automatically.
        """)
        .font(.caption)
        .foregroundStyle(.secondary)
    }

    private var continueButton: some View {
        Button("Continue") {
            onContinue(ignored)
        }
        .keyboardShortcut(.defaultAction)
    }
}
