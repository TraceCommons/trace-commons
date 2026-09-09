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
use trace_commons_operator_client::host_allowlist::HostAllowlist;
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

#[derive(Debug, thiserror::Error)]
enum ReceiptEndpointSetupError {
    #[error("admission_receipt_endpoint_required")]
    Required,
    #[error("admission_receipt_endpoint_invalid")]
    Invalid,
}
/// Take up a receipt endpoint this config does not have yet, and save it.
///
/// Split from the fetch so the decision and the persistence can be tested
/// without a commons to talk to. `published` is what the commons answered, or
/// `None` when it published nothing or could not be reached -- an absent value
/// is not an error here, it simply leaves [`require_receipt_endpoint`] to
/// refuse exactly as it did before.
///
/// Saved, not merely set: an endpoint that lived only in this call would be
/// the original defect again, one process-lifetime long.
fn adopt_receipt_endpoint(
    shared: &DaemonShared,
    cfg: &mut ContributorConfig,
    published: Option<&str>,
) -> Result<()> {
    // Each source is vetted by the list that source's own author controls.
    //
    // The operator's variable is checked against the operator's allowlist. A
    // published value cannot be: that list is permissive whenever nobody set
    // `TRACE_COMMONS_ALLOWED_HOSTS`, which is every shipped app, so vetting a
    // server-supplied host with it would be vetting it with nothing. It gets
    // the list this commons's own published hosts derive instead -- the same
    // basis on which that origin's issuer and witness are admitted -- and a
    // host outside it is dropped rather than adopted.
    let derived = published
        .and_then(|endpoint| {
            super::account_onboarding::published_host_allowlist(
                &allowlist_for(cfg.allowed_hosts.as_deref()),
                &cfg.ingest_url,
                endpoint,
            )
            .ok()
        })
        .unwrap_or_else(HostAllowlist::permissive);
    let Some(endpoint) = crate::config::receipt_endpoint_to_adopt(
        cfg.inference_receipt_endpoint.as_deref(),
        crate::config::inference_receipt_endpoint_from_env().as_deref(),
        &allowlist_for(cfg.allowed_hosts.as_deref()),
        published,
        &derived,
    ) else {
        return Ok(());
    };
    cfg.inference_receipt_endpoint = Some(endpoint);
    // The endpoint is a URL: it is saved, never logged.
    shared.store.save_config(cfg)?;
    Ok(())
}

/// Ask this config's commons for a receipt endpoint, when it has none.
///
/// An account enrolled before its commons published a receipt service, or
/// before a build that could read one, has nothing saved and no way to type
/// a value in. This is where it is first needed, so this is where it is asked
/// for -- once, and only when the answer is missing.
///
/// Never fatal. A commons that publishes nothing, is unreachable, or is no
/// longer ready leaves the config alone and [`require_receipt_endpoint`] then
/// refuses exactly as it did before -- an unreachable commons must not turn a
/// clear "no receipt endpoint" into a transport error nobody can read.
async fn adopt_published_receipt_endpoint(
    shared: &DaemonShared,
    cfg: &mut ContributorConfig,
) -> Result<()> {
    if cfg.inference_receipt_endpoint.is_some() {
        return Ok(());
    }
    let published = super::account_onboarding::published_receipt_endpoint(&cfg.ingest_url).await;
    adopt_receipt_endpoint(shared, cfg, published.as_deref())
}

/// A saved receipt endpoint this client will actually call.
///
/// Vetted on the same basis it was adopted on, which is the only basis that
/// can work: native signup persists `allowed_hosts: None`, so
/// `allowlist_for(cfg.allowed_hosts)` is the environment list, and that is
/// permissive on every machine where no operator set one. Checking a saved
/// endpoint against a permissive list refuses it as `Invalid` -- so gating it
/// that way would adopt an endpoint and then reject it one line later, moving
/// the wall rather than removing it.
///
/// The derived list is the commons's own published set, and an operator's
/// list still governs whenever they set one.
fn require_receipt_endpoint(cfg: &ContributorConfig) -> Result<()> {
    let endpoint = cfg
        .inference_receipt_endpoint
        .as_deref()
        .ok_or(ReceiptEndpointSetupError::Required)?;
    let allowlist = super::account_onboarding::published_host_allowlist(
        &allowlist_for(cfg.allowed_hosts.as_deref()),
        &cfg.ingest_url,
        endpoint,
    )
    .map_err(|_| ReceiptEndpointSetupError::Invalid)?;
    crate::config::validate_inference_receipt_endpoint(endpoint, &allowlist)
        .map_err(|_| ReceiptEndpointSetupError::Invalid)?;
    Ok(())
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
        Err(error) => {
            let label = match error.downcast_ref::<ReceiptEndpointSetupError>() {
                Some(ReceiptEndpointSetupError::Required) => "admission_receipt_endpoint_required",
                Some(ReceiptEndpointSetupError::Invalid) => "admission_receipt_endpoint_invalid",
                None => "admission_setup_unavailable",
            };
            Response::err(req.id, ERR_UNAVAILABLE, label)
        }
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
    let mut cfg = shared
        .store
        .load_config()?
        .ok_or_else(|| anyhow!("admission_setup_unenrolled"))?;
    let settings = shared.settings.lock().expect("settings lock").clone();
    check_consent(&cfg, settings.ironwire_attested_bodies, params.confirmed)?;
    adopt_published_receipt_endpoint(shared, &mut cfg).await?;
    require_receipt_endpoint(&cfg)?;
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
    #[tokio::test]
    async fn missing_receipt_endpoint_refuses_before_proxy_contact_with_useful_code() {
        let (_dir, store) = crate::config::tests_support::temp_store();
        store.save_config(&config()).unwrap();
        let shared = DaemonShared::load(store).unwrap();
        shared.settings.lock().unwrap().ironwire_attested_bodies = true;
        let error = prepare(
            &shared,
            Params {
                entry_id: uuid::Uuid::new_v4(),
                backend: "near".into(),
                confirmed: true,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<ReceiptEndpointSetupError>(),
            Some(ReceiptEndpointSetupError::Required)
        ));
        let mut cfg = config();
        cfg.inference_receipt_endpoint = Some("https://receipts.example/v1".into());
        cfg.allowed_hosts = Some("receipts.example,issuer.example,ingest.example".into());
        assert!(require_receipt_endpoint(&cfg).is_ok());
        cfg.inference_receipt_endpoint = Some("http://receipts.example/v1".into());
        let error = require_receipt_endpoint(&cfg).unwrap_err();
        assert!(matches!(
            error.downcast_ref::<ReceiptEndpointSetupError>(),
            Some(ReceiptEndpointSetupError::Invalid)
        ));
    }

    /// The defect, end to end, at the layer that has to fix it.
    ///
    /// A contributor with no `TRACE_COMMONS_INFERENCE_RECEIPT_ENDPOINT` and no
    /// saved endpoint could not reach a preparable session by any route that
    /// did not involve editing `contributor.json` by hand. Here the commons
    /// answers, and the endpoint is adopted and written down.
    ///
    /// The variable is asserted absent, not set: a harness that exported one
    /// would be exercising the single path that already worked, which is the
    /// mistake that let this ship.
    #[test]
    fn a_daemon_with_no_environment_variable_adopts_what_its_commons_published() {
        assert!(
            std::env::var(crate::config::TRACE_COMMONS_INFERENCE_RECEIPT_ENDPOINT).is_err(),
            "this proves nothing unless the environment is silent"
        );
        let (_dir, store) = crate::config::tests_support::temp_store();
        let mut cfg = config();
        // What native signup actually persists. Setting a list here instead
        // would be the test arranging the one condition a shipped app never
        // has, and it would pass while the app stayed broken.
        assert!(cfg.allowed_hosts.is_none());
        store.save_config(&cfg).unwrap();
        let shared = DaemonShared::load(store).unwrap();

        assert!(
            matches!(
                require_receipt_endpoint(&cfg)
                    .unwrap_err()
                    .downcast_ref::<ReceiptEndpointSetupError>(),
                Some(ReceiptEndpointSetupError::Required)
            ),
            "the starting state is the one a contributor is stuck in"
        );

        adopt_receipt_endpoint(&shared, &mut cfg, Some("https://receipts.example/v1")).unwrap();

        assert!(
            require_receipt_endpoint(&cfg).is_ok(),
            "adoption that does not clear the very next gate has moved the wall, not removed it"
        );
        assert_eq!(
            shared
                .store
                .load_config()
                .unwrap()
                .expect("a saved config")
                .inference_receipt_endpoint
                .as_deref(),
            Some("https://receipts.example/v1"),
            "an endpoint held only in memory is the same defect, one process long"
        );
    }

    /// A commons that publishes nothing changes nothing, and in particular
    /// does not write a null over a config.
    #[test]
    fn a_commons_that_publishes_no_endpoint_leaves_the_refusal_standing() {
        let (_dir, store) = crate::config::tests_support::temp_store();
        let mut cfg = config();
        cfg.allowed_hosts = Some("receipts.example,issuer.example,ingest.example".into());
        store.save_config(&cfg).unwrap();
        let shared = DaemonShared::load(store).unwrap();

        adopt_receipt_endpoint(&shared, &mut cfg, None).unwrap();

        assert!(cfg.inference_receipt_endpoint.is_none());
        assert!(matches!(
            require_receipt_endpoint(&cfg)
                .unwrap_err()
                .downcast_ref::<ReceiptEndpointSetupError>(),
            Some(ReceiptEndpointSetupError::Required)
        ));
    }

    /// A commons that cannot be reached is not an error, and does not become
    /// one on the way out.
    ///
    /// This drives the real fetch -- `published_receipt_endpoint`, its client
    /// construction and its validation -- against a commons that answers
    /// nothing, and requires the refusal a contributor sees to still be the
    /// readable `Required` rather than a transport failure wearing
    /// `admission_setup_unavailable`.
    #[tokio::test]
    async fn an_unreachable_commons_leaves_the_readable_refusal_in_place() {
        let (_dir, store) = crate::config::tests_support::temp_store();
        let mut cfg = config();
        cfg.allowed_hosts = Some("receipts.example,issuer.example,ingest.example".into());
        store.save_config(&cfg).unwrap();
        let shared = DaemonShared::load(store).unwrap();

        adopt_published_receipt_endpoint(&shared, &mut cfg)
            .await
            .expect("an unreachable commons is not a failure to adopt");

        assert!(cfg.inference_receipt_endpoint.is_none());
        assert!(matches!(
            require_receipt_endpoint(&cfg)
                .unwrap_err()
                .downcast_ref::<ReceiptEndpointSetupError>(),
            Some(ReceiptEndpointSetupError::Required)
        ));
    }

    /// The wiring itself: a config with no endpoint **does** ask its commons.
    ///
    /// This is the seam a mutation could previously gut without reddening
    /// anything -- replacing the fetch with `None` left every test green,
    /// because an unreachable commons and a commons never asked are
    /// indistinguishable by outcome. Only a connection tells them apart.
    ///
    /// Testable without touching the process environment only because #786
    /// made `client()` take its allowlist as a parameter: the list
    /// `signup_allowlist` derives from the chosen origin is enforcing by
    /// construction, so a loopback origin is dialable with no
    /// `TRACE_COMMONS_ALLOWED_HOSTS` set and no cross-test coupling.
    #[tokio::test]
    async fn a_config_with_no_endpoint_actually_asks_its_commons() {
        assert!(
            std::env::var(crate::config::TRACE_COMMONS_INFERENCE_RECEIPT_ENDPOINT).is_err(),
            "this proves nothing unless the environment is silent"
        );
        let (_dir, store) = crate::config::tests_support::temp_store();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut cfg = config();
        assert!(cfg.allowed_hosts.is_none(), "what native signup persists");
        cfg.ingest_url = format!("https://{}", listener.local_addr().unwrap());
        store.save_config(&cfg).unwrap();
        let shared = DaemonShared::load(store).unwrap();

        let asked = tokio::spawn(async move { listener.accept().await.is_ok() });
        adopt_published_receipt_endpoint(&shared, &mut cfg)
            .await
            .expect("a commons that answers nothing usable is not a failure");

        assert!(
            tokio::time::timeout(Duration::from_secs(5), asked)
                .await
                .expect("the commons was never contacted")
                .expect("accept task"),
            "the daemon has to ask before it can adopt"
        );
        // Nothing usable came back over that socket, so nothing is adopted.
        assert!(cfg.inference_receipt_endpoint.is_none());
    }

    /// An endpoint already saved is not re-fetched, and the check is the
    /// absence of a connection rather than the absence of a change: a commons
    /// asked on every prepare would be a new call on a path that does not
    /// need one.
    #[tokio::test]
    async fn a_config_that_already_has_an_endpoint_does_not_call_its_commons() {
        let (_dir, store) = crate::config::tests_support::temp_store();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut cfg = config();
        cfg.allowed_hosts = Some("127.0.0.1".into());
        cfg.ingest_url = format!("https://{}", listener.local_addr().unwrap());
        cfg.inference_receipt_endpoint = Some("https://127.0.0.1/v1".into());
        store.save_config(&cfg).unwrap();
        let shared = DaemonShared::load(store).unwrap();

        adopt_published_receipt_endpoint(&shared, &mut cfg)
            .await
            .unwrap();

        assert_eq!(
            cfg.inference_receipt_endpoint.as_deref(),
            Some("https://127.0.0.1/v1")
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(50), listener.accept())
                .await
                .is_err(),
            "the commons was asked for an endpoint this config already had"
        );
    }

    /// The commons is not trusted to pick this value: a published endpoint the
    /// operator's own allowlist does not admit is not adopted, and the
    /// contributor is left refusing rather than quietly calling a new host.
    #[test]
    fn a_published_endpoint_outside_the_allowlist_is_not_adopted() {
        let (_dir, store) = crate::config::tests_support::temp_store();
        let mut cfg = config();
        cfg.allowed_hosts = Some("issuer.example,ingest.example".into());
        store.save_config(&cfg).unwrap();
        let shared = DaemonShared::load(store).unwrap();

        adopt_receipt_endpoint(&shared, &mut cfg, Some("https://elsewhere.example/v1")).unwrap();

        assert!(cfg.inference_receipt_endpoint.is_none());
        assert!(
            shared
                .store
                .load_config()
                .unwrap()
                .expect("a saved config")
                .inference_receipt_endpoint
                .is_none()
        );
    }
}
