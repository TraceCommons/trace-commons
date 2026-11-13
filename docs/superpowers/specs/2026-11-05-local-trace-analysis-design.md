# Local trace analysis

Approved in conversation on 2026-11-05. When the daemon confirms local trace analysis is off, the existing Insights sample banner says **Local trace analysis is off**, **Enable it to see Insights from traces on this Mac.**, with **Enable local trace analysis**. Keep the sample-data label visible. The button saves only the counting setting, confirms the echo, then refreshes Insights. Keep it disabled during a write; show existing request-failure copy on refusal. Do not offer enabling for unknown/unavailable reads or when already enabled.

## Approaches

Use the existing confirmed AppModel setting writer rather than introduce another IPC path. A Settings link was considered but adds navigation instead of performing the requested action. Copy belongs in the shared Rust table. Scope is the native macOS Insights sample banner; no changes to collection/sharing permissions or defaults.
