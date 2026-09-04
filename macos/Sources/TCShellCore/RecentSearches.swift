import Foundation

/// Recent searches, kept for the life of the process and never written to
/// disk.
///
/// They used to persist in `UserDefaults`, on the reasoning that the second
/// trace should be one keystroke. That reasoning is sound; the storage was
/// not. A recent-search list is the contributor's list of the things they
/// were afraid of leaking -- a client name, an employer, an unreleased
/// product, the name of a person. It is assembled precisely because those
/// strings are sensitive, which makes it a worse thing to leave on disk
/// than most of what the search was checking for.
///
/// Nothing else in this product writes that class of string down. The Linux
/// and Windows shells both hold theirs in memory for the same reason, so
/// this is also what makes the three agree.
///
/// In memory it still does its job: a contributor checking six traces in one
/// sitting types each term once. It only stops helping across a restart,
/// which is the trade being made deliberately.
/// `@MainActor` because the list is process-lifetime mutable state and this
/// target builds in Swift 6 language mode. It is only ever touched from the
/// search tab, which is already on the main actor, so the isolation costs
/// nothing and states where it is allowed to be read.
@MainActor
public enum RecentSearches {
    private static let legacyKey = "trace-commons.recent-searches"

    private static var terms: [String] = []

    public static func load() -> [String] {
        // Earlier builds wrote this list to disk. Stopping the writes does
        // not unwrite what they already stored, and an install that has been
        // upgraded would otherwise keep those terms indefinitely with no
        // surface left in the app to clear them. So the key is removed the
        // first time this is read, rather than merely ignored.
        purgeLegacyStore()
        return terms
    }

    /// Records a term the contributor actually asked for.
    ///
    /// A blank term is not a question, and it must not take a slot from one:
    /// the strip holds six, and the field is empty every time the tab opens.
    public static func remember(_ term: String) -> [String] {
        let term = term.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !term.isEmpty else { return terms }
        terms = [term] + terms.filter { $0 != term }
        terms = Array(terms.prefix(6))
        return terms
    }

    /// Test seam. The list is process-lifetime state, so an XCTest run
    /// would otherwise leak terms between cases.
    public static func reset() {
        terms = []
    }

    /// Removes the old on-disk list. Idempotent, and safe when absent.
    public static func purgeLegacyStore() {
        guard UserDefaults.standard.object(forKey: legacyKey) != nil else {
            return
        }
        UserDefaults.standard.removeObject(forKey: legacyKey)
    }
}