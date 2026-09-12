import SwiftUI
import TCBridge

struct ComparisonSpecificationsView: View {
    @Bindable var model: ComparisonSpecificationsModel
    let copy: [String: String]
    let openTask: (String) -> Void
    private func text(_ key: String) -> String { copy[key] ?? "" }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text(text("comparison_specification_title")).font(.title2)
                Spacer(); Button(text("comparison_specification_refresh")) { model.refresh() }
                if model.busy { ProgressView().controlSize(.small) }
            }
            Text(text("comparison_retrospective_notice")).foregroundStyle(.secondary)
            Text(text("comparison_descriptive_notice")).foregroundStyle(.secondary)
            if let notice = model.notice { Text(text(notice)).foregroundStyle(.green) }
            if let error = model.error { Text(text(error)).foregroundStyle(.red) }
            draft
            if let specification = model.previewSpecification, let result = model.previewResult {
                Divider(); Text(text("comparison_preview_notice")).font(.headline)
                specificationView(specification); resultView(result)
                Button(text("comparison_specification_save")) { model.save() }
            }
            Divider(); saved
        }.disabled(model.busy)
    }

    private var draft: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(text("comparison_specification_draft")).font(.headline)
            if model.options.isEmpty { Text(text("comparison_specification_need_context")) }
            Picker(text("comparison_specification_stratum"), selection: Binding(
                get: { model.selectedStratumID }, set: model.selectStratum)) {
                ForEach(model.options) { option in
                    Text("\(option.label) · \(option.taskIDs.count) \(text("comparison_specification_matching_tasks"))")
                        .tag(option.id)
                }
            }
            if let option = model.selectedOption {
                DisclosureGroup(text("comparison_task_advanced_evidence")) {
                    Text(option.stratum.project_id).font(.caption.monospaced())
                    Text(option.stratum.configuration_fingerprint).font(.caption.monospaced())
                }
                Text(text("comparison_specification_candidate_declarations")).font(.headline)
                if option.cohortCandidates.count < 2 {
                    Text(text("comparison_specification_need_cohorts"))
                }
                ForEach(option.cohortCandidates, id: \.self) { label in
                    Toggle(label, isOn: Binding(get: { model.selectedCohorts.contains(label) },
                        set: { model.setCohort(label, selected: $0) })).toggleStyle(.checkbox)
                }
            }
            DatePicker(text("comparison_specification_date_start"), selection: $model.dateStart,
                       displayedComponents: .date)
            DatePicker(text("comparison_specification_date_end"), selection: $model.dateEnd,
                       displayedComponents: .date)
            DatePicker(text("comparison_specification_cutoff"), selection: $model.evidenceCutoff)
            Text(text("comparison_specification_cutoff_notice")).font(.caption).foregroundStyle(.secondary)
            Button(text("comparison_specification_preview")) { model.preview() }.disabled(!model.canDraft)
        }.onChange(of: model.dateStart) { _, _ in model.draftChanged() }
         .onChange(of: model.dateEnd) { _, _ in model.draftChanged() }
         .onChange(of: model.evidenceCutoff) { _, _ in model.draftChanged() }
    }

    private var saved: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(text("comparison_specification_saved")).font(.headline)
            if model.specifications.isEmpty { Text(text("comparison_specifications_empty")) }
            ForEach(model.specifications) { spec in
                Button { model.select(spec.id) } label: {
                    Text("\(spec.date_start) – \(spec.date_end) · \(spec.cohort_labels.joined(separator: " / "))")
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            if let selected = model.selected {
                specificationView(selected)
                Button(text("comparison_specification_evaluate")) { model.evaluate() }
                if let result = model.result {
                    resultView(result)
                    Button(text("comparison_specification_explain_result")) {
                        if let confirmation = model.resultConfirmation() { model.explain(confirmation) }
                    }
                }
            }
        }
    }

    private func specificationView(_ spec: ComparisonSpecification) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            Text("\(spec.date_start) – \(spec.date_end)").font(.headline)
            Text(spec.cohort_labels.joined(separator: " / "))
            Text("\(text("comparison_specification_cutoff")): \(InsightsDate.label(spec.evidence_cutoff))")
            Text(text("comparison_specification_cutoff_evidence")).font(.headline)
            ForEach(spec.cutoff_task_evidence) { evidence in
                Button { openTask(evidence.task_id) } label: {
                    Text("\(model.taskLabel(evidence.task_id)) · \(outcomeLabel(evidence.outcome)) · \(evidence.outcome_recorded_at.map(InsightsDate.label) ?? text("comparison_task_unknown"))")
                }
            }
            DisclosureGroup(text("comparison_task_advanced_evidence")) {
                Text(spec.id); Text(spec.specification_digest); Text(spec.saved_record_digest)
                ForEach(spec.cutoff_task_evidence) { evidence in
                    Text(evidence.task_id); Text(evidence.material_digest); Text(evidence.substantive_material_digest)
                }
            }.font(.caption.monospaced()).textSelection(.enabled)
        }
    }

    private func resultView(_ result: DescriptiveComparisonResult) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(text("comparison_specification_result")).font(.headline)
            if result.included_task_ids.isEmpty { Text(text("comparison_no_eligible_evidence")) }
            ForEach(result.cohorts) { cohort in
                VStack(alignment: .leading) {
                    Text(cohort.cohort_label).font(.headline)
                    Text("\(text("comparison_specification_included")): \(cohort.included_tasks)")
                    ForEach(DescriptiveOutcome.allCases, id: \.self) { outcome in
                        Text("\(outcomeLabel(outcome)): \(count(outcome, in: cohort.outcomes))")
                    }
                    Text("\(text("comparison_specification_assessed")): \(cohort.outcomes.assessed)")
                    Text("\(text("comparison_specification_usage_observed")): \(cohort.usage.tasks_with_observed_attributed_tokens)")
                    Text("\(text("comparison_specification_usage_unavailable")): \(cohort.usage.tasks_without_observed_attributed_tokens)")
                    Text("\(text("comparison_specification_observed_tokens")): \(cohort.usage.observed_attributed_tokens)")
                }.padding(8).background(.quaternary, in: RoundedRectangle(cornerRadius: 8))
            }
            Text(text("comparison_denominator_notice")).font(.caption).foregroundStyle(.secondary)
            if !result.included_task_ids.isEmpty {
                Text(text("comparison_specification_included")).font(.headline)
                ForEach(result.included_task_ids, id: \.self) { id in
                    Button(model.taskLabel(id)) { openTask(id) }
                }
            }
            if !result.excluded_tasks.isEmpty { Text(text("comparison_specification_exclusions")).font(.headline) }
            ForEach(result.excluded_tasks) { excluded in
                VStack(alignment: .leading) {
                    Button(model.taskLabel(excluded.task_id)) { openTask(excluded.task_id) }
                    ForEach(excluded.reasons, id: \.self) { reason in
                        Text(text("comparison_exclusion_\(reason.rawValue)"))
                    }
                }
            }
            DisclosureGroup(text("comparison_task_advanced_evidence")) {
                Text(result.audit_digest); Text(result.estimation_input_digest)
            }.font(.caption.monospaced()).textSelection(.enabled)
        }
    }
    private func count(_ outcome: DescriptiveOutcome, in counts: ComparisonOutcomeCounts) -> UInt64 {
        switch outcome { case .accepted: counts.accepted; case .partial: counts.partial
        case .rejected: counts.rejected; case .pending: counts.pending
        case .unknown: counts.unknown; case .unassessed: counts.unassessed }
    }
    private func outcomeLabel(_ outcome: DescriptiveOutcome) -> String {
        text(outcome == .unassessed ? "comparison_task_outcome_unassessed" : "outcome_\(outcome.rawValue)")
    }
}
