# Subscription limits in Insights

Insights → Overview shows saved native subscription profiles and captured quota windows separately from historical trace analytics.

## Claude

Claude usage is available by default for detected native profiles; no opt-in switch is required. Choose **Refresh** beside the specific login to read its saved Claude Code sign-in and ask Anthropic for usage. The explanation beside the action describes Keychain access. macOS may ask for `Claude Code-credentials`; **Always Allow** grants continuing access to that item, but this app still reads it only on an explicit selected-login Refresh.

Claude measurements are labeled **Captured on Refresh**. Refresh again after changing Claude accounts. Background work does not query Keychain, so the app does not claim continuous account tracking. Local credential/config file metadata invalidates known changes, and reset times invalidate old allowances. A missing measurement remains unknown.

The reader uses the exact profile's native credential file or Keychain service. `CLAUDE_SECURESTORAGE_CONFIG_DIR` selects the native secure-storage root when present, otherwise `CLAUDE_CONFIG_DIR` applies. Custom profiles never fall back to the global item. Managed launches remove inherited secure-storage overrides to preserve profile isolation. Native credentials remain owned by Claude Code.

Fixed OAuth usage and profile requests supply session, weekly and model-specific limits, including Fable when reported. The app uses stable profile/account/organization evidence. Expired usage tokens lead to **Open Claude Code, then Refresh**; a missing or inference-only token does not imply the user has signed out. Native CLI probing, token renewal and browser-cookie acquisition are outside this reader.

## Codex

In Settings → Tools, **Read Codex usage** permits current native subscription limits. It is off by default. Enabling saves permission only; choose **Refresh** in Insights to acquire a reading. The app reads file-backed default/saved managed profiles through the native Codex app, which may renew its own sign-in.

Detected Codex processes or ambiguous relevant process metadata refuse a probe. Close Codex before Refresh. Keyring and automatic stores return `native_storage_unsupported` before credential access or native launch. Their account changes cannot safely be verified through file metadata alone; check limits in Codex for those stores.

The native `account/rateLimits/read` interface supplies session/weekly allowance and banked resets. Modern parameters retry without parameters only for recognized older-schema rejection. Resets are not redeemed.

## Capture and lifecycle

Startup, opening views, status reads and background ticks never acquire credentials or launch provider clients. They only synchronize non-secret file metadata and expire held readings. Requests are scoped and serialized; Keychain approval can wait longer than a network request, with a bounded caller wait. An OS request that outlives its caller retains its concurrency lease, and late secret results are wiped without issuing a provider request after cancellation.

Account/binding removal or changes and unenroll invalidate cached/pending readings. Unenroll preserves the existing retryable persistence-failure behavior and cancels acquisitions; a later explicit Claude Refresh is still available without a Commons enrollment. Transient failures may retain a clearly marked old reading for at most one hour, never beyond a quota reset. Unknown figures are never zero or full allowances. Credentials, raw provider account identifiers and raw errors stay out of IPC, logs, contributed traces and persistent subscription history.

The Codex permission can be configured with `subscription_usage_codex_enabled`. The legacy `subscription_usage_claude_enabled` setting is cleanup-only and no longer controls Claude availability. Use `near-ai config schema` and `near-ai config preview` for current configuration metadata, with the CLI bundled with the app/channel being configured.

References: [Codex app-server](https://learn.chatgpt.com/docs/app-server), [CodexBar Claude acquisition](https://github.com/steipete/CodexBar/blob/5f93dc3068b3bee3f8753f3772614cc2949c335d/docs/claude.md), [owner decision](../superpowers/specs/2026-10-27-claude-subscription-activation-design.md).
