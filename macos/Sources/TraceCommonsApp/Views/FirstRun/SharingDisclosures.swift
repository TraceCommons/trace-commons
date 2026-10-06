import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// The Automatic path's two disclosures, in order (owner, 2026-09-28 points
/// 2 and 4): the scrub disclosure, then the witness disclosure. Pure, so the
/// order and the progress it reports can be tested apart from the sheets.
///
/// The core decides from that progress whether the grant may be asked for
/// and with which witness (`flow1::grant_request`); this only records what
/// the person was shown.
struct SharingDisclosureFlow: Equatable {
    enum Step: Equatable {
        case scrub
        case witness
        case done
    }

    private(set) var step: Step = .scrub
    /// The signing address the witness sheet showed, nil for none.
    private(set) var witnessShown: String?

    /// The label a not-ready answer carries when the core's answer could not
    /// be read at all.
    static let unreadableLabel = "flow1-grant-request-unreadable"

    /// Whether Start goes through the disclosures: Automatic, on an account
    /// that can choose it. Ask me and watching only start directly.
    static func isNeeded(for state: FirstRunState) -> Bool {
        state.sharing == .automatic && FirstRunNavigation.canChooseAutomatic(state.account)
    }

    /// The scrub sheet's Continue. Ignored out of order.
    mutating func acknowledgeScrub() {
        guard step == .scrub else { return }
        step = .witness
    }

    /// The witness sheet's Continue, with the signing address it showed.
    /// Ignored before the scrub sheet.
    mutating func acknowledgeWitness(shown: String?) {
        guard step == .witness else { return }
        witnessShown = shown
        step = .done
    }

    /// The progress the core reads. `connected` comes from the daemon's
    /// status, never from a step remembered here. The scopes are the ones
    /// Start saves first; the plan stops before the grant if that fails.
    func progress(connected: Bool, scopes: Set<String>) -> Flow1Progress {
        Flow1Progress(
            connected: connected,
            scopesSaved: scopes.sorted(),
            path: .automatic,
            scrubDisclosureSeen: step != .scrub,
            witnessDisclosureSeen: step == .done,
            witnessShown: witnessShown)
    }

    /// The core's answer for `progress`, or nil if it could not be read.
    static func grantRequest(_ progress: Flow1Progress) -> Flow1GrantRequest? {
        guard let json = progress.jsonString() else { return nil }
        return Flow1GrantRequest.decode(fromJSON: TCFlow1.grantRequestJSON(progressJSON: json))
    }

    /// The state Start commits, and the failure to show once it has run.
    /// Ready: Automatic stands, with the witness the core named, and
    /// `grantReady` set, the only place it is; the plan sends the grant on
    /// nothing else. Anything else finishes on Ask me and says why; it never
    /// claims Automatic.
    static func resolve(
        _ state: FirstRunState, request: Flow1GrantRequest?
    ) -> (FirstRunState, FirstRunFailure?) {
        var next = state
        if let request, request.ready {
            next.witnessSigningAddress = request.witnessSigningAddress
            next.grantReady = true
            return (next, nil)
        }
        next.sharing = .askMe
        next.witnessSigningAddress = nil
        next.grantReady = false
        return (next, .grantRefused(label: request?.blockers.first ?? unreadableLabel))
    }
}

/// The two sheets, one at a time, over the Uses screen. Every word is the
/// core's: the scrub lines from the automatic contribution copy, then the
/// route disclosure and the witness's state line. Cancel on either closes
/// both and grants nothing.
struct SharingDisclosureSheet: View {
    @EnvironmentObject private var model: AppModel
    let copy: FirstRunCopy
    let grant: AutomaticGrantCopy
    @Binding var flow: SharingDisclosureFlow?
    let onFinish: (SharingDisclosureFlow) -> Void

    /// The Automatic mode's own name heads the scrub sheet.
    private var automaticTitle: String {
        ProjectModeWords.table?.label(for: .autoUpload) ?? copy.uses.sharing
    }

    var body: some View {
        Group {
            switch flow?.step {
            case .scrub: scrubSheet
            case .witness: witnessSheet
            case .done, nil: EmptyView()
            }
        }
        .onAppear { model.refreshRouteDisclosure() }
    }

    private var scrubSheet: some View {
        modal(automaticTitle, isEnabled: true, onContinue: { flow?.acknowledgeScrub() }) {
            ForEach(grant.lines, id: \.self) { line in
                sentence(line)
            }
        }
    }

    @ViewBuilder
    private var witnessSheet: some View {
        switch model.routeDisclosureState {
        case .shown(let disclosure):
            modal(disclosure.copy.title, isEnabled: true, onContinue: {
                flow?.acknowledgeWitness(shown: disclosure.facts.witness?.signingAddress)
            }) {
                RouteDisclosureGlassBody(disclosure: disclosure)
                if let line = witnessStateLine {
                    sentence(line)
                }
            }
        case .loading:
            modal(automaticTitle, isEnabled: false, onContinue: {}) {
                GlassSpinner(standalone: true)
            }
        case .unreadable:
            // Not shown truthfully, so not seen: Continue stays disabled.
            modal(automaticTitle, isEnabled: false, onContinue: {}) {
                RouteDisclosureUnreadableGlassLine(
                    line: model.routeDisclosureUnreadableCopy?.panel,
                    fallback: model.routeDisclosureUnreadableCopy?.title)
            }
        }
    }

    /// The witness's state as the core words it, or nothing.
    private var witnessStateLine: String? {
        guard let status = model.witnessStatus else { return nil }
        return WitnessSurface.stateLine(status.stateCode, calls: model.witnessCalls)
    }

    /// One disclosure as a glass modal over the window. Cancel (and
    /// Escape) closes both and grants nothing; Continue is drawn as the
    /// primary but answers no key, so a disclosure is never passed with a
    /// stray Return.
    private func modal<Body: View>(
        _ title: String, isEnabled: Bool, onContinue: @escaping () -> Void, @ViewBuilder body: () -> Body
    ) -> some View {
        GlassModal(
            title: title,
            actions: [
                .cancel(copy.passkey.cancel) { flow = nil },
                GlassModalAction(copy.frame.continueButton, isEnabled: isEnabled, isProminent: true) {
                    onContinue()
                    if let finished = flow, finished.step == .done {
                        flow = nil
                        onFinish(finished)
                    }
                },
            ],
            onCancel: { flow = nil }
        ) {
            GlassModalBody(spacing: GlassTokens.Space.s4) { body() }
        }
    }

    private func sentence(_ text: String) -> some View {
        Text(text)
            .glassType(GlassTokens.TypeScale.body)
            .foregroundStyle(GlassColor.textSecondary)
            .fixedSize(horizontal: false, vertical: true)
    }
}
