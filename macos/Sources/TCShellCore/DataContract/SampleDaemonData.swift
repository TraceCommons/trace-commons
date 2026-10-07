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
/// The network methods (C3, #1187) -- `inference_summary`,
/// `inference_call_proof`, `model_spend`, `private_ai`, `mission_catalogue`,
/// `invite_lookup`, `passkey_state`, `account_session_status` and
/// `activity_missions_catalogue` -- are hand-written below in the shapes the
/// daemon serves, each reply marked `"_sample":"hand-written"`. They are
/// synthetic, never pilot observations: K2's recorder runs a temp store with
/// no network behind it, so it cannot capture them (see
/// `k2_sample_recorder.rs`), and Rust-emitted fixtures for them are a
/// follow-up. The PROVISIONAL shapes the Inference and Missions screens
/// still read (`provisional(_:in:)`) are hand-written too, marked
/// `"no source yet"`. So are `preview` / `preview_unsure_spans` (per-entry
/// templates, not a fixed shape -- see `previewCard(for:withEntry:)`) and
/// the write acknowledgements that carry a caller's own parameters
/// (`approvedGroup`, `approveSkipped`), which carry no marker. The marker
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
        // Network methods (C3, #1187): hand-written, see above.
        case "inference_summary": return inferenceSummary(set)
        case "inference_call_proof": return proofDetail(set)
        case "model_spend": return modelSpend(set)
        case "private_ai": return privateAI(set)
        case "mission_catalogue": return missionCatalogue
        case "invite_lookup": return inviteLookup
        case "passkey_state": return passkeyState(set)
        case "account_session_status": return accountState(set)
        case "activity_missions_catalogue":
            guard set != .unknownCounts else { return nil }
            return activityMissionsCatalogue
        default: return nil
        }
    }

    /// The PROVISIONAL shapes the Inference and Missions screens still read
    /// through `inferenceSummary()` and `missionCatalogue()`. The wire's own
    /// replies for those methods are in `reply(_:in:)`.
    static func provisional(_ method: String, in set: Sample) -> String? {
        switch method {
        case "inference_summary": return provisionalInferenceSummary(set)
        case "mission_catalogue": return provisionalMissionCatalogue
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

    /// The daemon's preview card (`preview_card_value`) for one sample
    /// entry: a title and counts that agree with the entry's scrub fields.
    /// A card is a scrub, not a build, so it has no `envelope_digest`, no
    /// `redactions_distinct` and no `subagent_count`. `preview` adds the
    /// entry; a scheduled card does not.
    static func previewCard(for entry: DaemonData.QueueEntry, withEntry: Bool) -> String {
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
        let entryField = withEntry ? #""entry":\#(entryJSON),"# : ""
        return #"{\#(entryField)"title":"\#(title)","would_send_bytes":\#((entry.sizeBytes ?? 40000) + 2545),"raw_session_bytes":\#(entry.sizeBytes ?? 40000),"event_count":\#((entry.userTurns ?? 3) * 6),"opening_prompt":"\#(title)","redactions":{"local_path":\#(marks - content),"email":\#(content)},"pii_labels_present":["email"],"consent_scopes":["debugging_evaluation"],"residual_risk":"pattern-based","input_fingerprint":"sha256:sample-input","enrolled":true,"subagents_dropped":\#(entry.subagentsDropped ?? 0),"scrub":"scrubbed","marks":\#(marks),"content_marks":\#(content),"unsure_spans":\#(unsure),"second_look":[\#(reasons)]}"#
    }

    static func unsureSpans(entryId: String, bodyDigest: String) -> String {
        #"{"entry_id":"\#(entryId)","body_digest":"\#(bodyDigest)","envelope_digest":"sha256:sample-envelope","span_count":2,"spans":[{"label":"looks-like-email","byte_offset":1408,"byte_len":11},{"label":"looks-like-phone","byte_offset":2210,"byte_len":12}],"spans_truncated":false}"#
    }

    // MARK: - Network methods (C3, #1187): hand-written, marked per reply

    static func inferenceSummary(_ set: Sample) -> String {
        guard set == .normalDay || set == .busyQueue else {
            return #"{"_sample":"hand-written","readable":false,"window_hours":24,"observed_at":null,"summary":null}"#
        }
        // SAMPLE: registry-priced cost is incomplete and never billed spend.
        return #"{"_sample":"hand-written","readable":true,"window_hours":24,"observed_at":"2026-10-01T00:00:00Z","summary":{"enabled":true,"receipts":true,"since":"2026-09-30T00:00:00Z","groups":[{"group_id":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","model":"example-model","backend":"nearai","route":"routed","work_kind":null,"calls":3,"priced_calls":2,"cost_usd":0.02,"proof":{"verified":1,"gateway_only":0,"unattested":0,"pending":1,"unavailable":0,"failed":1,"outside":0,"unrecorded":0}},{"group_id":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","model":"example-model","backend":"sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","route":"outside","work_kind":null,"calls":1,"priced_calls":1,"cost_usd":0.01,"proof":{"verified":0,"gateway_only":0,"unattested":0,"pending":0,"unavailable":0,"failed":0,"outside":1,"unrecorded":0}}],"routed":{"calls":3,"priced_calls":2,"cost_usd":0.02,"proof":{"verified":1,"gateway_only":0,"unattested":0,"pending":1,"unavailable":0,"failed":1,"outside":0,"unrecorded":0}},"outside":{"calls":1,"priced_calls":1,"cost_usd":0.01,"proof":{"verified":0,"gateway_only":0,"unattested":0,"pending":0,"unavailable":0,"failed":0,"outside":1,"unrecorded":0}},"unknown":{"calls":0,"priced_calls":0,"cost_usd":0.0,"proof":{"verified":0,"gateway_only":0,"unattested":0,"pending":0,"unavailable":0,"failed":0,"outside":0,"unrecorded":0}}}}"#
    }

    static func proofDetail(_ set: Sample) -> String {
        set == .normalDay || set == .busyQueue
            ? #"{"_sample":"hand-written","call_id":414,"proof":"verified","checked_at":null,"checks":null,"readable":true,"found":true}"#
            : #"{"_sample":"hand-written","call_id":414,"proof":"unrecorded","checked_at":null,"checks":null,"readable":false,"found":false}"#
    }

    static func modelSpend(_ set: Sample) -> String {
        // SAMPLE: default previews have no authoritative organization billing recording.
        #"{"_sample":"hand-written","known":false,"since":null,"models":[],"reason_label":"billed-model-spend-unavailable"}"#
    }

    static func privateAI(_ set: Sample) -> String {
        if set == .unknownCounts {
            return #"{"_sample":"hand-written","on":null,"state":null,"port":null,"disclosure":"SAMPLE: exposure disclosure from the Rust core"}"#
        }
        let on = set == .normalDay || set == .busyQueue
        return #"{"_sample":"hand-written","on":\#(on),"state":"\#(on ? "running" : "off")","port":\#(on ? "3128" : "null"),"disclosure":"SAMPLE: exposure disclosure from the Rust core"}"#
    }

    // SAMPLE: public skill-evaluation catalogue, no daily assignments or credits.
    static let missionCatalogue =
        #"{"_sample":"hand-written","kind":"skill_evaluation","disclosure":"SAMPLE: catalogue consent disclosure from the Rust core","catalogue":{"schema_version":1,"entries":[{"mission_id":"00000000-0000-4000-8000-000000000001","program_id":"00000000-0000-4000-8000-000000000002","package_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","offer_version_hash":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","task_preview":"SAMPLE: evaluate a reviewed skill package against its controls.","published_at":"2026-10-01T00:00:00Z"}],"next_cursor":null}}"#

    static let inviteLookup =
        #"{"_sample":"hand-written","valid":true,"issuer_display_name":"SAMPLE Pilot","credit_range":{"min":1,"max":5,"unit":"points_per_accepted_trace"}}"#

    static func passkeyState(_ set: Sample) -> String {
        switch set {
        case .empty:
            return #"{"_sample":"hand-written","state":"none","passkey_count":0,"remembered_name":null,"near_ai_connected":null}"#
        case .unknownCounts:
            return #"{"_sample":"hand-written","state":"unknown","passkey_count":null,"remembered_name":null,"near_ai_connected":null}"#
        default:
            return #"{"_sample":"hand-written","state":"bound","passkey_count":1,"remembered_name":"SAMPLE passkey","near_ai_connected":true}"#
        }
    }

    static func accountState(_ set: Sample) -> String {
        switch set {
        case .empty:
            return #"{"_sample":"hand-written","state":"known","signed_in":false,"account_id":null,"expires_at":null}"#
        case .unknownCounts:
            return #"{"_sample":"hand-written","state":"unknown","signed_in":null,"account_id":null,"expires_at":null}"#
        default:
            return #"{"_sample":"hand-written","state":"known","signed_in":true,"account_id":"00000000-0000-4000-8000-000000000003","expires_at":"2026-10-02T00:00:00Z"}"#
        }
    }

    static let activityMissionsCatalogue =
        #"{"_sample":"hand-written","catalogue":{"schema_version":1,"kind":"trace_activity","state":"unconfigured","policy_sha256":null,"policy":null,"rewards_enabled":false,"credit_points_pending":null,"credit_condition":"mission_credit_ledger_unavailable"},"disclosure":"SAMPLE: activity mission consent disclosure from the Rust core"}"#

    // MARK: - PROVISIONAL shapes the screens still read (no source)

    static func provisionalInferenceSummary(_ set: Sample) -> String {
        guard set == .normalDay || set == .busyQueue else {
            return #"{"_sample":"no source yet","readable":false,"window_hours":24,"models":[]}"#
        }
        return #"{"_sample":"no source yet","readable":true,"window_hours":24,"models":[{"model":"zai-org/GLM-4.6","family":"anthropic","calls":9,"priced_micros":61200,"proof_counts":{"verified":7,"pending":1,"failed":1}},{"model":"Qwen/Qwen3.6-27B-FP8","family":"openai","calls":3,"priced_micros":36900,"proof_counts":{"gateway_only":3}}]}"#
    }

    static let provisionalMissionCatalogue =
        #"{"_sample":"no source yet","fetched_at":"2026-09-30T06:00:00Z","posture":{"settlement":"disabled","graded":false,"explanation":"Sample settlement explanation, supplied by the commons"},"missions":[{"id":"mission-sample-1","title":"Debug a failing test","summary":"Failing test found, then fixed","credit_range":{"min":5,"max":20,"unit":"points"}},{"id":"mission-sample-2","title":"Review a pull request","summary":null,"credit_range":null}]}"#
}
#endif
