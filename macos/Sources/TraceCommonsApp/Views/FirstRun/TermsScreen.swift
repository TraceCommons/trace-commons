import SwiftUI
import TCDesign
import TCShellCore

/// Terms, the step before Join: the published terms on a page of their own,
/// read through and accepted before signing in (owner, 2026-11-09). Continue
/// stays off until the terms on screen are accepted
/// (`FirstRunRunner.hasAcceptedTerms`); without the terms copy it never
/// opens.
struct TermsScreen: View {
    let copy: FirstRunCopy
    @ObservedObject var runner: FirstRunRunner
    /// The published terms, loaded when the step opens.
    @StateObject private var termsLoader = TermsLoader()

    var body: some View {
        let legal = copy.welcome.legal
        FirstRunFrame(
            copy: copy,
            state: $runner.state,
            isCommitting: runner.isCommitting,
            footer: FirstRunFooter(
                title: copy.frame.continueButton,
                isEnabled: runner.hasAcceptedTerms(legal: legal, terms: termsLoader.terms),
                action: { runner.continueFromTerms(legal: legal, terms: termsLoader.terms) })
        ) {
            FirstRunHub()
                .firstRunActor(FirstRunStageLayout.hubActor)
                .frame(width: 56, height: 56)
                .accessibilityHidden(true)
        } title: {
            if let legal {
                FirstRunTitle(light: legal.titleLight, bold: legal.terms)
            }
        } content: {
            if let legal {
                TermsReader(
                    copy: legal, loader: termsLoader, accepted: runner.acceptedTerms,
                    onAccepted: { runner.acceptedTerms = $0 })
                    .frame(maxWidth: .infinity)
            }
        }
    }
}
