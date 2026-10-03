import SwiftUI
import TCBridge
import TCShellCore

/// Onboarding screen 4, "Extra privacy scan" -- shown only when the operator
/// has configured the second scanner (`DaemonSettingsView.nearAIConfigured`,
/// from `get_settings`). Every word on it is the core's
/// (`privacy_scan_copy`, through `tc_privacy_scan_copy_json`), which holds
/// the shared design spec's "### 4. Extra privacy scan" copy for every
/// shell; this screen used to hold its own transcription of it.
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
    var onContinue: () -> Void

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

    init(onContinue: @escaping () -> Void, previewChoice: Choice = .localOnly) {
        self.onContinue = onContinue
        _choice = State(initialValue: previewChoice)
    }

    /// The core's words for this screen. Nil draws the screen without its
    /// choices or its Continue: the scan is never chosen, and nothing is
    /// acknowledged, against a disclosure this screen could not show.
    private let copy = PrivacyScanCopy.decode(fromJSON: TCCoreCopy.privacyScanCopyJSON())

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.xl) {
            if let copy {
                header(copy)
                explanation(copy)
                choices(copy)
                continueButton
            }
        }
        .padding(TC.Space.xxl)
        .tcColumn(TC.Measure.prose)
        .tcScreen()
    }

    private func header(_ copy: PrivacyScanCopy) -> some View {
        Text(copy.title).font(TC.Font_.sectionTitle)
    }

    // `Text(verbatim:)`: the core's sentences are plain text, and the
    // `LocalizedStringKey` initialiser would read any `*` in them as
    // Markdown.
    private func explanation(_ copy: PrivacyScanCopy) -> some View {
        VStack(alignment: .leading, spacing: TC.Space.m) {
            Text(verbatim: copy.localAlways)
                .font(.body)
            Text(verbatim: copy.offer)
                .font(.body)
            Text(verbatim: copy.disclosure)
                .font(.body)
        }
    }

    private func choices(_ copy: PrivacyScanCopy) -> some View {
        VStack(alignment: .leading, spacing: TC.Space.s) {
            choiceRow(.localOnly, title: copy.localOnly)
            choiceRow(.localPlusScan, title: copy.withNear)
        }
    }

    private func choiceRow(_ value: Choice, title: String) -> some View {
        Button {
            choice = value
        } label: {
            HStack(spacing: TC.Space.m) {
                Image(systemName: choice == value ? "largecircle.fill.circle" : "circle")
                    .font(.system(size: 15))
                    .foregroundStyle(choice == value ? AnyShapeStyle(TC.accentText) : AnyShapeStyle(.tertiary))
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
                .tcPrimaryAction()
        .keyboardShortcut(.defaultAction)
    }
}
