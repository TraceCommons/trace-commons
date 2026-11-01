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
A session the app opened in Terminal can be shown (its tab brought to the
front) and stopped from its row; a CLI launch cannot, because the app neither
opened its window nor started its launcher. An ended session can be started
again in the same folder with the same account while both still exist.
Stop signals only the tool process the launcher supervises: SIGTERM, then
SIGKILL after a bounded wait. Processes the tool itself started (a shell
command, a dev server) are not signalled and can outlive it.
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
- App sessions open in the terminal chosen at Settings > Private AI > **Open
  sessions in** (see "Terminal choice" below). The packaged `near-ai` helper
  must be present. Flatpak host-terminal launching is unavailable; it requires
  an explicit host bridge, so under Flatpak the picker offers nothing and is
  not drawn. Show brings a session's Terminal tab to the front on macOS; a
  session opened in another terminal app cannot be shown.
- Each NEAR AI session owns an isolated IronWire proxy and key, pinned to the
  NEAR AI backend only, never to a model. It never changes the standard proxy
  or falls back to another provider/account. A request naming a model NEAR AI
  lists is served as that model, so each tool's own model picker works; a
  request naming an id NEAR AI does not list is given a listed model of the
  same tier by IronWire's routing. The session's default model -- the one the
  tool starts on -- is chosen from NEAR AI's catalogue (`managed/models.rs`):
  chat models only (text in, text out only, minus rerankers and the privacy
  filter), then the model the launch named, or the tool's default -- the
  newest `anthropic/claude-sonnet-*` for Claude Code, the newest `openai/gpt-*`
  for Codex (never a `mini`, `nano` or `lite` variant), falling back to the
  vendor's first model and then the first chat model; OpenCode on the sign-in
  takes the first chat model. A named model NEAR AI does not list as a chat
  model starts nothing (`managed-model-unavailable`). The session row shows
  the default model, not a model picked later inside the tool. Claude Code and
  Codex name a default with `near-ai launch <tool> --model <id>` or the macOS
  Launch sheet's model picker, for a saved NEAR AI account.
- Every model id a tool can send is a listed NEAR AI chat model:
  - Claude Code: `ANTHROPIC_MODEL` is the default. Each of
    `ANTHROPIC_DEFAULT_{OPUS,SONNET,HAIKU,FABLE}_MODEL` names the newest
    listed `anthropic/claude-<family>-*` model, with
    `ANTHROPIC_DEFAULT_<FAMILY>_MODEL_NAME` set to NEAR AI's display name when
    it gives one; a family NEAR AI does not list names the default.
    `ANTHROPIC_SMALL_FAST_MODEL` is the Haiku value. A default no alias names
    is offered as `ANTHROPIC_CUSTOM_MODEL_OPTION` (and `_NAME`). The aliases'
    `_DESCRIPTION` and `_SUPPORTED_CAPABILITIES` variables,
    `ANTHROPIC_CUSTOM_MODEL_OPTION` when unused, and
    `CLAUDE_CODE_SUBAGENT_MODEL` are removed from the inherited environment.
  - Codex: `-c model_catalog_json="<file>"` names a catalog of every chat
    model, the default first, and `-c model="<default>"`. The file is
    private to the launcher (`managed-codex-catalogs/<session>.json` in its
    state directory) and removed when the session ends. A model Codex ships
    settings for (matched as Codex matches an id: longest slug prefix, then
    the same after one `namespace/`) keeps those settings and prompt, read
    from `codex debug models --bundled`; any other model gets basic settings
    with the prompt of Codex's own first listed model. If that list cannot be
    read, every entry is basic, with a one-line prompt.
    Managed Codex uses IronWire's Responses-to-Chat compatibility route:
    some NEAR AI hosted models complete native Responses tool turns without
    returning an executable call. Function tools (including namespaces),
    streamed calls and tool-result history are preserved through translation.
    Server-side web search is disabled for these launches because Chat
    Completions cannot execute that built-in tool. These command-line settings
    do not change saved Codex settings or non-NEAR sessions. Start a new managed
    session after updating the app to use the repaired proxy.
  - OpenCode: every chat model is listed under the `trace-managed` provider
    (named by NEAR AI's display name or the id); `model` and `small_model`
    are the default. Its profile is isolated, so the list never reaches the
    person's own OpenCode settings.
- Each tool is told the context window NEAR AI lists for a model
  (`context_length` in `/v1/models`, with `max_output_length`), so it
  compacts before NEAR AI refuses a request as too long. A model NEAR AI
  gives no window for is left to the tool's own assumption.
  - Claude Code: `CLAUDE_CODE_MAX_CONTEXT_TOKENS` is the default model's
    window, or removed when NEAR AI gives none. Claude Code applies it only
    to an id it does not recognize (another vendor's model, or a Claude
    model newer than the installed Claude Code, which it would otherwise
    assume is 200,000 tokens); a Claude model it knows keeps its Anthropic
    API window.
  - Codex: each catalog entry's `context_window` is NEAR AI's window less
    the output limit (how Codex sizes its own models: GPT-5's 400,000 less
    128,000 is Codex's 272,000), and never more than the window Codex ships
    for a model it knows.
  - OpenCode: each listed model's `limit` is `{context, output}` from NEAR
    AI; without an output limit, OpenCode's own 32,000, never above the
    window.
- Each NEAR AI session's proxy has its own IronWire home,
  `managed-routes/<session id>` in the state directory, whichever process
  hosts it (the launcher for a saved key, the daemon for the sign-in). Its
  call log and receipt checks are on: `ledger.sqlite` in that home records each
  exchange's timings, facade and path, an opaque conversation hash, the
  client's session header, backend, requested and served model, token counts,
  cost, status and, for a call that failed, IronWire's error text. It stores
  no request or response body unless the separate
  `ironwire_attested_bodies` setting was enabled at launch. The error text
  is not body-free: for an error from NEAR AI it includes up to 600
  characters of the provider's error message, and a provider can quote the
  request there. These errors remain in the private local history; usage
  summaries never select them. The status line does not read the log.
  The home, ledger, proof verdicts and any consented body captures remain
  private to the person (mode 0700 on Unix) after shutdown. Shutdown stops
  the listener and removes its control token; it does not delete history.
  Calls and Verified totals include these histories over the same 24-hour
  window as the shared proxy. A source that cannot be read is unknown,
  never a measured zero.
- Managed Claude Code and Codex sessions collect only when the launch's
  **Collect trace for review** choice is on. After exit, new transcript files
  are staged separately from private account profiles. These explicit exports
  can enter review even when automatic source discovery is off; project arming,
  review and upload approval remain in force.
- A trace can carry its actual final model exchange only while
  `ironwire_attested_bodies` is enabled. Its request and response are read
  from the owning proxy's body store and checked against the stored digests;
  colliding store-local IDs never select another session's bodies. Receipt
  verification uses the existing enrolled quote verifier and measurement
  pins, including signed staging pins. Missing pins or invalid evidence
  cannot produce Verified. OpenCode binds its exact collected session ID
  to the local proxy; this header is not sent to direct external API routes.
  Revoking body consent stops body-capturing managed listeners; requests
  already in flight may drain, and reenabling requires a new launch. A
  listener that recorded metadata only continues running. Receipt checks
  still pending at shutdown may remain pending in local history; later
  witness verification independently checks the final exchange's receipt.
  Transcript collection, token capture and upload approval are independent
  choices. Enabling one does not enable the others.
- Claude Code on a NEAR AI session shows IronWire's status line. It is
  launched with `--settings '{"statusLine":{"type":"command","command":
  "<this executable> statusline --home <route home>"}}'`. Command-line
  settings take precedence over the profile's and a project's settings (only
  managed policy settings override them), and nothing is written to disk.
  The command is quoted for `/bin/sh` on Unix. On Windows it is written for
  Git Bash, with forward slashes; it also works in PowerShell, which Claude
  Code uses when it finds no Git Bash, only when neither path needs quoting.
  `statusline` reads the route's `endpoint.json` and `control.token`, creates
  nothing, and prints nothing and exits 0 on any failure, within IronWire's
  400 ms budget.
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

## Terminal choice

Decided in #1406 (design owner, 2026-10-14): the choice lives in Settings >
Private AI on every shell, the Start buttons are unchanged, and Windows and
Linux get the same choice as macOS, as an exception to the #1173 D13 freeze.

**The first launch asks.** Until the person has chosen a terminal
(`managed_terminal_chosen`, set by any write of `managed_terminal` and
never cleared), the snapshot's `capabilities.terminal_chosen` is false, and
the macOS launch dialogs (the sign-in card's and Launch managed session)
draw the same "Open sessions in" picker as Settings, over the installed
apps, starting on the current choice. Open saves the choice, then
launches, so the session opens there and later dialogs do not ask. With
one terminal installed nothing is asked. A settings file written before
the key existed counts as not chosen, so a person who has launched before
is asked once too.

**The test.** A terminal is offered only if it accepts both a command and a
working directory from its scripting interface or command line, documented
by its vendor. On Windows and Linux the command reaches it as an argv vector:
the folder, the helper's path, the config folder and the ticket are each one
argument and no shell parses them. On macOS the command is one escaped
AppleScript string literal in a fixed template, and the helper starts the
tool in the session's folder itself.

The settings key is `managed_terminal`; a daemon refuses another platform's
id. The default is what shipped before the choice: Terminal on macOS and
Windows Terminal on Windows. Linux shipped `x-terminal-emulator`, which fails
the test, so the Linux default is GNOME Terminal; on a computer without it a
launch is refused with "{terminal} wasn't found on this computer. Choose
another in Settings." until another is chosen.

| Platform | Terminal | id | How the command and folder reach it | Evidence |
| --- | --- | --- | --- | --- |
| macOS | Terminal | `terminal` | AppleScript `do script`, addressed by bundle id `com.apple.Terminal` | the launch shipped before the choice |
| macOS | Ghostty | `ghostty` | AppleScript `new surface configuration`, `command`, `wait after command` | Ghostty AppleScript dictionary |
| macOS | iTerm2 | `iterm2` | AppleScript `create window with default profile command` | iTerm2 AppleScript dictionary |
| macOS | kitty | `kitty` | `open -n -b net.kovidgoyal.kitty --args --directory <folder> --hold <helper> <args>` | as Linux kitty below; `--detach`: "On macOS use open -a kitty.app -n instead", kitty(1); bundle id from kitty's `setup.py` (`net.kovidgoyal.{appname}`) |
| macOS | WezTerm | `wezterm` | `open -n -b com.github.wez.wezterm --args --config exit_behavior="Hold" start --always-new-process --cwd <folder> -- <helper> <args>` | as Linux WezTerm below; the bundle's executable is `wezterm-gui`, which takes `--config` and `start` itself (`wezterm-gui/src/main.rs`); bundle id from `assets/macos/WezTerm.app/Contents/Info.plist` |
| macOS | Alacritty | `alacritty` | `open -n -b org.alacritty --args --working-directory <folder> --hold -e <helper> <args>` | as Linux Alacritty below; bundle id from `extra/osx/Alacritty.app/Contents/Info.plist` |
| Windows | Windows Terminal | `windows_terminal` | `wt.exe new-tab --title "NEAR AI" -d <folder> <helper> <args>` | `new-tab --startingDirectory, -d` and `commandline` ("Executable with optional arguments"), <https://learn.microsoft.com/en-us/windows/terminal/command-line-arguments> |
| Linux | GNOME Terminal | `gnome_terminal` | `gnome-terminal --working-directory=<folder> -- <helper> <args>` | `--working-directory=DIRNAME` "Set the terminal's working directory"; `--` is what the deprecated `-x, --execute` entry ("interpret all subsequent options as a program and arguments") says to use instead (`-e` is deprecated too), gnome-terminal(1), <https://manpages.debian.org/testing/gnome-terminal/gnome-terminal.1.en.html> |
| Linux | Konsole | `konsole` | `konsole --workdir <folder> --hold -e <helper> <args>` | `--workdir dir` "Open with dir as the initial working directory"; `-e` "will catch all following arguments ... and execute it as command"; `--hold` "Do not close the initial session automatically when it ends", <https://docs.kde.org/stable_kf6/en/konsole/konsole/command-line-options.html> |
| Linux | kitty | `kitty` | `kitty --directory <folder> --hold <helper> <args>` | synopsis `kitty [options] [program-to-run ...]`; `--directory` "Change to the specified directory when launching"; `--hold` "Remain open, at a shell prompt, after child process exits", kitty(1), <https://sw.kovidgoyal.net/kitty/invocation/> |
| Linux | WezTerm | `wezterm` | `wezterm --config exit_behavior="Hold" start --always-new-process --cwd <folder> -- <helper> <args>` | `start`: `--cwd` "Specify the current working directory for the initially spawned program", `--always-new-process` "don't try to ask an existing wezterm GUI instance to start the command", and the program after `--`, <https://wezterm.org/cli/start.html>; `--config 'exit_behavior="Hold"'`, <https://wezterm.org/config/files.html>, "keep the pane open after the program exits", <https://wezterm.org/config/lua/config/exit_behavior.html> |
| Linux | Alacritty | `alacritty` | `alacritty --working-directory <folder> --hold -e <helper> <args>` | `--working-directory` "Start the shell in the specified working directory"; `--hold` "Remain open after child process exits"; `-e, --command` "Command and args to execute (must be last argument)", alacritty(1), <https://manpages.debian.org/trixie/alacritty/alacritty.1.en.html> |
| Linux | Ghostty | `ghostty` | `ghostty --working-directory=<folder> --wait-after-command=true -e <helper> <args>` | "Every configuration key is a valid CLI flag", <https://ghostty.org/docs/config>; `working-directory`, `wait-after-command`, and `-e` sets `initial-command` and "Disables shell expansion since the input is expected to already be shell-expanded", <https://ghostty.org/docs/config/reference> |

Windows Terminal reads `;` anywhere in its command line as the start of a new
command (same page: "Commands provide the action, or list of actions separated
by semicolons"). A launch whose folder, helper path or config folder holds a
`;` is refused there rather than escaped.

**Left out.**

- `x-terminal-emulator` (Debian alternatives, the previous Linux
  destination) and xterm: Debian Policy requires only `-e command` and
  `-T title` of a terminal emulator, with no working-directory option
  (<https://www.debian.org/doc/debian-policy/ch-customized-programs.html>),
  so the alternative cannot be relied on to take one.

**What this costs.** A Linux computer with none of GNOME Terminal, Konsole or
Ghostty (an XFCE, Sway or i3 desktop, say) could launch through
`x-terminal-emulator` before and cannot now: the picker offers nothing and
is not drawn, and the managed-sessions screen shows its "unavailable" line
with Launch off, until another terminal is added by the test above. The
same holds on Windows without Windows Terminal, as it did before. On macOS
Launch is never turned off this way, because Terminal is always there.
- The Windows console host, through PowerShell or directly: `powershell.exe`
  takes a command only as PowerShell source (`-Command`, or `-EncodedCommand`
  for the same text in base64), so the folder would have to be quoted into a
  script; `-WorkingDirectory` exists only in PowerShell 7 (`pwsh`), which is
  not on every Windows computer. Starting the helper directly with
  `CREATE_NEW_CONSOLE` passes the test on paper, but whether the new console
  or the daemon's own standard handles reach the session depends on how the
  daemon was started, and that cannot be checked without a Windows run. It
  can be added once a Windows smoke test shows the session gets the console.
  `cmd.exe` takes its command only as one string.
- Other Linux terminals (foot, xfce4-terminal, Tilix) have not been
  evaluated; each can be added once its documented flags are checked
  against the test above.
- Warp (macOS), as of 2026-10-30. It has no AppleScript dictionary and no
  command line that opens a local window with a command
  (<https://github.com/warpdotdev/warp/issues/3364>; the `oz` CLI runs
  Warp's own agents, <https://docs.warp.dev/developers/cli>). Its URI
  scheme opens a folder but "does not run a shell command"
  (`warp://action/new_window?path=`, <https://docs.warp.dev/features/uri-scheme>).
  The only route that runs a command is a Tab Config, a TOML file under
  `~/.warp/tab_configs/` with `directory` and `commands`, opened by
  `warp://tab_config/<name>?new_window=true`
  (<https://docs.warp.dev/terminal/windows/tab-configs/>). That fails the
  test: the command reaches Warp as shell text typed into its shell, not as
  argv or a string literal, the one-time ticket would be written into a file
  Warp lists in its own menu, the file has to be removed after Warp has read
  it with no signal that it has, and nothing reports whether the window
  opened. Revisit if Warp ships a scripting interface or a CLI that takes a
  command and a folder.

Every launch path except macOS Terminal still needs the release smoke test
below on its platform: none of the Windows or Linux terminals is started by
the test suite.

## OpenCode sessions

OpenCode managed launches use the app's NEAR AI sign-in, an OpenAI API key
(`api-key`) or a saved NEAR AI key (`near-ai`). OpenCode subscription sign-in is
not supported. A saved-key launch requires an explicit **Model ID**; the app's
NEAR AI sign-in offers its model catalogue and default. Enable **Collect trace
for review** on the folder-selection screen to collect the launched session
when OpenCode exits.

The launcher detects OpenCode 1.x or 2.x with a bounded `--version` probe and
selects the corresponding configuration and session commands. Unknown versions
and major versions are refused. Session preparation also checks exact-ID
import/export compatibility before launch; format validation remains required
before any export enters review. Native synthetic-provider round trips cover
OpenCode 1.19.4 and 2.1.4.

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

Each launch owns a separate HOME/XDG profile beneath its saved account.
OpenCode 1.x uses `--pure` and the legacy provider configuration. OpenCode 2.x
uses `--standalone`, native provider configuration, and `session import/export`;
it never joins the shared background service. Project configuration and external
plugins are disabled. The one exception is the economizer's own plugin, which a
1.x launch loads when the economizer is on (see [Economizer](#economizer)). Since v2 cannot bypass overriding system or macOS MDM
configuration, managed launches refuse when those settings are present or
cannot be inspected. The ordinary OpenCode profile is unchanged.
The selected model also handles v1 small-model calls and v2 title/compaction calls. The session's
provider key is passed through the child environment, never a command argument
or saved configuration file.

Collection exports the exact seeded session ID, validates the complete export,
then atomically writes it to `managed-opencode-exports` beneath the contributor
state directory. These exports and an ordinary chosen export folder share one
OpenCode source. Turning automatic OpenCode discovery off pauses ordinary source
discovery; explicitly collected managed exports still enter review. It does not
delete the local files. Existing project policy, review, redaction,
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
chosen folder. The importer accepts v1 session versions **1.18.29** and
**1.19.0**, plus the v2 typed session-transfer envelope qualified with **2.1.4**.
V2 exports omit producer version, so collection leaves that field absent.
Native SQLite v2 discovery is not enabled. A local synthetic-model qualification
covers import, exact-ID run, selected-model routing, the native session header,
export, validation and staging for both launch paths.
It does not certify real provider credentials, NEAR AI receipts, or unattended
end-to-end contribution without an invite.

## Economizer

The economizer cuts the tokens a managed session spends on work the agent does
not need. It is off until the person runs `near-ai economizer on`, and applies
to new managed Claude Code, Codex and OpenCode sessions. It is our own Rust
implementation of the mechanisms described by Microsoft's TokenEconomizer
(arXiv 2610.22732); no TokenEconomizer code is used.

| Mechanism | What it does |
| --- | --- |
| Loop guard | A turn gets one short note in the model's context, asking it to step back, when it looks stuck: at least 20 tool calls, edits that keep returning to the same file (or spread over three files or more), at least six failed calls, and little progress. It must also have run past what this folder's turns usually take: the 90th percentile of turns that ended well, learned from at least five. Without five, no note. Once per turn. |
| Tool pruning | A tool from a short list per coding tool (below; for Claude Code, its subagent, scheduling, worktree, notebook, todo and web tools) that the agent has not used in 10 prompts over two sessions, or in five sessions, is left out of later sessions in that folder. Tools whose loss would be felt (web fetch and search, notebook editing, every Codex feature) need three times that. A tool once used, or given back with `restore`, is never left out again. |
| Learned commands | A command that is not found in a folder and then works through a launcher (`pytest` as `python -m pytest`) is told to the agent at the start of later sessions, once three sessions confirm it. Two sessions that run the plain command fine, with no confirmation between, forget it. |

How each tool takes it:

| Tool | Hooks | Pruning |
| --- | --- | --- |
| Claude Code 2.1.300+ | `hooks` in the launch's `--settings` | `permissions.deny` there |
| Codex 0.164+, Unix only | `-c hooks.<Event>=...`, each trusted by its hash | `-c features.<name>=false` for `multi_agent`, `goals`, `view_image`, `image_generation` |
| OpenCode 1.19.5+ (1.x) | A plugin written into the session's own profile | `tools.<name> = false` for `todowrite`, `task`, `webfetch` |

OpenCode 1.x launches normally run with `--pure`, which keeps every external
plugin out, ours too. With the economizer on, the launch drops `--pure` and
names only its own plugin. That plugin is the only one that loads: the
profile's configuration directories belong to the session alone, project
configuration is disabled, and on OpenCode 1.19.5 neither a project's
`opencode.json` plugins nor its `.opencode/plugin` directory load. OpenCode 2
could not be checked yet, so its launches run without the economizer.

Nothing is written into the person's folder. The economizer's state lives in
the contributor state directory under `economizer/`, owner-only, with each
folder named by a digest of the tool and the path. It is never uploaded.

A launch runs without the economizer when it resumes a conversation (a changed
tool list would make the tool rewrite its whole prompt cache) or collects its
trace (a contributed trace would carry the economizer's notes and a shorter
tool list, with no way yet to say so). Any economizer failure is silent and
leaves the session as if it were off. What it adds goes into the model's
context only; nothing is shown to the person.

```sh
near-ai economizer on
near-ai economizer status --cwd /path/to/project
near-ai economizer restore --cwd /path/to/project   # give back every left-out tool
near-ai economizer off
```

On macOS, Insights' Economizer tab shows the same switch and, for each
folder and coding tool a managed session ran in (from the session rows and
the remembered folder settings), how many sessions it learned from, the tools
it leaves out with a Give back button, and the commands it tells the agent.
It reads the daemon's `economizer_status`, and writes through
`economizer_set_enabled` and `economizer_restore`
([IPC contract](contributor-daemon-ipc-v1_1.md)). No path crosses the
socket: a row carries the folder's last path component and the store's
digest key.

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

## Agent CLI options

Settings > Tools saves additional CLI options separately for Claude Code, Codex
and OpenCode. The defaults are empty. A saved change applies to newly prepared
coding sessions from the app and `near-ai launch`, including a new launch made
with Start again. Existing sessions and account sign-in commands keep their
options. A launch ticket captures the options at preparation, so changing
Settings while a terminal opens cannot change that launch.

Quote values containing spaces with single or double quotes; backslashes escape
characters. These values become native process arguments directly. Shell
variables, substitutions, wildcard patterns and operators are passed literally;
there is no shell evaluation. Clear the field and choose Save to restore the
agent's defaults. The options use the canonical `managed_cli_options` daemon
setting in the private local state directory, shared with `near-ai config
schema`, `show`, `preview` and `apply`; they are not synced to an account.
Do not put API keys or other credentials in them.

Account routing, isolated profiles and session identity belong to the managed
launcher. Options that replace those controls are refused. Codex `-c` /
`--config` supports choices such as `model_reasoning_effort="high"`, while
provider, profile and credential-store overrides are unavailable. Claude Code
settings-file and session-resume overrides are unavailable. OpenCode's model
and session id remain the launcher's; select its model in the launch dialog.
Unsupported agent flags are reported by the native agent when it starts.

`managed_snapshot.cli_options` is an additive object with `claude`, `codex` and
`opencode` string values. Older daemons omit it. The
`managed_cli_options_set` mutation takes `{tool, options}`, validates quoting
and managed-control overrides, and returns `{tool, options, revision}` after
saving. The acknowledgment reads the current saved value and managed revision together, so concurrent later
saves are not mislabeled as an older value at a newer revision.
Invalid options return the fixed `managed-cli-options-invalid` refusal without
including user input. The `managed_changed` event announces successful saves.
`managed_launch_redeem` adds `cli_options`, the argv array frozen by the original
prepare; older contexts and stored entries default to an empty array.

## Remembered folder choices and collection

The launch dialog's folder choice also shows model, CLI options and **Collect
trace for review**. Choices are remembered separately for each canonical folder
and agent. Account selection stays in the interface or explicit CLI launch;
accounts and credentials are never included in a folder profile.

New folders start with global CLI options, the model default and collection off.
Launching saves the displayed choices; Reset forgets only that folder/agent pair.
A saved model that is no longer available must be changed before launching.
Switching folders or accounts cannot apply an older lookup over the new form.

The `managed_folder_settings` configuration field and
`managed_folder_settings_get/reset` IPC methods share these values with CLI
configuration. Explicit CLI model, collection and options flags override restored
choices. Existing sessions retain the arguments captured when they were prepared.

Collection is scoped to the opted-in managed session after it exits, including
only newly created native transcript files. It preserves the agent's original
local logs and feeds the normal review/contribution pipeline. It does not grant
upload permission or include older traces from sessions where collection was off.
Explicitly collected managed exports can be ingested even when automatic
agent-source discovery is off; ordinary watched folders keep their own policies.

Concurrent managed sessions may share the same account profile and folder,
including different collection choices. Claude Code receives an exact session
UUID at launch; only that initial conversation and its validated subagents are
collected. Codex uses a private per-launch SessionStart receipt containing its
native thread ID and transcript path. The receipt, rollout filename and native
metadata must agree. Original IDs and filenames are preserved for attestation
joins. OpenCode retains its exact seeded session identity.

Codex exact tracking is qualified on Unix with Codex 0.164+. Its tracking-hook
configuration selects an invocation-local app server through Codex's native
configuration-exclusion rules. Only the injected tracking hook is trusted; existing
user/project hook trust remains intact, and an explicitly disabled hook engine
is not enabled. Custom CLI hook overrides, unsupported platforms/versions,
missing or ambiguous receipts, and preexisting resumed logs leave collection
unavailable for that run while allowing the agent to launch. No collection path
falls back to guessing ownership from cwd, time windows or other new files.

Attested body capture continues to require the existing
`ironwire_attested_bodies` consent setting. Collection does not enable it or
manufacture an attestation. A full review queue delays ingestion; a saved local
export is retained until capacity is available. Collection status reports local
preservation, not an upload or guaranteed queue admission.
