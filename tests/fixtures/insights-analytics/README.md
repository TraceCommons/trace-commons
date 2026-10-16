# Insights analytics responses

Responses of the local Insights service (`tc_insights_call`, or
`insights::service::execute` in process) for the Overview, Patterns and
Sessions tabs, read by the Windows and GTK shell tests so that each shell
decodes and words the same bytes.

They were produced by the service itself, not written by hand: the
`claude-turn-series` session and the `codex-alpha-direct` rollout (its dates
moved to 2026-09-15, so both land in the week of 2026-09-14) were analyzed and
saved into a temporary store under a test build, which keeps the digest key
in memory, and each operation's response was written out unchanged. The saved
Claude session has no markers, re-read files or pattern sessions, so those
three parts (`session_drill_claude.json` `markers`, `patterns.json`
`reread_files`, `pattern_sessions.json` `sessions`) are the engine's own
`DrillMarker`, `RereadRow` and `PatternSession` values, serialized by serde.

Nothing here holds a path, a message or tool ID, or session text.

`insights_week_routed.json` is the daemon's `insights_week` answer with the
route fold on (design part B), as the contributor crate's
`daemon::insights_week::tests::a_routed_week_answer_for_shell_fixtures`
prints it between its `BEGIN insights_week` and `END insights_week` lines
under `--nocapture`. That test also compares its answer to this file, so a
wire change fails there until the file is re-recorded. One session has no
proxy record (`unobserved`); the other's calls went both through the route
and outside it (`mixed`), with one call whose counters are unknown, so its
`outside` tokens are `null`. The macOS decode and Overview tests read it.
