# Subscription limits in Insights

Insights → Overview shows **Subscription limits** separately from the selected analytics week. Each reading belongs to a native Codex or Claude login; allowances from different accounts are never added together.

Choose **Read usage** for the provider you want to inspect. The preference is off by default for each provider. It uses the login already owned by the native coding tool. **Stop reading** disables that provider and clears its held readings.

- **Codex:** session/weekly remaining allowance and the provider's banked-reset count. Native `codex app-server` supplies account/rateLimits/read, with its own authentication handling. The reset count is authoritative even when reset details are absent or capped. The app does not redeem a reset.
- **Claude:** session, weekly, and model-specific weekly limits such as Fable, when the provider reports them. The app reads the selected native credential file or its correctly scoped macOS Keychain item, then asks Anthropic's OAuth usage endpoint. It does not rotate or persist Claude tokens.

Default native profiles and explicitly saved native subscription profiles are separate rows. Managed profiles never borrow credentials from the default profile. API-key accounts are not subscription allowances.

A percentage is **remaining** allowance. Reset times use your local time zone. A missing measurement is unknown, not zero or a full allowance. A failed refresh may leave a reading marked stale while its window remains valid. A reset, sign-out, account change or disabled provider makes the corresponding old value unknown rather than transferring it to another account.

Codex profiles configured for Keychain or automatic credential storage require an explicit Refresh; background probes use file-backed native credentials to avoid an unexpected authentication dialog. A Claude secure-storage override selects its credential file and does not fall back to another profile’s Keychain item.

Refresh requests are coalesced with a one-minute minimum interval. Enabled providers are checked periodically; the screen can request a refresh. Background Claude Keychain reads inspect item metadata and its access rules before requesting a secret. If the running app is not already trusted, Insights asks you to Refresh to permit access. An explicit user action can permit the normal macOS Keychain prompt for the Claude credential item. A pending native access request is bounded and does not prevent daemon shutdown. If access is denied or credentials are expired, use the provider's native sign-in flow and refresh again.

Usage is read locally and sent only to the subscription's provider for its usage request. The app stores current snapshots in memory; it does not add subscription credentials or account usage to contributed traces or an Insights history database.

The provider switches are also available through the supported configuration API:

```json
{
  "subscription_usage_codex_enabled": true,
  "subscription_usage_claude_enabled": true
}
```

Discover and preview settings with `near-ai config schema` and `near-ai config preview` before applying them. Use the CLI bundled with the app/channel being configured; see [agent configuration](../agent-configuration.md).

Implementation references: [Codex app-server](https://learn.chatgpt.com/docs/app-server), [CodexBar Codex acquisition](https://github.com/steipete/CodexBar/blob/5f93dc3068b3bee3f8753f3772614cc2949c335d/docs/codex.md), and [CodexBar Claude acquisition](https://github.com/steipete/CodexBar/blob/5f93dc3068b3bee3f8753f3772614cc2949c335d/docs/claude.md). Browser-cookie acquisition and Claude PTY scraping are outside this implementation.
