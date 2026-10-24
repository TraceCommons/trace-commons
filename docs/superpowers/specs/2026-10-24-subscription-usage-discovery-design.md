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
in Private AI. It does not route anything through a subscription.

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

## Direction accepted

The owner accepted this direction on 2026-10-24. It is a direction, not a
set of settled decisions: section 11 lists what is still open.

- **Display only, now.** Learn plan, usage windows with their resets and,
  for Codex, credits. Show them in Insights and as read-only rows in
  Private AI, with the subscription's usage filled in where IronWire today
  shows `unknown`.
- **Opt in per provider.** Off until the contributor turns on Codex usage
  or Claude usage in Settings, separately.
- **On this Mac only**, like the rest of Insights. Nothing is uploaded.
- **Routing Private AI calls through a subscription is a separate, later
  decision.** This design leaves room for it (section 10) and does not make
  it.
- **Prefer sources that leave the provider's sign-in with the provider's
  own tool.** For Codex that is its app-server. For Claude, the accepted
  answer named the OAuth usage endpoint; section 2.3 shows why this design
  now recommends against making that the default until the owner (and,
  if he wants it, legal) has read the terms text quoted there.

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
  `authenticated: true` is known today. **This exposure predates this
  design** and is raised for the same terms review in section 2.3 rather
  than silently inherited.
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
2026-10-24. Recorded so the comparison is honest; where this design
differs, it says why.

- **Codex.** CodexBar's Auto order is a personal access token, then the
  OAuth API using `auth.json`
  (`GET https://chatgpt.com/backend-api/wham/usage`, `Authorization:
  Bearer`; `rate_limit.primary_window` / `secondary_window` and
  `additional_rate_limits[]`), then the app-server: `codex -s read-only -a
  never app-server`, JSON-RPC over stdio, `initialize`, `account/read`,
  `account/rateLimits/read`. It never writes refreshed tokens into
  `auth.json`. It warns that a bare `codex` "can start interactive auth and
  open browser tabs", and pauses background launches for 30 minutes when
  macOS blocks or quarantines `codex`. **Difference:** this design prefers
  the app-server, because there the Codex CLI holds its own token and we
  never read it.
- **Claude.** CodexBar's app Auto order is the OAuth API, then the CLI in a
  PTY, then cookies. The OAuth path reads `~/.claude/.credentials.json`
  first and the `Claude Code-credentials` Keychain item "only as a
  bootstrap/repair fallback", then calls
  `GET https://api.anthropic.com/api/oauth/usage` (CodexBar adds
  `?cedar_ember=1`) and `GET https://api.anthropic.com/api/oauth/profile`
  with `Authorization: Bearer <access_token>` and `anthropic-beta:
  oauth-2025-04-20`. Fields: `five_hour`, `seven_day`, `seven_day_sonnet`,
  `seven_day_opus`, `extra_usage`; plan from `subscriptionType`, else
  `rate_limit_tier`. The token needs the `user:profile` scope. **CodexBar
  refreshes Claude tokens itself; this design never does.** The CLI path
  runs `claude` in a PTY with `--allowed-tools ""`, `--strict-mcp-config`
  and `--settings` disabling hooks, sends `/usage`, parses "Current
  session" and "Current week", and runs in its own working directory. The
  document mentions "a non-PTY `/usage` fallback" without saying what it
  is; this design treats it as unverified.

### Anthropic's terms text

From <https://code.claude.com/docs/en/legal-and-compliance>, section
"Authentication and credential use", fetched 2026-10-24. Quoted, not
interpreted:

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
> Claude Code binary with their own Claude subscription, including where a
> platform hosts Claude Code as described under *Can customers offer
> Claude Code in their products?* above."

This design draws no legal conclusion from that text. Section 2.3 says
which sentence each Claude option sits next to. **OpenAI's terms for using
a ChatGPT sign-in through the Codex CLI were not checked**; that is open
question Q6.

## 1. Goals and non-goals

### Goals

1. When the contributor turns it on, show for each Codex login and each
   Claude Code login on this Mac: the plan, each usage window the provider
   reports (percent used and when it resets) and, for Codex, whether
   credits are available.
2. Show it in Insights, beside the share of work each tool did, using data
   Insights already has.
3. Show each subscription as a read-only row on the Private AI screen,
   with its usage in place of today's nothing.
4. Never show a number the provider did not report. Every failure is
   "unknown", in words, never zero and never the last good figure past
   its age limit.
5. Keep everything on this Mac. No usage figure, plan, account identifier
   or token is uploaded, logged in clear or written by us to disk, except
   the bounded on-device history in 3.5.

### Non-goals

- **Routing.** No Private AI call goes to a subscription because of this
  design. `consented` stays whatever IronWire says, and nothing here
  grants consent (section 10).
- **Token refresh.** We never refresh either provider's token and never
  write to either provider's credential store.
- **Browser cookies or dashboard scraping.** Not for either provider
  (CodexBar's optional web paths). Out of scope, and Q7 asks the owner to
  confirm.
- **Cost estimates from local transcripts.** Insights already counts
  tokens (`insights_week`); this design adds no price table.
- **Notifications.** No limit-reached or reset notification. A notice
  needs the owner's explicit approval (`docs/ui-copy-guide.md` rule 9);
  Q9 raises it as a question.
- **Shells other than native macOS**, for now (section 4.5).

## 2. Data sources

### 2.1 Codex, preferred: the Codex app-server

The daemon launches the contributor's own Codex CLI and asks it. Codex
reads and, in its own managed mode, refreshes its own sign-in; we never
see the token.

**Launch.** Exactly:

```text
<absolute path to codex> -s read-only -a never app-server
```

- Never a bare `codex`, which can start interactive sign-in and open
  browser tabs (CodexBar).
- `<absolute path>` comes from the daemon's existing harness detection
  (the harness list, `docs/contributor-daemon-ipc-v1_1.md:3865`), never from
  `PATH` lookup inside the daemon's launchd environment. Not found is
  `codex_not_found`.
- `CODEX_HOME` is set to the login's `config_dir` for a declared extra
  login, and unset for the default login, so each IronWire backend id is
  asked through its own home. The child's environment is otherwise
  cleared down to `HOME`, `PATH` (fixed, not inherited), `LANG` and
  `TMPDIR`: no NEAR AI key, no IronWire control token, no proxy URL is
  inherited.
- Working directory: a daemon-owned empty directory, not a project.
- stdin and stdout piped; stderr discarded unread (it is upstream text we
  neither show nor store).

**Exchange.** One JSON object per line over stdio, as the app-server's
own examples show. In order:

1. `initialize` with `clientInfo: { name: "trace_commons", title: null,
   version: <app version> }`, `capabilities: null`. The plan verifies
   whether the pinned schema also needs an `initialized` notification and
   whether a `jsonrpc` field is required; the app-server README's examples
   omit it.
2. `account/read` with `params: {}`. **`refreshToken` is never sent.**
3. `account/rateLimits/read` with no params.

Then close stdin, wait up to 2 s, SIGTERM, wait 2 s, SIGKILL, reap. Budgets:
10 s for `initialize`, 5 s per request, 20 s for the whole probe. Each
stdout line is capped at 1 MiB; a longer line ends the probe as
`unreadable`. Notifications (including `account/rateLimits/updated`) are
read and discarded in v1: the probe is a one-shot read, not a session.

**What is kept.** Parsed into narrow structs that have no field for
anything not listed here, so an email or account id cannot be stored by
accident:

| Source field | Kept as | Rule |
| --- | --- | --- |
| `account.type` | `signed_in: bool` | `chatgpt` is a subscription sign-in. `apiKey` or any other type is `not_subscription`: shown as "Not a ChatGPT sign-in", no usage. `account: null` with `requiresOpenaiAuth: true` is `signed_out`. |
| `rateLimits.planType`, else `account.planType` | `plan` | Fresh rate-limit plan wins, as CodexBar does. A value outside the pinned `PlanType` list, or `"unknown"`, is `unknown`. Never guessed from usage. |
| `rateLimits.primary`, `.secondary` | `windows[]` | `usedPercent` finite and within 0..=100, else that window is `unknown`. Kept as permille. `windowDurationMins` kept as is; a null duration is a window with no name ("Usage window"). Not assumed to be 5 hours or a week: the label comes from the duration (300 -> "5-hour", 10080 -> "Weekly", anything else -> "{n}-hour" or "{n}-day"). |
| `resetsAt` | `resets_at` | Unix seconds (the plan confirms the unit against a recorded fixture). In the past or more than 31 days ahead is dropped, not clamped. |
| `credits` | `credits` | `hasCredits` and `unlimited` as given. `balance` only when it parses as a non-negative decimal, else absent. Section 11 Q4 decides whether it is shown. |
| `ordinaryUsageAllowed` | `limit_reached` | `false` -> `true`; `null` -> unknown, and per the schema's own comment never inferred from percentages. |
| `accountId` | `account_tag` | Never kept. Only `HMAC-SHA256(install_salt, accountId)` truncated to 16 bytes, used to notice that a login now belongs to another account (3.5). |
| `account.email` | nothing | Never deserialized. |
| `rateLimitsByLimitId`, `rateLimitResetCredits`, `rateLimitUpsell`, `spendControlReached`, `individualLimit` | nothing in v1 | Q5. |

### 2.2 Codex, alternative: `auth.json` and `wham/usage` (not proposed)

`~/.codex/auth.json` (or `$CODEX_HOME/auth.json`) bearer ->
`GET https://chatgpt.com/backend-api/wham/usage`. It is CodexBar's
preferred path and needs no child process. This design does not build it:
it means our process reading and holding the contributor's ChatGPT token,
which the app-server path avoids entirely, and the endpoint is internal and
undocumented. It stays an owner option (Q1) if the app-server proves
unreliable.

### 2.3 Claude: two options, neither the default yet

The terms text quoted above is the reason this section does not simply
build the usage endpoint the accepted answer named.

**Option A -- the unmodified `claude` CLI asks for itself.** The daemon
starts the contributor's own Claude Code in a pseudo-terminal, sends
`/usage`, parses the panel, and exits. Claude Code reads its own sign-in;
we never touch the Keychain item or the file. This sits next to the
sentence "Nor does it prevent an end user from signing in to the
unmodified Claude Code binary with their own Claude subscription".

- Launch: `<absolute path to claude> --allowed-tools "" --strict-mcp-config
  --settings <daemon-written JSON disabling hooks and remote control>`, in
  a daemon-owned working directory, with the same cleared environment as
  2.1 plus `CLAUDE_CONFIG_DIR` for a declared extra login.
- **The probe's working directory must be invisible to our own watcher.**
  Claude Code writes a transcript for every session under
  `~/.claude/projects/<encoded cwd>/`. The daemon watches that tree; a
  probe would otherwise appear as a session in the queue. The probe
  directory is registered as a daemon-internal Never folder that is never
  shown, and the plan adds a test that a probe's transcript is never
  offered.
- Parse "Current session" and "Current week" headers and their percent
  and reset lines, as CodexBar does. Anything that does not parse is
  `unknown`. A CLI that shows an update or sign-in prompt instead of the
  panel is `cli_needs_attention`, and the probe ends without pressing
  anything but Escape.
- To verify before building: that `/usage` sends no model request and so
  consumes no usage; whether a non-PTY form exists (CodexBar's unexplained
  "non-PTY `/usage` fallback"); and the panel's wording on current Claude
  Code. A text-scraping parser breaks when Claude Code changes its panel,
  and that failure must read as `unknown`, which the fixtures in 7.2 pin.
- Needs a pseudo-terminal. That is either a new dependency or a new direct
  dependency on a crate already in the lockfile (6.4).

**Option B -- read the token, call the usage endpoint.** Read the Claude
Code access token (the file `~/.claude/.credentials.json`, else the
`Claude Code-credentials` Keychain item, as IronWire does), then:

```text
GET https://api.anthropic.com/api/oauth/usage
Authorization: Bearer <access token>
anthropic-beta: oauth-2025-04-20

GET https://api.anthropic.com/api/oauth/profile    (same headers, for plan)
```

Kept: `five_hour` and `seven_day` (`utilization`, `resets_at`); optionally
`seven_day_sonnet` / `seven_day_opus` (Q5); plan from `subscriptionType`,
else `rate_limit_tier`; `extra_usage` not in v1. The token is held in
memory for the two requests only, in a `secrecy::SecretString` (6.2), sent
only to `api.anthropic.com` (the host IronWire already binds the token to,
`claude.rs:26-27`), never refreshed, never written. An expired token
(`expiresAt` in the past) is `signed_out`, not refreshed. A token without
`user:profile` scope (a `401`/`403` from usage) is `unsupported_login`.

This sits next to "developers may not collect, store, or intermediate
Claude.ai credentials or session tokens". It is also the same kind of read
the embedded IronWire already performs today (Verified, "Our code").

**Recommendation.** Ship Codex first. Do not build Option B until the owner
has read the quoted text and decided, with legal if he wants it (Q2).
Build Option A only after the to-verify items above are confirmed and the
PTY dependency is approved (Q3). Until then the Claude rows say "Usage
unknown" (section 5), exactly as today, and Settings does not offer Claude
usage at all.

### 2.4 Failure handling

Every way of not knowing is a state with a label, never a number. Shown
states and their reasons (wire labels in 3.3):

| Reason | When | Shown as |
| --- | --- | --- |
| `off` | The provider's switch is off. | The row shows no usage line at all. |
| `codex_not_found` / `claude_not_found` | No CLI found by harness detection. | "Codex not found on this Mac" |
| `signed_out` | `account: null` with `requiresOpenaiAuth`; Claude token expired; CLI shows a sign-in prompt. | "Signed out of Codex" / "Signed out of Claude Code" |
| `not_subscription` | Codex signed in with an API key, not ChatGPT. | "Not a ChatGPT sign-in" |
| `unsupported_login` | Claude token without usage scope (B). | "Usage not available for this sign-in" |
| `cli_needs_attention` | The CLI asked to update, was blocked by macOS, or is quarantined. | "Codex needs attention. Open Codex once." |
| `timed_out` | A budget in 2.1 ran out. | "Usage unknown" |
| `unreadable` | A response did not parse, failed a range check, or a line was over the cap. | "Usage unknown" |
| `refused` | The Keychain denied (B), or the endpoint answered non-2xx other than the cases above. | "Usage unknown" |
| `stale` | The last good reading is older than 30 minutes. | "Usage unknown" (the reading is dropped from the row, kept in history) |

The endpoint or CLI changing shape lands in `unreadable`. No reason
carries upstream text. A field the parser does not know is ignored, as
`ironwire.rs:856-870` already tests for the status object; a field it
needs and does not find is that field's `unknown`.

### 2.5 Polling and backoff

- One probe per enabled login every 15 minutes while the daemon runs (the
  CodexBar local-history minimum; Q8).
- Opening Insights or the Private AI screen asks for a refresh, at most
  one per login per 60 seconds; otherwise it reads the held snapshot.
- Failure backoff per login: 15, 30, 60, then 120 minutes, reset by the
  first success. `cli_needs_attention` pauses that login for 30 minutes
  (CodexBar's figure) and a manual Refresh retries at once.
- At most one probe runs at a time, daemon-wide. A probe never runs while
  the daemon is quiescing (`quiesce`, `docs/contributor-daemon-ipc-v1_1.md:3751`).
- No probe runs during the developer dry run.

### 2.6 Logging

Label-only, per the repository rule. A probe logs at most: provider,
backend id (a label we or the contributor assigned, never a path),
reason label, duration in milliseconds, and the HTTP status for B. Never
a token, an email, an account id or its HMAC, a plan, a percentage, a
response body, a CLI's stdout or stderr, or a `config_dir` path. The
error from a failed parse is discarded, not logged, because serde quotes
what it choked on (the pattern at `ironwire.rs:664-668`).

## 3. Where it lives

### 3.1 A daemon module, not Swift

New module `crates/trace-commons-contributor/src/subscription_usage/`
(MIT OR Apache-2.0), with:

- `codex_app_server.rs` -- launch, exchange, parse (2.1).
- `claude_cli.rs` or `claude_oauth.rs` -- only once Q2/Q3 decide.
- `snapshot.rs` -- the held per-login snapshot, refreshed off the poll
  tick like `IronWireLedger::refresh_spend`, read synchronously by IPC.
- `history.rs` -- the bounded store (3.5).

Why the daemon: it already knows the logins (IronWire's status object and
the home's `config.toml`), already owns probes, settings and the
`unenroll` clearing path, and is what every shell talks to. Putting it in
Swift would make GTK and Windows re-implement it, and against rule 8 of
the copy guide the words would follow.

The provider probes sit behind a trait (`UsageSource`) so tests swap in
fakes (section 7) and so a later source -- an upstream IronWire hook, a
documented provider API -- replaces one implementation without touching
the IPC surface.

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
ours: that is the provider's own header on real traffic and needs no probe.
Today it is always `unknown`, because nothing is routed. The fixture at
`ironwire.rs:866` is corrected to upstream's `state` tag in the same
change.

### 3.3 IPC: `subscription_usage`

Additive. Synchronous and read-only: it answers from the held snapshot and
never launches a probe itself. Follows `insights_glance`.

Params: `refresh` (optional boolean). `true` schedules a probe for each
enabled login subject to the 60-second floor in 2.5 and returns the
current snapshot at once; the answer that follows arrives through the
existing `status_changed` event (no new event; the plan confirms that
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
      "updated_at": "2026-10-24T15:02:11+00:00",
      "stale": false,
      "plan": "plus",
      "limit_reached": false,
      "windows": [
        { "duration_mins": 300, "used_permille": 420, "resets_at": "2026-10-24T18:40:00+00:00" },
        { "duration_mins": 10080, "used_permille": 130, "resets_at": "2026-10-29T09:00:00+00:00" }
      ],
      "credits": { "has_credits": true, "unlimited": false, "balance": "12.50" },
      "source": "codex_app_server"
    },
    {
      "backend": "claude-sub",
      "provider": "claude",
      "default_login": true,
      "state": "unknown",
      "reason": "off",
      "updated_at": null,
      "stale": false,
      "plan": null,
      "limit_reached": null,
      "windows": [],
      "credits": null,
      "source": null
    }
  ]
}
```

- `logins[]` is one entry per IronWire backend whose `kind` is
  `subscription` and whose id maps to a Claude Code or Codex login, in
  fixed order: Claude logins, then Codex logins, each default login first,
  then by backend id. Never sorted by usage.
- `state` is `observed` or `unknown`. `reason` is a label from 2.4 when
  `unknown`, `null` when `observed`.
- `updated_at` is when the last good reading was taken, not now. `stale`
  is true past 30 minutes, and then `windows`, `plan`, `credits` and
  `limit_reached` are emptied in the answer (the shell never has to decide
  whether an old figure is still true).
- `used_permille` is an integer 0..=1000. A window the provider did not
  report is absent, not `0`.
- `plan` is a fixed label from the pinned plan list or `unknown`; it never
  carries free text. Display names live in the Rust copy table.
- `balance` is a decimal string as the provider gave it, present only if
  it parsed (2.1).
- Nothing on the wire is an email, account id, path, token, URL or
  upstream message.

Older shells ignore the method; a shell that gets `unknown_method` (an
older daemon) shows the rows exactly as today.

### 3.4 Settings

New key on `set_settings` / `get_settings`:

```json
{ "subscription_usage": { "codex": false, "claude": false } }
```

Both default `false`. A key the daemon does not know is kept on load and
save, as `notify` does. These are settings about this Mac, so `unenroll`
keeps them (the precedent at `docs/contributor-daemon-ipc-v1_1.md:5036`);
`unenroll` clears the history (3.5). `claude` is refused (`bad_params`, with a new reason label the plan
names) until a Claude source is built and approved (2.3).

### 3.5 On-device history for Insights

Insights wants to show how usage moved over the week. The daemon keeps:

- File `subscription_usage.jsonl` in the daemon state directory, mode
  0600, written atomically.
- One row per login per probe **only when the reading changed**:
  `backend`, `provider`, `observed_at`, and per window `duration_mins`,
  `used_permille`, `resets_at`. No plan, no credits, no account tag.
- Bounded: rows older than 8 days dropped on every write; at most 2,000
  rows overall, oldest dropped first.
- A header row holds each login's `account_tag` (2.1). When a probe sees
  a different tag for a login, that login's rows are deleted before the new
  row is written: a different account's history is not this account's.
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

Settings gets a section "Subscription usage" with one switch per provider:
"Codex usage" and, once built, "Claude usage". Each switch's description
says what turning it on does, before the click (copy guide rule 3; draft
copy in 5.3). Off by default. No first-run step, no offer card and no
nudge (rule 9).

### 4.2 What happens when Codex usage turns on

No macOS prompt. The daemon runs the first probe within a minute. If Codex
is not installed or not signed in, the row says so (2.4). If macOS has
quarantined `codex`, macOS may show its own dialog the first time; that is
macOS's, not ours.

Codex in managed sign-in mode may refresh its own token while the
app-server runs. That is Codex writing its own file, which it does
whenever the contributor uses it; we ask it to do nothing extra
(`refreshToken` is never sent). This is stated in the description.

### 4.3 The Keychain prompt (Option B only)

Option A never touches the Keychain: Claude Code reads its own item.

Option B, as IronWire does it, runs `/usr/bin/security`. On a Mac where
the item's access list does not already allow it, macOS shows its own
dialog: the requesting program is `security` and the item is
`Claude Code-credentials`, with **Deny**, **Allow** and **Always Allow**.
**Always Allow adds `security` -- not Trace Commons -- to the item's
access list**, after which any program running as the contributor can
read the item through `security` without a prompt. To be verified on a
test Mac with a test Claude account before anything is built; not
observed for this spec.

If Option B is chosen, this design requires:

- The read goes through the Security framework from the signed daemon
  (`security-framework` is already a dependency of
  `trace-commons-contributor-platform`, `Cargo.toml:50`), so the access
  list entry names our code signature, not `security`.
- The read happens only after the switch is turned on, at the first probe,
  and the description beside the switch says a macOS dialog will ask and
  that **Allow** is enough (draft copy, needs approval as it is beside an OS
  prompt).
- A denial is `refused`, the switch stays on, the row says "Usage
  unknown", and no further Keychain read happens until the contributor
  chooses **Refresh**, so the dialog is never repeated on a timer (the
  CodexBar default, "Only on user action").
- Raised separately for the owner: IronWire's existing
  `security`-based read for the `claude-sub` backend has the same
  property, today, whenever Private AI runs with a NEAR AI key (Q10).

### 4.4 Turning it off

Turning a switch off: no further probes for that provider, the held
snapshot for its logins is dropped, and its history rows are deleted at
once (3.5). Any child process for a running probe is killed and reaped.
Nothing at the provider changes: we never wrote anything there. An
"Always Allow" entry a contributor granted in Keychain Access stays until
they remove it; the description beside the switch says where (draft,
Option B only).

### 4.5 Other shells

Native macOS is the lead client (#1173). The daemon surface is
shell-neutral, so GTK and Windows can render the same rows later. On Linux
there is no Keychain and Option B reads only the file; on Windows the
Codex path is the same. Tauri gets nothing (frozen). None of this is in
the first delivery.

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
- One meter per window: label from the duration ("5-hour", "Weekly"),
  "{n}% used", and "Resets {relative time}". The meter is drawn only for
  `observed` windows. An `unknown` window is the words "Usage unknown",
  never an empty bar.
- Credits (Codex, if Q4 says show): "Credits available", "Unlimited
  credits", or "{balance} credits".
- "Updated {age} ago" under the meters.
- `limit_reached`: "Limit reached" in the row's status line. No
  notification (non-goal).
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
- No line divides tokens by the provider's percentage to estimate a limit
  ("never invent a number", the IronWire rule this design keeps).

### 5.2 Private AI

The Inference tab lists, under the existing NEAR AI connection card, one
read-only row per subscription login IronWire reports:

- Name: "ChatGPT subscription" / "Claude subscription" (IronWire's `name`
  is not shown; it is upstream prose).
- Status: "Not used by Private AI" while `consented` is false. This line
  is always shown so the usage beside it is never read as routing.
- Usage: the same meters and states as 5.1, compact: "5-hour {n}% ·
  Weekly {m}%", or "Usage unknown", or "Turn on usage in Settings" (no
  link styled as a switch; the button is **Review settings**, rule 2).
- No switch, no **Use** button. Making a row usable is section 10.

### 5.3 Draft copy

| Key | Draft text |
| --- | --- |
| `subscription_usage_section_title` | Subscription usage |
| `subscription_usage_codex_switch` | Codex usage |
| `subscription_usage_codex_detail` | Shows your ChatGPT plan and how much of each Codex limit you have used. Trace Commons asks the Codex app on this Mac; Codex keeps its own sign-in, and Trace Commons never reads it. Nothing leaves this Mac. |
| `subscription_usage_claude_switch` | Claude usage |
| `subscription_usage_claude_detail_cli` (Option A) | Shows your Claude plan and how much of each Claude Code limit you have used. Trace Commons asks Claude Code on this Mac; Claude Code keeps its own sign-in. Nothing leaves this Mac. |
| `subscription_usage_claude_detail_keychain` (Option B) | Shows your Claude plan and limits. Trace Commons reads Claude Code's sign-in from your keychain to ask Anthropic, and never saves or changes it. macOS will ask first; Allow is enough. |
| `subscription_usage_off_effect` | Turning this off deletes the usage history kept on this Mac. |
| `subscription_usage_card_title` | Subscriptions |
| `subscription_usage_window_5h` | 5-hour |
| `subscription_usage_window_weekly` | Weekly |
| `subscription_usage_window_hours` | {n}-hour |
| `subscription_usage_window_days` | {n}-day |
| `subscription_usage_window_unnamed` | Usage window |
| `subscription_usage_used` | {n}% used |
| `subscription_usage_resets` | Resets {relative} |
| `subscription_usage_updated` | Updated {age} ago |
| `subscription_usage_unknown` | Usage unknown |
| `subscription_usage_plan_unknown` | Plan unknown |
| `subscription_usage_limit_reached` | Limit reached |
| `subscription_usage_credits_available` | Credits available |
| `subscription_usage_credits_unlimited` | Unlimited credits |
| `subscription_usage_credits_balance` | {balance} credits |
| `subscription_usage_codex_not_found` | Codex not found on this Mac |
| `subscription_usage_claude_not_found` | Claude Code not found on this Mac |
| `subscription_usage_signed_out_codex` | Signed out of Codex |
| `subscription_usage_signed_out_claude` | Signed out of Claude Code |
| `subscription_usage_not_subscription` | Not a ChatGPT sign-in |
| `subscription_usage_unsupported_login` | Usage not available for this sign-in |
| `subscription_usage_needs_attention` | Codex needs attention. Open Codex once. |
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
| Codex plan, windows, credits, limit flag | Codex app-server stdout | Snapshot in memory; windows in 3.5 history | Never |
| Codex account id | Codex app-server stdout | HMAC tag only | Never |
| Codex email | Codex app-server stdout | Not deserialized | Never |
| Claude windows, plan (A) | Claude Code's `/usage` panel | As Codex | Never |
| Claude access token (B) | File or Keychain | Memory, for two requests | Only to `api.anthropic.com`, as the bearer for those two requests |
| Claude usage and profile JSON (B) | `api.anthropic.com` | As Codex; no email, no org id | Never |

"Never" is tested (7.3): the module has no network client except the one
Option B builds, bound to one host.

### 6.2 Threat model

- **A malicious process running as the contributor.** It can already read
  `~/.codex/auth.json` and `~/.claude/.credentials.json`, call `security`,
  and connect to our IPC socket. This design adds no file it could not
  already read: the history holds percentages and reset times. The IPC
  answer adds plan and usage to what the socket already exposes; the
  socket's existing authorization applies
  (`docs/contributor-daemon-ipc-v1_1.md:311`). With Option B, an "Always
  Allow" granted to `security` would widen the Keychain item's access to
  every such process; 4.3 avoids it by reading through the Security
  framework.
- **A planted `codex` or `claude` earlier on `PATH`.** We resolve an
  absolute path from harness detection and never search the daemon's
  `PATH`. A same-user attacker who replaces the real binary already owns
  the tool the contributor uses; out of scope.
- **Token in memory (B).** `secrecy::SecretString` (already a transitive
  dependency through IronWire's `ironwire_creds`; a direct dependency
  needs listing in 6.4), zeroed on drop, no `Debug`, never in an error
  value. Dropped as soon as the second request returns.
- **Token or identity in logs and crash reports.** 2.6. Panic messages
  and `Debug` of every probe type are tested not to contain a seeded
  token, email or account id (7.3). The child's stderr is never read, so
  a CLI that prints a token to stderr cannot reach our logs.
- **The child process.** Cleared environment, daemon-owned cwd, bounded
  time and output, killed and reaped on every path (the pattern of
  `ironwire_creds::claude::keychain_output`). The probe cwd is a Never
  folder to our own watcher (2.3).
- **A misread figure.** Range checks and `unknown` on any doubt (2.1,
  2.4). Display only, so a wrong figure misleads but routes nothing.
- **Provider endpoints and panels change.** Internal app-server methods,
  an undocumented OAuth endpoint and a TUI panel can all change without
  notice. Each change lands as `unreadable` -> "Usage unknown", which the
  contract fixtures pin (7.2).

### 6.3 Licensing

All new code is in `trace-commons-contributor`, MIT OR Apache-2.0. No
AGPL crate is involved; `license_boundary.rs` is unaffected. The Codex
path and Option B need no new dependency: `tokio` (with `process`),
`serde_json` and `reqwest` are already direct dependencies
(`crates/trace-commons-contributor/Cargo.toml:798`, `:809`).

### 6.4 Dependencies (Option A only, none approved)

Option A needs a pseudo-terminal. Two candidates; neither is added
without the owner's approval (Q3).

| Field | `libc` (direct, inline `openpty`) | `portable-pty` |
| --- | --- | --- |
| Version | 0.2.191 latest; 0.2.186 already in `Cargo.lock` | 0.9.0 |
| Purpose | `openpty`, `login_tty`-style setup in roughly 60-100 lines of our own `unsafe` | A cross-platform PTY API (Unix and Windows ConPTY) |
| Adoption (crates.io, 2026-10-24) | 1.84 billion downloads, 435 million recent | 23.2 million downloads, 13.7 million recent |
| New transitive crates | None: already in the tree | 13 normal non-optional dependencies, some new to this tree (`filedescriptor`, `serial2`, `shared_library`, `downcast-rs`, others) to be counted with `cargo tree` at proposal time |
| Maintenance | Released 2026-10-16 (rust-lang) | Last release 2025-02-11; lives in the `wezterm` monorepo |
| License | MIT OR Apache-2.0 | MIT |
| Advisories | Not checked for this spec; `cargo deny check advisories` at proposal time | Not checked for this spec; same |

Recommendation: `libc` direct, macOS and Linux only, since it adds nothing
to the tree and Windows has no Claude Code PTY need in v1. It is above the
~30-line inline threshold in the global dependency policy, so it still
needs explicit approval.

## 7. Testing

No test makes a live call to a provider, launches a real `codex` or
`claude`, or touches the real Keychain, in CI or locally.

### 7.1 Fakes

- **Fake app-server.** A small test binary (in the crate's `tests/` or a
  `[[bin]]` behind `cfg(test)` tooling) that speaks the JSONL exchange
  from a script: replies, delays, oversize lines, early exit, a sign-in
  prompt, garbage. The probe is pointed at it by absolute path, exactly as
  it is pointed at `codex`.
- **Fake Claude CLI** (Option A): a binary that writes recorded `/usage`
  panels to the PTY, including update and sign-in prompts.
- **Fake HTTP** (Option B): a loopback server with recorded bodies; the
  host binding is tested by asserting the client refuses any host but
  `api.anthropic.com` outside tests.
- **Fake Keychain** (Option B): the lookup is a parameter, as
  `ClaudeLogin::read_with` already does upstream.

### 7.2 Contract fixtures

- Recorded responses for `account/read` and `account/rateLimits/read`,
  generated from the TypeScript schema at the pinned `openai/codex`
  revision and checked in under
  `crates/trace-commons-contributor/tests/fixtures/subscription_usage/`
  with the revision in the file name. Every fixture field that a real
  account would make personal is a placeholder (`user@example.invalid`,
  `acct_example`).
- Each 2.4 reason has at least one fixture that produces it.
- A fixture with every field the parser does not keep set to a seeded
  marker, asserting the marker never appears in the IPC answer, the
  history file or captured logs.
- The plan records how a contributor regenerates a fixture from their own
  account without committing it.

### 7.3 Properties

- Never zero for unknown: every `unknown` answer has empty `windows`, no
  `used_permille: 0`.
- `refreshToken` never appears in any request the probe writes.
- Stale cut-off at 30 minutes drops figures from the answer.
- History bounds (8 days, 2,000 rows), account-tag change, switch-off and
  `unenroll` deletion, and no write after `unenroll` during a running probe.
- The probe's working directory is never offered as a session.
- Seeded token, email and account id never in `Debug`, `Display`, panic
  messages or logs.
- The child is reaped on every timeout path.

### 7.4 Shell

Swift decoder tests for the new payload, including an older daemon's
`unknown_method`, in `macos/` (`swift test`, the `macOS app tests` job).
Copy-contract tests for every new key. Per the repository gotcha, the
contributor crate's tests for any `subscription-usage` feature module
must run with `--features subscription-usage` (or `--workspace`), or a
green run proves nothing.

## 8. Rollout

1. **Build behind a feature.** Cargo feature `subscription-usage` on
   `trace-commons-contributor`, off by default. Without it `subscription_usage`
   is `unknown_method` and the setting is refused. The `cargo check
   (permissive crates, standalone)` job keeps the crate building without
   it.
2. **Staging.** The staging macOS build (staging environment design, S2)
   turns the feature on. Staging has its own IronWire home (S10), so the
   probe lists staging's backends. Internal testers turn on Codex usage
   and report: plan correct, percentages within a point of what Codex
   itself shows, resets correct, sign-out and not-installed states, no
   probe session in the queue, battery effect over a day.
3. **Production, off by default.** The release enables the feature; each
   switch stays off until the contributor turns it on.
4. **Claude** follows only after Q2 or Q3 is decided, as its own PR, with
   its own staging round.

Kill switch: the setting, per Mac. A release with the feature off removes
the surface entirely; nothing server-side exists to turn off.

## 9. Delivery

Not a plan; the order the plan should follow.

1. Correct the `headroom` fixture tag; deserialize IronWire's `headroom`
   (primary) beside `authenticated`.
2. `subscription_usage` module with the Codex app-server source, fakes and
   fixtures (no Claude).
3. Snapshot, poll, backoff; IPC method and setting; IPC doc section.
4. History store and deletion paths.
5. macOS Settings switch, Insights card, Private AI rows; Rust copy keys.
6. Staging round.
7. Claude source, after Q2/Q3.

## 10. Leaving room for routing

Routing a Private AI call through a subscription is a later owner decision,
and the quoted Anthropic sentence "route requests through Free, Pro, or
Max plan credentials on behalf of their users" bears on it directly for
Claude. This design keeps the way open without taking it:

- Everything is keyed by IronWire backend id, the same id IronWire routes
  by and records consent against.
- If routing is approved, IronWire fills `headroom` from response headers
  on routed traffic by itself; our probe becomes the reading for idle
  periods.
- An upstream `EmbedOptions` hook that accepts a host observation as
  `Headroom::Observed` (with the host's `observed_at`, so IronWire's
  age rules apply) would let our reading inform routing. That is an
  upstream change to propose then, not now.
- The Private AI rows are where a **Use** control would go. Its consent
  belongs to IronWire's existing per-login consent ledger
  (`ironwire_creds::consent`), not to the usage switch: turning on usage
  must never be read as consent to route.

## 11. Open questions and owner decisions

Each has a recommendation the owner may override.

- **Q1. Codex source.** App-server (2.1) or `auth.json` + `wham/usage`
  (2.2)? *Recommend the app-server only*: we never read the ChatGPT token.
  Revisit only if staging shows the app-server unreliable.
- **Q2. Claude via the usage endpoint (Option B)?** Given the quoted
  "may not collect, store, or intermediate Claude.ai credentials or
  session tokens", *recommend not building it unless the owner, with legal
  if he wishes, decides the read is acceptable*. Build nothing for Claude
  until Q2 or Q3 is decided.
- **Q3. Claude via the CLI (Option A)?** *Recommend yes, after the
  to-verify items in 2.3* (no model request, panel wording, non-PTY form)
  are confirmed on a test account, and with `libc` as a direct dependency
  (6.4) if a PTY is needed. It sits beside the "unmodified Claude Code
  binary" sentence; the owner may still want legal to look.
- **Q4. Show Codex credits?** *Recommend showing "Credits available" /
  "Unlimited credits", and the balance only once a staging tester confirms
  its unit*, since the schema gives it as a bare string.
- **Q5. Extra windows** (`rateLimitsByLimitId`, Claude's
  `seven_day_sonnet` / `seven_day_opus`, `extra_usage`)? *Recommend not in
  v1*; the two main windows answer the request.
- **Q6. OpenAI terms.** Not checked for this spec. *Recommend a read of
  the Codex and ChatGPT terms before production* (step 3 of section 8),
  since we launch their CLI on the user's behalf.
- **Q7. Browser cookies and dashboards.** *Recommend never*, for both
  providers.
- **Q8. Poll interval.** *Recommend 15 minutes* with the backoff in 2.5,
  plus refresh on opening the screens.
- **Q9. A "limit reached" or "resets soon" notification?** Not proposed.
  Rule 9 requires explicit approval for any notice. *Recommend none in
  v1.*
- **Q10. IronWire's existing Claude Keychain read.** The embedded proxy
  already reads `Claude Code-credentials` through `security` whenever
  Private AI runs with a NEAR AI key, to register `claude-sub`. *Recommend
  the owner put it in the same terms review as Q2*, and, separately,
  consider `CredentialFiles::Off` for Claude logins until a routing
  decision needs them -- a behaviour change for anyone who already
  consented, so it needs its own decision.
- **Q11. History on sign-out.** *Recommend deleting a login's history when
  a probe sees it signed out* (3.5), rather than keeping it for a
  re-sign-in.
- **Q12. Show plan names?** The plan labels (e.g. "ChatGPT Plus",
  "Claude Max 5x") name another company's product tiers. *Recommend
  plain-text names in the Rust copy table, no logos*, consistent with the
  name-use paragraph on the same Anthropic page.
