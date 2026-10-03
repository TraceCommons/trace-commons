import SwiftUI
import TCBridge
import TCDesign
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
/// Every mode is named by the core's one name for it
/// (`ProjectCopy.modeChoiceLabel`, owner decision 2026-10-02): the tag reads
/// the mode in force, and the button reads the mode it moves to -- Never,
/// or back to Ask me. With no table neither is drawn, so no control is
/// wordless and no mode is shown that the core did not name.
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
        VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
            header
            ProjectErrorNotice()
            projectList
            continueButton
        }
        .padding(GlassTokens.Space.panePadding)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// The subtitle states the default before the exception on purpose: the
    /// default is what happens to a contributor who reads nothing and clicks
    /// Continue, which is most of them.
    private var header: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            Text(OnboardingProjectsWords.heading)
                .glassType(GlassTokens.TypeScale.heading)
                .foregroundStyle(GlassColor.textPrimary)
            // "Ignore a project" is not offered when there is no project to
            // ignore, so the sentence stops before it.
            Text(model.projects.isEmpty ? OnboardingProjectsWords.everyProjectAsksFirst : OnboardingProjectsWords.everyProjectAsksFirstIgnore)
                .glassType(GlassTokens.TypeScale.body)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    /// On a fresh install this is almost always the empty branch: a session
    /// is queued only after 30 minutes of quiet, and `list_projects` is
    /// built from the queue, so nothing has had time to appear. The step
    /// collapses to its one line and Continue rather than a card over an
    /// empty list; nothing is invented to fill the space. Before the daemon
    /// answers, the list's empty default is not read as "no projects yet".
    @ViewBuilder
    private var projectList: some View {
        if !model.status.answered {
            SettingsAwaiting()
        } else if model.projects.isEmpty {
            Text(OnboardingProjectsWords.noProjectsYet)
                .glassType(GlassTokens.TypeScale.body)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
        } else {
            GlassEyebrowCard(OnboardingProjectsWords.projects) {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(model.projects) { project in
                        GlassTableRow(first: project.id == model.projects.first?.id) { projectRow(project) }
                    }
                }
            }
        }
    }

    private func projectRow(_ project: ProjectRow) -> some View {
        let isIgnored = project.mode == .ignore
        return HStack(alignment: .top, spacing: GlassTokens.Space.s4) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                // The bucket's own label is `unknown-project`, a slug that
                // means nothing to a contributor. The daemon marks the row;
                // the shell names it, with the words Settings uses too.
                Text(project.displayLabel)
                    .glassType(GlassTokens.TypeScale.bodyStrong)
                    .fixedSize(horizontal: false, vertical: true)
                if ProjectModeWords.table != nil {
                    GlassTag(ProjectCopy.modeChoiceLabel(project.mode), tone: isIgnored ? .neutral : .ask)
                }
                if project.isUnresolvedBucket {
                    Text(ProjectCopy.unresolvedBucketNote)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: GlassTokens.Space.s4)
            // Offered on the bucket too: it can be silenced even though it
            // can never be armed. Named by the mode it moves to.
            if ProjectModeWords.table != nil {
                Button(ProjectCopy.modeChoiceLabel(isIgnored ? .ask : .ignore)) {
                    model.setProjectMode(project, mode: isIgnored ? .ask : .ignore)
                }
                .buttonStyle(GlassButtonStyle(.glass))
            }
        }
    }

    // The standing note that used to live here is gone. It said the same
    // thing unconditionally, on every machine, whether or not any such
    // session existed -- and it could not carry `Ignore`, so a contributor
    // could read that these sessions are always ask-first and have no way to
    // silence them. The bucket is a real row in `list_projects`; it is now
    // rendered as one.

    private var continueButton: some View {
        HStack(spacing: GlassTokens.Space.s4) {
            Spacer(minLength: 0)
            Button(OnboardingProjectsWords.continueButton) {
                onContinue()
            }
            .buttonStyle(GlassButtonStyle(.primary))
            .keyboardShortcut(.defaultAction)
        }
    }
}

/// This screen's sentences, held verbatim from the legacy screen. A mode's
/// name is not here: it is the core's (`ProjectCopy.modeChoiceLabel`).
enum OnboardingProjectsWords {
    static let heading = "What to watch"
    static let everyProjectAsksFirst =
        "Every project starts at ask-first: you see each session before anything is sent."
    static let everyProjectAsksFirstIgnore = """
        Every project starts at ask-first: you see each session before \
        anything is sent. Ignore a project to leave it out entirely.
        """
    static let noProjectsYet = "No projects yet. Sessions you run later will appear here, and in Settings."
    static let projects = "Projects"
    static let continueButton = "Continue"
}
