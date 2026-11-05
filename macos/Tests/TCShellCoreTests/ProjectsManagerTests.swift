import XCTest

@testable import TCShellCore

/// Settings > Projects' summary and Manage sheet (owner design,
/// 2026-11-04): grouping by the daemon's disambiguating suffix, the mode
/// counts, Mixed, search and the mode filter, and the bulk rule that never
/// arms (an Automatic is a grant, made one project at a time).
final class ProjectsManagerTests: XCTestCase {
    private func row(
        _ id: String, _ label: String, _ mode: ProjectMode = .ask, pending: Int? = nil,
        bucket: Bool = false, offerable: [ProjectMode]? = nil
    ) -> ProjectRow {
        ProjectRow(projectId: id, projectLabel: label, mode: mode, isUnresolvedBucket: bucket,
                   pendingCount: pending, daemonOfferableModes: offerable)
    }

    // MARK: Base name

    func test_theSuffixIsTheDaemonsFourHexCharacters() {
        XCTAssertEqual(ProjectsManager.split("NEARPrivateChatIOS (57b3)")?.base, "NEARPrivateChatIOS")
        XCTAssertEqual(ProjectsManager.split("NEARPrivateChatIOS (57b3)")?.suffix, "57b3")
        XCTAssertEqual(ProjectsManager.split("my app (0a1f)")?.base, "my app")
        // Anything else is part of the name.
        XCTAssertNil(ProjectsManager.split("api"))
        XCTAssertNil(ProjectsManager.split("api (v2)"))
        XCTAssertNil(ProjectsManager.split("api (57B3)"), "the daemon writes lowercase hex")
        XCTAssertNil(ProjectsManager.split("api (57b)"))
        XCTAssertNil(ProjectsManager.split("api (57b30)"))
        XCTAssertNil(ProjectsManager.split("api(57b3)"))
        XCTAssertNil(ProjectsManager.split(" (57b3)"), "no name before the suffix")
    }

    // MARK: Grouping

    func test_rowsSharingABaseNameGroupInFirstSeenOrder() {
        let rows = [
            row("a", "NEARPrivateChatIOS (57b3)"),
            row("b", "memories"),
            row("c", "NEARPrivateChatIOS (6b4e)", .ignore),
            row("d", "trace-commons-server (1111)"),
            row("e", "trace-commons-server (2222)"),
        ]
        let entries = ProjectsManager.entries(rows)
        XCTAssertEqual(entries.count, 3)
        guard case .group(let chat) = entries[0] else { return XCTFail("first entry is the chat group") }
        XCTAssertEqual(chat.name, "NEARPrivateChatIOS")
        XCTAssertEqual(chat.members.map(\.id), ["a", "c"])
        guard case .single(let memories) = entries[1] else { return XCTFail("memories stands alone") }
        XCTAssertEqual(memories.id, "b")
        guard case .group(let server) = entries[2] else { return XCTFail("server group") }
        XCTAssertEqual(server.members.map(\.id), ["d", "e"])
    }

    /// The daemon suffixes labels whose basenames collide case-folded, so
    /// the group does too.
    func test_groupingFoldsCaseAsTheDaemonDoes() {
        let entries = ProjectsManager.entries([row("a", "Api (1234)"), row("b", "api (abcd)")])
        guard case .group(let group) = entries.first else { return XCTFail("one group") }
        XCTAssertEqual(entries.count, 1)
        XCTAssertEqual(group.name, "Api", "named by its first member")
    }

    /// One suffixed row alone is not a group: it keeps its whole label.
    func test_aLoneSuffixedRowIsNotAGroup() {
        let entries = ProjectsManager.entries([row("a", "api (1234)"), row("b", "web")])
        XCTAssertEqual(entries.count, 2)
        guard case .single(let only) = entries[0] else { return XCTFail("single") }
        XCTAssertEqual(only.id, "a")
    }

    /// The unresolved bucket never carries a suffix, so it never groups.
    func test_theUnresolvedBucketNeverGroups() {
        let entries = ProjectsManager.entries([
            row("u", "unknown-project", bucket: true), row("a", "x (1234)"), row("b", "x (5678)"),
        ])
        XCTAssertEqual(entries.count, 2)
        guard case .single(let bucket) = entries[0] else { return XCTFail("bucket alone") }
        XCTAssertTrue(bucket.isUnresolvedBucket)
    }

    /// A member row shows its suffix under the group header; a lone row its
    /// whole label.
    func test_memberLabelIsTheSuffix() {
        XCTAssertEqual(ProjectsManager.memberLabel(row("a", "api (1234)")), "1234")
        XCTAssertEqual(ProjectsManager.memberLabel(row("a", "api")), "api")
    }

    // MARK: Mixed

    func test_aGroupsModeIsItsMembersOnlyWhenTheyAgree() {
        let same = ProjectGroup(id: "x", name: "x", members: [row("a", "x (1111)", .ignore), row("b", "x (2222)", .ignore)])
        XCTAssertEqual(same.mode, .ignore)
        let mixed = ProjectGroup(id: "x", name: "x", members: [row("a", "x (1111)", .ask), row("b", "x (2222)", .ignore)])
        XCTAssertNil(mixed.mode, "Mixed")
        let empty = ProjectGroup(id: "x", name: "x", members: [])
        XCTAssertNil(empty.mode)
    }

    func test_aGroupsPendingIsItsMembersSum() {
        let group = ProjectGroup(id: "x", name: "x", members: [
            row("a", "x (1111)", pending: 2), row("b", "x (2222)"), row("c", "x (3333)", pending: 5),
        ])
        XCTAssertEqual(group.pending, 7)
    }

    // MARK: Counts

    func test_countsAreEveryRowByTheModeInForce() {
        let counts = ProjectsManager.counts([
            row("a", "a", .autoUpload), row("b", "b", .ask), row("c", "c", .ask),
            row("d", "d", .ignore), row("e", "e", .ignore), row("f", "f", .ignore),
        ])
        XCTAssertEqual(counts, ProjectModeCounts(automatic: 1, ask: 2, never: 3))
        XCTAssertEqual(counts.total, 6)
        XCTAssertEqual(counts.count(for: .autoUpload), 1)
        XCTAssertEqual(counts.count(for: .ask), 2)
        XCTAssertEqual(counts.count(for: .ignore), 3)
        XCTAssertEqual(ProjectsManager.counts([]).total, 0)
    }

    // MARK: Search and filter

    func test_searchMatchesTheShownNameIgnoringCaseAndSpaces() {
        let rows = [row("a", "NEARPrivateChatIOS (57b3)"), row("b", "memories"), row("c", "bouncy-fiction")]
        XCTAssertEqual(ProjectsManager.filter(rows, query: "chat", mode: nil).map(\.id), ["a"])
        XCTAssertEqual(ProjectsManager.filter(rows, query: "  MEM ", mode: nil).map(\.id), ["b"])
        XCTAssertEqual(ProjectsManager.filter(rows, query: "", mode: nil).map(\.id), ["a", "b", "c"])
        XCTAssertEqual(ProjectsManager.filter(rows, query: "57b3", mode: nil).map(\.id), ["a"])
        XCTAssertEqual(ProjectsManager.filter(rows, query: "zzz", mode: nil), [])
    }

    func test_theModeFilterKeepsThatModeOnly() {
        let rows = [row("a", "a", .autoUpload), row("b", "b", .ask), row("c", "c", .ignore), row("d", "d", .ask)]
        XCTAssertEqual(ProjectsManager.filter(rows, query: "", mode: .ask).map(\.id), ["b", "d"])
        XCTAssertEqual(ProjectsManager.filter(rows, query: "d", mode: .ask).map(\.id), ["d"])
        XCTAssertEqual(ProjectsManager.filter(rows, query: "", mode: .autoUpload).map(\.id), ["a"])
    }

    /// The bucket is found by the name it is shown under, not its slug.
    func test_theBucketIsSearchedByItsShownName() {
        let bucket = row("u", "unknown-project", bucket: true)
        XCTAssertEqual(ProjectsManager.filter([bucket], query: "unknown-project", mode: nil), [])
        let word = String(ProjectCopy.unresolvedBucketLabel.split(separator: " ").last ?? "")
        XCTAssertFalse(word.isEmpty)
        XCTAssertEqual(ProjectsManager.filter([bucket], query: word, mode: nil), [bucket])
    }

    /// Filtering runs before grouping: a group holds only the members that
    /// match, and one surviving member stands alone.
    func test_filteringRunsBeforeGrouping() {
        let rows = [row("a", "x (1111)", .ask), row("b", "x (2222)", .ignore), row("c", "x (3333)", .ignore)]
        let shown = ProjectsManager.entries(ProjectsManager.filter(rows, query: "", mode: .ignore))
        guard case .group(let group) = shown.first else { return XCTFail("group of two") }
        XCTAssertEqual(group.members.map(\.id), ["b", "c"])
        XCTAssertEqual(group.mode, .ignore)
        let one = ProjectsManager.entries(ProjectsManager.filter(rows, query: "", mode: .ask))
        guard case .single = one.first else { return XCTFail("one member stands alone") }
    }

    // MARK: Lines

    func test_collapsedGroupsShowTheirHeaderOnly() {
        let entries = ProjectsManager.entries([row("a", "x (1111)"), row("b", "x (2222)"), row("c", "y")])
        let collapsed = ProjectsManager.lines(entries, expanded: [], expandAll: false)
        XCTAssertEqual(collapsed.map(\.id), ["group:x", "row:c"])
        let open = ProjectsManager.lines(entries, expanded: ["x"], expandAll: false)
        XCTAssertEqual(open.map(\.id), ["group:x", "row:a", "row:b", "row:c"])
        // A search opens every group, so a match is never hidden.
        let searching = ProjectsManager.lines(entries, expanded: [], expandAll: true)
        XCTAssertEqual(searching.map(\.id), ["group:x", "row:a", "row:b", "row:c"])
    }

    // MARK: Selection

    func test_selectionCountsOnlyWhatIsShown() {
        let shown = [row("a", "a"), row("b", "b")]
        XCTAssertEqual(ProjectsManager.selected(["a", "z"], in: shown).map(\.id), ["a"])
        XCTAssertEqual(ProjectsManager.selected([], in: shown), [])
    }

    // MARK: Bulk

    /// Automatic is a grant: bulk and group menus never offer it.
    func test_bulkOffersNeverAndAskOnly() {
        XCTAssertEqual(ProjectsManager.bulkModes, [.ask, .ignore])
        XCTAssertFalse(ProjectsManager.bulkModes.contains(.autoUpload))
    }

    func test_bulkAutomaticIsRefused() {
        let rows = [row("a", "a", .ask), row("b", "b", .ignore)]
        XCTAssertEqual(ProjectsManager.plan(rows, to: .autoUpload), .refused)
    }

    func test_bulkSkipsRowsAlreadyThereAndRowsThatCannotTakeTheMode() {
        let rows = [
            row("a", "a", .ask), row("b", "b", .ignore), row("c", "c", .autoUpload),
            row("d", "d", .ask, offerable: [.ask]),
        ]
        XCTAssertEqual(ProjectsManager.plan(rows, to: .ignore), .apply(["a", "c"]))
        XCTAssertEqual(ProjectsManager.plan(rows, to: .ask), .apply(["b", "c"]))
        XCTAssertEqual(ProjectsManager.plan([row("a", "a", .ask)], to: .ask), .nothing)
        XCTAssertEqual(ProjectsManager.plan([], to: .ignore), .nothing)
    }

    /// Never clears what is waiting, so a bulk Never with traces waiting
    /// asks first, with their total -- as one row's Never does.
    func test_bulkNeverWithTracesWaitingAsksFirst() {
        let rows = [row("a", "a", .ask, pending: 2), row("b", "b", .ask), row("c", "c", .ignore, pending: 9)]
        XCTAssertEqual(ProjectsManager.plan(rows, to: .ignore), .confirmNever(["a", "b"], pending: 2))
        XCTAssertEqual(ProjectsManager.plan(rows, to: .ask), .apply(["c"]), "Ask clears nothing")
    }

    func test_planTargetsKeepTheirOrder() {
        let rows = [row("z", "z", .ask), row("a", "a", .ask)]
        XCTAssertEqual(ProjectsManager.plan(rows, to: .ignore), .apply(["z", "a"]))
    }
}
