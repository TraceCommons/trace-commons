import AppKit
import SwiftUI
import TCBridge
import TCDesign
import TCShellCore
import UniformTypeIdentifiers

/// What a folder picked with "add your tool" turned out to be.
enum AddToolOutcome: Equatable {
    /// One kind matched; the folder is added as that kind.
    case added(AddedFolder)
    /// More than one kind matched (a flat folder of `.json` files is both an
    /// OpenCode export and a trajectory export): the person picks which.
    /// `kinds` holds only the matches this build can offer.
    case ask(path: String, kinds: [AddedFolder.Kind])
    /// Nothing this build can declare matched, or the core could not
    /// describe the folder. Nothing is added.
    case refused
}

/// The Tools screen's decisions, apart from the view so they can be tested.
/// Every write goes through `FirstRunState.add(_:)`, `answerAdded`, or, on
/// a tool row, `ToolAnswerRowLayout.select`.
enum ToolsScreenLayout {
    /// The `tc_describe_folder` JSON for `path` as an outcome. Decoded with
    /// `FolderMatch.decodeList`, which keeps every row: a dropped trajectory
    /// row would turn a question into a certainty. A nil or undecodable
    /// answer is a refusal, so a folder the core could not read is never
    /// added on a guess.
    static func outcome(path: String, json: String?) -> AddToolOutcome {
        guard let json, let matches = try? FolderMatch.decodeList(from: json) else { return .refused }
        return outcome(path: path, matches: matches)
    }

    static func outcome(path: String, matches: [FolderMatch]) -> AddToolOutcome {
        let offerable = matches.compactMap { match -> AddedFolder.Kind? in
            switch match.kind {
            case .source(let kind): return .source(kind)
            case .trajectory: return .trajectory
            case .unrecognised: return nil
            }
        }
        if matches.count == 1, let kind = offerable.first {
            return .added(AddedFolder(kind: kind, path: matches[0].path))
        }
        // More than one match is a question even when only one of them can
        // be offered: the core said the folder is not certainly that kind.
        if matches.count > 1, !offerable.isEmpty {
            return .ask(path: path, kinds: offerable)
        }
        return .refused
    }

    /// Apply an outcome. An added folder is written as a row of its own,
    /// unanswered (Ron's review of #1235, item 3); a question writes nothing
    /// until it is answered. False for a refusal.
    @discardableResult
    static func apply(_ outcome: AddToolOutcome, to state: inout FirstRunState) -> Bool {
        switch outcome {
        case .added(let folder):
            state.add(folder)
            return true
        case .ask:
            return true
        case .refused:
            return false
        }
    }

    /// The person's answer to an ambiguous folder: which kind it is. Its
    /// row is then added, unanswered like any other.
    static func choose(_ kind: AddedFolder.Kind, path: String, in state: inout FirstRunState) {
        state.add(AddedFolder(kind: kind, path: path))
    }

    /// The folders added for a tool, each a row of its own. A folder of
    /// exported traces has its own row (`trajectoryFolder`).
    static func addedRows(_ state: FirstRunState) -> [AddedFolder] {
        state.addedFolders.filter { $0.kind != .trajectory }
    }

    /// An added row's answer as its picker shows it; nil is "Choose…".
    static func addedAnswer(_ folder: AddedFolder) -> ToolAnswer? {
        folder.watched.map { $0 ? .watch : .dontUse }
    }

    /// Answer an added row.
    static func selectAdded(_ answer: ToolAnswer?, path: String, in state: inout FirstRunState) {
        state.answerAdded(path: path, watched: answer.map { $0 == .watch })
    }

    /// The core's line beside an added folder that reads Watch while its
    /// tool is watched in another row too, naming the tool; nil otherwise.
    static func conflict(_ folder: AddedFolder, in state: FirstRunState, copy: FirstRunCopy.Tools) -> String? {
        guard folder.watched == true, case .source(let kind) = folder.kind, state.watchedTwice.contains(kind)
        else { return nil }
        return FirstRunCopy.fill(copy.oneFolderPerTool, ["tool": kind.displayName])
    }

    /// An added row's folder button: the newly picked folder, described,
    /// replaces the row. Picking a folder for a row watches it, as on every
    /// other row; a question replaces the row with the question, and a
    /// refusal leaves the row as it was. False for a refusal.
    @discardableResult
    static func replace(path: String, with outcome: AddToolOutcome, in state: inout FirstRunState) -> Bool {
        switch outcome {
        case .refused:
            return false
        case .ask:
            state.addedFolders.removeAll { $0.path == path }
            return true
        case .added(let folder):
            state.addedFolders.removeAll { $0.path == path }
            state.add(folder)
            state.answerAdded(path: folder.path, watched: true)
            return true
        }
    }

    /// One option on an ambiguous folder's picker: a kind it matched, by its
    /// index in the question's kinds, or the core's "Neither".
    enum PendingAnswer: Hashable {
        case kind(Int)
        case neither
    }

    /// The ambiguous folder's options: each kind it matched, then the
    /// core's `neither`, which closes the question with nothing added.
    static func pendingOptions(
        _ kinds: [AddedFolder.Kind], copy: FirstRunCopy.Tools
    ) -> [GlassPickerOption<PendingAnswer>] {
        kinds.indices.map { GlassPickerOption(name(kinds[$0], copy: copy), value: .kind($0)) }
            + [GlassPickerOption(copy.neither, value: .neither, dot: .off)]
    }

    /// Answer an ambiguous folder. A kind is added as `choose` adds it;
    /// Neither leaves the state as it is, so the folder is attributed to no
    /// tool. True when the question is closed; an index the picker never
    /// offered keeps it open and writes nothing.
    static func answer(
        _ answer: PendingAnswer, path: String, kinds: [AddedFolder.Kind], in state: inout FirstRunState
    ) -> Bool {
        switch answer {
        case .neither:
            return true
        case .kind(let index):
            guard kinds.indices.contains(index) else { return false }
            choose(kinds[index], path: path, in: &state)
            return true
        }
    }

    /// The core's line for a refused folder.
    static func refusal(_ copy: FirstRunCopy.Tools) -> String {
        copy.addToolRefused
    }

    /// An option's name: a tool's display name, or the core's label for a
    /// folder of exported traces.
    static func name(_ kind: AddedFolder.Kind, copy: FirstRunCopy.Tools) -> String {
        switch kind {
        case .source(let source): return source.displayName
        case .trajectory: return copy.trajectoryLabel
        }
    }

    /// The core's question for an ambiguous folder, naming it as its card
    /// does (`folderName`).
    static func question(path: String, copy: FirstRunCopy.Tools) -> String {
        FirstRunCopy.fill(copy.whichKind, ["folder": folderName(path)])
    }

    /// A picked folder's name: its last path component.
    static func folderName(_ path: String) -> String {
        URL(fileURLWithPath: path).lastPathComponent
    }

    /// The trajectory row's picker. "Not used" withdraws the folder;
    /// the placeholder and "Watch this folder" leave it as it is.
    static func selectTrajectory(_ answer: ToolAnswer?, in state: inout FirstRunState) {
        if answer == .dontUse { state.withdrawTrajectory() }
    }

    /// Continue: discovery read and the scan over, nothing committing, no
    /// question open, and something watched (`FirstRunNavigation`).
    static func canContinue(
        discovered: [SourceCandidate]?, state: FirstRunState, pending: Bool, isCommitting: Bool,
        scanned: Bool = true
    ) -> Bool {
        guard let discovered, scanned, !isCommitting, !pending else { return false }
        return FirstRunNavigation.canContinue(
            state, candidates: rows(discovered, state: state), requiredScope: nil)
    }

    /// Customize, beside Continue, opens once a tool is watched, as Continue
    /// does (Ron, 2026-10-21), and closes again at any time.
    static func customizeEnabled(state: FirstRunState, open: Bool, scanned: Bool, isCommitting: Bool) -> Bool {
        if open { return !isCommitting }
        return scanned && !isCommitting && state.watchesAnything
    }

    /// The tools the list offers: those found on this Mac, in the ring's
    /// order, and any the state watches at a folder the person gave. A tool
    /// not found has left the ring (Ron, 2026-10-21: "sorted out of view"),
    /// so the list does not ask about it.
    static func listed(_ discovered: [SourceCandidate], state: FirstRunState) -> [SourceCandidate] {
        FirstRunStageLayout.ringRows(rows(discovered, state: state)).filter(\.exists)
    }

    /// A tool's check: ticked is Watch, anything else is Not used.
    static func isWatched(_ candidate: SourceCandidate, in state: FirstRunState) -> Bool {
        ToolAnswerRowLayout.answer(in: state, for: candidate) == .watch
    }

    /// Tick or untick a tool's check.
    static func setWatched(_ on: Bool, _ candidate: SourceCandidate, in state: inout FirstRunState) {
        ToolAnswerRowLayout.select(on ? .watch : .dontUse, for: candidate, in: &state)
    }

    /// The watched tools, in ring order: the ones Customize lists repos
    /// under, Uses folds into This Mac, and Traces counts.
    static func watchedKinds(_ state: FirstRunState) -> [SourceKind] {
        let roots = state.sessionRoots
        return FirstRunStageLayout.ringOrder.filter {
            if case .watch = roots[$0] { return true }
            return false
        }
    }

    /// The repos under one watched tool: those found in its traces.
    static func repos(for kind: SourceKind, in projects: [ProjectRow]) -> [ProjectRow] {
        projects.filter { project in project.tools.contains { $0.source == kind.rawValue } }
    }

    /// Whether a described folder may still change the state. A drop and an
    /// off-main describe answer later; by then a commit may hold the start
    /// snapshot, or the screen may have moved on.
    static func acceptsFolder(step: FirstRunStep, isCommitting: Bool) -> Bool {
        step == .tools && !isCommitting
    }

    /// Discovery's rows. A kind discovery did not find that the state
    /// watches through its own row (a restored state) is shown found at that
    /// folder, so the row does not vanish while the watch stays declared.
    /// Folders added with the tile are rows of their own (`addedRows`).
    static func rows(_ discovered: [SourceCandidate], state: FirstRunState) -> [SourceCandidate] {
        var rows = discovered.map { candidate in
            personalPath(for: candidate.source, discovered: discovered, in: state)
                .map { added(candidate.source, at: $0) } ?? candidate
        }
        for kind in SourceKind.allCases where !rows.contains(where: { $0.source == kind }) {
            if let path = personalPath(for: kind, discovered: discovered, in: state) {
                rows.append(added(kind, at: path))
            }
        }
        return rows
    }

    /// A tool card's caption: "Added by you" for a folder the person gave
    /// through a missing tool's row, nothing otherwise. Tool cards show no
    /// session count and no "not found" line (owner, 2026-10-08).
    static func meta(
        for candidate: SourceCandidate, in state: FirstRunState, discovered: [SourceCandidate],
        copy: FirstRunCopy
    ) -> String? {
        personalPath(for: candidate.source, discovered: discovered, in: state) != nil ? copy.tools.addedByYou : nil
    }

    /// The added trajectory folder, which has no tool row of its own.
    static func trajectoryFolder(in state: FirstRunState) -> AddedFolder? {
        state.addedFolders.first { $0.kind == .trajectory }
    }

    /// The folder the person gave for `kind` through its row: a watch of a
    /// kind discovery did not find. Nil for discovery's own row.
    private static func personalPath(
        for kind: SourceKind, discovered: [SourceCandidate], in state: FirstRunState
    ) -> String? {
        let found = discovered.contains { $0.source == kind && $0.exists }
        if !found, case .watch(let path) = state.rowAnswer(kind) { return path }
        return nil
    }

    private static func added(_ kind: SourceKind, at path: String) -> SourceCandidate {
        SourceCandidate(
            source: kind, path: path, exists: true, sessionCount: 0, mostRecent: nil, relocatedByEnv: false)
    }
}

/// A folder that matched more than one kind, waiting for the person.
struct PendingFolder: Equatable {
    let path: String
    let kinds: [AddedFolder.Kind]
}

/// The add-tool flow the two tool screens share (owner, 2026-10-08: Quick's
/// Folders and Custom's Tools both offer it, with one component and one
/// behaviour): a picked or dropped folder is described by the core off the
/// main actor, and its answer lands only while a tool screen is up and
/// nothing is committing (`ToolsScreenLayout.acceptsFolder`).
@MainActor
final class AddToolModel: ObservableObject {
    @Published var pending: PendingFolder?
    @Published var refused = false

    /// Describe a picked folder and act on what the core reports.
    /// `replacing` names an added row whose folder button picked `path`.
    func describe(_ path: String, replacing: String? = nil, runner: FirstRunRunner) {
        guard Self.accepting(runner) else { return }
        Task.detached(priority: .userInitiated) {
            let json = TCDiscovery.describeFolderJSON(path)
            await MainActor.run {
                self.land(ToolsScreenLayout.outcome(path: path, json: json), replacing: replacing, runner: runner)
            }
        }
    }

    func land(_ outcome: AddToolOutcome, replacing: String? = nil, runner: FirstRunRunner) {
        guard Self.accepting(runner) else { return }
        if let replacing {
            refused = !ToolsScreenLayout.replace(path: replacing, with: outcome, in: &runner.state)
        } else {
            refused = !ToolsScreenLayout.apply(outcome, to: &runner.state)
        }
        if case .ask(let path, let kinds) = outcome {
            pending = PendingFolder(path: path, kinds: kinds)
        } else {
            pending = nil
        }
    }

    static func accepting(_ runner: FirstRunRunner) -> Bool {
        ToolsScreenLayout.acceptsFolder(step: runner.state.step, isCommitting: runner.isCommitting)
    }
}


// MARK: - The ring

/// Where each tile is in the Tools ring as the scan runs (Ron's 2026-10-21
/// canvas, 2026-10-22 rulings): every tile bursts out of the N into a ring,
/// one sweep turns once round it, each tile settles as the sweep passes, a
/// tool not found drifts out of view right then, and once the turn is over
/// the rest re-space and the ring tightens. Pure, so it can be tested.
enum ToolsRingLayout {
    /// The graphic's height, its centre's height, and its sizes.
    static let height: CGFloat = 300
    static let centerY: CGFloat = 158
    static let looseRadius: CGFloat = 130
    static let tightRadius: CGFloat = 118
    static let tile: CGFloat = 52
    static let hub: CGFloat = 64
    static let goneRadius: CGFloat = 470

    struct Placement: Equatable {
        var angle: Double
        var radius: CGFloat
        var scale: CGFloat
        var opacity: Double
    }

    /// Tile `index` of the ring's `rows`, `elapsed` seconds into the scan,
    /// or settled when `elapsed` is nil (the scan already ran, or Reduce
    /// Motion is on).
    static func placement(_ index: Int, rows: [SourceCandidate], elapsed: Double?) -> Placement {
        let found = rows.enumerated().filter { $0.element.exists }.map(\.offset)
        let start = FirstRunStageLayout.ringAngle(index, of: rows.count)
        let gone = !rows[index].exists
        let settled = found.firstIndex(of: index).map { FirstRunStageLayout.ringAngle($0, of: found.count) } ?? start
        guard let elapsed else {
            return gone
                ? Placement(angle: start, radius: goneRadius, scale: 0.6, opacity: 0)
                : Placement(angle: settled, radius: tightRadius, scale: 1, opacity: 1)
        }
        let out = FirstRunStageLayout.burst(elapsed - (0.26 + 0.08 * Double(index)), duration: 0.78)
        var placement = Placement(
            angle: start, radius: looseRadius * CGFloat(out), scale: CGFloat(0.3 + 0.7 * min(1, out)),
            opacity: min(1, out * 1.6))
        if gone {
            let leaving = elapsed - (FirstRunStageLayout.scanMoment(index, of: rows.count) + 0.12)
            let exit = pow(min(1, max(0, leaving / 0.8)), 2)
            placement.radius += (goneRadius - looseRadius) * CGFloat(exit)
            placement.scale *= CGFloat(1 - 0.4 * exit)
            placement.opacity *= 1 - exit
        } else if FirstRunStageLayout.scanDone(after: elapsed) {
            let respace = GlassMotionCurve.value((elapsed - FirstRunStageLayout.scanLead - FirstRunStageLayout.scanTurn - 0.25) / 0.9)
            placement.angle = start + (settled - start) * respace
            placement.radius = looseRadius + (tightRadius - looseRadius) * CGFloat(respace)
        }
        return placement
    }

    /// The flash round a tile as the sweep reaches it: 0 to 1 over its half
    /// second, or nil outside it.
    static func flash(_ index: Int, of count: Int, elapsed: Double?) -> Double? {
        guard let elapsed else { return nil }
        let into = (elapsed - (FirstRunStageLayout.scanMoment(index, of: count) - 0.06)) / 0.52
        return into >= 0 && into <= 1 ? into : nil
    }

    /// A tile's gentle bob, each tool on its own phase.
    static func bob(_ kind: SourceKind, at time: TimeInterval) -> CGFloat {
        let phase: Double = [0, 1.1, 2.3, 3.4, 4.2][FirstRunStageLayout.ringOrder.firstIndex(of: kind) ?? 0]
        return CGFloat(-2 + 2 * cos(2 * .pi * (time + phase) / 5))
    }
}

/// The Tools graphic: the N at the centre, the dashed ring turning slowly,
/// the sweep, the tiles, and a soft beam of light from each watched tile
/// into the N, dissolving into the glow behind it (Ron, 2026-10-22).
struct ToolsRing: View {
    let rows: [SourceCandidate]
    let state: FirstRunState
    /// Seconds since the scan started, as of `date`; nil once settled.
    let scanStart: Date?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        GlassAnimationTimeline(minimumInterval: scanStart == nil ? 1.0 / 30 : nil, paused: reduceMotion) { date in
            let elapsed = scanStart.map { date.timeIntervalSince($0) }
            let settled = elapsed.map(FirstRunStageLayout.scanDone(after:)) ?? true
            let time = date.timeIntervalSinceReferenceDate
            GeometryReader { geometry in
                let center = CGPoint(x: geometry.size.width / 2, y: ToolsRingLayout.centerY)
                ZStack {
                    ring(center: center, tight: settled, time: time, elapsed: elapsed)
                    if let rotation = elapsed.flatMap(FirstRunStageLayout.sweepRotation(after:)) {
                        sweep(rotation: rotation, center: center, elapsed: elapsed ?? 0)
                    }
                    ForEach(Array(rows.enumerated()), id: \.element.source) { index, row in
                        if row.exists {
                            ToolsBeam(
                                from: FirstRunStageLayout.point(center, radius: 90, degrees: settledAngle(index)),
                                to: FirstRunStageLayout.point(center, radius: 16, degrees: settledAngle(index)),
                                time: time)
                                .opacity(settled && ToolsScreenLayout.isWatched(row, in: state) ? 1 : 0)
                                .animation(GlassMotion.timed(reduceMotion, duration: 0.5), value: ToolsScreenLayout.isWatched(row, in: state))
                        }
                    }
                    FirstRunHub(charged: !ToolsScreenLayout.watchedKinds(state).isEmpty)
                        .firstRunActor(FirstRunStageLayout.hubActor)
                        .frame(width: ToolsRingLayout.hub, height: ToolsRingLayout.hub)
                        .position(center)
                    ForEach(Array(rows.enumerated()), id: \.element.source) { index, row in
                        tile(index, row: row, center: center, elapsed: elapsed, time: time)
                    }
                }
            }
        }
        .frame(height: ToolsRingLayout.height)
        .frame(maxWidth: .infinity)
        .accessibilityHidden(true)
    }

    private func settledAngle(_ index: Int) -> Double {
        ToolsRingLayout.placement(index, rows: rows, elapsed: nil).angle
    }

    private func ring(center: CGPoint, tight: Bool, time: TimeInterval, elapsed: Double?) -> some View {
        Circle()
            .stroke(GlassTokens.Color.firstRunRule.color, style: StrokeStyle(lineWidth: 1.5, dash: [3, 7]))
            .frame(width: ToolsRingLayout.looseRadius * 2, height: ToolsRingLayout.looseRadius * 2)
            .rotationEffect(.degrees(reduceMotion ? 0 : (time / 90).truncatingRemainder(dividingBy: 1) * 360))
            .scaleEffect(tight ? ToolsRingLayout.tightRadius / ToolsRingLayout.looseRadius : 1)
            .animation(GlassMotion.spatial(reduceMotion), value: tight)
            .opacity(min(1, max(0, ((elapsed ?? 10) - 0.3) / 0.7)))
            .position(center)
    }

    /// The radar sweep: a soft band with a long trail of light behind it,
    /// blurred, and clear near the N.
    private func sweep(rotation: Double, center: CGPoint, elapsed: Double) -> some View {
        let glow = GlassTokens.Color.firstRunGlow
        let beam = GlassTokens.Color.firstRunBeam
        let turnEnd = FirstRunStageLayout.scanLead + FirstRunStageLayout.scanTurn
        let fade = min(1, max(0, (elapsed - FirstRunStageLayout.scanLead) / 0.4), max(0, (turnEnd - elapsed) / 0.4))
        return Circle()
            .fill(AngularGradient(
                stops: [
                    .init(color: glow.opacity(0).color, location: 0),
                    .init(color: glow.opacity(0).color, location: 140.0 / 360),
                    .init(color: glow.opacity(0.03).color, location: 210.0 / 360),
                    .init(color: glow.opacity(0.11).color, location: 280.0 / 360),
                    .init(color: glow.opacity(0.24).color, location: 330.0 / 360),
                    .init(color: beam.opacity(0.4).color, location: 352.0 / 360),
                    .init(color: beam.opacity(0.5).color, location: 358.0 / 360),
                    .init(color: glow.opacity(0).color, location: 1),
                ],
                center: .center, startAngle: .degrees(-90), endAngle: .degrees(270)))
            .mask(RadialGradient(
                stops: [.init(color: .clear, location: 36.0 / 150), .init(color: .black, location: 64.0 / 150)],
                center: .center, startRadius: 0, endRadius: 150))
            .frame(width: 300, height: 300)
            .rotationEffect(.degrees(rotation))
            .blur(radius: 4)
            .opacity(fade)
            .position(center)
    }

    private func tile(_ index: Int, row: SourceCandidate, center: CGPoint, elapsed: Double?, time: TimeInterval) -> some View {
        let placement = ToolsRingLayout.placement(index, rows: rows, elapsed: elapsed)
        let point = FirstRunStageLayout.point(center, radius: placement.radius, degrees: placement.angle)
        let watched = ToolsScreenLayout.isWatched(row, in: state)
        return FirstRunToolFace(tool: row.source.glassTool, lit: watched)
            .firstRunActor(FirstRunStageLayout.tileActor(row.source))
            .frame(width: ToolsRingLayout.tile, height: ToolsRingLayout.tile)
            .overlay {
                if let flash = ToolsRingLayout.flash(index, of: rows.count, elapsed: elapsed) {
                    RoundedRectangle(cornerRadius: 19, style: .continuous)
                        .strokeBorder(GlassTokens.Color.firstRunGlow.color, lineWidth: 2)
                        .padding(-7)
                        .scaleEffect(0.88 + 0.28 * flash)
                        .opacity(flash < 0.4 ? flash / 0.4 : (1 - flash) / 0.6)
                }
            }
            .offset(y: reduceMotion ? 0 : ToolsRingLayout.bob(row.source, at: time))
            .scaleEffect(placement.scale)
            .opacity(placement.opacity)
            .position(point)
    }
}

/// One watched tile's beam into the N: a wide halo, a glowing body, a
/// softened core and soft blobs of light flowing in, every layer blurred
/// and fading toward the N so the light has no hard edge anywhere.
private struct ToolsBeam: View {
    let from: CGPoint
    let to: CGPoint
    let time: TimeInterval
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        Canvas { context, _ in
            var line = Path()
            line.move(to: from)
            line.addLine(to: to)
            let beam = GlassTokens.Color.firstRunBeam
            let core = GlassTokens.Color.firstRunBeamCore
            let body = GraphicsContext.Shading.linearGradient(
                Gradient(stops: [
                    .init(color: beam.opacity(0.95).color, location: 0),
                    .init(color: beam.opacity(0.4).color, location: 0.55),
                    .init(color: beam.opacity(0).color, location: 1),
                ]),
                startPoint: from, endPoint: to)
            let pulses = GraphicsContext.Shading.linearGradient(
                Gradient(stops: [
                    .init(color: core.color, location: 0),
                    .init(color: core.opacity(0.45).color, location: 0.6),
                    .init(color: core.opacity(0).color, location: 1),
                ]),
                startPoint: from, endPoint: to)
            let breath = reduceMotion ? 0 : 0.5 - 0.5 * cos(2 * .pi * time / 2.2)
            let flow = reduceMotion ? 0 : (time / 1.1).truncatingRemainder(dividingBy: 1) * 26.1
            func layer(blur: CGFloat, opacity: Double, _ draw: (inout GraphicsContext) -> Void) {
                context.drawLayer { inner in
                    inner.addFilter(.blur(radius: blur))
                    inner.opacity = opacity
                    draw(&inner)
                }
            }
            layer(blur: 7, opacity: 0.34 + 0.16 * breath) { inner in
                inner.stroke(line, with: body, style: StrokeStyle(lineWidth: 24 + 6 * breath, lineCap: .round))
            }
            layer(blur: 2.6, opacity: 0.55 + 0.17 * breath) { inner in
                inner.stroke(line, with: body, style: StrokeStyle(lineWidth: 9 + 2 * breath, lineCap: .round))
            }
            layer(blur: 0.9, opacity: 0.8) { inner in
                inner.stroke(line, with: body, style: StrokeStyle(lineWidth: 2.6, lineCap: .round))
            }
            layer(blur: 2.6, opacity: 0.95) { inner in
                inner.stroke(
                    line, with: pulses,
                    style: StrokeStyle(lineWidth: 8, lineCap: .round, dash: [0.1, 26], dashPhase: -flow))
            }
            layer(blur: 7, opacity: 0.4) { inner in
                inner.fill(
                    Path(ellipseIn: CGRect(x: from.x - 13, y: from.y - 13, width: 26, height: 26)),
                    with: .color(beam.color))
            }
        }
        .allowsHitTesting(false)
    }
}

// MARK: - The list

/// The found tools, one check each, and the add row; under Customize, the
/// repos found in each watched tool's traces with a rule and their past
/// traces (Ron, 2026-10-21).
struct ToolChecks: View {
    let copy: FirstRunCopy
    @ObservedObject var runner: FirstRunRunner
    @ObservedObject var adding: AddToolModel
    @ObservedObject var repos: CustomizeRepos
    let rows: [SourceCandidate]
    let showsRepos: Bool
    let open: Bool
    let shown: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(spacing: GlassTokens.Space.s4) {
            // One scroll view open or closed, so opening Customize grows the
            // same card rather than fading one list into another. Closed,
            // it is sized to its rows and does not scroll; while the repos
            // are held as it closes, it stays bounded by the window.
            ScrollView { list }
                .scrollDisabled(!open)
                .scrollIndicators(open ? .automatic : .hidden)
                .fixedSize(horizontal: false, vertical: !open && !showsRepos)
                .background(
                    RoundedRectangle(cornerRadius: 16, style: .continuous)
                        .fill(GlassTokens.Color.firstRunCard.color))
            AddToolTile(copy: copy, runner: runner, adding: adding)
                .modifier(RowRise(shown: shown, index: rows.count, reduceMotion: reduceMotion))
        }
        .frame(width: open ? 580 : nil)
        .fixedSize(horizontal: !open, vertical: false)
        // Nothing of the card shows until the scan is over.
        .opacity(shown ? 1 : 0)
        .animation(GlassMotion.timed(reduceMotion, duration: 0.3), value: shown)
    }

    private var list: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(rows.enumerated()), id: \.element.source) { index, row in
                VStack(alignment: .leading, spacing: 0) {
                    if index > 0 { GlassHairline() }
                    toolRow(row)
                    if showsRepos, ToolsScreenLayout.isWatched(row, in: runner.state) {
                        ToolRepoList(copy: copy, runner: runner, repos: repos, kind: row.source)
                            .padding(.leading, 36)
                            .padding(.bottom, GlassTokens.Space.s5)
                            .transition(.opacity)
                    }
                }
                .modifier(RowRise(shown: shown, index: index, reduceMotion: reduceMotion))
            }
            ForEach(ToolsScreenLayout.addedRows(runner.state), id: \.path) { folder in
                GlassHairline()
                addedRow(folder)
            }
            if let trajectory = ToolsScreenLayout.trajectoryFolder(in: runner.state) {
                GlassHairline()
                folderRow(
                    name: ToolsScreenLayout.name(.trajectory, copy: copy.tools), path: trajectory.path,
                    meta: copy.tools.addedByYou
                ) {
                    Toggle(
                        FirstRunCopy.fill(
                            copy.folders.watchQuestion, ["tool": ToolsScreenLayout.name(.trajectory, copy: copy.tools)]),
                        isOn: Binding(
                            get: { true },
                            set: { if !$0 { ToolsScreenLayout.selectTrajectory(.dontUse, in: &runner.state) } }))
                        .toggleStyle(FirstRunMarkerCheckStyle())
                }
            }
            if let pending = adding.pending {
                GlassHairline()
                folderRow(name: ToolsScreenLayout.folderName(pending.path), path: pending.path) {
                    GlassPicker(
                        ToolsScreenLayout.question(path: pending.path, copy: copy.tools),
                        selection: pendingChoice,
                        options: ToolsScreenLayout.pendingOptions(pending.kinds, copy: copy.tools),
                        placeholder: copy.frame.choose
                    )
                    .fixedSize()
                }
            }
        }
        .padding(.vertical, GlassTokens.Space.s2)
        .padding(.horizontal, GlassTokens.Space.s7)
        .disabled(!FoldersScreenLayout.rowsEnabled(isCommitting: runner.isCommitting))
    }

    private func toolRow(_ row: SourceCandidate) -> some View {
        HStack(spacing: GlassTokens.Space.s6) {
            GlassToolTile(.tool(row.source.glassTool))
            VStack(alignment: .leading, spacing: 0) {
                Text(row.source.displayName)
                    .glassType(GlassTokens.TypeScale.bodyStrong)
                    .foregroundStyle(GlassColor.textPrimary)
                Text(FirstRunCopy.fill(copy.frame.sessionCount, ["count": row.sessionCount.formatted()]))
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
            }
            .frame(minWidth: 150, alignment: .leading)
            Spacer(minLength: GlassTokens.Space.s6)
            Toggle(
                ToolAnswerRowLayout.fill(copy.folders.watchQuestion, tool: row.source),
                isOn: Binding(
                    get: { ToolsScreenLayout.isWatched(row, in: runner.state) },
                    set: { ToolsScreenLayout.setWatched($0, row, in: &runner.state) }))
                .toggleStyle(FirstRunMarkerCheckStyle())
                .padding(.trailing, GlassTokens.Space.s3)
        }
        .frame(height: rows.count > 3 ? 36 : 44)
    }

    /// A folder added with the tile: its name and folder, and the same check
    /// as every tool, unticked until the person ticks it.
    private func addedRow(_ folder: AddedFolder) -> some View {
        let name = ToolsScreenLayout.folderName(folder.path)
        return VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            folderRow(name: name, path: folder.path, meta: copy.tools.addedByYou) {
                Toggle(
                    FirstRunCopy.fill(copy.folders.watchQuestion, ["tool": name]),
                    isOn: Binding(
                        get: { folder.watched == true },
                        set: { ToolsScreenLayout.selectAdded($0 ? .watch : .dontUse, path: folder.path, in: &runner.state) }))
                    .toggleStyle(FirstRunMarkerCheckStyle())
            }
            if let line = ToolsScreenLayout.conflict(folder, in: runner.state, copy: copy.tools) {
                Text(line)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    /// A folder's row: the folder tile, its name, and "Added by you" for a
    /// folder the person gave (Ron's custom tool row), its path on hover.
    private func folderRow<Trailing: View>(
        name: String, path: String, meta: String? = nil, @ViewBuilder trailing: () -> Trailing
    ) -> some View {
        HStack(spacing: GlassTokens.Space.s6) {
            GlassToolTile(.folder)
            VStack(alignment: .leading, spacing: 0) {
                Text(name)
                    .glassType(GlassTokens.TypeScale.bodyStrong)
                    .foregroundStyle(GlassColor.textPrimary)
                    .lineLimit(1)
                Text(meta ?? ToolAnswerRowLayout.homeRelative(path))
                    .glassType(meta == nil ? GlassTokens.TypeScale.mono : GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            .help(path)
            .frame(minWidth: 150, maxWidth: 280, alignment: .leading)
            Spacer(minLength: GlassTokens.Space.s6)
            trailing()
                .padding(.trailing, GlassTokens.Space.s3)
        }
        .frame(minHeight: 44)
    }

    private var pendingChoice: Binding<ToolsScreenLayout.PendingAnswer?> {
        Binding(
            get: { nil },
            set: { answer in
                guard let answer, let pending = adding.pending,
                    ToolsScreenLayout.answer(answer, path: pending.path, kinds: pending.kinds, in: &runner.state)
                else { return }
                adding.pending = nil
            }
        )
    }
}

/// A list row rising into place once the scan is over, one after another.
private struct RowRise: ViewModifier {
    let shown: Bool
    let index: Int
    let reduceMotion: Bool

    func body(content: Content) -> some View {
        content
            .opacity(shown ? 1 : 0)
            .offset(y: shown ? 0 : 18)
            .animation(GlassMotion.spatial(reduceMotion, duration: 0.6)?.delay(0.08 * Double(index + 1)), value: shown)
    }
}

/// The add row: a dashed capsule, and the accent border over a faint fill
/// while a folder is dragged over it. Click to pick a folder, or drop one on
/// it.
struct AddToolTile: View {
    let copy: FirstRunCopy
    @ObservedObject var runner: FirstRunRunner
    @ObservedObject var adding: AddToolModel
    @State private var dragging = false

    var body: some View {
        VStack(spacing: GlassTokens.Space.s3) {
            Button {
                if let path = FolderPanel.choose() { adding.describe(path, runner: runner) }
            } label: {
                HStack(spacing: GlassTokens.Space.s4) {
                    Image(systemName: "plus")
                        .glassGlyph(11, weight: .semibold)
                        .accessibilityHidden(true)
                    Text(copy.tools.addTool)
                        .glassType(GlassTokens.TypeScale.label.weight(.regular))
                }
                .foregroundStyle(GlassColor.textSecondary)
                .frame(maxWidth: .infinity)
                .frame(height: 36)
                .padding(.horizontal, GlassTokens.Space.s8)
                .background(
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .fill(dragging ? GlassTokens.Color.tintNeutral.color : .clear))
                .overlay(
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .strokeBorder(
                            dragging ? GlassTokens.Color.purpleText.color : GlassTokens.Color.firstRunRule.color,
                            style: StrokeStyle(lineWidth: 1, dash: [4, 3])))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help(copy.tools.addToolCaption)
            // A refused folder: the failed request's red line.
            if adding.refused {
                GlassAlert(ToolsScreenLayout.refusal(copy.tools))
            }
        }
        .disabled(!FoldersScreenLayout.rowsEnabled(isCommitting: runner.isCommitting))
        .onDrop(of: [UTType.fileURL], isTargeted: $dragging) { providers in
            guard let provider = providers.first else { return false }
            _ = provider.loadObject(ofClass: URL.self) { url, _ in
                guard let url else { return }
                let isFolder = (try? url.resourceValues(forKeys: [.isDirectoryKey]))?.isDirectory == true
                let path = url.path
                Task { @MainActor in
                    guard AddToolModel.accepting(runner) else { return }
                    if isFolder { adding.describe(path, runner: runner) } else { adding.refused = true }
                }
            }
            return true
        }
    }
}

// MARK: - Customize

/// What Customize reads: the repos in the watched tools' traces and each
/// repo's past traces, through the running daemon. Opening it first starts
/// the daemon with the tools ticked so far (`CommitPoint.declareRoots`),
/// since only the daemon can name the repos.
@MainActor
final class CustomizeRepos: ObservableObject {
    @Published private(set) var projects: [ProjectRow]?
    @Published private(set) var loadFailed = false
    @Published private(set) var loading = false
    @Published private(set) var sessions: [String: [PastSession]] = [:]
    @Published private(set) var notListed: [String: Int] = [:]
    @Published private(set) var refused: Set<String> = []
    @Published var open: Set<String> = []
    /// The watched tools the last read was for.
    private var readFor: [SourceKind]?

    var phase: RulesLoadPhase {
        RulesScreenLayout.phase(projects: projects, loadFailed: loadFailed, loading: loading)
    }

    /// Read again only when the watched tools changed since the last read,
    /// it failed, or it found no repos: a daemon Customize has only just
    /// started may not have read the traces yet.
    func load(runner: FirstRunRunner, source: any FirstRunRulesSource, force: Bool = false) async {
        let watched = ToolsScreenLayout.watchedKinds(runner.state)
        guard !loading, force || readFor != watched || projects?.isEmpty != false else { return }
        loading = true
        defer { loading = false }
        await runner.commit(.declareRoots)
        guard runner.failure == nil, let all = await source.rulesProjects() else {
            loadFailed = true
            return
        }
        let offered = RulesScreenLayout.offered(all, state: runner.state)
        loadFailed = false
        readFor = watched
        if open.isEmpty, let first = offered.first { open.insert(first.projectId) }
        projects = offered
        for project in offered where sessions[project.projectId] == nil {
            guard let list = await source.pastSessions(projectID: project.projectId) else {
                refused.insert(project.projectId)
                continue
            }
            sessions[project.projectId] = RulesScreenLayout.offered(list.sessions, state: runner.state)
            notListed[project.projectId] = list.notListed
        }
    }
}

/// One watched tool's repos under Customize: a rule each, and a chevron
/// that opens its past traces.
private struct ToolRepoList: View {
    let copy: FirstRunCopy
    @ObservedObject var runner: FirstRunRunner
    @ObservedObject var repos: CustomizeRepos
    let kind: SourceKind
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var armingCandidate: ProjectRow?

    var body: some View {
        LazyVStack(alignment: .leading, spacing: 0) {
            Text(FirstRunCopy.fill(copy.rules.reposFound, ["tools": kind.displayName]))
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textTertiary)
                .padding(.bottom, GlassTokens.Space.s2)
            // Watching only queues the picked past traces on this Mac.
            if let note = RulesScreenLayout.pastSessionsNote(runner.state, copy: copy.rules) {
                Text(note)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            switch repos.phase {
            case .loaded(let projects):
                let mine = ToolsScreenLayout.repos(for: kind, in: projects)
                if mine.isEmpty {
                    Text(copy.rules.empty)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                } else {
                    ForEach(mine) { project in
                        repoRow(project)
                    }
                }
            case .failed:
                HStack(spacing: GlassTokens.Space.s4) {
                    Text(copy.rules.unavailable)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                    Button(RulesScreenLayout.retryTitle(copy)) {
                        Task { await repos.load(runner: runner, source: sourceModel, force: true) }
                    }
                    .buttonStyle(GlassButtonStyle(.secondary))
                }
            case .loading:
                HStack(spacing: GlassTokens.Space.s4) {
                    GlassSpinner()
                    Text(copy.rules.loading)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                }
            }
        }
        .glassModal(isPresented: Binding(
            get: { armingCandidate.flatMap(armingCopy) != nil },
            set: { if !$0 { armingCandidate = nil } })
        ) {
            // Never presented without the core's words: arming is never
            // confirmed against a sentence this shell wrote.
            if let project = armingCandidate, let words = armingCopy(project) {
                GlassConfirmation(
                    title: words.question, message: words.body,
                    actions: [
                        .cancel(words.decline) { armingCandidate = nil },
                        GlassModalAction(words.confirm, isDefault: true) {
                            _ = RulesScreenLayout.confirmArming(&runner.state, project: project)
                            armingCandidate = nil
                        },
                    ],
                    onCancel: { armingCandidate = nil })
            }
        }
    }

    @EnvironmentObject private var model: AppModel
    private var sourceModel: any FirstRunRulesSource { model }

    private func armingCopy(_ project: ProjectRow) -> ProjectArmingCopy? {
        ProjectArmingCopy.decode(
            fromJSON: TCCoreCopy.armingOfferCopyJSON(project: project.displayLabel, count: 0))
    }

    @ViewBuilder private func repoRow(_ project: ProjectRow) -> some View {
        #if DEBUG
        let _ = OtherCollectionRowBodies.record("repos", project.id)
        #endif
        let id = project.projectId
        let name = project.displayLabel
        let rows = repos.sessions[id] ?? []
        let never = RulesScreenLayout.rule(runner.state, for: project) == .ignore
        let isOpen = repos.open.contains(id)
        VStack(alignment: .leading, spacing: 0) {
            GlassHairline()
            HStack(spacing: GlassTokens.Space.s5) {
                Button {
                    if isOpen { repos.open.remove(id) } else { repos.open.insert(id) }
                } label: {
                    Image(systemName: "chevron.right")
                        .glassGlyph(9, weight: .semibold)
                        .rotationEffect(.degrees(isOpen ? 90 : 0))
                        .animation(GlassMotion.standard(reduceMotion), value: isOpen)
                        .frame(width: 20, height: 20)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .foregroundStyle(GlassColor.textSecondary)
                .accessibilityLabel(name)
                .accessibilityAddTraits(isOpen ? .isSelected : [])
                GlassToolTile(.folderInitial(name))
                VStack(alignment: .leading, spacing: 0) {
                    Text(name)
                        .glassType(GlassTokens.TypeScale.label.weight(.semibold))
                        .foregroundStyle(GlassColor.textPrimary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                    Text(summary(project, rows: rows, never: never))
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                }
                Spacer(minLength: GlassTokens.Space.s4)
                rulePicker(project)
            }
            .frame(height: 40)
            // Automatic picked before sign-in is armed at Start, as Uses'
            // Automatic is, and says so in Uses' words.
            if RulesScreenLayout.armsAfterSignIn(runner.state, project: project) {
                Text(copy.uses.automaticAfterSignIn)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textTertiary)
                    .padding(.leading, 62)
                    .padding(.bottom, GlassTokens.Space.s3)
            }
            if isOpen {
                pastTraces(project, rows: rows, never: never)
                    .padding(.leading, 30)
                    .padding(.bottom, GlassTokens.Space.s4)
                    .transition(.opacity)
            }
        }
    }

    private func summary(_ project: ProjectRow, rows: [PastSession], never: Bool) -> String {
        if never {
            let count = RulesScreenLayout.watchedSessionCount(project, state: runner.state)
            return FirstRunCopy.fill(copy.rules.neverCount, ["count": count.formatted()])
        }
        let chosen = runner.state.pastSelections[project.projectId] ?? []
        return FirstRunCopy.fill(
            copy.rules.folderSelected,
            ["selected": rows.filter { chosen.contains($0.id) }.count.formatted(), "total": rows.count.formatted()])
    }

    /// Ask, Automatic and Never side by side, as the canvas draws them (Ron,
    /// 2026-10-22: no status dot). Automatic can be picked before sign-in:
    /// it is confirmed now and armed at Start.
    @ViewBuilder private func rulePicker(_ project: ProjectRow) -> some View {
        if let words = ProjectModeWords.table {
            FirstRunSegmented(
                label: FirstRunCopy.fill(copy.rules.ruleFor, ["folder": project.displayLabel]),
                selection: Binding(
                    get: { RulesScreenLayout.rule(runner.state, for: project) },
                    set: { wanted in
                        // Arming is a grant, so it is never silent.
                        if RulesScreenLayout.pick(&runner.state, project: project, wanted: wanted) == .needsConfirmation {
                            armingCandidate = project
                        }
                    }),
                options: RulesScreenLayout.offeredModes(project, state: runner.state).compactMap { mode in
                    words.label(for: mode).map { .init(title: $0, value: mode) }
                })
        }
    }

    @ViewBuilder private func pastTraces(_ project: ProjectRow, rows: [PastSession], never: Bool) -> some View {
        let id = project.projectId
        if repos.refused.contains(id) {
            Text(copy.rules.sessionsUnavailable)
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textTertiary)
        } else {
            let tickable = rows.filter(RulesScreenLayout.isTickable)
            // The canvas's rows (Ron, 2026-10-22): the switch first, then
            // the words; a trace's date, then the rest in the quieter ink.
            LazyVStack(alignment: .leading, spacing: 0) {
                Toggle(isOn: Binding(
                    get: { RulesScreenLayout.groupState(runner.state, projectID: id, sessions: rows) == .all },
                    set: { RulesScreenLayout.includeEvery(&runner.state, projectID: id, sessions: rows, on: $0) })
                ) {
                    Text(FirstRunCopy.fill(copy.rules.includeEvery, ["folder": project.displayLabel]))
                        .foregroundStyle(GlassColor.textSecondary)
                }
                .toggleStyle(FirstRunSwitchStyle())
                .disabled(never || tickable.isEmpty)
                .frame(height: 28)
                ForEach(rows) { session in
                    #if DEBUG
                    let _ = OtherCollectionRowBodies.record("pastSessions", session.id)
                    #endif
                    let parts = RulesScreenLayout.labelParts(session, copy: copy.rules)
                    Toggle(isOn: Binding(
                        get: { runner.state.pastSelections[id]?.contains(session.id) == true },
                        set: { RulesScreenLayout.setTicked(&runner.state, projectID: id, session: session, on: $0) })
                    ) {
                        HStack(spacing: 10) {
                            Text(parts.first ?? "")
                                .foregroundStyle(GlassColor.textPrimary)
                                .monospacedDigit()
                                .frame(minWidth: 84, alignment: .leading)
                            Text(parts.dropFirst().joined(separator: " · "))
                                .foregroundStyle(GlassColor.textSecondary)
                                .monospacedDigit()
                        }
                        .accessibilityElement(children: .combine)
                    }
                    .toggleStyle(FirstRunSwitchStyle())
                    .disabled(never || !RulesScreenLayout.isTickable(session))
                    .frame(height: 28)
                }
                if let note = RulesScreenLayout.notListedNote(repos.notListed[id] ?? 0, copy: copy.rules) {
                    Text(note)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textTertiary)
                }
            }
            .glassType(GlassTokens.TypeScale.label.weight(.regular))
        }
    }
}

// MARK: - The screen

/// Tools: the ring narrowed to the tools found on this Mac, one check per
/// tool, Customize for per-repo rules, and Continue, which starts the
/// daemon watching what is ticked (`CommitPoint.leaveTools`).
struct ToolsScreen: View {
    let copy: FirstRunCopy
    @ObservedObject var runner: FirstRunRunner
    @ObservedObject var memory: FirstRunStageMemory
    let source: any FirstRunRulesSource

    @State private var discovery: DiscoveredRows = .loading
    @State private var onboarding = TCOnboardingCopy.load()
    @StateObject private var adding = AddToolModel()
    @StateObject private var repos = CustomizeRepos()
    @State private var customizing = false
    /// The repos stay in the card while it shrinks back, then leave.
    @State private var holdingRepos = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        let rows = discovery.rows.map(FirstRunStageLayout.ringRows) ?? []
        let listed = discovery.rows.map { ToolsScreenLayout.listed($0, state: runner.state) } ?? []
        let scanned = memory.toolsScanned
        FirstRunFrame(
            copy: copy,
            state: $runner.state,
            isCommitting: runner.isCommitting,
            notice: FoldersScreenLayout.notice(for: runner.failure, copy: copy, onboarding: onboarding),
            footer: FirstRunFooter(
                title: copy.frame.continueButton,
                isEnabled: ToolsScreenLayout.canContinue(
                    discovered: discovery.rows, state: runner.state, pending: adding.pending != nil,
                    isCommitting: runner.isCommitting, scanned: scanned),
                busy: runner.isCommitting,
                link: FirstRunFooter.Link(
                    title: copy.frame.customSetupInstead,
                    isEnabled: ToolsScreenLayout.customizeEnabled(
                        state: runner.state, open: customizing, scanned: scanned, isCommitting: runner.isCommitting),
                    isOpen: customizing,
                    action: toggleCustomize),
                action: { Task { await runner.commit(.leaveTools) } }),
            open: customizing
        ) {
            ToolsRing(rows: rows, state: runner.state, scanStart: memory.scanStartedAt)
        } title: {
            FirstRunTitle(light: copy.tools.titleLight, bold: copy.tools.titleBold)
        } content: {
            switch discovery {
            case .found:
                ZStack {
                    if !scanned {
                        HStack(spacing: GlassTokens.Space.s4) {
                            GlassSpinner()
                            Text(copy.folders.loading)
                                .glassType(GlassTokens.TypeScale.label.weight(.regular))
                                .foregroundStyle(GlassColor.textSecondary)
                        }
                        .transition(.opacity)
                    }
                    ToolChecks(
                        copy: copy, runner: runner, adding: adding, repos: repos, rows: listed,
                        showsRepos: customizing || holdingRepos, open: customizing, shown: scanned)
                        .allowsHitTesting(scanned)
                }
            case .failed:
                HStack(spacing: GlassTokens.Space.s4) {
                    Text(discovery.failureLine(copy.folders) ?? "")
                        .glassType(GlassTokens.TypeScale.body)
                        .foregroundStyle(GlassColor.textSecondary)
                    Button(copy.folders.retry, action: refreshDiscovery)
                        .buttonStyle(GlassButtonStyle(.secondary))
                }
            case .loading:
                HStack(spacing: GlassTokens.Space.s4) {
                    GlassSpinner()
                    Text(copy.folders.loading)
                        .glassType(GlassTokens.TypeScale.label.weight(.regular))
                        .foregroundStyle(GlassColor.textSecondary)
                }
            }
        }
        .task {
            if discovery == .loading { refreshDiscovery() }
        }
        // A tool installed while the app was in the background is offered
        // when the person comes back.
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            refreshDiscovery()
        }
        .onChange(of: ToolsScreenLayout.watchedKinds(runner.state)) { _, watched in
            // Nothing watched closes Customize: it has nothing to show.
            if watched.isEmpty, customizing { customizing = false }
            if customizing { Task { await repos.load(runner: runner, source: source) } }
        }
    }

    /// Discovery's rows, and what they say is not on this Mac: such a tool
    /// leaves the ring and is not asked (`FirstRunState.recordDiscovery`).
    /// Not recorded while a commit holds the state it started from. The
    /// scan starts the first time rows arrive, once per first run.
    private func refreshDiscovery() {
        discovery = FoldersScreenLayout.discovered(TCDiscovery.sourcesJSON(), keeping: discovery)
        if let rows = discovery.rows, !runner.isCommitting { runner.state.recordDiscovery(rows) }
        if let rows = discovery.rows {
            memory.discovered = rows
            memory.startScan(reduceMotion: reduceMotion)
        }
    }

    private func toggleCustomize() {
        if customizing {
            customizing = false
            holdingRepos = true
            Task {
                try? await Task.sleep(for: .milliseconds(800))
                withAnimation(GlassMotion.spatial(reduceMotion, duration: 0.5)) { holdingRepos = false }
            }
            return
        }
        customizing = true
        holdingRepos = false
        Task { await repos.load(runner: runner, source: source) }
    }
}
