import SwiftUI

/// Onboarding screen 4, "Extra privacy scan" -- shown only when the operator
/// has configured the second scanner (`DaemonSettingsView.nearAIConfigured`,
/// from `get_settings`). Copy is verbatim from the shared design spec
/// (`docs/superpowers/specs/2026-08-08-contributor-shell-shared-design.md`,
/// "## Onboarding", "### 4. Extra privacy scan"), not paraphrased.
///
/// Two rules from the spec are load-bearing and must not be softened:
///
/// 1. Never headline "PII filter" or "NEAR AI" -- those are internal names.
///    The heading and the picker labels below describe what actually
///    happens ("a second scanner run by a third party") instead. The one
///    place "NEAR AI" appears is inside the disclosure paragraph itself,
///    naming who the message text is sent to, exactly as the spec's copy
///    block does -- that is disclosure, not a headline.
/// 2. The disclosure has two halves and both must stay: message text really
///    does leave this machine to a third party before Trace Commons ever
///    sees it (the cost), and if that scanner is unreachable nothing is
///    sent at all rather than going out unscanned (the reassurance).
///    Cutting either half makes the screen dishonest in one direction.
///
/// Choosing the scan calls `AppModel.acknowledgeNearAINotice()` --
/// `acknowledge_near_ai_notice` on the wire -- because that is the only way
/// an app-only contributor (who never sees the CLI's stdout notice) clears
/// `near-ai-notice-not-acknowledged` and the daemon starts using the filter.
/// Skipping this call does not politely leave the filter off; it leaves the
/// daemon permanently refusing it with no way for a GUI-only person to find
/// out why, so the call happens the moment the scan is chosen, not deferred
/// to some later settings screen.
struct OnboardingPrivacyScanView: View {
    @EnvironmentObject private var model: AppModel
    var onContinue: () -> Void = {}

    var body: some View {
        // The operator-offers-it gate lives here, in the wrapper, the same
        // way `OnboardingProjectsView` leaves list-sourcing to its own
        // wrapper -- `OnboardingPrivacyScanContent` below is rendered
        // unconditionally by the screenshot hook, exactly as
        // `ConsentScopesContent` and `OnboardingProjectsContent` are, so a
        // capture does not depend on this operator ever having configured
        // the scanner.
        if model.daemonSettings?.nearAIConfigured == true {
            ScrollView {
                OnboardingPrivacyScanContent(onContinue: onContinue)
                    .environmentObject(model)
            }
        }
    }
}

/// The screen's content, split out of its `ScrollView` for the same
/// `ImageRenderer` reason documented on `ConsentScopesContent`.
struct OnboardingPrivacyScanContent: View {
    @EnvironmentObject private var model: AppModel

    enum Choice: Equatable {
        case localOnly
        case localPlusScan
    }

    @State private var choice: Choice

    var onContinue: () -> Void

    init(onContinue: @escaping () -> Void = {}, previewChoice: Choice = .localOnly) {
        self.onContinue = onContinue
        _choice = State(initialValue: previewChoice)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.xl) {
            header
            explanation
            choices
            continueButton
        }
        .padding(TC.Space.xxl)
        .tcColumn(TC.Measure.prose)
        .tcScreen()
    }

    private var header: some View {
        Text("Extra scrub before sending? (optional)").font(TC.Font_.sectionTitle)
    }

    private var explanation: some View {
        VStack(alignment: .leading, spacing: TC.Space.m) {
            Text("""
            Local scrubbing removes secrets, keys, tokens and credentials by pattern \
            before anything leaves this machine. It runs either way.
            """)
            .font(.body)

            Text("""
            You can additionally send the *message text* of each trace — not tool \
            output, not file contents — through a second scanner run by **NEAR AI**, \
            a third party, to catch personal information the patterns miss: names, \
            addresses, that kind of thing.
            """)
            .font(.body)

            Text("""
            This means your message text is transmitted to NEAR AI before it reaches \
            Trace Commons. If that scanner is unreachable, **nothing is sent at \
            all** — traces wait rather than going out unscanned.
            """)
            .font(.body)
        }
    }

    private var choices: some View {
        VStack(alignment: .leading, spacing: TC.Space.s) {
            choiceRow(.localOnly, title: "Local scrubbing only")
            choiceRow(.localPlusScan, title: "Local scrubbing + NEAR AI scan")
        }
    }

    private func choiceRow(_ value: Choice, title: String) -> some View {
        Button {
            choice = value
        } label: {
            HStack(spacing: TC.Space.m) {
                Image(systemName: choice == value ? "largecircle.fill.circle" : "circle")
                    .font(.system(size: 15))
                    .foregroundStyle(choice == value ? AnyShapeStyle(TC.green) : AnyShapeStyle(.tertiary))
                Text(title).font(TC.Font_.body.weight(choice == value ? .semibold : .regular))
                Spacer(minLength: 0)
            }
            .padding(TC.Space.m)
            .frame(maxWidth: .infinity, alignment: .leading)
            .tcCard()
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(choice == value ? [.isSelected] : [])
    }

    private var continueButton: some View {
        Button("Continue") {
            if choice == .localPlusScan {
                // Must happen the moment the scan is chosen -- see the type
                // comment above for why skipping this call leaves the
                // daemon refusing the filter with no recovery path for a
                // GUI-only contributor.
                model.acknowledgeNearAINotice()
            }
            onContinue()
        }
        .buttonStyle(.borderedProminent)
        .keyboardShortcut(.defaultAction)
    }
}
