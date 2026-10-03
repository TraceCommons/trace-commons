import SwiftUI
import TCBridge
import TCDesign

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
    var onGetStarted: () -> Void
    var onWhatGetsRemoved: () -> Void

    var body: some View {
        ScrollView {
            OnboardingWelcomeContent(onGetStarted: onGetStarted, onWhatGetsRemoved: onWhatGetsRemoved)
        }
    }
}

/// The screen's content, split out of its `ScrollView` for the same reason
/// `ConsentScopesContent` is split out of `ConsentScopesView`: `ImageRenderer`
/// renders a `ScrollView` as blank.
///
/// It sits inside the first-run pane (`FirstRunWindowView`), which supplies
/// the glass surface; the content draws no container of its own. The
/// headline is the display step of the type scale and the promise lines are
/// accent tags, so the emphasis is carried by the tokens rather than by a
/// painted highlight. One sentence is moved: "You decide what gets
/// contributed. Nothing is sent unless you say so." was bold inside the
/// second paragraph and is now the headline. There is no step counter: a
/// fresh install has a roots step after this one and the scan step exists
/// only when the operator configured it, so the real count is not knowable
/// here.
struct OnboardingWelcomeContent: View {
    var onGetStarted: () -> Void
    var onWhatGetsRemoved: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s9) {
            Text(OnboardingWelcomeWords.wordmark)
                .glassType(GlassTokens.TypeScale.eyebrow)
                .foregroundStyle(GlassColor.textSecondary)

            headline

            // The shared sentence follows source settings instead of keeping
            // a separate tool list that can drift from the adapters and the
            // other shells.
            VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
                Text(OnboardingWelcomeWords.lede)
                Text(TCOnboardingCopy.load()?.welcomeBody ?? "")
                Text(OnboardingWelcomeWords.scrubbing)
            }
            .glassType(GlassTokens.TypeScale.body)
            .foregroundStyle(GlassColor.textPrimary)
            .fixedSize(horizontal: false, vertical: true)

            // Directly under the sentence that raises the question.
            Button(OnboardingWelcomeWords.whatGetsRemoved, action: onWhatGetsRemoved)
                .buttonStyle(GlassButtonStyle(.link))

            Button(OnboardingWelcomeWords.getStarted, action: onGetStarted)
                .buttonStyle(GlassButtonStyle(.primary))
                .keyboardShortcut(.defaultAction)

            Text(OnboardingWelcomeWords.footer)
                .glassType(GlassTokens.TypeScale.eyebrow)
                .foregroundStyle(GlassColor.textSecondary)
        }
        .padding(GlassTokens.Space.panePadding)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// The claim at display size, the promise carried on accent tags.
    private var headline: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            Text(OnboardingWelcomeWords.headline)
                .glassType(GlassTokens.TypeScale.display)
                .foregroundStyle(GlassColor.textPrimary)
                .fixedSize(horizontal: false, vertical: true)
            HStack(spacing: GlassTokens.Space.s3) {
                GlassTag(OnboardingWelcomeWords.promiseLine1, tone: .accent)
                GlassTag(OnboardingWelcomeWords.promiseLine2, tone: .accent)
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(OnboardingWelcomeWords.spoken)
    }
}

/// This screen's sentences, verbatim from the shared design spec. The
/// scrubbing concession is load-bearing and must not be softened.
enum OnboardingWelcomeWords {
    static let wordmark = "Trace Commons — Contributor"
    static let headline = "You decide what gets contributed."
    static let promiseLine1 = "Nothing is sent"
    static let promiseLine2 = "unless you say so."
    static let lede = """
        Coding agents get better when there are real transcripts to learn \
        from. Almost all of that data is locked inside companies. Trace \
        Commons is a shared pool that isn't.
        """
    static let scrubbing = """
        Before anything leaves this machine it is scrubbed locally for secrets, \
        keys, and tokens. That scrubbing is good and it is not perfect — which \
        is why you get to look first.
        """
    static let whatGetsRemoved = "What gets removed?"
    static let getStarted = "Get started"
    static let footer = "Scrubbed locally · shown to you · sent only on your word"

    /// What VoiceOver reads for the headline block, as before the rebuild.
    static let spoken = "You decide what gets contributed. Nothing is sent unless you say so."
}

#Preview("Onboarding welcome") {
    OnboardingWelcomeContent(onGetStarted: {}, onWhatGetsRemoved: {})
        .frame(width: 640)
}
