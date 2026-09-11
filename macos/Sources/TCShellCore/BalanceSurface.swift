import Foundation

/// `near_ai_balance` as the daemon answers it.
///
/// The state is carried as the daemon's own string and never parsed into a
/// Swift enum, for the reason `CredentialStatus` carries its label that way:
/// a state a later daemon grows would otherwise have to be spelled here
/// before it could be shown, and the shared tables already answer an
/// unfamiliar label safely.
///
/// **Every figure is `Optional`, and that is the whole point.** The daemon
/// sends these keys present-and-null in every state it cannot read, because
/// an absent key is ambiguous between "this daemon is too old to know" and
/// "this daemon knows it does not know" while a null is only ever the second.
/// Nothing in this shell may collapse a null onto a zero: these amounts are
/// SIGNED, so absence cannot ride on an out-of-range integer the way every
/// other money field on this surface does, and a zero is a real balance that
/// means the money is gone.
public struct BalanceStatus: Equatable, Sendable {
    public let state: String
    /// What is left, or `null` when nobody has capped the account. NEVER
    /// rendered as zero.
    public let remainingNanos: Int64?
    /// The configured ceiling, or `null` when unset.
    public let spendLimitNanos: Int64?
    /// What the whole account has spent, everywhere.
    public let totalSpentNanos: Int64?
    /// The wire's own `scale`, carried rather than assumed. See
    /// `unreadableScale` for what a payload without one gets.
    public let scale: UInt8
    /// The DAEMON's clock at the moment the service answered -- not the
    /// service's own `updated_at`. The only true thing a shell can say with
    /// it is how long ago the question was put.
    public let observedAt: Date?

    public init(
        state: String,
        remainingNanos: Int64? = nil,
        spendLimitNanos: Int64? = nil,
        totalSpentNanos: Int64? = nil,
        scale: UInt8 = BalanceStatus.unreadableScale,
        observedAt: Date? = nil
    ) {
        self.state = state
        self.remainingNanos = remainingNanos
        self.spendLimitNanos = spendLimitNanos
        self.totalSpentNanos = totalSpentNanos
        self.scale = scale
        self.observedAt = observedAt
    }

    /// The scale used when the wire carried none.
    ///
    /// **Not a fallback scale -- a refusal.** `scale` is on the wire so that
    /// a daemon which changes it does not make three shells wrong by a factor
    /// of a thousand, and a shell that filled a missing one in with nine
    /// would be doing exactly what the field exists to prevent. This value is
    /// larger than any power of ten the Rust formatter can raise, so every
    /// figure formatted with it comes back as the empty string, and the row
    /// falls to the sentence that says the answer arrived in a form this app
    /// cannot read. A wrong figure is worse than no figure when the figure is
    /// somebody's money.
    public static let unreadableScale: UInt8 = .max

    /// What a payload this build cannot read says: that the question was not
    /// answered. NOT that no sign-in is kept here, which is a claim about
    /// this machine.
    public static let unreported = BalanceStatus(state: "")

    /// From the method's own result object.
    ///
    /// A missing or unreadable `state` reads as the empty label, which the
    /// shared table answers as unreported and the shared action table answers
    /// as no action at all.
    ///
    /// `as? Int64` is what keeps a JSON `null` from becoming a zero: an
    /// `NSNull` fails the cast and stays `nil`, which is the same answer an
    /// absent key gives. The two are indistinguishable HERE on purpose --
    /// the daemon's own contract is that it never sends one meaning the
    /// other, and this shell has nothing different to say about them.
    public static func parse(fromJSON json: String) -> BalanceStatus {
        guard let data = json.data(using: .utf8),
            let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return .unreported }
        return BalanceStatus(
            state: object["state"] as? String ?? "",
            remainingNanos: object["remaining_nanos"] as? Int64,
            spendLimitNanos: object["spend_limit_nanos"] as? Int64,
            totalSpentNanos: object["total_spent_nanos"] as? Int64,
            scale: (object["scale"] as? Int).flatMap { UInt8(exactly: $0) } ?? unreadableScale,
            observedAt: (object["observed_at"] as? String).flatMap(Self.timestamp))
    }

    /// RFC 3339, the shape `chrono`'s `DateTime<Utc>` serializes to.
    ///
    /// Fractional seconds are optional in what arrives and mandatory in one
    /// of these formatters, so both are tried. A timestamp that parses as
    /// neither is no timestamp, which draws no age line rather than an
    /// invented one.
    private static func timestamp(_ text: String) -> Date? {
        let withFraction = ISO8601DateFormatter()
        withFraction.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let date = withFraction.date(from: text) { return date }
        let plain = ISO8601DateFormatter()
        plain.formatOptions = [.withInternetDateTime]
        return plain.date(from: text)
    }
}

/// The balance row's tables and formatters, across the C ABI, injected so
/// `TCShellCore` can be tested without linking the dylib.
///
/// The three money closures take the ABI's own `(present, nanos, scale)`
/// rather than an `Int64?`, deliberately: the split from `Optional` to that
/// triple happens in exactly one place, `BalanceSurface.wire`, and a closure
/// that took the `Optional` would let a second place grow. Production wiring
/// is `TCNearAiBalance`; see `AppModel`.
public struct BalanceCalls: Sendable {
    /// The sentence for one state label. Empty for `known`.
    public let stateLine: @Sendable (String) -> String?
    /// How firmly that sentence reads, as a raw `TC_PRIVATE_INFERENCE_TONE_*`
    /// value.
    public let stateTone: @Sendable (String) -> Int32
    /// The one action offered, as a raw `TC_CREDENTIAL_ACTION_*` value.
    public let action: @Sendable (String) -> Int32
    /// One amount as money. `present == 0` gives the empty string, which is
    /// NEVER `$0.00`.
    public let amount: @Sendable (Int32, Int64, UInt8) -> String?
    /// What is left, as a finished sentence. `present == 0` does NOT give
    /// the empty string here -- it gives the sentence for an uncapped
    /// account.
    public let remainingLine: @Sendable (Int32, Int64, UInt8) -> String?
    /// The configured ceiling, or the empty string.
    public let limitLine: @Sendable (Int32, Int64, UInt8) -> String?
    /// What the whole account has spent, or the empty string.
    public let spentLine: @Sendable (Int32, Int64, UInt8) -> String?
    /// How long ago this computer asked. A negative value is "no
    /// observation" and gives the empty string.
    public let observedLine: @Sendable (Int64) -> String?

    public init(
        stateLine: @escaping @Sendable (String) -> String?,
        stateTone: @escaping @Sendable (String) -> Int32,
        action: @escaping @Sendable (String) -> Int32,
        amount: @escaping @Sendable (Int32, Int64, UInt8) -> String?,
        remainingLine: @escaping @Sendable (Int32, Int64, UInt8) -> String?,
        limitLine: @escaping @Sendable (Int32, Int64, UInt8) -> String?,
        spentLine: @escaping @Sendable (Int32, Int64, UInt8) -> String?,
        observedLine: @escaping @Sendable (Int64) -> String?
    ) {
        self.stateLine = stateLine
        self.stateTone = stateTone
        self.action = action
        self.amount = amount
        self.remainingLine = remainingLine
        self.limitLine = limitLine
        self.spentLine = spentLine
        self.observedLine = observedLine
    }
}

/// What this shell renders about the money behind the destination.
///
/// Holds no words and judges no amount. Every sentence comes back from
/// `calls` or is a field of `PrivateInferenceCopy`, and the decisions that
/// matter -- which sentence, which tone, which button, and whether a figure
/// exists at all -- are all the Rust's.
///
/// **Nothing here compares an amount to anything.** There is no threshold, no
/// "low balance", no colour that depends on a number. The ABI judges no
/// amount, and an account whose ceiling may not exist has no scale to be low
/// against; a warning painted here would be this shell inventing a claim
/// nobody made about somebody else's money.
public enum BalanceSurface {
    /// The IPC method, named once.
    public static let statusMethod = "near_ai_balance"

    /// What the remaining figure renders as: money, or the Rust's sentence
    /// for there being no money to render.
    ///
    /// Two cases rather than an optional string, so a caller cannot treat
    /// "no figure" as "nothing to draw" and leave the row silent. There is
    /// ALWAYS something to say about what is left; sometimes it is a
    /// sentence rather than a number.
    public enum Remaining: Equatable, Sendable {
        /// A real amount, including `$0.00` and including a debt.
        case figure(String)
        /// No figure exists. The string is the Rust's own explanation --
        /// the uncapped-account sentence for a null, the unreadable-answer
        /// sentence for a scale this build cannot honour.
        case sentence(String)
    }

    /// The one place an `Optional` becomes the ABI's `(present, nanos)`.
    ///
    /// `present` is a separate argument on every money export here because
    /// these amounts are SIGNED: an overdrawn account is negative, so absence
    /// cannot be folded onto an out-of-range integer the way it is elsewhere.
    /// A null crosses as `(0, 0)`, and the zero is inert -- every consumer
    /// ignores `nanos` when `present` is 0. If this function is ever
    /// duplicated, the duplicate is where a null becomes `$0.00`.
    private static func wire(_ nanos: Int64?) -> (Int32, Int64) {
        guard let nanos else { return (0, 0) }
        return (1, nanos)
    }

    // MARK: - The state

    /// The sentence over the row, or none at all.
    ///
    /// `nil` means DRAW NOTHING, and it is what `known` answers: that state's
    /// row is figures, and a sentence above them announcing that the read
    /// succeeded is this app narrating itself.
    ///
    /// A caught panic in the Rust falls back to the payload's unreported
    /// sentence -- the one that says the question was not answered, never one
    /// that claims something about what is kept here or what the service did.
    public static func stateLine(
        _ status: BalanceStatus, copy: PrivateInferenceCopy, calls: BalanceCalls
    ) -> String? {
        guard let line = calls.stateLine(status.state) else { return copy.balanceUnreported }
        return line.isEmpty ? nil : line
    }

    /// Whether the figures are the row, or the sentence is.
    ///
    /// **They are alternatives, never both.** A state that has a sentence has
    /// no reading behind it: `remaining_nanos` is null in every state but
    /// `known`, and the sentence for a null remaining figure says the ACCOUNT
    /// HAS NO CEILING -- which is true of an uncapped account and nonsense
    /// beside "no sign-in is kept here". Drawing the figures unconditionally
    /// is how a contributor who has never signed in gets told about their
    /// spending limit.
    ///
    /// Derived from the shared table rather than from a label spelled here:
    /// the empty sentence IS `known`, and a state a later daemon grows that
    /// answers a sentence gets the sentence, which is the safe half. A caught
    /// panic draws no figures either.
    public static func showsFigures(_ status: BalanceStatus, calls: BalanceCalls) -> Bool {
        calls.stateLine(status.state) == ""
    }

    /// The tone the row is painted in.
    ///
    /// `PrivateInferenceTone` is reused rather than duplicated, which is what
    /// the ABI asks for: a shell maps those five values onto colours once.
    ///
    /// `.clear` means THE READ SUCCEEDED, not that the balance is healthy.
    public static func tone(
        _ status: BalanceStatus, calls: BalanceCalls
    ) -> PrivateInferenceTone {
        PrivateInferenceTone.fromABI(calls.stateTone(status.state))
    }

    // MARK: - The one action

    /// The button this state may offer, or none at all.
    ///
    /// `CredentialAction` and not an enum of this row's own: the only action
    /// this row has ever needed is the sign-in row's `obtain`, and a second
    /// enum whose `obtain` had to mean the same thing is a second table to
    /// keep in agreement with the button that opens a browser.
    public static func action(
        _ status: BalanceStatus, calls: BalanceCalls
    ) -> CredentialAction {
        CredentialAction.fromABI(calls.action(status.state))
    }

    /// Which of the two actions on this card is actually drawn.
    ///
    /// Both rows read their own state and both may answer `obtain` -- a fresh
    /// install has no key AND no session, and stacking two identical sign-in
    /// buttons on one card is a defect a contributor reads as two different
    /// sign-ins.
    ///
    /// A pending ceremony suppresses renewal until it finishes or is cancelled.
    /// A retained key with a refused session still offers renewal alongside
    /// the credential row's forget action.
    ///
    /// This changes nothing the ABI decided. Both tables are still asked, and
    /// the action offered is still the one they answered; only the second
    /// drawing of the same button is dropped.
    public static func actionToDraw(
        balance: CredentialAction, credential: CredentialAction
    ) -> CredentialAction {
        credential == .cancel || balance == credential ? .none : balance
    }

    // MARK: - The figures

    /// What is left: a figure when there is one, and the Rust's sentence when
    /// there is not.
    ///
    /// The branch is on whether the FORMATTER produced anything, never on
    /// this shell's own reading of the optional -- so the two cases a null
    /// and an unhonourable scale fall into are distinguished by the Rust,
    /// which has different sentences for them, rather than collapsed here.
    public static func remaining(
        _ status: BalanceStatus, copy: PrivateInferenceCopy, calls: BalanceCalls
    ) -> Remaining {
        let (present, nanos) = wire(status.remainingNanos)
        if let amount = calls.amount(present, nanos, status.scale), !amount.isEmpty {
            return .figure(amount)
        }
        guard let sentence = calls.remainingLine(present, nanos, status.scale),
            !sentence.isEmpty
        else { return .sentence(copy.balanceUnknown) }
        return .sentence(sentence)
    }

    /// The configured ceiling, or nothing to draw.
    ///
    /// `nil` for an absent limit is right here and wrong on `remaining`: the
    /// remaining sentence has already said the part that matters about an
    /// uncapped account, and saying it twice is once too many.
    public static func limitLine(_ status: BalanceStatus, calls: BalanceCalls) -> String? {
        let (present, nanos) = wire(status.spendLimitNanos)
        return nonEmpty(calls.limitLine(present, nanos, status.scale))
    }

    /// What the whole account has spent -- not this computer. A zero is a
    /// real `$0.00` and is drawn; an absence is drawn as no line at all.
    public static func spentLine(_ status: BalanceStatus, calls: BalanceCalls) -> String? {
        let (present, nanos) = wire(status.totalSpentNanos)
        return nonEmpty(calls.spentLine(present, nanos, status.scale))
    }

    /// How long ago THIS COMPUTER asked, or nothing.
    ///
    /// The age is arithmetic this shell does over the daemon's clock, and it
    /// crosses as a negative value when there is no observation -- the ABI's
    /// own convention for "no figure", and distinct from a zero, which means
    /// just now. A clock that ran backwards clamps to zero rather than going
    /// negative: something WAS observed, and reporting no observation because
    /// of skew would be a different and false claim.
    public static func observedLine(
        _ status: BalanceStatus, now: Date, calls: BalanceCalls
    ) -> String? {
        guard let observedAt = status.observedAt else { return nonEmpty(calls.observedLine(-1)) }
        let elapsed = now.timeIntervalSince(observedAt)
        let seconds = elapsed.isFinite ? Int64(max(0, min(elapsed, 1e15))) : 0
        return nonEmpty(calls.observedLine(seconds))
    }

    /// An empty string from the Rust means "draw no line", and a `nil` means
    /// a caught panic. Both are nothing to draw.
    private static func nonEmpty(_ value: String?) -> String? {
        guard let value, !value.isEmpty else { return nil }
        return value
    }
}
