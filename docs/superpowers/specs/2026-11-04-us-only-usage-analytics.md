# Anonymous usage counts, US only

Status: DRAFT, NEEDS APPROVAL (2026-11-04). First slice built on branch
`insight-ops`; see "Built in this slice" and "Follow-ups".

## Why

Insights runs entirely on the contributor's computer, so nothing tells the
team whether anyone opens it, which tabs they use, or whether setup gets
finished. This adds the smallest possible signal: weekly totals of a fixed
list of events, from the United States only.

## The rule

A count is kept and sent only when all three hold:

1. The `share_usage_data` setting is on. Default on; the person can turn it
   off in Settings, which deletes the counts not yet sent.
2. The computer's region setting says United States (macOS `AppleLocale`,
   including an explicit `@rg=` region; elsewhere `LC_ALL` / `LC_MESSAGES` /
   `LANG`).
3. The commons answered `GET /v1/public/usage-region` with
   `analytics_allowed: true` within the last 7 days. The server looks the
   request's address up in a MaxMind-format country database
   (`TRACE_COMMONS_USAGE_REGION_MMDB`) and answers true only for `US`. No
   database, a lookup error or an unattributed address answers false.

The region is asked every 3 days (6 hours after a failed ask), and only when
1 and 2 hold, so a computer outside the US never makes the request. Outside
the US nothing is counted, stored or sent, and there is no prompt anywhere.

## Events (the whole list)

`tab_home_opened`, `tab_private_ai_opened`, `tab_traces_opened`,
`tab_insights_opened`, `insights_overview_opened`,
`insights_breakdown_opened`, `insights_sessions_opened`,
`insights_analyze_opened`, `insights_benchmarks_opened`,
`insights_refresh_clicked`, `insights_analyze_clicked`, `setup_finished`.

Defined once in `trace_commons_protocol::usage_analytics::USAGE_EVENTS`.
Anything else is refused by the daemon and by the server.

## What is never sent

No device, account, tenant or session identifier. No paths, project or file
names, prompts, model names, trace content or timestamps finer than an ISO
week. A request carries only `{event, count, week}` rows. The server keeps
nothing about the address it looked up and logs nothing about it.

## Built in this slice

- Protocol: `usage_analytics` (routes, event list, batch validation, ISO
  week).
- Server: `GET /v1/public/usage-region` (rate-limited per address and
  globally; `Cache-Control: private, no-store`).
- Daemon: `usage_analytics` (gate, local weekly totals in
  `usage-analytics.json`, region check, weekly send, keeps at most 8 unsent
  weeks), the `share_usage_data` setting, and IPC `record_usage_event`.

## Follow-ups

1. Server `POST /v1/usage-events`: re-check the US gate, validate with
   `UsageEventBatch::validate`, and add to weekly aggregate rows (new
   migration; no tenant, so it needs its own RLS stance and grants). Until it
   lands, the daemon's send gets a 404 and keeps the counts locally (at most
   8 weeks), so nothing is lost and nothing else is sent.
2. macOS shell: call `record_usage_event` on each tab and Insights sub-tab,
   Analyze, Refresh and setup finished; a Settings > General switch "Share
   anonymous usage data" with a one-line description. Strings in the core copy
   tables.
3. Windows, GTK and Tauri: the same, after macOS.
4. A way for the team to read the totals (an admin route or a query in the
   runbook).

## Open questions (owner and legal)

- Wording of the switch and its description, and the Privacy Policy
  paragraph on tracecommons.ai.
- The first-run promise "Nothing leaves this Mac until you choose" is no
  longer true for US users with the switch on; it needs rewording.
- Whether NEAR sees the totals.
- Legal review of on-by-default for US users (California's opt-out right is
  met by the switch).
