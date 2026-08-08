import SwiftUI

/// Onboarding screen 6, "Done" -- the last onboarding screen and, by design,
/// the first thing the app ever confirms about itself. Copy is verbatim from
/// the shared design spec
/// (`docs/superpowers/specs/2026-08-08-contributor-shell-shared-design.md`,
/// "## Onboarding", "### 6. Done").
///
/// "You're set up. Nothing has been sent." is the entire point of the
/// screen: the first thing this app ever does is nothing. Do not soften or
/// reorder it out of the first line.
struct OnboardingDoneView: View {
    var onFinish: () -> Void = {}

    var body: some View {
        ScrollView {
            OnboardingDoneContent(onFinish: onFinish)
        }
    }
}

/// The screen's content, split out of its `ScrollView` for the same
/// `ImageRenderer` reason documented on `ConsentScopesContent`.
struct OnboardingDoneContent: View {
    var onFinish: () -> Void = {}

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            Text("You're set up. Nothing has been sent.")
                .font(.title2.weight(.semibold))

            Text("""
            Trace Commons lives in your menu bar. When a session finishes and goes \
            quiet for 30 minutes, it'll show up there. You'll get at most one \
            notification every 4 hours, and none at all if there's nothing waiting.
            """)
            .font(.body)

            Button("Done", action: onFinish)
                .keyboardShortcut(.defaultAction)
        }
        .padding(24)
        .frame(maxWidth: 620, alignment: .leading)
    }
}
