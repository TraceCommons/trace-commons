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

- Supported native adapter versions begin at Claude Code 2.1.289 (major 2) and
  Codex 0.160.0 (major 0). Older or unrecognized versions are refused before
  login, logout, or managed use.
- App sessions open in the terminal chosen at Settings > Private AI > **Open
  sessions in** (see "Terminal choice" below). The packaged `near-ai` helper
  must be present. Flatpak host-terminal launching is unavailable; it requires
  an explicit host bridge, so under Flatpak the picker offers nothing and is
  not drawn. No terminal-focus operation is advertised.
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

## Terminal choice

Decided in #1406 (design owner, 2026-10-14): the choice lives in Settings >
Private AI on every shell, the Start buttons are unchanged, and Windows and
Linux get the same choice as macOS, as an exception to the #1173 D13 freeze.

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
| macOS | Terminal | `terminal` | AppleScript `do script` | unchanged from before the choice |
| macOS | Ghostty | `ghostty` | AppleScript `new surface configuration`, `command`, `wait after command` | Ghostty AppleScript dictionary |
| macOS | iTerm2 | `iterm2` | AppleScript `create window with default profile command` | iTerm2 AppleScript dictionary |
| Windows | Windows Terminal | `windows_terminal` | `wt.exe new-tab --title "NEAR AI" -d <folder> <helper> <args>` | `new-tab --startingDirectory, -d` and `commandline` ("Executable with optional arguments"), <https://learn.microsoft.com/en-us/windows/terminal/command-line-arguments> |
| Linux | GNOME Terminal | `gnome_terminal` | `gnome-terminal --working-directory=<folder> -- <helper> <args>` | `--working-directory=DIRNAME` "Set the terminal's working directory"; `--` is what the deprecated `-x, --execute` entry ("interpret all subsequent options as a program and arguments") says to use instead (`-e` is deprecated too), gnome-terminal(1), <https://manpages.debian.org/testing/gnome-terminal/gnome-terminal.1.en.html> |
| Linux | Konsole | `konsole` | `konsole --workdir <folder> --hold -e <helper> <args>` | `--workdir dir` "Open with dir as the initial working directory"; `-e` "will catch all following arguments ... and execute it as command"; `--hold` "Do not close the initial session automatically when it ends", <https://docs.kde.org/stable_kf6/en/konsole/konsole/command-line-options.html> |
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
- Other Linux terminals (kitty, Alacritty, WezTerm, foot, xfce4-terminal,
  Tilix) were not evaluated in this change; each can be added once its
  documented flags are checked against the test above.

Every launch path except macOS Terminal still needs the release smoke test
below on its platform: none of the Windows or Linux terminals is started by
the test suite.

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
export is staged. The CLI is also the launch surface on Windows and Linux;
this change does not extend the frozen desktop launch controls there.

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
