import SwiftUI

/// Onboarding screen 1, "What this is" -- the first thing a contributor ever
/// sees. Copy is verbatim from the shared design spec
/// (`docs/superpowers/specs/2026-08-08-contributor-shell-shared-design.md`,
/// "## Onboarding", "### 1. What this is"), not paraphrased.
///
/// The line "That scrubbing is good and it is not perfect -- which is why
/// you get to look first" is load-bearing and must not be softened: a
/// developer already knows automatic redaction is imperfect, and conceding
/// it before they ask is what makes every later claim in this app credible.
/// Do not reword it, and do not cut it for space.
struct OnboardingWelcomeView: View {
    var onGetStarted: () -> Void = {}
    var onWhatGetsRemoved: () -> Void = {}

    var body: some View {
        ScrollView {
            OnboardingWelcomeContent(onGetStarted: onGetStarted, onWhatGetsRemoved: onWhatGetsRemoved)
        }
    }
}

/// The screen's content, split out of its `ScrollView` for the same reason
/// `ConsentScopesContent` is split out of `ConsentScopesView`: `ImageRenderer`
/// renders a `ScrollView` as blank.
struct OnboardingWelcomeContent: View {
    var onGetStarted: () -> Void = {}
    var onWhatGetsRemoved: () -> Void = {}

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            Text("Trace Commons").font(.title.weight(.semibold))

            Text("""
            Coding agents get better when there are real transcripts to learn from. \
            Almost all of that data is locked inside companies. Trace Commons is a \
            shared pool that isn't.
            """)
            .font(.body)

            Text("""
            This app watches for finished Claude Code and Codex sessions on this \
            machine and shows them to you. **You decide what gets contributed. \
            Nothing is sent unless you say so.**
            """)
            .font(.body)

            Text("""
            Before anything leaves this machine it is scrubbed locally for secrets, \
            keys, and tokens. That scrubbing is good and it is not perfect — which \
            is why you get to look first.
            """)
            .font(.body)

            HStack(spacing: 12) {
                Button("Get started", action: onGetStarted)
                    .keyboardShortcut(.defaultAction)
                Button("What gets removed?", action: onWhatGetsRemoved)
            }
        }
        .padding(24)
        .frame(maxWidth: 620, alignment: .leading)
    }
}
