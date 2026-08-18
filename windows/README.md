# The Windows contributor app

A WinUI 3 shell over the same `trace-commons-contributor-ffi` C ABI the macOS
app uses. It hosts the daemon in-process rather than shipping a second binary,
matching what `macos/` does and what the ABI was built for.

## Layout

| Path | What it is |
| --- | --- |
| `src/TraceCommons.Interop/` | The C ABI binding. Targets plain `net8.0`. |
| `src/TraceCommons.App/` | The WinUI 3 shell. Targets `net8.0-windows`. |
| `tests/TraceCommons.Interop.Tests/` | Interop tests, including live ones against a real daemon. |
| `scripts/` | The GCE Windows dev box: provisioning, remote exec, screenshot capture. |
| `docs/dev-vm.md` | How to build, run, and see the app on that box. Read it before touching the WinUI half. |

### Why the interop layer is not a Windows project

`TraceCommons.Interop` deliberately targets `net8.0`, not `net8.0-windows`.
Nothing in it touches WinUI or WinRT — it is P/Invoke against a cdylib whose
filename .NET decorates per platform — so the same assembly and the same tests
run against a macOS `.dylib` or a Linux `.so` build of the identical Rust
crate.

That is what makes the risky half of this app testable without Windows.
Pointer ownership, UTF-8 marshalling, delegate rooting and the unsubscribe
barrier are all exercised on a developer machine, and CI then confirms the same
binding holds on Windows.

## Building and testing

The Rust cdylib must exist first; the app project fails its build if it is
missing rather than deferring that to a runtime error.

```bash
# From the repository root.
cargo build -p trace-commons-contributor-ffi            # debug, for the tests
cargo build -p trace-commons-contributor-ffi --release  # release, for the app

# Interop tests. These run on macOS and Linux as well as Windows.
dotnet test windows/tests/TraceCommons.Interop.Tests/TraceCommons.Interop.Tests.csproj

# The WinUI app. Windows only.
dotnet build windows/src/TraceCommons.App/TraceCommons.App.csproj -p:Platform=x64
```

`TC_FFI_LIB_DIR` overrides where the cdylib is looked for. It defaults to
`target/debug` for the tests and `target/release` for the app.

## Things the ABI requires that are easy to get wrong

All four are enforced in `TcDaemon`; they are listed here because each one
fails silently rather than loudly if a future change drops it.

1. **Owned returns must not be marshalled as `string`.** The CLR would free a
   `char*` return with `CoTaskMemFree`, which is not the allocator Rust used.
   Every owned return crosses as `IntPtr` and is released with
   `tc_string_free`.
2. **The subscribe delegate needs its own GC root.** Native code holding a
   function pointer does not keep the managed delegate alive. Both the delegate
   and the ctx box are rooted until `tc_unsubscribe` is *confirmed*.
3. **`tc_unsubscribe` refuses silently.** It returns `void` and declines when
   called from a thread inside any tokio runtime context, so success has to be
   inferred by comparing `tc_last_error` across the call. Assuming success frees
   ctx while callbacks can still fire.
4. **`tc_last_error` is thread-local.** An `await` between a failing call and
   the error read can resume on another pool thread and report nothing. Every
   read is on the calling thread with no await in between.

Teardown leaks rather than frees when it cannot prove the handle is idle. That
is deliberate and is not a bug to fix: the process is exiting, an unfreed handle
costs nothing, and a use-after-free is a crash or worse.

## A macOS-only test-harness trap

On Unix the daemon serves IPC over a unix domain socket inside its config
directory, and macOS caps `sun_path` at 104 bytes. A fixture using `$TMPDIR`
(48 characters on macOS) plus a nested folder and a 32-character GUID overruns
that cap once the socket filename is appended, and **every daemon start fails
with the opaque label `daemon-start-failed`** — nothing in the error points at
path length.

`NativeRoundTripTests.ShortTempDir` keeps the path short for this reason.
Windows is unaffected, since its transport is a named pipe.

## What is not here yet

Deliberately absent, and each is its own piece of work:

- Bulk withdrawal. This is a refusal rather than a gap, and it is the only
  affordance the shared design draws that this app states in words instead of
  drawing. `withdraw_bulk` reports only `withdrawn` and `failed` counts, so
  afterwards there is no per-trace tier to report — and the withdrawal
  contract's first rule is that no outcome may be reported as a generic
  "withdrawn". A bulk button could not honour it at any wording. The held
  group says so where a contributor would look for the button, and points at
  the per-row control that *can* tell them what it did.
- The rest of Settings. The rail's third row exists now and carries the public
  profile panel (see "Claiming a public handle"); the watcher knobs, the
  connection section and the per-project list the spec also puts on that screen
  are absent from the screen rather than drawn disabled, for the same reason
  the row itself was absent until now.
- The rest of the tray menu's vocabulary. The icon, the tooltip, the digest
  and run-at-login are here (see "The tray and the interruption budget"); what
  the shared spec also puts in that menu — pause with its three durations, the
  week summary, the per-project list of what is waiting, settings — is not,
  because those surfaces do not exist in this app yet and a menu item that
  opens nothing is worse than an absent one.
- MSIX packaging and signing, *as a shipping artifact*. The manifest, the
  packaging script and the signing path now exist under `packaging/` and
  `scripts/make-msix.ps1`, but none of it has ever been built: it is opt-in
  (`TcPackaged=true`), reachable only from a `workflow_dispatch` on
  `release-apps.yml`, and it needs a real icon before anyone should install it.
  The app still builds unpackaged by default so that CI can verify it without a
  certificate, and the shipping artifact is still the zip. Read
  `packaging/README.md` before touching any of it — particularly the part about
  the publisher string and about what packaging does to the state directory.
- The rest of the queue frame the design specifies: the health banner and the
  week band. Both need daemon state the app does not read yet, so neither is
  drawn.
- Persistent recent searches. The preview sheet remembers search terms for the
  life of the process and writes none of them to disk; the macOS shell persists
  its own. A recent search is the contributor's list of the things they are
  worried about leaking, so keeping it in memory is a deliberate narrowing
  rather than an omission.

## The tray and the interruption budget

The onboarding "Done" screen tells a contributor they "will get at most one
notification every 4 hours, and none at all if there's nothing waiting". Until
this slice that was a promise about a notification path that did not exist.
Now it exists, and the promise is enforced in two places, neither of which can
cause a notification:

1. **The daemon.** `daemon/notify.rs::digest_due` refuses on an empty queue and
   otherwise fires once per `digest_interval_secs`, persisting `last_digest_at`
   so the spacing survives a restart. This is the shared policy every shell
   obeys; delivery is the `digest_due` subscription event.
2. **`TraceCommons.Interop.DigestCadence`.** A second, in-process gate for the
   ways a shell can over-notify with a correctly-behaving daemon behind it: a
   resubscribe that replays, a duplicate handler, a future caller posting a
   digest from somewhere other than the event. Claim-and-stamp is one call, so
   the only way to be told yes is to have consumed the window.

**Nothing reachable from the tray or a notification approves or sends
anything.** Clicking the icon or the digest raises the window; the menu opens
the window, toggles run-at-login, and asks to quit. That is the same rule
`gtk/src/tray.rs` and `gtk/src/notify.rs` hold, for the same reason: a misfire
on a surface the contributor is not looking at ships real transcripts and is
unrecoverable.

The digest is a `Shell_NotifyIcon` balloon rather than a toast with buttons.
The spec's `[ Review ] [ Not now ]` becomes click-the-balloon and ignore-it;
`Not now` "does nothing but dismiss" anyway, and a richer toast needs an
activation identity this unpackaged app does not have. Worth revisiting with
MSIX.

Quitting warns first, from the window's close button as well as from the tray.
On Windows the app HOSTS the daemon in-process, so quitting stops the watcher,
and the shared spec is explicit that saying the Linux sentence here would be
"a lie about whether the machine is still watching".

### Run at login

`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, written by
`RunAtLogin`, toggled from the tray menu, opt-in. HKCU and never HKLM: this
app installs per user with no administrator rights, and every other Windows
mechanism costs elevation or a package — a Scheduled Task, a service, or the
MSIX `windows.startupTask` extension. Same hive and same reasoning as
`UrlSchemeRegistration`. The entry appears in Task Manager's Startup tab,
which is where a contributor audits what starts with their machine; Windows
can disable it there, and this class does not fight that.

Inert under MSIX. The packaged flavour (`TcPackaged=true`) disables registry
virtualization, so the write would be real and would record a path inside
`WindowsApps` — the same reason `UrlSchemeRegistration` skips its own
registration when packaged. A packaged build declares startup with a
`windows.startupTask` manifest extension instead; that extension is not in
`packaging/Package.appxmanifest` yet, so a packaged build has no run-at-login
and `RunAtLogin.IsSupported` reports false, which drops the item from the tray
menu rather than drawing a toggle that cannot work.

### What only Windows can confirm

The mark's rasterization, the tooltip and digest wording, the icon-state
precedence, the cadence, and the Run-key value and quoting are all unit-tested
off Windows in `tests/TraceCommons.Interop.Tests/TrayTests.cs`. What those
tests cannot reach: that `Shell_NotifyIcon` accepts the struct, that the shell
draws the bitmap, that `TrackPopupMenu` dismisses correctly, that the balloon
appears, and that the registry entry actually starts the app at sign-in.

## The read gate

The preview sheet is the only surface that can approve anything, and the queue
row has no `Contribute` button — approving from the row is approving without
looking. Contribute is armed by `TraceCommons.Interop.ReadGate`, which requires
three things at once: a pinned preview, the redacted transcript having actually
been on screen, and an acknowledgement the contributor ticks themselves.

The gate lives in the interop assembly rather than in a view model because it
is the safety property of this shell, and there it is exercised by
`tests/TraceCommons.Interop.Tests/PreviewTests.cs` on a machine that cannot
build WinUI at all. The XAML wiring that feeds it — `x:Load` on the transcript
panel, so realization means display rather than a collapsed element having
raised `Loaded` — is the part only Windows can confirm.

## Withdrawal copy is contract, not UI text

History is backed by `list_history`, `history_rollup`, `refresh_history` and
`queue_outcome_counts`, and the one thing on it a contributor can *do* is
`withdraw`. That last one is the reason this section exists.

The three confirmation bodies are not this shell's to write. They are fixed in
`docs/contributor-daemon-ipc-v1_1.md` under "Canonical confirmation copy",
transcribed word for word into `TraceCommons.Interop.WithdrawCopy`, and
compared whole against that table by
`tests/TraceCommons.Interop.Tests/WithdrawCopyTests.cs`. The Linux shell holds
the identical constants in
`crates/trace-commons-contributor-gtk/src/copy.rs`; the two must not diverge.

**The tier is not knowable before the call.** The server computes
`distribution_reach` *during* the withdrawal, from live export membership, and
the confirmation has to be shown before that response exists. All this machine
holds is the record's local `status`, so:

| local status | shown before the call |
| --- | --- |
| `submitted`, `quarantined` | the `not_distributed` body alone — that is the server's own rule |
| `accepted` | **both** commons bodies, the distributed one weighted, and a sentence saying the outcome is decided on the server |
| anything else | the `commons_distributed` body alone — the furthest reach cannot be ruled out |

Afterwards the row reports the tier the server actually applied, using that
tier's body. Never a generic "withdrawn".

Two consequences worth knowing before touching this code:

- **A withdrawn record stays on the list and reads as withdrawn.** It is never
  dropped and never re-labelled as something that failed, and on success
  history is re-read rather than the row optimistically flipped. The tier the
  server applied is held per submission across that re-read, because
  `list_history` reports a status and never a tier — losing it would break the
  never-a-generic-withdrawn rule by way of a refresh.
- **`withdraw` currently always answers `account-session-required`.** The
  daemon holds a device key and never an account session, deliberately, so
  withdrawal survives losing the device that submitted the trace. That makes
  the failure path the one contributors actually hit, so it renders the whole
  explanatory sentence rather than a bare label — and, like every failure
  branch here, opens by saying nothing was withdrawn and nothing was deleted.

Everything above is decided in the interop assembly and tested off Windows.
What only a real Windows box can confirm is that the `ContentDialog` shows,
that the weighted body is visibly the heavier of the two, and that the nav rail
switches panes.

## Claiming a public handle

The rail's Settings row carries one panel: the public profile from section 5.6
of the shared design spec. It is backed by `get_public_profile`,
`set_public_profile` and `clear_public_profile`, all three of which were
already in the daemon's pinned `METHODS` array — the gap on Windows was never
protocol, only that nothing here asked.

Three things on it are contract rather than layout.

- **`handle_persisted` is not whether the claim worked.** By the time that flag
  exists at all the server has already taken the handle; it reports only
  whether the daemon managed to write its own local copy afterwards. So a
  claim with `handle_persisted: false` is reported as **published**, and the
  false branch adds only the weaker thing that is true — that this window will
  show the contributor as unlisted again until the next successful save, and
  that nothing about what is public changed. Telling someone their handle did
  not go up when it did is a false statement about an outward-facing act, and
  it is the one error this surface must never make.
  `PublicProfileCopyTests.AProfileThatWasPublishedNeverReadsAsOneThatWasNot`
  pins it as an invariant rather than as a string: both sentences must open
  "You're on the roster" and neither may contain the vocabulary of a refusal,
  so the copy stays free to be reworded and not to be reversed. The Linux
  shell asserts the same properties in `copy.rs`.
- **The claim is not gated on the local consent-scope list.** The server
  authorizes the `PUT` against the grant ceiling on the claim, not against the
  scopes this device happens to have recorded. The local set can be narrower
  than what the credential carries, so refusing here would refuse contributors
  the server would have allowed. The daemon makes the same choice explicitly,
  and so do the CLI and the other two shells.
- **The words are the Linux shell's, verbatim.** The shared design spec
  specifies the consent-scope checkbox and nothing else about this surface, so
  `crates/trace-commons-contributor-gtk/src/copy.rs` is the source of truth and
  `PublicProfileCopy` mirrors it — dashes included. macOS mirrors the same
  constants in `PublicProfileCopy.swift`. Two shells that word an
  outward-facing consent action differently are two different promises about
  what becomes public, so a change to one belongs in all three.

Going public keeps its acknowledgement gate: nothing is pre-checked, and
`Go public` stays disabled until the box is ticked and there is a handle to
claim. Leaving the roster is not gated, because it withdraws a consent rather
than granting one.

What only a real Windows box can confirm here is that the go-public
`ContentDialog` lays out its two columns, that a refusal keeps that dialog open
beside the field it is about, and that the rail's third row selects.
