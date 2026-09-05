//! Prepare an explicitly selected session's next inference call. This operation
//! sends only a session identifier and account binding, never transcript bodies.

use super::ipc::{DaemonShared, ERR_BAD_PARAMS, ERR_UNAVAILABLE, Request, Response};
use crate::{
    config::{ContributorConfig, allowlist_for},
    identity::DeviceIdentity,
    issuer_client::IssuerClient,
};
use anyhow::{Result, anyhow, bail};
use chrono::Utc;
use serde::Deserialize;
use std::{
    io::{BufRead, BufReader, Read},
    path::Path,
    time::Duration,
};
use trace_commons_protocol::admission::AdmissionBinding;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Params {
    entry_id: uuid::Uuid,
    backend: String,
    confirmed: bool,
}
#[derive(Deserialize)]
struct ProxyCapability {
    supported: bool,
    protocol: String,
    max_lifetime_seconds: i64,
    body_capture_ready: bool,
}
#[derive(Deserialize)]
struct Challenge {
    binding: String,
    expires_at: i64,
}
#[derive(Deserialize)]
struct Registered {
    active: bool,
    expires_at: i64,
}

pub async fn handle_prepare_admission_session(shared: &DaemonShared, req: &Request) -> Response {
    let params: Params = match serde_json::from_value(req.params.clone()) {
        Ok(params) => params,
        Err(_) => return Response::err(req.id, ERR_BAD_PARAMS, "admission_setup_invalid"),
    };
    match prepare(shared, params).await {
        Ok(expires_at) => Response::ok(
            req.id,
            serde_json::json!({"status":"ready_for_next_inference","expires_at":expires_at}),
        ),
        Err(_) => Response::err(req.id, ERR_UNAVAILABLE, "admission_setup_unavailable"),
    }
}
fn check_consent(cfg: &ContributorConfig, body_export: bool, confirmed: bool) -> Result<()> {
    if !confirmed
        || !body_export
        || cfg.consent_scopes.is_empty()
        || !cfg
            .witness
            .as_ref()
            .is_some_and(|w| w.admission_evidence && w.trust().is_ok_and(|t| t.is_pinned()))
    {
        bail!("admission_setup_consent_required");
    }
    Ok(())
}
async fn prepare(shared: &DaemonShared, params: Params) -> Result<i64> {
    // All opt-in gates precede client construction, proxy discovery or any HTTP.
    if !params.confirmed {
        bail!("admission_setup_consent_required");
    }
    let cfg = shared
        .store
        .load_config()?
        .ok_or_else(|| anyhow!("admission_setup_unenrolled"))?;
    let settings = shared.settings.lock().expect("settings lock").clone();
    check_consent(&cfg, settings.ironwire_attested_bodies, params.confirmed)?;
    if params.backend.is_empty()
        || params.backend.len() > 128
        || params.backend.chars().any(char::is_control)
    {
        bail!("admission_setup_invalid");
    }
    let entry = shared
        .queue
        .lock()
        .expect("queue lock")
        .all()
        .iter()
        .find(|entry| entry.entry_id == params.entry_id)
        .cloned()
        .ok_or_else(|| anyhow!("admission_setup_session_missing"))?;
    // Bare sources deliberately omit the routing overlay: extracting this id
    // must not fetch routing records or export any transcript content.
    let sources = crate::source::all_sources(&settings.source_roots(&shared.store));
    let (source, session) = super::find_session(&sources, &entry)
        .ok_or_else(|| anyhow!("admission_setup_session_missing"))?;
    let session_id = exact_session_id(source.name(), &session.path)?;
    let declaration = settings
        .ironwire
        .as_ref()
        .ok_or_else(|| anyhow!("admission_setup_proxy_missing"))?;
    let port = declaration
        .port()
        .filter(|p| *p > 0)
        .ok_or_else(|| anyhow!("admission_setup_proxy_missing"))?;
    let path = super::settings::ironwire_token_path(declaration.token_dir())
        .ok_or_else(|| anyhow!("admission_setup_proxy_missing"))?;
    let metadata = super::ironwire_pointer::trustworthy_file(&path)
        .ok_or_else(|| anyhow!("admission_setup_proxy_untrusted"))?;
    if metadata.len() > 4096 {
        bail!("admission_setup_proxy_untrusted");
    }
    let token = std::fs::read_to_string(path)?;
    let token = token.trim();
    if token.is_empty() || token.len() > 4096 {
        bail!("admission_setup_proxy_untrusted");
    }
    let control = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()?;
    let endpoint = format!("http://127.0.0.1:{port}/_ironwire/admission-binding");
    let capability: ProxyCapability = control
        .get(&endpoint)
        .bearer_auth(token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    if !capability.supported
        || capability.protocol != "openai.chat"
        || !capability.body_capture_ready
        || capability.max_lifetime_seconds <= 0
        || capability.max_lifetime_seconds > 900
    {
        bail!("admission_setup_proxy_unsupported");
    }
    let device = DeviceIdentity::load(&shared.store)?
        .ok_or_else(|| anyhow!("admission_setup_device_missing"))?;
    let allowlist = allowlist_for(cfg.allowed_hosts.as_deref());
    if !allowlist.is_enforcing() {
        bail!("admission_setup_endpoint_untrusted");
    }
    for endpoint in [&cfg.issuer_url, &cfg.ingest_url] {
        let url = reqwest::Url::parse(endpoint)?;
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            bail!("admission_setup_endpoint_untrusted");
        }
        allowlist.check(&url)?;
    }
    let issuer = IssuerClient::new(allowlist.clone())?;
    let signed = crate::identity::build_signed_claim_request(&cfg, &device, Utc::now())?;
    let claim = issuer.mint_claim(&cfg.issuer_url, &signed).await?;
    if !claim.is_fresh(Utc::now()) {
        bail!("admission_setup_claim_expired");
    }
    let ingest = trace_commons_operator_client::Client::builder(
        &cfg.ingest_url,
        "TRACE_COMMONS_UNUSED_BEARER",
    )
    .bearer_token(&claim.access_token)
    .host_allowlist(allowlist)
    .build()?;
    let challenge: Challenge = ingest
        .call_json(
            reqwest::Method::POST,
            "/v1/admission/challenge",
            &[],
            None::<&serde_json::Value>,
        )
        .await?;
    validate_challenge(
        &challenge,
        &cfg.tenant_id,
        Utc::now().timestamp(),
        capability.max_lifetime_seconds,
    )?;
    // Settings/enrollment may change during either remote request. Recheck
    // consent and the complete configuration before mutating the proxy.
    let current = shared
        .store
        .load_config()?
        .ok_or_else(|| anyhow!("admission_setup_state_changed"))?;
    let current_settings = shared.settings.lock().expect("settings lock").clone();
    check_consent(&current, current_settings.ironwire_attested_bodies, true)?;
    if serde_json::to_value(&current)? != serde_json::to_value(&cfg)?
        || current_settings != settings
        || exact_session_id(source.name(), &session.path)? != session_id
    {
        bail!("admission_setup_state_changed");
    }
    register_binding(
        &control,
        &endpoint,
        token,
        &session_id,
        &params.backend,
        &challenge,
    )
    .await
}
async fn register_binding(
    control: &reqwest::Client,
    endpoint: &str,
    token: &str,
    session_id: &str,
    backend: &str,
    challenge: &Challenge,
) -> Result<i64> {
    let registered: Registered = control.post(endpoint).bearer_auth(token)
        .json(&serde_json::json!({"session_id":session_id,"backend":backend,"binding":challenge.binding,"confirmed":true}))
        .send().await?.error_for_status()?.json().await?;
    if !registered.active
        || registered.expires_at != challenge.expires_at
        || registered.expires_at <= Utc::now().timestamp()
    {
        bail!("admission_setup_registration_refused");
    }
    Ok(registered.expires_at)
}
fn validate_challenge(
    challenge: &Challenge,
    tenant: &str,
    now: i64,
    max_lifetime: i64,
) -> Result<()> {
    let binding = AdmissionBinding::parse(&challenge.binding)
        .map_err(|_| anyhow!("admission_setup_binding_invalid"))?;
    if tenant != format!("near-{}", binding.account_anchor_sha256)
        || binding.expires_at != challenge.expires_at
        || binding.expires_at <= now
        || binding.expires_at > now.saturating_add(max_lifetime.min(900))
    {
        bail!("admission_setup_binding_invalid");
    }
    Ok(())
}
fn exact_session_id(source: &str, path: &Path) -> Result<String> {
    if !matches!(source, "codex" | "claude-code") {
        bail!("admission_setup_source_unsupported");
    }
    let reader = BufReader::new(std::fs::File::open(path)?.take(128 * 1024));
    for line in reader.lines().take(128) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line?) else {
            continue;
        };
        let id = match source {
            "codex" if value["type"] == "session_meta" => value["payload"]["id"].as_str(),
            "claude-code" => value["sessionId"].as_str(),
            _ => None,
        };
        if let Some(id) = id {
            let parsed = uuid::Uuid::parse_str(id)?;
            if parsed.to_string() != id {
                bail!("admission_setup_session_invalid");
            }
            return Ok(id.to_string());
        }
    }
    bail!("admission_setup_session_unknown")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> ContributorConfig {
        serde_json::from_value(serde_json::json!({
            "schema_version":crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION,
            "issuer_url":"https://issuer.example","ingest_url":"https://ingest.example",
            "audience":"upload","tenant_id":format!("near-{}","ab".repeat(32)),
            "instance_id":"","user_subject":"device","device_key_id":"device",
            "consent_scopes":["debugging_evaluation"],
            "witness":{"url":"https://witness.example","signing_address":format!("0x{}","ab".repeat(20)),"expected_measurements":[format!("mrtd={}","ab".repeat(48))],"admission_evidence":true}
        })).unwrap()
    }
    #[test]
    fn challenge_is_canonical_account_bound_and_short_lived() {
        let tenant = format!("near-{}", "ab".repeat(32));
        let make = |expiry| Challenge {
            binding: AdmissionBinding {
                account_anchor_sha256: "ab".repeat(32),
                nonce_hex: "cd".repeat(32),
                expires_at: expiry,
            }
            .encode()
            .unwrap(),
            expires_at: expiry,
        };
        assert!(validate_challenge(&make(1100), &tenant, 1000, 900).is_ok());
        assert!(validate_challenge(&make(1100), "near-other", 1000, 900).is_err());
        assert!(validate_challenge(&make(1000), &tenant, 1000, 900).is_err());
        assert!(validate_challenge(&make(1901), &tenant, 1000, 900).is_err());
        let mut mismatch = make(1100);
        mismatch.expires_at = 1101;
        assert!(validate_challenge(&mismatch, &tenant, 1000, 900).is_err());
        let mut malformed = make(1100);
        malformed.binding.push(':');
        assert!(validate_challenge(&malformed, &tenant, 1000, 900).is_err());
    }
    #[test]
    fn extracts_source_metadata_not_queue_id_or_filename() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wrong-id.jsonl");
        let id = "019921c3-6a5c-7d4e-9f00-aaaaaaaaaaaa";
        std::fs::write(
            &path,
            format!("{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\"}}}}\n"),
        )
        .unwrap();
        assert_eq!(exact_session_id("codex", &path).unwrap(), id);
        assert!(exact_session_id("trajectory", &path).is_err());
        std::fs::write(
            &path,
            format!("{{\"sessionId\":\"{id}\",\"message\":\"private body\"}}\n"),
        )
        .unwrap();
        assert_eq!(exact_session_id("claude-code", &path).unwrap(), id);
        assert!(exact_session_id("codex", &path).is_err());
        std::fs::write(
            &path,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"opaque-queue-id\"}}\n",
        )
        .unwrap();
        assert!(exact_session_id("codex", &path).is_err());
    }
    #[tokio::test]
    async fn disabled_body_export_or_unconfirmed_request_makes_no_network_request() {
        let (dir, store) = crate::config::tests_support::temp_store();
        store.save_config(&config()).unwrap();
        let shared = DaemonShared::load(store).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        shared.settings.lock().unwrap().ironwire =
            Some(super::super::settings::IronWireDeclaration::Watch {
                port: listener.local_addr().unwrap().port(),
                token_dir: Some(dir.path().into()),
            });
        assert!(
            prepare(
                &shared,
                Params {
                    entry_id: uuid::Uuid::new_v4(),
                    backend: "near".into(),
                    confirmed: true
                }
            )
            .await
            .is_err()
        );
        shared.settings.lock().unwrap().ironwire_attested_bodies = true;
        assert!(
            prepare(
                &shared,
                Params {
                    entry_id: uuid::Uuid::new_v4(),
                    backend: "near".into(),
                    confirmed: false
                }
            )
            .await
            .is_err()
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(30), listener.accept())
                .await
                .is_err()
        );
    }
    #[test]
    fn admission_mode_does_not_supply_missing_consent() {
        let mut cfg = config();
        assert!(check_consent(&cfg, true, true).is_ok());
        cfg.consent_scopes.clear();
        assert!(check_consent(&cfg, true, true).is_err());
        cfg.consent_scopes.push("debugging_evaluation".into());
        cfg.witness.as_mut().unwrap().admission_evidence = false;
        assert!(check_consent(&cfg, true, true).is_err());
    }
    #[tokio::test]
    async fn registration_sends_exact_binding_and_session_with_control_auth_only() {
        let expires = Utc::now().timestamp() + 120;
        let binding = AdmissionBinding {
            account_anchor_sha256: "ab".repeat(32),
            nonce_hex: "cd".repeat(32),
            expires_at: expires,
        }
        .encode()
        .unwrap();
        let expected = binding.clone();
        let router=axum::Router::new().route("/_ironwire/admission-binding",axum::routing::post(move |headers:axum::http::HeaderMap,axum::Json(body):axum::Json<serde_json::Value>| { let expected=expected.clone(); async move {
            assert_eq!(headers["authorization"],"Bearer control-secret");
            assert_eq!(body,serde_json::json!({"session_id":"exact-session","backend":"funded-backend","binding":expected,"confirmed":true}));
            axum::Json(serde_json::json!({"active":true,"expires_at":expires}))
        }}));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!(
            "http://{}/_ironwire/admission-binding",
            listener.local_addr().unwrap()
        );
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        assert_eq!(
            register_binding(
                &client,
                &endpoint,
                "control-secret",
                "exact-session",
                "funded-backend",
                &Challenge {
                    binding,
                    expires_at: expires
                }
            )
            .await
            .unwrap(),
            expires
        );
        task.abort();
    }
}
