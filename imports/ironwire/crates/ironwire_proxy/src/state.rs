//! Daemon state: the backend registry, the routing policy, and consent.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures_util::{FutureExt, StreamExt, stream};

use ironwire_catalog::CatalogStore;
use ironwire_core::config::Config;
use ironwire_core::peek::{IdentityMarkers, RequestPeek};
use ironwire_core::policy::{Candidate, Policy};
use ironwire_core::protocol::BackendId;
use ironwire_creds::ConsentLedger;
use ironwire_ledger::Ledger;
use ironwire_update::UpdateStatus;
use ironwire_upstream::backend::{Backend, BackendStatus, UpstreamError};
use ironwire_upstream::breaker::BreakerBoard;
use ironwire_usage::UsageReport;

use crate::events::EventBus;
use crate::privacy::PrivacyFilter;
use crate::shutdown::Shutdown;

/// Take a lock, recovering from poisoning.
///
/// Every mutex in this module guards plain data. A thread that panicked while
/// holding one has not corrupted anything a reader cannot cope with, and
/// refusing to route because of it would turn one panic into an outage.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Whether consent lets `backend` be used.
///
/// A backend that reads a login is asked about that login, by the directory
/// it reads now: a yes recorded for `claude-sub` in `~/.claude` is not a yes to
/// the account `claude-sub` reads once `config_dir` moves it
/// (`docs/TRUST.md` §2). Everything else that needs consent keeps the plain
/// per-id answer.
#[must_use]
pub fn consents_to(consent: &ConsentLedger, backend: &dyn Backend) -> bool {
    if !backend.kind().requires_consent() {
        return true;
    }
    let id = backend.id().as_str();
    match backend.login() {
        Some(login) => consent.is_granted_for(id, login.consent_location()),
        None => consent.is_granted(id),
    }
}

/// A second backend was registered under an id that is already taken.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "two backends would be registered as `{0}`; every `[[backends]]` id in config.toml must \
     be unique, and must not reuse one IronWire discovers (claude-sub, anthropic-key, codex-sub, \
     openai-key, nearai) for anything else"
)]
pub struct DuplicateBackend(pub BackendId);

/// Maximum simultaneous credential checks, and separately network probes.
/// Shared by registry clones so concurrent requests cannot multiply the limit.
pub const BACKEND_CHECK_CONCURRENCY: usize = 8;

/// The set of backends this daemon can route to.
#[derive(Clone)]
pub struct BackendRegistry {
    backends: Vec<Arc<dyn Backend>>,
    positions: HashMap<BackendId, usize>,
    status_slots: Arc<tokio::sync::Semaphore>,
    probe_slots: Arc<tokio::sync::Semaphore>,
    /// The privacy config, for the trusted-backend constraint. `None` until
    /// state is built, which is only the case in tests that never route.
    ///
    /// Shared and swappable, because the mode is a routing constraint the user
    /// can now change while the daemon is running: under `full`, `trusts`
    /// decides which backends are eligible at all. A copy taken at startup
    /// would keep routing to a backend the user has since untrusted.
    privacy_policy: Arc<Mutex<Option<Arc<ironwire_core::config::PrivacyConfig>>>>,
}

impl Default for BackendRegistry {
    fn default() -> Self {
        Self {
            backends: Vec::new(),
            positions: HashMap::new(),
            status_slots: Arc::new(tokio::sync::Semaphore::new(BACKEND_CHECK_CONCURRENCY)),
            probe_slots: Arc::new(tokio::sync::Semaphore::new(BACKEND_CHECK_CONCURRENCY)),
            privacy_policy: Arc::new(Mutex::new(None)),
        }
    }
}

impl BackendRegistry {
    /// Empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a backend. Order is the tie-break for equally-preferred candidates.
    ///
    /// # Panics
    ///
    /// When a backend with the same id is already registered. That is a bug
    /// in whoever built the registry, never a state to route around — see
    /// [`Self::try_push`], which is what startup uses.
    pub fn push(&mut self, backend: Arc<dyn Backend>) {
        if let Err(duplicate) = self.try_push(backend) {
            panic!("{duplicate}");
        }
    }

    /// Add a backend, refusing one whose id is already taken.
    ///
    /// Consent, quota, the circuit breaker, pins and ledger rows are all keyed
    /// by id, and every lookup takes the first match. A second backend under
    /// one id would therefore be unreachable and would share the first one's
    /// consent and quota — the way a second subscription entry used to become
    /// a silent copy of the default login.
    ///
    /// # Errors
    ///
    /// [`DuplicateBackend`], naming the id, when one is already registered.
    pub fn try_push(&mut self, backend: Arc<dyn Backend>) -> Result<(), DuplicateBackend> {
        if self.get(backend.id()).is_some() {
            return Err(DuplicateBackend(backend.id().clone()));
        }
        self.positions
            .insert(backend.id().clone(), self.backends.len());
        self.backends.push(backend);
        Ok(())
    }

    /// Install the privacy config, so `candidates` can mark what may be routed
    /// to under `privacy.mode = "full"`.
    ///
    /// Takes `&self`: this is called again whenever the mode changes, from a
    /// control-API handler that only has a shared reference to the state.
    pub fn set_privacy(&self, privacy: Arc<ironwire_core::config::PrivacyConfig>) {
        *lock(&self.privacy_policy) = Some(privacy);
    }

    /// The privacy config currently in force, if one has been installed.
    fn privacy_policy(&self) -> Option<Arc<ironwire_core::config::PrivacyConfig>> {
        lock(&self.privacy_policy).clone()
    }

    /// Trusted ids named in config that no registered backend answers to.
    #[must_use]
    pub fn missing_trusted(&self) -> Vec<String> {
        self.privacy_policy()
            .filter(|privacy| privacy.mode() == ironwire_core::config::PrivacyMode::Full)
            .map(|privacy| {
                privacy
                    .trusted_backends
                    .iter()
                    .filter(|id| !self.backends.iter().any(|b| b.id().as_str() == id.as_str()))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Look up by id.
    #[must_use]
    pub fn get(&self, id: &BackendId) -> Option<&Arc<dyn Backend>> {
        self.positions
            .get(id)
            .map(|&position| &self.backends[position])
    }

    /// All backends.
    #[must_use]
    pub fn all(&self) -> &[Arc<dyn Backend>] {
        &self.backends
    }

    /// [`consents_to`] for the backend registered as `id`; false for an id
    /// nothing is registered under.
    #[must_use]
    pub fn consented(&self, id: &BackendId, consent: &ConsentLedger) -> bool {
        self.get(id)
            .is_some_and(|backend| consents_to(consent, backend.as_ref()))
    }

    /// Whether anything is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.backends.is_empty()
    }

    /// Snapshot every backend's status, including a fresh credential check.
    pub async fn statuses(&self) -> Vec<BackendStatus> {
        let checks: Vec<_> = self
            .backends
            .iter()
            .cloned()
            .map(|backend| self.checked_status(backend))
            .collect();
        stream::iter(checks)
            .buffered(BACKEND_CHECK_CONCURRENCY)
            .collect()
            .await
    }

    /// Credentials for subscriptions another client cannot use are not read on
    /// its request. Control status still observes every registered backend.
    pub async fn routing_statuses(
        &self,
        peek: &RequestPeek,
        cross_account_fallback: bool,
    ) -> Vec<BackendStatus> {
        let checks: Vec<_> = self
            .backends
            .iter()
            .filter(|backend| {
                let Some(identity) = backend.required_client_identity() else {
                    return true;
                };
                if peek.client_identity != Some(identity) {
                    return false;
                }
                cross_account_fallback
                    || match &peek.client_login {
                        Some(id) => backend.id() == id,
                        None => backend
                            .login()
                            .is_some_and(|login| login.default && login.location.is_none()),
                    }
            })
            .cloned()
            .map(|backend| self.checked_status(backend))
            .collect();
        stream::iter(checks)
            .buffered(BACKEND_CHECK_CONCURRENCY)
            .collect()
            .await
    }

    async fn checked_status(&self, backend: Arc<dyn Backend>) -> BackendStatus {
        // Backends offload their own blocking I/O. Nesting this whole async
        // operation in spawn_blocking deadlocks hosts with one blocking worker:
        // that worker waits for the credential read queued behind itself.
        let _permit = Arc::clone(&self.status_slots)
            .acquire_owned()
            .await
            .expect("status semaphore is never closed");
        match std::panic::AssertUnwindSafe(backend.status())
            .catch_unwind()
            .await
        {
            Ok(status) => status,
            Err(_) => BackendStatus {
                id: backend.id().clone(),
                name: backend.name().to_string(),
                kind: backend.kind(),
                authenticated: false,
                account_scope: None,
                detail: Some("credential check failed; run `ironwire doctor`".to_string()),
                quota: backend.quota(),
                models: backend.models(),
            },
        }
    }

    /// Probe a backend without blocking request workers on credential discovery.
    /// Probe capacity is separate so a slow provider cannot delay status reads.
    pub async fn probe(&self, backend: Arc<dyn Backend>) -> Result<(), UpstreamError> {
        let _permit = Arc::clone(&self.probe_slots)
            .acquire_owned()
            .await
            .expect("probe semaphore is never closed");
        let id = backend.id().clone();
        std::panic::AssertUnwindSafe(backend.probe())
            .catch_unwind()
            .await
            .unwrap_or_else(|_| {
                Err(UpstreamError::Transport {
                    backend: id,
                    detail: "backend probe failed; run `ironwire doctor`".to_string(),
                })
            })
    }

    /// Build the candidate list the router sees.
    ///
    /// `authenticated` comes from the freshly-collected statuses rather than a
    /// cached flag: a credential that expired ten seconds ago should not be
    /// routed to just because it worked at boot.
    #[must_use]
    pub fn candidates(
        &self,
        statuses: &[BackendStatus],
        consent: &ConsentLedger,
    ) -> Vec<Candidate> {
        self.candidates_capped(statuses, consent, &mut |_, quota| quota)
    }

    /// [`Self::candidates`], with a hook that can replace a backend's quota —
    /// which is how a spend cap makes one unavailable without a second
    /// exclusion mechanism running alongside `Headroom`.
    #[must_use]
    pub fn candidates_capped(
        &self,
        statuses: &[BackendStatus],
        consent: &ConsentLedger,
        cap: &mut dyn FnMut(
            &Arc<dyn Backend>,
            ironwire_core::quota::QuotaSnapshot,
        ) -> ironwire_core::quota::QuotaSnapshot,
    ) -> Vec<Candidate> {
        // Read once, so every candidate in one routing decision is judged
        // against the same policy — a mode changed mid-scan would otherwise
        // produce a candidate set that never existed.
        let privacy_policy = self.privacy_policy();
        let statuses: HashMap<_, _> = statuses.iter().map(|status| (&status.id, status)).collect();
        self.backends
            .iter()
            .map(|backend| {
                let status = statuses.get(backend.id()).copied();
                Candidate {
                    id: backend.id().clone(),
                    kind: backend.kind(),
                    caps: backend.capabilities().clone(),
                    quota: cap(backend, backend.quota()),
                    healthy: status.is_some_and(|s| s.authenticated),
                    account_scope: status.and_then(|s| s.account_scope.clone()),
                    consented: consents_to(consent, backend.as_ref()),
                    // Beside `consented` because it is the same shape of fact:
                    // a user instruction, read from config, that the router
                    // must obey without knowing where it came from.
                    trusted: privacy_policy
                        .as_ref()
                        .is_none_or(|privacy| privacy.trusts(backend.id().as_str())),
                    required_client_identity: backend.required_client_identity(),
                    // Only the default login where the client keeps it
                    // unprompted. A relocated `claude-sub` reads another
                    // directory's token, while the client still using the
                    // default directory goes on calling the plain façade —
                    // possibly logged in as someone else. The relocated login
                    // is reached through its bound URL instead.
                    default_login: backend
                        .login()
                        .is_some_and(|login| login.default && login.location.is_none()),
                    models: backend.models(),
                    catalogue_from_provider: backend.catalogue_from_provider(),
                }
            })
            .collect()
    }
}

/// Shared daemon state.
#[derive(Clone)]
pub struct AppState {
    /// Registered backends.
    pub backends: BackendRegistry,
    /// Routing policy, including per-conversation affinity.
    pub policy: Arc<Mutex<Policy>>,
    /// Explicit per-session challenges, bounded and memory-only; never status or logs.
    pub admission_bindings: Arc<Mutex<ironwire_core::admission::AdmissionBindings>>,
    /// Recorded subscription consents (`docs/TRUST.md` §2).
    pub consent: Arc<Mutex<ConsentLedger>>,
    /// Loaded configuration.
    pub config: Arc<Config>,
    /// Control-API bearer token. The control plane exposes the ledger and can
    /// change routing, so loopback alone is not enough on a shared machine.
    pub control_token: Arc<String>,
    /// Local trace ledger. `None` when `capture.enabled = false`, or when the
    /// ledger could not be opened — a ledger problem must never stop the proxy
    /// from doing its actual job.
    pub ledger: Option<Ledger>,
    /// Where verbatim request and response bodies are written. `None` unless
    /// `capture.bodies = true` -- bodies are the user's source code
    /// (`docs/TRUST.md` §4).
    pub bodies: Option<Arc<ironwire_ledger::bodies::BodyStore>>,
    /// Independently opted-in detailed evidence with durable leases.
    pub token_spool: Option<Arc<ironwire_ledger::token_spool::TokenSpool>>,
    /// Provider values refreshed through the signed catalog channel
    /// (`docs/UPDATES.md`).
    ///
    /// Behind a lock because a running daemon can install a newer document —
    /// which is the whole point of the channel: one that needed a restart to
    /// take effect would have the same latency as a release. The lock is held
    /// only long enough to clone an `Arc`, so the read path is a pointer copy.
    catalog: Arc<Mutex<Arc<CatalogStore>>>,
    /// What the last update check concluded. Notify-only — IronWire never
    /// applies an update itself.
    pub update: Arc<Mutex<UpdateStatus>>,
    /// Per-backend circuit state, so a failure is remembered past the end of
    /// the request that hit it (`ironwire_upstream::breaker`).
    pub breakers: Arc<BreakerBoard>,
    /// Route and health events, for `ironwire watch` and the menu bar app.
    /// Lossy and non-blocking by construction (`crate::events`).
    pub events: EventBus,
    /// Announced when the daemon starts stopping, so a handler holding a
    /// response open forever can end it (`crate::shutdown`). The event stream
    /// is the only one that needs it, and without it graceful shutdown waits
    /// for a connection that by design never closes.
    pub shutdown: Shutdown,
    /// The optional privacy filter. `None` unless the user turned it on and
    /// configured something for it to match (`docs/PRIVACY.md`).
    ///
    /// Swappable, because the mode is a setting the user can change without
    /// restarting. Read through [`AppState::privacy`], which hands back an
    /// `Arc` so an in-flight request keeps the filter it started with rather
    /// than changing behaviour halfway down a response stream.
    privacy: Arc<Mutex<Option<Arc<PrivacyFilter>>>>,
    /// The privacy settings the filter above was built from.
    privacy_config: Arc<Mutex<Arc<ironwire_core::config::PrivacyConfig>>>,
    /// Where this daemon's files live, when it was started from a real home.
    ///
    /// `None` in tests, which never persist anything. A settings change that
    /// cannot be written down is refused rather than applied silently, so this
    /// being absent is a reason to say no — see [`AppState::set_consent`].
    paths: Option<Arc<ironwire_core::config::PathsConfig>>,
    /// Port actually bound. Distinct from `config.server.port`, which is only
    /// a request: a `--port` override or a config reload would otherwise make
    /// `status` report a number nothing is listening on.
    pub port: u16,
    /// The most recent route this daemon took.
    ///
    /// Kept here rather than read back from the ledger because the ledger is
    /// optional and this is not: a status line has to be able to say where the
    /// last turn went on a machine where trace capture is off.
    last_route: Arc<Mutex<Option<LastRoute>>>,
    /// Most recent usage report, and when it was built.
    ///
    /// `ironwire statusline` calls `/status` on *every render of somebody's
    /// editor*, and building this reads every ledger row in the history window
    /// — eight days, thousands of rows on a working machine. Recomputing that
    /// per keystroke would put a SQLite scan on an interactive path for a
    /// figure that cannot meaningfully change between two renders.
    usage: Arc<Mutex<Option<CachedUsage>>>,
    /// What has been spent today against any configured cap
    /// (`crate::spend`). Shared, because it is written from the response path
    /// and read from the routing path.
    pub spend: Arc<Mutex<crate::spend::SpendTracker>>,
}

/// A usage report, when it was built, and the ledger write token it was built
/// against.
type CachedUsage = (chrono::DateTime<chrono::Utc>, u64, UsageReport);

/// How long a usage report may be reused *while the ledger has not moved*.
///
/// Only time drifts under this — "closes in 4h" ages by up to ten seconds on
/// an idle machine. New traffic does not wait for it: the write token is what
/// makes a report stale, and it moves the instant an exchange is recorded.
const USAGE_MAX_AGE: chrono::Duration = chrono::Duration::seconds(10);

/// The most recent routing decision, for anything that needs to display it.
#[derive(Debug, Clone)]
pub struct LastRoute {
    /// Backend chosen.
    pub backend: String,
    /// Model sent upstream, when policy named one. `None` means the client's
    /// own choice was forwarded untouched.
    pub model: Option<String>,
    /// Backend this conversation was on before, when this was a change.
    pub from: Option<String>,
    /// How far down the ladder it sits.
    pub rung: ironwire_core::policy::Rung,
    /// When.
    pub at: chrono::DateTime<chrono::Utc>,
}

impl AppState {
    /// Build state around a registry.
    #[must_use]
    pub fn new(
        backends: BackendRegistry,
        config: Config,
        consent: ConsentLedger,
        control_token: String,
    ) -> Self {
        // The registry has to know the privacy policy before it can say which
        // backends may be routed to, and this is the one place both are in
        // scope.
        backends.set_privacy(Arc::new(config.privacy.clone()));
        let mut policy = Policy::new();
        policy.set_cross_account_fallback(config.subscriptions.cross_account_fallback);
        Self {
            backends,
            policy: Arc::new(Mutex::new(policy)),
            admission_bindings: Arc::new(Mutex::new(
                ironwire_core::admission::AdmissionBindings::default(),
            )),
            consent: Arc::new(Mutex::new(consent)),
            ledger: None,
            bodies: None,
            token_spool: None,
            catalog: Arc::new(Mutex::new(Arc::new(CatalogStore::new(
                ironwire_catalog::CATALOG_PUBLIC_KEY,
            )))),
            update: Arc::new(Mutex::new(UpdateStatus::Unknown)),
            breakers: Arc::new(BreakerBoard::default()),
            events: EventBus::new(),
            shutdown: Shutdown::new(),
            privacy: Arc::new(Mutex::new(
                PrivacyFilter::from_config(&config.privacy).map(Arc::new),
            )),
            privacy_config: Arc::new(Mutex::new(Arc::new(config.privacy.clone()))),
            paths: None,
            port: config.server.port,
            config: Arc::new(config),
            control_token: Arc::new(control_token),
            last_route: Arc::new(Mutex::new(None)),
            spend: Arc::new(Mutex::new(crate::spend::SpendTracker::default())),
            usage: Arc::new(Mutex::new(None)),
        }
    }

    /// The privacy filter currently in force.
    ///
    /// Handed out as an `Arc` rather than borrowed, so a request that started
    /// under one mode finishes under it. A filter swapped out from under an
    /// in-flight exchange would substitute in the request and then fail to
    /// reverse the substitution in the response — the one failure mode that
    /// turns the filter into corruption rather than protection.
    #[must_use]
    pub fn privacy(&self) -> Option<Arc<PrivacyFilter>> {
        lock(&self.privacy).clone()
    }

    /// Whether a client whose own subscription is spent may be served by
    /// another of the user's subscriptions to the same product.
    ///
    /// Read from the policy rather than from `config`, because the policy is
    /// what routing obeys: anything that lists what a client could be served
    /// by has to agree with where its requests will actually go.
    #[must_use]
    pub fn cross_account_fallback(&self) -> bool {
        lock(&self.policy).cross_account_fallback()
    }

    /// The privacy settings currently in force.
    #[must_use]
    pub fn privacy_config(&self) -> Arc<ironwire_core::config::PrivacyConfig> {
        Arc::clone(&lock(&self.privacy_config))
    }

    /// Change the privacy mode, everywhere it is read from.
    ///
    /// Two things have to move, and they cannot move atomically: the filter,
    /// which decides what gets substituted, and the registry's copy, which
    /// decides which backends are eligible at all under `full`. A request
    /// routed between the two sees one of them updated and not the other.
    ///
    /// The constraint goes first, because the combination worth avoiding is
    /// *un*-substituted content reaching a backend the user has not named. That
    /// is the state a raise to `full` starts from, so narrowing eligibility
    /// before strengthening the filter shortens it rather than extending it.
    /// The reverse order would leave the widest window in exactly the direction
    /// that matters. (The mirror-image window — content substituted for `full`
    /// reaching an untrusted backend — is strictly safer than what preceded it,
    /// so it costs nothing.)
    ///
    /// Persistence is the caller's job: this is the running daemon's state, and
    /// a change that could not be written to `config.toml` is still a change
    /// the user asked for and can see.
    pub fn set_privacy_mode(&self, mode: ironwire_core::config::PrivacyMode) {
        let mut updated = (*self.privacy_config()).clone();
        updated.mode = Some(mode);
        let updated = Arc::new(updated);

        self.backends.set_privacy(Arc::clone(&updated));
        *lock(&self.privacy) = PrivacyFilter::from_config(&updated).map(Arc::new);
        *lock(&self.privacy_config) = updated;
    }

    /// A usage report, reusing the last one only if nothing has changed.
    ///
    /// `writes` is the ledger's change token: a report is reused only when the
    /// ledger has not moved since it was built *and* it is younger than
    /// [`USAGE_MAX_AGE`]. Both halves are load-bearing. Without the token, a
    /// request arriving a millisecond after a report was built stays invisible
    /// until the timer expires — which is exactly how a fast end-to-end run
    /// sees a status screen with no session on it at all.
    ///
    /// Deliberately not a background task: on a machine where nobody runs
    /// `status`, this costs nothing.
    pub fn usage_report(
        &self,
        now: chrono::DateTime<chrono::Utc>,
        writes: u64,
        build: impl FnOnce() -> UsageReport,
    ) -> UsageReport {
        let mut slot = match self.usage.lock() {
            Ok(slot) => slot,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some((built_at, built_writes, report)) = slot.as_ref()
            && *built_writes == writes
            && now >= *built_at
            && now - *built_at < USAGE_MAX_AGE
        {
            return report.clone();
        }
        let report = build();
        *slot = Some((now, writes, report.clone()));
        report
    }

    /// Remember where the last turn went.
    pub fn set_last_route(&self, route: LastRoute) {
        match self.last_route.lock() {
            Ok(mut slot) => *slot = Some(route),
            Err(poisoned) => *poisoned.into_inner() = Some(route),
        }
    }

    /// Where the last turn went, if there has been one.
    #[must_use]
    pub fn last_route(&self) -> Option<LastRoute> {
        match self.last_route.lock() {
            Ok(slot) => slot.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Install a catalog store loaded at startup.
    #[must_use]
    pub fn with_catalog(self, catalog: CatalogStore) -> Self {
        self.set_catalog(Arc::new(catalog));
        self
    }

    /// The catalog in force. A pointer copy, not a deep clone.
    #[must_use]
    pub fn catalog(&self) -> Arc<CatalogStore> {
        match self.catalog.lock() {
            Ok(guard) => Arc::clone(&guard),
            Err(poisoned) => Arc::clone(&poisoned.into_inner()),
        }
    }

    /// Install a newer catalog document, for the background refresh.
    pub fn set_catalog(&self, catalog: Arc<CatalogStore>) {
        match self.catalog.lock() {
            Ok(mut guard) => *guard = catalog,
            Err(poisoned) => *poisoned.into_inner() = catalog,
        }
    }

    /// Record what the last update check concluded.
    pub fn set_update_status(&self, status: UpdateStatus) {
        match self.update.lock() {
            Ok(mut slot) => *slot = status,
            Err(poisoned) => *poisoned.into_inner() = status,
        }
    }

    /// The last update check's conclusion.
    #[must_use]
    pub fn update_status(&self) -> UpdateStatus {
        match self.update.lock() {
            Ok(slot) => slot.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Identity markers currently in force.
    #[must_use]
    pub fn identity_markers(&self) -> IdentityMarkers {
        let store = self.catalog();
        let catalog = store.current();
        IdentityMarkers {
            claude_code_system_prefix: catalog.client_identity.claude_code_system_prefix.clone(),
            claude_code_user_agent_prefix: catalog
                .client_identity
                .claude_code_user_agent_prefix
                .clone(),
            codex_instructions_marker: catalog.client_identity.codex_instructions_marker.clone(),
            codex_originator_prefix: catalog.client_identity.codex_originator_prefix.clone(),
            compaction_markers: catalog.client_identity.compaction_markers.clone(),
        }
    }

    /// Attach a store for verbatim bodies.
    #[must_use]
    pub fn with_bodies(mut self, bodies: Option<Arc<ironwire_ledger::bodies::BodyStore>>) -> Self {
        self.bodies = bodies;
        self
    }

    /// Attach the independently bounded detailed-evidence store.
    #[must_use]
    pub fn with_token_spool(
        mut self,
        spool: Option<Arc<ironwire_ledger::token_spool::TokenSpool>>,
    ) -> Self {
        self.token_spool = spool;
        self
    }

    /// Attach a trace ledger.
    #[must_use]
    pub fn with_ledger(mut self, ledger: Option<Ledger>) -> Self {
        self.ledger = ledger;
        self
    }

    /// Record the port actually bound.
    #[must_use]
    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Record where this daemon's files live, so settings changes can be saved.
    #[must_use]
    pub fn with_paths(mut self, paths: ironwire_core::config::PathsConfig) -> Self {
        self.paths = Some(Arc::new(paths));
        self
    }

    /// `config.toml`, when there is one to write.
    #[must_use]
    pub fn config_path(&self) -> Option<std::path::PathBuf> {
        self.paths.as_ref().map(|paths| paths.config_file())
    }

    /// Record or withdraw consent for a backend, and write it down.
    ///
    /// In that order, and both or neither. The router reads the in-memory
    /// ledger on every decision, so a grant takes effect on the next request —
    /// but a grant that only lived in memory would vanish at the next restart,
    /// and `docs/TRUST.md` §2 is that consent is *recorded*. So a failed write
    /// rolls the in-memory change back and reports the failure: consent we
    /// could not record must never be treated as granted.
    ///
    /// A grant is recorded for the login the backend reads now, which is the
    /// one its prompt named ([`consents_to`]).
    ///
    /// # Errors
    ///
    /// A message naming what could not be written.
    pub fn set_consent(&self, backend_id: &str, granted: bool) -> Result<(), String> {
        let Some(paths) = self.paths.as_ref() else {
            return Err("this daemon was not started from a home directory".to_string());
        };
        let path = paths.consent_file();
        let login = self
            .backends
            .get(&BackendId::from(backend_id))
            .and_then(|backend| backend.login());

        let previous = self.consent_snapshot();
        {
            let mut ledger = lock(&self.consent);
            if granted {
                ledger.grant_for(
                    backend_id,
                    login.as_ref().and_then(|login| login.consent_location()),
                    chrono::Utc::now(),
                );
            } else {
                ledger.revoke(backend_id);
            }
        }

        let updated = self.consent_snapshot();
        if let Err(error) = updated.save(&path) {
            *lock(&self.consent) = previous;
            return Err(format!("writing {}: {error}", path.display()));
        }
        Ok(())
    }

    /// Read the consent ledger.
    #[must_use]
    pub fn consent_snapshot(&self) -> ConsentLedger {
        match self.consent.lock() {
            Ok(c) => c.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ironwire_core::protocol::BackendKind;
    use ironwire_core::quota::QuotaSnapshot;
    use ironwire_upstream::anthropic::{AnthropicBackend, anthropic_models};

    fn registry() -> BackendRegistry {
        let mut registry = BackendRegistry::new();
        registry.push(Arc::new(
            AnthropicBackend::subscription(None, 60).expect("client builds"),
        ));
        registry
    }

    fn status(id: &str, authenticated: bool) -> BackendStatus {
        BackendStatus {
            id: BackendId::from(id),
            name: id.to_string(),
            kind: BackendKind::Subscription,
            authenticated,
            account_scope: None,
            detail: None,
            quota: QuotaSnapshot::default(),
            models: anthropic_models(),
        }
    }

    #[test]
    fn a_subscription_without_consent_is_not_a_usable_candidate() {
        let registry = registry();
        let statuses = vec![status("claude-sub", true)];
        let candidates = registry.candidates(&statuses, &ConsentLedger::default());
        assert_eq!(candidates.len(), 1);
        assert!(!candidates[0].consented);
    }

    #[test]
    fn granting_consent_makes_the_candidate_usable() {
        let registry = registry();
        let mut consent = ConsentLedger::default();
        consent.grant("claude-sub", Utc::now());
        let statuses = vec![status("claude-sub", true)];
        let candidates = registry.candidates(&statuses, &consent);
        assert!(candidates[0].consented);
    }

    #[test]
    fn an_unauthenticated_backend_is_marked_unhealthy() {
        let registry = registry();
        let statuses = vec![status("claude-sub", false)];
        let candidates = registry.candidates(&statuses, &ConsentLedger::default());
        assert!(!candidates[0].healthy);
    }

    #[test]
    fn lookup_by_id_finds_registered_backends() {
        let registry = registry();
        assert!(registry.get(&BackendId::from("claude-sub")).is_some());
        assert!(registry.get(&BackendId::from("nope")).is_none());
        assert!(!registry.is_empty());
    }

    /// A Claude subscription backend for the login in `dir` (`None` for the
    /// default), never reading this machine's Keychain.
    fn claude_login(id: &str, dir: Option<&str>) -> Arc<dyn Backend> {
        let login = ironwire_creds::ClaudeLogin::with_home(
            dir.map(ToString::to_string),
            Some(std::path::PathBuf::from("/home/alice")),
        )
        .without_keychain();
        Arc::new(
            AnthropicBackend::subscription_for(
                BackendId::from(id),
                id.to_string(),
                login,
                None,
                60,
            )
            .expect("client builds"),
        )
    }

    /// Every lookup takes the first match, so a second backend under one id
    /// would be unreachable and would share the first one's consent and quota.
    #[test]
    fn a_second_backend_under_a_taken_id_is_refused_by_name() {
        let mut registry = BackendRegistry::new();
        registry
            .try_push(claude_login(
                "claude-work",
                Some("/home/alice/.claude-work"),
            ))
            .expect("the first is registered");
        let refused = registry
            .try_push(claude_login(
                "claude-work",
                Some("/home/alice/.claude-other"),
            ))
            .expect_err("the id is taken");
        assert_eq!(refused.0.as_str(), "claude-work");
        assert!(refused.to_string().contains("`claude-work`"), "{refused}");
        assert_eq!(registry.all().len(), 1, "the first one is left alone");
    }

    #[test]
    #[should_panic(expected = "claude-sub")]
    fn pushing_a_duplicate_is_a_bug_not_a_shadow() {
        let mut registry = registry();
        registry.push(claude_login("claude-sub", None));
    }

    /// The plain façade belongs to the client that keeps its login where the
    /// client puts it unprompted, and so to the default login in that place.
    /// Relocated, the default reads another directory's token, which the
    /// client still on the default directory may not be logged in as; it is
    /// reached through its bound URL like any additional login. A metered key
    /// is not a login at all.
    #[test]
    fn the_plain_facade_belongs_only_to_a_default_login_left_in_its_place() {
        let candidates_with = |default_dir: Option<&str>| {
            let mut registry = BackendRegistry::new();
            registry.push(claude_login("claude-sub", default_dir));
            registry.push(claude_login(
                "claude-work",
                Some("/home/alice/.claude-work"),
            ));
            registry.push(Arc::new(
                AnthropicBackend::api_key(secrecy::SecretString::from("sk-ant-test"), None, 60)
                    .expect("client builds"),
            ));
            registry.candidates(&[], &ConsentLedger::default())
        };
        let default_login = |candidates: &[Candidate], id: &str| {
            candidates
                .iter()
                .find(|c| c.id.as_str() == id)
                .expect("registered")
                .default_login
        };

        let in_place = candidates_with(None);
        assert!(default_login(&in_place, "claude-sub"), "the default");
        assert!(
            !default_login(&in_place, "claude-work"),
            "an additional login"
        );
        assert!(!default_login(&in_place, "anthropic-key"), "not a login");

        let moved = candidates_with(Some("/home/alice/.claude-moved"));
        assert!(
            !default_login(&moved, "claude-sub"),
            "a relocated default is not the plain façade's"
        );
        assert!(!default_login(&moved, "claude-work"));
    }

    /// Consent answers the login it was given for. The id stays when
    /// `config_dir` moves a login, the account does not, so a yes recorded for
    /// the old place is no yes at all.
    #[test]
    fn consent_given_for_another_directory_does_not_make_a_login_usable() {
        let mut registry = BackendRegistry::new();
        registry.push(claude_login(
            "claude-sub",
            Some("/home/alice/.claude-moved"),
        ));
        registry.push(claude_login(
            "claude-work",
            Some("/home/alice/.claude-work"),
        ));
        let consented = |consent: &ConsentLedger| -> Vec<(String, bool)> {
            registry
                .candidates(&[], consent)
                .into_iter()
                .map(|c| (c.id.to_string(), c.consented))
                .collect()
        };

        // A file written before the move: it answered for `~/.claude`.
        let mut before = ConsentLedger::default();
        before.grant("claude-sub", Utc::now());
        before.grant_for("claude-work", Some("/home/alice/.claude-old"), Utc::now());
        assert_eq!(
            consented(&before),
            vec![
                ("claude-sub".to_string(), false),
                ("claude-work".to_string(), false)
            ]
        );
        assert!(!registry.consented(&BackendId::from("claude-sub"), &before));

        let mut now = ConsentLedger::default();
        now.grant_for("claude-sub", Some("/home/alice/.claude-moved"), Utc::now());
        now.grant_for("claude-work", Some("/home/alice/.claude-work"), Utc::now());
        assert_eq!(
            consented(&now),
            vec![
                ("claude-sub".to_string(), true),
                ("claude-work".to_string(), true)
            ]
        );
        assert!(registry.consented(&BackendId::from("claude-work"), &now));
        assert!(
            !registry.consented(&BackendId::from("claude-play"), &now),
            "nothing is registered there"
        );
    }

    #[test]
    fn cross_account_fallback_follows_the_configuration_into_routing() {
        let off = AppState::new(
            BackendRegistry::new(),
            Config::default(),
            ConsentLedger::default(),
            "t".to_string(),
        );
        assert!(!off.cross_account_fallback(), "off unless the user says so");

        let mut config = Config::default();
        config.subscriptions.cross_account_fallback = true;
        let on = AppState::new(
            BackendRegistry::new(),
            config,
            ConsentLedger::default(),
            "t".to_string(),
        );
        assert!(on.cross_account_fallback());
        assert!(
            on.policy.lock().expect("policy").cross_account_fallback(),
            "the policy is what routing obeys, so that is where it has to land"
        );
    }
}

#[cfg(test)]
mod catalog_tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use ironwire_core::config::Config;
    use ironwire_creds::ConsentLedger;

    fn state() -> AppState {
        AppState::new(
            BackendRegistry::new(),
            Config::default(),
            ConsentLedger::default(),
            "t".to_string(),
        )
    }

    #[test]
    fn a_fresh_daemon_runs_on_the_compiled_in_values() {
        let state = state();
        assert_eq!(
            state.catalog().serial(),
            0,
            "0 means built-ins, not a document"
        );
        assert_eq!(
            state.identity_markers().claude_code_system_prefix,
            ironwire_core::peek::CLAUDE_CODE_SYSTEM_PREFIX
        );
    }

    #[test]
    fn a_newer_signed_document_takes_effect_without_a_restart() {
        // The whole point of the channel. One that needed a restart would have
        // the same latency as a release, which is the problem it solves.
        //
        // Signed properly rather than installed through a back door: an
        // unverified-install path would defeat the channel entirely
        // (`docs/TRUST.md` I2), so this test uses its own keypair and goes
        // through `apply`, exercising the real path.
        let signing = SigningKey::from_bytes(&[11u8; 32]);
        let verifying = signing.verifying_key().to_bytes();

        let document = serde_json::json!({
            "schema_version": 1,
            "serial": 42,
            "issued_at": "2026-08-08T00:00:00Z",
            "client_identity": {
                "claude_code_system_prefix": "You are Something Else",
                "codex_instructions_marker": "Codex",
            },
        })
        .to_string();
        let signed = ironwire_catalog::SignedCatalog {
            signature: hex::encode(signing.sign(document.as_bytes()).to_bytes()),
            document,
        };

        let mut store = CatalogStore::new(verifying);
        store
            .apply(&signed)
            .expect("a correctly signed document applies");

        let state = state();
        assert_eq!(state.catalog().serial(), 0);
        state.set_catalog(Arc::new(store));

        assert_eq!(state.catalog().serial(), 42);
        assert_eq!(
            state.identity_markers().claude_code_system_prefix,
            "You are Something Else"
        );
    }

    #[test]
    fn a_document_signed_by_the_wrong_key_never_reaches_the_daemon() {
        let attacker = SigningKey::from_bytes(&[99u8; 32]);
        let ours = SigningKey::from_bytes(&[11u8; 32])
            .verifying_key()
            .to_bytes();

        let document = serde_json::json!({
            "schema_version": 1,
            "serial": 99,
            "issued_at": "2026-08-08T00:00:00Z",
            "client_identity": {"claude_code_system_prefix": "You are Evil"},
        })
        .to_string();
        let signed = ironwire_catalog::SignedCatalog {
            signature: hex::encode(attacker.sign(document.as_bytes()).to_bytes()),
            document,
        };

        let mut store = CatalogStore::new(ours);
        assert!(
            store.apply(&signed).is_err(),
            "a document signed by the wrong key was accepted"
        );
        assert_eq!(store.serial(), 0, "the built-ins must stay in force");
    }

    #[test]
    fn reading_the_catalog_is_a_pointer_copy_not_a_deep_clone() {
        // It happens on every request; a deep clone of the model catalogue per
        // request would be a real cost for no reason.
        let state = state();
        let a = state.catalog();
        let b = state.catalog();
        assert!(Arc::ptr_eq(&a, &b));
    }
}

#[cfg(test)]
mod usage_cache_tests {
    use super::*;
    use ironwire_core::config::Config;
    use ironwire_creds::ConsentLedger;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn state() -> AppState {
        AppState::new(
            BackendRegistry::new(),
            Config::default(),
            ConsentLedger::default(),
            "t".to_string(),
        )
    }

    fn report(completed: usize) -> UsageReport {
        UsageReport {
            completed_sessions: completed,
            ..UsageReport::default()
        }
    }

    #[test]
    fn a_status_line_rendering_repeatedly_scans_the_ledger_once() {
        // `ironwire statusline` calls `/status` on every render of somebody's
        // editor. Without this, each one is an eight-day SQLite scan on an
        // interactive path.
        let state = state();
        let builds = AtomicUsize::new(0);
        let now = chrono::Utc::now();
        for _ in 0..20 {
            state.usage_report(now, 7, || {
                builds.fetch_add(1, Ordering::Relaxed);
                report(1)
            });
        }
        assert_eq!(builds.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_single_new_exchange_invalidates_it_immediately() {
        // The failure this exists to stop: an end-to-end run that asks for
        // status, sends traffic and asks again inside one second, and is shown
        // the empty report from before the traffic.
        let state = state();
        let now = chrono::Utc::now();
        assert_eq!(
            state.usage_report(now, 0, || report(0)).completed_sessions,
            0
        );
        assert_eq!(
            state.usage_report(now, 1, || report(1)).completed_sessions,
            1
        );
    }

    #[test]
    fn a_stale_report_is_rebuilt_so_status_reflects_work_done_since() {
        let state = state();
        let now = chrono::Utc::now();
        let first = state.usage_report(now, 1, || report(1));
        assert_eq!(first.completed_sessions, 1);
        let later = state.usage_report(now + USAGE_MAX_AGE, 1, || report(2));
        assert_eq!(later.completed_sessions, 2);
    }

    #[test]
    fn a_clock_that_jumps_backwards_rebuilds_rather_than_serving_the_future() {
        // Suspend/resume and NTP corrections both do this, and a cached entry
        // stamped in the future would otherwise never expire.
        let state = state();
        let now = chrono::Utc::now();
        state.usage_report(now, 1, || report(1));
        let earlier = state.usage_report(now - chrono::Duration::hours(1), 1, || report(2));
        assert_eq!(earlier.completed_sessions, 2);
    }
}
