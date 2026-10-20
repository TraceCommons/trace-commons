# Trust, credentials and consent

IronWire sits at the point where every byte a developer's coding agent sends —
proprietary source, secrets in tracebacks, customer data in test fixtures — and
every credential that pays for it, pass through one process. This document is
the set of promises that position requires. They are **design invariants**, not
defaults: changing one is a product decision, not a config change.

---

## 1. Structural invariants

| # | Invariant | Enforcement |
|---|---|---|
| I1 | **Loopback only.** IronWire binds `127.0.0.1` and has no remote mode, no `--host`, no tunnel helper. | Hardcoded bind address; test asserts the listener is loopback. |
| I2 | **Credentials never leave the machine.** No credential, refresh token or derived bearer is written to any network destination other than the credential's own issuer. | Egress allowlist keyed by credential; test asserts a credential is only ever attached to its issuer's host. The signed catalog channel **may say what to set, never what to set it to** (§6), so I2 holds even against a compromised signing key. |
| I3 | **No hosted IronWire holds anyone's subscription token.** There is no server-side product that proxies a user's subscription. | Non-goal, stated in DESIGN.md §11. |
| I4 | **Single user.** No multi-tenant routing, no shared pools, no capacity resale. Several subscriptions held by that one user — a work and a personal Claude login — are neither tenancy nor a pool between people: there is still one person, and each login stays a backend of its own, serving its own client (I8). | No tenancy concept exists in the type system. A login other than the default exists only because the user named its directory in their own `config.toml` (§2); nothing scans for logins. |
| I5 | **No identity forgery.** IronWire never synthesizes another product's client identity to unlock a subscription. | See §3. |
| I6 | **Nothing is uploaded without a recorded, specific consent.** | See §4. |
| I7 | **IronWire never claims to protect data more than it does.** The optional privacy filter is described by what it is *doing*, never by what the user is *safe from*. | See §8 and `docs/PRIVACY.md` §7. |
| I8 | **One request, one login.** A request's bearer and every account-bound header come from the same login, and a client is served by its own login unless the user turned on cross-account fallback. | `usable()` refuses another login (`Refusal::AnotherLogin`) unless `[subscriptions] cross_account_fallback` is set, whatever that login's consent; the plain façade is the default login's only while it is in its default place (`Candidate::default_login`); `chatgpt-account-id` is stripped from every inbound request and attached from the token presented (`PROTOCOL.md` §2.2). Tests: `an_account_header_the_client_sends_never_reaches_any_provider`, `a_request_served_by_another_login_carries_that_logins_bearer_and_account_id`, `a_bound_codex_request_carries_its_own_homes_bearer_and_account`, `a_spent_login_falls_to_the_metered_key_not_to_the_other_subscription`, `a_pin_cannot_hand_a_client_another_login`, `a_bound_url_naming_no_such_login_is_refused_naming_the_fix`, `a_moved_default_login_serves_its_bound_url_and_never_the_plain_one`, `connecting_a_login_by_directory_declares_it_and_binds_only_its_claude_code`. See §2. |

I1–I4 are the line between *using capacity the user already pays for, on the
user's own machine, for the workload it was sold for* and *reselling
subscription capacity*. That line is not one we approach.

First-party upstream clients parse the configured URL before attaching a
credential: userinfo and foreign hosts are refused, and an issuer is reached
only over HTTPS. Loopback HTTP remains available for local conformance mocks.
These clients never follow redirects, including during model discovery and
probes. A redirect cannot carry a bearer, API key, account header or request
body to an unchecked destination
(`redirects_never_forward_a_logins_request_or_probe_to_another_origin`).

---

## 2. Subscription credential replay

IronWire reads credentials the official clients already store:

| Backend | Source | Auth |
|---|---|---|
| Claude subscription | macOS Keychain `Claude Code-credentials`, else `~/.claude/.credentials.json` | `Authorization: Bearer` + `anthropic-beta: oauth-2025-04-20` |
| Another Claude login (`config_dir = "~/.claude-work"`) | macOS Keychain `Claude Code-credentials-<8 hex>`, else `<dir>/.credentials.json` | the same |
| ChatGPT/Codex subscription | `~/.codex/auth.json` (`auth_mode: "chatgpt"`), or `$CODEX_HOME/auth.json` when the daemon's own environment sets it | `Authorization: Bearer` + that token's `chatgpt-account-id`, `chatgpt.com/backend-api/codex` |
| Another ChatGPT login (`config_dir = "~/.codex-work"`) | `<dir>/auth.json` (`auth_mode: "chatgpt"`) | the same |

These are undocumented, private surfaces. They can change without notice, and
using them from a third-party process may fall outside what the provider
intends. **The account at risk is the user's.** That fact must be presented
before the first request, not buried.

### Consent gate

Each subscription backend is **off until explicitly enabled**, once, in plain
language. `ironwire init` asks for every subscription it found, in one question
(prompt v2):

```
$ ironwire init

  IronWire can use the Claude and ChatGPT subscriptions above, by replaying
  the OAuth tokens Claude Code and Codex have already stored on this machine.
  Each token goes only to its own provider (api.anthropic.com and chatgpt.com),
  from this computer.

  · Anthropic and OpenAI do not document this authentication path and may
    change or block it at any time.
  · Using it from a third-party proxy may fall outside your
    subscription's intended use. If they object, it is your account
    that is affected.
  · You can use an Anthropic API key or an OpenAI API key instead —
    fully supported, no ambiguity.

  Use the Claude and ChatGPT subscriptions? [Y/n]
```

`ironwire connect <target> --subscription` asks the same question for one
backend, and remains the way to enable one later or to change your mind.

**Why one question, and why the default is yes.** The facts a user needs are
identical for both subscriptions, and asking twice teaches them that the second
prompt is a formality — which is the opposite of what a gate is for. The
default is yes because this is the only question `ironwire init` asks, it is
asked immediately after listing what was found, and declining is a complete
answer: the metered keys and local models found alongside still work, which is
what keeps "no" cheap enough to mean something. **This is a weaker default than
v1's `[y/N]`, and it is why `CONSENT_PROMPT_VERSION` is 2** — consent recorded
against v1 does not carry over, and every existing user is asked again.

The prompt is never shown when stdin is not a terminal. A script that cannot
answer gets nothing enabled, rather than a default that grants a credential.

Until the answer is yes, IronWire sends that token nowhere on its own
initiative either. The startup probe that learns a provider's model list skips
the login, and `POST /_ironwire/probe`, which `ironwire doctor` runs, reports
it unprobed with the command that grants consent. Running `doctor` is not
saying yes (`startup_discovery_never_sends_the_token_of_a_login_awaiting_consent`,
`a_probe_never_sends_the_token_of_a_login_awaiting_consent`,
`doctor_reports_a_login_awaiting_consent_once_and_not_as_a_failure`).

The answer is recorded with a timestamp, the exact prompt version and, for a
login outside its usual place, the directory it was given for, in
`$IRONWIRE_HOME/consent.json`. `ironwire status` always shows which backends are
running on subscription credentials. `ironwire disconnect claude --subscription`
revokes it — for the login the Claude Code in that shell uses, which in a shell
that sets `CLAUDE_CONFIG_DIR` is the login in that directory, not the default
(§2, "More than one login").

#### In the menu bar app

The GUI grants the same consent with a switch rather than a two-step gate, and
the difference is deliberate. The switch lives **on the backend's own row**:
before consent that row invites you to enable it, after consent it shows the
capacity the backend is reporting. There is no separate settings pane, because
"why is nothing routing here" and "turn this on" are the same question.

What it keeps: the daemon's wording, whole. The summary and every point are
carried on the switch itself — its tooltip and its VoiceOver hint — in the
daemon's order and unabridged, never reworded, never summarised, never reordered
so the cost reads last. Flipping the switch sends the answer with the
`prompt_version` the daemon served, so an answer can still be checked against
the question it answered. The app never hardcodes that number: at
`CONSENT_PROMPT_VERSION` 2 it records 2. A prompt that arrives incomplete
produces no switch at all.

**What it gives up, stated plainly.** A user can enable a subscription without
having read a word of the prompt. The CLI renders the whole question and waits
for an answer; this renders a switch, and the question is a hover away. That is
a weaker reading of I6 than the CLI's, and weaker than the two versions of this
app that came before it — the first drew the summary beside the switch, the
second put it behind a **"What you are taking on"** disclosure with a line of
orange text above it. Both are gone for the same reason, one step apart: a menu
bar dropdown is read at a glance, and a paragraph per backend is what nobody
reads at all. What remains is a control that says its own state and nothing
that repeats it.

So the honest statement of the GUI's guarantee is narrower than §2's opening
claim that the risk "must be presented before the first request": in the app it
is *available* before the first request, and recorded with its version either
way. If that trade ever looks wrong, the fix is to draw `consentText(_:)` on the
row instead of passing it to `.help` — it is one line of `consentSwitch(_:prompt:)`
in `MenuContent.swift`, pinned by `test_the_consent_text_is_never_drawn_on_the_row`
and `test_a_row_with_a_switch_does_not_repeat_what_the_switch_says`. The CLI's
flow is unchanged and remains the stronger gate.

`ConsentPromptView.isComplete` gates the switch: a prompt that arrived without
its summary or points produces no toggle at all, only the CLI command. A switch
is not offered for a question this build could not read.

`ironclaw_llm` emits a `tracing::warn!` at this point. A warning in a log is not
consent; this is.

### More than one login

Claude Code keeps one login per config directory, and Anthropic's own
documentation recommends exactly that for keeping a work and a personal account
apart: give each its own `CLAUDE_CONFIG_DIR`. Codex does the same per
`CODEX_HOME`. IronWire reads each login as a backend of its own — its own id,
consent record, quota, circuit breaker and ledger rows — and the default logins
keep the ids `claude-sub` and `codex-sub` forever, because everything already
recorded is keyed on them.

**Declared, never discovered.** A login other than the default exists for
IronWire only when `config.toml` names its directory; `ironwire connect claude
--config-dir DIR` writes that entry. Nothing scans the disk or the Keychain for
logins. A login the user did not name is a destination they did not choose, and
the Keychain is not a reliable list of them anyway: stale hash-suffixed items
accumulate there
([anthropics/claude-code#90527](https://github.com/anthropics/claude-code/issues/90527)).

**The directory is kept exactly as typed.** Claude Code names a login's Keychain
item after the exact `CLAUDE_CONFIG_DIR` string it was started with: `-` and the
first eight hex digits of SHA-256 over its NFC form, with no symlink resolution
and no trailing-slash trim. So `config_dir` is stored as written, with only a
leading `~/` expanded the way a shell expands it before Claude Code ever sees
it; a tidier, canonical path would name a different item. An explicitly
exported `~/.claude` is therefore *not* the default login's item. The documented
contract is only that Claude Code "keys the macOS Keychain entry to that
directory"; the derivation itself is read off Claude Code's binary and pinned by
its worked examples (`a_config_dir_keys_its_own_keychain_item_exactly_as_claude_code_does`,
`an_explicitly_exported_default_dir_is_not_the_default_logins_keychain_item`).

Filesystem aliases such as a trailing slash can name different Keychain
accounts while sharing one settings file. Credential reads never try a trimmed
alias, and `connect` refuses an alias of a declared login whose Keychain identity
differs, naming the exact `CLAUDE_CONFIG_DIR` to export instead
(`a_trailing_slash_dir_never_reads_another_logins_keychain_item`,
`a_shell_using_an_alias_of_a_declared_claude_directory_is_refused`).

On macOS, only a Keychain lookup reporting the item absent (`security` exit 44)
permits a credentials-file fallback. A locked store, failed command, timeout,
or malformed result refuses the login and asks the user to unlock the Keychain
and retry. An inaccessible active Keychain account must never silently become
the stale account left in a credentials file. The lookup has a five-second
deadline and a bounded output; tests pin absence, failure, and timeout separately.

**The daemon does not follow its own `CLAUDE_CONFIG_DIR`.** Under launchd or
systemd it has none of your shell's variables, so a daemon that followed one
would disagree with the CLI about which login is the default. The default
Claude login is always the unsuffixed Keychain item and
`~/.claude/.credentials.json`. That holds for what the daemon writes as well as
what it reads: the control API's connect for Claude Code — the menu bar app's —
wires the default login's own `~/.claude/settings.json`, never the directory a
`CLAUDE_CONFIG_DIR` in the daemon's environment names. A daemon started from a
work shell would otherwise point that work Claude Code at the default login's
subscription (`the_default_claude_login_never_follows_claude_config_dir`,
`a_control_api_connect_never_writes_the_plain_url_into_another_logins_directory`).
The CLI does see your shell, and turns a `CLAUDE_CONFIG_DIR` it finds into an
explicit `config_dir`: `ironwire connect claude` run there connects that
directory's login, and says so. `ironwire init` run there, with the directory
not yet declared, never points it at the plain URL — the default login's token
would then serve a Claude Code logged in as someone else — and does not claim
the `claude` that shell runs goes through IronWire; it names `ironwire connect
claude` as the next step
(`init_with_an_undeclared_claude_config_dir_wires_only_the_default_login`).
`ironwire doctor` there, with no `~/.claude` on the machine, names the file it
checked and that same command rather than `ironwire init`, which would leave
the shell's Claude Code as it was
(`doctor_points_a_shell_with_only_an_undeclared_claude_config_dir_at_connect_not_init`).
The default Codex login still
honours a `CODEX_HOME` in the daemon's own environment, unchanged, because
existing setups depend on it.

**The plain URL belongs to the default login in its default place.** A client
says which login it is by the URL it was pointed at: `ironwire connect`
writes `…/anthropic/account/claude-work` into that login's `settings.json`, and
a client at the plain `…/anthropic` or `…/openai/v1` is the one that kept the
default directory. So moving the default — `config_dir` on `claude-sub` or
`codex-sub` — moves it away from the plain URL too. The moved login is reached
only at its bound URL (`/anthropic/account/claude-sub`), which is what
`connect` wires it with, and a client still at the plain URL is served by no
subscription: the ladder without one, or a refusal naming the command that binds
it — never the moved login's token, which belongs to a different Claude Code's
account (`a_moved_default_login_serves_its_bound_url_and_never_the_plain_one`,
`a_moved_default_codex_login_serves_only_its_bound_url`). When `connect` moves
the default and the original client's file still points at the plain URL, it
says so and names the command that takes that client off, leaving the file for
the user to decide
(`moving_the_default_login_warns_about_the_client_still_pointed_at_the_plain_url`);
`ironwire doctor` flags the same state, for either product
(`doctor_flags_each_client_a_moved_default_login_left_on_the_plain_url`). A
`config_dir` that names the product's default directory itself — `~/.claude`,
`~/.codex` — is the default moved there, never a second login: `connect
--config-dir ~/.claude` moves `claude-sub`, and an additional entry for it
fails at load, saying to put `config_dir` on `claude-sub` or `codex-sub`
instead, because two backends would otherwise read one login
(`naming_the_default_directory_moves_the_default_login_rather_than_adding_one`,
`an_additional_login_in_the_default_directory_is_refused_in_favour_of_relocating_the_default`).
On macOS that move has a cost no URL can avoid: Claude Code run with
`CLAUDE_CONFIG_DIR=~/.claude` and plain `claude` keep different Keychain items
but read one `settings.json`, so whichever URL that file holds serves both, and
a plain `claude` is served by the moved login from then on. `connect` says so
before it writes anything, and `init` run in such a shell names connecting it
as the next step rather than claiming it is done; on Linux both read
`~/.claude/.credentials.json`, are one login, and nothing needs saying
(`moving_the_default_login_into_its_own_directory_says_plain_claude_shares_it_on_macos`,
`init_with_claude_config_dir_set_to_the_default_directory_says_one_thing_about_claude`).
The menu bar app's connect writes `~/.claude/settings.json` and Codex's own
`config.toml`, so it refuses to put the plain URL there when config has moved a
login into that very directory, naming the `connect --config-dir` command that
wires it with its bound URL; a file already bound stays bound
(`a_control_api_connect_never_writes_the_plain_url_where_a_moved_login_reads`).

**What is not read.** `CLAUDE_CODE_CUSTOM_OAUTH_URL` (a non-production
endpoint, which renames the item) and `CLAUDE_SECURESTORAGE_CONFIG_DIR`
(undocumented; it separates the credential store from the config directory) are
not followed. A Codex home in keyring mode (`cli_auth_credentials_store =
"keyring"` or `"auto"`) has no `auth.json`, and IronWire does not read Codex's
keyring item; the not-logged-in message says to set it to `"file"` and log in
again.

**No login is ever refreshed.** Refreshing rotates the refresh token out from
under the client that owns it, and writing the result back would be writing
another product's credential store. So an idle login's token goes stale — the
usual state of a second login — and a token past its own stated expiry is
refused before anything is sent, with the one command that fixes it
(`CLAUDE_CONFIG_DIR=~/.claude-work claude`, `CODEX_HOME=~/.codex-work codex
login`).

**Consent is per login.** Each login has its own record in `consent.json`, under
its own id, and saying yes to one says nothing about another
(`consent_for_a_second_login_leaves_the_first_alone`). A login with a directory
is asked about by that directory, because with two Claude logins "enable the
Claude subscription" does not say which account is being handed over. The four
points are the default login's, unchanged. `ironwire connect claude --config-dir
~/.claude-work --subscription` asks:

```
  IronWire will read the OAuth token that Claude Code stores for the login
  in ~/.claude-work and send requests to api.anthropic.com with it, from
  this computer only.

  · This uses a private authentication path. Anthropic does not document
    it and may change or block it at any time.
  · Using it from a third-party proxy may fall outside your subscription's
    intended use. If Anthropic objects, it is your account that is
    affected.
  · Your token is never sent anywhere except api.anthropic.com.
  · You can use an Anthropic API key instead — fully supported, no
    ambiguity.

  Enable the Claude subscription in ~/.claude-work? [y/N]
```

`ironwire disconnect claude --subscription` with the same `--config-dir`, or
with `--id claude-work`, withdraws it. In a shell that sets `CLAUDE_CONFIG_DIR`,
plain `ironwire disconnect claude --subscription` means that directory's login,
exactly as `connect` does there. When the default login still holds consent
afterwards — a yes given from that same shell before logins were told apart
was recorded as `claude-sub` — it says so and names the command that withdraws
it, `ironwire disconnect claude --id claude-sub --subscription`, whether the
shell's login had a record to revoke or not
(`a_shell_login_with_no_consent_names_the_default_login_that_still_has_it`,
`withdrawing_a_shell_login_that_had_consent_still_names_the_default_that_keeps_it`). The daemon serves each
login's question on `GET /_ironwire/settings`, which is where the menu bar app's
switch takes its words from.

**Consent answers one directory.** The backend id alone does not say which
account was handed over: `config_dir` on `claude-sub` keeps the id and changes
the account. So a record names the directory of the login it was given for — the
expanded string the backend reads, which is the one Claude Code's Keychain hash
is taken over — and counts only while the backend still reads that directory.
Moving a login, default or not, withdraws its consent until it is asked about
again by its new directory
(`moving_claude_sub_to_another_directory_withdraws_its_consent`,
`an_additional_logins_consent_does_not_survive_its_config_dir_changing`,
`moving_a_login_withdraws_its_consent_until_its_new_directory_is_asked_about`).
One move does not: the daemon's own `CODEX_HOME` still selects the default
Codex login, a compatibility path older than logins with directories, and
changing it does not withdraw `codex-sub`'s consent — put that home in
`config_dir` on `codex-sub` to have its consent bound to it.
The default login in its usual place records no directory, so every
`consent.json` written before this keeps answering exactly what it answered and
is written back byte for byte
(`an_old_consent_file_still_grants_the_unrelocated_default`,
`a_ledger_this_version_writes_is_still_readable_by_the_old_schema`). Routing,
the control API's `consented` flags, its balance and the façades' model lists
all ask that one question, and `POST /_ironwire/consent` records the directory
the backend reads only after the grant echoes the prompt's opaque `binding`.
That binding names its product, backend id, version and actual credential
source directory, including the default Codex home selected by `CODEX_HOME`.
A prompt fetched before a daemon restart cannot authorize a login relocated
during that restart. Missing bindings refuse grants and ask the client
to refresh settings. The menu bar offers a grant switch only when that binding
is present, while a consented row can still withdraw using an older prompt.
The legacy ledger compatibility for `CODEX_HOME` above does not weaken this
check on a new control-API grant. Withdrawing is not bound to a directory:
`disconnect` removes the id's record whichever login it was given for
(`withdrawing_consent_removes_it_whichever_login_it_was_given_for`), because a
user taking consent back must never be told it belonged to somewhere else.

**Why this did not bump `CONSENT_PROMPT_VERSION`.** The version exists so that
an answer is never read as an answer to a question it was not asked. The
additional-login wording is new, but nothing can have answered an older form of
it: until it existed, no prompt was served for any id but `claude-sub` and
`codex-sub`, and the control API refused to record consent for one. Every
recorded answer still answers the question it was given, and bumping would have
withdrawn every default login's consent to protect nothing. The same change
corrected the Codex prompt's "the OAuth token that Codex Code stores" to
"Codex": a misspelled product, not a different question. Binding a record to its
directory needed no bump either: it narrows what an existing yes covers to what
it was actually given for, and reinterprets nothing.

**One weaker case, stated so it is not discovered.** A build from before
consent named a directory ignores the field. Going back to one after saying yes
to a moved `claude-sub` or `codex-sub` would read that yes as consent for the
login in the default directory, which it never answered. Withdrawing the moved
login's consent before downgrading (`ironwire disconnect claude --id
claude-sub --subscription`, or `codex` and `codex-sub`) closes it; a record for
a default login left in place is unaffected.

### Cross-account fallback

By default a client is served by its own login and no other. The Claude Code
using `~/.claude-work` is served by that login's subscription; under sustained
quota pressure, or once it is spent, that Claude Code descends the ladder
exactly as it would with one login — an API key, NEAR AI, a local model — and
never lands on the user's personal subscription
(`a_spent_login_falls_to_the_metered_key_not_to_the_other_subscription`). If
nothing else is left, it is told its own subscription is spent, as a 429 its
back-off already handles — whether or not the other login was ever consented
to, since it was never this client's to use
(`a_spent_login_beside_an_unconsented_one_is_still_told_it_is_spent`). A client
with no login of its own at all — a Claude Code on the plain URL when the only
Claude login is another one — is told which login exists and the command that
binds it, as a 503
(`a_client_on_the_wrong_url_is_told_the_command_that_binds_it`).

```toml
[subscriptions]
cross_account_fallback = true
```

lifts that: a client whose own subscription is under sustained quota pressure —
at least 90% used as the provider reports it, and for a conversation already on
it, for longer than the twenty-second descent debounce — or is spent, or
otherwise cannot serve it, may be served by another of the user's subscriptions
to the same product, at rung 2 and ahead of a metered key. That is the trigger every rung of the ladder uses. It is not
reserved for a subscription that has run out: with the opt-in on, a client at
92% of its own plan moves to a sibling that is not under pressure
(`with_cross_account_fallback_sustained_pressure_moves_the_client_off_its_own_login`).
It is off unless the user writes that line, the control API reports it on every
status snapshot so `ironwire status` can say so for as long as it is on, and
every request served by a login other than the client's own is recorded at rung
2, whatever else was or was not available, so the ledger and `status` show each
one (`a_request_served_by_another_login_is_always_recorded_as_rung_two`). The
exception is a pin or `X-IronWire-Route` naming another login: the user chose
it, so it is recorded at rung 0. A pin still reaches another login only with the
opt-in on (`a_pin_cannot_hand_a_client_another_login`), and never a subscription
whose consent was not given
(`a_pin_cannot_reach_a_subscription_nobody_consented_to`).

It is off by default for three reasons, any one of which would be enough.

**The terms.** Holding several plans is not the problem: Anthropic documents
separate logins for work and personal use, and we found no text in either
provider's terms that bars one person from holding more than one. Moving to
another account *because* the first hit its limit is the problem. OpenAI's Terms
of Use forbid using the services to "circumvent any rate limits or
restrictions". Anthropic's Usage Policy (effective 15 September 2025) forbids
attempts to "intentionally bypass capabilities, restrictions, or guardrails
established within our products", and the Claude Code legal page says
advertised Pro and Max limits "assume ordinary, individual usage". Switching
accounts after a 429 is the most direct reading of all three. That is a reading,
not legal advice — but it is the reading that sets the default, and taking the
other one is the user's decision, not IronWire's.

**The provider can see it.** The native lane forwards bytes (`PROTOCOL.md` §2),
and Claude Code's request body names the account it is logged in as:
`metadata.user_id` carries the login's account UUID and the device id. A request
moved to login B arrives on B's token naming A — to Anthropic, one device and
one session on two accounts, which is what account sharing looks like. IronWire
does not rewrite it: that would be a body mutation the native lane does not
make, and editing a client's statement of who it is would be I5 by another
name. Codex sends no account id to a custom provider, and the one IronWire
attaches is always the presented token's own (I8), but Codex's per-home
installation id and its session id are forwarded unchanged.

**Codex encrypted replay stays on a known origin.** Codex requests encrypted
reasoning and replays it on later turns; OpenAI rejects encrypted content from
another organization (`invalid_encrypted_content`). The eligibility gate
therefore requires both the recorded native Responses backend and a matching
known scope. For a ChatGPT subscription, scope is a hash of the provider's
account id, stable across token refresh. For an API key, scope is a separately
namespaced hash of the exact credential, which identifies that key without
asserting its organization. Raw credentials never enter origin records.
An API-key rotation can refuse continuation even if both keys happen to belong
to one organization: IronWire has no observation establishing that equivalence.

The sender checks that same scope against the fresh credential before sending,
so relogging a directory between selection and send cannot move reasoning to a
new account. The record survives provider failures and route retries, and pins
cannot bypass it. Missing subscription account identity, restart, expiry or
origin eviction refuses native encrypted replay instead of assuming the
client's own login. A first turn without encrypted state can establish a new
origin, and translation may cross families at a turn boundary where it drops
the private reasoning. See `PROTOCOL.md` §6 for tests and state limits.

Session affinity includes protocol, the client's valid session header and its
login. Concurrent sessions no longer share a sticky route merely because their
instructions and tools match. Clients without a session header retain the
content-derived fallback, which cannot distinguish identical conversations.
The two-real-account and subscription-to-API provider checks remain live
acceptance work; local enforcement does not prove provider behavior.

What the opt-in does not change: the product half of the identity check (a
Codex request never reaches a Claude subscription,
`cross_account_fallback_never_lets_another_product_in`); consent (an unconsented
login is never used); and the order among logins equally free of pressure, in
which a client's own login comes first and the others follow in the order they
were registered — the default login, then additional ones as `config.toml` lists
them. Pressure outranks that order, as it outranks cost. The routing half of I8
is pinned in `ironwire_core::policy`'s tests; the wire half in
`tests/several_logins.rs`, `tests/codex_on_subscription.rs` and
`tests/passthrough.rs`, and in `ironwire_upstream/tests/subscription_logins.rs`.

Sources: [Claude Code authentication, "Log in with multiple
accounts"](https://code.claude.com/docs/en/authentication); [Anthropic Usage
Policy](https://www.anthropic.com/legal/aup); [Claude Code legal and
compliance](https://code.claude.com/docs/en/legal-and-compliance); [OpenAI Terms
of Use](https://openai.com/policies/terms-of-use), whose wording was checked
against archived copies because openai.com refused the fetch; the
`invalid_encrypted_content` rejection as [reported by a Codex
user](https://community.openai.com/t/codex-app-cannot-resume-session-after-api-key-switch-encrypted-content-organization-id-mismatch/1374803),
not yet reproduced here (`ROADMAP.md`).

---

## 3. No identity forgery (I5)

Anthropic's OAuth path expects Claude Code's identifying first system block.
That creates a fork with only one acceptable branch:

- ❌ **Inject it for every client.** Aider hitting our Anthropic façade would be
  dressed up as Claude Code to unlock the subscription. This is impersonation of
  another product to a provider, and no framing makes it not that.
- ✅ **Require it.** The subscription backend is eligible only for requests that
  *already* carry that identity — i.e. actual Claude Code. Everything else
  routes to an API key, NEAR AI, or a local model.

The same rule applies to Codex and the ChatGPT backend.

This is a hard eligibility rule in `ironwire_core::policy`, not a default.
It costs us the "point Aider at localhost and use your Claude Max" feature. That
feature was never ours to sell.

### The identity is a *product*, not a boolean

The check names which client a request carries and which client a backend
belongs to, and requires them to match. It was once a single boolean — "does
this request carry some product's identity" — which is a different question with
the same shape, and it gave the wrong answer in one direction.

Claude Code carries a real identity: its own. So a Claude Code request satisfied
the *ChatGPT* subscription's requirement, and a turn falling back across wires
would have reached `chatgpt.com` wearing Claude Code's name — impersonation of
one product to another provider, which is exactly what I5 forbids. It was
unreachable only for as long as no translator went that way; adding the full
translation matrix (`docs/TRANSLATION.md`) made it reachable, and the check was
corrected in the same change rather than after somebody found it.

Verifiable against the real providers: `crates/ironwire_proxy/tests/live_subscriptions.rs`
asserts each subscription serves its own client and refuses the other's. It is
opt-in, it reads consent rather than granting it, and it forges nothing.

### …and a login

With several logins of one product registered, the identity a subscription
requires is also a *login*: the URL a client was pointed at says which of the
user's logins it is (`DESIGN.md` §5), and a subscription serves that login's
client only. The plain URL is the default login's while that login is in its
default place, and nobody's once it is moved (§2). Cross-account fallback
relaxes the login half and never the product half (I8, §2).

---

## 4. Traces

### Defaults

- **Local capture: on.** Metadata only — timing, model, route, rung, attempts,
  status, the token counts the *provider* reported, and the session id the agent
  put in its own request header. Bodies require `capture.bodies = true`, and so
  do the body digests that go with them — a digest is derived from a body we
  would have had to hold, so it is not a way to record less. Even with it on,
  only the *current* exchange of each session is held: the next turn releases
  the previous one's bodies. Visible via `ironwire log`.
- **The session id is read, never added.** It is a header the client already
  sends and IronWire already forwards; recording it changes nothing about what
  reaches the provider, and it stays on this machine like every other row.
- **No fabricated numbers in the ledger either.** An exchange whose usage the
  provider never reported is stored as unknown and rendered as `—`, not as
  zero: a fabricated zero would silently understate what the user spent.
- **Upload: off.** Not "off until onboarding asks"; off until the user runs an
  explicit command, per scope.

### Why local-first is the right order

The trace ledger has to be worth having for a user who will *never* share
anything: `ironwire log`, `ironwire replay`, per-project cost attribution, "what
did my agent actually send before it did that". If the feature only pays off
when uploaded, the incentive is to nudge users into uploading, and that is how
trust in this position gets spent.

### Upload path

Contribution is delegated wholesale to `ironclaw_trace_commons`, which already
owns:

- deterministic redaction before anything leaves the process (its
  security-critical obligation, with tests pinning the *absence* of raw values),
- standing consent policy and preflight accept/reject,
- an on-disk queue with manual-review holds for high residual-PII-risk
  envelopes,
- classification, allowed-uses and retention labelling,
- credit claims and device-key onboarding.

IronWire does not implement its own redaction. Adding a field to an uploaded
envelope is a change in that crate, under its rules.

### The training question, stated plainly

Both major providers' terms restrict using their model outputs to train
competing models. Traces of a user's own subscription sessions, uploaded by that
user's explicit choice, are the user's decision and the user's terms to honor.

IronWire's design does not depend on that reading holding. The trace corpus is
justified first by evaluation, routing quality and user-facing analytics. If the
training use turns out to be unavailable, the product still works and the ledger
is still worth having. That is deliberate: a product whose economics *require* a
contested legal reading is a product with a single point of failure that
engineering cannot fix.

---

## 5. Handling of secrets in the process

- Tokens are held in `secrecy::SecretString`; no `Debug` impl exposes them.
- No credential is ever written to a log, a trace record, or the control API —
  `status` reports *which* credential is in use, never its value.
- The control API is loopback-only and additionally requires a token from
  `$IRONWIRE_HOME/control.token` (mode 0600), so another local user cannot read
  the ledger or drive routing.
- Request/response bodies, when captured, are stored under `$IRONWIRE_HOME` with
  0700 directory permissions, and are excluded from any `ironwire report`
  bundle unless `--include-bodies` is passed. Each session holds at most one
  exchange's bodies at a time — the next turn releases the last — and what
  survives is removed by the same retention setting that prunes the ledger rows
  it belongs to. Files a crash left unreferenced are swept at daemon start.

---

## 6. Updates and the catalog channel

Full design in [`UPDATES.md`](./UPDATES.md); the trust-relevant parts:

**The daemon never updates itself.** It holds credentials in the middle of a
streamed response, and a self-update is arbitrary code execution as the user
with access to their subscription tokens. `ironwire update` reports and prints
the command that belongs to the user's install; it never acts.

**The menu bar app may.** It is a separate artifact and a pure client: it holds
no credentials, carries no traffic, and can be replaced while the daemon keeps
serving. The mid-stream argument above is about the daemon and does not reach
it. Two of the three reasons behind notify-only still do, so an app updater is
bounded by them rather than exempt from them:

- It runs **unsandboxed as the user** (`macos/README.md` explains why), so it can
  read `~/.claude/.credentials.json` and `$IRONWIRE_HOME/control.token`. A
  hijacked update channel is therefore still credential access, and the updater
  must be Developer ID signed, notarised, and its appcast signed with a key held
  in release infrastructure — not in this repository.
- If a package manager owns the install, self-updating desyncs it. A Homebrew
  cask must declare `auto_updates true` so the two do not fight.

**If the app ever carries the daemon binary, the daemon's rule wins.** Bundling
the two into one artifact means an app update is also a daemon update, and the
mid-stream argument returns in full. The only acceptable form is the one
`UPDATES.md` §3 already names: stage the new binary, and swap it when the daemon
has no in-flight stream — or at next login. Never under a live response.

**The update check is the only request IronWire makes that is not the user's own
work.** It is bounded accordingly:

- At most once every 24 hours, cached on disk.
- Never blocks startup.
- **No install id and no per-check identifier.** The request carries a version
  string and an OS name, and nothing else. There is no way to correlate two
  checks as coming from the same machine beyond what any HTTP request reveals.
- Off with `updates.check = false`, honoured before the first check — switching
  it off means no request is ever made, not one last one.

**The catalog channel refreshes provider values without a release** — an
`anthropic-beta` flag, an API version, a client-identity marker. It is signed,
rollback-protected, and fails closed onto the values compiled into the binary.

The constraint that makes a remotely-updatable document acceptable in a
credential-holding proxy is structural, not procedural:

> **The catalog may say what to set, never what to set it to.**

Whoever holds the signing key can change which beta flag we send. They cannot
change where we send the token.

This was originally the stronger-sounding "no type in the schema can express a
host, a URL, or a filesystem path", enforced by banning field *names* containing
`url`, `host` or `path`. That was a proxy for the real property, and it had to
go when the catalog began describing where a tool keeps its config. What
replaced it is narrower in what it forbids and stricter about what matters:

- **A value for a location is unrepresentable.** A catalog entry points a tool's
  config key at one of *our* façades by naming it — `anthropic` or `open_ai` —
  and the scheme, host and port come from the running daemon. It also picks
  which spelling of that façade's address the tool is handed — the origin form
  `…/anthropic` or the versioned form `…/openai/v1`, because both conventions
  are in use among the tools worth describing — but that suffix is compiled in
  beside the façade path and the set of spellings is closed. There is no variant
  that carries a string, so "point Claude Code at evil.example" cannot be
  written down, let alone signed.
- **A location is constrained, not free.** A tool's config is a dotdir under the
  user's home plus a `.json` or `.toml` file. `.` and `..` are refused,
  separators are outside the permitted charset, and the extension requirement is
  what rules out `~/.ssh/config` and `~/.aws/credentials`. The worst a
  compromised key achieves is writing the user's own loopback URL into another
  of their dotfiles.
- **The provider constants still name nothing**, and the original name-walk test
  still runs over that part of the document.

Adding a field that carries a value for a location, or widening where a config
may live, is a change to *this* document, not to a schema.

---

## 7. What we ask providers for

The end state is explicit support: a sanctioned way for a user to authorize a
local router against their own subscription, with a documented endpoint and a
client identity of its own. Until then IronWire is careful, local, honest with
the user about the risk, and does not build a business on someone else's
ambiguity.

---

## 8. The privacy filter (I7)

The filter (`docs/PRIVACY.md`) is optional and off by default. Three things
about it are trust commitments rather than implementation details.

**It is a mutation, and mutations are opt-in.** Everything else in this document
rests on IronWire forwarding bytes it did not change (`PROTOCOL.md` §2). The
filter changes the request body and the response stream by design. So it is
never on unless the user turned it on, `ironwire status` says so permanently
while it is, and the ledger marks every exchange it touched — a filtered
exchange is not comparable to an unfiltered one and the log must not imply it is.

**It never claims safety.** The mechanism has a false-negative rate we cannot
measure on the user's own data. The interface therefore states what is *running*
("secrets + named values"), never what the user is *protected from*. No
compliance language, no green checkmark, no "protected" badge. A privacy tool
that manufactures confidence is worse than no privacy tool, because it changes
what people are willing to paste into an agent.

**Nothing about it leaves the machine.** Tier 3 uses a local model and makes no
network call — asserted by a test, because a privacy feature that phones out is
the worst available outcome and exactly the kind of thing that arrives through a
dependency. The substitution map is derived per request and never written to
disk (`PRIVACY.md` §4): a persistent map would be a purpose-built plaintext PII
database created in the course of trying not to expose PII.


## 9. Explicit admission binding

The local control bearer may register an account-bound challenge for one exact
session/backend for at most 15 minutes. This is separate consent to a metadata
mutation, not consent to capture bodies, export them, obtain credit, or fund
inference. Registrations live only in bounded memory. Other sessions are
unchanged, and no subscription backend receives added client identity.
Conflicting metadata, expiry, wrong protocol, or routing to a different backend
refuses the selected session. Revocation stops future insertions; it cannot
recall requests already forwarded. `tests/passthrough.rs` and the admission
binding unit/control tests pin these properties and byte/capture identity.

Session headers are local assertions, not authentication. Any local process able
to reach the facade can claim a registered session identifier and make calls
bearing that account's binding. The control bearer authorizes registration, not
each inference request; loopback binding does not isolate users or processes.
Do not treat this feature as protection against a hostile local process.

The provider sees the canonical account anchor and challenge nonce in metadata.
Privacy substitution runs before insertion, so it does not hide this binding.
One registration can bind several inference calls until expiry; it is not a
local one-shot token. Trace Commons consumes a challenge once at admission,
independently of the proxy's repeated use, and only the server can grant credit.

Expired payloads release active slots, but bounded tombstones keep those exact
sessions refusing until authenticated renewal or revocation. At 1024 tombstones,
new capacity is refused rather than evicting protected sessions or blocking
unrelated traffic. `GET /_ironwire/admission-binding?session_id=...` requires the
control bearer and returns only that session's active/expired/inactive status
and expiry; it does not list session identifiers or return binding values.
Register again to renew, or DELETE that exact session to revoke. State is
memory-only and a proxy restart clears registrations and tombstones.

## Token-distribution evidence capture

`capture.token_capture` is a separate, off-by-default opt-in for explicitly
named backend/model pairs. It records verbatim evidence in a separate bounded
spool; it does not change the rolling retention of ordinary body capture.
Default limits are 512 MiB and three days; leases can retain a selected snapshot
for at most seven days from capture. A full spool stops new capture, not inference.
Only authenticated control clients can acquire/read/release exact snapshots.
One destination releasing its lease does not release another destination's lease
or a later exchange. No lease names a user filesystem path. Contribution approval
and durable server acknowledgment are responsibilities of the consuming client;
local capture itself never grants upload consent.

The detailed spool also enforces a 64 MiB per-session ceiling (or the smaller
configured global ceiling). Exact request/response digest lookup searches all
retained captures and refuses ambiguous matches; the bounded display listing
is not used as a complete session index. Missing-capture counters are persisted
as bounded reason labels, separately from raw evidence.

On Windows, owned state and the detailed spool receive protected current-user
ACLs. Foreign-owned paths and reparse ancestors are refused. Fresh payloads
inherit the verified spool ACL; status and control never depend on a broadly
readable custom home. The Windows spool CI leg checks raw-file ACLs.
