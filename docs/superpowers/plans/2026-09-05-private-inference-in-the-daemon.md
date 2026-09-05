# Private Inference In The Daemon — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Run IronWire inside the contributor daemon behind an off-by-default switch, so private inference becomes something the app can offer rather than something a contributor must discover.

**Architecture:** IronWire's startup assembly lives in its binary crate, so the work starts upstream (sub-project A): one library entry point in `nearai/ironwire` that takes a home directory and returns a running proxy with a shutdown handle. The contributor daemon then owns one instance of it (sub-project B), driven by a `private_inference` setting, using `~/.ironwire` as its home so the CLI and the existing ledger reader are unaffected. The GUI offer is sub-project C, its own plan once B lands.

**Tech Stack:** Rust; `axum` (via `ironwire_proxy`), `rusqlite` (via `ironwire_ledger`), `tokio`; the daemon's existing NDJSON IPC.

**Spec:** `docs/superpowers/specs/2026-09-05-private-inference-in-the-daemon-design.md`

## Global Constraints

- Verify with `RUSTFLAGS='-D warnings'`. Plain `cargo check` does not apply it; CI does.
- `cargo --workspace` misses two configurations CI gates. After ANY change to `-contributor`, also run the four permissive crates with `--no-default-features` and the GTK workspace with `--manifest-path crates/trace-commons-contributor-gtk/Cargo.toml`.
- Clippy allow-list, verbatim: `-A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`.
- No emojis. Commit subjects short and imperative, no `feat:`/`fix:` prefix.
- **Hash-only logging.** IronWire proxies prompts. Nothing this plan adds may log a prompt, completion, token, control token, or body. Fixed labels, counts, ports and durations only.
- License boundary: `-contributor` is MIT OR Apache-2.0. `ironwire_proxy` and its tree are MIT OR Apache-2.0, so the boundary holds. Never edit `license_boundary.rs`.
- **The dependency is approved and bounded:** `ironwire_proxy` adds 83 packages to `trace-commons-contributor` (223 in its tree, 140 already shared), notably `axum` and `rusqlite`/`libsqlite3-sys`. Adding anything beyond that tree needs separate approval.
- **`flatpak/cargo-sources.json` must be regenerated** in the task that adds the dependency. Nothing in PR CI validates it; the first failure is the `linux-flatpak` job on an `app-v*` tag.
- The daemon uses `~/.ironwire` as IronWire's home — never a private directory.
- `private_inference` defaults to **false** and never turns itself on.

---

### Task 1 (sub-project A, nearai/ironwire): an embeddable seam

The assembly that produces `serve_on`'s arguments lives in the binary crate (`src/commands/serve.rs`), so no library consumer can start IronWire. This extracts it, changing no behaviour: the binary keeps its output and its `port_in_use` diagnostics by calling the new seam.

**Repo:** `/Users/zakimanian/code/ironwire` (separate from trace-commons).

**Files:**
- Create: `crates/ironwire_proxy/src/embed.rs`
- Modify: `crates/ironwire_proxy/src/lib.rs` (declare the module)
- Modify: `src/commands/serve.rs` (call the seam instead of inlining the assembly)
- Test: `crates/ironwire_proxy/tests/embed.rs`

**Interfaces:**
- Consumes: `ironwire_proxy::state::AppState`, `server::{bind, serve_on, ServeError}`, and the crates `serve.rs` already uses (`ironwire_core::config::Config`, `ironwire_creds::ConsentLedger`, `ironwire_ledger::{Ledger, bodies::BodyStore}`, `ironwire_catalog::CatalogStore`, `ironwire_upstream::*`).
- Produces:
  ```rust
  pub struct EmbeddedProxy { /* private */ }
  impl EmbeddedProxy {
      pub fn port(&self) -> u16;
      pub async fn shutdown(self);              // graceful; returns when the server has stopped
  }
  pub enum EmbedError {                          // carries no prompt, token, or body
      Paths, Config, Lock { port: u16 }, PortInUse { port: u16 }, Bind, Registry,
  }
  pub async fn start(home: &std::path::Path, port_override: Option<u16>)
      -> Result<EmbeddedProxy, EmbedError>;
  ```
  Task 2 calls `start` and holds the `EmbeddedProxy`.

- [ ] **Step 1: Write the failing test**

`crates/ironwire_proxy/tests/embed.rs`:

```rust
//! The embeddable seam, exercised the way a host application uses it.

/// A host can start the proxy against a scratch home and stop it, and the
/// port it reports is the port it is listening on.
///
/// Port 0 asks the OS for a free one, so this test cannot collide with a
/// developer's own IronWire on 8463 -- which is exactly the case the seam
/// exists to coexist with.
#[tokio::test]
async fn a_host_can_start_and_stop_the_proxy_on_an_ephemeral_port() {
    let home = tempfile::tempdir().expect("a temp home");
    let proxy = ironwire_proxy::embed::start(home.path(), Some(0))
        .await
        .expect("the proxy starts against an empty home");

    let port = proxy.port();
    assert_ne!(port, 0, "the reported port must be the bound one, not the request");

    let url = format!("http://127.0.0.1:{port}/_ironwire/health");
    let response = reqwest::get(&url).await.expect("the health route answers");
    assert!(response.status().is_success());

    proxy.shutdown().await;

    // After shutdown the port is free: binding it again succeeds.
    let rebound = tokio::net::TcpListener::bind(("127.0.0.1", port)).await;
    assert!(rebound.is_ok(), "shutdown must release the port");
}

/// An empty home is a working home. A host that has never run IronWire must
/// not have to pre-create a config, a ledger, or a token.
#[tokio::test]
async fn an_empty_home_needs_no_preparation() {
    let home = tempfile::tempdir().expect("a temp home");
    let proxy = ironwire_proxy::embed::start(home.path(), Some(0))
        .await
        .expect("an empty home is enough");
    assert!(home.path().join("control.token").exists(), "the token is created");
    proxy.shutdown().await;
}

/// Two hosts cannot serve one home. The second start refuses by name rather
/// than racing for the port or the ledger.
#[tokio::test]
async fn a_second_start_against_the_same_home_is_refused() {
    let home = tempfile::tempdir().expect("a temp home");
    let first = ironwire_proxy::embed::start(home.path(), Some(0))
        .await
        .expect("the first start succeeds");

    let second = ironwire_proxy::embed::start(home.path(), Some(0)).await;
    assert!(
        matches!(second, Err(ironwire_proxy::embed::EmbedError::Lock { .. })),
        "the second start must refuse with Lock"
    );

    first.shutdown().await;
}
```

Add `tempfile` and `reqwest` to `ironwire_proxy`'s `[dev-dependencies]` if absent; both are already in the workspace.

- [ ] **Step 2: Run it and watch it fail**

```bash
cd /Users/zakimanian/code/ironwire
cargo test -p ironwire_proxy --test embed
```

Expected: compile error — `ironwire_proxy::embed` does not exist.

- [ ] **Step 3: Extract the assembly**

Create `crates/ironwire_proxy/src/embed.rs` by **moving** the body of `src/commands/serve.rs::run` up to and including the `AppState::new(..)` builder chain, plus its helpers (`build_registry`, `restore_quota`, `open_ledger`, `open_bodies`, `sweep_bodies`, and the prune spawn), changing only what must change:

- Take `home: &Path` instead of calling `paths()`; build `PathsConfig` from it the way `paths()` does.
- Return `EmbedError` variants instead of `anyhow::bail!` — each variant carries at most a port. Keep the *conditions*: the `limits`-without-`capture` bail becomes `EmbedError::Config`.
- Return the `EmbeddedProxy` instead of awaiting `serve_on` to completion: hold the lock guard, the `JoinHandle` from `tokio::spawn(serve_on(listener, state, shutdown_rx))`, the bound port, and a `oneshot::Sender` for shutdown.
- `shutdown(self)` sends on the oneshot and awaits the `JoinHandle`.

Module doc:

```rust
//! Starting the proxy from inside another application.
//!
//! Everything here was `src/commands/serve.rs`, which is a *binary* crate: no
//! library consumer could produce `serve_on`'s arguments, so a host could not
//! start IronWire without reimplementing a dozen steps that would drift.
//!
//! The binary now calls this too, so there is one assembly rather than two.
//! What stays in the binary is what only a terminal wants: the printed
//! summary and the `port_in_use` diagnostic that inspects the other process.
//!
//! Errors here carry a port at most -- never a prompt, a token, or a body.
```

- [ ] **Step 4: Rewire the binary to call the seam**

In `src/commands/serve.rs::run`, replace the extracted body with a call to `ironwire_proxy::embed::start(&home, port_override)`, mapping `EmbedError::PortInUse` to the existing `port_in_use(port).await` diagnostic and the rest through `anyhow::Error`. Keep every line the binary prints today. Then await the proxy until the shutdown signal, and call `shutdown().await`.

- [ ] **Step 5: Verify, including that the binary is unchanged in behaviour**

```bash
cd /Users/zakimanian/code/ironwire
RUSTFLAGS='-D warnings' cargo test -p ironwire_proxy --test embed
RUSTFLAGS='-D warnings' cargo test --all-features
cargo fmt --all -- --check
RUSTFLAGS='-D warnings' cargo clippy --all-targets --all-features
```

Expected: the new tests pass and the existing suites are unchanged — `passthrough`, `verbatim_bodies`, `rolling_bodies` and the settings tests all exercise the binary's behaviour through the same assembly.

- [ ] **Step 6: Mutation — prove the second-start test bites**

Temporarily drop the lock acquisition from `embed::start`. Expected: `a_second_start_against_the_same_home_is_refused` FAILS (the second start succeeds). Revert, re-run, paste both outputs.

- [ ] **Step 7: Commit and open a PR**

```bash
git checkout -b embeddable-proxy
git add -A
git commit -m "Let another application start the proxy

The assembly that produces serve_on's arguments -- paths, config, consent,
control token, backend registry, lock, listener, ledger, catalog, body
store, prune task -- lived in the binary crate, so no library consumer
could start IronWire without reimplementing a dozen steps that would
drift.

Moved to ironwire_proxy::embed, which returns a running proxy and a
shutdown handle. The binary calls it too, so there is one assembly rather
than two; what stays behind is what only a terminal wants, the printed
summary and the port-in-use diagnostic."
git push -u origin embeddable-proxy
gh pr create --repo nearai/ironwire --base main --fill
```

**This PR must merge before Task 2 starts** — Task 2 depends on the published seam.

---

### Task 2 (sub-project B): the daemon hosts IronWire

**Files:**
- Modify: `crates/trace-commons-contributor/Cargo.toml` (the dependency)
- Modify: `crates/trace-commons-contributor-gtk/flatpak/cargo-sources.json` (regenerate)
- Create: `crates/trace-commons-contributor/src/daemon/private_inference.rs`
- Modify: `crates/trace-commons-contributor/src/daemon/mod.rs` (declare the module; own the instance)
- Modify: `crates/trace-commons-contributor/src/daemon/settings.rs` (the switch)
- Modify: `crates/trace-commons-contributor/src/daemon/ipc.rs` (the `set_settings` key and the reported state)
- Modify: `docs/contributor-daemon-ipc-v1_1.md` (the new key — a test enforces this)
- Test: in `private_inference.rs`'s `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `ironwire_proxy::embed::{start, EmbeddedProxy, EmbedError}` from Task 1.
- Produces:
  ```rust
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub enum PrivateInferenceState {
      Off,
      Running { port: u16 },
      RunningElsewhere { port: u16 },   // someone else's IronWire, not ours
      Failed { label: &'static str },   // "port_in_use" | "start_failed" | "crashed"
  }
  pub fn ironwire_home() -> Option<PathBuf>;   // $IRONWIRE_HOME, else ~/.ironwire

  pub struct PrivateInference { /* private */ }
  impl PrivateInference {
      pub fn new(home: PathBuf) -> Self;              // port from IronWire's config
      pub fn with_port(home: PathBuf, port: u16) -> Self;  // tests use 0 for ephemeral
      pub fn state(&self) -> PrivateInferenceState;
      pub async fn apply(&mut self, on: bool);        // idempotent both ways
  }
  ```
  Task 3 (sub-project C, separate plan) renders this state.

- [ ] **Step 1: Write the failing tests**

In `crates/trace-commons-contributor/src/daemon/private_inference.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Off is the default and starting nothing is not a failure. A daemon
    /// that has never been asked for private inference reports `Off`, not
    /// an error, and binds no port.
    #[tokio::test]
    async fn the_switch_is_off_until_asked() {
        let home = tempfile::tempdir().expect("a temp home");
        let mut host = PrivateInference::new(home.path().to_path_buf());
        assert_eq!(host.state(), PrivateInferenceState::Off);
        host.apply(false).await;
        assert_eq!(host.state(), PrivateInferenceState::Off);
    }

    /// Turning it on binds, serves, and reports the bound port; turning it
    /// off releases it. The port is ephemeral so this cannot collide with a
    /// developer's own IronWire.
    #[tokio::test]
    async fn turning_it_on_serves_and_turning_it_off_releases() {
        let home = tempfile::tempdir().expect("a temp home");
        let mut host = PrivateInference::with_port(home.path().to_path_buf(), 0);

        host.apply(true).await;
        let port = match host.state() {
            PrivateInferenceState::Running { port } => port,
            other => panic!("expected Running, got {other:?}"),
        };
        assert!(
            reqwest::get(format!("http://127.0.0.1:{port}/_ironwire/health"))
                .await
                .is_ok_and(|r| r.status().is_success())
        );

        host.apply(false).await;
        assert_eq!(host.state(), PrivateInferenceState::Off);
        assert!(
            tokio::net::TcpListener::bind(("127.0.0.1", port)).await.is_ok(),
            "turning it off must release the port"
        );
    }

    /// An IronWire this daemon did not start is left alone. The state says
    /// so, nothing is bound, and the other process keeps running -- a
    /// contributor's own proxy is not something to fight for a port.
    #[tokio::test]
    async fn someone_elses_ironwire_is_not_replaced() {
        let home = tempfile::tempdir().expect("a temp home");
        let theirs = ironwire_proxy::embed::start(home.path(), Some(0))
            .await
            .expect("their proxy starts");
        let port = theirs.port();
        write_pointer(home.path(), port);

        let mut host = PrivateInference::with_port(home.path().to_path_buf(), port);
        host.apply(true).await;

        assert_eq!(host.state(), PrivateInferenceState::RunningElsewhere { port });
        assert!(
            reqwest::get(format!("http://127.0.0.1:{port}/_ironwire/health"))
                .await
                .is_ok_and(|r| r.status().is_success()),
            "their proxy must still be serving"
        );

        theirs.shutdown().await;
    }

    /// A port held by something that is not IronWire is a refusal by name,
    /// not a panic and not a silent Off.
    #[tokio::test]
    async fn a_port_held_by_a_stranger_is_a_named_refusal() {
        let home = tempfile::tempdir().expect("a temp home");
        let squatter = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("a squatter binds");
        let port = squatter.local_addr().unwrap().port();

        let mut host = PrivateInference::with_port(home.path().to_path_buf(), port);
        host.apply(true).await;

        assert_eq!(
            host.state(),
            PrivateInferenceState::Failed { label: "port_in_use" }
        );
    }
}
```

`write_pointer` is a test helper writing `{"control_url":"http://127.0.0.1:<port>","token_path":"<home>/control.token"}` to `home/endpoint.json`, matching what IronWire writes.

- [ ] **Step 2: Run and watch them fail**

```bash
cd /Users/zakimanian/code/trace-commons-server
cargo test -p trace-commons-contributor --lib daemon::private_inference
```

Expected: compile error — the module does not exist.

- [ ] **Step 3: Add the dependency and regenerate the vendored sources**

In `crates/trace-commons-contributor/Cargo.toml`:

```toml
# IronWire's proxy, run in-process behind the private_inference switch.
#
# Measured cost: 83 packages this crate did not already have (223 in
# ironwire_proxy's tree, 140 already shared), notably axum -- an HTTP
# server this crate did not previously contain -- and rusqlite for
# IronWire's ledger. All MIT OR Apache-2.0, so the permissive boundary
# holds. Approved at that figure; anything beyond this tree needs its own
# approval.
ironwire_proxy = { git = "https://github.com/nearai/ironwire", rev = "<merge commit of Task 1>" }
```

Then, because nothing in PR CI validates the vendored set:

```bash
pip install aiohttp tomlkit
python3 flatpak-cargo-generator.py \
  crates/trace-commons-contributor-gtk/Cargo.lock \
  -o crates/trace-commons-contributor-gtk/flatpak/cargo-sources.json
git diff --stat -- crates/trace-commons-contributor-gtk/flatpak/cargo-sources.json
```

Expected: a large diff. If it is empty, the generator did not run — the GTK lockfile gained 83 packages, so an empty diff means the regeneration silently failed and the next `app-v*` tag will break.

- [ ] **Step 4: Implement the host**

`private_inference.rs`. The type owns at most one `EmbeddedProxy` and a state:

```rust
//! Running IronWire inside this daemon.
//!
//! IronWire proxies inference, so it must not be started by discovery: the
//! `private_inference` setting is the contributor's declaration and it
//! defaults to off. Finding a pointer on disk is never enough.
//!
//! The home is `$IRONWIRE_HOME`, else `~/.ironwire` -- deliberately the same
//! home the `ironwire` CLI uses, so a contributor who installs it sees one
//! ledger, one token, one pointer, and the routing reader keeps talking to
//! 127.0.0.1 exactly as before.
//!
//! Nothing here logs a prompt, a completion, a token, or a body. Fixed
//! labels, a port, and counts.
```

- `apply(true)`: if a pointer exists and probes and its token is not ours → `RunningElsewhere` and **return without binding**. Else `embed::start(home, port)`; map `EmbedError::PortInUse`/`Lock` → `Failed{"port_in_use"}`, other errors → `Failed{"start_failed"}`. On success, `Running{port}`.
- `apply(false)`: `shutdown().await` if held, then `Off`. Idempotent.
- The proxy's `JoinHandle` is watched; if it ends unexpectedly, state becomes `Failed{"crashed"}` and the daemon keeps running. A panic in the proxy must never take the daemon down.

**The `crashed` path has no automated test, deliberately.** Inducing a panic inside `serve_on` from outside means either a fault-injection hook in `ironwire_proxy` — a test-only seam in someone else's crate — or aborting the task, which exercises the watcher rather than a real panic. What *is* testable and must be tested is that the watcher observes an ended handle at all: drive it with a handle that returns immediately and assert the state becomes `Failed{"crashed"}` rather than staying `Running`. Note in the report which of the two you did.

Wire it into `daemon/mod.rs`'s shared state, applied at start from settings and on every settings change, and shut down on daemon shutdown.

- [ ] **Step 5: The setting and the IPC surface**

In `daemon/settings.rs` add `#[serde(default)] pub private_inference: bool` with:

```rust
    /// Run IronWire inside this daemon, so tools can send inference through
    /// it. Off by default and never turned on by discovery: finding
    /// IronWire's pointer on disk means someone else is running it, which is
    /// a different fact from the contributor asking us to.
    ///
    /// Turning this on does not repoint any agent. Which tools route through
    /// IronWire stays a per-tool declaration.
```

Accept it as a `set_settings` key in `ipc.rs`, and report `private_inference_state` in `get_settings`/`status` as the lowercase label plus the port when there is one. Document both in `docs/contributor-daemon-ipc-v1_1.md` — the doc test enforces the method table and this plan's reviewer will check the fields.

- [ ] **Step 6: Verify everything, including the two hidden configurations**

```bash
cargo fmt --all
RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor --lib daemon::private_inference
RUSTFLAGS='-D warnings' cargo test --workspace
for c in trace-commons-protocol trace-commons-attestation trace-commons-contributor trace-commons-contributor-ffi; do
  RUSTFLAGS='-D warnings' cargo check -p $c --no-default-features
done
cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
cargo clippy --workspace --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
cargo test -p trace-commons-server --test license_boundary
```

Expected: all clean. `license_boundary` matters here — it proves the 83 new packages did not drag an AGPL edge into a permissive crate.

- [ ] **Step 7: Mutation — prove the "someone else's" test bites**

Temporarily make `apply(true)` skip the pointer check and bind unconditionally. Expected: `someone_elses_ironwire_is_not_replaced` FAILS — the state is `Failed{"port_in_use"}` rather than `RunningElsewhere`, and in the worst case the other proxy is disturbed. Revert and re-run; paste both.

- [ ] **Step 8: Commit and open a PR**

```bash
git add -A
git commit -m "Run IronWire inside the daemon behind an off-by-default switch

private_inference makes the daemon host IronWire on loopback, using
~/.ironwire as its home so the CLI and the existing routing reader are
unaffected. Off by default and never turned on by discovery: a pointer on
disk means someone else is running it, which is a different fact from the
contributor asking us to.

An IronWire this daemon did not start is left alone and reported as such,
not fought for the port. A stranger on the port is a refusal by name. A
proxy panic is contained in its task and never takes the daemon down.

Turning this on repoints no agent; which tools route through IronWire
stays a per-tool declaration."
```

Open the PR with the dependency figure and the regenerated vendored set called out explicitly.

---

### Task 3 (sub-project C): the first-start offer

**Not in this plan.** Once Task 2 lands, `private_inference_state` is a settings field a shell can render, and the offer is a UI slice across three shells — its own spec and plan, following the core-owns-the-words rule (every sentence from Rust copy, no shell-authored wording) and the refusal rules (`Failed` renders REFUSED with a way out, never as attention or a caption).

Two things that plan must carry, recorded here so they are not lost:

1. **The quit confirmation on macOS and Windows must say that quitting stops routing.** `AppDelegate.swift:59-73` already explains that quitting stops the watcher; with IronWire inside the daemon it also stops inference routing, and the existing sentence no longer covers it.
2. **The offer is shown where the contributor looks** — the main window on first start after install or upgrade — not only in Settings, which is the failure this whole design exists to fix.
