# Subscription limits in Insights

Insights → Overview shows saved native subscription profiles and captured quota windows separately from historical trace analytics.

## Claude

Claude usage is on by default for detected native profiles. **Show Claude Code usage** in Settings → Tools turns it off (`subscription_usage_claude_on`). Choose **Refresh** beside the specific login to read its saved Claude Code sign-in and ask Anthropic for usage. The info button beside Claude describes Keychain access. macOS may ask for `Claude Code-credentials`; **Always Allow** grants continuing access to that item, but this app still reads it only on an explicit selected-login Refresh.

The card's caption reads **Last captured account limits**, with **Refresh to see your latest limits.** beneath it. Refresh again after changing Claude accounts. Background work does not query Keychain, so the app does not claim continuous account tracking. Local credential/config file metadata invalidates known changes, and reset times invalidate old allowances. A missing measurement remains unknown.

The reader uses the exact profile's native credential file or Keychain service. `CLAUDE_SECURESTORAGE_CONFIG_DIR` selects the native secure-storage root when present, otherwise `CLAUDE_CONFIG_DIR` applies. Custom profiles never fall back to the global item. Managed launches remove inherited secure-storage overrides to preserve profile isolation. Native credentials remain owned by Claude Code.

Fixed OAuth usage and profile requests supply session, weekly and model-specific limits, including Fable when reported. The app uses stable profile/account/organization evidence. Expired usage tokens lead to **Open Claude Code, then Refresh**; a missing or inference-only token does not imply the user has signed out. Native CLI probing, token renewal and browser-cookie acquisition are outside this reader.

## Codex

In Settings → Tools, **Show Codex usage** controls current native subscription limits. It is on by default since 2026-10-27; a settings file that already saved it off keeps it off. The switch saves a preference only; choose **Refresh** in Insights to acquire a reading.

Refresh follows CodexBar's preferred Codex source. It reads the file-backed sign-in in `$CODEX_HOME/auth.json` (default and saved managed profiles) without writing it, and sends its access token in a bearer header to `GET <chatgpt_base_url>/wham/usage` (`https://chatgpt.com/backend-api/wham/usage` by default; `<chatgpt_base_url>/api/codex/usage` when the configured base has no `/backend-api` path), with `ChatGPT-Account-Id` when the account is known. A missing, unreadable or partially written file, or one due for renewal, is reread twice, 50 ms apart. This path does not need Codex to be closed: nothing is written, so it cannot race Codex renewing its own sign-in.

A sign-in is due for renewal when its access token expires within five minutes, or, without a recognized expiry, when its saved `last_refresh` is older than eight days (file modification time is not used). The app never redeems the refresh token. A sign-in that is due for renewal, that the server rejects with HTTP 401, or that has no refresh token is handed to the native `codex app-server`, which may renew it. Only that path refuses while a Codex process or ambiguous relevant process metadata is detected, with **Codex needs to renew its sign-in. Close Codex, then Refresh.** Network, timeout, other HTTP status and decode failures keep their own labels and do not fall back. API-key sign-ins and files without OAuth tokens report a missing sign-in without launching Codex.

Keyring and automatic stores return `native_storage_unsupported` before credential access, a network request or native launch. Their account changes cannot safely be verified through file metadata alone; check limits in Codex for those stores. `chatgpt_base_url` must be an HTTPS ChatGPT or OpenAI host on both paths.

`rate_limit.primary_window` and `secondary_window` supply the session and weekly windows by their reported duration, and `additional_rate_limits[]` supply named model-specific windows. That response has no banked-reset count, so after a successful usage read the app makes CodexBar's best-effort `GET <base>/wham/rate-limit-reset-credits` with the same token and account, plus `OpenAI-Beta: codex-1` and `originator: Codex Desktop`, and shows its `available_count` as banked resets. It shares the usage request's 15-second budget and waits at most 4 seconds. Any failure leaves banked resets unknown without affecting the usage reading or falling back. Credits are only read, never redeemed or modified. The native `account/rateLimits/read` interface supplies session/weekly allowance and banked resets; modern parameters retry without parameters only for recognized older-schema rejection. Resets are not redeemed.

## Capture and lifecycle

Startup, opening views, status reads and background ticks never acquire credentials or launch provider clients. They only synchronize non-secret file metadata and expire held readings. Requests are scoped and serialized; Keychain approval can wait longer than a network request, with a bounded caller wait. An OS request that outlives its caller retains its concurrency lease, and late secret results are wiped without issuing a provider request after cancellation.

Account/binding removal or changes and unenroll invalidate cached/pending readings. Unenroll preserves the existing retryable persistence-failure behavior and cancels acquisitions; a later explicit Claude Refresh is still available without a Commons enrollment. Transient failures may retain a clearly marked old reading for at most one hour, never beyond a quota reset. Unknown figures are never zero or full allowances. Credentials, raw provider account identifiers and raw errors stay out of IPC, logs, contributed traces and persistent subscription history.

Each provider has its own switch, on by default since 2026-10-27: `subscription_usage_codex_enabled` for Codex and `subscription_usage_claude_on` for Claude. Either reads only on an explicit Refresh. A settings file that already holds `subscription_usage_codex_enabled: false` keeps Codex off; there is no migration. The legacy `subscription_usage_claude_enabled` setting is cleanup-only and does not control Claude. Use `near-ai config schema` and `near-ai config preview` for current configuration metadata, with the CLI bundled with the app/channel being configured.

References: [Codex app-server](https://learn.chatgpt.com/docs/app-server), [CodexBar Codex acquisition](https://github.com/steipete/CodexBar/blob/5f93dc3068b3bee3f8753f3772614cc2949c335d/docs/codex.md), [CodexBar Claude acquisition](https://github.com/steipete/CodexBar/blob/5f93dc3068b3bee3f8753f3772614cc2949c335d/docs/claude.md), [owner decision](../superpowers/specs/2026-10-27-claude-subscription-activation-design.md).
