//! Explicit wallet signup: daemon-owned device signing and PKCE, hosted wallet
//! handoff, and a state-bound loopback callback. No shell handles private keys.
//!
//! Bootstrap trust is TOFU: witness and issuer pins are obtained over allowlisted
//! HTTPS from the ingest origin chosen by the user. HTTPS authenticates that
//! origin; it does not independently endorse its advertised keys or measurements.
//! Later witness checks enforce the persisted pins. Operators must authenticate
//! the service origin out of band before directing contributors to it.

use super::ipc::{DaemonShared, ERR_BAD_PARAMS, ERR_UNAVAILABLE, Request, Response};
use crate::config::{
    ACCOUNT_SESSION_FILE, CONTRIBUTOR_CONFIG_SCHEMA_VERSION, ConfigStore, ContributorConfig,
    WitnessSettings, allowlist_for,
};
use crate::identity::DeviceIdentity;
use anyhow::{Result, anyhow, bail};
use base64::Engine;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use trace_commons_operator_client::host_allowlist::HostAllowlist;

pub const LOOPBACK_PATH: &str = "/trace-commons/near-onboarding/callback";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    account_id: String,
    ingest_url: String,
    #[serde(default)]
    issuer_url: String,
    #[serde(default)]
    audience: String,
}
#[derive(Deserialize)]
struct Capability {
    ready: bool,
    witness: Option<serde_json::Value>,
    issuer_url: Option<String>,
    audience: Option<String>,
}

#[derive(Deserialize)]
struct Challenge {
    ceremony_id: String,
    message: String,
    nonce: String,
    recipient: String,
    expires_at: i64,
    device_signing_bytes: String,
    wallet_url: String,
    network: String,
}
#[derive(Deserialize)]
struct WalletResult {
    state: String,
    account_id: String,
    wallet_public_key: String,
    wallet_signature: String,
}
#[derive(Deserialize)]
struct Completed {
    access_token: String,
    token_type: String,
    expires_in_secs: i64,
    account_id: String,
    tenant_id: String,
    device_key_id: String,
    anchor_hash: String,
}
#[derive(Clone, Serialize)]
struct Status {
    attempt_id: String,
    status: &'static str,
}
struct Attempt {
    state: Status,
    abort: Option<tokio::task::AbortHandle>,
}
static ATTEMPTS: OnceLock<Mutex<HashMap<PathBuf, Attempt>>> = OnceLock::new();
// Embedded FFI creates a short-lived runtime per IPC call. Wallet work and its
// reactor must outlive that call, including while the app waits in the browser.
fn signup_runtime() -> Result<&'static tokio::runtime::Runtime> {
    static RUNTIME: OnceLock<Option<tokio::runtime::Runtime>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .thread_name("near-signup")
                .enable_all()
                .build()
                .ok()
        })
        .as_ref()
        .ok_or_else(|| anyhow!("near_signup_unavailable"))
}
fn attempts() -> &'static Mutex<HashMap<PathBuf, Attempt>> {
    ATTEMPTS.get_or_init(Default::default)
}
fn random() -> Result<String> {
    let mut bytes = [0; 32];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
        .map_err(|_| anyhow!("near_signup_unavailable"))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}
/// Why a signup step stopped, in the three classes a person acts on
/// differently. Every failure below is one of these, and the shells render
/// one sentence per class -- see [`crate::witness_copy::wallet_refusal_line`].
/// Collapsing them, as this surface used to, leaves a person told only that
/// signup is "unavailable" with nothing to check and nothing to report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignupRefusal {
    /// The address was rejected here, before any request left the process:
    /// not https, carrying credentials or a query, unparseable, or not on a
    /// host list this installation was configured with.
    AddressRefused,
    /// The address was dialled and did not answer, or answered in a way this
    /// client could not read.
    Unreachable,
    /// The commons answered and does not offer wallet signup, or published
    /// trust material this client will not accept.
    Unsupported,
}

impl SignupRefusal {
    /// The stable wire value. Native shells map this to a sentence; they must
    /// never re-derive the class from anything else in the response.
    #[must_use]
    pub fn wire(self) -> &'static str {
        match self {
            Self::AddressRefused => "address_refused",
            Self::Unreachable => "unreachable",
            Self::Unsupported => "unsupported",
        }
    }
}

/// The hosts one signup step may reach, and who chose each of them.
///
/// This exists because the shipped applications set no
/// `TRACE_COMMONS_ALLOWED_HOSTS`, and [`client`] refuses when the allowlist
/// is not enforcing -- so signup was impossible from Finder, the Start Menu
/// or a flatpak, and the only documented remedy was an environment variable
/// no contributor can reasonably be asked to set.
///
/// The list is now:
///
/// * An operator's `TRACE_COMMONS_ALLOWED_HOSTS`, unchanged and in full,
///   whenever it is set. An operator who pins hosts keeps pinning them, and
///   an origin outside that list is still refused.
/// * Otherwise exactly the hosts named in `origin` and `published`: `origin`
///   is the commons address the person typed on the signup screen, and
///   `published` holds hosts that same origin returned over authenticated
///   HTTPS for this step (its issuer, its witness). Nothing else is
///   reachable, and no host enters the list that the person did not choose
///   or that their chosen origin did not name.
///
/// The derivation never degrades to permissive: an origin with no host is an
/// error, and [`HostAllowlist::from_hosts`] treats an empty set as
/// "nothing", not "everything".
fn signup_allowlist(origin: &str, published: &[&str]) -> Result<HostAllowlist> {
    derive_signup_allowlist(&allowlist_for(None), origin, published)
}

fn derive_signup_allowlist(
    configured: &HostAllowlist,
    origin: &str,
    published: &[&str],
) -> Result<HostAllowlist> {
    if configured.is_enforcing() {
        return Ok(configured.clone());
    }
    let mut hosts = Vec::with_capacity(1 + published.len());
    for url in std::iter::once(origin).chain(published.iter().copied()) {
        let parsed =
            reqwest::Url::parse(url).map_err(|_| anyhow!("near_signup_endpoint_refused"))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| anyhow!("near_signup_endpoint_refused"))?;
        hosts.push(host.to_string());
    }
    Ok(HostAllowlist::from_hosts(hosts))
}

fn client(url: &str, allowed: &HostAllowlist) -> Result<trace_commons_operator_client::Client> {
    let parsed = reqwest::Url::parse(url)?;
    if !allowed.is_enforcing()
        || parsed.scheme() != "https"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        bail!("near_signup_endpoint_refused");
    }
    allowed.check(&parsed)?;
    trace_commons_operator_client::Client::builder(
        url,
        "TRACE_COMMONS_CONTRIBUTOR_UNUSED_BEARER_ENV",
    )
    .bearer_token("unauthenticated")
    .host_allowlist(allowed.clone())
    .build()
    .map_err(|_| anyhow!("near_signup_endpoint_refused"))
}

pub async fn handle_capabilities(_shared: &DaemonShared, req: &Request) -> Response {
    let Some(url) = req.params.get("ingest_url").and_then(|v| v.as_str()) else {
        return Response::err(req.id, ERR_BAD_PARAMS, "near_signup_invalid");
    };
    match validated_capability(url).await {
        Ok((issuer, audience, witness)) => Response::ok(
            req.id,
            serde_json::json!({"ready":true,"issuer_url":issuer,"audience":audience,"witness":witness,"funding_available":false}),
        ),
        // A refusal names its class. Without it every failure here -- an
        // address this daemon will not dial, a commons that did not answer,
        // and a commons that does not offer wallet signup -- reached the
        // person as one "unavailable" sentence they could not act on.
        Err(refusal) => Response::ok(
            req.id,
            serde_json::json!({"ready":false,"reason":refusal.wire(),"funding_available":false}),
        ),
    }
}

async fn validated_capability(
    url: &str,
) -> std::result::Result<(String, String, WitnessSettings), SignupRefusal> {
    let origin_only = signup_allowlist(url, &[]).map_err(|_| SignupRefusal::AddressRefused)?;
    let capability: Capability = client(url, &origin_only)
        .map_err(|_| SignupRefusal::AddressRefused)?
        .call_json(
            reqwest::Method::GET,
            "/v1/account/near/provision/capabilities",
            &[],
            None::<&serde_json::Value>,
        )
        .await
        .map_err(|_| SignupRefusal::Unreachable)?;
    validate_capability(url, capability)
}

/// Everything the capabilities response has to satisfy, with no I/O of its
/// own, so the decision is testable without a live HTTPS commons.
fn validate_capability(
    origin: &str,
    capability: Capability,
) -> std::result::Result<(String, String, WitnessSettings), SignupRefusal> {
    if !capability.ready {
        return Err(SignupRefusal::Unsupported);
    }
    let issuer = capability.issuer_url.ok_or(SignupRefusal::Unsupported)?;
    let audience = capability.audience.ok_or(SignupRefusal::Unsupported)?;
    if audience.trim().is_empty() || audience.len() > 256 {
        return Err(SignupRefusal::Unsupported);
    }
    let witness = published_witness(capability.witness.ok_or(SignupRefusal::Unsupported)?)
        .map_err(|_| SignupRefusal::Unsupported)?;
    // The issuer and the witness are hosts this origin named for itself. They
    // enter the list because the person chose this origin, and only for as
    // long as this call: an operator's own allowlist, when set, still governs
    // and refuses either of them if it does not list them.
    let published = signup_allowlist(origin, &[&issuer, &witness.url])
        .map_err(|_| SignupRefusal::AddressRefused)?;
    client(&issuer, &published).map_err(|_| SignupRefusal::AddressRefused)?;
    client(&witness.url, &published).map_err(|_| SignupRefusal::AddressRefused)?;
    Ok((issuer, audience, witness))
}

pub async fn handle_start(shared: &DaemonShared, req: &Request) -> Response {
    let options: Options = match serde_json::from_value(req.params.clone()) {
        Ok(v) => v,
        Err(_) => return Response::err(req.id, ERR_BAD_PARAMS, "near_signup_invalid"),
    };
    match begin(&shared.store, options).await {
        Ok(value) => Response::ok(req.id, value),
        Err(_) => Response::err(req.id, ERR_UNAVAILABLE, "near_signup_unavailable"),
    }
}
pub fn handle_status(shared: &DaemonShared, req: &Request) -> Response {
    let map = attempts().lock().expect("signup state lock");
    match map.get(shared.store.dir()).filter(|a| {
        req.params.get("attempt_id").and_then(|v| v.as_str()) == Some(a.state.attempt_id.as_str())
    }) {
        Some(a) => Response::ok(req.id, serde_json::to_value(&a.state).unwrap_or_default()),
        None => Response::err(req.id, ERR_BAD_PARAMS, "near_signup_unknown"),
    }
}
pub fn handle_cancel(shared: &DaemonShared, req: &Request) -> Response {
    let mut map = attempts().lock().expect("signup state lock");
    let Some(a) = map.get_mut(shared.store.dir()).filter(|a| {
        req.params.get("attempt_id").and_then(|v| v.as_str()) == Some(a.state.attempt_id.as_str())
    }) else {
        return Response::err(req.id, ERR_BAD_PARAMS, "near_signup_unknown");
    };
    if matches!(a.state.status, "starting" | "waiting_for_wallet") {
        a.state.status = "cancelled";
        if let Some(task) = &a.abort {
            task.abort();
        }
    }
    Response::ok(req.id, serde_json::to_value(&a.state).unwrap_or_default())
}

async fn begin(store: &ConfigStore, options: Options) -> Result<serde_json::Value> {
    if store.load_config()?.is_some() {
        bail!("near_signup_already_enrolled")
    }
    let ingest = client(
        &options.ingest_url,
        &signup_allowlist(&options.ingest_url, &[])?,
    )?;

    let id = random()?;
    let dir = store.dir().to_path_buf();
    {
        let mut map = attempts().lock().expect("signup state lock");
        if map
            .get(&dir)
            .is_some_and(|a| matches!(a.state.status, "starting" | "waiting_for_wallet"))
        {
            bail!("near_signup_busy")
        }
        if map.len() > 128 && !map.contains_key(&dir) {
            bail!("near_signup_busy")
        }
        map.insert(
            dir.clone(),
            Attempt {
                state: Status {
                    attempt_id: id.clone(),
                    status: "starting",
                },
                abort: None,
            },
        );
    }
    let result = prepare(store, options, ingest, &id).await;
    if result.is_err()
        && let Some(a) = attempts().lock().expect("signup state lock").get_mut(&dir)
        && a.state.attempt_id == id
        && a.state.status != "cancelled"
    {
        a.state.status = "failed";
    }
    result
}

async fn prepare(
    store: &ConfigStore,
    mut options: Options,
    ingest: trace_commons_operator_client::Client,
    id: &str,
) -> Result<serde_json::Value> {
    let receipt_endpoint = crate::config::inference_receipt_endpoint_from_env();
    if let Some(endpoint) = receipt_endpoint.as_deref() {
        crate::config::validate_inference_receipt_endpoint(endpoint, &allowlist_for(None))?;
    }
    let (issuer, audience, witness) = validated_capability(&options.ingest_url)
        .await
        .map_err(|refusal| anyhow!(refusal.wire()))?;
    if (!options.issuer_url.is_empty() && options.issuer_url != issuer)
        || (!options.audience.is_empty() && options.audience != audience)
    {
        bail!("near_signup_invalid");
    }
    options.issuer_url = issuer;
    options.audience = audience;
    let identity = DeviceIdentity::load_or_generate(store)?;
    let verifier = random()?;
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    let signed:Challenge=ingest.call_json(reqwest::Method::POST,"/v1/account/near/provision/start",&[],Some(&serde_json::json!({"account_id":options.account_id,"device_public_key":identity.public_key_b64,"code_challenge":challenge,"code_challenge_method":"S256"}))).await.map_err(|_|anyhow!("near_signup_start_failed"))?;
    let device: [u8; 32] = base64::engine::general_purpose::STANDARD
        .decode(&identity.public_key_b64)?
        .try_into()
        .map_err(|_| anyhow!("near_signup_invalid"))?;
    let nonce: [u8; 32] = base64::engine::general_purpose::STANDARD
        .decode(&signed.nonce)?
        .try_into()
        .map_err(|_| anyhow!("near_signup_invalid"))?;
    let binding: [u8; 32] =
        Sha256::digest(format!("trace_commons.near_native_pkce.v1\n{challenge}")).into();
    let wallet = reqwest::Url::parse(&signed.wallet_url)?;
    let base = reqwest::Url::parse(&options.ingest_url)?;
    if wallet.origin() != base.origin()
        || wallet.path() != "/account/near/provision/wallet"
        || wallet.query().is_some()
        || wallet.fragment().is_some()
        || !matches!(signed.network.as_str(), "mainnet" | "testnet")
        || signed.expires_at <= Utc::now().timestamp()
        || signed.expires_at > Utc::now().timestamp() + 600
    {
        bail!("near_signup_invalid")
    }
    let expected = trace_commons_protocol::onboarding::near_provisioning_message(
        &signed.network,
        &options.account_id,
        &device,
        &binding,
        signed.expires_at,
    );
    if signed.message != expected {
        bail!("near_signup_invalid")
    }
    let bytes = trace_commons_protocol::onboarding::near_provisioning_device_bytes(
        &nonce,
        &expected,
        &signed.recipient,
        &binding,
        Some(&signed.wallet_url),
    );
    if base64::engine::general_purpose::STANDARD.decode(&signed.device_signing_bytes)? != bytes {
        bail!("near_signup_invalid")
    }
    let device_signature = identity.sign_b64(&bytes);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let callback = format!(
        "http://127.0.0.1:{}{LOOPBACK_PATH}",
        listener.local_addr()?.port()
    );
    let state = random()?;
    let fragment=base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(serde_json::to_vec(&serde_json::json!({"account_id":options.account_id,"message":signed.message,"nonce":signed.nonce,"recipient":signed.recipient,"expires_at":signed.expires_at,"network":signed.network,"loopback":callback,"state":state}))?);
    let browser_url = format!("{}#{fragment}", signed.wallet_url);
    let dir = store.dir().to_path_buf();
    let attempt_id = id.to_string();
    let mut map = attempts().lock().expect("signup state lock");
    let entry = map
        .get_mut(&dir)
        .ok_or_else(|| anyhow!("near_signup_cancelled"))?;
    if entry.state.attempt_id != id || entry.state.status == "cancelled" {
        bail!("near_signup_cancelled")
    }
    entry.state.status = "waiting_for_wallet";
    let listener = listener.into_std()?;
    let task = signup_runtime()?.spawn(async move {
        let result = async {
            let listener = tokio::net::TcpListener::from_std(listener)?;
            // Never reuse a connection pool attached to the caller's reactor.
            let ingest = client(
                &options.ingest_url,
                &signup_allowlist(&options.ingest_url, &[])?,
            )?;
            finish(
                listener,
                &state,
                &options,
                &identity,
                signed,
                &verifier,
                &device_signature,
                &ingest,
            )
            .await
        }
        .await;
        let mut map = attempts().lock().expect("signup state lock");
        if let Some(entry) = map
            .get_mut(&dir)
            .filter(|a| a.state.attempt_id == attempt_id && a.state.status == "waiting_for_wallet")
        {
            entry.state.status = match result.and_then(|completed| {
                persist(
                    &dir,
                    &options,
                    &identity,
                    completed,
                    &attempt_id,
                    witness,
                    receipt_endpoint,
                )
            }) {
                Ok(()) => "complete",
                Err(_) => "failed",
            };
            entry.abort = None;
        }
    });
    entry.abort = Some(task.abort_handle());
    Ok(serde_json::json!({"attempt_id":id,"status":"waiting_for_wallet","browser_url":browser_url}))
}

#[allow(clippy::too_many_arguments)] // One private, purpose-bound ceremony, never shell-provided keys.
async fn finish(
    listener: tokio::net::TcpListener,
    state: &str,
    options: &Options,
    identity: &DeviceIdentity,
    signed: Challenge,
    verifier: &str,
    device_signature: &str,
    client: &trace_commons_operator_client::Client,
) -> Result<Completed> {
    let result = tokio::time::timeout(Duration::from_secs(300), receive_wallet(listener, state))
        .await
        .map_err(|_| anyhow!("near_signup_expired"))??;
    if result.account_id != options.account_id {
        bail!("near_signup_account_changed")
    }
    client.call_json(reqwest::Method::POST,"/v1/account/near/provision/finish",&[],Some(&serde_json::json!({"ceremony_id":signed.ceremony_id,"account_id":options.account_id,"device_public_key":identity.public_key_b64,"wallet_public_key":result.wallet_public_key,"wallet_signature":result.wallet_signature,"device_signature":device_signature,"code_verifier":verifier}))).await.map_err(|_|anyhow!("near_signup_verification_failed"))
}

fn parse_callback(target: &str, state: &str) -> Result<WalletResult> {
    let url = reqwest::Url::parse(&format!("http://127.0.0.1{target}"))?;
    if url.path() != LOOPBACK_PATH || url.fragment().is_some() {
        bail!("near_signup_callback_invalid")
    }
    let values: Vec<_> = url.query_pairs().filter(|(k, _)| k == "result").collect();
    if values.len() != 1 || values[0].1.len() > 4096 {
        bail!("near_signup_callback_invalid")
    }
    let result: WalletResult = serde_json::from_slice(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(values[0].1.as_bytes())?,
    )?;
    if result.state != state
        || result.wallet_public_key.len() > 120
        || result.wallet_signature.len() > 128
    {
        bail!("near_signup_callback_invalid")
    }
    Ok(result)
}
async fn receive_wallet(listener: tokio::net::TcpListener, state: &str) -> Result<WalletResult> {
    // Ignore unrelated local requests, but bound both each connection and total
    // waiting time so a process cannot monopolize the callback listener.
    loop {
        let (mut stream, _) = listener.accept().await?;
        let mut bytes = vec![0; 8192];
        let read_headers = async {
            let mut n = 0;
            while n < bytes.len() {
                let count = stream.read(&mut bytes[n..]).await?;
                if count == 0 {
                    bail!("near_signup_callback_invalid");
                }
                n += count;
                if bytes[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                    return Ok(n);
                }
            }
            Err(anyhow!("near_signup_callback_invalid"))
        };
        let Ok(Ok(n)) = tokio::time::timeout(Duration::from_secs(2), read_headers).await else {
            continue;
        };
        let text = String::from_utf8_lossy(&bytes[..n]);
        let line = text.lines().next().unwrap_or("");
        let parts: Vec<_> = line.split_whitespace().collect();
        let result = if parts.len() == 3 && parts[0] == "GET" {
            parse_callback(parts[1], state)
        } else {
            Err(anyhow!("near_signup_callback_invalid"))
        };
        let body = if result.is_ok() {
            "The signed request was received. Return to Trace Commons while it verifies your account."
        } else {
            "This callback was not accepted."
        };
        let code = if result.is_ok() {
            "200 OK"
        } else {
            "400 Bad Request"
        };
        let response = format!(
            "HTTP/1.1 {code}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = tokio::time::timeout(
            Duration::from_secs(2),
            stream.write_all(response.as_bytes()),
        )
        .await;
        if let Ok(result) = result {
            return Ok(result);
        }
    }
}

/// Shape and pin checks on the witness an origin published. Deliberately does
/// NOT decide whether its host may be reached -- the caller owns that, because
/// only the caller knows which origin published it. See
/// [`validated_capability`].
fn published_witness(mut value: serde_json::Value) -> Result<WitnessSettings> {
    value
        .as_object_mut()
        .ok_or_else(|| anyhow!("near_signup_trust_invalid"))?
        .insert("admission_evidence".into(), serde_json::Value::Bool(true));
    let witness: WitnessSettings = serde_json::from_value(value)?;
    let url = reqwest::Url::parse(&witness.url)?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("near_signup_trust_invalid")
    }
    let address = witness
        .signing_address
        .strip_prefix("0x")
        .ok_or_else(|| anyhow!("near_signup_trust_invalid"))?;
    if address.len() != 40
        || !address.bytes().all(|b| b.is_ascii_hexdigit())
        || !witness.trust()?.is_pinned()
    {
        bail!("near_signup_trust_invalid")
    }
    Ok(witness)
}

/// Shape check for a wallet tenant id: the `near-` namespace plus 64 lowercase
/// hex characters.
///
/// This used to be `tenant_id == format!("near-{}", &anchor_hash[7..])`, and it
/// stopped being true when the server salted the anchor. The tenant id is now
/// drawn from the OS RNG and is a function of nothing -- that is the point of
/// the change, since the old binding let anyone who knew a NEAR account name
/// compute its tenant id offline. The client cannot re-derive it, so shape is
/// all there is to check here; the value is authenticated by the session token
/// issued alongside it, not by its own contents.
fn is_near_tenant_id(tenant_id: &str) -> bool {
    tenant_id.strip_prefix("near-").is_some_and(|suffix| {
        suffix.len() == 64
            && suffix
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    })
}

fn persist(
    dir: &std::path::Path,
    options: &Options,
    identity: &DeviceIdentity,
    result: Completed,
    id: &str,
    witness: WitnessSettings,
    receipt_endpoint: Option<String>,
) -> Result<()> {
    if result.token_type != "Bearer"
        || !result.access_token.starts_with("tcn1_")
        || result.expires_in_secs <= 0
        || result.expires_in_secs > 43200
        || result.device_key_id != identity.device_key_id
        || !result.anchor_hash.starts_with("sha256:")
        || result.anchor_hash.len() != 71
        || !result.anchor_hash[7..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || !is_near_tenant_id(&result.tenant_id)
    {
        bail!("near_signup_result_invalid")
    }
    let store = ConfigStore::open(dir.to_path_buf())?;
    if store.load_config()?.is_some()
        || DeviceIdentity::load(&store)?.is_none_or(|k| k.device_key_id != identity.device_key_id)
    {
        bail!("near_signup_state_changed")
    }
    let config = ContributorConfig {
        schema_version: CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
        issuer_url: options.issuer_url.clone(),
        ingest_url: options.ingest_url.clone(),
        audience: options.audience.clone(),
        tenant_id: result.tenant_id,
        instance_id: String::new(),
        user_subject: identity.device_key_id.clone(),
        device_key_id: identity.device_key_id.clone(),
        consent_scopes: Vec::new(),
        pii_filter: None,
        allowed_hosts: None,
        display_handle: None,
        public_bio: None,
        public_since: None,
        witness: Some(witness),
        inference_receipt_endpoint: receipt_endpoint,
        inference_receipt_check_attestation: true,
    };
    let session = crate::account_auth::AccountSession {
        access_token: result.access_token,
        expires_at: Utc::now() + chrono::Duration::seconds(result.expires_in_secs),
        account_id: result.account_id,
    };
    let session_bytes = serde_json::to_vec(&session)?;
    // Stage the config, then persist the session before publishing enrollment.
    // A failed session write leaves no enrolled config to block a fresh attempt.
    // The final create-only link still cannot replace concurrent invite enrollment.
    let temporary = format!("near-signup-{id}.json");
    store.write_daemon_file(&temporary, &serde_json::to_vec(&config)?)?;
    let published = store
        .write_daemon_file(ACCOUNT_SESSION_FILE, &session_bytes)
        .and_then(|()| {
            std::fs::hard_link(dir.join(&temporary), dir.join("contributor.json"))
                .map_err(Into::into)
        });
    let _ = store.remove_daemon_file(&temporary);
    published
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_requires_exact_path_state_and_single_result() {
        let state = "expected";
        let result = serde_json::json!({"state":state,"account_id":"alice.near","wallet_public_key":"ed25519:key","wallet_signature":"sig"});
        let value = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&result).unwrap());
        let target = format!("{LOOPBACK_PATH}?result={value}");
        assert!(parse_callback(&target, state).is_ok());
        assert!(parse_callback(&target, "wrong").is_err());
        assert!(parse_callback(&format!("/wrong?result={value}"), state).is_err());
        assert!(parse_callback(&format!("{target}&result={value}"), state).is_err());
    }
    #[test]
    fn refuses_untrusted_or_unpinned_witness_capabilities() {
        for witness in [
            serde_json::json!({"url":"http://localhost:1234","signing_address":format!("0x{}","ab".repeat(20)),"expected_measurements":[format!("mrtd={}","ab".repeat(48))]}),
            serde_json::json!({"url":"https://api.tracecommons.org","signing_address":format!("0x{}","ab".repeat(20)),"expected_measurements":[]}),
            serde_json::json!({"url":"https://api.tracecommons.org","signing_address":"broken","expected_measurements":["broken"]}),
            serde_json::json!({"url":"https://api.tracecommons.org?x=1","signing_address":format!("0x{}","ab".repeat(20)),"expected_measurements":[format!("mrtd={}","ab".repeat(48))]}),
        ] {
            assert!(published_witness(witness).is_err());
        }
    }

    /// A witness on a host the operator did not list stays refused. This case
    /// used to live in the loop above, where it passed for the wrong reason:
    /// `client` refused *every* host, because the allowlist was never
    /// enforcing. The refusal that remains is the real one -- an operator's
    /// `TRACE_COMMONS_ALLOWED_HOSTS` still governs what any published host
    /// may be.
    #[test]
    fn an_operator_allowlist_still_refuses_a_published_witness_host() {
        let operator = HostAllowlist::from_csv("commons.example");
        let allowed = derive_signup_allowlist(
            &operator,
            "https://commons.example",
            &["https://attacker.invalid"],
        )
        .unwrap();
        assert!(client("https://commons.example", &allowed).is_ok());
        assert!(client("https://attacker.invalid", &allowed).is_err());
    }

    #[test]
    fn with_no_operator_list_the_person_chosen_origin_is_the_list() {
        // The defect: with `TRACE_COMMONS_ALLOWED_HOSTS` unset the allowlist
        // was permissive, `client` refused on `!is_enforcing()`, and signup
        // was impossible in every shipped application.
        let unset = HostAllowlist::permissive();
        let allowed = derive_signup_allowlist(&unset, "https://commons.example/join", &[]).unwrap();
        assert!(allowed.is_enforcing());
        assert!(client("https://commons.example", &allowed).is_ok());
        // And it is a list, not a bypass: nothing the person did not choose
        // is reachable through it.
        assert!(client("https://evil.example", &allowed).is_err());
        assert!(client("https://sub.commons.example", &allowed).is_err());
    }

    #[test]
    fn a_published_host_is_reachable_only_once_the_origin_has_named_it() {
        let unset = HostAllowlist::permissive();
        let origin = "https://commons.example";
        let before = derive_signup_allowlist(&unset, origin, &[]).unwrap();
        assert!(client("https://issuer.example", &before).is_err());
        let after = derive_signup_allowlist(&unset, origin, &["https://issuer.example"]).unwrap();
        assert!(client("https://issuer.example", &after).is_ok());
        // Naming one host does not name its neighbours.
        assert!(client("https://witness.example", &after).is_err());
    }

    #[test]
    fn an_origin_without_a_host_is_an_error_and_never_a_permissive_list() {
        let unset = HostAllowlist::permissive();
        for origin in ["", "not a url", "file:///etc/passwd", "https://"] {
            assert!(
                derive_signup_allowlist(&unset, origin, &[]).is_err(),
                "{origin} must not yield a list"
            );
        }
    }

    #[test]
    fn a_shape_refusal_does_not_depend_on_the_allowlist_admitting_the_host() {
        // Every one of these names a host the derived list contains, so the
        // only thing that can refuse them is the shape check in `client`.
        let unset = HostAllowlist::permissive();
        for url in [
            "http://commons.example",
            "https://user@commons.example",
            "https://user:pw@commons.example",
            "https://commons.example/?token=abc",
            "https://commons.example/#frag",
        ] {
            let allowed = derive_signup_allowlist(&unset, url, &[]).unwrap();
            assert!(client(url, &allowed).is_err(), "{url} must be refused");
        }
    }

    fn capability(issuer: &str, witness_url: &str) -> Capability {
        Capability {
            ready: true,
            issuer_url: Some(issuer.into()),
            audience: Some("trace-commons-upload".into()),
            witness: Some(serde_json::json!({
                "url": witness_url,
                "signing_address": format!("0x{}", "ab".repeat(20)),
                "expected_measurements": [format!("mrtd={}", "ab".repeat(48))],
            })),
        }
    }

    /// A real commons publishes its issuer and its witness on hosts that are
    /// not the ingest host. If those do not enter the list this step enforces,
    /// signup is refused against every such deployment -- which is the same
    /// failure this change exists to remove, moved one step later.
    #[test]
    fn a_commons_may_publish_its_issuer_and_witness_on_other_hosts() {
        let (issuer, audience, witness) = validate_capability(
            "https://commons.example",
            capability("https://issuer.example", "https://witness.example"),
        )
        .expect("hosts the chosen origin published must be reachable");
        assert_eq!(issuer, "https://issuer.example");
        assert_eq!(audience, "trace-commons-upload");
        assert_eq!(witness.url, "https://witness.example");
    }

    /// And the list is still a list. A published host that fails the address
    /// rules is refused as an address, not admitted because the origin named
    /// it.
    #[test]
    fn a_published_host_still_has_to_be_an_address_this_daemon_will_dial() {
        for issuer in [
            "http://issuer.example",
            "https://user@issuer.example",
            "https://issuer.example/?token=abc",
            "https://",
        ] {
            assert_eq!(
                validate_capability(
                    "https://commons.example",
                    capability(issuer, "https://witness.example")
                ),
                Err(SignupRefusal::AddressRefused),
                "{issuer}"
            );
        }
        // A witness address that breaks the same rules is caught earlier, by
        // the trust checks, and is reported as trust material this client will
        // not accept rather than as an address.
        for witness_url in ["http://witness.example", "https://witness.example?x=1"] {
            assert_eq!(
                validate_capability(
                    "https://commons.example",
                    capability("https://issuer.example", witness_url)
                ),
                Err(SignupRefusal::Unsupported),
                "{witness_url}"
            );
        }
    }

    #[test]
    fn a_commons_that_is_not_offering_signup_is_unsupported_rather_than_refused() {
        let mut not_ready = capability("https://issuer.example", "https://witness.example");
        not_ready.ready = false;
        assert_eq!(
            validate_capability("https://commons.example", not_ready),
            Err(SignupRefusal::Unsupported)
        );
        let mut no_audience = capability("https://issuer.example", "https://witness.example");
        no_audience.audience = Some("   ".into());
        assert_eq!(
            validate_capability("https://commons.example", no_audience),
            Err(SignupRefusal::Unsupported)
        );
        let mut no_witness = capability("https://issuer.example", "https://witness.example");
        no_witness.witness = None;
        assert_eq!(
            validate_capability("https://commons.example", no_witness),
            Err(SignupRefusal::Unsupported)
        );
    }

    /// `client` refuses a permissive list outright rather than trusting
    /// `HostAllowlist::check`, which is a no-op on one. Every caller here
    /// passes an enforcing list, so this guard is what keeps a future one
    /// from quietly reaching anything at all.
    #[test]
    fn a_permissive_list_is_refused_rather_than_checked() {
        let permissive = HostAllowlist::permissive();
        assert!(
            permissive
                .check(&reqwest::Url::parse("https://evil.example").unwrap())
                .is_ok()
        );
        assert!(client("https://evil.example", &permissive).is_err());
        assert!(client("https://commons.example", &permissive).is_err());
    }

    #[test]
    fn the_three_refusal_classes_have_three_distinct_wire_values_and_sentences() {
        use crate::witness_copy::wallet_refusal_line;
        let classes = [
            SignupRefusal::AddressRefused,
            SignupRefusal::Unreachable,
            SignupRefusal::Unsupported,
        ];
        let wires: std::collections::BTreeSet<_> = classes.iter().map(|c| c.wire()).collect();
        assert_eq!(wires.len(), classes.len(), "the classes must not collide");
        let lines: std::collections::BTreeSet<_> = classes
            .iter()
            .map(|c| wallet_refusal_line(Some(c.wire())))
            .collect();
        assert_eq!(
            lines.len(),
            classes.len(),
            "each class must read differently to a person"
        );
        // A class this build does not know about must not be described.
        assert_eq!(
            wallet_refusal_line(Some("a_class_from_2027")),
            wallet_refusal_line(None)
        );
    }
    #[tokio::test]
    async fn callback_accepts_fragmented_http_headers() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { receive_wallet(listener, "state").await });
        let result = serde_json::json!({"state":"state","account_id":"alice.near","wallet_public_key":"key","wallet_signature":"sig"});
        let value = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&result).unwrap());
        let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
        stream
            .write_all(b"GET /trace-commons/near-onboarding/")
            .await
            .unwrap();
        tokio::task::yield_now().await;
        stream
            .write_all(
                format!("callback?result={value} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes(),
            )
            .await
            .unwrap();
        assert_eq!(task.await.unwrap().unwrap().account_id, "alice.near");
    }
    #[test]
    fn persists_no_capture_consent_and_never_replaces_enrollment() {
        for receipt_endpoint in [None, Some("https://receipts.example/v1".to_string())] {
            let dir = tempfile::tempdir().unwrap();
            let store = ConfigStore::open(dir.path().to_path_buf()).unwrap();
            let identity = DeviceIdentity::load_or_generate(&store).unwrap();
            let options = Options {
                account_id: "alice.near".into(),
                ingest_url: "https://commons.example".into(),
                issuer_url: "https://issuer.example".into(),
                audience: "traces".into(),
            };
            let witness: WitnessSettings = serde_json::from_value(serde_json::json!({"url":"https://witness.example","signing_address":format!("0x{}","ab".repeat(20)),"expected_measurements":[format!("mrtd={}","ab".repeat(48))],"admission_evidence":true})).unwrap();
            let completed = || Completed {
                access_token: "tcn1_example".into(),
                token_type: "Bearer".into(),
                expires_in_secs: 3600,
                account_id: "example-account".into(),
                // Deliberately unrelated to anchor_hash below. The server's
                // tenant id is now drawn at random, so a fixture where the two
                // agreed would keep passing under the retired
                // `tenant_id == "near-" || anchor_hash[7..]` binding and prove
                // nothing about the check that replaced it.
                tenant_id: format!("near-{}", "3c".repeat(32)),
                device_key_id: identity.device_key_id.clone(),
                anchor_hash: format!("sha256:{}", "ab".repeat(32)),
            };
            // A directory at the session destination makes atomic session install
            // fail after config staging. Enrollment must remain absent and retryable.
            std::fs::create_dir(dir.path().join(ACCOUNT_SESSION_FILE)).unwrap();
            assert!(
                persist(
                    dir.path(),
                    &options,
                    &identity,
                    completed(),
                    "failed-session",
                    witness.clone(),
                    None
                )
                .is_err()
            );
            assert!(store.load_config().unwrap().is_none());
            assert!(!dir.path().join("near-signup-failed-session.json").exists());
            std::fs::remove_dir(dir.path().join(ACCOUNT_SESSION_FILE)).unwrap();
            persist(
                dir.path(),
                &options,
                &identity,
                completed(),
                "first",
                witness.clone(),
                receipt_endpoint.clone(),
            )
            .unwrap();
            let before = std::fs::read(dir.path().join("contributor.json")).unwrap();
            let config = store.load_config().unwrap().unwrap();
            assert!(config.consent_scopes.is_empty());
            assert!(config.inference_receipt_check_attestation);
            assert!(config.witness.as_ref().unwrap().admission_evidence);
            assert_eq!(config.witness, Some(witness.clone()));
            assert_eq!(
                config.inference_receipt_endpoint.as_deref(),
                receipt_endpoint.as_deref()
            );
            assert!(
                persist(
                    dir.path(),
                    &options,
                    &identity,
                    completed(),
                    "second",
                    witness,
                    None
                )
                .is_err()
            );
            assert_eq!(
                std::fs::read(dir.path().join("contributor.json")).unwrap(),
                before
            );
        }
    }
    #[test]
    fn wallet_tenant_ids_are_accepted_on_shape_and_not_on_a_derivation() {
        assert!(is_near_tenant_id(&format!("near-{}", "3c".repeat(32))));
        assert!(is_near_tenant_id(&format!("near-{}", "ab".repeat(32))));
        // Upper-case hex, the wrong length, and the wrong namespace are all
        // rejected; the server emits lower-case hex in the `near-` namespace.
        assert!(!is_near_tenant_id(&format!("near-{}", "AB".repeat(32))));
        assert!(!is_near_tenant_id(&format!("near-{}", "ab".repeat(31))));
        assert!(!is_near_tenant_id(&format!("near-{}", "ab".repeat(33))));
        assert!(!is_near_tenant_id(&format!("tenant-{}", "ab".repeat(32))));
        assert!(!is_near_tenant_id("near-"));
        assert!(!is_near_tenant_id(&format!("near-{}", "gz".repeat(32))));
    }

    #[test]
    fn callback_survives_originating_ipc_runtime_drop() {
        use std::io::Write;
        let origin = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (address, result) = origin.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let listener = listener.into_std().unwrap();
            let (send, result) = std::sync::mpsc::channel();
            signup_runtime().unwrap().spawn(async move {
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                let _ = send.send(
                    receive_wallet(listener, "state")
                        .await
                        .map(|v| v.account_id),
                );
            });
            (address, result)
        });
        drop(origin);
        let value = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(serde_json::to_vec(&serde_json::json!({"state":"state","account_id":"alice.near","wallet_public_key":"key","wallet_signature":"sig"})).unwrap());
        let mut stream = std::net::TcpStream::connect(address).unwrap();
        stream
            .write_all(
                format!("GET {LOOPBACK_PATH}?result={value} HTTP/1.1\r\nHost: localhost\r\n\r\n")
                    .as_bytes(),
            )
            .unwrap();
        assert_eq!(
            result
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
                .unwrap(),
            "alice.near"
        );
    }
}
