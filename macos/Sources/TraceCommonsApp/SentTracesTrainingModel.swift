import Foundation
import SwiftUI
import TCBridge
import TCShellCore

/// Sent traces without Train coding models: how many there are, the one
/// sheet that offers the use on all of them, and the marks a grant leaves.
/// Owned by `AppModel`, so it outlives every view and spans the Settings,
/// first-run and Monitor windows. Every sentence is the core's draft copy
/// (`SentTracesTrainingCopy`); the daemon does every network call.
///
/// Nothing is preselected and nothing is sent until the person chooses:
/// the sheet is raised once per turn-on of Train coding models (and by the
/// Insights line's Review), Not now closes it and changes nothing.
@MainActor
final class SentTracesTrainingModel: ObservableObject {
    /// The core's words, nil if the export did not arrive (then nothing is
    /// drawn and no sheet is raised).
    let copy: SentTracesTrainingCopy? =
        TCConsentCopy.sentTracesTrainingCopyJSON()
        .flatMap(SentTracesTrainingCopy.decode(fromJSON:))

    @Published private(set) var state = SentTracesTrainingState()
    /// The sheet while it is up, and the window it is drawn in.
    @Published private(set) var sheet: SentTracesTrainingSheet?
    /// A grant is in flight: the sheet cannot be closed and no second
    /// grant starts.
    @Published private(set) var busy = false
    /// The sheet's grant was refused; the sheet stays up and says so.
    @Published private(set) var sheetFailed = false

    /// The daemon client, read when a call starts (set by `AppModel`).
    var client: () -> DaemonClient? = { nil }
    private var generation: UInt64 = 0

    /// Reads the eligible traces again. A refusal of any kind hides every
    /// affordance rather than show a stale count.
    func refresh() async {
        generation &+= 1
        let current = generation
        guard copy != nil, let client = client() else {
            state.applyStatusRefusal()
            return
        }
        let result = await DaemonWork.run { Result { try client.sentTracesTraining.status() } }
        guard current == generation else { return }
        switch result {
        case .success(let status): state.apply(status)
        case .failure: state.applyStatusRefusal()
        }
    }

    /// Train coding models was just turned on: read the eligible traces
    /// and, while there are any, raise the sheet once at `site`.
    func offerAfterTurnOn(at site: SentTracesTrainingSite) async {
        await refresh()
        guard sheet == nil, let raised = state.sheet(at: site) else { return }
        sheetFailed = false
        sheet = raised
    }

    /// The Insights line's Review: the same sheet, on the Monitor.
    func review() {
        guard sheet == nil, let raised = state.sheet(at: .monitor) else { return }
        sheetFailed = false
        sheet = raised
    }

    /// Not now: closes the sheet and sends nothing.
    func notNow() {
        guard !busy else { return }
        sheet = nil
        sheetFailed = false
    }

    /// Allow: adds the use to every eligible trace. The sheet closes on an
    /// answer, stays up and says it failed on a refusal, and closes on a
    /// refusal that hides the whole offer.
    func allowAll() async {
        guard !busy, sheet != nil, let client = client() else { return }
        busy = true
        sheetFailed = false
        let result = await DaemonWork.run { Result { try client.sentTracesTraining.grantAll() } }
        busy = false
        switch result {
        case .success(let answer):
            state.apply(answer)
            sheet = nil
            await refresh()
        case .failure(let error):
            let refusal = (error as? SentTracesTrainingClient.Failure)?.refusal ?? .failed
            state.applyGrantRefusal(refusal, for: [])
            if refusal == .hidden { sheet = nil } else { sheetFailed = true }
        }
    }

    /// The inspector's Allow: adds the use to this one trace.
    func allow(_ submissionId: String) async {
        guard !busy, state.offers(submissionId), let client = client() else { return }
        busy = true
        let ids = [submissionId]
        let result = await DaemonWork.run { Result { try client.sentTracesTraining.grant(submissionIds: ids) } }
        busy = false
        switch result {
        case .success(let answer):
            state.apply(answer)
        case .failure(let error):
            let refusal = (error as? SentTracesTrainingClient.Failure)?.refusal ?? .failed
            state.applyGrantRefusal(refusal, for: ids)
        }
    }
}
