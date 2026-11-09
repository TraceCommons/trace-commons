import Foundation
import TCBridge
import TCShellCore

extension TCTerms {
    /// The published terms, fetched and checked by the core and decoded
    /// here. Blocks for up to the fetch's timeouts: call it off the main
    /// actor. An answer this build cannot decode is `terms-malformed`.
    static func fetch() -> Result<FetchedTerms, TCTermsRefusal> {
        switch documentJSON() {
        case .failure(let refusal):
            return .failure(refusal)
        case .success(let json):
            guard let fetched = FetchedTerms.decode(json) else {
                return .failure(TCTermsRefusal(label: "terms-malformed"))
            }
            return .success(fetched)
        }
    }

    /// `fetch()` on a detached task, for `TermsLoader`.
    static func fetchDetached() async -> Result<FetchedTerms, TCTermsRefusal> {
        await Task.detached(priority: .userInitiated) { fetch() }.value
    }
}
