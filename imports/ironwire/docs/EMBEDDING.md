# Hosting IronWire in another application

`ironwire_proxy::embed::start(home, port_override)` starts the same proxy assembly
used by `ironwire serve`. It prepares an empty home, loads existing configuration
and consent, acquires exclusive home ownership, binds loopback, restores quota
and spend, and starts the configured maintenance tasks. It does not repoint tools
or grant subscription, body-capture, or contribution consent.

```rust,no_run
# async fn example() -> Result<(), ironwire_proxy::embed::EmbedError> {
let home = std::path::Path::new("/path/to/.ironwire");
let mut proxy = ironwire_proxy::embed::start(home, None).await?;
let port = proxy.port(); // actual bound port; Some(0) requests an ephemeral port
// A host can select between its own shutdown request and proxy.wait().
// proxy.is_finished() supports hosts that poll their existing lifecycle loop.
proxy.shutdown().await;
# Ok(())
# }
```

Embedded starts use `UpdatePolicy::HostManaged`: the embedding application's
release process upgrades this library. They neither fetch standalone IronWire
release notifications nor load cached installer commands left by a prior CLI
run. The control status reports `update: {"state":"unknown"}`; the host can
present its own application update UI. The CLI explicitly selects
`UpdatePolicy::Standalone` through `start_with_policy`, preserving its existing
release checks and cached notifications. Signed provider-catalog refresh still
honors `updates.check`, and provider model discovery is unchanged for both hosts.

`UpdatePolicy` says who owns upgrading the binary; it does not say whether the
release check and the catalog refresh happen. That remains `updates.check`, whose
only home is `$IRONWIRE_HOME/config.toml` — a file an embedding host may not own,
because the home can be a real user's and shared with the CLI.
`start_with_options` takes the same switch in code:

```rust,no_run
# async fn example() -> Result<(), ironwire_proxy::embed::EmbedError> {
use ironwire_proxy::embed::{EmbedOptions, UpdateChecks, start_with_options};
let home = std::path::Path::new("/path/to/.ironwire");
let proxy = start_with_options(
    home,
    None,
    EmbedOptions::default().with_update_checks(UpdateChecks::Off),
    |_, _| {},
)
.await?;
# proxy.shutdown().await;
# Ok(())
# }
```

`UpdateChecks::Off` suppresses the release check and the catalog refresh, the two
requests IronWire makes that are not the user's own work; someone who declines one
means both. There is deliberately no value that turns them back on over a
configured `updates.check = false`. `UpdateChecks::FromConfig` is the default and
is what every existing entry point does, so a standalone install is unaffected.
`StartupReport::update_checks` reports the decision that was actually applied, so
a host can confirm it declined rather than assume it.

**This is not a general outbound kill switch**, and it is not the startup probe.
Startup catalogue discovery probes registered backends over the network under
either value, and a registered backend is not the same as a configured one:
`build_registry` registers the default Claude and Codex subscription logins and
the Anthropic/OpenAI key backends from credentials found in the environment with
no config entry naming them, and the NEAR AI backend is registered unconditionally so
that `privacy.mode = "full"` has a visible destination. Which backends are probed
is `StartupProbes`, below.

## The startup probe

At startup IronWire calls `Backend::probe` on each registered backend: a live
request that proves the backend works right now and learns the model catalogue the
provider actually serves. It is real work, and a host that declines it is making a
trade, not a saving. Declining means the daemon runs on configured or compiled-in
catalogue values, and an expired credential surfaces on the first real request
rather than at startup. `ironwire doctor` probes on demand and is unaffected.

The reason a host may want to decline is that it did not ask for all of these
requests. A bare embedded start against a home with no `config.toml` still asks
NEAR AI for its model catalogue, because that backend is registered whether or not
anything names it, and a host whose user happens to be logged into Claude Code or
Codex probes those providers too, once that subscription has the user's consent.
A subscription awaiting consent is never probed, under any value below: the
probe would send its token before the user said yes (`docs/TRUST.md` §2).

```rust,no_run
# async fn example() -> Result<(), ironwire_proxy::embed::EmbedError> {
use ironwire_proxy::embed::{EmbedOptions, StartupProbes, start_with_options};
let home = std::path::Path::new("/path/to/.ironwire");
let proxy = start_with_options(
    home,
    None,
    EmbedOptions::default().with_startup_probes(StartupProbes::Configured),
    |_, _| {},
)
.await?;
# proxy.shutdown().await;
# Ok(())
# }
```

- `StartupProbes::All` is the default and what the CLI does: probe everything
  registered and consented to. Nothing changes for a standalone install.
- `StartupProbes::Configured` probes only backends an entry in `config.toml`
  names. A host that declared its backends deliberately keeps the startup answer
  for those, and makes no request on behalf of one IronWire found on its own.
- `StartupProbes::Off` probes nothing. With `UpdateChecks::Off`, this is the only
  combination under which an embedded start makes no outbound request of its own
  accord.

The alternative for a host that wants none of this remains `enabled = false`
entries for each backend, which is again a configuration file it may not own.

## Backend credentials

Credential discovery is four lookups, not one. IronWire reads API keys from
variable names — `NEARAI_API_KEY`, `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, or
whatever `api_key_env` names for a configured backend — and it reads three
things off disk: the credential Claude Code writes, the credential Codex
writes, and the metered key Codex stores after `codex login --api-key`.

For an embedding host, the environment was the only channel into the first of
those and there was no channel into the other three. Reaching the environment
after startup means `std::env::set_var`, which is `unsafe` in Rust 2024 because
mutating the environment once threads exist is a data race with every thread
reading it — and an embedded start is inside a running Tokio runtime by
definition. It is also the wrong place for a secret in a shared process, where
every other library can read it.

`EmbedOptions::with_credentials` hands the whole question to the host:

```rust,no_run
# async fn example() -> Result<(), ironwire_proxy::embed::EmbedError> {
use ironwire_proxy::embed::{EmbedOptions, HostSecret, start_with_options};
# fn key_from_the_hosts_own_vault(_name: &str) -> Option<HostSecret> { None }
let home = std::path::Path::new("/path/to/.ironwire");
let proxy = start_with_options(
    home,
    None,
    EmbedOptions::default().with_credentials(key_from_the_hosts_own_vault),
    |_, _| {},
)
.await?;
# proxy.shutdown().await;
# Ok(())
# }
```

The closure is called with the variable name IronWire would otherwise read, so
a host answers per backend without knowing anything about how the registry is
built. It returns a `SecretString` (re-exported as `HostSecret`), which has no
`Debug` rendering and is zeroized on drop; the value stays a `SecretString` all
the way into the backend, and nothing about it is logged, reported by
`startup_report`, or rendered by `EmbedOptions`'s own `Debug`.

**This replaces credential discovery; it does not extend it.** With a source
supplied, nothing else is consulted: not the process environment, and not those
three files. `None` from the closure means there is no credential for that
name. An empty answer is the same as no answer.

The alternative — the host answers where it can, the environment fills the rest
— was rejected. Under it a stray `ANTHROPIC_API_KEY`, or a user's Claude Code
login sitting in their home, registers a backend the host never authorized:
the same surprise `StartupProbes::Configured` exists to prevent, one rung
earlier. A host that must be able to state which destinations are possible
cannot state it while IronWire is still finding its own. Either the host
manages credentials or IronWire does, never half of each.

The consequence is worth planning for. A host that answers only
`NEARAI_API_KEY` gets NEAR AI and nothing else — no subscription backend for
any login, however logged-in the user running it happens to be and however
many logins its `config.toml` declares. A
host that answers nothing, with the unconditional NEAR AI entry disabled, gets
an empty registry: `StartupReport::no_backends` is the field for that state,
and until now almost nothing could reach it. Check it.

### Whose answer counts, and whether the files are read

Those are two questions, and until recently one value answered both.
`CredentialSource` settles whose answer counts for a *name*; the credential
files are a different matter, because the subscription backends are key-less by
construction. Every Claude Code or Codex login — the default one, and each one
`config.toml` declares by directory — is a token in a file, not a value any
name-keyed source can supply. So a host that answers `NEARAI_API_KEY` cannot
answer for those, and deriving one question from the other left it unable to
say "the answer for this name is mine, and the user's own logins are still
fine": it had to give up every subscription the user already had in exchange for
supplying one key of its own.

`EmbedOptions::with_credential_files` answers the second question on its own:

```rust,no_run
# async fn example() -> Result<(), ironwire_proxy::embed::EmbedError> {
use ironwire_proxy::embed::{CredentialFiles, EmbedOptions, HostSecret, start_with_options};
# fn key_from_the_hosts_own_vault(_name: &str) -> Option<HostSecret> { None }
let home = std::path::Path::new("/path/to/.ironwire");
let proxy = start_with_options(
    home,
    None,
    EmbedOptions::default()
        .with_credentials(key_from_the_hosts_own_vault)
        // The host answers for its own key; the user's Claude Code or Codex
        // login goes on answering for itself.
        .with_credential_files(CredentialFiles::Discover),
    |_, _| {},
)
.await?;
# proxy.shutdown().await;
# Ok(())
# }
```

- `CredentialFiles::FollowCredentialOwner` is the default and changes nothing:
  the files are read when IronWire owns the names and not when a host does. A
  host that calls `with_credentials` and says nothing about files still reads
  none of them, exactly as above.
- `CredentialFiles::Discover` reads them whoever owns the names. Everything the
  replacement rule promises about *names* still holds — the process environment
  is not consulted, and a stray `ANTHROPIC_API_KEY` still registers nothing —
  but a subscription backend can now be registered from a login the host never
  named, and a request can go to it. That is the point of the value, and a host
  that must be able to state every possible destination should not select it.
- `CredentialFiles::Off` reads none of them even though IronWire owns the names.
  For a host content with the environment it started the process with, but not
  with a real user's home being read for logins it never asked about.

The metered key Codex stores after `codex login --api-key` is a file, so it
follows this switch rather than the source: it is the one credential that is
both a file and a key.

**The switch covers every login, not two.** A Claude or ChatGPT login that
`config.toml` declares with `config_dir` is read from that directory on every
request, so it is a file credential exactly like the default ones. With the
files off it is not registered at all — not registered and unauthenticated, but
absent — and with them on it is registered whether or not anyone has logged in
there yet, so `status` can say what to run. That is held by kind rather than by
the two default ids (`a_start_that_may_not_read_the_files_registers_no_login_of_any_kind`),
so a login added under any id is covered. A login switched off with
`enabled = false` is not read either.

There is no way to name a login directory in code. A host that wants a second
login declares it in `config.toml` like any other backend — a file it may not
own, as above — and finds it in the control API's status beside the rest, with
the `connect_command` that sets it up. `StartupProbes::Configured` probes it
once it has consent, because an entry names it.

Supplying no source is the default and discovers credentials exactly as before,
from the same places, at the same points, so the CLI and every existing
embedder are unaffected. Carrying a host's closure costs `EmbedOptions` its
`Copy`, `PartialEq` and `Eq` derives; `Clone` and `Default` remain, and the
`EmbedOptions::default().with_*` form above never used the others.

Run this inside a Tokio runtime and keep that runtime alive through shutdown.
The application owns the choice to start and stop; no signal handler, tracing
subscriber, or process exit handler is installed by the library. The CLI keeps
its terminal output, actionable port diagnostics, and Ctrl-C/SIGTERM handling.
`startup_report()` gives hosts the startup observations the CLI renders.
`start_with` optionally calls a synchronous announcement hook after successful
binding and assembly, before health can answer or background tasks start. The
CLI uses it to finish its startup instructions before reporting readiness.
The hook must return promptly and cannot wait for the proxy to serve a request.

`wait(&mut self)` observes final completion without giving away shutdown
ownership. It is cancellation-safe and can be read again after completion. It
returns fixed-label `ExitError::Server` or `ExitError::Task` on abnormal exits.
The supervisor waits for the server, cancels and joins housekeeping, flushes
quota, and then releases the pointer and home ownership. Task panics are
containable only with **panic unwinding**; `panic = "abort"` terminates the
process and cannot provide this guarantee. The IronWire CLI's release profile
currently uses abort, so a downstream host must choose its own panic policy.

`shutdown(self)` is graceful: in-flight model streams finish, while the control
event stream closes itself. There is no internal timeout that silently cuts a
model response. Dropping the handle, or canceling an in-progress shutdown
future, requests the same drain in the background; it does **not** release the
home lock early. Await `shutdown` when the host needs proof of cleanup. Dropping
the entire runtime cannot provide graceful completion.

The home contains the same ledger, token, consent, quota, and body files as the
CLI. `daemon.lock.guard` is a persistent lock inode and must not be removed while
an instance can be running. Its OS lock ends when the owner exits; the separate
`daemon.lock` remains a readable actual-port record for older CLIs. Startup also
probes a legacy owner's recorded port. A crashed legacy port record does not
prevent a new start. Older binaries do not participate in the new OS lock, so
this cannot make concurrent startup with an unmodified legacy binary atomic.

The legacy `daemon.lock` probe accepts only a nonzero decimal port in at most
1 KiB of text, including surrounding whitespace. Its health request goes to
numeric IPv4 loopback, ignores environment proxies, and does not follow
redirects. A successful response is advisory grounds to refuse takeover, not
an authenticated claim about the responder.

Legacy port and home-lock opens refuse final-component links/reparse points
and non-regular files. Unix files owned by a different effective user or writable by group or others
are refused based on metadata from the opened handle,
and newly created files use mode 0600. On macOS and Linux x86_64/aarch64, nonblocking opens also
prevent a planted FIFO from blocking before validation; unsupported Unix
platforms refuse this ownership path. This does not confine ancestor paths or
promise a wall-clock bound for arbitrary filesystems. Publication validates the
opened file before truncating it; it is not atomic publication. Cleanup checks
the published inode on Unix and port contents on every platform, keeping the
home lock until the published file handle explicitly closes. Windows cleanup retains the
legacy content check without a file-identity guarantee. A non-cooperating writer
can still replace a path between the cleanup check and deletion. These limits
concern `daemon.lock`; discovery `endpoint.json` has its separate ownership
protocol.

Embedded instances publish `home/endpoint.json` only. The CLI additionally
publishes the conventional `~/.ironwire/endpoint.json` for desktop discovery.
Cleanup checks that a pointer still describes this instance before removing it;
it does not deliberately remove a replacement owner's pointer. That check and
remove are not an atomic compare-and-delete against a non-cooperating writer.
An embedded instance in the conventional home is discoverable there normally;
a custom-home host must arrange any additional discovery explicitly.

Tokens use OS randomness on Unix and Windows. Unix homes and tokens retain modes
0700 and 0600. This extraction does not add Windows ACL hardening; the inherited
implementation relies on the user's profile-directory permissions. No new
package or package version is added: `anyhow`, `reqwest`, and `getrandom 0.2`
become direct uses of packages already in the proxy's dependency tree.

## Receipt proof: supplying the attestor

A NEAR AI receipt proves which model answered only if its ed25519 signing key
is bound by a **verified** TDX quote whose measurements are pinned. IronWire
checks the receipt itself -- signature, digests, model, `provider_tee` kind --
but carries no DCAP quote verifier, so on its own the best a row can reach is
`unattested` (`docs/DESIGN.md` §8).

A host that already verifies quotes supplies that last step:

```rust,ignore
let options = EmbedOptions::default()
    .with_signer_attestor(Arc::new(my_attestor)); // impl proof::SignerAttestor
```

`SignerAttestor::model_keys(backend, model)` answers with the per-model keys a
verified, measurement-pinned quote binds -- never the gateway key -- or says why
it cannot. Supplying one turns `capture.receipts` on for that start: request
and response digests are taken as the bytes stream past (no body is kept or
written unless `capture.bodies` is also on), and a background task settles each
NEAR AI row off the response path. `with_proof_settings` tunes its batch,
concurrency, retry budget and period.

## Pointing a coding tool at an embedded instance

`ironwire_agents::tools::plan_connect` works out the edit; the host shows it and
`commit`s it. For Claude Code the edit fills two slots: `env.ANTHROPIC_BASE_URL`,
which routes the tool here, and `statusLine`, which runs a command on every
render and shows its stdout in the status bar. The second one names a command,
and `ConnectOptions` says which:

```rust
use ironwire_agents::tools::{ConnectOptions, StatusLine};
// The host serves the line itself (below).
let hosted = ConnectOptions::default().with_status_line(StatusLine::Command(
    "/opt/host/bin/host ironwire-statusline --session 7".to_string(),
));
// The host has no command that can serve it.
let declined = ConnectOptions::default().with_status_line(StatusLine::Decline);
```

- `StatusLine::Offer` (the default, and what `plan_connect` uses) writes
  `std::env::current_exe()` plus a `statusline` subcommand. Under the CLI that is
  `ironwire statusline`, which exists. Under a host it is the host's own
  executable, which may not implement the subcommand at all, or may be a GUI
  bundle with no command-line surface. Claude Code renders the command's stdout,
  so a binary that rejects the argument produces a blank line rather than a
  visible failure.
- `StatusLine::Command(String)` writes the host's own command, verbatim, so it
  must already be a complete command line for the agent's shell (quote paths
  with spaces). A blank command declines.
- `StatusLine::Decline` writes none.

All three follow the same slot rules. The slot is filled only when it is empty,
and a status line the user wrote themselves is left alone and reported as
occupied. A line IronWire installed is marked `"installedBy": "ironwire"`, which
is how a later connect knows to replace it when the command changes and how
`plan_disconnect` removes it and nothing else. Routing is the same edit
whichever is chosen.

Declining also removes a status line IronWire installed on an earlier connect,
so a host that shipped `plan_connect` before adopting this fixes the
installations it already made rather than only the next one; switching to
`Command` replaces that line with the host's. The `installedBy` marker names
IronWire and not a particular binary, so in a home shared with the `ironwire`
CLI either choice replaces or removes the CLI's line as well; `ironwire connect
claude` puts it back.

### Serving the status line from a host

The line `ironwire statusline` prints is rendered by
`ironwire_proxy::statusline`, so a host can print the same bytes from its own
executable. Expose a subcommand, install it with `StatusLine::Command`, and have
it do this:

```rust
use ironwire_proxy::statusline;

// Inside the host's `ironwire-statusline` subcommand, on a Tokio runtime.
let model = statusline::client_model_from_stdin();
if let Some(line) = statusline::line_for_home(&session_home, model.as_deref()).await {
    println!("{line}");
}
// Exit 0 either way.
```

`line_for_home` takes the IronWire home the host passed to `embed::start` for
that session. It reads the port from `<home>/endpoint.json`, which the proxy
publishes once it has bound (so an ephemeral port is found), falling back to
the home's configured port when no pointer is there, and reads
`<home>/control.token`. It creates and writes nothing: no home, no token. A
pointer that names anything other than `127.0.0.1` is not followed. A host that
already holds the port and token calls `statusline::line(port, &token, model)`
instead, and one that already holds a `StatusView` calls
`statusline::render(&status, model)`, or `render_at` with an explicit clock.

The helpers keep the three properties the CLI has always had. They never block
the render loop: the whole status request, connection included, is bounded by
`statusline::BUDGET` (400 ms). They never invent: every value printed comes
from the daemon's control API. They are quiet when things are normal, and every
failure (no pointer, no token, refused, wrong token, too slow, unparseable) is
`None`, never an error. The subcommand should therefore print nothing and exit
0 when the helper returns `None`.

The command line the host installs has to identify the session when the host
runs one proxy per home: the stdin document names Claude Code's session, not
IronWire's home. Put the home, or an id the host can map to one, in the
command's arguments.

## Implementation checkpoint

This completes upstream Task 1 of the [Trace Commons private-inference
plan](https://github.com/TraceCommons/trace-commons/pull/609).

- [x] Write lifecycle tests and observe the missing-module compile failure.
- [x] Move startup assembly and its existing helper tests into the library.
- [x] Rewire the CLI to use the same assembly and retain terminal diagnostics.
- [x] Cover ephemeral startup, empty homes, health, owner-only permissions,
      competing/concurrent starts, stale locks, failed-start cleanup, pointer
      ownership, handle drop, graceful stream draining, and observable completion.
- [x] Prove the home-lock test fails when acquisition is removed, then restore it.
- [x] Prepare the verified upstream PR for review.

Warnings-denied `cargo test --all-features --locked --offline` passes with 948 tests and two existing ignored tests. All-target/all-feature Clippy, formatting, and diff checks pass.

The mutation produced `FAILED. 0 passed; 1 failed` at the assertion that the
second start returns `EmbedError::Lock`. Restoring acquisition produced
`ok. 1 passed; 0 failed`. The empty-home test runs in a child process with an
isolated credential environment and blocked external HTTP; other lifecycle tests
disable provider/update discovery and use loopback fixtures only.

**Task 2 has not started.** It must wait for this upstream PR to merge and pin
the merged revision. Its host should use `wait`/`is_finished` for unexpected
completion, keep the runtime alive through draining, respect another instance's
ownership, and explicitly choose a panic policy. The GUI offer remains Task 3,
a separate plan after daemon integration.

Startup diagnostics preserve IronWire's local housekeeping error context. The
host-facing registry refusal carries only a fixed backend construction label,
not the underlying error chain. `StartupReport::home` is the canonical home used
for discovery, including when the caller supplied a symlink. A CLI lock refusal
confirms health before claiming that the recorded port is already running;
an owner still starting or stopping is reported separately.
