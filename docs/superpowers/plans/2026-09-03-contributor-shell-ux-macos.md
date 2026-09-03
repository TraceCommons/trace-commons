# Contributor Shell UX -- macOS Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the macOS contributor app's queue and history into folder-first
screens, and make the scrubber say what it removed.

**Architecture:** Every behavior worth guarding goes into a plain Swift type
in `TCShellCore` (or `TCBridge` for ABI calls), with an XCTest beside it; the
SwiftUI views stay thin and consume those types. This is not a stylistic
preference -- see Global Constraints.

**Tech Stack:** Swift 6, SwiftUI, XCTest, `CTraceCommons` (the C ABI).

**Spec:** [`docs/superpowers/specs/2026-09-03-contributor-shell-queue-ux-design.md`](../specs/2026-09-03-contributor-shell-queue-ux-design.md)

**Depends on:** [`2026-09-03-contributor-shell-ux-foundation.md`](2026-09-03-contributor-shell-ux-foundation.md)
(plan 1). Every daemon field and ABI call this plan decodes is produced
there. **Do not start this plan until plan 1 is merged** -- Task 1 decodes
fields that do not exist before it.

## Global Constraints

- **`swift test` is the only thing CI runs against this shell**, in the
  `macOS app tests` job on `macos-26`. It runs XCTest targets. It does
  **not** render SwiftUI views, and there is no snapshot testing.
  Consequence, and the architecture of this whole plan: **a behavior that
  lives in a `View` body is a behavior nothing tests.** Put parsing,
  navigation state, tone selection, and copy in `TCShellCore`; leave layout
  in the view.
- **The Swift package links the FFI dylib**, so `cargo build -p
  trace-commons-contributor-ffi` must run before `swift test`.
- **Design tokens only.** Use `TC.Space.*`, `TC.Font_.*`, `TC.ink*`, and the
  `tcCard` / `tcPrimaryAction` modifiers from `DesignSystem.swift`. Do not
  introduce raw numbers or system colors; the GTK and Windows shells mirror
  these tokens and drift shows up as three apps that look different.
- **No emojis** in commits, code, or copy.
- **`Look inside` keeps `.tcPrimaryAction()`.** Task 6 adds a second route to
  it; the button is not demoted, moved, or restyled. The reasoning is in
  `QueueView.swift`'s `actions` doc comment -- read it before touching that
  block.
- **The redacted body is the C ABI's one content exemption.** It must never
  reach a log line, a notification, or a history record. Task 7 parses it;
  the parse results carry labels and offsets, never matched text.

---

### Task 1: Decode the new daemon fields

**Files:**
- Modify: `macos/Sources/TraceCommonsApp/Models.swift:39-90` (`QueueEntry`), `:256-286` (`PreviewSummary`), `:337-364` (`HistoryRecord`)
- Modify: `macos/Sources/TCShellCore/ProjectRow.swift:49` (field), `:78-91` (`CodingKeys` and `init(from:)`), `:64-71` (memberwise init)
- Test: `macos/Tests/TraceCommonsAppTests/DaemonFieldDecodingTests.swift` (create)
- Test: `macos/Tests/TCShellCoreTests/ProjectRowTests.swift` (extend the existing file)

**Interfaces:**
- Consumes: plan 1 Tasks 5, 6, 7 (the wire fields).
- Produces: `QueueEntry.projectPath: String`, `QueueEntry.sessionPath: String?`,
  `PreviewSummary.redactionsDistinct: [String: Int]`,
  `HistoryRecord.projectID: String`, `ProjectRow.projectPath: String`.

`ProjectRow` is the one Task 11 needs: history records carry an opaque
`project_id` and no path, so the folder path in History is resolved by
matching that id against the live `list_projects` rows. It is also what lets
Settings show `~/work/api` and `~/client/api` instead of `api` and
`api (3f9c)`.

- [ ] **Step 1: Write the failing tests**

Create `macos/Tests/TraceCommonsAppTests/DaemonFieldDecodingTests.swift`:

```swift
import XCTest

@testable import TraceCommonsApp

/// The daemon gained a project path, a session path, distinct redaction
/// counts, and a project id on history records. Every one of them must be
/// optional in practice: this app is shipped separately from the daemon and
/// routinely runs against an older one.
final class DaemonFieldDecodingTests: XCTestCase {
    private func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return try decoder.decode(type, from: Data(json.utf8))
    }

    func testQueueEntryDecodesProjectAndSessionPaths() throws {
        let entry = try decode(QueueEntry.self, """
        {"entry_id":"e1","session_hash":"sha256:a","source":"claude_code",
         "project_id":"proj_abc","project_label":"repo",
         "project_path":"~/code/repo","session_path":"~/code/repo/crates/inner",
         "size_bytes":12,"discovered_at":"2026-09-03T00:00:00Z",
         "state":"pending","attempts":0}
        """)
        XCTAssertEqual(entry.projectPath, "~/code/repo")
        XCTAssertEqual(entry.sessionPath, "~/code/repo/crates/inner")
    }

    func testQueueEntryFromAnOlderDaemonHasNoPaths() throws {
        let entry = try decode(QueueEntry.self, """
        {"entry_id":"e1","session_hash":"sha256:a","source":"claude_code",
         "project_id":"proj_abc","project_label":"repo",
         "size_bytes":12,"discovered_at":"2026-09-03T00:00:00Z",
         "state":"pending","attempts":0}
        """)
        XCTAssertEqual(entry.projectPath, "")
        XCTAssertNil(entry.sessionPath)
    }

    func testPreviewSummaryDecodesDistinctCounts() throws {
        let summary = try decode(PreviewSummary.self, """
        {"would_send_bytes":10,"raw_session_bytes":20,"event_count":3,
         "opening_prompt":"hi","redactions":{"local_path":185},
         "redactions_distinct":{"local_path":12},
         "pii_labels_present":[],"consent_scopes":[],"residual_risk":"low"}
        """)
        XCTAssertEqual(summary.redactions["local_path"], 185)
        XCTAssertEqual(summary.redactionsDistinct["local_path"], 12)
    }

    func testPreviewSummaryFromAnOlderDaemonHasNoDistinctCounts() throws {
        let summary = try decode(PreviewSummary.self, """
        {"would_send_bytes":10,"raw_session_bytes":20,"event_count":3,
         "opening_prompt":"hi","redactions":{"local_path":185},
         "pii_labels_present":[],"consent_scopes":[],"residual_risk":"low"}
        """)
        XCTAssertTrue(summary.redactionsDistinct.isEmpty)
    }

    func testHistoryRecordDecodesProjectID() throws {
        let record = try decode(HistoryRecord.self, """
        {"submission_id":"11111111-1111-1111-1111-111111111111",
         "submitted_at":"2026-09-03T00:00:00Z","project_id":"proj_abc",
         "project_label":"repo","source":"claude_code","status":"accepted",
         "consent_scopes":[],"credit_points_pending":0,"explanations":[]}
        """)
        XCTAssertEqual(record.projectID, "proj_abc")
    }

    func testHistoryRecordFromBeforeTheUpgradeHasNoProjectID() throws {
        let record = try decode(HistoryRecord.self, """
        {"submission_id":"11111111-1111-1111-1111-111111111111",
         "submitted_at":"2026-09-03T00:00:00Z",
         "project_label":"repo","source":"claude_code","status":"accepted",
         "consent_scopes":[],"credit_points_pending":0,"explanations":[]}
        """)
        XCTAssertEqual(record.projectID, "")
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo build -p trace-commons-contributor-ffi
cd macos && swift test --filter DaemonFieldDecodingTests
```

Expected: compile error, `value of type 'QueueEntry' has no member 'projectPath'`.

- [ ] **Step 3: Add the fields**

In `Models.swift`, in `QueueEntry` after `let projectLabel: String`:

```swift
    /// The project's folder, `~`-abbreviated, for display only.
    ///
    /// The daemon relaxed its path rule in exactly one place to send this
    /// (see `ipc::display_path`), because `projectLabel` can keep two
    /// projects distinct but can never make them identifiable, and the
    /// queue's folder rows are where that difference is decided. Never
    /// logged, never in a notification, never in a history record.
    ///
    /// Empty against a daemon predating the field. A folder row with no
    /// path renders its label alone rather than an empty line.
    let projectPath: String
    /// Where this session actually ran, when that is not the project root.
    ///
    /// `nil` both when the daemon predates the field and when the session
    /// ran at the root -- the daemon sends null in the second case rather
    /// than repeating `projectPath`, so a row renders this line only when
    /// it says something.
    let sessionPath: String?
```

Add to `QueueEntry.CodingKeys`:

```swift
        case projectPath = "project_path"
        case sessionPath = "session_path"
```

`projectPath` is non-optional but must tolerate absence, so `QueueEntry`
needs an explicit `init(from:)` -- unless the type already has one, in which
case extend it. Add:

```swift
    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        entryID = try c.decode(String.self, forKey: .entryID)
        sessionHash = try c.decode(String.self, forKey: .sessionHash)
        source = try c.decode(String.self, forKey: .source)
        declaredSource = try c.decodeIfPresent(String.self, forKey: .declaredSource)
        projectID = try c.decode(String.self, forKey: .projectID)
        projectLabel = try c.decode(String.self, forKey: .projectLabel)
        projectPath = try c.decodeIfPresent(String.self, forKey: .projectPath) ?? ""
        sessionPath = try c.decodeIfPresent(String.self, forKey: .sessionPath)
        sizeBytes = try c.decode(Int.self, forKey: .sizeBytes)
        discoveredAt = try c.decode(Date.self, forKey: .discoveredAt)
        state = try c.decode(QueueState.self, forKey: .state)
        reasonLabel = try c.decodeIfPresent(String.self, forKey: .reasonLabel)
        attempts = try c.decode(Int.self, forKey: .attempts)
        subagentCount = try c.decodeIfPresent(Int.self, forKey: .subagentCount)
        subagentsDropped = try c.decodeIfPresent(Int.self, forKey: .subagentsDropped)
    }
```

In `PreviewSummary`, after `let redactions: [String: Int]`:

```swift
    /// Distinct values removed per label, beside `redactions`' occurrence
    /// counts. `185 local path` is occurrences; `(12 distinct)` is how much
    /// of the session's surface was really touched, which is the figure a
    /// person estimating risk is reaching for.
    ///
    /// Empty against a daemon predating the field; `RedactionTally` renders
    /// occurrences alone in that case.
    let redactionsDistinct: [String: Int]
```

with `case redactionsDistinct = "redactions_distinct"` in its `CodingKeys`,
and the same `decodeIfPresent(...) ?? [:]` treatment via an explicit
`init(from:)`.

In `HistoryRecord`, before `let projectLabel: String`:

```swift
    /// The opaque project handle, so History can group by folder the way
    /// the queue does. Grouping on `projectLabel` instead would merge two
    /// different repositories that share a basename.
    ///
    /// Empty on records cached before the daemon carried it, and on records
    /// submitted before project keys were normalized -- those cannot be
    /// resolved to a folder and group under their label alone. Nothing
    /// retained the key they were minted from, so this is not backfillable.
    let projectID: String
```

with `case projectID = "project_id"` and the same optional-tolerant decode.

In `macos/Sources/TCShellCore/ProjectRow.swift`, after
`public let projectLabel: String`:

```swift
    /// The project's folder, `~`-abbreviated, for display only.
    ///
    /// Empty against a daemon predating the field, and for the unresolved
    /// bucket, which is not a directory. A row with no path shows its label
    /// alone.
    public let projectPath: String
```

Add `case projectPath = "project_path"` to its `CodingKeys`,
`projectPath = try c.decodeIfPresent(String.self, forKey: .projectPath) ?? ""`
to its `init(from:)`, and a `projectPath: String = ""` parameter to the
memberwise `init` so existing call sites keep compiling.

- [ ] **Step 4: Add the ProjectRow test**

Add two cases to `macos/Tests/TCShellCoreTests/ProjectRowTests.swift`,
decoding a `list_projects` row with `project_path` present and absent, and
asserting `row.projectPath` is the path and `""` respectively. Reuse
whatever decoder helper the existing cases in that file use rather than a
bare `JSONDecoder`.

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cd macos && swift test --filter DaemonFieldDecodingTests && swift test --filter ProjectRowTests
```

Expected: 6 + 2 tests pass.

- [ ] **Step 6: Commit**

```bash
git add macos/Sources/TraceCommonsApp/Models.swift \
        macos/Sources/TCShellCore/ProjectRow.swift \
        macos/Tests/TraceCommonsAppTests/DaemonFieldDecodingTests.swift \
        macos/Tests/TCShellCoreTests/ProjectRowTests.swift
git commit -m "Decode the project path, session path, distinct counts, and history project id"
```

---

### Task 2: `RedactionTally` -- the removed-by-pattern figure, testable

`QueueRow.removedSummary` is a `static func` on a SwiftUI view, so nothing
tests it. It now has to also fold in distinct counts. Move it out and test
it, rather than growing untested logic inside a view body.

**Files:**
- Create: `macos/Sources/TCShellCore/RedactionTally.swift`
- Modify: `macos/Sources/TraceCommonsApp/Views/QueueView.swift:544-551` (delete `removedSummary`, call the new type), `:452` (`redactionCount`)
- Modify: `macos/Sources/TraceCommonsApp/Models.swift:279-285` (`redactionReceipt` delegates to the new type)
- Test: `macos/Tests/TCShellCoreTests/RedactionTallyTests.swift` (create)

**Interfaces:**
- Consumes: `PreviewSummary.redactions`, `PreviewSummary.redactionsDistinct` (Task 1).
- Produces:
  - `public enum RedactionTally`
  - `public static func line(occurrences: [String: Int], distinct: [String: Int]) -> String`
  - `public static func total(_ occurrences: [String: Int]) -> Int`

- [ ] **Step 1: Write the failing tests**

Create `macos/Tests/TCShellCoreTests/RedactionTallyTests.swift`:

```swift
import XCTest

@testable import TCShellCore

/// The card's "removed by pattern" figure. It carries two different
/// numbers -- how many times a pattern fired, and how many distinct values
/// that was -- and conflating them overstates or understates the reach of
/// scrubbing depending on which one you drop.
final class RedactionTallyTests: XCTestCase {
    func testAnEmptyTallyIsNothingMatched() {
        XCTAssertEqual(RedactionTally.line(occurrences: [:], distinct: [:]), "nothing matched")
        XCTAssertEqual(RedactionTally.total([:]), 0)
    }

    func testLabelsAreHumanReadable() {
        XCTAssertEqual(
            RedactionTally.line(occurrences: ["local_path": 3], distinct: [:]),
            "3 local path"
        )
    }

    func testDistinctCountsAreShownWhenTheyDifferFromOccurrences() {
        XCTAssertEqual(
            RedactionTally.line(occurrences: ["local_path": 185], distinct: ["local_path": 12]),
            "185 local path (12 distinct)"
        )
    }

    func testDistinctIsOmittedWhenEveryOccurrenceIsItsOwnValue() {
        // "3 secret (3 distinct)" is noise: it says the same thing twice.
        XCTAssertEqual(
            RedactionTally.line(occurrences: ["secret": 3], distinct: ["secret": 3]),
            "3 secret"
        )
    }

    func testDistinctIsOmittedWhenTheDaemonDidNotReportIt() {
        XCTAssertEqual(
            RedactionTally.line(occurrences: ["secret": 3], distinct: [:]),
            "3 secret"
        )
    }

    func testBiggestCountLeadsAndTiesBreakOnLabel() {
        let line = RedactionTally.line(
            occurrences: ["secret": 3, "local_path": 185, "email": 3],
            distinct: [:]
        )
        XCTAssertEqual(line, "185 local path  ·  3 email  ·  3 secret")
    }

    func testTotalSumsOccurrencesNotDistinct() {
        XCTAssertEqual(RedactionTally.total(["a": 2, "b": 3]), 5)
    }

    /// `residual_secret_at:*` counts a secret that was DETECTED AND NOT
    /// REMOVED. It arrives in the same map as every genuine removal, and
    /// this line renders under the heading "Removed by pattern" -- so
    /// including it states the exact opposite of what happened, on the
    /// screen where someone is deciding whether to send the thing.
    func testAResidualSurvivorIsNotCountedAsRemoved() {
        XCTAssertEqual(
            RedactionTally.line(
                occurrences: ["local_path": 3, "residual_secret_at:events.correction": 1],
                distinct: [:]
            ),
            "3 local path"
        )
        XCTAssertEqual(
            RedactionTally.total(["local_path": 3, "residual_secret_at:events.correction": 1]),
            3
        )
    }

    /// A session whose ONLY count is a survivor removed nothing. Saying
    /// "nothing matched" is true, and it is what puts the card in the tone
    /// that asks someone to look.
    func testASessionWithOnlyAResidualMatchedNothing() {
        XCTAssertEqual(
            RedactionTally.line(occurrences: ["residual_secret_at:events.x": 1], distinct: [:]),
            "nothing matched"
        )
        XCTAssertEqual(RedactionTally.total(["residual_secret_at:events.x": 1]), 0)
    }

    func testADistinctCountAboveItsOccurrenceCountIsIgnored() {
        // Cannot happen from a correct daemon; if it ever does, saying
        // "3 secret (9 distinct)" would be worse than saying nothing.
        XCTAssertEqual(
            RedactionTally.line(occurrences: ["secret": 3], distinct: ["secret": 9]),
            "3 secret"
        )
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cd macos && swift test --filter RedactionTallyTests
```

Expected: `cannot find 'RedactionTally' in scope`.

- [ ] **Step 3: Write the implementation**

Create `macos/Sources/TCShellCore/RedactionTally.swift`:

```swift
import Foundation

/// The card's "removed by pattern" figure, and the one place that decides
/// how the two redaction counts are worded.
///
/// The daemon reports two maps. `redactions` counts OCCURRENCES -- how many
/// times a pattern fired. `redactions_distinct` counts VALUES -- how many
/// different strings those firings covered, because the redactor mints one
/// placeholder per distinct value and reuses it. One path referenced two
/// hundred times is two hundred occurrences and one value, and a card that
/// reports only the first overstates how much of the session was touched.
///
/// Lives here rather than on the view because it is the only part of that
/// card with a right and a wrong answer, and `swift test` cannot reach a
/// SwiftUI body.
public enum RedactionTally {
    /// What the card shows when nothing fired. `ScrubbingCaveat` supplies
    /// the sentence that says what that does and does not prove.
    public static let nothingMatched = "nothing matched"

    /// The prefix marking a secret that was DETECTED AND NOT REMOVED.
    ///
    /// `note_residual_secret_location` increments this when a secret
    /// survives redaction -- a credential inside a human correction, which
    /// is preserved by design, or a field the typed traversal never visits,
    /// which is a real gap. It rides in the same map as every genuine
    /// removal, and everything here renders under the heading "Removed by
    /// pattern", so it must be excluded from both figures.
    public static let residualPrefix = "residual_secret_at"

    static func isRemoval(_ label: String) -> Bool {
        family(label) != residualPrefix
    }

    /// The part of a label before its first `:`. The count vocabulary is
    /// namespaced and open -- `secret:{pattern}`, `privacy_filter:{label}`,
    /// `tool_sensitive_field:{action}` are all generated -- so a shell can
    /// only reason about families, never about a closed set of labels.
    public static func family(_ label: String) -> String {
        label.split(separator: ":", maxSplits: 1).first.map(String.init) ?? label
    }

    /// Total occurrences of things that were actually removed.
    public static func total(_ occurrences: [String: Int]) -> Int {
        occurrences.filter { isRemoval($0.key) }.values.reduce(0, +)
    }

    /// "185 local path (12 distinct)  ·  3 secret"
    ///
    /// Ordered by count so the biggest number is first, which is what a
    /// person scanning a column of cards is looking for; ties break on the
    /// label so the order is stable between two redraws.
    public static func line(occurrences: [String: Int], distinct: [String: Int]) -> String {
        let occurrences = occurrences.filter { isRemoval($0.key) }
        if occurrences.isEmpty { return nothingMatched }
        return occurrences
            .sorted { $0.value == $1.value ? $0.key < $1.key : $0.value > $1.value }
            .map { label, count in
                let words = label.replacingOccurrences(of: "_", with: " ")
                // Only when it says something the occurrence count did not:
                // equal counts are the same fact twice, and a distinct count
                // above its occurrence count is impossible from a correct
                // daemon and not worth rendering from an incorrect one.
                guard let values = distinct[label], values > 0, values < count else {
                    return "\(count) \(words)"
                }
                return "\(count) \(words) (\(values) distinct)"
            }
            .joined(separator: "  ·  ")
    }
}
```

- [ ] **Step 4: Point the view and the model at it**

In `QueueView.swift`, delete the `static func removedSummary` block
(`:544-551`) and replace its call site in `footer` with:

```swift
                            Text(RedactionTally.line(
                                occurrences: summary.redactions,
                                distinct: summary.redactionsDistinct
                            ))
```

Replace `QueueRow.redactionCount`'s body with
`RedactionTally.total(summary?.redactions ?? [:])`.

In `Models.swift`, make `redactionReceipt` delegate rather than keep a
second copy of the wording:

```swift
    var redactionReceipt: String {
        if redactions.isEmpty { return "scrubbed: nothing matched" }
        return "scrubbed: " + RedactionTally.line(
            occurrences: redactions,
            distinct: redactionsDistinct
        ).replacingOccurrences(of: "  ·  ", with: ", ")
    }
```

Add `import TCShellCore` to any file that now needs it.

- [ ] **Step 5: Run the tests**

```bash
cd macos && swift test --filter RedactionTallyTests && swift test
```

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add macos/Sources/TCShellCore/RedactionTally.swift \
        macos/Tests/TCShellCoreTests/RedactionTallyTests.swift \
        macos/Sources/TraceCommonsApp/Views/QueueView.swift \
        macos/Sources/TraceCommonsApp/Models.swift
git commit -m "Move the removed-by-pattern figure into a tested type"
```

---

### Task 3: `QueueNavigation` -- where the drill-in can be

The queue gets a second level. The interesting part is not pushing a view,
it is what happens to the pushed view when the queue moves underneath it:
approve a folder's last session and the folder ceases to exist while you are
standing in it.

**Files:**
- Create: `macos/Sources/TCShellCore/QueueNavigation.swift`
- Test: `macos/Tests/TCShellCoreTests/QueueNavigationTests.swift` (create)

**Interfaces:**
- Consumes: `QueueGroup<Entry>` (existing, `QueueGrouping.swift`).
- Produces:
  - `public enum QueueLocation: Equatable { case root; case project(String) }`
  - `public static func resolve<E>(_ location: QueueLocation, in groups: [QueueGroup<E>]) -> QueueLocation`

- [ ] **Step 1: Write the failing tests**

Create `macos/Tests/TCShellCoreTests/QueueNavigationTests.swift`:

```swift
import XCTest

@testable import TCShellCore

/// The queue is now two levels, and the second one can be pulled out from
/// under the person standing on it: approving a folder's last session
/// removes the folder. Every one of these tests is that situation.
final class QueueNavigationTests: XCTestCase {
    private struct Entry: Equatable { let id: String }

    private func groups(_ ids: [String]) -> [QueueGroup<Entry>] {
        ids.map { QueueGroup(id: $0, label: $0, bytes: 1, entries: [Entry(id: $0)]) }
    }

    func testRootStaysRoot() {
        XCTAssertEqual(QueueNavigation.resolve(.root, in: groups(["a"])), .root)
        XCTAssertEqual(QueueNavigation.resolve(.root, in: groups([])), .root)
    }

    func testAProjectThatStillExistsIsKept() {
        XCTAssertEqual(
            QueueNavigation.resolve(.project("a"), in: groups(["a", "b"])),
            .project("a")
        )
    }

    func testAProjectThatEmptiedFallsBackToRoot() {
        // Submit all inside a folder: the folder goes, and standing in it
        // would show an empty screen with a back button and no explanation.
        XCTAssertEqual(QueueNavigation.resolve(.project("a"), in: groups(["b"])), .root)
    }

    func testTheLastProjectEmptyingFallsBackToRoot() {
        XCTAssertEqual(QueueNavigation.resolve(.project("a"), in: groups([])), .root)
    }

    func testResolutionIsByIDNotLabel() {
        // Two projects can share a label; only the id identifies one.
        let two = [
            QueueGroup(id: "proj_1", label: "api", bytes: 1, entries: [Entry(id: "x")]),
            QueueGroup(id: "proj_2", label: "api", bytes: 1, entries: [Entry(id: "y")]),
        ]
        XCTAssertEqual(QueueNavigation.resolve(.project("proj_2"), in: two), .project("proj_2"))
        XCTAssertEqual(QueueNavigation.resolve(.project("proj_3"), in: two), .root)
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cd macos && swift test --filter QueueNavigationTests
```

Expected: `cannot find 'QueueNavigation' in scope`.

- [ ] **Step 3: Write the implementation**

Create `macos/Sources/TCShellCore/QueueNavigation.swift`:

```swift
import Foundation

/// Which level of the queue is on screen.
public enum QueueLocation: Equatable, Hashable {
    /// The folder list.
    case root
    /// One folder's sessions, by `project_id` -- never by label, which two
    /// different projects can share.
    case project(String)
}

/// Keeping the queue's location honest as the queue moves.
///
/// The drill-in level names a project that may stop existing at any moment:
/// approving a folder's last session removes the folder, and so does an
/// upload finishing in the background. Without this, the detail view would
/// be left rendering an empty list with a back button and no account of
/// where its contents went.
///
/// This is a pure function of the location and the current groups rather
/// than a mutation, so the view can call it on every redraw and the
/// resolved location is never stale.
public enum QueueNavigation {
    /// The location that is actually valid, given what the queue now holds.
    /// A project that is gone resolves to `.root`.
    public static func resolve<Entry>(
        _ location: QueueLocation,
        in groups: [QueueGroup<Entry>]
    ) -> QueueLocation {
        switch location {
        case .root:
            return .root
        case .project(let id):
            return groups.contains(where: { $0.id == id }) ? .project(id) : .root
        }
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cd macos && swift test --filter QueueNavigationTests
```

Expected: 5 tests pass.

- [ ] **Step 5: Commit**

```bash
git add macos/Sources/TCShellCore/QueueNavigation.swift \
        macos/Tests/TCShellCoreTests/QueueNavigationTests.swift
git commit -m "Add queue navigation state that survives a folder emptying"
```

---

### Task 4: The folder list, and the folder detail

The layout change itself. Views only -- every decision inside it was made
testable in Tasks 2 and 3.

**Files:**
- Modify: `macos/Sources/TraceCommonsApp/Views/QueueView.swift` (`QueueContent.waiting`, `ProjectQueueGroup`)
- Create: `macos/Sources/TraceCommonsApp/Views/QueueFolderRow.swift`

**Interfaces:**
- Consumes: `QueueLocation`, `QueueNavigation.resolve` (Task 3);
  `QueueEntry.projectPath`, `.sessionPath` (Task 1).
- Produces: no new types outside the views.

- [ ] **Step 1: Add the location state**

In `QueueContent`, beside `visibleRowIDs`:

```swift
    /// Which level of the queue is showing. Resolved against the live
    /// groups on every redraw (`QueueNavigation.resolve`), so a folder that
    /// empties while it is open returns to the list rather than rendering
    /// an empty detail view.
    @State private var location: QueueLocation = .root
```

Replace the body of `waiting` with a switch on the resolved location:

```swift
    private var waiting: some View {
        let here = QueueNavigation.resolve(location, in: model.waitingByProject)
        return VStack(alignment: .leading, spacing: TC.Space.md) {
            switch here {
            case .root:
                folderList
            case .project(let id):
                if let group = model.waitingByProject.first(where: { $0.id == id }) {
                    folderDetail(group)
                }
            }
            ScrubbingCaveatNote()
                .padding(.top, TC.Space.xxs)
        }
        // Writing the resolved location back is what makes a vanished
        // folder's back button unnecessary rather than broken.
        .onChange(of: model.waitingByProject.map(\.id)) { _, _ in
            location = QueueNavigation.resolve(location, in: model.waitingByProject)
        }
    }
```

- [ ] **Step 2: Write the folder list**

Add to `QueueContent`:

```swift
    private var folderList: some View {
        VStack(alignment: .leading, spacing: TC.Space.md) {
            Text("^[\(model.decisionsOwed) session](inflect: true) waiting for your decision")
                .font(TC.Font_.sectionTitle)
                .foregroundStyle(TC.inkPrimary)

            LazyVStack(spacing: TC.Space.md) {
                ForEach(model.waitingByProject) { group in
                    QueueFolderRow(
                        group: group,
                        onOpen: { location = .project(group.id) },
                        onSubmitAll: { model.submitProject(id: group.id) },
                        onSubmitAllAs: { model.submitProject(id: group.id, verdict: $0) },
                        onIgnoreProject: {
                            model.ignoreProject(
                                id: group.id,
                                label: group.label,
                                promised: group.count
                            )
                        }
                    )
                }
            }
        }
    }
```

- [ ] **Step 3: Write the folder detail**

Add to `QueueContent`. This is today's `ProjectQueueGroup`, scoped to one
group, with a heading and a back control:

```swift
    private func folderDetail(_ group: QueueGroup<QueueEntry>) -> some View {
        VStack(alignment: .leading, spacing: TC.Space.md) {
            Button {
                location = .root
            } label: {
                HStack(spacing: TC.Space.xs) {
                    QueueGlyph(glyph: .chevronLeft, size: 11, color: TC.inkSecondary)
                    Text("All folders")
                        .font(TC.Font_.meta)
                        .foregroundStyle(TC.inkSecondary)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            VStack(alignment: .leading, spacing: TC.Space.xxs) {
                Text(group.label)
                    .font(TC.Font_.sectionTitle)
                    .foregroundStyle(TC.inkPrimary)
                if let path = group.entries.first?.projectPath, !path.isEmpty {
                    Text(path)
                        .font(TC.Font_.meta)
                        .foregroundStyle(TC.inkSecondary)
                        .textSelection(.enabled)
                }
            }

            ProjectQueueGroup(
                group: group,
                summaries: model.summaries,
                summaryErrors: model.summaryErrors,
                tooLarge: model.tooLarge,
                onLookInside: { previewing = $0 },
                onSubmit: { model.approve($0) },
                onDismiss: { model.dismiss($0) },
                onSubmitAll: { model.submitProject(id: group.id) },
                onSubmitAllAs: { model.submitProject(id: group.id, verdict: $0) },
                onIgnoreProject: {
                    model.ignoreProject(
                        id: group.id,
                        label: group.label,
                        promised: group.count
                    )
                },
                onAppear: { entry in
                    model.requestPreview(for: entry)
                    visibleRowIDs.insert(entry.entryID)
                    model.setPreviewVisible(visibleRowIDs)
                },
                onDisappear: { entry in
                    visibleRowIDs.remove(entry.entryID)
                    model.setPreviewVisible(visibleRowIDs)
                }
            )
        }
    }
```

Then delete the group-header `HStack` from `ProjectQueueGroup.body` (the
label, `Submit all`, `Submit all as`, and `Ignore`), leaving it as
`rowList` alone: those actions now live on the folder row one level up, and
two copies would be two things to keep in step. Keep the
`confirmationDialog` with whichever view still owns `Ignore` -- that is
`QueueFolderRow` after this task.

The glyph pair in play here is `QueueGlyph` / `QueueGlyphs`, at the foot of
`QueueView.swift`. `MacGlyphs` is a second, `fileprivate` copy in
`MainWindowView.swift` and is not reachable from this file.

`QueueGlyphs` has `.chevronRight` already; it has no `.chevronLeft`, so add
one beside the existing cases, drawn the way its neighbours are (the
existing `chevronRight` is `m6 4 4 4-4 4`, so its mirror is `m10 4-4 4 4 4`).

Both `QueueGlyph` and `QueueGlyphs` are `private` to `QueueView.swift`. The
new `QueueFolderRow` in Step 4 therefore either lives in `QueueView.swift`
beside them, or the pair is promoted to internal in the same commit. Prefer
the promotion and say so in the commit body -- a third copy of the glyph
machinery is what the comment on `QueueGlyph` already warns about.

- [ ] **Step 4: Write the folder row**

Create `macos/Sources/TraceCommonsApp/Views/QueueFolderRow.swift`:

```swift
import SwiftUI
import TCShellCore

/// One folder in the queue's root list.
///
/// The folder name is this row's largest text. It used to be its smallest
/// -- `TC.Font_.meta` in `inkSecondary`, beside a primary-styled
/// `Submit all` -- so the line read as a button with a caption rather than
/// as a place with actions. At 149 waiting sessions that inversion is the
/// difference between a list you can scan and one you cannot.
///
/// `Submit all` is shown at EVERY count, including one. The old rule hid it
/// at one because the row's own `Submit` was on the same screen and did the
/// same thing. Under drill-in it is a level down, so hiding it here would
/// mean opening a folder to do the thing the folder is offering. The rule
/// expired with the layout it was written for.
struct QueueFolderRow: View {
    let group: QueueGroup<QueueEntry>
    let onOpen: () -> Void
    let onSubmitAll: () -> Void
    let onSubmitAllAs: (ContributorVerdict) -> Void
    let onIgnoreProject: () -> Void

    @State private var confirmingIgnore = false

    private var path: String { group.entries.first?.projectPath ?? "" }

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.sm) {
            Button(action: onOpen) {
                HStack(alignment: .firstTextBaseline, spacing: TC.Space.s) {
                    VStack(alignment: .leading, spacing: TC.Space.xxs) {
                        Text(group.label)
                            .font(TC.Font_.cardTitle)
                            .foregroundStyle(TC.inkPrimary)
                        if !path.isEmpty {
                            Text(path)
                                .font(TC.Font_.meta)
                                .foregroundStyle(TC.inkSecondary)
                        }
                    }
                    Spacer(minLength: TC.Space.m)
                    Text("^[\(group.count) session](inflect: true)")
                        .font(TC.Font_.ledger)
                        .monospacedDigit()
                        .foregroundStyle(TC.inkSecondary)
                    Text(Format.bytes(group.bytes))
                        .font(TC.Font_.ledger)
                        .monospacedDigit()
                        .foregroundStyle(TC.inkTertiary)
                    QueueGlyph(glyph: .chevronRight, size: 11, color: TC.inkTertiary)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("\(group.label), \(group.count) waiting. Open.")

            HStack(spacing: TC.Space.s) {
                Button("Submit all (\(group.count))", action: onSubmitAll)
                    .tcPrimaryAction()
                    .help("""
                    Submits every session waiting in \(group.label). Each is scrubbed \
                    the same way a single Submit would be, and flagged sessions are \
                    included, not held back.
                    """)
                Menu(VerdictCopy.submitAllAs) {
                    ForEach(ContributorVerdict.allCases, id: \.rawValue) { option in
                        Button(option.label) { onSubmitAllAs(option) }
                    }
                }
                .menuStyle(.borderlessButton)
                .fixedSize()
                .help(VerdictCopy.submitAllAsTooltip)
                Spacer(minLength: TC.Space.m)
                // Never `.tcPrimaryAction()`: it sits beside a control that
                // uploads the very traces this removes, and two adjacent
                // actions that do opposite things must not look alike.
                Button(ProjectIgnoreCopy.buttonLabel) { confirmingIgnore = true }
                    .help(ProjectIgnoreCopy.tooltip)
            }
        }
        .padding(TC.Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .tcCard()
        .confirmationDialog(
            ProjectIgnoreCopy.confirmationTitle(project: group.label),
            isPresented: $confirmingIgnore,
            titleVisibility: .visible
        ) {
            Button(ProjectIgnoreCopy.buttonLabel, role: .destructive, action: onIgnoreProject)
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(ProjectIgnoreCopy.confirmationBody(
                project: group.label,
                pendingCount: group.count
            ))
        }
    }
}
```

- [ ] **Step 5: Show where a session actually ran**

In `QueueRow.identity` (`QueueView.swift:365`), after the `projectLabel`
text, add the subdirectory line so a repo-rooted group still says where each
session came from:

```swift
            if let sessionPath = entry.sessionPath, !sessionPath.isEmpty {
                Text(sessionPath)
                    .font(TC.Font_.meta)
                    .foregroundStyle(TC.inkTertiary)
                    .lineLimit(1)
                    .truncationMode(.head)
            }
```

- [ ] **Step 6: Build and run the whole suite**

```bash
cargo build -p trace-commons-contributor-ffi
cd macos && swift build && swift test
```

Expected: builds, all tests pass. No new tests here -- the testable parts
were Tasks 2 and 3.

- [ ] **Step 7: Commit**

```bash
git add macos/Sources/TraceCommonsApp/Views/
git commit -m "Show folders first in the queue, with sessions one level in"
```

---

### Task 5: The card opens the preview

**Files:**
- Modify: `macos/Sources/TraceCommonsApp/Views/QueueView.swift` (`QueueRow.body`)

**Interfaces:**
- Consumes: `QueueRow.onLookInside` (existing).
- Produces: nothing new.

- [ ] **Step 1: Make the card a hit target**

In `QueueRow.body`, after `.tcCard(...)`:

```swift
        // A second route to `Look inside`, never a replacement for it. The
        // button keeps its emphasis: one-click submit added AVAILABILITY,
        // and primary styling is a RECOMMENDATION -- see `actions`. What
        // this adds is that the obvious gesture on a card does the obvious
        // thing.
        .contentShape(Rectangle())
        .onTapGesture(perform: onLookInside)
        .accessibilityElement(children: .contain)
```

The three footer buttons are `Button`s inside the card and consume their own
taps, so `Not this one`, `Submit`, and `Look inside` keep working. Verify
that by hand in Step 2 -- if any of them starts opening the preview instead,
the fix is `.allowsHitTesting` scoping on the footer, not removing the
gesture.

- [ ] **Step 2: Build, and check the buttons by hand**

```bash
cd macos && swift build && swift test
```

Then launch the app and confirm: clicking the card body opens the preview;
clicking `Not this one`, `Submit`, and `Look inside` each still do their own
thing. Paste what you observed into the commit message -- `swift test`
cannot see any of this.

- [ ] **Step 3: Commit**

```bash
git add macos/Sources/TraceCommonsApp/Views/QueueView.swift
git commit -m "Open the preview from anywhere on the card"
```

---

### Task 6: Name the chips the shell already draws

The spec's 3.1 is a correction: **all three shells already mark the
redactor's tokens**, and this one has done it since
`TranscriptMarkerScan`/`TranscriptMarkers.chipped` landed. That scan is
deliberately shared with the chunker so a marker is never cut in half, and
the chip styling is a considered choice recorded in its own doc comment. A
shell must not add a second marker pass, restyle the existing chips, or
bypass the chunk-boundary contract.

What is missing is the *naming*: every chip today draws as the same
anonymous token. This task adds one pure function that turns a matched token
into words, and calls it from the one place chips are already built.

**Files:**
- Create: `macos/Sources/TCShellCore/RedactionMarkerNames.swift`
- Modify: `macos/Sources/TraceCommonsApp/Views/PreviewSheet.swift` (`TranscriptMarkers.chipped`, and the `caption` sentence)
- Test: `macos/Tests/TCShellCoreTests/RedactionMarkerNamesTests.swift` (create)

**Interfaces:**
- Consumes: `TCShellCore.TranscriptMarkerScan.spans(in:)` -- the existing
  scan, unchanged. No new scan, no new pattern, no new constant.
- Produces:
  - `public struct RedactionMarkerName: Equatable { public let text: String; public let ordinal: Int? }`
  - `public static func RedactionMarkerNames.of(_ token: String) -> RedactionMarkerName`

- [ ] **Step 1: Write the failing tests**

Create `macos/Tests/TCShellCoreTests/RedactionMarkerNamesTests.swift`:

```swift
import XCTest

@testable import TCShellCore

/// The scan already finds every marker. What it does not do is say what one
/// was. These are the tokens `TranscriptMarkerScan.pattern` matches, and
/// every one of them has to come back with words a contributor can read.
final class RedactionMarkerNamesTests: XCTestCase {
    func testTheNumberedFormCarriesALabelAndAnOrdinal() {
        let name = RedactionMarkerNames.of("<PRIVATE_LOCAL_PATH_3>")
        XCTAssertEqual(name.text, "local path")
        XCTAssertEqual(name.ordinal, 3)
    }

    func testTheOtherNumberedLabelIsNamedToo() {
        // `apply_placeholder_regex` mints the numbered form for exactly two
        // labels: `local_path` and `private_email`.
        XCTAssertEqual(RedactionMarkerNames.of("<PRIVATE_PRIVATE_EMAIL_1>").text, "private email")
    }

    /// The ordinal is the LAST underscore-delimited run of digits, so a
    /// label that itself ends in a number must not steal it.
    func testALabelContainingDigitsIsParsedCorrectly() {
        let name = RedactionMarkerNames.of("<PRIVATE_SHA256_KEY_7>")
        XCTAssertEqual(name.text, "sha256 key")
        XCTAssertEqual(name.ordinal, 7)
    }

    /// The five fixed tokens the pipeline emits, taken from the same sources
    /// as the GTK scanner's `every_fixed_token_the_pipeline_emits_is_matched`
    /// guard: `apply_redaction_ranges`, `apply_pem_block_redaction`,
    /// `redacted_marker`, and `redaction.rs`'s `REDACTED`. None of them
    /// carries an ordinal -- there is no second number to report, and
    /// inventing one would claim a distinctness the token does not have.
    func testEveryFixedTokenIsNamedAndCarriesNoOrdinal() {
        let cases = [
            ("[REDACTED]", "something removed"),
            ("[REDACTED:aws_secret_key]", "aws secret key"),
            ("[REDACTED:person_name]", "person name"),
            ("[REDACTED_PATH]", "URL path"),
            ("<REDACTED_PRIVATE_KEY>", "private key"),
        ]
        for (token, expected) in cases {
            let name = RedactionMarkerNames.of(token)
            XCTAssertEqual(name.text, expected, "\(token)")
            XCTAssertNil(name.ordinal, "\(token) carries no ordinal")
        }
    }

    /// Labels are an open, namespaced vocabulary. A token this build has no
    /// words for must still say that something left, never nothing.
    func testAnUnrecognizedTokenStillSaysSomethingLeft() {
        let name = RedactionMarkerNames.of("[REDACTED:some_future_detector]")
        XCTAssertEqual(name.text, "some future detector")
        XCTAssertNil(name.ordinal)
    }

    /// Every token the shared scan matches must name to something. This is
    /// the guard on that: it drives the naming from the scan rather than
    /// from a second list that could drift away from it.
    func testEveryTokenTheScanFindsIsNamed() {
        let body = """
            <PRIVATE_LOCAL_PATH_1> [REDACTED] [REDACTED:aws_secret_key] \
            [REDACTED_PATH] <REDACTED_PRIVATE_KEY>
            """
        let spans = TranscriptMarkerScan.spans(in: body)
        XCTAssertEqual(spans.count, 5)
        for span in spans {
            XCTAssertFalse(RedactionMarkerNames.of(String(body[span])).text.isEmpty)
        }
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cd macos && swift test --filter RedactionMarkerNamesTests
```

Expected: `cannot find 'RedactionMarkerNames' in scope`.

If `testEveryTokenTheScanFindsIsNamed` reports fewer than 5 spans, this
checkout predates the widened pattern -- `<REDACTED_PRIVATE_KEY>` was
unmatched until it was added to `TranscriptMarkerScan.pattern`. Rebase
rather than working around it here; the pattern is the chunker's too.

- [ ] **Step 3: Write the implementation**

Create `macos/Sources/TCShellCore/RedactionMarkerNames.swift`. It takes the
matched token text -- nothing else -- and returns words:

- `<PRIVATE_<LABEL>_<n>>`: the label lowercased with `_` as a space, and `n`
  as the ordinal. The redactor mints one token per distinct value and reuses
  it, so two chips with the same ordinal are the same original string, which
  is the fact worth surfacing.
- `<REDACTED_PRIVATE_KEY>`: `"private key"`, no ordinal.
- `[REDACTED:<label>]`: the label lowercased with `_` as a space, no ordinal.
- `[REDACTED_PATH]`: `"URL path"` -- it replaces a URL's path component, not
  a local one.
- `[REDACTED]` and anything else the scan matched: `"something removed"`.
  Never empty, and never a guess at a category.

The doc comment records the two things the type must not be read as saying:
a region with no chip is not a region with nothing sensitive in it -- the
detector scans every leaf while the rewriter reaches only typed fields --
and a name is not a distinct count. Only `local_path` and `private_email`
mint placeholders, so only those can report "the same value twice".

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cd macos && swift test --filter RedactionMarkerNamesTests
```

- [ ] **Step 5: Put the name on the chip**

In `PreviewSheet.swift`'s existing `TranscriptMarkers.chipped` -- the same
loop over `TranscriptMarkerScan.spans(in:)`, with the same
`TC.redactionChipBackground` / `TC.redactionChipForeground` and the same
bold weight. Only the chip's *string* changes:

```swift
            let name = RedactionMarkerNames.of(String(text[range]))
            let label = name.ordinal.map { "\(name.text) #\($0)" } ?? name.text
            var chip = AttributedString(label)
```

Do not touch the scan, the pattern, the tone, or the chunk loop.

Two consequences to carry rather than discover:

1. The transcript's on-screen text is no longer byte-identical to the body,
   because a chip now reads `local path #1` where the token stood. The
   caption above it says "These are the exact bytes an approval covers", so
   amend that sentence to say the marks are named rather than literal. The
   `copyAll` path is unaffected -- it copies `document.wholeText()`, the
   body itself, not the chipped `AttributedString`.
2. The chip's width changes, so `TranscriptRowIndex`'s row estimate is
   slightly further off for a chunk full of markers. It is an estimate by
   construction and the error is already bounded per chunk; no change.

Keep `ScrubbingCaveatNote()` visible on the same tab as the marks.

- [ ] **Step 6: Build and run the suite**

```bash
cargo build -p trace-commons-contributor-ffi
cd macos && swift build && swift test
```

Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add macos/Sources/TCShellCore/RedactionMarkerNames.swift \
        macos/Tests/TCShellCoreTests/RedactionMarkerNamesTests.swift \
        macos/Sources/TraceCommonsApp/Views/PreviewSheet.swift
git commit -m "Say what each redaction chip stood for"
```

---

### Task 6b: The removed-summary panel

Marking placeholders answers *where*. It does not answer "so I can right away
see what doesn't go", because collecting the marks means scrolling the whole
transcript. This is the panel that answers it -- and the surface where the
`residual_secret_at` defect gets stated correctly rather than backwards.

**Files:**
- Create: `macos/Sources/TCShellCore/RedactionSummary.swift`
- Modify: `macos/Sources/TraceCommonsApp/Views/PreviewSheet.swift` (scrubbing tab)
- Test: `macos/Tests/TCShellCoreTests/RedactionSummaryTests.swift` (create)

**Interfaces:**
- Consumes: `RedactionTally.family`, `.residualPrefix`, `.isRemoval` (Task 2);
  `PreviewSummary.redactions`, `.redactionsDistinct` (Task 1).
- Produces:
  - `public struct RedactionSummaryRow: Equatable { public let family: String; public let display: String; public let description: String; public let occurrences: Int; public let distinct: Int; public let detail: [String] }`
  - `public static func rows(occurrences: [String: Int], distinct: [String: Int]) -> (removed: [RedactionSummaryRow], stillPresent: [RedactionSummaryRow])`

- [ ] **Step 1: Write the failing tests**

Create `macos/Tests/TCShellCoreTests/RedactionSummaryTests.swift`:

```swift
import XCTest

@testable import TCShellCore

/// The scrubbing tab's "what left, and what didn't" panel.
///
/// The label vocabulary is open and namespaced -- `secret:{pattern}`,
/// `privacy_filter:{label}`, `tool_sensitive_field:{action}` are all
/// generated -- so this type can only reason about families, and it must
/// never claim to know what an unfamiliar one means.
final class RedactionSummaryTests: XCTestCase {
    func testAnEmptyMapProducesNoRows() {
        let out = RedactionSummary.rows(occurrences: [:], distinct: [:])
        XCTAssertTrue(out.removed.isEmpty)
        XCTAssertTrue(out.stillPresent.isEmpty)
    }

    func testOneFamilyBecomesOneRow() {
        let out = RedactionSummary.rows(
            occurrences: ["local_path": 185],
            distinct: ["local_path": 12]
        )
        XCTAssertEqual(out.removed.count, 1)
        XCTAssertEqual(out.removed[0].family, "local_path")
        XCTAssertEqual(out.removed[0].display, "local path")
        XCTAssertEqual(out.removed[0].occurrences, 185)
        XCTAssertEqual(out.removed[0].distinct, 12)
        XCTAssertFalse(out.removed[0].description.isEmpty)
    }

    /// Nine secret patterns are one `secret` row, not nine rows. The
    /// sub-labels go on a detail line.
    func testSubLabelsCollapseIntoTheirFamily() {
        let out = RedactionSummary.rows(
            occurrences: ["secret:contextual_entropy": 3, "secret:pem_private_key": 1, "secret": 2],
            distinct: ["secret:contextual_entropy": 2, "secret:pem_private_key": 1, "secret": 2]
        )
        XCTAssertEqual(out.removed.count, 1)
        XCTAssertEqual(out.removed[0].family, "secret")
        XCTAssertEqual(out.removed[0].occurrences, 6)
        XCTAssertEqual(out.removed[0].distinct, 5)
        XCTAssertEqual(
            out.removed[0].detail,
            ["contextual entropy", "pem private key"]
        )
    }

    /// A secret that was DETECTED AND NOT REMOVED. Putting it in `removed`
    /// would state the exact opposite of what happened.
    func testAResidualSurvivorIsReportedAsStillPresent() {
        let out = RedactionSummary.rows(
            occurrences: ["local_path": 3, "residual_secret_at:events.correction": 1],
            distinct: [:]
        )
        XCTAssertEqual(out.removed.map(\.family), ["local_path"])
        XCTAssertEqual(out.stillPresent.map(\.family), ["residual_secret_at"])
        XCTAssertEqual(out.stillPresent[0].detail, ["events.correction"])
    }

    /// An unfamiliar family gets a neutral description and is NEVER dropped.
    /// Hiding a category because this build has no words for it would
    /// understate what happened, which is the one direction this panel must
    /// not fail in.
    func testAnUnknownFamilyIsKeptWithANeutralDescription() {
        let out = RedactionSummary.rows(occurrences: ["future_category": 4], distinct: [:])
        XCTAssertEqual(out.removed.count, 1)
        XCTAssertEqual(out.removed[0].family, "future_category")
        XCTAssertFalse(out.removed[0].description.isEmpty)
        XCTAssertFalse(
            out.removed[0].description.contains("future"),
            "a neutral description must not pretend to know the category"
        )
    }

    func testRowsAreOrderedByOccurrencesThenFamily() {
        let out = RedactionSummary.rows(
            occurrences: ["secret": 3, "local_path": 185, "email": 3],
            distinct: [:]
        )
        XCTAssertEqual(out.removed.map(\.family), ["local_path", "email", "secret"])
    }

    /// The panel names kinds, never values. There is no value left to name.
    func testARowCarriesNoMatchedText() {
        let out = RedactionSummary.rows(occurrences: ["secret": 1], distinct: [:])
        XCTAssertEqual(out.removed[0].detail, [])
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cd macos && swift test --filter RedactionSummaryTests
```

Expected: `cannot find 'RedactionSummary' in scope`.

- [ ] **Step 3: Write the implementation**

Create `macos/Sources/TCShellCore/RedactionSummary.swift`:

```swift
import Foundation

/// One category's line in the scrubbing panel.
public struct RedactionSummaryRow: Equatable {
    /// The label family -- the part before the first `:`.
    public let family: String
    /// The family as a person reads it.
    public let display: String
    /// What this category IS. The panel's actual value to a reader who has
    /// never seen these words.
    public let description: String
    public let occurrences: Int
    public let distinct: Int
    /// The specific sub-labels this family covered, humanised. Safe to
    /// render: sub-labels are schema-shaped identifiers by construction --
    /// `log_residual_secret_locations` depends on the same property -- never
    /// contributor strings. Empty when the family had no sub-labels.
    public let detail: [String]
}

/// What scrubbing took out of this session, and what it found and left in.
///
/// Marking placeholders in the transcript answers *where*. This answers
/// *what*, without scrolling, which is the half the card's one-line figure
/// could only gesture at.
///
/// It names KINDS, never values. The value is gone by construction, and a
/// panel listing the actual strings would make the preview window the single
/// best thing on the machine to photograph.
public enum RedactionSummary {
    /// What each family is, in words. Deliberately not exhaustive -- the
    /// vocabulary is generated and open -- which is why `describe` has a
    /// neutral fallback rather than a `fatalError`.
    static let descriptions: [String: String] = [
        "local_path": "File paths from this machine.",
        "secret": """
        API keys, tokens, private keys, and high-entropy strings found next to \
        credential words.
        """,
        "privacy_filter": "Names, emails, and other personal details found in prose.",
        "sensitive_field": "Fields whose name marks them sensitive, like password or authorization.",
        "tool_sensitive_field": "Tool-call arguments whose name marks them sensitive.",
        RedactionTally.residualPrefix: """
        Found, and still in what would be sent. Either a credential inside a \
        correction you wrote, which is kept on purpose, or a field scrubbing \
        does not reach.
        """,
    ]

    /// The neutral description for a family this build has no words for.
    ///
    /// It must still appear. Dropping an unrecognised category would
    /// understate what happened, and this panel may only ever err toward
    /// saying more than it can explain.
    static let unknownDescription = "Removed by a pattern this version has no description for."

    static func describe(_ family: String) -> String {
        descriptions[family] ?? unknownDescription
    }

    static func humanise(_ text: String) -> String {
        text.replacingOccurrences(of: "_", with: " ")
    }

    public static func rows(
        occurrences: [String: Int],
        distinct: [String: Int]
    ) -> (removed: [RedactionSummaryRow], stillPresent: [RedactionSummaryRow]) {
        var byFamily: [String: (occurrences: Int, distinct: Int, detail: [String])] = [:]
        for (label, count) in occurrences {
            let family = RedactionTally.family(label)
            var bucket = byFamily[family] ?? (0, 0, [])
            bucket.occurrences += count
            bucket.distinct += distinct[label] ?? 0
            if label != family {
                bucket.detail.append(humanise(String(label.dropFirst(family.count + 1))))
            }
            byFamily[family] = bucket
        }

        let all = byFamily
            .map { family, bucket in
                RedactionSummaryRow(
                    family: family,
                    display: humanise(family),
                    description: describe(family),
                    occurrences: bucket.occurrences,
                    distinct: bucket.distinct,
                    detail: bucket.detail.sorted()
                )
            }
            .sorted {
                $0.occurrences == $1.occurrences
                    ? $0.family < $1.family
                    : $0.occurrences > $1.occurrences
            }

        return (
            removed: all.filter { RedactionTally.isRemoval($0.family) },
            stillPresent: all.filter { !RedactionTally.isRemoval($0.family) }
        )
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cd macos && swift test --filter RedactionSummaryTests
```

Expected: 7 tests pass.

- [ ] **Step 5: Draw the panel**

On the preview sheet's scrubbing tab, above the transcript marks, render the
two sections:

- **"Removed"** -- each row as its display name and count
  (`185 local path`, plus `(12 distinct)` when that differs), the description
  in secondary ink beneath, and the detail line in tertiary ink when
  non-empty. `TC.inkPrimary` / `TC.inkSecondary` / `TC.inkTertiary`, no new
  tokens.
- **"Found, and still in what would be sent"** -- only when `stillPresent` is
  non-empty, in `TC.goldText` with the same warning glyph the
  nothing-matched chip uses, listing the schema paths from `detail`.

Keep `ScrubbingCaveat`'s sentence below both: a panel that enumerates
categories makes the app look more thorough than it is, which is exactly when
that sentence earns its place.

- [ ] **Step 6: Run the whole suite**

```bash
cargo build -p trace-commons-contributor-ffi
cd macos && swift build && swift test
```

Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add macos/Sources/TCShellCore/RedactionSummary.swift \
        macos/Tests/TCShellCoreTests/RedactionSummaryTests.swift \
        macos/Sources/TraceCommonsApp/Views/PreviewSheet.swift
git commit -m "Summarise what scrubbing removed, and what it left in"
```

---

### Task 7: Recent searches stop recording prefixes

Typing `xyz` records `x`, `xy`, and `xyz`, filling a six-slot strip with
prefixes of one word. The search runs on every keystroke and `run()`
remembers every hit.

**Files:**
- Create: `macos/Sources/TCShellCore/RecentSearches.swift` (moved from `PreviewSheet.swift:1237-1283`)
- Modify: `macos/Sources/TraceCommonsApp/Views/PreviewSheet.swift:735-830`
- Test: `macos/Tests/TCShellCoreTests/RecentSearchesTests.swift` (create)

**Interfaces:**
- Consumes: nothing.
- Produces: `public enum RecentSearches` with `load()`, `remember(_:) -> [String]`, `purgeLegacyStore()`, and a test-only `reset()`.

- [ ] **Step 1: Write the failing tests**

Create `macos/Tests/TCShellCoreTests/RecentSearchesTests.swift`:

```swift
import XCTest

@testable import TCShellCore

/// A recent-search list is the contributor's list of the things they were
/// afraid of leaking. It stays in memory for that reason, and it must hold
/// what they actually asked -- not every prefix they typed on the way there.
final class RecentSearchesTests: XCTestCase {
    override func setUp() {
        super.setUp()
        RecentSearches.reset()
    }

    func testAnEmptyListStartsEmpty() {
        XCTAssertTrue(RecentSearches.load().isEmpty)
    }

    func testACommittedTermIsRemembered() {
        XCTAssertEqual(RecentSearches.remember("acme-corp"), ["acme-corp"])
    }

    func testTheMostRecentTermLeads() {
        _ = RecentSearches.remember("first")
        XCTAssertEqual(RecentSearches.remember("second"), ["second", "first"])
    }

    func testRepeatingATermMovesItToTheFrontWithoutDuplicating() {
        _ = RecentSearches.remember("a")
        _ = RecentSearches.remember("b")
        XCTAssertEqual(RecentSearches.remember("a"), ["a", "b"])
    }

    func testTheListIsCappedAtSix() {
        for term in ["1", "2", "3", "4", "5", "6", "7"] {
            _ = RecentSearches.remember(term)
        }
        XCTAssertEqual(RecentSearches.load().count, 6)
        XCTAssertEqual(RecentSearches.load().first, "7")
        XCTAssertFalse(RecentSearches.load().contains("1"))
    }

    func testAnEmptyOrBlankTermIsNotRemembered() {
        _ = RecentSearches.remember("")
        _ = RecentSearches.remember("   ")
        XCTAssertTrue(RecentSearches.load().isEmpty)
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cd macos && swift test --filter RecentSearchesTests
```

Expected: `cannot find 'RecentSearches' in scope` (it is currently private
to a view file).

- [ ] **Step 3: Move the type and add the guards**

Cut `enum RecentSearches` out of `PreviewSheet.swift` and put it in
`macos/Sources/TCShellCore/RecentSearches.swift`, keeping its doc comment
verbatim -- the reasoning about why it is in memory rather than on disk is
the most valuable thing in it -- and making it `public`. Add:

```swift
    public static func remember(_ term: String) -> [String] {
        let term = term.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !term.isEmpty else { return terms }
        terms = [term] + terms.filter { $0 != term }
        terms = Array(terms.prefix(6))
        return terms
    }

    /// Test seam. The list is process-lifetime state, so an XCTest run
    /// would otherwise leak terms between cases.
    public static func reset() {
        terms = []
    }
```

- [ ] **Step 4: Only remember a committed search**

In `PreviewSheet.swift`, split the live search from the act of remembering:

```swift
    /// Runs the search. Live, on every keystroke -- that part is the good
    /// part and stays.
    private func run() {
        searched = true
        guard !needle.isEmpty, let preview else {
            offsets = []
            return
        }
        offsets = preview.search(needle)
    }

    /// Runs the search AND records the term.
    ///
    /// Separate from `run` because `run` fires on every keystroke, and
    /// remembering there filled the six-slot strip with the prefixes of one
    /// word: typing "xyz" recorded "x", "xy", and "xyz". A recent search is
    /// a question the contributor asked, and they ask it by pressing Return
    /// or the button -- not by passing through a prefix on the way.
    private func commit() {
        run()
        if let offsets, !offsets.isEmpty {
            recents = RecentSearches.remember(needle)
        }
    }
```

Point `.onSubmit` and the `Search` button at `commit`, and leave
`.onChange(of: needle)` pointed at `run`. Add `import TCShellCore` if the
file lacks it.

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cd macos && swift test --filter RecentSearchesTests && swift test
```

Expected: 6 new tests pass, whole suite green.

- [ ] **Step 6: Commit**

```bash
git add macos/Sources/TCShellCore/RecentSearches.swift \
        macos/Tests/TCShellCoreTests/RecentSearchesTests.swift \
        macos/Sources/TraceCommonsApp/Views/PreviewSheet.swift
git commit -m "Remember a search when it is asked, not on every keystroke"
```

---

### Task 8: Tell "never there" apart from "removed"

`tc_preview_search` scans the redacted body, so a removed value returns zero
matches -- which looks exactly like a value that was never in the session.
Those are the two answers a contributor checking for a client name most
needs to tell apart.

**Files:**
- Modify: `macos/Sources/TCBridge/TCDaemon.swift` (add `searchOriginal`)
- Create: `macos/Sources/TCShellCore/OriginalSearchOutcome.swift`
- Modify: `macos/Sources/TraceCommonsApp/Views/PreviewSheet.swift` (`resultSummary`)
- Test: `macos/Tests/TCShellCoreTests/OriginalSearchOutcomeTests.swift` (create)

**Interfaces:**
- Consumes: `tc_search_original` (plan 1 Task 8).
- Produces:
  - `TCDaemon.searchOriginal(entryID: String, needle: String) -> Int?` (nil on error)
  - `public enum OriginalSearchOutcome: Equatable { case absent, allRemoved(Int), someRemain(remaining: Int, total: Int), unknown }`
  - `public static func classify(remaining: Int, original: Int?) -> OriginalSearchOutcome`
  - `public var sentence: String`

- [ ] **Step 1: Write the failing tests**

Create `macos/Tests/TCShellCoreTests/OriginalSearchOutcomeTests.swift`:

```swift
import XCTest

@testable import TCShellCore

/// Three answers, and they are not interchangeable. "0 matches" against the
/// redacted body means either "it was never here" or "we took it out", and
/// a contributor checking whether their employer's name is in a trace needs
/// to know which.
final class OriginalSearchOutcomeTests: XCTestCase {
    func testNowhereInEitherTextIsAbsent() {
        XCTAssertEqual(OriginalSearchOutcome.classify(remaining: 0, original: 0), .absent)
    }

    func testPresentOriginallyAndGoneNowIsAllRemoved() {
        XCTAssertEqual(
            OriginalSearchOutcome.classify(remaining: 0, original: 3),
            .allRemoved(3)
        )
    }

    func testStillPresentIsSomeRemain() {
        XCTAssertEqual(
            OriginalSearchOutcome.classify(remaining: 2, original: 5),
            .someRemain(remaining: 2, total: 5)
        )
    }

    func testAFailedOriginalSearchIsUnknownNotAbsent() {
        // Reporting "not in this trace" because a call failed would be the
        // single most dangerous wrong answer this tab can give.
        XCTAssertEqual(OriginalSearchOutcome.classify(remaining: 0, original: nil), .unknown)
        XCTAssertEqual(
            OriginalSearchOutcome.classify(remaining: 2, original: nil),
            .someRemain(remaining: 2, total: 2)
        )
    }

    func testAnOriginalCountBelowTheRemainingCountFallsBackToWhatIsCertain() {
        // Impossible from a correct daemon. The certain half is that 2 are
        // still there.
        XCTAssertEqual(
            OriginalSearchOutcome.classify(remaining: 2, original: 1),
            .someRemain(remaining: 2, total: 2)
        )
    }

    func testTheSentencesSayWhichCaseItIs() {
        XCTAssertEqual(OriginalSearchOutcome.absent.sentence, "0 matches -- not in this session")
        XCTAssertEqual(
            OriginalSearchOutcome.allRemoved(3).sentence,
            "3 matches -- all 3 were removed"
        )
        XCTAssertEqual(
            OriginalSearchOutcome.someRemain(remaining: 2, total: 5).sentence,
            "5 matches -- 2 would still be sent"
        )
        XCTAssertEqual(
            OriginalSearchOutcome.unknown.sentence,
            "0 matches in what would be sent. Couldn't check the original."
        )
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cd macos && swift test --filter OriginalSearchOutcomeTests
```

Expected: `cannot find 'OriginalSearchOutcome' in scope`.

- [ ] **Step 3: Write the outcome type**

Create `macos/Sources/TCShellCore/OriginalSearchOutcome.swift`:

```swift
import Foundation

/// What a search actually found, across both the redacted body and the
/// original session.
///
/// `tc_preview_search` scans the REDACTED body, which is the right thing
/// for "what would be sent" and the wrong thing for "was it ever here". A
/// value the scrubber removed returns zero matches, and so does a value that
/// never existed. This type is the difference between those.
public enum OriginalSearchOutcome: Equatable {
    /// Not in the session at all.
    case absent
    /// It was there, and scrubbing took all of it. The count is how many
    /// times it appeared originally.
    case allRemoved(Int)
    /// Still present in what would be sent. The alarming case.
    case someRemain(remaining: Int, total: Int)
    /// The original could not be checked. Never reported as `absent`.
    case unknown

    /// `remaining` is the match count in the redacted body; `original` is
    /// the count in the pre-redaction session, or nil if that call failed.
    public static func classify(remaining: Int, original: Int?) -> OriginalSearchOutcome {
        guard let original else {
            // Fail toward what is certain. The redacted body is in hand, so
            // matches in it are known; the absence of a check is not the
            // same as a clean result and must never render as one.
            return remaining > 0
                ? .someRemain(remaining: remaining, total: remaining)
                : .unknown
        }
        if remaining > 0 {
            return .someRemain(remaining: remaining, total: max(original, remaining))
        }
        return original > 0 ? .allRemoved(original) : .absent
    }

    public var sentence: String {
        switch self {
        case .absent:
            return "0 matches -- not in this session"
        case .allRemoved(let count):
            return "\(count) matches -- all \(count) were removed"
        case .someRemain(let remaining, let total):
            return "\(total) matches -- \(remaining) would still be sent"
        case .unknown:
            return "0 matches in what would be sent. Couldn't check the original."
        }
    }

    /// Whether this is the answer to slow down on.
    public var isAlarming: Bool {
        if case .someRemain = self { return true }
        return false
    }
}
```

- [ ] **Step 4: Add the bridge call**

In `macos/Sources/TCBridge/TCDaemon.swift`, beside the other handle-taking
calls:

```swift
    /// How many times `needle` appears in an entry's PRE-redaction session
    /// text. `nil` on error.
    ///
    /// A COUNT, never content -- that is the whole bound of the ABI call
    /// behind this, and the reason it is allowed to read unredacted bytes at
    /// all. Takes an entry id rather than an open preview because a preview
    /// lives as long as its sheet, and an unredacted transcript must not.
    public func searchOriginal(entryID: String, needle: String) -> Int? {
        guard let handle else { return nil }
        let count: Int32 = entryID.withCString { cEntry in
            needle.withCString { cNeedle in
                tc_search_original(handle, cEntry, cNeedle)
            }
        }
        return count >= 0 ? Int(count) : nil
    }
```

Match the file's existing name for the stored handle -- if it is not
`handle`, use whatever `TCDaemon` calls it.

- [ ] **Step 5: Wire it into the search tab**

In `PreviewSheet.swift`, hold the outcome alongside `offsets`:

```swift
    @State private var outcome: OriginalSearchOutcome?
```

In `run()`, after setting `offsets`, classify:

```swift
        outcome = OriginalSearchOutcome.classify(
            remaining: offsets?.count ?? 0,
            original: daemon.searchOriginal(entryID: entry.entryID, needle: needle)
        )
```

Replace `resultSummary`'s two count branches with the outcome's sentence,
keeping the existing glyph-and-tone treatment and picking the tone from
`outcome.isAlarming`:

```swift
        } else if let outcome {
            Label(
                outcome.sentence,
                systemImage: outcome.isAlarming ? TC.Tone.attention.symbol : TC.Tone.clear.symbol
            )
            .font(TC.Font_.headingAlert)
            .foregroundStyle(outcome.isAlarming ? TC.Tone.attention.textColor : TC.Tone.clear.textColor)
        }
```

`SearchTab` today holds exactly `document` and `preview`, so it needs the
entry id and a way to call the daemon. Neither comes from `QueueView`, which
constructs `PreviewSheet(entry:)` and nothing further. Thread them from
`PreviewSheet` instead, which already holds `entry` and an
`@EnvironmentObject private var model: AppModel`:

- Add `searchOriginal(entryID:needle:) -> Int?` to `AppModel`, beside
  `openPreview(entryID:)`. `AppModel.daemon` is `private` and stays that
  way -- a view holding the handle is what `DaemonCalling` exists to avoid.
- Give `SearchTab` an `entryID: String` and pass `entry.entryID` at the
  `SearchTab(...)` call site in `PreviewSheet.body`, alongside the existing
  `document` / `preview` / `initialNeedle` / `initialOffsets`. `SearchTab`
  reads `model` from the environment like its parent does.

- [ ] **Step 6: Run the tests**

```bash
cargo build -p trace-commons-contributor-ffi
cd macos && swift build && swift test
```

Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add macos/Sources/TCShellCore/OriginalSearchOutcome.swift \
        macos/Tests/TCShellCoreTests/OriginalSearchOutcomeTests.swift \
        macos/Sources/TCBridge/TCDaemon.swift \
        macos/Sources/TraceCommonsApp/Views/PreviewSheet.swift
git commit -m "Say whether a searched value was removed or was never there"
```

---

### Task 9: The nothing-matched chip becomes a control

The gold is right and stays gold -- a session where no pattern fired is the
one worth slowing down on. What it lacks is a next step.

**Files:**
- Modify: `macos/Sources/TraceCommonsApp/Views/QueueView.swift:503-522` (`nothingMatchedChip`)
- Modify: `macos/Sources/TraceCommonsApp/Views/ScrubbingCaveat.swift:60-70` (the row line)
- Test: `macos/Tests/TCShellCoreTests/` -- `ScrubbingCaveat` is in the app target; if it has no test file, add the assertion to an existing app-target test rather than creating a target

**Interfaces:**
- Consumes: `QueueRow.onLookInside` (existing).
- Produces: `QueueRow.onSearch: () -> Void`, a new closure parameter.

- [ ] **Step 1: Write the failing test**

Add to `macos/Tests/TraceCommonsAppTests/` (a new
`ScrubbingCaveatTests.swift`):

```swift
import XCTest

@testable import TraceCommonsApp

/// The gold line is a judgement -- "nothing matched" is the case to slow
/// down on -- but it was a judgement with nothing to do about it.
final class ScrubbingCaveatTests: XCTestCase {
    func testTheNothingMatchedLineOffersANextStep() {
        let line = ScrubbingCaveat.rowLine(redactionCount: 0)
        XCTAssertTrue(
            line.lowercased().contains("search"),
            "the line must point at the thing to do about it: \(line)"
        )
    }

    func testALineWithRedactionsIsUnchangedInTone() {
        let line = ScrubbingCaveat.rowLine(redactionCount: 4)
        XCTAssertFalse(line.isEmpty)
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

```bash
cd macos && swift test --filter ScrubbingCaveatTests
```

Expected: FAIL on the first assertion.

- [ ] **Step 3: Add the clause and the control**

In `ScrubbingCaveat.swift`, extend the zero-redaction sentence:

```swift
            ? "Nothing matched a pattern. That is not the same as nothing being there -- search it for anything you need to be sure isn't in it."
```

In `QueueView.swift`, make the chip a button and give `QueueRow` the new
closure:

```swift
    private var nothingMatchedChip: some View {
        Button(action: onSearch) {
            HStack(spacing: TC.Space.xxs) {
                QueueGlyph(glyph: .triangle, size: 11, stroke: 1.6, color: TC.gold)
                Text(RedactionTally.nothingMatched)
                    .font(TC.Font_.monoChip)
                    .foregroundStyle(TC.goldText)
            }
            .padding(.horizontal, TC.Space.s)
            .padding(.vertical, 3)
            .overlay {
                Capsule().strokeBorder(
                    TC.gold.opacity(TC.Border.chipAlpha),
                    lineWidth: TC.Border.hairline
                )
            }
            .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .help("Opens this session's search, which is the thing to do about it.")
        .accessibilityLabel("Nothing matched a pattern. Search this session.")
    }
```

Add `let onSearch: () -> Void` to `QueueRow`, thread it through
`ProjectQueueGroup.rows`, and have `QueueContent` set it to open the preview
on its search tab. If `PreviewSheet` has no way to select a tab on open, add
an `initialTab` parameter to it defaulting to today's first tab.

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cd macos && swift build && swift test
```

Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add macos/Sources/TraceCommonsApp/Views/QueueView.swift \
        macos/Sources/TraceCommonsApp/Views/ScrubbingCaveat.swift \
        macos/Sources/TraceCommonsApp/Views/PreviewSheet.swift \
        macos/Tests/TraceCommonsAppTests/ScrubbingCaveatTests.swift
git commit -m "Give the nothing-matched chip something to do"
```

---

### Task 10: A shield beside the count

**Files:**
- Create: `macos/Sources/TCShellCore/QueueShieldState.swift`
- Modify: `macos/Sources/TraceCommonsApp/Views/MainWindowView.swift:159-193` (`navRow`)
- Test: `macos/Tests/TCShellCoreTests/QueueShieldStateTests.swift` (create)

**Interfaces:**
- Consumes: `PreviewSummary.redactions`, `QueueEntry.subagentsDropped`.
- Produces: `public enum QueueShieldState { case clear, waiting, attention }` and
  `public static func state(waiting: Int, nothingMatched: Int, trimmed: Int) -> QueueShieldState`

- [ ] **Step 1: Write the failing tests**

Create `macos/Tests/TCShellCoreTests/QueueShieldStateTests.swift`:

```swift
import XCTest

@testable import TCShellCore

/// The nav item's shield. It adds a state the bare count could never carry;
/// it does NOT replace the count -- at 149 waiting sessions, "how many" is
/// the signal a person is actually using.
final class QueueShieldStateTests: XCTestCase {
    func testAnEmptyQueueIsClear() {
        XCTAssertEqual(
            QueueShieldState.state(waiting: 0, nothingMatched: 0, trimmed: 0),
            .clear
        )
    }

    func testAnOrdinaryQueueIsWaiting() {
        XCTAssertEqual(
            QueueShieldState.state(waiting: 12, nothingMatched: 0, trimmed: 0),
            .waiting
        )
    }

    func testASessionWhereNothingMatchedRaisesAttention() {
        XCTAssertEqual(
            QueueShieldState.state(waiting: 12, nothingMatched: 1, trimmed: 0),
            .attention
        )
    }

    func testATrimmedSessionRaisesAttention() {
        XCTAssertEqual(
            QueueShieldState.state(waiting: 12, nothingMatched: 0, trimmed: 1),
            .attention
        )
    }

    func testAnEmptyQueueIsClearEvenWithStaleFlags() {
        // Nothing is waiting, so there is nothing to be attentive about.
        XCTAssertEqual(
            QueueShieldState.state(waiting: 0, nothingMatched: 3, trimmed: 2),
            .clear
        )
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

```bash
cd macos && swift test --filter QueueShieldStateTests
```

Expected: `cannot find 'QueueShieldState' in scope`.

- [ ] **Step 3: Write the implementation**

Create `macos/Sources/TCShellCore/QueueShieldState.swift`:

```swift
import Foundation

/// What the queue's nav item says about the queue, beyond how much is in it.
///
/// This is deliberately NOT a replacement for the numeric badge. The ask was
/// to swap the count for an icon; the count is the signal a contributor with
/// 149 waiting sessions is actually reading, and an icon meaning "some" is a
/// downgrade exactly at the scale that prompted the request. The shield adds
/// a state the count cannot carry. The two go together.
public enum QueueShieldState: Equatable {
    /// Nothing waiting.
    case clear
    /// Sessions waiting, none of them flagged.
    case waiting
    /// Something in the queue is worth a second look: a session where no
    /// pattern fired, or one trimmed to fit the byte budget.
    case attention

    public static func state(waiting: Int, nothingMatched: Int, trimmed: Int) -> QueueShieldState {
        guard waiting > 0 else { return .clear }
        return (nothingMatched > 0 || trimmed > 0) ? .attention : .waiting
    }
}
```

- [ ] **Step 4: Wire it into the nav row**

In `AppModel`, publish the counts the state needs, derived where
`awaitingDecision` is published:

```swift
    /// Sessions waiting whose preview reported that no pattern fired.
    /// Drives `QueueShieldState` only.
    @Published private(set) var nothingMatchedCount: Int = 0
```

Set it beside `publishIfChanged(\.awaitingDecision, waiting)`, counting
entries whose `summaries[entryID]` exists and has empty `redactions`.

In `MainWindowView.navRow`, replace the fixed glyph for `.queue` with the
shield and keep the count exactly as it is:

```swift
        let shield = item == .queue
            ? QueueShieldState.state(
                waiting: model.decisionsOwed,
                nothingMatched: model.nothingMatchedCount,
                trimmed: model.awaitingDecision.filter(\.wasTrimmed).count
            )
            : .clear
```

with the glyph color chosen from `shield` (`TC.inkSecondary` for `.clear`,
`TC.greenText` for `.waiting`, `TC.goldText` for `.attention`) and the
accessibility label extended for `.attention`.

- [ ] **Step 5: Run the tests**

```bash
cd macos && swift build && swift test
```

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add macos/Sources/TCShellCore/QueueShieldState.swift \
        macos/Tests/TCShellCoreTests/QueueShieldStateTests.swift \
        macos/Sources/TraceCommonsApp/Views/MainWindowView.swift \
        macos/Sources/TraceCommonsApp/AppModel.swift
git commit -m "Show queue state as a shield beside the waiting count"
```

---

### Task 11: History, grouped by folder

**Files:**
- Create: `macos/Sources/TCShellCore/HistoryFolders.swift`
- Modify: `macos/Sources/TraceCommonsApp/Views/HistoryView.swift:46-57`
- Test: `macos/Tests/TCShellCoreTests/HistoryFoldersTests.swift` (create)

**Interfaces:**
- Consumes: `HistoryRecord.projectID` (Task 1); `ProjectRow` (existing,
  whose id field is `projectId`, for
  the path); `QueueGrouping` (existing).
- Produces: `public static func folders<R>(_ records: [R], projectID: (R) -> String, projectLabel: (R) -> String, path: (String) -> String?) -> [QueueGroup<R>]`

- [ ] **Step 1: Write the failing tests**

Create `macos/Tests/TCShellCoreTests/HistoryFoldersTests.swift`:

```swift
import XCTest

@testable import TCShellCore

/// History groups by folder the way the queue does. The interesting part is
/// what happens to records that predate the change: their project id is
/// empty, they cannot be resolved to a folder, and they must not all be
/// swept into one bogus group.
final class HistoryFoldersTests: XCTestCase {
    private struct Record: Equatable {
        let id: String
        let projectID: String
        let label: String
    }

    private func folders(_ records: [Record]) -> [QueueGroup<Record>] {
        HistoryFolders.folders(
            records,
            projectID: \.projectID,
            projectLabel: \.label
        )
    }

    func testRecordsGroupByProjectID() {
        let groups = folders([
            Record(id: "1", projectID: "proj_a", label: "api"),
            Record(id: "2", projectID: "proj_b", label: "web"),
            Record(id: "3", projectID: "proj_a", label: "api"),
        ])
        XCTAssertEqual(groups.map(\.id), ["proj_a", "proj_b"])
        XCTAssertEqual(groups[0].count, 2)
    }

    func testTwoProjectsSharingALabelStaySeparate() {
        let groups = folders([
            Record(id: "1", projectID: "proj_a", label: "api"),
            Record(id: "2", projectID: "proj_b", label: "api"),
        ])
        XCTAssertEqual(groups.count, 2, "a label is not an identity")
    }

    func testRecordsWithNoProjectIDGroupByLabelInstead() {
        // Pre-upgrade records carry no id. Grouping them all under "" would
        // put two different repositories in one row.
        let groups = folders([
            Record(id: "1", projectID: "", label: "api"),
            Record(id: "2", projectID: "", label: "web"),
            Record(id: "3", projectID: "", label: "api"),
        ])
        XCTAssertEqual(groups.count, 2)
        XCTAssertEqual(groups.first(where: { $0.label == "api" })?.count, 2)
    }

    func testAnIdentifiedAndAnUnidentifiedRecordDoNotMerge() {
        // Same label, but one is resolvable and one is not. Claiming they
        // are the same folder is a guess, and the honest answer is two rows.
        let groups = folders([
            Record(id: "1", projectID: "proj_a", label: "api"),
            Record(id: "2", projectID: "", label: "api"),
        ])
        XCTAssertEqual(groups.count, 2)
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

```bash
cd macos && swift test --filter HistoryFoldersTests
```

Expected: `cannot find 'HistoryFolders' in scope`.

- [ ] **Step 3: Write the implementation**

Create `macos/Sources/TCShellCore/HistoryFolders.swift`:

```swift
import Foundation

/// Grouping history records into folders, on the same `QueueGrouping` the
/// queue uses so the two screens navigate identically.
///
/// The wrinkle is records that predate the daemon carrying `project_id`,
/// and records submitted before project keys were normalized -- both arrive
/// with an empty id. Grouping on the empty string would sweep every one of
/// them into a single row spanning unrelated repositories.
///
/// So an unidentified record falls back to grouping by label, under a
/// synthetic key that cannot collide with a real `proj_` id. That merges two
/// same-named repositories, which is a real loss -- but it is the loss that
/// was already there before this screen grouped at all, and it is smaller
/// than merging everything. An unidentified record is never merged with an
/// identified one: same label or not, claiming they are the same folder is a
/// guess.
public enum HistoryFolders {
    /// The prefix for a group keyed by label because its records carry no
    /// project id. `project_id_for` always emits `proj_`, so these two key
    /// spaces cannot collide.
    static let unresolvedPrefix = "label:"

    public static func folders<Record>(
        _ records: [Record],
        projectID: (Record) -> String,
        projectLabel: (Record) -> String
    ) -> [QueueGroup<Record>] {
        QueueGrouping.groups(
            records,
            projectID: { record in
                let id = projectID(record)
                return id.isEmpty ? unresolvedPrefix + projectLabel(record) : id
            },
            projectLabel: projectLabel,
            sizeBytes: { _ in 0 }
        )
    }
}
```

- [ ] **Step 4: Group the view**

In `HistoryView.swift`, replace the flat `ForEach(model.history)` with the
same two-level shape as the queue: a `@State private var location:
QueueLocation = .root`, a folder list built from `HistoryFolders.folders`,
and a detail view listing that folder's `HistoryRow`s. Resolve the location
through `QueueNavigation.resolve` on every redraw, exactly as Task 4 does.

Show each folder's path by matching the group id against `model.projects`
(`ProjectRow` carries `projectId` -- lowercase `d`, unlike this plan's new
`HistoryRecord.projectID` -- and now `projectPath`); a group whose id
starts with `HistoryFolders.unresolvedPrefix`, or that matches no known
project, renders its label alone.

- [ ] **Step 5: Run the tests**

```bash
cd macos && swift build && swift test
```

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add macos/Sources/TCShellCore/HistoryFolders.swift \
        macos/Tests/TCShellCoreTests/HistoryFoldersTests.swift \
        macos/Sources/TraceCommonsApp/Views/HistoryView.swift
git commit -m "Group history by folder, the way the queue is grouped"
```

---

### Task 12: Full verification

- [ ] **Step 1: Build the dylib the Swift package links**

```bash
cargo build -p trace-commons-contributor-ffi
```

- [ ] **Step 2: Build and test, the way CI does**

```bash
cd macos && swift build && swift test
```

Expected: all pass. Paste the summary line into the PR body.

- [ ] **Step 3: Run the app and check what tests cannot see**

Launch the app against a daemon with a populated queue and confirm, by hand:

1. The queue opens on a folder list; folder names are the largest text.
2. Clicking a folder opens its sessions; the back control returns.
3. `Submit all` on a one-session folder works without opening it.
4. Approving a folder's last session returns you to the folder list rather
   than leaving an empty detail view.
5. Clicking a card body opens the preview; the three footer buttons still do
   their own jobs.
6. Redactions are marked in the transcript, and the scrubbing tab lists
   what was removed with a description per category.
7. Typing `xyz` in search leaves one recent entry, not three.
8. Searching for a value you know was redacted says it was removed.
9. The nothing-matched chip opens search.
10. History is grouped by folder.
11. On a session with a `residual_secret_at` count, the panel reports it
    under "still in what would be sent" and NOT as removed.

Record the results in the PR body. `swift test` sees none of this.

- [ ] **Step 4: Open the PR**

```bash
git push -u origin shell-ux-macos
gh pr create --repo TraceCommons/trace-commons \
  --title "Folder-first queue and scrubber transparency, macOS" \
  --body "Implements docs/superpowers/plans/2026-09-03-contributor-shell-ux-macos.md.

Depends on the daemon and FFI foundation PR.

Spec: docs/superpowers/specs/2026-09-03-contributor-shell-queue-ux-design.md"
```

---

## Self-Review

**Spec coverage.**

| Spec section | Task |
|---|---|
| §2.1 folder list, name prominence | Task 4 |
| §2.2 folder detail, `session_path` | Task 4 |
| §2.3 `Submit all` at n = 1 | Task 4 (`QueueFolderRow`) |
| §2.4 card click | Task 5 |
| §3.1 chips named (already marked) | Task 6 |
| §3.1b removed-summary panel | Task 6b |
| §3.1b `residual_secret_at` excluded from the card figure | Task 2 |
| §3.1 distinct counts on the card | Task 2 |
| §3.2 original search | Task 8 |
| §3.3 recent-search prefixes | Task 7 |
| §3.4 nothing-matched affordance | Task 9 |
| §4 shield plus count | Task 10 |
| §5 history grouping | Task 11 |
| §1.1 `project_path` consumed by the shell | Task 1 (`QueueEntry`, `ProjectRow`) |

**Placeholder scan:** no TBDs. Three steps say "match the file's existing
name for X" (Task 8 Step 4's daemon call, Task 9 Step 3's `PreviewSheet` tab
parameter, Task 11 Step 4's `ProjectRow` fields) -- each names exactly what
to look for and what to do with it, and quoting the surrounding code would
go stale. Task 4 Step 3's `QueueGlyphs.chevronLeft` is named outright,
because it does not exist yet in either glyph enum.

**Type consistency check.** `RedactionTally.line(occurrences:distinct:)` is
defined in Task 2 and called in Task 2 Step 4 and Task 9 Step 3
(`RedactionTally.nothingMatched`). `QueueLocation` / `QueueNavigation.resolve`
are defined in Task 3 and used in Tasks 4 and 11. `RedactionMarkerNames.of`
is defined and used in Task 6 only; the scan it names is the existing
`TranscriptMarkerScan.spans(in:)`. `RecentSearches.remember` keeps its
existing signature (Task 7). `OriginalSearchOutcome.classify(remaining:original:)`
is defined in Task 8 Step 3 and called in Task 8 Step 5.
`QueueShieldState.state(waiting:nothingMatched:trimmed:)` is defined and
called in Task 10. `HistoryFolders.folders(_:projectID:projectLabel:)` is
defined in Task 11 Step 3 and called in Task 11 Steps 1 and 4 with matching
labels.

**One thing this plan cannot test, stated plainly.** Tasks 4, 5, 6, 9, 10
and 11 change SwiftUI view bodies, and `swift test` does not render views.
Task 12 Step 3's hand-check is the only thing standing behind those, which
is why it is a numbered list to be reported rather than a "verify it works".
