import SwiftUI
import TCDesign
import TCShellCore

enum SkillsSection: String, CaseIterable, Hashable {
    case inventory, usage, catalog, following

    var copyKey: String { "skills_tab_" + rawValue }
}

/// Skills are observed and followed here; publisher instructions own setup.
struct InsightsSkillsTab: View {
    let model: InsightsSkillsModel
    @State private var catalogQuery = ""
    @State private var inventoryQuery = ""
    @State private var selectedSection: SkillsSection
    @Environment(\.scenePhase) private var scenePhase

    init(model: InsightsSkillsModel, initialSection: SkillsSection = .inventory) {
        self.model = model
        _selectedSection = State(initialValue: initialSection)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: InsightsLayout.headingGap) {
            HStack(spacing: GlassTokens.Space.s4) {
                Text(model.text("skills_title")).insightsHeading()
                Spacer(minLength: 0)
                if model.busy || model.shown?.refreshing == true {
                    GlassSpinner().accessibilityLabel(model.text("skills_loading"))
                }
                Button(model.text("skills_refresh")) { Task { await model.load(refresh: true) } }
                    .accessibilityLabel(model.text("skills_refresh_accessibility"))
                    .disabled(model.busy)
            }
            GlassSegmentedTabs(model.text("skills_title"), selection: $selectedSection,
                               segments: SkillsSection.allCases.map {
                GlassSegment(model.text($0.copyKey), value: $0)
            })
            if let failure = model.failureKey { Text(model.text(failure)).insightsError() }
            InsightsTabPage(getStarted: nil) {
                if let snapshot = model.shown {
                    if snapshot.enabled {
                        switch selectedSection {
                        case .inventory:
                            inventory(snapshot)
                        case .usage:
                            usage(snapshot.usage)
                        case .catalog:
                            publishers(snapshot)
                            catalog(snapshot)
                        case .following:
                            feed(snapshot)
                        }
                    } else {
                        Text(model.text("skills_disabled")).insightsNote()
                    }
                }
            }
            .id(selectedSection)
            .accessibilityIdentifier("insights-skills-" + selectedSection.rawValue)
        }
        .padding(.top, GlassTokens.Space.s7)
        .insightsSurface()
        .task(id: scenePhase) {
            guard scenePhase == .active else { return }
            await model.observe()
        }
    }

    private func section<Content: View>(_ key: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: InsightsLayout.headingGap) {
            Text(model.text(key)).insightsHeading()
            content().insightsCardBody()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .insightsSection()
    }

    private func inventory(_ snapshot: DaemonData.SkillsSnapshot) -> some View {
        let skills = model.inventorySkills(in: snapshot, matching: inventoryQuery)
        return section("skills_inventory") {
            LazyVStack(alignment: .leading, spacing: InsightsLayout.groupGap) {
                if snapshot.inventory.partial { Text(model.text("skills_partial_inventory")).insightsNote() }
                GlassTextField(model.text("skills_search"), text: $inventoryQuery,
                               prompt: model.text("skills_search"), showsLabel: false)
                    .accessibilityLabel(model.text("skills_search"))
                if skills.isEmpty {
                    Text(model.text(snapshot.inventory.skills.isEmpty ? "skills_inventory_empty" : "skills_search_empty"))
                        .insightsNote()
                }
                ForEach(skills) { skill in
                    VStack(alignment: .leading, spacing: InsightsLayout.lineGap) {
                        Text(skill.name).insightsHeading()
                        if !skill.description.isEmpty { Text(skill.description) }
                        Text([model.agentLabel(skill.agent), model.scopeLabel(skill.scope)].joined(separator: " · ")).insightsCaption()
                        Text(skill.location).insightsMono().textSelection(.enabled)
                        if let update = snapshot.updates.first(where: { $0.installedID == skill.id }) {
                            status(update.status)
                        }
                        if let matched = model.catalogMatch(for: skill, in: snapshot) {
                            instructions(matched.instructionsURL, name: skill.name)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .accessibilityElement(children: .contain)
                }
                ForEach(snapshot.inventory.coverage, id: \.location) { coverage in
                    VStack(alignment: .leading, spacing: InsightsLayout.lineGap) {
                        Text(coverage.location).insightsMono()
                        status(coverage.status)
                    }
                }
            }
        }
    }

    private func usage(_ usage: DaemonData.SkillsUsage) -> some View {
        section("skills_usage") {
            LazyVStack(alignment: .leading, spacing: InsightsLayout.groupGap) {
                Text(model.text("skills_sessions_observed", values: ["count": String(usage.sessionsObserved)]))
                    .insightsNote()
                if usage.sessionsUnknown > 0 {
                    Text(model.text("skills_sessions_unknown", values: ["count": String(usage.sessionsUnknown)]))
                        .insightsNote()
                }
                if usage.partial { Text(model.text("skills_partial_usage")).insightsNote() }
                ForEach(usage.coverage, id: \.agent) { coverage in
                    HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s3) {
                        Text(model.agentLabel(coverage.agent)).insightsCaption()
                        Text(model.text("skills_coverage_\(coverage.status)")).insightsNote()
                    }
                }
                if usage.skills.isEmpty { Text(model.text("skills_usage_empty")).insightsNote() }
                ForEach(usage.skills, id: \.identity) { row in
                    VStack(alignment: .leading, spacing: InsightsLayout.lineGap) {
                        HStack(alignment: .firstTextBaseline) {
                            Text(row.name).insightsHeading()
                            Text(model.agentLabel(row.agent)).insightsCaption()
                        }
                        Text(model.usageSummary(row, coverage: usage.coverage)).insightsNote()
                        if let date = row.lastSeen {
                            Text(model.text("skills_last_seen", values: ["date": model.timestamp(date)])).insightsCaption()
                        }
                    }
                    .accessibilityElement(children: .combine)
                }
            }
        }
    }

    private func publishers(_ snapshot: DaemonData.SkillsSnapshot) -> some View {
        section("skills_publishers") {
            LazyVStack(alignment: .leading, spacing: InsightsLayout.groupGap) {
                ForEach(snapshot.catalog.publishers) { publisher in
                    publisherRow(publisher, snapshot: snapshot)
                }
            }
        }
    }

    private func publisherRow(_ publisher: DaemonData.SkillsPublisher,
                              snapshot: DaemonData.SkillsSnapshot) -> some View {
        let followed = snapshot.followedPublishers.contains(publisher.id)
        return VStack(alignment: .leading, spacing: InsightsLayout.lineGap) {
            HStack(alignment: .firstTextBaseline) {
                Text(publisher.name).insightsHeading()
                Spacer(minLength: 0)
                Toggle(model.text(followed ? "skills_unfollow" : "skills_follow"), isOn: Binding(
                    get: { model.shown?.followedPublishers.contains(publisher.id) == true },
                    set: { value in Task { await model.setFollow(publisherID: publisher.id, followed: value) } }))
                    .accessibilityLabel(model.text(followed ? "skills_unfollow_accessibility" : "skills_follow_accessibility",
                                                   values: ["publisher": publisher.name]))
                    .disabled(!model.canFollow)
                    .fixedSize()
            }
            Text(publisher.repository).insightsMono()
            if let state = snapshot.catalog.publisherStates.first(where: { $0.publisherID == publisher.id }) {
                status(state.status)
                if let checked = state.checkedAt {
                    Text(model.text("skills_checked_at", values: ["date": model.timestamp(checked)])).insightsCaption()
                }
            }
            instructions(publisher.instructionsURL, name: publisher.name)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func catalog(_ snapshot: DaemonData.SkillsSnapshot) -> some View {
        let skills = model.catalogSkills(in: snapshot, matching: catalogQuery)
        return section("skills_catalog") {
            LazyVStack(alignment: .leading, spacing: InsightsLayout.groupGap) {
                if snapshot.catalog.partial { Text(model.text("skills_partial_catalog")).insightsNote() }
                if let checked = snapshot.catalog.checkedAt {
                    Text(model.text("skills_checked_at", values: ["date": model.timestamp(checked)])).insightsCaption()
                }
                GlassTextField(model.text("skills_search"), text: $catalogQuery,
                               prompt: model.text("skills_search"), showsLabel: false)
                    .accessibilityLabel(model.text("skills_search"))
                if skills.isEmpty {
                    Text(model.text(snapshot.catalog.skills.isEmpty ? "skills_catalog_empty" : "skills_search_empty"))
                        .insightsNote()
                }
                ForEach(skills) { skill in
                    VStack(alignment: .leading, spacing: InsightsLayout.lineGap) {
                        Text(skill.name).insightsHeading()
                        if !skill.description.isEmpty { Text(skill.description) }
                        Text(skill.repository + "/" + skill.path).insightsMono()
                        if let installed = snapshot.inventory.skills.first(where: {
                            model.catalogMatch(for: $0, in: snapshot)?.id == skill.id
                        }), let update = snapshot.updates.first(where: { $0.installedID == installed.id }) {
                            status(update.status)
                        }
                        instructions(skill.instructionsURL, name: skill.name)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }
    }

    private func feed(_ snapshot: DaemonData.SkillsSnapshot) -> some View {
        let events = snapshot.feed.filter { snapshot.followedPublishers.contains($0.publisherID) }
        return section("skills_feed") {
            LazyVStack(alignment: .leading, spacing: InsightsLayout.groupGap) {
                if snapshot.feedPartial { Text(model.text("skills_partial_feed")).insightsNote() }
                if events.isEmpty { Text(model.text("skills_feed_empty")).insightsNote() }
                ForEach(events) { event in
                    VStack(alignment: .leading, spacing: InsightsLayout.lineGap) {
                        let skill = snapshot.catalog.skills.first { $0.id == event.skillID }
                        let publisher = snapshot.catalog.publishers.first { $0.id == event.publisherID }
                        Text(skill?.name ?? event.skillID).insightsHeading()
                        if let publisher { Text(publisher.name).insightsCaption() }
                        Text(model.text("skills_feed_\(event.kind)")).insightsNote()
                        Text(model.timestamp(event.observedAt)).insightsCaption()
                        if let skill { instructions(skill.instructionsURL, name: skill.name) }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }
    }

    private func status(_ value: String) -> some View {
        Text(model.text("skills_status_\(value)")).insightsCaption()
    }

    @ViewBuilder private func instructions(_ string: String, name: String) -> some View {
        if let url = URL(string: string), url.scheme == "https", url.host != nil {
            Link(model.text("skills_instructions"), destination: url)
                .buttonStyle(GlassButtonStyle(.link))
                .accessibilityLabel(model.text("skills_instructions_accessibility", values: ["name": name]))
        }
    }
}

private extension DaemonData.SkillsUsageRow {
    var identity: String { agent + "\u{0}" + name }
}
