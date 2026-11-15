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
        /// Reporting public skills the catalog lacks. Absent from older
        /// daemons, which read as unavailable.
        public let suggestions: SkillsSuggestions

        public init(enabled: Bool = false, readable: Bool = true, refreshing: Bool = false, error: String? = nil, inventory: SkillsInventory = .init(), catalog: SkillsCatalog = .init(), followedPublishers: [String] = [], updates: [SkillsUpdate] = [], feed: [SkillsFeedEvent] = [], feedPartial: Bool = false, usage: SkillsUsage = .init(), copy: [String: String] = [:], suggestions: SkillsSuggestions = .init()) {
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
            self.suggestions = suggestions
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
            case suggestions
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
            suggestions = (try? values.decodeIfPresent(SkillsSuggestions.self, forKey: .suggestions)) ?? .init()
        }
    }

    /// Whether the app may offer reporting public skills the catalog lacks,
    /// and the user's own choice. Never lists a skill.
    public struct SkillsSuggestions: Codable, Equatable, Sendable {
        public let available: Bool
        public let optedIn: Bool
        public let pending: Int
        public let sentTotal: Int
        public let lastSentAt: String?

        public init(available: Bool = false, optedIn: Bool = false, pending: Int = 0, sentTotal: Int = 0, lastSentAt: String? = nil) {
            self.available = available
            self.optedIn = optedIn
            self.pending = pending
            self.sentTotal = sentTotal
            self.lastSentAt = lastSentAt
        }

        enum CodingKeys: String, CodingKey {
            case available
            case optedIn = "opted_in"
            case pending
            case sentTotal = "sent_total"
            case lastSentAt = "last_sent_at"
        }

        public init(from decoder: Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            available = try values.decodeIfPresent(Bool.self, forKey: .available) ?? false
            optedIn = try values.decodeIfPresent(Bool.self, forKey: .optedIn) ?? false
            pending = try values.decodeIfPresent(Int.self, forKey: .pending) ?? 0
            sentTotal = try values.decodeIfPresent(Int.self, forKey: .sentTotal) ?? 0
            lastSentAt = try values.decodeIfPresent(String.self, forKey: .lastSentAt)
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
        /// `server` (the signed server catalog) or `github` (the four fixed
        /// publishers). Nil from older daemons.
        public let source: String?
        /// The server catalog's identity; nil for `github`.
        public let revision: Int?
        public let issuedAt: String?
        public let expiresAt: String?
        /// Skills in the whole server catalog; `skills` may hold fewer.
        public let totalSkills: Int?

        public init(publishers: [SkillsPublisher] = [], skills: [CatalogSkill] = [], checkedAt: String? = nil, publisherStates: [SkillsPublisherState] = [], partial: Bool = false, source: String? = nil, revision: Int? = nil, issuedAt: String? = nil, expiresAt: String? = nil, totalSkills: Int? = nil) {
            self.publishers = publishers
            self.skills = skills
            self.checkedAt = checkedAt
            self.publisherStates = publisherStates
            self.partial = partial
            self.source = source
            self.revision = revision
            self.issuedAt = issuedAt
            self.expiresAt = expiresAt
            self.totalSkills = totalSkills
        }

        enum CodingKeys: String, CodingKey {
            case publishers
            case skills
            case checkedAt = "checked_at"
            case publisherStates = "publisher_states"
            case partial
            case source
            case revision
            case issuedAt = "issued_at"
            case expiresAt = "expires_at"
            case totalSkills = "total_skills"
        }

        public init(from decoder: Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            publishers = try values.decodeIfPresent([SkillsPublisher].self, forKey: .publishers) ?? []
            skills = try values.decodeIfPresent([CatalogSkill].self, forKey: .skills) ?? []
            checkedAt = try values.decodeIfPresent(String.self, forKey: .checkedAt)
            publisherStates = try values.decodeIfPresent([SkillsPublisherState].self, forKey: .publisherStates) ?? []
            partial = try values.decodeIfPresent(Bool.self, forKey: .partial) ?? false
            source = try values.decodeIfPresent(String.self, forKey: .source)
            revision = try values.decodeIfPresent(Int.self, forKey: .revision)
            issuedAt = try values.decodeIfPresent(String.self, forKey: .issuedAt)
            expiresAt = try values.decodeIfPresent(String.self, forKey: .expiresAt)
            totalSkills = try values.decodeIfPresent(Int.self, forKey: .totalSkills)
        }
    }

    public struct SkillsPublisher: Codable, Equatable, Sendable, Identifiable {
        public let id: String
        public let name: String
        public let repository: String
        public let skillsRoot: String
        public let instructionsURL: String
        /// The repository page; server catalog publishers only.
        public let url: String?
        /// `seed`, `list`, `marketplace` or `suggested`; server catalog only.
        public let sourceKind: String?

        public init(id: String, name: String, repository: String, skillsRoot: String, instructionsURL: String, url: String? = nil, sourceKind: String? = nil) {
            self.id = id
            self.name = name
            self.repository = repository
            self.skillsRoot = skillsRoot
            self.instructionsURL = instructionsURL
            self.url = url
            self.sourceKind = sourceKind
        }

        enum CodingKeys: String, CodingKey {
            case id
            case name
            case repository
            case skillsRoot = "skills_root"
            case instructionsURL = "instructions_url"
            case url
            case sourceKind = "source_kind"
        }

        public init(from decoder: Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            id = try values.decode(String.self, forKey: .id)
            name = try values.decode(String.self, forKey: .name)
            repository = try values.decode(String.self, forKey: .repository)
            skillsRoot = try values.decode(String.self, forKey: .skillsRoot)
            instructionsURL = try values.decode(String.self, forKey: .instructionsURL)
            url = try values.decodeIfPresent(String.self, forKey: .url)
            sourceKind = try values.decodeIfPresent(String.self, forKey: .sourceKind)
        }
    }

    public struct SkillsPublisherState: Codable, Equatable, Sendable {
        public let publisherID: String
        public let status: String
        public let checkedAt: String?
        /// Public repository signals from the server catalog; nil when the
        /// daemon knows none (GitHub path, older daemons).
        public let signals: SkillsRepositorySignals?

        public init(publisherID: String, status: String, checkedAt: String? = nil, signals: SkillsRepositorySignals? = nil) {
            self.publisherID = publisherID
            self.status = status
            self.checkedAt = checkedAt
            self.signals = signals
        }

        enum CodingKeys: String, CodingKey {
            case publisherID = "publisher_id"
            case status
            case checkedAt = "checked_at"
            case signals
        }

        public init(from decoder: Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            publisherID = try values.decode(String.self, forKey: .publisherID)
            status = try values.decode(String.self, forKey: .status)
            checkedAt = try values.decodeIfPresent(String.self, forKey: .checkedAt)
            // Decorative data: a malformed value is unknown and never fails the snapshot.
            signals = (try? values.decodeIfPresent(SkillsRepositorySignals.self, forKey: .signals)) ?? nil
        }
    }

    /// Anonymous public GitHub signals for a publisher's whole repository,
    /// as the server catalog measured them. Every field is optional: nil is
    /// unknown, never zero.
    public struct SkillsRepositorySignals: Codable, Equatable, Sendable {
        public let stars: Int?
        /// Latest push to any branch.
        public let pushedAt: String?
        public let repositoryCheckedAt: String?
        /// Default-branch commits in the last week and the last four weeks.
        public let commits7d: Int?
        public let commits28d: Int?
        public let activityCheckedAt: String?

        public init(stars: Int? = nil, pushedAt: String? = nil, repositoryCheckedAt: String? = nil, commits7d: Int? = nil, commits28d: Int? = nil, activityCheckedAt: String? = nil) {
            self.stars = stars
            self.pushedAt = pushedAt
            self.repositoryCheckedAt = repositoryCheckedAt
            self.commits7d = commits7d
            self.commits28d = commits28d
            self.activityCheckedAt = activityCheckedAt
        }

        enum CodingKeys: String, CodingKey {
            case stars
            case pushedAt = "pushed_at"
            case repositoryCheckedAt = "repository_checked_at"
            case commits7d = "commits_7d"
            case commits28d = "commits_28d"
            case activityCheckedAt = "activity_checked_at"
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
        /// Facts the server catalog publishes; nil from the GitHub path and
        /// from older daemons. Facts, never a verdict.
        public let license: SkillsLicense?
        public let firstSeenAt: String?
        public let lastChangedAt: String?
        public let checks: SkillsChecks?
        public let checksVersion: Int?

        public init(id: String, publisherID: String, name: String, description: String, repository: String, path: String, revision: String, sha256: String, sourceURL: String, instructionsURL: String, license: SkillsLicense? = nil, firstSeenAt: String? = nil, lastChangedAt: String? = nil, checks: SkillsChecks? = nil, checksVersion: Int? = nil) {
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
            self.license = license
            self.firstSeenAt = firstSeenAt
            self.lastChangedAt = lastChangedAt
            self.checks = checks
            self.checksVersion = checksVersion
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
            case license
            case firstSeenAt = "first_seen_at"
            case lastChangedAt = "last_changed_at"
            case checks
            case checksVersion = "checks_version"
        }

        public init(from decoder: Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            id = try values.decode(String.self, forKey: .id)
            publisherID = try values.decode(String.self, forKey: .publisherID)
            name = try values.decode(String.self, forKey: .name)
            description = try values.decode(String.self, forKey: .description)
            repository = try values.decode(String.self, forKey: .repository)
            path = try values.decode(String.self, forKey: .path)
            revision = try values.decode(String.self, forKey: .revision)
            sha256 = try values.decode(String.self, forKey: .sha256)
            sourceURL = try values.decode(String.self, forKey: .sourceURL)
            instructionsURL = try values.decode(String.self, forKey: .instructionsURL)
            // Facts are optional: a malformed one is unknown and never fails
            // the snapshot.
            license = (try? values.decodeIfPresent(SkillsLicense.self, forKey: .license)) ?? nil
            firstSeenAt = (try? values.decodeIfPresent(String.self, forKey: .firstSeenAt)) ?? nil
            lastChangedAt = (try? values.decodeIfPresent(String.self, forKey: .lastChangedAt)) ?? nil
            checks = (try? values.decodeIfPresent(SkillsChecks.self, forKey: .checks)) ?? nil
            checksVersion = (try? values.decodeIfPresent(Int.self, forKey: .checksVersion)) ?? nil
        }
    }

    /// A catalog skill's licence as the server found it: `declared` (with
    /// an SPDX identifier when one was given), `absent`, or `unknown`.
    public struct SkillsLicense: Codable, Equatable, Sendable {
        public let status: String
        public let spdx: String?

        public init(status: String, spdx: String? = nil) {
            self.status = status
            self.spdx = spdx
        }
    }

    /// What the server's automatic checks observed about a published
    /// skill. Facts to show, never a "safe" verdict.
    public struct SkillsChecks: Codable, Equatable, Sendable {
        public let frontmatter: String
        public let documentBytes: Int
        public let directoryBytes: Int
        public let directoryFiles: Int
        public let bundled: SkillsBundledFiles
        /// Instruction-pattern labels, such as `pipe_to_shell`.
        public let patterns: [String]
        /// No push to the repository for a year.
        public let inactive: Bool

        public init(frontmatter: String = "valid", documentBytes: Int = 0, directoryBytes: Int = 0, directoryFiles: Int = 0, bundled: SkillsBundledFiles = .init(), patterns: [String] = [], inactive: Bool = false) {
            self.frontmatter = frontmatter
            self.documentBytes = documentBytes
            self.directoryBytes = directoryBytes
            self.directoryFiles = directoryFiles
            self.bundled = bundled
            self.patterns = patterns
            self.inactive = inactive
        }

        enum CodingKeys: String, CodingKey {
            case frontmatter
            case documentBytes = "document_bytes"
            case directoryBytes = "directory_bytes"
            case directoryFiles = "directory_files"
            case bundled
            case patterns
            case inactive
        }
    }

    public struct SkillsBundledFiles: Codable, Equatable, Sendable {
        public let scripts: Int
        public let executables: Int
        public let binaries: Int

        public init(scripts: Int = 0, executables: Int = 0, binaries: Int = 0) {
            self.scripts = scripts
            self.executables = executables
            self.binaries = binaries
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
        public let delegatedSessionsObserved: Int?
        public let delegatedSessionsUnknown: Int?
        public let unclassifiedSessionsObserved: Int?
        public let unclassifiedSessionsUnknown: Int?
        public let partial: Bool
        public let skills: [SkillsUsageRow]
        public let coverage: [SkillsUsageCoverage]

        public init(enabled: Bool = false, lastAnalysisAt: String? = nil, earliestEvidence: String? = nil, sessionsPartial: Int = 0, sessionsObserved: Int = 0, sessionsUnknown: Int = 0, partial: Bool = true, skills: [SkillsUsageRow] = [], coverage: [SkillsUsageCoverage] = [], delegatedSessionsObserved: Int? = nil, delegatedSessionsUnknown: Int? = nil, unclassifiedSessionsObserved: Int? = nil, unclassifiedSessionsUnknown: Int? = nil) {
            self.enabled = enabled
            self.lastAnalysisAt = lastAnalysisAt
            self.earliestEvidence = earliestEvidence
            self.sessionsPartial = sessionsPartial
            self.sessionsObserved = sessionsObserved
            self.sessionsUnknown = sessionsUnknown
            self.delegatedSessionsObserved = delegatedSessionsObserved
            self.delegatedSessionsUnknown = delegatedSessionsUnknown
            self.unclassifiedSessionsObserved = unclassifiedSessionsObserved
            self.unclassifiedSessionsUnknown = unclassifiedSessionsUnknown
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
            case delegatedSessionsObserved = "delegated_sessions_observed"
            case delegatedSessionsUnknown = "delegated_sessions_unknown"
            case unclassifiedSessionsObserved = "unclassified_sessions_observed"
            case unclassifiedSessionsUnknown = "unclassified_sessions_unknown"
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
            delegatedSessionsObserved = try values.decodeIfPresent(Int.self, forKey: .delegatedSessionsObserved)
            delegatedSessionsUnknown = try values.decodeIfPresent(Int.self, forKey: .delegatedSessionsUnknown)
            unclassifiedSessionsObserved = try values.decodeIfPresent(Int.self, forKey: .unclassifiedSessionsObserved)
            unclassifiedSessionsUnknown = try values.decodeIfPresent(Int.self, forKey: .unclassifiedSessionsUnknown)
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
        public let population: String
        public let evidence: [SkillsUsageEvidence]
        public let parentSessionRefs: [String]
        public var identity: String { agent + "\u{0}" + population + "\u{0}" + name }

        public init(name: String, agent: String, invocations: Int, loads: Int, sessions: Int, lastSeen: String? = nil, sessionRefs: [String] = [], windows: [SkillsUsageWindow] = [], daily: [SkillsUsageDay] = [], undatedEvents: Int = 0, population: String = "direct", evidence: [SkillsUsageEvidence] = [], parentSessionRefs: [String] = []) {
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
            self.population = population
            self.evidence = evidence
            self.parentSessionRefs = parentSessionRefs
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
            case population, evidence
            case parentSessionRefs = "parent_session_refs"
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
            population = try values.decodeIfPresent(String.self, forKey: .population) ?? "direct"
            evidence = try values.decodeIfPresent([SkillsUsageEvidence].self, forKey: .evidence) ?? []
            parentSessionRefs = try values.decodeIfPresent([String].self, forKey: .parentSessionRefs) ?? []
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
        public let agent: String?
        public let population: String?
        public let skillUsage: Observations?
        enum CodingKeys: String, CodingKey {
            case enabled, readable, session, agent, population
            case skillUsage = "skill_usage"
        }
        public struct Observations: Decodable, Sendable {
            public let coverage: String
            public let partial: Bool
            public let skills: [Count]
        }
        public struct Count: Decodable, Sendable {
            public let name: String
            public let invocations: Int
            public let loads: Int
        }
        public struct Session: Decodable, Sendable {
            public let source: String?
            public let date: String?
            public let turns: UInt64?
            public let tokens: UInt64?
        }
    }
}

extension DaemonData {
    public struct SkillsUsageEvidence: Codable, Equatable, Sendable {
        public let sessionRef: String
        public let parentSessionRef: String?
        public init(sessionRef: String, parentSessionRef: String? = nil) {
            self.sessionRef = sessionRef; self.parentSessionRef = parentSessionRef
        }
        enum CodingKeys: String, CodingKey {
            case sessionRef = "session_ref"
            case parentSessionRef = "parent_session_ref"
        }
    }
}
