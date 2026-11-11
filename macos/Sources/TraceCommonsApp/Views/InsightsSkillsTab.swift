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
    let reviewSettings: (() -> Void)?
    @State private var evidenceOpen = false
    @State private var catalogQuery = ""
    @State private var inventoryQuery = ""
    @State private var inventoryAgent = ""
    @State private var inventoryAvailability = ""
    @State private var inventorySort = "name"
    @State private var usageDays = 30
    @State private var selectedSkillID: String?
    @State private var selectedSection: SkillsSection
    @Environment(\.scenePhase) private var scenePhase

    init(model: InsightsSkillsModel, reviewSettings: (() -> Void)? = nil, initialSection: SkillsSection = .inventory) {
        self.model = model
        self.reviewSettings = reviewSettings
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
            if evidenceOpen {
                HStack {
                    Text(model.text("skills_evidence_title")).insightsHeading()
                    Spacer()
                    Button(model.text("skills_close")) { evidenceOpen = false; model.invalidateDetails() }
                }
                if model.evidenceFailure { Text(model.text("skills_evidence_unavailable")).insightsNote() }
                else if let session = model.evidence?.session {
                    if let source = session.source { Text(model.agentLabel(source)) }
                    if let date = session.date { Text(date) }
                    if let turns = session.turns { Text(model.text("skills_evidence_turns", values: ["count": String(turns)])) }
                    if let tokens = session.tokens { Text(model.text("skills_evidence_tokens", values: ["count": String(tokens)])) }
                } else { Text(model.text("skills_loading")).insightsNote() }
            }
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
        .onDisappear { model.invalidateDetails() }
        .onChange(of: selectedSkillID) { _, _ in model.invalidateDetails() }
        .onChange(of: selectedSection) { _, _ in model.invalidateDetails() }
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
        let skills = model.inventorySkills(in: snapshot, matching: inventoryQuery, agent: inventoryAgent,
                                          availability: inventoryAvailability, sort: inventorySort, days: usageDays)
        return section("skills_inventory") {
            LazyVStack(alignment: .leading, spacing: InsightsLayout.groupGap) {
                GlassTextField(model.text("skills_search"), text: $inventoryQuery,
                               prompt: model.text("skills_search"), showsLabel: false)
                    .accessibilityLabel(model.text("skills_search"))
                HStack {
                    Picker(model.text("skills_filter_agent"), selection: $inventoryAgent) {
                        Text(model.text("skills_all_agents")).tag("")
                        ForEach(Array(Set(snapshot.inventory.skills.map(\.agent))).sorted(), id: \.self) {
                            Text(model.agentLabel($0)).tag($0)
                        }
                    }
                    Picker(model.text("skills_filter_availability"), selection: $inventoryAvailability) {
                        Text(model.text("skills_all_availability")).tag("")
                        ForEach(["available", "disabled", "cached", "missing", "unknown"], id: \.self) {
                            Text(model.text("skills_availability_" + $0)).tag($0)
                        }
                    }
                }
                HStack {
                    Picker(model.text("skills_window"), selection: $usageDays) {
                        ForEach([7, 30, 90], id: \.self) { Text(model.text("skills_days", values: ["count": String($0)])).tag($0) }
                    }
                    Picker(model.text("skills_sort"), selection: $inventorySort) {
                        ForEach(["name", "sessions", "recent"], id: \.self) { Text(model.text("skills_sort_" + $0)).tag($0) }
                    }
                }
                if skills.isEmpty {
                    Text(model.text(snapshot.refreshing ? "skills_loading" : snapshot.inventory.skills.isEmpty ? "skills_inventory_empty" : "skills_search_empty")).insightsNote()
                }
                if !snapshot.usage.enabled {
                    Text(model.text("skills_analysis_off")).insightsNote()
                    if let reviewSettings { Button(model.text("skills_review_settings")) { model.invalidateDetails(); reviewSettings() } }
                }
                ForEach(skills) { skill in
                    VStack(alignment: .leading, spacing: InsightsLayout.lineGap) {
                        Button { selectedSkillID = selectedSkillID == skill.id ? nil : skill.id } label: {
                            HStack {
                                Text(skill.name).insightsHeading()
                                Spacer()
                                Image(systemName: selectedSkillID == skill.id ? "chevron.up" : "chevron.down")
                            }
                        }.buttonStyle(.plain)
                        .accessibilityLabel(model.text("skills_inspect", values: ["name": skill.name]))
                        if !skill.description.isEmpty { Text(skill.description) }
                        Text(model.agentLabel(skill.agent)).insightsCaption()
                        Text(model.text("skills_availability_" + (snapshot.inventory.details[skill.id]?.availability ?? "unknown"))).insightsCaption()
                        if let definitionStatus = snapshot.inventory.details[skill.id]?.definitionStatus,
                           ["invalid", "unreadable"].contains(definitionStatus) {
                            Text(model.text("skills_definition_status_" + definitionStatus)).insightsNote()
                        }
                        let usageRows = model.usageRows(for: skill, in: snapshot).filter { $0.windows.contains { $0.days == usageDays } }
                        if !usageRows.isEmpty && snapshot.usage.enabled {
                          ForEach(usageRows, id: \.agent) { row in
                           if let window = row.windows.first(where: { $0.days == usageDays }) {
                            Text(model.agentLabel(row.agent)).insightsCaption()
                            Text(model.text("skills_coverage_name_only")).insightsCaption()
                            Text(model.text("skills_sessions", values: ["count": String(window.sessions)]))
                            Text(model.text("skills_invocations", values: ["count": String(window.invocations)]))
                            if let date = row.lastSeen { Text(model.text("skills_last_seen", values: ["date": model.timestamp(date)])).insightsCaption() }
                            if selectedSkillID == skill.id {
                                Text(model.text("skills_loads", values: ["count": String(window.loads)]))
                                Text(model.text("skills_undated", values: ["count": String(row.undatedEvents)])).insightsCaption()
                                ForEach(Array(row.sessionRefs.enumerated()), id: \.element) { index, reference in
                                    Button(model.text("skills_evidence_session", values: ["count": String(index + 1)])) {
                                        evidenceOpen = true
                                        Task { await model.loadEvidence(reference) }
                                    }
                                }
                                Text(model.text("skills_daily_retained")).insightsCaption()
                                ForEach(row.daily, id: \.day) { day in
                                    HStack {
                                        Text(day.day)
                                        Text(model.text("skills_invocations", values: ["count": String(day.invocations)]))
                                        Text(model.text("skills_loads", values: ["count": String(day.loads)]))
                                    }.insightsCaption()
                                }
                            }
                           }
                          }
                        } else { Text(model.text(model.usageState(for: skill, in: snapshot))).insightsNote() }
                        if selectedSkillID == skill.id {
                            Text(skill.location).insightsMono().textSelection(.enabled)
                            Text(model.scopeLabel(skill.scope)).insightsCaption()
                            Group {
                                if model.definition?.skillID == skill.id, let body = model.definition?.definition {
                                    Text(body).insightsMono().textSelection(.enabled)
                                } else if model.definitionFailure {
                                    Text(model.text("skills_definition_unavailable")).insightsNote()
                                } else { Text(model.text("skills_loading")).insightsNote() }
                            }.task(id: "\(skill.id):\(model.detailEpoch)") { await model.loadDefinition(skill.id) }

                        }
                    }.accessibilityElement(children: .contain)
                }
                DisclosureGroup(model.text("skills_coverage")) {
                    Text(model.text("skills_usage_semantics")).insightsNote()
                    if snapshot.inventory.partial { Text(model.text("skills_partial_inventory")).insightsNote() }
                    if snapshot.usage.partial { Text(model.text("skills_partial_usage")).insightsNote() }
                    if let date = snapshot.usage.lastAnalysisAt { Text(model.text("skills_checked_at", values: ["date": model.timestamp(date)])) }
                    if let date = snapshot.usage.earliestEvidence { Text(model.text("skills_earliest", values: ["date": model.timestamp(date)])) }
                    Text(model.text("skills_sessions_observed", values: ["count": String(snapshot.usage.sessionsObserved)]))
                    ForEach(snapshot.usage.coverage, id: \.agent) { coverage in
                        Text(model.agentLabel(coverage.agent) + " · " + model.text("skills_coverage_" + coverage.status))
                    }
                    ForEach(snapshot.inventory.coverage, id: \.location) { coverage in
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
