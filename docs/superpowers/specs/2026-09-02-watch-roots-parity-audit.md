# Watch-roots parity: what the three shells actually do

Audit only. No behaviour was changed. One documenting test was added
(`crates/trace-commons-contributor/src/daemon/settings.rs:838`).

Read against `origin/main` at `eaa609c5`. Every claim below is from the code
at that commit; what could not be settled by reading is marked **unverified**.

## The claim under test

> macOS refuses to watch a root the contributor has not declared, while Linux
> (GTK) and Windows silently watch the real `~/.claude`.

**Stale.** It described the tree before
`docs/superpowers/specs/2026-08-19-fail-closed-roots-parity-design.md` landed.
All three shells now refuse, on one shared rule, and report one shared label.
There is a residual Linux-only exposure, but it is not the one the claim
names, and it is not in the GTK shell's own start path — see §4.

## 1. The vocabulary, before the table

`SourceDeclaration` (`crates/trace-commons-contributor/src/daemon/settings.rs:186`)
is a tri-state per agent, and the third state is the dangerous one:

| declaration | meaning | what is constructed |
| --- | --- | --- |
| `{"mode":"watch","path":…}` | watch that folder | an adapter rooted there |
| `{"mode":"off"}` | "I do not use this agent" | nothing, and no fallback |
| absent (`unset`) | never asked | **per-adapter policy** |

The per-adapter policy is `source::Undeclared`
(`crates/trace-commons-contributor/src/source/mod.rs:412`), applied in
`all_sources` (`.../source/mod.rs:631-643`):

- `claude-code` → `Undeclared::Conventional` → real `~/.claude/projects`
  (`.../source/mod.rs:435-446`)
- `codex` → `Undeclared::Conventional` → real `~/.codex/sessions`
  (`.../source/mod.rs:447-451`)
- `gemini-cli` → `Undeclared::Nothing` → no adapter at all
  (`.../source/mod.rs:452-461`)

So `unset` for claude or codex is **a tool in use**: a live scan of the
contributor's real home. `unset` for gemini is not. That asymmetry is
deliberate and documented — gemini was added after every desktop client had
shipped without a field for it, so an undeclared gemini must construct
nothing.

The gate that stops a shell from reaching the `unset` fallback is
`roots_declared` (`.../daemon/settings.rs:584`): **claude AND codex**, both
`is_some()`. `off` counts as declared. Gemini is deliberately not a third
conjunct (`.../daemon/settings.rs:588-602`).

`roots_declared` is written once. The shells consult it; they do not
re-implement it.

## 2. The table

| platform | with nothing declared | refuses? | where the decision is made |
| --- | --- | --- | --- |
| macOS | nothing — the daemon never starts; the app renders the roots screen | yes | `crates/trace-commons-contributor-ffi/src/lib.rs:348` (`roots_refusal`), called at `:512` (`tc_daemon_start`) and `:684` (`tc_daemon_start_with_settings`). Label `roots-not-declared` at `:308`. Surfaced at `macos/Sources/TCBridge/TCDaemon.swift:176`, routed at `macos/Sources/TraceCommonsApp/AppModel.swift:266` |
| Windows | nothing — same C ABI, same refusal | yes | same two FFI call sites. Label carried at `windows/src/TraceCommons.Interop/TcDaemon.cs:123`, tested at `:589` (`IsRootsNotDeclared`), routed at `windows/src/TraceCommons.App/ViewModels/MainViewModel.cs:880` → `SessionRootsViewModel.cs:190` |
| Linux (GTK), hosting | nothing — `Backend::open` bails before `start_embedded` | yes | `crates/trace-commons-contributor-gtk/src/backend.rs:157`, label at `:57`, routed at `crates/trace-commons-contributor-gtk/src/main.rs:157`. Also refuses to *persist* a half-declaration at `backend.rs:96` |
| Linux (GTK), attaching | **whatever the already-running daemon watches** — for a daemon with nothing declared, the real `~/.claude/projects` and `~/.codex/sessions` | no | `crates/trace-commons-contributor-gtk/src/backend.rs:153` returns `Backend::Attached` before the gate at `:157` |
| daemon core (`trace-commons-contributor daemon run`, any platform) | real `~/.claude/projects` + `~/.codex/sessions`; no gemini | no, by design | `daemon/settings.rs:580-583` states the daemon does not consult the gate; roots reach the watcher via `daemon/watcher.rs:122` and `:197` through `ipc.rs:512` |

## 3. Where the divergence lives

**Not in the shells, and not in shared Rust.** The gate itself is one
function with three call sites, and all three shells route on one label. This
is the good outcome: nothing here needs three fixes.

Verified by running, in this worktree:

```
cargo test -p trace-commons-contributor-ffi --test abi root    # 7 passed
cargo test -p trace-commons-contributor --test macos_shell_settings_contract  # 8 passed
cargo test refuse   # in crates/trace-commons-contributor-gtk -> 3 passed
```

## 4. The residual Linux exposure

It is real, and it is reached by a deliberate CLI act rather than by opening
the app:

1. `trace-commons-contributor daemon install`
   (`crates/trace-commons-contributor/src/daemon/install.rs:47`) writes a
   systemd user unit whose `ExecStart` is `<exe> daemon run` (`:23-40`).
   **`install` does not consult `roots_declared`.** It refuses on unpersisted
   NEAR AI credentials (`:52`) and on a non-Linux target (`:70`) and on
   nothing else.
2. That daemon reads `DaemonSettings` — not `cli_source_roots` — so with a
   fresh `daemon-settings.json` it constructs conventional claude and codex
   adapters over the contributor's real home.
3. The GTK shell then finds a running daemon and **attaches without gating**
   (`backend.rs:153`), so the app is a client over a watcher it did not start
   and would have refused to start itself.

The module doc calls the unit "the primary deployment" on Linux
(`backend.rs:3-5`), which is what makes this more than a corner. macOS and
Windows have no equivalent: their autostart registers *the app*
(`macos/Sources/TraceCommonsApp/LoginItemManager.swift:73` uses
`SMAppService.mainApp`; `windows/src/TraceCommons.Interop/AutostartCommand.cs:62`
stores the app's own path with no arguments), and the app goes through the
gate. The GTK shell's own XDG autostart entry also launches the app
(`crates/trace-commons-contributor-gtk/src/autostart.rs:112-125`), so it is
gated too.

The `Attached` path being ungated is defended in the code
(`backend.rs:143-150`): a running daemon was started by somebody typing a
command, and the CLI keeps its defaults on purpose. That reasoning holds for
`daemon run` typed at a prompt. It holds less well for `daemon install`,
which is a one-time command that leaves a scan running forever.

### Smallest change that closes it

Add the gate to `install`, not to the attach path:

```rust
// crates/trace-commons-contributor/src/daemon/install.rs, after :49
if !crate::daemon::settings::roots_declared(&settings) {
    bail!("refusing to install: …declare your session folders first…");
}
```

Roughly five lines. Cost and blast radius:

- It refuses for anyone whose current flow is `daemon install` before ever
  answering the roots question — which is the point, but it is a new failure
  on a path that used to succeed. The CLI needs a way to answer; today the
  only writers of the declaration are `set_settings` over the socket, the C
  ABI's settings-bearing start, and the GTK `declare_sources`. **Unverified:**
  whether `trace-commons-contributor daemon settings` can already write
  `claude_source` / `codex_source` — I did not read the CLI's settings
  subcommand.
- Tests that would move: `install.rs`'s own test module (`:132-150`) gains a
  case; nothing else asserts on `install`. The documenting test added at
  `daemon/settings.rs:838` pins the *fallback*, not the gate, so it survives.
- It does **not** close the `daemon run` case, deliberately. Gating that would
  break the CLI's documented posture and every scripted use.

Leaving the attach path ungated is the right call either way: refusing to
attach would only blind the shell to a scan that continues regardless, which
is strictly worse for the contributor.

## 5. Does the contributor ever see what is being watched?

**No — and one of the two things they are shown is wrong.**

The path never crosses the IPC boundary. `redacted_settings`
(`crates/trace-commons-contributor/src/daemon/ipc.rs:2818`) strips
`claude_source` / `codex_source` / `gemini_source` and replaces them with a
mode word and a boolean (`:2840-2874`). That is correct — a settings blob is
not the place for a home-directory path — but it means no shell can display
the folder it is watching. The roots screen shows the path the contributor
just picked; nothing shows it back afterwards.

Worse, `*_root_configured` is `mode == "watch"`, so it is **false for both
`off` and `unset`** (`ipc.rs:2850-2859`), and two shells print a sentence on
the false branch that is only true for `unset`:

- GTK: `crates/trace-commons-contributor-gtk/src/ui/settings.rs:712-727`
  renders `copy::CHECK_CLAUDE_DEFAULT` = `"Claude Code sessions read from the
  usual place"` (`crates/trace-commons-contributor-gtk/src/copy.rs:784`).
- Windows: `windows/src/TraceCommons.App/ViewModels/ContributorSettingsViewModel.cs:558-567`
  renders the same sentence.

A contributor who declared Claude Code **off** is told their Claude Code
sessions are being read from the usual place. Nothing is being read. It is a
false statement in the fail-*open* direction, in the one screen a
privacy-conscious contributor would check.

macOS does not have this bug — `macos/Sources/TraceCommonsApp/Views/SettingsView.swift:130`
renders an unchecked "Claude Code sessions folder set" row and says nothing
about the usual place — but it also tells the contributor nothing about what
`off` means.

The daemon's own comment anticipated exactly this failure in the other
direction (`ipc.rs:2835-2839`: reporting `off` as configured "would tell a
settings screen to print 'sessions folder set' about an agent the contributor
said they do not use"). The `true` branch was fixed; the `false` branch was
not.

`*_source_mode` — which carries the three-way answer — is already on the wire
and already parsed by all three shells
(`crates/trace-commons-contributor-gtk/src/model.rs:532-536`,
`macos/Sources/TraceCommonsApp/Models.swift:511-513`,
`windows/src/TraceCommons.Interop/SettingsProtocol.cs:69-82`), but only the
routing surfaces read it. Fixing the copy is a three-shell change with no
protocol change: branch on the mode word, not on the boolean.

**This is a divergence in three shells, not in shared Rust** — and unlike the
gate, it has no single owner. Three copies of a sentence about what is being
scanned is three chances to be wrong, and two of them currently are.

## 6. Tests, and whether they can fail

| test | pins | falsifiable? |
| --- | --- | --- |
| `crates/trace-commons-contributor-ffi/tests/abi.rs:1727,1747,1808` | `tc_daemon_start` / `…_with_settings` return NULL + `roots-not-declared` on nothing / half a declaration | **yes** — calls the real ABI against a real tempdir; deleting the `roots_refusal` call makes them fail |
| `crates/trace-commons-contributor-ffi/tests/abi.rs:1775` | the settings-bearing start clears the refusal | yes |
| `crates/trace-commons-contributor-gtk/src/backend.rs:477,490,504` | `Backend::open` bails, and `declare_roots` un-bails it | **yes** — calls the real `Backend::open` |
| `crates/trace-commons-contributor/tests/macos_shell_settings_contract.rs` (8 tests) | the exact JSON the macOS roots screen emits is accepted and flips `roots_declared` | yes — runs the real validator on literal payloads |
| `crates/trace-commons-contributor/src/source/mod.rs:864,893,911` | `off` never reaches the conventional location; `unset` still does for claude/codex; `unset` gemini constructs nothing | yes |
| `windows/tests/TraceCommons.Interop.Tests/SessionRootsTests.cs:365-370` | `IsRootsNotDeclared` is true for a `TcException` constructed with that label | **weak** — it constructs the exception with the label and asserts the property reads it back. It cannot fail unless `IsRootsNotDeclared` is rewritten, and it does not exercise `TcDaemon.cs:123`, which is what actually puts the ABI's label there. Not unfalsifiable — the string equality is real — but it pins the accessor, not the plumbing |
| `macos/Tests/TCShellCoreTests/SessionRootsTests.swift` | `SessionRoots.isComplete` / `settingsJSON()` shape | yes, and correctly scoped: the refusal itself is Rust's, and Swift only pins that it does not send something already known to be refused |

No test on any platform pins §5 — nothing asserts what the settings screen
says about an `off` source. That is why the bug survived.

### Added by this audit

`crates/trace-commons-contributor/src/daemon/settings.rs:838`,
`an_undeclared_daemon_still_builds_the_conventional_claude_and_codex_adapters`.
It pins, through `DaemonSettings::source_roots` (the daemon's real path, not
the `SourceRoots` layer the existing tests use), that default settings
construct claude and codex adapters and no gemini adapter. Passing today by
design; it is the fact §4 turns on, stated once so nobody has to re-derive it
from `Undeclared`.

## 7. Verdict on the original claim, item by item

- "macOS refuses" — **true**, and it refuses in the C ABI rather than in
  Swift, so the rule cannot drift from the daemon's.
- "Windows silently watches the real `~/.claude`" — **false as of the
  2026-08-19 parity work.** Windows uses the same ABI and the same refusal.
- "Linux (GTK) silently watches the real `~/.claude`" — **false for the
  shell's own start path; true in substance for the systemd unit path**, which
  the GTK shell attaches to and which `daemon install` writes without a roots
  check. The mechanism is different from the one the claim describes.

## 8. Unverified

- Whether the CLI (`trace-commons-contributor daemon settings` or similar)
  can write `claude_source` / `codex_source` today. This decides whether the
  §4 fix is five lines or a fix plus a new CLI surface. I did not read
  `commands.rs`'s settings subcommand.
- Whether a Windows or macOS contributor can reach a daemon started outside
  the app (e.g. by running the CLI binary by hand alongside the app). The
  C ABI has no attach path — `tc_daemon_start` always hosts — so the app would
  fail on the lock rather than attach, but I did not trace what the shells
  render for that failure.
- The Windows and macOS settings screens were read as source, not run. The
  §5 finding is from the view code and the data it is given; I did not
  observe either app rendering it.
