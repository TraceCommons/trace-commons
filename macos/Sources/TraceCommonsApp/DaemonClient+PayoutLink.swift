import TCShellCore

// The five payout-link IPC calls (`payout_rails_status`, `payout_link_orgs`,
// `payout_link_start`, `payout_link_confirm`, `payout_link_unlink`) go
// through `PayoutLinkClient` in TCShellCore, so their method names and
// parameters are asserted without a daemon. Calls block (the ABI is
// synchronous): callers run them off the main actor.
extension DaemonClient {
    var payoutLink: PayoutLinkClient { PayoutLinkClient(transport: daemon) }
}
