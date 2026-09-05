//! Explicit wallet signup: daemon-owned device signing and PKCE, hosted wallet
//! handoff, and a state-bound loopback callback. No shell handles private keys.

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
fn client(url: &str) -> Result<trace_commons_operator_client::Client> {
    let parsed = reqwest::Url::parse(url)?;
    let allowed = allowlist_for(None);
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
    .host_allowlist(allowlist_for(None))
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
        Err(_) => Response::ok(
            req.id,
            serde_json::json!({"ready":false,"funding_available":false}),
        ),
    }
}
async fn validated_capability(url: &str) -> Result<(String, String, WitnessSettings)> {
    if reqwest::Url::parse(url)?.scheme() != "https" {
        bail!("near_signup_endpoint_refused");
    }
    let capability: Capability = client(url)?
        .call_json(
            reqwest::Method::GET,
            "/v1/account/near/provision/capabilities",
            &[],
            None::<&serde_json::Value>,
        )
        .await
        .map_err(|_| anyhow!("near_signup_unavailable"))?;
    if !capability.ready {
        bail!("near_signup_unavailable");
    }
    let issuer = capability
        .issuer_url
        .ok_or_else(|| anyhow!("near_signup_unavailable"))?;
    let audience = capability
        .audience
        .ok_or_else(|| anyhow!("near_signup_unavailable"))?;
    if reqwest::Url::parse(&issuer)?.scheme() != "https"
        || audience.trim().is_empty()
        || audience.len() > 256
    {
        bail!("near_signup_invalid");
    }
    let _issuer = client(&issuer)?;
    let witness = validate_published_witness(
        capability
            .witness
            .ok_or_else(|| anyhow!("near_signup_unavailable"))?,
    )?;
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
    let ingest = client(&options.ingest_url)?;

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
    let (issuer, audience, witness) = validated_capability(&options.ingest_url).await?;
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
            let ingest = client(&options.ingest_url)?;
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

fn validate_published_witness(mut value: serde_json::Value) -> Result<WitnessSettings> {
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
    let _allowed = client(&witness.url)?;
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
        || result.tenant_id != format!("near-{}", &result.anchor_hash[7..])
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
            serde_json::json!({"url":"https://attacker.invalid","signing_address":format!("0x{}","ab".repeat(20)),"expected_measurements":[format!("mrtd={}","ab".repeat(48))]}),
            serde_json::json!({"url":"https://api.tracecommons.org","signing_address":format!("0x{}","ab".repeat(20)),"expected_measurements":[]}),
            serde_json::json!({"url":"https://api.tracecommons.org","signing_address":"broken","expected_measurements":["broken"]}),
        ] {
            assert!(validate_published_witness(witness).is_err());
        }
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
                tenant_id: format!("near-{}", "ab".repeat(32)),
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
