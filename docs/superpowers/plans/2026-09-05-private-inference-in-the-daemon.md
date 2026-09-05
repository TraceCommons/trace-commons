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
- **Hash-only logging, in THIS repository only.** IronWire proxies prompts, so nothing added here may log a prompt, completion, token, control token, or body: fixed labels, counts, ports and durations. **This rule does not govern `nearai/ironwire`** — applying it there stripped error detail from eight log sites during Task 1. A sqlite or io error is not a prompt.
- License boundary: `-contributor` is MIT OR Apache-2.0. `ironwire_proxy` and its tree are MIT OR Apache-2.0, so the boundary holds. Never edit `license_boundary.rs`.
- **The dependency is approved and bounded:** `ironwire_proxy` adds 83 packages to `trace-commons-contributor` (223 in its tree, 140 already shared), notably `axum` and `rusqlite`/`libsqlite3-sys`. Adding anything beyond that tree needs separate approval.
- **`flatpak/cargo-sources.json` must be regenerated** in the task that adds the dependency. Nothing in PR CI validates it; the first failure is the `linux-flatpak` job on an `app-v*` tag.
- The daemon uses `~/.ironwire` as IronWire's home — never a private directory.
- `private_inference` defaults to **false** and never turns itself on.

---

### Task 1 (sub-project A, nearai/ironwire): an embeddable seam — **LANDED**

Merged as nearai/ironwire#26, `b1ecde4f`, "Let another application own the
IronWire proxy lifecycle". Do not implement; read this section only for what
it corrects.

**The shipped API, which is what Task 2 must call:**

```rust
pub async fn start(home: &Path, port_override: Option<u16>)
    -> Result<EmbeddedProxy, EmbedError>;
pub async fn start_with(home: &Path, port_override: Option<u16>,
    on_start: impl FnOnce(u16, &StartupReport)) -> Result<EmbeddedProxy, EmbedError>;

impl EmbeddedProxy {
    pub fn port(&self) -> u16;
    pub fn startup_report(&self) -> &StartupReport;
    pub fn is_finished(&self) -> bool;
    pub async fn wait(&mut self) -> Result<(), ExitError>;   // cancellation-safe, memoized
    pub async fn shutdown(mut self);
}

pub enum EmbedError { Paths, Config, Lock { port: u16 }, PortInUse { port: u16 },
                      Bind, Registry { label: &'static str } }
pub enum ExitError { Server, Task }
pub struct StartupReport {
    pub home: PathBuf, pub no_backends: bool, pub catalog_serial: u64,
    pub ledger_warning: Option<String>, pub bodies_warning: Option<String>,
    pub pointer_warning: bool,
}
```

**Six things this plan got wrong, recorded because Task 2 was written against
the same assumptions:**

1. **"Change no behaviour" with only `port()` and `shutdown()` is not
   implementable.** `serve.rs` interleaved `println!` *inside* the assembly
   (empty registry, ledger warning, catalog serial, bodies warning), so
   extraction forces a report channel back to the host. `StartupReport` and
   `start_with`'s `on_start` are invention this plan should have contained.
2. **There was no exit outcome.** A host must learn when the server dies on
   its own. `wait` / `is_finished` / `ExitError` are a requirement, not scope
   creep — and Task 2's `Failed { label: "crashed" }` depends on them.
3. **The second-start test was unpassable as specified.** It expected
   `EmbedError::Lock` while keeping the CLI's port-file lock, which
   deliberately ignores a same-port record and probes liveness by HTTP
   against a port nothing has bound. `Lock` cannot be produced without an OS
   lock. The OS lock was this plan's bug, not the PR's.
4. **It under-counted what lives in the binary.** `control_token`, `prune`,
   `catalog` and `update::spawn_check` were all CLI-private and all had to
   move, which is why `src/commands/lock.rs` is gone and `embed/` has
   submodules.
5. **It ignored discovery.** `Endpoint::publish()` writes the conventional
   `~/.ironwire`, which an embedded host with a custom home must not hijack.
   The home-local / CLI-conventional split was necessary and unplanned.
6. **It never mentioned `--port 0`,** despite `Some(0)` being central to its
   own first test.

**One constraint in this plan caused a defect upstream.** "Hash-only logging"
in the Global Constraints was written for the trace-commons side, where the
daemon proxies prompts. Applied to `nearai/ironwire` it stripped `%error` and
`backend = %id` from eight moved log sites, so `ironwire serve` no longer says
why a prune failed or which backend was disabled. A sqlite error is not a
prompt. **That rule governs this repository only** — see the corrected Global
Constraints above. Restoring those fields is a separate upstream follow-up.

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
- Consumes: `ironwire_proxy::embed::{start, start_with, EmbeddedProxy, EmbedError, ExitError, StartupReport}` — see the shipped API in Task 1. Note `EmbedError::Registry` now carries `label: &'static str`, and `Lock`/`PortInUse` both carry `port`.
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
ironwire_proxy = { git = "https://github.com/nearai/ironwire", rev = "b1ecde4f" }
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
- **Use `is_finished()` and `wait()`, not a `JoinHandle`.** The shipped API gives a proper exit outcome, which this plan originally lacked. Poll `is_finished()` where the daemon already reports state, and on a finished proxy call `wait()` for the `Result<(), ExitError>`: `Err(_)` becomes `Failed { label: "crashed" }`, `Ok(())` after an unrequested exit is the same label. `wait` is cancellation-safe and memoized, so calling it twice is fine. A proxy that ends must never take the daemon down.

**Testing the crashed path is now straightforward and is required.** `ExitError` is a plain enum, so the state mapping is a pure function of the `wait` result — test that directly, both variants and the unrequested-`Ok` case. Do not attempt to induce a real panic inside `serve_on`; that needs a fault-injection seam in someone else's crate.

- **Surface `StartupReport` rather than discarding it.** `start_with`'s `on_start` gives `no_backends`, `ledger_warning`, `bodies_warning`, `pointer_warning` and `catalog_serial`. A contributor whose IronWire started with **no backends configured** has private inference "running" and nothing will route — so `no_backends` must reach the reported state (a distinct label, not `Running`), or sub-project C will render a green light over a dead proxy. Decide the shape and say so in the report.

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
