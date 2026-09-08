import Foundation

/// One coding tool on this machine, as `harness_list` described it.
///
/// A carrier, and nothing more. Every word shown about a row is either
/// IronWire's own -- `name`, `connectCommand`, `configPath` -- or comes from
/// `PrivateInferenceCopy`. Nothing here is phrased by this shell.
public struct HarnessRow: Decodable, Equatable, Sendable, Identifiable {
    public let id: String
    /// IronWire's name for the tool. Never spelled by this shell or by the
    /// copy module: the day the list grows, a hard-coded name goes stale.
    public let name: String
    public let installed: Bool
    /// Its config currently sends calls here. Proof a file has a value in
    /// it, and no evidence at all that a call was ever answered -- which is
    /// why `state` exists separately and why nothing paints from this.
    public let connected: Bool
    /// The file a connect or a disconnect would change, when this build can
    /// work out where it is.
    public let configPath: String?
    /// What to run instead, for a contributor who would rather not have an
    /// app edit their file. Shown verbatim; it is a command, not prose.
    public let connectCommand: String
    /// The protocol family the ledger stamps, when this build knows it.
    public let family: String?
    /// The daemon's own label, carried as a string and handed to the shared
    /// table. Never matched on here: a state a later daemon grows would
    /// otherwise have to be spelled in this shell before it could be shown.
    public let state: String
    /// When a call last arrived, where the family belongs to this tool alone.
    public let lastCallAt: Date?
    /// The daemon's answer from `tc_harness_action_available`. Carried so a
    /// caller may read either it or the table; `HarnessSurface` asks the
    /// table, so the two cannot drift in silence.
    public let canConnect: Bool
    public let canDisconnect: Bool

    public enum CodingKeys: String, CodingKey, CaseIterable {
        case id, name, installed, connected, family, state
        case configPath = "config_path"
        case connectCommand = "connect_command"
        case lastCallAt = "last_call_at"
        case canConnect = "can_connect"
        case canDisconnect = "can_disconnect"
    }
}

/// A call arrived in one protocol family, with no tool named.
public struct HarnessFamilyActivity: Decodable, Equatable, Sendable {
    public let family: String
    public let lastCallAt: Date?
    public let calls: Int

    public enum CodingKeys: String, CodingKey, CaseIterable {
        case family, calls
        case lastCallAt = "last_call_at"
    }
}

/// What the ledger could say, rolled up by family.
///
/// `readable` false is "no evidence about any tool", which is not the same
/// as evidence of no calls, and the two must never be drawn the same way.
public struct HarnessActivity: Decodable, Equatable, Sendable {
    public let readable: Bool
    public let windowHours: Int
    public let lastCallAt: Date?
    public let families: [HarnessFamilyActivity]

    public enum CodingKeys: String, CodingKey, CaseIterable {
        case readable, families
        case windowHours = "window_hours"
        case lastCallAt = "last_call_at"
    }

    /// What an unreadable payload says: nothing.
    public static let none = HarnessActivity(
        readable: false, windowHours: 0, lastCallAt: nil, families: [])

    public init(readable: Bool, windowHours: Int, lastCallAt: Date?, families: [HarnessFamilyActivity]) {
        self.readable = readable
        self.windowHours = windowHours
        self.lastCallAt = lastCallAt
        self.families = families
    }
}

/// What the calls answered on this computer have cost today.
///
/// `known` false is "nobody could measure this", which is NOT a day with
/// nothing on it. The two must never be drawn the same way, which is why
/// this is a block with a flag rather than a bare number defaulting to zero.
public struct HarnessSpend: Decodable, Equatable, Sendable {
    public let known: Bool
    /// Millionths of a dollar. Absent whenever `known` is false, and
    /// treated as absent even when it is not: a claimed figure with no
    /// number is a contradiction, and the safe reading of it is that
    /// nothing was measured.
    public let micros: UInt64?

    public enum CodingKeys: String, CodingKey, CaseIterable {
        case known, micros
    }

    /// What a payload with no spend block says: nothing.
    public static let none = HarnessSpend(known: false, micros: nil)

    public init(known: Bool, micros: UInt64?) {
        self.known = known
        self.micros = micros
    }

    /// The number to hand the shared sentence, in the convention that
    /// sentence uses.
    ///
    /// ABSENCE IS OUT OF RANGE, never zero. A shell that passed `0` for a
    /// figure nobody measured would get back `$0.00`, which is the one
    /// rendering this whole block exists to prevent.
    public var abiValue: Int64 {
        guard known, let micros, let value = Int64(exactly: micros) else { return -1 }
        return value
    }
}

/// The whole `harness_list` answer.
public struct HarnessList: Decodable, Equatable, Sendable {
    /// A fact about this build, not about the machine. False means the list
    /// is the tools compiled in, and says nothing about every other tool
    /// that exists -- which is what the payload's own scope sentence is for.
    public let catalogPresent: Bool
    public let harnesses: [HarnessRow]
    public let activity: HarnessActivity
    /// What today's calls cost, or the absence that must not read as zero.
    ///
    /// Defaulted rather than required, so a daemon older than the release
    /// that reports it leaves the amount unknown instead of taking the whole
    /// tool list down with it -- the same reasoning `HarnessActivity.none`
    /// carries, and the safe direction here as well.
    public let spend: HarnessSpend
    /// The port a connect would write. Nil when nothing here answers model
    /// calls, which is what the daemon refuses a connect with.
    public let destinationPort: UInt16?
    /// Whether the destination holds a key of its own, as a TRI-STATE.
    ///
    /// `nil` is a daemon that does not report the field, and it is a third
    /// value rather than a flavour of `false`. A daemon that predates the
    /// credential gate connects tools perfectly well, and a destination the
    /// contributor runs themselves reports `true`; telling either of them to
    /// sign in first would be false. `false` alone is the refused connect.
    public let destinationCredentialed: Bool?

    public enum CodingKeys: String, CodingKey, CaseIterable {
        case harnesses, activity, spend
        case catalogPresent = "catalog_present"
        case destinationPort = "destination_port"
        case destinationCredentialed = "destination_credentialed"
    }

    /// What a payload this build cannot read says: nothing about any tool.
    public static let none = HarnessList(
        catalogPresent: false, harnesses: [], activity: .none, spend: .none,
        destinationPort: nil, destinationCredentialed: nil)

    public init(
        catalogPresent: Bool, harnesses: [HarnessRow], activity: HarnessActivity,
        spend: HarnessSpend = .none,
        destinationPort: UInt16?,
        destinationCredentialed: Bool? = nil
    ) {
        self.catalogPresent = catalogPresent
        self.harnesses = harnesses
        self.activity = activity
        self.spend = spend
        self.destinationPort = destinationPort
        self.destinationCredentialed = destinationCredentialed
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        catalogPresent = try c.decode(Bool.self, forKey: .catalogPresent)
        harnesses = try c.decode([HarnessRow].self, forKey: .harnesses)
        activity = try c.decode(HarnessActivity.self, forKey: .activity)
        spend = (try? c.decode(HarnessSpend.self, forKey: .spend)) ?? .none
        destinationPort = try c.decodeIfPresent(UInt16.self, forKey: .destinationPort)
        // `decodeIfPresent`, so an absent field stays absent. Decoding it as
        // a plain `Bool` with a `false` default is the one mistake this
        // field exists to make impossible.
        destinationCredentialed = try c.decodeIfPresent(
            Bool.self, forKey: .destinationCredentialed)
    }

    /// The tri-state on the wire the ABI reads it in: any negative value is
    /// the absent field, `0` false, `1` true. Absence is deliberately NOT
    /// zero, the way `HarnessSpend.abiValue` keeps an unmeasured amount out
    /// of range rather than passing it as `$0.00`.
    public static func credentialedABIValue(_ credentialed: Bool?) -> Int32 {
        guard let credentialed else { return -1 }
        return credentialed ? 1 : 0
    }

    /// This list's own answer, in that convention.
    public var credentialedABIValue: Int32 {
        Self.credentialedABIValue(destinationCredentialed)
    }
}

/// One slot a plan refused to take over, and what the contributor has in it.
public struct HarnessOccupied: Decodable, Equatable, Sendable {
    public let slot: String
    public let current: String
}

/// An edit that has been worked out and not made.
///
/// `occupied` is NOT folded into `outcome`, and the separation is the point:
/// one pass can fill two empty slots and leave a third alone, so a plan may
/// carry changes and occupied slots at once.
public struct HarnessPlan: Decodable, Equatable, Sendable {
    public let id: String
    public let action: String
    /// The daemon's own label, handed to the shared table rather than
    /// matched on here.
    public let outcome: String
    /// Minted by the daemon for a committable plan and for nothing else.
    /// This shell cannot construct one, which is what stops it from
    /// constructing a write.
    public let planID: String?
    public let path: String?
    /// IronWire's own words for what would change. Rendered verbatim; they
    /// are already phrased for a reader.
    public let changes: [String]
    public let occupied: [HarnessOccupied]

    public enum CodingKeys: String, CodingKey, CaseIterable {
        case id, action, outcome, path, changes, occupied
        case planID = "plan_id"
    }
}

/// What a commit actually did.
public struct HarnessCommit: Decodable, Equatable, Sendable {
    public let id: String
    public let action: String
    public let committed: Bool
    public let path: String?
    /// The file as it was before this app ever touched it. Written once and
    /// never overwritten, so this is not a fresh copy per change and must
    /// not be described as one.
    public let backupPath: String?

    public enum CodingKeys: String, CodingKey, CaseIterable {
        case id, action, committed, path
        case backupPath = "backup_path"
    }
}

/// The two things a contributor can ask for, one tool at a time.
public enum HarnessAction: String, Sendable {
    case connect
    case disconnect
}

/// The state of one tool, decoded from `TC_HARNESS_STATE_*`.
///
/// The arms are spelled out rather than derived from declaration order, and
/// anything unknown is `.unknown`. That is the safe direction here because
/// the dangerous value is `.answering`: it claims a call actually arrived,
/// and a state a later daemon grows must never be drawn as one.
public enum HarnessState: Equatable, Sendable {
    case unknown
    case notConnected
    case connectedNoCalls
    case answering
    /// A call arrived in this tool's protocol family and more than one
    /// connected tool speaks it, so it cannot be attributed to either.
    /// Its own value, not a flavour of `.answering`.
    case activityShared

    public static func fromABI(_ value: Int32) -> HarnessState {
        switch value {
        case 31: return .notConnected
        case 32: return .connectedNoCalls
        case 33: return .answering
        case 34: return .activityShared
        default: return .unknown
        }
    }
}

/// What planning an edit turned out to be, decoded from `TC_HARNESS_PLAN_*`.
public enum HarnessPlanOutcome: Equatable, Sendable {
    case unknown
    case changes
    case noop
    /// The file could not be read, so it was refused rather than rewritten.
    /// Distinct from `.noop` on purpose: nothing was decided and the file
    /// needs a human.
    case unparseable
    case notInstalled
    case entryUnusable
    case noConfigPath

    public static func fromABI(_ value: Int32) -> HarnessPlanOutcome {
        switch value {
        case 41: return .changes
        case 42: return .noop
        case 43: return .unparseable
        case 44: return .notInstalled
        case 45: return .entryUnusable
        case 46: return .noConfigPath
        default: return .unknown
        }
    }
}

/// The shared tables and sentences this surface reads across the C ABI,
/// injected so `TCShellCore` can be tested without linking the dylib.
///
/// `stateLine` and `lastCallLine` are here for the reason the branch tables
/// are: the sentence a state gets is one decision, and three shells choosing
/// between the payload's fields is three copies of it. Production wiring is
/// `TCHarness`; see `AppModel`.
public struct HarnessCalls: Sendable {
    public let stateCode: @Sendable (String) -> Int32
    public let planOutcomeCode: @Sendable (String) -> Int32
    public let actionAvailable: @Sendable (String, Bool, Bool) -> Bool
    /// The sentence for one row's `state` label, or the empty string for the
    /// two states that must claim nothing.
    public let stateLine: @Sendable (String) -> String
    /// The when-line, for a number of seconds. Negative means nothing to
    /// report, and answers the empty string.
    public let lastCallLine: @Sendable (Int64) -> String
    /// The sentence a plan's outcome carries, or the empty string for
    /// `changes`, whose changes are shown instead.
    public let outcomeLine: @Sendable (String) -> String
    /// The amount sentence, for a number of millionths of a dollar.
    /// Negative means not known, and answers the empty string -- never a
    /// zero.
    public let spendLine: @Sendable (Int64) -> String

    public init(
        stateCode: @escaping @Sendable (String) -> Int32,
        planOutcomeCode: @escaping @Sendable (String) -> Int32,
        actionAvailable: @escaping @Sendable (String, Bool, Bool) -> Bool,
        stateLine: @escaping @Sendable (String) -> String,
        lastCallLine: @escaping @Sendable (Int64) -> String,
        outcomeLine: @escaping @Sendable (String) -> String,
        spendLine: @escaping @Sendable (Int64) -> String
    ) {
        self.stateCode = stateCode
        self.planOutcomeCode = planOutcomeCode
        self.actionAvailable = actionAvailable
        self.stateLine = stateLine
        self.lastCallLine = lastCallLine
        self.outcomeLine = outcomeLine
        self.spendLine = spendLine
    }
}

/// What this shell renders about the tools on this machine.
///
/// Holds no words. Every sentence it hands back is a field of
/// `PrivateInferenceCopy`, and every branch it takes is the shared table's.
public enum HarnessSurface {
    // MARK: - Decoding

    /// A payload this build cannot read is "no evidence about any tool",
    /// never a verdict about one. All or nothing: a half-decoded list would
    /// show some tools and silently omit others, and the omitted one is the
    /// one the contributor came to look at.
    public static func list(fromJSON json: String) -> HarnessList {
        guard let data = json.data(using: .utf8),
            let list = try? decoder().decode(HarnessList.self, from: data)
        else { return .none }
        return list
    }

    public static func plan(fromJSON json: String) -> HarnessPlan? {
        guard let data = json.data(using: .utf8) else { return nil }
        return try? decoder().decode(HarnessPlan.self, from: data)
    }

    public static func commit(fromJSON json: String) -> HarnessCommit? {
        guard let data = json.data(using: .utf8) else { return nil }
        return try? decoder().decode(HarnessCommit.self, from: data)
    }

    /// Both RFC 3339 shapes, because the daemon emits fractional seconds and
    /// `.iso8601` alone refuses them -- which would turn one unparseable
    /// timestamp into an empty tool list.
    private static func decoder() -> JSONDecoder {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .custom { decoder in
            let text = try decoder.singleValueContainer().decode(String.self)
            // Built inside the closure rather than captured: the formatter is
            // not `Sendable`, and this is called once per timestamp on a list
            // of two.
            let withFraction = ISO8601DateFormatter()
            withFraction.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
            if let date = withFraction.date(from: text) { return date }
            let plain = ISO8601DateFormatter()
            plain.formatOptions = [.withInternetDateTime]
            if let date = plain.date(from: text) { return date }
            throw DecodingError.dataCorrupted(
                .init(codingPath: decoder.codingPath, debugDescription: "unparseable timestamp"))
        }
        return decoder
    }

    // MARK: - The state of one row

    public static func state(_ label: String, calls: HarnessCalls) -> HarnessState {
        HarnessState.fromABI(calls.stateCode(label))
    }

    public static func state(_ row: HarnessRow, calls: HarnessCalls) -> HarnessState {
        state(row.state, calls: calls)
    }

    /// The sentence for one state, or nothing at all.
    ///
    /// ONE TABLE, ASKED. This used to be a `switch` here over
    /// `PrivateInferenceCopy`'s fields, and the Windows and GNOME shells each
    /// held their own; three copies of one decision, agreeing today and
    /// drifting in silence tomorrow. The label goes to the shared table and
    /// the sentence comes back. It takes the LABEL, not the decoded state,
    /// so a state a later daemon grows never has to be spelled in Swift
    /// before it can be shown.
    ///
    /// `activity_shared` and `unknown` answer nil, and the nil is the point.
    /// Neither has a sentence, and neither may borrow one: the shared case
    /// would have to claim either that a call arrived from this tool -- which
    /// is exactly what cannot be attributed -- or that none did, which is
    /// false. A row with no state line claims nothing, and claiming nothing
    /// is the honest answer to a question the ledger cannot settle.
    public static func stateSentence(_ label: String, calls: HarnessCalls) -> String? {
        let sentence = calls.stateLine(label)
        return sentence.isEmpty ? nil : sentence
    }

    public static func stateSentence(_ row: HarnessRow, calls: HarnessCalls) -> String? {
        stateSentence(row.state, calls: calls)
    }

    /// The sentence one ROW shows, which is not always its state's.
    ///
    /// A tool that is not on this machine gets the missing-tool sentence and
    /// nothing else. Both halves matter. The row is LISTED rather than
    /// hidden, because a tool left out cannot be told apart from a tool this
    /// app was never taught about; and it may not keep the not-connected
    /// sentence, which says a tool's own settings still send its calls
    /// wherever they went before -- a claim about the settings of something
    /// that is not here. Before this the two rendered identically, with the
    /// connect button simply absent and nothing saying why.
    public static func rowSentence(
        _ row: HarnessRow, copy: PrivateInferenceCopy, calls: HarnessCalls
    ) -> String? {
        guard row.installed else { return copy.harnessNotInstalled }
        return stateSentence(row.state, calls: calls)
    }

    /// When the last call from this tool was answered here, or nothing.
    ///
    /// Assembled on the far side, like the state sentence: this shell works
    /// out how many seconds ago and hands the number over. An absent
    /// timestamp, and one dated in the future, both cross as a negative
    /// number, which is the shared convention for "nothing to report" and
    /// comes back empty.
    ///
    /// Drawn as no line at all when nil. The state sentence above it has
    /// already said the part that is true.
    public static func lastCallSentence(
        _ row: HarnessRow, now: Date = Date(), calls: HarnessCalls
    ) -> String? {
        guard let at = row.lastCallAt else { return nil }
        let elapsed = now.timeIntervalSince(at)
        guard elapsed >= 0, elapsed.isFinite, elapsed < Double(Int64.max) else { return nil }
        let sentence = calls.lastCallLine(Int64(elapsed))
        return sentence.isEmpty ? nil : sentence
    }

    /// What the calls answered on this computer cost today, or nothing.
    ///
    /// Assembled on the far side, like the state sentence and the when-line:
    /// this shell hands over a number and renders whatever comes back. The
    /// amount, its rounding and its window are all decided there.
    ///
    /// NIL WHEN THE FIGURE IS NOT KNOWN, and nil draws no line at all --
    /// which is the whole point. A day nobody could measure and a day with
    /// nothing on it are different facts, and the second says so in words.
    /// `HarnessSpend.abiValue` is what keeps the first out of range on the
    /// way across.
    public static func spendSentence(_ list: HarnessList, calls: HarnessCalls) -> String? {
        let sentence = calls.spendLine(list.spend.abiValue)
        return sentence.isEmpty ? nil : sentence
    }

    /// The sentence about the copy of the tool that is still running, or
    /// none.
    ///
    /// Shown while a tool's file sends its calls here and no call has been
    /// attributed to it, and taken away the moment one is -- which is what
    /// the payload's own wording asks for. The window in front of the
    /// contributor is a process that read its settings when it started, and
    /// a list claiming a tool sends its calls here while that window does
    /// not is the failure this whole destination exists to stop.
    public static func restartSentence(
        _ row: HarnessRow, state: HarnessState, copy: PrivateInferenceCopy
    ) -> String? {
        guard row.connected, state != .answering else { return nil }
        return copy.harnessNeedsRestart
    }

    /// How firmly that sentence reads.
    ///
    /// `.clear` for `.answering` and for nothing else. `PrivateInferenceTone`
    /// is reused rather than duplicated so `readsAsWorking` stays one rule on
    /// this destination: a tool is painted as working when a call arrived and
    /// could only have come from it.
    public static func tone(_ state: HarnessState) -> PrivateInferenceTone {
        state == .answering ? .clear : .neutral
    }

    // MARK: - The actions offered on a row

    /// The daemon's own answer, not this shell's re-derivation.
    ///
    /// It must be the daemon's, because the two questions differ. `connected`
    /// on the row is narrowed to "names OUR destination port"; the daemon
    /// computes `can_disconnect` from the broader `wired` -- "names any local
    /// proxy" -- precisely so a line pointing at a stale or foreign port
    /// still has a control that removes it. `wired` is not on the wire, so
    /// re-deriving from `connected` here silently answers false for exactly
    /// those rows: the button becomes Connect, the daemon refuses it as a
    /// no-op, and the contributor is told their file "already says what this
    /// would have written" about a file naming somebody else's port, with no
    /// route to the disconnect that would fix it.
    public static func canConnect(_ row: HarnessRow, calls: HarnessCalls) -> Bool {
        row.canConnect
    }

    public static func canDisconnect(_ row: HarnessRow, calls: HarnessCalls) -> Bool {
        row.canDisconnect
    }

    /// The action a row's one button would take, or nothing to offer.
    public static func action(_ row: HarnessRow, calls: HarnessCalls) -> HarnessAction? {
        if canDisconnect(row, calls: calls) { return .disconnect }
        if canConnect(row, calls: calls) { return .connect }
        return nil
    }

    /// That button's words, from the payload.
    public static func actionLabel(_ action: HarnessAction, copy: PrivateInferenceCopy) -> String {
        switch action {
        case .connect: return copy.harnessConnect
        case .disconnect: return copy.harnessDisconnect
        }
    }

    // MARK: - The plan, and the preview it feeds

    /// `harness_plan` names a tool and an action. It never names a file, a
    /// port or a value: what gets written is worked out on the far side.
    public static func planParams(id: String, action: HarnessAction) -> [String: Any] {
        ["id": id, "action": action.rawValue]
    }

    public static func outcome(_ plan: HarnessPlan, calls: HarnessCalls) -> HarnessPlanOutcome {
        HarnessPlanOutcome.fromABI(calls.planOutcomeCode(plan.outcome))
    }

    /// Whether the confirm button may appear at all.
    ///
    /// Two conditions, and both are required. The outcome must be the one
    /// committable outcome, and the daemon must have minted an id -- this
    /// shell has no way to make one, which is what stops it from writing
    /// anything the contributor was not shown.
    public static func canCommit(_ plan: HarnessPlan, calls: HarnessCalls) -> Bool {
        guard let planID = plan.planID, !planID.isEmpty else { return false }
        return outcome(plan, calls: calls) == .changes
    }

    /// The sentence the preview carries about the outcome itself, or none.
    ///
    /// ONE TABLE, ASKED. This used to be a `== .unparseable` branch here
    /// choosing one of the payload's fields, and Windows and GNOME each held
    /// the same arm; the four other non-committable outcomes had no sentence
    /// at all, so their preview held a title, a path, no changes, no
    /// explanation and a way out. The LABEL goes to the shared table and the
    /// sentence comes back, so an outcome a later daemon grows never has to
    /// be spelled in Swift before it can be shown.
    ///
    /// `changes` answers nil, and the nil is the point: the preview shows the
    /// changes themselves, and a sentence above them announcing that there
    /// are changes is this app narrating its own list.
    public static func outcomeSentence(_ plan: HarnessPlan, calls: HarnessCalls) -> String? {
        let sentence = calls.outcomeLine(plan.outcome)
        return sentence.isEmpty ? nil : sentence
    }

    /// The words over the occupied slots. They report what was left alone
    /// and stop there -- there is no take-it-over anywhere on this surface.
    public static func occupiedSentence(copy: PrivateInferenceCopy) -> String {
        copy.harnessSlotTaken
    }

    // MARK: - The commit

    /// `harness_commit` takes the minted id and nothing else.
    public static func commitParams(planID: String) -> [String: Any] {
        ["plan_id": planID]
    }

    /// Whether a plan id is spent after a commit that did not succeed.
    ///
    /// It always is, and that is a fact about the daemon rather than a
    /// judgement made here: the plan is taken out of the store before the
    /// digest is re-checked and before the write is attempted, so expired,
    /// already used, never minted, moved-underneath and write-failed all
    /// leave nothing to commit again. Every one of them is therefore the same
    /// instruction -- plan again and show the contributor the new result --
    /// and a shell that re-sent the id would be trying to write something
    /// nobody had been shown.
    ///
    /// Spelled out rather than left implicit because a preview sheet with a
    /// confirm button on it is exactly where a retry gets added.
    public static func planIsSpent(afterCommitFailure code: String) -> Bool {
        _ = code
        return true
    }

    /// A connect asked for while nothing here answers model calls. The
    /// daemon refuses it rather than writing a config that names no port.
    public static func isNoDestination(_ code: String) -> Bool {
        code == "harness-no-destination"
    }

    /// What a contributor is told when a change did not go through. The
    /// payload's, and the same sentence the switch's own failed write uses:
    /// the change was not made, and the thing to do is look and try again.
    public static func commitFailureSentence(copy: PrivateInferenceCopy) -> String {
        copy.writeUnconfirmed
    }

    // MARK: - The exposure gate

    /// Whether a connect must put the exposure question first.
    ///
    /// While nothing here answers model calls, connecting a tool starts a
    /// listener that is open to EVERYTHING on this machine -- which does not
    /// follow from "connect this one tool", and is the whole reason the
    /// question exists. So the gate is the listener's own state, and it is
    /// deliberately wider than `tc_private_inference_should_offer`: that
    /// stops asking once the question has been answered, while this asks
    /// again whenever a connect would have to reopen the listener. A
    /// contributor who turned it off on purpose is making the decision
    /// afresh, and is owed the words afresh.
    public static func connectNeedsExposure(listenerOn: Bool) -> Bool { !listenerOn }

    /// The `set_settings` body for one answer to that question.
    ///
    /// Delegated rather than restated: declining writes the marker ALONE,
    /// and that rule lives in one place.
    public static func exposureParams(accepted: Bool) -> [String: Any] {
        PrivateInferenceSurface.offerParams(accepted: accepted)
    }
}
