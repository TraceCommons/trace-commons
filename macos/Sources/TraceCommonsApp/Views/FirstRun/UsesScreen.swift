import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// The Uses screen's decisions, apart from the view so they can be tested.
/// Every string is the core's; these only choose and fill placeholders.
enum UsesScreenLayout {
    /// The optional group's checkbox: every optional use on, some, or none.
    enum Group: Equatable {
        case all
        case some
        case none
    }

    /// The floor scope (`consent_options`' `always_on`), shown ticked,
    /// locked and required: it is always included (owner, 2026-10-08).
    static func requiredScope(_ options: [ConsentScope]) -> ConsentScope? {
        options.first(where: \.alwaysOn)
    }

    /// The three optional data uses, always shown independently.
    static func optionalScopes(_ options: [ConsentScope]) -> [ConsentScope] {
        options.filter { !$0.alwaysOn && $0.grantsDataUse }
    }

    /// The optional data uses as Uses lists them: the one that earns credit
    /// (`uses.creditScope`) first, the rest in the daemon's order (owner,
    /// 2026-10-27).
    static func shownOptionalScopes(_ options: [ConsentScope], uses: FirstRunCopy.Uses) -> [ConsentScope] {
        let optional = optionalScopes(options)
        return optional.filter { $0.name == uses.creditScope } + optional.filter { $0.name != uses.creditScope }
    }

    /// The Always on section's title, naming the required use.
    static func alwaysOnTitle(_ required: ConsentScope, uses: FirstRunCopy.Uses) -> String {
        FirstRunCopy.fill(uses.alwaysOn, ["title": required.title])
    }

    /// Scopes that grant no data use (the public handle), beside them.
    static func handleScopes(_ options: [ConsentScope]) -> [ConsentScope] {
        options.filter { !$0.alwaysOn && !$0.grantsDataUse }
    }

    /// What the optional group's expander opens onto: the optional data
    /// uses only. The handle is a row of its own after the group (Ron's
    /// review of #1235, item 2).
    static func expandedScopes(_ options: [ConsentScope]) -> [ConsentScope] {
        optionalScopes(options)
    }

    /// Data uses shown on this screen; public attribution is in profile
    /// settings. The retained argument accepts older callers.
    static func visibleScopes(_ options: [ConsentScope], optionalOpen: Bool) -> [ConsentScope] {
        (requiredScope(options).map { [$0] } ?? []) + optionalScopes(options)
    }

    static func isTicked(_ scope: String, in state: FirstRunState) -> Bool {
        state.scopes.contains(scope)
    }

    static func group(_ scopes: Set<String>, optional: [ConsentScope]) -> Group {
        let on = optional.filter { scopes.contains($0.name) }.count
        if !optional.isEmpty, on == optional.count { return .all }
        return on == 0 ? .none : .some
    }

    /// The two buttons above the optional uses (owner, 2026-10-27), of
    /// equal weight so neither is the suggested answer. The uses start
    /// unticked (R7); only the person's press of one, or of a box, changes
    /// them.
    enum Bulk: CaseIterable {
        /// Every optional data use on. The handle grants no data use and
        /// is not among them (`optionalScopes`).
        case allowAll
        /// Every optional data use off; the required use stays.
        case onlyRequired

        func title(_ uses: FirstRunCopy.Uses) -> String {
            switch self {
            case .allowAll: return uses.allowAll
            case .onlyRequired: return uses.onlyRequired
            }
        }
    }

    /// The state after the person presses `bulk`. Nothing but the optional
    /// data uses changes.
    static func pressing(_ bulk: Bulk, _ state: FirstRunState, optional: [ConsentScope]) -> FirstRunState {
        var pressed = state
        let names = Set(optional.map(\.name))
        switch bulk {
        case .allowAll: pressed.scopes.formUnion(names)
        case .onlyRequired: pressed.scopes.subtract(names)
        }
        return pressed
    }

    /// The Sharing radios' selection: none until the person picks one
    /// (owner, 2026-10-27), then their pick.
    static func sharingSelection(_ state: FirstRunState) -> SharingPath? {
        state.sharingChosen ? state.sharing : nil
    }

    /// The person's pick of a Sharing choice.
    static func choosingSharing(_ state: FirstRunState, _ path: SharingPath?) -> FirstRunState {
        guard let path else { return state }
        var chosen = state
        chosen.sharing = path
        chosen.sharingChosen = true
        return chosen
    }

    /// Start sharing, on Join: the required use known and an account Start
    /// can finish (`FirstRunNavigation.canContinue` on Join), the sharing
    /// line for the path chosen present (the fallback says Starting is
    /// disabled, so it is), and no Start already running. The required use
    /// is not waited on: it is always included (`includingRequired`).
    static func canStart(
        _ state: FirstRunState, uses: FirstRunCopy.Uses, requiredScope: ConsentScope?, grant: AutomaticGrantCopy?,
        isCommitting: Bool
    ) -> Bool {
        guard !isCommitting,
            sharingLine(uses, path: effectiveSharing(state), grant: grant) != uses.sharingUnavailable
        else { return false }
        return FirstRunNavigation.canContinue(state, candidates: [], requiredScope: requiredScope?.name)
    }

    /// Where Start goes.
    enum StartRoute: Equatable {
        /// The two disclosures, then the core's answer, then the commit.
        case disclose
        /// Private AI on without Automatic: the witness disclosure, then
        /// the commit, which asks for no grant.
        case discloseWitness
        /// Straight to the commit.
        case commit
    }

    /// Automatic, on an account that can choose it, goes through both
    /// disclosures; Private AI turned on otherwise goes through the witness
    /// disclosure (spec rule 10); everything else commits directly.
    static func startRoute(_ state: FirstRunState) -> StartRoute {
        if SharingDisclosureFlow.isNeeded(for: state) { return .disclose }
        if SharingDisclosureFlow.isWitnessOnlyNeeded(for: state) { return .discloseWitness }
        return .commit
    }

    /// The state Start commits: the required use always among its scopes,
    /// as its locked, ticked box shows (`FirstRunState.includingRequiredScope`).
    static func includingRequired(_ state: FirstRunState, required: ConsentScope?) -> FirstRunState {
        state.includingRequiredScope(required?.name)
    }

    /// The path Uses shows and Start's line is read for: the person's
    /// choice. Automatic is offered before sign-in (Ron's 2026-10-21
    /// canvas) and granted only at Start with an enrollment
    /// (`FirstRunNavigation.canChooseAutomatic`); until then its line says
    /// it turns on once the person signs in (`sharingOptions`).
    static func effectiveSharing(_ state: FirstRunState) -> SharingPath {
        state.sharing
    }

    /// The Sharing card's line: the first line of the core's words for the
    /// path chosen (owner, 2026-10-08), the loading line until they are
    /// read, or the fallback that disables Start. The rest is
    /// `sharingDetail`, behind an info button; a copy missing either half
    /// reads the fallback, so nothing of the answer is shown without the
    /// rest being there to read.
    static func sharingLine(
        _ uses: FirstRunCopy.Uses, path: SharingPath, grant: AutomaticGrantCopy?, isLoading: Bool = false
    ) -> String {
        if isLoading { return uses.sharingLoading }
        guard let grant, let split = split(path: path, grant: grant) else { return uses.sharingUnavailable }
        return path == .automatic ? (uses.automaticScope ?? split.line) : split.line
    }

    /// The rest of the path's words, behind the Sharing line's info button:
    /// Ask's detail; Automatic's detail, then the scrub's scope and
    /// limit. Nil whenever the line is a fallback.
    static func sharingDetail(path: SharingPath, grant: AutomaticGrantCopy?, uses: FirstRunCopy.Uses? = nil) -> String? {
        guard let grant, let detail = split(path: path, grant: grant)?.detail else { return nil }
        if path == .automatic, let scope = uses?.automaticScope, let scrub = grant.scrub {
            return [scope, scrub.scope, scrub.limit].joined(separator: " ")
        }
        return detail
    }

    private static func split(path: SharingPath, grant: AutomaticGrantCopy) -> (line: String, detail: String)? {
        switch path {
        case .askMe:
            guard let line = grant.pathAskFirstTitle, !line.isEmpty, let detail = grant.pathAskFirstDetail,
                !detail.isEmpty
            else { return nil }
            return (line, detail)
        case .automatic:
            guard let line = grant.pathAutomaticTitle, !line.isEmpty, let detail = grant.pathAutomaticDetail,
                !detail.isEmpty, let scrub = grant.scrub
            else { return nil }
            return (line, [detail, scrub.scope, scrub.limit].joined(separator: " "))
        }
    }

    /// The info button's accessible name for a row titled `title`.
    static func moreAbout(_ title: String, uses: FirstRunCopy.Uses) -> String {
        FirstRunCopy.fill(uses.moreAbout, ["title": title])
    }

    /// The Sharing radios, named by the contribution mode table (Ask,
    /// Automatic), each with the first line of the core's words for it; and
    /// under Automatic, while no enrollment is held, that it turns on once
    /// the person signs in, the last step.
    static func sharingOptions(
        for state: FirstRunState, modes: ContributionModeCopy?, grant: AutomaticGrantCopy?, uses: FirstRunCopy.Uses
    ) -> [GlassRadioOption<SharingPath?>] {
        FirstRunNavigation.sharingPaths(for: state).compactMap { path in
            let mode: ProjectMode = path == .automatic ? .autoUpload : .ask
            guard let modeTitle = modes?.label(for: mode) else { return nil }
            let title = path == .askMe ? (uses.askEachTime ?? modeTitle) : modeTitle
            var lines = [grant.flatMap { split(path: path, grant: $0)?.line }].compactMap { $0 }
            if path == .automatic, let scope = uses.automaticScope { lines = [scope] }
            if path == .automatic, !FirstRunNavigation.canChooseAutomatic(state) {
                lines.append(uses.automaticAfterSignIn)
            }
            return GlassRadioOption(title, value: path, sub: lines.isEmpty ? nil : lines.joined(separator: "\n"))
        }
    }

    /// The gate's label in the Uses graphic: the core's "You review" on Ask,
    /// the mode table's Automatic on Automatic.
    static func gateLabel(_ state: FirstRunState, uses: FirstRunCopy.Uses, modes: ContributionModeCopy?) -> String {
        guard state.sharing == .automatic, let automatic = modes?.label(for: .autoUpload) else { return uses.youReview }
        return automatic
    }

    /// The notice for the runner's failure on this screen, one for each
    /// way Start can end. A refused grant comes after setup finished, so it
    /// reads the first run's own line whatever the label; the Private AI
    /// failure reads the Private AI copy's, or the first run's without it.
    static func notice(
        for failure: FirstRunFailure?, uses: FirstRunCopy.Uses, privateAI: PrivateInferenceCopy?
    ) -> String? {
        switch failure {
        case .none: return nil
        case .grantRefused: return uses.sharingRefused
        case .scopesFailed: return uses.scopesFailed
        case .sharingFailed: return uses.sharingSaveFailed ?? uses.rulesFailed
        case .rulesFailed: return uses.rulesFailed
        case .privateAIFailed: return privateAI?.writeUnconfirmed ?? uses.privateAiFailed
        // The marker was not written, so setup did not finish: its own
        // line, never the refused grant's "Setup finished".
        case .completeFailed: return uses.completeFailed
        // Tools' and sign-in's failures, which never stop Start.
        case .startFailed, .passkeyUnavailable, .settingsFailed, .inviteDead, .lookupUnavailable, .enrollFailed, .signInFailed,
            .nearAIEnrollFailed:
            return nil
        }
    }

    /// What a finished first run must still say once the first-run host has
    /// gone: Automatic was refused, so sharing is on Ask. Shown by the
    /// shell notices (`AppModel.firstRunNotice`), not by this screen.
    static func finishedNotice(_ refusal: FirstRunFailure?, uses: FirstRunCopy.Uses) -> String? {
        if case .grantRefused? = refusal { return uses.sharingRefused }
        return nil
    }

    /// The refusal a Start carries. A fresh pass through the disclosures
    /// supersedes any earlier verdict; a plain Start keeps the earlier one.
    static func refusalToCarry(
        decidedNow: Bool, refused: FirstRunFailure?, pending: FirstRunFailure?
    ) -> FirstRunFailure? {
        decidedNow ? refused : pending
    }

    /// After a Start: the failure to show, and a refusal decided before it
    /// that is still to be shown. A Start that failed keeps the refusal for
    /// the retry, which goes through on Ask; one that succeeded shows it.
    static func afterStart(
        failure: FirstRunFailure?, pending: FirstRunFailure?
    ) -> (shown: FirstRunFailure?, pending: FirstRunFailure?) {
        guard failure == nil else { return (failure, pending) }
        return (pending, nil)
    }
}

/// Start sharing's two commits, on Join, apart from the view so a runner can
/// drive them.
/// Each returns the refusal still to be shown after a later Start.
@MainActor
enum UsesStart {
    /// Start without the disclosures (Ask, watching only). It carries an
    /// earlier refusal; it cannot grant, since only `finish` marks the
    /// grant ready.
    static func plainStart(runner: FirstRunRunner, pending: FirstRunFailure?) async -> FirstRunFailure? {
        let carried = UsesScreenLayout.refusalToCarry(decidedNow: false, refused: nil, pending: pending)
        return await commit(runner: runner, carrying: carried)
    }

    /// Both disclosures seen: the core's answer decides the state Start
    /// commits (Automatic with `grantReady`, or Ask with the refusal),
    /// then Start runs.
    static func finish(
        runner: FirstRunRunner, request: Flow1GrantRequest?, pending: FirstRunFailure?
    ) async -> FirstRunFailure? {
        let (next, refused) = SharingDisclosureFlow.resolve(runner.state, request: request)
        runner.state = next
        let carried = UsesScreenLayout.refusalToCarry(decidedNow: true, refused: refused, pending: pending)
        return await commit(runner: runner, carrying: carried)
    }

    /// The one Start path. A refusal is kept past a failed Start and shown
    /// once one succeeds; the commit clears failures when it begins.
    private static func commit(runner: FirstRunRunner, carrying refusal: FirstRunFailure?) async -> FirstRunFailure? {
        // The refusal goes into the commit too: once the marker is written
        // the host can leave before this function resumes.
        await runner.commit(.start, carrying: refusal)
        // `completeFailed` outranks a refusal the daemon gave in this same
        // Start; that refusal is kept for the retry, like one decided before.
        let kept = runner.failure == .completeFailed ? runner.refusedGrant ?? refusal : refusal
        let after = UsesScreenLayout.afterStart(failure: runner.failure, pending: kept)
        runner.failure = after.shown
        return after.pending
    }
}

// MARK: - The graphic

/// The Uses graphic's choreography (Ron's 2026-10-21 canvas, rulings through
/// 2026-10-22), apart from the view so it can be tested: a soft orb travels
/// a straight line from This Mac to the commons; on Ask it waits at the
/// review gate while the gate fills a muted green and a white thumbs-up pops
/// in with a small wag, on Automatic it passes straight through; each orb
/// that arrives takes the next of five places on the ring round the N, and
/// once full the ring clears and fills again.
enum UsesFlowLayout {
    static let lineStart: CGFloat = 270
    static let lineEnd: CGFloat = 600
    static let lineY: CGFloat = 46
    static let height: CGFloat = 112

    /// A loop's length and when it first starts, in seconds.
    static func period(_ path: SharingPath) -> Double { path == .automatic ? 2.6 : 4.4 }
    static func delay(_ path: SharingPath) -> Double { path == .automatic ? 0.6 : 1 }
    /// Where in a loop an orb arrives at the commons.
    static func arrival(_ path: SharingPath) -> Double { path == .automatic ? 0.92 : 0.93 }

    /// How far through its loop the flow is, `elapsed` seconds after the
    /// path was chosen; nil before the first loop.
    static func loop(_ path: SharingPath, elapsed: Double) -> Double? {
        let into = elapsed - delay(path)
        guard into >= 0 else { return nil }
        return (into / period(path)).truncatingRemainder(dividingBy: 1)
    }

    /// The orb: how far along the line (0 to 1) and how visible.
    static func orb(_ path: SharingPath, loop t: Double) -> (along: Double, opacity: Double) {
        switch path {
        case .askMe:
            let along = keyframes(t, [(0, 0), (0.32, 0.397), (0.64, 0.397), (0.71, 0.573), (0.93, 1), (1, 1)])
            return (along, keyframes(t, [(0, 0), (0.05, 1), (0.93, 1), (1, 0)]))
        case .automatic:
            let along = keyframes(t, [(0, 0), (0.92, 1), (1, 1)])
            return (along, keyframes(t, [(0, 0), (0.08, 1), (0.92, 1), (1, 0)]))
        }
    }

    /// The gate: its green fill, and the thumbs-up's scale, turn (degrees)
    /// and whiteness (0 dim, 1 bright white).
    struct Gate: Equatable {
        var fill: Double
        var scale: Double
        var turn: Double
        var white: Double

        static let rest = Gate(fill: 0, scale: 1, turn: 0, white: 0)
    }

    static func gate(_ path: SharingPath, loop t: Double) -> Gate {
        switch path {
        case .askMe:
            return Gate(fill:
                keyframes(t, [(0, 0), (0.33, 0), (0.37, 1), (0.6, 1), (0.68, 0), (1, 0)]),
                scale: keyframes(t, [(0, 0.86), (0.33, 0.86), (0.36, 1.34), (0.4, 0.94), (0.44, 1.08), (0.48, 1.02), (0.6, 1.02), (0.68, 0.86), (1, 0.86)]),
                turn: keyframes(t, [(0, 0), (0.33, 0), (0.36, -18), (0.4, 9), (0.44, -4), (0.48, 0), (1, 0)]),
                white: keyframes(t, [(0, 0), (0.33, 0), (0.36, 1), (0.6, 1), (0.68, 0), (1, 0)]))
        case .automatic:
            return Gate(
                fill: keyframes(t, [(0, 0), (0.39, 0), (0.42, 1), (0.54, 1), (0.62, 0), (1, 0)]),
                scale: keyframes(t, [(0, 0.86), (0.39, 0.86), (0.42, 1.3), (0.45, 0.95), (0.48, 1.02), (0.54, 1.02), (0.62, 0.86), (1, 0.86)]),
                turn: keyframes(t, [(0, 0), (0.39, 0), (0.42, -16), (0.45, 8), (0.48, 0), (1, 0)]),
                white: keyframes(t, [(0, 0), (0.39, 0), (0.42, 1), (0.54, 1), (0.62, 0), (1, 0)]))
        }
    }

    /// The commons' soft pulse as an orb arrives: its scale and opacity.
    struct Pulse: Equatable {
        var scale: Double
        var opacity: Double

        static let rest = Pulse(scale: 1, opacity: 0)
    }

    static func pulse(_ path: SharingPath, loop t: Double) -> Pulse {
        let at = path == .automatic ? 0.9 : 0.92
        let peak = path == .automatic ? 0.93 : 0.94
        return Pulse(
            scale: keyframes(t, [(0, 0.7), (at, 0.7), (peak, 0.75), (1, 1.45)]),
            opacity: keyframes(t, [(0, 0), (at, 0), (peak, 0.9), (1, 0)]))
    }

    /// How many of the ring's five places are filled: one more at each
    /// arrival, and once all five are, they clear 1.3 seconds later.
    static func satellites(_ path: SharingPath, elapsed: Double) -> Int {
        let first = delay(path) + period(path) * arrival(path)
        guard elapsed >= first else { return 0 }
        let since = elapsed - first
        let index = Int(since / period(path))
        let filled = index % 5 + 1
        if filled == 5, since - Double(index) * period(path) >= 1.3 { return 0 }
        return filled
    }

    /// Linear interpolation through `(time, value)` points.
    static func keyframes(_ t: Double, _ points: [(Double, Double)]) -> Double {
        guard let first = points.first else { return 0 }
        if t <= first.0 { return first.1 }
        for (a, b) in zip(points, points.dropFirst()) where t <= b.0 {
            let span = b.0 - a.0
            return span > 0 ? a.1 + (b.1 - a.1) * (t - a.0) / span : b.1
        }
        return points[points.count - 1].1
    }
}

/// The Uses graphic: the watched tools folded into This Mac, a straight
/// dashed line through the review gate to the commons, the N.
struct UsesFlow: View {
    let copy: FirstRunCopy
    let state: FirstRunState
    let watched: [SourceKind]
    /// When the sharing path was last chosen: the loop restarts from it.
    let since: Date
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        GlassAnimationTimeline(minimumInterval: 1.0 / 60, paused: reduceMotion) { date in
            let elapsed = date.timeIntervalSince(since)
            let path = state.sharing
            let t = reduceMotion ? nil : UsesFlowLayout.loop(path, elapsed: elapsed)
            GeometryReader { geometry in
                let left = (geometry.size.width - FirstRunStageLayout.canvas.width) / 2
                let y = UsesFlowLayout.lineY
                ZStack {
                    Path { line in
                        line.move(to: CGPoint(x: left + UsesFlowLayout.lineStart, y: y))
                        line.addLine(to: CGPoint(x: left + UsesFlowLayout.lineEnd, y: y))
                    }
                    .stroke(GlassTokens.Color.firstRunRule.color, style: StrokeStyle(lineWidth: 2, dash: [4, 6]))
                    mac(at: CGPoint(x: left + 210, y: y))
                    gate(at: CGPoint(x: left + 430, y: y), t: t, path: path)
                    commons(at: CGPoint(x: left + 650, y: y), t: t, path: path, elapsed: elapsed)
                    if let t {
                        let orb = UsesFlowLayout.orb(path, loop: t)
                        FirstRunOrb(diameter: 18)
                            .opacity(orb.opacity)
                            .position(
                                x: left + UsesFlowLayout.lineStart
                                    + (UsesFlowLayout.lineEnd - UsesFlowLayout.lineStart) * CGFloat(orb.along),
                                y: y)
                    }
                    label(copy.uses.thisMac, at: CGPoint(x: left + 210, y: 101))
                    label(UsesScreenLayout.gateLabel(state, uses: copy.uses, modes: ProjectModeWords.table), at: CGPoint(x: left + 430, y: 101))
                    label(copy.uses.theCommons, at: CGPoint(x: left + 650, y: 101))
                }
            }
        }
        .frame(height: UsesFlowLayout.height)
        .frame(maxWidth: .infinity)
    }

    private func mac(at center: CGPoint) -> some View {
        ZStack {
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .fill(GlassTokens.Color.firstRunCard.color)
                .frame(width: 112, height: 78)
                .overlay(alignment: .bottom) {
                    Capsule()
                        .fill(GlassTokens.Color.firstRunRule.color)
                        .frame(width: 44, height: 3)
                        .padding(.bottom, 10)
                }
                .position(x: center.x, y: center.y)
            ForEach(Array(watched.enumerated()), id: \.element) { index, kind in
                let spread: CGFloat = watched.count > 3 ? 19 : 28
                FirstRunToolFace(tool: kind.glassTool)
                    .firstRunActor(FirstRunStageLayout.tileActor(kind))
                    .frame(width: 24, height: 24)
                    .position(x: center.x + (CGFloat(index) - CGFloat(watched.count - 1) / 2) * spread, y: center.y - 8)
            }
        }
        .accessibilityHidden(true)
    }

    private func gate(at center: CGPoint, t: Double?, path: SharingPath) -> some View {
        let look: UsesFlowLayout.Gate = t.map { UsesFlowLayout.gate(path, loop: $0) } ?? .rest
        return ZStack {
            Circle()
                .fill(GlassTokens.Color.firstRunCardSolid.color)
                .shadow(color: GlassTokens.Color.firstRunTileShadow.color, radius: 10, y: 8)
            approval
                .opacity(look.fill)
            thumb(look)
        }
        .frame(width: 58, height: 58)
        .position(center)
        .accessibilityHidden(true)
    }

    /// The gate's green as a trace is approved: deeper toward the rim.
    private var approval: some View {
        Circle()
            .fill(RadialGradient(
                colors: [GlassTokens.Color.firstRunApprove.color, GlassTokens.Color.firstRunApproveDeep.color],
                center: UnitPoint(x: 0.5, y: 0.38), startRadius: 0, endRadius: 30))
    }

    /// The thumbs-up: dim at rest, bright white as it pops in and wags.
    private func thumb(_ look: UsesFlowLayout.Gate) -> some View {
        let anchor = UnitPoint(x: 0.5, y: 0.6)
        return ZStack {
            Image(systemName: "hand.thumbsup")
                .glassGlyph(22, weight: .regular)
                .foregroundStyle(GlassColor.textSecondary)
                .opacity(0.6 * (1 - look.white))
            Image(systemName: "hand.thumbsup.fill")
                .glassGlyph(22, weight: .regular)
                .foregroundStyle(GlassTokens.Color.heroInk.color)
                .opacity(look.white)
        }
        .scaleEffect(CGFloat(look.scale), anchor: anchor)
        .rotationEffect(.degrees(look.turn), anchor: anchor)
    }

    private func commons(at center: CGPoint, t: Double?, path: SharingPath, elapsed: Double) -> some View {
        let filled = reduceMotion ? 0 : UsesFlowLayout.satellites(path, elapsed: elapsed)
        let pulse: UsesFlowLayout.Pulse = t.map { UsesFlowLayout.pulse(path, loop: $0) } ?? .rest
        let turn: Double = reduceMotion ? 0 : (elapsed / 40).truncatingRemainder(dividingBy: 1) * 360
        return ZStack {
            commonsPulse(pulse)
            satellites(filled)
                .rotationEffect(.degrees(turn))
            FirstRunHub()
                .firstRunActor(FirstRunStageLayout.hubActor)
                .frame(width: 52, height: 52)
        }
        .position(center)
        .accessibilityHidden(true)
    }

    /// The commons' soft gradient ring of light as a trace arrives.
    private func commonsPulse(_ pulse: UsesFlowLayout.Pulse) -> some View {
        let beam = GlassTokens.Color.firstRunBeam
        let ring = RadialGradient(
            stops: [
                .init(color: beam.opacity(0).color, location: 0.3),
                .init(color: beam.opacity(0.6).color, location: 0.46),
                .init(color: beam.opacity(0).color, location: 0.64),
            ],
            center: .center, startRadius: 0, endRadius: 60)
        return Circle()
            .fill(ring)
            .frame(width: 120, height: 120)
            .blur(radius: 3)
            .scaleEffect(CGFloat(pulse.scale))
            .opacity(pulse.opacity)
    }

    /// The ring round the N, each arrived trace a soft orb in its place.
    private func satellites(_ filled: Int) -> some View {
        ZStack {
            Circle()
                .strokeBorder(GlassTokens.Color.firstRunRule.color, style: StrokeStyle(lineWidth: 1, dash: [3, 4]))
            ForEach(0..<5, id: \.self) { place in
                satellite(place, filled: filled)
            }
        }
        .frame(width: 92, height: 92)
    }

    private func satellite(_ place: Int, filled: Int) -> some View {
        let at = FirstRunStageLayout.point(
            CGPoint(x: 46, y: 46), radius: 46, degrees: FirstRunStageLayout.ringAngle(place, of: 5))
        let shown = place < filled
        return FirstRunOrb(diameter: 14)
            .scaleEffect(shown ? 1 : 0)
            .opacity(shown ? 1 : 0)
            .animation(GlassMotion.expressive(reduceMotion, duration: 0.46), value: filled)
            .position(at)
    }

    private func label(_ text: String, at point: CGPoint) -> some View {
        Text(text)
            .glassType(GlassTokens.TypeScale.caption.weight(.semibold))
            .foregroundStyle(GlassColor.textSecondary)
            .frame(width: 160)
            .multilineTextAlignment(.center)
            .position(point)
    }
}

// MARK: - The screen

/// Uses: how traces may be used and how they are shared, under the flow
/// from This Mac through the review gate to the commons. Continue moves on;
/// what is chosen here is sent at Start sharing, on Join.
struct UsesScreen: View {
    @EnvironmentObject private var model: AppModel
    let copy: FirstRunCopy
    @ObservedObject var runner: FirstRunRunner

    /// The core's sharing words for this configuration, read once on
    /// appear; nil shows the fallback line, and Start stays disabled.
    @State private var grant: AutomaticGrantCopy?
    @State private var grantRead = false
    /// The Always on section starts collapsed.
    @State private var requiredOpen = false
    @State private var flowSince = Date()
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        let options = model.consentScopes
        let required = UsesScreenLayout.requiredScope(options)
        FirstRunFrame(
            copy: copy,
            state: $runner.state,
            isCommitting: runner.isCommitting,
            footer: FirstRunFooter(
                title: copy.frame.continueButton,
                isEnabled: FirstRunNavigation.canContinue(runner.state, candidates: [], requiredScope: required?.name),
                action: { runner.state = FirstRunNavigation.next(runner.state) })
        ) {
            UsesFlow(copy: copy, state: runner.state, watched: ToolsScreenLayout.watchedKinds(runner.state), since: flowSince)
        } title: {
            FirstRunTitle(light: copy.uses.titleLight, bold: copy.uses.titleBold)
        } content: {
            ScrollView {
                list(options: options, required: required)
                    .frame(maxWidth: 460, alignment: .leading)
                    .firstRunCard()
                    .frame(maxWidth: .infinity)
            }
            .frame(maxWidth: 520)
            .scrollBounceBehavior(.basedOnSize)
        }
        // The required use is in the state as soon as it is known, so the
        // state matches its ticked box; Start includes it again regardless.
        .onChange(of: required?.name, initial: true) { _, _ in includeRequired() }
        .onChange(of: runner.state.sharing) { _, _ in flowSince = Date() }
        .task {
            model.refreshStatus()
            model.refreshConsentOptions()
            if !grantRead {
                grant = AutomaticGrantCopy.decode(
                    fromJSON: TCCoreCopy.automaticContributionCopyJSON(configDir: model.configDirectory))
                grantRead = true
            }
        }
    }

    private func list(options: [ConsentScope], required: ConsentScope?) -> some View {
        let optional = UsesScreenLayout.optionalScopes(options)
        return VStack(alignment: .leading, spacing: 0) {
            if !optional.isEmpty {
                Text(copy.uses.rewardOpportunities ?? "")
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.bottom, GlassTokens.Space.s3)
            }
            if !optional.isEmpty {
                HStack(spacing: GlassTokens.Space.s3) {
                    // One style and one width for both: neither is the
                    // suggested answer.
                    ForEach(UsesScreenLayout.Bulk.allCases, id: \.self) { bulk in
                        Button {
                            runner.state = UsesScreenLayout.pressing(bulk, runner.state, optional: optional)
                        } label: {
                            Text(bulk.title(copy.uses)).frame(maxWidth: .infinity)
                        }
                        .buttonStyle(GlassButtonStyle(.secondary))
                    }
                }
                .padding(.top, GlassTokens.Space.s3)
                ForEach(UsesScreenLayout.shownOptionalScopes(options, uses: copy.uses)) { option in
                    scopeRow(option)
                }
            }
            // The required use, in a collapsed section under the choices
            // (owner, 2026-10-28): every box above is a real choice, so
            // Only required leaves them all empty.
            if let required {
                GlassExpander(UsesScreenLayout.alwaysOnTitle(required, uses: copy.uses), isOpen: $requiredOpen)
                    .padding(.top, GlassTokens.Space.s3)
                if requiredOpen {
                    VStack(alignment: .leading, spacing: 0) {
                        HStack(spacing: GlassTokens.Space.s2) {
                            // The baseline permission is required for sharing.
                            Text(required.title)
                                .glassType(GlassTokens.TypeScale.label.weight(.semibold))
                            // Keep the required-sharing label inline rather than as a tag.
                            Text(copy.uses.required)
                                .glassType(GlassTokens.TypeScale.mono)
                                .foregroundStyle(GlassTokens.Color.statusOnText.color)
                        }
                        .frame(minHeight: 32)
                        Text(required.description)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textSecondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    .padding(.leading, GlassTokens.Space.s5)
                }
            }
            GlassHairline()
                .padding(.top, GlassTokens.Space.s3)
            HStack(spacing: GlassTokens.Space.s2) {
                eyebrow(copy.uses.sharing)
                if let detail = UsesScreenLayout.sharingDetail(path: runner.state.sharing, grant: grant, uses: copy.uses) {
                    infoButton(title: copy.uses.sharing, text: detail)
                }
            }
            if grantRead, grant == nil {
                Text(copy.uses.sharingUnavailable)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
            }
            GlassRadioGroup(
                copy.uses.sharing,
                selection: Binding(
                    get: { UsesScreenLayout.sharingSelection(runner.state) },
                    set: { runner.state = UsesScreenLayout.choosingSharing(runner.state, $0) }),
                options: UsesScreenLayout.sharingOptions(
                    for: runner.state, modes: ProjectModeWords.table, grant: grant, uses: copy.uses))
                .padding(.bottom, GlassTokens.Space.s3)
        }
    }

    private func eyebrow(_ text: String) -> some View {
        Text(text)
            .glassType(GlassTokens.TypeScale.eyebrow)
            .foregroundStyle(GlassColor.textTertiary)
            .padding(.top, GlassTokens.Space.s4)
            .padding(.bottom, GlassTokens.Space.s2)
    }

    /// Each permission names its use and shows the scope before choosing.
    private func scopeRow(_ option: ConsentScope) -> some View {
        Toggle(isOn: scope(option.name)) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                Text(option.title).glassType(GlassTokens.TypeScale.label.weight(.semibold))
                Text(option.description)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                // The use that earns credit says so, in the required use's
                // inline style, no louder.
                if option.name == copy.uses.creditScope {
                    Text(copy.uses.creditRequired)
                        .glassType(GlassTokens.TypeScale.mono)
                        .foregroundStyle(GlassTokens.Color.statusOnText.color)
                }
            }
        }
        .toggleStyle(GlassCheckboxStyle())
        .padding(.vertical, GlassTokens.Space.s3)
    }

    private func infoButton(title: String, text: String) -> some View {
        GlassInfoButton(UsesScreenLayout.moreAbout(title, uses: copy.uses), text: text)
    }

    /// One optional scope's box. Each starts unticked; the person's toggle,
    /// or their press of Allow all, is the only thing that ticks one. The required
    /// one is always included (`includeRequired`).
    private func scope(_ name: String) -> Binding<Bool> {
        Binding(
            get: { runner.state.scopes.contains(name) },
            set: { on in
                if on {
                    runner.state.scopes.insert(name)
                } else {
                    runner.state.scopes.remove(name)
                }
            })
    }

    /// Put the required use in the state, once it is known.
    private func includeRequired() {
        let included = UsesScreenLayout.includingRequired(
            runner.state, required: UsesScreenLayout.requiredScope(model.consentScopes))
        if included != runner.state { runner.state = included }
    }
}
