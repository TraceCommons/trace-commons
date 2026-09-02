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
    /// tool absent from the list -- Gemini CLI on every machine today -- is
    /// not in this map and gets no verdict.
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

    public init(claude: String, codex: String, gemini: String) {
        self.claude = claude
        self.codex = codex
        self.gemini = gemini
    }

    /// A daemon that answered nothing about a source is watching the
    /// conventional location, which is a tool in use.
    public static let unset = RoutingSourceModes(claude: "unset", codex: "unset", gemini: "unset")
}

/// One rendered row: the tool's name and its one word, both from the shared
/// payload.
public struct RoutingToolWord: Equatable, Sendable {
    public let name: String
    public let word: String
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
public struct RoutingSentences: Sendable {
    public let tokenLine: @Sendable (String?) -> String?
    public let unreachableLine: @Sendable (UInt16?) -> String?

    public init(
        tokenLine: @escaping @Sendable (String?) -> String?,
        unreachableLine: @escaping @Sendable (UInt16?) -> String?
    ) {
        self.tokenLine = tokenLine
        self.unreachableLine = unreachableLine
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
    public static func fromDeclaration(
        mode: String?, port: UInt16?, tokenDir: String?
    ) -> RoutingForm {
        RoutingForm(
            on: mode == "watch",
            port: port ?? conventionalPort,
            tokenDir: tokenDir ?? ""
        )
    }
}

// MARK: - The surface

public enum RoutingSurface {
    /// IronWire's own stable ids for the three tools this card names.
    ///
    /// `ironwire connect <id>` takes these and its settings response is
    /// keyed by them. Gemini CLI has no row upstream at all today -- neither
    /// built-in nor in the catalogue -- which is why it is named here and
    /// expected to be missing rather than left out and quietly defaulted.
    enum ToolID {
        static let claude = "claude"
        static let codex = "codex"
        static let gemini = "gemini"
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
        _ outcome: RoutingProbeOutcome, copy: RoutingCopy, sentences: RoutingSentences
    ) -> String {
        switch outcome {
        case .reachable:
            return copy.probeReachable
        case .tokenUnusable(let path):
            return sentences.tokenLine(path) ?? copy.checkUnavailable
        case .unreachable(let port):
            return sentences.unreachableLine(port) ?? copy.checkUnavailable
        case .unknown:
            return copy.checkUnavailable
        }
    }

    // MARK: The status line

    /// The daemon's three states, in words. A state this build does not know
    /// says what the off state says: it claims nothing.
    public static func stateLine(_ state: String, copy: RoutingCopy) -> String {
        switch state {
        case State.awaitingRows: return copy.stateWaiting
        case State.rowsSeen: return copy.stateReading
        default: return copy.stateOff
        }
    }

    /// `awaiting_rows` is `.held` and **not** an error tone. A reader built
    /// a moment ago starts empty by construction, so this is the state a
    /// contributor sees immediately after touching anything on this card;
    /// painting it as a fault would accuse a working proxy of being broken
    /// at exactly that moment.
    public static func tone(forState state: String) -> RoutingTone {
        switch state {
        case State.awaitingRows: return .held
        case State.rowsSeen: return .clear
        default: return .neutral
        }
    }

    /// Whether the "last checked" stamp says anything on this state.
    ///
    /// It is a per-process stamp on the running daemon -- never an install
    /// date, never a connected-since -- and it starts empty again every time
    /// that process comes back up. On a state that has had no answer at all
    /// there is nothing for it to report.
    public static func showsLastChecked(forState state: String) -> Bool {
        tone(forState: state) != .neutral
    }

    // MARK: Per-tool words

    /// One tool's word, from what the contributor said about that tool's
    /// sessions and what IronWire said about that tool.
    ///
    /// Only `off` means not used: `unset` watches the conventional location,
    /// which is a tool in use.
    static func toolWord(
        sourceMode: String, wiring: RoutingToolWiring, copy: RoutingCopy
    ) -> String {
        if sourceMode == "off" { return copy.wordNotUsed }
        switch wiring {
        case .wired: return copy.wordPrivate
        case .notWired: return copy.wordDirect
        case .unknown: return copy.wordUnknown
        }
    }

    /// All three rows, always, in one order: a missing answer is a word
    /// rather than a vanished row.
    ///
    /// `evidence` is nil when nothing has been asked yet, or when what was
    /// asked did not run. Neither is a fact about any tool.
    public static func toolRows(
        sourceModes: RoutingSourceModes, evidence: RoutingEvidence?, copy: RoutingCopy
    ) -> [RoutingToolWord] {
        [
            (copy.toolClaude, sourceModes.claude, ToolID.claude),
            (copy.toolCodex, sourceModes.codex, ToolID.codex),
            (copy.toolGemini, sourceModes.gemini, ToolID.gemini),
        ].map { name, mode, id in
            RoutingToolWord(
                name: name,
                word: toolWord(
                    sourceMode: mode,
                    wiring: evidence?.wiring(forToolID: id) ?? .unknown,
                    copy: copy
                )
            )
        }
    }

    /// Only the wired word is painted as reassurance. Every other word is
    /// neutral, "not used" included: that is a preference, not an outcome.
    public static func tone(forWord word: String, copy: RoutingCopy) -> RoutingTone {
        word == copy.wordPrivate ? .clear : .neutral
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
