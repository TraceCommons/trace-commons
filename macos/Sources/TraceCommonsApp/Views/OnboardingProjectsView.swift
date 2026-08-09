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
/// preview asks for trust they have no basis to give yet.
///
/// Choosing `Ignore` calls `AppModel.setProjectMode` -- a real
/// `set_project_mode` call, not local-only state -- and the row reflects
/// `project.mode` from `model.projects` (the daemon's own answer) rather
/// than a set this view invented and would otherwise have discarded on
/// `Continue`. A failure is shown inline, not swallowed: see
/// `DaemonClient.setProjectMode` for why `project-key-unrecognized` is a
/// real, reachable outcome today, not a hypothetical one.
struct OnboardingProjectsView: View {
    @EnvironmentObject private var model: AppModel
    var onContinue: () -> Void = {}

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

    var onContinue: () -> Void = {}

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            header
            if let error = model.lastActionError {
                Text(error).font(.callout).foregroundStyle(.secondary)
            }
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
        let isIgnored = project.mode == .ignore
        return HStack(spacing: 8) {
            VStack(alignment: .leading, spacing: 2) {
                Text(project.projectLabel).font(.callout.weight(.semibold))
                Text(isIgnored ? "Never offered" : "Asks you first")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            Button(isIgnored ? "Ignored" : "Ignore") {
                model.setProjectMode(project, mode: isIgnored ? .ask : .ignore)
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
            onContinue()
        }
        .keyboardShortcut(.defaultAction)
    }
}
