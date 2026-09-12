import Foundation

public struct ExactComparisonStratum: Codable, Sendable, Equatable, Hashable {
    public let project_id, language, configuration_fingerprint: String
    public init(projectID: String, language: String, configurationFingerprint: String) {
        project_id = projectID; self.language = language; configuration_fingerprint = configurationFingerprint
    }
}
public struct ComparisonSpecificationDraftInput: Codable, Sendable, Equatable {
    public let evidence_cutoff: String
    public let cohort_labels: [String]
    public let date_start, date_end: String
    public let stratum: ExactComparisonStratum
    public init(evidenceCutoff: String, cohortLabels: [String], dateStart: String, dateEnd: String,
                stratum: ExactComparisonStratum) {
        evidence_cutoff = evidenceCutoff; cohort_labels = cohortLabels.sorted()
        date_start = dateStart; date_end = dateEnd; self.stratum = stratum
    }
}
public enum DescriptiveOutcome: String, Decodable, Sendable, CaseIterable {
    case pending, accepted, partial, rejected, unknown, unassessed
}
public struct CutoffTaskEvidence: Decodable, Sendable, Equatable, Identifiable {
    public let task_id: String; public let material_revision: UInt64
    public let material_digest, substantive_material_digest: String; public let outcome: DescriptiveOutcome
    public let outcome_recorded_at: String?; public var id: String { task_id }
}
public struct ComparisonSpecification: Decodable, Sendable, Equatable, Identifiable {
    public let schema_version: UInt32; public let id, provenance, created_at, evidence_cutoff: String
    public let cutoff_task_evidence: [CutoffTaskEvidence]
    public let category: String; public let cohort_labels: [String]
    public let date_start, date_end: String; public let stratum: ExactComparisonStratum
    public let task_rule, attempt_rule, outcome_rubric_version: String
    public let estimands: [String]; public let estimator_state: String
    public let specification_digest, saved_record_digest: String
    public func validateStructure() throws {
        guard schema_version == 1, LocalComparisonTask.uuid(id), provenance == "retrospective_user_specification",
              category == "refactor", cohort_labels.count == 2, cohort_labels == cohort_labels.sorted(),
              Set(cohort_labels).count == 2, date_start <= date_end,
              LocalComparisonTask.uuid(stratum.project_id), LocalComparisonTask.digest(stratum.configuration_fingerprint),
              LocalComparisonTask.digest(specification_digest), LocalComparisonTask.digest(saved_record_digest),
              task_rule == "one-user-confirmed-work-item-v1", attempt_rule == "canonical-snapshot-union-v1",
              outcome_rubric_version == "categorical-user-report-v1",
              estimands == ["categorical-outcome-distribution-v1"], estimator_state == "not_yet_calibrated"
        else { throw InsightsError.invalidResponse }
    }
}
public enum ComparisonExclusionReason: String, Decodable, Sendable, CaseIterable {
    case evidenceAfterCutoff = "evidence_after_cutoff", cutoffTimeUnavailable = "cutoff_time_unavailable"
    case categoryMismatch = "category_mismatch", dateOutsideWindow = "date_outside_window"
    case stratumMismatch = "stratum_mismatch", contextUnavailable = "context_unavailable"
    case cohortUnavailable = "cohort_unavailable", cohortNotSelected = "cohort_not_selected"
    case evidenceStale = "evidence_stale", independenceUnconfirmed = "independence_unconfirmed"
    case overlappingTaskEvidence = "overlapping_task_evidence"
    case sourceAttributionUnavailable = "source_attribution_unavailable"
    case sourceAttributionStale = "source_attribution_stale"
}
public struct ExcludedComparisonTask: Decodable, Sendable, Equatable, Identifiable {
    public let task_id: String; public let reasons: [ComparisonExclusionReason]; public var id: String { task_id }
}
public struct ComparisonOutcomeCounts: Decodable, Sendable, Equatable {
    public let accepted, partial, rejected, pending, unknown, unassessed, assessed: UInt64
}
public struct ComparisonUsageCoverage: Decodable, Sendable, Equatable {
    public let tasks_with_observed_attributed_tokens, tasks_without_observed_attributed_tokens: UInt64
    public let observed_attributed_tokens: UInt64
}
public struct CohortDescriptiveResult: Decodable, Sendable, Equatable, Identifiable {
    public let cohort_label: String; public let included_tasks: UInt64
    public let outcomes: ComparisonOutcomeCounts; public let usage: ComparisonUsageCoverage
    public var id: String { cohort_label }
}
public struct DescriptiveComparisonResult: Decodable, Sendable, Equatable {
    public let schema_version: UInt32; public let specification_id, specification_digest: String
    public let specification_record_digest, audit_digest, estimation_input_digest: String
    public let included_task_ids: [String]; public let excluded_tasks: [ExcludedComparisonTask]
    public let cohorts: [CohortDescriptiveResult]
    public func validateStructure(expectedSpecification: ComparisonSpecification? = nil) throws {
        guard schema_version == 1, LocalComparisonTask.uuid(specification_id),
              [specification_digest, specification_record_digest, audit_digest, estimation_input_digest]
                .allSatisfy(LocalComparisonTask.digest),
              Set(included_task_ids).count == included_task_ids.count,
              Set(excluded_tasks.map(\.id)).count == excluded_tasks.count,
              Set(cohorts.map(\.id)).count == cohorts.count,
              expectedSpecification == nil || (expectedSpecification?.id == specification_id
                && expectedSpecification?.specification_digest == specification_digest
                && expectedSpecification?.saved_record_digest == specification_record_digest)
        else { throw InsightsError.invalidResponse }
    }
}
