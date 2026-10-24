# Subscription usage discovery -- design

Date: 2026-10-24. Owner: Zaki. Status: draft, nothing built. No
implementation plan yet; one follows once the owner settles section 11.
Every `file:line` below is against `origin/main` at `bb4c90841` unless it
names another tree. IronWire references are against the pinned revision
`a2ae5d97aaee005c30dd51b7a873e6eba06940af`
(`crates/trace-commons-contributor/Cargo.toml:781`).

This design lets the contributor app learn, on the Mac and only when the
contributor turns it on, what their Codex (ChatGPT) and Claude subscriptions
report about plan and usage, and show it in Insights and as read-only rows
in Private AI. It learns it the way CodexBar does (owner decision U1). It
does not route anything through a subscription.

## The owner's request

Verbatim, 2026-10-24:

> "can the app automatically learn about your claude and codex
> subscriptions like codexbar does and then use this in insights and as
> options in private ai"

and, after the answer summarised under "Direction accepted" below:

> "write a spec for this"

A later instruction the same day: "open a PR and spec kristi about the
subscription ussage" -- the PR asks Kristi (`poldsam`) to review this
design.

## Owner decisions

- **(U1) Do what CodexBar does.** On PR #1608, 2026-10-24: "I think we
  should do whatever codexbar dpes". So the source order, endpoints,
  fields and opt-in extras below follow CodexBar's documented behaviour
  (`docs/codex.md` and `docs/claude.md` in `steipete/CodexBar`, recorded
  under "Verified"). This settles what the first draft asked as Q1 (Codex
  source), Q2 (Claude usage endpoint), Q3 (Claude CLI fallback) and Q7
  (browser cookies). Where this design still differs from CodexBar, it is
  listed in "Where this differs from CodexBar" and each difference is one
  the owner set in the same instruction (never refresh or write a token,
  read at use and hold in memory only) or a default he may override in
  section 11.

## Direction accepted

The owner accepted this direction on 2026-10-24, before U1. U1 replaces
its source choices; the rest stands.

- **Display only, now.** Learn plan, usage windows with their resets and
  credits. Show them in Insights and as read-only rows in Private AI, with
  the subscription's usage filled in where IronWire today shows `unknown`.
- **Opt in per provider.** Off until the contributor turns on Codex usage
  or Claude usage in Settings, separately.
- **On this Mac only**, like the rest of Insights. Nothing is uploaded.
- **Routing Private AI calls through a subscription is a separate, later
  decision.** This design leaves room for it (section 10) and does not make
  it.

## Verified on 2026-10-24

### Our code

- **The proxy's status object already lists both subscriptions.** IronWire
  serves `GET /_ironwire/status` with one `BackendView` per registered
  backend, including `kind`, `authenticated`, `consented`, `headroom`,
  `health`, `models` and `connect_command`
  (IronWire `crates/ironwire_proxy/src/control.rs:1292-1340`). On a Mac
  with Claude Code and Codex signed in, the backends `claude-sub` and
  `codex-sub` appear with `authenticated: true`, `consented: false` and
  `headroom: {"state":"unknown"}`.
- **Our daemon reads only two facts from it.** `StatusView` and
  `BackendView` deserialize `balance.spend_today_usd` and, per backend,
  `id` and `authenticated` -- nothing else
  (`crates/trace-commons-contributor/src/routing/ironwire.rs:201-224`).
  `nearai_authenticated` (`:254`) and `spend_micros` (`:236`) are the only
  consumers; `refresh_spend` (`:629`) reads the status once per refresh
  tick and writes `None` on any failure, never zero. `headroom` is not
  read at all. The test fixture at `:866` spells it `{"kind":"unknown"}`,
  which is not upstream's shape (upstream tags with `state`, below); it is
  harmless only because nothing reads the field.
- **The embedded proxy already reads both providers' sign-ins.** When the
  daemon holds a NEAR AI key, `embed_options` selects
  `CredentialFiles::Discover`
  (`crates/trace-commons-contributor/src/daemon/private_inference.rs:453-463`;
  the reasoning is the doc comment at `:405-452`). IronWire then reads the
  Claude Code login from the macOS Keychain item `Claude Code-credentials`
  by running `/usr/bin/security find-generic-password -s <service> -w`,
  falling back to `~/.claude/.credentials.json`
  (IronWire `crates/ironwire_creds/src/claude.rs:192-257`, `:431-435`), and
  the Codex login from `~/.codex/auth.json` or `$CODEX_HOME/auth.json`
  (IronWire `crates/ironwire_creds/src/codex.rs:151-200`). That is how
  `authenticated: true` is known today. This exposure predates this design;
  4.3 and Q10 keep a security item open on it.
- **Neither tool's token is refreshed by IronWire.** Codex: "IronWire does
  not refresh it ... `~/.codex/auth.json` is Codex's file, not ours"
  (IronWire `crates/ironwire_creds/src/codex.rs:108-120`). This design keeps
  the same rule for both providers.
- **Multiple logins per product exist.** Since IronWire #64 a home can
  declare more than one Claude Code or Codex login, each with its own
  backend id and `config_dir` (`docs/multi-subscription-routing.md`).
  This design keys everything by IronWire backend id, so a second login is
  a second row.
- **The IPC conventions this design copies** are those of `insights_glance`
  (`docs/contributor-daemon-ipc-v1_1.md:4409` onward: `enabled: false`,
  `readable: false` never drawn as zero, `updated_at`, `stale`), the
  counter store's clearing rules (`:4527`, `unenroll` clears the store and a
  running pass writes nothing afterwards) and the Mac-scoped settings that
  `unenroll` keeps (`:5036`).
- **Copy is shared.** Private AI's sentences come from
  `PrivateInferenceCopy`, composed in the Rust contributor crate
  (`macos/Sources/TraceCommonsApp/Views/PrivateInferenceView.swift:14-15`).
  New strings in this design go in the same Rust tables
  (`docs/ui-copy-guide.md` rule 8).
- **Crates already in the contributor crate** that this design can use:
  `tokio` with `process`, `reqwest` (`Cargo.toml:798`, `:809`),
  `base64` 0.22 (`:79`), `sha2` (`:803`), `rusqlite` 0.37 bundled (`:85`).
  `security-framework` 3.7.0 is a direct dependency of
  `trace-commons-contributor-platform` (`Cargo.toml:50`). `aes` 0.8.4,
  `cbc` 0.2.1, `hmac` 0.12.1 and `sha1` 0.10.6 are in `Cargo.lock`
  transitively, not as direct dependencies.

### IronWire at `a2ae5d97`: headroom is observed, not supplied

- `Headroom` is `Observed { used_pct, resets_at, observed_at }`,
  `Exhausted { until }`, `CapReached { .. }` or `Unknown`, serialized with
  `#[serde(tag = "state", rename_all = "snake_case")]`
  (`crates/ironwire_core/src/quota.rs:12-47`). The module's rule: "either
  the provider told us, or we say `unknown`" (`:3-7`).
- It is filled only from response headers on traffic the proxy forwards:
  `anthropic-ratelimit-unified-5h-utilization`, `-7d-utilization`, their
  `-reset`s and the representative claim
  (`crates/ironwire_upstream/src/observe.rs:157-245`, consumed at
  `crates/ironwire_upstream/src/anthropic.rs:527`). With `consented: false`
  no request is forwarded, so `headroom` stays `unknown`.
- `QuotaSnapshot` has `primary` and `secondary`, but the status object
  serializes only `primary` (`control.rs:1319`).
- Observations survive a restart for at most 15 minutes
  (`crates/ironwire_core/src/quota_store.rs:42`).
- `EmbedOptions` (`crates/ironwire_proxy/src/embed.rs:235`) has no way for a
  host to supply a quota observation, and the control router
  (`control.rs:573-595`) has no route that accepts one. **There is no
  headroom provider hook.** So this design does not write into IronWire's
  `headroom`; it keeps a parallel daemon surface (section 3) and names an
  upstream hook as a later option (section 10).

### Codex app-server protocol

From `openai/codex` at `970111f6cb62cc02fe36a82ea7d71b37a2889c97`, the
generated schema in `codex-rs/app-server-protocol/schema/typescript/`:

- `account/rateLimits/read` (`src/protocol/common.rs:1360`, no params) ->
  `GetAccountRateLimitsResponse`: `ordinaryUsageAllowed: boolean | null`,
  `rateLimits: RateLimitSnapshot`, `rateLimitsByLimitId`,
  `rateLimitResetCredits`, `accountId: string | null`,
  `rateLimitUpsell`.
- `RateLimitSnapshot`: `limitId`, `limitName`, `primary` and `secondary`
  (`RateLimitWindow | null`), `credits` (`CreditsSnapshot | null`),
  `spendControlReached`, `planType`, `rateLimitReachedType`, and others.
- `RateLimitWindow`: `usedPercent: number`, `windowDurationMins:
  number | null`, `resetsAt: number | null`.
- `CreditsSnapshot`: `hasCredits: boolean`, `unlimited: boolean`,
  `balance: string | null`.
- `account/read` (`common.rs:1500`) takes `GetAccountParams { refreshToken?:
  boolean }` -- "When `true`, requests a proactive token refresh before
  returning" -- and returns `{ account: Account | null, requiresOpenaiAuth:
  boolean }`, where `Account` is `{type:"chatgpt", email, planType}`,
  `{type:"apiKey"}`, or others.
- `PlanType` is a closed list ending in `"unknown"` (`PlanType.ts`).
- `account/rateLimits/updated` is a sparse notification to merge into the
  last read (`AccountRateLimitsUpdatedNotification.ts`).
- `InitializeParams { clientInfo: { name, title, version }, capabilities }`.
- Neither account method is marked `#[experimental]` in `common.rs`.

### CodexBar

`steipete/CodexBar` `docs/codex.md` and `docs/claude.md`, fetched
2026-10-24. Neither page carries a date or commit; the plan pins a commit
of the repository when it is written. U1 makes this the behaviour to
mirror.

**Codex** (`docs/codex.md`):

- Auto order: "PAT, when available", then the OAuth API using `auth.json`,
  then CLI RPC through `codex app-server` ("A managed workspace disables
  this fallback"). "Authentication failures can allow fallback; network,
  server, and decode failures keep their original error." The web
  dashboard "is not an Auto fallback."
- OAuth API: credentials `~/.codex/auth.json`, or `$CODEX_HOME/auth.json`;
  `GET https://chatgpt.com/backend-api/wham/usage` with `Authorization:
  Bearer <token>`; a `chatgpt_base_url` in `$CODEX_HOME/config.toml`
  (default `~/.codex/config.toml`) overrides the base, `#` lines ignored;
  `GET https://chatgpt.com/backend-api/wham/rate-limit-reset-credits` read
  once per refresh, best effort, never redeemed. `rate_limit.primary_window`
  / `secondary_window` map to the session and weekly lanes;
  `additional_rate_limits[]` to model-specific limits. Identity: "`~/.codex/auth.json`
  (JWT claims: email + plan)". A missing, unreadable or partly written file
  is retried twice, 50 ms apart.
- Refresh: CodexBar "never writes refreshed tokens into `auth.json`"; for
  stale native credentials it delegates recovery to the Codex CLI, and if
  the CLI is unavailable "the OAuth error is surfaced instead of mutating
  the shared file". Freshness: a recognised expiry needs renewal within
  five minutes; otherwise `last_refresh` against an eight-day threshold;
  file modification time is not used. "Stale external OAuth credentials
  fail closed."
- CLI RPC: `codex -s read-only -a never app-server`, JSON-RPC over stdio,
  `initialize`, `account/read`, `account/rateLimits/read`; primary and
  secondary windows with resets, credits (`balance`, `hasCredits`,
  `unlimited`), email and plan. A fresh rate-limit plan overrides the
  account plan; email always comes from the account response.
  Initialization has a longer budget; on timeout close stdin, SIGTERM,
  then SIGKILL. If macOS blocks or quarantines `codex`, background launches
  pause for 30 minutes and a manual refresh retries at once.
- A bare `codex` TUI "can start interactive auth and open browser tabs";
  the PTY `/status` path is a manual diagnostic only.
- OpenAI web dashboard, "optional, off by default", toggle "OpenAI web
  extras": `https://chatgpt.com/codex/cloud/settings/analytics#usage` in an
  off-screen `WKWebView`, cookies imported from Safari
  (`~/Library/Cookies/Cookies.binarycookies`), then Chrome/Chromium
  profiles, then Firefox (`cookies.sqlite`), domains `chatgpt.com` and
  `openai.com`; the signed-in email must match the Codex email or the web
  path is rejected; it "fills credits only when OAuth/CLI do not provide
  them"; cookie cache in Keychain service `com.steipete.codexbar.cache`,
  account `cookie.codex`.

**Claude** (`docs/claude.md`):

- App Auto order: OAuth API, then CLI PTY, then Web API.
- OAuth credentials, in order: an explicit OAuth environment override, the
  CodexBar OAuth cache, `~/.claude/.credentials.json`, then the Keychain
  item `Claude Code-credentials` "used only as a bootstrap/repair
  fallback".
- `GET https://api.anthropic.com/api/oauth/usage?cedar_ember=1`, retried
  once without the query on HTTP 400 or 403;
  `GET https://api.anthropic.com/api/oauth/profile`; headers
  `Authorization: Bearer <access_token>` and `anthropic-beta:
  oauth-2025-04-20`. Fields: `five_hour` (session), `seven_day` (weekly,
  and the primary fallback when `five_hour` is missing), `seven_day_sonnet`,
  `seven_day_opus` and `limits[].weekly_scoped` (model-specific weekly),
  `seven_day_routines` / `seven_day_cowork`, `extra_usage` (monthly extra
  spend and limit). Plan: `subscriptionType`, else `rate_limit_tier`
  ("Max 5x", "Max 20x" for the two Max tiers). The token needs
  `user:profile`. A free account with null `five_hour` and `seven_day` is
  "limits unavailable".
- CodexBar refreshes Claude tokens itself, and can refresh through the
  `claude` CLI.
- Keychain prompt policy: "Never prompt", "Only on user action (default)",
  "Always allow prompts"; "Ordinary OAuth fetches stay noninteractive under
  every setting"; the freshness check "reads only Keychain attributes with
  UI disabled ... It never requests the credential payload."
- CLI PTY: `claude` in a 160x200 PTY with `--allowed-tools ""`,
  `--strict-mcp-config` and `--settings` setting `remoteControlAtStartup`
  false and `disableAllHooks` true; sends `/usage`, waits, presses Escape,
  optionally `/status`; parses "Current session" and "Current week" and the
  `Account:` / `Org:` lines; its own working directory; exits after each
  probe; "a non-PTY `/usage` fallback" is mentioned but not described.
- Web API: cookie `sessionKey` on `claude.ai`; Safari, then
  Chrome/Chromium (`~/Library/Application Support/Google/Chrome/*/Cookies`,
  decryption "needs the browser's Safe Storage Keychain item"), then
  Firefox (`~/Library/Application Support/Firefox/Profiles/*/cookies.sqlite`);
  "Use Claude desktop app cookies", macOS, "off by default"; endpoints
  `GET https://claude.ai/api/organizations`,
  `.../organizations/{orgId}/usage?cedar_ember=1`,
  `.../overage_spend_limit`, `.../prepaid/credits`,
  `https://claude.ai/api/account`, `.../subscription_details`; cookie cache
  in Keychain `com.steipete.codexbar.cache`, account `cookie.claude`.
- "If every live Auto source fails, the last captured percentages from
  history remain visible, marked as stale."

### Anthropic's terms text

From <https://code.claude.com/docs/en/legal-and-compliance>, section
"Authentication and credential use", fetched 2026-10-24. Quoted in the
Terms note (section 6.5), not interpreted.

## Where this differs from CodexBar

Each is set by the owner's own instruction with U1, or is a default the
owner may override in section 11.

| CodexBar | This design | Why |
| --- | --- | --- |
| Refreshes Claude tokens itself | Never refreshes either token; an expired Claude token goes to the CLI fallback, which refreshes its own | Owner: never refresh or write the token |
| Keeps an OAuth cache and a Keychain cookie cache | No cache of any token or cookie; read at use, held in memory for the request | Owner: read at use, in memory only |
| Codex PAT source first | Omitted; CodexBar's page does not say what the PAT is | Q13 |
| Claude Web API is the third Auto fallback | Web extras are a separate opt-in phase, off by default, for both providers (2.5) | Reads browser cookie stores; owner allowed a separate phase |
| `chatgpt_base_url` honoured as written | Honoured only for `https` on a ChatGPT or OpenAI host | The bearer goes wherever the base points (6.2); Q14 |
| `claude-cli/<version>` user agent on the reset-inventory request | Reset inventory not read in v1; our own user agent everywhere | We do not present as another program |
| Last percentages shown "marked as stale" | The same, until the window's own reset time passes, then "Usage unknown" | A reading past its reset says nothing (IronWire's `is_pressured` rule, `quota.rs:71-93`) |

## 1. Goals and non-goals

### Goals

1. When the contributor turns it on, show for each Codex login and each
   Claude Code login on this Mac: the plan, each usage window the provider
   reports (percent used and when it resets), model-specific windows,
   credits and extra usage where the provider reports them.
2. Show it in Insights, beside the share of work each tool did, using data
   Insights already has.
3. Show each subscription as a read-only row on the Private AI screen,
   with its usage in place of today's nothing.
4. Never show a number the provider did not report. Every failure is a
   named state in words, never zero.
5. Keep everything on this Mac. No usage figure, plan, account identifier,
   token or cookie is uploaded, logged in clear or written by us to disk,
   except the bounded on-device history in 3.5.

### Non-goals

- **Routing.** No Private AI call goes to a subscription because of this
  design. `consented` stays whatever IronWire says, and nothing here
  grants consent (section 10).
- **Token refresh or writes.** We never refresh either provider's token
  and never write to either provider's credential store or config.
- **Redeeming anything.** Reset credits and prepaid credits are read, never
  spent or changed.
- **Cost estimates from local transcripts.** Insights already counts
  tokens (`insights_week`); this design adds no price table.
- **Notifications.** No limit-reached or reset notification. A notice
  needs the owner's explicit approval (`docs/ui-copy-guide.md` rule 9);
  Q9 raises it as a question.
- **Shells other than native macOS**, for now (section 4.6).

## 2. Data sources

Two phases. **Phase 1** is each provider's OAuth source with its CLI
fallback (2.1-2.3). **Phase 2** is the browser-cookie web extras (2.5), a
separate opt-in that ships after Phase 1.

Fallback rule for both providers, CodexBar's: move to the next source only
on an authentication failure (missing, expired or refused credential;
HTTP 401 or 403 after the documented retry). A network, server or decode
failure ends the probe with that failure, so a provider outage does not
fan out into a CLI launch every 15 minutes.

### 2.1 Codex, first: `auth.json` and `wham/usage`

**Credential.** For the default login, `~/.codex/auth.json`; for a
declared extra login, `<config_dir>/auth.json` (the IronWire backend's
`config_dir`, as `$CODEX_HOME`). Read at use, as CodexBar does: a missing,
unreadable or partly written file is retried twice, 50 ms apart. Parsed
into a struct that keeps only the access token (as a
`secrecy::SecretString`), the ID token's payload claims we name below,
`last_refresh` and the expiry. The token is never refreshed and the file is
never written.

**Freshness, before any request.** As CodexBar: a recognised expiry within
five minutes, or with no expiry a `last_refresh` older than eight days, is
stale. A stale or API-key-mode `auth.json` skips this source and goes to
the CLI (2.2), which renews its own sign-in.

**Base URL.** Read `chatgpt_base_url` from `config.toml` beside
`auth.json`, ignoring `#` lines. Default `https://chatgpt.com/backend-api`.
Accepted only when it is `https` and its host is `chatgpt.com`,
`openai.com` or a subdomain of either; anything else is
`base_url_refused` and no request is made (Q14).

**Requests.**

```text
GET {base}/wham/usage
Authorization: Bearer <access token>

GET {base}/wham/rate-limit-reset-credits     (best effort, one per refresh)
Authorization: Bearer <access token>
```

`reqwest`, 10 s timeout, no redirects followed, response body capped at
1 MiB, our own user agent.

**Kept.**

| Source field | Kept as | Rule |
| --- | --- | --- |
| `rate_limit.primary_window` | session window | CodexBar's mapping. The window's used percent and reset, read with the same range checks as 2.2. The exact sub-field names are pinned from a recorded fixture in the plan; CodexBar's page names only the windows. |
| `rate_limit.secondary_window` | weekly window | As above. |
| `additional_rate_limits[]` | model-specific windows | Each with its limit name as a label only if it matches `[A-Za-z0-9 ._-]{1,40}`, else "Model limit". |
| reset-credits response | `reset_credits` | A count, if it parses; never redeemed. |
| ID-token claims: plan | `plan` | CodexBar's identity source ("JWT claims: email + plan"). The claim is read from the payload without signature verification, for display only; it gates nothing. Mapped onto the pinned `PlanType` list, else `unknown`. |
| ID-token claims: email | nothing | Not deserialized. CodexBar uses it to match the web path's account; this design matches by account tag instead (2.5). |
| account id claim | `account_tag` | Never kept. Only `SHA-256(install_salt ‖ account id)` truncated to 16 bytes, to notice another account (3.5). |

An HTTP 401 or 403 falls back to the CLI. Credits are not in CodexBar's
`wham/usage` mapping, so with this source `credits` is filled only if the
CLI fallback runs or the web extras (2.5) supply them.

### 2.2 Codex, fallback: the Codex app-server

The daemon launches the contributor's own Codex CLI and asks it. Codex
reads and, in its own managed mode, may refresh its own sign-in; we never
see the token on this path.

**Launch.** Exactly:

```text
<absolute path to codex> -s read-only -a never app-server
```

- Never a bare `codex`.
- `<absolute path>` comes from the daemon's existing harness detection
  (the harness list, `docs/contributor-daemon-ipc-v1_1.md:3865`), never from
  `PATH` lookup inside the daemon's launchd environment. Not found is
  `codex_not_found`.
- `CODEX_HOME` is set to the login's `config_dir` for a declared extra
  login, and unset for the default login. The child's environment is
  otherwise cleared down to `HOME`, a fixed `PATH`, `LANG` and `TMPDIR`: no
  NEAR AI key, no IronWire control token, no proxy URL is inherited.
- Working directory: a daemon-owned empty directory, not a project.
- stdin and stdout piped; stderr discarded unread.

**Exchange.** One JSON object per line over stdio. In order:

1. `initialize` with `clientInfo: { name: "trace_commons", title: null,
   version: <app version> }`, `capabilities: null`. The plan verifies
   whether the pinned schema also needs an `initialized` notification and
   whether a `jsonrpc` field is required; the app-server README's examples
   omit it.
2. `account/read` with `params: {}`. **`refreshToken` is never sent.**
3. `account/rateLimits/read` with no params.

Then close stdin, wait up to 2 s, SIGTERM, wait 2 s, SIGKILL, reap. Budgets:
10 s for `initialize` (CodexBar's "longer budget"), 5 s per request, 20 s
for the whole probe. Each stdout line is capped at 1 MiB; a longer line
ends the probe as `unreadable`. Notifications, including
`account/rateLimits/updated`, are read and discarded: the probe is a
one-shot read. If macOS blocks or quarantines `codex`, this login's CLI
launches pause for 30 minutes and **Refresh** retries at once (CodexBar).

**Kept.** Parsed into narrow structs that have no field for anything not
listed here:

| Source field | Kept as | Rule |
| --- | --- | --- |
| `account.type` | `signed_in` | `chatgpt` is a subscription sign-in. `apiKey` or any other type is `not_subscription`: "Not a ChatGPT sign-in", no usage. `account: null` with `requiresOpenaiAuth: true` is `signed_out`. |
| `rateLimits.planType`, else `account.planType` | `plan` | CodexBar's precedence. A value outside the pinned `PlanType` list, or `"unknown"`, is `unknown`. |
| `rateLimits.primary`, `.secondary` | session and weekly windows | `usedPercent` finite and within 0..=100, else that window is `unknown`. Kept as permille. `windowDurationMins` kept; the label comes from the duration (300 -> "5-hour", 10080 -> "Weekly", otherwise "{n}-hour" or "{n}-day", null -> "Usage window"), never assumed. |
| `rateLimitsByLimitId` | model-specific windows | As `additional_rate_limits[]` in 2.1, keyed by `limitName` under the same label rule. |
| `resetsAt` | `resets_at` | Unix seconds (unit confirmed against a recorded fixture in the plan). In the past or more than 31 days ahead is dropped, not clamped. |
| `credits` | `credits` | `hasCredits` and `unlimited` as given; `balance` only when it parses as a non-negative decimal. |
| `ordinaryUsageAllowed` | `limit_reached` | `false` -> `true`; `null` -> unknown, and per the schema's own comment never inferred from percentages. |
| `accountId` | `account_tag` | As 2.1. |
| `account.email` | nothing | Never deserialized. |
| `rateLimitResetCredits`, `rateLimitUpsell`, `spendControlReached`, `individualLimit` | nothing in v1 | The upsell is a provider banner; the others have no display yet. |

### 2.3 Claude, first: the OAuth usage endpoint

**Credential.** CodexBar's order, without its own cache and without an
environment override (the daemon runs under launchd with none):

1. The file `~/.claude/.credentials.json`, or
   `$CLAUDE_CONFIG_DIR/.credentials.json` for a declared extra login.
2. The Keychain item `Claude Code-credentials` (or the per-directory name
   IronWire derives for an extra login, `claude.rs:142-158`), as a
   fallback only, read through the Security framework with the rules in
   4.3. No interactive read happens in the background.

Parsed into a struct that keeps only the access token (`SecretString`)
and `expiresAt`. A token without the `sk-ant-oat` prefix is ignored, as
IronWire does (`claude.rs:38-40`). An item holding only `mcpOAuth` state is
`unsupported_login` (CodexBar treats it as a configuration error). The
token is held for the requests below and dropped; never refreshed, never
written.

**Freshness.** `expiresAt` in the past, or within 60 s, skips this source
and goes to the CLI fallback (2.4), where Claude Code renews its own.

**Requests.**

```text
GET https://api.anthropic.com/api/oauth/usage?cedar_ember=1
Authorization: Bearer <access token>
anthropic-beta: oauth-2025-04-20

GET https://api.anthropic.com/api/oauth/profile
Authorization: Bearer <access token>
anthropic-beta: oauth-2025-04-20
```

On HTTP 400 or 403 from the usage request, retried once without
`?cedar_ember=1`, as CodexBar does. Host fixed to `api.anthropic.com`
(the host IronWire binds the token to, `claude.rs:26-27`). 10 s timeout, no
redirects, body capped at 1 MiB.

**Kept.**

| Source field | Kept as | Rule |
| --- | --- | --- |
| `five_hour` | session window | Its utilization and `resets_at`. Booleans and strings in a percentage field are ignored (CodexBar). |
| `seven_day` | weekly window | Also the session fallback when `five_hour` is missing or has no utilization (CodexBar). |
| `seven_day_sonnet`, `seven_day_opus`, `limits[].weekly_scoped` | model-specific weekly windows | Labelled "Sonnet", "Opus", or the scoped limit's name under the 2.1 label rule. |
| `seven_day_routines`, `seven_day_cowork` | "Daily Routines" window | As CodexBar maps them. |
| `extra_usage` | `extra_usage` | Monthly spend and limit, as numbers with their currency if stated; else absent. |
| profile `subscriptionType`, else `rate_limit_tier` | `plan` | Fixed labels: Pro, Max 5x, Max 20x, Team, Enterprise, Free; anything else `unknown`. |
| profile account and org ids | `account_tag` | As 2.1. Email and org name not deserialized. |
| null `five_hour` and `seven_day` | state `limits_unavailable` | "Limits not available for this plan". |

HTTP 401, or 403 after the retry, is an authentication failure and falls
back to the CLI. A token without `user:profile` scope lands there too; if
the CLI then also cannot answer, the row says
`unsupported_login`.

### 2.4 Claude, fallback: the `claude` CLI and `/usage`

The daemon starts the contributor's own Claude Code in a pseudo-terminal,
sends `/usage`, parses the panel, and exits. Claude Code reads and renews
its own sign-in.

- Launch, with CodexBar's safety flags: `<absolute path to claude>
  --allowed-tools "" --strict-mcp-config --settings <daemon-written JSON
  with remoteControlAtStartup false and disableAllHooks true>`, in a
  160x200 PTY, in a daemon-owned probe directory, with the cleared
  environment of 2.2 plus `CLAUDE_CONFIG_DIR` for a declared extra login.
- Send `/usage`, wait for the panel (8 s), Enter once more if it has not
  appeared, read, Escape, exit. Whole probe 20 s, then SIGTERM, SIGKILL,
  reap. Output read is capped at 1 MiB. `/status` is not sent: its
  `Account:` and `Org:` lines are identity we do not keep.
- Parse "Current session" and "Current week" and their percent and reset
  lines. Anything that does not parse is `unknown`. A CLI that shows an
  update or sign-in prompt instead of the panel is `cli_needs_attention`,
  and the probe presses nothing but Escape.
- **The probe directory must be invisible to our own watcher.** Claude
  Code writes a transcript for every session under
  `~/.claude/projects/<encoded cwd>/`. The daemon watches that tree; a
  probe would otherwise appear as a session in the queue. The probe
  directory is a daemon-internal Never folder that is never shown, and the
  probe deletes its own transcript directory after each run (CodexBar
  limits its cleanup to its probe directory; so does this).
- To verify before shipping: that `/usage` sends no model request; whether
  a non-PTY form exists (CodexBar's undescribed fallback); the panel's
  wording on current Claude Code. A changed panel must read as `unknown`,
  which the fixtures in 7.2 pin.
- Needs a pseudo-terminal: 6.4 lists the dependency, which still needs
  explicit approval under the dependency policy even with U1 (Q3).

### 2.5 Phase 2: browser-cookie web extras (separate opt-in)

Kept, as U1 asks, and held to a separate phase. Reading a browser's cookie
store is a step beyond reading a tool's own sign-in: it reads the store
every site's sessions live in, and decrypting Chrome's cookies needs the
browser's "Safe Storage" Keychain item, the key to all of them. So:

- Off by default, **per provider and per source**: "Codex web extras",
  "Claude web extras", and "Use Claude desktop app cookies", each its own
  switch under its provider, each available only while that provider's
  usage is on.
- Never an automatic fallback for Codex (as CodexBar). For Claude, web is
  tried after OAuth and CLI only when its switch is on (CodexBar's order,
  with the switch CodexBar does not have).
- Cookie stores, in CodexBar's order: Safari
  (`~/Library/Cookies/Cookies.binarycookies`), Chrome and Chromium forks
  (`~/Library/Application Support/Google/Chrome/*/Cookies` and the forks'
  equivalents), Firefox
  (`~/Library/Application Support/Firefox/Profiles/*/cookies.sqlite`).
  Only the named cookie for the named domains is read: Claude `sessionKey`
  on `claude.ai`; Codex the session cookies on `chatgpt.com` and
  `openai.com`. Never cached: read at use, held in memory, dropped.
- To verify: whether current macOS lets the daemon read Safari's cookie
  file without Full Disk Access (Safari's store may sit inside its
  container); whether to support Chromium at all given the Safe Storage
  prompt (Q15).
- **Claude web**: `GET https://claude.ai/api/organizations`, then
  `/api/organizations/{orgId}/usage?cedar_ember=1`, `/overage_spend_limit`,
  `/prepaid/credits`, `/api/account` (plan only) and
  `/subscription_details` (renewal date only), with `Cookie:
  sessionKey=<value>`, host fixed to `claude.ai`. A Cloudflare challenge is
  `network_restricted`, not a stale cookie (CodexBar). The org id is kept
  only as part of the account tag.
- **Codex web**: CodexBar loads
  `https://chatgpt.com/codex/cloud/settings/analytics#usage` in an
  off-screen `WKWebView`. That is a Swift-side component in the macOS app,
  not the daemon; it hands the daemon only the parsed figures over the
  shell's existing daemon client. It fills credits only when 2.1 and 2.2 did
  not, and is rejected when its account does not match the Codex login's
  account tag (CodexBar matches by email; we match by tag so no email is
  kept). CodexBar warns it "can significantly increase battery and network
  use"; it runs at most once per poll and never while on battery (Q15).

### 2.6 Failure handling

Every way of not knowing is a state with a label, never a number. Shown
states and their reasons (wire labels in 3.3):

| Reason | When | Shown as |
| --- | --- | --- |
| `off` | The provider's switch is off. | No usage line. |
| `codex_not_found` / `claude_not_found` | The OAuth source could not answer and no CLI was found. | "Codex not found on this Mac" |
| `signed_out` | No credential, `account: null` with `requiresOpenaiAuth`, or the CLI shows a sign-in prompt. | "Signed out of Codex" / "Signed out of Claude Code" |
| `not_subscription` | Codex signed in with an API key. | "Not a ChatGPT sign-in" |
| `unsupported_login` | Claude token without usage scope and no CLI answer; an `mcpOAuth`-only item. | "Usage not available for this sign-in" |
| `limits_unavailable` | Claude reports null session and weekly windows. | "Limits not available for this plan" |
| `cli_needs_attention` | The CLI asked to update, was blocked by macOS, or is quarantined. | "Codex needs attention. Open Codex once." |
| `keychain_needs_permission` | The Keychain read would prompt and the policy (4.3) forbids it now. | "Choose Refresh to allow access" |
| `base_url_refused` | `chatgpt_base_url` failed the 2.1 check. | "Usage unknown" |
| `network_restricted` | A web request met a challenge page. | "Usage unknown" |
| `timed_out` | A budget ran out. | "Usage unknown" |
| `unreadable` | A response did not parse, failed a range check, or exceeded a cap. | "Usage unknown" |
| `refused` | The Keychain denied, or an endpoint answered non-2xx other than the cases above. | "Usage unknown" |

A probe that fails keeps the last good reading on the row, marked
"Last updated {age} ago" (CodexBar's stale rule), until that window's own
`resets_at` passes; after that the window is "Usage unknown". The answer
says `stale: true` whenever the reading is older than one poll interval
plus five minutes.

The endpoint or CLI changing shape lands in `unreadable`. No reason
carries upstream text. A field the parser does not know is ignored, as
`ironwire.rs:856-870` already tests for the status object; a field it
needs and does not find is that field's `unknown`.

### 2.7 Polling and backoff

- One probe per enabled login every 15 minutes while the daemon runs
  (CodexBar's minimum for local history; its page states no quota
  interval; Q8). 30 minutes in Low Power Mode, as CodexBar.
- Opening Insights or the Private AI screen asks for a refresh, at most
  one per login per 60 seconds; otherwise it reads the held snapshot.
- Failure backoff per login: 15, 30, 60, then 120 minutes, reset by the
  first success. CLI quarantine pauses that login's CLI for 30 minutes.
  **Refresh** retries at once.
- At most one probe runs at a time, daemon-wide. A probe never runs while
  the daemon is quiescing (`quiesce`, `docs/contributor-daemon-ipc-v1_1.md:3751`).
- No probe runs during the developer dry run.

### 2.8 Logging

Label-only, per the repository rule. A probe logs at most: provider,
backend id (a label, never a path), source (`oauth`, `cli`, `web`), reason
label, duration in milliseconds, and an HTTP status. Never a token, a
cookie, an email, an account or org id or its tag, a plan, a percentage, a
response body, a CLI's output, a base URL or a `config_dir` path. The error
from a failed parse is discarded, not logged, because serde quotes what it
choked on (the pattern at `ironwire.rs:664-668`).

## 3. Where it lives

### 3.1 A daemon module, not Swift

New module `crates/trace-commons-contributor/src/subscription_usage/`
(MIT OR Apache-2.0), with:

- `codex_oauth.rs`, `codex_app_server.rs` -- 2.1 and 2.2.
- `claude_oauth.rs`, `claude_cli.rs` -- 2.3 and 2.4.
- `web/` -- Phase 2 (2.5), Claude web and the cookie readers.
- `snapshot.rs` -- the held per-login snapshot, refreshed off the poll
  tick like `IronWireLedger::refresh_spend`, read synchronously by IPC.
- `history.rs` -- the bounded store (3.5).

Exception: the Codex web extra's off-screen `WKWebView` is Swift (2.5).

Why the daemon: it already knows the logins (IronWire's status object and
the home's `config.toml`), already owns probes, settings and the
`unenroll` clearing path, and is what every shell talks to. Putting it in
Swift would make GTK and Windows re-implement it, and the words would
follow, against copy-guide rule 8.

The sources sit behind a trait (`UsageSource`) with an ordered chain per
provider, so tests swap in fakes (section 7) and a later source -- an
upstream IronWire hook, a documented provider API -- replaces one
implementation without touching the IPC surface.

### 3.2 Not IronWire's `headroom`

IronWire's `headroom` is upstream-owned, has no host input, and is the
input to IronWire's routing (`Headroom::is_available`, `is_pressured`,
`quota.rs:49-93`). Writing our display reading into it would make it a
routing input, which is the decision this design defers. So our surface is
parallel and keyed by IronWire backend id: the Private AI row for
`codex-sub` shows IronWire's `authenticated`, `consented` and `health`
beside our usage.

The daemon also starts deserializing IronWire's `headroom` (primary only,
as served), and the row prefers it when it is `observed` and newer than
ours: that is the provider's own header on real traffic. Today it is
always `unknown`, because nothing is routed. The fixture at
`ironwire.rs:866` is corrected to upstream's `state` tag in the same
change.

### 3.3 IPC: `subscription_usage`

Additive. Synchronous and read-only: it answers from the held snapshot and
never launches a probe itself. Follows `insights_glance`.

Params: `refresh` (optional boolean). `true` schedules a probe for each
enabled login subject to the 60-second floor and returns the current
snapshot at once. It is also the only call that may let a Keychain read
prompt (4.3). The fresh answer arrives through the existing
`status_changed` event (no new event; the plan confirms that
`status_changed` may carry a non-queue change, or the shell re-polls).

With both switches off, and the logins not read:

```json
{ "enabled": false }
```

Otherwise:

```json
{
  "enabled": true,
  "logins": [
    {
      "backend": "codex-sub",
      "provider": "codex",
      "default_login": true,
      "state": "observed",
      "reason": null,
      "source": "oauth",
      "updated_at": "2026-10-24T15:02:11+00:00",
      "stale": false,
      "plan": "plus",
      "limit_reached": false,
      "windows": [
        { "lane": "session", "label": null, "duration_mins": 300, "used_permille": 420, "resets_at": "2026-10-24T18:40:00+00:00" },
        { "lane": "weekly", "label": null, "duration_mins": 10080, "used_permille": 130, "resets_at": "2026-10-29T09:00:00+00:00" },
        { "lane": "model", "label": "Spark", "duration_mins": null, "used_permille": 50, "resets_at": null }
      ],
      "credits": { "has_credits": true, "unlimited": false, "balance": "12.50" },
      "reset_credits": 1,
      "extra_usage": null
    },
    {
      "backend": "claude-sub",
      "provider": "claude",
      "default_login": true,
      "state": "unknown",
      "reason": "off",
      "source": null,
      "updated_at": null,
      "stale": false,
      "plan": null,
      "limit_reached": null,
      "windows": [],
      "credits": null,
      "reset_credits": null,
      "extra_usage": null
    }
  ]
}
```

- `logins[]` is one entry per IronWire backend whose `kind` is
  `subscription` and whose id maps to a Claude Code or Codex login, in
  fixed order: Claude logins, then Codex logins, each default login first,
  then by backend id. Never sorted by usage.
- `state` is `observed` or `unknown`; `reason` is a 2.6 label or `null`.
  A stale reading is `observed` with `stale: true` and the failure's
  `reason`.
- `source` says which source gave the reading: `oauth`, `cli` or `web`.
- `lane` is `session`, `weekly`, `model` or `routines`. `label` is the
  checked model-limit name or `null`.
- `used_permille` is an integer 0..=1000. A window the provider did not
  report is absent, not `0`.
- `plan` is a fixed label or `unknown`; never free text. Display names live
  in the Rust copy table.
- `extra_usage` is `{ "spent": "<decimal>", "limit": "<decimal>",
  "currency": "<ISO 4217>" | null }` or `null`.
- Nothing on the wire is an email, account or org id, path, token,
  cookie, URL or upstream message.

Older shells ignore the method; a shell that gets `unknown_method` (an
older daemon) shows the rows exactly as today.

### 3.4 Settings

New key on `set_settings` / `get_settings`:

```json
{
  "subscription_usage": {
    "codex": false,
    "claude": false,
    "keychain_prompts": "user_action",
    "web": { "codex": false, "claude": false, "claude_desktop_app": false }
  }
}
```

All switches default `false`; `keychain_prompts` is `never`,
`user_action` (default) or `always`, CodexBar's three. `web.*` is refused
(`bad_params`, with a new reason label the plan names) until Phase 2 ships,
and while its provider's switch is off. A key the daemon does not know is
kept on load and save, as `notify` does. These are settings about this
Mac, so `unenroll` keeps them (the precedent at
`docs/contributor-daemon-ipc-v1_1.md:5036`); `unenroll` clears the history
(3.5).

### 3.5 On-device history for Insights

- File `subscription_usage.jsonl` in the daemon state directory, mode
  0600, written atomically.
- One row per login per probe **only when the reading changed**:
  `backend`, `provider`, `observed_at`, and per window `lane`, `label`,
  `duration_mins`, `used_permille`, `resets_at`. No plan, credits, extra
  usage or account tag.
- Bounded: rows older than 8 days dropped on every write; at most 2,000
  rows overall, oldest dropped first.
- A header row holds each login's `account_tag`. When a probe sees a
  different tag for a login, that login's rows are deleted before the new
  row is written.
- The `install_salt` is 32 random bytes in the daemon's existing keychain
  entry, not in the file.

**Deleted:**

- that provider's rows, at once, when its switch goes off;
- a login's rows when a probe sees it `signed_out` or its tag change;
- a login's rows when IronWire stops listing its backend for 24 hours;
- the whole file on `unenroll`. A probe that was running at `unenroll`
  writes nothing afterwards (the counter-store rule, `:4527`).

## 4. Consent

### 4.1 The switches

Settings gets a section "Subscription usage" with one switch per provider,
"Codex usage" and "Claude usage", and under each, once Phase 2 ships, its
web switches. Each switch's description says what turning it on reads,
before the click (copy-guide rule 3; draft copy in 5.3). Off by default.
No first-run step, no offer card and no nudge (rule 9).

### 4.2 Codex usage on

No macOS prompt on the Codex path. The first probe runs within a minute.
It reads `auth.json` and calls ChatGPT with its token; if that cannot
answer for an authentication reason, it runs the Codex CLI. Codex in
managed sign-in mode may refresh its own token while the app-server runs;
that is Codex writing its own file. `refreshToken` is never sent.

### 4.3 The Keychain read and its prompt

The Claude path reads the file first and the Keychain only when there is
no file (2.3). The Keychain read follows CodexBar's prompt policy, set in
`keychain_prompts`:

- **Background probes never prompt**, under every setting. The read uses
  the Security framework from the signed daemon with authentication UI
  disabled (`security-framework`, already in
  `trace-commons-contributor-platform`), so a read that would need the
  contributor's permission fails as `keychain_needs_permission` instead of
  showing a dialog.
- **`user_action` (default)**: a read may prompt only inside a
  `subscription_usage` call with `refresh: true` from a click on **Refresh**,
  or the first probe right after the contributor turns Claude usage on.
- **`never`**: no read ever prompts; the CLI fallback answers instead.
- **`always`**: background reads may prompt too. Offered because CodexBar
  offers it; not recommended in the copy.
- A freshness check reads only the item's attributes, never its secret
  (CodexBar).

The dialog is macOS's. Read through the Security framework, it names the
requesting program (our daemon) and the item `Claude Code-credentials`,
with **Deny**, **Allow** and **Always Allow**. To be verified on a test Mac
with a test Claude account before anything ships.

**Kept open (Q10): IronWire's existing read.** IronWire reads the same item
by running `/usr/bin/security`. On that path the dialog names `security`,
and, to be verified, **Always Allow adds `security` -- not Trace Commons --
to the item's access list**, after which any program running as the
contributor can read the item through `security` without a prompt. That
happens today whenever Private AI runs with a NEAR AI key. This design
does not depend on it, and the item stays open until it is verified and
fixed or ruled out (an upstream change to read through the Security
framework, or `CredentialFiles::Off` for Claude logins until routing needs
them).

### 4.4 Web extras on (Phase 2)

Turning on a web switch reads that browser's cookie store at the next
probe. Firefox and Safari need no prompt (Safari may need Full Disk Access,
to verify). Chrome asks for its "Chrome Safe Storage" Keychain item; that
prompt follows the same `keychain_prompts` policy. The description beside
each switch names which browsers are read and that only one cookie is
used (draft, 5.3).

### 4.5 Turning it off

Turning a switch off: no further probes for that provider or source, the
held snapshot for its logins is dropped, and its history rows are deleted
at once (3.5). Any child process for a running probe is killed and reaped.
Nothing at the provider or in a browser changes: we never wrote anything
there and cached nothing. An "Always Allow" entry the contributor granted
in Keychain Access stays until they remove it; the description beside the
switch says where (draft).

### 4.6 Other shells

Native macOS is the lead client (#1173). The daemon surface is
shell-neutral, so GTK and Windows can render the same rows later. On
Linux there is no Keychain and the Claude path reads only the file; Phase
2 on Linux would read only Firefox and Chromium stores. Tauri gets nothing
(frozen). None of this is in the first delivery.

## 5. Screens

All wording below is **DRAFT, for owner review**. Nothing is approved.
Strings go in the Rust copy tables with separate title, button and
accessibility keys (rules 4, 5, 8). Placeholders are in braces.

### 5.1 Insights

A "Subscriptions" card on the Insights Overview, shown only while at least
one switch is on.

Per login:

- Title: the plan, e.g. "ChatGPT Plus" / "Claude Max 5x"; "Codex" or
  "Claude" with "Plan unknown" when `plan` is `unknown`. A second login
  adds its backend id: "ChatGPT Plus (codex-work)".
- One meter per session and weekly window: the lane's name, "{n}% used",
  "Resets {relative time}". Model-specific windows below, smaller, named
  by their label. An unknown window is the words "Usage unknown", never an
  empty bar.
- Credits: "Credits available", "Unlimited credits" or "{balance}
  credits"; reset credits "{n} reset credits". Claude extra usage:
  "Extra usage {spent} of {limit} this month".
- "Updated {age} ago", or "Last updated {age} ago" while stale.
- `limit_reached`: "Limit reached" in the row's status line. No
  notification.
- **Refresh** button, accessible name "Refresh subscription usage".

**Share of work.** Beside the card, the split Insights already computes,
not a new measurement:

- Tokens by tool from `insights_week` (Claude Code, Codex), and tokens
  through Private AI from the routing split `insights_week.routing`
  already reports.
- Labelled by tool, not by subscription: a Claude Code session may have
  run on an API key, and Insights cannot tell from the transcript. The
  line reads "Claude Code: {tokens} this week", never "Claude Max:
  {tokens}". Where the IronWire ledger names a backend id for a call
  (`inference_summary` groups by backend), that call may be labelled by
  its subscription; today none is, because none is routed.
- No line divides tokens by the provider's percentage to estimate a limit.

### 5.2 Private AI

The Inference tab lists, under the existing NEAR AI connection card, one
read-only row per subscription login IronWire reports:

- Name: "ChatGPT subscription" / "Claude subscription" (IronWire's `name`
  is upstream prose and is not shown).
- Status: "Not used by Private AI" while `consented` is false, always
  shown so the usage beside it is never read as routing.
- Usage: compact, "5-hour {n}% · Weekly {m}%", or "Usage unknown", or
  "Turn on usage in Settings" with a **Review settings** button (rule 2).
- No switch, no **Use** button. Making a row usable is section 10.

### 5.3 Draft copy

| Key | Draft text |
| --- | --- |
| `subscription_usage_section_title` | Subscription usage |
| `subscription_usage_codex_switch` | Codex usage |
| `subscription_usage_codex_detail` | Shows your ChatGPT plan and how much of each Codex limit you have used. Trace Commons reads Codex's sign-in on this Mac to ask ChatGPT, or asks the Codex app. It never saves or changes your sign-in. Nothing leaves this Mac except that request. |
| `subscription_usage_claude_switch` | Claude usage |
| `subscription_usage_claude_detail` | Shows your Claude plan and how much of each limit you have used. Trace Commons reads Claude Code's sign-in on this Mac to ask Anthropic, or asks Claude Code. It never saves or changes your sign-in. macOS may ask first; Allow is enough. |
| `subscription_usage_keychain_title` | Keychain access |
| `subscription_usage_keychain_never` | Never ask |
| `subscription_usage_keychain_user_action` | Ask when I choose Refresh |
| `subscription_usage_keychain_always` | Always allow asking |
| `subscription_usage_web_codex_switch` | Codex web extras |
| `subscription_usage_web_claude_switch` | Claude web extras |
| `subscription_usage_web_desktop_switch` | Use Claude desktop app cookies |
| `subscription_usage_web_detail` | Reads one sign-in cookie for {site} from Safari, Chrome or Firefox to read extra figures from the website. Chrome asks for its keychain item first. Nothing is saved. |
| `subscription_usage_off_effect` | Turning this off deletes the usage history kept on this Mac. |
| `subscription_usage_always_allow_note` | If you chose Always Allow, you can remove it in Keychain Access. |
| `subscription_usage_card_title` | Subscriptions |
| `subscription_usage_window_5h` | 5-hour |
| `subscription_usage_window_weekly` | Weekly |
| `subscription_usage_window_hours` | {n}-hour |
| `subscription_usage_window_days` | {n}-day |
| `subscription_usage_window_unnamed` | Usage window |
| `subscription_usage_window_routines` | Daily Routines |
| `subscription_usage_used` | {n}% used |
| `subscription_usage_resets` | Resets {relative} |
| `subscription_usage_updated` | Updated {age} ago |
| `subscription_usage_last_updated` | Last updated {age} ago |
| `subscription_usage_unknown` | Usage unknown |
| `subscription_usage_plan_unknown` | Plan unknown |
| `subscription_usage_limit_reached` | Limit reached |
| `subscription_usage_limits_unavailable` | Limits not available for this plan |
| `subscription_usage_credits_available` | Credits available |
| `subscription_usage_credits_unlimited` | Unlimited credits |
| `subscription_usage_credits_balance` | {balance} credits |
| `subscription_usage_reset_credits` | {n} reset credits |
| `subscription_usage_extra_usage` | Extra usage {spent} of {limit} this month |
| `subscription_usage_codex_not_found` | Codex not found on this Mac |
| `subscription_usage_claude_not_found` | Claude Code not found on this Mac |
| `subscription_usage_signed_out_codex` | Signed out of Codex |
| `subscription_usage_signed_out_claude` | Signed out of Claude Code |
| `subscription_usage_not_subscription` | Not a ChatGPT sign-in |
| `subscription_usage_unsupported_login` | Usage not available for this sign-in |
| `subscription_usage_needs_attention` | Codex needs attention. Open Codex once. |
| `subscription_usage_keychain_needs_permission` | Choose Refresh to allow access |
| `subscription_usage_refresh` (button) | Refresh |
| `subscription_usage_refresh_a11y` | Refresh subscription usage |
| `subscription_usage_not_used` | Not used by Private AI |
| `subscription_usage_review_settings` (button) | Review settings |
| `subscription_usage_turn_on_hint` | Turn on usage in Settings |
| `subscription_usage_share_tool` | {tool}: {tokens} this week |

## 6. Privacy and security

### 6.1 What is read, and what never leaves the Mac

| Read | From | Kept | Leaves the Mac |
| --- | --- | --- | --- |
| Codex access token, ID-token plan claim | `auth.json` | Token in memory for the request; plan in the snapshot | The token, only as the bearer to the checked ChatGPT host |
| Codex windows, credits, reset credits | `wham/usage` or the app-server | Snapshot; windows in history | Never |
| Codex account id, email | `auth.json` claims, app-server | Tag only; email never deserialized | Never |
| Claude access token | File, else Keychain | Memory, for two requests | Only as the bearer to `api.anthropic.com` |
| Claude windows, plan, extra usage | `api.anthropic.com` or Claude Code's panel | Snapshot; windows in history | Never |
| Browser cookie (Phase 2) | Safari, Chrome, Firefox stores | Memory, for the requests | Only to `claude.ai`, or `chatgpt.com` in the web view |

"Never" is tested (7.3): the module's HTTP client accepts only
`api.anthropic.com`, `claude.ai` and the checked ChatGPT/OpenAI hosts.

### 6.2 Threat model

- **A malicious process running as the contributor.** It can already read
  `auth.json`, `.credentials.json` and Firefox's cookie store, call
  `security`, and connect to our IPC socket. This design adds no file it
  could not already read: the history holds percentages and reset times.
  The IPC answer adds plan and usage to what the socket already exposes;
  the socket's existing authorization applies
  (`docs/contributor-daemon-ipc-v1_1.md:311`). Our Keychain read is through
  the Security framework, so an "Always Allow" names our daemon, not
  `security` (4.3).
- **A redirected base URL.** `chatgpt_base_url` decides where the ChatGPT
  bearer goes. An edited `config.toml` could send it to an attacker's host;
  the 2.1 host check refuses that. A same-user attacker who can edit the
  file can also read `auth.json`, so the check stops exfiltration through
  us, not the attacker.
- **A planted `codex` or `claude`.** We resolve an absolute path from
  harness detection and never search the daemon's `PATH`.
- **Tokens and cookies in memory.** `secrecy::SecretString` (in the tree
  through IronWire's `ironwire_creds`; a direct dependency is listed in
  6.4), zeroed on drop, no `Debug`, never in an error value, dropped when
  the request returns. No cache.
- **Tokens, cookies or identity in logs and crash reports.** 2.8. Panic
  messages and `Debug` of every probe type are tested not to contain a
  seeded token, cookie, email or account id (7.3). A child's stderr is
  never read.
- **Phase 2's reach.** Decrypting Chrome's cookies means holding the key to
  every Chrome cookie for the length of one read. The reader decrypts only
  the rows for the named cookie on the named domains, holds the key in a
  zeroing buffer and drops it at once. This is the main reason Phase 2 is
  its own opt-in (Q15).
- **The child processes.** Cleared environment, daemon-owned directory,
  bounded time and output, killed and reaped on every path (the pattern of
  `ironwire_creds::claude::keychain_output`). The Claude probe directory is
  a Never folder to our own watcher (2.4).
- **A misread figure.** Range checks, and `unknown` on any doubt. Display
  only, so a wrong figure misleads but routes nothing.
- **Provider endpoints and panels change.** `wham/usage`, the OAuth usage
  endpoint, the app-server's account methods and a TUI panel are all
  internal and can change without notice. Each change lands as
  `unreadable`, which the contract fixtures pin (7.2).

### 6.3 Licensing

All new code is in `trace-commons-contributor`, MIT OR Apache-2.0, except
the Codex web view, which is in the macOS app. No AGPL crate is involved;
`license_boundary.rs` is unaffected.

### 6.4 Dependencies

Phase 1 OAuth and app-server sources need none beyond what the crate has
(`tokio`, `serde_json`, `reqwest`, `base64` for the ID-token payload,
`sha2` for the tag). Two need approval; none is added without it.

**`secrecy`, as a direct dependency.** Already in `Cargo.lock` through
IronWire. Fields gathered at proposal time; it adds no new crate to the
tree.

**A pseudo-terminal, for 2.4.**

| Field | `libc` (direct, inline `openpty`) | `portable-pty` |
| --- | --- | --- |
| Version | 0.2.191 latest; 0.2.186 already in `Cargo.lock` | 0.9.0 |
| Purpose | `openpty` and terminal setup in roughly 60-100 lines of our own `unsafe` | A cross-platform PTY API (Unix and Windows ConPTY) |
| Adoption (crates.io, 2026-10-24) | 1.84 billion downloads, 435 million recent | 23.2 million downloads, 13.7 million recent |
| New transitive crates | None: already in the tree | 13 normal non-optional dependencies, some new to this tree (`filedescriptor`, `serial2`, `shared_library`, `downcast-rs`, others), to be counted with `cargo tree` at proposal time |
| Maintenance | Released 2026-10-16 (rust-lang) | Last release 2025-02-11, in the `wezterm` monorepo |
| License | MIT OR Apache-2.0 | MIT |
| Advisories | Not checked for this spec; `cargo deny check advisories` at proposal time | Not checked for this spec; same |

Recommendation: `libc` direct, macOS and Linux only. It is above the ~30-line
inline threshold, so it still needs explicit approval.

**Phase 2 cookie readers.** Firefox and Chrome stores are SQLite;
`rusqlite` is already a direct dependency. Safari's binary cookie format is
parsed inline. Chrome's decryption is PBKDF2-SHA1 then AES-128-CBC: `aes`,
`cbc`, `hmac` and `sha1` are in the lockfile transitively and would become
direct dependencies; PBKDF2 over `hmac` is about 15 lines inline. All four
are RustCrypto crates under MIT OR Apache-2.0; the full proposal fields are
gathered when Phase 2 is planned.

### 6.5 Terms note

Recorded for a review before production, per U1, without blocking the
build.

- **CodexBar is software a person installs and runs for themselves.** We
  distribute the same behaviour inside an app we ship to contributors.
  The terms below may read differently for the two.
- **Anthropic**, from <https://code.claude.com/docs/en/legal-and-compliance>,
  "Authentication and credential use", fetched 2026-10-24:

  > "**OAuth authentication** is intended exclusively for purchasers of
  > Claude Free, Pro, Max, Team, and Enterprise subscription plans and is
  > designed to support ordinary use of Claude Code and other native
  > Anthropic applications."

  > "Anthropic does not permit third-party developers to offer Claude.ai
  > login into their own applications, or to route requests through Free,
  > Pro, or Max plan credentials on behalf of their users. Moreover,
  > developers may not collect, store, or intermediate Claude.ai credentials
  > or session tokens — sign-in to a Claude account must complete through
  > Anthropic's own flow."

  > "Nor does it prevent an end user from signing in to the unmodified
  > Claude Code binary with their own Claude subscription, including where
  > a platform hosts Claude Code as described under *Can customers offer
  > Claude Code in their products?* above."

  The OAuth source (2.3) and the Claude web extra (2.5) read a Claude
  token or session cookie; the CLI fallback (2.4) runs the unmodified
  Claude Code binary. Routing (section 10) is the subject of the "route
  requests" sentence. This note draws no conclusion.
- **OpenAI.** The terms for using a ChatGPT sign-in through `auth.json`,
  `wham/usage`, the Codex app-server and the ChatGPT website were not
  checked for this spec. They need the same review.

## 7. Testing

No test makes a live call to a provider, launches a real `codex` or
`claude`, reads a real browser store or touches the real Keychain, in CI
or locally.

### 7.1 Fakes

- **Fake HTTP**: a loopback server serving recorded `wham/usage`,
  reset-credits, OAuth usage and profile bodies, and the Claude web
  endpoints, including 400/403 for the `cedar_ember` retry and 401 for
  fallback. Production builds refuse every host but the allowed ones; tests
  inject the loopback base through a test-only constructor.
- **Fake app-server**: a small test binary that speaks the JSONL exchange
  from a script: replies, delays, oversize lines, early exit, garbage. The
  probe is pointed at it by absolute path.
- **Fake Claude CLI**: a binary that writes recorded `/usage` panels to the
  PTY, including update and sign-in prompts.
- **Fake credential stores**: temp-dir `auth.json`, `config.toml`,
  `.credentials.json`, cookie databases built in the test; the Keychain
  lookup is a parameter, as `ClaudeLogin::read_with` already does upstream.

### 7.2 Contract fixtures

- App-server responses generated from the TypeScript schema at the pinned
  `openai/codex` revision; `wham/usage` and Claude bodies recorded by a
  maintainer from a test account and scrubbed. All under
  `crates/trace-commons-contributor/tests/fixtures/subscription_usage/`,
  each with its source and date in the file name. Every personal field is
  a placeholder (`user@example.invalid`, `acct_example`).
- Each 2.6 reason has at least one fixture that produces it.
- A fixture with every field the parser does not keep set to a seeded
  marker, asserting the marker never appears in the IPC answer, the
  history file or captured logs.
- The plan records how a maintainer regenerates a fixture from their own
  account without committing it.

### 7.3 Properties

- Fallback only on authentication failures; a 500 or a decode error does
  not launch a CLI.
- `refreshToken` never appears in any request; no source writes to any
  credential file, `config.toml` or Keychain item (tested by file hashes
  and a recording Keychain fake).
- `chatgpt_base_url` host check, including look-alike hosts.
- Background Keychain reads never request UI under any policy except
  `always`.
- Stale readings drop each window at its `resets_at`.
- History bounds, account-tag change, switch-off and `unenroll` deletion,
  and no write after `unenroll` during a running probe.
- The Claude probe directory is never offered as a session.
- Seeded token, cookie, email and account id never in `Debug`, `Display`,
  panic messages or logs.
- Every child is reaped on every timeout path.

### 7.4 Shell

Swift decoder tests for the new payload, including an older daemon's
`unknown_method`, in `macos/` (`swift test`, the `macOS app tests` job).
Copy-contract tests for every new key. Per the repository gotcha, the
contributor crate's tests for the `subscription-usage` feature must run
with `--features subscription-usage` (or `--workspace`), or a green run
proves nothing.

## 8. Rollout

1. **Build behind a feature.** Cargo feature `subscription-usage` on
   `trace-commons-contributor`, off by default. Without it
   `subscription_usage` is `unknown_method` and the setting is refused. The
   `cargo check (permissive crates, standalone)` job keeps the crate
   building without it.
2. **Staging, Phase 1.** The staging macOS build (staging environment
   design, S2) turns the feature on. Staging has its own IronWire home
   (S10). Internal testers turn on Codex and Claude usage and report: plan
   correct, percentages within a point of what each tool shows, resets
   correct, both fallbacks exercised (by signing the OAuth source out),
   Keychain prompt behaviour under each policy, no probe session in the
   queue, battery effect over a day.
3. **Terms review** (6.5) before production.
4. **Production, Phase 1, off by default.** Each switch stays off until the
   contributor turns it on.
5. **Phase 2** (web extras) as its own PR, its own staging round, and its
   own terms check.

Kill switch: the setting, per Mac. A release with the feature off removes
the surface entirely; nothing server-side exists to turn off.

## 9. Delivery

Not a plan; the order the plan should follow.

1. Correct the `headroom` fixture tag; deserialize IronWire's `headroom`
   (primary) beside `authenticated`.
2. `subscription_usage` module: Codex OAuth and app-server sources, fakes,
   fixtures.
3. Claude OAuth source with the Keychain policy; Claude CLI fallback once
   the PTY dependency is approved.
4. Snapshot, poll, backoff; IPC method and settings; IPC doc section.
5. History store and deletion paths.
6. macOS Settings, Insights card, Private AI rows; Rust copy keys.
7. Staging round; terms review; production.
8. Phase 2.

## 10. Leaving room for routing

Routing a Private AI call through a subscription is a later owner decision,
and the quoted "route requests through Free, Pro, or Max plan credentials
on behalf of their users" bears on it directly for Claude. This design
keeps the way open without taking it:

- Everything is keyed by IronWire backend id, the same id IronWire routes
  by and records consent against.
- If routing is approved, IronWire fills `headroom` from response headers
  on routed traffic by itself; our probe becomes the reading for idle
  periods.
- An upstream `EmbedOptions` hook that accepts a host observation as
  `Headroom::Observed` (with the host's `observed_at`, so IronWire's age
  rules apply) would let our reading inform routing. That is an upstream
  change to propose then, not now.
- The Private AI rows are where a **Use** control would go. Its consent
  belongs to IronWire's existing per-login consent ledger
  (`ironwire_creds::consent`), not to the usage switch: turning on usage
  must never be read as consent to route.

## 11. Open questions and owner decisions

**Decided by U1 (2026-10-24):** Q1 Codex source order (OAuth, then the
app-server); Q2 Claude OAuth usage endpoint (built, first); Q3 Claude CLI
`/usage` fallback (built, second -- its PTY dependency still needs
approval under the dependency policy); Q7 browser cookies (kept, as the
separate opt-in Phase 2). Also settled by mirroring CodexBar: Q4 credits
are shown; Q5 model-specific windows, reset credits and extra usage are
shown.

Still open, each with a recommendation the owner may override:

- **Q3a. PTY dependency.** *Recommend `libc` as a direct dependency* (6.4).
- **Q6. Terms review** (6.5), Anthropic and OpenAI. *Recommend before
  production*, not blocking the build or staging.
- **Q8. Poll interval.** *Recommend 15 minutes, 30 in Low Power Mode*, plus
  refresh on opening the screens.
- **Q9. A "limit reached" or "resets soon" notification?** Rule 9 requires
  explicit approval for any notice. *Recommend none in v1.*
- **Q10. IronWire's existing Claude Keychain read through `security`.**
  To verify on a test Mac whether "Always Allow" opens the item to every
  same-user program (4.3). *Recommend verifying before production, then
  either an upstream change to the Security framework or
  `CredentialFiles::Off` for Claude logins until routing needs them* -- a
  behaviour change for anyone who already consented, so its own decision.
- **Q11. History on sign-out.** *Recommend deleting a login's history when
  a probe sees it signed out.*
- **Q12. Plan names.** *Recommend plain-text names in the Rust copy table,
  no logos*, consistent with the name-use paragraph on the same Anthropic
  page.
- **Q13. CodexBar's Codex "PAT" source.** CodexBar's page lists it first
  and does not say what it is. *Recommend omitting it* until it is
  identified.
- **Q14. `chatgpt_base_url`.** *Recommend honouring it only for `https`
  on ChatGPT or OpenAI hosts* (2.1); CodexBar honours any value.
- **Q15. Phase 2 scope.** *Recommend shipping Firefox and Safari first and
  Chromium last, after a separate look at holding the Safe Storage key*,
  and the Codex web view never on battery.
- **Q16. Stale display.** CodexBar keeps the last percentages "marked as
  stale". *Recommend the same, until each window's own reset passes* (2.6).
