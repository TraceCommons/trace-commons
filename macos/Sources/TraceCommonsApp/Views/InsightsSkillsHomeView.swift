import SwiftUI
import TCDesign
import TCShellCore

enum SkillsHomeDestination: Equatable {
    case usage(id: String)
    case catalog(id: String, installedID: String? = nil)
    case index
    case browse
}

/// Resolve Home actions against the current snapshot, not a stale feed item.
struct SkillsHomeRoute: Equatable {
    let section: SkillsSection
    var usageID: String?
    var catalogID: String?
    var installedID: String?

    static func resolve(_ destination: SkillsHomeDestination, in snapshot: DaemonData.SkillsSnapshot) -> Self? {
        guard snapshot.enabled, snapshot.readable else { return nil }
        switch destination {
        case .usage(let id):
            guard snapshot.usage.enabled,
                  snapshot.usage.skills.contains(where: { row in
                      row.agent + "\u{0}" + row.name == id && ["direct", "delegated"].contains(row.population)
                          && snapshot.usage.coverage.contains { $0.agent == row.agent && $0.status == "explicit_invocations" }
                  }) else { return nil }
            return .init(section: .usage, usageID: id)
        case .catalog(let id, let installedID):
            guard let catalog = snapshot.catalog.skills.first(where: { $0.id == id }) else { return nil }
            if let installedID {
                guard snapshot.inventory.skills.contains(where: {
                    $0.id == installedID && $0.source?.repository == catalog.repository && $0.source?.path == catalog.path
                }) else { return nil }
            }
            return .init(section: .catalog, catalogID: id, installedID: installedID)
        case .index:
            return .init(section: .inventory)
        case .browse:
            return .init(section: .catalog)
        }
    }

    static func catalogUpdate(catalogID: String, installedID: String?, in snapshot: DaemonData.SkillsSnapshot) -> DaemonData.SkillsUpdate? {
        guard let catalog = snapshot.catalog.skills.first(where: { $0.id == catalogID }) else { return nil }
        let installed = snapshot.inventory.skills.first {
            (installedID == nil || $0.id == installedID)
                && $0.source?.repository == catalog.repository && $0.source?.path == catalog.path
        }
        guard let installed else { return nil }
        return snapshot.updates.first { $0.installedID == installed.id }
    }
}

struct InsightsSkillsHomeView: View {
    let feed: InsightsSkillsHome.Snapshot
    let model: InsightsSkillsModel
    let open: (SkillsHomeDestination) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: InsightsLayout.sectionGap) {
            if feed.isEmpty {
                VStack(alignment: .leading, spacing: InsightsLayout.headingGap) {
                    Text(model.text("skills_home_empty")).insightsNote()
                    HStack {
                        Button(model.text("skills_home_index")) { open(.index) }
                            .accessibilityLabel(model.text("skills_home_index_accessibility"))
                        Button(model.text("skills_home_browse")) { open(.browse) }
                            .accessibilityLabel(model.text("skills_home_browse_accessibility"))
                    }
                }.insightsCard()
            }
            if !feed.updates.isEmpty {
                section("skills_home_updates") {
                    ForEach(feed.updates) { item in
                        VStack(alignment: .leading, spacing: InsightsLayout.lineGap) {
                            HStack {
                                Text(item.name).insightsHeading()
                                Spacer(minLength: GlassTokens.Space.s2)
                                InsightsSkillsAgentLogos(agents: item.agents, label: model.agentLabel)
                                viewButton(key: "skills_home_open_skill", name: item.name + ", " + item.agents.map(model.agentLabel).joined(separator: ", ") + ", " + item.catalogSkill.repository + "/" + item.catalogSkill.path) {
                                    open(.catalog(id: item.catalogSkill.id, installedID: item.installedID))
                                }
                            }
                            Text(model.text("skills_status_update_available")).insightsNote()
                        }
                    }
                }
            }
            if !feed.trends.isEmpty {
                section("skills_home_trends") {
                    Text(model.text("skills_home_trend_period")).insightsCaption()
                    ForEach(feed.trends) { item in
                        VStack(alignment: .leading, spacing: InsightsLayout.lineGap) {
                            HStack {
                                Text(item.name).insightsHeading()
                                Spacer(minLength: GlassTokens.Space.s2)
                                InsightsSkillsAgentLogos(agents: [item.agent], label: model.agentLabel)
                                viewButton(key: "skills_home_open_usage", name: item.name + ", " + model.agentLabel(item.agent)) {
                                    open(.usage(id: item.id))
                                }
                            }
                            Text(model.text(trendKey(item.direction), values: ["count": String(abs(item.currentUses - item.previousUses))])).insightsNote()
                        }
                    }
                }
            }
            if !feed.creators.isEmpty {
                section("skills_home_creators") {
                    ForEach(feed.creators) { item in
                        VStack(alignment: .leading, spacing: InsightsLayout.lineGap) {
                            HStack {
                                Text(item.skill.name).insightsHeading()
                                Spacer(minLength: GlassTokens.Space.s2)
                                Button(model.text("skills_home_view")) { open(.catalog(id: item.skill.id)) }
                                    .buttonStyle(GlassButtonStyle(.link))
                                    .accessibilityLabel(model.text("skills_home_open_creator", values: ["name": item.skill.name + ", " + item.skill.path, "publisher": item.publisher.name]))
                            }
                            HStack {
                                Text(item.publisher.name).insightsCaption()
                                Text(model.text("skills_feed_" + item.event.kind)).insightsNote()
                                Spacer(minLength: 0)
                                Text(item.observedAt, format: .dateTime.month(.abbreviated).day()).insightsCaption()
                            }
                        }
                    }
                }
            }
        }
    }

    private func trendKey(_ direction: InsightsSkillsHome.Direction) -> String {
        switch direction {
        case .more: "skills_home_more"
        case .less: "skills_home_less"
        case .started: "skills_home_started"
        }
    }

    private func viewButton(key: String, name: String, action: @escaping () -> Void) -> some View {
        Button(model.text("skills_home_view"), action: action)
            .buttonStyle(GlassButtonStyle(.link))
            .accessibilityLabel(model.text(key, values: ["name": name]))
    }

    private func section<Content: View>(_ key: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: InsightsLayout.headingGap) {
            Text(model.text(key)).insightsHeading()
            VStack(alignment: .leading, spacing: InsightsLayout.groupGap, content: content).insightsCardBody()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .insightsSection()
    }
}

struct InsightsSkillsAgentLogos: View {
    let agents: [String]
    let label: (String) -> String

    var body: some View {
        HStack(spacing: GlassTokens.Space.s2) {
            ForEach(agents, id: \.self) { agent in
                let tool: GlassTool = switch agent {
                case "codex": .codex
                case "claude_code": .claudeCode
                case "opencode": .openCode
                default: .other(initials: String(label(agent).prefix(2)))
                }
                Group {
                    if let logo = tool.logo {
                        GlassToolLogoShape(logo).fill(tool.tint.color).frame(width: 14, height: 14)
                    } else {
                        Text(tool.initials).insightsCaption()
                    }
                }
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(label(agent))
                .help(label(agent))
            }
        }
    }
}
