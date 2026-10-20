//! Start IronWire inside a host application using the same assembly as the CLI.
//!
//! Errors carry fixed labels and at most a port; never credentials or bodies.

use std::collections::HashSet;
use std::sync::Arc;

use futures_util::{StreamExt, stream};

use crate::state::{AppState, BackendRegistry};
use anyhow::{Context, Result};
use ironwire_catalog::CatalogStore;
use ironwire_core::config::ModelEntry;
use ironwire_core::config::{BackendImpl, Config, PathsConfig};
use ironwire_core::protocol::{BackendId, BackendKind, ModelTier};
use ironwire_creds::claude::ClaudeCodeCredentials;
use ironwire_creds::codex::{CodexCredentials, CodexMode};
use ironwire_creds::{ClaudeLogin, CodexLogin, ConsentLedger, SubscriptionProduct};
use ironwire_ledger::Ledger;
use ironwire_ledger::bodies::BodyStore;
use ironwire_upstream::anthropic::AnthropicBackend;
use ironwire_upstream::openai_chat::ChatCompletionsBackend;
use ironwire_upstream::openai_responses::ResponsesBackend;
use secrecy::SecretString;

mod catalog;
#[doc(hidden)]
pub mod files;
mod lock;
mod prune;
#[doc(hidden)]
pub mod updates;

/// Who owns upgrading the running proxy implementation.
///
/// Hosts must allow future policies rather than exhaustively matching today's variants.
///
/// ```compile_fail,E0004
/// use ironwire_proxy::embed::UpdatePolicy;
/// fn standalone(policy: UpdatePolicy) -> bool {
///     match policy {
///         UpdatePolicy::HostManaged => false,
///         UpdatePolicy::Standalone => true,
///     }
/// }
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum UpdatePolicy {
    /// The embedding application ships and upgrades the library. Do not read
    /// standalone update notifications or check standalone IronWire releases.
    #[default]
    HostManaged,
    /// The process is the standalone IronWire CLI; retain its release checks
    /// and installer-specific notifications, subject to `updates.check`.
    Standalone,
}

/// Whether this process may make the two requests IronWire makes on its own
/// behalf: the release check, and the signed provider-catalog refresh from
/// `ironwire.dev`.
///
/// [`UpdatePolicy`] answers who owns upgrading the binary; this answers whether
/// those two requests happen. They are separate questions, and an embedding host
/// could previously only answer the second through `updates.check` in
/// `$IRONWIRE_HOME/config.toml` — a file it may not own, because the home can
/// be a real user's, shared with the CLI. This is the same switch, expressed in
/// code, and it covers exactly what that switch covers.
///
/// **It is not a general network kill switch, and must not be described as
/// one.** Startup catalogue discovery still probes every registered backend
/// but a subscription awaiting consent over the network
/// (`spawn_catalogue_discovery`, calling `Backend::probe`),
/// under either value. Two things make that reach further than "only what the
/// host configured": `build_registry` registers Claude, Codex, and API-key
/// backends from credentials it discovers in the environment with no config
/// entry naming them, and the NEAR AI backend is registered unconditionally.
/// So a bare embedded start does make a request, whatever is chosen here. A
/// host that must make no outbound request at all cannot get that from this
/// option today; see `docs/EMBEDDING.md`.
///
/// Hosts must allow future values rather than exhaustively matching today's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum UpdateChecks {
    /// Honor `updates.check` from the home's configuration, defaulting to on
    /// when there is no file. What every caller got before this existed.
    #[default]
    FromConfig,
    /// Make neither of those two requests, whatever the configuration says. For
    /// a host that would set `updates.check = false` but cannot write the home's
    /// config. Startup backend probes are unaffected.
    Off,
}

/// Which backends are probed at startup, and what a host gives up by declining.
///
/// Startup discovery calls `Backend::probe` on registered backends: a live
/// request that both proves the backend works right now and learns the model
/// catalogue the provider actually serves. Never on a subscription awaiting
/// consent, under any value: the probe would send its OAuth token. It is real work, so declining is a
/// trade, not a free saving. A host that declines starts without that answer:
/// the daemon runs on configured or compiled-in catalogue values, and an
/// expired credential is discovered by the first real request rather than at
/// startup. `ironwire doctor` probes on demand and is unaffected.
///
/// The reason a host may want to decline is that registered is not the same as
/// configured. `build_registry` registers the Claude subscription, the Codex
/// subscription, and the Anthropic/OpenAI key backends from credentials found
/// in the environment with no entry naming them, and registers NEAR AI
/// unconditionally. So an embedded start probes providers the host never named,
/// and today its only way to prevent that is `enabled = false` entries in
/// `$IRONWIRE_HOME/config.toml` — a file it may not own.
///
/// Hosts must allow future values rather than exhaustively matching today's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum StartupProbes {
    /// Probe every registered backend but a subscription awaiting consent.
    /// The default, and what the CLI does.
    #[default]
    All,
    /// Probe only backends an entry in `config.toml` names.
    ///
    /// The middle position, and the one most embedding hosts want: a host that
    /// declared its backends deliberately keeps the startup answer for those,
    /// and makes no request on behalf of a backend IronWire found on its own.
    Configured,
    /// Probe nothing. The only value under which an embedded start makes no
    /// outbound request of its own accord.
    Off,
}

/// Where every backend credential comes from, for a host that owns them.
///
/// Called with the name of the variable IronWire would otherwise read —
/// `NEARAI_API_KEY`, or whatever `api_key_env` names for a configured backend —
/// so a host answers per backend without knowing IronWire's internals. The name
/// is the only thing that crosses in that direction; nothing about the host's
/// answer is logged, reported, or rendered.
///
/// **Supplying one of these replaces credential discovery, it does not extend
/// it.** `None` means there is no credential for that name: the process
/// environment is not consulted afterwards, and neither are the credential
/// files Claude Code and Codex write. An empty answer is the same as no answer.
///
/// The alternative — answer where the host can, environment where it cannot —
/// was rejected. Under it a stray `ANTHROPIC_API_KEY` in the host's environment
/// registers a backend the host never authorized, which is the same surprise
/// [`StartupProbes::Configured`] exists to prevent, and it leaves a host that
/// must state which destinations are possible unable to state it. Either the
/// host manages credentials or IronWire does; never half of each.
///
/// [`SecretString`] rather than `String` so the value has no `Debug` rendering
/// and is zeroized on drop; it is also what the registry wants, so the host's
/// secret is never widened back into a plain `String` on the way through.
pub type CredentialSource = Arc<dyn Fn(&str) -> Option<SecretString> + Send + Sync>;

/// Re-exported so a host names the same type this crate does, rather than
/// depending on `secrecy` and discovering the mismatch as a type error.
pub use secrecy::SecretString as HostSecret;

/// Whether the credential files Claude Code and Codex write may be read.
///
/// This is the second of two questions about credentials, and it used to be
/// answered by the first. [`CredentialSource`] settles *whose answer counts for
/// a name* — the host's, or the process environment's. This settles *whether
/// the files on disk are read at all*. They are separate questions because the
/// subscription backends are key-less by construction: a Claude Code or Codex
/// login is a token in a file, not a value any name-keyed source can supply.
/// So a host that answers `NEARAI_API_KEY` cannot answer for those two, and
/// deriving one question from the other left it unable to say "the answer for
/// this name is mine, and the user's own logins are still fine". It had to give
/// up every subscription backend the user already had in exchange for supplying
/// one key of its own.
///
/// Nothing here weakens what [`CredentialSource`] decides. A host that owns
/// credentials still owns every name; a file discovered under
/// [`CredentialFiles::Discover`] registers a subscription backend and supplies
/// no key for any name the host answers. The metered key Codex stores after
/// `codex login --api-key` is a file, so it follows this switch too — it is the
/// one credential that is both a file and a key.
///
/// It covers every login, not just the default ones discovery finds: an
/// additional Claude or ChatGPT login that `config.toml` declares is a token in
/// a directory too, and with the files off no subscription of any login is
/// registered.
///
/// Hosts must allow future values rather than exhaustively matching today's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum CredentialFiles {
    /// Read them when IronWire owns credential lookup, and not when a host
    /// does. The default, and exactly what every caller got before this
    /// existed: a host that supplies nothing still gets the files, and a host
    /// that supplies a [`CredentialSource`] and says nothing about files still
    /// gets none of them. Changing either of those silently would be its own
    /// surprise, so neither changes.
    #[default]
    FollowCredentialOwner,
    /// Read them whoever owns the names. The case the API could not express:
    /// the host answers for the keys it holds, and a contributor's existing
    /// Claude or ChatGPT subscription goes on answering for itself.
    ///
    /// A host selecting this is accepting what it declines under
    /// [`FollowCredentialOwner`][Self::FollowCredentialOwner]: a backend can be
    /// registered from a login the host never named, and a request can go to a
    /// subscription the user set up rather than to a destination the host
    /// chose. That is the point — but a host that must be able to state every
    /// possible destination should not select it.
    Discover,
    /// Read none of them, whoever owns the names. For a host that leaves
    /// name-keyed discovery to IronWire — the process environment it was
    /// started with is fine — but does not want a real user's home read for
    /// logins it never asked about.
    Off,
}

/// Everything an embedding host chooses about a start, other than the home,
/// the port, and the announcement hook.
///
/// Construct from [`EmbedOptions::default`] and set what differs, so a future
/// choice does not break existing callers.
///
/// ```
/// use ironwire_proxy::embed::{EmbedOptions, UpdateChecks};
/// let options = EmbedOptions::default().with_update_checks(UpdateChecks::Off);
/// ```
///
/// Carrying a host's credential source costs this type `Copy`, `PartialEq` and
/// `Eq`: a closure has none of them. `Clone` and `Default` remain, and the
/// builder form above never needed the others.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct EmbedOptions {
    /// Optional host consent override. Enabling requires explicitly configured
    /// qualified targets; disabling preserves cleanup of existing captures.
    pub token_capture_enabled: Option<bool>,
    /// Who owns upgrading the running proxy implementation.
    pub update_policy: UpdatePolicy,
    /// Whether the release check and catalog refresh may run at all.
    pub update_checks: UpdateChecks,
    /// Which backends are probed for their catalogue at startup.
    pub startup_probes: StartupProbes,
    /// Where backend credentials come from, before the process environment.
    pub credentials: Option<CredentialSource>,
    /// Whether the credential files Claude Code and Codex write may be read.
    pub credential_files: CredentialFiles,
    /// Who ties a receipt's signing key to a verified TDX quote. See
    /// [`EmbedOptions::with_signer_attestor`].
    pub signer_attestor: Option<Arc<dyn crate::proof::SignerAttestor>>,
    /// How hard the receipt check works, when it runs.
    pub proof_settings: crate::proof::ProofSettings,
}

/// Renders whether a host supplied a credential source, never anything it
/// answered and never a name it could be asked for.
impl std::fmt::Debug for EmbedOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbedOptions")
            .field("update_policy", &self.update_policy)
            .field("update_checks", &self.update_checks)
            .field("startup_probes", &self.startup_probes)
            .field(
                "credentials",
                match self.credentials {
                    Some(_) => &"host-owned",
                    None => &"discovered",
                },
            )
            .field("credential_files", &self.credential_files)
            .field("proof_settings", &self.proof_settings)
            .field(
                "signer_attestor",
                match self.signer_attestor {
                    Some(_) => &"host-supplied",
                    None => &"none",
                },
            )
            .finish()
    }
}

impl EmbedOptions {
    /// Supply the attestor that ties a NEAR AI receipt's signing key to a
    /// verified TDX quote, and turn receipt checks on.
    ///
    /// IronWire carries no DCAP quote verifier of its own. Without one, a
    /// receipt that checks out is recorded `unattested` and never `verified`
    /// (`crate::proof`). A host that already verifies quotes -- and pins their
    /// measurements; a genuine TDX VM proves nothing about whose -- supplies
    /// it here, and rows it vouches for can reach `verified`.
    ///
    /// Supplying one is the host deciding receipts are checked, so it turns
    /// `capture.receipts` on for this start whatever the configuration says.
    /// That costs one `GET` per NEAR AI exchange, to NEAR AI, with the NEAR AI
    /// key, off the response path; the provider learns that a call it already
    /// served is being checked.
    #[must_use]
    pub fn with_signer_attestor(mut self, attestor: Arc<dyn crate::proof::SignerAttestor>) -> Self {
        self.signer_attestor = Some(attestor);
        self
    }

    /// Tune the background receipt check: batch, concurrency, retry budget
    /// and period. The defaults suit a daemon; a host that wants answers
    /// sooner shortens the period.
    #[must_use]
    pub fn with_proof_settings(mut self, settings: crate::proof::ProofSettings) -> Self {
        self.proof_settings = settings;
        self
    }

    /// Select who owns upgrading the running proxy implementation.
    #[must_use]
    pub fn with_update_policy(mut self, update_policy: UpdatePolicy) -> Self {
        self.update_policy = update_policy;
        self
    }

    /// Decline the requests IronWire makes on its own behalf, in code.
    ///
    /// There is deliberately no value that turns them back *on* over a
    /// configured `updates.check = false`: someone who turned it off meant it,
    /// and a host cannot un-mean it on their behalf.
    #[must_use]
    pub fn with_update_checks(mut self, update_checks: UpdateChecks) -> Self {
        self.update_checks = update_checks;
        self
    }

    /// Choose which backends are probed at startup.
    ///
    /// See [`StartupProbes`] for what a host gives up by narrowing this.
    #[must_use]
    pub fn with_startup_probes(mut self, startup_probes: StartupProbes) -> Self {
        self.startup_probes = startup_probes;
        self
    }

    /// Take over backend credentials entirely.
    ///
    /// The environment is the only channel an embedding host has had for a
    /// backend key, and reaching it after startup means `std::env::set_var`,
    /// which is `unsafe` in Rust 2024 because mutating the environment once
    /// threads exist is a data race. It is also the wrong place for a secret in
    /// a shared process: every other library in that process can read it, and a
    /// host does not necessarily trust all of them with a credential it holds.
    ///
    /// This is a transfer of ownership, not an additional source. With one
    /// supplied, nothing else is consulted: not the process environment, and
    /// not the credential files Claude Code and Codex write. A host that
    /// answers only `NEARAI_API_KEY` gets NEAR AI and nothing else — no
    /// subscription backend registered from a login it never asked about, and
    /// no API backend registered from a variable that happened to be set.
    /// [`StartupReport::no_backends`] becomes genuinely reachable as a result.
    ///
    /// A host that supplies nothing is exactly where it was: the process
    /// environment and the same credential files, read the same way, at the
    /// same points.
    ///
    /// The credential files are a separate question, and a host that wants to
    /// own the names *and* leave a user's existing subscriptions answering says
    /// so with [`EmbedOptions::with_credential_files`]. What this method
    /// decides on its own — whose answer counts for a name — is unchanged
    /// either way.
    ///
    /// ```
    /// use ironwire_proxy::embed::EmbedOptions;
    /// use secrecy::SecretString;
    /// # fn vault(_name: &str) -> Option<SecretString> { None }
    /// let options = EmbedOptions::default().with_credentials(vault);
    /// ```
    #[must_use]
    pub fn with_credentials(
        mut self,
        credentials: impl Fn(&str) -> Option<SecretString> + Send + Sync + 'static,
    ) -> Self {
        self.credentials = Some(Arc::new(credentials));
        self
    }

    /// Say whether the credential files Claude Code and Codex write may be
    /// read, separately from who owns name-keyed credentials.
    ///
    /// Both questions default to what they have always answered, so a call to
    /// [`with_credentials`][Self::with_credentials] alone still reads no file
    /// and a start with neither call still reads every one. See
    /// [`CredentialFiles`] for what a host takes on by widening it.
    ///
    /// ```
    /// use ironwire_proxy::embed::{CredentialFiles, EmbedOptions};
    /// use secrecy::SecretString;
    /// # fn vault(_name: &str) -> Option<SecretString> { None }
    /// // The host answers for its own key; the user's Claude Code or Codex
    /// // login goes on answering for itself.
    /// let options = EmbedOptions::default()
    ///     .with_credentials(vault)
    ///     .with_credential_files(CredentialFiles::Discover);
    /// ```
    #[must_use]
    pub fn with_credential_files(mut self, credential_files: CredentialFiles) -> Self {
        self.credential_files = credential_files;
        self
    }

    /// Resolve `updates.check` against the host's choice.
    fn checks_enabled(&self, configured: bool) -> bool {
        match self.update_checks {
            UpdateChecks::FromConfig => configured,
            UpdateChecks::Off => false,
        }
    }
}

/// Fixed-label startup failures, suitable for a host's refusal state.
#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    /// Home or token initialization failed.
    #[error("could not prepare the IronWire home or control token")]
    Paths,
    /// Configuration was unreadable, invalid, or incompatible with spend limits.
    #[error("invalid config.toml; check the configuration and enable capture for spend limits")]
    Config,
    /// Another process owns this home.
    #[error("another IronWire owns this home on port {port}")]
    Lock {
        /// Port published by the current owner, or the requested port during startup.
        port: u16,
    },
    /// Another listener occupies the requested loopback port.
    #[error("port {port} is already in use")]
    PortInUse {
        /// Requested port.
        port: u16,
    },
    /// A loopback bind failed for another reason.
    #[error("could not bind the loopback listener")]
    Bind,
    /// A configured backend could not be constructed.
    #[error("could not build the backend registry: {label}")]
    Registry {
        /// Fixed backend construction label; excludes credentials and URLs.
        label: &'static str,
    },
}

/// Startup observations rendered by the CLI; an embedded host controls its UI.
#[derive(Default)]
pub struct StartupReport {
    /// Canonical home used by this proxy and its discovery pointer.
    pub home: std::path::PathBuf,
    /// No backend could be registered.
    pub no_backends: bool,
    /// Verified catalog serial, or zero for built-in values.
    pub catalog_serial: u64,
    /// Optional local ledger-open diagnostic for the CLI.
    pub ledger_warning: Option<String>,
    /// Optional body-store-open diagnostic for the CLI.
    pub bodies_warning: Option<String>,
    /// The home discovery pointer could not be published.
    pub pointer_warning: bool,
    /// This start will make the two requests IronWire makes on its own behalf:
    /// the release check and the catalog refresh. `updates.check` from the
    /// home's configuration, after the host's [`UpdateChecks`] choice. A host
    /// that declined can confirm here that it declined. Says nothing about
    /// startup backend probes, which this switch does not govern.
    pub update_checks: bool,
}

/// A server's final outcome. No internal error text crosses the host boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ExitError {
    /// The server returned an I/O error.
    #[error("the IronWire server stopped with an error")]
    Server,
    /// The server task panicked or was canceled.
    #[error("the IronWire server task ended unexpectedly")]
    Task,
}

/// Owns one running proxy. Dropping it requests graceful shutdown; resources
/// remain owned by the supervisor until in-flight requests finish. Hosts that
/// need confirmation of cleanup must await `shutdown`, not merely drop.
///
/// Task panics can be observed only when the host compiles with panic unwinding.
/// A host using `panic = "abort"` cannot contain a panic in any Tokio task.
pub struct EmbeddedProxy {
    port: u16,
    report: StartupReport,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<Result<(), ExitError>>>,
    exit: Option<Result<(), ExitError>>,
}

impl EmbeddedProxy {
    /// Actual bound loopback port.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }
    /// Startup observations for the host to render.
    #[must_use]
    pub fn startup_report(&self) -> &StartupReport {
        &self.report
    }
    /// True after the server and its housekeeping have ended.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.exit.is_some()
            || self
                .task
                .as_ref()
                .is_none_or(tokio::task::JoinHandle::is_finished)
    }
    /// Observe an unexpected exit without transferring shutdown ownership.
    /// Cancellation-safe: dropping this future leaves the proxy running.
    pub async fn wait(&mut self) -> Result<(), ExitError> {
        if let Some(exit) = self.exit {
            return exit;
        }
        let exit = match self.task.as_mut() {
            Some(task) => task.await.unwrap_or(Err(ExitError::Task)),
            None => Ok(()),
        };
        self.task = None;
        self.exit = Some(exit);
        exit
    }
    /// Drain in-flight requests, stop housekeeping, flush quota, and release
    /// the listener, discovery pointer, and home lock. No timeout truncates a
    /// model stream; the host owns any policy for abandoning its own process.
    pub async fn shutdown(mut self) {
        self.request_shutdown();
        let _ = self.wait().await;
    }
    fn request_shutdown(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for EmbeddedProxy {
    fn drop(&mut self) {
        self.request_shutdown();
    }
}

struct OwnedPointer {
    path: std::path::PathBuf,
    endpoint: ironwire_core::discovery::Endpoint,
}
impl Drop for OwnedPointer {
    fn drop(&mut self) {
        if ironwire_core::discovery::Endpoint::read_from(&self.path).as_ref()
            == Some(&self.endpoint)
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

struct Tasks(Vec<tokio::task::JoinHandle<()>>);
impl Tasks {
    async fn stop(&mut self) {
        for task in &self.0 {
            task.abort();
        }
        for task in self.0.drain(..) {
            let _ = task.await;
        }
    }
}
impl Drop for Tasks {
    fn drop(&mut self) {
        for task in &self.0 {
            task.abort();
        }
    }
}

/// Start a loopback proxy against an explicit IronWire home. An empty home is
/// prepared automatically; existing configuration and consent are preserved.
/// `Some(0)` requests an ephemeral port, reported by `EmbeddedProxy::port`.
///
/// The pointer is published within this home only. The CLI additionally
/// publishes its conventional desktop-discovery pointer. Starting an embedded
/// instance never overwrites an unrelated real user's discovery file.
///
/// # Errors
/// Returns a fixed-label refusal before starting any background task.
pub async fn start(
    home: &std::path::Path,
    port_override: Option<u16>,
) -> Result<EmbeddedProxy, EmbedError> {
    start_with(home, port_override, |_, _| {}).await
}

/// Start with a synchronous announcement after binding and assembly, but before
/// accepting requests or starting background tasks. The CLI uses this to finish
/// its startup instructions before health can report readiness. The callback
/// must return promptly; it must not wait for this proxy to answer a request.
///
/// # Errors
/// The same fixed-label startup refusals as [`start`].
pub async fn start_with(
    home: &std::path::Path,
    port_override: Option<u16>,
    on_start: impl FnOnce(u16, &StartupReport),
) -> Result<EmbeddedProxy, EmbedError> {
    start_with_policy(home, port_override, UpdatePolicy::HostManaged, on_start).await
}

/// Start with an explicit owner for binary updates and a startup announcement.
/// Only standalone CLI hosts should select [`UpdatePolicy::Standalone`].
/// Provider catalog refresh and model discovery are independent of this policy;
/// the existing `updates.check` configuration still controls catalog refresh.
/// A host that cannot write that file declines in code with
/// [`start_with_options`] and [`UpdateChecks::Off`].
/// The announcement has the same ordering and restrictions as [`start_with`].
///
/// # Errors
/// The same fixed-label startup refusals as [`start`].
pub async fn start_with_policy(
    home: &std::path::Path,
    port_override: Option<u16>,
    update_policy: UpdatePolicy,
    on_start: impl FnOnce(u16, &StartupReport),
) -> Result<EmbeddedProxy, EmbedError> {
    start_with_options(
        home,
        port_override,
        EmbedOptions::default().with_update_policy(update_policy),
        on_start,
    )
    .await
}

/// Start with every host-owned choice stated, and a startup announcement.
///
/// [`EmbedOptions::default`] is what [`start`] uses: the host owns upgrading
/// this library, and `updates.check` in the home's configuration still governs
/// the release check and the catalog refresh. A host that wants neither of
/// those two requests selects [`UpdateChecks::Off`], which suppresses both
/// without writing to a configuration file it may not own. It does not
/// suppress startup backend probes; see [`UpdateChecks`].
/// The announcement has the same ordering and restrictions as [`start_with`].
///
/// # Errors
/// The same fixed-label startup refusals as [`start`].
pub async fn start_with_options(
    home: &std::path::Path,
    port_override: Option<u16>,
    options: EmbedOptions,
    on_start: impl FnOnce(u16, &StartupReport),
) -> Result<EmbeddedProxy, EmbedError> {
    let update_policy = options.update_policy;
    std::fs::create_dir_all(home).map_err(|_| EmbedError::Paths)?;
    files::restrict_permissions(home, 0o700).map_err(|_| EmbedError::Paths)?;
    let paths = PathsConfig::rooted_at(std::fs::canonicalize(home).map_err(|_| EmbedError::Paths)?);
    let mut config = Config::load(&paths).map_err(|_| EmbedError::Config)?;
    apply_token_capture_override(&mut config, options.token_capture_enabled)?;
    // A host that supplies an attestor has decided receipts are checked.
    if options.signer_attestor.is_some() {
        config.capture.receipts = true;
    }
    let checks = options.checks_enabled(config.updates.check);
    let port = port_override.unwrap_or(config.server.port);
    if config.limits.any_cap() && !config.capture.enabled {
        return Err(EmbedError::Config);
    }
    // Acquire ownership before reading mutable consent or minting a token.
    let mut lock = Some(lock::acquire(&paths.lock_file(), port).await?);
    let token = files::control_token(&paths).map_err(|_| EmbedError::Paths)?;
    let consent = ConsentLedger::load(&paths.consent_file());
    let credentials = match options.credentials.as_ref() {
        Some(host) => Credentials::host_owned(host),
        None => Credentials::discovered(&real_env),
    }
    .with_files(options.credential_files);
    let registry = build_registry(&config, &credentials).map_err(|error| EmbedError::Registry {
        label: error
            .downcast_ref::<RegistryLabel>()
            .map_or("backend construction", |label| label.0),
    })?;
    restore_quota(&registry, &paths);
    let listener = crate::server::bind(port).await.map_err(|e| match e {
        crate::server::ServeError::PortInUse { port } => EmbedError::PortInUse { port },
        _ => EmbedError::Bind,
    })?;
    let port = listener.local_addr().map_err(|_| EmbedError::Bind)?.port();
    if let Some(lock) = lock.as_mut() {
        lock.publish(port)?;
    }
    let mut report = StartupReport {
        home: paths.home.clone(),
        no_backends: registry.is_empty(),
        update_checks: checks,
        ..StartupReport::default()
    };
    let ledger = open_ledger(&paths, &config, &mut report);
    let catalog = CatalogStore::load(ironwire_catalog::CATALOG_PUBLIC_KEY, &paths.catalog_file());
    report.catalog_serial = catalog.serial();
    let bodies = open_bodies(&paths, &config, &mut report);
    sweep_bodies(&ledger, bodies.as_deref());
    // Open even when capture has subsequently been disabled: pending leases
    // and deletion work must remain accessible to their owners.
    let spool_dir = paths.home.join("token-captures");
    let token_spool = if config.capture.token_capture.is_some() || spool_dir.exists() {
        let limits = config.capture.token_capture.clone().unwrap_or_default();
        match ironwire_ledger::token_spool::TokenSpool::open(
            &spool_dir,
            limits.max_bytes,
            limits.retain_seconds,
        ) {
            Ok(spool) => Some(std::sync::Arc::new(spool)),
            Err(error) => {
                tracing::warn!(%error, "detailed capture unavailable");
                None
            }
        }
    } else {
        None
    };
    let state = AppState::new(registry, config, consent, token)
        .with_port(port)
        .with_paths(paths.clone())
        .with_ledger(ledger)
        .with_bodies(bodies)
        .with_token_spool(token_spool)
        .with_catalog(catalog);
    seed_spend(&state);
    let endpoint = ironwire_core::discovery::Endpoint::new(port, paths.control_token_file());
    let pointer = OwnedPointer {
        path: paths.home.join("endpoint.json"),
        endpoint,
    };
    let published = serde_json::to_string_pretty(&pointer.endpoint)
        .ok()
        .is_some_and(|body| ironwire_core::atomic::write(&pointer.path, &body).is_ok());
    report.pointer_warning = !published;
    let pointer = published.then_some(pointer);
    on_start(port, &report);
    let mut background = Tasks(Vec::new());
    if let Some(task) = prune::spawn(
        state.ledger.clone(),
        state.config.capture.retain_days,
        state.bodies.clone(),
    ) {
        background.0.push(task);
    }
    if let Some(spool) = state.token_spool.clone() {
        background.0.push(tokio::spawn(async move {
            loop {
                let store = spool.clone();
                let result = tokio::task::spawn_blocking(move || {
                    store.prune(chrono::Utc::now().timestamp())
                })
                .await;
                if !matches!(result, Ok(Ok(_))) {
                    tracing::warn!("detailed capture cleanup pending");
                }
                tokio::time::sleep(std::time::Duration::from_secs(300)).await;
            }
        }));
    }
    // Standalone cache entries can contain installer commands for a different
    // executable. Skip hydration as well as fetching when the host owns updates.
    if update_policy == UpdatePolicy::Standalone
        && let Some(task) = updates::spawn_check(state.clone(), &paths, checks)
    {
        background.0.push(task);
    }
    if let Some(task) = catalog::spawn_refresh(state.clone(), &paths, checks) {
        background.0.push(task);
    }
    if let Some(task) = spawn_catalogue_discovery(state.clone(), options.startup_probes) {
        background.0.push(task);
    }
    // Receipt checks: opt-in, and only with a ledger to settle rows in. Off
    // the response path by construction -- it reads rows already written.
    if state.config.capture.receipts
        && let Some(ledger) = state.ledger.clone()
    {
        background.0.push(crate::proof::spawn(
            ledger,
            state.backends.clone(),
            options
                .signer_attestor
                .clone()
                .unwrap_or_else(|| Arc::new(crate::proof::NoQuoteVerification)),
            options.proof_settings,
        ));
    }
    let quota = QuotaWriter::new(paths.quota_file());
    background.0.push(quota.spawn(state.clone()));
    let (tx, rx) = tokio::sync::oneshot::channel();
    let server_state = state.clone();
    let server = tokio::spawn(crate::server::serve_on(
        listener,
        server_state,
        async move {
            let _ = rx.await;
        },
    ));
    let task = tokio::spawn(async move {
        // Await the server in a separate task so unwinding still runs cleanup.
        let result = server
            .await
            .map_err(|_| ExitError::Task)
            .and_then(|r| r.map_err(|_| ExitError::Server));
        background.stop().await;
        quota.write_now(&state);
        drop(state);
        drop(pointer);
        drop(lock);
        result
    });
    Ok(EmbeddedProxy {
        port,
        report,
        shutdown: Some(tx),
        task: Some(task),
        exit: None,
    })
}

/// Writes observed quota to disk, on a timer and once at shutdown.
///
/// One writer, holding the last rendered document, so the periodic write and
/// the shutdown write cannot race into a last-writer-wins where the loser is
/// the *newer* snapshot. Both go through [`Self::write_now`].
#[derive(Clone)]
struct QuotaWriter {
    path: std::path::PathBuf,
    last: std::sync::Arc<std::sync::Mutex<Option<String>>>,
}

impl QuotaWriter {
    /// How often to check whether anything changed.
    ///
    /// A write per request would be absurd — SSE-observed usage updates several
    /// times within a single stream — and the value being protected is only
    /// useful at restart, so half a minute of lag costs nothing.
    const PERIOD: std::time::Duration = std::time::Duration::from_secs(30);

    fn new(path: std::path::PathBuf) -> Self {
        Self {
            path,
            last: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }

    fn spawn(&self, state: crate::state::AppState) -> tokio::task::JoinHandle<()> {
        let writer = self.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Self::PERIOD);
            ticker.tick().await; // fires immediately; nothing has changed yet
            loop {
                ticker.tick().await;
                writer.write_now(&state);
            }
        })
    }

    /// Render and write, unless nothing has changed since the last write.
    ///
    /// Reads `Backend::quota()` directly rather than `statuses()`: the latter
    /// does a fresh credential check per backend, which on a thirty-second
    /// timer would re-read the Keychain a few thousand times a day to learn
    /// something it already has in a mutex.
    fn write_now(&self, state: &crate::state::AppState) {
        let quotas: Vec<(String, ironwire_core::quota::QuotaSnapshot)> = state
            .backends
            .all()
            .iter()
            .map(|backend| (backend.id().to_string(), backend.quota()))
            .collect();
        let rendered = ironwire_core::quota_store::render(&quotas, chrono::Utc::now());

        let mut last = match self.last.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        // `written_at` changes every time, so compare the part that carries
        // meaning rather than the whole document.
        if last.as_deref().map(strip_written_at) == Some(strip_written_at(&rendered)) {
            return;
        }
        // A bookkeeping failure must never affect routing — same rule as the
        // ledger. Warn once per change, not once per tick.
        if let Err(error) = ironwire_core::quota_store::write(&self.path, &rendered) {
            tracing::warn!(path = %self.path.display(), %error, "could not persist observed quota");
            return;
        }
        *last = Some(rendered);
    }
}

/// The document minus its timestamp line, for change detection.
fn strip_written_at(document: &str) -> String {
    document
        .lines()
        .filter(|line| !line.trim_start().starts_with("\"written_at\""))
        .collect()
}

/// Seed each backend with what it had observed before the last shutdown.
///
/// A backend id in the file that is not in this registry is ignored rather than
/// resurrected: the user may have disconnected it, and quota for a backend we
/// will not route to is not a fact about anything.
fn restore_quota(registry: &BackendRegistry, paths: &PathsConfig) {
    let stored = ironwire_core::quota_store::load(&paths.quota_file(), chrono::Utc::now());
    if stored.is_empty() {
        return;
    }
    for backend in registry.all() {
        if let Some(quota) = ironwire_core::quota_store::for_backend(&stored, backend.id().as_str())
        {
            backend.restore_quota(quota);
        }
    }
}

/// Seed the spend tracker from what the ledger already recorded today.
///
/// Metered backends only, and the same local-midnight window `status` reports.
/// Without this a daemon restarted after $8 of a $10 cap would resume at zero,
/// and the cap would be resettable by restarting.
fn seed_spend(state: &crate::state::AppState) {
    if !state.config.limits.any_cap() {
        return;
    }
    let Some(ledger) = state.ledger.as_ref() else {
        return;
    };
    let now = chrono::Utc::now();
    let Ok(summary) = ledger.summary(crate::spend::window_start(now)) else {
        return;
    };
    let metered: std::collections::HashSet<&str> = state
        .backends
        .all()
        .iter()
        .filter(|backend| backend.kind().is_metered())
        .map(|backend| backend.id().as_str())
        .collect();
    let spent = summary
        .cost_by_backend
        .iter()
        .filter(|(backend, _)| metered.contains(backend.as_str()))
        .map(|(backend, cost)| (BackendId::from(backend.as_str()), *cost));

    let seeded = crate::spend::SpendTracker::seeded(spent, now);
    match state.spend.lock() {
        Ok(mut tracker) => *tracker = seeded,
        Err(poisoned) => *poisoned.into_inner() = seeded,
    }
}

/// Learn each backend's catalogue in the background, once, at startup.
///
/// Which backends, and whether at all, is [`StartupProbes`]. `All` keeps the
/// behaviour every caller had before that existed.
///
/// In the background because a probe is a network round trip per backend and
/// the daemon must be answering requests immediately; a request that arrives
/// first simply routes on the compiled-in list, which is what it did before.
/// Failures are logged at debug and otherwise ignored — this is an
/// optimisation of what we know, not a health check, and `ironwire doctor`
/// remains the place that reports whether a backend actually works.
///
/// A subscription nobody has said yes to is not probed: a probe sends its
/// OAuth token as surely as a request does, and consent is what lets IronWire
/// send it at all (`docs/TRUST.md` §2). Its catalogue is learned on the first
/// probe after consent.
fn spawn_catalogue_discovery(
    state: crate::state::AppState,
    probes: StartupProbes,
) -> Option<tokio::task::JoinHandle<()>> {
    if probes == StartupProbes::Off {
        return None;
    }
    Some(tokio::spawn(async move {
        let consent = state.consent_snapshot();
        let configured: HashSet<_> = state
            .config
            .backends
            .iter()
            .map(|entry| entry.id.as_str())
            .collect();
        let mut checks = Vec::new();
        for backend in state.backends.all().iter().cloned() {
            if !crate::state::consents_to(&consent, backend.as_ref())
                || (probes == StartupProbes::Configured
                    && !configured.contains(backend.id().as_str()))
            {
                continue;
            }
            let registry = &state.backends;
            checks.push(async move {
                let id = backend.id().clone();
                if let Err(error) = registry.probe(backend).await {
                    tracing::debug!(backend = %id, %error, "could not learn this backend's catalogue at startup");
                }
            });
        }
        stream::iter(checks)
            .buffer_unordered(crate::state::BACKEND_CHECK_CONCURRENCY)
            .collect::<Vec<_>>()
            .await;
    }))
}

/// Open the trace ledger, or explain why we are running without one.
///
/// A ledger failure must never stop the proxy: the user's agent working matters
/// more than our bookkeeping.
fn open_ledger(paths: &PathsConfig, config: &Config, report: &mut StartupReport) -> Option<Ledger> {
    if !config.capture.enabled {
        return None;
    }
    match Ledger::open(&paths.ledger_file()) {
        Ok(ledger) => Some(ledger),
        Err(error) => {
            report.ledger_warning = Some(error.to_string());
            None
        }
    }
}

/// Open the body store, or explain why we are running without one.
///
/// Same rule as the ledger: bookkeeping never stops the proxy from serving.
fn open_bodies(
    paths: &PathsConfig,
    config: &Config,
    report: &mut StartupReport,
) -> Option<std::sync::Arc<BodyStore>> {
    if !config.capture.enabled || !config.capture.bodies {
        return None;
    }
    match BodyStore::open(&paths.bodies_dir()) {
        Ok(store) => Some(std::sync::Arc::new(store)),
        Err(error) => {
            report.bodies_warning = Some(error.to_string());
            None
        }
    }
}

/// Collect body files no ledger row claims. Never fatal: an uncollected
/// orphan costs disk, and failing to serve costs the user their agent.
fn sweep_bodies(ledger: &Option<Ledger>, bodies: Option<&BodyStore>) {
    let (Some(ledger), Some(bodies)) = (ledger.as_ref(), bodies) else {
        return;
    };
    match ledger
        .live_body_refs()
        .map_err(|e| e.to_string())
        .and_then(|live| bodies.retain_only(&live).map_err(|e| e.to_string()))
    {
        Ok(0) => {}
        Ok(removed) => tracing::info!(removed, "swept captured bodies no exchange claims"),
        Err(error) => tracing::warn!(%error, "could not sweep the captured bodies"),
    }
}

/// Construct every backend the environment can support.
///
/// Whether a backend with no credential is registered depends on who asked
/// for it (`docs/DESIGN.md` §6). Discovery registers only what it finds —
/// someone who never ran Claude Code is not shown a Claude subscription —
/// except NEAR AI, which `privacy.mode = "full"` names by default. A login the
/// user declared by directory is registered either way, so `status` can say
/// "not logged in — run …" rather than silently omitting it, which reads as
/// though IronWire never heard of it.
///
/// `credentials` is where every credential comes from — variables and files
/// both. It is a parameter rather than a set of direct calls so that an
/// embedding host can take the whole question over; see [`CredentialSource`].
fn build_registry(config: &Config, credentials: &Credentials<'_>) -> Result<BackendRegistry> {
    let timeout = config.server.upstream_timeout_secs;
    let mut registry = BackendRegistry::new();

    // `enabled = false` is filtered here, at the point of registration, rather
    // than by skipping the config entry. A backend can arrive from discovery
    // without any entry naming it, so filtering the config list would let a
    // backend the user switched off come straight back.
    let disabled: HashSet<_> = config
        .backends
        .iter()
        .filter(|entry| !entry.enabled)
        .map(|entry| entry.id.as_str())
        .collect();
    let mut push = |backend: Arc<dyn ironwire_upstream::backend::Backend>| -> Result<()> {
        if disabled.contains(backend.id().as_str()) {
            tracing::info!(backend = %backend.id(), "backend disabled in config.toml; not registered");
            return Ok(());
        }
        registry.try_push(backend).map_err(|duplicate| {
            tracing::error!(%duplicate, "not starting with two backends under one id");
            anyhow::Error::new(duplicate).context(RegistryLabel(
                "two backends would share one id; give each `[[backends]]` entry its own",
            ))
        })
    };

    if let Some(backend) = default_login(config, credentials, SubscriptionProduct::Claude, timeout)?
    {
        push(backend)?;
    }

    if let Some(key) = api_key_for(config, "anthropic-key", "ANTHROPIC_API_KEY", credentials) {
        push(Arc::new(
            AnthropicBackend::api_key(
                key,
                base_url_for(config, "anthropic-key", "IRONWIRE_ANTHROPIC_BASE_URL"),
                timeout,
            )
            .context(RegistryLabel("building the Anthropic API backend"))?,
        ))?;
    }

    // The ChatGPT subscription. Same wire as the metered OpenAI key below, so
    // the two are rungs of one ladder and falling between them costs nothing
    // but money (`docs/DESIGN.md` §3).
    if let Some(backend) =
        default_login(config, credentials, SubscriptionProduct::ChatGpt, timeout)?
    {
        push(backend)?;
    }

    // A key from the environment, or the one Codex itself stored — a user who
    // ran `codex login --api-key` has exactly one place they expect it to live.
    // That file is not read at all for a user who switched this backend off.
    let stored_key = || {
        if is_disabled(config, "openai-key") {
            None
        } else {
            codex_stored_key(credentials)
        }
    };
    if let Some(key) =
        api_key_for(config, "openai-key", "OPENAI_API_KEY", credentials).or_else(stored_key)
    {
        push(Arc::new(
            ResponsesBackend::openai_api_key(
                key,
                base_url_for(config, "openai-key", "IRONWIRE_OPENAI_BASE_URL"),
                timeout,
            )
            .context(RegistryLabel("building the OpenAI API backend"))?,
        ))?;
    }

    // NEAR AI is a different API family, so it is only ever reached through the
    // translated lane — and only at a turn boundary (`docs/PROTOCOL.md` §6).
    //
    // Registered whether or not a key was found, unlike the backends above. It
    // is the destination `privacy.mode = "full"` names by default, and a
    // trusted destination the user cannot see is one they cannot act on: with
    // no entry at all, `full` reads as broken rather than as needing a key.
    // Without one it reports `authenticated: false` and names what to set.
    push(Arc::new(
        ChatCompletionsBackend::nearai(
            api_key_for(config, "nearai", "NEARAI_API_KEY", credentials),
            base_url_for(config, "nearai", "IRONWIRE_NEARAI_BASE_URL"),
            models_for(config, "nearai", BackendKind::Credits).unwrap_or_default(),
            timeout,
        )
        .context(RegistryLabel("building the NEAR AI backend"))?,
    ))?;

    // Anything the user declared that discovery does not produce — including
    // every additional subscription login, in the order the file lists them.
    // An entry naming one of discovery's own ids configures that backend
    // rather than constructing a second one.
    //
    // Appended rather than prepended: registration order is the tie-break in
    // `Policy::select`, so putting config entries first would silently change
    // which backend wins a tie for every existing user. Among logins of one
    // product it is the order cross-account fallback tries them in.
    for entry in &config.backends {
        if !entry.enabled || BackendImpl::discovered_as(&entry.id).is_some() {
            continue;
        }
        if let Some(backend) = backend_from_config(entry, timeout, credentials)? {
            push(backend)?;
        }
    }

    Ok(registry)
}

/// The default login of `product`, if it is to be registered.
///
/// Discovered where the client keeps it unprompted, and registered only when
/// a login is there: someone who never ran Claude Code should not be shown a
/// Claude subscription. An entry for the default id with a `config_dir` moves
/// the login, and then the user has named the directory — so, like every login
/// a config declares, it is registered whether or not it holds a login yet.
///
/// Nothing is read when the files may not be, or when the user switched this
/// login off: neither start has any business looking at the disk.
fn default_login(
    config: &Config,
    credentials: &Credentials<'_>,
    product: SubscriptionProduct,
    timeout: u64,
) -> Result<Option<Arc<dyn ironwire_upstream::backend::Backend>>> {
    let id = product.default_backend_id();
    if !credentials.discovers_files() || is_disabled(config, id) {
        return Ok(None);
    }
    let entry = config.backends.iter().find(|entry| entry.id == id);
    let relocated = entry.and_then(|entry| entry.config_dir.as_deref());
    let present = || match product {
        SubscriptionProduct::Claude => ClaudeCodeCredentials::discover().is_ok(),
        // An `auth.json` holding an API key is the metered key, not a plan.
        SubscriptionProduct::ChatGpt => {
            CodexCredentials::discover().is_ok_and(|c| c.mode == CodexMode::ChatGpt)
        }
    };
    if relocated.is_none() && !present() {
        return Ok(None);
    }
    subscription_login(
        product,
        id,
        relocated,
        base_url_of(entry, base_url_variable(product)),
        timeout,
    )
    .map(Some)
}

/// The backend for one subscription login: the one in `config_dir`, or the
/// product's default login for `None`.
///
/// Named by its login, so two logins of one product never share a heading in
/// `status`. Reads nothing: whether the login is there is the backend's
/// question, asked again on every request.
fn subscription_login(
    product: SubscriptionProduct,
    id: &str,
    config_dir: Option<&str>,
    base_url: Option<String>,
    timeout: u64,
) -> Result<Arc<dyn ironwire_upstream::backend::Backend>> {
    let default = id == product.default_backend_id();
    Ok(match product {
        SubscriptionProduct::Claude => {
            let login = config_dir.map_or_else(ClaudeLogin::default_login, ClaudeLogin::in_dir);
            let name = login.describe(default).label();
            Arc::new(
                AnthropicBackend::subscription_for(
                    BackendId::from(id),
                    name,
                    login,
                    base_url,
                    timeout,
                )
                .context(RegistryLabel("building a Claude subscription backend"))?,
            )
        }
        SubscriptionProduct::ChatGpt => {
            let home = config_dir.map(std::path::PathBuf::from);
            let name = home
                .clone()
                .map_or_else(CodexLogin::default_login, CodexLogin::in_home)
                .describe(default)
                .label();
            Arc::new(
                ResponsesBackend::codex_subscription_for(
                    BackendId::from(id),
                    name,
                    home,
                    base_url,
                    timeout,
                )
                .context(RegistryLabel("building a ChatGPT subscription backend"))?,
            )
        }
    })
}

/// The variable the conformance harness uses to point a product's backends at
/// a mock. Every login of the product follows it: a harness that redirected the
/// default login and left a second one dialling the real provider would be
/// sending a real token somewhere the test never meant to.
fn base_url_variable(product: SubscriptionProduct) -> &'static str {
    match product {
        SubscriptionProduct::Claude => "IRONWIRE_ANTHROPIC_BASE_URL",
        SubscriptionProduct::ChatGpt => "IRONWIRE_CODEX_BASE_URL",
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct RegistryLabel(&'static str);

/// Build a backend that discovery would not have produced.
///
/// `Ok(None)` when the credential is simply absent — the same rule discovery
/// follows, where not being logged in is a normal state rather than an error.
///
/// # Errors
///
/// Propagates a client build failure. Configuration that cannot describe a
/// backend at all is rejected earlier, by `Config::validate`, so that a bad
/// file fails before the port is bound rather than at the first request.
fn backend_from_config(
    entry: &ironwire_core::config::BackendConfig,
    timeout: u64,
    credentials: &Credentials<'_>,
) -> Result<Option<Arc<dyn ironwire_upstream::backend::Backend>>> {
    let key = |default: &str| entry_key(entry, default, credentials);
    let backend: Option<Arc<dyn ironwire_upstream::backend::Backend>> =
        match BackendImpl::parse(&entry.kind) {
            // An additional login. Registered whether or not its directory
            // holds a login yet: the user named it, and "not logged in — run
            // …" in `status` is what they need to see, where silence would
            // read as IronWire never having heard of it. An unauthenticated
            // backend is already unhealthy to the router, so nothing is sent.
            //
            // Still behind the files switch, because its token is read from
            // that directory on every request.
            Some(kind @ (BackendImpl::ClaudeSubscription | BackendImpl::CodexSubscription)) => {
                if !credentials.discovers_files() {
                    return Ok(None);
                }
                // Unreachable for a config that came through `Config::validate`,
                // which requires a directory on every login but the default's.
                // Building one without would be the default login again under
                // another id — the copy this entry used to silently become.
                let Some(config_dir) = entry.config_dir.as_deref() else {
                    return Ok(None);
                };
                let product = if kind == BackendImpl::ClaudeSubscription {
                    SubscriptionProduct::Claude
                } else {
                    SubscriptionProduct::ChatGpt
                };
                Some(subscription_login(
                    product,
                    &entry.id,
                    Some(config_dir),
                    base_url_of(Some(entry), base_url_variable(product)),
                    timeout,
                )?)
            }
            Some(BackendImpl::AnthropicApi) => key("ANTHROPIC_API_KEY")
                .map(|key| {
                    AnthropicBackend::api_key(key, entry.base_url.clone(), timeout)
                        .context(RegistryLabel("building a configured Anthropic API backend"))
                })
                .transpose()?
                .map(|b| Arc::new(b) as Arc<dyn ironwire_upstream::backend::Backend>),
            Some(BackendImpl::OpenAiApi) => key("OPENAI_API_KEY")
                .or_else(|| codex_stored_key(credentials))
                .map(|key| {
                    ResponsesBackend::openai_api_key(key, entry.base_url.clone(), timeout)
                        .context(RegistryLabel("building a configured OpenAI API backend"))
                })
                .transpose()?
                .map(|b| Arc::new(b) as Arc<dyn ironwire_upstream::backend::Backend>),
            Some(BackendImpl::NearAi) => key("NEARAI_API_KEY")
                .map(|key| {
                    ChatCompletionsBackend::nearai(
                        Some(key),
                        entry.base_url.clone(),
                        entry
                            .models
                            .as_ref()
                            .map(|models| models_from(models, BackendKind::Credits))
                            .unwrap_or_default(),
                        timeout,
                    )
                    .map(|backend| backend.refusing_model_aliases(entry.refuse_model_aliases))
                    .context(RegistryLabel("building a configured NEAR AI backend"))
                })
                .transpose()?
                .map(|b| Arc::new(b) as Arc<dyn ironwire_upstream::backend::Backend>),
            Some(BackendImpl::Local) => {
                // `validate` has already established the base URL. The key is
                // genuinely optional here: most local servers take no auth, and
                // `None` sends no `Authorization` header at all.
                let Some(base_url) = entry.base_url.clone() else {
                    return Ok(None);
                };
                Some(Arc::new(
                    ChatCompletionsBackend::local(
                        BackendId::from(entry.id.as_str()),
                        &entry.id,
                        base_url,
                        entry
                            .api_key_env
                            .as_deref()
                            .and_then(|name| credentials.key(name)),
                        entry
                            .models
                            .as_ref()
                            .map(|models| models_from(models, BackendKind::Local))
                            .unwrap_or_default(),
                        timeout,
                    )
                    .context(RegistryLabel("building a configured local backend"))?,
                )
                    as Arc<dyn ironwire_upstream::backend::Backend>)
            }
            Some(BackendImpl::OpenAiCompatible) => {
                // `validate` has already established that both are present.
                let (Some(base_url), Some(key)) = (entry.base_url.clone(), key("")) else {
                    return Ok(None);
                };
                Some(Arc::new(
                    ChatCompletionsBackend::new(
                        BackendId::from(entry.id.as_str()),
                        &entry.id,
                        BackendKind::ApiKey,
                        Some(key),
                        base_url,
                        entry
                            .models
                            .as_ref()
                            .map(|models| models_from(models, BackendKind::ApiKey))
                            .unwrap_or_default(),
                        timeout,
                    )
                    .context(RegistryLabel(
                        "building a configured OpenAI-compatible backend",
                    ))?,
                )
                    as Arc<dyn ironwire_upstream::backend::Backend>)
            }
            // Unreachable for a config that came through `Config::validate`.
            None => None,
        };
    Ok(backend)
}

/// Reading a credential by variable name, as a value so tests need not mutate a
/// global that every other test in the process shares.
///
/// [`SecretString`] rather than `String` because every caller wants one, and a
/// value that spends part of its life as a plain `String` is a value with a
/// `Debug` rendering and no zeroize on drop for that stretch.
///
/// `Send + Sync` because a [`Credentials`] built from this is held across an
/// await in [`start_with_options`], which makes it part of that future.
/// Without the bounds the future is not `Send`, and an embedding host cannot
/// `tokio::spawn` a start onto a multi-threaded runtime. Nothing in this
/// repository spawns one, so nothing here fails without them.
type EnvLookup<'a> = &'a (dyn Fn(&str) -> Option<SecretString> + Send + Sync);

/// The real environment.
fn real_env(name: &str) -> Option<SecretString> {
    std::env::var(name).ok().map(SecretString::from)
}

/// What answers for a credential while the registry is built.
///
/// Credential discovery is not one lookup but four — environment variables, the
/// file Claude Code writes, the file Codex writes, and the key Codex stores
/// after `codex login --api-key` — and a host that supplies a source can only
/// speak for the first. That used to mean the other three were switched off
/// with it, because one value answered both questions.
///
/// Two fields rather than one variant, so the two questions are answered
/// independently at the point each is asked: `key` for the names, `files` for
/// the disk. Their defaults still move together — a host source implies no
/// files — which is what keeps every existing caller where it was.
struct Credentials<'a> {
    /// Who answers for a name.
    keys: Keys<'a>,
    /// Whether the credential files those two products write may be read.
    files: bool,
}

/// Whose answer counts for a credential name.
///
/// Exclusive on purpose: letting the environment fill in behind a host would
/// register a backend from a variable the host never authorized, which is the
/// surprise a host declares ownership in order to avoid.
enum Keys<'a> {
    /// The process environment. What the CLI does, and what every caller that
    /// supplies no source gets.
    Discovered(EnvLookup<'a>),
    /// Only what the host answers.
    HostOwned(&'a CredentialSource),
}

impl<'a> Credentials<'a> {
    /// IronWire owns both questions: the environment for names, and the files.
    fn discovered(env: EnvLookup<'a>) -> Self {
        Self {
            keys: Keys::Discovered(env),
            files: true,
        }
    }

    /// The host owns the names. Files stay off unless it says otherwise, which
    /// is what [`CredentialSource`] has always promised.
    fn host_owned(host: &'a CredentialSource) -> Self {
        Self {
            keys: Keys::HostOwned(host),
            files: false,
        }
    }

    /// Apply the host's answer to the second question, leaving the first alone.
    fn with_files(mut self, files: CredentialFiles) -> Self {
        self.files = match files {
            CredentialFiles::FollowCredentialOwner => self.files,
            CredentialFiles::Discover => true,
            CredentialFiles::Off => false,
        };
        self
    }

    /// The credential for a variable name, from whichever source is in charge.
    ///
    /// An empty answer is no answer, the rule an empty environment variable has
    /// always followed here.
    fn key(&self, name: &str) -> Option<SecretString> {
        match &self.keys {
            Keys::Discovered(env) => env(name),
            Keys::HostOwned(host) => host(name),
        }
        .filter(non_empty)
    }

    /// Whether the credential files Claude Code and Codex write may be read.
    ///
    /// Checked before `discover()` rather than after, so a start that may not
    /// read them does not go looking at a real user's home at all.
    fn discovers_files(&self) -> bool {
        self.files
    }
}

/// An empty credential is an absent credential.
fn non_empty(key: &SecretString) -> bool {
    !secrecy::ExposeSecret::expose_secret(key).is_empty()
}

/// The key for a config entry: the variable it names, or the default name.
fn entry_key(
    entry: &ironwire_core::config::BackendConfig,
    default_env: &str,
    credentials: &Credentials<'_>,
) -> Option<SecretString> {
    let name = entry.api_key_env.as_deref().unwrap_or(default_env);
    if name.is_empty() {
        return None;
    }
    credentials.key(name)
}

/// The key for a discovered backend, honouring an `api_key_env` override.
///
/// A user whose key lives in `ANTHROPIC_API_KEY_WORK` had no way to say so:
/// the name was hardcoded, and the field that exists to name it was never read.
fn api_key_for(
    config: &Config,
    id: &str,
    default_env: &str,
    credentials: &Credentials<'_>,
) -> Option<SecretString> {
    let name = config
        .backends
        .iter()
        .find(|entry| entry.id == id)
        .and_then(|entry| entry.api_key_env.as_deref())
        .unwrap_or(default_env);
    credentials.key(name)
}

/// Whether the user switched this backend off.
///
/// Checked where a backend is registered rather than where a config entry is
/// read: discovery produces backends no entry names, so filtering the config
/// list would let a disabled backend come straight back.
fn is_disabled(config: &Config, id: &str) -> bool {
    config
        .backends
        .iter()
        .any(|entry| entry.id == id && !entry.enabled)
}

/// The metered OpenAI key Codex itself stored, for a user who ran
/// `codex login --api-key` and expects it to be found there.
///
/// A file, not a variable, so it follows [`CredentialFiles`] rather than
/// [`CredentialSource`]: it is offered exactly when the credential files may be
/// read. A host that owns credentials and leaves the files alone is not handed
/// it, because the whole point of owning them is that no key arrives from
/// somewhere the host did not name — and a host that opts the files back in has
/// asked for what is on that disk, this key included.
fn codex_stored_key(credentials: &Credentials<'_>) -> Option<SecretString> {
    if !credentials.discovers_files() {
        return None;
    }
    let creds = CodexCredentials::discover().ok()?;
    (creds.mode == CodexMode::ApiKey).then(|| creds.bearer().token)
}

/// Models configured for a backend, if the user listed any.
///
/// Applies to every kind that accepts a catalogue, not only NEAR AI: a
/// configured list was previously read for one id and silently ignored for
/// every other. `None` means "nothing configured", which is different from an
/// empty list and leaves the provider's own catalogue in charge.
fn models_for(config: &Config, id: &str, kind: BackendKind) -> Option<Vec<(String, ModelTier)>> {
    config
        .backends
        .iter()
        .find(|entry| entry.id == id)
        .and_then(|entry| entry.models.as_ref())
        .map(|models| models_from(models, kind))
}

/// Tier each configured slug for a backend of `kind`.
///
/// The tier is not a property of the name alone: the same slug means something
/// different on local capacity, which sorts cheapest and must not inherit a
/// frontier tier from a name nobody recognises (`ModelEntry::tier_on`).
fn models_from(models: &[ModelEntry], kind: BackendKind) -> Vec<(String, ModelTier)> {
    models
        .iter()
        .map(|entry| (entry.name().to_string(), entry.tier_on(kind)))
        .collect()
}

/// Base-URL override for a backend.
///
/// `config.toml` is the user-facing form; the environment variable exists so
/// the conformance harness can point a real backend at a recording mock
/// without writing config (`docs/PROTOCOL.md` §7.2).
///
/// The environment variable is named per family rather than shared. A single
/// override would send *every* backend to whichever mock the harness happened to
/// start — including one holding a credential for a different provider, which
/// `check_host` would then have to refuse (`docs/TRUST.md` I2).
fn base_url_for(config: &Config, id: &str, env_key: &str) -> Option<String> {
    base_url_of(config.backends.iter().find(|b| b.id == id), env_key)
}

/// [`base_url_for`], for an entry already in hand.
fn base_url_of(
    entry: Option<&ironwire_core::config::BackendConfig>,
    env_key: &str,
) -> Option<String> {
    entry
        .and_then(|b| b.base_url.clone())
        .or_else(|| std::env::var(env_key).ok())
        .filter(|url| !url.is_empty())
}

fn apply_token_capture_override(
    config: &mut Config,
    enabled: Option<bool>,
) -> Result<(), EmbedError> {
    match enabled {
        Some(true)
            if config
                .capture
                .token_capture
                .as_ref()
                .is_none_or(|c| c.targets.is_empty()) =>
        {
            return Err(EmbedError::Config);
        }
        Some(false) => config.capture.token_capture = None,
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn host_capture_override_preserves_targets_and_requires_explicit_configuration() {
        use ironwire_core::config::{Config, TokenCaptureConfig, TokenCaptureTarget};
        let mut config = Config::default();
        assert!(super::apply_token_capture_override(&mut config, Some(true)).is_err());
        config.capture.token_capture = Some(TokenCaptureConfig {
            targets: vec![TokenCaptureTarget {
                backend: "fixture".into(),
                model: "qualified".into(),
            }],
            ..Default::default()
        });
        super::apply_token_capture_override(&mut config, None).unwrap();
        super::apply_token_capture_override(&mut config, Some(true)).unwrap();
        assert_eq!(
            config.capture.token_capture.as_ref().unwrap().targets[0].model,
            "qualified"
        );
        super::apply_token_capture_override(&mut config, Some(false)).unwrap();
        assert!(config.capture.token_capture.is_none());
    }

    use super::*;
    use ironwire_core::config::BackendConfig;

    pub(super) fn entry(id: &str, kind: &str) -> BackendConfig {
        BackendConfig {
            id: id.to_string(),
            kind: kind.to_string(),
            enabled: true,
            base_url: None,
            api_key_env: None,
            config_dir: None,
            models: None,
            refuse_model_aliases: false,
        }
    }

    pub(super) fn env_with(
        pairs: &[(&str, &str)],
    ) -> impl Fn(&str) -> Option<SecretString> + use<> {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |name: &str| {
            owned
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| SecretString::from(value.clone()))
        }
    }

    /// A host source over a fixed table, in the shape a host would supply one.
    fn host_with(pairs: &[(&str, &str)]) -> CredentialSource {
        Arc::new(env_with(pairs))
    }

    fn exposed(key: &SecretString) -> &str {
        secrecy::ExposeSecret::expose_secret(key)
    }

    /// The field existed and was never read, so a key that did not live under
    /// the hardcoded name was unreachable.
    #[test]
    fn api_key_env_names_the_variable_the_key_is_read_from() {
        let env = env_with(&[("WORK_KEY", "sk-work"), ("ANTHROPIC_API_KEY", "sk-default")]);
        let mut config = Config::default();
        let mut declared = entry("anthropic-key", "anthropic-api");
        declared.api_key_env = Some("WORK_KEY".to_string());
        config.backends.push(declared);

        let key = api_key_for(
            &config,
            "anthropic-key",
            "ANTHROPIC_API_KEY",
            &Credentials::discovered(&env),
        )
        .expect("a key was configured");
        assert_eq!(secrecy::ExposeSecret::expose_secret(&key), "sk-work");
    }

    #[test]
    fn the_default_variable_is_used_when_none_is_named() {
        let env = env_with(&[("ANTHROPIC_API_KEY", "sk-default")]);
        let key = api_key_for(
            &Config::default(),
            "anthropic-key",
            "ANTHROPIC_API_KEY",
            &Credentials::discovered(&env),
        )
        .expect("a key was configured");
        assert_eq!(secrecy::ExposeSecret::expose_secret(&key), "sk-default");
    }

    /// The kind that could not be built at all before this: the one documented
    /// route to an endpoint IronWire does not discover.
    #[test]
    fn an_openai_compatible_entry_builds_a_backend_at_its_own_url() {
        let mut declared = entry("local", "openai-compatible");
        declared.base_url = Some("http://127.0.0.1:11434/v1".to_string());
        declared.api_key_env = Some("LOCAL_KEY".to_string());
        declared.models = Some(vec![ModelEntry::Name("qwen3-coder".to_string())]);

        let env = env_with(&[("LOCAL_KEY", "sk-local")]);
        let built = backend_from_config(&declared, 60, &Credentials::discovered(&env))
            .expect("builds")
            .expect("a credential was available");
        assert_eq!(built.id().as_str(), "local");
        assert_eq!(
            built.models(),
            vec![("qwen3-coder".to_string(), ModelTier::Frontier)]
        );
    }

    /// A missing credential is a normal state, not an error — the same rule
    /// discovery follows.
    #[test]
    fn an_entry_whose_key_is_absent_is_skipped_rather_than_failing() {
        let mut declared = entry("local", "openai-compatible");
        declared.base_url = Some("http://127.0.0.1:11434/v1".to_string());
        declared.api_key_env = Some("LOCAL_KEY".to_string());

        let env = env_with(&[]);
        let built =
            backend_from_config(&declared, 60, &Credentials::discovered(&env)).expect("no error");
        assert!(built.is_none());
    }

    /// The landmine: a backend discovery produces on its own must still be
    /// switched off by config, or `enabled = false` is a no-op that reads like
    /// a kill switch.
    #[test]
    fn a_discovered_backend_can_be_disabled_by_config() {
        let mut config = Config::default();
        let mut declared = entry("claude-sub", "claude-subscription");
        declared.enabled = false;
        config.backends.push(declared);

        assert!(is_disabled(&config, "claude-sub"));
        assert!(
            !is_disabled(&config, "codex-sub"),
            "unrelated backends stay"
        );
        assert!(
            !is_disabled(&Config::default(), "claude-sub"),
            "a config with no entries disables nothing"
        );
    }

    #[test]
    fn a_configured_catalogue_is_read_for_any_backend_not_just_nearai() {
        let mut config = Config::default();
        let mut declared = entry("anthropic-key", "anthropic-api");
        declared.models = Some(vec![ModelEntry::Name("claude-haiku-4-5".to_string())]);
        config.backends.push(declared);

        assert_eq!(
            models_for(&config, "anthropic-key", BackendKind::ApiKey),
            Some(vec![("claude-haiku-4-5".to_string(), ModelTier::Fast)])
        );
        assert_eq!(
            models_for(&config, "nearai", BackendKind::Credits),
            None,
            "nothing configured is not the same as an empty catalogue"
        );
    }

    /// The point of the option: a credential that never touched the process
    /// environment, and so never needed `set_var` and was never readable by
    /// anything else sharing the process.
    #[test]
    fn a_host_source_answers_a_name_the_environment_cannot() {
        let host = host_with(&[("NEARAI_API_KEY", "sk-from-host")]);

        let key = Credentials::host_owned(&host)
            .key("NEARAI_API_KEY")
            .expect("the host answered");
        assert_eq!(exposed(&key), "sk-from-host");
    }

    /// Replacement, not precedence. A stray variable in the host's environment
    /// must not register a backend the host never authorized — the same
    /// surprise `StartupProbes::Configured` exists to prevent.
    #[test]
    fn a_host_owned_start_does_not_read_the_environment_at_all() {
        let host = host_with(&[("NEARAI_API_KEY", "sk-from-host")]);
        let credentials = Credentials::host_owned(&host);

        assert!(
            credentials.key("ANTHROPIC_API_KEY").is_none(),
            "a name the host does not answer has no credential, wherever else it may be set"
        );
        // `PATH` stands in for a variable that is genuinely set in this
        // process, so this asserts the environment was not consulted rather
        // than that it happened to be empty.
        assert!(std::env::var_os("PATH").is_some(), "PATH is set");
        assert!(credentials.key("PATH").is_none());
        assert_eq!(
            exposed(&credentials.key("NEARAI_API_KEY").expect("a key")),
            "sk-from-host",
            "and the one it does answer is the host's own"
        );
    }

    /// An empty answer is no answer, the rule an empty environment variable has
    /// always followed here — and under replacement it means no credential,
    /// rather than a reason to go looking elsewhere.
    #[test]
    fn an_empty_host_answer_is_no_credential() {
        let host = host_with(&[("NEARAI_API_KEY", "")]);

        assert!(
            Credentials::host_owned(&host)
                .key("NEARAI_API_KEY")
                .is_none()
        );
    }

    /// The non-negotiable: a host that supplies nothing is exactly where it
    /// was, name for name, including names the environment does not have.
    #[test]
    fn discovery_reads_the_environment_unchanged() {
        let env = env_with(&[("NEARAI_API_KEY", "sk-from-env"), ("EMPTY_KEY", "")]);
        let credentials = Credentials::discovered(&env);

        for name in ["NEARAI_API_KEY", "EMPTY_KEY", "UNSET_KEY"] {
            let through = credentials.key(name).map(|key| exposed(&key).to_string());
            let direct = env(name)
                .filter(non_empty)
                .map(|key| exposed(&key).to_string());
            assert_eq!(through, direct, "{name} must read the same either way");
        }
        assert!(
            EmbedOptions::default().credentials.is_none(),
            "the default supplies no host source"
        );
    }

    /// End to end through the seam the host is actually buying: the registry
    /// authenticates a backend from a credential the environment never held.
    #[tokio::test]
    async fn a_host_key_authenticates_the_backend_the_registry_builds() {
        let config = Config::default();
        let host = host_with(&[("NEARAI_API_KEY", "sk-from-host")]);
        let empty = env_with(&[]);

        let with_host = build_registry(&config, &Credentials::host_owned(&host)).expect("builds");
        let without = build_registry(&config, &Credentials::discovered(&empty)).expect("builds");

        assert!(status_of(&with_host, "nearai").await.authenticated);
        assert!(
            !status_of(&without, "nearai").await.authenticated,
            "registered but unauthenticated is still the no-credential state"
        );
    }

    /// The half a variable lookup cannot reach. Claude Code and Codex
    /// credentials come from files, so exposing the environment alone would
    /// leave a host that declared it owns credentials still routing through a
    /// subscription login it never asked about — the exact failure this is for.
    ///
    /// Asserted in the direction that holds on any machine: under host
    /// ownership these backends are absent whether or not the developer running
    /// the suite happens to be logged in. The converse is only observable where
    /// a real credential exists, so it is asserted where one does.
    #[tokio::test]
    async fn a_host_that_owns_credentials_gets_no_file_discovered_backend() {
        let config = Config::default();
        let host = host_with(&[("NEARAI_API_KEY", "sk-from-host")]);
        let empty = env_with(&[]);

        let owned = build_registry(&config, &Credentials::host_owned(&host)).expect("builds");
        assert!(
            subscriptions(&owned).is_empty(),
            "{:?} registered from a file the host never named",
            subscriptions(&owned)
        );

        let discovered = build_registry(&config, &Credentials::discovered(&empty)).expect("builds");
        if ClaudeCodeCredentials::discover().is_ok() {
            assert!(
                discovered.get(&BackendId::from("claude-sub")).is_some(),
                "a logged-in user must still get their subscription by default"
            );
        }
        if CodexCredentials::discover().is_ok_and(|c| c.mode == CodexMode::ChatGpt) {
            assert!(discovered.get(&BackendId::from("codex-sub")).is_some());
        }
    }

    /// Neither question moves on its own. The whole risk of adding a second
    /// knob is that it silently changes what the first one answered, so this
    /// pins both defaults: IronWire owns the files when it owns the names, and
    /// a host that says nothing about files still gets none of them.
    #[test]
    fn both_questions_default_to_what_they_have_always_answered() {
        let env = env_with(&[]);
        let host = host_with(&[]);

        assert!(
            Credentials::discovered(&env)
                .with_files(CredentialFiles::default())
                .discovers_files(),
            "a start with no host source reads the credential files, as it always has"
        );
        assert!(
            !Credentials::host_owned(&host)
                .with_files(CredentialFiles::default())
                .discovers_files(),
            "and a host that owns credentials and says nothing about files reads none"
        );
        assert_eq!(
            EmbedOptions::default().credential_files,
            CredentialFiles::FollowCredentialOwner,
            "which is what the default option value selects"
        );
    }

    /// The case the API could not express, and the reason for the change: the
    /// host answers for its own key while a contributor's existing subscription
    /// goes on answering for itself.
    ///
    /// The name half must not move with it. A host that opts into files has
    /// asked for what is on that disk, not for the process environment to start
    /// registering backends again.
    #[test]
    fn a_host_can_own_the_names_and_still_permit_the_files() {
        let host = host_with(&[("NEARAI_API_KEY", "sk-from-host")]);
        let credentials = Credentials::host_owned(&host).with_files(CredentialFiles::Discover);

        assert!(credentials.discovers_files(), "the files are readable");
        assert_eq!(
            exposed(&credentials.key("NEARAI_API_KEY").expect("a key")),
            "sk-from-host",
            "and the host still answers for the names it holds"
        );
        assert!(std::env::var_os("PATH").is_some(), "PATH is set");
        assert!(
            credentials.key("PATH").is_none(),
            "replacement survives: the environment is still not consulted"
        );
    }

    /// The other direction, which the one-bit form could not express either: a
    /// host content to let IronWire read the environment it started the process
    /// with, but not a real user's home for logins it never asked about.
    #[test]
    fn the_files_can_be_declined_while_ironwire_still_owns_the_names() {
        let env = env_with(&[("NEARAI_API_KEY", "sk-from-env")]);
        let credentials = Credentials::discovered(&env).with_files(CredentialFiles::Off);

        assert!(!credentials.discovers_files());
        assert_eq!(
            exposed(&credentials.key("NEARAI_API_KEY").expect("a key")),
            "sk-from-env",
            "the environment answers exactly as it did"
        );
        assert!(
            codex_stored_key(&credentials).is_none(),
            "including the one credential that is both a file and a key"
        );
    }

    /// The consequence worth stating: with the host answering nothing and the
    /// unconditional NEAR AI entry switched off, a proxy legitimately has no
    /// backends at all. `StartupReport::no_backends` already models this state;
    /// until now almost nothing could reach it.
    #[test]
    fn a_host_that_answers_nothing_can_leave_the_registry_empty() {
        let mut config = Config::default();
        config.backends.push({
            let mut nearai = entry("nearai", "nearai");
            nearai.enabled = false;
            nearai
        });
        let host = host_with(&[]);

        let registry = build_registry(&config, &Credentials::host_owned(&host)).expect("builds");
        assert!(registry.is_empty());
    }

    /// Every subscription backend in a registry, by id — whatever its id is.
    fn subscriptions(registry: &BackendRegistry) -> Vec<String> {
        registry
            .all()
            .iter()
            .filter(|backend| backend.kind() == BackendKind::Subscription)
            .map(|backend| backend.id().to_string())
            .collect()
    }

    /// A config whose discovered backends are all switched off, so building
    /// it never looks at this developer's own logins or Keychain, plus `extra`.
    fn config_with(extra: Vec<BackendConfig>) -> Config {
        let mut config = Config::default();
        for (id, kind) in [
            ("claude-sub", "claude-subscription"),
            ("codex-sub", "codex-subscription"),
            ("openai-key", "openai-api"),
        ] {
            let mut off = entry(id, kind);
            off.enabled = false;
            config.backends.push(off);
        }
        config.backends.extend(extra);
        config
    }

    /// A directory's last component — what survives the home being shown as
    /// `~` when the temporary directory happens to live under it.
    fn leaf(dir: &std::path::Path) -> String {
        dir.file_name()
            .expect("a named directory")
            .to_string_lossy()
            .into_owned()
    }

    fn login_entry(id: &str, kind: &str, dir: &std::path::Path) -> BackendConfig {
        let mut declared = entry(id, kind);
        declared.config_dir = Some(dir.display().to_string());
        declared
    }

    /// The gate covers every login, not two literal ids. Asserted on kind, so
    /// a login added under any id is covered — and with a real credential in
    /// each directory, so "not registered" means "not read", not "not there".
    #[test]
    fn a_start_that_may_not_read_the_files_registers_no_login_of_any_kind() {
        let claude_dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            claude_dir.path().join(".credentials.json"),
            r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-work"}}"#,
        )
        .expect("writes");
        let codex_dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            codex_dir.path().join("auth.json"),
            r#"{"auth_mode":"chatgpt","tokens":{"access_token":"eyJ","refresh_token":"rt","account_id":"a"}}"#,
        )
        .expect("writes");
        let config = config_with(vec![
            login_entry("claude-work", "claude-subscription", claude_dir.path()),
            login_entry("codex-work", "codex-subscription", codex_dir.path()),
        ]);
        let host = host_with(&[]);
        let env = env_with(&[]);

        for (what, credentials) in [
            (
                "a host that owns credentials",
                Credentials::host_owned(&host),
            ),
            (
                "a start that declined the files",
                Credentials::discovered(&env).with_files(CredentialFiles::Off),
            ),
        ] {
            let registry = build_registry(&config, &credentials).expect("builds");
            assert!(
                subscriptions(&registry).is_empty(),
                "{what}: {:?} registered",
                subscriptions(&registry)
            );
        }

        // The control: the same config, files allowed, registers both.
        let registry = build_registry(&config, &Credentials::discovered(&env)).expect("builds");
        assert_eq!(subscriptions(&registry), vec!["claude-work", "codex-work"]);
    }

    /// The latent bug this change found: an entry for a second subscription used
    /// to build the *default* login again under the default's id — one login
    /// counted twice, sharing consent and quota, the second copy unreachable.
    #[test]
    fn a_second_subscription_entry_is_its_own_backend_not_a_copy_of_the_first() {
        let claude_dir = tempfile::tempdir().expect("tempdir");
        let codex_dir = tempfile::tempdir().expect("tempdir");
        let config = config_with(vec![
            login_entry("claude-work", "claude-subscription", claude_dir.path()),
            login_entry("codex-work", "codex-subscription", codex_dir.path()),
        ]);
        let env = env_with(&[]);
        let registry = build_registry(&config, &Credentials::discovered(&env)).expect("builds");

        for (id, dir, product) in [
            (
                "claude-work",
                claude_dir.path(),
                SubscriptionProduct::Claude,
            ),
            ("codex-work", codex_dir.path(), SubscriptionProduct::ChatGpt),
        ] {
            let backend = registry
                .get(&BackendId::from(id))
                .unwrap_or_else(|| panic!("{id} is registered under its own id"));
            let login = backend.login().expect("a subscription reads a login");
            assert_eq!(login.product, product);
            assert!(!login.default, "{id} is not the default login");
            let location = login.location.clone().expect("its own directory");
            assert!(location.ends_with(&leaf(dir)), "{location}");
            assert!(
                backend.name().contains(&location),
                "{id} is named by its login, so two never share a heading: {}",
                backend.name()
            );
        }
        assert_eq!(
            subscriptions(&registry),
            vec!["claude-work", "codex-work"],
            "and nothing else: no copy under a default id"
        );
    }

    /// A declared login is registered before it is logged in, so `status` can
    /// say what to run rather than leave it out — which reads as IronWire never
    /// having heard of it. Codex here, because its store is a file on every
    /// platform; the Claude half runs in a child process (`tests/embed.rs`).
    #[tokio::test]
    async fn a_declared_login_with_nothing_in_its_directory_says_how_to_log_in() {
        let codex_dir = tempfile::tempdir().expect("tempdir");
        let config = config_with(vec![login_entry(
            "codex-work",
            "codex-subscription",
            codex_dir.path(),
        )]);
        let env = env_with(&[]);
        let registry = build_registry(&config, &Credentials::discovered(&env)).expect("builds");

        let status = status_of(&registry, "codex-work").await;
        assert!(!status.authenticated);
        let detail = status.detail.expect("says why");
        assert!(
            detail.contains("CODEX_HOME=") && detail.contains("codex login"),
            "{detail}"
        );
    }

    /// The order logins are declared in is the order cross-account fallback
    /// would try them, so registration must keep it.
    #[test]
    fn additional_logins_register_in_the_order_the_config_lists_them() {
        let dirs: Vec<_> = (0..3)
            .map(|_| tempfile::tempdir().expect("tempdir"))
            .collect();
        let config = config_with(vec![
            login_entry("claude-play", "claude-subscription", dirs[0].path()),
            login_entry("claude-work", "claude-subscription", dirs[1].path()),
            login_entry("claude-client", "claude-subscription", dirs[2].path()),
        ]);
        let env = env_with(&[]);
        let registry = build_registry(&config, &Credentials::discovered(&env)).expect("builds");
        assert_eq!(
            subscriptions(&registry),
            vec!["claude-play", "claude-work", "claude-client"]
        );
    }

    /// An entry for a default id with a directory moves that login. It keeps
    /// the id — quota and history are recorded under it — and is still the
    /// default. It is not the plain façade's, though: its client is reached
    /// through the login-bound URL (`state::tests`).
    #[test]
    fn a_default_login_can_be_moved_and_is_still_the_default() {
        let moved = tempfile::tempdir().expect("tempdir");
        let mut config = config_with(Vec::new());
        config.backends.retain(|entry| entry.id != "claude-sub");
        config.backends.push(login_entry(
            "claude-sub",
            "claude-subscription",
            moved.path(),
        ));
        let env = env_with(&[]);
        let registry = build_registry(&config, &Credentials::discovered(&env)).expect("builds");

        let backend = registry
            .get(&BackendId::from("claude-sub"))
            .expect("registered, though nothing is logged in there yet");
        let login = backend.login().expect("a login");
        assert!(login.default);
        assert!(
            login
                .location
                .as_deref()
                .is_some_and(|location| location.ends_with(&leaf(moved.path()))),
            "{login:?}"
        );
        assert_eq!(subscriptions(&registry), vec!["claude-sub"]);
    }

    async fn status_of(
        registry: &BackendRegistry,
        id: &str,
    ) -> ironwire_upstream::backend::BackendStatus {
        registry
            .get(&BackendId::from(id))
            .expect("the backend is registered whether or not a key was found")
            .status()
            .await
    }

    /// A credential must not reach a log through a host's own `{:?}` of the
    /// options it built, and neither must a variable name that carries one.
    #[test]
    fn options_do_not_render_a_credential_or_a_name_that_carries_one() {
        let options = EmbedOptions::default()
            .with_credentials(|_| Some(SecretString::from("sk-from-host".to_string())));

        let rendered = format!("{options:?}");
        assert!(!rendered.contains("sk-from-host"), "no credential");
        assert!(!rendered.contains("NEARAI"), "no variable name");
        assert!(rendered.contains("host-owned"), "presence is reportable");
        assert!(
            format!("{:?}", EmbedOptions::default()).contains("discovered"),
            "and so is its absence"
        );
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    #[tokio::test]
    async fn the_host_can_observe_an_ended_task_and_read_the_result_again() {
        let task = tokio::spawn(async { Err(ExitError::Server) });
        let mut proxy = EmbeddedProxy {
            port: 1,
            report: StartupReport::default(),
            shutdown: None,
            task: Some(task),
            exit: None,
        };
        assert_eq!(proxy.wait().await, Err(ExitError::Server));
        assert!(proxy.is_finished());
        assert_eq!(proxy.wait().await, Err(ExitError::Server));
    }

    #[tokio::test]
    async fn canceling_a_wait_keeps_shutdown_ownership() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async {
            let _ = rx.await;
            Ok(())
        });
        let mut proxy = EmbeddedProxy {
            port: 1,
            report: StartupReport::default(),
            shutdown: Some(tx),
            task: Some(task),
            exit: None,
        };
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), proxy.wait())
                .await
                .is_err()
        );
        assert!(!proxy.is_finished());
        proxy.request_shutdown();
        assert_eq!(proxy.wait().await, Ok(()));
    }

    #[tokio::test]
    async fn stopping_housekeeping_waits_until_its_captured_resources_drop() {
        struct Marker(std::sync::Arc<std::sync::atomic::AtomicBool>);
        impl Drop for Marker {
            fn drop(&mut self) {
                self.0.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let dropped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let marker = Marker(dropped.clone());
        let mut tasks = Tasks(vec![tokio::spawn(async move {
            let _marker = marker;
            std::future::pending::<()>().await;
        })]);
        tasks.stop().await;
        assert!(dropped.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn pointer_cleanup_leaves_another_owners_endpoint_alone() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("endpoint.json");
        let ours = ironwire_core::discovery::Endpoint::new(1, home.path().join("ours"));
        let theirs = ironwire_core::discovery::Endpoint::new(2, home.path().join("theirs"));
        let pointer = OwnedPointer {
            path: path.clone(),
            endpoint: ours,
        };
        ironwire_core::atomic::write(&path, &serde_json::to_string(&theirs).unwrap()).unwrap();
        drop(pointer);
        assert_eq!(
            ironwire_core::discovery::Endpoint::read_from(&path),
            Some(theirs)
        );
    }
}

#[cfg(test)]
mod update_check_tests {
    use super::*;
    use ironwire_creds::ConsentLedger;

    fn state_in(paths: &PathsConfig) -> AppState {
        AppState::new(
            BackendRegistry::new(),
            Config::default(),
            ConsentLedger::default(),
            "test-token".to_owned(),
        )
        .with_paths(paths.clone())
    }

    /// The switch a host cannot reach on disk, reached in code. `FromConfig`
    /// must stay transparent, or every existing caller changes behaviour.
    #[test]
    fn a_host_can_decline_checks_the_configuration_leaves_on() {
        assert!(EmbedOptions::default().checks_enabled(true));
        assert!(!EmbedOptions::default().checks_enabled(false));
        let declined = EmbedOptions::default().with_update_checks(UpdateChecks::Off);
        assert!(!declined.checks_enabled(true), "the host's choice applies");
        assert!(!declined.checks_enabled(false));
    }

    /// Declining is a decision about this process's own requests, not about
    /// who ships the binary, so it composes with either policy.
    #[test]
    fn declining_checks_is_independent_of_who_owns_the_binary() {
        let standalone = EmbedOptions::default()
            .with_update_policy(UpdatePolicy::Standalone)
            .with_update_checks(UpdateChecks::Off);
        assert_eq!(standalone.update_policy, UpdatePolicy::Standalone);
        assert!(!standalone.checks_enabled(true));
    }

    /// The point of the option: no catalog task exists to make the request.
    #[tokio::test]
    async fn a_declining_host_spawns_no_catalog_refresh() {
        let home = tempfile::tempdir().unwrap();
        let paths = PathsConfig::rooted_at(home.path().to_owned());
        let configured = true;

        let declined = EmbedOptions::default().with_update_checks(UpdateChecks::Off);
        assert!(
            catalog::spawn_refresh(
                state_in(&paths),
                &paths,
                declined.checks_enabled(configured)
            )
            .is_none()
        );

        let default = EmbedOptions::default();
        let task =
            catalog::spawn_refresh(state_in(&paths), &paths, default.checks_enabled(configured))
                .expect("a host that has not declined still refreshes");
        task.abort();
    }
}

/// The config switch that decides whether NEAR AI is told to refuse an aliased
/// model, end to end from a `BackendConfig` to the bytes on the wire.
///
/// `backend_from_config` is one line away from silently dropping
/// `refuse_model_aliases` on the floor -- the flag would still parse, the
/// backend would still build, every other test would still pass, and an
/// operator who asked for a fail-closed guarantee would quietly not have one.
/// Nothing else crosses that seam, so this does.
#[cfg(test)]
mod refuse_model_aliases_wiring {
    use super::tests::{entry, env_with};
    use super::*;
    use ironwire_upstream::backend::UpstreamRequest;

    /// A listener that records one request head and answers with an empty
    /// non-streaming body.
    async fn spawn() -> (String, Arc<std::sync::Mutex<Option<String>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        let seen = Arc::new(std::sync::Mutex::new(None));
        let sink = Arc::clone(&seen);
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            while let Ok(n) = socket.read(&mut chunk).await {
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            *sink.lock().expect("lock") = Some(String::from_utf8_lossy(&buf).to_string());
            let _ = socket
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 2\r\n\r\n{}")
                .await;
            let _ = socket.flush().await;
        });
        (format!("http://{addr}/v1"), seen)
    }

    async fn head_sent_with(refuse: bool) -> String {
        let (base, seen) = spawn().await;
        let mut declared = entry("nearai", "nearai");
        declared.base_url = Some(base);
        declared.refuse_model_aliases = refuse;
        let env = env_with(&[("NEARAI_API_KEY", "sk-near")]);
        let credentials = Credentials::discovered(&env);

        let backend = backend_from_config(&declared, 30, &credentials)
            .expect("the backend builds")
            .expect("a key was configured, so there is a backend");
        let _ = backend
            .send(UpstreamRequest {
                expected_account_scope: None,
                path: "/v1/chat/completions".to_string(),
                body: bytes::Bytes::from_static(b"{}"),
                headers: Vec::new(),
                stream: false,
            })
            .await;
        let head = seen.lock().expect("lock").clone().expect("upstream saw it");
        head.to_ascii_lowercase()
    }

    #[tokio::test]
    async fn the_flag_reaches_the_wire_when_an_operator_sets_it() {
        assert!(
            head_sent_with(true).await.contains("x-no-aliasing"),
            "refuse_model_aliases was configured and never left the config file"
        );
    }

    #[tokio::test]
    async fn nothing_is_sent_when_the_operator_does_not_ask() {
        assert!(
            !head_sent_with(false).await.contains("x-no-aliasing"),
            "a caller was fail-closed without asking to be"
        );
    }
}
