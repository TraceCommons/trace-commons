import XCTest

@testable import TCShellCore

/// Settings > Projects' summary and Manage sheet (owner design,
/// 2026-11-04; #1952 review): grouping by the daemon's collision group,
/// the mode counts, Mixed, search and the mode filter, and selection. The
/// bulk rule is the daemon's (`daemon::project_bulk`), tested there.
final class ProjectsManagerTests: XCTestCase {
    private func row(
        _ id: String, _ label: String, _ mode: ProjectMode = .ask, pending: Int? = nil,
        bucket: Bool = false, offerable: [ProjectMode]? = nil
    ) -> ProjectRow {
        ProjectRow(projectId: id, projectLabel: label, mode: mode, isUnresolvedBucket: bucket,
                   pendingCount: pending, daemonOfferableModes: offerable)
    }

    /// A row whose label the daemon suffixed, with the collision it sent.
    private func member(_ id: String, _ base: String, _ suffix: String, _ mode: ProjectMode = .ask) -> ProjectRow {
        ProjectRow(
            projectId: id, projectLabel: "\(base) (\(suffix))", mode: mode,
            collision: ProjectCollision(group: base.lowercased(), baseLabel: base, suffix: suffix))
    }

    // MARK: Grouping

    func test_rowsInOneCollisionGroupGroupInFirstSeenOrder() {
        let rows = [
            member("a", "NEARPrivateChatIOS", "57b3"),
            row("b", "memories"),
            member("c", "NEARPrivateChatIOS", "6b4e", .ignore),
            member("d", "trace-commons-server", "1111"),
            member("e", "trace-commons-server", "2222"),
        ]
        let entries = ProjectsManager.entries(rows)
        XCTAssertEqual(entries.count, 3)
        guard case .group(let chat) = entries[0] else { return XCTFail("first entry is the chat group") }
        XCTAssertEqual(chat.name, "NEARPrivateChatIOS")
        XCTAssertEqual(chat.id, "nearprivatechatios", "the daemon's group")
        XCTAssertEqual(chat.members.map(\.id), ["a", "c"])
        XCTAssertEqual(chat.memberIDs, ["a", "c"])
        guard case .single(let memories) = entries[1] else { return XCTFail("memories stands alone") }
        XCTAssertEqual(memories.id, "b")
        guard case .group(let server) = entries[2] else { return XCTFail("server group") }
        XCTAssertEqual(server.members.map(\.id), ["d", "e"])
    }

    /// The group is the daemon's, not the label's: two spellings it put in
    /// one group are one group, named by the first member.
    func test_groupingIsTheDaemonsCollisionGroup() {
        let entries = ProjectsManager.entries([member("a", "Api", "1234"), member("b", "api", "abcd")])
        guard case .group(let group) = entries.first else { return XCTFail("one group") }
        XCTAssertEqual(entries.count, 1)
        XCTAssertEqual(group.name, "Api", "named by its first member")
    }

    /// A label that merely looks suffixed, with no collision from the
    /// daemon, is a folder's own name and never groups (#1952 review: a
    /// folder literally named `notes (beef)`).
    func test_aSuffixShapedNameWithNoCollisionNeverGroups() {
        let entries = ProjectsManager.entries([row("a", "notes (beef)"), row("b", "notes (cafe)")])
        XCTAssertEqual(entries.count, 2)
        XCTAssertEqual(ProjectsManager.memberLabel(row("a", "notes (beef)")), "notes (beef)")
    }

    /// One collided row shown alone is not a group: it keeps its whole label.
    func test_aLoneSuffixedRowIsNotAGroup() {
        let entries = ProjectsManager.entries([member("a", "api", "1234"), row("b", "web")])
        XCTAssertEqual(entries.count, 2)
        guard case .single(let only) = entries[0] else { return XCTFail("single") }
        XCTAssertEqual(only.id, "a")
    }

    /// The unresolved bucket never groups.
    func test_theUnresolvedBucketNeverGroups() {
        let entries = ProjectsManager.entries([
            row("u", "unknown-project", bucket: true), member("a", "x", "1234"), member("b", "x", "5678"),
        ])
        XCTAssertEqual(entries.count, 2)
        guard case .single(let bucket) = entries[0] else { return XCTFail("bucket alone") }
        XCTAssertTrue(bucket.isUnresolvedBucket)
    }

    /// A member row shows the daemon's suffix under the group header; a
    /// lone row its whole label.
    func test_memberLabelIsTheSuffix() {
        XCTAssertEqual(ProjectsManager.memberLabel(member("a", "api", "1234")), "1234")
        XCTAssertEqual(ProjectsManager.memberLabel(row("a", "api")), "api")
    }

    /// `collision` decodes when the daemon sends it, and its absence (every
    /// unsuffixed row, an older daemon) is nil.
    func test_theCollisionDecodes() throws {
        let json = #"[{"project_id":"a","project_label":"api (1234)","mode":"notify_only","#
            + #""collision":{"group":"api","base_label":"Api","suffix":"1234"}},"#
            + #"{"project_id":"b","project_label":"web","mode":"ignore"}]"#
        let rows = try JSONDecoder().decode([ProjectRow].self, from: Data(json.utf8))
        XCTAssertEqual(rows[0].collision, ProjectCollision(group: "api", baseLabel: "Api", suffix: "1234"))
        XCTAssertNil(rows[1].collision)
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
        let rows = [member("a", "x", "1111", .ask), member("b", "x", "2222", .ignore), member("c", "x", "3333", .ignore)]
        let shown = ProjectsManager.entries(ProjectsManager.filter(rows, query: "", mode: .ignore))
        guard case .group(let group) = shown.first else { return XCTFail("group of two") }
        XCTAssertEqual(group.members.map(\.id), ["b", "c"])
        XCTAssertEqual(group.mode, .ignore)
        let one = ProjectsManager.entries(ProjectsManager.filter(rows, query: "", mode: .ask))
        guard case .single = one.first else { return XCTFail("one member stands alone") }
    }

    // MARK: Lines

    func test_collapsedGroupsShowTheirHeaderOnly() {
        let entries = ProjectsManager.entries([member("a", "x", "1111"), member("b", "x", "2222"), row("c", "y")])
        let collapsed = ProjectsManager.lines(entries, expanded: [], expandAll: false)
        XCTAssertEqual(collapsed.map(\.id), ["group:x", "row:c"])
        let open = ProjectsManager.lines(entries, expanded: ["x"], expandAll: false)
        XCTAssertEqual(open.map(\.id), ["group:x", "row:a", "row:b", "row:c"])
        // A search opens every group, so a match is never hidden.
        let searching = ProjectsManager.lines(entries, expanded: [], expandAll: true)
        XCTAssertEqual(searching.map(\.id), ["group:x", "row:a", "row:b", "row:c"])
    }

    // MARK: Selection

    /// The selection acted on is what is shown: the search and the filter
    /// hide a selection; a collapsed group does not.
    func test_selectionCountsOnlyWhatIsShown() {
        let shown = [row("a", "a"), row("b", "b")]
        XCTAssertEqual(ProjectsManager.selected(["a", "z"], in: shown).map(\.id), ["a"])
        XCTAssertEqual(ProjectsManager.selected([], in: shown), [])
        let grouped = [member("a", "x", "1111"), member("b", "x", "2222"), row("c", "y")]
        let collapsed = ProjectsManager.lines(ProjectsManager.entries(grouped), expanded: [], expandAll: false)
        XCTAssertEqual(collapsed.map(\.id), ["group:x", "row:c"])
        XCTAssertEqual(ProjectsManager.selected(["a", "c"], in: grouped).map(\.id), ["a", "c"],
                       "a member of a collapsed group is still selected, and its header shows it")
    }

    /// A checkbox over several rows reads none, some (mixed) or all.
    func test_aCheckboxOverRowsReadsNoneSomeOrAll() {
        XCTAssertEqual(ProjectsManager.selectionState([], of: ["a", "b"]), .none)
        XCTAssertEqual(ProjectsManager.selectionState(["a"], of: ["a", "b"]), .some)
        XCTAssertEqual(ProjectsManager.selectionState(["a", "b", "z"], of: ["a", "b"]), .all)
        XCTAssertEqual(ProjectsManager.selectionState(["z"], of: []), .none)
    }

    /// Setting it selects or clears those rows only, the same however many
    /// times it is written (a mixed box writes each of its sources).
    func test_aCheckboxSelectsOrClearsItsRowsOnly() {
        let on = ProjectsManager.selecting(["z", "a"], ids: ["a", "b"], on: true)
        XCTAssertEqual(on, ["z", "a", "b"])
        XCTAssertEqual(ProjectsManager.selecting(on, ids: ["a", "b"], on: true), on)
        let off = ProjectsManager.selecting(on, ids: ["a", "b"], on: false)
        XCTAssertEqual(off, ["z"])
        XCTAssertEqual(ProjectsManager.selecting(off, ids: ["a", "b"], on: false), off)
    }

    /// Only the projects a change reached leave the selection.
    func test_onlyWhatChangedLeavesTheSelection() {
        XCTAssertEqual(ProjectsManager.afterChange(["a", "b", "c"], changed: ["a"]), ["b", "c"])
    }

    /// The daemon's plan decodes whole, the confirmation only when it asks.
    func test_theBulkPlanDecodes() throws {
        let json = #"{"outcome":"confirm","project_ids":["a"],"turns_off_automatic":1,"pending":0,"#
            + #""confirmation":{"title":"Set a to Never?","body":"This turns off Automatic for 1 project.","#
            + #""button":"Never","keep":"Keep","destructive":true},"bulk_modes":["notify_only","ignore"]}"#
        let plan = try JSONDecoder().decode(ProjectsBulkPlan.self, from: Data(json.utf8))
        XCTAssertEqual(plan.outcome, .confirm)
        XCTAssertEqual(plan.bulkModes, [.ask, .ignore])
        XCTAssertNil(plan.confirmation?.help)
        let apply = #"{"outcome":"apply","project_ids":["a"],"turns_off_automatic":0,"pending":0,"bulk_modes":["notify_only","ignore"]}"#
        XCTAssertNil(try JSONDecoder().decode(ProjectsBulkPlan.self, from: Data(apply.utf8)).confirmation)
    }
}
