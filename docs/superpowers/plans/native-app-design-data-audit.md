# Native app: design-data audit

Snapshot of the WYSIWYG design checked against `main` at ba8a8777 (2026-10-01), for #1173. Most gaps listed here have since been closed by the K and Z items; this is the record the plan was built from.


Audited at `origin/main` ba8a8777 (2026-10-01). Read-only. Sources: design.txt / design.html (2026-09-28), PR #1146's monitor files and gap list, #1030, `daemon/ipc.rs` METHODS, `docs/contributor-daemon-ipc-v1_1.md`, `macos/Sources`.

**Limits of the source material.** design.html pulls the window mocks (Traces, Missions, Inference, History, menu bar) from an external `TC Monitor Window` component that isn't in the file. Those rows come from the design's flow text, its swim lanes and #1146's implementation and gap list. Missions also uses Michael's mechanism draft (missions-doc.txt).

**Transport.** The macOS app talks to the daemon through the C ABI, not the socket directly. It calls `tc_call(handle, method, params)` (`ffi/src/lib.rs:1163`), which runs the same handlers the socket serves, in process or attached. So every IPC method below is reachable from Swift with no new ABI work. The gaps are the features that have no IPC method at all: passkeys, invite lookup and the mission catalogue. Most K1–K9 fields aren't decoded in Swift yet; `git grep` finds no `inference_calls`, `tool_destinations`, `commons_credit_summary`, `keep`/`list_kept`, `scrub_check`, `started_at`, `user_turns`, `second_look`, `include_backlog`, `unpurposed_traces`, `taken_back` or `approved_unattended` in `macos/Sources`. That's KN0 and is assumed for every A row.

Classes: **A** backed · **B** partly backed · **C** no data · **D** native-only platform work.

## First run: Join (W-1, W-1b)

| screen | element | class | source on main, or what's missing | owner |
|---|---|---|---|---|
| Join | Invite link field + "Look up" | B | Server route `/v1/invite/lookup` (#1121) and the client function `IssuerClient::lookup_invite` (`issuer_client.rs:187`) exist, but nothing calls it. There's no IPC method and no C ABI export. | daemon/K (IPC + ABI wrapper) |
| Join | "names the issuer and its pay range" | B | `InviteLookupResponse.issuer_display_name` and `credit_range{min,max,unit=points_per_accepted_trace}` exist server-side only, and need the same IPC method. The pay range is in points per accepted trace, and the product must decide whether showing it counts as projected credit. | daemon/K; product decision on the copy |
| Join | Redeem invite / join commons | A | `enroll`, `near_ai_account_enroll`, `near_account_capabilities` | — |
| Join | "Sign in with near.ai" | A | `near_account_start` / `near_account_status`, `native_wallet_flow`, `near_ai_credential_start` | — |
| Join | "Create passkey" card | C | There's no passkey IPC or ABI in the contributor or FFI crates and no passkey code in `macos/`. The Z2 server side is merged (#1135, #1136, #1124, #1127). | daemon/K or Z (ZN4 client) |
| Join | Skip | A | Native only. Nothing is enrolled. | native UI |

## Passkey popups (P-1 to P-7)

| screen | element | class | source on main, or what's missing | owner |
|---|---|---|---|---|
| P-1 | Use existing / Create new passkey | C | No client passkey surface | daemon (ZN4) |
| P-2 | Name the passkey, loss warning | C | No storage for the name; the copy isn't in the core | daemon (ZN4); product (copy approval) |
| P-3/P-4 | macOS "Save a passkey?" + Touch ID | D | Needs `ASAuthorizationPlatformPublicKeyCredentialProvider`. `macos/entitlements.plist` has no `com.apple.developer.associated-domains`, and the provisioning profile isn't generated. The AASA file is smoke-tested server-side (#1124). | native UI (+ Apple provisioning) |
| P-5 | Verify passkey / bind to near.ai; cancelling signs out | C | Server bind exists (#1135 `account_binding.rs`). There's no daemon method to request the challenge, sign or report binding state. | daemon (ZN4) |
| P-6 | macOS Sign In sheet | D | ASAuthorization assertion, plus the same entitlement | native UI |
| P-7 | Welcome back / Sign in with passkey / Other options | C | No stored-passkey state or sign-in method in the daemon | daemon (ZN4) |

## First run: Folders and tools (W-2, W-4)

| screen | element | class | source on main, or what's missing | owner |
|---|---|---|---|---|
| Folders | Detected tools, path, "62 sessions · last one 4 hours ago" | A | `tc_discover_sources` (`exists`, `session_count`, `most_recent`, `answers_at`) | — |
| Folders | "Not found on this Mac · Get Codex" | A | `tc_discover_sources.exists`. The link is static. | native UI |
| Folders | Antigravity at `~/.gemini/tmp`, "answers at Google" | B | `~/.gemini/tmp` is the **Gemini CLI** adapter (`gemini-cli`), and "Google" comes from `source_answers_at`. Antigravity conversations come in through a separate import adapter (`src/antigravity/`) and appear only as `declared_source`, not as their own tool row. | product decision (label); daemon/K if Antigravity needs its own row |
| Folders | Watch / Different folder / I don't use it per tool; Continue disabled until each is answered | A | `set_settings` `claude_source`…`opencode_source` (`watch`/`off`/unset). Unset is distinguishable. | — |
| Tools | "+ Add another tool or machine / dev server over SSH / drag & drop" | C | No remote-machine or SSH source. Only the five local adapters. | daemon/K + product decision |
| Tools | OpenCode, Theia IDE, Cursor in that list | B | OpenCode exists (`opencode_source`, export dir). There's no Theia or Cursor adapter. | daemon/K |

## First run: Rules and past sessions (W-5)

| screen | element | class | source on main, or what's missing | owner |
|---|---|---|---|---|
| Rules | "Repos found in Claude Code sessions" | B | `list_projects` has per-repo rows (`session_count`, `last_session_at`) but no tool/source per project, so the rows can't be filtered by tool. They can be inferred only from pending entries' `source`. | daemon/K |
| Rules | Ask me / Share automatically / Never per repo | A | `set_project_mode` (`notify_only`/`auto_upload`/`ignore`), arming from now by default (K5) | — |
| Rules | "22 sessions" per repo | A | `list_projects.session_count` | — |
| Rules | "6 sessions · client" tag | C | No classification of a repo as client work | product decision |
| Past sessions | Picker by folder, "N of 43 selected", "Show all 22" | A | `list_pending` with `project_id`, then a per-entry `approve` (doc: "The past-session picker"); `pending_count` | — |
| Past sessions | Row date and duration ("Sat 12 Sep · 1 h 18 min") | A | `started_at`, `duration_secs` on every entry (K1) | — |
| Past sessions | Row title ("fix payments retry test") | B | `title` exists only on preview summaries, never in `list_pending`. A list has to schedule previews (`preview_request`) to get titles. | daemon/K (title on the entry) |
| Past sessions | Folder whose rule is Never ("6 · rule is Never") | A | `list_projects.mode = ignore` | — |

## First run: Uses, sharing, Private AI (W-3, W-6)

| screen | element | class | source on main, or what's missing | owner |
|---|---|---|---|---|
| Uses | Always-on use and the 3 optional uses | A | `consent_options` (`always_on`, `description`), `set_consent_scopes` (needs an existing enrollment, so it isn't available after Skip) | — |
| Uses | "List my handle publicly" toggle | A | `set_public_profile` (needs a handle and an account; the design shows only a toggle) | native UI |
| Sharing | Share automatically / Ask me each time | A | `grant_automatic` / `withdraw_automatic_grant` / `automatic_grant`, `set_project_mode` | — |
| Sharing | "Undetermined sessions wait for your approval" | A | `scrub_check: automatic` (K4, now the default, #1162) | — |
| Private AI | Switch, off by default | A | `inference_connection_offers`, `_select`, `_install`; `harness_plan`/`harness_commit`; `set_settings.private_inference` | — |
| Start | "Start watching" → Traces, menu-bar eye | A | Settings write and `status` | — |

## Traces tab, tree and toolbar

| screen | element | class | source on main, or what's missing | owner |
|---|---|---|---|---|
| Traces | Tree: tool › folder › session | A | `list_pending` (`source`, `project_id`), `list_projects`, `tool_destinations.tools[]` | — |
| Traces | Tool switch | A | `*_source` settings, `source_settings` copy; `tool_destinations.sessions.watch` | — |
| Traces | Folder rule chip and switch | A | `set_project_mode`, `list_projects.mode`/`from_now` | — |
| Traces | Session row title | B | Same as the picker: only after a preview loads | daemon/K |
| Traces | Session row turns and duration | A | `user_turns`, `duration_secs` | — |
| Traces | Session size in KB (pending) | B | `size_bytes` on every entry is the **raw file** size. The would-send size (`would_send_bytes`) exists only after a preview. | daemon/K if the row must show the would-send size |
| Traces | Byte sums per folder ("sums bytes") | B | Can be summed app-side from raw `size_bytes`. There's no daemon total, and it isn't what would be sent. | daemon/K or native UI |
| Traces | Scrub state "Scrubbed · 7 marks", amber eye/shield | A | `scrub`, `marks`, `content_marks`, `second_look` (K3); `QueueShieldState.swift` | — |
| Traces | "Worth a second look" grouping | A | `second_look` reasons `nothing-matched` / `looks-unsure` / `trimmed-to-fit` | — |
| Traces | Folder "Submit all" (count and withheld line) | A | `approve{project_id}`, `pending_count`/`contributable_count`, `tc_contribution_withheld_line`, `tc_contribution_group_control` | — |
| Traces | Badge on the Traces tab | A | `status.decisions_owed` (K6) | — |
| Traces | Upsell "27 unpurposed traces are waiting" | A | `list_projects.unpurposed_traces` (K7) | — |
| Traces | Graph: shared/kept per period | B | Counts only: `history_rollup` week/month buckets and `list_kept`. History records carry no size, so a KB graph isn't possible. | daemon/K (bytes on `HistoryRecord`) |
| View menu | Show ignored folders | A | `list_projects.mode` | — |
| View menu | Group by | D | The fields exist (`source`, `project_id`, `started_at`, `scrub`). Grouping is client-side. | native UI |
| View menu | Sort by | D | The fields exist (`started_at`, `size_bytes`, `user_turns`, `marks`). | native UI |
| View menu | Other users' sessions | C | The daemon is per OS user, and nothing reads other accounts. Doing so raises privacy and consent questions. | product decision |
| View menu | Float on top | D | `NSWindow.level = .floating`. No data needed. | native UI |
| Toolbar | Settings button, pane toggles | D | Window shell (KN1) | native UI |

## Review sheet (W-7 "Exactly what would be sent") and inspector

| screen | element | class | source on main, or what's missing | owner |
|---|---|---|---|---|
| Sheet | Header "orchard-api · add rate limiter" | A | `project_label` + preview `title` | — |
| Sheet | "Sat 12 Sep · 29 min · 12 turns · Claude Code" | A | `started_at`, `duration_secs`, `user_turns`/`turn_count`, `source` | — |
| Sheet | "Nothing has been sent. This is what would be.", consent sentence | A | Core consent copy (ABI) | — |
| Sheet | Redacted: "0 marks. No detector matched…" | A | `marks`, `second_look`, `redactions` labels | — |
| Sheet | "Leaves this Mac: 19 KB · 12 turns · …" | A | `preview_turns.leaves_this_mac.line`/`fields`. The mock's "project label" is wrong: the envelope carries none. | product decision (fix the mock) |
| Sheet | Body with [name] [path] [secret] [email] marks | A | `preview_body`, `preview_turns` | — |
| Sheet | "looks like an email. Not matched. Your call." | A | `preview_unsure_spans` (offsets + label) | — |
| Sheet | Search the text for a name, amber hit | A | Client search over `preview_body`; `search_original`; `RecentSearches.swift` / `OriginalSearchOutcome.swift` already exist | — |
| Sheet | Worked / Partly / Failed | A | `approve.outcome` (+ `correction`) | — |
| Sheet | Submit this session | A | `approve{entry_id}`, the approval hold / `cancel` undo | — |
| Sheet | Keep on this Mac | A | `keep` / `undo_keep` / `list_kept` (K5) | — |
| Sheet | Eligibility check | A | Entry `eligibility`/`eligibility_reason` | — |
| Inspector | Summary: decisions, statistics | A | `decisions_owed`, `queue_outcome_counts`, `history_rollup` | — |
| Inspector | Map dims to the selected tool's arcs | D | Pure UI | native UI |

## Map (Traces view and Private inference view)

| screen | element | class | source on main, or what's missing | owner |
|---|---|---|---|---|
| Map | Tool → folders (by rule) → commons / witness | A | `tool_destinations` (`sessions.to`, `sessions_route`, `folders`) | — |
| Map | Tool → NEAR AI arc (observed vs configured) | A | `tool_destinations.tools[].model_calls{to,basis}` | — |
| Map | Antigravity → Google node | B | `model_calls.to = google` exists for the **gemini-cli** row. There's no Antigravity tool row. | daemon/K + product (label) |
| Map | Outside-model red nodes | A | `model_calls.to` = vendor / `unknown`; `inference_calls.route = outside`, `proof = outside` | — |
| Map | Per-model nodes | B | Models only as free-text `inference_calls.model` over the ledger's 24 h window, aggregated app-side. There's no per-model rollup. | daemon/K |
| Map | Call counts on arcs | B | `harness_list.activity.families[].calls` is per protocol family (24 h), not per tool. `inference_calls.tool` is `unknown` unless exactly one connected tool speaks the family. | daemon/K; IronWire/Z for per-tool attribution |
| Map | Per-call pulses | B | Rows have `at`, but no event is pushed for a new call (Events: `snapshot`, `queue_changed`, `status_changed`, `digest_due`, `resync_required`, `preview_ready`), so the app has to poll `inference_calls`. | daemon/K (event) |
| Map | Cloud dev-server hub | C | No remote/dev-server source or destination concept | daemon/K + product decision |
| Map | Node cards on hover/pin, binoculars focus | D | UI over the data above | native UI |
| Map | Paused, gate and void states | A | `status` health, `automatic_contribution_held`, void notices | — |

## Inference tab

| screen | element | class | source on main, or what's missing | owner |
|---|---|---|---|---|
| Inference | Private AI On/Off and state line | A | `status.private_inference_state`, `harness_list.harnesses[].state`, core `state_line` | — |
| Inference | Table column: kind of work (`work_kind`) | C | "no field records it; there is no classifier" (doc, "What the ledger does not record") | IronWire/Z + daemon/K; post-cutoff |
| Inference | Table column: model | A | `inference_calls.model` (free text, may be `unknown`) | — |
| Inference | Table column: calls | B | Count rows app-side by paging `inference_calls` (≤200/page, 24 h window only) | daemon/K (rollup) |
| Inference | Table column: cost per model | B | `cost.priced_micros` per call is **priced, not billed**, and can't be drawn as money spent. Summed app-side. | daemon/K; product decision on wording |
| Inference | Spend per model | C | The only billed figure is `harness_list.spend` (today's metered total since local midnight, all models). IronWire `/_ironwire/summary` is **not read anywhere on main**: `routing/ironwire.rs` reads only `/_ironwire/log` and `/_ironwire/status`. So per-model spend isn't available even indirectly. | IronWire/Z + daemon/K |
| Inference | Proof per routed row, "opens a proof" | A/B | Per-row label `inference_calls.proof` (only `verified` counts) is A. Opening a proof's detail is B: there's no `inference_call_proof` method in METHODS (it's named once, in `inference_map.rs`, but not dispatched). | daemon/K (proof detail) |
| Inference | Proof counts | B | Tally `proof` app-side over the 24 h window | daemon/K (rollup) |
| Inference | Insight card ("which model wins for which work") | C | Needs `work_kind` plus outcome per call. Nothing joins calls to outcomes. ("Insights computed locally" has no input.) | daemon/K + IronWire/Z; product decision |
| Inference | Smart router auto toggle | C | "nothing records which model a router picked or why"; no router control | IronWire/Z; post-cutoff |
| Inference | Pick a model per kind of work | C | Depends on `work_kind` and a router | IronWire/Z |
| Inference | "Balance · $24.80" | A | `near_ai_balance.remaining_nanos` + `tc_near_ai_balance_amount` | — |
| Inference | Balance link-out to near.ai | A | `near_ai_funding` → verified `browser_url` (`https://cloud.near.ai/dashboard/organizations/{id}/credits`) | — |
| Inference | "Debits credit for private answers; earned first, bought after" | C | No link between commons credit and the NEAR AI balance | product decision + server/Z + NEAR AI |
| Inference | Outside-call warning: Stop / Send anyway | C | Needs interception in the proxy path. A tool not connected never reaches the ledger. | IronWire/Z; post-cutoff |

## Missions tab

Everything on main for Missions:
- The local **mission drafts** inbox, reached through `tc_mission_drafts_call` (`ffi/src/mission_drafts.rs`). It imports, lists, shows and deletes local proposal files, and never fetches, executes or funds anything.
- A server **mission catalogue**: `/v1/missions`, `/v1/missions/{id}`, reward offers, reservations and `/v1/account/rewards` (`ingest_internal/rewards.rs`, `mission_rewards.rs`). Its operator-reviewed awards carry `sponsor_hash`, `award_units` and `activity_kind`, are "independent of Trace Credit", and have "no automated completion-verification path".
- A contributor client library for that catalogue (`src/mission_catalog.rs`) that no daemon method, ABI export or Swift code calls.

| screen | element | class | source on main, or what's missing | owner |
|---|---|---|---|---|
| Missions | Local mission drafts list | A | `tc_mission_drafts_call`, `MissionDraftsModel.swift` | — |
| Missions | Vendor missions list (catalogue) | B | Server catalogue + reward programs exist, and so does a contributor client library. There's no IPC method or ABI export. | daemon/K (expose the catalogue); server/Z (catalogue content) |
| Missions | Missions "matched to vendor requests from your activity" | C | No matching. A local activity profile is needed so activity never leaves the Mac. | daemon/K + server/Z; product decision |
| Missions | Daily mission, one per day or per activity type | C | No assignment or scheduling. This is the doc's open question. | product decision, then server/Z |
| Missions | Completion tracking | C | Reward awards are operator-reviewed with no automated verification. `mission_attempt.rs` records skill-evaluation attempts, not mission completion. Reservations and award history exist only on the account route. | server/Z + daemon/K |
| Missions | Daily "factor bonus" / credit multiplier | C | Nothing applies a multiplier to credit. `account_trust_rule.rs`'s `multiplier` scales upload *allowance*, is shadow-only and isn't credit. | product decision (off-limits as projected credit) |
| Missions | "Trace Activity" accruing monthly | B | `history_rollup.month` counts and `commons_credit_summary.commons_points_earned_this_period` with period start and end. "Trace Activity" itself is undefined. | product decision; server/Z |
| Missions | Levels that raise rewards | C | None | product decision + server/Z |
| Missions | Streaks | B | Derivable locally from `list_history.submitted_at`. Nothing computes them. | daemon/K or native UI; product decision |
| Missions | Badges | C | None | product decision + server/Z |
| Missions | "Top 8%" standing | B | `history_rollup.community` has `rank`, `novelty_credit` and `accept_rate`, only for a public handle. There's no roster size or percentile, and #1146 calls this off-limits. | product decision |

**Conflicts with the design rules:** a factor bonus, a multiplier or "levels raise rewards" shown in the app is projected credit, which #1146 rules off-limits, and implies "earned" before settlement. Server mission awards are a third ledger, separate from Trace Credit, and #1118's open question of which ledger is of record is still open. Any Missions number must say which ledger it comes from and stay pending until settlement.

## History tab (W-8)

| screen | element | class | source on main, or what's missing | owner |
|---|---|---|---|---|
| History | "Credit pending · 45.5 credits" | A | `history_rollup.credit_pending` (local), or `commons_credit_summary` points. Which ledger is of record is still open. | product decision (ledger) |
| History | "Settlement off" chip + explanation | A | `commons_credit_summary.commons_settlement`/`_explanation` (Z3 #1119, K7) | — |
| History | "13 contributed · 4 under review · 2 taken back" | A | `history_rollup.all_time` accepted/submitted + `taken_back`. `quarantined` has to be shown separately, and the design has no slot for it. | product decision (quarantine slot) |
| History | Tiles: pending / contributed / kept | A | `history_rollup` + `list_kept` / `queue_outcome_counts["kept-on-this-mac"]` | — |
| History | "Open your balance on near.ai" | A | `near_ai_funding.browser_url` | — |
| History | "Use for Private AI" | C | No conversion of commons credit into inference funds | product decision + server/Z + NEAR AI |
| History | Unpurposed upsell card + "Review the 27 in Traces" | A | `unpurposed_traces` | — |
| Log row | Title "orchard-api · add rate limiter" | B | `HistoryRecord` has `project_label` but no task title | daemon/K |
| Log row | "just now · you approved · Worked" / "armed · went without asking" | A | `submitted_at`, `approved_unattended`, `approved_verdict` (K7). `null` means not recorded. | — |
| Log row | "Thu · taken back Fri" | B | `withdrawn_at` exists for local withdrawals. A server-side `revoked` from a web withdrawal has no date. | daemon/K |
| Log row | State: Under review / Contributed / Taken back | A | `status` (`submitted`/`accepted`/`withdrawn`/`revoked`) | — |
| Log row | Credit "+3.0" / "pending" / "0" | A | `credit_points_pending` / `credit_points_final` | — |
| History | Filters | A | Client-side over `status` and `project_id` | native UI |
| History | Size per trace in KB | C | No size on `HistoryRecord` | daemon/K |
| Flow note | "who took it" per trace | unclassified | Unclear whether it means the approver (A, `approved_unattended`) or which consumer took the trace (C, nothing records that). | product decision |

## Menu bar, notifications and toasts

| screen | element | class | source on main, or what's missing | owner |
|---|---|---|---|---|
| Menu bar | Eye icon, "Watching · 0 waiting" | A | `status`; `MenuBarStatus.swift` | — |
| Menu bar | Badge = decisions owed, "—" when unknown | A | `status.decisions_owed` (already used in 2 Swift files) | — |
| Menu bar | Amber shield | A | `second_look` / `QueueShieldState.swift` | — |
| Menu bar | Popover with rows "orchard-api · Scrubbed · 7 marks → Review" | D | The data is A. `MenuBarExtra` uses the default `.menu` style (`TraceCommonsAppMain.swift:38`) and needs `.window` style or an NSPopover. | native UI |
| Menu bar | Popover "Private AI · On" row + near.ai link | D | Data A (`private_inference_state`, `near_ai_funding`); needs the popover | native UI |
| Menu bar | Review opens the window on that row | D | Window routing; `MainWindowNavigation.swift` | native UI |
| Notification | Body = consent sentence; one action, "Look, then decide" | D | `tc_session_notification_copy` (K9) exists. `Notifier.swift` still registers "Review" + "Not now". | native UI |
| Digest | "1 session contributed from orchard-api. 6.0 credit pending." | A | `digest_due.text`, `DigestCopy.swift`, `digest_schedule` evening (K9) | — |
| Toast | "Sent. 1 left to decide · upload limit 7 of 20" | A | `tc_toast_sent_text` + `status.daily_budget` + `decisions_owed`. Not bound in Swift yet. | native UI |
| Toast | "Sent" after Flow 3 submit | A | Same | — |

## Settings

| screen | element | class | source on main, or what's missing | owner |
|---|---|---|---|---|
| Settings | Projects: rule per folder, change log | A | `list_projects`, `set_project_mode`, `list_audit` | — |
| Settings | Scrub check Automatic / Manual | A | `scrub_check` (K4) | — |
| Settings | Notifications and digest timing | A | `local_notifications`, `digest_schedule`, `digest_interval_secs` | — |
| Settings | Daily limits | A | `max_uploads_per_day`, `max_bytes_per_day`, `daily_budget` | — |
| Settings | Private AI section | A | Inference connection methods, `harness_list` | — |
| Settings | Updates "Check Now" | A | macOS has it: Sparkle `UpdateController.checkNow()`, `SettingsView.swift:359` (hidden for Homebrew installs) | — |
| Settings | Account / passkey section | C | ZN4 | daemon (ZN4) |
| Settings | Standard `Settings` scene (⌘,) | D | Decision 4 in the native issue | native UI |
