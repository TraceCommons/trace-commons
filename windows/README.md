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

- Withdrawal, credit and history views. The macOS app has roughly eighteen
  views; this has three.
- System tray presence and run-at-login.
- MSIX packaging and signing. The app builds unpackaged so that CI can verify
  it without a certificate.
- The rest of the queue frame the design specifies: the health banner and the
  week band. Both need daemon state the app does not read yet, so neither is
  drawn.
- Persistent recent searches. The preview sheet remembers search terms for the
  life of the process and writes none of them to disk; the macOS shell persists
  its own. A recent search is the contributor's list of the things they are
  worried about leaking, so keeping it in memory is a deliberate narrowing
  rather than an omission.

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
