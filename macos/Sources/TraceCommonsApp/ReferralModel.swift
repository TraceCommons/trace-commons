import AppKit
import Foundation
import TCBridge
import TCShellCore

/// The Settings Referrals section's state: the account's codes and whether
/// a code is applied, from the commons through the daemon, and the one line
/// answering the person's last action. All sentences come from the core's
/// copy (`ReferralCopy`, every one a draft for owner review); the daemon
/// does every network call. Nothing here posts a notice.
@MainActor
final class ReferralModel: ObservableObject {
    /// The core's words, nil if the export did not arrive (the section then
    /// says it is unavailable rather than inventing words).
    let surface: ReferralSurface? =
        TCConsentCopy.referralCopyJSON()
        .flatMap(ReferralCopy.decode(fromJSON:))
        .map(ReferralSurface.init(copy:))

    @Published private(set) var listing: ReferralCodesListing?
    @Published private(set) var status: ReferralStatus?
    /// Whether both reads have answered at least once, and whether the last
    /// one failed with nothing answered before it (`SettingsRead`).
    @Published private(set) var answered = false
    @Published private(set) var failed = false
    /// A missing account session: the section says Sign in again instead of
    /// the codes.
    @Published private(set) var sessionEnded = false
    @Published private(set) var busy = false
    /// The refusal of the person's last create, turn off or apply.
    @Published private(set) var refusal: ReferralRefusal?
    /// The code whose Copy was just pressed, for its "Copied" label.
    @Published private(set) var copiedCodeId: String?
    /// The code field, as typed.
    @Published var draft = ""

    private var client: DaemonClient?
    private var generation: UInt64 = 0

    func attach(_ client: DaemonClient?) {
        generation &+= 1
        self.client = client
    }

    /// The field's check before Apply: nil, or the invalid-code line.
    var fieldProblem: String? {
        surface?.fieldProblem(draft, display: TCConsentCopy.referralCodeDisplay)
    }

    var canApply: Bool {
        !busy && fieldProblem == nil
            && !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    func refresh() async {
        generation &+= 1
        let current = generation
        guard surface != nil, let client else {
            listing = nil
            status = nil
            return
        }
        let result = await DaemonWork.run {
            Result { () throws -> (ReferralCodesListing, ReferralStatus) in
                let referral = client.referral
                return (try referral.list(), try referral.status())
            }
        }
        guard current == generation else { return }
        switch result {
        case .success(let (listing, status)):
            self.listing = listing
            self.status = status
            answered = true
            failed = false
            sessionEnded = false
        case .failure(let error):
            if (error as? ReferralClient.Failure)?.refusal == .sessionEnded {
                // Nothing stale stays beside Sign in again.
                listing = nil
                status = nil
                sessionEnded = true
                answered = true
                failed = false
            } else {
                // An earlier answer stays: it is still what the daemon said.
                failed = !answered
            }
        }
    }

    func create() async {
        await act { try $0.referral.create() }
    }

    func disable(_ row: ReferralCodeRow) async {
        let id = row.codeId
        await act { try $0.referral.disable(codeId: id) }
    }

    func apply() async {
        guard canApply else { return }
        let typed = draft
        let applied = await act { try $0.referral.apply(code: typed) }
        if applied { draft = "" }
    }

    /// Puts a code on the pasteboard. The code goes nowhere else.
    func copy(_ row: ReferralCodeRow) {
        guard let code = row.code else { return }
        let board = NSPasteboard.general
        board.clearContents()
        board.setString(code, forType: .string)
        copiedCodeId = row.codeId
        let id = row.codeId
        Task { @MainActor [weak self] in
            try? await Task.sleep(nanoseconds: 2_000_000_000)
            if self?.copiedCodeId == id { self?.copiedCodeId = nil }
        }
    }

    /// Runs one action, then reads again: a refused action may mean the
    /// codes moved underneath.
    @discardableResult
    private func act<T: Sendable>(_ work: @escaping @Sendable (DaemonClient) throws -> T) async -> Bool {
        guard let client, !busy else { return false }
        busy = true
        let result = await DaemonWork.run { Result { try work(client) } }
        busy = false
        let succeeded: Bool
        switch result {
        case .success:
            refusal = nil
            succeeded = true
        case .failure(let error):
            refusal = (error as? ReferralClient.Failure)?.refusal ?? .failed
            succeeded = false
        }
        await refresh()
        return succeeded
    }
}
