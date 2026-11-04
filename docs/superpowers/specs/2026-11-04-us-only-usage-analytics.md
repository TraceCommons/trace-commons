# Anonymous usage counts, US only

Status: DRAFT, NEEDS APPROVAL (2026-11-04). MVP built on branch
`insight-ops` (#1920); see "Built" and "Follow-ups".

## Why

Insights runs entirely on the contributor's computer, so nothing tells the
team whether anyone opens it, which tabs they use, whether setup gets
finished, or whether people come back. This adds the smallest useful signal:
weekly totals of a fixed list of events, each week tagged with one anonymous
install id, from the United States only.

## The rule (owner, 2026-11-04)

**US by server IP only, on by default.** A count is kept and sent only when
both hold:

1. The `share_usage_data` setting is on. Default on; the person can turn it
   off in Settings, which deletes the counts not yet sent and the install id.
2. The commons answered `GET /v1/public/usage-region` with
   `analytics_allowed: true` within the last 7 days. The server looks the
   request's IP address up in a MaxMind-format country database
   (`TRACE_COMMONS_USAGE_REGION_MMDB`) and answers true only for `US`. No
   database, a lookup error, an unattributed address or another country
   answers false. The address is dropped after the lookup.

There is no check of the computer's own region setting (dropped by the
owner for the MVP). The region is asked every 3 days (6 hours after a
failed ask) while the setting is on. Outside the US nothing is counted,
stored or sent, and there is no prompt anywhere.

## The anonymous install id

A random UUIDv4 made on the computer the first time counting turns on,
kept in the daemon's `usage-analytics.json`, and sent with every weekly
batch as `install_id`. It is what lets the team count returning users
(installs active in more than one week).

- It is not derived from, stored with or sent beside any device key,
  account, enrollment, path or hardware id. Nothing in the database joins
  to it.
- Turning the setting off deletes it; turning it back on makes a new one.
- The server accepts only a version 4 UUID; anything else is refused.

## Events (the whole list)

`tab_home_opened`, `tab_private_ai_opened`, `tab_traces_opened`,
`tab_insights_opened`, `insights_overview_opened`,
`insights_breakdown_opened`, `insights_sessions_opened`,
`insights_analyze_opened`, `insights_benchmarks_opened`,
`insights_refresh_clicked`, `insights_analyze_clicked`, `setup_finished`.

Defined once in `trace_commons_protocol::usage_analytics::USAGE_EVENTS`.
Anything else is refused by the daemon and by the server.

## What is never sent or stored

No device key, account, tenant, enrollment, session or hardware id. No IP
address (looked up for the region answer, then dropped; never stored or
logged). No paths, project or file names, prompts, model names or trace
content. Nothing finer than an ISO week. A batch carries only `install_id`
and `{event, count, week}` rows; any other field refuses the whole batch.

## Server storage (V152)

`trace_usage_event_weeks (install_id, week, event, event_count,
updated_at)`, primary key `(install_id, week, event)`.

These rows belong to no tenant and arrive before anyone has an account, so
the table has no tenant column or tenant policy, like V145's cross-tenant
table. It stays inside the repo's rules this way:

- Row security is enabled and forced; PUBLIC has nothing.
- The only role with privileges or a policy on it is
  `trace_usage_events_guard` (NOLOGIN, NOSUPERUSER, NOBYPASSRLS).
- The ingest runtime cannot read, update or delete a row. It may only call
  `trace_usage_events_record(install_id, weeks[], events[], counts[])`, a
  SECURITY DEFINER function owned by the guard, granted to
  `trace_ingest_runtime` (and to the migrator, V149's reason).
- Column checks repeat the protocol's bounds (week form, event form, count
  1 to 100,000) as a backstop.
- A week's count is replaced, not added to, so a resend after a lost
  response does not double it.

`POST /v1/usage-events` is unauthenticated, rate-limited per address and
globally, body-limited to 64 KiB, validated whole with
`UsageEventBatch::validate`, and logs fixed labels only. It does not ask the
region again (owner, 2026-11-04: too early).

## Built (MVP)

- Protocol: `usage_analytics` (routes, events, batch with install id and
  validation, ISO week, `new_install_id`).
- Server: `GET /v1/public/usage-region`, `POST /v1/usage-events`, V152.
- Daemon: `usage_analytics` (gate, install id, weekly totals, region check,
  weekly send, at most 8 unsent weeks), the `share_usage_data` setting, IPC
  `record_usage_event`.

## Follow-ups

1. macOS shell: call `record_usage_event` on each tab and Insights sub-tab,
   Analyze, Refresh and setup finished; a Settings > General switch "Share
   anonymous usage data" with a one-line description. Strings in the core
   copy tables.
2. Windows, GTK and Tauri, after macOS.
3. A way for the team to read the totals (an admin route or an operator
   query run as the guard).
4. Retention for `trace_usage_event_weeks` (none yet).
5. Server-side US re-check on the events route, if wanted later.

## Open questions (owner and legal)

- Wording of the switch and its description.
- The Privacy Policy paragraph on tracecommons.ai, which must now mention
  the install id.
- The first-run promise "Nothing leaves this Mac until you choose" is no
  longer true for US users with the switch on; it needs rewording.
- Whether NEAR sees the totals.
- Legal review of on-by-default for US users with an install id
  (California's opt-out right is met by the switch).
