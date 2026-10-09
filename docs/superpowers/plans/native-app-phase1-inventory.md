# Native app: phase 1 inventory

> **Snapshot 2026-10-01, superseded by `docs/superpowers/plans/2026-10-02-macos-glass-rebuild-and-r15-cutover.md`. Do not use it for current status.** It is kept as the record #1173 was planned from (Refs #1173). Many gaps it lists have since closed.


Read from `origin/main` at `ba8a8777` (Publish set_project_mode changes when a write fails, #1171) and from PR #1146 at `09a26c0d` (`claude/tc-monitor-frontend-refactor-8ecf22`). Nothing in the working tree was touched.

**Route key**

- **FFI:** a dedicated export in `crates/trace-commons-contributor-ffi`.
- **IPC:** a daemon method in `METHODS` (`daemon/ipc.rs`, 94 methods), reached with `tc_call`. `tc_call` goes through `handle_local` and then `block_on_ipc`, so it also reaches the methods listed in `ASYNC_ONLY_METHODS`.
- **OS:** a platform capability. The core has no route for it and doesn't need one: SwiftUI uses AppKit, `SMAppService`, `UNUserNotificationCenter` or `NSWorkspace` directly.
- **NONE:** the capability exists only through Tauri's Rust. That code calls a core function that has no FFI export or IPC method, or it holds the logic itself.

**Logic flag (P2):** Tauri's own Rust holds logic beyond pass-through: validation, assembly, choices or ConfigStore reads. This logic should move into `trace-commons-contributor` in phase 2.

**Used by macOS today?** "yes" means `macos/Sources` already calls the route. "no" means it doesn't.

---

## 1. Tauri commands vs the C ABI

Main registers **163** commands (`tauri-desktop/src-tauri/src/commands/mod.rs`). #1146 adds **1**, `crate::glass::set_glass_regions`, for **164**.

### 1a. daemon.rs (26)

| Command | macOS route | macOS uses? | Logic flag (P2) |
|---|---|---|---|
| core_status | IPC `status`, plus FFI `tc_daemon_start*` / `tc_daemon_attach` for lifecycle | yes | Tauri builds the `startup` enum (`needs_roots` / `daemon_unavailable` / `running`) and a placeholder status object (`state.rs::core_status`). This is shell state; macOS has `DaemonStartup.swift`. |
| retry_daemon_startup | FFI `tc_daemon_start` / `tc_daemon_start_with_settings` / `tc_daemon_attach` | yes | - |
| queue_outcome_line | FFI `tc_queue_outcome_line` | yes | - |
| residual_secret_line | **NONE.** `preview_copy::residual_secret_line` is not exported. | no; macOS writes its own in `TCShellCore/RedactionLabels.swift` | **P2:** export it, then delete the Swift copy. |
| redaction_summary_copy | **NONE.** `redaction_summary::rows` is not exported. | no; duplicated in `TCShellCore/RedactionSummary.swift` | **P2:** export it, then delete the Swift copy. |
| project_ignore_copy | **NONE.** `project_copy::ignore_project_*` is not exported. | no; duplicated in `TCShellCore/ProjectIgnoreCopy.swift` | **P2:** export it. |
| arming_offer_copy | **NONE.** `project_copy::arming_offer_*`, `ARMING_BODY_WITH_BACKLOG` and `customize_copy()` (K5) are not exported. | no; `ProjectArmingCopy.swift` holds a GTK copy of the text, and Customize / "Keep on this Mac" is missing | **P2:** export it. |
| attestation_copy | FFI `tc_contribution_attestation_line/_tone/_reason_line` | yes | Maps the tone enum to a string (trivial). |
| certificate_copy | FFI `tc_certificate_list_title`, `tc_certificate_row_line`; `certificate_list_empty` is in `tc_private_inference_copy` | yes | - |
| eligibility_copy | FFI `tc_contribution_eligibility_line/_reason_line/_control` | yes | - |
| eligibility_group_copy | FFI `tc_contribution_group_control` + `tc_contribution_withheld_line` | yes | **P2 (small):** `eligible = contributable.unwrap_or(pending).min(pending)` and `withheld = pending - eligible` are computed in Tauri. One core helper should return the whole view. |
| daemon_call (read-only pass-through for 10 methods) | IPC (all 10) | yes | **P2:** before the daemon starts, `bootstrap_settings` and `bootstrap_profile` read ConfigStore and DaemonSettings directly and answer `get_settings` / `get_public_profile`. No route exists before the daemon starts. |
| preview_entry | IPC `preview`, plus `gate_statement` from FFI `tc_consent_copy` | yes | Injects `gate_statement` (assembly). |
| dismiss_entry | IPC `dismiss` | yes | - |
| approve_entry | IPC `approve` | yes | - |
| approve_project | IPC `approve` (project_id) | yes | - |
| pause_daemon | IPC `pause` | yes | - |
| resume_daemon | IPC `resume` | yes | - |
| arming_suggestion | IPC | yes | - |
| accept_arming | IPC `set_project_mode` (auto_upload) | yes | - |
| decline_arming | IPC | yes | - |
| cancel_entry | IPC `cancel` | yes | - |
| cancel_project | IPC `cancel` | yes | - |
| preview_body | FFI `tc_preview_open` / `tc_preview_body` (also IPC) | yes (FFI) | - |
| preview_turns | FFI `tc_preview_turns_json` (also IPC) | yes (FFI) | - |
| search_original | FFI `tc_search_original` / `tc_preview_search` (also IPC) | yes (FFI) | - |

### 1b. profile.rs (2), settings.rs (9), privacy.rs (6), routing.rs (4)

| Command | macOS route | macOS uses? | Logic flag (P2) |
|---|---|---|---|
| publish_profile | IPC `set_public_profile` | yes | - (macOS copy comes from GTK in `PublicProfileCopy.swift`) |
| withdraw_profile | IPC `clear_public_profile` | yes | - |
| set_quiescence_minutes | IPC `set_settings` | **no UI** | **P2:** range 1..=240 and the minutes-to-seconds conversion live only in Tauri. The core takes any u64 (`settings.rs:1241`). |
| set_approval_hold_seconds | IPC `set_settings` | **no** | **P2:** range 0..=300 lives only in Tauri. |
| set_digest_hours | IPC `set_settings` | **no UI** | **P2:** range 1..=24 and the hours-to-seconds conversion. |
| set_max_uploads_per_day | IPC `set_settings` | **no UI** | **P2:** range 1..=1000. |
| set_max_bytes_per_day_mb | IPC `set_settings` | **no UI** | **P2:** range 1..=5120 MB and the MB-to-bytes conversion. |
| set_source_declaration | IPC `set_settings`. Before the daemon starts: FFI `tc_daemon_start_with_settings` | yes | **P2:** `existing_directory` canonicalises the path. The pre-daemon branch writes ConfigStore through `apply_settings_object` and `roots_declared`, then starts the daemon. |
| set_project_mode | IPC | yes | - |
| list_projects | IPC | yes | - |
| list_audit | IPC (limit 20) | yes | - |
| set_private_inference | IPC `set_settings` {private_inference, offer_seen} | yes | - |
| set_inference_evidence | IPC `set_settings` {ironwire_attested_bodies} | yes | **P2 (small):** the `confirmed` / `disclosure-required` gate is enforced only in the shell. |
| set_token_contribution | IPC `set_settings` | yes | Same `confirmed` gate. |
| set_token_capture | IPC `set_settings` | yes | Same `confirmed` gate. |
| token_storage_status | IPC | macOS reads it through `get_settings` | - |
| clean_token_storage | IPC `discard_token_reviews` / `remove_token_local_copies` | yes | `confirmed` gate. |
| discover_routing | IPC | yes | - |
| configure_routing | IPC `set_settings` {ironwire} | yes | **P2:** port ≠ 0 and the absolute token_dir check run in Tauri, and again in `TCShellCore/RoutingSurface.swift`. |
| probe_routing | IPC | yes | Same validation. |
| probe_routed_tools | IPC | yes | Same validation. |

### 1c. witness.rs (6)

| Command | macOS route | macOS uses? | Logic flag (P2) |
|---|---|---|---|
| witness_status | FFI `tc_witness_status_json` | yes | Tauri reads ContributorConfig directly. |
| configure_witness | FFI `tc_witness_configure` | yes | **P2:** URL, address and pin validation is written twice, in Tauri `witness.rs` and in FFI `lib.rs`, and neither copy is in the core. Move it to `config` or `witness`. |
| clear_witness | FFI `tc_witness_clear` | yes | - |
| route_disclosure | IPC `route_disclosure` + FFI `tc_route_disclosure_copy` | yes | Tauri parses the facts into `RouteDisclosure` and refuses unknown shapes. |
| route_disclosure_unreadable_copy | FFI | yes | - |
| certificate_detail | IPC + FFI `tc_certificate_detail_copy` | yes | - |

### 1d. consent.rs (18)

| Command | macOS route | macOS uses? | Logic flag (P2) |
|---|---|---|---|
| consent_options | IPC. Before the daemon starts, Tauri calls `daemon::enroll::consent_options()`, which is **not exported**. | yes (IPC) | Pre-daemon route missing. |
| scrubber_pattern_names | FFI `tc_scrub_detector_names` | yes | - |
| enroll_with_invite | IPC `enroll` | yes | - |
| set_consent_scopes | IPC | yes | - |
| acknowledge_near_ai_notice | IPC | yes | - |
| grant_void_notice | FFI `tc_grant_void_notice` | yes | **P2:** the FFI exports `void_notice_for_wire`, while Tauri uses `void_notice_for_wire_with_regrant`. macOS needs the regrant variant once it has Flow 1 screens. |
| witness_capacity_notice | FFI | yes | - |
| arming_reworded_notice | FFI | yes | - |
| gate_held_notice | FFI | yes | - |
| acknowledge_arming_rewordings | IPC | yes | - |
| acknowledge_grant_voids | IPC | yes | - |
| legacy_migration_copy | **NONE.** `consent_copy::legacy_migration_offer()` is not exported. | no | **P2:** export it. |
| legacy_migration_notice | FFI `tc_legacy_migration_notice` | yes | - |
| migrate_legacy_invite | IPC `legacy_invite_migrate`. The refusal line comes from `consent_copy::legacy_migration_refusal_line`, which is **not exported**. | **no** | **P2:** Tauri turns an IPC error into `{refused, line}`. Export the refusal line, or have the daemon answer with it. |
| acknowledge_legacy_invite_migration | IPC | yes | - |
| automatic_grant | IPC | **no** | - |
| grant_automatic | IPC | **no** | **P2:** `grant_precondition` (confirmed, enrolled, `consent_scopes_chosen`) reads ContributorConfig in Tauri, and `tauri_copy_surface_is_central` pins it there. Move it into the daemon's `grant_automatic`. |
| withdraw_automatic_grant | IPC | **no** | - |

### 1e. inference_connection.rs (6), the K12 onboarding step

| Command | macOS route | macOS uses? | Logic flag (P2) |
|---|---|---|---|
| inference_connection_copy | **NONE.** `consent_copy::inference_connection_copy()` is not exported. | no | **P2:** export it. |
| inference_connection_offers | IPC, plus `consent_copy::inference_connection_disclosure` per offer (**not exported**) | no | **P2:** `with_disclosures` attaches each offer's disclosure in Tauri. The daemon or a core helper should do it. |
| inference_connection_current | IPC | no | - |
| inference_connection_select | IPC | no | **P2:** `select_params` refuses an unconfirmed offer or an unknown disclosure version before the daemon is asked. |
| inference_connection_install | IPC | no | `confirmed` gate. |
| inference_connection_disconnect | IPC | no | - |

### 1f. private_ai.rs (10)

| Command | macOS route | macOS uses? | Logic flag (P2) |
|---|---|---|---|
| start_private_ai_credential | IPC `near_ai_credential_start` | yes | Tauri defaults the provider to `"github"`. |
| private_ai_credential_status | IPC `near_ai_credential_status` + FFI `tc_near_ai_credential_state_line` / `_action` | yes | **P2:** the `keychain` block (state, key_prefix, minted_at, session_expires_at, migration) comes from `DaemonSettings::load_with_cloud_credentials` in Tauri, and **no core route serves it**. The `view` assembly also happens here. |
| private_ai_balance | IPC `near_ai_balance` + FFI `tc_near_ai_balance_*` | yes | View assembly. |
| private_ai_funding | IPC `near_ai_funding` | yes | Param shaping (both values or neither). |
| private_ai_harnesses | IPC `harness_list` + FFI `tc_harness_state_line` / `_credential_notice` / `_spend_line` | yes | View assembly. |
| plan_harness | IPC `harness_plan` + FFI `tc_harness_outcome_line` | yes | `can_commit` (outcome == "changes" and a plan_id is present) is computed in Tauri. macOS uses `tc_harness_plan_outcome_code`. |
| commit_harness | IPC `harness_commit` | yes | - |
| cancel_private_ai_credential | IPC | yes | - |
| forget_private_ai_credential | IPC | yes | - |
| migrate_private_ai_credential | IPC | yes | - |

### 1g. platform.rs (12)

| Command | macOS route | macOS uses? | Logic flag (P2) |
|---|---|---|---|
| open_external_url | OS (`NSWorkspace`) | own | **P2:** the URL allowlist (NEAR credits page, tracecommons.ai/runs, fixture commits, loopback callback) lives in Tauri. |
| platform_capabilities | OS (bundle, `SMAppService`, `UNUserNotificationCenter`), plus Homebrew detection | own (`TCUpdates/HomebrewDetector`) | Tauri's `update_state` duplicates `TCUpdates`. |
| notification_permission | OS | yes (`Notifier`) | - |
| request_notification_permission | OS | yes | - |
| set_start_at_login | OS | yes (`LoginItemManager`) | - |
| open_system_settings | OS | yes | - |
| quit_app | OS | yes | - |
| quit_confirmation_copy | **NONE.** `quit_copy::quit_prompt(QuitRole)` is not exported. | no; `AppDelegate.swift:177` hard-codes "Quit Trace Commons?" | **P2:** export it, with the role taken from the handle (hosting, attached or none). |
| open_native_wallet_url | OS | own | Single-use URL authorisation in `state.rs`. |
| open_account_sign_in_url | OS | **no** | Same single-use authorisation. |
| pick_directory | OS (`NSOpenPanel`) | yes | `existing_directory` / `git_repository` validation. |
| consume_deep_link | OS (`onOpenURL`) + core `commands::invite_from_deep_link` (**not exported**; `tc_invite_issuer_host` is a different export) | partial | **P2:** run, credential and review link parsing and invite-link validation are about 150 lines of Tauri Rust. |

### 1h. native_flows.rs (9)

| Command | macOS route | macOS uses? | Logic flag (P2) |
|---|---|---|---|
| native_wallet_flow | IPC | yes | Action allowlist, plus `browser_url` authorisation. |
| contributor_disclosure_copy | **Mixed.** FFI covers `tc_witness_copy`, `tc_onboarding_copy`, `tc_source_settings_copy`, `tc_source_check_line`, `tc_insights_copy_json`, `tc_mission_drafts_call` {copy}, `tc_public_run_copy` (the awaiting_pii_backstop label) and `tc_private_inference_copy`. In #1146, `states` also come from `tc_private_inference_state_line` / `_tone`. **NONE** for `privacy_scan_copy`, `outcome_copy` (only `tc_outcome_refusal_line` exists), `history_copy::HELD_ROW_BODY` and `AUTO_PROJECT_DISCLOSURE_UNAVAILABLE`. | partial; macOS hard-codes the privacy-scan text (`OnboardingPrivacyScanView.swift:84-103`) | **P2:** the whole bundle is assembled in Tauri, and #1146 adds a 10-label state map with a unit test there. Move it into a core `contributor_disclosure_copy()` plus one FFI export. |
| witness_review_copy | FFI `tc_witness_copy` (.review) | yes | - |
| automatic_contribution_copy | **NONE.** `automatic_gate::disclosure(config)` followed by `consent_copy::automatic_grant_copy` | no | **P2:** the choice of disclosure (R1) is made in Tauri and pinned there by `tauri_copy_surface_is_central`. It needs a core function plus FFI or IPC. |
| project_automatic_contribution_copy | IPC `project_automatic_copy` | **no** | - |
| near_ai_account_enroll | IPC + FFI `tc_near_ai_enroll_line` | yes | Turns an error into a view. |
| prepare_admission_session | IPC | yes | `confirmed` gate. |
| witness_preview_support | IPC `hello` | yes | - |
| witness_preview_request | IPC | yes | `raw_session_confirmed` gate. |

### 1i. history.rs (17)

| Command | macOS route | macOS uses? | Logic flag (P2) |
|---|---|---|---|
| history_detail | IPC | yes | - |
| request_history_refresh | IPC `refresh_history` | yes | - |
| account_session_status | **NONE.** `account_auth::try_load_token` / `session_status` | **no** | **P2:** needs an IPC method or an FFI export. |
| account_sign_in | **NONE.** `account_auth::sign_in` (browser loopback) | **no** | **P2:** the same. Tauri also authorises the sign-in URL. |
| withdrawal_confirmation_prompt | **NONE.** `withdraw::confirmation_prompt_unknown()` is not exported. | no; duplicated in `Views/WithdrawalCopy.swift` | **P2:** export it. |
| withdraw_history | IPC `withdraw` | yes | - |
| validate_public_run_editor | FFI `tc_public_run_validate_editor` | yes | - |
| publish_public_run | IPC | yes | - |
| unpublish_public_run | IPC | yes | - |
| skill_learning_copy | FFI `tc_skill_learning_copy` | yes | - |
| skill_candidate | IPC | yes | - |
| skill_review | IPC | yes | - |
| skill_evaluate | IPC | yes | - |
| skill_install_plan | IPC | yes | - |
| skill_install_commit | IPC | yes | - |
| skill_install_status | IPC | yes | - |
| skill_install_rollback | IPC | yes | - |

### 1j. Insights (29), mission drafts (4) and compute (5)

| Commands | macOS route | macOS uses? | Logic flag (P2) |
|---|---|---|---|
| insights_list, insights_summary, analyze_insight, explain_insight, delete_insight, annotate_insight, clear_insight_annotation, link_test_report, link_git_insight, unlink_insight_evidence, insights_question_cards, insights_episode_{list,create,explain,annotate,clear_assessment,delete} (17) | FFI `tc_insights_call` (`insights::service::dispatch_json`) | yes | Tauri stages uploaded bytes in a temp file with size caps (16 MiB / 64 KiB) and validates git repos. This is shell plumbing. |
| comparison_task_{create,list,explain,replace_episodes,set_context,set_outcome,clear_outcome,reconfirm}, comparison_specification_{list,preview,save,evaluate} (12) | FFI `tc_insights_call` | yes | Tauri maps `comparison_task_list`'s not_found to an empty list. |
| mission_draft_{list,show,import,delete} (4) | FFI `tc_mission_drafts_call` | yes | Import bytes are staged in a temp file. |
| compute_status, enable_compute, resume_compute, pause_compute, disable_compute (5) | FFI `tc_compute_status_json` / `tc_compute_command_json` | yes | - |

### 1k. Added by #1146

| Command | macOS route | Logic flag (P2) |
|---|---|---|
| set_glass_regions (`glass.rs` + `native_macos.m`) | OS. SwiftUI uses `.glassEffect` / `NSGlassEffectView` on macOS 26 and `NSVisualEffectView` before that. | None for the core. The rect math only exists because a webview is in the way. |
| (changed) contributor_disclosure_copy | It gains `private_inference.states`, `state_unknown` and `state_unreported`. FFI covers it with `tc_private_inference_state_line` / `_tone`. | Same assembly flag as in 1h. |

### 1l. Totals

| Route | Commands (of 164) |
|---|---|
| IPC only (`tc_call`) | 71 |
| FFI only (dedicated export, including the 38 dispatched through `tc_insights_call`, `tc_mission_drafts_call` and `tc_compute_*`) | 61 |
| IPC + FFI (data from IPC, words from FFI) | 9 |
| OS-native (no core route needed) | 12 (11 platform + set_glass_regions) |
| **NONE (no route for macOS)** | **11** |

By file: daemon 26, profile 2, settings 9, witness 6, routing 4, consent 18, inference_connection 6, privacy 6, private_ai 10, platform 12, native_flows 9, history 17, insights 29, mission_drafts 4, compute 5 (= 163), plus set_glass_regions.

The 11 with no route are residual_secret_line, redaction_summary_copy, project_ignore_copy, arming_offer_copy, legacy_migration_copy, inference_connection_copy, automatic_contribution_copy, quit_confirmation_copy, withdrawal_confirmation_prompt, account_session_status and account_sign_in. Four more are counted under other routes above but have a no-route part: contributor_disclosure_copy (4 missing pieces), private_ai_credential_status (the `keychain` block), migrate_legacy_invite (the refusal line) and consume_deep_link (the parsing).

**IPC methods no Tauri command and no macOS screen uses:** `inference_calls` and `tool_destinations` (K8: the map and the Inference tab), `commons_credit_summary` (K7: settlement posture and credit points), `keep`, `list_kept` and `undo_keep` (K5: "Keep on this Mac"), `withdraw_bulk`, `near_account_*` and `quiesce`. #1146 lists exactly these as its design gaps ("no calls-and-cost table, spend, proof counts", "Kept tile is Credit pending"), but **the daemon already serves most of that data.** Neither shell reads it.

---

## 2. Screens: #1146 React vs SwiftUI (`macos/Sources/TraceCommonsApp/Views`)

| #1146 screen or panel | SwiftUI counterpart | Status |
|---|---|---|
| **Chrome** | | |
| Monitor window: three glass panes (tree, map, inspector), overlay title bar, 1320×760 | `MainWindowView` (a `NavigationSplitView` with a drawn sidebar) | exists, needs redesign (different structure) |
| Toolbar: view menu (Show ignored folders), graph/map/inspector toggles, round Settings button | `MainWindowCommands` (menu items only) | **missing** |
| Tabs Home · Inference · Traces, with the decisions-owed badge and Private AI dot | sidebar rows with badges in `MainWindowView` | exists, needs redesign |
| Core notices: startup, grant voids, legacy migration, arming rewordings, gate held | `DaemonStartupNotice`, `GrantVoidNotices`, `LegacyMigrationNoticeCard`, `ArmingRewordingNotices`, `GateHeldNoticeCard` in `MainWindowView.swift` | exists and matches (content); the placement moves to the top of the left pane |
| Quit confirmation | `AppDelegate` NSAlert + `QuitCoordinator` | exists, needs redesign (the copy is hard-coded, not from the core) |
| **Home** | | |
| Home view: status, counts, Missions and History cards | none | **missing** |
| Missions (Home sub-view with breadcrumb) | `MissionDraftsView`, `MissionDraftDetailView` | exists, needs redesign (move under Home, add the breadcrumb) |
| History (Home sub-view): rows, filter, refresh, detail, public-run editor, community, credit record, skill learning, withdrawal | `HistoryView`, `SessionDetailView` (`PublicRunEditor`), `SessionContributionOverview`, `CreditRecordView`, `CommunitySection`, `SkillLearningView` | exists, needs redesign |
| History account sign-in control | none | **missing** (no route; see 1i) |
| **Traces (left pane)** | | |
| Traces tree: tool › folder › session | `QueueView` + `QueueFolderRow` (a list grouped by project, with no tool level) | **missing** (nearest is QueueView) |
| Tool watch switch + `tool-watch-dialog` (source-settings disclosure, folder picker) | `SourceRootRow` in Settings only | **missing** in the tree |
| Folder switch (Never offer / Ask me first) | `QueueFolderRow` ignore confirmation | exists, needs redesign |
| Folder Submit "Submit all eligible" + withheld line | `QueueFolderRow` "Submit all (n)" | exists, needs redesign |
| Row menu (ignore folder, dismiss session) | `QueueRow` dismiss, `QueueFolderRow` ignore | exists, needs redesign |
| Traces graph (shared/kept per period) + binoculars focus | `WeekBand` (a "This week" KPI band) | **missing** (not the same visualisation) |
| **Flow map (middle pane)** | | |
| Traces map: tools, folders ringed by rule, arc to the commons | none | **missing** |
| Private AI map: harness nodes, links to the NEAR AI credential, node cards (hover/pin) | none (`HarnessListView` lists the same facts) | **missing** |
| **Inspector (right pane)** | | |
| Summary: decisions, statistics, queue safeguards (queue-status-panel, queue-outcome-disclosure, witness capacity, daily budget) | parts of the `QueueView` header (`HealthBanner`, `NotOfferedDisclosure`, witness capacity, `DailyBudgetCopy`) | exists, needs redesign |
| Certificate panel | `CertificateSection` | exists and matches (placement moves) |
| Tool / folder detail with contribution rule | none | **missing** |
| Session review: preview-inspector, waiting-review, redacted transcript, "Exactly what would be sent", outcome fields, Contribute | `PreviewSheet` (tabs: Search, What's in it, Transcript, Permissions), `SessionSendDisclosureView`, `ScrubbingCaveat` | exists, needs redesign (sheet becomes inspector pane) |
| Undo bar (pinned top, auto-opens the inspector) | `UndoBar` | exists and matches (placement moves) |
| Arming offer, including K5 Customize / Keep on this Mac | `ArmingOfferCard` (no Customize, no Keep) | exists, needs redesign |
| Private inference offer | `PrivateInferenceOfferCard` | exists and matches |
| Witness review overlay | `PreviewSheet` → `WitnessReviewConsent` sheet | exists and matches |
| Admission preparation overlay | `AdmissionPreparationView` | exists and matches |
| NEAR AI notice recovery | `AppModel` acknowledge flow plus the health banner | exists, needs redesign |
| Submit-all-as (outcome for bulk) | `ApproveParams` "all" in `QueueView` | exists, needs redesign |
| **Inference** | | |
| Inference tab (Private AI page: connection, credential action, balance, funding, harness list + harness preview) | `PrivateInferenceView`, `CredentialSection`, `BalanceRow`, `FundingRow`, `HarnessListView` (`HarnessPreviewSheet`, `HarnessExposureSheet`) | exists, needs redesign |
| Inference inspector (status line, tools, balance) | none | **missing** |
| Calls and cost per model, spend, proof counts, insight card (design; not built in #1146) | none | **missing** in both shells |
| **Settings modal** (section nav + one scrolling body) | `SettingsView` (a sidebar destination, one column) | exists, needs redesign |
| 1. Connection (+ legacy migration panel) | `connection` | connection matches; **legacy migration panel missing** (notice only) |
| 2. Startup & notifications | `loginItem`, `notifications`, `updatesSection` | exists and matches (macOS also has Updates "Check now") |
| 3. Watching: pause/resume, session discovery, behaviour settings | `watching` | pause/resume matches; **behaviour settings missing** (quiescence, approval hold, digest, daily caps) |
| 4. How traces may be used: consent + automatic grant panel | `consent` | consent matches; **automatic grant panel missing** |
| 5. Public profile | `publicProfile` + `GoPublicDialog` | exists and matches |
| 6. Watched folders | `watchedFolders` + `SourceRootRow` | exists and matches |
| 7. Tools (routing) | `routing` | exists and matches |
| 8. Private AI (state line + link) | `privateInference` ("settings moved" pointer) | exists, needs redesign (add the core state line) |
| 9. Redaction witness: witness, privacy controls, route disclosure | `witness` + `RouteDisclosureSection` + token/evidence confirmation dialogs | exists and matches |
| 10. Projects (+ project-mode confirmation, project automatic disclosure) | `projects` + arming confirmation | exists; **project automatic disclosure missing** (`project_automatic_copy` unused) |
| 11. Changes on this machine | `audit` | exists and matches |
| 12. Compute | `ComputeView` (its own sidebar destination) | exists, needs redesign (move it into Settings) |
| **First run** (single pane + step progress: Folders, Join, Uses, Inference, Sharing, Projects) | `OnboardingCoordinatorView` (steps: welcome, roots, connect, consent, privacyScan, projects, done) | exists, needs redesign |
| welcome | `OnboardingWelcomeView` | exists, needs redesign |
| roots | `OnboardingRootsView` | exists, needs redesign |
| connect (invite, connection options, wallet, NEAR AI join) | `OnboardingConnectView`, `NearAccountConnectView`, `NearAiJoinView` | exists, needs redesign |
| consent (scope picker) | `ConsentScopesView` | exists, needs redesign |
| path (ask-first vs automatic) | none | **missing** |
| privacy (extra scrub) | `OnboardingPrivacyScanView` (hard-coded copy) | exists, needs redesign |
| inference (K12 connect inference) | none | **missing** |
| Flow 1: disclosure_scrub (K11) | none (`WhatGetsRemovedSheet` covers ask-first only) | **missing** |
| Flow 1: disclosure_witness (K11) | none (`RouteDisclosureBody` can be reused) | **missing** |
| Flow 1: grant | none | **missing** |
| projects | `OnboardingProjectsView` | exists, needs redesign |
| done | `OnboardingDoneView` | exists, needs redesign |
| Automatic-contributing re-grant flow (from Settings and from the void notice) | none | **missing** |
| **Modals not listed above** | | |
| Withdrawal confirmation | `WithdrawalConfirmationView`, `WithdrawalConfirmationCapture` | exists and matches |
| Scrub disclosure (onboarding) | `WhatGetsRemovedSheet` | exists and matches |
| Public-profile consent | `GoPublicDialog` | exists and matches |
| Ignore-project confirmation | `QueueFolderRow` `.confirmationDialog` | exists and matches |
| Mission draft delete | `MissionDraftsView` `.confirmationDialog` | exists and matches |
| **Menu bar** popover (in the design; #1146 did not build it because the Tauri tray is a native menu) | `MenuBarView` (`MenuBarLabel`, `MenuBarContent`) | exists, needs redesign |
| (Insights: no place in the #1146 design) | `InsightsView` and children, plus comparisons | macOS only; the design has a gap here |

**Counts (61 design rows, not counting the Insights row):** 17 exist and match, 23 exist but need redesign, 17 missing. Another 4 are mixed: the Settings sections Connection, Watching, Uses and Projects exist, but each lacks a panel (legacy migration, behaviour settings, automatic grant, project automatic disclosure). That makes 21 missing pieces in total.

---

## 3. Tests that read React or Tauri sources

| Test | What it reads | SwiftUI counterpart needed |
|---|---|---|
| `crates/trace-commons-contributor-ffi/tests/tauri_copy_surface_is_central.rs` → `tauri_commands_project_shared_contributor_copy` | Tauri `native_flows.rs`, `daemon.rs`, `history.rs`, `consent.rs`, `inference_connection.rs`, `platform.rs`, `state.rs`. It requires calls to `automatic_gate::disclosure`, `automatic_grant_copy`, `grant_precondition`/`consent_scopes_chosen`, eligibility helpers, `confirmation_prompt_unknown`, `void_notice_for_wire_with_regrant`, `inference_connection_*`, the notice helpers, `gate_statement`, `quit_copy::quit_prompt` and the `QuitRole` mapping. | Yes. Once that logic moves to the core, the test should assert it at the core or FFI layer and that `macos/Sources/TCBridge` calls those exports. It is the main brake on moving the logic: it pins grant_precondition and the disclosure choice inside Tauri files. |
| same file → `copy_commands_reach_the_frontend_through_tauri_and_render_at_safety_surfaces` | `build.rs`, `commands/mod.rs`, and about 35 React files (`contributor-copy-api.ts`, `grant-void-notices.tsx`, `app-routes.tsx`, `app-shell.tsx`, `switch-on-notices.tsx`, `quit-confirmation.tsx`, `waiting-page.tsx`, `queue-status-panel.tsx`, `preview-inspector.tsx`, `waiting-review.tsx`, `session-send-disclosure.tsx`, `witness-review-overlay.tsx`, `admission-preparation-overlay.tsx`, `arming-offer.tsx`, `private-inference-offer.tsx`, `withdrawal-control.tsx`, `history-row.tsx`, `history-view-model.ts`, every onboarding step, `project-automatic-disclosure.tsx`, `use-automatic-grant.ts`, `route-disclosure.tsx`, ...) | Yes. It needs a Swift-source equivalent that checks each safety surface (notices, review, witness review, admission, withdrawal, quit, onboarding and Flow 1 steps) renders core copy. #1146 kept file paths stable only to satisfy this test. |
| same file → `tauri_never_says_private_inference_to_a_contributor` | all of `tauri-desktop/frontend/src` and `src-tauri/src` | Yes, as a scan of `macos/Sources/TraceCommonsApp`. |
| same file → `legacy_migration_copy_is_central_and_the_shell_holds_no_literal` | `legacy-migration-panel.tsx`, `legacy-migration-notice.tsx`, `consent.rs` | Yes, once macOS gets the migration panel. |
| `tauri-desktop/src-tauri/tests/command_permissions.rs` (the #1165 guard, merged) | `build.rs` `TAURI_COMMANDS` vs `permissions/default.toml` | No equivalent needed (Tauri ACL only). The closest native analogue is a check that every IPC method or FFI export a screen needs is declared in `macos/Sources/CTraceCommons/include/trace_commons.h`, which `abi_header_surface.rs` already does. |
| `crates/trace-commons-contributor-ffi/tests/abi_header_surface.rs` | `macos/Sources/CTraceCommons/include/trace_commons.h` (macOS, not Tauri) | Already native. Every new export in phase 2 must be added to both headers. |
| `crates/trace-commons-contributor-ffi/tests/harness_copy_is_central.rs` | macOS `HarnessSurface.swift`, `TCHarness.swift`, `AppModel.swift`, plus Windows and GTK (not Tauri) | Already native. Update it if the harness UI moves into the Inference inspector. |
| Tauri unit tests in `src-tauri/src` (51 `#[test]`: consent 7, native_flows 6 (+1 in #1146), witness 5, platform 5, native 5, inference_connection 4, state 4, tray 4, app 3, macos_signing 3, credential_store_check 2, runtime 2, history 1, routing 1) and `glass.rs` (4, #1146) | Tauri Rust logic | Yes, for each logic move: grant_precondition, select_params, witness configure validation, deep-link parsing, URL allowlist, the private-AI state map. The tests move with the logic into core tests. |
| `tauri-desktop/src-tauri/src/macos_signing.rs` + `scripts/ci/test-verify-macos-entitlements.sh` + `scripts/ci/check-release-credential-store.sh` | Tauri `entitlements.plist`, `tauri.release.conf.json`, `Cargo.toml`, compared against `macos/entitlements.plist` | They need changes when the Tauri macOS bundle retires. The native-side check stays. |
| `crates/trace-commons-server/tests/license_boundary.rs` | `tauri-desktop/src-tauri/Cargo.toml` | No change while Tauri ships on Windows and Linux. |
| Frontend `.test.mjs` (14): `flow1`, `inference-connection`, `roots-readiness`, `health-recovery`, `history-view-model`, `withdrawal-eligibility`, `automatic-grant-copy`, `decisions-owed`, `grant-void-notice`, `legacy-migration`, `quit-confirmation-copy`, `route-disclosure`, `switch-on-notices`, `witness-capacity` | React view models | Swift already has `GrantVoidNoticeTests`, `LegacyMigrationNoticeTests`, `RouteDisclosureTests`, `SwitchOnNoticesTests`, `WitnessCapacityTests`, `MenuBarStatusTests` (decisions owed), `SessionRootsTests` and `HistoryFoldersTests`. **No Swift counterpart:** flow1, inference-connection, automatic-grant-copy, quit-confirmation-copy, health-recovery, withdrawal-eligibility. |
| macOS source-scan tests that pin current view files (`ShellWordingTests`, `SettingsPointerTests`, `ConsentBindingTests`, `RoutingBindingTests`, `WitnessBindingTests`, `SourceCheckBindingTests`, `AttestationRowTests`, `AdmissionPlacementTests`, `CertificateSectionTests`, `ActionNoticeDismissTests`, `LineHeightScalingTests`, `NearAiJoinViewTests`, `SkillLearningRenderingTests`, `ComputeNavigationTests`, ...) | `SettingsView.swift` (6 reads), `PreviewSheet.swift` (4), `AppModel.swift`, and others | Not a Tauri dependency, but each one breaks when the redesign splits or renames those views. Plan to rewrite them. |

---

## 4. Phase 2 work list

### 4a. Local-data half: sessions, queue, previews, projects, tools, history, settings, consent copy

**Core and FFI moves** (do these first, so SwiftUI never copies Tauri logic):

1. **A `contributor_disclosure_copy()` in the core plus one FFI export.** This replaces Tauri's assembly in `native_flows.rs`, including #1146's `private_inference.states`. It also adds exports for `privacy_scan_copy`, `outcome_copy`, `history_copy::HELD_ROW_BODY` and `AUTO_PROJECT_DISCLOSURE_UNAVAILABLE`, and deletes the hard-coded privacy-scan text in `OnboardingPrivacyScanView.swift`.
2. **Exports for the copy macOS currently duplicates in Swift:**
   - `preview_copy::residual_secret_line` and `redaction_summary::rows` (replace `RedactionLabels.swift` and `RedactionSummary.swift`);
   - `project_copy` ignore, arming offer, `ARMING_BODY_WITH_BACKLOG` and `customize_copy` (replace `ProjectIgnoreCopy.swift` and `ProjectArmingCopy.swift`);
   - `withdraw::confirmation_prompt_unknown` (replace part of `WithdrawalCopy.swift`);
   - `quit_copy::quit_prompt(role)`, with the role taken from the handle (replace the alert text in `AppDelegate`);
   - while there, check `DigestCopy.swift`, `DailyBudgetCopy.swift` and `PublicProfileCopy.swift` against the core.
3. **Flow 1 grant logic into the core:**
   - `automatic_gate::disclosure` plus `automatic_grant_copy`, exposed as an FFI export or an IPC method `automatic_grant_copy`;
   - `grant_precondition` (confirmed, enrolled, `consent_scopes_chosen`) inside the daemon's `grant_automatic`;
   - an FFI variant of `void_notice_for_wire_with_regrant`.
   - Then rewrite `tauri_copy_surface_is_central::tauri_commands_project_shared_contributor_copy` to assert at the core and FFI layer.
4. **Settings validation into `daemon::settings::apply_settings_object`:**
   - ranges and unit conversions for quiescence, approval hold, digest, max uploads and max bytes;
   - the source-path canonicalisation (`existing_directory`);
   - the routing checks: port ≠ 0 and an absolute token_dir (now written in Tauri and again in `RoutingSurface.swift`).
5. **Witness configure validation into the core.** It is now duplicated in Tauri `witness.rs` and FFI `tc_witness_configure`.
6. **Eligibility group helper.** One core function returns `{can_contribute, eligible_count, withheld_line}`.
7. **Confirmation gates in the daemon.** `confirmed` / `disclosure-required` for inference evidence, token contribution, token capture and token discard. They are enforced only in shells today.
8. **Pre-daemon reads.** `consent_options` and the bootstrap settings and profile, or a documented rule that macOS starts the daemon first.
9. **Deep-link parsing and the external-URL allowlist into the core.** Invite validation, `tracecommons://run|credential|review`, the NEAR credits page, `tracecommons.ai/runs` and fixture commits, each with an FFI export.

**SwiftUI** (against #1146):

10. **Monitor shell:**
    - replace the `NavigationSplitView` sidebar with three glass panes (tree, flow map, inspector);
    - add the toolbar (view menu, graph/map/inspector toggles, Settings button);
    - add the Home · Inference · Traces segmented tabs, with the decisions-owed badge and the Private AI dot;
    - use native glass (`NSGlassEffectView` / `NSVisualEffectView`);
    - open the inspector by itself when a demand appears (undo, a review just opened, a folder submit, an offer).
11. **Traces tree, tool › folder › session.** Built from `list_pending`, `list_projects`, `get_settings` source modes and `list_history`. It needs:
    - the tool watch switch with a tool-watch dialog (the source-settings disclosure, saving disabled until the copy loads);
    - the folder Never offer / Ask me first switch;
    - Submit all eligible with the withheld line;
    - the row menu.
12. **Traces graph and Home.** The graph shows shared/kept per period. Home shows status, counts, and Missions and History cards. Missions and History become Home sub-views with a breadcrumb.
13. **Flow map, Traces view.** Tools, folders ringed by rule and the commons arc, with node cards and binoculars focus.
14. **Inspector.**
    - **Summary:** decisions, statistics, safeguards, certificate panel.
    - **Tool and folder detail:** with the contribution rule.
    - **Session review:** `PreviewSheet` becomes an inspector pane.
    - **Pinned at the top:** undo, the arming offer and the Private AI offer.
    - **K5:** Customize and "Keep on this Mac" (`keep` / `list_kept` / `undo_keep` are already IPC).
15. **Settings modal** with section nav and 12 sections. New panels:
    - behaviour settings;
    - automatic grant;
    - project automatic disclosure (`project_automatic_copy`);
    - the Private AI state line;
    - Compute moved in from the sidebar.
16. **First run and Flow 1.**
    - Restyle into a single pane with step progress.
    - Add the **path**, **disclosure_scrub**, **disclosure_witness** and **grant** steps.
    - Add the re-grant flow from Settings and from the grant void notice.
17. **Tests.**
    - A Swift-source equivalent of `copy_commands_reach_the_frontend...` and `tauri_never_says_private_inference...`.
    - Swift tests for flow1, automatic-grant-copy, quit-confirmation-copy, withdrawal-eligibility and health-recovery.
    - Rewrites of the macOS source-scan tests the redesign breaks.

### 4b. Network-data half: IronWire inference, spend, proofs, credit, settlement, missions, invite, passkeys, updater, Private AI

1. **Inference tab and inspector.** Calls and cost per model, spend, and IronWire proof counts. `inference_calls` and `tool_destinations` are already IPC (K8; they read the local routing ledger). Neither shell uses them. Core copy is still needed for proof labels and the cost lines (check `inference_map` for copy first). The Inference inspector shows the status line, tools and balance.
2. **Flow map, Private AI and model view.** Harness nodes, the NEAR AI credential link, model and outside-model nodes, and call counts, from `harness_list` plus `tool_destinations` and `inference_calls`. #1146 left out the model nodes, call counts and pulses.
3. **Credit and settlement.** `commons_credit_summary` is IPC (K7: settlement posture plus account points) but unused. Use it for Home's credit/"Kept" tile and the History credit record. The design's "Top 8%" standing and credit multipliers have no route and are off-limits for projected credit.
4. **Missions.** Vendor missions and credit multipliers have **no route** in the contributor core. Only local drafts exist (`tc_mission_drafts_call`).
5. **Invite and enrollment.**
   - The K12 inference-connection onboarding step: IPC `inference_connection_*` exists. Export `inference_connection_copy` and the disclosure per version, and move `select_params` and `with_disclosures` into the core.
   - The legacy invite migration panel: IPC `legacy_invite_migrate` exists. Export `legacy_migration_offer` and the refusal line, and add the macOS panel.
   - Invite deep links (shared with 4a item 9).
6. **Account session and passkeys.** `account_sign_in` and `account_session_status` have **no route**: Tauri calls `account_auth` directly. Add IPC methods or FFI exports, and the History sign-in control. Passkeys exist only on the server (`account_native_passkey.rs`), with no contributor-side route in either shell.
7. **Private AI.**
   - Restyle the existing SwiftUI credential, balance, funding and harness screens into the Inference tab.
   - Move the `keychain` block that Tauri builds in `private_ai_credential_status` (key prefix, minted time, session expiry, migration) into the daemon's `near_ai_credential_status`. No route serves it today.
   - Move the default provider ("github") and `can_commit` into the core.
8. **Updater.** Tauri only reports managed/unmanaged and has no feed. macOS is ahead (`TCUpdates`, `UpdateController`). Wire the design's "Check now" to `UpdateController`. Retire Tauri's `update_state` on macOS.
9. **Wallet, witness preview and admission.** These already work on macOS. Restyle them only, and keep the single-use browser URL authorisation behaviour.
10. **Release.** Retire the Tauri macOS bundle from `tauri-desktop.yml`, `test-verify-macos-entitlements.sh`, `check-release-credential-store.sh` and `macos_signing.rs`. Keep the native entitlements checks.
