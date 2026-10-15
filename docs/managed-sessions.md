# Saved accounts and managed sessions

Keep Trace Commons running, then open the model-calls screen. Use **Add account**
to choose Claude Code or Codex, a connection, and a label such as Personal or Work.
For a subscription, **Save and sign in** opens the native tool's sign-in flow in
a terminal with a separate profile. Repeat Add account for each additional
subscription. For an API key or NEAR AI key, enter the key in the secure field;
Trace Commons stores it in the operating system credential store.

**Launch managed session** lets you choose an account and project. The app names
the destination terminal before launch. **Use as default** changes the default
for future managed launches. Changing a default never changes an existing
session. **Change global settings** contains the existing preview-and-commit
controls for standard tool settings; those changes apply after restarting the
tool. Managed launches do not edit those settings.

The CLI is named `near-ai`; `trace-commons-contributor` remains a compatibility
command. Installers install both names from the same verified release bytes.

```sh
near-ai accounts add --tool claude --label Personal
near-ai accounts add --tool claude --label Work
near-ai accounts add --tool codex --label Work
near-ai accounts list
near-ai accounts select Personal --tool claude
near-ai launch claude --account Personal --cwd /path/to/project
near-ai launch codex --account Work --cwd /path/to/another-project
near-ai sessions list
```

CLI launches use the current terminal or multiplexer pane. All managed launches,
including those initiated from the CLI, appear in the app's managed-session list.
Standard sessions and multiple managed accounts can run in parallel.

For an API key, supply it through stdin using a secret-manager pipe, rather than
a command argument or shell-history literal:

```sh
secret-manager-command | near-ai accounts add --tool codex --connection api-key --label Work --key-stdin
```

`secret-manager-command` is a placeholder for your existing secret manager.
Use `--connection near-ai` for a NEAR AI key. Subscription credentials remain
owned by the native tool; they are not pasted into Trace Commons. Account IDs
can be used when labels are duplicated. Removing the selected account requires
an explicit new selection before a CLI launch without `--account`.

## Availability and recovery

The app checks for executable Claude Code, Codex and OpenCode installations
using the same resolver as managed launches. This does not run the tool, scan
its session store, or enable watching, importing or collection. **Install** opens
the tool's official installation page; **Check again** refreshes availability.
Removing a tool leaves its saved accounts and session history intact. The
desktop screens do not offer a launch or sign-in for a tool the resolver cannot
find. On an explicit launch or sign-in request, an executable the daemon finds
is version-checked before a ticket is created, with a ten-second probe deadline;
the helper resolves and checks the tool again where it runs, before starting
it. A CLI launch is not refused because the daemon cannot see a tool that is on
the launching terminal's PATH: the helper's check decides.

The additive `managed_snapshot.tools` array contains `tool`, `installed` and
`install_url`. The read-only `tc_managed_tools_json` FFI export returns that same
array before daemon startup; `tc_managed_copy_json` provides its shared wording.
`tool_destinations` includes `managed_copy` and a separate `managed_sessions`
projection containing `id`, `tool`, `connection`, `provider`, `model`,
`collection_state` and `state`. It excludes sign-in sessions, paths, account
labels and native session identifiers. An unreadable registry yields null rather
than an empty history. Collection states are `off`, `pending`, `collected` and
`failed`; provider labels are `near_ai`, `anthropic` and `openai`. These session
routes do not change the standard harness route or claim observed model calls.
Map tool rows expose `installed` for the three managed tools and null for other
tools whose executable availability is not checked.


- Supported native adapter versions begin at Claude Code 2.1.289 (major 2) and
  Codex 0.160.0 (major 0). Older or unrecognized versions are refused before
  login, logout, or managed use.
- On Windows, discovery requires the native `.exe` on PATH. npm `.cmd` wrappers
  are not executed by this resolver; an installed wrapper alone is unavailable.
- App terminal adapters target Terminal on macOS, Windows Terminal when found,
  and `x-terminal-emulator` on an unsandboxed Linux installation. The packaged
  `near-ai` helper must be present. Flatpak host-terminal launching is unavailable;
  it requires an explicit host bridge. No terminal-focus operation is advertised.
- Each NEAR AI session owns an isolated IronWire proxy and key. It waits for
  NEAR's model catalogue and pins its own proxy to the first advertised model.
  It never changes the standard proxy or falls back to another provider/account.
  Model selection within that catalogue is a follow-up UI capability.
- A profile cannot be reconnected, have its key replaced, or be removed while a
  managed session or sign-in holds it. Multiple coding sessions may share the
  same account, but sign-in requires that account to be idle.
- Starting becomes Running only when the helper reports the native child.
  Terminal exits and confirmed setup failures have private durable receipts that
  reconcile after daemon downtime. An unredeemed launch expires after two minutes.
  Unknown means liveness has not been confirmed, not that it is safe to relaunch
  or remove the account. Forced helper termination may require local recovery.
- A project's explicit provider/auth overrides are refused when they could
  contradict the selected connection. Ordinary project settings are preserved.

No real subscription sign-in or provider-key validation is performed by the
fixture test suite. Native sign-in and external-terminal behavior must also be
smoke-tested on each release platform.

## OpenCode sessions

OpenCode **1.19.0** is supported for managed launches using an OpenAI API key
(`api-key`) or NEAR AI key (`near-ai`). OpenCode subscription sign-in is not
supported. The native macOS Add account dialog includes OpenCode; select an
account, project, and explicit **Model ID** when launching. Enable **Collect
this session** to collect the launched session when OpenCode exits.

```sh
secret-manager-command | near-ai accounts add --tool opencode --connection api-key --label Work --key-stdin
near-ai launch opencode --account Work --model gpt-5 --cwd /path/to/project --collect-traces
```

Use an upstream model ID, without a provider prefix added by Trace Commons.
For NEAR AI, use an ID advertised by that account's model catalogue. A model
absent from the catalogue is refused; the route never substitutes the first
model. Without `--collect-traces`, the native session remains local and no
export is staged. The native macOS, Windows and GTK managed-session screens also offer
OpenCode account setup, an explicit model and opt-in collection. The frozen
Tauri controls are unchanged. Windows and GTK do not currently have a routing
map; their managed rows show the selected account, model and collection state.

Each launch owns a separate HOME/XDG profile beneath its saved account and
uses OpenCode's pure mode. Project OpenCode configuration and external plugins
are disabled for this flow, and the ordinary OpenCode profile is unchanged.
The selected model also handles OpenCode's small-model calls. The session's
provider key is passed through the child environment, never a command argument
or saved configuration file.

Collection exports the exact seeded session ID, validates the complete export,
then atomically writes it to `managed-opencode-exports` beneath the contributor
state directory. These exports and an ordinary chosen export folder share one
OpenCode source. Explicitly turning that source off pauses discovery of both;
it does not delete the local files. Existing project policy, review, redaction,
approval and admission rules still apply. Collection itself grants no upload
permission and creates no attestation evidence.

The process exit and collection result are separate: a session can exit normally
while collection fails. The managed-session row reports pending, collected, or
failed collection; `near-ai sessions list` exposes the same `collection` field.
On export failure the native profile is preserved, and nothing partial enters
the review pipeline. A forcibly terminated helper may leave an unknown process
and pending collection; no automatic recovery or guessed session join occurs.
Only the launched session is collected, not additional sessions created inside
the TUI. Collection occurs on exit, not continuously while OpenCode is running.

Ordinary, independently launched OpenCode sessions still require export into a
chosen folder. The importer accepts session versions **1.18.29** and **1.19.0**;
other versions fail closed. A local synthetic-model qualification covers
import, exact-ID run, explicit model routing, export, validation and staging.
It does not certify real provider credentials, NEAR AI receipts, or unattended
end-to-end contribution without an invite.

## Native shell visibility and map

New setup and launch choices hide unavailable coding tools when at least one
supported executable is installed. With none installed, the managed screens
offer Claude Code, Codex and OpenCode installation instructions and Check again.
Installation is user initiated. Existing accounts, imported sources and session
history survive removal of a tool; unavailable accounts cannot launch.

The macOS first-run tool screen uses the same read-only executable discovery
before starting the daemon. Finding OpenCode does not grant source-watch consent
or invent an export location. It explains that ordinary sessions need an export
folder and managed sessions can opt into collection when launched.

The macOS map includes managed provider/model and collection details. Separate
managed route nodes describe session configuration, using still dashed arcs.
Neither a running process nor a collected export is evidence of an answered
model call, contribution, or global routing configuration. Sign-in-only sessions
are excluded. An unreadable registry remains unknown, and older payloads remain
readable without the added availability or managed-route fields.
