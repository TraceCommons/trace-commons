# Opening Private AI sessions in Rex and herdr -- research and design

Date: 2026-11-03. Owner: Zaki. Status: research only; nothing is built.
Asked for "on top of #1870" (Warp Tab Configs, open at the time of
writing). Written against `origin/main` at 64a9ec753 plus the #1870 diff,
which this note reads as the terminal-choice model to extend.

## The ask and the rules it must meet

- Kristi (2026-11-03) wants Private AI sessions "in one window", without
  the macOS permission prompts #1852 caused. #1867 removed those prompts;
  #1870 adds Warp, which opens a tab through a `warp://` link and a 0600
  Tab Config file, with no prompt.
- The rules this note holds both tools to:
  1. **No macOS permission prompt** (Automation, Accessibility). No Apple
     Events to another app.
  2. **The command reaches the terminal as argv or a string literal**, not
     as shell text typed into a shell (`docs/managed-sessions.md`, "The
     test"). Warp was rejected on 2026-10-30 partly for failing this; #1870
     takes Warp anyway.
  3. **The single-use launch ticket** (hex, stored as a hash, 120 s, bound
     to the session id, redeemed only over the daemon's per-user socket;
     `managed/sessions.rs`, `ticket_expires: now + Duration::seconds(120)`)
     should not land anywhere more durable or more visible than today.
  4. No new Rust dependency.

## Summary

| | Rex | herdr |
| --- | --- | --- |
| What it is | Superlogical's terminal app with a built-in multiplexer (server + native app clients), built on libghostty | A terminal multiplexer TUI (like tmux) for coding agents; runs inside whatever terminal you use |
| Platforms | macOS app only today (`LSMinimumSystemVersion` 15.0); server code mentions Windows | macOS, Linux, Windows (named pipe) |
| Availability | Private beta (`Rex Beta 0.1.0 (1758)`), proprietary | Public, Apache-2.0 (relicensed from AGPL-3.0 on 2026-07-22), ~47.8k stars, v0.9.5 (2026-10-14), pushed 2026-11-03 |
| How we would open a session | `rex api call -s <session> session.new_window -` with JSON on stdin: argv `command`, `cwd`, `shell: "none"` | NDJSON `layout.apply` on `~/.config/herdr/herdr.sock`: one pane with argv `command`, `cwd`, `env` |
| Prompt | None (unix socket, verified) | None (unix socket; not run locally, herdr is not installed) |
| Ticket | argv only (no `env` option); same exposure as kitty/WezTerm/Alacritty today | `env` -- better than every current terminal; argv is persisted to disk by herdr, so argv must hold no ticket |
| One window | Yes: one Rex session "Near.ai", one tab (Rex "window") per Private AI session | Yes: one herdr workspace "Near.ai", one tab per session |
| Show | `rex focus` (verified) + app action `session.select` (not verified) + `open -b` to raise the app | `tab.focus` switches an attached client's view; cannot raise the host terminal window without Apple Events |
| When not running | `open -b com.superlogical.rex`, wait for an app client, then launch | Needs a herdr client in some terminal: open the chosen terminal running `herdr`, wait for the socket, then launch |
| Build now? | No -- private beta, not testable in CI, few users | Not yet -- worth building after #1870 lands, if the owner wants a cross-platform "one window" |

Neither integration is a code dependency: both talk to a separately
installed program over its CLI or socket, so neither touches the license
boundary (`license_boundary.rs`) or `deny.toml`.

## Rex

### What it is

Rex is the terminal multiplexer from Superlogical (Mitchell Hashimoto's
company after HashiCorp and Ghostty), built on libghostty. A background
server owns the terminals; the macOS app and the `rex` CLI are clients of it.
Coverage: The Register, 2026-07-31,
<https://www.theregister.com/2026/07/31/202614/>; <https://www.bitdoze.com/superlogical-rex/>.
No public documentation site was found; everything below comes from the
installed beta and its CLI.

Terms (from `rex skill show`, its own agent guide):

| Rex term | Means |
| --- | --- |
| session | a workspace; shown in the app's sidebar |
| window | a **tab** within a session |
| block | a pane (a terminal) in a window |
| client | a connected program, e.g. Rex.app on one Mac |

### Evidence

Installed build and API:

```
$ rex version
rex 0.0.1793645657-gd018d2
commit: d018d2de8cc31281265906a4648ec9cae8d2f4c5
built: 2026-11-02T18:54:17Z
api: revision 10 (supports 0 to 10)
```

App bundle (`plutil -p "/Applications/Rex Beta.app/Contents/Info.plist"`):

```
"CFBundleIdentifier" => "com.superlogical.rex"
"CFBundleName" => "Rex Beta"
"CFBundleShortVersionString" => "0.1.0"
"CFBundleVersion" => "1758"
"CFBundleURLTypes" => [ { "CFBundleURLSchemes" => [ "rex" ] } ]
"LSMinimumSystemVersion" => "15.0"
```

The `rex:` URL scheme exists, but the only route found in the app binary
(`strings`) is `rex://settings/server`; there is no URL that opens a
session or runs a command. The integration has to use the CLI.

The CLI ships inside the app bundle, at
`Rex Beta.app/Contents/Helpers/Rex Server.rex/Contents/MacOS/rex`, and the
app also installs a current copy under
`~/Library/Application Support/com.superlogical.rex/RexServer/Versions/Current/Rex Server.app/Contents/MacOS/rex`.
Neither is on the default `PATH`; Rex adds its directory to `PATH` only
inside its own terminals (`REX_BIN_DIR`). The daemon would find it from the
bundle path that the existing installed-apps probe already returns
(`installed_probe` in `terminal.rs`: `NSWorkspace
URLForApplicationWithBundleIdentifier`, run through JXA, which sends no
Apple Event to another app and asks for no permission).

The server socket is owner-only:

```
$ ls -la ~/Library/Application\ Support/rex/
drwx------  ...  .
srw-------  ...  server.sock
```

Terminal block options (`rex api call block_creator.list`, flavor
`com.superlogical.terminal.shell`):

```
['shell', 'command', 'args', 'login', 'cwd', 'path_prepend', 'size',
 'theme', 'initial_input', 'exit']
```

`command` is "A program and its arguments ... The arguments reach the
program unchanged"; `shell: "none"` starts it "directly, with no shell".
There is **no `env` option**, at the block or at `session.new_window`
(the method's description has no `"env"` key). `initial_input` is "Text to
type into the terminal once its program starts".

`rex api call` reads its JSON arguments from stdin with `-`, so nothing in
the request appears in the `rex` process's own argv:

```
$ rex api call --help
... Pass the object directly, use @path to read it from a file, or use - to
read it from stdin.
```

### Scratch-session test

One scratch session, `tc-research-scratch`, was created and then killed; no
other session was touched.

```
$ rex new tc-research-scratch --json -c /tmp -- /bin/sleep 300
session=session:01a1a1cb-...
$ printf '{"window_label":"Near.ai test","focus":false,"layout":{"block":{
    "flavor":"com.superlogical.terminal.shell","label":"near-ai-test",
    "options":{"shell":"none","cwd":"<scratch dir>",
      "command":["/bin/sh","-c","pwd; env | grep -E \"^REX_|^TERM=\"; echo ARGV_TEST=$0; sleep 60","FAKE_TICKET_abc123"],
      "exit":{"on_completion":false}}}}}' \
  | rex api call -s "$S" session.new_window -
{"window_id":"window:01a1a1cb-e32b-...","block_ids":["block:01a1a1cb-e327-..."],"revision":2}
```

The pane ran the argv in the folder, and the argument arrived unchanged:

```
$ rex capture -s "$S" -b "$B" --trim
<scratch dir>
TERM=xterm-256color
REX_SESSION=session:01a1a1cb-...
REX_BLOCK=block:01a1a1cb-e327-...
REX_BIN_DIR=~/Library/Application Support/com.superlogical.rex/RexServer/Versions/Current/Rex Server.app/Contents/MacOS
REX_SERVER=unix://~/Library/Application%20Support/rex/server.sock
ARGV_TEST=FAKE_TICKET_abc123
```

Rex starts every terminal through `/usr/bin/login`, so the argv is held by
a root-owned process too:

```
$ ps -axww -o user,pid,ppid,args
root   74320 48331 /usr/bin/login -flp <user> /bin/bash --noprofile --norc -c name=$1; shift; exec -a "-$name" -- "$@" rex-login sh /bin/sh -c pwd; ... FAKE_TICKET_abc123
<user> 74323 74320 -sh -c pwd; ... FAKE_TICKET_abc123
```

Show, server side, and the handle a row could keep:

```
$ rex focus -s "$S" "$B"; echo rc=$?
rc=0
$ rex session inspect "$S" --json   # .active_window_id
window:01a1a1cb-e32b-...            # the test tab is now the active tab
$ rex block inspect -s "$S" "$B" --json
{'block_id': 'block:01a1a1cb-e327-...', 'window_id': 'window:01a1a1cb-e32b-...',
 'label': 'near-ai-test', 'flavor': 'com.superlogical.terminal.shell'}
$ rex kill "$S"; rex block inspect -s "$S" "$B"
Error: block target "block:..." not found: session scope: session "session:01a1a1cb-..." not found
```

`rex block inspect` does not return a block's launch options, so another
client cannot read the command back through the API. After the kill, a
`grep -r FAKE_TICKET` over `~/Library/Application Support/rex/` found
nothing. Whether Rex writes block options to disk *while the block is
alive* was not checked.

The app's client actions (`rex -C <app client> actions`, read only)
include `session.select  Go to Session  session_id, window_id?`. It was
**not** run: it would switch the owner's app to another session. Whether it
also brings the app to the front is unverified.

### Recommended shape

- `TerminalApp::Rex`, macOS only, bundle id `com.superlogical.rex`, label
  "Rex". Offered when the installed probe finds the bundle.
- `MacosLaunch::Rex(RexTab)`. The daemon:
  1. Finds the CLI at `<bundle>/Contents/Helpers/Rex Server.rex/Contents/MacOS/rex`.
     If missing, `TerminalNotFound`.
  2. Finds or creates the grouping session: `rex ls --json`, match the label
     "Near.ai", else `rex new Near.ai --json`. Kristi's "one window" is this
     session; each Private AI session is a tab in it.
  3. Launches with `rex api call -s <session> session.new_window -` and
     this JSON on stdin: `window_label` (the tool and folder name),
     `focus: true`, one block with `shell: "none"`, `command` = the helper's
     argv (what `helper_args` already builds), `cwd`, and
     `exit.on_completion: false` so an error stays readable.
  4. Records `TerminalHandle::Rex { session_id, window_id, block_id }` from
     the reply, each checked against a strict `session:`/`window:`/`block:`
     plus UUID pattern, as `valid_tty` checks a tty.
  5. Brings the session into view: `rex -C <app client> do session.select
     session_id=<id> window_id=<id>` when an app client is connected, then
     `open -b com.superlogical.rex` (LaunchServices; no permission) to raise
     the app.
- **Show**: `rex focus -s <session> <block>`, then the same
  `session.select` and `open -b`. A missing block means the tab was closed:
  `NotFound`, as Terminal's handle does today.
- **Rex not running**: the CLI's `--autostart` (default on) starts a
  headless server, not the app, and "The command doesn't ask an app to
  display the new session." So: `open -b com.superlogical.rex`, poll `rex
  client ls --json` for a `kind: "app"` client for up to ~10 s, then launch.
  Probes that only list (installed detection, the grouping-session lookup
  before a launch) pass `--autostart=false`, so looking never starts a
  server.
- Every `rex` call through `bounded_output`, with `--timeout`.

### Ticket treatment

Rex has no way to give a program an environment variable, so the choices
are:

1. **argv** (recommended for parity). The ticket is in the helper's argv
   for the helper's lifetime, visible to every local account through `ps`
   (macOS shows other users' argv; checked), and also in the root-owned
   `login` process. This is exactly the exposure the kitty, WezTerm,
   Alacritty, Windows Terminal and Linux launches have today, and the same
   mitigations hold: 120 s, single use, and only redeemable over the
   daemon's owner-only socket, which another account cannot reach. The
   `rex` CLI process itself never sees it (stdin).
2. **A 0600 ticket file**, path in argv, which the helper reads and deletes
   on start. Strictly better than #1870's "delete after 60 s", because the
   reader removes it, and better than argv because nothing secret is in
   `ps`. It needs a helper change (`redeem-launch --ticket-file`) and a
   daemon-side sweep for files never read.
3. `initial_input`: rejected. Rex types it into the terminal "once its
   program starts", before the helper can turn echo off, so the ticket
   would show on screen and in the 64 MiB scrollback, readable by
   `rex capture` from any client, including Rex's own Tailscale-paired
   devices.

## herdr

### What it is

"the runtime your coding agents live on" (<https://github.com/herdrdev/herdr>,
<https://herdr.dev>). A single Rust binary: a background server owns the
terminals; `herdr` in any terminal attaches a TUI client to it. It marks
each pane working, blocked or idle, recognises Claude Code, Codex,
opencode and others by process and output, and lets agents drive it over a
CLI and a socket API. Not installed on this machine; everything below is
from its docs and source at `herdrdev/herdr` main, 2026-11-03.

Terms (<https://herdr.dev/docs/concepts/>): a **workspace** is "the
top-level project container"; a **tab** is "a layout inside a workspace";
a **pane** is "a real terminal". "The server owns panes and process state.
The client is the terminal UI attached to that server." Several clients
may attach; "each can view its own workspace and tab".

```
$ gh api repos/herdrdev/herdr --jq '{license:.license.spdx_id,stars:.stargazers_count,pushed_at,created_at}'
{"license":"Apache-2.0","stars":47775,"pushed_at":"2026-11-03T11:36:13Z","created_at":"2026-03-27T17:54:33Z"}
$ gh api 'repos/herdrdev/herdr/commits?path=LICENSE' --jq '.[] | [.commit.committer.date,.commit.message]'
2026-07-22T19:56:57Z  chore: relicense herdr under apache-2.0
2026-05-26T20:28:56Z  docs: clarify dual licensing
2026-03-27T18:37:05Z  initial release ...
$ gh api repos/herdrdev/herdr/releases --jq '.[0:3][] | [.tag_name,.published_at]'
v0.9.5   2026-10-14T16:07:28Z
v0.9.4   2026-10-13T15:41:22Z
v0.9.3   2026-09-29T19:29:23Z
```

### Evidence

Socket API (<https://herdr.dev/docs/socket-api/>): default socket
`~/.config/herdr/herdr.sock` (named sessions under
`~/.config/herdr/sessions/<name>/`); newline-delimited JSON,
`{"id":"req_1","method":"ping","params":{}}`; a named pipe on Windows.
"JSON API clients should ignore unknown fields and treat unsupported
methods as normal errors." The schema ships in the repo as
`docs/next/api/herdr-api.schema.json` (`protocol` 22, `schema_version` 1)
and from `herdr api schema --json`.

The socket is created owner-only: `src/api/server.rs`,
`const SOCKET_PERMISSION_MODE: u32 = 0o600;`, applied after bind and
covered by `restrict_socket_permissions_sets_user_only_mode`.

Methods that create a terminal, from the schema:

- `workspace.create`, `tab.create`: `cwd`, `env`, `label`, `focus` -- **no
  command**. The pane starts the user's shell; a command would then have to
  be typed in (`pane run` / `pane.send_input`), which fails rule 2.
- `layout.apply`: `workspace_id` or `tab_id`, `tab_label`, `focus`, and a
  `root` `LayoutNode`; a pane node takes `command` (argv array), `cwd`,
  `env`, `label`. In `src/app/api/layouts.rs`, `handle_layout_apply`
  without `tab_id` creates a **new tab** in the given (or active)
  workspace with `ws.create_tab_argv_command(..., argv, extra_env, ...)`;
  with `tab_id` it replaces that tab.
- `env` values are checked by `normalize_launch_env` (no empty key, no
  `=`, no NUL) and "apply only to the newly launched process".

The CLI has no generic raw call: `herdr api` offers only `schema` and
`snapshot` (`src/cli/api.rs`). So the only launch that meets rule 2 is a
`layout.apply` request written to the socket by the daemon itself: a
`std::os::unix::net::UnixStream` and `serde_json`, both already used. No new
dependency.

Persistence: herdr saves the layout and **each argv-launched pane's
`launch_argv`** to its session snapshot on disk
(`src/persist/snapshot.rs`, `pub launch_argv: Option<Vec<String>>`) and,
after a server or machine restart, re-runs it
(`src/persist/restore.rs`: `terminal.with_launch_argv(argv).with_respawn_shell_on_exit()`).
Launch `env` is **not** saved: restore builds
`PaneLaunchEnv::from_extra(Vec::new())`.

Starting: `herdr server` runs a headless server (`src/main.rs`); a CLI
call with no server answers `server_not_running` ("run `herdr` to start
or attach it", `src/cli/server_not_running.rs`). Installed by `brew
install herdr`, `mise`, or `install.sh` into `~/.local/bin`; Windows by
`install.ps1`.

### Recommended shape

- `TerminalApp::Herdr` on all three platforms, label "herdr". It is not a
  terminal app but a layer inside one, so it is a different kind of choice
  (open question 4).
- Detected when the `herdr` executable is found on the daemon's `PATH`,
  `/opt/homebrew/bin`, `/usr/local/bin` or `~/.local/bin`, with its version
  read by `herdr --version`.
- Launch:
  1. Connect to `~/.config/herdr/herdr.sock` (honour `HERDR_SOCKET_PATH`
     only if the owner wants named sessions). Check the socket's owner is
     the current uid and its mode is 0600 before writing to it, as the
     daemon's own socket is checked.
  2. `ping`; then find or create the workspace labelled "Near.ai" from
     `session.snapshot` / `workspace.create { label, cwd }`.
  3. `layout.apply { workspace_id, tab_label, focus: true, root: { type:
     "pane", command: [helper, ...args without the ticket], cwd, env: {
     NEAR_AI_LAUNCH_TICKET: <ticket> } } }`.
  4. Record `TerminalHandle::Herdr { tab_id, terminal_id }` from the reply.
- **Show**: re-read `session.snapshot`; if a tab still holds
  `terminal_id`, `tab.focus` it (public ids like `w1:t1` look positional
  and `pane.move` reassigns them, so the terminal id is the stable key).
  This changes what an attached herdr client shows; it cannot raise the
  host terminal's window without Apple Events, so on macOS Show only works
  when that terminal is already in front, or after `open -b <host
  terminal>` if the owner accepts that.
- **herdr not running**: open the user's chosen host terminal with argv
  `herdr` (which starts the server and attaches; no secret involved), poll
  for the socket for up to ~10 s, then launch. Starting `herdr server`
  headless instead would launch a session nobody can see.
- Windows: the named pipe needs its own small client; defer until the
  Windows shell leaves its freeze (#1173 D13).

### Ticket treatment

- **The ticket goes in `env`, never in argv.** argv is written to herdr's
  snapshot on disk and replayed after a restart, so a ticket in argv would
  outlive its 120 s on disk for as long as the tab exists. `env` is not
  persisted; on restore the helper runs with no ticket and is refused, as a
  replayed spent ticket would be.
- `env` is readable only by the same account on Linux
  (`/proc/<pid>/environ` is mode 0400) and, by design, on macOS (not
  measured here), where argv is readable by every account; so this is
  better than every current launch path.
- The request crosses herdr's 0600 socket and lives in the herdr server's
  memory; herdr is the user's own process, as Terminal and Warp are.
- Needs a helper change: `redeem-launch` reads `NEAR_AI_LAUNCH_TICKET`
  when `--ticket` is absent, and **removes it from its environment before
  starting the tool**, so Claude Code or Codex never inherits it.
- The replayed argv (helper path, config folder, session id) is not secret.

## What these give beyond Warp

- **One window, as Kristi asked**: both hold every session in one named
  group (a Rex session, a herdr workspace) as tabs, instead of tabs landing
  in whatever Warp window is active.
- **Show**: both return ids a row can keep; Warp returns nothing (#1870:
  "No tab is recorded, so the row cannot bring it forward").
- **argv, not shell text**: both pass the command as an argv array; Warp's
  Tab Config `commands` entry is typed into Warp's shell.
- **No ticket file**: Rex keeps the ticket out of files; herdr keeps it out
  of argv and files.
- herdr only: Linux and Windows, and per-pane "blocked / done" agent state.

## Risks

- **Rex** is a private beta at 0.1.0 with a "0.0" CLI; its API is versioned
  (`api: revision 10 (supports 0 to 10)`) but nothing promises stability, and
  the CLI lives inside the app bundle, so a release can move it. Only beta
  users can install it, so the picker would show it to almost nobody, and
  CI cannot test it.
- **herdr** is pre-1.0 and moves fast (three releases in two weeks); the
  JSON API is documented and tolerant of unknown fields, but `layout.apply`
  is not marked stable. Its pane detection keys on the foreground process,
  so whether it recognises a tool started through the `near-ai` helper as
  "Claude Code" is unverified.
- Both are separate programs the daemon talks to; a broken release fails a
  launch with `LaunchUnknown`, never silently elsewhere.

## Open questions for the owner

1. **Build either now?** Recommendation: no. Land #1870 first. Rex is a
   private beta that almost no contributor can install; herdr is real but
   needs a socket client and a helper change. Revisit herdr when someone
   asks for it, or when Linux/Windows want "one window".
2. **Rex ticket: argv (parity) or a ticket file the helper deletes?**
   Recommendation: argv for parity now; do the helper's `--ticket-file` /
   env read once, for all argv terminals, as its own change.
3. **herdr ticket in `env`, with the helper change it needs?**
   Recommendation: yes; argv is ruled out by herdr's persistence.
4. **Is herdr a terminal choice, or a layer over one?** It runs inside a
   terminal, so "Open sessions in: herdr" also needs "which terminal hosts
   herdr" when no client is attached. Recommendation: a separate switch
   under the terminal picker, "Group sessions in herdr", which uses the
   chosen terminal to attach.
5. **Group name.** Recommendation: one Rex session / herdr workspace named
   "Near.ai", found by label, recreated if the user closes it.
6. **May Show raise the app with `open -b`?** It asks for no permission,
   but it moves focus. Recommendation: yes for Rex (it is the app); for
   herdr, only switch the tab and do not raise the host terminal.
7. **Rex's macOS 15 floor.** The contributor app supports macOS 14
   (#1173). Recommendation: offer Rex only when the bundle is found, which
   already implies macOS 15; no special case.
8. **Ask Superlogical for a stable "open a tab with argv" entry point**
   (or a `rex://` route), since Rex is in beta and the CLI shape may change.
   Recommendation: yes, before building.

## Not checked

- Whether Rex writes block launch options to disk while a block is alive.
- Whether `session.select` brings Rex.app to the front.
- Anything at runtime for herdr (not installed; installing needs approval).
- Rex on Linux or Windows.
