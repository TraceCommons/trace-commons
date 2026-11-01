import SwiftUI
import TCDesign
import TCShellCore

/// The Economizer tab (2026-11-01): the `near-ai economizer` switch, and
/// what it learned in each folder a managed coding session ran in, one
/// card per folder and tool: the built-in tools it leaves out (with "Give
/// back"), the commands it tells the agent, and how many more it is still
/// confirming. Every figure is the daemon's `economizer_status`; every
/// word is the core's analytics copy. No estimate of what was saved.
@MainActor
@Observable
final class InsightsEconomizerModel {
    struct Calls: Sendable {
        /// Whether the daemon serves the economizer at all (its `hello`
        /// methods), without reading what it learned.
        let served: @Sendable () async throws -> Bool
        let status: @Sendable () async throws -> DaemonData.EconomizerStatus
        let setEnabled: @Sendable (Bool) async throws -> DaemonData.EconomizerSwitch
        let restore: @Sendable (String, String) async throws -> DaemonData.EconomizerRestored
    }

    private let calls: Calls?
    private(set) var answer: DaemonData.EconomizerStatus?
    private(set) var failed = false
    /// The read after a switch or Give back failed, so `answer` is from
    /// before it. The page keeps it and says it may be out of date under
    /// the switch, until a read succeeds.
    private(set) var stale = false
    /// The daemon predates the economizer (`unknown_method`): the tab is
    /// hidden, since a tab that cannot be used is not drawn greyed. A
    /// later probe or read that succeeds clears it, so a daemon upgraded
    /// while the app runs brings the tab back.
    private(set) var notServed = false
    private(set) var busy = false
    /// Folder keys with a "Give back" in flight.
    private(set) var restoring: Set<String> = []
    /// The request-failed words under the switch after a refused switch
    /// write, until the next confirmed one.
    private(set) var switchRefusal: String?
    /// The request-failed words on a folder's card after its Give back
    /// failed, by folder key.
    private(set) var folderRefusals: [String: String] = [:]
    /// Reads are numbered as they start; a reply older than one already
    /// shown is dropped. A newer read that is cancelled drops nothing.
    private var started = 0
    private var shown = 0

    init(calls: Calls?) { self.calls = calls }

    /// Whether this daemon can answer at all.
    var available: Bool { calls != nil && !notServed }

    /// Asks the daemon whether it serves the economizer, so the tab is
    /// shown or hidden before anyone opens it, without reading the store.
    func probe() async {
        guard let calls else { return }
        do {
            notServed = !(try await calls.served())
        } catch {
            // An unreachable daemon says nothing about what it serves.
        }
    }

    /// Reads what the economizer learned. A read the tab makes when it
    /// opens that fails says the details are unavailable; the read after
    /// a write (`afterWrite`) that fails keeps the page and marks it
    /// stale instead, so a refusal or a confirmed switch is not wiped.
    func load(afterWrite: Bool = false) async {
        guard let calls else { failed = true; return }
        started += 1
        let mine = started
        do {
            let reply = try await calls.status()
            guard mine > shown else { return }
            shown = mine
            answer = reply
            failed = false
            stale = false
            notServed = false
        } catch is CancellationError {
            // Leaving the tab is not a failure.
        } catch {
            guard mine > shown else { return }
            shown = mine
            if (error as? DaemonDataError)?.isNotServed == true { notServed = true }
            if afterWrite, answer != nil {
                stale = true
            } else {
                answer = nil
                stale = false
                failed = true
            }
        }
    }

    /// Turns the economizer on or off; the switch then shows what the
    /// daemon confirmed, never what was asked.
    func setEnabled(_ on: Bool) async {
        guard let calls, !busy else { return }
        busy = true
        defer { busy = false }
        do {
            let reply = try await calls.setEnabled(on)
            switchRefusal = reply.enabled == on ? nil : MonitorWords.table?.requestFailed
            // Draw what the daemon confirmed, even if the read after this
            // fails and the rest of the page stays as it was.
            if let shown = answer {
                answer = DaemonData.EconomizerStatus(
                    enabled: reply.enabled, folders: shown.folders, foldersTruncated: shown.foldersTruncated)
            }
        } catch {
            switchRefusal = MonitorWords.table?.requestFailed
        }
        await load(afterWrite: true)
    }

    /// Gives back every tool left out in one folder, then reads again.
    func restore(_ folder: DaemonData.EconomizerFolder) async {
        guard let calls, !restoring.contains(folder.key) else { return }
        restoring.insert(folder.key)
        defer { restoring.remove(folder.key) }
        do {
            _ = try await calls.restore(folder.harness, folder.key)
            folderRefusals[folder.key] = nil
        } catch {
            folderRefusals[folder.key] = MonitorWords.table?.requestFailed
        }
        await load(afterWrite: true)
    }
}

/// The tab's words, kept pure for tests.
enum InsightsEconomizerWords {
    struct Card: Equatable, Identifiable {
        let id: String
        let title: String
        let sessions: String
        /// The left-out tools, comma separated; the "None yet" words when
        /// there are none.
        let leftOut: String
        let canGiveBack: Bool
        /// `nil` when nothing was given back.
        let givenBack: String?
        let told: [String]
        /// `nil` when nothing is being confirmed.
        let forming: String?
    }

    private static func text(_ key: String, _ copy: [String: String]) -> String { copy[key] ?? "" }

    static func harness(_ harness: String, copy: [String: String]) -> String {
        let words = text("analytics_econ_harness_" + harness, copy)
        return words.isEmpty ? harness : words
    }

    static func card(_ folder: DaemonData.EconomizerFolder, copy: [String: String]) -> Card {
        func fill(_ key: String, _ values: [String: String]) -> String {
            InsightsOverviewWords.fill(text(key, copy), values)
        }
        let sessionsKey = folder.sessions == 1 ? "analytics_econ_sessions_one" : "analytics_econ_sessions"
        return Card(
            id: folder.key,
            title: fill("analytics_econ_folder_tool",
                        ["folder": folder.folder, "harness": harness(folder.harness, copy: copy)]),
            sessions: fill(sessionsKey, ["n": String(folder.sessions)]),
            leftOut: folder.leftOut.isEmpty ? text("analytics_econ_none", copy) : folder.leftOut.joined(separator: ", "),
            canGiveBack: !folder.leftOut.isEmpty,
            givenBack: folder.givenBack.isEmpty ? nil : folder.givenBack.joined(separator: ", "),
            told: folder.told.map {
                fill("analytics_econ_told_line", ["first": $0.program, "second": $0.prefix + " " + $0.program])
            },
            forming: folder.forming == 0 ? nil : fill("analytics_econ_forming", ["n": String(folder.forming)]))
    }

    static func truncated(_ count: UInt32, copy: [String: String]) -> String? {
        count == 0 ? nil : InsightsOverviewWords.fill(text("analytics_econ_truncated", copy), ["n": String(count)])
    }
}

struct InsightsEconomizerTab: View {
    let model: InsightsEconomizerModel
    let copy: [String: String]

    private func text(_ key: String) -> String { copy[key] ?? "" }

    var body: some View {
        InsightsTabPage(getStarted: nil) {
            if let answer = model.answer {
                switchCard(answer)
                if answer.folders.isEmpty {
                    VStack(alignment: .leading, spacing: InsightsLayout.lineGap) {
                        Text(text("analytics_econ_empty_title")).insightsHeading()
                        Text(text("analytics_econ_empty")).insightsNote()
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .insightsSection()
                } else {
                    InsightsCardFlow(minimum: 420) {
                        ForEach(answer.folders) { folder in
                            card(folder)
                        }
                    }
                    if let more = InsightsEconomizerWords.truncated(answer.foldersTruncated, copy: copy) {
                        Text(more).insightsCaption().insightsSection()
                    }
                }
            } else if model.failed {
                Text(text("analytics_econ_unavailable")).insightsNote().insightsSection()
            } else {
                GlassSpinner()
            }
        }
        .task { await model.load() }
    }

    private func switchCard(_ answer: DaemonData.EconomizerStatus) -> some View {
        VStack(alignment: .leading, spacing: InsightsLayout.lineGap) {
            HStack(alignment: .center, spacing: GlassTokens.Space.s3) {
                Toggle(text("analytics_econ_switch"), isOn: Binding(
                    get: { answer.enabled },
                    set: { on in Task { await model.setEnabled(on) } }
                ))
                .toggleStyle(GlassToggleStyle(.settings))
                .disabled(model.busy)
                InsightsInfo(text("analytics_econ_switch"), [text("analytics_econ_switch_detail")])
                Spacer(minLength: 0)
            }
            if let refusal = model.switchRefusal {
                GlassNotice(tone: .outside) { Text(refusal) }
            }
            // What is below may be from before the last change.
            if model.stale {
                Text(text("analytics_econ_stale")).insightsCaption()
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .insightsSection()
    }

    private func card(_ folder: DaemonData.EconomizerFolder) -> some View {
        let card = InsightsEconomizerWords.card(folder, copy: copy)
        return VStack(alignment: .leading, spacing: InsightsLayout.headingGap) {
            HStack(alignment: .firstTextBaseline) {
                Text(card.title).insightsHeading().lineLimit(1).truncationMode(.middle)
                Spacer(minLength: GlassTokens.Space.s3)
                Text(card.sessions).insightsCaption()
            }
            VStack(alignment: .leading, spacing: InsightsLayout.lineGap) {
                HStack(spacing: GlassTokens.Space.s2) {
                    Text(text("analytics_econ_left_out")).insightsEyebrow()
                    InsightsInfo(text("analytics_econ_left_out"), [text("analytics_econ_left_out_detail")])
                }
                Text(card.leftOut).insightsMono().fixedSize(horizontal: false, vertical: true)
                if card.canGiveBack {
                    HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s3) {
                        Button(text("analytics_econ_give_back")) { Task { await model.restore(folder) } }
                            .buttonStyle(GlassButtonStyle(.glass, small: true))
                            .disabled(model.restoring.contains(folder.key))
                        // An irreversible consequence stays on the page.
                        Text(text("analytics_econ_give_back_detail")).insightsCaption()
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                if let refusal = model.folderRefusals[folder.key] {
                    GlassNotice(tone: .outside) { Text(refusal) }
                }
                if let given = card.givenBack {
                    Text(text("analytics_econ_given_back")).insightsEyebrow()
                    Text(given).insightsMono().fixedSize(horizontal: false, vertical: true)
                }
            }
            VStack(alignment: .leading, spacing: InsightsLayout.lineGap) {
                Text(text("analytics_econ_told")).insightsEyebrow()
                if card.told.isEmpty {
                    Text(text("analytics_econ_none")).insightsNote()
                } else {
                    ForEach(card.told, id: \.self) { line in
                        Text(line).insightsMono().fixedSize(horizontal: false, vertical: true)
                    }
                }
                if let forming = card.forming { Text(forming).insightsCaption() }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .insightsSection()
    }
}
