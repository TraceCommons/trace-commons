import Foundation

/// The wire shape both `preview_request`'s immediate response and the later
/// `preview_ready` event carry -- see
/// `docs/contributor-daemon-ipc-v1_1.md`, "Scheduled previews", and
/// `crates/trace-commons-contributor/src/daemon/preview_scheduler.rs`'s
/// `PreviewOutcome::to_value`. One shape serves both call sites because the
/// daemon documents them as identical: `entry_id`, `state`, and whichever
/// fields that state carries.
///
/// Generic over `Summary` rather than decoding the redacted summary fields
/// itself: the app target already owns a decoder for `preview`'s identical
/// summary shape (`PreviewSummary` in `TraceCommonsApp/Models.swift`), and
/// this module has no reason to keep a second copy of that in step. What
/// belongs here -- and is what has rules worth being wrong about -- is the
/// state machine a shell drives on top of these four states, not the
/// payload of one of them.
public enum PreviewRequestState: String, Decodable, Equatable, Sendable {
    case queued
    case running
    case ready
    case tooLarge = "too_large"
    case failed
}

public struct PreviewRequestResult<Summary: Decodable & Equatable & Sendable>: Decodable, Equatable,
    Sendable
{
    public let entryID: String
    public let state: PreviewRequestState
    /// Present only for `.ready`. A `.tooLarge` or `.failed` result never
    /// carries one on the wire -- see `MAX_PREVIEW_SESSION_BYTES`'s doc in
    /// the scheduler for why a would-send figure is never synthesized for a
    /// refusal -- and this type does not invent one either: it decodes
    /// exactly whatever key was present, nothing more.
    public let summary: Summary?
    /// Present only for `.tooLarge`: a `stat` of the session, never an
    /// estimate.
    public let rawSessionBytes: Int?
    /// Present only for `.tooLarge`: the admission cap the daemon refused
    /// against.
    public let limitBytes: Int?
    /// Present only for `.failed`: the same fixed IPC error code the
    /// blocking `preview` method would have returned.
    public let code: String?
    /// Present only for `.failed`: a fixed label, safe to show -- never a
    /// path, a token, or trace content.
    public let label: String?

    enum CodingKeys: String, CodingKey {
        case entryID = "entry_id"
        case state
        case summary
        case rawSessionBytes = "raw_session_bytes"
        case limitBytes = "limit_bytes"
        case code
        case label
    }

    public init(
        entryID: String,
        state: PreviewRequestState,
        summary: Summary? = nil,
        rawSessionBytes: Int? = nil,
        limitBytes: Int? = nil,
        code: String? = nil,
        label: String? = nil
    ) {
        self.entryID = entryID
        self.state = state
        self.summary = summary
        self.rawSessionBytes = rawSessionBytes
        self.limitBytes = limitBytes
        self.code = code
        self.label = label
    }
}

/// Client-side bookkeeping for the daemon's bounded preview scheduler: which
/// entries this shell has already asked about, so a row that recomputes on
/// every redraw -- or a queue that refreshes on every `queue_changed` event
/// -- does not put a `preview_request` on the wire for the same entry on
/// every tick.
///
/// This is not a correctness requirement. The daemon dedups and caches on
/// its own side (`PreviewScheduler::request`'s doc: "calling this repeatedly
/// ... is free and is the intended usage"), so a resend would not start a
/// second build. What this tracker actually buys is fewer socket round
/// trips for the common case -- a card whose row re-renders while its
/// preview is still `queued` should not ask again just because SwiftUI
/// redrew it.
public struct PreviewRequestTracker: Sendable {
    private var inFlight: Set<String> = []
    private var answered: Set<String> = []

    public init() {}

    /// Whether `entryID` is worth asking the daemon about right now.
    ///
    /// A caller also checks its own summary/too-large/error dictionaries
    /// before this -- those are the durable record of a terminal answer.
    /// `answered` here exists so a *fresh* `PreviewRequestTracker` (a
    /// relaunch, a test) does not have to reconstruct its state from those
    /// dictionaries to get this right; in the running app the two are kept
    /// in step by `apply` running alongside the dictionary update.
    public func shouldRequest(_ entryID: String) -> Bool {
        !inFlight.contains(entryID) && !answered.contains(entryID)
    }

    public mutating func markRequested(_ entryID: String) {
        inFlight.insert(entryID)
    }

    /// Applies one state transition. `queued`/`running` leave (or put) the
    /// entry in flight -- a `preview_ready` event is still coming. The three
    /// terminal states move it to `answered`, where `shouldRequest` will not
    /// offer it again.
    public mutating func apply(state: PreviewRequestState, to entryID: String) {
        switch state {
        case .queued, .running:
            inFlight.insert(entryID)
        case .ready, .tooLarge, .failed:
            inFlight.remove(entryID)
            answered.insert(entryID)
        }
    }

    /// Drops all bookkeeping for entries that left the pending list for
    /// good -- approved, dismissed, expired, or superseded. A caller sends
    /// `preview_cancel` for these regardless of whether this tracker had
    /// heard of them (the daemon's `dropped: false` is a defined no-op, not
    /// an error -- see the contract's note on `preview_cancel`), so this
    /// does not need to report which were known; it only needs to stop
    /// tracking them.
    public mutating func forget(_ ids: some Sequence<String>) {
        for id in ids {
            inFlight.remove(id)
            answered.remove(id)
        }
    }

    public var inFlightCount: Int { inFlight.count }
    public var answeredCount: Int { answered.count }
}

/// Coalesces `preview_visible` sends against a scroll that has not yet
/// settled.
///
/// `preview_visible` replaces the daemon's on-screen set wholesale and is
/// cheap to call (see the design spec's "Priority" section), but "cheap" and
/// "meant to be sent on every frame" are different claims -- a fast scroll
/// through hundreds of rows should coalesce into one send of the set that
/// was actually on screen when the scroll stopped, not one send per row
/// that crossed the viewport on the way. This type holds the pure
/// bookkeeping for that coalescing; the actual settle timer is a real clock
/// and lives in the app target, which calls `setVisible` on every
/// visibility change and asks `takePendingSend()` only when its debounce
/// timer fires.
public struct PreviewVisibilityCoalescer: Sendable {
    private var visible: Set<String> = []
    private var pendingSend: Set<String>?

    public init() {}

    /// Replaces the on-screen set wholesale -- never an incremental
    /// add/remove, for the same reason the wire method does not offer one:
    /// a shell knows its own visible set after a scroll and should not have
    /// to diff what left it. Marks a send pending; repeated calls before
    /// the caller's debounce timer fires simply replace what is queued, so
    /// only the latest set is ever sent.
    public mutating func setVisible(_ ids: Set<String>) {
        visible = ids
        pendingSend = ids
    }

    /// What the debounce timer should send now that it has fired, clearing
    /// the pending flag. `nil` when nothing changed since the last take --
    /// a second timer fire with no intervening `setVisible` must not
    /// resend an identical set.
    public mutating func takePendingSend() -> Set<String>? {
        defer { pendingSend = nil }
        return pendingSend
    }

    public var currentlyVisible: Set<String> { visible }
}
