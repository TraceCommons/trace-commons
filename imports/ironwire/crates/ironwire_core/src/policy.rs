//! Routing policy: the fidelity ladder, conversation affinity, and hysteresis.
//!
//! The decision this module makes is *per conversation*, not per request. A
//! coding agent's conversation carries a large warm prompt cache and, often,
//! provider-private reasoning state; moving it costs real money and real
//! latency, and moving it across API families can cost correctness
//! (`docs/CRITIQUE.md` §1). So a route change is a state transition taken under
//! sustained pressure, not a fresh choice made 200 times an hour.

use std::collections::HashMap;
use std::fmt;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::capability::{Capabilities, Ineligible, RequestRequirements, eligible};
use crate::peek::RequestPeek;
use crate::protocol::{BackendId, BackendKind, ClientIdentity, ModelTier, Protocol};
use crate::quota::QuotaSnapshot;

/// How far down the fidelity ladder a route sits, relative to the ideal.
///
/// Rungs 0–2 are silent because nothing the user can observe changes. Rung 3
/// changes how their agent behaves, so it is announced — pretending otherwise
/// is how we lose their trust the first time it matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rung {
    /// Preferred backend, preferred model. Cache warm, reasoning intact.
    ///
    /// The default, so a client reading an older daemon's status assumes the
    /// undegraded case rather than inventing a fallback that never happened.
    #[default]
    Preferred,
    /// Same account, smaller model. Cache mostly warm, reasoning intact.
    SmallerModel,
    /// Same wire format, different credential. Cache cold, reasoning intact,
    /// zero translation.
    ///
    /// With one exception, enforced rather than tolerated: a conversation
    /// replaying ChatGPT encrypted reasoning never changes ChatGPT account,
    /// because the provider rejects reasoning minted under another
    /// organization ([`Ineligible::ReasoningBoundToAccount`]).
    AlternateCredential,
    /// Different API family. Cache cold, reasoning dropped, translation
    /// required. The user is told.
    CrossFamily,
}

impl Rung {
    /// Whether descending to this rung is worth surfacing to the user.
    #[must_use]
    pub fn is_user_visible(self) -> bool {
        self == Self::CrossFamily
    }
}

/// Opaque per-conversation identity derived from request content.
///
/// A supplied client session takes precedence; otherwise IronWire derives a
/// key from the stable head of the conversation. Keys are memory-only and never persisted
/// alongside content.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ConversationKey(pub u64);

impl ConversationKey {
    /// A client's session remains one conversation across changed preambles,
    /// tools and compaction. The protocol and login keep equal session strings
    /// from different clients from sharing a route or privacy salt.
    #[must_use]
    pub fn for_session(protocol: Protocol, session: &str, login: Option<&BackendId>) -> Self {
        use std::hash::{DefaultHasher, Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        protocol.hash(&mut hasher);
        session.hash(&mut hasher);
        login.hash(&mut hasher);
        Self(hasher.finish())
    }
    /// Derive a key from the parts of a request that stay fixed as a
    /// conversation grows: the façade, the system preamble and the tool set.
    ///
    /// Deliberately *not* including the message list — that changes every turn,
    /// and a key that changes every turn is not affinity, it is noise.
    #[must_use]
    pub fn derive(protocol: Protocol, system_prefix: &str, tool_names: &[&str]) -> Self {
        use std::hash::{DefaultHasher, Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        protocol.family().hash(&mut hasher);
        // Bound the prefix: system prompts embed a live timestamp and a working
        // directory in some clients, and hashing the whole thing would split
        // one conversation into many.
        system_prefix
            .as_bytes()
            .iter()
            .take(512)
            .for_each(|b| b.hash(&mut hasher));
        let mut names: Vec<&str> = tool_names.to_vec();
        names.sort_unstable();
        names.hash(&mut hasher);
        Self(hasher.finish())
    }
}

/// A candidate backend, as the router sees it.
#[derive(Debug, Clone)]
pub struct Candidate {
    /// Which backend.
    pub id: BackendId,
    /// Capacity kind, for marginal-cost preference.
    pub kind: BackendKind,
    /// What it can preserve.
    pub caps: Capabilities,
    /// Observed capacity state.
    pub quota: QuotaSnapshot,
    /// Whether the credential is currently usable.
    pub healthy: bool,
    /// Provider-observed credential account scope, when the backend has one.
    pub account_scope: Option<String>,
    /// Whether the user has consented to this backend, where consent is
    /// required (`docs/TRUST.md` §2).
    pub consented: bool,
    /// The one client identity this backend will serve, if it has one. Set for
    /// subscription backends; see `docs/TRUST.md` §3.
    pub required_client_identity: Option<ClientIdentity>,
    /// Whether this is the login the plain façade belongs to: its product's
    /// default login (`claude-sub`, `codex-sub`) in the default location, the
    /// one a client with no `CLAUDE_CONFIG_DIR` or `CODEX_HOME` is using.
    ///
    /// False for a default login relocated with `config_dir`. Its token is
    /// another directory's, and the client still reading the default
    /// directory may well be logged in as someone else, so a relocated default
    /// is reached only through its login-bound URL, like an additional login.
    /// False for anything that is not a subscription.
    pub default_login: bool,
    /// Models this backend offers, best-first, with the tier each satisfies.
    pub models: Vec<(String, ModelTier)>,
    /// Whether the user's privacy mode permits routing here.
    ///
    /// Computed where the config lives, beside `consented`, so this crate stays
    /// free of config knowledge. Always true below `privacy.mode = "full"`.
    pub trusted: bool,
    /// Whether [`Self::models`] came from the provider rather than from a list
    /// compiled into this binary.
    ///
    /// The difference decides what an unknown model means. Against a list the
    /// provider gave us, "not in the catalogue" means the backend genuinely
    /// cannot serve it, and descending to something it can is correct. Against
    /// a compiled-in guess it means only that this build is older than the
    /// model — and substituting our newest known name for the client's is how
    /// `claude-opus-5` became `claude-opus-4-6` and failed.
    pub catalogue_from_provider: bool,
}

/// The router's answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouteDecision {
    /// Chosen backend.
    pub backend: BackendId,
    /// Model to request from it. `None` means forward the client's choice
    /// untouched — the native lane's preferred case, since it means the body
    /// needs no edit at all.
    pub model: Option<String>,
    /// How far down the ladder this sits.
    pub rung: Rung,
    /// Whether serving this needs protocol translation.
    pub translated: bool,
    /// Short reason, for logs and the control API.
    pub reason: String,
}

/// Why no route was possible. Every variant is something we can explain to the
/// user in one sentence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NoRoute {
    /// Nothing is configured for this façade.
    NoBackendsConfigured,
    /// Every backend is rate-limited or unhealthy.
    AllExhausted,
    /// Backends exist and none of them could take this request.
    ///
    /// Carries **every** backend that was considered, including the ones that
    /// never reached the capability gate. That is the whole point of the
    /// variant: it used to report only the candidates that failed *eligibility*,
    /// so a machine with two unconsented subscriptions and one backend on the
    /// wrong wire told the user about the wrong wire and stayed silent about
    /// the two credentials that were sitting right there. The reason a user can
    /// act on is almost never the interesting one.
    NoneUsable {
        /// Per-backend refusals, most actionable first.
        refusals: Vec<(BackendId, Refusal)>,
    },
    /// The only backends that could serve this need a client identity the
    /// request does not carry.
    RequiresClientIdentity,
    /// `privacy.mode = "full"` is on and no trusted backend could serve this.
    ///
    /// A refusal, never a fallback. Everywhere else IronWire's job is to keep
    /// the agent alive; this is the one place that is subordinate, because a
    /// user who set `full` said that not sending the data matters more than
    /// finishing the turn.
    NoTrustedBackendAvailable {
        /// Trusted backends that exist, and why each could not serve this.
        tried: Vec<(BackendId, String)>,
        /// Ids named in `trusted_backends` that are not registered at all — a
        /// typo here refuses everything, and saying only "no trusted backend"
        /// sends the user hunting in the wrong place.
        missing: Vec<String>,
    },
    /// Every backend that could have served this is stopped by a spend cap the
    /// user set.
    ///
    /// Distinct from [`Self::AllExhausted`] on purpose: a cap the user set and
    /// then cannot recognise in the error is worse than no cap, so the message
    /// names their own number and the key they set it with.
    SpendCapReached {
        /// A backend that was capped, for the message.
        backend: String,
        /// Spent against the cap.
        spent_usd: f64,
        /// The cap.
        cap_usd: f64,
    },
    /// An `X-IronWire-Route` header named a backend that does not exist.
    ///
    /// An error rather than a fall-through: the caller asked for something
    /// specific, and quietly serving them from somewhere else would be worse
    /// than saying we could not.
    UnknownRoute {
        /// What they asked for.
        requested: String,
        /// What exists, so the answer is actionable.
        available: Vec<String>,
    },
}

/// The refusal as a sentence, for a log line.
///
/// The `Debug` rendering this replaces put a struct literal in the daemon's
/// log — `AllIneligible { reasons: [(BackendId("nearai"), NoTranslationPath)] }`
/// — which is readable if you already know the codebase and useless otherwise.
/// A log line is the one place a user looks after their agent stops.
impl fmt::Display for NoRoute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoBackendsConfigured => f.write_str("no backends are configured"),
            Self::AllExhausted => f.write_str("every backend is rate limited or unhealthy"),
            Self::NoneUsable { refusals } => {
                f.write_str("no backend could take this request")?;
                for (id, why) in refusals {
                    write!(f, "; {id}: {}", why.describe())?;
                }
                Ok(())
            }
            Self::RequiresClientIdentity => f.write_str(
                "every backend that could serve this needs the originating product's own client identity",
            ),
            Self::NoTrustedBackendAvailable { tried, missing } => {
                f.write_str("privacy.mode = full and no trusted backend could serve this")?;
                for (id, why) in tried {
                    write!(f, "; {id}: {why}")?;
                }
                if !missing.is_empty() {
                    write!(f, "; not registered at all: {}", missing.join(", "))?;
                }
                Ok(())
            }
            Self::SpendCapReached {
                backend,
                spent_usd,
                cap_usd,
            } => write!(
                f,
                "{backend} has reached the ${cap_usd:.2} spend cap you set (${spent_usd:.2} spent today)"
            ),
            Self::UnknownRoute {
                requested,
                available,
            } => write!(
                f,
                "`{requested}` is not a connected backend (connected: {})",
                if available.is_empty() {
                    "none".to_string()
                } else {
                    available.join(", ")
                }
            ),
        }
    }
}

/// One turn's routing inputs, travelling together because they always do.
#[derive(Clone, Copy)]
struct Turn<'a> {
    inbound: Protocol,
    peek: &'a RequestPeek,
    tier: ModelTier,
    candidates: &'a [Candidate],
    now: DateTime<Utc>,
    accounts: Accounts<'a>,
}

/// What a decision knows about the user's logins beyond the candidate and the
/// request.
#[derive(Clone, Copy)]
struct Accounts<'a> {
    /// `[subscriptions] cross_account_fallback`.
    cross_account_fallback: bool,
    /// The native Responses backend and observed scope that served this
    /// session. A missing record makes every native encrypted replay ineligible.
    reasoning_account: Option<&'a ReasoningAccount>,
}

/// Which Responses credential scope last served a client session a native Responses
/// turn, and so minted the encrypted reasoning its history replays.
///
/// Kept apart from [`Affinity`] because the two have different lifetimes. An
/// affinity is where the conversation should go next, and the pipeline forgets
/// it whenever its backend fails so the retry can re-select. Which account the
/// reasoning came from does not change when that account fails. Taking the
/// lock's answer from the affinity handed a sibling's reasoning to the
/// client's own login on the sibling's first 429, the one move the lock
/// exists to prevent, and the wrong account then held the lock.
#[derive(Debug, Clone)]
struct ReasoningAccount {
    backend: BackendId,
    account_scope: Option<String>,
    last_seen: DateTime<Utc>,
}

/// Whose [`ReasoningAccount`] a request reads and writes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ReasoningKey {
    /// The client's own session id ([`RequestPeek::client_session`]), with
    /// the login it came in on. The conversation key cannot tell two
    /// concurrent Codex sessions apart, and when one of them moves to a
    /// sibling the other's reasoning has not moved with it.
    Session(String, Option<BackendId>),
    /// No session id: the conversation key and the login, the best a request
    /// that names no session allows.
    Conversation(ConversationKey, Option<BackendId>),
}

impl ReasoningKey {
    fn of(key: &ConversationKey, peek: &RequestPeek) -> Self {
        match &peek.client_session {
            Some(session) => Self::Session(session.clone(), peek.client_login.clone()),
            None => Self::Conversation(key.clone(), peek.client_login.clone()),
        }
    }
}

/// Sticky per-conversation route state.
#[derive(Debug, Clone)]
pub struct Affinity {
    /// Where this conversation is pinned.
    pub backend: BackendId,
    /// Model in use.
    pub model: Option<String>,
    /// Rung it settled on.
    pub rung: Rung,
    /// When the affinity was established.
    pub since: DateTime<Utc>,
    /// Last time a request actually routed under this affinity.
    ///
    /// Distinct from [`Self::since`], which is set once and never moves. Using
    /// `since` to decide what to evict would throw out the longest-lived
    /// healthy sessions first — exactly backwards, and invisible in a test
    /// short enough that the two never diverge.
    pub last_seen: DateTime<Utc>,
    /// First time we saw sustained pressure on the current backend. Cleared
    /// when pressure lifts; a descent needs this to be older than the debounce.
    pub pressure_since: Option<DateTime<Utc>>,
    /// First time a better rung was available again. The mirror of
    /// [`Self::pressure_since`], cleared whenever pressure returns, and a
    /// promotion needs it older than [`PROMOTION_DEBOUNCE`].
    pub recovery_since: Option<DateTime<Utc>>,
    /// The client login this route was chosen for ([`RequestPeek::client_login`]).
    ///
    /// The conversation key is the system preamble and the tool names, which
    /// every session of one client version shares, so a Claude Code on one
    /// login and a Claude Code on another derive the same key. Neither may
    /// inherit the other's route — least of all its account lock, which would
    /// send one login's encrypted reasoning to the other's account.
    pub client_login: Option<BackendId>,
}

/// How long pressure must persist before a conversation descends a rung.
///
/// A single 429 with a three-second `retry-after` must not throw away a
/// 200k-token warm cache. Waiting is almost always cheaper than moving.
pub const DESCENT_DEBOUNCE: Duration = Duration::seconds(20);

/// How long a better rung must stay available before a conversation climbs back
/// to it.
///
/// Deliberately far longer than [`DESCENT_DEBOUNCE`], and the asymmetry is the
/// design rather than an oversight. Descending is urgent: the alternative is a
/// failed turn. Promoting is not, because the conversation is working. And
/// `Headroom::is_pressured` is a step function at 90% with no hysteresis band
/// of its own, so this debounce is the only thing standing between a provider
/// hovering at that threshold and a conversation that discards a warm prompt
/// cache every twenty seconds — which would be strictly worse than never
/// promoting at all.
pub const PROMOTION_DEBOUNCE: Duration = Duration::minutes(5);

/// How long a conversation may go unheard from before its route is forgotten.
///
/// A coding session is minutes to hours; a key untouched for a day is a session
/// that ended. Forgetting is cheap and safe — the next request re-selects, and
/// `select` is deterministic given the same candidates, so under unchanged
/// conditions it lands on the same backend and pays one cold prompt cache.
pub const AFFINITY_TTL: Duration = Duration::hours(24);

/// Hard ceiling on tracked conversations, regardless of age.
///
/// The same number, for the same reason, as `PrivacyFilter::MAX_SALTS`: a
/// daemon meant to run under launchd or systemd for weeks must not accumulate
/// state for its whole life, and the thing being dropped costs one cold cache.
pub const MAX_AFFINITIES: usize = 512;

/// Replay origins outlive the route cache. Losing one refuses an encrypted
/// native replay rather than guessing which account minted it.
pub const MAX_REASONING_ACCOUNTS: usize = 4096;

/// Routing policy over a set of candidate backends.
#[derive(Debug, Default)]
pub struct Policy {
    affinities: HashMap<ConversationKey, Affinity>,
    /// The account lock's memory. Bounded by [`AFFINITY_TTL`] and
    /// [`MAX_REASONING_ACCOUNTS`], independently of route affinities. Restart,
    /// expiry or eviction leaves an unknown origin; native replay is refused.
    reasoning_accounts: HashMap<ReasoningKey, ReasoningAccount>,
    /// User-forced backend/model (`ironwire pin`). Overrides everything except
    /// eligibility — we will not serve a pin that would corrupt the request.
    pin: Option<(BackendId, Option<String>)>,
    /// `[subscriptions] cross_account_fallback`. Off unless the user set it.
    cross_account_fallback: bool,
}

impl Policy {
    /// New policy with no affinities and no pin.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a client may be served by another of the user's subscriptions
    /// to the same product, when its own cannot serve it
    /// (`[subscriptions] cross_account_fallback`).
    ///
    /// Held here rather than passed per call because it is a setting, read
    /// once, like the pin — and every decision, including a sticky one, has to
    /// be judged against the same value.
    pub fn set_cross_account_fallback(&mut self, enabled: bool) {
        self.cross_account_fallback = enabled;
    }

    /// Whether cross-account fallback is on.
    #[must_use]
    pub fn cross_account_fallback(&self) -> bool {
        self.cross_account_fallback
    }

    /// Force all conversations onto a backend (and optionally a model).
    pub fn set_pin(&mut self, backend: Option<BackendId>, model: Option<String>) {
        self.pin = backend.map(|b| (b, model));
    }

    /// Current pin, if any.
    #[must_use]
    pub fn pin(&self) -> Option<&(BackendId, Option<String>)> {
        self.pin.as_ref()
    }

    /// Where a conversation is currently pinned, if anywhere.
    ///
    /// Read *before* deciding, so the caller can tell a genuine route change
    /// from a request that simply stayed put. Announcing every request as a
    /// "route" would drown the one announcement that matters.
    #[must_use]
    pub fn current_backend(&self, key: &ConversationKey) -> Option<BackendId> {
        self.affinities.get(key).map(|a| a.backend.clone())
    }

    /// Forget a conversation's affinity.
    ///
    /// Not which account its reasoning came from: a failed backend still
    /// minted what the history replays.
    pub fn forget(&mut self, key: &ConversationKey) {
        self.affinities.remove(key);
    }

    /// Number of conversations currently tracked: those routed within
    /// [`AFFINITY_TTL`], capped at [`MAX_AFFINITIES`].
    ///
    /// A description of the present, which is what `ironwire status` renders it
    /// as. Before the map was bounded this was a lifetime counter that only
    /// ever went up, and meant nothing after a day of use.
    #[must_use]
    pub fn tracked_conversations(&self) -> usize {
        self.affinities.len()
    }

    /// Drop conversations that have gone quiet, then any excess over the cap.
    ///
    /// A full `retain` per request is O(n) with n capped at 512 — microseconds,
    /// lost entirely in the noise of an HTTP round trip, and this runs under
    /// the lock that `AppState` holds across a decision. The alternative is an
    /// LRU list, which `PrivacyFilter` already considered and rejected for the
    /// same trade-off; a second, more complicated answer to the same question
    /// in the same daemon is worse than a slightly slower one.
    fn sweep(&mut self, now: DateTime<Utc>) {
        bound(&mut self.affinities, now, MAX_AFFINITIES, |affinity| {
            affinity.last_seen
        });
        bound(
            &mut self.reasoning_accounts,
            now,
            MAX_REASONING_ACCOUNTS,
            |record| record.last_seen,
        );
    }

    /// Record the native Responses credential scope that served this turn.
    /// Cross-family turns leave its origin intact because earlier encrypted
    /// history can still be replayed. Unknown scopes never authorize a replay.
    fn note_reasoning_account(
        &mut self,
        key: ReasoningKey,
        inbound: Protocol,
        decision: &RouteDecision,
        candidates: &[Candidate],
        now: DateTime<Utc>,
    ) {
        let minted_here = inbound == Protocol::OpenAiResponses
            && !decision.translated
            && candidates
                .iter()
                .find(|c| c.id == decision.backend)
                .is_some_and(|candidate| candidate.caps.wires.speaks(Protocol::OpenAiResponses));
        if minted_here {
            self.reasoning_accounts.insert(
                key,
                ReasoningAccount {
                    backend: decision.backend.clone(),
                    account_scope: candidates
                        .iter()
                        .find(|c| c.id == decision.backend)
                        .and_then(|c| c.account_scope.clone()),
                    last_seen: now,
                },
            );
            bound(
                &mut self.reasoning_accounts,
                now,
                MAX_REASONING_ACCOUNTS,
                |record| record.last_seen,
            );
        } else if let Some(record) = self.reasoning_accounts.get_mut(&key) {
            record.last_seen = now;
        }
    }

    /// Choose where a request goes.
    ///
    /// `inbound` is the façade's protocol; a candidate speaking a different
    /// family means translation, and is only reachable at [`Rung::CrossFamily`].
    ///
    /// # Errors
    ///
    /// Returns [`NoRoute`] when nothing can serve the request, with a reason
    /// specific enough to show the user.
    pub fn decide(
        &mut self,
        key: ConversationKey,
        inbound: Protocol,
        peek: &RequestPeek,
        candidates: &[Candidate],
        now: DateTime<Utc>,
    ) -> Result<RouteDecision, NoRoute> {
        self.decide_with_override(key, inbound, peek, candidates, now, None)
    }

    /// [`Self::decide`], with a per-request route override from
    /// `X-IronWire-Route`.
    ///
    /// The override outranks a daemon-wide pin: it is the more specific
    /// instruction, and it came with this request.
    ///
    /// # Errors
    ///
    /// [`NoRoute::UnknownRoute`] when the override names a backend that does
    /// not exist, and [`NoRoute::AllIneligible`] when it names one that cannot
    /// serve this request.
    pub fn decide_with_override(
        &mut self,
        key: ConversationKey,
        inbound: Protocol,
        peek: &RequestPeek,
        candidates: &[Candidate],
        now: DateTime<Utc>,
        route_override: Option<(BackendId, Option<String>)>,
    ) -> Result<RouteDecision, NoRoute> {
        let reasoning_key = ReasoningKey::of(&key, peek);
        // A model named by the override or the pin is the user's instruction,
        // and outranks the one in the request.
        let forced_model = route_override
            .as_ref()
            .or(self.pin.as_ref())
            .and_then(|(backend, model)| model.as_ref().map(|_| backend.clone()));
        let mut decision =
            self.route(key.clone(), inbound, peek, candidates, now, route_override)?;
        if forced_model.as_ref() != Some(&decision.backend)
            && keep_listed_model(&mut decision, peek, candidates)
            && let Some(affinity) = self.affinities.get_mut(&key)
            && affinity.backend == decision.backend
        {
            // Store what is actually served, so a later turn does not replay a
            // fallback this one did not take.
            affinity.model = decision.model.clone();
            affinity.rung = decision.rung;
        }
        self.note_reasoning_account(reasoning_key, inbound, &decision, candidates, now);
        Ok(decision)
    }

    fn route(
        &mut self,
        key: ConversationKey,
        inbound: Protocol,
        peek: &RequestPeek,
        candidates: &[Candidate],
        now: DateTime<Utc>,
        route_override: Option<(BackendId, Option<String>)>,
    ) -> Result<RouteDecision, NoRoute> {
        self.sweep(now);
        if candidates.is_empty() {
            return Err(NoRoute::NoBackendsConfigured);
        }

        // A named backend that does not exist is an error, not a fall-through.
        if let Some((requested, _)) = &route_override
            && !candidates.iter().any(|c| c.id == *requested)
        {
            return Err(NoRoute::UnknownRoute {
                requested: requested.to_string(),
                available: candidates.iter().map(|c| c.id.to_string()).collect(),
            });
        }

        // A cache-control breakpoint says a prefix *may* be cached, not that it
        // is. Claude Code marks its system prompt on every request, the first
        // one included, so the peek alone would call every Claude Code turn a
        // large warm cache and refuse it any backend without prompt caching.
        // A prefix is warm only when an earlier turn of this conversation went
        // to a backend that caches and is still connected; otherwise there is
        // nothing for the cache rule in `eligible` to protect.
        let cold;
        let peek = if self.cache_is_warm(&key, candidates) {
            peek
        } else {
            let mut unwarmed = peek.clone();
            unwarmed.requirements.cached_prefix_tokens = 0;
            cold = unwarmed;
            &cold
        };

        let tier = peek
            .requested_model
            .as_deref()
            .map_or(ModelTier::Frontier, ModelTier::from_model_hint);

        // A pin bypasses preference but not eligibility: an unusable route is
        // still unusable, and silently corrupting a request because the user
        // asked for a backend is not obedience, it is a bug.
        let forced = route_override.clone().or_else(|| self.pin.clone());
        if let Some((pinned, model)) = forced
            && let Some(candidate) = candidates.iter().find(|c| c.id == pinned)
        {
            // A pin overrides *preference*. It cannot override the privacy
            // mode: a per-request header that could relax a constraint set in
            // the user's own config would make the setting decorative for
            // anything able to set a header.
            if !candidate.trusted {
                return Err(NoRoute::NoTrustedBackendAvailable {
                    tried: vec![(
                        candidate.id.clone(),
                        "named by a pin or X-IronWire-Route, but not in \
                         privacy.trusted_backends"
                            .to_string(),
                    )],
                    missing: Vec::new(),
                });
            }
            // Nor whose capacity this is. A header a client can set must not
            // make one product's subscription serve another product's client,
            // nor one login serve a client logged in as another while
            // `cross_account_fallback` is off.

            if let Err(why) = serves_this_client(candidate, peek, self.cross_account_fallback) {
                return Err(match why {
                    Unusable::NeedsClientIdentity => NoRoute::RequiresClientIdentity,
                    other => NoRoute::NoneUsable {
                        refusals: vec![(candidate.id.clone(), Refusal::from(&other))],
                    },
                });
            }
            // Nor the consent the user never gave. Consent is what authorises
            // replaying another product's token at all (`docs/TRUST.md` §2),
            // and a pin, or a header any local client can set, is not that.
            if candidate.kind.requires_consent() && !candidate.consented {
                return Err(NoRoute::NoneUsable {
                    refusals: vec![(candidate.id.clone(), Refusal::NotConsented)],
                });
            }
            // The same wire question `usable` asks, and for the same reason. It
            // used to compare *families* here, which is a different question
            // with the same shape: Responses and Chat Completions share a
            // family and are different wires, so a pinned Codex request was
            // handed to a Chat backend as a native forward — `translated:
            // false`, a Responses body, and an endpoint that cannot read it.
            // The unpinned path refused that route correctly; a pin is meant to
            // override preference, never turn a refusal into a malformed
            // request.
            let cross = !candidate.caps.wires.speaks(inbound);
            let origin = self.reasoning_accounts.get(&ReasoningKey::of(&key, peek));
            let requirements = requirements_for(candidate, peek, origin);
            eligible(
                &requirements,
                &candidate.caps,
                cross,
                Some((
                    candidate.id.as_str(),
                    if cross {
                        candidate.caps.wires.primary()
                    } else {
                        inbound
                    },
                )),
            )
            .map_err(|why| NoRoute::NoneUsable {
                refusals: vec![(candidate.id.clone(), Refusal::Ineligible(why))],
            })?;
            return Ok(RouteDecision {
                backend: candidate.id.clone(),
                model: model.or_else(|| pick_model(candidate, tier)),
                // `Preferred` even on another of the user's logins, unlike
                // the same route chosen by `select`: the user named it, so it
                // is not a fallback the opt-in took on their behalf.
                rung: if cross {
                    Rung::CrossFamily
                } else {
                    Rung::Preferred
                },
                translated: cross,
                reason: if route_override.is_some() {
                    "X-IronWire-Route".to_string()
                } else {
                    "pinned by user".to_string()
                },
            });
        }

        // Sticky: if this conversation already has a home and that home is
        // still fine, stay. Moving costs a cache; staying costs nothing.
        //
        // One exception. A conversation that descended a rung earlier is
        // sitting on degraded capacity, and a compaction turn is the worst
        // possible turn to spend it on: the summary becomes the conversation.
        // So a compaction turn re-selects rather than inheriting a degraded
        // affinity, and pays the cache cost to do it.
        //
        // Only an affinity chosen for this client's own login counts; see
        // `Affinity::client_login`.
        let affinity = self
            .affinities
            .get(&key)
            .filter(|a| a.client_login == peek.client_login)
            .cloned();
        let inherit_affinity =
            !peek.likely_compaction || affinity.as_ref().is_none_or(|a| a.rung == Rung::Preferred);
        // Binds whether or not that account is among the candidates. One
        // missing from them is not known to be up or down, and not knowing is
        // no licence to send its reasoning to an account that will refuse it.
        let reasoning_account = self
            .reasoning_accounts
            .get(&ReasoningKey::of(&key, peek))
            .cloned();
        let turn = Turn {
            inbound,
            peek,
            tier,
            candidates,
            now,
            accounts: Accounts {
                cross_account_fallback: self.cross_account_fallback,
                reasoning_account: reasoning_account.as_ref(),
            },
        };

        if inherit_affinity
            && let Some(affinity) = affinity.clone()
            && let Some(candidate) = candidates.iter().find(|c| c.id == affinity.backend)
            && incumbent_is_usable(candidate, turn)
        {
            let pressured = candidate.quota.is_pressured(now);
            let entry = self.affinities.get_mut(&key).expect("just found above");
            // Both early returns below leave through here, so this is the one
            // place that has to be right: miss it on the unpressured branch —
            // the common one — and every healthy conversation ages out while
            // actively in use.
            entry.last_seen = now;
            // Scoped so the mutable borrow ends before `select` needs `&self`
            // below. Everything the rest of this block depends on is read out
            // here.
            let pressure_started = if pressured {
                // A return to pressure discards accumulated recovery: a backend
                // hovering at the 90% threshold must not creep towards a
                // promotion one unpressured turn at a time.
                entry.recovery_since = None;
                Some(*entry.pressure_since.get_or_insert(now))
            } else {
                entry.pressure_since = None;
                None
            };

            match pressure_started {
                // Hysteresis: only descend once pressure has persisted.
                Some(since) if now - since < DESCENT_DEBOUNCE => {
                    return Ok(RouteDecision {
                        backend: affinity.backend,
                        model: affinity.model,
                        rung: affinity.rung,
                        translated: affinity.rung == Rung::CrossFamily,
                        reason: "sticky: pressure not yet sustained".to_string(),
                    });
                }
                // Sustained pressure: fall through to a fresh selection.
                Some(_) => {}
                None => {
                    if let Some(promoted) = self.consider_promotion(&key, &affinity, turn) {
                        return Ok(promoted);
                    }
                    return Ok(RouteDecision {
                        backend: affinity.backend,
                        model: affinity.model,
                        rung: affinity.rung,
                        translated: affinity.rung == Rung::CrossFamily,
                        reason: "sticky affinity".to_string(),
                    });
                }
            }
        }

        let decision = self.select(turn)?;
        self.affinities.insert(
            key,
            Affinity {
                backend: decision.backend.clone(),
                model: decision.model.clone(),
                rung: decision.rung,
                since: now,
                last_seen: now,
                pressure_since: None,
                recovery_since: None,
                client_login: peek.client_login.clone(),
            },
        );
        // After the insert, not before: sweeping first leaves room for one more
        // and the map settles at the cap plus one. This is also the only point
        // the map grows, and it is safely clear of the `get`/`get_mut` pair
        // above, where an eviction could have pulled the ground out from under
        // that `expect`.
        bound(&mut self.affinities, now, MAX_AFFINITIES, |affinity| {
            affinity.last_seen
        });
        Ok(decision)
    }

    /// Whether this conversation has a prompt cache worth protecting: its
    /// current backend caches prompts and is still among the candidates. A
    /// backend that was removed took its cache with it.
    fn cache_is_warm(&self, key: &ConversationKey, candidates: &[Candidate]) -> bool {
        self.affinities.get(key).is_some_and(|affinity| {
            candidates
                .iter()
                .any(|c| c.id == affinity.backend && c.caps.prompt_cache)
        })
    }

    /// Climb back up the ladder, if a better rung has been available long
    /// enough to believe in.
    ///
    /// Descent is a one-way door without this: a conversation that fell to a
    /// cross-family backend at nine in the morning finds it perfectly usable
    /// and unpressured all afternoon, and stays there on a cold cache while the
    /// capacity the user pays for sits idle. A state machine with no path back
    /// is not a transition, it is a trap.
    ///
    /// `None` means stay put, which is the answer on the overwhelming majority
    /// of turns — an undegraded conversation does not even reach here.
    fn consider_promotion(
        &mut self,
        key: &ConversationKey,
        affinity: &Affinity,
        turn: Turn<'_>,
    ) -> Option<RouteDecision> {
        let Turn { peek, now, .. } = turn;
        // A conversation already at the top has nowhere to climb, and this is
        // the common case: no `select` runs for it.
        if affinity.rung == Rung::Preferred {
            return None;
        }

        // Ask the ladder rather than asking whether the preferred backend's
        // quota recovered. `select` already encodes eligibility, consent,
        // client identity, circuit state, catalogue and tier fit; a
        // reimplementation here would drift from it and would miss a backend
        // whose quota came back while its circuit stayed open.
        //
        // Compared by *rung*, never by backend id: rung 1 to rung 0 can be a
        // model change on the very same backend.
        //
        // That includes the account lock. A conversation replaying encrypted
        // reasoning minted on a sibling ChatGPT login cannot climb back to the
        // client's own login — the move would be rejected exactly as the
        // descent would — and `select` refuses that login for it, while still
        // reporting the sibling as rung 2, so nothing here looks better.
        let better = self
            .select(turn)
            .ok()
            .filter(|decision| decision.rung < affinity.rung);
        let Some(candidate) = better else {
            // The improvement went away again. Recovery accumulated so far is
            // discarded rather than banked: a backend crossing in and out of
            // availability must not creep towards a promotion one good turn at
            // a time, because each promotion costs a warm prompt cache.
            if let Some(entry) = self.affinities.get_mut(key) {
                entry.recovery_since = None;
            }
            return None;
        };

        let entry = self.affinities.get_mut(key)?;
        // Recovery starts the first turn a better rung is available, whether or
        // not this turn can act on it.
        let recovering_since = *entry.recovery_since.get_or_insert(now);
        if now - recovering_since < PROMOTION_DEBOUNCE {
            return None;
        }

        // The same rule as the descent that created this route
        // (`docs/PROTOCOL.md` §6), and for the same reason rather than out of
        // symmetry: the recent assistant turns were produced by the foreign
        // family and carry none of this family's signed reasoning state, so
        // replaying that history mid-loop is the rejection risk the gate
        // exists to prevent. Blocked is not cancelled — `recovery_since`
        // stands, so the next turn boundary promotes rather than restarting
        // the wait.
        if affinity.rung == Rung::CrossFamily && peek.requirements.mid_tool_loop {
            return None;
        }

        *entry = Affinity {
            backend: candidate.backend.clone(),
            model: candidate.model.clone(),
            rung: candidate.rung,
            since: now,
            last_seen: now,
            pressure_since: None,
            recovery_since: None,
            client_login: affinity.client_login.clone(),
        };
        // The decision `select` produced, not a patched copy of the old one:
        // `translated` has to come from the new route. Updating the rung while
        // leaving `translated` derived from the stale one would translate a
        // request to a native backend and corrupt it.
        Some(RouteDecision {
            reason: format!("recovered to {:?}", candidate.rung).to_lowercase(),
            ..candidate
        })
    }

    /// Fresh selection, ignoring affinity — except for the account the
    /// conversation's reasoning is bound to. Walks the ladder rung by rung.
    fn select(&self, turn: Turn<'_>) -> Result<RouteDecision, NoRoute> {
        let Turn {
            inbound,
            peek,
            tier,
            candidates,
            now,
            ..
        } = turn;
        // Every backend that did not take the request, and why. Collected for
        // all of them rather than only the ones that failed the capability gate
        // — see `NoRoute::NoneUsable`.
        let mut refusals: Vec<(BackendId, Refusal)> = Vec::new();
        let mut capped: Vec<(BackendId, f64, f64)> = Vec::new();

        // Rungs 0-2 all forward the request's own bytes, so they need a backend
        // speaking the *same wire* — not merely one in the same family, which
        // would put a Chat Completions backend on a Responses request. Rung 3
        // is the translated lane. Within each we prefer free capacity, then
        // exact tier, then anything.
        let mut same_wire: Vec<&Candidate> = Vec::new();
        let mut cross_family: Vec<&Candidate> = Vec::new();

        for candidate in candidates {
            if let Err(unusable) = usable(candidate, turn) {
                if matches!(unusable, Unusable::NoCapacity)
                    && let crate::quota::Headroom::CapReached {
                        spent_usd, cap_usd, ..
                    } = candidate.quota.primary
                {
                    capped.push((candidate.id.clone(), spent_usd, cap_usd));
                }
                refusals.push((candidate.id.clone(), Refusal::from(&unusable)));
                continue;
            }
            if candidate.caps.wires.speaks(inbound) {
                same_wire.push(candidate);
            } else {
                cross_family.push(candidate);
            }
        }

        // On an ordinary turn: free capacity first, then quality. On a
        // compaction turn the two swap, because the output of a compaction turn
        // *becomes the conversation* — it is written into the client's
        // permanent history and resent every turn afterwards
        // (`docs/PROTOCOL.md` §8). Saving money there buys one cheaper request
        // and pays for it for the rest of the session.
        //
        // Among equal cost, the client's own login comes before the user's
        // other logins, and only then registration order. Without cross-account
        // fallback the others are never usable and this changes nothing. With
        // it, pressure still outranks the own-login preference: an own login
        // at 90% or more sorts behind an unpressured sibling, so a fresh
        // selection moves at once, and a conversation already on the own login
        // once that pressure outlasts `DESCENT_DEBOUNCE`.
        let sort_key = |c: &&Candidate| {
            let pressured = u8::from(c.quota.is_pressured(now));
            let cost = c.kind.marginal_cost_rank();
            let another_login = u8::from(!is_own_login(c, peek));
            // Local capacity is the cheapest there is — rank 0, ahead of a
            // subscription — and descending a rung is normally fine, because
            // rung 1 means "same account, smaller model". A local backend
            // breaks that assumption: descending onto it means a 30B model on
            // the user's laptop taking work they asked Opus for, silently,
            // because it sorted cheapest. So a local backend that cannot serve
            // the requested tier yields to anything that can. It is still
            // tried when it is all there is — being last is not being refused.
            let beyond_its_tier = u8::from(c.kind == BackendKind::Local && !serves_tier(c, tier));
            if peek.likely_compaction {
                // `pick_model` falls back to a lesser tier rather than refusing,
                // so "has no model at all" is the wrong question here — what
                // matters is whether this backend can serve the tier the client
                // actually asked for, without descending.
                (
                    pressured,
                    beyond_its_tier,
                    u8::from(!serves_tier(c, tier)),
                    cost,
                    another_login,
                )
            } else {
                (
                    pressured,
                    beyond_its_tier,
                    cost,
                    another_login,
                    u8::from(pick_model(c, tier).is_none()),
                )
            }
        };
        same_wire.sort_by_key(sort_key);
        cross_family.sort_by_key(sort_key);

        if let Some(best) = same_wire.first() {
            // Two independent ways a same-wire route can be degraded, and the
            // rung is the worse of them. Deriving it from the model tier alone
            // — as this did — recorded a fall from a subscription to a metered
            // key as `Preferred`, because the key serves the same tier. That
            // made the most expensive fallback in the product invisible in the
            // ledger and, worse, permanent: `consider_promotion` returns early
            // on `Preferred`, so the conversation never went back to the
            // subscription once its window reset.
            //
            // "Better credential" is `marginal_cost_rank`, which is already the
            // router's own statement of preference — a second definition of
            // which backend the user wanted would drift from it. Only
            // candidates that could actually have served this request count, so
            // a cheaper backend skipped for tier reasons, or refused for good
            // (`was_passed_over`), does not make every route look degraded.
            // Preference is the order `sort_key` uses: cost, then the client's
            // own login, then registration order (`BackendRegistry::push`).
            // Cost alone was not enough: two API keys rank equal, so falling
            // from one to the other read as `Preferred`, which is the same
            // invisible-and-permanent fallback rung 2 exists to prevent, one
            // class narrower.
            let positions: HashMap<_, _> = candidates
                .iter()
                .enumerate()
                .map(|(index, candidate)| (&candidate.id, index))
                .collect();
            let position = |id: &BackendId| positions.get(id).copied();
            let preference = |c: &Candidate| {
                (
                    c.kind.marginal_cost_rank(),
                    u8::from(!is_own_login(c, peek)),
                    position(&c.id),
                )
            };
            let passed_over_a_preferred_credential = candidates
                .iter()
                .filter(|c| c.caps.wires.speaks(inbound) && c.id != best.id)
                .filter(|c| serves_tier(c, tier))
                .filter(|c| was_passed_over(usable(c, turn)))
                .any(|c| preference(c) < preference(best));
            // Another of the user's logins is a credential change whatever
            // else was refused. Asking only what was passed over recorded a
            // client served on another account's token as rung 0 whenever its
            // own login was absent or could never have served, which hid
            // exactly the move the opt-in makes.
            let alternate_credential = passed_over_a_preferred_credential
                || (best.required_client_identity.is_some() && !is_own_login(best, peek));

            // A model missing from a catalogue we only *guessed* is not a model
            // that does not exist — it is a build older than the model. The
            // providers ship faster than we do, so on the native lane, with
            // capacity to spare, forward the client's own string and let the
            // provider be the authority. Substituting the newest name we happen
            // to know is how `claude-opus-5` became `claude-opus-4-6` and
            // failed. Once the provider has told us its catalogue, that list is
            // authoritative and a missing model means what it says.
            let unrecognised = !best.catalogue_from_provider
                && peek.requested_model.as_deref().is_some_and(|requested| {
                    !best.models.iter().any(|(known, _)| known == requested)
                });
            if unrecognised && !best.quota.is_pressured(now) {
                return Ok(RouteDecision {
                    backend: best.id.clone(),
                    model: None,
                    // No tier to compare without a catalogue entry, but the
                    // credential question still has an answer. Reporting
                    // `Preferred` here after falling off the first choice
                    // hid the fallback and, because promotion stops at
                    // `Preferred`, kept the conversation on it for good.
                    rung: if alternate_credential {
                        Rung::AlternateCredential
                    } else {
                        Rung::Preferred
                    },
                    translated: false,
                    reason: format!("native lane, {} capacity", kind_label(best.kind)),
                });
            }

            let model = pick_model(best, tier);
            let served_tier = model.as_deref().map_or(tier, ModelTier::from_model_hint);
            let rung = match (served_tier < tier, alternate_credential) {
                (_, true) => Rung::AlternateCredential,
                (true, false) => Rung::SmallerModel,
                (false, false) => Rung::Preferred,
            };
            return Ok(RouteDecision {
                backend: best.id.clone(),
                // Forward the client's own model string when the backend
                // already offers exactly it: no body edit, no risk.
                model: model.filter(|m| Some(m.as_str()) != peek.requested_model.as_deref()),
                rung,
                translated: false,
                reason: format!("native lane, {} capacity", kind_label(best.kind)),
            });
        }

        if let Some(best) = cross_family.first() {
            return Ok(RouteDecision {
                backend: best.id.clone(),
                model: pick_model(best, tier),
                rung: Rung::CrossFamily,
                translated: true,
                reason: "no same-family capacity available".to_string(),
            });
        }

        // A cap is reported ahead of a generic exhaustion, and only when it is
        // the *reason*: with `on_breach = "descend"` there is other capacity to
        // fall to and we never reach here at all.
        if let Some((backend, spent_usd, cap_usd)) = capped.first().cloned() {
            return Err(NoRoute::SpendCapReached {
                backend: backend.to_string(),
                spent_usd,
                cap_usd,
            });
        }
        // Reported ahead of everything else: when the mode is what stopped the
        // request, any other reason is incidental to a backend the user had
        // already excluded.
        if refusals
            .iter()
            .any(|(_, why)| *why == Refusal::Ineligible(Ineligible::NotTrustedUnderFullPrivacy))
        {
            return Err(NoRoute::NoTrustedBackendAvailable {
                tried: refusals
                    .iter()
                    .filter(|(_, why)| {
                        *why != Refusal::Ineligible(Ineligible::NotTrustedUnderFullPrivacy)
                    })
                    .map(|(id, why)| (id.clone(), why.describe()))
                    .collect(),
                missing: Vec::new(),
            });
        }
        // Kept ahead of the general answer rather than behind it. This check
        // used to sit *after* the ineligible list was returned, so one backend
        // on the wrong wire was enough to hide the fact that everything else
        // was waiting on a client identity — the one refusal with an actual
        // instruction attached.
        if !refusals.is_empty()
            && refusals
                .iter()
                .all(|(_, why)| *why == Refusal::NeedsClientIdentity)
        {
            return Err(NoRoute::RequiresClientIdentity);
        }
        if !refusals.is_empty() {
            // Everything merely busy is a 429 that says come back later, which
            // is a better answer than a list of backends with nothing wrong
            // with them. Anything else is a configuration problem and gets the
            // full list.
            //
            // A login that belongs to another client stays out of that
            // question: it was never this request's capacity. Counting it
            // turned "your subscription is spent, come back later" into a 503
            // about a configuration nobody got wrong, on every machine with a
            // second login. When it is all there is, it still gets named below.
            //
            // Unless this client has no login of its own among the
            // candidates. Then nothing is coming back later: it is pointed at
            // the wrong URL, and the refusal that says which login exists and
            // what binds a client to it is the answer. A transient refusal
            // beside it is incidental, and there always is one: NEAR AI is
            // registered without a key and reads as unhealthy.
            let wrong_url = refusals
                .iter()
                .any(|(_, why)| matches!(why, Refusal::AnotherLogin { .. }))
                && !candidates.iter().any(|c| {
                    c.required_client_identity.is_some()
                        && c.required_client_identity == peek.client_identity
                        && is_own_login(c, peek)
                });
            if !wrong_url
                && refusals.iter().any(|(_, why)| why.is_transient())
                && refusals.iter().all(|(_, why)| {
                    why.is_transient() || matches!(why, Refusal::AnotherLogin { .. })
                })
            {
                return Err(NoRoute::AllExhausted);
            }
            // Stable sort, so backends keep registration order within a group
            // and only actionability moves them.
            refusals.sort_by_key(|(_, why)| u8::from(!why.is_actionable()));
            return Err(NoRoute::NoneUsable { refusals });
        }
        Err(NoRoute::AllExhausted)
    }
}

/// Why one backend could not take this request.
///
/// Split finer than the router strictly needs, because the router is not the
/// only reader: this is what a refused request tells the user, and "unavailable"
/// covers a subscription nobody has consented to, a circuit that is open, and a
/// window that is spent — three different things with three different answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Refusal {
    /// A subscription whose consent has not been recorded on this machine.
    ///
    /// The most actionable refusal there is, and the one most likely to be
    /// hiding behind something else: consent is per-machine, so a second device
    /// has it unset while the credential is right there and working.
    NotConsented,
    /// Needs the originating product's own client identity, which this request
    /// does not carry (`docs/TRUST.md` §3).
    NeedsClientIdentity,
    /// One of the user's other logins for this product. The request carries
    /// the right product's identity, but its client is logged in as someone
    /// else, and `[subscriptions] cross_account_fallback` is off.
    AnotherLogin {
        /// The client this login serves, so the answer can name the command
        /// that points one at it.
        client: ClientIdentity,
    },
    /// The breaker has it open, or it is failing.
    Unhealthy,
    /// Rate limited, or out of window capacity.
    NoCapacity,
    /// It could have taken the request and cannot preserve it.
    Ineligible(Ineligible),
}

impl Refusal {
    /// Whether this is something the user can do something about right now.
    ///
    /// Used to order a refusal list. A capability mismatch is a fact about the
    /// request; an unconsented subscription is a switch nobody has flipped, and
    /// it belongs first however many other backends have more interesting
    /// problems.
    #[must_use]
    pub fn is_actionable(&self) -> bool {
        matches!(
            self,
            Self::NotConsented | Self::NeedsClientIdentity | Self::AnotherLogin { .. }
        )
    }

    /// Whether this is a capacity problem rather than a configuration one.
    ///
    /// When every refusal answers `true`, the honest reply is
    /// [`NoRoute::AllExhausted`] — a 429 that says "come back later" — rather
    /// than a 503 listing backends that are working fine and merely busy.
    #[must_use]
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::Unhealthy | Self::NoCapacity)
    }

    /// One sentence, for a log line or an error body.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::NotConsented => {
                "not enabled: this subscription's consent has not been recorded on this machine"
                    .to_string()
            }
            Self::NeedsClientIdentity => {
                "needs the originating product's own client identity, which this request does not carry"
                    .to_string()
            }
            Self::AnotherLogin { client } => format!(
                "another login: this subscription serves only the {client} logged in with \
                 it. To point that {client} here, run `ironwire connect {command} \
                 --config-dir DIR` with the directory it keeps its login in",
                command = match client {
                    ClientIdentity::ClaudeCode => "claude",
                    ClientIdentity::Codex => "codex",
                },
            ),
            Self::Unhealthy => "unhealthy: recent requests to it failed".to_string(),
            Self::NoCapacity => "no capacity: rate limited or out of window".to_string(),
            // The variant name stays in the sentence, as it does for every
            // other ineligibility, so the two read alike in a log.
            Self::Ineligible(Ineligible::ReasoningBoundToAccount) => {
                "cannot preserve this request (ReasoningBoundToAccount): its encrypted \
                 reasoning requires the original ChatGPT account or API credential. Its \
                 origin is unknown or another account was selected; start a new session, \
                 restore the original login, or use another family at a turn boundary"
                    .to_string()
            }
            Self::Ineligible(why) => format!("cannot preserve this request ({why:?})"),
        }
    }
}

/// Why a candidate cannot serve this request right now.
enum Unusable {
    Unhealthy,
    NoCapacity,
    NotConsented,
    NeedsClientIdentity,
    AnotherLogin(ClientIdentity),
    Ineligible(Ineligible),
}

impl From<&Unusable> for Refusal {
    fn from(unusable: &Unusable) -> Self {
        match unusable {
            Unusable::Unhealthy => Self::Unhealthy,
            Unusable::NoCapacity => Self::NoCapacity,
            Unusable::NotConsented => Self::NotConsented,
            Unusable::NeedsClientIdentity => Self::NeedsClientIdentity,
            Unusable::AnotherLogin(client) => Self::AnotherLogin { client: *client },
            Unusable::Ineligible(why) => Self::Ineligible(*why),
        }
    }
}

/// Whether a candidate refused this way could still have been the better
/// route, so that serving the request elsewhere is a descent.
///
/// Only refusals that are about *now* count: a busy or failing backend, and the
/// account lock, which lasts only as long as this conversation. A switch nobody
/// flipped (consent), a client that is not this backend's, or a capability the
/// request needs and the backend lacks are permanent facts about this request,
/// and counting them labelled every route rung 2 for good — an unconsented
/// second login, or the Claude subscription under an Aider request, was enough.
///
/// The lock counts for the opposite reason. A conversation whose reasoning is
/// bound to a sibling login really is on an alternate credential, and saying
/// otherwise would let `consider_promotion` relabel it `Preferred`, after
/// which promotion never looks again and the conversation key stays on the
/// sibling even once the lock has lifted.
fn was_passed_over(result: Result<(), Unusable>) -> bool {
    matches!(
        result,
        Ok(())
            | Err(Unusable::NoCapacity
                | Unusable::Unhealthy
                | Unusable::Ineligible(Ineligible::ReasoningBoundToAccount))
    )
}

/// Whether the backend a conversation is *already on* can serve this turn.
///
/// Everything `usable` refuses, except the mid-tool-loop rule — which does not
/// apply to the incumbent, and applying it here inverts the rule it comes from.
///
/// `eligible` refuses a cross-family route mid tool loop to stop a conversation
/// *switching* families in the middle of a loop (`docs/PROTOCOL.md` §6). A
/// conversation already on that backend is not switching; it is continuing, on
/// the backend whose reasoning state the loop is built from. Treating it as
/// ineligible meant a mid-loop turn found its own backend unusable and fell
/// through to a fresh selection, which dragged it back to the native family —
/// mid tool loop, which is the exact move the rule exists to prevent.
///
/// The account lock is not tolerated, and needs no tolerance: it already lets
/// the account the reasoning came from serve. Tolerating it trusted the
/// affinity to be that account, and every session of one Codex version shares
/// a conversation key, so a session that moved to a sibling took the others'
/// affinity, and their reasoning, with it.
fn incumbent_is_usable(candidate: &Candidate, turn: Turn<'_>) -> bool {
    matches!(
        usable(candidate, turn),
        Ok(()) | Err(Unusable::Ineligible(Ineligible::MidToolLoop))
    )
}

fn usable(candidate: &Candidate, turn: Turn<'_>) -> Result<(), Unusable> {
    let Turn {
        inbound,
        peek,
        now,
        accounts,
        ..
    } = turn;
    // Ownership is independent of credential health. Unobserved sibling
    // logins must explain how to bind the client, never imply a bad credential.
    serves_this_client(candidate, peek, accounts.cross_account_fallback)?;
    if !candidate.healthy {
        return Err(Unusable::Unhealthy);
    }
    if !candidate.quota.is_available(now) {
        return Err(Unusable::NoCapacity);
    }
    // Asked after health and capacity so the answer names the thing the user
    // has to do, rather than the thing that happens to be true as well.
    if candidate.kind.requires_consent() && !candidate.consented {
        return Err(Unusable::NotConsented);
    }
    // Checked before the capability gate on purpose: a user who forbade this
    // destination should be told that, not handed an incidental capability
    // mismatch on a backend they were never going to reach.
    if !candidate.trusted {
        return Err(Unusable::Ineligible(Ineligible::NotTrustedUnderFullPrivacy));
    }
    // "Needs translation" is a question about the wire, not about the family:
    // Responses and Chat Completions share a family and are different wires.
    // A backend serving several wires at one base URL answers `true` for each
    // of them, and none of those is a translation.
    //
    // Every pair has a translator (`docs/TRANSLATION.md`), so this is no longer
    // a question of whether the route is *possible* — only of what it costs,
    // which is what the rung and the eligibility gate below are for. Content
    // the target genuinely cannot express still refuses the route, but that is
    // decided by the emitter, on the body, in `pipeline::translate_request`.
    let cross = !candidate.caps.wires.speaks(inbound);
    let requirements = requirements_for(candidate, peek, accounts.reasoning_account);
    eligible(
        &requirements,
        &candidate.caps,
        cross,
        Some((
            candidate.id.as_str(),
            if cross {
                candidate.caps.wires.primary()
            } else {
                inbound
            },
        )),
    )
    .map_err(Unusable::Ineligible)
}

/// Serve the model the client named when the chosen backend's provider lists
/// it by that exact name.
///
/// The model string is a tier hint for choosing *between* backends
/// (`docs/DESIGN.md` §3). Once a backend is chosen, a name its own provider
/// reported is not a hint any more: it is a selection the provider can serve,
/// and replacing it with a sibling of the same tier is what made a tool's
/// model picker decorative. A one-provider, many-family catalogue such as
/// NEAR AI's puts almost everything at one tier, so the tier pick always
/// landed on its first entry.
///
/// A compiled-in catalogue is a guess and does not count, and a model the
/// user forced wins (handled by the caller). A `SmallerModel` rung is not a
/// reason to refuse: it only ever comes from the tier pick falling back, and a
/// priced catalogue tiers by price rather than by name, so the hinted tier can
/// be empty on a backend that lists the exact model. Serving that model is not
/// a descent, so the rung is put back.
///
/// Returns whether the decision now serves the requested model.
fn keep_listed_model(
    decision: &mut RouteDecision,
    peek: &RequestPeek,
    candidates: &[Candidate],
) -> bool {
    let Some(requested) = peek.requested_model.as_deref() else {
        return false;
    };
    let listed = candidates.iter().any(|c| {
        c.id == decision.backend
            && c.catalogue_from_provider
            && c.models.iter().any(|(model, _)| model == requested)
    });
    if !listed {
        return false;
    }
    // On the native lane `None` forwards the client's own bytes untouched; a
    // translated body is rebuilt and has to be told the model.
    decision.model = decision.translated.then(|| requested.to_string());
    if decision.rung == Rung::SmallerModel {
        decision.rung = Rung::Preferred;
    }
    true
}

/// Whether a subscription is this request's client's to use: the right
/// product, and — unless the user turned on cross-account fallback — the right
/// login of it. Anything without a required client is anyone's.
fn serves_this_client(
    candidate: &Candidate,
    peek: &RequestPeek,
    cross_account_fallback: bool,
) -> Result<(), Unusable> {
    let Some(required) = candidate.required_client_identity else {
        return Ok(());
    };
    // TRUST.md §3: we serve a subscription only for the client it belongs to,
    // and we never synthesize that client's identity to unlock it.
    // The identity must be the one *this* backend belongs to. Asking only
    // "does the request carry some product's identity" let Claude Code unlock
    // the ChatGPT subscription the moment a translator existed between the two
    // wires — `docs/TRUST.md` I5, in the one direction it forbids.
    if peek.client_identity != Some(required) {
        return Err(Unusable::NeedsClientIdentity);
    }
    // One request, one login. Claude Code's body names the account it is
    // logged in as, so the same bytes on another account's token are a
    // request the provider can see was moved — and moving after a 429 is the
    // plainest reading of both providers' rate-limit terms. Serving a client
    // from another of the user's logins is therefore the user's explicit
    // choice, never a default (`docs/TRUST.md`).
    if !cross_account_fallback && !is_own_login(candidate, peek) {
        return Err(Unusable::AnotherLogin(required));
    }
    Ok(())
}

/// Whether `candidate` is the login this request's client is using. A request
/// at the plain façade belongs to each product's default login.
fn is_own_login(candidate: &Candidate, peek: &RequestPeek) -> bool {
    match &peek.client_login {
        Some(login) => candidate.id == *login,
        None => candidate.default_login,
    }
}

/// Native encrypted reasoning requires a recorded and currently observed
/// credential scope; missing history or a relogin cannot authorize a guess.
fn requirements_for(
    candidate: &Candidate,
    peek: &RequestPeek,
    origin: Option<&ReasoningAccount>,
) -> RequestRequirements {
    let mut requirements = peek.requirements.clone();
    requirements.reasoning_backend = origin
        .filter(|origin| {
            origin.backend == candidate.id
                && origin.account_scope.is_some()
                && origin.account_scope == candidate.account_scope
        })
        .map(|origin| origin.backend.to_string());
    requirements
}

/// Drop the entries unused for [`AFFINITY_TTL`], then the stalest beyond
/// [`MAX_AFFINITIES`].
fn bound<K: Clone + Eq + std::hash::Hash, V>(
    map: &mut HashMap<K, V>,
    now: DateTime<Utc>,
    capacity: usize,
    last_seen: impl Fn(&V) -> DateTime<Utc>,
) {
    map.retain(|_, value| now - last_seen(value) < AFFINITY_TTL);

    // Oldest use first, so a busy conversation outlives an idle one even
    // when both are inside the TTL.
    while map.len() > capacity {
        let Some(stalest) = map
            .iter()
            .min_by_key(|(_, value)| last_seen(value))
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        map.remove(&stalest);
    }
}

/// Best model this candidate offers at or below `tier`, preferring an exact
/// match. `None` when the backend advertises no models (forward as-is).
fn pick_model(candidate: &Candidate, tier: ModelTier) -> Option<String> {
    candidate
        .models
        .iter()
        .find(|(_, t)| *t == tier)
        .or_else(|| {
            candidate
                .models
                .iter()
                .filter(|(_, t)| *t < tier)
                .max_by_key(|(_, t)| *t)
        })
        .map(|(m, _)| m.clone())
}

/// Whether this backend can serve the requested tier without descending.
///
/// Distinct from [`pick_model`], which deliberately falls back to a lesser model
/// rather than refusing. Both behaviours are wanted, at different moments:
/// falling back keeps an ordinary turn working, and refusing to fall back is
/// what a compaction turn needs (`docs/PROTOCOL.md` §8).
fn serves_tier(candidate: &Candidate, tier: ModelTier) -> bool {
    candidate.models.iter().any(|(_, t)| *t >= tier)
}

fn kind_label(kind: BackendKind) -> &'static str {
    match kind {
        BackendKind::Subscription => "subscription",
        BackendKind::ApiKey => "metered",
        BackendKind::Credits => "credit",
        BackendKind::Local => "local",
    }
}

/// Convenience: requirements that demand nothing.
#[must_use]
pub fn trivial_requirements() -> RequestRequirements {
    RequestRequirements::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::ReasoningNeed;
    use crate::protocol::Wires;
    use crate::quota::Headroom;

    fn t(offset: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + offset, 0).expect("valid timestamp")
    }

    fn caps(protocol: Protocol) -> Capabilities {
        Capabilities {
            wires: Wires::only(protocol),
            tools: true,
            parallel_tool_calls: true,
            images: true,
            reasoning: true,
            prompt_cache: true,
            structured_output: true,
            context_tokens: 200_000,
            unsupported_responses_tools: Vec::new(),
        }
    }

    fn candidate(id: &str, kind: BackendKind, protocol: Protocol) -> Candidate {
        Candidate {
            id: BackendId::from(id),
            kind,
            caps: caps(protocol),
            quota: QuotaSnapshot::default(),
            healthy: true,
            account_scope: Some(format!("scope-{id}")),
            consented: true,
            required_client_identity: None,
            // The two ids production gives the default logins, so every test
            // written before additional logins existed still means what it
            // meant.
            default_login: matches!(id, "claude-sub" | "codex-sub"),
            models: vec![
                ("claude-opus-4-6".to_string(), ModelTier::Frontier),
                ("claude-sonnet-4-6".to_string(), ModelTier::Balanced),
            ],
            catalogue_from_provider: true,
            trusted: true,
        }
    }

    fn peek(model: &str) -> RequestPeek {
        RequestPeek {
            requested_model: Some(model.to_string()),
            stream: true,
            requirements: RequestRequirements::default(),
            client_identity: Some(ClientIdentity::ClaudeCode),
            message_count: 3,
            likely_compaction: false,
            client_login: None,
            client_session: None,
        }
    }

    #[test]
    fn client_sessions_are_stable_and_isolated_by_protocol_and_login() {
        let protocol = Protocol::OpenAiResponses;
        let personal = BackendId::from("personal");
        let work = BackendId::from("work");
        let key = ConversationKey::for_session(protocol, "session", Some(&personal));
        assert_eq!(
            key,
            ConversationKey::for_session(protocol, "session", Some(&personal))
        );
        assert_ne!(
            key,
            ConversationKey::for_session(protocol, "another-session", Some(&personal))
        );
        assert_ne!(
            key,
            ConversationKey::for_session(protocol, "session", Some(&work))
        );
        assert_ne!(
            key,
            ConversationKey::for_session(Protocol::AnthropicMessages, "session", Some(&personal))
        );
    }

    fn key() -> ConversationKey {
        ConversationKey::derive(
            Protocol::AnthropicMessages,
            "You are Claude Code",
            &["Read"],
        )
    }

    /// The test the local-backend feature exists under, written before it.
    ///
    /// `BackendKind::Local` has `marginal_cost_rank() == 0` — cheaper than a
    /// subscription — so the *only* thing keeping a 30B model on someone's
    /// laptop from taking every frontier request is the tier its slugs carry.
    /// `from_model_hint` resolves anything it does not recognise to `Frontier`,
    /// which is right for a hosted catalogue and catastrophic for a local one.
    /// Local models are therefore `Fast` unless the user says otherwise, and
    /// this asserts the consequence rather than the mechanism.
    #[test]
    fn a_frontier_request_does_not_land_on_a_local_model() {
        let mut local = candidate("ollama", BackendKind::Local, Protocol::AnthropicMessages);
        local.models = vec![("qwen3-coder:30b".to_string(), ModelTier::Fast)];
        let subscription = candidate(
            "claude-sub",
            BackendKind::Subscription,
            Protocol::AnthropicMessages,
        );

        let mut policy = Policy::new();
        let decision = policy
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek("claude-opus-4-6"),
                &[local, subscription],
                t(0),
            )
            .expect("routes");
        assert_eq!(
            decision.backend.as_str(),
            "claude-sub",
            "a frontier request went to a local model because it sorted cheapest"
        );
    }

    /// The other half: a local backend is not decoration. Asked for something
    /// it can serve, it wins on cost exactly as intended.
    #[test]
    fn a_fast_request_does_land_on_a_local_model() {
        let mut local = candidate("ollama", BackendKind::Local, Protocol::AnthropicMessages);
        local.models = vec![("qwen3-coder:30b".to_string(), ModelTier::Fast)];
        let mut subscription = candidate(
            "claude-sub",
            BackendKind::Subscription,
            Protocol::AnthropicMessages,
        );
        subscription.models = vec![("claude-haiku-4-5".to_string(), ModelTier::Fast)];

        let mut policy = Policy::new();
        let decision = policy
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek("claude-haiku-4-5"),
                &[local, subscription],
                t(0),
            )
            .expect("routes");
        assert_eq!(decision.backend.as_str(), "ollama");
    }

    /// Descent without promotion is a trap rather than a transition: a
    /// conversation that fell to a cross-family backend in the morning stays
    /// there all day, cold cache and all, while the capacity the user pays for
    /// sits idle.
    mod promotion {
        use super::*;

        /// The preferred backend, and the foreign-family one below it.
        ///
        /// Exhausted rather than merely pressured: a same-wire backend that is
        /// *usable* always wins its partition, so a cross-family descent only
        /// happens once the preferred one is genuinely unavailable. Pressure
        /// alone moves a conversation down a model tier, not across a family.
        fn ladder(exhausted_until: Option<DateTime<Utc>>) -> Vec<Candidate> {
            let mut preferred = candidate(
                "claude-sub",
                BackendKind::Subscription,
                Protocol::AnthropicMessages,
            );
            if let Some(until) = exhausted_until {
                preferred.quota.primary = Headroom::Exhausted { until };
            }
            let fallback = candidate("nearai", BackendKind::Credits, Protocol::OpenAiChat);
            vec![preferred, fallback]
        }

        fn route(
            policy: &mut Policy,
            peek: &RequestPeek,
            candidates: &[Candidate],
            at: DateTime<Utc>,
        ) -> RouteDecision {
            policy
                .decide(key(), Protocol::AnthropicMessages, peek, candidates, at)
                .expect("routes")
        }

        /// A conversation stranded on the foreign family, as a morning rate
        /// limit would leave it.
        fn stranded() -> (Policy, DateTime<Utc>) {
            let mut policy = Policy::new();
            let recovers_at = t(600);
            let decision = route(
                &mut policy,
                &peek("claude-opus-4-6"),
                &ladder(Some(recovers_at)),
                t(0),
            );
            assert_eq!(decision.backend.as_str(), "nearai");
            assert_eq!(decision.rung, Rung::CrossFamily);
            assert!(decision.translated);
            (policy, recovers_at)
        }

        #[test]
        fn a_recovered_backend_takes_the_conversation_back() {
            let (mut policy, recovers_at) = stranded();
            let ordinary = peek("claude-opus-4-6");
            // The first turn after recovery starts the clock; it does not move.
            route(&mut policy, &ordinary, &ladder(None), recovers_at);
            let decision = route(
                &mut policy,
                &ordinary,
                &ladder(None),
                recovers_at + PROMOTION_DEBOUNCE,
            );
            assert_eq!(decision.backend.as_str(), "claude-sub");
            assert_eq!(decision.rung, Rung::Preferred);
            assert!(
                !decision.translated,
                "a promoted route must not still be marked translated"
            );
        }

        #[test]
        fn it_does_not_climb_back_the_moment_pressure_lifts() {
            let (mut policy, recovers_at) = stranded();
            let ordinary = peek("claude-opus-4-6");
            route(&mut policy, &ordinary, &ladder(None), recovers_at);
            let decision = route(
                &mut policy,
                &ordinary,
                &ladder(None),
                recovers_at + Duration::seconds(30),
            );
            assert_eq!(
                decision.backend.as_str(),
                "nearai",
                "promoted before the debounce elapsed"
            );
        }

        /// The debounce is the only damping there is — `is_pressured` is a step
        /// function at 90% — so a backend crossing the line repeatedly must not
        /// accumulate its way to a promotion one good turn at a time.
        #[test]
        fn a_flapping_backend_never_promotes() {
            let (mut policy, recovers_at) = stranded();
            let ordinary = peek("claude-opus-4-6");
            let mut at = recovers_at;
            for _ in 0..10 {
                // Available: recovery starts accruing.
                route(&mut policy, &ordinary, &ladder(None), at);
                at += Duration::minutes(2);
                // Gone again before the debounce elapsed: it starts over.
                route(
                    &mut policy,
                    &ordinary,
                    &ladder(Some(at + Duration::hours(1))),
                    at,
                );
                at += Duration::minutes(2);
            }
            assert_eq!(
                policy.current_backend(&key()).map(|b| b.to_string()),
                Some("nearai".to_string()),
                "a flapping backend promoted a conversation"
            );
        }

        /// Leaving the foreign family is the same hazard as entering it: the
        /// recent assistant turns carry none of this family's signed reasoning
        /// state, so replaying them mid-loop is the rejection risk the gate
        /// exists to prevent (`docs/PROTOCOL.md` §6).
        #[test]
        fn a_cross_family_promotion_waits_for_a_turn_boundary() {
            let (mut policy, recovers_at) = stranded();
            let mut mid_loop = peek("claude-opus-4-6");
            mid_loop.requirements.mid_tool_loop = true;

            route(&mut policy, &mid_loop, &ladder(None), recovers_at);
            let blocked = route(
                &mut policy,
                &mid_loop,
                &ladder(None),
                recovers_at + PROMOTION_DEBOUNCE,
            );
            assert_eq!(
                blocked.backend.as_str(),
                "nearai",
                "promoted across families mid tool loop"
            );

            // The next clean turn promotes immediately: being blocked is not
            // being reset, or a busy tool loop would postpone recovery forever.
            let promoted = route(
                &mut policy,
                &peek("claude-opus-4-6"),
                &ladder(None),
                recovers_at + PROMOTION_DEBOUNCE + Duration::seconds(1),
            );
            assert_eq!(promoted.backend.as_str(), "claude-sub");
        }

        #[test]
        fn a_conversation_at_the_top_stays_where_it_is() {
            let mut policy = Policy::new();
            let ordinary = peek("claude-opus-4-6");
            let first = route(&mut policy, &ordinary, &ladder(None), t(0));
            assert_eq!(first.rung, Rung::Preferred);
            let later = route(
                &mut policy,
                &ordinary,
                &ladder(None),
                t(0) + PROMOTION_DEBOUNCE * 10,
            );
            assert_eq!(later.reason, "sticky affinity");
            assert_eq!(later.backend.as_str(), "claude-sub");
        }
    }

    /// Rung 2 was declared, documented in the ladder, and never produced — so
    /// a fall from a subscription to a metered key looked undegraded, and the
    /// promotion that should carry it back never fired.
    mod alternate_credential {
        use super::*;

        fn subscription_and_key(subscription_exhausted: bool) -> Vec<Candidate> {
            let mut subscription = candidate(
                "claude-sub",
                BackendKind::Subscription,
                Protocol::AnthropicMessages,
            );
            if subscription_exhausted {
                subscription.quota.primary = Headroom::Exhausted { until: t(600) };
            }
            let key = candidate(
                "anthropic-key",
                BackendKind::ApiKey,
                Protocol::AnthropicMessages,
            );
            vec![subscription, key]
        }

        #[test]
        fn falling_to_a_metered_key_is_recorded_as_a_credential_change() {
            let mut policy = Policy::new();
            let decision = policy
                .decide(
                    key(),
                    Protocol::AnthropicMessages,
                    &peek("claude-opus-4-6"),
                    &subscription_and_key(true),
                    t(0),
                )
                .expect("routes");
            assert_eq!(decision.backend.as_str(), "anthropic-key");
            assert_eq!(
                decision.rung,
                Rung::AlternateCredential,
                "a different credential on the same wire is rung 2, not rung 0"
            );
        }

        /// The consequence that costs money: without a rung to compare, the
        /// conversation stays on the paid key after the subscription resets.
        #[test]
        fn the_conversation_returns_to_the_subscription_when_it_recovers() {
            let mut policy = Policy::new();
            policy
                .decide(
                    key(),
                    Protocol::AnthropicMessages,
                    &peek("claude-opus-4-6"),
                    &subscription_and_key(true),
                    t(0),
                )
                .expect("routes");

            let recovered = subscription_and_key(false);
            let ordinary = peek("claude-opus-4-6");
            policy
                .decide(
                    key(),
                    Protocol::AnthropicMessages,
                    &ordinary,
                    &recovered,
                    t(700),
                )
                .expect("routes");
            let decision = policy
                .decide(
                    key(),
                    Protocol::AnthropicMessages,
                    &ordinary,
                    &recovered,
                    t(700) + PROMOTION_DEBOUNCE,
                )
                .expect("routes");
            assert_eq!(
                decision.backend.as_str(),
                "claude-sub",
                "the conversation stayed on the metered key after the \
                 subscription came back"
            );
        }

        /// Both dimensions at once reports the worse rung, not whichever check
        /// happened to run last.
        #[test]
        fn a_cheaper_credential_and_a_smaller_model_reports_the_worse_rung() {
            let mut candidates = subscription_and_key(true);
            candidates[1].models = vec![("claude-sonnet-4-6".to_string(), ModelTier::Balanced)];
            let mut policy = Policy::new();
            let decision = policy
                .decide(
                    key(),
                    Protocol::AnthropicMessages,
                    &peek("claude-opus-4-6"),
                    &candidates,
                    t(0),
                )
                .expect("routes");
            assert_eq!(decision.rung, Rung::AlternateCredential);
        }

        /// Two API keys rank equal on cost, so only registration order says
        /// which the user preferred — and a fallback between them is still a
        /// different credential.
        #[test]
        fn falling_between_equal_cost_credentials_is_still_rung_two() {
            let mut first = candidate("primary", BackendKind::ApiKey, Protocol::AnthropicMessages);
            first.quota.primary = Headroom::Exhausted { until: t(600) };
            let second = candidate("backup", BackendKind::ApiKey, Protocol::AnthropicMessages);

            let mut policy = Policy::new();
            let decision = policy
                .decide(
                    key(),
                    Protocol::AnthropicMessages,
                    &peek("claude-opus-4-6"),
                    &[first, second],
                    t(0),
                )
                .expect("routes");
            assert_eq!(decision.backend.as_str(), "backup");
            assert_eq!(decision.rung, Rung::AlternateCredential);
        }

        /// And the first-choice backend, when it is available, is rung 0 — the
        /// rule must not mark every route degraded merely for having a
        /// second backend registered.
        #[test]
        fn the_first_choice_backend_is_still_rung_zero() {
            let mut policy = Policy::new();
            let decision = policy
                .decide(
                    key(),
                    Protocol::AnthropicMessages,
                    &peek("claude-opus-4-6"),
                    &[
                        candidate("primary", BackendKind::ApiKey, Protocol::AnthropicMessages),
                        candidate("backup", BackendKind::ApiKey, Protocol::AnthropicMessages),
                    ],
                    t(0),
                )
                .expect("routes");
            assert_eq!(decision.backend.as_str(), "primary");
            assert_eq!(decision.rung, Rung::Preferred);
        }

        /// A backend that could never have served the request was not passed
        /// over. Counting one made every route rung 2 for good — and promotion
        /// then ran `select` on every turn, forever, for nothing.
        #[test]
        fn a_backend_that_could_not_have_served_the_request_does_not_make_the_route_rung_two() {
            // An unconsented subscription ahead of a metered key.
            let mut unconsented = candidate(
                "claude-sub",
                BackendKind::Subscription,
                Protocol::AnthropicMessages,
            );
            unconsented.consented = false;
            let key_backend = candidate(
                "anthropic-key",
                BackendKind::ApiKey,
                Protocol::AnthropicMessages,
            );
            let decision = Policy::new()
                .decide(
                    key(),
                    Protocol::AnthropicMessages,
                    &peek("claude-opus-4-6"),
                    &[unconsented, key_backend.clone()],
                    t(0),
                )
                .expect("routes");
            assert_eq!(decision.backend.as_str(), "anthropic-key");
            assert_eq!(decision.rung, Rung::Preferred, "an unconsented backend");

            // The Claude subscription under an Aider request.
            let mut claude_code_only = candidate(
                "claude-sub",
                BackendKind::Subscription,
                Protocol::AnthropicMessages,
            );
            claude_code_only.required_client_identity = Some(ClientIdentity::ClaudeCode);
            let mut aider = peek("claude-opus-4-6");
            aider.client_identity = None;
            let decision = Policy::new()
                .decide(
                    key(),
                    Protocol::AnthropicMessages,
                    &aider,
                    &[claude_code_only, key_backend.clone()],
                    t(0),
                )
                .expect("routes");
            assert_eq!(decision.backend.as_str(), "anthropic-key");
            assert_eq!(decision.rung, Rung::Preferred, "a subscription for Aider");

            // Both at once: it is still not a login of this client's.
            let mut neither = candidate(
                "claude-sub",
                BackendKind::Subscription,
                Protocol::AnthropicMessages,
            );
            neither.required_client_identity = Some(ClientIdentity::ClaudeCode);
            neither.consented = false;
            let decision = Policy::new()
                .decide(
                    key(),
                    Protocol::AnthropicMessages,
                    &aider,
                    &[neither, key_backend.clone()],
                    t(0),
                )
                .expect("routes");
            assert_eq!(decision.backend.as_str(), "anthropic-key");
            assert_eq!(
                decision.rung,
                Rung::Preferred,
                "an unconsented subscription for Aider"
            );

            // And one the request needs something from that it cannot give.
            let mut text_only = candidate(
                "claude-sub",
                BackendKind::Subscription,
                Protocol::AnthropicMessages,
            );
            text_only.caps.images = false;
            let mut with_image = peek("claude-opus-4-6");
            with_image.requirements.images = true;
            let decision = Policy::new()
                .decide(
                    key(),
                    Protocol::AnthropicMessages,
                    &with_image,
                    &[text_only, key_backend],
                    t(0),
                )
                .expect("routes");
            assert_eq!(decision.rung, Rung::Preferred, "an ineligible backend");
        }

        /// The fast path for a model newer than this build skipped the rung
        /// calculation entirely, so a fallback taken there was recorded as
        /// `Preferred` — invisible, and never promoted back.
        #[test]
        fn the_unrecognised_model_path_reports_rung_two_after_passing_over_a_preferred_backend() {
            let guessed = |mut c: Candidate| {
                c.catalogue_from_provider = false;
                c
            };
            let newer = peek("claude-opus-5");

            let decision = Policy::new()
                .decide(
                    key(),
                    Protocol::AnthropicMessages,
                    &newer,
                    &subscription_and_key(true)
                        .into_iter()
                        .map(guessed)
                        .collect::<Vec<_>>(),
                    t(0),
                )
                .expect("routes");
            assert_eq!(decision.backend.as_str(), "anthropic-key");
            assert_eq!(decision.model, None, "still the client's own model string");
            assert_eq!(decision.rung, Rung::AlternateCredential);

            // The first choice, available, is still rung 0 on the same path.
            let decision = Policy::new()
                .decide(
                    key(),
                    Protocol::AnthropicMessages,
                    &newer,
                    &subscription_and_key(false)
                        .into_iter()
                        .map(guessed)
                        .collect::<Vec<_>>(),
                    t(0),
                )
                .expect("routes");
            assert_eq!(decision.backend.as_str(), "claude-sub");
            assert_eq!(decision.model, None);
            assert_eq!(decision.rung, Rung::Preferred);
        }

        /// The routing consequence of a reading outliving its window. Nothing
        /// refreshes the subscription's 95% once traffic has left it, so if
        /// that reading still counted it would lose to the metered key long
        /// after the provider said it would reset.
        #[test]
        fn a_subscription_whose_window_has_reset_is_chosen_over_a_metered_key_again() {
            let mut candidates = subscription_and_key(false);
            candidates[0].quota.primary = Headroom::Observed {
                used_pct: 95.0,
                resets_at: Some(t(600)),
                observed_at: t(0),
            };
            let route_at = |at| {
                Policy::new()
                    .decide(
                        key(),
                        Protocol::AnthropicMessages,
                        &peek("claude-opus-4-6"),
                        &candidates,
                        at,
                    )
                    .expect("routes")
            };
            assert_eq!(route_at(t(1)).backend.as_str(), "anthropic-key");
            let after_reset = route_at(t(601));
            assert_eq!(after_reset.backend.as_str(), "claude-sub");
            assert_eq!(after_reset.rung, Rung::Preferred);
        }

        /// Unchanged: descending a model tier on the *same* backend is rung 1.
        #[test]
        fn a_smaller_model_on_the_same_backend_is_still_rung_one() {
            let mut only = vec![candidate(
                "claude-sub",
                BackendKind::Subscription,
                Protocol::AnthropicMessages,
            )];
            only[0].models = vec![("claude-sonnet-4-6".to_string(), ModelTier::Balanced)];
            let mut policy = Policy::new();
            let decision = policy
                .decide(
                    key(),
                    Protocol::AnthropicMessages,
                    &peek("claude-opus-4-6"),
                    &only,
                    t(0),
                )
                .expect("routes");
            assert_eq!(decision.rung, Rung::SmallerModel);
        }
    }

    /// `privacy.mode = "full"` names the destinations that may receive a
    /// request, and nothing may route around it.
    mod full_privacy {
        use super::*;

        fn ladder() -> Vec<Candidate> {
            let mut trusted =
                candidate("nearai", BackendKind::Credits, Protocol::AnthropicMessages);
            trusted.trusted = true;
            let mut untrusted = candidate(
                "claude-sub",
                BackendKind::Subscription,
                Protocol::AnthropicMessages,
            );
            untrusted.trusted = false;
            // Untrusted first, so a test cannot pass by accident of ordering.
            vec![untrusted, trusted]
        }

        #[test]
        fn only_a_trusted_backend_is_routed_to() {
            let mut policy = Policy::new();
            let decision = policy
                .decide(
                    key(),
                    Protocol::AnthropicMessages,
                    &peek("claude-opus-4-6"),
                    &ladder(),
                    t(0),
                )
                .expect("routes");
            assert_eq!(
                decision.backend.as_str(),
                "nearai",
                "routed to a backend the user excluded"
            );
        }

        /// The refusal is the feature. Everywhere else IronWire keeps the agent
        /// alive; here the user said not sending the data matters more.
        #[test]
        fn nothing_trusted_means_refused_not_redirected() {
            let mut candidates = ladder();
            candidates.retain(|c| !c.trusted);
            let mut policy = Policy::new();
            let error = policy
                .decide(
                    key(),
                    Protocol::AnthropicMessages,
                    &peek("claude-opus-4-6"),
                    &candidates,
                    t(0),
                )
                .expect_err("must refuse");
            assert!(
                matches!(error, NoRoute::NoTrustedBackendAvailable { .. }),
                "got {error:?}"
            );
        }

        /// A header a client can set must not relax a constraint the user put
        /// in their own config, or the setting is decorative.
        #[test]
        fn a_pin_cannot_reach_an_untrusted_backend() {
            let mut policy = Policy::new();
            let error = policy
                .decide_with_override(
                    key(),
                    Protocol::AnthropicMessages,
                    &peek("claude-opus-4-6"),
                    &ladder(),
                    t(0),
                    Some((BackendId::from("claude-sub"), None)),
                )
                .expect_err("must refuse");
            assert!(
                matches!(error, NoRoute::NoTrustedBackendAvailable { .. }),
                "a pin routed around the privacy mode: {error:?}"
            );
        }

        /// The silent-leak case: an affinity established before the config
        /// changed must not survive it. The sticky path runs `usable`, and this
        /// pins that it does.
        #[test]
        fn an_existing_affinity_to_a_now_untrusted_backend_is_dropped() {
            let mut policy = Policy::new();
            // First, with everything trusted, settle on claude-sub.
            let mut open = ladder();
            for candidate in &mut open {
                candidate.trusted = true;
            }
            let first = policy
                .decide(
                    key(),
                    Protocol::AnthropicMessages,
                    &peek("claude-opus-4-6"),
                    &open,
                    t(0),
                )
                .expect("routes");
            assert_eq!(first.backend.as_str(), "claude-sub");

            // The user then restricts the trusted set. The very next request
            // must leave, not continue on the affinity.
            let decision = policy
                .decide(
                    key(),
                    Protocol::AnthropicMessages,
                    &peek("claude-opus-4-6"),
                    &ladder(),
                    t(60),
                )
                .expect("routes");
            assert_eq!(
                decision.backend.as_str(),
                "nearai",
                "a sticky affinity kept sending to a backend the user excluded"
            );
        }
    }

    /// Several logins to one product, each its own backend. By default a client
    /// is served by its own; another login is the user's explicit opt-in, and
    /// even then a Codex conversation's encrypted reasoning never changes
    /// account.
    mod logins {
        #[test]
        fn hundreds_of_new_sessions_do_not_move_an_existing_sessions_reasoning() {
            let mut policy = with_fallback();
            let own = codex_login("codex-sub", true);
            let sibling = codex_login("codex-work", false);
            let mut request = codex(None, false);
            request.client_session = Some("long-running-session".into());
            let first = codex_route(
                &mut policy,
                &request,
                &[exhausted(own.clone(), t(600)), sibling.clone()],
                t(0),
            )
            .expect("the first turn may start on a sibling");
            assert_eq!(first.backend, sibling.id);
            for index in 0..1024 {
                let mut new_session = codex(None, false);
                new_session.client_session = Some(format!("new-{index}"));
                codex_route(
                    &mut policy,
                    &new_session,
                    std::slice::from_ref(&own),
                    t(1 + index),
                )
                .expect("new own-account session");
            }
            request.requirements.encrypted_reasoning = true;
            let replay = codex_route(&mut policy, &request, &[own, sibling.clone()], t(1100))
                .expect("the original account is still available");
            assert_eq!(
                replay.backend, sibling.id,
                "session churn must not forget the minting account"
            );
        }

        #[test]
        fn relogging_the_same_backend_cannot_replay_the_old_accounts_reasoning() {
            let mut policy = with_fallback();
            let original = codex_login("codex-sub", true);
            codex_route(
                &mut policy,
                &codex(None, false),
                std::slice::from_ref(&original),
                t(0),
            )
            .expect("first turn");
            let mut relogged = original.clone();
            relogged.account_scope = Some("another-account".into());
            assert!(codex_route(&mut policy, &codex(None, true), &[relogged], t(1)).is_err());
            codex_route(&mut policy, &codex(None, true), &[original], t(2))
                .expect("restored original account");
        }

        #[test]
        fn an_unknown_scope_never_authorizes_a_native_encrypted_replay() {
            let mut policy = with_fallback();
            let mut unknown = codex_login("codex-sub", true);
            unknown.account_scope = None;
            codex_route(
                &mut policy,
                &codex(None, false),
                std::slice::from_ref(&unknown),
                t(0),
            )
            .expect("plain first turn");
            assert!(codex_route(&mut policy, &codex(None, true), &[unknown], t(1)).is_err());
        }

        #[test]
        fn an_expired_origin_refuses_a_replay_even_when_the_same_account_is_pinned() {
            let mut policy = with_fallback();
            let own = codex_login("codex-sub", true);
            codex_route(
                &mut policy,
                &codex(None, false),
                std::slice::from_ref(&own),
                t(0),
            )
            .expect("first turn");
            policy.set_pin(Some(own.id.clone()), None);
            assert!(codex_route(&mut policy, &codex(None, true), &[own], t(86401)).is_err());
            assert!(policy.reasoning_accounts.is_empty());
        }

        #[test]
        fn evicting_an_origin_at_capacity_refuses_instead_of_guessing_an_account() {
            let mut policy = with_fallback();
            let own = codex_login("codex-sub", true);
            let mut original = codex(None, false);
            original.client_session = Some("old-session".into());
            codex_route(&mut policy, &original, std::slice::from_ref(&own), t(0))
                .expect("first turn");
            for index in 0..MAX_REASONING_ACCOUNTS {
                let mut request = codex(None, false);
                request.client_session = Some(format!("session-{index}"));
                codex_route(
                    &mut policy,
                    &request,
                    std::slice::from_ref(&own),
                    t(1 + index as i64),
                )
                .expect("new session");
            }
            assert_eq!(policy.reasoning_accounts.len(), MAX_REASONING_ACCOUNTS);
            original.requirements.encrypted_reasoning = true;
            assert!(codex_route(&mut policy, &original, &[own], t(5000)).is_err());
        }

        #[test]
        fn a_route_override_cannot_move_another_accounts_encrypted_reasoning() {
            let mut policy = with_fallback();
            let both = [
                codex_login("codex-sub", true),
                codex_login("codex-work", false),
            ];
            codex_route(&mut policy, &codex(None, false), &both, t(0)).expect("first turn");
            let result = policy.decide_with_override(
                codex_key(),
                Protocol::OpenAiResponses,
                &codex(None, true),
                &both,
                t(1),
                Some((BackendId::from("codex-work"), None)),
            );
            assert!(
                result.is_err(),
                "an override must preserve the same eligibility rules"
            );
        }

        use super::*;

        /// A Claude subscription login. `default` is the one Claude Code uses
        /// with no `CLAUDE_CONFIG_DIR`.
        fn claude_login(id: &str, default: bool) -> Candidate {
            let mut login = candidate(id, BackendKind::Subscription, Protocol::AnthropicMessages);
            login.required_client_identity = Some(ClientIdentity::ClaudeCode);
            login.default_login = default;
            login
        }

        /// A ChatGPT subscription login, for Codex.
        fn codex_login(id: &str, default: bool) -> Candidate {
            let mut login = candidate(id, BackendKind::Subscription, Protocol::OpenAiResponses);
            login.required_client_identity = Some(ClientIdentity::Codex);
            login.default_login = default;
            login.models = vec![("gpt-5".to_string(), ModelTier::Frontier)];
            login
        }

        fn anthropic_key() -> Candidate {
            candidate(
                "anthropic-key",
                BackendKind::ApiKey,
                Protocol::AnthropicMessages,
            )
        }

        fn openai_key() -> Candidate {
            let mut key = candidate("openai-key", BackendKind::ApiKey, Protocol::OpenAiResponses);
            key.models = vec![("gpt-5".to_string(), ModelTier::Frontier)];
            key
        }

        /// NEAR AI as every daemon registers it unless the user disabled it:
        /// present without a key, and so unhealthy to the router.
        fn nearai_without_a_key() -> Candidate {
            let mut nearai = candidate("nearai", BackendKind::Credits, Protocol::OpenAiChat);
            nearai.healthy = false;
            nearai
        }

        fn exhausted(mut backend: Candidate, until: DateTime<Utc>) -> Candidate {
            backend.quota.primary = Headroom::Exhausted { until };
            backend
        }

        /// Claude Code, pointed at a login-bound URL or at the plain façade.
        fn claude_code(login: Option<&str>) -> RequestPeek {
            let mut request = peek("claude-opus-4-6");
            request.client_login = login.map(BackendId::from);
            request
        }

        /// A Codex turn. `replayed` is whether it carries encrypted reasoning,
        /// which every turn after a conversation's first does.
        fn codex(login: Option<&str>, replayed: bool) -> RequestPeek {
            let mut request = peek("gpt-5");
            request.client_identity = Some(ClientIdentity::Codex);
            request.client_login = login.map(BackendId::from);
            request.requirements.encrypted_reasoning = replayed;
            if replayed {
                request.requirements.reasoning = ReasoningNeed::LoadBearing;
            }
            request
        }

        fn codex_key() -> ConversationKey {
            ConversationKey::derive(Protocol::OpenAiResponses, "You are Codex", &["shell"])
        }

        fn with_fallback() -> Policy {
            let mut policy = Policy::new();
            policy.set_cross_account_fallback(true);
            policy
        }

        fn claude(
            policy: &mut Policy,
            request: &RequestPeek,
            candidates: &[Candidate],
        ) -> Result<RouteDecision, NoRoute> {
            policy.decide(
                key(),
                Protocol::AnthropicMessages,
                request,
                candidates,
                t(0),
            )
        }

        fn codex_route(
            policy: &mut Policy,
            request: &RequestPeek,
            candidates: &[Candidate],
            at: DateTime<Utc>,
        ) -> Result<RouteDecision, NoRoute> {
            policy.decide(
                codex_key(),
                Protocol::OpenAiResponses,
                request,
                candidates,
                at,
            )
        }

        fn refusals_by_id(error: NoRoute) -> HashMap<String, Refusal> {
            match error {
                NoRoute::NoneUsable { refusals } => refusals
                    .into_iter()
                    .map(|(id, why)| (id.to_string(), why))
                    .collect(),
                other => panic!("expected a refusal list, got {other:?}"),
            }
        }

        #[test]
        fn by_default_an_exhausted_own_login_descends_to_the_metered_key_not_to_the_sibling() {
            let candidates = [
                claude_login("claude-sub", true),
                claude_login("claude-work", false),
                anthropic_key(),
            ];
            let decision =
                claude(&mut Policy::new(), &claude_code(None), &candidates).expect("routes");
            assert_eq!(decision.backend.as_str(), "claude-sub", "its own login");
            assert_eq!(decision.rung, Rung::Preferred);

            let candidates = [
                exhausted(claude_login("claude-sub", true), t(600)),
                claude_login("claude-work", false),
                anthropic_key(),
            ];
            let decision =
                claude(&mut Policy::new(), &claude_code(None), &candidates).expect("routes");
            assert_eq!(
                decision.backend.as_str(),
                "anthropic-key",
                "fell onto another of the user's logins without the opt-in"
            );
            assert_eq!(decision.rung, Rung::AlternateCredential);

            // The same from the other side: a client on the additional login
            // does not fall onto the default one either.
            let candidates = [
                claude_login("claude-sub", true),
                exhausted(claude_login("claude-work", false), t(600)),
                anthropic_key(),
            ];
            let decision = claude(
                &mut Policy::new(),
                &claude_code(Some("claude-work")),
                &candidates,
            )
            .expect("routes");
            assert_eq!(decision.backend.as_str(), "anthropic-key");
            assert_eq!(decision.rung, Rung::AlternateCredential);
        }

        /// Registration order is preference among equals, and it must not
        /// outrank whose login a client is using.
        #[test]
        fn a_client_bound_to_the_additional_login_is_served_by_it_though_the_default_is_registered_first()
         {
            let candidates = [
                claude_login("claude-sub", true),
                claude_login("claude-work", false),
                anthropic_key(),
            ];
            let decision = claude(
                &mut Policy::new(),
                &claude_code(Some("claude-work")),
                &candidates,
            )
            .expect("routes");
            assert_eq!(decision.backend.as_str(), "claude-work");
            assert_eq!(
                decision.rung,
                Rung::Preferred,
                "another client's login was counted as a credential passed over"
            );
        }

        #[test]
        fn with_cross_account_fallback_an_exhausted_own_login_falls_to_the_sibling_at_rung_two() {
            let candidates = [
                exhausted(claude_login("claude-sub", true), t(600)),
                claude_login("claude-work", false),
                anthropic_key(),
            ];
            let decision =
                claude(&mut with_fallback(), &claude_code(None), &candidates).expect("routes");
            assert_eq!(
                decision.backend.as_str(),
                "claude-work",
                "a subscription the user opted into is cheaper than the key"
            );
            assert_eq!(decision.rung, Rung::AlternateCredential);
        }

        /// With the opt-in, the default login is not "the" preferred one: the
        /// client's own login is, wherever it was registered.
        #[test]
        fn with_cross_account_fallback_a_client_bound_to_the_sibling_still_prefers_it() {
            let both = [
                claude_login("claude-sub", true),
                claude_login("claude-work", false),
            ];
            let decision = claude(
                &mut with_fallback(),
                &claude_code(Some("claude-work")),
                &both,
            )
            .expect("routes");
            assert_eq!(decision.backend.as_str(), "claude-work");
            assert_eq!(decision.rung, Rung::Preferred);

            // And when its own runs out, falling to the default login is the
            // fallback, recorded as one.
            let own_exhausted = [
                claude_login("claude-sub", true),
                exhausted(claude_login("claude-work", false), t(600)),
            ];
            let decision = claude(
                &mut with_fallback(),
                &claude_code(Some("claude-work")),
                &own_exhausted,
            )
            .expect("routes");
            assert_eq!(decision.backend.as_str(), "claude-sub");
            assert_eq!(decision.rung, Rung::AlternateCredential);
        }

        /// The opt-in does not loosen the product rule it refines.
        #[test]
        fn cross_account_fallback_never_lets_another_product_in() {
            let mut aider = claude_code(None);
            aider.client_identity = None;
            let error = claude(
                &mut with_fallback(),
                &aider,
                &[
                    claude_login("claude-sub", true),
                    claude_login("claude-work", false),
                ],
            )
            .expect_err("neither login is Aider's");
            assert_eq!(error, NoRoute::RequiresClientIdentity);
        }

        /// A request on another account's token is the move the opt-in makes,
        /// and the ledger has to show it whether or not the client's own login
        /// was there to be passed over. Deriving the rung from what was
        /// passed over recorded these as rung 0.
        #[test]
        fn a_request_served_by_another_login_is_always_recorded_as_rung_two() {
            let sibling_serves = |own: Option<Candidate>| {
                let candidates: Vec<Candidate> = own
                    .into_iter()
                    .chain([claude_login("claude-work", false)])
                    .collect();
                claude(&mut with_fallback(), &claude_code(None), &candidates).expect("routes")
            };

            let mut unconsented = claude_login("claude-sub", true);
            unconsented.consented = false;
            let mut untrusted = claude_login("claude-sub", true);
            untrusted.trusted = false;
            for (own, case) in [
                (Some(unconsented), "the client's own login is not consented"),
                (Some(untrusted), "the client's own login is not trusted"),
                (None, "the client has no login of its own"),
            ] {
                let decision = sibling_serves(own);
                assert_eq!(decision.backend.as_str(), "claude-work", "{case}");
                assert_eq!(decision.rung, Rung::AlternateCredential, "{case}");
            }

            // The client's own login, the only one there, is still rung 0.
            let decision = claude(
                &mut with_fallback(),
                &claude_code(Some("claude-work")),
                &[claude_login("claude-work", false)],
            )
            .expect("routes");
            assert_eq!(decision.rung, Rung::Preferred);
        }

        /// The P1 rule itself, for a login: one that belongs to another
        /// client could never have served this request, so the metered key
        /// serving it is the first choice, not a descent.
        #[test]
        fn another_clients_login_is_not_a_credential_passed_over() {
            let decision = claude(
                &mut Policy::new(),
                &claude_code(None),
                &[claude_login("claude-work", false), anthropic_key()],
            )
            .expect("routes");
            assert_eq!(decision.backend.as_str(), "anthropic-key");
            assert_eq!(decision.rung, Rung::Preferred);
        }

        /// The client's own login comes first on the turn whose output
        /// becomes the conversation, too.
        #[test]
        fn a_compaction_turn_still_prefers_the_clients_own_login() {
            let mut compaction = claude_code(Some("claude-work"));
            compaction.likely_compaction = true;
            let decision = claude(
                &mut with_fallback(),
                &compaction,
                &[
                    claude_login("claude-sub", true),
                    claude_login("claude-work", false),
                ],
            )
            .expect("routes");
            assert_eq!(decision.backend.as_str(), "claude-work");
            assert_eq!(decision.rung, Rung::Preferred);
        }

        /// The opt-in moves a client off its own login on the trigger every
        /// rung uses — sustained pressure, 90% observed for longer than the
        /// descent debounce — not only once that login is spent.
        #[test]
        fn with_cross_account_fallback_sustained_pressure_moves_the_client_off_its_own_login() {
            let pressured_own = || {
                let mut own = claude_login("claude-sub", true);
                own.quota.primary = Headroom::Observed {
                    used_pct: 92.0,
                    resets_at: Some(t(3600)),
                    observed_at: t(5),
                };
                [own, claude_login("claude-work", false)]
            };
            let at = |policy: &mut Policy, candidates: &[Candidate], when| {
                policy
                    .decide(
                        key(),
                        Protocol::AnthropicMessages,
                        &claude_code(None),
                        candidates,
                        when,
                    )
                    .expect("routes")
            };

            // A fresh selection sorts the pressured login behind the sibling.
            let decision = at(&mut with_fallback(), &pressured_own(), t(10));
            assert_eq!(decision.backend.as_str(), "claude-work");
            assert_eq!(decision.rung, Rung::AlternateCredential);

            // A conversation already on its own login waits out the debounce,
            // then moves.
            let mut policy = with_fallback();
            let fresh = [
                claude_login("claude-sub", true),
                claude_login("claude-work", false),
            ];
            assert_eq!(at(&mut policy, &fresh, t(0)).backend.as_str(), "claude-sub");
            assert_eq!(
                at(&mut policy, &pressured_own(), t(10)).backend.as_str(),
                "claude-sub",
                "pressure not yet sustained"
            );
            let moved = at(&mut policy, &pressured_own(), t(10) + DESCENT_DEBOUNCE);
            assert_eq!(moved.backend.as_str(), "claude-work");
            assert_eq!(moved.rung, Rung::AlternateCredential);

            // Without the opt-in the same pressure leaves it where it is.
            let decision = at(&mut Policy::new(), &pressured_own(), t(10));
            assert_eq!(decision.backend.as_str(), "claude-sub");
        }

        /// Ownership is asked before consent. Consenting to the sibling would
        /// change nothing for this client, so it is not the answer — and a
        /// spent own login stays a 429 rather than a 503 telling the user to
        /// enable the sibling.
        #[test]
        fn an_unconsented_sibling_does_not_turn_a_spent_own_login_into_a_configuration_error() {
            let mut unconsented = claude_login("claude-sub", true);
            unconsented.consented = false;
            for candidates in [
                vec![
                    unconsented.clone(),
                    exhausted(claude_login("claude-work", false), t(600)),
                ],
                vec![
                    unconsented.clone(),
                    exhausted(claude_login("claude-work", false), t(600)),
                    nearai_without_a_key(),
                ],
            ] {
                let error = claude(
                    &mut Policy::new(),
                    &claude_code(Some("claude-work")),
                    &candidates,
                )
                .expect_err("its own login is spent");
                assert_eq!(error, NoRoute::AllExhausted);
            }

            // For the client's own login consent is still the first answer.
            let error = claude(
                &mut Policy::new(),
                &claude_code(None),
                &[unconsented, claude_login("claude-work", false)],
            )
            .expect_err("its own login is not enabled");
            let NoRoute::NoneUsable { refusals } = error else {
                panic!("expected a refusal list, got {error:?}");
            };
            assert_eq!(
                refusals[0],
                (BackendId::from("claude-sub"), Refusal::NotConsented)
            );
        }

        #[test]
        fn a_first_codex_turn_may_start_on_the_sibling() {
            let decision = codex_route(
                &mut with_fallback(),
                &codex(None, false),
                &[
                    exhausted(codex_login("codex-sub", true), t(600)),
                    codex_login("codex-work", false),
                ],
                t(0),
            )
            .expect("nothing has been minted yet, so any account will do");
            assert_eq!(decision.backend.as_str(), "codex-work");
            assert_eq!(decision.rung, Rung::AlternateCredential);
        }

        /// OpenAI rejects encrypted reasoning minted under another
        /// organization, so moving the conversation would fail the turn it was
        /// meant to save.
        #[test]
        fn a_codex_conversation_carrying_encrypted_reasoning_is_not_moved_to_a_sibling() {
            let mut policy = with_fallback();
            let first = codex_route(
                &mut policy,
                &codex(None, false),
                &[
                    codex_login("codex-sub", true),
                    codex_login("codex-work", false),
                ],
                t(0),
            )
            .expect("routes");
            assert_eq!(first.backend.as_str(), "codex-sub");

            let spent = [
                exhausted(codex_login("codex-sub", true), t(3600)),
                codex_login("codex-work", false),
            ];
            let error = codex_route(&mut policy, &codex(None, true), &spent, t(60))
                .expect_err("the sibling is a different ChatGPT account");
            let refusals = refusals_by_id(error);
            assert_eq!(refusals.get("codex-sub"), Some(&Refusal::NoCapacity));
            let lock = refusals.get("codex-work").expect("named");
            assert_eq!(
                *lock,
                Refusal::Ineligible(Ineligible::ReasoningBoundToAccount)
            );
            let said = lock.describe();
            assert!(said.contains("ChatGPT account"), "{said}");
            assert!(said.contains("turn boundary"), "{said}");

            // A metered key also belongs to a different credential scope.
            let error = codex_route(
                &mut policy,
                &codex(None, true),
                &[spent[0].clone(), spent[1].clone(), openai_key()],
                t(61),
            )
            .expect_err("a metered key cannot replay subscription ciphertext");
            assert_eq!(
                refusals_by_id(error).get("openai-key"),
                Some(&Refusal::Ineligible(Ineligible::ReasoningBoundToAccount))
            );
        }

        /// The mirror image: a conversation that started on the sibling has
        /// its reasoning there, and promotion back to the client's own login
        /// would be rejected exactly as a descent would.
        #[test]
        fn a_codex_conversation_that_started_on_the_sibling_stays_there_and_is_not_promoted_back() {
            let mut policy = with_fallback();
            let first = codex_route(
                &mut policy,
                &codex(None, false),
                &[
                    exhausted(codex_login("codex-sub", true), t(600)),
                    codex_login("codex-work", false),
                ],
                t(0),
            )
            .expect("routes");
            assert_eq!(first.backend.as_str(), "codex-work");

            let recovered = [
                codex_login("codex-sub", true),
                codex_login("codex-work", false),
            ];
            for at in [
                t(700),
                t(700) + PROMOTION_DEBOUNCE,
                t(700) + PROMOTION_DEBOUNCE * 3,
            ] {
                let decision =
                    codex_route(&mut policy, &codex(None, true), &recovered, at).expect("routes");
                assert_eq!(
                    decision.backend.as_str(),
                    "codex-work",
                    "moved a conversation's encrypted reasoning to another account"
                );
                assert_eq!(
                    decision.rung,
                    Rung::AlternateCredential,
                    "the sibling is still not the client's own login"
                );
            }

            // Nor does losing the sibling send it back to its own login.
            let error = codex_route(
                &mut policy,
                &codex(None, true),
                &[
                    codex_login("codex-sub", true),
                    exhausted(codex_login("codex-work", false), t(99_999)),
                ],
                t(700) + PROMOTION_DEBOUNCE * 4,
            )
            .expect_err("the only account this reasoning fits is spent");
            assert_eq!(
                refusals_by_id(error).get("codex-sub"),
                Some(&Refusal::Ineligible(Ineligible::ReasoningBoundToAccount))
            );
        }

        /// A backend missing from the candidates — as one whose circuit was
        /// open once was — must not read as "this conversation is on no
        /// account" and release the reasoning to the client's own login.
        #[test]
        fn an_incumbent_whose_circuit_is_open_still_holds_the_conversations_reasoning() {
            let mut policy = with_fallback();
            codex_route(
                &mut policy,
                &codex(None, false),
                &[
                    exhausted(codex_login("codex-sub", true), t(600)),
                    codex_login("codex-work", false),
                ],
                t(0),
            )
            .expect("routes");

            // codex-work is failing and has been dropped; codex-sub recovered.
            let error = codex_route(
                &mut policy,
                &codex(None, true),
                &[codex_login("codex-sub", true)],
                t(700),
            )
            .expect_err("the reasoning was minted on codex-work");
            assert_eq!(
                refusals_by_id(error).get("codex-sub"),
                Some(&Refusal::Ineligible(Ineligible::ReasoningBoundToAccount))
            );
        }

        /// Why the sibling keeps reporting rung 2 under the lock: it is what
        /// lets the conversation key come home once nothing binds it — which,
        /// for Codex, is the next session, since every session of one Codex
        /// version derives the same key.
        #[test]
        fn once_nothing_binds_the_conversation_it_returns_to_its_own_login() {
            let mut policy = with_fallback();
            codex_route(
                &mut policy,
                &codex(None, false),
                &[
                    exhausted(codex_login("codex-sub", true), t(600)),
                    codex_login("codex-work", false),
                ],
                t(0),
            )
            .expect("routes");
            let recovered = [
                codex_login("codex-sub", true),
                codex_login("codex-work", false),
            ];
            codex_route(&mut policy, &codex(None, true), &recovered, t(700)).expect("routes");

            let fresh = codex(None, false);
            codex_route(&mut policy, &fresh, &recovered, t(800)).expect("routes");
            let decision =
                codex_route(&mut policy, &fresh, &recovered, t(800) + PROMOTION_DEBOUNCE)
                    .expect("routes");
            assert_eq!(decision.backend.as_str(), "codex-sub");
            assert_eq!(decision.rung, Rung::Preferred);
        }

        /// Every session of one client version shares a conversation key, so
        /// a route chosen for one login must not be inherited by a client on
        /// another — least of all its account lock.
        #[test]
        fn a_conversation_on_one_login_lends_no_account_lock_to_a_client_on_another() {
            let mut policy = with_fallback();
            let both = [
                codex_login("codex-sub", true),
                codex_login("codex-work", false),
            ];
            let bound = codex_route(&mut policy, &codex(Some("codex-work"), false), &both, t(0))
                .expect("routes");
            assert_eq!(bound.backend.as_str(), "codex-work");

            // Record the plain client's own first turn independently.
            codex_route(&mut policy, &codex(None, false), &both, t(4)).expect("plain first turn");
            let plain = codex_route(&mut policy, &codex(None, true), &both, t(5)).expect("routes");
            assert_eq!(
                plain.backend.as_str(),
                "codex-sub",
                "a client inherited another login's conversation"
            );
        }

        /// A conversation that fell back to codex-work and is now replaying
        /// codex-work's reasoning, with codex-sub's window reset.
        fn replaying_the_siblings_reasoning() -> Policy {
            let mut policy = with_fallback();
            let first = codex_route(
                &mut policy,
                &codex(None, false),
                &[
                    exhausted(codex_login("codex-sub", true), t(600)),
                    codex_login("codex-work", false),
                ],
                t(0),
            )
            .expect("routes");
            assert_eq!(first.backend.as_str(), "codex-work");
            let sticky = codex_route(
                &mut policy,
                &codex(None, true),
                &[
                    codex_login("codex-sub", true),
                    codex_login("codex-work", false),
                ],
                t(700),
            )
            .expect("routes");
            assert_eq!(sticky.backend.as_str(), "codex-work");
            policy
        }

        /// What the pipeline does when the chosen backend answers 429, 5xx,
        /// a transport error or `NeedsAuth`: forget the affinity and decide
        /// again with that backend excluded. The reasoning in the history is
        /// still codex-work's, so the client's own login must not get it.
        #[test]
        fn a_failing_sibling_keeps_the_reasoning_it_minted_from_the_clients_own_login() {
            let failing = || {
                let mut work = codex_login("codex-work", false);
                work.healthy = false;
                vec![codex_login("codex-sub", true), work]
            };

            let mut policy = replaying_the_siblings_reasoning();
            policy.forget(&codex_key());
            let error = codex_route(&mut policy, &codex(None, true), &failing(), t(701))
                .expect_err("the only account this reasoning fits is failing");
            assert_eq!(
                refusals_by_id(error).get("codex-sub"),
                Some(&Refusal::Ineligible(Ineligible::ReasoningBoundToAccount)),
                "a failure of the sibling released its reasoning to the own login"
            );

            // Once codex-work recovers it serves the conversation again.
            let decision = codex_route(
                &mut policy,
                &codex(None, true),
                &[
                    codex_login("codex-sub", true),
                    codex_login("codex-work", false),
                ],
                t(702),
            )
            .expect("routes");
            assert_eq!(decision.backend.as_str(), "codex-work");

            // Translation at a turn boundary drops the ciphertext safely.
            {
                let (elsewhere, rung) = (anthropic_key(), Rung::CrossFamily);
                let mut policy = replaying_the_siblings_reasoning();
                policy.forget(&codex_key());
                let mut candidates = failing();
                candidates.push(elsewhere.clone());
                let decision = codex_route(&mut policy, &codex(None, true), &candidates, t(701))
                    .expect("routes");
                assert_eq!(decision.backend, elsewhere.id);
                assert_eq!(decision.rung, rung);
            }
        }

        /// Every session of one Codex version derives one conversation key, so
        /// a session that moves to the sibling moves the shared affinity with
        /// it. Each session's own reasoning stays where it was minted.
        #[test]
        fn a_session_that_moves_to_the_sibling_does_not_take_another_sessions_reasoning_with_it() {
            let session = |id: &str, replayed: bool| {
                let mut request = codex(None, replayed);
                request.client_session = Some(id.to_string());
                request
            };
            let both = [
                codex_login("codex-sub", true),
                codex_login("codex-work", false),
            ];
            let spent = [
                exhausted(codex_login("codex-sub", true), t(3600)),
                codex_login("codex-work", false),
            ];

            let mut policy = with_fallback();
            for (id, at) in [("one", t(0)), ("two", t(1))] {
                let decision =
                    codex_route(&mut policy, &session(id, false), &both, at).expect("routes");
                assert_eq!(decision.backend.as_str(), "codex-sub");
            }
            let third =
                codex_route(&mut policy, &session("three", false), &spent, t(2)).expect("routes");
            assert_eq!(third.backend.as_str(), "codex-work");

            for id in ["one", "two"] {
                let error = codex_route(&mut policy, &session(id, true), &spent, t(3))
                    .expect_err("its reasoning is codex-sub's, which is spent");
                assert_eq!(
                    refusals_by_id(error).get("codex-work"),
                    Some(&Refusal::Ineligible(Ineligible::ReasoningBoundToAccount)),
                    "session {id} was sent to the account another session moved to"
                );
            }
            let decision = codex_route(&mut policy, &session("three", true), &spent, t(4))
                .expect("its reasoning is codex-work's");
            assert_eq!(decision.backend.as_str(), "codex-work");
        }

        /// A conversation that crossed to another family at a turn boundary
        /// still replays the reasoning its own login minted; the other family
        /// minted none. When that backend drops out of the candidates — its
        /// circuit opened — the own login serves it again.
        #[test]
        fn a_codex_conversation_back_from_another_family_returns_to_its_own_login() {
            for fallback in [false, true] {
                let mut policy = Policy::new();
                policy.set_cross_account_fallback(fallback);
                let first = codex_route(
                    &mut policy,
                    &codex(None, false),
                    &[
                        codex_login("codex-sub", true),
                        codex_login("codex-work", false),
                        anthropic_key(),
                    ],
                    t(0),
                )
                .expect("routes");
                assert_eq!(first.backend.as_str(), "codex-sub");

                let crossed = codex_route(
                    &mut policy,
                    &codex(None, true),
                    &[
                        exhausted(codex_login("codex-sub", true), t(600)),
                        codex_login("codex-work", false),
                        anthropic_key(),
                    ],
                    t(60),
                )
                .expect("routes");
                assert_eq!(crossed.backend.as_str(), "anthropic-key");
                assert_eq!(crossed.rung, Rung::CrossFamily);

                let back = codex_route(
                    &mut policy,
                    &codex(None, true),
                    &[
                        codex_login("codex-sub", true),
                        codex_login("codex-work", false),
                    ],
                    t(700),
                )
                .unwrap_or_else(|e| panic!("fallback {fallback}: {e}"));
                assert_eq!(back.backend.as_str(), "codex-sub", "fallback {fallback}");
                assert_eq!(back.rung, Rung::Preferred, "fallback {fallback}");
            }
        }

        #[test]
        fn encrypted_reasoning_minted_by_a_metered_key_stays_on_that_key() {
            let mut policy = with_fallback();
            let first = codex_route(&mut policy, &codex(None, false), &[openai_key()], t(0))
                .expect("first turn may use the metered key");
            assert_eq!(first.backend.as_str(), "openai-key");
            let replay = codex_route(
                &mut policy,
                &codex(None, true),
                &[codex_login("codex-sub", true), openai_key()],
                t(700),
            )
            .expect("replay");
            assert_eq!(replay.backend.as_str(), "openai-key");
            assert!(
                codex_route(
                    &mut policy,
                    &codex(None, true),
                    &[codex_login("codex-sub", true)],
                    t(701)
                )
                .is_err()
            );
        }

        #[test]
        fn with_no_record_encrypted_reasoning_is_refused_rather_than_assigned_an_account() {
            let error = codex_route(
                &mut with_fallback(),
                &codex(None, true),
                &[
                    codex_login("codex-sub", true),
                    codex_login("codex-work", false),
                    openai_key(),
                ],
                t(0),
            )
            .expect_err("no credential is known to have minted this ciphertext");
            for refusal in refusals_by_id(error).values() {
                assert_eq!(
                    *refusal,
                    Refusal::Ineligible(Ineligible::ReasoningBoundToAccount)
                );
            }
        }

        /// The refusal a user can act on, named with the command that acts on
        /// it — for a machine whose only Claude login is an additional one.
        ///
        /// With NEAR AI beside it, unkeyed, as every daemon registers it: a
        /// transient refusal next to the wrong URL used to turn this into a
        /// 429, and the client backed off forever without being told.
        #[test]
        fn a_login_reserved_for_another_client_is_refused_by_name_with_the_command_that_binds_one()
        {
            let error = claude(
                &mut Policy::new(),
                &claude_code(None),
                &[claude_login("claude-work", false), nearai_without_a_key()],
            )
            .expect_err("the plain façade belongs to the default login");
            assert_eq!(
                error,
                NoRoute::NoneUsable {
                    refusals: vec![
                        (
                            BackendId::from("claude-work"),
                            Refusal::AnotherLogin {
                                client: ClientIdentity::ClaudeCode
                            }
                        ),
                        (BackendId::from("nearai"), Refusal::Unhealthy),
                    ]
                }
            );
            let said = error.to_string();
            assert!(said.contains("claude-work"), "{said}");
            assert!(
                said.contains("ironwire connect claude --config-dir"),
                "{said}"
            );

            let codex_error = codex_route(
                &mut Policy::new(),
                &codex(None, false),
                &[codex_login("codex-work", false), nearai_without_a_key()],
                t(0),
            )
            .expect_err("the plain façade belongs to the default login");
            assert!(
                codex_error
                    .to_string()
                    .contains("ironwire connect codex --config-dir"),
                "{codex_error}"
            );
        }

        /// A second login on the machine must not turn "your subscription is
        /// spent, come back later" into a configuration error.
        #[test]
        fn an_exhausted_own_login_beside_another_login_is_still_exhaustion() {
            let error = claude(
                &mut Policy::new(),
                &claude_code(None),
                &[
                    exhausted(claude_login("claude-sub", true), t(600)),
                    claude_login("claude-work", false),
                    nearai_without_a_key(),
                ],
            )
            .expect_err("nothing of this client's has capacity");
            assert_eq!(error, NoRoute::AllExhausted);
        }

        /// A header a client can set must not relax a constraint the user set
        /// in their own config — nor the product rule that constraint refines.
        #[test]
        fn a_pin_cannot_hand_a_client_another_login() {
            let both = [
                claude_login("claude-sub", true),
                claude_login("claude-work", false),
            ];
            let override_to = |id: &str| Some((BackendId::from(id), None));

            let error = Policy::new()
                .decide_with_override(
                    key(),
                    Protocol::AnthropicMessages,
                    &claude_code(None),
                    &both,
                    t(0),
                    override_to("claude-work"),
                )
                .expect_err("cross-account fallback is off");
            assert!(
                refusals_by_id(error)
                    .get("claude-work")
                    .is_some_and(|why| matches!(why, Refusal::AnotherLogin { .. }))
            );

            // With the opt-in, the user's instruction stands.
            let decision = with_fallback()
                .decide_with_override(
                    key(),
                    Protocol::AnthropicMessages,
                    &claude_code(None),
                    &both,
                    t(0),
                    override_to("claude-work"),
                )
                .expect("the user opted in");
            assert_eq!(decision.backend.as_str(), "claude-work");

            // And no opt-in makes a subscription serve another product.
            let mut aider = claude_code(None);
            aider.client_identity = None;
            let error = with_fallback()
                .decide_with_override(
                    key(),
                    Protocol::AnthropicMessages,
                    &aider,
                    &both,
                    t(0),
                    override_to("claude-sub"),
                )
                .expect_err("Aider is not Claude Code");
            assert_eq!(error, NoRoute::RequiresClientIdentity);
        }

        /// Pins may choose the first turn's account, but cannot move a replay.
        #[test]
        fn a_pin_can_start_reasoning_on_an_account_but_cannot_move_a_replay() {
            let both = [
                codex_login("codex-sub", true),
                codex_login("codex-work", false),
            ];

            let mut policy = with_fallback();
            let first = codex_route(&mut policy, &codex(None, false), &both, t(0)).expect("routes");
            assert_eq!(first.backend.as_str(), "codex-sub");
            policy.set_pin(Some(BackendId::from("codex-work")), None);
            assert!(
                codex_route(&mut policy, &codex(None, true), &both, t(1)).is_err(),
                "the user's pin cannot move ciphertext to another account"
            );
            policy.set_pin(None, None);
            let unpinned =
                codex_route(&mut policy, &codex(None, true), &both, t(2)).expect("routes");
            assert_eq!(
                unpinned.backend.as_str(),
                "codex-sub",
                "a pinned replay that minted nothing kept the conversation on the pinned account"
            );

            let mut policy = with_fallback();
            policy.set_pin(Some(BackendId::from("codex-work")), None);
            let first =
                codex_route(&mut policy, &codex(None, false), &both, t(0)).expect("the user's pin");
            assert_eq!(first.backend.as_str(), "codex-work");
            policy.set_pin(None, None);
            let unpinned =
                codex_route(&mut policy, &codex(None, true), &both, t(1)).expect("routes");
            assert_eq!(
                unpinned.backend.as_str(),
                "codex-work",
                "the reasoning a pinned first turn minted was sent to another account"
            );
        }
    }

    /// A distinct conversation per index, so a test can fill the map.
    fn key_n(n: usize) -> ConversationKey {
        ConversationKey::derive(
            Protocol::AnthropicMessages,
            &format!("You are Claude Code, session {n}"),
            &["Read"],
        )
    }

    fn route(policy: &mut Policy, key: ConversationKey, at: DateTime<Utc>) {
        policy
            .decide(
                key,
                Protocol::AnthropicMessages,
                &peek("claude-opus-4-6"),
                &[candidate(
                    "claude-sub",
                    BackendKind::Subscription,
                    Protocol::AnthropicMessages,
                )],
                at,
            )
            .expect("routes");
    }

    #[test]
    fn the_affinity_map_is_bounded() {
        // Left unbounded this grew one entry per distinct conversation for the
        // life of a daemon that is meant to run for weeks.
        let mut policy = Policy::new();
        for n in 0..1000 {
            route(&mut policy, key_n(n), t(0));
        }
        assert!(
            policy.tracked_conversations() <= MAX_AFFINITIES,
            "kept {} affinities",
            policy.tracked_conversations()
        );
    }

    #[test]
    fn a_conversation_that_goes_quiet_is_forgotten() {
        let mut policy = Policy::new();
        route(&mut policy, key(), t(0));
        assert_eq!(policy.tracked_conversations(), 1);

        // Advance the clock rather than sleep: `now` is a parameter precisely
        // so this is testable.
        route(&mut policy, key_n(99), t(AFFINITY_TTL.num_seconds() + 1));
        assert_eq!(
            policy.tracked_conversations(),
            1,
            "the idle conversation should have been swept, leaving only the new one"
        );
    }

    /// The `since`-versus-`last_seen` trap. A conversation established days ago
    /// but used a second ago is the *most* alive thing in the map; evicting on
    /// `since` would drop it first.
    #[test]
    fn a_conversation_still_in_use_is_never_aged_out() {
        let mut policy = Policy::new();
        let hourly = AFFINITY_TTL.num_seconds() / 2;
        for step in 0..6 {
            route(&mut policy, key(), t(step * hourly));
        }
        assert_eq!(
            policy.tracked_conversations(),
            1,
            "an actively used conversation was evicted by age"
        );
        assert_eq!(
            policy.current_backend(&key()).map(|b| b.to_string()),
            Some("claude-sub".to_string())
        );
    }

    /// Over the cap, the entry that goes is the least recently *used* — not
    /// whatever `HashMap` iteration happens to surface first.
    #[test]
    fn the_stalest_conversation_is_the_one_evicted() {
        let mut policy = Policy::new();
        for n in 0..MAX_AFFINITIES {
            route(&mut policy, key_n(n), t(0));
        }
        // Touch every key except one, so that one is unambiguously the stalest.
        for n in 1..MAX_AFFINITIES {
            route(&mut policy, key_n(n), t(60));
        }
        assert!(
            policy.current_backend(&key_n(0)).is_some(),
            "not yet evicted"
        );

        // One more conversation puts us over the cap.
        route(&mut policy, key_n(MAX_AFFINITIES), t(120));
        assert!(
            policy.current_backend(&key_n(0)).is_none(),
            "the stalest conversation survived"
        );
        assert!(
            policy.current_backend(&key_n(1)).is_some(),
            "a recently used conversation was evicted instead"
        );
    }

    /// Eviction is invisible: the next request re-selects and, conditions
    /// unchanged, lands in the same place.
    #[test]
    fn an_evicted_conversation_simply_re_establishes_itself() {
        let mut policy = Policy::new();
        route(&mut policy, key(), t(0));
        let later = t(AFFINITY_TTL.num_seconds() + 1);
        route(&mut policy, key_n(1), later);
        assert!(policy.current_backend(&key()).is_none(), "was swept");

        let decision = policy
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek("claude-opus-4-6"),
                &[candidate(
                    "claude-sub",
                    BackendKind::Subscription,
                    Protocol::AnthropicMessages,
                )],
                later,
            )
            .expect("routes again without error");
        assert_eq!(decision.backend.as_str(), "claude-sub");
    }

    #[test]
    fn prefers_free_capacity_over_metered() {
        let mut policy = Policy::new();
        let candidates = vec![
            candidate(
                "anthropic-key",
                BackendKind::ApiKey,
                Protocol::AnthropicMessages,
            ),
            candidate(
                "claude-sub",
                BackendKind::Subscription,
                Protocol::AnthropicMessages,
            ),
        ];
        let d = policy
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek("claude-opus-4-6"),
                &candidates,
                t(0),
            )
            .expect("a route exists");
        assert_eq!(d.backend.as_str(), "claude-sub");
        assert_eq!(d.rung, Rung::Preferred);
        assert!(!d.translated);
    }

    #[test]
    fn forwards_the_clients_model_untouched_when_the_backend_has_it() {
        // No body edit is the safest possible native-lane request.
        let mut policy = Policy::new();
        let candidates = vec![candidate(
            "claude-sub",
            BackendKind::Subscription,
            Protocol::AnthropicMessages,
        )];
        let d = policy
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek("claude-opus-4-6"),
                &candidates,
                t(0),
            )
            .expect("a route exists");
        assert_eq!(d.model, None);
    }

    #[test]
    fn affinity_survives_a_brief_rate_limit_rather_than_dumping_the_cache() {
        let mut policy = Policy::new();
        let mut sub = candidate(
            "claude-sub",
            BackendKind::Subscription,
            Protocol::AnthropicMessages,
        );
        let fallback = candidate(
            "anthropic-key",
            BackendKind::ApiKey,
            Protocol::AnthropicMessages,
        );

        let first = policy
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek("claude-opus-4-6"),
                &[sub.clone(), fallback.clone()],
                t(0),
            )
            .expect("a route exists");
        assert_eq!(first.backend.as_str(), "claude-sub");

        // Pressure appears; within the debounce we must stay put.
        sub.quota.primary = Headroom::Observed {
            used_pct: 97.0,
            resets_at: None,
            observed_at: t(1),
        };
        let during = policy
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek("claude-opus-4-6"),
                &[sub.clone(), fallback.clone()],
                t(5),
            )
            .expect("a route exists");
        assert_eq!(during.backend.as_str(), "claude-sub");

        // Sustained pressure past the debounce descends to the API key.
        let after = policy
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek("claude-opus-4-6"),
                &[sub, fallback],
                t(40),
            )
            .expect("a route exists");
        assert_eq!(after.backend.as_str(), "anthropic-key");
    }

    #[test]
    fn pressure_that_lifts_resets_the_debounce() {
        let mut policy = Policy::new();
        let mut sub = candidate(
            "claude-sub",
            BackendKind::Subscription,
            Protocol::AnthropicMessages,
        );
        let fallback = candidate(
            "anthropic-key",
            BackendKind::ApiKey,
            Protocol::AnthropicMessages,
        );
        let all = |s: &Candidate| vec![s.clone(), fallback.clone()];

        policy
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek("claude-opus-4-6"),
                &all(&sub),
                t(0),
            )
            .expect("route");
        sub.quota.primary = Headroom::Observed {
            used_pct: 95.0,
            resets_at: None,
            observed_at: t(1),
        };
        policy
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek("claude-opus-4-6"),
                &all(&sub),
                t(5),
            )
            .expect("route");
        // Pressure lifts.
        sub.quota.primary = Headroom::Observed {
            used_pct: 40.0,
            resets_at: None,
            observed_at: t(10),
        };
        policy
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek("claude-opus-4-6"),
                &all(&sub),
                t(10),
            )
            .expect("route");
        // Returns; the clock must have restarted, so we stay put at t=25.
        sub.quota.primary = Headroom::Observed {
            used_pct: 95.0,
            resets_at: None,
            observed_at: t(20),
        };
        let d = policy
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek("claude-opus-4-6"),
                &all(&sub),
                t(25),
            )
            .expect("route");
        assert_eq!(d.backend.as_str(), "claude-sub");
    }

    #[test]
    fn a_subscription_is_not_unlocked_for_a_client_that_is_not_its_own() {
        // TRUST.md §3 — Aider does not get to be Claude Code.
        let mut policy = Policy::new();
        let mut sub = candidate(
            "claude-sub",
            BackendKind::Subscription,
            Protocol::AnthropicMessages,
        );
        sub.required_client_identity = Some(ClientIdentity::ClaudeCode);
        let mut third_party = peek("claude-opus-4-6");
        third_party.client_identity = None;

        let err = policy
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &third_party,
                &[sub],
                t(0),
            )
            .expect_err("subscription must be refused");
        assert_eq!(err, NoRoute::RequiresClientIdentity);
    }

    /// `docs/TRUST.md` I5, in the direction that only became reachable once
    /// every wire could translate to every other.
    ///
    /// Claude Code carries Claude Code's identity, which is a real identity —
    /// and the check used to ask only whether *some* identity was present. So a
    /// Claude Code turn falling back cross-wire would have arrived at
    /// `chatgpt.com` wearing Claude Code's name, which is the impersonation the
    /// invariant exists to forbid. One product's identity must never unlock
    /// another's subscription.
    #[test]
    fn one_products_identity_does_not_unlock_anothers_subscription() {
        let mut chatgpt = candidate(
            "codex-sub",
            BackendKind::Subscription,
            Protocol::OpenAiResponses,
        );
        chatgpt.required_client_identity = Some(ClientIdentity::Codex);

        let mut from_claude_code = peek("claude-opus-4-6");
        from_claude_code.client_identity = Some(ClientIdentity::ClaudeCode);

        let err = Policy::new()
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &from_claude_code,
                &[chatgpt.clone()],
                t(0),
            )
            .expect_err("Claude Code must not reach the ChatGPT subscription");
        assert_eq!(err, NoRoute::RequiresClientIdentity);

        // And the same backend serves the client it does belong to, translated
        // or not — the rule is about whose subscription it is, not about wires.
        let mut from_codex = peek("gpt-5");
        from_codex.client_identity = Some(ClientIdentity::Codex);
        let decision = Policy::new()
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &from_codex,
                &[chatgpt],
                t(0),
            )
            .expect("Codex's own identity reaches Codex's own subscription");
        assert_eq!(decision.backend.as_str(), "codex-sub");
    }

    #[test]
    fn cross_family_is_a_last_resort_and_is_flagged() {
        let mut policy = Policy::new();
        let mut exhausted = candidate(
            "claude-sub",
            BackendKind::Subscription,
            Protocol::AnthropicMessages,
        );
        exhausted.quota.primary = Headroom::Exhausted { until: t(3600) };
        let near = candidate("nearai", BackendKind::Credits, Protocol::OpenAiChat);

        let d = policy
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek("claude-opus-4-6"),
                &[exhausted, near],
                t(0),
            )
            .expect("a route exists");
        assert_eq!(d.backend.as_str(), "nearai");
        assert_eq!(d.rung, Rung::CrossFamily);
        assert!(d.translated);
        assert!(d.rung.is_user_visible());
    }

    #[test]
    fn a_conversation_mid_tool_loop_waits_for_the_turn_boundary() {
        // The corrected rule: a family change is deferred to the next clean
        // turn, not refused for the life of the conversation.
        let mut policy = Policy::new();
        let mut exhausted = candidate(
            "claude-sub",
            BackendKind::Subscription,
            Protocol::AnthropicMessages,
        );
        exhausted.quota.primary = Headroom::Exhausted { until: t(3600) };
        let near = candidate("nearai", BackendKind::Credits, Protocol::OpenAiChat);

        let mut mid_loop = peek("claude-opus-4-6");
        mid_loop.requirements.reasoning = ReasoningNeed::LoadBearing;
        mid_loop.requirements.mid_tool_loop = true;

        let err = policy
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &mid_loop,
                &[exhausted.clone(), near.clone()],
                t(0),
            )
            .expect_err("a family change mid tool loop must be refused");
        match err {
            NoRoute::NoneUsable { refusals } => {
                assert!(
                    refusals
                        .iter()
                        .any(|(_, why)| *why == Refusal::Ineligible(Ineligible::MidToolLoop))
                );
            }
            other => panic!("expected ineligibility, got {other:?}"),
        }

        // Same conversation, next turn boundary: NEAR AI is now eligible.
        let mut boundary = mid_loop;
        boundary.requirements.mid_tool_loop = false;
        let decision = policy
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &boundary,
                &[exhausted, near],
                t(0),
            )
            .expect("a turn boundary is a clean switch point");
        assert_eq!(decision.backend.as_str(), "nearai");
        assert!(decision.translated);
    }

    #[test]
    fn a_pin_still_cannot_corrupt_a_request() {
        let mut policy = Policy::new();
        let mut near = candidate("nearai", BackendKind::Credits, Protocol::OpenAiChat);
        near.caps.images = false;
        policy.set_pin(Some(BackendId::from("nearai")), None);

        let mut p = peek("claude-opus-4-6");
        p.requirements.images = true;

        let err = policy
            .decide(key(), Protocol::AnthropicMessages, &p, &[near], t(0))
            .expect_err("pin must not override eligibility");
        assert!(matches!(err, NoRoute::NoneUsable { .. }));
    }

    /// Consent is what lets IronWire replay another product's token at all,
    /// and neither a pin nor a header any local client can set gives it.
    #[test]
    fn a_pin_cannot_reach_a_subscription_nobody_consented_to() {
        let mut unconsented = candidate(
            "claude-sub",
            BackendKind::Subscription,
            Protocol::AnthropicMessages,
        );
        unconsented.consented = false;
        let candidates = [
            unconsented,
            candidate(
                "anthropic-key",
                BackendKind::ApiKey,
                Protocol::AnthropicMessages,
            ),
        ];
        let refused = NoRoute::NoneUsable {
            refusals: vec![(BackendId::from("claude-sub"), Refusal::NotConsented)],
        };

        let mut pinned = Policy::new();
        pinned.set_pin(Some(BackendId::from("claude-sub")), None);
        let err = pinned
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek("claude-opus-4-6"),
                &candidates,
                t(0),
            )
            .expect_err("a pin is not consent");
        assert_eq!(err, refused);

        let err = Policy::new()
            .decide_with_override(
                key(),
                Protocol::AnthropicMessages,
                &peek("claude-opus-4-6"),
                &candidates,
                t(0),
                Some((BackendId::from("claude-sub"), None)),
            )
            .expect_err("nor is X-IronWire-Route");
        assert_eq!(err, refused);
    }

    /// The pin path used to ask a *family* question where the wire question is
    /// the one that matters. Responses and Chat Completions share a family, so
    /// a pinned Codex request was handed to a Chat backend as a **native
    /// forward** — `translated: false`, a Responses body, and an endpoint that
    /// cannot read it.
    ///
    /// Now that every pair has a translator the route is legal, which makes the
    /// assertion sharper rather than weaker: the pin is honoured *and* the body
    /// is translated. A pin overrides preference, never the shape of the
    /// request.
    #[test]
    fn a_pin_translates_a_body_the_backend_cannot_read_natively() {
        let mut policy = Policy::new();
        let chat_only = candidate("local", BackendKind::Local, Protocol::OpenAiChat);
        policy.set_pin(Some(BackendId::from("local")), None);

        let decision = policy
            .decide(
                key(),
                Protocol::OpenAiResponses,
                &peek("gpt-5"),
                &[chat_only],
                t(0),
            )
            .expect("a pinned backend with a translator is a legal route");
        assert_eq!(decision.backend.as_str(), "local");
        assert!(
            decision.translated,
            "a Responses body was forwarded to a Chat Completions endpoint as-is"
        );
        assert_eq!(decision.rung, Rung::CrossFamily);
    }

    /// A provider serving several wires at one base URL is on the native lane
    /// for each of them. NEAR AI answers both `/v1/chat/completions` and
    /// `/v1/responses`, and claiming only the first is what left Codex with
    /// nowhere to go on a machine where the subscriptions were not consented.
    #[test]
    fn a_backend_that_serves_two_wires_is_native_on_both() {
        let mut near = candidate("nearai", BackendKind::Credits, Protocol::OpenAiChat);
        near.caps.wires = Wires::new(Protocol::OpenAiChat, &[Protocol::OpenAiResponses]);

        for inbound in [Protocol::OpenAiResponses, Protocol::OpenAiChat] {
            let decision = Policy::new()
                .decide(key(), inbound, &peek("gpt-5"), &[near.clone()], t(0))
                .expect("the wire it serves is the native lane");
            assert_eq!(decision.backend.as_str(), "nearai");
            assert!(
                !decision.translated,
                "{inbound} is served natively and must not be translated"
            );
            assert_eq!(decision.rung, Rung::Preferred);
        }
    }

    /// The refusal that started this: two subscriptions nobody consented to and
    /// one backend on a wire with no translator. The old answer named only the
    /// wire, because it collected reasons for candidates that failed the
    /// *capability* gate and dropped everything refused before it — so the log
    /// discussed the one backend the user could do nothing about.
    #[test]
    fn a_refusal_names_the_backends_that_never_reached_the_gate() {
        let mut claude = candidate(
            "claude-sub",
            BackendKind::Subscription,
            Protocol::AnthropicMessages,
        );
        claude.consented = false;
        let mut codex = candidate(
            "codex-sub",
            BackendKind::Subscription,
            Protocol::OpenAiResponses,
        );
        codex.consented = false;
        let mut near = candidate("nearai", BackendKind::Credits, Protocol::OpenAiChat);
        near.healthy = false;

        let err = Policy::new()
            .decide(
                key(),
                Protocol::OpenAiResponses,
                &peek("gpt-5"),
                &[claude, codex, near],
                t(0),
            )
            .expect_err("nothing can serve this");

        match err {
            NoRoute::NoneUsable { refusals } => {
                let by_id: HashMap<_, _> = refusals
                    .iter()
                    .map(|(id, why)| (id.as_str().to_string(), why.clone()))
                    .collect();
                assert_eq!(by_id.get("claude-sub"), Some(&Refusal::NotConsented));
                assert_eq!(by_id.get("codex-sub"), Some(&Refusal::NotConsented));
                assert_eq!(by_id.get("nearai"), Some(&Refusal::Unhealthy));
                // Consent leads: it is the only entry with an instruction in it.
                assert!(refusals[0].1.is_actionable());
            }
            other => panic!("expected a full refusal list, got {other:?}"),
        }
    }

    /// Backends that are merely busy still get the 429 answer. Listing three
    /// healthy backends with nothing wrong with them, as a 503, would be a
    /// worse reply than "come back in a minute".
    #[test]
    fn everything_merely_rate_limited_is_still_exhaustion() {
        let mut a = candidate(
            "claude-sub",
            BackendKind::Subscription,
            Protocol::AnthropicMessages,
        );
        a.quota = QuotaSnapshot {
            primary: Headroom::Exhausted { until: t(60) },
            ..QuotaSnapshot::default()
        };
        let mut b = candidate(
            "anthropic-key",
            BackendKind::ApiKey,
            Protocol::AnthropicMessages,
        );
        b.healthy = false;

        let err = Policy::new()
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek("claude-opus-4-6"),
                &[a, b],
                t(0),
            )
            .expect_err("nothing has capacity");
        assert!(matches!(err, NoRoute::AllExhausted), "got {err:?}");
    }

    /// The identity refusal used to be unreachable whenever anything else was
    /// ineligible: the general answer returned first and the check below it
    /// never ran. It is the refusal with an actual instruction attached, so it
    /// is the one that must survive.
    #[test]
    fn an_identity_refusal_is_not_masked_by_an_incidental_one() {
        let mut codex = candidate(
            "codex-sub",
            BackendKind::Subscription,
            Protocol::OpenAiResponses,
        );
        codex.required_client_identity = Some(ClientIdentity::Codex);
        let mut p = peek("gpt-5");
        p.client_identity = None;

        let err = Policy::new()
            .decide(key(), Protocol::OpenAiResponses, &p, &[codex], t(0))
            .expect_err("nothing can serve this");
        assert!(
            matches!(err, NoRoute::RequiresClientIdentity),
            "got {err:?}"
        );
    }

    #[test]
    fn conversation_key_is_stable_as_the_conversation_grows() {
        // Same system prompt and tools, different turn counts: one key.
        let a = ConversationKey::derive(
            Protocol::AnthropicMessages,
            "You are Claude Code",
            &["Read", "Bash"],
        );
        let b = ConversationKey::derive(
            Protocol::AnthropicMessages,
            "You are Claude Code",
            &["Bash", "Read"],
        );
        assert_eq!(a, b, "tool order must not split a conversation");

        let other = ConversationKey::derive(
            Protocol::AnthropicMessages,
            "You are a helpful bot",
            &["Read", "Bash"],
        );
        assert_ne!(a, other);
    }

    #[test]
    fn no_backends_is_reported_as_such() {
        let mut policy = Policy::new();
        let err = policy
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek("claude-opus-4-6"),
                &[],
                t(0),
            )
            .expect_err("no candidates");
        assert_eq!(err, NoRoute::NoBackendsConfigured);
    }

    /// Not routed to, and — the part this test used to get wrong — not reported
    /// as exhaustion either.
    ///
    /// `AllExhausted` renders as a 429: come back later, capacity will return.
    /// It never will here, because nothing is spent and nothing is broken; a
    /// switch has not been flipped. Answering "rate limited" to a question
    /// whose answer is "you have not enabled this" is the same class of lie as
    /// a capacity bar drawn at zero.
    #[test]
    fn an_unconsented_subscription_is_refused_by_name_rather_than_as_exhaustion() {
        let mut policy = Policy::new();
        let mut sub = candidate(
            "claude-sub",
            BackendKind::Subscription,
            Protocol::AnthropicMessages,
        );
        sub.consented = false;
        let err = policy
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek("claude-opus-4-6"),
                &[sub],
                t(0),
            )
            .expect_err("unconsented backend must not be routed to");
        assert_eq!(
            err,
            NoRoute::NoneUsable {
                refusals: vec![(BackendId::from("claude-sub"), Refusal::NotConsented)]
            }
        );
    }

    /// A catalogue shaped like NEAR AI's: one backend serving many families,
    /// listed alphabetically, almost every model at the same tier. A tier pick
    /// on it always lands on the first frontier entry, whatever was asked for.
    fn near_catalogue(protocol: Protocol) -> Candidate {
        let mut near = candidate("nearai", BackendKind::Credits, protocol);
        near.models = [
            "anthropic/claude-fable-5",
            "anthropic/claude-opus-5-5",
            "anthropic/claude-sonnet-4-6",
            "anthropic/claude-sonnet-5-5",
            "openai/gpt-5.5",
            "qwen/qwen3.7-max",
        ]
        .into_iter()
        .map(|m| (m.to_string(), ModelTier::from_model_hint(m)))
        .collect();
        near
    }

    /// The model a client names is a hint only when the backend cannot say
    /// whether it serves it. A pin on the backend alone used to re-pick by
    /// tier, so a tool's own model picker chose nothing: every request went to
    /// `anthropic/claude-fable-5`, the first frontier model alphabetically.
    #[test]
    fn a_backend_pin_keeps_a_model_the_provider_lists() {
        let mut policy = Policy::new();
        policy.set_pin(Some(BackendId::from("nearai")), None);

        let decision = policy
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek("anthropic/claude-opus-5-5"),
                &[near_catalogue(Protocol::OpenAiChat)],
                t(0),
            )
            .expect("a pinned backend serving the model is a route");
        assert!(decision.translated);
        assert_eq!(
            decision.model.as_deref(),
            Some("anthropic/claude-opus-5-5"),
            "the translated body must carry the model the client chose"
        );
    }

    /// The translated lane without a pin, which is where a Claude Code request
    /// lands on a Chat Completions backend.
    #[test]
    fn the_translated_lane_keeps_a_model_the_provider_lists() {
        let decision = Policy::new()
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek("qwen/qwen3.7-max"),
                &[near_catalogue(Protocol::OpenAiChat)],
                t(0),
            )
            .expect("routes");
        assert_eq!(decision.rung, Rung::CrossFamily);
        assert_eq!(decision.model.as_deref(), Some("qwen/qwen3.7-max"));
    }

    /// On the native lane the client's own string is forwarded untouched,
    /// which is `None`: no body edit at all (`docs/PROTOCOL.md` §2).
    #[test]
    fn the_native_lane_forwards_a_model_the_provider_lists_without_an_edit() {
        let decision = Policy::new()
            .decide(
                key(),
                Protocol::OpenAiChat,
                &peek("anthropic/claude-sonnet-5-5"),
                &[near_catalogue(Protocol::OpenAiChat)],
                t(0),
            )
            .expect("routes");
        assert!(!decision.translated);
        assert_eq!(
            decision.model, None,
            "a tier pick replaced `anthropic/claude-sonnet-5-5` with a sibling"
        );
    }

    /// Switching model mid-conversation keeps the conversation key, so the
    /// sticky route answered with the model stored on the first turn.
    #[test]
    fn a_model_switch_mid_conversation_is_honoured() {
        let mut policy = Policy::new();
        let near = [near_catalogue(Protocol::OpenAiChat)];
        let first = policy
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek("anthropic/claude-sonnet-5-5"),
                &near,
                t(0),
            )
            .expect("routes");
        assert_eq!(first.model.as_deref(), Some("anthropic/claude-sonnet-5-5"));

        let second = policy
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek("openai/gpt-5.5"),
                &near,
                t(5),
            )
            .expect("routes");
        assert_eq!(second.backend.as_str(), "nearai");
        assert_eq!(second.model.as_deref(), Some("openai/gpt-5.5"));
    }

    /// A pin that names a model is the user's instruction and still wins.
    #[test]
    fn a_pinned_model_outranks_the_requested_one() {
        let mut policy = Policy::new();
        policy.set_pin(
            Some(BackendId::from("nearai")),
            Some("qwen/qwen3.7-max".to_string()),
        );
        let decision = policy
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek("anthropic/claude-opus-5-5"),
                &[near_catalogue(Protocol::OpenAiChat)],
                t(0),
            )
            .expect("routes");
        assert_eq!(decision.model.as_deref(), Some("qwen/qwen3.7-max"));
    }

    /// A compiled-in catalogue is a guess about the provider, so a name in it
    /// is not evidence the provider serves that name. The tier pick stands.
    #[test]
    fn a_guessed_catalogue_still_picks_by_tier() {
        let mut near = near_catalogue(Protocol::OpenAiChat);
        near.catalogue_from_provider = false;
        let decision = Policy::new()
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek("qwen/qwen3.7-max"),
                &[near],
                t(0),
            )
            .expect("routes");
        assert_eq!(decision.model.as_deref(), Some("anthropic/claude-fable-5"));
    }

    /// A priced catalogue tiers models by price, not by name, so the tier the
    /// request's name hints at can be empty on a backend that lists the exact
    /// model. The fallback pick then reads as a descent, and that must not veto
    /// serving the listed model.
    #[test]
    fn a_listed_model_is_served_when_its_price_tier_differs_from_its_name() {
        let mut near = candidate("nearai", BackendKind::Credits, Protocol::OpenAiChat);
        near.models = vec![
            (
                "anthropic/claude-sonnet-5-5".to_string(),
                ModelTier::Frontier,
            ),
            ("qwen/qwen3-small".to_string(), ModelTier::Fast),
        ];
        let decision = Policy::new()
            .decide(
                key(),
                Protocol::OpenAiChat,
                &peek("anthropic/claude-sonnet-5-5"),
                &[near],
                t(0),
            )
            .expect("routes");
        assert_eq!(decision.model, None, "{decision:?}");
        assert_eq!(decision.rung, Rung::Preferred);
    }

    /// The same gap through the sticky route: a first turn that fell back to a
    /// smaller model stored that rung, and later turns replayed it.
    #[test]
    fn a_fallback_on_the_first_turn_does_not_stick_to_later_listed_models() {
        let mut sub = candidate(
            "claude-key",
            BackendKind::ApiKey,
            Protocol::AnthropicMessages,
        );
        sub.models = vec![
            ("claude-sonnet-4-6".to_string(), ModelTier::Balanced),
            ("claude-haiku-4-5".to_string(), ModelTier::Fast),
        ];
        let mut policy = Policy::new();
        let first = policy
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek("claude-opus-9"),
                std::slice::from_ref(&sub),
                t(0),
            )
            .expect("routes");
        assert_eq!(first.rung, Rung::SmallerModel);

        let second = policy
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek("claude-haiku-4-5"),
                &[sub],
                t(5),
            )
            .expect("routes");
        assert_eq!(second.model, None, "{second:?}");
        assert_eq!(second.rung, Rung::Preferred);
    }

    /// Only the chosen backend's catalogue counts. Another backend listing the
    /// name says nothing about whether this one can serve it.
    #[test]
    fn a_model_listed_by_another_backend_is_not_forwarded_to_this_one() {
        let mut key_backend = candidate(
            "claude-key",
            BackendKind::ApiKey,
            Protocol::AnthropicMessages,
        );
        key_backend.models = vec![("claude-opus-4-6".to_string(), ModelTier::Frontier)];
        let decision = Policy::new()
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek("qwen/qwen3.7-max"),
                &[key_backend, near_catalogue(Protocol::OpenAiChat)],
                t(0),
            )
            .expect("routes");
        assert_eq!(decision.backend.as_str(), "claude-key");
        assert_eq!(decision.model.as_deref(), Some("claude-opus-4-6"));
    }

    /// A name the provider does not list is still a hint, mapped to a tier.
    #[test]
    fn an_unlisted_model_is_still_a_tier_hint() {
        let decision = Policy::new()
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek("claude-opus-5-5"),
                &[near_catalogue(Protocol::OpenAiChat)],
                t(0),
            )
            .expect("routes");
        assert_eq!(decision.model.as_deref(), Some("anthropic/claude-fable-5"));
    }

    /// The prompt-cache rule protects a cache that exists. A cache-control
    /// marker says a prefix *may* be cached, not that it is: Claude Code marks
    /// its system prompt on every request, the first one included, and that
    /// prompt alone is far over the threshold. Reading the marker as a warm
    /// cache refused every Claude Code request to a backend without prompt
    /// caching, so a user whose only backend was NEAR AI got a 503 on "hello".
    mod prompt_cache {
        use super::*;

        /// A first turn as Claude Code sends it: a large prefix behind a
        /// breakpoint, and no turn before it to have warmed anything.
        fn claude_code_turn() -> RequestPeek {
            let mut p = peek("claude-sonnet-4-6");
            p.requirements.prompt_cache = true;
            p.requirements.cached_prefix_tokens = 30_000;
            p
        }

        fn nearai() -> Candidate {
            let mut c = candidate("nearai", BackendKind::Credits, Protocol::OpenAiChat);
            c.caps.prompt_cache = false;
            c
        }

        fn claude_sub() -> Candidate {
            candidate(
                "claude-sub",
                BackendKind::Subscription,
                Protocol::AnthropicMessages,
            )
        }

        fn route(
            policy: &mut Policy,
            candidates: &[Candidate],
            at: DateTime<Utc>,
        ) -> Result<RouteDecision, NoRoute> {
            policy.decide(
                key(),
                Protocol::AnthropicMessages,
                &claude_code_turn(),
                candidates,
                at,
            )
        }

        #[test]
        fn a_first_turn_reaches_a_backend_without_prompt_caching() {
            let mut policy = Policy::new();
            let decision = route(&mut policy, &[nearai()], t(0)).expect("nothing is cached yet");
            assert_eq!(decision.backend.as_str(), "nearai");
            assert_eq!(decision.rung, Rung::CrossFamily);
            assert!(decision.translated);
        }

        #[test]
        fn a_pinned_first_turn_reaches_a_backend_without_prompt_caching() {
            let mut policy = Policy::new();
            policy.set_pin(Some(BackendId::from("nearai")), None);
            let decision =
                route(&mut policy, &[claude_sub(), nearai()], t(0)).expect("nothing is cached yet");
            assert_eq!(decision.backend.as_str(), "nearai");
            assert!(decision.translated);
        }

        #[test]
        fn a_warm_conversation_still_will_not_discard_its_cache() {
            let mut policy = Policy::new();
            let first = route(&mut policy, &[claude_sub(), nearai()], t(0)).expect("routes");
            assert_eq!(first.backend.as_str(), "claude-sub");

            let mut down = claude_sub();
            down.healthy = false;
            let refused = route(&mut policy, &[down, nearai()], t(60))
                .expect_err("moving would discard a warm cache");
            let NoRoute::NoneUsable { refusals } = refused else {
                panic!("expected NoneUsable, got {refused:?}");
            };
            assert!(refusals.iter().any(|(id, why)| id.as_str() == "nearai"
                && *why == Refusal::Ineligible(Ineligible::WouldDiscardLargePromptCache)));
        }

        #[test]
        fn a_conversation_whose_cache_holder_is_gone_has_no_cache_to_protect() {
            let mut policy = Policy::new();
            let first = route(&mut policy, &[claude_sub(), nearai()], t(0)).expect("routes");
            assert_eq!(first.backend.as_str(), "claude-sub");

            let decision = route(&mut policy, &[nearai()], t(60))
                .expect("the cache went with the backend that held it");
            assert_eq!(decision.backend.as_str(), "nearai");
        }

        #[test]
        fn a_conversation_on_a_backend_without_caching_has_nothing_to_discard() {
            let mut policy = Policy::new();
            let mut uncached = claude_sub();
            uncached.caps.prompt_cache = false;
            let first = route(&mut policy, &[uncached.clone(), nearai()], t(0)).expect("routes");
            assert_eq!(first.backend.as_str(), "claude-sub");

            uncached.healthy = false;
            let decision =
                route(&mut policy, &[uncached, nearai()], t(60)).expect("no cache was ever warmed");
            assert_eq!(decision.backend.as_str(), "nearai");
        }
    }
}

#[cfg(test)]
mod compaction_tests {
    //! `docs/PROTOCOL.md` §8 — a compaction turn's output *becomes* the
    //! conversation, so fidelity outranks cost on exactly that turn.

    use super::*;
    use crate::protocol::Wires;
    use crate::quota::Headroom;

    fn t(offset: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + offset, 0).expect("valid timestamp")
    }

    fn caps(protocol: Protocol) -> Capabilities {
        Capabilities {
            wires: Wires::only(protocol),
            tools: true,
            parallel_tool_calls: true,
            images: true,
            reasoning: true,
            prompt_cache: true,
            structured_output: true,
            context_tokens: 200_000,
            unsupported_responses_tools: Vec::new(),
        }
    }

    /// A backend offering exactly the models it is given.
    fn backend(
        id: &str,
        kind: BackendKind,
        protocol: Protocol,
        models: &[(&str, ModelTier)],
    ) -> Candidate {
        Candidate {
            id: BackendId::from(id),
            kind,
            caps: caps(protocol),
            quota: QuotaSnapshot::default(),
            healthy: true,
            account_scope: Some(format!("scope-{id}")),
            consented: true,
            required_client_identity: None,
            default_login: false,
            models: models
                .iter()
                .map(|(m, tier)| ((*m).to_string(), *tier))
                .collect(),
            // These fixtures state what a backend really offers, so they stand
            // for a catalogue the provider gave us, not one we guessed.
            catalogue_from_provider: true,
            trusted: true,
        }
    }

    fn peek(compaction: bool) -> RequestPeek {
        RequestPeek {
            requested_model: Some("claude-opus-4-6".to_string()),
            stream: true,
            requirements: RequestRequirements::default(),
            client_identity: Some(ClientIdentity::ClaudeCode),
            message_count: 40,
            likely_compaction: compaction,
            client_login: None,
            client_session: None,
        }
    }

    fn key() -> ConversationKey {
        ConversationKey::derive(
            Protocol::AnthropicMessages,
            "You are Claude Code",
            &["Read"],
        )
    }

    /// A free subscription that can only serve a lesser model, and a metered
    /// key that can serve the one the client asked for. The interesting case:
    /// the two orderings disagree.
    fn split_candidates() -> Vec<Candidate> {
        vec![
            backend(
                "claude-sub",
                BackendKind::Subscription,
                Protocol::AnthropicMessages,
                &[("claude-sonnet-4-6", ModelTier::Balanced)],
            ),
            backend(
                "anthropic-key",
                BackendKind::ApiKey,
                Protocol::AnthropicMessages,
                &[("claude-opus-4-6", ModelTier::Frontier)],
            ),
        ]
    }

    #[test]
    fn an_ordinary_turn_prefers_free_capacity_over_the_exact_model() {
        // Unchanged behaviour, asserted so the compaction change cannot quietly
        // alter what every other turn does.
        let mut policy = Policy::new();
        let decision = policy
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek(false),
                &split_candidates(),
                t(0),
            )
            .expect("routes");
        assert_eq!(decision.backend.as_str(), "claude-sub");
    }

    #[test]
    fn a_compaction_turn_pays_for_the_model_the_client_asked_for() {
        // The summary is written into the client's permanent history and
        // resent every turn afterwards. A cheaper summary is not a saving.
        let mut policy = Policy::new();
        let decision = policy
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek(true),
                &split_candidates(),
                t(0),
            )
            .expect("routes");
        assert_eq!(decision.backend.as_str(), "anthropic-key");
        assert_eq!(decision.rung, Rung::Preferred);
    }

    #[test]
    fn a_compaction_turn_still_avoids_pressured_capacity_first() {
        // Fidelity outranks cost, not availability. Sending a compaction turn
        // at an exhausted backend fails the turn, which is strictly worse than
        // a cheaper summary.
        let mut candidates = split_candidates();
        candidates[1].quota.primary = Headroom::Exhausted { until: t(3600) };
        let mut policy = Policy::new();
        let decision = policy
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek(true),
                &candidates,
                t(0),
            )
            .expect("routes");
        assert_eq!(decision.backend.as_str(), "claude-sub");
    }

    #[test]
    fn a_compaction_turn_leaves_a_degraded_affinity_behind() {
        // A conversation that descended earlier is sitting on degraded
        // capacity. Inheriting that for the one turn whose output is permanent
        // is the mistake this rule exists to prevent.
        let mut policy = Policy::new();

        // Establish an affinity on the cheaper backend via an ordinary turn.
        let first = policy
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek(false),
                &split_candidates(),
                t(0),
            )
            .expect("routes");
        assert_eq!(first.backend.as_str(), "claude-sub");
        assert_eq!(first.rung, Rung::SmallerModel, "the affinity is degraded");

        let compaction = policy
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek(true),
                &split_candidates(),
                t(60),
            )
            .expect("routes");
        assert_eq!(
            compaction.backend.as_str(),
            "anthropic-key",
            "a compaction turn must not inherit a degraded affinity"
        );
    }

    #[test]
    fn a_compaction_turn_keeps_an_undegraded_affinity() {
        // The rule is about *degraded* affinity. A conversation already on a
        // full-fidelity backend should stay there — moving would throw away a
        // warm prompt cache for nothing.
        let candidates = vec![backend(
            "claude-sub",
            BackendKind::Subscription,
            Protocol::AnthropicMessages,
            &[("claude-opus-4-6", ModelTier::Frontier)],
        )];
        let mut policy = Policy::new();
        let first = policy
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek(false),
                &candidates,
                t(0),
            )
            .expect("routes");
        assert_eq!(first.rung, Rung::Preferred);

        let compaction = policy
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek(true),
                &candidates,
                t(60),
            )
            .expect("routes");
        assert_eq!(compaction.backend.as_str(), "claude-sub");
        assert_eq!(compaction.reason, "sticky affinity");
    }

    #[test]
    fn a_compaction_turn_is_still_served_when_only_a_foreign_family_is_left() {
        // Fidelity is a preference, not a veto. Refusing the turn would leave
        // the user unable to compact at all, which means the session cannot
        // continue — strictly worse than a summary from another family.
        let candidates = vec![backend(
            "nearai",
            BackendKind::Credits,
            Protocol::OpenAiChat,
            &[("deepseek-v3", ModelTier::Frontier)],
        )];
        let mut policy = Policy::new();
        let decision = policy
            .decide(
                key(),
                Protocol::AnthropicMessages,
                &peek(true),
                &candidates,
                t(0),
            )
            .expect("a compaction turn must still be served");
        assert_eq!(decision.rung, Rung::CrossFamily);
        assert!(decision.translated);
    }
}
