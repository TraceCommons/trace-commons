import Charts
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

/// What moves on Skills Home. Everything here is off under Reduce Motion:
/// the feed is then still, and complete on its first frame.
struct SkillsHomeMotion: Equatable {
    let reduce: Bool

    /// Nothing moves: Reduce Motion, and a render that has one frame.
    static let still = Self(reduce: true)

    /// A chart's bars rise once, when their card first appears.
    var growsBars: Bool { !reduce }
    /// A card rises slightly under the pointer.
    var lifts: Bool { !reduce }
    /// A card eases in as it scrolls into view.
    var scrolls: Bool { !reduce }
    /// A count rolls to its new value.
    var rollsNumbers: Bool { !reduce }
    /// A story opening in place.
    var expand: Animation? { reduce ? nil : .snappy(duration: 0.25) }
    var hover: Animation? { reduce ? nil : .easeOut(duration: 0.15) }
    var grow: Animation? { reduce ? nil : .easeOut(duration: 0.5) }
}

/// Skills Home: one column of stories, newest first, under the period each
/// falls in. The first story today is drawn tall under its cover; a usage
/// change carries its fourteen days; several changes from one publisher are
/// one card.
struct InsightsSkillsHomeView: View {
    let feed: InsightsSkillsHome.Snapshot
    let model: InsightsSkillsModel
    var now = Date()
    var calendar = Calendar.current
    /// Nil follows the system's Reduce Motion setting.
    var motion: SkillsHomeMotion?
    let open: (SkillsHomeDestination) -> Void

    /// A feed is read down one column: on a wide pane it does not stretch.
    static let columnMaximum: CGFloat = 560
    static let heroBand: CGFloat = 96

    @ScaledMetric(relativeTo: .body) private var markSize: CGFloat = 40
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var expanded: Set<String> = []

    private var moves: SkillsHomeMotion { motion ?? SkillsHomeMotion(reduce: reduceMotion) }

    var body: some View {
        let groups = feed.timeline(now: now, calendar: calendar)
        let hero = InsightsSkillsHome.Snapshot.heroID(in: groups)
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
            ForEach(groups) { group in
                VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                    Text(model.text(group.period.copyKey)).insightsCaption()
                        .padding(.leading, GlassTokens.Space.cardPaddingHorizontal)
                        .accessibilityAddTraits(.isHeader)
                    ForEach(group.stories) { story in
                        SkillsHomeCard(motion: moves) { card(story, hero: story.id == hero) }
                    }
                }
            }
        }
        .frame(maxWidth: Self.columnMaximum, alignment: .leading)
        .frame(maxWidth: .infinity, alignment: .center)
    }

    @ViewBuilder private func card(_ story: InsightsSkillsHome.Story, hero: Bool) -> some View {
        switch story {
        case .update(let item): update(item, hero: hero)
        case .trend(let item): trend(item)
        case .creator(let item): creator(item, hero: hero)
        case .rollup(let item): rollup(item)
        case .week(let skills): week(skills)
        }
    }

    // MARK: Stories about a catalog skill

    private func update(_ item: InsightsSkillsHome.Update, hero: Bool) -> some View {
        let agents = item.agents.map(model.agentLabel).joined(separator: ", ")
        let skill = item.catalogSkill
        return catalogStory(id: item.id, hero: hero, skill: skill, publisher: item.publisherName ?? skill.repository,
                            headline: "skills_home_story_update", name: item.name, time: nil,
                            view: model.text("skills_home_open_skill", values: ["name": item.name + ", " + agents + ", " + skill.repository + "/" + skill.path]),
                            destination: .catalog(id: skill.id, installedID: item.installedID)) {
            Text(skill.repository).insightsCaption().lineLimit(1).truncationMode(.middle)
            InsightsSkillsAgentLogos(agents: item.agents, label: model.agentLabel)
        }
    }

    private func creator(_ item: InsightsSkillsHome.CreatorUpdate, hero: Bool) -> some View {
        catalogStory(id: "creator:" + item.id, hero: hero, skill: item.skill, publisher: item.publisher.name,
                     headline: "skills_home_story_" + item.event.kind, name: item.skill.name,
                     time: Format.when(item.observedAt, relativeTo: now),
                     view: model.text("skills_home_open_creator", values: ["name": item.skill.name + ", " + item.skill.path, "publisher": item.publisher.name]),
                     destination: .catalog(id: item.skill.id)) {
            Text(item.publisher.name).insightsCaption().lineLimit(1)
        }
    }

    /// A story with a cover: a tile beside the words, or, for the hero, a
    /// band above them. Its description opens in place.
    private func catalogStory<Byline: View>(
        id: String, hero: Bool, skill: DaemonData.CatalogSkill, publisher: String,
        headline: String, name: String, time: String?, view: String, destination: SkillsHomeDestination,
        @ViewBuilder byline: () -> Byline
    ) -> some View {
        let spec = SkillCoverArt.Spec(skillID: skill.id, publisherID: skill.publisherID)
        let initials = SkillCoverArt.initials(publisher)
        let isOpen = expanded.contains(id)
        let words = VStack(alignment: .leading, spacing: InsightsLayout.lineGap) {
            headlineRow(headline, name: name, time: time, hero: hero)
            HStack(spacing: GlassTokens.Space.s3) { byline() }
            if !skill.description.isEmpty {
                Text(skill.description).insightsNote().lineLimit(isOpen ? nil : (hero ? 3 : 2))
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.top, GlassTokens.Space.s1)
            }
            if isOpen {
                Text(skill.repository + "/" + skill.path).insightsMono()
                    .fixedSize(horizontal: false, vertical: true)
                    .textSelection(.enabled)
            }
            HStack(spacing: GlassTokens.Space.s6) {
                viewButton(view, destination)
                Spacer(minLength: 0)
                Button {
                    withAnimation(moves.expand) {
                        if isOpen { expanded.remove(id) } else { expanded.insert(id) }
                    }
                } label: {
                    Image(systemName: isOpen ? "chevron.up" : "chevron.down").glassGlyph(11, weight: .semibold)
                        .foregroundStyle(GlassColor.textSecondary)
                        .frame(width: 24, height: 20).contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(model.text("skills_inspect", values: ["name": name]))
            }
            .padding(.top, GlassTokens.Space.s3)
        }
        return Group {
            if hero {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
                    SkillCover(spec: spec, initials: initials, cornerMark: true, radius: 10)
                        .frame(maxWidth: .infinity).frame(height: Self.heroBand)
                    words
                }
            } else {
                HStack(alignment: .top, spacing: GlassTokens.Space.s6) {
                    SkillCover(spec: spec, initials: initials).frame(width: markSize, height: markSize)
                    words
                }
            }
        }
    }

    // MARK: Several changes from one publisher

    private func rollup(_ item: InsightsSkillsHome.Rollup) -> some View {
        let publisher = item.publisher
        return HStack(alignment: .top, spacing: GlassTokens.Space.s6) {
            SkillCover(spec: .init(skillID: "", publisherID: publisher.id), initials: SkillCoverArt.initials(publisher.name))
                .frame(width: markSize, height: markSize)
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s4) {
                    Text(model.text("skills_home_rollup", values: ["count": String(item.changes.count), "publisher": publisher.name]))
                        .fixedSize(horizontal: false, vertical: true)
                    Spacer(minLength: 0)
                    Text(Format.when(item.changes[0].observedAt, relativeTo: now)).insightsCaption().lineLimit(1).layoutPriority(1)
                }
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 84), spacing: GlassTokens.Space.s4, alignment: .top)],
                          alignment: .leading, spacing: GlassTokens.Space.s4) {
                    ForEach(item.changes) { change in
                        Button { open(.catalog(id: change.skill.id)) } label: {
                            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                                SkillCover(spec: .init(skillID: change.skill.id, publisherID: change.skill.publisherID))
                                    .frame(maxWidth: .infinity).frame(height: 48)
                                Text(change.skill.name).insightsCaption().lineLimit(1).truncationMode(.tail)
                            }
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(model.text("skills_home_open_creator", values: ["name": change.skill.name + ", " + change.skill.path, "publisher": publisher.name]))
                        .help(change.skill.name)
                    }
                }
            }
        }
    }

    // MARK: Usage

    private func trend(_ item: InsightsSkillsHome.Trend) -> some View {
        let tool = Self.tool(item.agent, label: model.agentLabel)
        let line = model.text(item.currentUses > item.previousUses ? "skills_home_compare_up" : "skills_home_compare_down",
                              values: ["count": String(item.currentUses), "previous": String(item.previousUses)])
        return HStack(alignment: .top, spacing: GlassTokens.Space.s6) {
            InsightsSkillsAgentLogos(agents: [item.agent], label: model.agentLabel, size: markSize * 0.45)
                .frame(width: markSize, height: markSize)
                .background(Circle().fill(tool.tint.color.opacity(0.16)))
            VStack(alignment: .leading, spacing: InsightsLayout.lineGap) {
                headlineRow(item.direction == .less ? "skills_home_story_less" : "skills_home_story_more",
                            name: item.name, time: model.text("skills_home_trend_period"), hero: false)
                if !item.bars.isEmpty {
                    SkillsHomeUsageBars(bars: item.bars, tint: tool.tint.color, motion: moves,
                                        axis: model.text("skills_usage_period_label"), value: model.text("skills_uses"))
                        .frame(height: 44)
                        .padding(.vertical, GlassTokens.Space.s2)
                        .accessibilityElement(children: .ignore)
                        .accessibilityLabel(model.text("skills_usage_chart_accessibility", values: ["name": item.name]))
                        .accessibilityValue(line)
                }
                Text(line).insightsNote()
                    .contentTransition(moves.rollsNumbers ? .numericText() : .identity)
                    .animation(moves.expand, value: item.currentUses)
                viewButton(model.text("skills_home_open_usage", values: ["name": item.name + ", " + model.agentLabel(item.agent)]),
                           .usage(id: item.id))
                    .padding(.top, GlassTokens.Space.s3)
            }
        }
    }

    /// The week's most-used skills, each a row that opens its usage.
    private func week(_ skills: [InsightsSkillsHome.WeekSkill]) -> some View {
        let most = max(1, skills.map(\.uses).max() ?? 1)
        return VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
            Text(model.text("skills_home_week")).insightsHeading().accessibilityAddTraits(.isHeader)
            ForEach(skills) { skill in
                let tool = Self.tool(skill.agent, label: model.agentLabel)
                Button { open(.usage(id: skill.id)) } label: {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                        HStack(spacing: GlassTokens.Space.s4) {
                            InsightsSkillsAgentLogos(agents: [skill.agent], label: model.agentLabel)
                            Text(skill.name).lineLimit(1)
                            Spacer(minLength: GlassTokens.Space.s4)
                            Text(model.text("skills_invocations", values: ["count": String(skill.uses)])).insightsCaption()
                                .contentTransition(moves.rollsNumbers ? .numericText() : .identity)
                                .animation(moves.expand, value: skill.uses)
                        }
                        SkillsHomeShare(share: Double(skill.uses) / Double(most), tint: tool.tint.color, motion: moves)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(model.text("skills_home_open_usage", values: ["name": skill.name + ", " + model.agentLabel(skill.agent)]))
                .accessibilityValue(model.text("skills_invocations", values: ["count": String(skill.uses)]))
            }
        }
    }

    // MARK: Parts

    private func headlineRow(_ key: String, name: String, time: String?, hero: Bool) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s4) {
            Text(Self.headline(model.text(key), name: name))
                .glassType(hero ? GlassTokens.TypeScale.title.weight(.regular) : GlassTokens.TypeScale.body)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
            if let time, !time.isEmpty {
                Text(time).insightsCaption().lineLimit(1).layoutPriority(1)
            }
        }
    }

    private func viewButton(_ label: String, _ destination: SkillsHomeDestination) -> some View {
        Button(model.text("skills_home_view")) { open(destination) }
            .buttonStyle(GlassButtonStyle(.link))
            .accessibilityLabel(label)
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
}

/// One story's card: it rises a little under the pointer and eases in as it
/// scrolls into view, unless motion is off.
private struct SkillsHomeCard<Content: View>: View {
    let motion: SkillsHomeMotion
    @ViewBuilder let content: Content
    @State private var hovering = false

    var body: some View {
        let card = GlassCard(interactive: true) {
            content.padding(.vertical, GlassTokens.Space.s1).frame(maxWidth: .infinity, alignment: .leading)
        }
        .accessibilityElement(children: .contain)
        if motion.lifts || motion.scrolls {
            card.offset(y: hovering && motion.lifts ? -1.5 : 0)
                .animation(motion.hover, value: hovering)
                .onHover { hovering = $0 }
                .scrollTransition(.interactive) { view, phase in
                    view.opacity(phase.isIdentity ? 1 : 0.6).scaleEffect(phase.isIdentity ? 1 : 0.985)
                }
        } else {
            card
        }
    }
}

/// Fourteen days of uses: the previous seven muted, the last seven in the
/// agent's tint. No axes: the line under it gives both totals.
struct SkillsHomeUsageBars: View {
    let bars: [InsightsSkillsHome.Trend.Bar]
    let tint: Color
    let motion: SkillsHomeMotion
    let axis: String
    let value: String
    @State private var grown = false

    var body: some View {
        Chart(bars) { bar in
            BarMark(x: .value(axis, String(bar.id)), y: .value(value, bar.uses), width: .ratio(0.62))
                .foregroundStyle(bar.recent ? tint : GlassColor.ink(0.22))
                .clipShape(RoundedRectangle(cornerRadius: 2, style: .continuous))
        }
        .chartXAxis(.hidden)
        .chartYAxis(.hidden)
        .chartYScale(domain: 0 ... max(1, bars.map(\.uses).max() ?? 1))
        .chartLegend(.hidden)
        .overlay(alignment: .bottom) { GlassHairline(GlassColor.hairline) }
        .scaleEffect(x: 1, y: motion.growsBars && !grown ? 0.02 : 1, anchor: .bottom)
        .onAppear {
            guard motion.growsBars, !grown else { return }
            withAnimation(motion.grow) { grown = true }
        }
    }
}

/// A share of the week's most-used skill, as a bar.
private struct SkillsHomeShare: View {
    let share: Double
    let tint: Color
    let motion: SkillsHomeMotion
    @State private var grown = false

    var body: some View {
        Capsule().fill(GlassColor.ink(0.1)).frame(height: 4)
            .overlay(alignment: .leading) {
                Capsule().fill(tint).frame(height: 4)
                    .scaleEffect(x: max(0.02, min(1, share)) * (motion.growsBars && !grown ? 0.02 : 1), y: 1, anchor: .leading)
            }
            .onAppear {
                guard motion.growsBars, !grown else { return }
                withAnimation(motion.grow) { grown = true }
            }
            .accessibilityHidden(true)
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
