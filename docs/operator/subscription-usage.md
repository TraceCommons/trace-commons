# Subscription limits in Insights

In Settings → Tools, **Read Codex usage** permits current native subscription limits to be displayed in Insights → Overview. It is off by default. The disclosure beside the switch explains that **Refresh** asks the native Codex app using the saved default and managed subscription sign-ins. Only file-backed Codex sign-ins are supported. Codex may renew its own sign-in. Enabling the switch does not run a probe. Nothing is offered in Insights while it is off.

After enabling, choose **Refresh** in Insights to read current limits. Opening a screen, starting the daemon, and background ticks do not launch Codex or read credentials. Background work only checks local file metadata and expires held values. Reads are refused while Codex is running; close Codex before choosing Refresh. Process inspection is conservative and may also withhold a probe when it cannot establish that it is safe to start one.

Codex supplies session/weekly allowance and banked-reset inventory through its native `account/rateLimits/read` interface. Modern parameters are retried without parameters only when an older native schema rejects their shape. The app never redeems resets. Window durations are carried as optional minutes; absent durations and percentages remain unknown. Default and explicitly saved native subscription profiles stay separate. API-key accounts are not subscription allowances.

Keyring-only and automatic credential stores are refused with `native_storage_unsupported`, before reading credentials or starting Codex. Their account changes cannot be verified from local file metadata. This revision does not query Keychain metadata in the background or prompt for Keychain access. Use Codex itself to check limits for these stores.

**Claude usage is unavailable in this revision.** Its switch is hidden and enabling it through configuration is refused. The direct Anthropic OAuth collector and its PR-added native credential/ACL helpers have been removed pending the acquisition review. No subscription-usage action reads Claude files, queries its Keychain item, or sends its token. Claude PTY acquisition and browser-cookie sources are outside this implementation.

A percentage is remaining allowance. Reset times use the local time zone. A missing measurement is unknown, not zero or full. A transient failed refresh without identity evidence can retain a marked-stale reading for at most one hour and never past a window reset. Confirmed authentication loss, changed native file metadata, removed profiles, disabled reading, and unenrollment invalidate held readings. A successful native renewal can publish a new reading only when its final identity-checked file revision still matches the current login; other file changes invalidate old values rather than transferring them to another account.

Current readings remain separate from the historical week picker and trace analytics. They stay in daemon memory; credentials, provider account IDs, and raw errors never cross IPC or enter contributed traces. Unenroll turns both legacy provider switches off in memory and on disk and invalidates held readings. If persisted revocation fails, local enrollment cleanup still completes and the call returns a retryable failure; unreadable settings are never overwritten.

The supported configuration switch is:

```json
{"subscription_usage_codex_enabled": true}
```

Discover and preview with `near-ai config schema` and `near-ai config preview` before applying. Setting this flag grants permission but does not request a read. The legacy `subscription_usage_claude_enabled` field remains readable for compatibility and its schema accepts only `false` for cleanup. Use the CLI bundled with the app/channel being configured; see [agent configuration](../agent-configuration.md).

Implementation reference: [Codex app-server](https://learn.chatgpt.com/docs/app-server). No subscription history, routing, reset redemption, browser cookies, or Claude credential acquisition is implemented here.
