import Charts
import SwiftUI
import TCDesign
import TCShellCore

enum SkillsSection: String, CaseIterable, Hashable {
    case home, inventory, usage, catalog, following

    static let defaultSection: Self = .home

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
    @State private var selectedUsageID: String?
    @State private var selectedCatalogID: String?
    @State private var selectedCatalogInstallationID: String?
    @State private var selectedSection: SkillsSection
    @Environment(\.scenePhase) private var scenePhase

    init(model: InsightsSkillsModel, reviewSettings: (() -> Void)? = nil, initialSection: SkillsSection = SkillsSection.defaultSection) {
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
                } else if let observations = model.evidence?.skillUsage {
                    if let agent = model.evidence?.agent { Text(model.agentLabel(agent)).insightsCaption() }
                    Text(model.populationLabel(model.evidence?.population ?? "unknown")).insightsCaption()
                    Text(model.text("skills_coverage_" + observations.coverage)).insightsCaption()
                    if observations.partial { Text(model.text("skills_partial_usage")).insightsNote() }
                    ForEach(observations.skills, id: \.name) { count in
                        VStack(alignment: .leading, spacing: InsightsLayout.lineGap) {
                            Text(count.name).insightsHeading()
                            Text(model.text("skills_invocations", values: ["count": String(count.invocations)]))
                            Text(model.text("skills_loads", values: ["count": String(count.loads)]))
                        }
                    }
                } else { Text(model.text("skills_loading")).insightsNote() }
            }
            if let failure = model.failureKey { Text(model.text(failure)).insightsError() }
            InsightsTabPage(getStarted: nil) {
                if let snapshot = model.shown {
                    if snapshot.enabled {
                        switch selectedSection {
                        case .home:
                            InsightsSkillsHomeView(feed: InsightsSkillsHome.make(snapshot, model: model), model: model,
                                                   open: { openHome($0, snapshot: snapshot) })
                        case .inventory:
                            inventory(snapshot)
                        case .usage:
                            usage(snapshot)
                        case .catalog:
                            if selectedCatalogID == nil { publishers(snapshot) }
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
        .onChange(of: selectedSection) { _, section in
            model.invalidateDetails()
            if section != .catalog {
                selectedCatalogID = nil
                selectedCatalogInstallationID = nil
            }
        }
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

    private func indexAgentLogos(_ agents: [String]) -> some View {
        InsightsSkillsAgentLogos(agents: agents, label: model.agentLabel)
    }

    private func openHome(_ destination: SkillsHomeDestination, snapshot: DaemonData.SkillsSnapshot) {
        guard let route = SkillsHomeRoute.resolve(destination, in: snapshot) else { return }
        evidenceOpen = false
        model.invalidateDetails()
        switch route.section {
        case .inventory:
            inventoryQuery = ""
            inventoryAgent = ""
            inventoryAvailability = ""
            selectedSkillID = nil
        case .usage:
            usageDays = 7
            selectedUsageID = route.usageID
        case .catalog:
            catalogQuery = ""
            selectedCatalogID = route.catalogID
            selectedCatalogInstallationID = route.installedID
        case .home, .following:
            break
        }
        selectedSection = route.section
    }

    private func inventory(_ snapshot: DaemonData.SkillsSnapshot) -> some View {
        let skills = model.inventorySkills(in: snapshot, matching: inventoryQuery, agent: inventoryAgent,
                                          availability: inventoryAvailability, sort: inventorySort, days: usageDays)
        let groups = InsightsSkillsIndex.groups(snapshot.inventory.skills, matching: skills.map(\.id))
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
                ForEach(groups) { group in
                    let skill = group.representative
                    let definitionSkill = group.definitionInstallation(in: snapshot)
                    VStack(alignment: .leading, spacing: InsightsLayout.lineGap) {
                        Button { selectedSkillID = selectedSkillID == group.id ? nil : group.id } label: {
                            HStack {
                                Text(skill.name).insightsHeading()
                                Spacer()
                                indexAgentLogos(group.agents)
                                Image(systemName: selectedSkillID == group.id ? "chevron.up" : "chevron.down")
                            }
                        }.buttonStyle(.plain)
                        .accessibilityLabel(model.text("skills_inspect", values: ["name": skill.name + ", " + group.agents.map(model.agentLabel).joined(separator: ", ")]))
                        if !skill.description.isEmpty { Text(skill.description) }
                        Text(Array(Set(group.installations.map {
                            model.text("skills_availability_" + (snapshot.inventory.details[$0.id]?.availability ?? "unknown"))
                        })).sorted().joined(separator: " · ")).insightsCaption()
                        if let definitionStatus = snapshot.inventory.details[skill.id]?.definitionStatus,
                           ["invalid", "unreadable"].contains(definitionStatus) {
                            Text(model.text("skills_definition_status_" + definitionStatus)).insightsNote()
                        }
                        let usageRows = InsightsSkillsIndex.usageRows(group, snapshot: snapshot, model: model).filter { $0.windows.contains { $0.days == usageDays } }
                        if !usageRows.isEmpty && snapshot.usage.enabled {
                          ForEach(usageRows, id: \.identity) { row in
                           if let window = row.windows.first(where: { $0.days == usageDays }) {
                            Text(model.agentLabel(row.agent)).insightsCaption()
                            Text(model.populationLabel(row.population)).insightsCaption()
                            Text(model.text("skills_coverage_name_only")).insightsCaption()
                            Text(model.text("skills_sessions", values: ["count": String(window.sessions)]))
                            if snapshot.usage.coverage.contains(where: { $0.agent == row.agent && $0.status == "explicit_invocations" }) {
                                Text(model.text("skills_invocations", values: ["count": String(window.invocations)]))
                            }
                            if let date = row.lastSeen { Text(model.text("skills_last_seen", values: ["date": model.timestamp(date)])).insightsCaption() }
                            if selectedSkillID == group.id {
                                Text(model.text("skills_loads", values: ["count": String(window.loads)]))
                                Text(model.text("skills_undated", values: ["count": String(row.undatedEvents)])).insightsCaption()
                                evidenceLinks(row)
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
                        if selectedSkillID == group.id {
                            ForEach(group.installations) { installation in
                                HStack {
                                    indexAgentLogos([installation.agent])
                                    Text(installation.location).insightsMono().textSelection(.enabled)
                                }
                                .accessibilityElement(children: .ignore)
                                .accessibilityLabel(model.agentLabel(installation.agent) + ", " + installation.location)
                                Text(model.scopeLabel(installation.scope)).insightsCaption()
                                if installation.id != skill.id {
                                    Text(model.text("skills_availability_" + (snapshot.inventory.details[installation.id]?.availability ?? "unknown"))).insightsCaption()
                                }
                            }
                            Group {
                                if model.definition?.skillID == definitionSkill.id, let body = model.definition?.definition {
                                    Text(body).insightsMono().textSelection(.enabled)
                                } else if model.definitionFailure {
                                    Text(model.text("skills_definition_unavailable")).insightsNote()
                                } else { Text(model.text("skills_loading")).insightsNote() }
                            }.task(id: "\(definitionSkill.id):\(model.detailEpoch)") { await model.loadDefinition(definitionSkill.id) }

                        }
                    }.accessibilityElement(children: .contain)
                }
                DisclosureGroup(model.text("skills_coverage")) {
                    if snapshot.inventory.partial { Text(model.text("skills_partial_inventory")).insightsNote() }
                    if snapshot.usage.partial { Text(model.text("skills_partial_usage")).insightsNote() }
                    if let date = snapshot.usage.lastAnalysisAt { Text(model.text("skills_checked_at", values: ["date": model.timestamp(date)])) }
                    if let date = snapshot.usage.earliestEvidence { Text(model.text("skills_earliest", values: ["date": model.timestamp(date)])) }
                    observationCoverage(snapshot.usage)
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

    private func usage(_ snapshot: DaemonData.SkillsSnapshot) -> some View {
        let ranked = InsightsSkillsUsage.ranked(snapshot.usage, days: usageDays, includingID: selectedUsageID)
        let picked = selectedUsageID == nil ? ranked.first : ranked.first { $0.id == selectedUsageID }
        let candidates = InsightsSkillsUsage.removalCandidates(snapshot, days: usageDays, model: model)
        return VStack(alignment: .leading, spacing: InsightsLayout.sectionGap) {
            Picker(model.text("skills_usage_period_label"), selection: $usageDays) {
                ForEach([7, 30, 90], id: \.self) { days in
                    Text(model.text("skills_days", values: ["count": String(days)])).tag(days)
                }
            }.pickerStyle(.segmented)
            section("skills_most_used") {
                VStack(alignment: .leading, spacing: InsightsLayout.lineGap) {
                    if snapshot.usage.partial { Text(model.text("skills_partial_usage")).insightsNote() }
                    if ranked.isEmpty {
                        Text(model.text(InsightsSkillsUsage.emptyState(snapshot.usage, days: usageDays))).insightsNote()
                        if !snapshot.usage.enabled, let reviewSettings {
                            Button(model.text("skills_review_settings")) {
                                model.invalidateDetails()
                                reviewSettings()
                            }
                        }
                    }
                    ForEach(ranked) { skill in
                        Button {
                            selectedUsageID = skill.id
                            model.invalidateDetails()
                        } label: {
                            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                                HStack {
                                    Text(skill.name).insightsHeading()
                                    Spacer()
                                    Text(model.text("skills_invocations", values: ["count": String(skill.uses)]))
                                }
                                Text(model.agentLabel(skill.agent)).insightsCaption()
                                GeometryReader { geometry in
                                    RoundedRectangle(cornerRadius: 2)
                                        .fill(GlassTokens.Color.chartSeries1.color)
                                        .frame(width: geometry.size.width * CGFloat(skill.uses) / CGFloat(max(1, ranked.first?.uses ?? 1)))
                                }.frame(height: 6)
                            }.padding(GlassTokens.Space.s2)
                                .background(picked?.id == skill.id ? GlassTokens.Color.chartGrid.color : .clear,
                                            in: RoundedRectangle(cornerRadius: 6))
                        }.buttonStyle(.plain)
                            .accessibilityLabel(model.text("skills_select_usage_accessibility", values: ["name": skill.name + ", " + model.agentLabel(skill.agent)]))
                            .accessibilityValue(model.text("skills_invocations", values: ["count": String(skill.uses)]))
                            .accessibilityAddTraits(picked?.id == skill.id ? [.isSelected] : [])
                    }
                }
            }
            if !candidates.isEmpty {
                section("skills_removal_candidates") {
                    VStack(alignment: .leading, spacing: InsightsLayout.headingGap) {
                        Text(model.text("skills_no_use_period", values: ["count": String(usageDays)])).insightsCaption()
                        ForEach(candidates) { skill in
                            VStack(alignment: .leading, spacing: InsightsLayout.lineGap) {
                                Text(skill.name).insightsHeading()
                                Text(model.agentLabel(skill.agent)).insightsCaption()
                                Text(skill.location).insightsMono().textSelection(.enabled)
                            }
                        }
                    }
                }
            }
            if picked == nil, selectedUsageID != nil {
                Text(model.text("skills_usage_unavailable")).insightsNote()
            }
            if let picked {
                VStack(alignment: .leading, spacing: InsightsLayout.headingGap) {
                    HStack {
                        Text(picked.name).insightsHeading()
                        Spacer()
                        Text(model.text("skills_invocations", values: ["count": String(picked.uses)])).insightsNote()
                    }
                    Text(model.agentLabel(picked.agent)).insightsCaption()
                    if !picked.daily.isEmpty { usageChart(picked) }

                }.insightsCard()
            }
        }
    }

    private func usageChart(_ skill: InsightsSkillsUsage.Skill) -> some View {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.dateFormat = "yyyy-MM-dd"
        return Chart(skill.daily) { day in
            if let date = formatter.date(from: day.id) {
                BarMark(x: .value(model.text("skills_usage_period_label"), date, unit: .day),
                        y: .value(model.text("skills_uses"), day.uses), width: .ratio(0.56))
                    .foregroundStyle(GlassTokens.Color.chartSeries1.color)
                    .accessibilityLabel(day.id)
                    .accessibilityValue(model.text("skills_invocations", values: ["count": String(day.uses)]))
            }
        }
        // A continuous axis preserves gaps where no daily evidence exists.
        .chartXScale(domain: skill.start...skill.end)
        .chartYAxis {
            AxisMarks(position: .leading, values: .automatic(desiredCount: 3)) { value in
                AxisGridLine().foregroundStyle(GlassTokens.Color.chartGrid.color)
                AxisValueLabel {
                    if let count = value.as(Int.self) { Text(String(count)).insightsTick() }
                }
            }
        }
        .chartXAxis {
            AxisMarks(values: .automatic(desiredCount: 3)) { value in
                AxisValueLabel {
                    if let date = value.as(Date.self) {
                        Text(date, format: .dateTime.month(.abbreviated).day()).insightsTick()
                    }
                }
            }
        }
        .frame(height: 120)
        .insightsPlot()
        .accessibilityLabel(model.text("skills_usage_chart_accessibility", values: ["name": skill.name + ", " + model.agentLabel(skill.agent)]))
    }

    private func observationCoverage(_ usage: DaemonData.SkillsUsage) -> some View {
        VStack(alignment: .leading, spacing: InsightsLayout.lineGap) {
            Text(model.text("skills_population_direct")).insightsCaption()
            Text(model.text("skills_sessions_observed", values: ["count": String(usage.sessionsObserved)]))
                .insightsNote()
            if usage.sessionsUnknown > 0 {
                Text(model.text("skills_sessions_unknown", values: ["count": String(usage.sessionsUnknown)]))
                    .insightsNote()
            }
            Text(model.text("skills_population_delegated")).insightsCaption()
            if let observed = usage.delegatedSessionsObserved {
                Text(model.text("skills_sessions_observed", values: ["count": String(observed)])).insightsNote()
            } else {
                Text(model.text("skills_usage_pending")).insightsNote()
            }
            if let unknown = usage.delegatedSessionsUnknown, unknown > 0 {
                Text(model.text("skills_sessions_unknown", values: ["count": String(unknown)])).insightsNote()
            }
            if (usage.unclassifiedSessionsObserved ?? 0) > 0 || (usage.unclassifiedSessionsUnknown ?? 0) > 0 {
                Text(model.text("skills_population_unknown")).insightsCaption()
                if let observed = usage.unclassifiedSessionsObserved {
                    Text(model.text("skills_sessions_observed", values: ["count": String(observed)])).insightsNote()
                }
                if let unknown = usage.unclassifiedSessionsUnknown, unknown > 0 {
                    Text(model.text("skills_sessions_unknown", values: ["count": String(unknown)])).insightsNote()
                }
            }
        }
    }

    private func evidenceLinks(_ row: DaemonData.SkillsUsageRow) -> some View {
        DisclosureGroup(model.text("skills_evidence_links")) {
            let links = model.evidenceLinks(for: row)
            if links.isEmpty { Text(model.text("skills_evidence_unavailable")).insightsNote() }
            ForEach(Array(links.enumerated()), id: \.element.sessionRef) { index, link in
                VStack(alignment: .leading, spacing: InsightsLayout.lineGap) {
                    Button(model.text(row.population == "delegated" ? "skills_evidence_child" : "skills_evidence_session",
                                      values: ["count": String(index + 1)])) {
                        evidenceOpen = true
                        Task { await model.loadEvidence(link.sessionRef) }
                    }
                    .accessibilityLabel(model.text(row.population == "delegated" ? "skills_evidence_child_accessibility" : "skills_evidence_accessibility",
                                                   values: ["name": row.name + " · " + model.agentLabel(row.agent), "count": String(index + 1)]))
                    if let parent = link.parentSessionRef {
                        Button(model.text("skills_evidence_parent")) {
                            evidenceOpen = true
                            Task { await model.loadEvidence(parent) }
                        }
                        .accessibilityLabel(model.text("skills_evidence_parent_accessibility", values: ["name": row.name + " · " + model.agentLabel(row.agent), "count": String(index + 1)]))
                    }
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
        let skills = model.catalogSkills(in: snapshot, matching: catalogQuery).sorted {
            if ($0.id == selectedCatalogID) != ($1.id == selectedCatalogID) { return $0.id == selectedCatalogID }
            return $0.name.localizedStandardCompare($1.name) == .orderedAscending
        }
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
                        if let update = SkillsHomeRoute.catalogUpdate(catalogID: skill.id,
                            installedID: skill.id == selectedCatalogID ? selectedCatalogInstallationID : nil,
                            in: snapshot) {
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
