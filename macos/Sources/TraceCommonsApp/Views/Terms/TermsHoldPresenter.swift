import SwiftUI
import TCDesign
import TCShellCore

/// The new-terms sheet (spec section 5), raised by Home's terms notice or
/// the menu bar's row through `AppModel.termsSheetOpen`. It fetches the
/// published terms afresh each time it opens, so the person accepts what
/// they read, and draws the first run's reader: loading and failed states,
/// scroll to the end to unlock the checkbox, then Accept. Closing it
/// without accepting leaves the hold and the notice.
struct TermsHoldPresenter: ViewModifier {
    @EnvironmentObject private var app: AppModel

    func body(content: Content) -> some View {
        content
            .glassModal(isPresented: Binding(
                get: { app.termsSheetOpen && TermsWords.legal != nil },
                set: { if !$0 { app.termsSheetOpen = false } })
            ) {
                if let legal = TermsWords.legal {
                    TermsHoldSheet(legal: legal, loader: app.termsHoldLoader)
                }
            }
    }
}

/// The sheet's body: the terms reader and Accept, which stays disabled until
/// the checkbox is ticked for the document on screen.
struct TermsHoldSheet: View {
    @EnvironmentObject private var app: AppModel
    let legal: FirstRunCopy.Welcome.Legal
    @ObservedObject var loader: TermsLoader
    @State private var accepted: AcceptedTerms?
    @State private var sending = false

    var body: some View {
        GlassModal(
            title: legal.terms, width: .regular,
            actions: [
                GlassModalAction(
                    legal.acceptButton, isDefault: true,
                    isEnabled: accepted != nil && !sending && loader.terms != nil
                ) {
                    guard let accepted else { return }
                    sending = true
                    Task {
                        let done = await app.acceptNewTerms(accepted)
                        sending = false
                        // Stale or refused: the reader shows the terms again
                        // and asks for a fresh tick.
                        if !done { self.accepted = nil }
                    }
                }
            ],
            busy: sending,
            onCancel: { app.termsSheetOpen = false }
        ) {
            TermsReader(
                copy: legal, loader: loader, accepted: accepted, autoload: false,
                onAccepted: { accepted = $0 })
                .frame(maxWidth: .infinity)
        }
        .task { await loader.load() }
    }
}
