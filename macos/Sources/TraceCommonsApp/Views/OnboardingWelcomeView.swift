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
/// ## Why this screen is built differently from every other one
///
/// It is the only screen in the product with a hero, and the only one with
/// motion. Everywhere else this app is deliberately quiet, because quiet is
/// what a tool holding your source code should be. But the first frame has
/// one job the others do not: a developer who has just installed something
/// that reads their transcripts is deciding, in about four seconds, whether
/// this is a serious piece of software. A wall of undifferentiated body copy
/// does not answer that.
///
/// So the screen is banded like the community site's `.hero-band`: a two
/// column grid, the argument on the left at display size, the mark on the
/// right at 148pt assembling itself once. Type runs from a very heavy
/// headline down through a lede to body copy, which is the site's own scale
/// (`h1` clamp(34-70px) / `.lede` / body) rather than the four same-sized
/// paragraphs this screen used to be.
///
/// ## Copy
///
/// Every sentence is the spec's, unchanged. One is MOVED: "You decide what
/// gets contributed. Nothing is sent unless you say so." was set bold inside
/// the second paragraph, where it was the most important claim on the screen
/// and the least likely to be read. It is now the headline, which is what
/// bold inside a paragraph was trying and failing to do. The paragraph it
/// came from still reads as a complete sentence without it.
///
/// The scrubbing concession -- "That scrubbing is good and it is not perfect
/// -- which is why you get to look first" -- stays verbatim, stays on this
/// screen, and is not demoted into the small print. It is what makes every
/// later claim credible.
struct OnboardingWelcomeContent: View {
    var onGetStarted: () -> Void = {}
    var onWhatGetsRemoved: () -> Void = {}

    /// Big, but not fixed: `@ScaledMetric` means the accessibility text
    /// sizes still move it, which a hardcoded 42 would not.
    @ScaledMetric(relativeTo: .largeTitle) private var displaySize: CGFloat = 42

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.xxl) {
            hero
            supporting
            actions
        }
        .padding(.horizontal, TC.Space.xxl)
        .padding(.vertical, TC.Space.xxl)
        .tcColumn()
        .tcScreen()
    }

    /// The site's `.hero-band`: `minmax(0, 1.15fr) minmax(320px, 0.85fr)`,
    /// bottom-aligned, ruled off underneath.
    private var hero: some View {
        VStack(alignment: .leading, spacing: TC.Space.xl) {
            HStack(alignment: .bottom, spacing: TC.Space.xxl) {
                VStack(alignment: .leading, spacing: TC.Space.l) {
                    TCFieldLabel("Trace Commons", tone: .clear)

                    // The thesis, at the size of a thesis.
                    Text("You decide what gets contributed.\nNothing is sent unless you say so.")
                        .font(.system(size: displaySize, weight: .heavy))
                        .lineSpacing(-2)
                        .fixedSize(horizontal: false, vertical: true)

                    Text("""
                    Coding agents get better when there are real transcripts to learn \
                    from. Almost all of that data is locked inside companies. Trace \
                    Commons is a shared pool that isn't.
                    """)
                    .font(.title3)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                }
                .frame(maxWidth: .infinity, alignment: .leading)

                BrandMarkIntro(size: 148)
                    .padding(.bottom, TC.Space.xxs)
            }
            Rectangle().fill(TC.line).frame(height: TC.Space.hairline)
        }
    }

    private var supporting: some View {
        HStack(alignment: .top, spacing: TC.Space.xxl) {
            Text("""
            This app watches for finished Claude Code and Codex sessions on this \
            machine and shows them to you.
            """)
            .frame(maxWidth: .infinity, alignment: .leading)

            Text("""
            Before anything leaves this machine it is scrubbed locally for secrets, \
            keys, and tokens. That scrubbing is good and it is not perfect — which \
            is why you get to look first.
            """)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .font(.body)
        .fixedSize(horizontal: false, vertical: true)
    }

    private var actions: some View {
        HStack(spacing: TC.Space.m) {
            Button("Get started", action: onGetStarted)
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
                .keyboardShortcut(.defaultAction)
            Button("What gets removed?", action: onWhatGetsRemoved)
                .controlSize(.large)
                .tint(.primary)
        }
    }
}
