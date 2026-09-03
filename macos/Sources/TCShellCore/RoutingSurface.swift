import Foundation

/// The routing surface's state machine: which word each state reaches for.
///
/// **Nothing in this file is a word.** Every string a contributor reads
/// arrives on a `RoutingCopy` decoded from
/// `trace_commons_contributor::routing_copy`, or is a sentence that crate
/// assembled and handed across the ABI. What lives here is the mapping --
/// which field, for which state -- because that is logic and not wording,
/// and because it can be tested in a target that does not link the dylib.
///
/// The literals below are wire values, not display text: the daemon's own
/// `outcome` strings, its three routing states, and IronWire's stable tool
/// ids. They are spelled here for the same reason `DaemonClient`'s method
/// names are spelled there -- they are the protocol, and the daemon answers
/// `bad_params` rather than rendering them.

// MARK: - The daemon's probe answer

/// What a probe of the declared proxy answered, in the three shapes it can.
///
/// Deliberately not a boolean, and deliberately carrying the path and the
/// port: those are the two facts that make a failure fixable, and they come
/// from the daemon rather than from anything this shell guessed.
public enum RoutingProbeOutcome: Equatable, Sendable {
    /// The proxy answered and the credential was accepted.
    case reachable
    /// The credential file could not be read, or was read and refused.
    /// Carries the absolute path the daemon reported -- **absent, not
    /// null**, when nothing resolved at all, which is a different sentence.
    ///
    /// This is the likely macOS failure rather than an exotic one: a
    /// GUI-launched daemon inherits no login-shell environment, so it never
    /// sees `$IRONWIRE_HOME` and reads `~/.ironwire` whatever a profile says.
    case tokenUnusable(path: String?)
    /// Nothing usable answered. Carries the port that was tried.
    case unreachable(port: UInt16?)
    /// An answer this build cannot read. Claims nothing about the proxy in
    /// either direction, and must not send anybody to check a port or a file
    /// that is fine.
    case unknown

    /// The daemon's own spellings, from `daemon::ipc`'s `PROBE_*`.
    enum Wire {
        static let reachable = "reachable"
        static let tokenUnreadable = "token_unreadable"
        static let unreachable = "unreachable"
    }

    /// Read a `probe_routing` or `probe_routed_tools` result.
    ///
    /// Both calls answer in the same vocabulary, deliberately: it is the
    /// same connection to the same proxy with the same credential, and a
    /// caller that reads one must not have to learn a second set of words.
    public static func parse(_ result: [String: Any]) -> RoutingProbeOutcome {
        switch result["outcome"] as? String {
        case Wire.reachable:
            return .reachable
        case Wire.tokenUnreadable:
            return .tokenUnusable(path: result["token_path"] as? String)
        case Wire.unreachable:
            let port = (result["port"] as? NSNumber)
                .map(\.intValue)
                .flatMap { UInt16(exactly: $0) }
            return .unreachable(port: port)
        default:
            return .unknown
        }
    }
}

/// What IronWire said about one tool, as far as a word may be built on it.
public struct RoutingToolRow: Equatable, Sendable {
    public let installed: Bool
    public let wired: Bool

    public init(installed: Bool, wired: Bool) {
        self.installed = installed
        self.wired = wired
    }
}

/// What IronWire last answered when asked which tools are pointed at it.
///
/// `outcome` is what makes a dead proxy stop producing verdicts: on
/// anything but `.reachable` every tool reads as not known, whatever this
/// app's own switch says.
public struct RoutingEvidence: Equatable, Sendable {
    public let outcome: RoutingProbeOutcome
    /// One entry per tool IronWire listed, keyed by its own stable id. A
    /// tool absent from the list -- Gemini CLI and Cline on every machine
    /// today -- is not in this map and gets no verdict.
    public let tools: [String: RoutingToolRow]

    public init(outcome: RoutingProbeOutcome, tools: [String: RoutingToolRow]) {
        self.outcome = outcome
        self.tools = tools
    }

    /// Read a `probe_routed_tools` result.
    ///
    /// Anything unreadable degrades to no evidence rather than to a default:
    /// a missing `wired` is not a claim that a tool is wired, and a row
    /// without an id is not a row.
    public static func parse(_ result: [String: Any]) -> RoutingEvidence {
        var tools: [String: RoutingToolRow] = [:]
        for row in result["tools"] as? [[String: Any]] ?? [] {
            guard let id = row["id"] as? String, !id.isEmpty else { continue }
            tools[id] = RoutingToolRow(
                installed: row["installed"] as? Bool ?? false,
                wired: row["wired"] as? Bool ?? false
            )
        }
        return RoutingEvidence(outcome: RoutingProbeOutcome.parse(result), tools: tools)
    }

    /// What may be said about one tool.
    ///
    /// * **Nothing answered.** `unreachable` and `token_unreadable` are
    ///   stable states, so a word built on them would keep asserting while
    ///   the card underneath says nothing answered. They yield `.unknown`.
    /// * **Listed but not present.** IronWire saying a tool is not
    ///   installed, while this app is watching that tool's sessions, is two
    ///   detectors disagreeing about one machine -- not evidence.
    public func wiring(forToolID id: String) -> RoutingToolWiring {
        guard outcome == .reachable else { return .unknown }
        switch tools[id] {
        case .some(let row) where row.wired: return .wired
        case .some(let row) where row.installed: return .notWired
        default: return .unknown
        }
    }
}

/// Three states, not a boolean. The missing third state is the whole defect
/// this surface was rebuilt to remove: a dead proxy and an unlisted tool
/// both used to render as a confident verdict.
public enum RoutingToolWiring: Equatable, Sendable {
    case wired
    case notWired
    case unknown

    /// This state as the C ABI spells it: `TC_TOOL_WIRING_*`.
    ///
    /// A wire value, like the daemon's outcome strings above, and the reason
    /// it is here rather than in the bridge: the bridge deals in pointers,
    /// and the numbering is part of the contract this file is written
    /// against. Pinned in `RoutingSurfaceExportTests` against the real dylib,
    /// because a renumbering here would send "wired" across as "not wired" --
    /// a wrong verdict on a privacy claim, not a crash.
    public var abiValue: Int32 {
        switch self {
        case .wired: return 0
        case .notWired: return 1
        case .unknown: return 2
        }
    }
}

/// What the contributor said about each tool's sessions, from
/// `get_settings`'s `*_source_mode`.
///
/// Not the routing declaration. The declaration switch is **not** an input
/// to any tool word -- it was the only input before, and that is what let a
/// contributor read the wired word on the same card as "nothing answered".
public struct RoutingSourceModes: Equatable, Sendable {
    public let claude: String
    public let codex: String
    public let gemini: String
    public let cline: String

    public init(claude: String, codex: String, gemini: String, cline: String) {
        self.claude = claude
        self.codex = codex
        self.gemini = gemini
        self.cline = cline
    }

    /// A daemon that answered nothing about a source is watching the
    /// conventional location, which is a tool in use.
    public static let unset = RoutingSourceModes(
        claude: "unset", codex: "unset", gemini: "unset", cline: "unset"
    )
}

/// One rendered row: the tool's name, its one word, and how that word is
/// painted. All three come from the shared source.
///
/// The tone travels with the word because both are decided by the same branch
/// table, from the same two inputs. A view takes it from here and never
/// re-derives it from `word`: that would be a text comparison against a
/// privacy claim, and `Private` is a substring of the denial that must never
/// come back.
public struct RoutingToolWord: Equatable, Sendable {
    public let name: String
    public let word: String
    public let tone: RoutingTone
}

/// How a line or a word is painted. Named rather than valued so this target
/// stays free of AppKit; the view maps these onto its own tokens.
public enum RoutingTone: Equatable, Sendable {
    /// Says nothing either way.
    case neutral
    /// True and fine, but not yet an answer.
    case held
    /// The reassuring reading.
    case clear

    /// A tone as the ABI answers it: `TC_ROUTING_TONE_*`.
    ///
    /// One numbering serves both the tool words and the daemon's state, so
    /// this is the only decoder. Anything this build does not know is
    /// `.neutral`, the tone that claims nothing. Spelled out rather than
    /// derived from this enum's declaration order, which is a Swift detail
    /// and not the contract.
    public static func fromABI(_ value: Int32) -> RoutingTone {
        switch value {
        case 1: return .held
        case 2: return .clear
        default: return .neutral
        }
    }
}

/// The sentences that cannot be finished without an argument, injected
/// rather than imported.
///
/// They are assembled in Rust and cross the ABI already finished --
/// `TCBridge` supplies these two closures in the app. This target does not
/// link the dylib, which is why they arrive as values: a template filled in
/// on this side would be a fourth place the wording could drift.
///
/// Each returns nil when the ABI would not produce a sentence, which is what
/// a caught panic looks like from here.
public struct RoutingCalls: Sendable {
    public let tokenLine: @Sendable (String?) -> String?
    public let unreachableLine: @Sendable (UInt16?) -> String?
    /// What discovery found, in one sentence. `nil` port is the machine
    /// that published no pointer -- the ordinary machine, and not an error.
    public let discoveryLine: @Sendable (UInt16?) -> String?

    /// Which of the four words a tool gets, from its source mode and
    /// `RoutingToolWiring.abiValue`.
    ///
    /// THE BRANCH TABLE CROSSES, NOT ONLY THE WORDS. This used to be a
    /// `switch` in this file, beside an identical one in C# and a third in
    /// Rust. Every string all three returned was the same shared field, so
    /// the words could not drift -- but the branching could, in three places,
    /// and the three-shell test that proves the wording is shared would not
    /// have caught it.
    public let toolWord: @Sendable (String, Int32) -> String?

    /// How that word is painted, from the same two inputs:
    /// `TC_ROUTING_TONE_*`. Never nil, because a styling call that failed
    /// would leave this shell choosing a tone for itself.
    public let toolTone: @Sendable (String, Int32) -> Int32

    /// The daemon's state, in words. Crosses for the reason `toolWord` does.
    public let stateLine: @Sendable (String) -> String?

    /// How firmly that sentence reads. The last routing branch table that
    /// was still a `switch` in this file; it crosses for the same reason.
    public let stateTone: @Sendable (String) -> Int32

    public init(
        tokenLine: @escaping @Sendable (String?) -> String?,
        unreachableLine: @escaping @Sendable (UInt16?) -> String?,
        discoveryLine: @escaping @Sendable (UInt16?) -> String?,
        toolWord: @escaping @Sendable (String, Int32) -> String?,
        toolTone: @escaping @Sendable (String, Int32) -> Int32,
        stateLine: @escaping @Sendable (String) -> String?,
        stateTone: @escaping @Sendable (String) -> Int32
    ) {
        self.tokenLine = tokenLine
        self.unreachableLine = unreachableLine
        self.discoveryLine = discoveryLine
        self.toolWord = toolWord
        self.toolTone = toolTone
        self.stateLine = stateLine
        self.stateTone = stateTone
    }
}

/// What a running IronWire published about itself, as `discover_routing`
/// answers.
///
/// # Nothing here is a failure
///
/// `discover_routing` answers `{"found": false}` for every reason there is
/// nothing to read -- never installed, not running, a version that
/// publishes no pointer, a pointer this reader will not act on -- and they
/// are one state here for the same reason they are one boolean there: they
/// are one fact to the contributor and one next step. A vocabulary of
/// outcomes would invite a shell to match on one, and this screen has
/// already been bitten once by a word that is a prefix of another.
///
/// # It carries no token
///
/// `tokenPath` is a path the daemon reported, for display beside the port.
/// The credential at it is opened by the daemon, at call time. Nothing on
/// this type has ever held one.
public struct RoutingDiscovery: Equatable, Sendable {
    /// The loopback port IronWire published, or nil for nothing found.
    public let port: UInt16?
    /// Where IronWire said it wrote its credential, when it said.
    public let tokenPath: String?

    public init(port: UInt16?, tokenPath: String?) {
        self.port = port
        self.tokenPath = tokenPath
    }

    /// The state of a machine that published nothing.
    public static let none = RoutingDiscovery(port: nil, tokenPath: nil)

    /// Whether there is anything to offer.
    public var found: Bool { port != nil }

    /// Read a `discover_routing` result.
    ///
    /// `found` without a usable port is nothing found: a port is the fact
    /// the whole call exists to supply, and offering to connect to a port
    /// this shell invented would be worse than asking.
    public static func parse(_ result: [String: Any]) -> RoutingDiscovery {
        guard result["found"] as? Bool == true else { return .none }
        let port = (result["port"] as? NSNumber)
            .map(\.intValue)
            .flatMap { UInt16(exactly: $0) }
            .flatMap { $0 > 0 ? $0 : nil }
        guard let port else { return .none }
        return RoutingDiscovery(
            port: port,
            tokenPath: (result["token_path"] as? String).flatMap { $0.isEmpty ? nil : $0 }
        )
    }
}

// MARK: - The declaration

/// The three controls, as the window holds them.
///
/// `port` is what is *shown*. Showing the conventional number so nobody has
/// to know it is not the same as declaring it, and `settingsParams` is the
/// only thing that turns this into a write.
public struct RoutingForm: Equatable, Sendable {
    public var on: Bool
    public var port: UInt16
    public var tokenDir: String

    public init(on: Bool, port: UInt16, tokenDir: String) {
        self.on = on
        self.port = port
        self.tokenDir = tokenDir
    }

    /// IronWire's conventional port, shown in the field so nobody has to
    /// know it. **Shown is not declared**: nothing is written until the
    /// contributor turns the switch on, because absence means off with no
    /// fallback, and a displayed default that wrote itself would have this
    /// window announce a local service nobody mentioned.
    public static let conventionalPort: UInt16 = 8463

    /// The daemon's `ironwire` declaration, or its absence, as fields.
    ///
    /// `mode` is `watch`, `off`, or nil for nothing declared. Only `watch`
    /// is on; the other two show the conventional port without declaring it.
    /// `discoveredPort` is what `discover_routing` reported, and it is
    /// third in line on purpose.
    ///
    /// **The contributor's declared port always wins.** A declared port is
    /// a human instruction; the pointer is a file on disk that survives the
    /// daemon that wrote it, and IronWire removes it only on a clean stop.
    /// Letting a stale pointer overwrite a declaration is not one refused
    /// connection -- it is a contributor who declared 8463, whose leftover
    /// pointer says 9000, and whose card now shows a number they never
    /// typed. `ironwire_ledger_for` refuses the same substitution on the
    /// reading side; this is the same rule on the showing side.
    ///
    /// Discovery fills only where there is nothing declared, and the
    /// conventional number is the last resort rather than the first.
    /// Filling it in is a *display*: `settingsParams` still writes nothing
    /// until the contributor acts.
    public static func fromDeclaration(
        mode: String?, port: UInt16?, tokenDir: String?, discoveredPort: UInt16? = nil
    ) -> RoutingForm {
        RoutingForm(
            on: mode == "watch",
            port: port ?? discoveredPort ?? conventionalPort,
            tokenDir: tokenDir ?? ""
        )
    }
}

// MARK: - The surface

public enum RoutingSurface {
    /// IronWire's own stable ids for the four tools this card names.
    ///
    /// `ironwire connect <id>` takes these and its settings response is
    /// keyed by them. Gemini CLI and Cline have no row upstream at all today
    /// -- neither built-in nor in the catalogue -- which is why they are
    /// named here and expected to be missing rather than left out and
    /// quietly defaulted.
    enum ToolID {
        static let claude = "claude"
        static let codex = "codex"
        static let gemini = "gemini"
        static let cline = "cline"
    }

    /// The daemon's three routing states, from `daemon::ipc`'s `ROUTING_*`.
    ///
    /// Public because the status decoder falls back to `notDeclared`, and a
    /// second spelling of that literal beside the decoder would be a place
    /// the two could disagree about what silence means.
    public enum State {
        public static let notDeclared = "not_declared"
        static let awaitingRows = "awaiting_rows"
        static let rowsSeen = "rows_seen"
    }

    /// The `set_settings` key. That call refuses an object holding a key it
    /// does not recognise, so a drift here is a silent no-write.
    static let settingsKey = "ironwire"

    // MARK: The probe result

    /// One outcome, one sentence.
    ///
    /// A sentence the ABI would not assemble degrades to the
    /// claims-nothing line, never to a half-sentence and never to wording
    /// this shell invented.
    public static func probeLine(
        _ outcome: RoutingProbeOutcome, copy: RoutingCopy, calls: RoutingCalls
    ) -> String {
        switch outcome {
        case .reachable:
            return copy.probeReachable
        case .tokenUnusable(let path):
            return calls.tokenLine(path) ?? copy.checkUnavailable
        case .unreachable(let port):
            return calls.unreachableLine(port) ?? copy.checkUnavailable
        case .unknown:
            return copy.checkUnavailable
        }
    }

    // MARK: What the machine already knows

    /// The discovery sentence, or the claims-nothing line if the ABI would
    /// not assemble one.
    ///
    /// Never a half-sentence and never wording this shell invented. A
    /// machine that published nothing still gets a sentence, because it is
    /// the ordinary machine and the screen has to say what to do on it.
    public static func discoveryLine(
        _ discovery: RoutingDiscovery, copy: RoutingCopy, calls: RoutingCalls
    ) -> String {
        calls.discoveryLine(discovery.port) ?? copy.checkUnavailable
    }

    /// Whether the port and folder are offered as a disclosure rather than
    /// as two boxes to fill in.
    ///
    /// Only once discovery has supplied the port. On a machine that
    /// published nothing they are the only way to answer, so they stay
    /// where they were: this inverts the default, it does not hide the
    /// manual path.
    public static func overrideIsCollapsed(_ discovery: RoutingDiscovery) -> Bool {
        discovery.found
    }

    /// The form the connect action writes: the one on screen, turned on.
    ///
    /// Deliberately built from `form` rather than from `discovery`. What is
    /// on screen is what the contributor has been reading, discovered port
    /// and any override they opened the disclosure to type both -- and a
    /// press that wrote a different number from the one displayed would be
    /// the displayed-default defect in its worst form.
    public static func connecting(_ form: RoutingForm) -> RoutingForm {
        var next = form
        next.on = true
        return next
    }

    // MARK: The status line

    /// The daemon's three states, in words. A state this build does not know
    /// says what the off state says: it claims nothing.
    /// NOT A BRANCH TABLE HERE. Which sentence each state reaches is decided
    /// once, in `routing_copy.rs`, and crosses the ABI. A line the ABI would
    /// not produce falls back to the off line -- which is what an unknown
    /// state reads as anyway, because it claims nothing.
    public static func stateLine(
        _ state: String, copy: RoutingCopy, calls: RoutingCalls
    ) -> String {
        calls.stateLine(state) ?? copy.stateOff
    }

    /// NOT A BRANCH TABLE HERE, for the reason on `stateLine`. This was the
    /// last one in this file.
    ///
    /// `awaiting_rows` is `.held` and **not** an error tone. A reader built
    /// a moment ago starts empty by construction, so this is the state a
    /// contributor sees immediately after touching anything on this card;
    /// painting it as a fault would accuse a working proxy of being broken
    /// at exactly that moment.
    public static func tone(forState state: String, calls: RoutingCalls) -> RoutingTone {
        RoutingTone.fromABI(calls.stateTone(state))
    }

    /// Whether the "last checked" stamp says anything on this state.
    ///
    /// It is a per-process stamp on the running daemon -- never an install
    /// date, never a connected-since -- and it starts empty again every time
    /// that process comes back up. On a state that has had no answer at all
    /// there is nothing for it to report.
    public static func showsLastChecked(forState state: String, calls: RoutingCalls) -> Bool {
        tone(forState: state, calls: calls) != .neutral
    }

    // MARK: Per-tool words

    /// One tool's word, from what the contributor said about that tool's
    /// sessions and what IronWire said about that tool.
    ///
    /// NOT A BRANCH TABLE HERE, for the reason on `stateLine`. A word the ABI
    /// would not produce falls back to the one that claims nothing, never to
    /// a word chosen on this side.
    public static func toolWord(
        sourceMode: String, wiring: RoutingToolWiring, copy: RoutingCopy, calls: RoutingCalls
    ) -> String {
        calls.toolWord(sourceMode, wiring.abiValue) ?? copy.wordUnknown
    }

    /// How that word is painted, from the same two inputs.
    ///
    /// From the wiring, never from the rendered word. A comparison against
    /// the private word would be a text match on a privacy claim, and
    /// `Private` is a substring of the denial that must never come back.
    public static func toolTone(
        sourceMode: String, wiring: RoutingToolWiring, calls: RoutingCalls
    ) -> RoutingTone {
        RoutingTone.fromABI(calls.toolTone(sourceMode, wiring.abiValue))
    }

    /// All three rows, always, in one order: a missing answer is a word
    /// rather than a vanished row.
    ///
    /// `evidence` is nil when nothing has been asked yet, or when what was
    /// asked did not run. Neither is a fact about any tool.
    public static func toolRows(
        sourceModes: RoutingSourceModes,
        evidence: RoutingEvidence?,
        copy: RoutingCopy,
        calls: RoutingCalls
    ) -> [RoutingToolWord] {
        [
            (copy.toolClaude, sourceModes.claude, ToolID.claude),
            (copy.toolCodex, sourceModes.codex, ToolID.codex),
            (copy.toolGemini, sourceModes.gemini, ToolID.gemini),
            (copy.toolCline, sourceModes.cline, ToolID.cline),
        ].map { name, mode, id in
            let wiring = evidence?.wiring(forToolID: id) ?? .unknown
            return RoutingToolWord(
                name: name,
                word: toolWord(sourceMode: mode, wiring: wiring, copy: copy, calls: calls),
                tone: toolTone(sourceMode: mode, wiring: wiring, calls: calls)
            )
        }
    }

    // MARK: The declaration

    /// The one-key object `set_settings` is called with.
    ///
    /// Off is spelled `null` and not omitted: absence means off with no
    /// fallback, and the key has to be present for the daemon to see the
    /// change at all. The port in the field rides along only when the switch
    /// is on -- which is what keeps a displayed default from becoming a
    /// declaration.
    public static func settingsParams(_ form: RoutingForm) -> [String: Any] {
        guard form.on else { return [settingsKey: NSNull()] }
        var declaration: [String: Any] = ["mode": "watch", "port": Int(form.port)]
        let dir = form.tokenDir.trimmingCharacters(in: .whitespacesAndNewlines)
        if !dir.isEmpty { declaration["token_dir"] = dir }
        return [settingsKey: declaration]
    }

    /// What either probe is asked. Same rule about the empty box: the
    /// daemon refuses an empty string outright, and absence is what falls
    /// back to the conventional location.
    public static func probeParams(_ form: RoutingForm) -> [String: Any] {
        var params: [String: Any] = ["port": Int(form.port)]
        let dir = form.tokenDir.trimmingCharacters(in: .whitespacesAndNewlines)
        if !dir.isEmpty { params["token_dir"] = dir }
        return params
    }
}
