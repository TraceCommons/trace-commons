# Insights subscription limits implementation

## Approved scope

Implement the owner's request for a PR using native Codex and Claude subscription credentials to show current remaining usage and resets on Insights Overview, following CodexBar's acquisition approach. Reuse the direction in spec PR #1608; this implementation narrows it to current usage in Overview. It does not implement history, routing, Private AI account selection, cookie sources, reset redemption, or Claude interactive PTY probing.

User clarified the desired fields: Codex weekly remaining and banked weekly resets; Claude session, weekly and model-specific limits (including Fable when reported). All values come from the provider; missing is unknown, never zero or an inferred full allowance.

## Current source findings

- CodexBar MIT source at 5f93dc3068b3bee3f8753f3772614cc2949c335d; docs/codex.md and docs/claude.md.
- Installed Codex0.164 static protocol schema supports account/rateLimits/read and rateLimitResetCredits.availableCount. Its detail list can be null/capped, so count is authoritative.
- Native Codex app-server reads its selected login, preserving its own authentication/refresh handling. Initialize with our own client identity; account/read refreshToken:false; account/rateLimits/read excludeResetCreditDetails:true and supportsLunaReserve:false. No thread or tool execution.
- Claude uses the selected native .credentials.json, then its exact-profile Keychain service on macOS; fixed api.anthropic.com OAuth usage endpoint and beta header. Current weekly_scoped entries include Fable. Own app user agent, bounded response, redirects refused.
- Claude Keychain reads use existing security-framework in contributor-platform. Background access preflights non-secret item metadata and decrypt ACL trust for the running executable before reading data. Native credential jobs use bounded dedicated threads so an unanswered access dialog cannot hold daemon shutdown. Exact custom-directory hashing requires NFC; use the macOS CoreFoundation normalizer through a narrow safe wrapper if no existing safe helper is available. No new Cargo dependencies.

## Data contract

New read-only IPC subscription_usage takes {refresh?:bool,interactive?:bool}. Returns:

```
{schema_version:"trace_commons.subscription_usage.v1",providers:[
 {provider:"codex"|"claude",enabled:bool,refreshing:bool,logins:[
  {id:string,label:string,default_login:bool,state:"observed"|"unknown",reason:string|null,
   updated_at:ISO8601|null,stale:bool,windows:[
    {lane:"session"|"weekly"|"model",label:string|null,used_permille:integer|null,
     remaining_text:string,resets_at:ISO8601|null}
   ],banked_resets:integer|null}
 ]}
]}
```

Only local login IDs/user-assigned labels cross IPC. Tokens, paths, provider account IDs and raw errors do not. Snapshot percentages are 0..1000; remaining text is formatted in Rust. At/past a reset, cached remaining becomes unknown until measured again. A transient failed refresh may retain marked-stale windows until reset/one-hour maximum; auth loss, identity change or disabling clears the affected data. Missing bank inventory does not discard valid usage windows.

Provider switches subscription_usage_codex_enabled and subscription_usage_claude_enabled default false and are exposed through existing set_settings and config schema. New provider usage card offers per-provider Read usage/Stop reading and Refresh. User-triggered reads may permit a Claude Keychain prompt; background reads never prompt. Default native roots and explicitly saved native subscription profiles stay separate; custom Claude profiles never fall back to the global credential item.

## Ownership and tasks

1. Provider collector: new daemon/subscription_usage/providers.rs and platform external-Claude Keychain helper. Bounded process/network/file I/O, fixed sanitized errors, parsers and deterministic fixtures. Codex official app-server route; Claude OAuth route. Native authentication remains native-owned.
2. Daemon service: new daemon/subscription_usage/mod.rs owns contract/state/cache/generation. Nonblocking IPC schedules work outside locks; supervisor polls enabled providers at15 minutes, manual floor60 seconds, max4 concurrent probes. Enumerate metadata from existing managed service without opening/mutating profiles. Add settings/schema/IPC registration/docs and tests for off/no-reads, disable/in-flight, account separation, stale/reset behavior and bounded cache.
3. macOS UI: typed snapshot/client default compatibility, dedicated injectable usage model and Overview card independent of historical week/store. Integrate new shared analytics copy. Test missing/partial usage, Fable labels, reset count, refresh/coalescing/disable, errors/unknowns and render the card.
4. Integrate: focused Rust/Swift tests, provider fixtures, platform helper tests without real credentials, fresh FFI build, native render, scoped Clippy, formatting and license-boundary. Inspect live usage only through an explicit read-only probe after fixture tests. No real settings toggled during development. Open a PR with validation and unsupported-source limitations.
