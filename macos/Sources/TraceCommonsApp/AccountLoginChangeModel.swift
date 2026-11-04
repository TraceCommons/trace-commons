import Foundation
import SwiftUI
import TCBridge
import TCShellCore

/// The change of login (F10)'s sheet state. The daemon runs the ceremony and
/// every network call (`account_login_change`); this opens the browser for
/// the two browser legs and asks how they went until they end. Every
/// sentence is the core's (`AccountChangeCopy`).
@MainActor
final class AccountLoginChangeModel: ObservableObject {
    static let shared = AccountLoginChangeModel()

    /// The core's words, nil if the export did not arrive (no entry point
    /// then draws).
    let copy: AccountChangeCopy? =
        TCConsentCopy.accountChangeCopyJSON().flatMap(AccountChangeCopy.decode(fromJSON:))

    /// The ceremony as the daemon last answered it; nil while no sheet is up.
    @Published private(set) var state: AccountLoginChangeState?
    @Published var presented = false
    /// The NEAR account name, typed in step 1 of a wallet-origin account.
    @Published var walletAccount = ""
    /// A call to the daemon is running.
    @Published private(set) var calling = false
    /// The daemon refused to start a change (shown as the generic line).
    @Published private(set) var startFailed = false

    /// How often the sheet asks how a browser leg is going, and for how
    /// long before it stops asking (the daemon's own leg ends sooner).
    static let pollEvery: Duration = .seconds(1)
    static let pollLimit: TimeInterval = 6 * 60

    private var client: DaemonClient?
    private var openBrowser: (URL) -> Void = { NSWorkspace.shared.open($0) }
    private var onFinished: () -> Void = {}
    private var generation: UInt64 = 0

    func attach(_ client: DaemonClient?, openBrowser: @escaping (URL) -> Void, onFinished: @escaping () -> Void) {
        if self.client !== client { generation &+= 1 }
        self.client = client
        self.openBrowser = openBrowser
        self.onFinished = onFinished
    }

    /// The sheet's page for the current state.
    var page: AccountLoginChangePage? {
        guard let copy, let state else { return nil }
        return AccountLoginChangeSurface.page(state, copy: copy)
    }

    /// Open the sheet and start (or resume) the ceremony. A login held
    /// aside moves into it, so its step 2 is skipped.
    func begin() async {
        guard let client, !calling else { return }
        generation &+= 1
        startFailed = false
        walletAccount = ""
        calling = true
        let started = await DaemonWork.run { Result { try client.accountLoginChange.start() } }
        calling = false
        switch started {
        case .success(let state):
            self.state = state
            presented = true
        case .failure:
            startFailed = true
        }
    }

    /// The page's primary button.
    func primary() async {
        guard let client, let state, let page, !calling, !page.busy else { return }
        let id = state.changeID
        switch page.action {
        case .close:
            close()
            return
        case .proveCurrent:
            let account = state.isWallet
                ? walletAccount.trimmingCharacters(in: .whitespacesAndNewlines) : nil
            if state.isWallet, account?.isEmpty != false { return }
            await leg { try client.accountLoginChange.proveCurrent(changeID: id, walletAccountID: account) }
        case .signInNew:
            await leg { try client.accountLoginChange.signInNew(changeID: id) }
        case .confirm:
            calling = true
            let answer = await DaemonWork.run { Result { try client.accountLoginChange.confirm(changeID: id) } }
            calling = false
            if case .success(let next) = answer { self.state = next } else { await reread() }
            if self.state?.step == .done {
                close()
                onFinished()
            }
        }
    }

    /// Cancel: the daemon drops the ceremony and any login held for it.
    func cancel() {
        let id = state?.changeID
        if let client {
            Task { _ = await DaemonWork.run { Result { try client.accountLoginChange.cancel(changeID: id) } } }
        }
        close()
    }

    private func close() {
        generation &+= 1
        presented = false
        state = nil
        walletAccount = ""
    }

    /// Begin a browser leg, open the browser, and ask until it ends.
    private func leg(_ call: @escaping @Sendable () throws -> AccountLoginChangeState) async {
        calling = true
        let answer = await DaemonWork.run { Result { try call() } }
        calling = false
        guard case .success(let next) = answer else {
            await reread()
            return
        }
        state = next
        if let text = next.browserURL, let url = URL(string: text) {
            openBrowser(url)
        }
        await follow(next.changeID)
    }

    private func follow(_ id: String) async {
        let current = generation
        let deadline = Date().addingTimeInterval(Self.pollLimit)
        while state?.busy == true, Date() < deadline {
            try? await Task.sleep(for: Self.pollEvery)
            guard current == generation, let client else { return }
            let answer = await DaemonWork.run { Result { try client.accountLoginChange.status(changeID: id) } }
            guard current == generation else { return }
            if case .success(let next) = answer { state = next } else { return }
        }
    }

    private func reread() async {
        guard let client, let id = state?.changeID else { return }
        let answer = await DaemonWork.run { Result { try client.accountLoginChange.status(changeID: id) } }
        if case .success(let next) = answer { state = next }
    }
}
