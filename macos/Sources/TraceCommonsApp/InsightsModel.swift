import Foundation
import Observation
import TCBridge
import TCShellCore

@Observable @MainActor
final class InsightsModel {
    typealias Service = @Sendable (InsightsRequest) async throws -> InsightsResponse
    /// Shared with the sibling tab models, so every tab reads the same store.
    let service: Service
    /// Reads `insights_week` through the daemon client (Insights feed T).
    typealias WeekReader = @Sendable (String?) async throws -> DaemonData.InsightsWeek
    /// Which counter rows the window's week figures come from. One at a time;
    /// the two are never mixed.
    enum WeekFeed: Equatable, Sendable { case saved, counterPass }
    /// The core's feed-line keys (`service::INSIGHTS_FEED_LINE_KEYS`). The
    /// window names the feed it shows with these and nothing else.
    enum FeedLineKey {
        static let saved = "insights_feed_saved"
        static let counterPass = "insights_feed_counter_pass"
        static let counterPassUnavailable = "insights_feed_counter_pass_unavailable"
    }
    private let weekReader: WeekReader?
    private var weekGeneration = UUID()
    /// Feed T only when the daemon answered enabled and readable; feed S
    /// (saved imports) otherwise.
    private(set) var weekFeed: WeekFeed = .saved
    /// The daemon's week, held only while `weekFeed` is `.counterPass`.
    private(set) var counterWeek: DaemonData.InsightsWeek?
    /// Set when feed T is switched on but could not be read, shown with the
    /// saved-imports line; `nil` when it is off or the daemon predates it.
    private(set) var counterPassNoticeKey: String?
    var feedLineKey: String { weekFeed == .counterPass ? FeedLineKey.counterPass : FeedLineKey.saved }
    /// Whether the daemon has answered for the week at least once (or there
    /// is no daemon to ask), so the get-started card is not drawn while the
    /// first answer is on its way.
    private(set) var weekSettled = false
    private var task: Task<Void, Never>?
    private var generation = UUID()
    private var active = false
    private(set) var copy: [String: String] = [:]
    func text(_ key: String) -> String { copy[key] ?? "" }
    /// The saved snapshots, newest first, so a drill-down row reads as its
    /// session's label.
    private(set) var snapshots: [LocalInsight] = []

    init(service: @escaping Service = { request in
        try await Task.detached { try TCInsights.call(request) }.value
    }, weekReader: WeekReader? = nil) {
        self.service = service
        self.weekReader = weekReader
    }

    /// Ask the daemon for one week of feed T (`nil` is the current week).
    /// Shows it only when it is switched on and readable; otherwise the
    /// saved-imports feed, with a notice only when it is on but unreadable.
    /// A later failure drops an earlier T week whole.
    func loadWeek(isoWeek: String? = nil) async {
        let generation = UUID(); weekGeneration = generation
        guard let weekReader else { showSavedFeed(notice: false); weekSettled = true; return }
        let outcome: Result<DaemonData.InsightsWeek, Error>
        do { outcome = .success(try await weekReader(isoWeek)) } catch { outcome = .failure(error) }
        guard weekGeneration == generation else { return }
        weekSettled = true
        switch outcome {
        case .success(let week) where week.showsCounterPass:
            weekFeed = .counterPass; counterWeek = week; counterPassNoticeKey = nil
        case .success(let week):
            showSavedFeed(notice: week.enabled)
        case .failure(let error):
            showSavedFeed(notice: !((error as? DaemonDataError)?.isNotServed ?? false))
        }
    }
    private func showSavedFeed(notice: Bool) {
        weekFeed = .saved; counterWeek = nil
        counterPassNoticeKey = notice ? FeedLineKey.counterPassUnavailable : nil
    }

    /// Reads the core's copy, then the saved snapshots, and the daemon's week.
    func open() {
        active = true
        load()
        if weekReader != nil { Task { [weak self] in await self?.loadWeek() } } else { weekSettled = true }
    }
    func close() {
        active = false; generation = UUID(); task?.cancel(); task = nil
        weekGeneration = UUID()
    }
    private func load() {
        task?.cancel()
        let token = generation
        let service = service
        task = Task { [weak self] in
            if let response = try? await service(.init(operation: .init("copy"))),
               response.type == "copy", let copy = response.copy {
                guard let self, self.active, self.generation == token, !Task.isCancelled else { return }
                self.copy = copy
            }
            if let response = try? await service(.init(operation: .init("list"))),
               response.type == "list", let values = response.insights {
                guard let self, self.active, self.generation == token, !Task.isCancelled else { return }
                self.snapshots = values
            }
        }
    }
}

/// The Overview tab's state: one saved week (feed S) and, when asked, what
/// makes up one card's figure. Every figure is the core's; a failed read
/// clears the figures rather than keep stale ones.
@Observable @MainActor
final class InsightsOverviewModel {
    private let service: InsightsModel.Service
    private var task: Task<Void, Never>?
    private var token = UUID()
    private var active = false
    private(set) var busy = false
    private(set) var failed = false
    private(set) var overview: InsightsWeekOverview?
    /// The daemon's week (feed T), in the same shape. While it is set it is
    /// the only week shown; the saved week is never mixed in.
    private(set) var counter: InsightsWeekOverview?
    /// The open figure's drill-down: tokens or cache share.
    private(set) var inputs: InsightsCardInputs?
    /// The Sessions figure's breakdown under feed S, read with every week
    /// and always shown under the Sessions card (owner, 2026-10-22: never
    /// behind a chevron). Nil while unread, after a failed read, and under
    /// feed T.
    private(set) var sessionsInputs: InsightsCardInputs?
    private var sessionsTask: Task<Void, Never>?
    /// Feed T's counted sessions, the Sessions figure's breakdown while the
    /// daemon's week shows: the daemon's rows, not the core's, since they
    /// carry no reference the core could look up.
    private(set) var counterSessions: InsightsCounterSessions?
    /// The week asked for; `nil` is the current week.
    private(set) var requestedWeek: String?

    init(service: @escaping InsightsModel.Service) { self.service = service }

    /// The shell's current UTC offset, which buckets the week's days.
    static var offset: Int32 { Int32(TimeZone.current.secondsFromGMT()) }

    /// The week on screen: feed T when the daemon sent one, feed S otherwise.
    var shown: InsightsWeekOverview? { counter ?? overview }

    /// Show the daemon's week and its counted sessions, or `nil` to go back
    /// to the saved week. Any open drill-down closes either way.
    func showCounter(_ week: InsightsWeekOverview?, sessions: InsightsCounterSessions? = nil) {
        counter = week; counterSessions = week == nil ? nil : sessions
        inputs = nil
    }

    /// Whether the tokens or cache share figure opens a drill-down in the
    /// week on screen. Feed T rows carry no session reference the core
    /// could look up, so under feed T neither does.
    func drills(_ card: String) -> Bool { counter == nil && card != "sessions" }

    /// The figure whose drill-down is open.
    var openCard: String? { inputs?.card }

    func open() { active = true; load() }
    func close() {
        active = false; token = UUID(); task?.cancel(); task = nil; busy = false
        sessionsTask?.cancel(); sessionsTask = nil
        inputs = nil; sessionsInputs = nil
    }
    func reload() { load() }
    func selectWeek(_ weekStart: String) { requestedWeek = weekStart; load() }

    /// Read the drill-down for `card` (`tokens` or `cache_share`) in the
    /// saved week on screen.
    func showInputs(_ card: String) {
        guard active, counter == nil, card != "sessions", let week = overview?.week_start else { return }
        run(.init("card_inputs", weekStart: week, tz: Self.offset, card: card)) { model, response in
            guard let inputs = response.inputs, inputs.card == card else { throw InsightsError.invalidResponse }
            model.inputs = inputs
        }
    }
    func hideInputs() { inputs = nil }

    private func load() {
        guard active else { return }
        inputs = nil; sessionsInputs = nil
        sessionsTask?.cancel(); sessionsTask = nil
        run(.init("week_overview", weekStart: requestedWeek, tz: Self.offset)) { model, response in
            guard let overview = response.overview else { throw InsightsError.invalidResponse }
            model.overview = overview
            model.readSessions(week: overview.week_start)
        }
    }

    /// Read the Sessions figure's breakdown for `week`, on its own task so
    /// a figure's drill-down never cancels it. A failed read draws none.
    private func readSessions(week: String) {
        sessionsTask?.cancel()
        let service = service
        sessionsTask = Task { [weak self] in
            let response = try? await service(
                .init(operation: .init("card_inputs", weekStart: week, tz: Self.offset, card: "sessions")))
            guard let self, self.active, !Task.isCancelled, self.overview?.week_start == week else { return }
            let inputs = response?.type == "card_inputs" ? response?.inputs : nil
            self.sessionsInputs = inputs?.card == "sessions" ? inputs : nil
        }
    }

    private func run(_ operation: InsightsRequest.Operation,
                     apply: @escaping @MainActor (InsightsOverviewModel, InsightsResponse) throws -> Void) {
        task?.cancel()
        token = UUID(); let current = token
        busy = true
        if operation.type != "card_inputs" { failed = false }
        let service = service
        task = Task { [weak self] in
            do {
                let response = try await service(.init(operation: operation))
                guard let self, self.active, self.token == current, !Task.isCancelled else { return }
                guard response.type == operation.type else { throw InsightsError.invalidResponse }
                try apply(self, response)
                self.busy = false
            } catch {
                guard let self, self.active, self.token == current, !Task.isCancelled else { return }
                // A failed drill-down closes only itself; a failed week read
                // never keeps the old figures.
                if operation.type != "card_inputs" { self.overview = nil; self.failed = true }
                self.inputs = nil; self.busy = false
            }
        }
    }
}

/// The core's analytics words, filled. Nothing here composes a sentence: a
/// template's `{name}` holes take numbers the core computed.
enum InsightsOverviewWords {
    static func text(_ key: String, _ copy: [String: String]) -> String { copy[key] ?? "" }

    static func fill(_ template: String, _ values: [String: String]) -> String {
        values.reduce(template) { result, hole in
            result.replacingOccurrences(of: "{" + hole.key + "}", with: hole.value)
        }
    }

    /// The figure, or the dash for unknown. A measured zero stays zero.
    static func figure(_ value: UInt64?, copy: [String: String]) -> String {
        guard let value else { return text("analytics_unavailable", copy) }
        return value.formatted(.number.notation(.compactName))
    }

    /// Whole percent from the core's per mille, rounded half up.
    static func share(_ share: InsightsShareFigure?, copy: [String: String]) -> String {
        guard let share else { return text("analytics_unavailable", copy) }
        return String((share.permille + 5) / 10)
    }

    /// Whole percent from a per mille, rounded half up.
    static func percent(_ permille: UInt64) -> String { String((permille + 5) / 10) }

    /// "vs last week". The core sends a figure only under feed T; any
    /// unavailable reason is the dash.
    static func change(_ source: InsightsWeekSource, copy: [String: String]) -> String {
        changeLine(source.change_permille, copy: copy)
    }

    /// "▼ {p}% vs last week" / "▲ {p}% vs last week", or the dash.
    static func changeLine(_ permille: Int64?, copy: [String: String]) -> String {
        guard let permille else { return text("analytics_unavailable", copy) }
        let key = permille < 0 ? "analytics_change_down" : "analytics_change_up"
        return fill(text(key, copy), ["p": percent(permille.magnitude)])
    }

    /// "Your best week · previous best {q}%", or the dash.
    static func bestWeek(_ source: InsightsWeekSource, copy: [String: String]) -> String {
        guard let best = source.best else { return text("analytics_unavailable", copy) }
        return fill(text("analytics_best_week", copy), ["q": percent(best.previous_best_permille)])
    }

    /// Where the previous best sits on the share bar; `nil` without one.
    static func bestTick(_ source: InsightsWeekSource) -> UInt64? { source.best?.previous_best_permille }

    static func sourceLine(_ source: String, copy: [String: String]) -> String {
        text("analytics_source_" + source, copy)
    }

    static func harness(_ source: String?, copy: [String: String]) -> String {
        guard let source else { return text("analytics_unavailable", copy) }
        return text(source, copy)
    }

    static func coverageLine(_ coverage: InsightsWeekCoverage, copy: [String: String]) -> String {
        fill(text("analytics_coverage_line", copy), [
            "k": String(coverage.known), "n": String(coverage.sessions),
            "p": String(coverage.partial), "u": String(coverage.unknown),
        ])
    }

    /// Which feed is showing. Feed S also says that weeks are not compared.
    static func feedLines(_ feed: String, copy: [String: String]) -> [String] {
        switch feed {
        case "saved":
            return [text("analytics_feed_saved", copy), text("analytics_feed_comparisons_need_counter_pass", copy)]
        case "counter_pass":
            return [text("analytics_feed_counter_pass", copy)]
        case "ledger":
            return [text("analytics_feed_ledger", copy)]
        default:
            return []
        }
    }

    /// `yyyy-MM-dd` (a Monday) as the ISO week `insights_week` takes.
    static func isoWeek(_ weekStart: String) -> String? {
        let parse = Date.ISO8601FormatStyle().year().month().day()
        guard let day = try? Date(weekStart, strategy: parse) else { return nil }
        var calendar = Calendar(identifier: .iso8601)
        calendar.timeZone = TimeZone(secondsFromGMT: 0) ?? .current
        let parts = calendar.dateComponents([.yearForWeekOfYear, .weekOfYear], from: day)
        guard let year = parts.yearForWeekOfYear, let week = parts.weekOfYear else { return nil }
        return String(format: "%04d-W%02d", year, week)
    }

    static func reason(_ wire: String, copy: [String: String]) -> String { text("analytics_reason_" + wire, copy) }
    static func state(_ wire: String, copy: [String: String]) -> String { text("analytics_state_" + wire, copy) }

    struct ModelRow: Identifiable, Equatable {
        let id: Int
        let label: String
        let tokens: UInt64
    }

    /// By model, in the core's fixed order. Never sorted by value here.
    static func modelRows(_ overview: InsightsWeekOverview, copy: [String: String]) -> [ModelRow] {
        overview.by_model.enumerated().map { index, row in
            ModelRow(id: index, label: row.label ?? text("analytics_unknown_label", copy), tokens: row.tokens)
        }
    }

    /// The week's dates, Monday to Sunday, in the user's locale.
    static func weekRange(start: String, end: String) -> String {
        let parse = Date.ISO8601FormatStyle().year().month().day()
        guard let first = try? Date(start, strategy: parse), let last = try? Date(end, strategy: parse) else {
            return start
        }
        let style = Date.FormatStyle(timeZone: TimeZone(secondsFromGMT: 0) ?? .current).month(.abbreviated).day()
        return first.formatted(style) + " \u{2013} " + last.formatted(style)
    }
}

/// Feed T's counted sessions for the Sessions card's drill-down: the
/// daemon's rows in its order, and whether their route column is drawn.
/// Local counter references identify the displayed sessions. Row identity
/// retains its place for compatibility with older daemon payloads.
struct InsightsCounterSessions: Equatable, Sendable {
    struct Row: Identifiable, Equatable, Sendable {
        let id: Int
        let session: DaemonData.InsightsWeekSession
    }

    let rows: [Row]
    let column: InsightsRouteWords.Column

    /// `nil` for an answer with no rollup.
    init?(week: DaemonData.InsightsWeek) {
        guard let sessions = week.rollup?.sessions else { return nil }
        self.init(sessions: sessions, column: InsightsRouteWords.column(week))
    }

    init(sessions: [DaemonData.InsightsWeekSession], column: InsightsRouteWords.Column) {
        rows = sessions.enumerated().map { Row(id: $0.offset, session: $0.element) }
        self.column = column
    }
}

/// One day picked on "Tokens by day": its place in the week, Monday first,
/// as each session's `by_day` counts it, and its weekday for the title.
struct InsightsDayPick: Equatable {
    let index: Int
    let weekday: String

    /// The day dated `date` among the week's `days`; `nil` when none is.
    /// The weekday is in `locale`, the person's own outside the tests.
    static func pick(_ date: String?, days: [InsightsDayTokens]?,
                     locale: Locale = .autoupdatingCurrent) -> InsightsDayPick? {
        guard let date, let index = days?.firstIndex(where: { $0.date == date }) else { return nil }
        let parse = Date.ISO8601FormatStyle().year().month().day()
        let wide = Date.FormatStyle(locale: locale, calendar: Calendar(identifier: .gregorian),
                                    timeZone: TimeZone(secondsFromGMT: 0) ?? .current).weekday(.wide)
        return InsightsDayPick(index: index, weekday: (try? Date(date, strategy: parse))?.formatted(wide) ?? date)
    }
}

/// The Sessions breakdown for a picked day: the sessions with tokens that
/// day, most first, each with that day's tokens from its own `by_day`. A
/// session with no split (Codex, or an unknown figure) is in no day.
enum InsightsDayBreakdown {
    struct Row<Session: Identifiable>: Identifiable {
        let session: Session
        let tokens: UInt64
        var id: Session.ID { session.id }
    }

    /// `nil` when no session carries a split, as from a daemon or core
    /// that predates it: the week shows, never an empty day.
    static func rows<Session: Identifiable>(_ sessions: [Session], day: Int,
                                            split: (Session) -> [UInt64]?) -> [Row<Session>]? {
        guard sessions.contains(where: { split($0) != nil }) else { return nil }
        let counted = sessions.enumerated().compactMap { offset, session -> (offset: Int, row: Row<Session>)? in
            guard let days = split(session), days.indices.contains(day), days[day] > 0 else { return nil }
            return (offset, Row(session: session, tokens: days[day]))
        }
        // Ties keep the breakdown's own order.
        return counted.sorted { $0.row.tokens != $1.row.tokens ? $0.row.tokens > $1.row.tokens : $0.offset < $1.offset }
            .map(\.row)
    }

    /// A feed T row's split, in the core's unsigned figures.
    static func split(_ session: DaemonData.InsightsWeekSession) -> [UInt64]? {
        session.byDay?.map { UInt64(max(0, $0)) }
    }
}

/// Where a counted session's calls went, in the core's words. Every label
/// comes from the proxy's proof, never from a backend's name; only
/// verified proof reads as verified, and no proxy record is never "not
/// private". The tokens here are the proxy's per-call count, drawn beside
/// the transcript's and never added to it.
enum InsightsRouteWords {
    /// The Private AI column: drawn, replaced by the feed-off line or the
    /// no-ledger line, or left out with nothing said.
    enum Column: Equatable, Sendable { case shown, feedOff, noLedger, hidden }

    private static func text(_ key: String, _ copy: [String: String]) -> String { copy[key] ?? "" }
    private static func dash(_ copy: [String: String]) -> String { text("analytics_unavailable", copy) }

    /// Drawn only when the daemon says the rows carry a route. The feed
    /// turned off in Settings and no proxy ledger each get their line; an
    /// older daemon or a reason this build does not know draw nothing.
    static func column(_ week: DaemonData.InsightsWeek) -> Column {
        if week.routingAvailable == true { return .shown }
        switch week.routingUnavailable {
        case "ledger_feed_off": return .feedOff
        case "no_ledger": return .noLedger
        default: return .hidden
        }
    }

    /// The one line shown in place of the column: the feed is off, or no
    /// Private AI call is recorded yet. `nil` when the column is drawn or
    /// left out with nothing said.
    static func unavailableLine(_ column: Column, copy: [String: String]) -> String? {
        switch column {
        case .feedOff: return text("analytics_route_feed_off", copy)
        case .noLedger: return text("analytics_route_no_ledger", copy)
        case .shown, .hidden: return nil
        }
    }

    /// The category's word; the dash for a missing route or a category this
    /// build does not know, so nothing is claimed for it.
    static func category(_ routing: DaemonData.InsightsWeekRouting?, copy: [String: String]) -> String {
        let key: String
        switch routing?.category {
        case "unobserved": key = "analytics_route_unobserved"
        case "unrecorded": key = "analytics_route_unrecorded"
        case "outside": key = "analytics_route_outside"
        case "mixed": key = "analytics_route_mixed"
        case "check_failed": key = "analytics_route_check_failed"
        case "routed_verified": key = "analytics_route_verified"
        case "routed_unverified": key = "analytics_route_unverified"
        default: return dash(copy)
        }
        return text(key, copy)
    }

    /// The route's qualifiers that have words; one this build does not
    /// know is left out rather than shown as its wire label.
    static func reasons(_ routing: DaemonData.InsightsWeekRouting?, copy: [String: String]) -> [String] {
        (routing?.reasons ?? []).compactMap { reason in
            reason == "some_calls_unrecorded" ? text("analytics_reason_" + reason, copy) : nil
        }
    }

    /// The split in proxy tokens, each of the five buckets the daemon's
    /// figure or the dash, the failed-check one included. `nil` only with
    /// no figures (no proxy record).
    static func split(_ routing: DaemonData.InsightsWeekRouting?, copy: [String: String]) -> String? {
        guard let tokens = routing?.tokens else { return nil }
        let figure = { (value: Int64?) in
            InsightsOverviewWords.figure(value.flatMap { UInt64(exactly: $0) }, copy: copy)
        }
        return InsightsOverviewWords.fill(text("analytics_route_split", copy), [
            "v": figure(tokens.verified),
            "u": figure(tokens.routedUnverified),
            "f": figure(tokens.checkFailed),
            "o": figure(tokens.outside),
            "n": figure(tokens.unrecorded),
        ])
    }
}

/// The Patterns tab ("Where tokens went") over the saved snapshots (feed S).
@Observable @MainActor
final class InsightsPatternsModel {
    private let service: InsightsModel.Service
    private var task: Task<Void, Never>?
    private var token = UUID()
    private var active = false
    private(set) var busy = false
    private(set) var failed = false
    private(set) var patterns: InsightsWeekPatterns?
    /// The daemon's week (feed T), in the same shape; while it is set it is
    /// the only week shown.
    private(set) var counter: InsightsWeekPatterns?
    /// The sessions behind one card, while its list is open.
    private(set) var sessions: InsightsPatternSessions?

    /// The week on screen: feed T when the daemon sent one, feed S otherwise.
    var shown: InsightsWeekPatterns? { counter ?? patterns }

    func showCounter(_ week: InsightsWeekPatterns?) {
        counter = week; sessions = nil
    }
    /// The week asked for; `nil` is the current week.
    private(set) var requestedWeek: String?

    init(service: @escaping InsightsModel.Service) { self.service = service }

    func open() { active = true; load() }
    func close() { active = false; token = UUID(); task?.cancel(); task = nil; busy = false; sessions = nil }
    func reload() { load() }
    func selectWeek(_ weekStart: String) { requestedWeek = weekStart; load() }

    /// Read the sessions behind the card of `kind` in the week on screen.
    func showSessions(_ kind: String) {
        guard active, counter == nil, let week = patterns?.week_start else { return }
        run(.init("pattern_sessions", weekStart: week, tz: InsightsOverviewModel.offset, pattern: kind)) { model, response in
            guard let found = response.pattern_sessions, found.pattern == kind else { throw InsightsError.invalidResponse }
            model.sessions = found
        }
    }
    func hideSessions() { sessions = nil }

    private func load() {
        guard active else { return }
        sessions = nil
        run(.init("patterns", weekStart: requestedWeek, tz: InsightsOverviewModel.offset)) { model, response in
            guard let patterns = response.patterns else { throw InsightsError.invalidResponse }
            model.patterns = patterns
        }
    }

    private func run(_ operation: InsightsRequest.Operation,
                     apply: @escaping @MainActor (InsightsPatternsModel, InsightsResponse) throws -> Void) {
        task?.cancel()
        token = UUID(); let current = token
        busy = true
        let listing = operation.type == "pattern_sessions"
        if !listing { failed = false }
        let service = service
        task = Task { [weak self] in
            do {
                let response = try await service(.init(operation: operation))
                guard let self, self.active, self.token == current, !Task.isCancelled else { return }
                guard response.type == operation.type else { throw InsightsError.invalidResponse }
                try apply(self, response)
                self.busy = false
            } catch {
                guard let self, self.active, self.token == current, !Task.isCancelled else { return }
                // A failed session list closes only itself; a failed week
                // read never keeps the old figures.
                if !listing { self.patterns = nil; self.failed = true }
                self.sessions = nil; self.busy = false
            }
        }
    }
}

/// The core's Patterns words, filled with the core's figures. Nothing here
/// composes a sentence.
enum InsightsPatternsWords {
    private static func text(_ key: String, _ copy: [String: String]) -> String { copy[key] ?? "" }
    private static func dash(_ copy: [String: String]) -> String { text("analytics_unavailable", copy) }
    private static func count(_ value: UInt32?, _ copy: [String: String]) -> String {
        value.map(String.init) ?? dash(copy)
    }

    /// The card's name, by its wire kind.
    static func title(_ kind: String, copy: [String: String]) -> String {
        text("analytics_pattern_" + kind, copy)
    }

    /// Whether the core says the card leads with its count: its estimate
    /// rounds to 0 while it found something (owner decision 2026-10-20).
    /// An older core, or a word this build does not know, leads with tokens.
    static func leadsWithCount(_ card: InsightsPatternCard) -> Bool {
        card.headline == "count" && card.count != nil
    }

    /// Each card's hole for its count in the headline words.
    private static let countHole = [
        "repeated_reads": "r", "retried_calls": "c", "edit_fail_edit": "l", "long_context": "n",
    ]

    /// The card's headline: the token figure, or the count with its unit
    /// ("2 reads", "1 call") when the core leads with the count.
    static func headline(_ card: InsightsPatternCard, copy: [String: String]) -> String {
        guard leadsWithCount(card), let found = card.count, let hole = countHole[card.kind] else {
            return InsightsOverviewWords.figure(card.tokens, copy: copy)
        }
        let key = "analytics_pattern_" + card.kind + (found == 1 ? "_headline_one" : "_headline_other")
        return InsightsOverviewWords.fill(text(key, copy), [hole: String(found)])
    }

    /// The line under the headline: the core's count sentence, or the
    /// long-context threshold line. Under a count headline the sentence
    /// without the count, so the count is not said twice.
    static func countLine(_ card: InsightsPatternCard, threshold: UInt64, copy: [String: String]) -> String {
        let fill = InsightsOverviewWords.fill
        let underCount = leadsWithCount(card)
        switch card.kind {
        case "repeated_reads" where underCount:
            let key = "analytics_pattern_repeated_reads_count_under_count"
                + (card.files == 1 ? "_one" : "_other")
            return fill(text(key, copy), ["f": count(card.files, copy)])
        case "retried_calls" where underCount:
            return text("analytics_pattern_retried_calls_count_under_count", copy)
        case "edit_fail_edit" where underCount:
            return text("analytics_pattern_edit_fail_edit_count_under_count", copy)
        case "repeated_reads":
            return fill(text("analytics_pattern_repeated_reads_count", copy),
                        ["r": count(card.count, copy), "f": count(card.files, copy)])
        case "retried_calls":
            return fill(text("analytics_pattern_retried_calls_count", copy), ["c": count(card.count, copy)])
        case "edit_fail_edit":
            return fill(text("analytics_pattern_edit_fail_edit_count", copy), ["l": count(card.count, copy)])
        case "long_context":
            return fill(text("analytics_pattern_long_context_line", copy),
                        ["threshold": InsightsOverviewWords.figure(threshold, copy: copy)])
        default:
            return dash(copy)
        }
    }

    /// How the figure was arrived at: the inferred label first, then the basis.
    static func basisLines(_ card: InsightsPatternCard, copy: [String: String]) -> [String] {
        var lines: [String] = []
        if card.inferred { lines.append(text("analytics_inferred_from_order", copy)) }
        switch card.basis {
        case "estimate_from_result_size": lines.append(text("analytics_estimate_from_result_size", copy))
        case "from_counters": lines.append(text("analytics_from_counters", copy))
        default: break
        }
        return lines
    }

    /// "vs last week". Only feed T sends a figure; anything else is the dash.
    static func change(_ card: InsightsPatternCard, copy: [String: String]) -> String {
        guard let permille = card.change else { return dash(copy) }
        let percent = String((abs(permille) + 5) / 10)
        let key = permille < 0 ? "analytics_change_down" : "analytics_change_up"
        return InsightsOverviewWords.fill(text(key, copy), ["p": percent])
    }

    struct Bar: Identifiable, Equatable {
        let week: String
        /// `nil` is a gap: no bar is drawn, never a zero.
        let tokens: UInt64?
        var id: String { week }
    }

    /// Every week the card covers, oldest first, gaps included.
    static func bars(_ card: InsightsPatternCard) -> [Bar] {
        card.weeks.map { Bar(week: $0.week_start, tokens: $0.tokens) }
    }

    /// The figure above the current week's bar. A card that leads with its
    /// count draws none: its estimate rounds to 0 and a "0" would contradict
    /// the headline. `nil` also when the week has no figure.
    static func currentWeekFigure(_ card: InsightsPatternCard, copy: [String: String]) -> String? {
        guard !leadsWithCount(card), let tokens = bars(card).last?.tokens else { return nil }
        return InsightsOverviewWords.figure(tokens, copy: copy)
    }

    /// The bars to draw: weeks with a figure only.
    static func drawnBars(_ card: InsightsPatternCard) -> [Bar] {
        bars(card).filter { $0.tokens != nil }
    }

    static func seeSessions(_ card: InsightsPatternCard, copy: [String: String]) -> String {
        InsightsOverviewWords.fill(text("analytics_see_sessions", copy), ["n": String(card.sessions)])
    }

    /// "File B · .rs", or "File B" with no extension.
    static func fileLabel(_ row: InsightsRereadRow, copy: [String: String]) -> String {
        guard let ext = row.ext else {
            return InsightsOverviewWords.fill(text("analytics_file_label_no_ext", copy), ["letter": row.letter])
        }
        return InsightsOverviewWords.fill(text("analytics_file_label", copy), ["letter": row.letter, "ext": ext])
    }

    /// "Claude Code sessions only: {k} of {n}." when another harness is in
    /// the week; `nil` otherwise.
    static func claudeOnlyLine(_ patterns: InsightsWeekPatterns, copy: [String: String]) -> String? {
        guard patterns.claude_only else { return nil }
        return InsightsOverviewWords.fill(text("analytics_claude_sessions_only", copy), [
            "k": String(patterns.claude_sessions), "n": String(patterns.sessions),
        ])
    }
}

/// The core's words for a session row, filled with the core's figures. Nothing here
/// composes a sentence.
enum InsightsSessionsWords {
    private static func text(_ key: String, _ copy: [String: String]) -> String { copy[key] ?? "" }
    private static func dash(_ copy: [String: String]) -> String { text("analytics_unavailable", copy) }

    /// A saved session's label: its first recorded event's local date and
    /// time, its harness and `tokens`, the core's figure when the caller
    /// holds one and the dash otherwise. Never the snapshot ID. The time is
    /// read in the fixed offset the app sends the core
    /// (`InsightsOverviewModel.offset`), so the date is the one the drill
    /// header names.
    static func label(_ insight: LocalInsight, tokens: UInt64? = nil, copy: [String: String],
                      timeZone: TimeZone = TimeZone(secondsFromGMT: TimeZone.current.secondsFromGMT()) ?? .current)
        -> String {
        label(first: firstEvent(insight), source: insight.source_format, tokens: tokens, copy: copy,
              timeZone: timeZone)
    }

    /// A counted session's folder label and opaque short reference.
    /// Older daemon payloads omit both; do not invent a session identifier.
    static func counterTitle(_ row: DaemonData.InsightsWeekSession, copy: [String: String]) -> String {
        let folder = row.folder.flatMap { $0.isEmpty ? nil : $0 }
            ?? text("analytics_folder_unknown", copy)
        guard let reference = row.sessionRef, !reference.isEmpty else { return folder }
        return folder + " · " + reference
    }

    /// A counted session's label (feed T): its recorded activity range,
    /// harness and transcript tokens, beneath its folder and reference.
    /// `tokens`, when given, stands in for the week's (a picked day's).
    /// A missing or invalid last event retains the first-event-only label.
    static func counterLabel(_ row: DaemonData.InsightsWeekSession, tokens: UInt64? = nil, copy: [String: String],
                             timeZone: TimeZone = .current) -> String {
        let tokens = tokens ?? row.tokens.flatMap { UInt64(exactly: $0) }
        let first = row.startedAt.flatMap(parseEvent)
        if let first, let last = row.lastEventAt.flatMap(parseEvent), last > first,
           let template = copy["analytics_session_label_range"] {
            var calendar = Calendar(identifier: .gregorian)
            calendar.timeZone = timeZone
            let startStyle = Date.FormatStyle(timeZone: timeZone).month(.abbreviated).day().hour().minute()
            let end: String
            if calendar.isDate(first, inSameDayAs: last) {
                end = last.formatted(Date.FormatStyle(timeZone: timeZone).hour().minute())
            } else if calendar.component(.year, from: first) == calendar.component(.year, from: last) {
                end = last.formatted(startStyle)
            } else {
                end = last.formatted(startStyle.year())
            }
            return InsightsOverviewWords.fill(template, [
                "span": first.formatted(startStyle) + " – " + end,
                "harness": InsightsOverviewWords.harness(row.source, copy: copy),
                "t": InsightsOverviewWords.figure(tokens, copy: copy),
            ])
        }
        return label(first: first, source: row.source, tokens: tokens, copy: copy, timeZone: timeZone)
    }

    private static func label(first: Date?, source: String?, tokens: UInt64?, copy: [String: String],
                              timeZone: TimeZone) -> String {
        let harness = InsightsOverviewWords.harness(source, copy: copy)
        let figure = InsightsOverviewWords.figure(tokens, copy: copy)
        guard let first else {
            return InsightsOverviewWords.fill(text("analytics_session_label_undated", copy),
                                              ["harness": harness, "t": figure])
        }
        return InsightsOverviewWords.fill(text("analytics_session_label", copy), [
            "date": first.formatted(Date.FormatStyle(timeZone: timeZone).month(.abbreviated).day()),
            "time": first.formatted(Date.FormatStyle(timeZone: timeZone).hour().minute()),
            "harness": harness,
            "t": figure,
        ])
    }

    /// The first recorded event. The core writes it with or without
    /// fractional seconds; the tests pin that both read.
    private static func firstEvent(_ insight: LocalInsight) -> Date? {
        guard let wire = insight.time_evidence?.earliest?.recorded_at else { return nil }
        return parseEvent(wire)
    }

    private static func parseEvent(_ wire: String) -> Date? {
        try? Date(wire, strategy: Date.ISO8601FormatStyle(includingFractionalSeconds: true))
    }

    /// A drill-down row's session: its saved snapshot's label with the row's
    /// own figure, or the dash when no saved snapshot has that ID.
    static func rowLabel(_ sessionRef: String, tokens: UInt64?, snapshots: [LocalInsight],
                         copy: [String: String]) -> String {
        guard let insight = snapshots.first(where: { $0.id == sessionRef }) else { return dash(copy) }
        return label(insight, tokens: tokens, copy: copy)
    }


}

/// The daemon's week (feed T) in the core's own shapes: the same `TCBridge`
/// types the saved week decodes into, so one view draws either feed.
extension DaemonData.InsightsWeek {
    private func core<T: Decodable>(_ part: DaemonData.CoreJSON?, as type: T.Type) -> T? {
        part.flatMap { try? JSONDecoder().decode(T.self, from: $0.data) }
    }
    var coreOverview: InsightsWeekOverview? { core(overview, as: InsightsWeekOverview.self) }
    var corePatterns: InsightsWeekPatterns? { core(patterns, as: InsightsWeekPatterns.self) }
    /// The kept weeks, passed to the core's `comparisons` unchanged.
    var coreHistory: [InsightsWeekFigures]? { core(history, as: [InsightsWeekFigures].self) }
}

/// Goals, the lever of the week and the weekly summary card (feed T only).
/// The core marks the goals it stores against the daemon's weekly figures,
/// which this model passes through unchanged; nothing is compared here.
@Observable @MainActor
final class InsightsComparisonsModel {
    private let service: InsightsModel.Service
    private var task: Task<Void, Never>?
    private var token = UUID()
    private(set) var busy = false
    private(set) var failed = false
    private(set) var comparisons: InsightsComparisons?
    private var counterWeeks: [InsightsWeekFigures]?
    private var weekStart: String?
    private var recapCardEnabled = false

    init(service: @escaping InsightsModel.Service) { self.service = service }

    /// Read for the daemon's week; `nil` weeks (feed S) clears everything,
    /// because nothing is compared under feed S.
    func load(counterWeeks: [InsightsWeekFigures]?, weekStart: String?, recapCardEnabled: Bool) {
        self.counterWeeks = counterWeeks; self.weekStart = weekStart
        self.recapCardEnabled = recapCardEnabled
        guard counterWeeks != nil else { clear(); return }
        reload()
    }

    func clear() {
        token = UUID(); task?.cancel(); task = nil
        comparisons = nil; busy = false; failed = false
    }

    func reload() {
        guard let counterWeeks else { return }
        run(.init("comparisons", weekStart: weekStart, tz: InsightsOverviewModel.offset,
                  counterWeeks: counterWeeks, recapCardEnabled: recapCardEnabled))
    }

    func addGoal(_ goal: InsightsGoal) { write(.init("goal_set", goal: goal)) }
    func deleteGoal(_ id: String) { write(.init("goal_delete", id: id)) }

    /// "Not useful" on the lever shown, for the week shown.
    func notUseful() {
        guard let found = comparisons, let pick = found.lever.pick else { return }
        write(.init("lever_feedback", weekStart: found.week_start, kind: pick.kind, action: "not_useful"))
    }

    /// "Open recap": the card is not shown again. Returns the closed week to
    /// put on screen.
    func openRecap() -> String? {
        guard let week = comparisons?.recap?.week_start else { return nil }
        write(.init("recap_opened", weekStart: week))
        return week
    }

    private func write(_ operation: InsightsRequest.Operation) {
        guard counterWeeks != nil else { return }
        let service = service
        let current = UUID(); token = current
        task?.cancel(); busy = true
        task = Task { [weak self] in
            do {
                let response = try await service(.init(operation: operation))
                guard let self, self.token == current, !Task.isCancelled else { return }
                guard response.type == operation.type, response.state != nil else {
                    throw InsightsError.invalidResponse
                }
                self.busy = false
                self.reload()
            } catch {
                guard let self, self.token == current, !Task.isCancelled else { return }
                self.busy = false; self.failed = true
            }
        }
    }

    private func run(_ operation: InsightsRequest.Operation) {
        let service = service
        let current = UUID(); token = current
        task?.cancel(); busy = true; failed = false
        task = Task { [weak self] in
            do {
                let response = try await service(.init(operation: operation))
                guard let self, self.token == current, !Task.isCancelled else { return }
                guard response.type == operation.type, let found = response.comparisons else {
                    throw InsightsError.invalidResponse
                }
                self.comparisons = found; self.busy = false
            } catch {
                guard let self, self.token == current, !Task.isCancelled else { return }
                // A failed read never keeps an earlier week's marks.
                self.comparisons = nil; self.failed = true; self.busy = false
            }
        }
    }
}

/// The core's goal, lever and summary-card words, filled with the core's
/// figures. Nothing here composes a sentence or compares a figure.
enum InsightsComparisonsWords {
    private static func text(_ key: String, _ copy: [String: String]) -> String { copy[key] ?? "" }
    private static func dash(_ copy: [String: String]) -> String { text("analytics_unavailable", copy) }
    private static let fill = InsightsOverviewWords.fill

    /// The goal kinds, in the core's order, and their words.
    static let goalKinds = ["cache_share_at_least", "repeated_reads_under", "long_context_under", "weekly_tokens_under"]
    private static let goalKeys = [
        "cache_share_at_least": "analytics_goal_cache_share",
        "repeated_reads_under": "analytics_goal_repeated_reads",
        "long_context_under": "analytics_goal_long_context",
        "weekly_tokens_under": "analytics_goal_weekly_tokens",
    ]

    /// Whether a kind names one harness.
    static func goalNeedsSource(_ kind: String) -> Bool {
        kind == "cache_share_at_least" || kind == "weekly_tokens_under"
    }

    /// The goal's line, its harness first when it names one.
    static func goal(_ goal: InsightsGoal, copy: [String: String]) -> String {
        let line = fill(text(goalKeys[goal.kind] ?? "", copy), [
            "p": goal.permille.map(InsightsOverviewWords.percent) ?? dash(copy),
            "t": InsightsOverviewWords.figure(goal.tokens, copy: copy),
        ])
        guard let source = goal.source else { return line }
        return InsightsOverviewWords.harness(source, copy: copy) + " \u{b7} " + line
    }

    /// A goal from what the user typed: a whole percent for a share, a token
    /// count otherwise. `nil` when the number is not one.
    static func newGoal(kind: String, source: String, number: String) -> InsightsGoal? {
        guard let value = UInt64(number.trimmingCharacters(in: .whitespaces)), value > 0 else { return nil }
        let harness = goalNeedsSource(kind) ? source : nil
        if kind == "cache_share_at_least" {
            guard value <= 100 else { return nil }
            return InsightsGoal(kind: kind, source: harness, permille: value * 10)
        }
        return InsightsGoal(kind: kind, source: harness, tokens: value)
    }

    /// One weekly mark: met, not met, or the dash for a week with no figure.
    static func mark(_ wire: String, copy: [String: String]) -> String {
        switch wire {
        case "met": return text("analytics_goal_met", copy)
        case "not_met": return text("analytics_goal_not_met", copy)
        default: return dash(copy)
        }
    }

    /// The figure the goal is judged on, in its own unit.
    static func goalFigure(_ view: InsightsGoalView, copy: [String: String]) -> String {
        guard let figure = view.figure else { return dash(copy) }
        return view.goal.kind == "cache_share_at_least"
            ? InsightsOverviewWords.percent(figure) + "%" : InsightsOverviewWords.figure(figure, copy: copy)
    }

    /// "Down from {x} last week." / "Up from {x} last week."; `nil` when the
    /// same, or when either week is not comparable.
    static func goalChange(_ view: InsightsGoalView, copy: [String: String]) -> String? {
        guard let change = view.marks?.change else { return nil }
        let from = view.goal.kind == "cache_share_at_least"
            ? InsightsOverviewWords.percent(change.from) + "%" : InsightsOverviewWords.figure(change.from, copy: copy)
        switch change.direction {
        case "down": return fill(text("analytics_goal_down_from", copy), ["x": from])
        case "up": return fill(text("analytics_goal_up_from", copy), ["x": from])
        default: return nil
        }
    }

    /// The lever's observation. Repeated reads have their own line; any
    /// other kind shows its card's name and figure. No advice (owner
    /// decision D2, open).
    static func leverLines(_ pick: InsightsLeverView, copy: [String: String]) -> [String] {
        let tokens = InsightsOverviewWords.figure(pick.tokens, copy: copy)
        if pick.kind == "repeated_reads" {
            return [fill(text("analytics_lever_line", copy), [
                "f": pick.files.map(String.init) ?? dash(copy),
                "r": pick.count.map(String.init) ?? dash(copy),
                "t": tokens,
            ])]
        }
        return [InsightsPatternsWords.title(pick.kind, copy: copy), tokens]
    }

    /// The summary card's title.
    static func recapTitle(_ recap: InsightsRecap, copy: [String: String]) -> String {
        fill(text("analytics_your_week", copy), [
            "range": InsightsOverviewWords.weekRange(start: recap.week_start, end: recap.week_end),
        ])
    }

    /// "{n} sessions · {p}% fewer|more tokens than last week", per harness;
    /// `nil` when last week cannot be compared.
    static func recapChange(_ source: InsightsRecapSource, sessions: UInt32, copy: [String: String]) -> String? {
        guard let permille = source.change_permille else { return nil }
        let key = permille < 0 ? "analytics_recap_fewer" : "analytics_recap_more"
        return fill(text(key, copy), ["n": String(sessions), "p": InsightsOverviewWords.percent(permille.magnitude)])
    }

    /// One item's line, by its rule; `nil` for a kind this build does not know.
    static func recapItem(_ item: InsightsRecapItem, copy: [String: String]) -> String? {
        switch item.kind {
        case "best_cache_week":
            guard let now = item.permille, let before = item.previous_best_permille else { return nil }
            return fill(text("analytics_recap_best_cache", copy), [
                "p": InsightsOverviewWords.percent(now), "q": InsightsOverviewWords.percent(before),
            ])
        case "goal":
            guard let goal = item.goal, let met = item.met else { return nil }
            return fill(text(met ? "analytics_recap_goal_met" : "analytics_recap_goal_not_met", copy),
                        ["goal": Self.goal(goal, copy: copy)])
        case "pattern_up":
            guard let pattern = item.pattern, let up = item.up_permille else { return nil }
            return fill(text("analytics_recap_pattern_up", copy), [
                "pattern": InsightsPatternsWords.title(pattern, copy: copy), "p": InsightsOverviewWords.percent(up),
            ])
        default:
            return nil
        }
    }

    /// "{n} sessions passed your {threshold} threshold.", only when the user
    /// has set a threshold.
    static func recapThreshold(_ recap: InsightsRecap, copy: [String: String]) -> String? {
        guard let past = recap.past_threshold else { return nil }
        return fill(text("analytics_recap_threshold", copy), [
            "n": String(past.sessions), "threshold": InsightsOverviewWords.figure(past.threshold, copy: copy),
        ])
    }
}
