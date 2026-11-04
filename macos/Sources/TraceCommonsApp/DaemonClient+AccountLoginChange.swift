import TCShellCore

// The change of login's IPC calls (`account_login_change`,
// `account_login_change_cancel`) go through `AccountLoginChangeClient` in
// TCShellCore, so their method names and parameters are asserted without a
// daemon. Calls block (the ABI is synchronous): callers run them off the
// main actor.
extension DaemonClient {
    var accountLoginChange: AccountLoginChangeClient { AccountLoginChangeClient(transport: daemon) }
}
