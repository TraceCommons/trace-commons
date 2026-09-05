# Private inference in the contributor daemon

**Status:** approved design, 2026-09-05.

A contributor who upgraded to 0.9.0 with IronWire already running on the
machine saw nothing in the app. The daemon discovers IronWire's pointer, but
nothing is declared until someone finds a toggle under Settings > Tools and
presses Connect. The decision: **IronWire ships inside the contributor
daemon**, and the GUI offers it on first start.

## What the survey changed

The first draft of this design made the daemon a persistent user service on
macOS and Windows, because IronWire sits on the inference path and must
outlive a window. That prerequisite **does not exist**, and building it would
have fought code that is already here:

- `daemon/install.rs:1-13` states systemd-only is deliberate: "on macOS and
  Windows the native application is what registers as a login item, and
  having the daemon fight it for ownership of that registration would be
  worse than not offering it."
- macOS already registers the app at login for real —
  `LoginItemManager.swift` calls `SMAppService.mainApp.register()`, wired
  into onboarding and Settings, modelling "requires approval" as its own
  state.
- Windows already declares `<desktop:StartupTask TaskId="TraceCommonsStartup">`
  (`Package.appxmanifest:102-110`), with `StartupRegistration.cs` choosing
  between the packaged task and the HKCU Run key, and citing the Linux
  precedent: "Two mechanisms at once is how a contributor ends up with two
  copies starting."

So persistence is already solved by the app opening at login. The daemon
stays where it is, and IronWire goes inside it.

## The asymmetry, accepted deliberately

| | daemon lives in | private inference runs |
|---|---|---|
| Linux | systemd user service; GTK attaches as a client | always |
| macOS, Windows | the app process | while the app is open |

On Linux `backend.rs:153-170` already attaches to a running daemon over the
socket. On macOS and Windows the app **is** the daemon
(`AppDelegate.swift:59-73`: "This app is the daemon -- the watcher runs
in-process -- so quitting it stops the thing the contributor installed it
for"), so quitting stops the proxy.

Ruled: accept it. Closing it means `SMAppService.agent` beside
`mainApp` — the two-mechanism failure the Windows code warns about — plus a
matching Windows answer, for a case a login item already covers. The cost is
one sentence: the existing quit confirmation must say that quitting also
stops routing.

## What "yes" means

Separate declarations, as today:

1. **Start IronWire** — the daemon binds loopback 8463, serves, writes
   `endpoint.json`, and declares routing so the record is read.
2. **Route a tool through it** — per tool, under Tools, unchanged.

One yes does not repoint any agent. This matches the existing routing card
and keeps the traffic-changing step explicit per tool.

## The structural obstacle

**IronWire's startup assembly lives in its binary crate, not a library.**
`src/commands/serve.rs` builds `paths()`, `Config::load`, `ConsentLedger`,
`control_token()`, the backend registry, quota restore, the port lock, the
listener, the ledger, the catalog, the body store, the prune task, and only
then `AppState::new(..).with_port().with_paths().with_ledger().with_bodies()`
before `server::serve_on`. `ironwire_proxy::server::serve_on(listener, state,
shutdown)` is public and library-side; everything that produces its arguments
is not.

So the work starts upstream: **extract an embeddable seam in
nearai/ironwire** — one library entry point that takes a home directory and
returns a running proxy with a shutdown handle. Reimplementing the assembly
inside the contributor daemon would duplicate a dozen steps that drift.

## Home directory

The daemon uses **`~/.ironwire`**, not a private directory: the `ironwire`
CLI, if installed, then sees the same ledger, token and pointer, and the
existing ledger reader keeps talking HTTP to `127.0.0.1:8463` unchanged.

## Existing instance

If `endpoint.json` exists, probes, and the control token is not ours, the
state is `Running{theirs}`: offer the existing connect, never bind, never
stop it. Bind failure on a foreign process is `Failed{port_in_use}` — a
refusal with a way out. A proxy panic is contained in its task, surfaces as
`Failed{crashed}`, and never takes the daemon down.

## Dependency cost

`ironwire_proxy` brings **83 packages** not already in
`trace-commons-contributor` (223 in its tree, 140 shared). The substantive
additions are `axum` — an HTTP *server*, which the contributor does not have
today — and `rusqlite`/`libsqlite3-sys` for IronWire's ledger. All MIT OR
Apache-2.0, so the permissive boundary holds. `flatpak/cargo-sources.json`
must be regenerated in the same change or the next `app-v*` tag fails, since
nothing in PR CI validates it.

## Sub-projects

- **A — upstream:** an embeddable seam in nearai/ironwire. **Landed** as ironwire#26 (`b1ecde4f`) on 2026-09-05, larger than this spec anticipated — see the plan for the six things it got wrong.
- **B — the daemon hosts it**, behind `private_inference`, default off,
  settable over IPC. Ships working and headless.
- **C — the first-start offer** in three shells. Its own spec and plan, once
  B lands.

## Out of scope

Repointing agents (`ironwire init`'s job, still per-tool); bundling the
`ironwire` CLI binary; making the daemon a service on macOS or Windows;
attested inference UI, which remains IPC-only.
