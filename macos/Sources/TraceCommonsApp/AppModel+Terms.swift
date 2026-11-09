import Foundation
import TCBridge
import TCShellCore

/// The terms words, from the core's first-run table, decoded once: the
/// reader's (`Welcome.Legal`) and the hold's notice and menu row.
enum TermsWords {
    static let table: FirstRunCopy? = TCCoreCopy.firstRunCopyJSON().flatMap(FirstRunCopy.decode)
    static var legal: FirstRunCopy.Welcome.Legal? { table?.welcome.legal }
    static var hold: FirstRunCopy.TermsHold? { table?.termsHold }
}

/// What Home and the menu bar draw while new terms are owed (spec section
/// 5): the Home notice, with no close action (it goes only when the hold
/// does), and the menu bar row. Nothing while not held, or without words.
struct TermsHoldSurface: Equatable {
    static let noticeID = "terms-hold"
    /// The menu bar row opens Home's overview, where the notice and the
    /// sheet are.
    static let menuDestination: MonitorDestination = .home(.overview)

    let title: String
    let body: String
    let review: String
    let menuRow: String
    /// Always false: the notice has no X and no Not now.
    let closable = false

    static func of(held: Bool, words: FirstRunCopy.TermsHold?) -> TermsHoldSurface? {
        guard held, let words else { return nil }
        return TermsHoldSurface(
            title: words.noticeTitle, body: words.noticeBody, review: words.review,
            menuRow: words.menuRow)
    }
}

/// How one `terms_accept` from the sheet ended.
enum TermsAcceptOutcome: Equatable {
    /// Recorded; the sheet closes.
    case accepted
    /// The version read is no longer the published one: the sheet stays and
    /// shows the terms again.
    case stale
    /// Refused or not answered, by label. The sheet stays.
    case failed(String)
}

extension AppModel {
    static let termsVersionStale = "terms-version-stale"

    /// The notice and menu row for the current status, or nil.
    var termsHold: TermsHoldSurface? {
        TermsHoldSurface.of(held: status.termsHold, words: TermsWords.hold)
    }

    /// Home's notice button: raise the sheet.
    func reviewNewTerms() {
        termsSheetOpen = true
    }

    /// The menu bar row: raise the sheet, then open the window on Home.
    func reviewNewTermsFromMenu() {
        termsSheetOpen = true
        OpenMonitor.request(TermsHoldSurface.menuDestination)
    }

    /// The sheet's Accept. On success the sheet closes and the status is read
    /// again (the daemon lifted the hold); on `terms-version-stale` the
    /// terms are fetched again and the sheet stays; on anything else the
    /// sheet stays.
    @discardableResult
    func acceptNewTerms(_ terms: AcceptedTerms) async -> Bool {
        switch await termsAcceptOutcome(terms) {
        case .accepted:
            termsSheetOpen = false
            refreshStatus()
            return true
        case .stale:
            await termsHoldLoader.load()
            return false
        case .failed(let label):
            NSLog("trace-commons: terms-accept-failed \(label)")
            return false
        }
    }

    private func termsAcceptOutcome(_ terms: AcceptedTerms) async -> TermsAcceptOutcome {
        let result: Result<Bool, Error>
        #if DEBUG
        if let seam = termsAcceptForTesting {
            return Self.outcome(Result { try seam(terms) })
        }
        #endif
        guard let client = daemonClientForTerms else { return .failed("daemon-unavailable") }
        result = await DaemonWork.run { Result { try client.termsAccept(terms) } }
        return Self.outcome(result)
    }

    private static func outcome(_ result: Result<Bool, Error>) -> TermsAcceptOutcome {
        switch result {
        case .success:
            return .accepted
        case .failure(let error):
            let label = (error as? DaemonClient.Failure)?.message ?? "terms-accept-failed"
            return label == Self.termsVersionStale ? .stale : .failed(label)
        }
    }
}
