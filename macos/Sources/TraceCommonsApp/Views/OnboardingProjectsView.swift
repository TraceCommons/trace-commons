import SwiftUI
import TCShellCore

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
/// `Continue`. A failure is shown inline, not swallowed.
///
/// ## The unresolvable bucket
///
/// One row can be the bucket for sessions whose working directory had no
/// usable final segment. It is recognised by `is_unresolved_bucket`, which
/// the daemon sets -- never by its label, which is a slug this screen
/// replaces, and never by re-deriving the daemon's id hash.
///
/// It carries a permanent note that these can never be armed. That is
/// enforcement this screen REPORTS rather than performs: `Policy` refuses
/// `auto_upload` for that key regardless of any client. The note is worded as
/// a consequence and not a fault, because none of it is the contributor's to
/// fix -- the bucket exists so that a directory the daemon cannot name never
/// has its path written into `daemon-audit.jsonl`, notification text or
/// `HistoryRecord`. `Ignore` is still offered: it can be silenced even though
/// it cannot be armed.
struct OnboardingProjectsView: View {
    @EnvironmentObject private var model: AppModel
    var onContinue: () -> Void

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

    var onContinue: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.xl) {
            header
            if let error = model.lastActionError {
                Text(error).font(.callout).foregroundStyle(.secondary)
            }
            projectList
            continueButton
        }
        .padding(TC.Space.xxl)
        .tcColumn(TC.Measure.prose)
        .tcScreen()
    }

    /// The subtitle states the default before the exception on purpose: the
    /// default is what happens to a contributor who reads nothing and clicks
    /// Continue, which is most of them.
    private var header: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("What to watch").font(TC.Font_.sectionTitle)
            // "Ignore a project" is not offered when there is no project to
            // ignore, so the sentence stops before it.
            Text(
                model.projects.isEmpty
                    ? "Every project starts at ask-first: you see each session before anything is sent."
                    : """
                    Every project starts at ask-first: you see each session before \
                    anything is sent. Ignore a project to leave it out entirely.
                    """
            )
            .font(.callout)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
        }
    }

    /// On a fresh install this is almost always the empty branch: a session
    /// is queued only after 30 minutes of quiet, and `list_projects` is
    /// built from the queue, so nothing has had time to appear. The step
    /// collapses to its one line and Continue rather than a field label
    /// over an empty list; nothing is invented to fill the space.
    @ViewBuilder
    private var projectList: some View {
        if model.projects.isEmpty {
            Text("No projects yet. Sessions you run later will appear here, and in Settings.")
                .font(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        } else {
            VStack(alignment: .leading, spacing: 10) {
                TCFieldLabel("Projects")
                ForEach(model.projects) { project in
                    projectRow(project)
                }
            }
        }
    }

    private func projectRow(_ project: ProjectRow) -> some View {
        let isIgnored = project.mode == .ignore
        let isBucket = project.isUnresolvedBucket
        return HStack(alignment: .top, spacing: TC.Space.m) {
            VStack(alignment: .leading, spacing: TC.Space.xxs) {
                // The bucket's own label is `unknown-project`, a slug that
                // means nothing to a contributor. The daemon marks the row;
                // the shell names it, with the words Settings uses too.
                Text(project.displayLabel)
                    .font(TC.Font_.body.weight(.semibold))
                // `Ask me first` and `Ignored` are the words Settings already
                // uses for these modes. Two screens setting one field must
                // not name it two ways.
                TCTag(
                    text: isIgnored ? "Ignored" : "Ask me first",
                    tone: isIgnored ? .neutral : .clear,
                    symbol: isIgnored ? "minus.circle" : "hand.raised"
                )
                if isBucket {
                    Text(ProjectCopy.unresolvedBucketNote)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: TC.Space.m)
            // Offered on the bucket too: it can be silenced even though it
            // can never be armed.
            Button(isIgnored ? "Ignored" : "Ignore") {
                model.setProjectMode(project, mode: isIgnored ? .ask : .ignore)
            }
            .buttonStyle(.bordered)
        }
        .padding(TC.Space.m)
        .frame(maxWidth: .infinity, alignment: .leading)
        .tcCard()
    }

    // The standing note that used to live here is gone. It said the same
    // thing unconditionally, on every machine, whether or not any such
    // session existed -- and it could not carry `Ignore`, so a contributor
    // could read that these sessions are always ask-first and have no way to
    // silence them. The bucket is a real row in `list_projects`; it is now
    // rendered as one.

    private var continueButton: some View {
        Button("Continue") {
            onContinue()
        }
                .tcPrimaryAction()
        .keyboardShortcut(.defaultAction)
    }
}
