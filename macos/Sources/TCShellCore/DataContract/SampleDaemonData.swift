#if DEBUG
import Foundation

/// The JSON `SampleDaemonClient` serves. DEBUG ONLY.
///
/// Every reply is an IPC `result` object in the shape the daemon sends on
/// main. The shapes were taken from real replies: a throwaway test in
/// `daemon::ipc` printed `entry_value`, `status`, `get_settings`,
/// `list_projects`, `harness_list`, `tool_destinations`, `inference_calls`,
/// `history_rollup`, `list_history`, `list_kept` and `commons_credit_summary`
/// against a temp store, and these sets fill those shapes with plausible
/// values. K2 replaces them with exact recordings and a drift test.
///
/// Values with no source on main are marked `"_sample": "no source yet"`
/// in the JSON. That is every PROVISIONAL network method (Zaki's C3); the
/// marker key is ignored by every decoder.
enum SampleDaemonData {
    typealias Sample = SampleDaemonClient.SampleSet

    static func reply(_ method: String, in set: Sample) -> String? {
        switch method {
        case "status": return status(set)
        case "list_pending": return #"{"pending":\#(pending(set))}"#
        case "list_kept": return #"{"kept":\#(kept(set))}"#
        case "list_projects": return projects(set)
        case "harness_list": return harnessList(set)
        case "get_settings": return settings(set)
        case "list_history": return history(set)
        case "history_rollup": return rollup(set)
        case "commons_credit_summary": return credit(set)
        case "tool_destinations": return toolDestinations(set)
        case "inference_calls": return inferenceCalls(set)
        case "approve": return approved
        case "keep": return #"{"kept":true}"#
        case "undo_keep": return #"{"kept":false}"#
        case "set_project_mode": return #"{"ok":true,"purged":0,"retracted":0,"from_now":true}"#
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

    // MARK: - approve

    /// One entry approved: what the scrub removed, nothing skipped.
    static let approved =
        #"{"approved":1,"hold_secs":10,"hold_until":"2026-09-30T09:20:10Z","flagged":1,"redactions":{"email":2,"local_path":3},"skipped":[]}"#

    /// A folder approve: a group call always carries `excluded_held`.
    static func approvedGroup(approved: Int, excludedHeld: Int) -> String {
        let holdUntil = approved > 0 ? #""2026-09-30T09:20:10Z""# : "null"
        return #"{"approved":\#(approved),"hold_secs":10,"hold_until":\#(holdUntil),"flagged":0,"redactions":{},"skipped":[],"excluded_held":\#(excludedHeld)}"#
    }

    /// The daemon's OK-but-skipped answer for a single entry it did not act on.
    static func approveSkipped(entryId: String, reason: String) -> String {
        #"{"approved":0,"hold_secs":10,"hold_until":null,"flagged":0,"redactions":{},"skipped":[{"entry_id":"\#(entryId)","reason_label":"\#(reason)"}]}"#
    }

    // MARK: - Projects used across sets

    struct Project {
        let id: String
        let label: String
        var path: String { "/Users/sample/src/\(label)" }
    }

    static let api = Project(id: "proj_f521cc304a06c2a9", label: "api")
    static let web = Project(id: "proj_9c3104d8d0e56788", label: "web")
    static let infra = Project(id: "proj_4be81f07c2d95a13", label: "infra")

    // MARK: - Queue entries (`ipc::entry_value`)

    struct Scrub {
        let marks: Int
        let contentMarks: Int
        let unsureSpans: Int
    }

    static func entry(
        _ n: Int,
        _ project: Project,
        state: String = "pending",
        reason: String? = nil,
        scrub: Scrub? = nil,
        secondLook: [String] = [],
        source: String = "claude-code",
        subagents: Int = 0,
        dropped: Int = 0,
        hour: Int = 9,
        minutes: Int = 42,
        turns: Int? = 9,
        size: Int = 48213
    ) -> String {
        let id = String(format: "00000000-0000-4000-8000-%012d", n)
        let startMinute = (hour - 1) * 60 + 5
        let endMinute = startMinute + minutes
        let started = String(format: "2026-09-30T%02d:%02d:00Z", startMinute / 60, startMinute % 60)
        let ended = String(format: "2026-09-30T%02d:%02d:00Z", endMinute / 60, endMinute % 60)
        let duration = String(minutes * 60)
        let scrubFields: String
        if let scrub {
            scrubFields = #""scrub":"scrubbed","marks":\#(scrub.marks),"content_marks":\#(scrub.contentMarks),"unsure_spans":\#(scrub.unsureSpans),"#
        } else {
            scrubFields = #""scrub":"not-yet-scrubbed","#
        }
        let reasonJSON = reason.map { "\"\($0)\"" } ?? "null"
        let shape = turns.map {
            #""started_at":"\#(started)","ended_at":"\#(ended)","duration_secs":\#(duration),"user_turns":\#($0)"#
        } ?? #""started_at":null,"ended_at":null,"duration_secs":null,"user_turns":null"#
        let reasons = secondLook.map { "\"\($0)\"" }.joined(separator: ",")
        return #"{"entry_id":"\#(id)","session_hash":"sha256:sample\#(n)","source":"\#(source)","declared_source":null,"project_id":"\#(project.id)","project_label":"\#(project.label)","project_path":"\#(project.path)","session_path":null,"size_bytes":\#(size),"discovered_at":"2026-09-30T\#(String(format: "%02d", hour)):12:00Z","state":"\#(state)","reason_label":\#(reasonJSON),"attempts":0,"retry_after":null,"submission_id":null,"subagent_count":\#(subagents),"subagents_dropped":\#(dropped),\#(shape),"holds_certificate":false,"attestation":"unknown",\#(scrubFields)"second_look":[\#(reasons)]}"#
    }

    static func pending(_ set: Sample) -> String {
        let entries: [String]
        switch set {
        case .empty, .coreDown:
            entries = []
        case .normalDay, .unknownCounts:
            entries = [
                entry(1, api, scrub: Scrub(marks: 7, contentMarks: 4, unsureSpans: 0), subagents: 2),
                entry(2, web, hour: 10, minutes: 18, turns: 4),
                entry(3, api, reason: "returned-from-keep", source: "codex", hour: 11, minutes: 75, turns: 14),
            ]
        case .busyQueue:
            entries = (1...14).map { n in
                let project = [api, web, infra][n % 3]
                let scrub = n % 4 == 0 ? Scrub(marks: n, contentMarks: n / 2, unsureSpans: n % 3) : nil
                return entry(
                    n, project, scrub: scrub, source: n % 5 == 0 ? "codex" : "claude-code",
                    subagents: n % 6, hour: 8 + n % 12, minutes: 6 + n * 7, turns: 2 + n % 11,
                    size: 9000 + n * 3100)
            }
        case .heldSessions:
            entries = [
                entry(
                    1, api, reason: "second-look-review-required",
                    scrub: Scrub(marks: 3, contentMarks: 0, unsureSpans: 2),
                    secondLook: ["nothing-matched", "looks-unsure", "trimmed-to-fit"], subagents: 2, dropped: 1),
                entry(
                    2, web, reason: "second-look-review-required",
                    scrub: Scrub(marks: 5, contentMarks: 2, unsureSpans: 1), secondLook: ["looks-unsure"],
                    hour: 10),
                entry(3, web, reason: "scrub-check-manual", hour: 11, minutes: 30, turns: 6),
                // An entry queued before the daemon recorded a shape: started_at, ended_at, duration_secs and user_turns all null.
                entry(4, api, reason: "scrub-check-manual", hour: 12, turns: nil),
            ]
        case .armedFolder:
            entries = [
                // In the armed folder, settling: goes on its own, not owed.
                entry(1, infra, scrub: Scrub(marks: 2, contentMarks: 1, unsureSpans: 0), hour: 13, minutes: 12, turns: 3),
                entry(2, infra, hour: 14, minutes: 8, turns: 2),
                // Backlog the from-now arming left waiting for a person: owed.
                entry(3, infra, hour: 9, minutes: 50, turns: 11),
            ]
        }
        return "[" + entries.joined(separator: ",") + "]"
    }

    static func kept(_ set: Sample) -> String {
        switch set {
        case .normalDay, .busyQueue, .heldSessions:
            return "[" + entry(90, web, state: "refused", reason: "kept-on-this-mac", hour: 8, minutes: 25, turns: 5) + "]"
        default:
            return "[]"
        }
    }

    // MARK: - status

    static func status(_ set: Sample) -> String {
        // (queue_depth, decisions_owed): decisions_owed is never derived from
        // queue_depth; the armed set shows them diverging.
        let counts: (Int, Int?)
        switch set {
        case .empty, .coreDown: counts = (0, 0)
        case .normalDay: counts = (3, 3)
        case .busyQueue: counts = (14, 14)
        case .unknownCounts: counts = (3, nil)
        case .heldSessions: counts = (4, 4)
        case .armedFolder: counts = (3, 1)
        }
        let owed = counts.1.map { #""decisions_owed":\#($0),"# } ?? ""
        let loggedIn = set == .empty ? "false" : "true"
        let privateAI = set == .normalDay || set == .busyQueue ? #"{"state":"running","port":8463}"# : #"{"state":"off","port":null}"#
        let routing = set == .normalDay || set == .busyQueue
            ? #"{"state":"rows_seen","derived":true,"last_refresh_at":"2026-09-30T09:14:00Z","unreadable_rows":0}"#
            : #"{"state":"not_declared","derived":false,"last_refresh_at":null,"unreadable_rows":0}"#
        let held = #"{"held_sessions":0,"reasons":[],"projects":[]}"#
        return #"{"schema_version":"trace_commons.daemon.v1_1","logged_in":\#(loggedIn),"tenant_id":\#(set == .empty ? "null" : "\"tenant-sample\""),"consent_scopes":["debugging_evaluation"],"paused":false,"queue_depth":\#(counts.0),\#(owed)"next_digest_at":"2026-09-30T18:00:00Z","health":{"last_error_label":null,"since":null},"daily_budget":{"bytes_today":1048576,"max_bytes_per_day":209715200,"bytes_remaining":208666624,"uploads_today":4,"max_uploads_per_day":50,"uploads_remaining":46,"resets_at":"2026-10-01T00:00:00Z","blocked":false,"blocked_entries":0,"blocked_bytes":0},"routing":\#(routing),"private_inference_state":\#(privateAI),"grant_voids":[],"witness_capacity":{"waiting_sessions":0,"next_retry_at":null},"legacy_invite_migration":{"offered":false,"notice":null},"arming_rewordings":[],"automatic_contribution_held":\#(held)}"#
    }

    // MARK: - list_projects

    static func projectRow(
        _ p: Project, mode: String = "notify_only", configured: Bool = false, sessions: Int?, last: String?,
        pending: Int, fromNow: Bool? = nil
    ) -> String {
        let added = configured ? "\"2026-09-02T10:00:00Z\"" : "null"
        let lastJSON = last.map { "\"\($0)\"" } ?? "null"
        let sessionsJSON = sessions.map(String.init) ?? "null"
        let fromNowJSON = fromNow.map { #","from_now":\#($0)"# } ?? ""
        return #"{"project_id":"\#(p.id)","project_label":"\#(p.label)","project_path":"\#(p.path)","mode":"\#(mode)","added_at":\#(added),"configured":\#(configured),"is_unresolved_bucket":false,"session_count":\#(sessionsJSON),"last_session_at":\#(lastJSON),"pending_count":\#(pending)\#(fromNowJSON)}"#
    }

    static func projects(_ set: Sample) -> String {
        let rows: [String]
        var unpurposed = 0
        switch set {
        case .empty, .coreDown:
            rows = []
        case .normalDay, .unknownCounts:
            rows = [
                projectRow(api, sessions: 22, last: "2026-09-30T11:58:00Z", pending: 2),
                projectRow(web, configured: true, sessions: 9, last: "2026-09-30T10:31:00Z", pending: 1),
            ]
            unpurposed = 1
        case .busyQueue:
            rows = [
                projectRow(api, sessions: 61, last: "2026-09-30T19:40:00Z", pending: 4),
                projectRow(web, sessions: 38, last: "2026-09-30T18:02:00Z", pending: 5),
                projectRow(infra, sessions: 17, last: "2026-09-30T16:11:00Z", pending: 5),
            ]
            unpurposed = 3
        case .heldSessions:
            rows = [
                projectRow(api, mode: "auto_upload", configured: true, sessions: 22, last: "2026-09-30T12:40:00Z", pending: 2, fromNow: true),
                projectRow(web, mode: "auto_upload", configured: true, sessions: 9, last: "2026-09-30T11:31:00Z", pending: 2, fromNow: false),
            ]
        case .armedFolder:
            rows = [
                projectRow(infra, mode: "auto_upload", configured: true, sessions: 17, last: "2026-09-30T14:16:00Z", pending: 3, fromNow: true),
                projectRow(api, configured: true, sessions: 22, last: "2026-09-29T17:00:00Z", pending: 0),
                projectRow(web, mode: "ignore", configured: true, sessions: 9, last: "2026-09-28T09:00:00Z", pending: 0),
            ]
        }
        return #"{"projects":[\#(rows.joined(separator: ","))],"unpurposed_traces":\#(unpurposed)}"#
    }

    // MARK: - harness_list

    static func harnessList(_ set: Sample) -> String {
        let connected = set == .normalDay || set == .busyQueue
        let state = connected ? "answering" : "not_connected"
        let last = connected ? "\"2026-09-30T09:04:11+00:00\"" : "null"
        let spend: String
        switch set {
        case .normalDay, .busyQueue: spend = #"{"known":true,"micros":1230000}"#
        case .empty: spend = #"{"known":true,"micros":0}"#
        default: spend = #"{"known":false,"micros":null}"#
        }
        let families = connected ? #"[{"family":"anthropic","last_call_at":"2026-09-30T09:04:11+00:00","calls":12}]"# : "[]"
        return #"{"catalog_present":false,"harnesses":[{"id":"claude","name":"Claude Code","installed":true,"connected":\#(connected),"config_path":"/Users/sample/.claude/settings.json","connect_command":"ironwire connect claude","family":"anthropic","answers_at":"Anthropic","state":"\#(state)","last_call_at":\#(last),"can_connect":\#(!connected),"can_disconnect":\#(connected)},{"id":"codex","name":"Codex","installed":true,"connected":false,"config_path":"/Users/sample/.codex/config.toml","connect_command":"ironwire connect codex","family":"openai","answers_at":"OpenAI","state":"not_connected","last_call_at":null,"can_connect":true,"can_disconnect":false}],"activity":{"readable":\#(connected),"window_hours":24,"last_call_at":\#(last),"families":\#(families)},"spend":\#(spend),"destination_port":\#(connected ? "8463" : "null"),"destination_credentialed":\#(connected)}"#
    }

    // MARK: - get_settings

    static func settings(_ set: Sample) -> String {
        let scrubCheck = set == .heldSessions ? "manual" : "automatic"
        let schedule = set == .normalDay ? #"{"mode":"evening","hour":18}"# : #"{"mode":"interval"}"#
        let notifications = set == .empty ? "false" : "true"
        let watch = set == .empty ? "unset" : "watch"
        return #"{"schema_version":"trace_commons.daemon_settings.v1","poll_interval_secs":60,"quiescence_secs":1800,"digest_interval_secs":14400,"digest_schedule":\#(schedule),"queue_ttl_days":14,"max_uploads_per_day":50,"max_bytes_per_day":209715200,"max_queue_entries":500,"approval_hold_secs":10,"local_notifications":\#(notifications),"scrub_check_defaulted_on_upgrade":false,"opencode_source_mode":"unset","claude_source_mode":"\#(watch)","codex_source_mode":"\#(watch)","gemini_source_mode":"off","cline_source_mode":"unset","ironwire_attested_bodies":false,"private_inference":\#(set == .normalDay || set == .busyQueue),"private_inference_offer_seen":true,"scrub_check":"\#(scrubCheck)","near_ai_configured":true,"near_ai_inference_configured":true,"claude_root_configured":false,"codex_root_configured":false,"admission_evidence_required":false}"#
    }

    // MARK: - list_history, history_rollup, commons_credit_summary

    static func historyRow(
        _ n: Int, _ p: Project, status: String, unattended: Bool?, verdict: String?, pending: Double,
        final: Double?, withdrawnAt: String? = nil, day: Int
    ) -> String {
        let id = String(format: "10000000-0000-4000-8000-%012d", n)
        let unattendedJSON = unattended.map { String($0) } ?? "null"
        let verdictJSON = verdict.map { "\"\($0)\"" } ?? "null"
        let finalJSON = final.map { String($0) } ?? "null"
        let withdrawn = withdrawnAt.map { "\"\($0)\"" } ?? "null"
        return #"{"submission_id":"\#(id)","submitted_at":"2026-09-\#(String(format: "%02d", day))T15:00:00Z","project_id":"\#(p.id)","project_label":"\#(p.label)","source":"claude-code","session_hash":"sha256:h\#(n)","status":"\#(status)","consent_scopes":["debugging_evaluation"],"credit_points_pending":\#(pending),"credit_points_final":\#(finalJSON),"explanations":[],"last_refreshed_at":"2026-09-30T09:00:00Z","withdrawn_at":\#(withdrawn),"approved_unattended":\#(unattendedJSON),"approved_verdict":\#(verdictJSON)}"#
    }

    static func history(_ set: Sample) -> String {
        let rows: [String]
        switch set {
        case .empty, .coreDown:
            rows = []
        default:
            rows = [
                historyRow(1, api, status: "submitted", unattended: false, verdict: "worked", pending: 3.5, final: nil, day: 30),
                historyRow(2, infra, status: "accepted", unattended: true, verdict: nil, pending: 0, final: 6.0, day: 29),
                historyRow(3, web, status: "quarantined", unattended: false, verdict: "partly", pending: 2.0, final: nil, day: 27),
                historyRow(4, api, status: "withdrawn", unattended: false, verdict: "failed", pending: 0, final: nil, withdrawnAt: "2026-09-26T10:00:00Z", day: 25),
                // Predates provenance: approved_unattended null is "not recorded".
                historyRow(5, web, status: "accepted", unattended: nil, verdict: nil, pending: 0, final: 4.0, day: 12),
            ]
        }
        return #"{"history":[\#(rows.joined(separator: ","))]}"#
    }

    static func rollup(_ set: Sample) -> String {
        if set == .empty || set == .coreDown {
            return #"{"week":{"submitted":0,"accepted":0,"quarantined":0,"withdrawn":0,"other":0},"month":{"submitted":0,"accepted":0,"quarantined":0,"withdrawn":0,"other":0},"all_time":{"submitted":0,"accepted":0,"quarantined":0,"withdrawn":0,"other":0},"credit_pending":0.0,"credit_final":0.0,"quarantined":0,"taken_back":0,"last_refreshed_at":null}"#
        }
        return #"{"week":{"submitted":1,"accepted":1,"quarantined":1,"withdrawn":1,"other":0},"month":{"submitted":1,"accepted":2,"quarantined":1,"withdrawn":1,"other":0},"all_time":{"submitted":1,"accepted":2,"quarantined":1,"withdrawn":1,"other":0},"credit_pending":5.5,"credit_final":10.0,"quarantined":1,"taken_back":1,"last_refreshed_at":"2026-09-30T09:00:00Z"}"#
    }

    static func credit(_ set: Sample) -> String {
        switch set {
        case .normalDay, .busyQueue, .armedFolder, .heldSessions:
            return #"{"posture_state":"known","commons_settlement":"disabled","commons_settlement_explanation":"Sample settlement explanation, supplied by the commons","commons_graded":false,"points_state":"known","commons_points_earned_this_period":12,"commons_points_lifetime_earned":45,"commons_pending_review":2,"commons_currency_code":null,"commons_currency_earned_this_period":null,"commons_period_start":"2026-09-01T00:00:00Z","commons_period_end":"2026-10-01T00:00:00Z","observed_at":"2026-09-30T09:17:15Z"}"#
        default:
            return #"{"posture_state":"unknown","commons_settlement":null,"commons_settlement_explanation":null,"commons_graded":null,"points_state":"unknown","commons_points_earned_this_period":null,"commons_points_lifetime_earned":null,"commons_pending_review":null,"commons_currency_code":null,"commons_currency_earned_this_period":null,"commons_period_start":null,"commons_period_end":null,"observed_at":"2026-09-30T09:17:15Z"}"#
        }
    }

    // MARK: - tool_destinations, inference_calls

    static func toolDestinations(_ set: Sample) -> String {
        let running = set == .normalDay || set == .busyQueue
        let route = set == .empty ? "not_enrolled" : "witness"
        let to = set == .empty ? "[]" : #"["commons","witness"]"#
        let watch = set == .empty ? "not_watched" : "watched"
        let folders: String
        switch set {
        case .armedFolder: folders = #"{"armed":1,"ask_first":1,"ignored":1}"#
        case .heldSessions: folders = #"{"armed":2,"ask_first":0,"ignored":0}"#
        case .busyQueue: folders = #"{"armed":0,"ask_first":3,"ignored":0}"#
        case .empty, .coreDown: folders = #"{"armed":0,"ask_first":0,"ignored":0}"#
        default: folders = #"{"armed":0,"ask_first":2,"ignored":0}"#
        }
        let claudeCalls = running ? #"{"to":"near_ai","basis":"observed"}"# : #"{"to":"anthropic","basis":"tool_default"}"#
        let codexCalls = running ? #"{"to":"near_ai","basis":"configured"}"# : #"{"to":"openai","basis":"tool_default"}"#
        return #"{"private_ai":"\#(running ? "running" : "off")","sessions_route":"\#(route)","folders":\#(folders),"tools":[{"tool":"claude-code","name":"Claude Code","sessions":{"watch":"\#(watch)","to":\#(watch == "watched" ? to : "[]")},"model_calls":\#(claudeCalls)},{"tool":"codex","name":"Codex","sessions":{"watch":"\#(watch)","to":\#(watch == "watched" ? to : "[]")},"model_calls":\#(codexCalls)},{"tool":"gemini-cli","name":"Gemini CLI","sessions":{"watch":"off","to":[]},"model_calls":{"to":"google","basis":"tool_default"}},{"tool":"cline","name":"Cline","sessions":{"watch":"not_watched","to":[]},"model_calls":{"to":"unknown","basis":"tool_default"}},{"tool":"opencode","name":"OpenCode","sessions":{"watch":"not_watched","to":[]},"model_calls":{"to":"unknown","basis":"tool_default"}}]}"#
    }

    static func inferenceCalls(_ set: Sample) -> String {
        guard set == .normalDay || set == .busyQueue else {
            return #"{"readable":false,"window_hours":24,"calls":[],"next_cursor":null}"#
        }
        return #"{"readable":true,"window_hours":24,"calls":[{"id":414,"at":"2026-09-30T09:04:11+00:00","tool":"claude-code","family":"anthropic","model":"zai-org/GLM-4.6","route":"routed","cost":{"known":true,"priced_micros":8400},"proof":"verified"},{"id":413,"at":"2026-09-30T08:58:40+00:00","tool":"codex","family":"openai","model":"Qwen/Qwen3.6-27B-FP8","route":"routed","cost":{"known":true,"priced_micros":12300},"proof":"gateway_only"},{"id":412,"at":"2026-09-30T08:51:02+00:00","tool":"unknown","family":"unknown","model":"unknown","route":"outside","cost":{"known":false,"priced_micros":null},"proof":"outside"},{"id":411,"at":"2026-09-30T08:40:19+00:00","tool":"claude-code","family":"anthropic","model":"zai-org/GLM-4.6","route":"routed","cost":{"known":true,"priced_micros":5100},"proof":"pending"},{"id":410,"at":"2026-09-30T08:31:55+00:00","tool":"claude-code","family":"anthropic","model":"zai-org/GLM-4.6","route":"routed","cost":{"known":true,"priced_micros":4900},"proof":"failed"}],"next_cursor":null}"#
    }

    // MARK: - preview, preview_unsure_spans

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
