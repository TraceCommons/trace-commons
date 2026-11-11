import Foundation

/// Local inventory, explicit usage observations and publisher catalog; no execution authority.
extension DaemonData {
    public struct SkillsSnapshot: Codable, Equatable, Sendable {
        public let enabled: Bool
        public let readable: Bool
        public let refreshing: Bool
        public let error: String?
        public let inventory: SkillsInventory
        public let catalog: SkillsCatalog
        public let followedPublishers: [String]
        public let updates: [SkillsUpdate]
        public let feed: [SkillsFeedEvent]
        public let feedPartial: Bool
        public let usage: SkillsUsage
        public let copy: [String: String]

        public init(enabled: Bool = false, readable: Bool = true, refreshing: Bool = false, error: String? = nil, inventory: SkillsInventory = .init(), catalog: SkillsCatalog = .init(), followedPublishers: [String] = [], updates: [SkillsUpdate] = [], feed: [SkillsFeedEvent] = [], feedPartial: Bool = false, usage: SkillsUsage = .init(), copy: [String: String] = [:]) {
            self.enabled = enabled
            self.readable = readable
            self.refreshing = refreshing
            self.error = error
            self.inventory = inventory
            self.catalog = catalog
            self.followedPublishers = followedPublishers
            self.updates = updates
            self.feed = feed
            self.feedPartial = feedPartial
            self.usage = usage
            self.copy = copy
        }

        enum CodingKeys: String, CodingKey {
            case enabled
            case readable
            case refreshing
            case error
            case inventory
            case catalog
            case followedPublishers = "followed_publishers"
            case updates
            case feed
            case feedPartial = "feed_partial"
            case usage
            case copy
        }

        public init(from decoder: Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            enabled = try values.decodeIfPresent(Bool.self, forKey: .enabled) ?? false
            readable = try values.decodeIfPresent(Bool.self, forKey: .readable) ?? true
            refreshing = try values.decodeIfPresent(Bool.self, forKey: .refreshing) ?? false
            error = try values.decodeIfPresent(String.self, forKey: .error)
            inventory = try values.decodeIfPresent(SkillsInventory.self, forKey: .inventory) ?? .init()
            catalog = try values.decodeIfPresent(SkillsCatalog.self, forKey: .catalog) ?? .init()
            followedPublishers = try values.decodeIfPresent([String].self, forKey: .followedPublishers) ?? []
            updates = try values.decodeIfPresent([SkillsUpdate].self, forKey: .updates) ?? []
            feed = try values.decodeIfPresent([SkillsFeedEvent].self, forKey: .feed) ?? []
            feedPartial = try values.decodeIfPresent(Bool.self, forKey: .feedPartial) ?? false
            usage = try values.decodeIfPresent(SkillsUsage.self, forKey: .usage) ?? .init()
            copy = try values.decodeIfPresent([String: String].self, forKey: .copy) ?? [:]
        }
    }

    public struct SkillsInventory: Codable, Equatable, Sendable {
        public let skills: [InstalledSkill]
        public let partial: Bool
        public let scannedAt: String?
        public let coverage: [SkillsInventoryCoverage]
        public let details: [String: SkillInstallationDetails]

        public init(skills: [InstalledSkill] = [], partial: Bool = true, coverage: [SkillsInventoryCoverage] = [], details: [String: SkillInstallationDetails] = [:], scannedAt: String? = nil) {
            self.skills = skills
            self.partial = partial
            self.scannedAt = scannedAt
            self.coverage = coverage
            self.details = details
        }

        enum CodingKeys: String, CodingKey {
            case scannedAt = "scanned_at"
            case skills
            case partial
            case coverage
            case details
        }

        public init(from decoder: Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            scannedAt = try values.decodeIfPresent(String.self, forKey: .scannedAt)
            skills = try values.decodeIfPresent([InstalledSkill].self, forKey: .skills) ?? []
            partial = try values.decodeIfPresent(Bool.self, forKey: .partial) ?? true
            coverage = try values.decodeIfPresent([SkillsInventoryCoverage].self, forKey: .coverage) ?? []
            details = try values.decodeIfPresent([String: SkillInstallationDetails].self, forKey: .details) ?? [:]
        }
    }

    public struct InstalledSkill: Codable, Equatable, Sendable, Identifiable {
        public let id: String
        public let name: String
        public let description: String
        public let agent: String
        public let scope: String
        public let location: String
        public let sha256: String
        public let source: SkillsSource?

        public init(id: String, name: String, description: String, agent: String, scope: String, location: String, sha256: String, source: SkillsSource? = nil) {
            self.id = id
            self.name = name
            self.description = description
            self.agent = agent
            self.scope = scope
            self.location = location
            self.sha256 = sha256
            self.source = source
        }

        enum CodingKeys: String, CodingKey {
            case id
            case name
            case description
            case agent
            case scope
            case location
            case sha256
            case source
        }

        public init(from decoder: Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            id = try values.decode(String.self, forKey: .id)
            name = try values.decode(String.self, forKey: .name)
            description = try values.decode(String.self, forKey: .description)
            agent = try values.decode(String.self, forKey: .agent)
            scope = try values.decode(String.self, forKey: .scope)
            location = try values.decode(String.self, forKey: .location)
            sha256 = try values.decode(String.self, forKey: .sha256)
            source = try values.decodeIfPresent(SkillsSource.self, forKey: .source)
        }
    }

    public struct SkillsSource: Codable, Equatable, Sendable {
        public let repository: String
        public let path: String
        public let revision: String?

        public init(repository: String, path: String, revision: String? = nil) {
            self.repository = repository
            self.path = path
            self.revision = revision
        }

        enum CodingKeys: String, CodingKey {
            case repository
            case path
            case revision
        }

        public init(from decoder: Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            repository = try values.decode(String.self, forKey: .repository)
            path = try values.decode(String.self, forKey: .path)
            revision = try values.decodeIfPresent(String.self, forKey: .revision)
        }
    }

    public struct SkillsInventoryCoverage: Codable, Equatable, Sendable {
        public let location: String
        public let status: String

        public init(location: String, status: String) {
            self.location = location
            self.status = status
        }

        enum CodingKeys: String, CodingKey {
            case location
            case status
        }
    }

    public struct SkillsCatalog: Codable, Equatable, Sendable {
        public let publishers: [SkillsPublisher]
        public let skills: [CatalogSkill]
        public let checkedAt: String?
        public let publisherStates: [SkillsPublisherState]
        public let partial: Bool

        public init(publishers: [SkillsPublisher] = [], skills: [CatalogSkill] = [], checkedAt: String? = nil, publisherStates: [SkillsPublisherState] = [], partial: Bool = false) {
            self.publishers = publishers
            self.skills = skills
            self.checkedAt = checkedAt
            self.publisherStates = publisherStates
            self.partial = partial
        }

        enum CodingKeys: String, CodingKey {
            case publishers
            case skills
            case checkedAt = "checked_at"
            case publisherStates = "publisher_states"
            case partial
        }

        public init(from decoder: Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            publishers = try values.decodeIfPresent([SkillsPublisher].self, forKey: .publishers) ?? []
            skills = try values.decodeIfPresent([CatalogSkill].self, forKey: .skills) ?? []
            checkedAt = try values.decodeIfPresent(String.self, forKey: .checkedAt)
            publisherStates = try values.decodeIfPresent([SkillsPublisherState].self, forKey: .publisherStates) ?? []
            partial = try values.decodeIfPresent(Bool.self, forKey: .partial) ?? false
        }
    }

    public struct SkillsPublisher: Codable, Equatable, Sendable, Identifiable {
        public let id: String
        public let name: String
        public let repository: String
        public let skillsRoot: String
        public let instructionsURL: String

        public init(id: String, name: String, repository: String, skillsRoot: String, instructionsURL: String) {
            self.id = id
            self.name = name
            self.repository = repository
            self.skillsRoot = skillsRoot
            self.instructionsURL = instructionsURL
        }

        enum CodingKeys: String, CodingKey {
            case id
            case name
            case repository
            case skillsRoot = "skills_root"
            case instructionsURL = "instructions_url"
        }
    }

    public struct SkillsPublisherState: Codable, Equatable, Sendable {
        public let publisherID: String
        public let status: String
        public let checkedAt: String?

        public init(publisherID: String, status: String, checkedAt: String? = nil) {
            self.publisherID = publisherID
            self.status = status
            self.checkedAt = checkedAt
        }

        enum CodingKeys: String, CodingKey {
            case publisherID = "publisher_id"
            case status
            case checkedAt = "checked_at"
        }

        public init(from decoder: Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            publisherID = try values.decode(String.self, forKey: .publisherID)
            status = try values.decode(String.self, forKey: .status)
            checkedAt = try values.decodeIfPresent(String.self, forKey: .checkedAt)
        }
    }

    public struct CatalogSkill: Codable, Equatable, Sendable, Identifiable {
        public let id: String
        public let publisherID: String
        public let name: String
        public let description: String
        public let repository: String
        public let path: String
        public let revision: String
        public let sha256: String
        public let sourceURL: String
        public let instructionsURL: String

        public init(id: String, publisherID: String, name: String, description: String, repository: String, path: String, revision: String, sha256: String, sourceURL: String, instructionsURL: String) {
            self.id = id
            self.publisherID = publisherID
            self.name = name
            self.description = description
            self.repository = repository
            self.path = path
            self.revision = revision
            self.sha256 = sha256
            self.sourceURL = sourceURL
            self.instructionsURL = instructionsURL
        }

        enum CodingKeys: String, CodingKey {
            case id
            case publisherID = "publisher_id"
            case name
            case description
            case repository
            case path
            case revision
            case sha256
            case sourceURL = "source_url"
            case instructionsURL = "instructions_url"
        }
    }

    public struct SkillsUpdate: Codable, Equatable, Sendable {
        public let installedID: String
        public let status: String
        public let catalogSkillID: String?

        public init(installedID: String, status: String, catalogSkillID: String? = nil) {
            self.installedID = installedID
            self.status = status
            self.catalogSkillID = catalogSkillID
        }

        enum CodingKeys: String, CodingKey {
            case installedID = "installed_id"
            case status
            case catalogSkillID = "catalog_skill_id"
        }

        public init(from decoder: Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            installedID = try values.decode(String.self, forKey: .installedID)
            status = try values.decode(String.self, forKey: .status)
            catalogSkillID = try values.decodeIfPresent(String.self, forKey: .catalogSkillID)
        }
    }

    public struct SkillsFeedEvent: Codable, Equatable, Sendable, Identifiable {
        public let id: String
        public let publisherID: String
        public let skillID: String
        public let kind: String
        public let observedAt: String

        public init(id: String, publisherID: String, skillID: String, kind: String, observedAt: String) {
            self.id = id
            self.publisherID = publisherID
            self.skillID = skillID
            self.kind = kind
            self.observedAt = observedAt
        }

        enum CodingKeys: String, CodingKey {
            case id
            case publisherID = "publisher_id"
            case skillID = "skill_id"
            case kind
            case observedAt = "observed_at"
        }
    }

    public struct SkillsUsage: Codable, Equatable, Sendable {
        public let enabled: Bool
        public let lastAnalysisAt: String?
        public let earliestEvidence: String?
        public let sessionsPartial: Int
        public let sessionsObserved: Int
        public let sessionsUnknown: Int
        public let partial: Bool
        public let skills: [SkillsUsageRow]
        public let coverage: [SkillsUsageCoverage]

        public init(enabled: Bool = false, lastAnalysisAt: String? = nil, earliestEvidence: String? = nil, sessionsPartial: Int = 0, sessionsObserved: Int = 0, sessionsUnknown: Int = 0, partial: Bool = true, skills: [SkillsUsageRow] = [], coverage: [SkillsUsageCoverage] = []) {
            self.enabled = enabled
            self.lastAnalysisAt = lastAnalysisAt
            self.earliestEvidence = earliestEvidence
            self.sessionsPartial = sessionsPartial
            self.sessionsObserved = sessionsObserved
            self.sessionsUnknown = sessionsUnknown
            self.partial = partial
            self.skills = skills
            self.coverage = coverage
        }

        enum CodingKeys: String, CodingKey {
            case enabled
            case lastAnalysisAt = "last_analysis_at"
            case earliestEvidence = "earliest_evidence"
            case sessionsPartial = "sessions_partial"
            case sessionsObserved = "sessions_observed"
            case sessionsUnknown = "sessions_unknown"
            case partial
            case skills
            case coverage
        }

        public init(from decoder: Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            enabled = try values.decodeIfPresent(Bool.self, forKey: .enabled) ?? false
            lastAnalysisAt = try values.decodeIfPresent(String.self, forKey: .lastAnalysisAt)
            earliestEvidence = try values.decodeIfPresent(String.self, forKey: .earliestEvidence)
            sessionsPartial = try values.decodeIfPresent(Int.self, forKey: .sessionsPartial) ?? 0
            sessionsObserved = try values.decodeIfPresent(Int.self, forKey: .sessionsObserved) ?? 0
            sessionsUnknown = try values.decodeIfPresent(Int.self, forKey: .sessionsUnknown) ?? 0
            partial = try values.decodeIfPresent(Bool.self, forKey: .partial) ?? true
            skills = try values.decodeIfPresent([SkillsUsageRow].self, forKey: .skills) ?? []
            coverage = try values.decodeIfPresent([SkillsUsageCoverage].self, forKey: .coverage) ?? []
        }
    }

    public struct SkillsUsageCoverage: Codable, Equatable, Sendable {
        public let agent: String
        public let status: String

        public init(agent: String, status: String) {
            self.agent = agent
            self.status = status
        }

        enum CodingKeys: String, CodingKey {
            case agent
            case status
        }
    }

    public struct SkillsUsageRow: Codable, Equatable, Sendable {
        public let windows: [SkillsUsageWindow]
        public let daily: [SkillsUsageDay]
        public let undatedEvents: Int
        public let name: String
        public let agent: String
        public let invocations: Int
        public let loads: Int
        public let sessions: Int
        public let lastSeen: String?
        public let sessionRefs: [String]

        public init(name: String, agent: String, invocations: Int, loads: Int, sessions: Int, lastSeen: String? = nil, sessionRefs: [String] = [], windows: [SkillsUsageWindow] = [], daily: [SkillsUsageDay] = [], undatedEvents: Int = 0) {
            self.windows = windows
            self.daily = daily
            self.undatedEvents = undatedEvents
            self.name = name
            self.agent = agent
            self.invocations = invocations
            self.loads = loads
            self.sessions = sessions
            self.lastSeen = lastSeen
            self.sessionRefs = sessionRefs
        }

        enum CodingKeys: String, CodingKey {
            case windows, daily
            case undatedEvents = "undated_events"
            case name
            case agent
            case invocations
            case loads
            case sessions
            case lastSeen = "last_seen"
            case sessionRefs = "session_refs"
        }

        public init(from decoder: Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            windows = try values.decodeIfPresent([SkillsUsageWindow].self, forKey: .windows) ?? []
            daily = try values.decodeIfPresent([SkillsUsageDay].self, forKey: .daily) ?? []
            undatedEvents = try values.decodeIfPresent(Int.self, forKey: .undatedEvents) ?? 0
            name = try values.decode(String.self, forKey: .name)
            agent = try values.decode(String.self, forKey: .agent)
            invocations = try values.decode(Int.self, forKey: .invocations)
            loads = try values.decode(Int.self, forKey: .loads)
            sessions = try values.decode(Int.self, forKey: .sessions)
            lastSeen = try values.decodeIfPresent(String.self, forKey: .lastSeen)
            sessionRefs = try values.decodeIfPresent([String].self, forKey: .sessionRefs) ?? []
        }
    }

}

extension DaemonData {
    public struct SkillInstallationDetails: Codable, Equatable, Sendable {
        public let aliases: [String]
        public let availability: String
        public let definitionStatus: String
        enum CodingKeys: String, CodingKey {
            case aliases, availability
            case definitionStatus = "definition_status"
        }
    }
    public struct SkillsUsageWindow: Codable, Equatable, Sendable {
        public let days: Int
        public let invocations: Int
        public let loads: Int
        public let sessions: Int
        public init(days: Int, invocations: Int, loads: Int, sessions: Int) {
            self.days = days; self.invocations = invocations; self.loads = loads; self.sessions = sessions
        }
    }
    public struct SkillsUsageDay: Codable, Equatable, Sendable {
        public let day: String
        public let invocations: Int
        public let loads: Int
        public let sessions: Int
    }
}


extension DaemonData {
    public struct SkillDefinition: Decodable, Sendable {
        public let skillID: String
        public let status: String
        public let definition: String?
        enum CodingKeys: String, CodingKey { case skillID = "skill_id", status, definition }
    }
    public struct SkillEvidence: Decodable, Sendable {
        public let enabled: Bool
        public let readable: Bool?
        public let session: Session?
        public struct Session: Decodable, Sendable {
            public let source: String?
            public let date: String?
            public let turns: UInt64?
            public let tokens: UInt64?
        }
    }
}
