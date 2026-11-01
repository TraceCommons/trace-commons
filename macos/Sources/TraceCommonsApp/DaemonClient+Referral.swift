import TCBridge
import TCShellCore

// The five referral IPC calls (`referral_codes_list`, `referral_codes_create`,
// `referral_codes_disable`, `referral_apply`, `referral_status`) go through
// `ReferralClient` in TCShellCore, so their method names and parameters are
// asserted without a daemon. Calls block (the ABI is synchronous): callers
// run them off the main actor.
extension DaemonClient {
    var referral: ReferralClient { ReferralClient(transport: daemon) }
}

/// Referral and payout amounts arrive as whole nano-USD. Formatted by the
/// core (`tc_near_ai_balance_amount`, scale 9): rounded down, and "less than
/// $0.01" for a nonzero amount under a cent, never a zero it does not mean.
enum ReferralMoney {
    static let nanoScale: UInt8 = 9

    static func format(_ nanoUSD: UInt64) -> String {
        TCNearAiBalance.amount(present: 1, nanos: Int64(clamping: nanoUSD), scale: nanoScale) ?? "\u{2014}"
    }
}
