//! Change the NEAR AI login an account uses (F10).
//!
//! Spec: `docs/superpowers/specs/2026-11-03-f10-change-account-design.md`,
//! with the owner's decisions of 2026-11-03. Wire contract:
//! [`trace_commons_protocol::account_login_change`].
//!
//! # The ceremony
//!
//! One sheet, three steps, driven by `account_login_change { step }`:
//!
//! 1. **Confirm it's you** (`prove_current`). A NEAR AI account signs in
//!    again with the login it used last time: the browser chooser offers
//!    only that login, marked (decision 1). The access token is held in
//!    memory for this ceremony and never stored; its identity must be the
//!    stored login's, or the step fails `current_login`. A wallet-origin
//!    account (`near-` tenant) signs the commons' change message with its
//!    NEAR wallet on the Commons wallet page (W1, decision 2): the commons'
//!    `start` comes first here, because the wallet signs its nonce.
//! 2. **Sign in with the new account** (`sign_in_new`), skipped when a
//!    login the commons refused as another account is held aside. The
//!    chooser offers every login, none marked. This device's inference key
//!    is minted in the new account and held, unstored.
//! 3. **Confirm** (`confirm`). For a NEAR AI account the commons' `start`
//!    and `finish` run back to back now, so the 15-minute ceremony is not
//!    spent while the person is in the browser. The finish carries both
//!    proofs and this device's signature over them.
//!
//! After the commons answers, the device re-binds: the new login replaces
//! the stored NEAR AI credential (`ceremony::persist_change`), the account's
//! login and identity are recorded, the config records that the account now
//! signs in with a NEAR AI login, and the old key is revoked with the old
//! session, best effort (a failed revoke stays pending, tagged with the old
//! identity). The account session is kept: the commons mints none, and the
//! tenant and account do not change.
//!
//! # What is never kept
//!
//! Neither access token nor the wallet signature is written, logged or
//! returned over IPC. The held-aside login lives in memory only; its key is
//! revoked when the hold ends without a change (a cancel, an expiry, a
//! newer login held aside). Every answer and refusal is a fixed label.
//!
//! # What this does not use
//!
//! `commons_credentials_switch::switch_identity` (decision 12 names it) is
//! not used: it replaces the device key, the config and the account session
//! together, and a change keeps all three. The NEAR AI credential's own
//! commit (`cloud_credential_lifecycle::replace`) is the one that moves.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};

use trace_commons_protocol::account_login_change::{
    self as wire, CurrentProof, IdentityKind, RebindFinishRequest, RebindFinishResponse,
    RebindStartRequest, RebindStartResponse,
};

use super::ipc::{DaemonShared, ERR_BAD_PARAMS, ERR_UNAVAILABLE, Request, Response};
use super::nearai_credential::api::{CloudApi, MintedKey};
use super::nearai_credential::ceremony::UnstoredKey;
use super::nearai_credential::loopback::SessionTokens;
use crate::config::{ConfigStore, ContributorConfig};
use crate::identity::DeviceIdentity;

#[cfg(test)]
#[path = "account_login_change_tests.rs"]
mod tests;

/// How long a login the commons refused is held aside for a change. A
/// person who saw the mismatch block and walks away does not leave a key
/// minted in the other account for longer than this.
const HOLD_TTL: Duration = Duration::from_secs(15 * 60);

/// How long a change ceremony lives on this device, both browser legs and
/// the person's reading included. The commons' own ceremony
/// ([`wire::REBIND_CEREMONY_TTL_SECS`]) is shorter and is started as late
/// as the proof allows.
const CEREMONY_TTL: Duration = Duration::from_secs(30 * 60);

/// A wallet challenge whose commons ceremony has less than this left is
/// started again rather than finished: the finish would be refused.
const WALLET_FINISH_MARGIN_SECS: i64 = 30;

// Errors a step answers in the ceremony's state, each the suffix of an
// approved string `account.change.error.<label>` (spec section 2.2).

/// The NEAR AI sign-in that should prove the current identity was another
/// login (`account.change.error.current_login`).
pub const STEP_ERROR_CURRENT_LOGIN: &str = "current_login";
/// The wallet did not confirm (`account.change.error.wallet`).
pub const STEP_ERROR_WALLET: &str = "wallet";
/// The new login already holds an account (`account.change.error.in_use`).
pub const STEP_ERROR_IN_USE: &str = "in_use";
/// The account changed its login within the cooldown
/// (`account.change.error.too_soon`).
pub const STEP_ERROR_TOO_SOON: &str = "too_soon";
/// Anything else (`account.change.error.unavailable`).
pub const STEP_ERROR_UNAVAILABLE: &str = "unavailable";

// IPC refusals, as fixed labels.

/// This account cannot change its login from this device: not an account
/// namespace, a device that joined by code (decision 8), a revoked key, or
/// a commons that does not publish the change routes.
pub const ERR_NOT_AVAILABLE: &str = "account_login_change_not_available";
/// No change ceremony is running, or the caller named another one.
pub const ERR_NO_CEREMONY: &str = "account_login_change_no_ceremony";
/// The step asked for is not the one the ceremony is at, or a browser leg
/// or a commons call is already running.
pub const ERR_OUT_OF_ORDER: &str = "account_login_change_out_of_order";
/// A wallet-origin account's proof needs the NEAR account name, typed in
/// the sheet as at sign-up.
pub const ERR_WALLET_ACCOUNT_REQUIRED: &str = "account_login_change_wallet_account_required";
/// No account session to present: the account is signed out.
pub const ERR_ACCOUNT_SESSION_REQUIRED: &str = "account-session-required";

/// A login the commons refused as another NEAR AI account, or one signed in
/// for step 2, held unstored for a change. Dropped, its key is revoked
/// ([`UnstoredKey`]). Never logged: `Debug` names nothing secret.
pub(crate) struct HeldLogin {
    pub(crate) tokens: SessionTokens,
    pub(crate) expires_at: Option<DateTime<Utc>>,
    pub(crate) user_agent: String,
    pub(crate) provider: Option<&'static str>,
    pub(crate) minted: MintedKey,
    /// [`super::nearai_credential::spend_org::identity_digest`] of the
    /// login's NEAR AI user, when it could be read.
    pub(crate) identity: Option<String>,
    pub(crate) unstored: UnstoredKey,
}

impl std::fmt::Debug for HeldLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeldLogin")
            .field("provider", &self.provider)
            .finish_non_exhaustive()
    }
}

struct Held {
    login: HeldLogin,
    at: Instant,
}

/// The commons' half of a change ceremony, as this device holds it.
struct CommonsCeremony {
    ceremony_id: String,
    nonce: [u8; 32],
    expires_at: i64,
    verifier: String,
    challenge: String,
}

enum CurrentProofHeld {
    NearAi {
        access_token: String,
    },
    Wallet {
        account_id: String,
        wallet_public_key: String,
        wallet_signature: String,
        commons: CommonsCeremony,
    },
}

/// A wallet challenge the browser is signing, kept until its callback.
struct WalletPending {
    account_id: String,
    commons: CommonsCeremony,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    /// Confirm it's you: prove the current identity.
    Current,
    /// Sign in with the new account.
    New,
    /// Both proofs in hand: the person confirms.
    Confirm,
    /// The account moved to the new login.
    Done,
}

struct Ceremony {
    id: String,
    kind: IdentityKind,
    old_provider: Option<&'static str>,
    old_identity: Option<String>,
    current: Option<CurrentProofHeld>,
    wallet_pending: Option<WalletPending>,
    new_login: Option<HeldLogin>,
    error: Option<&'static str>,
    leg: Option<tokio::task::AbortHandle>,
    busy: bool,
    done: bool,
    started: Instant,
}

impl Ceremony {
    fn step(&self) -> Step {
        if self.done {
            Step::Done
        } else if self.current.is_none() {
            Step::Current
        } else if self.new_login.is_none() {
            Step::New
        } else {
            Step::Confirm
        }
    }

    fn view(&self) -> serde_json::Value {
        serde_json::json!({
            "change_id": self.id,
            "step": self.step(),
            "current_kind": self.kind,
            "old_provider": self.old_provider,
            "new_provider": self.new_login.as_ref().and_then(|login| login.provider),
            "busy": self.busy,
            "error": self.error,
        })
    }

    /// End a browser leg still running.
    fn stop_leg(&mut self) {
        if let Some(leg) = self.leg.take() {
            leg.abort();
        }
        self.busy = false;
    }
}

impl Drop for Ceremony {
    fn drop(&mut self) {
        self.stop_leg();
    }
}

#[derive(Default)]
struct Slot {
    held: Option<Held>,
    ceremony: Option<Ceremony>,
}

static STATE: Mutex<Option<HashMap<PathBuf, Slot>>> = Mutex::new(None);

/// Whether the commons was seen to publish the change routes, per state
/// directory: `Some(false)` once a start found them missing. Status reads
/// this and nothing on the network.
static ROUTES: Mutex<Option<HashMap<PathBuf, bool>>> = Mutex::new(None);

fn with_slot<T>(dir: &Path, f: impl FnOnce(&mut Slot) -> T) -> T {
    let mut state = STATE.lock().unwrap_or_else(PoisonError::into_inner);
    let map = state.get_or_insert_with(HashMap::new);
    let slot = map.entry(dir.to_path_buf()).or_default();
    // Expired holds and ceremonies go first, whoever asks.
    if slot
        .held
        .as_ref()
        .is_some_and(|held| held.at.elapsed() >= HOLD_TTL)
    {
        slot.held = None;
    }
    if slot
        .ceremony
        .as_ref()
        .is_some_and(|ceremony| ceremony.started.elapsed() >= CEREMONY_TTL)
    {
        slot.ceremony = None;
    }
    f(slot)
}

fn routes_known_missing(dir: &Path) -> bool {
    ROUTES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .and_then(|map| map.get(dir).copied())
        == Some(false)
}

fn note_routes(dir: &Path, published: bool) {
    ROUTES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get_or_insert_with(HashMap::new)
        .insert(dir.to_path_buf(), published);
}

/// Whether this device may offer "Use a different account" (spec 2.4,
/// `login_change_available`): an account in one of the two anchor
/// namespaces, a device that did not join by code (decision 8) and whose
/// key the account has not revoked, and a commons not seen to lack the
/// routes. No network, no credential store.
pub(crate) fn available(store: &ConfigStore, cfg: &ContributorConfig) -> bool {
    cfg.signs_in_with().is_some()
        && !super::device_codes::joined_by_code(store)
        && !super::account_session_renewal::key_revoked_for(store, cfg)
        && !routes_known_missing(store.dir())
}

/// The status fields this module adds (spec 2.4): whether the change is
/// offered, whether a wallet-origin account's payouts need a NEAR AI login,
/// and whether a refused login is held aside for the change.
pub(crate) fn status_fields(
    store: &ConfigStore,
    cfg: Option<&ContributorConfig>,
) -> serde_json::Value {
    let available = cfg.is_some_and(|cfg| available(store, cfg));
    let payouts_need_login =
        cfg.is_some_and(|cfg| cfg.signs_in_with() == Some(IdentityKind::Wallet));
    let held = with_slot(store.dir(), |slot| slot.held.is_some());
    serde_json::json!({
        "login_change_available": available,
        "payouts_need_login": payouts_need_login,
        "login_change_held": available && held,
    })
}

/// Hold a login the commons refused as another account aside for a change
/// (spec 2.1, entry point 1). Answers it back when the change is not
/// offered here, for the caller to revoke at once. A login held before is
/// replaced, and its key revoked as it drops.
pub(crate) fn hold_aside(dir: &Path, login: HeldLogin) -> Option<HeldLogin> {
    let offered = ConfigStore::open(dir.to_path_buf())
        .ok()
        .and_then(|store| store.load_config().ok().flatten().map(|cfg| (store, cfg)))
        .is_some_and(|(store, cfg)| available(&store, &cfg));
    if !offered {
        return Some(login);
    }
    with_slot(dir, |slot| {
        slot.held = Some(Held {
            login,
            at: Instant::now(),
        });
    });
    None
}

/// Drop whatever this directory holds for a change: the ceremony and the
/// login held aside, revoking keys minted for them. Sign-out and unenroll
/// call this too, so a held login never outlives the account.
pub(crate) fn forget(dir: &Path) {
    let removed = with_slot(dir, |slot| (slot.held.take(), slot.ceremony.take()));
    drop(removed);
}

// ---- IPC ----

pub(super) async fn handle(shared: &DaemonShared, req: &Request) -> Response {
    let step = req.params.get("step").and_then(serde_json::Value::as_str);
    let change_id = req
        .params
        .get("change_id")
        .and_then(serde_json::Value::as_str);
    match step {
        Some("status") => status(shared, req, change_id),
        Some("start") => start(shared, req),
        Some("prove_current") => prove_current(shared, req, change_id).await,
        Some("sign_in_new") => sign_in_new(shared, req, change_id).await,
        Some("confirm") => match CloudApi::live() {
            Ok(api) => confirm(shared, req, change_id, &api).await,
            Err(_) => Response::err(req.id, ERR_UNAVAILABLE, ERR_NOT_AVAILABLE),
        },
        _ => Response::err(req.id, ERR_BAD_PARAMS, "step"),
    }
}

pub(super) fn handle_cancel(shared: &DaemonShared, req: &Request) -> Response {
    let change_id = req
        .params
        .get("change_id")
        .and_then(serde_json::Value::as_str);
    let dir = shared.store.dir();
    let cancelled = with_slot(dir, |slot| {
        let named = slot
            .ceremony
            .as_ref()
            .is_some_and(|ceremony| change_id.is_none_or(|id| id == ceremony.id));
        // The held login goes too: cancelling the change is declining it.
        let ceremony = if named { slot.ceremony.take() } else { None };
        (ceremony, slot.held.take())
    });
    let had = cancelled.0.is_some() || cancelled.1.is_some();
    drop(cancelled);
    Response::ok(req.id, serde_json::json!({ "cancelled": had }))
}

fn status(shared: &DaemonShared, req: &Request, change_id: Option<&str>) -> Response {
    let view = with_slot(shared.store.dir(), |slot| {
        slot.ceremony
            .as_ref()
            .filter(|ceremony| change_id.is_none_or(|id| id == ceremony.id))
            .map(Ceremony::view)
    });
    match view {
        Some(view) => Response::ok(req.id, view),
        None => Response::err(req.id, ERR_UNAVAILABLE, ERR_NO_CEREMONY),
    }
}

/// Start a change. A login held aside moves into the ceremony, so step 2 is
/// skipped. A ceremony already running is answered as it stands.
fn start(shared: &DaemonShared, req: &Request) -> Response {
    let store = &shared.store;
    let Ok(Some(cfg)) = store.load_config() else {
        return Response::err(req.id, ERR_UNAVAILABLE, ERR_NOT_AVAILABLE);
    };
    if !available(store, &cfg) {
        return Response::err(req.id, ERR_UNAVAILABLE, ERR_NOT_AVAILABLE);
    }
    let Some(kind) = cfg.signs_in_with() else {
        return Response::err(req.id, ERR_UNAVAILABLE, ERR_NOT_AVAILABLE);
    };
    let old_provider = super::nearai_credential::login_record::commons_login(store);
    let old_identity = super::nearai_credential::spend_org::last_identity(store);
    let Ok(id) = super::account_onboarding::random() else {
        return Response::err(req.id, ERR_UNAVAILABLE, ERR_NOT_AVAILABLE);
    };
    let view = with_slot(store.dir(), |slot| {
        if let Some(ceremony) = slot.ceremony.as_ref().filter(|c| !c.done) {
            return ceremony.view();
        }
        let ceremony = Ceremony {
            id,
            kind,
            old_provider,
            old_identity,
            current: None,
            wallet_pending: None,
            new_login: slot.held.take().map(|held| held.login),
            error: None,
            leg: None,
            busy: false,
            done: false,
            started: Instant::now(),
        };
        let view = ceremony.view();
        slot.ceremony = Some(ceremony);
        view
    });
    Response::ok(req.id, view)
}

/// Mark the named ceremony busy for a step, if it is at `expected` and not
/// already busy. Answers what the step needs to know.
fn claim(
    dir: &Path,
    change_id: Option<&str>,
    expected: Step,
) -> std::result::Result<(String, IdentityKind, Option<&'static str>), &'static str> {
    with_slot(dir, |slot| {
        let ceremony = slot
            .ceremony
            .as_mut()
            .filter(|ceremony| change_id.is_none_or(|id| id == ceremony.id))
            .ok_or(ERR_NO_CEREMONY)?;
        if ceremony.busy || ceremony.step() != expected {
            return Err(ERR_OUT_OF_ORDER);
        }
        ceremony.busy = true;
        ceremony.error = None;
        Ok((ceremony.id.clone(), ceremony.kind, ceremony.old_provider))
    })
}

/// Settle a step on the named ceremony, if it is still the one running and
/// the step was claimed ([`claim`]): an outcome nobody is waiting for (a
/// late browser callback after a cancel and a new start) lands nowhere.
fn settle(dir: &Path, id: &str, f: impl FnOnce(&mut Ceremony)) {
    with_slot(dir, |slot| {
        if let Some(ceremony) = slot.ceremony.as_mut().filter(|c| c.id == id && c.busy) {
            ceremony.busy = false;
            ceremony.leg = None;
            f(ceremony);
        }
    });
}

fn set_leg(dir: &Path, id: &str, leg: tokio::task::AbortHandle) {
    with_slot(dir, |slot| {
        match slot.ceremony.as_mut().filter(|c| c.id == id) {
            Some(ceremony) if ceremony.busy => ceremony.leg = Some(leg),
            // The leg already settled (or the ceremony is gone): nothing to keep.
            _ => {}
        }
    });
}

fn view_of(dir: &Path, id: &str) -> serde_json::Value {
    with_slot(dir, |slot| {
        slot.ceremony
            .as_ref()
            .filter(|c| c.id == id)
            .map(Ceremony::view)
            .unwrap_or(serde_json::Value::Null)
    })
}

/// The login this build offers as `provider`, as a one-element slice of the
/// build's own list, so the chooser can offer it alone.
fn offered_only(provider: &str) -> Option<&'static [&'static str]> {
    let all = crate::channel::NEAR_AI_SIGN_IN_PROVIDERS;
    all.iter()
        .position(|offered| *offered == provider)
        .map(|index| &all[index..=index])
}

/// Step 1: prove the current identity. Begins a browser leg and answers its
/// URL with the ceremony's state.
async fn prove_current(shared: &DaemonShared, req: &Request, change_id: Option<&str>) -> Response {
    let dir = shared.store.dir().to_path_buf();
    let (id, kind, old_provider) = match claim(&dir, change_id, Step::Current) {
        Ok(claimed) => claimed,
        Err(label) => return Response::err(req.id, ERR_UNAVAILABLE, label),
    };
    let begun = match kind {
        IdentityKind::NearAiLogin => begin_current_near_ai(&shared.store, &id, old_provider).await,
        IdentityKind::Wallet => {
            let account = req
                .params
                .get("wallet_account_id")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|account| !account.is_empty());
            match account {
                Some(account) => begin_current_wallet(shared, &id, account).await,
                None => Err(ERR_WALLET_ACCOUNT_REQUIRED),
            }
        }
    };
    match begun {
        Ok((browser_url, leg)) => {
            set_leg(&dir, &id, leg);
            let mut view = view_of(&dir, &id);
            view["browser_url"] = browser_url.into();
            Response::ok(req.id, view)
        }
        Err(label) => {
            settle(&dir, &id, |ceremony| {
                if label != ERR_WALLET_ACCOUNT_REQUIRED {
                    ceremony.error = Some(step_error(label));
                }
            });
            Response::err(req.id, ERR_UNAVAILABLE, label)
        }
    }
}

/// The NEAR AI current-identity leg: the chooser offers only the login used
/// last time, marked; any login when none is remembered. The session is
/// checked against the stored login's identity and held, unstored.
async fn begin_current_near_ai(
    store: &ConfigStore,
    id: &str,
    old_provider: Option<&'static str>,
) -> std::result::Result<(String, tokio::task::AbortHandle), &'static str> {
    let offered = old_provider
        .and_then(offered_only)
        .unwrap_or(crate::channel::NEAR_AI_SIGN_IN_PROVIDERS);
    let dir = store.dir().to_path_buf();
    let id_owned = id.to_owned();
    let leg_store = store.clone();
    super::nearai_credential::ceremony::begin_change_leg(
        offered,
        old_provider,
        move |api, outcome| async move {
            let proof = match outcome {
                Ok((tokens, _, _, _)) => {
                    let identity = super::nearai_credential::spend_org::read_identity(
                        &leg_store, &api, &tokens,
                    )
                    .await;
                    Some((tokens.access_token, identity))
                }
                Err(_) => None,
            };
            record_current_near_ai(&dir, &id_owned, proof);
        },
    )
    .await
    .map_err(|_| ERR_NOT_AVAILABLE)
}

/// Record the outcome of the NEAR AI current-identity leg. `None`: the leg
/// ended without a session (cancelled, timed out, failed). A session whose
/// identity is known to differ from the stored login's is refused
/// `current_login`; an identity that cannot be read on either side is left
/// to the commons, which refuses a token that does not anchor to the
/// account's login.
pub(crate) fn record_current_near_ai(
    dir: &Path,
    id: &str,
    proof: Option<(String, Option<String>)>,
) {
    settle(dir, id, |ceremony| match proof {
        None => {}
        Some((access_token, identity)) => {
            let mismatch = matches!(
                (&ceremony.old_identity, &identity),
                (Some(old), Some(new)) if old != new
            );
            if mismatch {
                ceremony.error = Some(STEP_ERROR_CURRENT_LOGIN);
            } else {
                ceremony.current = Some(CurrentProofHeld::NearAi { access_token });
            }
        }
    });
}

/// The wallet current-identity leg: the commons' start (its nonce is what
/// the wallet signs), the message checked against the one this device
/// expects, then the Commons wallet page with a loopback callback.
async fn begin_current_wallet(
    shared: &DaemonShared,
    id: &str,
    account: &str,
) -> std::result::Result<(String, tokio::task::AbortHandle), &'static str> {
    let (cfg, mut session, identity) = commons_inputs(shared).await?;
    let (started, commons) =
        commons_start(shared, &cfg, &mut session, &identity, Some(account)).await?;
    let Some(challenge) = started.wallet.as_ref() else {
        return Err(ERR_NOT_AVAILABLE);
    };
    let device = device_bytes(&identity).map_err(|_| ERR_NOT_AVAILABLE)?;
    // The sign-up leg's hardening: the wallet signs only the message this
    // device expects, so a commons cannot have it sign something else.
    let expected =
        wire::rebind_wallet_message(&challenge.network, account, &device, started.expires_at);
    let wallet_url = reqwest::Url::parse(&challenge.wallet_url).map_err(|_| ERR_NOT_AVAILABLE)?;
    let base = reqwest::Url::parse(&cfg.ingest_url).map_err(|_| ERR_NOT_AVAILABLE)?;
    if started.current_kind != IdentityKind::Wallet
        || challenge.message != expected
        || wallet_url.origin() != base.origin()
        || wallet_url.path() != "/account/near/provision/wallet"
        || wallet_url.query().is_some()
        || wallet_url.fragment().is_some()
        || !matches!(challenge.network.as_str(), "mainnet" | "testnet")
    {
        return Err(ERR_NOT_AVAILABLE);
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|_| ERR_NOT_AVAILABLE)?;
    let port = listener.local_addr().map_err(|_| ERR_NOT_AVAILABLE)?.port();
    let callback = format!(
        "http://127.0.0.1:{port}{}",
        super::account_onboarding::LOOPBACK_PATH
    );
    let state = super::account_onboarding::random().map_err(|_| ERR_NOT_AVAILABLE)?;
    let fragment = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&serde_json::json!({
            "account_id": account,
            "message": challenge.message,
            "nonce": started.nonce,
            "recipient": challenge.recipient,
            "expires_at": started.expires_at,
            "network": challenge.network,
            "loopback": callback,
            "state": state,
        }))
        .map_err(|_| ERR_NOT_AVAILABLE)?,
    );
    let browser_url = format!("{}#{fragment}", challenge.wallet_url);
    let dir = shared.store.dir().to_path_buf();
    with_slot(&dir, |slot| {
        if let Some(ceremony) = slot.ceremony.as_mut().filter(|c| c.id == id) {
            ceremony.wallet_pending = Some(WalletPending {
                account_id: account.to_owned(),
                commons,
            });
        }
    });
    let listener = listener.into_std().map_err(|_| ERR_NOT_AVAILABLE)?;
    let id_owned = id.to_owned();
    let task = super::account_onboarding::signup_runtime()
        .map_err(|_| ERR_NOT_AVAILABLE)?
        .spawn(async move {
            let received = async {
                let listener = tokio::net::TcpListener::from_std(listener)?;
                tokio::time::timeout(
                    Duration::from_secs(300),
                    super::account_onboarding::receive_wallet(listener, &state),
                )
                .await
                .map_err(|_| anyhow!("near_signup_expired"))?
            }
            .await;
            let proof = received.ok().map(|result| {
                (
                    result.account_id,
                    result.wallet_public_key,
                    result.wallet_signature,
                )
            });
            record_current_wallet(&dir, &id_owned, proof);
        });
    Ok((browser_url, task.abort_handle()))
}

/// Record the wallet's signed callback. A callback for another account name
/// than the one the challenge was for is refused `wallet`.
pub(crate) fn record_current_wallet(dir: &Path, id: &str, proof: Option<(String, String, String)>) {
    settle(dir, id, |ceremony| {
        let Some(pending) = ceremony.wallet_pending.take() else {
            return;
        };
        match proof {
            Some((account_id, wallet_public_key, wallet_signature))
                if account_id == pending.account_id =>
            {
                ceremony.current = Some(CurrentProofHeld::Wallet {
                    account_id,
                    wallet_public_key,
                    wallet_signature,
                    commons: pending.commons,
                });
            }
            Some(_) => ceremony.error = Some(STEP_ERROR_WALLET),
            None => {}
        }
    });
}

/// Step 2: sign in with the new login. Every login is offered, none marked;
/// the key is minted in the new account and held.
async fn sign_in_new(shared: &DaemonShared, req: &Request, change_id: Option<&str>) -> Response {
    let dir = shared.store.dir().to_path_buf();
    let (id, _, _) = match claim(&dir, change_id, Step::New) {
        Ok(claimed) => claimed,
        Err(label) => return Response::err(req.id, ERR_UNAVAILABLE, label),
    };
    let leg_dir = dir.clone();
    let leg_id = id.clone();
    let leg_store = shared.store.clone();
    let begun = super::nearai_credential::ceremony::begin_change_leg(
        crate::channel::NEAR_AI_SIGN_IN_PROVIDERS,
        None,
        move |api, outcome| async move {
            let held = match outcome {
                Ok((tokens, expires_at, user_agent, provider)) => {
                    match super::nearai_credential::ceremony::mint_for_change(
                        &api, &leg_store, &tokens,
                    )
                    .await
                    {
                        Ok((minted, identity, unstored)) => Some(Ok(HeldLogin {
                            tokens,
                            expires_at,
                            user_agent,
                            provider: crate::channel::NEAR_AI_SIGN_IN_PROVIDERS
                                .iter()
                                .find(|offered| **offered == provider)
                                .copied(),
                            minted,
                            identity,
                            unstored,
                        })),
                        Err(_) => Some(Err(())),
                    }
                }
                Err(_) => None,
            };
            record_new_login(&leg_dir, &leg_id, held);
        },
    )
    .await;
    match begun {
        Ok((browser_url, leg)) => {
            set_leg(&dir, &id, leg);
            let mut view = view_of(&dir, &id);
            view["browser_url"] = browser_url.into();
            Response::ok(req.id, view)
        }
        Err(_) => {
            settle(&dir, &id, |ceremony| {
                ceremony.error = Some(STEP_ERROR_UNAVAILABLE);
            });
            Response::err(req.id, ERR_UNAVAILABLE, ERR_NOT_AVAILABLE)
        }
    }
}

/// Record the new login. `None`: the leg ended without a session.
/// `Some(Err)`: signed in, but no key could be minted. A login that is the
/// current identity again is not a new one, and is refused `in_use`.
pub(crate) fn record_new_login(
    dir: &Path,
    id: &str,
    held: Option<std::result::Result<HeldLogin, ()>>,
) {
    let mut refused = None;
    settle(dir, id, |ceremony| match held {
        None => {}
        Some(Err(())) => ceremony.error = Some(STEP_ERROR_UNAVAILABLE),
        Some(Ok(login)) => {
            let same = matches!(
                (&ceremony.old_identity, &login.identity),
                (Some(old), Some(new)) if old == new
            );
            if same {
                ceremony.error = Some(STEP_ERROR_IN_USE);
                refused = Some(login);
            } else {
                ceremony.new_login = Some(login);
            }
        }
    });
    // Revoked as it drops, outside the state lock.
    drop(refused);
}

/// What the commons calls need: the config, the account session, and this
/// device's key.
async fn commons_inputs(
    shared: &DaemonShared,
) -> std::result::Result<
    (
        ContributorConfig,
        crate::account_auth::LoadedAccountSession,
        DeviceIdentity,
    ),
    &'static str,
> {
    // An expired session is renewed first, as every account feature does.
    let api = CloudApi::live().map_err(|_| STEP_ERROR_UNAVAILABLE)?;
    let via = super::account_session_renewal::RenewVia::live(&api);
    let session = match super::account_session_renewal::account_session(shared, Some(&via)).await {
        Ok(Some(session)) => session,
        Ok(None) => return Err(ERR_ACCOUNT_SESSION_REQUIRED),
        Err(_) => return Err(STEP_ERROR_UNAVAILABLE),
    };
    let Ok(Some(cfg)) = shared.store.load_config() else {
        return Err(ERR_NOT_AVAILABLE);
    };
    let Ok(Some(identity)) = DeviceIdentity::load(&shared.store) else {
        return Err(ERR_NOT_AVAILABLE);
    };
    if identity.device_key_id != cfg.device_key_id {
        return Err(ERR_NOT_AVAILABLE);
    }
    Ok((cfg, session, identity))
}

fn device_bytes(identity: &DeviceIdentity) -> Result<[u8; 32]> {
    base64::engine::general_purpose::STANDARD
        .decode(&identity.public_key_b64)?
        .try_into()
        .map_err(|_| anyhow!("device_key_invalid"))
}

/// One call to a change route on the account session, keeping the rotated
/// session token the commons answers with.
async fn call<Req, Resp>(
    shared: &DaemonShared,
    cfg: &ContributorConfig,
    session: &mut crate::account_auth::LoadedAccountSession,
    path: &str,
    body: &Req,
) -> std::result::Result<Resp, trace_commons_operator_client::Error>
where
    Req: Serialize,
    Resp: serde::de::DeserializeOwned,
{
    let client = trace_commons_operator_client::Client::builder(
        &cfg.ingest_url,
        "TRACE_COMMONS_CONTRIBUTOR_UNUSED_BEARER_ENV",
    )
    .bearer_token(&session.session.access_token)
    .host_allowlist(crate::config::allowlist_for(cfg.allowed_hosts.as_deref()))
    .timeout(Duration::from_secs(30))
    .build();
    let client = match client {
        Ok(client) => client,
        Err(error) => return Err(error),
    };
    let response = client
        .call_json_with_response_headers::<Req, Resp>(
            reqwest::Method::POST,
            path,
            &[],
            Some(body),
            trace_commons_protocol::ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER,
            trace_commons_protocol::ACCOUNT_NATIVE_SESSION_EXPIRES_AT_HEADER,
        )
        .await;
    let rotated = crate::account_auth::RotatedToken::from_headers(
        response.response_header,
        response.companion_header,
    );
    let _ = super::withdraw::keep_rotated_token(shared, session, rotated);
    response.result
}

/// A commons refusal as a step error: the three it names, a commons that
/// did not answer, and everything else (the uniform deny) as `fallback`.
fn refusal(error: &trace_commons_operator_client::Error, fallback: &'static str) -> &'static str {
    match error.server_label() {
        Some(wire::REBIND_IN_USE) => STEP_ERROR_IN_USE,
        Some(wire::REBIND_TOO_SOON) => STEP_ERROR_TOO_SOON,
        Some(wire::REBIND_PAYOUT_REVIEW_REQUIRED) => {
            // No approved words for it (spec section 10, item 3): the sheet
            // says it could not switch, and the log names why.
            crate::daemon::diagnostics_log::record_labels(
                tracing::Level::WARN,
                "account_login_change_refused",
                &[("reason", wire::REBIND_PAYOUT_REVIEW_REQUIRED)],
            );
            STEP_ERROR_UNAVAILABLE
        }
        Some(wire::REBIND_WALLET_ACCOUNT_REQUIRED) => ERR_WALLET_ACCOUNT_REQUIRED,
        _ if super::nearai_onboarding::commons_unanswered(error) => STEP_ERROR_UNAVAILABLE,
        _ => fallback,
    }
}

/// The commons' start, with a fresh PKCE pair. Answers the start and the
/// ceremony this device keeps for the finish.
async fn commons_start(
    shared: &DaemonShared,
    cfg: &ContributorConfig,
    session: &mut crate::account_auth::LoadedAccountSession,
    identity: &DeviceIdentity,
    wallet_account_id: Option<&str>,
) -> std::result::Result<(RebindStartResponse, CommonsCeremony), &'static str> {
    let verifier = super::account_onboarding::random().map_err(|_| STEP_ERROR_UNAVAILABLE)?;
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    let request = RebindStartRequest {
        device_public_key: identity.public_key_b64.clone(),
        code_challenge: challenge.clone(),
        code_challenge_method: "S256".into(),
        wallet_account_id: wallet_account_id.map(str::to_owned),
    };
    let started: RebindStartResponse =
        match call(shared, cfg, session, wire::REBIND_START_PATH, &request).await {
            Ok(started) => started,
            Err(error) => {
                // A commons without the routes answers 404: remember it, so the
                // change is not offered again until the daemon restarts.
                if matches!(
                    error,
                    trace_commons_operator_client::Error::HttpFailure { status, .. }
                        if status.as_u16() == 404
                ) {
                    note_routes(shared.store.dir(), false);
                    return Err(ERR_NOT_AVAILABLE);
                }
                return Err(refusal(&error, STEP_ERROR_UNAVAILABLE));
            }
        };
    note_routes(shared.store.dir(), true);
    let nonce: [u8; 32] = base64::engine::general_purpose::STANDARD
        .decode(&started.nonce)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or(STEP_ERROR_UNAVAILABLE)?;
    let now = Utc::now().timestamp();
    if started.ceremony_id.is_empty()
        || started.ceremony_id.len() > 64
        || started.expires_at <= now
        || started.expires_at > now + wire::REBIND_CEREMONY_TTL_SECS + 60
        || started.current_kind != cfg.signs_in_with().ok_or(ERR_NOT_AVAILABLE)?
    {
        return Err(STEP_ERROR_UNAVAILABLE);
    }
    let commons = CommonsCeremony {
        ceremony_id: started.ceremony_id.clone(),
        nonce,
        expires_at: started.expires_at,
        verifier,
        challenge,
    };
    Ok((started, commons))
}

/// The proofs a finish carries, taken from the ceremony while it runs.
struct Proofs {
    current: CurrentProofHeld,
    new_login: HeldLogin,
}

/// Step 3: the person confirmed. Runs the commons' finish (and, for a NEAR
/// AI account, its start first), then re-binds this device. `routed` is a
/// test's stub commons in place of the enrolled one.
pub(super) async fn confirm(
    shared: &DaemonShared,
    req: &Request,
    change_id: Option<&str>,
    api: &CloudApi,
) -> Response {
    let dir = shared.store.dir().to_path_buf();
    let (id, kind, _) = match claim(&dir, change_id, Step::Confirm) {
        Ok(claimed) => claimed,
        Err(label) => return Response::err(req.id, ERR_UNAVAILABLE, label),
    };
    if !matches!(
        req.params.get("confirmed"),
        Some(serde_json::Value::Bool(true))
    ) {
        settle(&dir, &id, |_| {});
        return Response::err(req.id, ERR_BAD_PARAMS, "confirmed");
    }
    // The proofs leave the ceremony for the call, and go back if it fails
    // in a way that keeps them.
    let proofs = with_slot(&dir, |slot| {
        let ceremony = slot.ceremony.as_mut().filter(|c| c.id == id)?;
        Some(Proofs {
            current: ceremony.current.take()?,
            new_login: ceremony.new_login.take()?,
        })
    });
    let Some(proofs) = proofs else {
        settle(&dir, &id, |c| c.error = Some(STEP_ERROR_UNAVAILABLE));
        return Response::err(req.id, ERR_UNAVAILABLE, ERR_OUT_OF_ORDER);
    };
    let outcome = finish(shared, &id, kind, proofs, api).await;
    match outcome {
        Ok(()) => {
            settle(&dir, &id, |ceremony| {
                ceremony.done = true;
                ceremony.error = None;
            });
            Response::ok(req.id, view_of(&dir, &id))
        }
        // The step's error is in the state; the call itself answered.
        Err(_) => Response::ok(req.id, view_of(&dir, &id)),
    }
}

/// The finish and the re-bind. On a failure the ceremony is put back where
/// the person can go on: a wallet proof is spent with its commons ceremony
/// and asked again; a new login the commons refused as in use is dropped
/// (its key revoked) and asked again; otherwise both proofs are kept.
async fn finish(
    shared: &DaemonShared,
    id: &str,
    kind: IdentityKind,
    proofs: Proofs,
    api: &CloudApi,
) -> std::result::Result<(), &'static str> {
    let dir = shared.store.dir().to_path_buf();
    // A new login not put back (refused as in use) is revoked as it drops.
    let restore =
        |current: Option<CurrentProofHeld>, new_login: Option<HeldLogin>, error: &'static str| {
            settle(&dir, id, |ceremony| {
                ceremony.current = current;
                ceremony.new_login = new_login;
                ceremony.error = Some(error);
            });
        };
    let Proofs { current, new_login } = proofs;
    let (cfg, mut session, identity) = match commons_inputs(shared).await {
        Ok(inputs) => inputs,
        Err(label) => {
            restore(Some(current), Some(new_login), STEP_ERROR_UNAVAILABLE);
            return Err(label);
        }
    };
    let Ok(device) = device_bytes(&identity) else {
        restore(Some(current), Some(new_login), STEP_ERROR_UNAVAILABLE);
        return Err(STEP_ERROR_UNAVAILABLE);
    };
    // The commons ceremony and the current proof as the wire carries it.
    let (commons, proof, keep_current) = match current {
        CurrentProofHeld::NearAi { access_token } => {
            let started = commons_start(shared, &cfg, &mut session, &identity, None).await;
            let commons = match started {
                Ok((_, commons)) => commons,
                Err(label) => {
                    restore(
                        Some(CurrentProofHeld::NearAi { access_token }),
                        Some(new_login),
                        step_error(label),
                    );
                    return Err(label);
                }
            };
            let keep = CurrentProofHeld::NearAi {
                access_token: access_token.clone(),
            };
            (
                commons,
                CurrentProof::NearAiLogin { access_token },
                Some(keep),
            )
        }
        CurrentProofHeld::Wallet {
            account_id,
            wallet_public_key,
            wallet_signature,
            commons,
        } => {
            if commons.expires_at - Utc::now().timestamp() < WALLET_FINISH_MARGIN_SECS {
                // Bound to the old nonce: the wallet signs again (spec 2.4).
                restore(None, Some(new_login), STEP_ERROR_WALLET);
                return Err(STEP_ERROR_WALLET);
            }
            (
                commons,
                CurrentProof::Wallet {
                    account_id,
                    wallet_public_key,
                    wallet_signature,
                },
                None,
            )
        }
    };
    let signing = wire::rebind_device_bytes(
        &commons.nonce,
        &commons.ceremony_id,
        &device,
        &commons.challenge,
        commons.expires_at,
        &proof.digest(),
        &wire::new_login_digest(&new_login.tokens.access_token),
    );
    let request = RebindFinishRequest {
        ceremony_id: commons.ceremony_id.clone(),
        code_verifier: commons.verifier.clone(),
        device_public_key: identity.public_key_b64.clone(),
        device_signature: identity.sign_b64(&signing),
        current_proof: proof,
        new_access_token: new_login.tokens.access_token.clone(),
    };
    let answer: std::result::Result<RebindFinishResponse, _> = call(
        shared,
        &cfg,
        &mut session,
        wire::REBIND_FINISH_PATH,
        &request,
    )
    .await;
    drop(request);
    let finished = match answer {
        Ok(finished) => finished,
        Err(error) => {
            let fallback = match kind {
                IdentityKind::Wallet => STEP_ERROR_WALLET,
                IdentityKind::NearAiLogin => STEP_ERROR_UNAVAILABLE,
            };
            let label = refusal(&error, fallback);
            crate::daemon::diagnostics_log::record_labels(
                tracing::Level::WARN,
                "account_login_change_refused",
                &[("reason", label)],
            );
            // The commons ceremony is spent either way. A wallet proof is
            // bound to it; a NEAR AI token is not, and is kept.
            match label {
                STEP_ERROR_IN_USE => restore(keep_current, None, STEP_ERROR_IN_USE),
                _ => restore(keep_current, Some(new_login), step_error(label)),
            }
            return Err(label);
        }
    };
    if finished.tenant_id != cfg.tenant_id
        || finished.account_id != session.session.account_id
        || finished.identity != IdentityKind::NearAiLogin
    {
        // The commons moved something this device cannot recognize as its
        // account. Nothing local changes; the person signs in again.
        restore(None, Some(new_login), STEP_ERROR_UNAVAILABLE);
        return Err(STEP_ERROR_UNAVAILABLE);
    }
    rebind(shared, id, cfg, new_login, api).await
}

fn step_error(label: &'static str) -> &'static str {
    match label {
        STEP_ERROR_TOO_SOON | STEP_ERROR_IN_USE | STEP_ERROR_WALLET | STEP_ERROR_CURRENT_LOGIN => {
            label
        }
        _ => STEP_ERROR_UNAVAILABLE,
    }
}

/// The commons moved the account to the new login: store it as this
/// device's NEAR AI credential, record what the account signs in with, and
/// revoke the old key with the old session, best effort.
async fn rebind(
    shared: &DaemonShared,
    id: &str,
    cfg: ContributorConfig,
    new_login: HeldLogin,
    api: &CloudApi,
) -> std::result::Result<(), &'static str> {
    let store = shared.store.clone();
    let dir = store.dir().to_path_buf();
    let id_owned = id.to_owned();
    let persisted = tokio::task::spawn_blocking(move || {
        // What the old key's revoke needs, copied before it is replaced.
        let revocation = super::nearai_credential::disconnect::capture_for_change(&store);
        super::nearai_credential::ceremony::persist_change(&store, new_login, || {
            // Still this ceremony: a cancel or sign-out since wins.
            let current = with_slot(&dir, |slot| {
                slot.ceremony.as_ref().is_some_and(|c| c.id == id_owned)
            });
            if current {
                Ok(())
            } else {
                Err(anyhow!("account_login_change_cancelled"))
            }
        })?;
        let _ = super::nearai_onboarding::record_account_identity(
            &store,
            &cfg.tenant_id,
            IdentityKind::NearAiLogin,
        );
        Ok::<_, anyhow::Error>(revocation)
    })
    .await;
    let revocation = match persisted {
        Ok(Ok(revocation)) => revocation,
        _ => {
            // The commons moved the account; this device's NEAR AI login is
            // still the old one, which it now refuses as another account.
            // Signing in with the new login fixes it.
            crate::daemon::diagnostics_log::record_labels(
                tracing::Level::WARN,
                "account_login_change_not_stored",
                &[("reason", "persist")],
            );
            settle(shared.store.dir(), id, |c| {
                c.error = Some(STEP_ERROR_UNAVAILABLE)
            });
            return Err(STEP_ERROR_UNAVAILABLE);
        }
    };
    // The running daemon takes the new key now, before the old one is
    // revoked under its proxy.
    shared.absorb_near_ai_credential_change().await;
    if let Some(revocation) = revocation {
        let api = api.clone();
        let revoke = async move {
            let _ = revocation.run_for_change(Some(&api)).await;
        };
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(revoke);
            }
            Err(_) => drop(revoke),
        }
    }
    Ok(())
}

/// Put a wallet proof bound to a commons ceremony into the named ceremony,
/// as the wallet leg's callback does after the commons' start.
#[cfg(test)]
pub(crate) fn hold_wallet_proof_for_test(
    dir: &Path,
    id: &str,
    account_id: &str,
    ceremony_id: &str,
    nonce: [u8; 32],
    expires_at: i64,
    verifier: &str,
) {
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    with_slot(dir, |slot| {
        let ceremony = slot
            .ceremony
            .as_mut()
            .filter(|c| c.id == id)
            .expect("the ceremony is running");
        ceremony.current = Some(CurrentProofHeld::Wallet {
            account_id: account_id.into(),
            wallet_public_key: "ed25519:synthetic".into(),
            wallet_signature: "c2lnbmF0dXJl".into(),
            commons: CommonsCeremony {
                ceremony_id: ceremony_id.into(),
                nonce,
                expires_at,
                verifier: verifier.into(),
                challenge,
            },
        });
    });
}
