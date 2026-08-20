# Fail-closed session roots across all three contributor apps

Status: design, not yet implemented.

## The problem

The same product either protects a developer's real source tree from being
scanned or does not, depending on which operating system they installed it on.

macOS refuses to start the watcher unless `daemon-settings.json` declares both
`claude_root` and `codex_root`. The rule is at
`macos/Sources/TraceCommonsApp/DaemonHost.swift:87-100`, and the comment above
it argues the case well: an unset root does not mean "no source", it means the
daemon watches the real `~/.codex`, so half a declaration buys none of the
protection while reading as though it had.

Linux and Windows have no such check anywhere. The GTK app opens a
`ConfigStore` and starts the embedded daemon with no inspection of the roots
at all (`crates/trace-commons-contributor-gtk/src/backend.rs:56-63`). The
Windows app constructs `new TcDaemon(_configDir)` with no settings
(`windows/src/TraceCommons.App/DaemonHost.cs:134`). In both cases
`DaemonSettings::claude_root` stays `None`, which
`crates/trace-commons-contributor/src/daemon/settings.rs:106-108` documents as
meaning "the conventional per-user location for that agent". The app starts
and watches the contributor's actual work.

Windows even carries a doc comment asserting the opposite of what its code
does. `windows/src/TraceCommons.Interop/TcDaemon.cs:88-92` says the caller is
expected to have written `daemon-settings.json` pointing the roots somewhere
deliberate, and that "`DaemonHost` does exactly that". `DaemonHost` does not.

Meanwhile the macOS refusal is not a working guardrail either — it is a dead
end. `AppModel.start()` (`macos/Sources/TraceCommonsApp/AppModel.swift:112-132`)
sets `startup = .refused` and returns without constructing a daemon or a
client. `MainWindowView` renders that as a centred notice
(`macos/Sources/TraceCommonsApp/Views/MainWindowView.swift:44-48`), and
onboarding only renders under `.running`. Every onboarding method — `enroll`,
`consent_options`, `list_projects` — is daemon IPC. So on a fresh macOS
install the app refuses, and the only screen that could clear the refusal is
behind the refusal.

Nothing in any of the three clients can write `daemon-settings.json`. The only
code in the tree that does is the FFI demo binary,
`macos/Sources/tc-ffi-demo/main.swift:73`, whose own comment says it is doing
what onboarding would do.

## What this is not

It is not server work, protocol work, or daemon-core work, and it does not
change the CLI. `trace-commons-contributor daemon` is an explicit, deliberate
invocation by someone who typed the command; the posture described here is an
application-shell posture, and the CLI keeps its current defaults.

It is not the icon pipeline and it is not the macOS Dock-icon or menu-bar
work. Those are separate slices with their own specs.

## The gap nobody has noticed

The obvious plan — refuse when roots are undeclared, and route the contributor
into the projects screen to declare them — does not work, because the projects
screen is not a roots screen and cannot become one.

Onboarding screen 5, "What to watch", lists projects the daemon has **already
discovered** through `list_projects` and sets a per-project mode through
`set_project_mode`. See
`macos/Sources/TraceCommonsApp/Views/OnboardingProjectsView.swift:3-25` and the
shared design at
`docs/superpowers/specs/2026-08-08-contributor-shell-shared-design.md:161-167`.
Discovery presupposes a running daemon that has already scanned the roots. It
is downstream of the very thing that has not happened yet.

Read the six screens in that shared spec in order — "What this is", "Connect",
"Consent scopes", "Extra privacy scan", "What to watch", "Done" — and there is
no screen at any point that asks which session folders to watch. The product
has a fully designed onboarding flow that never declares roots. That is the
root cause of `daemon-settings.json` having no writer: no screen was ever
specified to write it.

So this slice adds one: a roots-declaration step that runs **before** the
daemon starts, and therefore before any IPC is available to it.

## Where the rule should live

The rule belongs in Rust, once, rather than being transcribed into Swift, C#
and Rust separately.

The brief for this slice said "mirror the macOS rules" in the other two
clients. Three transcriptions of a security-relevant predicate is the same
shape of mistake the mark geometry already made (`BrandMark.swift`,
`gtk/src/ui/mark.rs`, `Controls/BrandMark.xaml.cs`), and it is the shape this
project has explicitly rejected before: `trace_commons.h:125-129` justifies
`tc_daemon_start_with_settings` sharing `set_settings`' validator precisely so
there is "one definition of a valid settings object rather than two that can
drift". The BOTH-not-either rule deserves the same treatment.

Concretely:

- **Rust, one implementation.** A predicate on the settings —
  `roots_declared()`, beside `apply_settings_object` in
  `crates/trace-commons-contributor/src/daemon/settings.rs:204` — that answers
  whether both `claude_root` and `codex_root` are `Some`. This is the only
  place the rule is written.
- **FFI enforces it at the start boundary.** `tc_daemon_start` and
  `tc_daemon_start_with_settings` consult it after any pre-start settings have
  been applied, and return `NULL` with a new fixed label `roots-not-declared`,
  alongside the existing labels at
  `crates/trace-commons-contributor-ffi/src/lib.rs:271` and `448-451`. This is
  a behaviour change to a shipped export and must be documented in
  `macos/Sources/CTraceCommons/include/trace_commons.h`; no signature changes.
- **GTK calls the same predicate directly.** The Linux app does not use the
  FFI — it links `trace_commons_contributor` and calls
  `daemon::start_embedded` in process
  (`crates/trace-commons-contributor-gtk/src/backend.rs:63`) — so `Backend::open`
  consults `roots_declared()` itself before starting.
- **macOS and Windows delete their local checks.** The Swift block at
  `DaemonHost.swift:87-100` goes away rather than being copied twice more.

The label has to be distinguishable from `daemon-start-failed`, because the
client must route a roots refusal to the roots screen and everything else to
the existing failure notice. `finish_daemon_start`
(`crates/trace-commons-contributor-ffi/src/lib.rs:381-396`) flattens every
error to `ERR_DAEMON_START_FAILED` today, which is why this needs its own
label rather than reusing that path.

Note that the daemon core is untouched: `start_embedded` itself does not gain
the refusal, so the CLI is unaffected.

## Per-platform work

### macOS

1. `DaemonHost.resolveConfigDirectory()`
   (`macos/Sources/TraceCommonsApp/DaemonHost.swift:67-102`) currently reads
   only `TRACE_COMMONS_CONTRIBUTOR_DIR` and throws `.noDirectory` when it is
   absent. A Finder-launched app never has a shell environment, so the shipped
   DMG always refuses. Give it the precedence Rust
   (`crates/trace-commons-contributor/src/config.rs:138-147`) and C#
   (`windows/src/TraceCommons.App/DaemonHost.cs:81-95`) already have: explicit,
   then the environment variable, then
   `~/Library/Application Support/trace-commons`. That is where the
   contributor's identity already lives, and the Homebrew cask's `zap` stanza
   already treats it as the state directory.
2. Bind the existing `tc_daemon_start_with_settings` in
   `macos/Sources/TCBridge/TCDaemon.swift`, matching the optional-settings
   shape Windows already uses at
   `windows/src/TraceCommons.Interop/TcDaemon.cs:107-114`. No Rust change and
   no ABI change is needed for this step; the export has been there since
   `crates/trace-commons-contributor-ffi/src/lib.rs:544`.
3. Add the roots screen and run it ahead of Connect in
   `OnboardingCoordinatorView`. Its Continue calls the new settings-bearing
   initializer, which persists the roots and starts the daemon in one step —
   `apply_pre_start_settings` saves durably before `start_daemon_handle`
   (`crates/trace-commons-contributor-ffi/src/lib.rs:563-576`), so the first
   supervisor tick already sees them.
4. Delete the stale comment at `DaemonHost.swift:6-12` and the local
   BOTH-not-either block at `DaemonHost.swift:87-100`.
5. Keep the socket-length pre-flight (`DaemonHost.swift:61-65`). It is a
   property of the resolved path, not of the settings, and the 104-byte budget
   still applies.

### Linux

1. `Backend::open` (`crates/trace-commons-contributor-gtk/src/backend.rs:56`)
   gains the `roots_declared()` check before `daemon::start_embedded`, and an
   optional settings object so the roots screen can supply them at start,
   mirroring the FFI's shape.
2. Add the roots screen to `crates/trace-commons-contributor-gtk/src/ui/onboarding.rs`.
3. The GTK settings view displays `claude_root_configured` today
   (`crates/trace-commons-contributor-gtk/src/ui/settings.rs:501-515`) but
   offers no way to change it. Once roots are declarable in onboarding, the
   same control belongs in settings.
4. The Unix-socket length budget applies on Linux too and has no equivalent
   pre-flight. Worth adding in the same pass.

### Windows

1. `DaemonHost` passes a settings JSON to `TcDaemon` when the roots screen
   supplies one; today it never does
   (`windows/src/TraceCommons.App/DaemonHost.cs:134`).
2. Add the roots screen to `windows/src/TraceCommons.App/OnboardingWindow.xaml`
   and `ViewModels/OnboardingViewModel.cs`.
3. Surface `roots-not-declared` distinctly from other `TcException` start
   failures.
4. Delete the false doc comment at
   `windows/src/TraceCommons.Interop/TcDaemon.cs:88-92`.

## Refusal copy

One sentence, same meaning on all three platforms. The existing macOS
`rootsNotDeclared` text (`macos/Sources/TraceCommonsApp/DaemonHost.swift:51-56`)
is close but was written for a dead end, so it ends on "Nothing is being
watched" with nowhere to go. The replacement has to name the way out, because
now there is one:

> This app hasn't been told which session folders to watch, and it won't guess.
> Nothing is being watched.

with the roots screen reachable directly from the notice. The refusal must not
name a path the contributor did not choose, and must not echo `settings_json`
back — `trace_commons.h:139-144` is explicit that settings text is the one
input at that boundary that may itself contain a filesystem path.

## Data flow

Fresh install, any platform:

    resolve state dir (explicit -> env -> per-user default)
      -> open ConfigStore (creates the directory, 0700 on unix)
      -> roots_declared()?  no
      -> roots screen: contributor names both folders
      -> start with settings {claude_root, codex_root}  (persists, then starts)
      -> roots_declared()?  yes
      -> daemon running; onboarding proceeds to Connect
      -> enroll / consent / privacy scan / what to watch / done

Existing install with roots already declared skips straight from
`roots_declared()` to running, which is the path every current macOS developer
build takes.

## Rejected alternatives

**A one-time migration that writes the implicitly-watched roots.** Every
existing Linux and Windows contributor is currently running with undeclared
roots, because neither client has ever had a way to declare them. Turning on
the refusal makes their next launch do nothing, and the obvious remedy is to
write `~/.claude` and `~/.codex` into `daemon-settings.json` on first upgrade
so behaviour is preserved. Do not do this, and do not "finish" it later. It
silently converts "we never asked" into "they declared it", which is exactly
the consent the fail-closed posture exists to require. The honest version is to
refuse once and route them into the roots screen — the same one-time friction a
new contributor gets.

**Dropping macOS to fail-open to match the other two.** Cheapest, and it would
have made the app work on first launch. It also deletes the only guardrail
stopping a contributor's real source tree from being scanned without them
asking, on the one platform that currently has it.

**Keeping the divergence and documenting it.** Defensible only if the macOS
check was a platform-specific stopgap for a missing onboarding flow, which its
own comment at `DaemonHost.swift:14-16` hints at. Once onboarding actually
declares roots, the stopgap has served its purpose and the posture should be
the product's, not one platform's.

**Mirroring the rule into Swift and C# as well as Rust.** Three copies of a
predicate that decides whether a developer's source tree gets scanned. See
"Where the rule should live".

**Having Swift write `daemon-settings.json` directly**, the way
`macos/Sources/tc-ffi-demo/main.swift:73` does. Smaller change, no ABI
involvement, but it puts a second writer on a file whose schema Rust owns and
duplicates root validation in a language that can drift from it.

## Verification

What CI can prove:

- The GTK refusal and the Rust predicate are unit-testable in the workspace and
  in the GTK crate's own test step. That crate is its own workspace, so the
  root test job never touches it; `.github/workflows/ci.yml:533-534` is the
  only thing that runs its tests.
- The FFI behaviour change is covered by the Windows interop tests, which run
  against the real cdylib (`.github/workflows/ci.yml:633`).
- The Linux shell builds and runs under weston in
  `.github/workflows/ci.yml:425`.

What CI cannot prove:

- **Nothing builds the Swift.** There is no macOS job in
  `.github/workflows/ci.yml` at all; macOS is built only by
  `.github/workflows/release-apps.yml` on a tag. Every macOS change in this
  slice has to be verified by hand with `macos/scripts/make-app-bundle.sh` and
  a launch of the resulting bundle — and launched from Finder, not from a
  shell, since a shell launch inherits `TRACE_COMMONS_CONTRIBUTOR_DIR` and
  would hide the exact defect being fixed.
- No UI test job exists for WinUI or for the GTK onboarding screens.
- CI never runs the Postgres suite, so a green CI has never been evidence about
  anything requiring real state. It is not relevant to this slice — no server
  code is touched — but it is the reason "CI is green" is not the standard here.

Manual gates, each on a real machine:

1. Fresh install, no state directory, no environment variable: the app starts,
   shows the roots screen, and completes onboarding through to Done. Run this
   on all three platforms.
2. State directory present with roots declared: no roots screen, watcher runs.
3. State directory present with exactly one root declared: refused. This is the
   case the `||` fail-open would have let through and the one most likely to
   regress.
4. Confirm on each platform that with roots undeclared, the daemon has not
   scanned anything — check that the queue is empty and no project list was
   built, rather than trusting the screen.

## Release notes

This lands in 0.4.0, which is stamped but deliberately untagged, so there is no
in-field upgrade to sequence for macOS. Linux and Windows contributors running
0.3.0 will see a behaviour change, and the notes must say so plainly: the app
now refuses to watch anything until you tell it which session folders to watch,
because until now it was defaulting to your real `~/.claude` and `~/.codex`
without asking.
