#if DEBUG
import Foundation

/// The JSON `SampleDaemonClient` serves. DEBUG ONLY.
///
/// Eleven methods (`status`, `list_pending`, `list_kept`, `list_projects`,
/// `harness_list`, `get_settings`, `list_history`, `history_rollup`,
/// `commons_credit_summary`, `tool_destinations`, `inference_calls`) and four
/// more whose reply never depends on the sample set (`approve`, `keep`,
/// `undo_keep`, `set_project_mode`) answer from `RecordedSamples/`: real
/// replies from the real daemon (`daemon::ipc::{bind, serve}`) on a
/// throwaway store, captured by
/// `crates/trace-commons-contributor/tests/k2_sample_recorder.rs` (K2 of
/// #1173). See that file's module doc for exactly how each sample set is
/// built and how to re-record.
///
/// To re-record: `cargo test -p trace-commons-contributor --test
/// k2_sample_recorder -- --ignored record_samples_to_disk` from the repo
/// root, then `swift test` here, and commit both the regenerated
/// `RecordedSamples/` files and whatever Rust change prompted the
/// re-recording.
///
/// Values with no source on main are marked `"_sample": "no source yet"`
/// in the JSON. That is every PROVISIONAL network method (Zaki's C3):
/// `inference_summary`, `inference_call_proof`, `model_spend`,
/// `private_ai`, `mission_catalogue`, `invite_lookup`, `passkey_state` and
/// `account_session_status` stay hand-written below, and so does `preview`
/// / `preview_unsure_spans` (per-entry templates, not a fixed shape -- see
/// `previewSummary(for:)`) and the write acknowledgements that carry a
/// caller's own parameters (`approvedGroup`, `approveSkipped`). The marker
/// key is ignored by every decoder.
///
/// Three more files under `RecordedSamples/` carry a `"_sample"` marker of
/// their own for the same reason: a temp store's real daemon cannot exhibit
/// the state the screen needs to draw. `unknownCounts/status.json` is the
/// real `status` recording with `decisions_owed` removed by hand (the badge
/// draws "—" for an older daemon or an unreachable one, and the real
/// `status_value` always computes a concrete count); `normalDay/` and
/// `busyQueue/inference_calls.json` are hand-written `readable: true` pages
/// shaped exactly like the real reply `calls_page` builds
/// (`daemon/inference_map.rs`), because `readable` needs a live IronWire
/// proxy answering and no temp store runs one. All three are listed in
/// `k2_sample_recorder.rs`'s `HAND_WRITTEN_OVERRIDES`: re-recording still
/// regenerates them (`apply_hand_written_overrides` runs after the real
/// capture, before the write), but its drift test excludes them, since a
/// fresh real capture can never equal a value it was deliberately edited
/// away from.
enum SampleDaemonData {
    typealias Sample = SampleDaemonClient.SampleSet

    static func reply(_ method: String, in set: Sample) -> String? {
        switch method {
        case "status": return recorded("status", in: set)
        case "list_pending": return #"{"pending":\#(pending(set))}"#
        case "list_kept": return #"{"kept":\#(kept(set))}"#
        case "list_projects": return recorded("list_projects", in: set)
        case "harness_list": return recorded("harness_list", in: set)
        case "get_settings": return recorded("get_settings", in: set)
        case "list_history": return recorded("list_history", in: set)
        case "history_rollup": return recorded("history_rollup", in: set)
        case "commons_credit_summary": return recorded("commons_credit_summary", in: set)
        case "tool_destinations": return recorded("tool_destinations", in: set)
        case "inference_calls": return recorded("inference_calls", in: set)
        case "approve": return approved
        case "keep": return recordedShared("keep")
        case "undo_keep": return recordedShared("undo_keep")
        case "set_project_mode": return recordedShared("set_project_mode")
        // PROVISIONAL (Zaki's C3): no source on main.
        case "inference_summary": return inferenceSummary(set)
        case "inference_call_proof": return proofDetail
        case "model_spend": return modelSpend(set)
        case "private_ai": return privateAI(set)
        case "mission_catalogue": return missionCatalogue
        case "invite_lookup": return inviteLookup
        case "passkey_state": return passkeyState(set)
        case "account_session_status": return accountState(set)
        default: return nil
        }
    }

    // MARK: - approve, keep, undo_keep, set_project_mode (recorded once: K2)

    /// One entry approved against the real daemon, in `k2_sample_recorder`'s
    /// `normal_day` build: what the real redaction pipeline found, nothing
    /// skipped. `SampleDaemonData.reply` never reads `set` for this method,
    /// so one recording covers every sample set.
    static let approved: String = recordedShared("approve")

    /// A folder approve: a group call always carries `excluded_held`. Kept
    /// hand-written, unlike the single-entry `approved` above -- its shape
    /// is fixed, but its *values* (`approved`, `excludedHeld`) are exactly
    /// the caller's own count of what this folder held back, not something
    /// a single recording could stand in for.
    static func approvedGroup(approved: Int, excludedHeld: Int) -> String {
        let holdUntil = approved > 0 ? #""2026-09-30T09:20:10Z""# : "null"
        return #"{"approved":\#(approved),"hold_secs":10,"hold_until":\#(holdUntil),"flagged":0,"redactions":{},"skipped":[],"excluded_held":\#(excludedHeld)}"#
    }

    /// The daemon's OK-but-skipped answer for a single entry it did not act
    /// on. Hand-written like `approvedGroup`, for the same reason: the
    /// entry id is the caller's own.
    static func approveSkipped(entryId: String, reason: String) -> String {
        #"{"approved":0,"hold_secs":10,"hold_until":null,"flagged":0,"redactions":{},"skipped":[{"entry_id":"\#(entryId)","reason_label":"\#(reason)"}]}"#
    }

    // MARK: - Recorded replies (K2 of #1173)

    /// `list_pending`'s bare `pending` array, read back out of the recorded
    /// `{"pending":[...]}` file. `reply`'s own `list_pending` case re-wraps
    /// it under that same key; `SampleDaemonClient.events()` needs the array
    /// on its own, for a snapshot event's `data.pending`.
    static func pending(_ set: Sample) -> String {
        recordedArray("list_pending", key: "pending", in: set)
    }

    /// `list_kept`'s bare `kept` array -- see `pending(_:)`.
    static func kept(_ set: Sample) -> String {
        recordedArray("list_kept", key: "kept", in: set)
    }

    private static func recorded(_ method: String, in set: Sample) -> String {
        recordedFile(method, directory: directory(for: set))
    }

    private static func recordedShared(_ method: String) -> String {
        recordedFile(method, directory: "shared")
    }

    /// `coreDown` never reaches here: `SampleDaemonClient.json(for:)` answers
    /// `nil` for it before calling `reply`, by construction. It folds to
    /// `empty`'s files only so this switch can stay exhaustive.
    private static func directory(for set: Sample) -> String {
        switch set {
        case .empty, .coreDown: return "empty"
        case .normalDay: return "normalDay"
        case .busyQueue: return "busyQueue"
        case .unknownCounts: return "unknownCounts"
        case .heldSessions: return "heldSessions"
        case .armedFolder: return "armedFolder"
        }
    }

    private static func recordedArray(_ method: String, key: String, in set: Sample) -> String {
        let full = recorded(method, in: set)
        guard
            let data = full.data(using: .utf8),
            let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let array = object[key],
            let arrayData = try? JSONSerialization.data(withJSONObject: array)
        else {
            fatalError("K2 recording malformed: \(method)/\(key) in \(set)")
        }
        return String(decoding: arrayData, as: UTF8.self)
    }

    /// The raw text of one K2 recording,
    /// `RecordedSamples/<directory>/<method>.json`, bundled as a `TCShellCore`
    /// resource (`Package.swift`) and read through `Bundle.module`.
    private static func recordedFile(_ method: String, directory: String) -> String {
        guard
            let url = Bundle.module.url(
                forResource: method, withExtension: "json",
                subdirectory: "RecordedSamples/\(directory)"),
            let text = try? String(contentsOf: url, encoding: .utf8)
        else {
            fatalError("K2 recording missing: RecordedSamples/\(directory)/\(method).json -- re-record with " +
                "`cargo test -p trace-commons-contributor --test k2_sample_recorder -- --ignored record_samples_to_disk`")
        }
        return text
    }

    // MARK: - preview, preview_unsure_spans
    //
    // Per-entry templates, not a fixed shape: a card is built from whatever
    // entry a caller names, so there is nothing here a single recording
    // could replace. Both stay hand-written.

    /// A `preview` summary for one sample entry: the entry itself, plus a
    /// title and counts that agree with the entry's scrub fields.
    static func previewSummary(for entry: DaemonData.QueueEntry) -> String {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        let entryJSON = (try? String(decoding: encoder.encode(entry), as: UTF8.self)) ?? "null"
        let marks = entry.marks ?? 4
        let content = entry.contentMarks ?? 2
        let unsure = entry.unsureSpans ?? 0
        let reasons = (entry.secondLook ?? []).map { "\"\($0)\"" }.joined(separator: ",")
        let titles = [
            "Flaky retry test, upload pass",
            "History endpoint pagination",
            "Watcher re-reads unchanged files",
            "Settings validation refactor",
        ]
        let title = titles[entry.entryId.unicodeScalars.reduce(0) { $0 + Int($1.value) } % titles.count]
        return #"{"entry":\#(entryJSON),"title":"\#(title)","would_send_bytes":\#((entry.sizeBytes ?? 40000) + 2545),"raw_session_bytes":\#(entry.sizeBytes ?? 40000),"event_count":\#((entry.userTurns ?? 3) * 6),"opening_prompt":"\#(title)","redactions":{"local_path":\#(marks - content),"email":\#(content)},"redactions_distinct":{"local_path":\#(min(1, marks - content)),"email":\#(min(1, content))},"pii_labels_present":["email"],"consent_scopes":["debugging_evaluation"],"residual_risk":"pattern-based","envelope_digest":"sha256:sample-envelope","input_fingerprint":"sha256:sample-input","enrolled":true,"subagent_count":\#(entry.subagentCount ?? 0),"subagents_dropped":\#(entry.subagentsDropped ?? 0),"scrub":"scrubbed","marks":\#(marks),"content_marks":\#(content),"unsure_spans":\#(unsure),"second_look":[\#(reasons)]}"#
    }

    static func unsureSpans(entryId: String, bodyDigest: String) -> String {
        #"{"entry_id":"\#(entryId)","body_digest":"\#(bodyDigest)","envelope_digest":"sha256:sample-envelope","span_count":2,"spans":[{"label":"looks-like-email","byte_offset":1408,"byte_len":11},{"label":"looks-like-phone","byte_offset":2210,"byte_len":12}],"spans_truncated":false}"#
    }

    // MARK: - PROVISIONAL network methods (Zaki's C3): no source on main

    static func inferenceSummary(_ set: Sample) -> String {
        guard set == .normalDay || set == .busyQueue else {
            return #"{"_sample":"no source yet","readable":false,"window_hours":24,"models":[]}"#
        }
        return #"{"_sample":"no source yet","readable":true,"window_hours":24,"models":[{"model":"zai-org/GLM-4.6","family":"anthropic","calls":9,"priced_micros":61200,"proof_counts":{"verified":7,"pending":1,"failed":1}},{"model":"Qwen/Qwen3.6-27B-FP8","family":"openai","calls":3,"priced_micros":36900,"proof_counts":{"gateway_only":3}}]}"#
    }

    static let proofDetail =
        #"{"_sample":"no source yet","call_id":414,"proof":"verified","checked_at":"2026-09-30T09:04:13Z","checks":["receipt-signature-valid","quote-measurement-pinned","digests-match"]}"#

    static func modelSpend(_ set: Sample) -> String {
        guard set == .normalDay || set == .busyQueue else {
            return #"{"_sample":"no source yet","known":false,"since":null,"models":[]}"#
        }
        return #"{"_sample":"no source yet","known":true,"since":"2026-09-30T00:00:00Z","models":[{"model":"zai-org/GLM-4.6","billed_micros":820000},{"model":"Qwen/Qwen3.6-27B-FP8","billed_micros":410000}]}"#
    }

    static func privateAI(_ set: Sample) -> String {
        let on = set == .normalDay || set == .busyQueue
        return #"{"_sample":"no source yet","on":\#(on),"state":"\#(on ? "running" : "off")","disclosure":"Sample disclosure text, supplied by the core"}"#
    }

    static let missionCatalogue =
        #"{"_sample":"no source yet","fetched_at":"2026-09-30T06:00:00Z","posture":{"settlement":"disabled","graded":false,"explanation":"Sample settlement explanation, supplied by the commons"},"missions":[{"id":"mission-sample-1","title":"Debug a failing test","summary":"Failing test found, then fixed","credit_range":{"min":5,"max":20,"unit":"points"}},{"id":"mission-sample-2","title":"Review a pull request","summary":null,"credit_range":null}]}"#

    static let inviteLookup =
        #"{"_sample":"no source yet","valid":true,"issuer_display_name":"Sample Labs","credit_range":{"min":10,"max":40,"unit":"points"}}"#

    static func passkeyState(_ set: Sample) -> String {
        set == .empty
            ? #"{"_sample":"no source yet","state":"none","passkey_count":0,"near_ai_connected":false}"#
            : #"{"_sample":"no source yet","state":"bound","passkey_count":1,"near_ai_connected":true}"#
    }

    static func accountState(_ set: Sample) -> String {
        set == .empty
            ? #"{"_sample":"no source yet","signed_in":false,"account_id":null}"#
            : #"{"_sample":"no source yet","signed_in":true,"account_id":"sample.near"}"#
    }
}
#endif
