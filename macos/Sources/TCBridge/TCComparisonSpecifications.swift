import Foundation

private struct ComparisonDynamicKey: CodingKey, Hashable {
    let stringValue: String
    init?(stringValue: String) { self.stringValue = stringValue }
    let intValue: Int? = nil
    init?(intValue: Int) { return nil }
}

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
    public static let assessed: [Self] = [.accepted, .partial, .rejected]
}
public struct CutoffTaskEvidence: Decodable, Sendable, Equatable, Identifiable {
    public let task_id: String; public let material_revision: UInt64
    public let material_digest, substantive_material_digest: String; public let outcome: DescriptiveOutcome
    public let outcome_recorded_at: String?; public var id: String { task_id }
}
public struct QualifiedExactEstimatorState: Decodable, Sendable, Equatable {
    public let method, protocol_sha256, contrast_orientation: String
    public let outcome_order: [String]
    public let interval_grid_denominator, one_sided_tail_denominator: UInt32
    public let minimum_assessed_tasks_per_cohort, maximum_total_assessed_tasks: UInt32
    public let maximum_interval_width_millionths: UInt32
    private enum Keys: String, CodingKey, CaseIterable {
        case method, protocol_sha256, contrast_orientation, outcome_order, interval_grid_denominator
        case one_sided_tail_denominator, minimum_assessed_tasks_per_cohort
        case maximum_total_assessed_tasks, maximum_interval_width_millionths
    }
    public init(from decoder: Decoder) throws {
        let dynamic = try decoder.container(keyedBy: ComparisonDynamicKey.self)
        guard Set(dynamic.allKeys.map(\.stringValue)) == Set(Keys.allCases.map(\.rawValue)) else {
            throw InsightsError.invalidResponse
        }
        let values = try decoder.container(keyedBy: Keys.self)
        method = try values.decode(String.self, forKey: .method)
        protocol_sha256 = try values.decode(String.self, forKey: .protocol_sha256)
        contrast_orientation = try values.decode(String.self, forKey: .contrast_orientation)
        outcome_order = try values.decode([String].self, forKey: .outcome_order)
        interval_grid_denominator = try values.decode(UInt32.self, forKey: .interval_grid_denominator)
        one_sided_tail_denominator = try values.decode(UInt32.self, forKey: .one_sided_tail_denominator)
        minimum_assessed_tasks_per_cohort = try values.decode(UInt32.self, forKey: .minimum_assessed_tasks_per_cohort)
        maximum_total_assessed_tasks = try values.decode(UInt32.self, forKey: .maximum_total_assessed_tasks)
        maximum_interval_width_millionths = try values.decode(UInt32.self, forKey: .maximum_interval_width_millionths)
    }
    public var isSupported: Bool {
        method == "exact_binomial_components_bonferroni_v1"
            && protocol_sha256 == "d257605993aaa94220ce31144203c0c05a28ff6c33cc129d678f90a8986e26c8"
            && contrast_orientation == "second_canonical_cohort_minus_first"
            && outcome_order == ["accepted", "partial", "rejected"]
            && interval_grid_denominator == 1_000_000 && one_sided_tail_denominator == 240
            && minimum_assessed_tasks_per_cohort == 2 && maximum_total_assessed_tasks == 256
            && maximum_interval_width_millionths == 500_000
    }
}
public enum ComparisonEstimatorState: Decodable, Sendable, Equatable {
    case notYetCalibrated
    case qualifiedExactCategoricalV1(QualifiedExactEstimatorState)
    private enum Key: String, CodingKey { case qualified = "qualified_exact_categorical_v1" }
    public init(from decoder: Decoder) throws {
        if let value = try? decoder.singleValueContainer().decode(String.self) {
            guard value == "not_yet_calibrated" else { throw InsightsError.invalidResponse }
            self = .notYetCalibrated; return
        }
        let dynamic = try decoder.container(keyedBy: ComparisonDynamicKey.self)
        guard Set(dynamic.allKeys.map(\.stringValue)) == Set([Key.qualified.rawValue]) else {
            throw InsightsError.invalidResponse
        }
        let values = try decoder.container(keyedBy: Key.self)
        let state = try values.decode(QualifiedExactEstimatorState.self, forKey: .qualified)
        guard state.isSupported else { throw InsightsError.invalidResponse }
        self = .qualifiedExactCategoricalV1(state)
    }
}
public struct ComparisonSpecification: Decodable, Sendable, Equatable, Identifiable {
    public let schema_version: UInt32; public let id, provenance, created_at, evidence_cutoff: String
    public let cutoff_task_evidence: [CutoffTaskEvidence]
    public let category: String; public let cohort_labels: [String]
    public let date_start, date_end: String; public let stratum: ExactComparisonStratum
    public let task_rule, attempt_rule, outcome_rubric_version: String
    public let estimands: [String]; public let estimator_state: ComparisonEstimatorState
    public let specification_digest, saved_record_digest: String
    public func validateStructure() throws {
        guard schema_version == 1, LocalComparisonTask.uuid(id), provenance == "retrospective_user_specification",
              category == "refactor", cohort_labels.count == 2, cohort_labels == cohort_labels.sorted(),
              Set(cohort_labels).count == 2, date_start <= date_end,
              LocalComparisonTask.uuid(stratum.project_id), LocalComparisonTask.digest(stratum.configuration_fingerprint),
              LocalComparisonTask.digest(specification_digest), LocalComparisonTask.digest(saved_record_digest),
              task_rule == "one-user-confirmed-work-item-v1", attempt_rule == "canonical-snapshot-union-v1",
              outcome_rubric_version == "categorical-user-report-v1",
              estimands == ["categorical-outcome-distribution-v1"]
        else { throw InsightsError.invalidResponse }
        switch estimator_state {
        case .notYetCalibrated: break
        case .qualifiedExactCategoricalV1(let state): guard state.isSupported else {
            throw InsightsError.invalidResponse
        }
        }
        guard Set(cutoff_task_evidence.map(\.task_id)).count == cutoff_task_evidence.count,
              cutoff_task_evidence.allSatisfy({ LocalComparisonTask.uuid($0.task_id)
                  && LocalComparisonTask.digest($0.material_digest)
                  && LocalComparisonTask.digest($0.substantive_material_digest) })
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
public enum ExactCandidateDecision: String, Decodable, Sendable, Equatable {
    case insufficientPrecision = "insufficient_precision"
    case indeterminateBoundary = "indeterminate_boundary"
    case excludesZero = "excludes_zero"
    case includesZero = "includes_zero"
}
public struct ExactCandidateComponentInterval: Decodable, Sendable, Equatable {
    public let lower_millionths, upper_millionths: UInt32
    private enum Keys: String, CodingKey, CaseIterable { case lower_millionths, upper_millionths }
    public init(from decoder: Decoder) throws {
        try Self.requireKeys(decoder, Keys.allCases.map(\.rawValue))
        let values = try decoder.container(keyedBy: Keys.self)
        lower_millionths = try values.decode(UInt32.self, forKey: .lower_millionths)
        upper_millionths = try values.decode(UInt32.self, forKey: .upper_millionths)
    }
    private static func requireKeys(_ decoder: Decoder, _ expected: [String]) throws {
        let dynamic = try decoder.container(keyedBy: ComparisonDynamicKey.self)
        guard Set(dynamic.allKeys.map(\.stringValue)) == Set(expected) else {
            throw InsightsError.invalidResponse
        }
    }
}
public struct ExactCandidateContrast: Decodable, Sendable, Equatable {
    public let lower_millionths, upper_millionths, width_millionths: Int64
    public let decision: ExactCandidateDecision
    private enum Keys: String, CodingKey, CaseIterable {
        case lower_millionths, upper_millionths, width_millionths, decision
    }
    public init(from decoder: Decoder) throws {
        let dynamic = try decoder.container(keyedBy: ComparisonDynamicKey.self)
        guard Set(dynamic.allKeys.map(\.stringValue)) == Set(Keys.allCases.map(\.rawValue)) else {
            throw InsightsError.invalidResponse
        }
        let values = try decoder.container(keyedBy: Keys.self)
        lower_millionths = try values.decode(Int64.self, forKey: .lower_millionths)
        upper_millionths = try values.decode(Int64.self, forKey: .upper_millionths)
        width_millionths = try values.decode(Int64.self, forKey: .width_millionths)
        decision = try values.decode(ExactCandidateDecision.self, forKey: .decision)
    }
}
public enum ExactCandidateEvaluation: Decodable, Sendable, Equatable {
    case supported(first: [ExactCandidateComponentInterval], second: [ExactCandidateComponentInterval],
                   contrasts: [ExactCandidateContrast])
    case suppressedBelowMinimumCohortSupport
    private enum Keys: String, CodingKey { case status, first_components, second_components, contrasts }
    private enum Status: String, Decodable { case supported; case suppressed = "suppressed_below_minimum_cohort_support" }
    public init(from decoder: Decoder) throws {
        let dynamic = try decoder.container(keyedBy: ComparisonDynamicKey.self)
        let keys = Set(dynamic.allKeys.map(\.stringValue))
        let values = try decoder.container(keyedBy: Keys.self)
        switch try values.decode(Status.self, forKey: .status) {
        case .suppressed:
            guard keys == Set([Keys.status.rawValue]) else { throw InsightsError.invalidResponse }
            self = .suppressedBelowMinimumCohortSupport
        case .supported:
            guard keys == Set([Keys.status, .first_components, .second_components, .contrasts].map(\.rawValue)) else {
                throw InsightsError.invalidResponse
            }
            let first = try values.decode([ExactCandidateComponentInterval].self, forKey: .first_components)
            let second = try values.decode([ExactCandidateComponentInterval].self, forKey: .second_components)
            let contrasts = try values.decode([ExactCandidateContrast].self, forKey: .contrasts)
            guard first.count == 3, second.count == 3, contrasts.count == 3 else {
                throw InsightsError.invalidResponse
            }
            self = .supported(first: first, second: second, contrasts: contrasts)
        }
    }
}
public struct AssessedCategoricalCounts: Decodable, Sendable, Equatable {
    public let accepted, partial, rejected, total: UInt64
    private enum Keys: String, CodingKey, CaseIterable { case accepted, partial, rejected, total }
    public init(from decoder: Decoder) throws {
        let dynamic = try decoder.container(keyedBy: ComparisonDynamicKey.self)
        guard Set(dynamic.allKeys.map(\.stringValue)) == Set(Keys.allCases.map(\.rawValue)) else {
            throw InsightsError.invalidResponse
        }
        let values = try decoder.container(keyedBy: Keys.self)
        accepted = try values.decode(UInt64.self, forKey: .accepted)
        partial = try values.decode(UInt64.self, forKey: .partial)
        rejected = try values.decode(UInt64.self, forKey: .rejected)
        total = try values.decode(UInt64.self, forKey: .total)
    }
}
public struct QualifiedExactEstimation: Decodable, Sendable, Equatable {
    public let cohort_labels: [String]
    public let assessed_counts: [AssessedCategoricalCounts]
    public let assessed_estimation_input_digest: String
    public let evaluation: ExactCandidateEvaluation
    public let output_digest: String
    private enum Keys: String, CodingKey, CaseIterable {
        case cohort_labels, assessed_counts, assessed_estimation_input_digest, evaluation, output_digest
    }
    public init(from decoder: Decoder) throws {
        let dynamic = try decoder.container(keyedBy: ComparisonDynamicKey.self)
        guard Set(dynamic.allKeys.map(\.stringValue)) == Set(Keys.allCases.map(\.rawValue)) else {
            throw InsightsError.invalidResponse
        }
        let values = try decoder.container(keyedBy: Keys.self)
        cohort_labels = try values.decode([String].self, forKey: .cohort_labels)
        assessed_counts = try values.decode([AssessedCategoricalCounts].self, forKey: .assessed_counts)
        assessed_estimation_input_digest = try values.decode(String.self, forKey: .assessed_estimation_input_digest)
        evaluation = try values.decode(ExactCandidateEvaluation.self, forKey: .evaluation)
        output_digest = try values.decode(String.self, forKey: .output_digest)
    }
}
public struct DescriptiveComparisonResult: Decodable, Sendable, Equatable {
    public let schema_version: UInt32; public let specification_id, specification_digest: String
    public let specification_record_digest, audit_digest, estimation_input_digest: String
    public let included_task_ids: [String]; public let excluded_tasks: [ExcludedComparisonTask]
    public let cohorts: [CohortDescriptiveResult]
    public let exact_estimation: QualifiedExactEstimation?
    public func validateStructure(expectedSpecification: ComparisonSpecification? = nil) throws {
        guard [1, 2].contains(schema_version), LocalComparisonTask.uuid(specification_id),
              [specification_digest, specification_record_digest, audit_digest, estimation_input_digest]
                .allSatisfy(LocalComparisonTask.digest),
              Set(included_task_ids).count == included_task_ids.count,
              included_task_ids.allSatisfy(LocalComparisonTask.uuid),
              Set(excluded_tasks.map(\.id)).count == excluded_tasks.count,
              excluded_tasks.allSatisfy({ LocalComparisonTask.uuid($0.task_id) && !$0.reasons.isEmpty }),
              Set(cohorts.map(\.id)).count == cohorts.count,
              expectedSpecification == nil || (expectedSpecification?.id == specification_id
                && expectedSpecification?.specification_digest == specification_digest
                && expectedSpecification?.saved_record_digest == specification_record_digest)
        else { throw InsightsError.invalidResponse }
        if schema_version == 1 {
            guard exact_estimation == nil,
                  expectedSpecification.map({ if case .notYetCalibrated = $0.estimator_state { true } else { false } }) ?? true
            else { throw InsightsError.invalidResponse }
        } else {
            guard let exact = exact_estimation, cohorts.count == 2,
                  exact.cohort_labels == cohorts.map(\.cohort_label), exact.assessed_counts.count == 2,
                  LocalComparisonTask.digest(exact.assessed_estimation_input_digest),
                  LocalComparisonTask.digest(exact.output_digest),
                  included_task_ids.count + excluded_tasks.count <= 256,
                  expectedSpecification.map({ spec in
                      spec.cohort_labels == exact.cohort_labels
                          && { if case .qualifiedExactCategoricalV1 = spec.estimator_state { true } else { false } }()
                  }) ?? true
            else { throw InsightsError.invalidResponse }
        }
        guard cohorts.allSatisfy({ cohort in
            guard let total = Self.checkedSum([cohort.outcomes.accepted, cohort.outcomes.partial,
                cohort.outcomes.rejected, cohort.outcomes.pending, cohort.outcomes.unknown,
                cohort.outcomes.unassessed]),
                let assessed = Self.checkedSum([cohort.outcomes.accepted, cohort.outcomes.partial,
                                                cohort.outcomes.rejected]) else { return false }
            return cohort.included_tasks == total && cohort.outcomes.assessed == assessed
        }) else { throw InsightsError.invalidResponse }
        guard let included = Self.checkedSum(cohorts.map(\.included_tasks)),
              included == UInt64(included_task_ids.count) else { throw InsightsError.invalidResponse }
        if let exact = exact_estimation {
            guard let assessed = Self.checkedSum(cohorts.map(\.outcomes.assessed)), assessed <= 256 else {
                throw InsightsError.invalidResponse
            }
            guard zip(exact.assessed_counts, cohorts).allSatisfy({ counts, cohort in
                counts.accepted == cohort.outcomes.accepted && counts.partial == cohort.outcomes.partial
                    && counts.rejected == cohort.outcomes.rejected && counts.total == cohort.outcomes.assessed
            }) else { throw InsightsError.invalidResponse }
            switch exact.evaluation {
            case .suppressedBelowMinimumCohortSupport:
                guard exact.assessed_counts.contains(where: { $0.total < 2 }) else {
                    throw InsightsError.invalidResponse
                }
            case .supported(let first, let second, let contrasts):
                guard exact.assessed_counts.allSatisfy({ $0.total >= 2 }),
                      zip(zip(first, second), contrasts).allSatisfy({ pair, contrast in
                          let (firstInterval, secondInterval) = pair
                          guard firstInterval.lower_millionths <= firstInterval.upper_millionths,
                                secondInterval.lower_millionths <= secondInterval.upper_millionths,
                                firstInterval.upper_millionths <= 1_000_000,
                                secondInterval.upper_millionths <= 1_000_000 else { return false }
                          let lower = Int64(secondInterval.lower_millionths) - Int64(firstInterval.upper_millionths)
                          let upper = Int64(secondInterval.upper_millionths) - Int64(firstInterval.lower_millionths)
                          let width = upper - lower
                          let decision: ExactCandidateDecision = if width > 500_000 { .insufficientPrecision }
                              else if width == 500_000 || lower == 0 || upper == 0 { .indeterminateBoundary }
                              else if upper < 0 || lower > 0 { .excludesZero } else { .includesZero }
                          return contrast.lower_millionths == lower && contrast.upper_millionths == upper
                              && contrast.width_millionths == width && contrast.decision == decision
                      }) else { throw InsightsError.invalidResponse }
            }
        }
    }
    private static func checkedSum(_ values: [UInt64]) -> UInt64? {
        values.reduce(Optional(UInt64.zero)) { partial, value in
            guard let partial else { return nil }
            let (sum, overflow) = partial.addingReportingOverflow(value)
            return overflow ? nil : sum
        }
    }
}
