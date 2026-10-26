import SwiftUI
import TCDesign
import TCShellCore

/// The Models card (model answers spec section 8, first version): the
/// daemon's six answers over the last 13 weeks of watched traces (feed T),
/// one row each. A row says which question it answers as a scoped fact
/// about the contributor's own traces, then the leading model and the next
/// one with their figures and trace counts, or why it names none: not
/// enough traces, not priced, or no clear difference. How the figure is
/// measured and the traces left out sit behind the row's "i". Every word is
/// the core's analytics copy (`analytics_models_*`); every figure the
/// daemon's.
struct InsightsModelsCard: View {
    let answers: DaemonData.InsightsModelAnswers
    let copy: [String: String]

    private func text(_ key: String) -> String { copy[key] ?? "" }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            InsightsSectionTitle(text("analytics_card_models"),
                                 info: [text("analytics_models_window"), text("analytics_model_labels_note")])
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                // On the card itself, not only behind the "i", so a
                // screenshot keeps it (spec section 8).
                Text(text("analytics_models_scope")).insightsCaption()
                    .fixedSize(horizontal: false, vertical: true)
                VStack(spacing: 0) {
                    ForEach(Array(InsightsModelsWords.rows(answers, copy: copy).enumerated()),
                            id: \.element.id) { index, row in
                        GlassTableRow(first: index == 0) { InsightsModelRow(row: row) }
                    }
                }
            }
            .insightsCardBody()
        }
        .textSelection(.enabled)
        .frame(maxWidth: .infinity, alignment: .leading)
        .insightsSection()
    }
}

/// One answer: its name with the "i", the fact it answers, its state when
/// it names no model first, its models, and the qualifiers under them.
struct InsightsModelRow: View {
    let row: InsightsModelsWords.Row

    var body: some View {
        VStack(alignment: .leading, spacing: InsightsLayout.lineGap) {
            InsightsGroupHeading(row.title, info: row.info)
            Text(row.lead).insightsCaption()
                .fixedSize(horizontal: false, vertical: true)
            if let state = row.state {
                Text(state).insightsNote()
            }
            ForEach(Array(row.models.enumerated()), id: \.offset) { index, line in
                VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                    if index == 0 && row.state == nil {
                        // The leading model, on an answered row.
                        Text(line.text).foregroundStyle(GlassColor.textPrimary)
                    } else {
                        Text(line.text).insightsNote()
                    }
                    if let detail = line.detail { Text(detail).insightsCaption() }
                }
                .fixedSize(horizontal: false, vertical: true)
            }
            ForEach(row.notes, id: \.self) { note in
                Text(note).insightsCaption()
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// The Models card's words from the daemon's answer, kept apart from the
/// view so each row state can be tested. Lower is ahead on every figure;
/// no word ranks a model beyond the scoped fact the row states.
enum InsightsModelsWords {
    struct Line: Equatable {
        let text: String
        /// Interrupts per trace, beside a rejection figure.
        var detail: String?
    }

    struct Row: Equatable, Identifiable {
        /// The wire's `question`.
        let id: String
        let title: String
        let lead: String
        /// `nil` when the row names a leading model.
        let state: String?
        let models: [Line]
        let notes: [String]
        let info: [String]
    }

    private static func text(_ key: String, _ copy: [String: String]) -> String { copy[key] ?? "" }
    private static func fill(_ key: String, _ copy: [String: String], _ values: [String: String]) -> String {
        InsightsOverviewWords.fill(text(key, copy), values)
    }

    /// The rows this build has words for, in the daemon's order. A
    /// question it does not know is left out, never drawn under a blank
    /// heading.
    static func rows(_ answers: DaemonData.InsightsModelAnswers, copy: [String: String]) -> [Row] {
        (answers.rows ?? []).filter { !text("analytics_models_row_" + $0.question, copy).isEmpty }
            .map { row($0, copy: copy) }
    }

    static func row(_ row: DaemonData.InsightsModelAnswerRow, copy: [String: String]) -> Row {
        Row(id: row.question,
            title: text("analytics_models_row_" + row.question, copy),
            lead: text("analytics_models_lead_" + row.question + (row.basis == "undone" ? "_undone" : ""), copy),
            state: state(row.state, copy: copy),
            models: models(row, copy: copy),
            notes: notes(row, copy: copy),
            info: info(row, copy: copy))
    }

    /// The state's words, or `nil` for an answered row. A state this build
    /// does not know is the dash, with no model under it.
    static func state(_ state: String, copy: [String: String]) -> String? {
        switch state {
        case "answered": return nil
        case "not_enough_sessions": return text("analytics_models_not_enough", copy)
        case "not_priced": return text("analytics_models_not_priced", copy)
        case "no_clear_difference": return text("analytics_models_no_clear_difference", copy)
        default: return text("analytics_unavailable", copy)
        }
    }

    /// Answered: the leading model, then "Next:". No clear difference: both
    /// figures, neither first. Short of traces or prices: each model's
    /// traces in the question.
    static func models(_ row: DaemonData.InsightsModelAnswerRow, copy: [String: String]) -> [Line] {
        switch row.state {
        case "answered":
            return [row.leader.map { modelLine($0, key: "analytics_models_model_line", copy: copy) },
                    row.next.map { modelLine($0, key: "analytics_models_next_line", copy: copy) }]
                .compactMap { $0 }
        case "no_clear_difference":
            return [row.leader, row.next].compactMap { $0 }
                .map { modelLine($0, key: "analytics_models_model_line", copy: copy) }
        case "not_enough_sessions", "not_priced":
            return row.sessionsByModel.map { count in
                Line(text: fill(count.sessions == 1 ? "analytics_models_count_line_one"
                                                    : "analytics_models_count_line_other",
                                copy, ["label": count.label, "n": String(count.sessions)]))
            }
        default:
            return []
        }
    }

    static func modelLine(_ model: DaemonData.InsightsModelFigure, key: String,
                          copy: [String: String]) -> Line {
        Line(text: fill(key, copy, ["label": model.label, "x": figure(model.figure, copy: copy),
                                    "n": String(model.sessions)]),
             detail: model.figure.kind == "rate_permille"
                ? model.interruptsPerSessionPermille.map {
                    fill("analytics_models_interrupts", copy, ["x": perSession($0)])
                }
                : nil)
    }

    /// The figure in words; a kind this build does not know, or one
    /// missing its value, is the dash, never a zero.
    static func figure(_ figure: DaemonData.InsightsModelFigureValue, copy: [String: String]) -> String {
        switch figure.kind {
        case "rate_permille":
            guard let value = figure.value else { break }
            return fill("analytics_models_rate", copy, ["p": percent(value)])
        case "usd_micros":
            guard let value = figure.value else { break }
            return fill("analytics_models_cost", copy, ["x": money(value)])
        case "finish_time":
            guard let secs = figure.medianActiveSecs, let turns = figure.medianUserTurns else { break }
            return fill(turns == 1 ? "analytics_models_finish_one" : "analytics_models_finish_other", copy,
                        ["span": duration(secs), "u": String(turns)])
        default:
            break
        }
        return text("analytics_unavailable", copy)
    }

    /// What the row's figure counts, the share of each model's traces the
    /// person marked Kept, which traces the time row compares, and what
    /// Private AI charged, under the models.
    static func notes(_ row: DaemonData.InsightsModelAnswerRow, copy: [String: String]) -> [String] {
        var notes: [String] = []
        switch row.basis {
        case "rejected": notes.append(text("analytics_models_basis_rejected", copy))
        case "undone": notes.append(text("analytics_models_basis_undone", copy))
        default: break
        }
        notes += (row.keptByModel ?? []).compactMap { keptLine($0, copy: copy) }
        if row.leader != nil {
            switch row.editMode {
            case "prompting": notes.append(text("analytics_models_mode_prompting", copy))
            case "not_prompting": notes.append(text("analytics_models_mode_not_prompting", copy))
            default: break
            }
        }
        if let charge = row.nearAiCharge, let line = chargeLine(charge, copy: copy) { notes.append(line) }
        return notes
    }

    /// One model's kept share from the person's Outcome labels: "{label}:
    /// 70% kept, of 10 traces you marked". Beside the rejection figure,
    /// never folded into it; `nil` for a model with none marked.
    static func keptLine(_ entry: DaemonData.InsightsModelKept, copy: [String: String]) -> String? {
        let figure = entry.figure
        guard figure.labelled > 0, figure.keptPermille >= 0 else { return nil }
        return fill(figure.labelled == 1 ? "analytics_models_kept_one" : "analytics_models_kept_other", copy,
                    ["label": entry.label, "p": percent(UInt64(figure.keptPermille)),
                     "n": String(figure.labelled)])
    }

    /// What Private AI charged per trace, apart from the API-price figure;
    /// "not recorded", never zero, when no charge is known.
    static func chargeLine(_ charge: DaemonData.InsightsNearAiCharge, copy: [String: String]) -> String? {
        guard charge.basis == "near_ai_ledger" else { return nil }
        let routed = String(charge.routedSessions)
        guard let median = charge.median, median.kind == "usd_micros", let value = median.value else {
            return fill("analytics_models_charge_unknown", copy, ["n": routed])
        }
        var line = fill("analytics_models_charge", copy, ["x": money(value), "n": routed])
        if charge.chargeUnknown > 0 {
            line += " \u{b7} " + fill("analytics_models_charge_some_unknown", copy,
                                      ["u": String(charge.chargeUnknown)])
        }
        return line
    }

    /// Behind the row's "i": how its figure is measured, why a model is or
    /// is not named first, that labels are as recorded, and the traces left
    /// out, by reason.
    static func info(_ row: DaemonData.InsightsModelAnswerRow, copy: [String: String]) -> [String] {
        var info: [String] = []
        switch (row.question, row.basis) {
        case ("docs_cost", _): info.append(text("analytics_models_info_cost", copy))
        case ("finish_time", _): info.append(text("analytics_models_info_finish", copy))
        case (_, "undone"): info.append(text("analytics_models_info_undone", copy))
        case (_, "rejected"): info.append(text("analytics_models_info_rejected", copy))
        default: break
        }
        switch row.state {
        case "answered": info.append(text("analytics_models_info_lead", copy))
        case "no_clear_difference": info.append(text("analytics_models_info_no_clear_difference", copy))
        default: break
        }
        info.append(text("analytics_model_labels_note", copy))
        let leftOut = row.leftOut.reasons.filter { $0.sessions > 0 }.map {
            fill("analytics_models_left_out_" + $0.reason, copy, ["n": String($0.sessions)])
        }
        if !leftOut.isEmpty {
            info.append(([text("analytics_models_left_out", copy)] + leftOut).joined(separator: "\n"))
        }
        return info
    }

    /// A per mille as a percent, to one decimal where it has one: 42 is
    /// "4.2", 40 is "4".
    static func percent(_ permille: UInt64) -> String {
        (Double(permille) / 10).formatted(.number.precision(.fractionLength(0...1)))
    }

    /// Interrupts per trace from the daemon's per mille: 180 is "0.2", and
    /// under a tenth one significant digit, so 1 to 50 per mille never
    /// reads as "0": 50 is "0.05".
    static func perSession(_ permille: Int) -> String {
        let value = Double(permille) / 1_000
        guard permille > 0, permille < 100 else {
            return value.formatted(.number.precision(.fractionLength(0...1)))
        }
        return value.formatted(.number.precision(.significantDigits(1)))
    }

    /// Integer USD micros as dollars, as the app's other prices are
    /// (`InferenceViews.money`): two to four decimals, so "$0.50", "$1.00"
    /// and a docs trace's "$0.0412". Under a hundredth of a cent, one
    /// significant digit, so a charge is never shown as zero.
    static func money(_ micros: UInt64) -> String {
        let dollars = Decimal(micros) / 1_000_000
        let style = Decimal.FormatStyle.Currency(code: "USD")
        return micros > 0 && micros < 100
            ? dollars.formatted(style.precision(.significantDigits(1)))
            : dollars.formatted(style.precision(.fractionLength(2...4)))
    }

    /// Active seconds as "5 min, 12 sec".
    static func duration(_ secs: UInt64) -> String {
        Duration.seconds(Int64(clamping: secs))
            .formatted(.units(allowed: [.hours, .minutes, .seconds], width: .abbreviated, maximumUnitCount: 2))
    }
}
