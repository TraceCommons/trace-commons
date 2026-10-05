# Saved model accounts and per-tool connection switching

Status: proposed specification, awaiting written-spec review.

## Intent and approved product direction

Let a contributor switch Claude Code and Codex between NEAR AI, their native
subscription, and an API key, and retain multiple named subscription accounts
for later use. The user confirmed multiple saved accounts and approved the
proposed UI and the rule that switching applies to newly opened sessions.
The user additionally authorized changes to IronWire as needed for this feature.

Each tool has an independent selected connection and account. Subscription
sign-in and refresh remain owned by the native tool. A running session keeps
its original account. A saved account is local to this computer; it is separate
from the contributor's Trace Commons identity.

The implementation is not authorized by this document's existence. The next
stage is written-spec review, followed by an implementation plan.

## Existing implementation and boundaries

The source baseline is `cf89adb3b`. The contributor pins IronWire to
`6c5d62897a9018b75bb306de9074548899e9642c`.

- `macos/Sources/TraceCommonsApp/Views/PrivateInferenceView.swift` renders
  Model calls and the per-tool harness list. Equivalent destinations exist in
  the Windows and GTK shells.
- `crates/trace-commons-contributor/src/daemon/harness.rs` separates a proposed
  connect/disconnect edit from commit. Its existing conflict, unreadable-file,
  and stale-preimage checks must survive this feature.
- `crates/trace-commons-contributor/src/daemon/private_inference.rs` owns the
  embedded listener. Listener health alone cannot establish which account a
  tool uses.
- At this pin, IronWire's Claude credential discovery uses the default
  `.claude/.credentials.json` or a fixed macOS Keychain service. Its Codex
  credential reader honors `CODEX_HOME`. Neither fact establishes a complete
  saved-account switching implementation.

Do not implement the UI as a cosmetic selector over automatic backend
discovery. The selected tool, connection, and account must determine the
effective session configuration.

## Approach and alternatives

Use isolated, native-tool profiles with a Trace Commons launcher. Each saved
subscription account owns its native configuration directory and credential
namespace. Trace Commons stores a label and an opaque profile reference, and
asks the tool to sign in within that profile. The native tool refreshes its
own credentials in place.

This avoids copying rotating subscription credentials into a shared login
slot. Such copying risks refresh races, overwriting another login, and changing
the account used by a process that is already running. A second alternative,
putting every subscription through IronWire, requires account-aware credential
and routing capabilities the pinned integration does not currently expose.

Subscription and direct API-key modes launch the matching native tool against
its native provider. NEAR AI uses the existing local routing integration with
an explicit NEAR destination. No automatic cross-provider or cross-account
fallback is part of this feature.

## Product surface

Extend Model calls with a card for Claude Code and a card for Codex:

```text
Claude Code
Connection   [Claude subscription v]
Account      [Personal            v]
             Personal — Selected
             Work
             + Add account

Ready for new sessions
[Open Claude Code]  [Manage accounts]
```

Claude Code's connections are NEAR AI, Claude subscription, and Anthropic API
key. Codex's are NEAR AI, ChatGPT subscription, and OpenAI API key. Account
choices are filtered by the selected connection. Each provider remembers its
last selected account without activating it until the user applies the choice.

Manage accounts supports Add, Rename, Reconnect, and Remove. API keys use a
masked input and a user-chosen label. A subscription account uses the native
tool's browser/device login; the app never asks the user to paste OAuth tokens.
Adding or reconnecting an account does not silently select it for either tool.

Account rows distinguish Saved, Sign-in required, Checking, Ready, and
Unavailable. Selection is a separate indicator: a selected account can still
require sign-in. Tool/session status distinguishes Active, Ready for new
sessions, Restart required, Configuration conflict, and Unsupported version.
Show the last verification time and method where useful. Do not label cached
identity or a running proxy as a verified working connection.

Removing a profile with a running managed session is refused until that session
ends. Removing the selected idle profile clears selection and requires a new
choice; it never selects another account automatically. Removal deletes the
local managed credentials/profile through the supported adapter, and does not
claim to revoke remote sessions unless the provider confirms revocation.

## Session and launch semantics

The selection is the default for the next session launched through Trace
Commons or its explicit launcher command. It does not silently alter arbitrary
terminal shells, IDE integrations, or an already-running Claude/Codex desktop
application. The UI states this scope beside Open and Switch controls.

The launcher passes a session-specific environment to a child process. It never
changes the daemon's process-global environment. Profile paths are generated
from opaque IDs beneath a private application directory, never from labels.
Codex uses its profile's `CODEX_HOME`; Claude Code uses its profile's
`CLAUDE_CONFIG_DIR`, subject to the compatibility gates below.

Each launch captures an immutable selection generation, account ID, connection,
and resolved configuration. Selecting Work while Personal is running affects
the next launch only. Native refresh remains local to the profile. Multiple
processes using one profile rely on the native tool's supported refresh
coordination; Trace Commons does not introduce a second refresh implementation.

Keep native profile settings/history separate by default. The UI explains that
another account's profile can have different history, plugins, and settings.
Do not copy or symlink the user's entire existing home to populate a profile.
Project configuration continues to be handled by the native tool. Reusing an
existing default installation is presented as an external, unmanaged profile,
not as an isolated saved account; adopting it must not duplicate live tokens.

Returning an existing globally connected tool from NEAR AI to native routing
uses the current harness disconnect preview and commit guards. That operation
is distinct from changing the launch default and can require restarting an
externally launched tool. The UI must not promise that such an external session
retains its route after its shared config is edited. Guaranteed session
isolation applies to managed launches; this distinction is visible before an
existing global configuration is changed.

## Data and credential ownership

Add an account/profile service in the permissive contributor crate, exposed
through daemon IPC and the existing FFI boundary. Native shells render the same
state and invoke the same operations. No server, gate API, or AGPL dependency
may enter a permissive client crate.

An account record contains an opaque ID, provider, auth kind, user label,
profile reference, lifecycle state, and verification metadata. A per-tool
selection contains its connection, account ID, and monotonic generation.
Session records refer to the captured generation. Labels are local UI data;
logs and audit events use opaque IDs or safe status codes, never raw account
identities, keys, login URLs, tokens, or auth-file content.

Subscription credentials stay in native-tool storage. On supported platforms,
API keys use an OS secret store scoped by account ID; metadata contains only a
secret reference. If secure storage is unavailable, persistent API-key saving
is unavailable with an actionable error. Any new dependency requires explicit
human approval under repository rules.

Secret retrieval happens only in the launch/auth adapter. Secrets must not
appear in process arguments, diagnostic output, ordinary settings JSON, event
payloads, or config previews. Where the native API supports a credential helper
or protected stdin, prefer it; otherwise provide an API key only in the child
environment and document that process-local exposure. Shell command strings
must not embed user-controlled labels or secret values.

## State transitions and errors

1. Add creates a pending account/profile and launches the provider's supported
   sign-in path in that profile. Cancel or failed sign-in leaves existing
   accounts and selections unchanged; incomplete profiles are cleaned up or
   clearly marked recoverable.
2. Verification reads native auth status for the exact profile. An optional
   live check is explicit about any model call or charge. Successful sign-in
   does not by itself prove inference entitlement.
3. Select validates provider/account compatibility and commits a new default
   generation atomically. A stale UI generation is rejected and refreshed.
4. Launch resolves the captured generation, validates effective routing and
   credential precedence, and starts the native tool. Launch failure preserves
   the selected default but reports the failure; it does not claim Active.
5. Reconnect operates on the same idle profile through native login. If a
   session uses the profile, defer reconnect until it ends to preserve its
   account identity. Refuse identity replacement during an active session.

Inherited API keys, proxy endpoints, cloud-provider flags, and native managed
settings can override an intended subscription. Adapters explicitly remove
conflicting inherited values from the managed child environment and detect
remaining configuration conflicts. They never weaken administrator policy or
silently overwrite project/user configuration. Conflicts explain the affected
setting without exposing its secret value.

Route changes touching existing external files retain preview-before-write,
occupied-slot refusal, parsing checks, and preimage-bound commits. A multi-file
change must journal recoverable state, validate every preimage before mutation,
and report partial recovery honestly. Never restore over subsequent user edits.

NEAR AI routing for a managed session must be explicitly pinned and verified,
with no ambient native-account fallback. If the pinned integration cannot
express this isolation, the implementation plan must include the required
upstream capability; a working listener does not satisfy this requirement.

## Authorized IronWire changes

IronWire changes needed to implement this design are in scope. Make them in an
isolated IronWire checkout, following that repository's own guidance, and
integrate the resulting revision into Trace Commons. No separate permission
round is needed merely because an implementation fix belongs upstream.

The upstream work has three concrete contract requirements:

- Profile-aware harness edits: accept an explicit target configuration root
  when planning connect/disconnect. Do not select a profile by mutating global
  `CODEX_HOME` or `CLAUDE_CONFIG_DIR` inside a multithreaded host. Preserve the
  existing preview and commit safety properties for every target root.
- Explicit routing scope: a NEAR AI session must bind to its chosen provider
  and credential reference for its lifetime. A process-global backend pin is
  not sufficient when independent sessions have different selections. Prefer
  an existing isolated instance mechanism if it meets the contract; otherwise
  add an explicit session route handle. Missing or revoked bindings fail
  closed, and route handles cannot access another session's credentials.
- Account-aware credential lookup where proxy use requires it: pass explicit
  profile/secret references instead of falling back to whichever default
  Claude or Codex login happens to exist. Keep legacy discovery behavior for
  existing callers that do not opt into managed profiles. Native subscription
  calls in this design still go directly through their native tools; adding
  subscription proxying is not required to satisfy the feature.

Test these contracts upstream with multiple simultaneous profiles and with
ambient credentials present. Maintain fixed-label/redacted debug output.
Trace Commons must update the IronWire pin, both relevant lockfiles, and the
Flatpak vendor inputs together when the upstream change is integrated. An
upstream fix alone is not evidence that a shipped Trace Commons build uses it.

## Compatibility gates

Official documentation consulted on 2026-10-05:

- [Codex authentication](https://learn.chatgpt.com/docs/auth): credentials may
  be stored under `CODEX_HOME` or in OS storage; native authentication refresh
  and administrator login restrictions apply.
- [Claude Code authentication](https://code.claude.com/docs/en/authentication):
  `CLAUDE_CONFIG_DIR` scopes credential files and macOS Keychain entries, and
  environment credentials can override subscription login.
- [Claude Code settings](https://code.claude.com/docs/en/settings): changing
  the configuration directory also affects settings, history, and plugins.

These docs support the proposed approach but are not installed-version tests.
Before enabling a platform/tool adapter, verify two independent profile logins,
OS-store namespace isolation, native refresh persistence, and launch routing
on the supported native version. Derive and record a minimum supported version
from that evidence. An unverified adapter reports Unsupported version or
Unavailable rather than displaying a working account switch.

Use fixture credentials for automated tests. Any real-account sign-in is
performed interactively by the user; testing must not read or mutate unrelated
existing credentials. Do not promise account switching for standalone desktop
apps or IDEs until their profile-selection mechanism is independently verified.

## Shell scope and delivery

The target is parity across macOS, Windows, and GTK using shared daemon/FFI
contracts. Implement the backend contract and macOS adapter first as a vertical
slice, then bring Windows and GTK through their platform verification gates.
The feature is not complete while a requested shell is a mock or lacks the
account lifecycle. Platform limitations must be visible rather than hidden by
an enabled control.

Likely change areas are new contributor account and launch modules, daemon IPC,
contributor FFI, shared UI copy/state, and each shell's Model calls surface.
Extend existing modules through focused helpers; avoid unrelated refactoring.
No hosted schema or contributor-identity change is needed.

## Acceptance and verification

- Save Personal and Work for each supported subscription provider; restarting
  Trace Commons retains both profiles and the per-tool selections.
- Launch Personal, select Work, and launch Work. Verify distinct account
  identity in each native session and unchanged identity in the first session.
- Change Claude's selection without changing Codex's selection or sessions.
- Exercise native subscription, direct API key, and NEAR AI routing with exact
  effective-destination assertions and no automatic fallback.
- Test inherited credentials and proxy settings, administrator restrictions,
  expired/revoked login, denied/locked OS storage, sign-in cancellation, failed
  launch, native refresh, concurrent selection, reconnect, and removal.
- Test default/global config restoration separately from managed-session
  isolation, including occupied slots, non-UTF-8 input, stale preimages, and
  crash recovery without overwriting subsequent edits.
- Verify secrets do not enter IPC responses, logs, previews, argv, or ordinary
  account metadata. Assert restrictive profile permissions/ACLs per platform.
- Test every shell's selection, keyboard navigation, pending/error states, and
  distinction between Selected, Ready, and Active. Render and inspect the UI.
- Run focused Rust/FFI/shell suites plus relevant repository CI gates with
  `RUSTFLAGS=-D warnings`; run the license-boundary test unchanged. If
  dependencies change, obtain approval and run all four license-check variants.

Documentation-only approval does not establish any of these runtime results.
