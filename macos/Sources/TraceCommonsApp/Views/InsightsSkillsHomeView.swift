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

/// Skills Home: one column of stories, newest first, under the period each
/// falls in. A story is a card with a mark, a sentence about one skill,
/// where it comes from, and one action.
struct InsightsSkillsHomeView: View {
    let feed: InsightsSkillsHome.Snapshot
    let model: InsightsSkillsModel
    var now = Date()
    var calendar = Calendar.current
    let open: (SkillsHomeDestination) -> Void

    /// A feed is read down one column: on a wide pane it does not stretch.
    static let columnMaximum: CGFloat = 560

    @ScaledMetric(relativeTo: .body) private var markSize: CGFloat = 32

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s10) {
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
            ForEach(feed.timeline(now: now, calendar: calendar)) { group in
                VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                    Text(model.text(group.period.copyKey)).insightsCaption()
                        .padding(.leading, GlassTokens.Space.cardPaddingHorizontal)
                        .accessibilityAddTraits(.isHeader)
                    ForEach(group.stories) { story in
                        switch story {
                        case .update(let item): update(item)
                        case .trend(let item): trend(item)
                        case .creator(let item): creator(item)
                        }
                    }
                }
            }
        }
        .frame(maxWidth: Self.columnMaximum, alignment: .leading)
        .frame(maxWidth: .infinity, alignment: .center)
    }

    // MARK: Stories

    private func update(_ item: InsightsSkillsHome.Update) -> some View {
        let agents = item.agents.map(model.agentLabel).joined(separator: ", ")
        let source = item.catalogSkill.repository + "/" + item.catalogSkill.path
        return post(headline: "skills_home_story_update", name: item.name, time: nil,
                    summary: item.catalogSkill.description,
                    view: model.text("skills_home_open_skill", values: ["name": item.name + ", " + agents + ", " + source]),
                    destination: .catalog(id: item.catalogSkill.id, installedID: item.installedID)) {
            glyphMark("arrow.down")
        } byline: {
            Text(item.catalogSkill.repository).insightsCaption().lineLimit(1).truncationMode(.middle)
            InsightsSkillsAgentLogos(agents: item.agents, label: model.agentLabel)
        }
    }

    private func trend(_ item: InsightsSkillsHome.Trend) -> some View {
        let key = item.direction == .less ? "skills_home_story_less" : "skills_home_story_more"
        return post(headline: key, name: item.name, time: model.text("skills_home_trend_period"),
                    summary: "",
                    view: model.text("skills_home_open_usage", values: ["name": item.name + ", " + model.agentLabel(item.agent)]),
                    destination: .usage(id: item.id)) {
            mark(tint: Self.tool(item.agent, label: model.agentLabel).tint.color) {
                InsightsSkillsAgentLogos(agents: [item.agent], label: model.agentLabel, size: markSize / 2)
            }
        } byline: {
            Text(model.text(trendKey(item.direction), values: ["count": String(abs(item.currentUses - item.previousUses))]))
                .insightsNote()
        }
    }

    private func creator(_ item: InsightsSkillsHome.CreatorUpdate) -> some View {
        post(headline: "skills_home_story_" + item.event.kind, name: item.skill.name,
             time: Format.when(item.observedAt, relativeTo: now), summary: item.skill.description,
             view: model.text("skills_home_open_creator", values: ["name": item.skill.name + ", " + item.skill.path, "publisher": item.publisher.name]),
             destination: .catalog(id: item.skill.id)) {
            glyphMark(item.event.kind == "added" ? "plus" : "arrow.triangle.2.circlepath")
        } byline: {
            Text(item.publisher.name).insightsCaption().lineLimit(1)
        }
    }

    /// One story. The mark and the time are beside the headline; where the
    /// skill comes from, what it is and the one action sit under it, in
    /// line with it.
    private func post<Mark: View, Byline: View>(
        headline: String, name: String, time: String?, summary: String, view: String,
        destination: SkillsHomeDestination,
        @ViewBuilder mark: () -> Mark, @ViewBuilder byline: () -> Byline
    ) -> some View {
        HStack(alignment: .top, spacing: GlassTokens.Space.s6) {
            mark()
            VStack(alignment: .leading, spacing: InsightsLayout.lineGap) {
                HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s4) {
                    Text(Self.headline(model.text(headline), name: name))
                        .fixedSize(horizontal: false, vertical: true)
                    Spacer(minLength: 0)
                    if let time, !time.isEmpty {
                        Text(time).insightsCaption().lineLimit(1).layoutPriority(1)
                    }
                }
                HStack(spacing: GlassTokens.Space.s3) { byline() }
                if !summary.isEmpty {
                    Text(summary).insightsNote().lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(.top, GlassTokens.Space.s1)
                }
                Button(model.text("skills_home_view")) { open(destination) }
                    .buttonStyle(GlassButtonStyle(.link))
                    .accessibilityLabel(view)
                    .padding(.top, GlassTokens.Space.s3)
            }
        }
        .padding(.vertical, GlassTokens.Space.s1)
        .frame(maxWidth: .infinity, alignment: .leading)
        .insightsSection()
        .accessibilityElement(children: .contain)
    }

    private func glyphMark(_ symbol: String) -> some View {
        mark(tint: GlassColor.accentText) {
            Image(systemName: symbol).glassGlyph(markSize * 0.42, weight: .semibold)
                .foregroundStyle(GlassColor.accentText)
        }
        .accessibilityHidden(true)
    }

    private func mark<Content: View>(tint: Color, @ViewBuilder content: () -> Content) -> some View {
        content()
            .frame(width: markSize, height: markSize)
            .background(Circle().fill(tint.opacity(0.16)))
    }

    /// The sentence with the skill's name set strong. The name is public
    /// catalog text: it is placed as plain characters, never parsed.
    static func headline(_ template: String, name: String) -> AttributedString {
        var strong = AttributedString(name)
        strong.inlinePresentationIntent = .stronglyEmphasized
        guard let slot = template.range(of: "{name}") else { return strong }
        return AttributedString(String(template[..<slot.lowerBound])) + strong
            + AttributedString(String(template[slot.upperBound...]))
    }

    static func tool(_ agent: String, label: (String) -> String) -> GlassTool {
        switch agent {
        case "codex": .codex
        case "claude_code": .claudeCode
        case "opencode": .openCode
        default: .other(initials: String(label(agent).prefix(2)))
        }
    }

    private func trendKey(_ direction: InsightsSkillsHome.Direction) -> String {
        switch direction {
        case .more: "skills_home_more"
        case .less: "skills_home_less"
        case .started: "skills_home_started"
        }
    }
}

struct InsightsSkillsAgentLogos: View {
    let agents: [String]
    let label: (String) -> String
    var size: CGFloat = 14

    var body: some View {
        HStack(spacing: GlassTokens.Space.s2) {
            ForEach(agents, id: \.self) { agent in
                let tool = InsightsSkillsHomeView.tool(agent, label: label)
                Group {
                    if let logo = tool.logo {
                        GlassToolLogoShape(logo).fill(tool.tint.color).frame(width: size, height: size)
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
