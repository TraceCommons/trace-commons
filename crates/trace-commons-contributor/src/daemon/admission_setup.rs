//! Prepare an explicitly selected session's next inference call. This operation
//! sends only a session identifier and account binding, never transcript bodies.

use super::ipc::{DaemonShared, ERR_BAD_PARAMS, ERR_UNAVAILABLE, Request, Response};
use crate::{
    config::{ContributorConfig, config_allowlist},
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

#[derive(Debug, thiserror::Error)]
enum ReceiptEndpointSetupError {
    #[error("admission_receipt_endpoint_required")]
    Required,
    #[error("admission_receipt_endpoint_invalid")]
    Invalid,
}
fn require_receipt_endpoint(cfg: &ContributorConfig) -> Result<()> {
    let endpoint = cfg
        .inference_receipt_endpoint
        .as_deref()
        .ok_or(ReceiptEndpointSetupError::Required)?;
    crate::config::validate_inference_receipt_endpoint(endpoint, &config_allowlist(cfg))
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
    let cfg = shared
        .store
        .load_config()?
        .ok_or_else(|| anyhow!("admission_setup_unenrolled"))?;
    let settings = shared.settings.lock().expect("settings lock").clone();
    check_consent(&cfg, settings.ironwire_attested_bodies, params.confirmed)?;
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
    let allowlist = validated_endpoint_allowlist(&cfg)?;
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
/// The hosts this enrolled config may dial for admission, or a refusal.
///
/// Extracted from `prepare` so it can be tested. Inline, it sat downstream of
/// a queue lookup and a session-file read, so no test could reach it without
/// building a whole session -- and a mutation reverting it to
/// `allowlist_for(cfg.allowed_hosts)`, which is the defect this PR fixes,
/// survived the suite.
///
/// The gate itself is unchanged and just as fail-closed: a non-enforcing list
/// refuses, and both endpoints must be clean HTTPS *and* on the list. What
/// changed is only where the list comes from -- see
/// [`crate::config::config_allowlist`].
fn validated_endpoint_allowlist(
    cfg: &ContributorConfig,
) -> Result<trace_commons_operator_client::host_allowlist::HostAllowlist> {
    let allowlist = config_allowlist(cfg);
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
    Ok(allowlist)
}

fn validate_challenge(
    challenge: &Challenge,
    tenant: &str,
    now: i64,
    max_lifetime: i64,
) -> Result<()> {
    let binding = AdmissionBinding::parse(&challenge.binding)
        .map_err(|_| anyhow!("admission_setup_binding_invalid"))?;
    // The tenant id is NOT derivable from the anchor. This used to require
    // `tenant == format!("near-{}", binding.account_anchor_sha256)`, which V58
    // held true with `CHECK (tenant_id = 'near-' || substring(anchor_hash from
    // 8))`. V61 dropped that constraint on purpose -- `anchor_hash` became a
    // keyed blind index and `tenant_id` 32 random bytes -- so the equality has
    // been false for every real account since, and this refused every genuine
    // challenge. See #785, which removed the same comparison on the server.
    //
    // What stays is the namespace discriminator: a challenge for this path
    // belongs to a wallet tenant, and the anchor's own shape is already
    // enforced by `AdmissionBinding::parse`, which round-trips through
    // `encode` and so rejects anything that is not a lowercase hex digest.
    // Nothing re-derives one value from the other, in either direction: that
    // coupling is the offline-computability defect V61 fixed (#716).
    if !crate::config::is_near_tenant_id(tenant)
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
            "audience":"upload","tenant_id":format!("near-{}",TENANT_SUFFIX),
            "instance_id":"","user_subject":"device","device_key_id":"device",
            "consent_scopes":["debugging_evaluation"],
            "witness":{"url":"https://witness.example","signing_address":format!("0x{}","ab".repeat(20)),"expected_measurements":[format!("mrtd={}","ab".repeat(48))],"admission_evidence":true}
        })).unwrap()
    }
    /// A tenant id and an account anchor drawn independently, which is the
    /// only shape V61 leaves possible: `tenant_id` became 32 random bytes and
    /// `anchor_hash` a keyed blind index, and V58's
    /// `CHECK (tenant_id = 'near-' || substring(anchor_hash from 8))` was
    /// dropped. Every fixture in this file used to set them equal -- the one
    /// shape a real account can no longer have -- so the fixture agreed with
    /// the bug and the test passed while the application refused every
    /// genuine challenge.
    const TENANT_SUFFIX: &str = "3c9f21d8be4a07655c1e3fba8d02947613ae5c80f9d64b2718a350ecdb6f4192";
    const ANCHOR: &str = "ab00c4d1e97f3625b8a01d4fce7382905b6ad3f1e0c95847a2b6f30d19e4c785";

    /// The config here is written by `account_onboarding::persist` -- the real
    /// signup path -- and never by hand. A hand-built config sets
    /// `allowed_hosts` itself and so passes these gates before and after the
    /// fix, which is why nothing caught that a wallet-enrolled contributor
    /// could not prepare a bound session on any shipped application.
    ///
    /// Deliberately asserts on the two gates `prepare` reaches before it opens
    /// a socket -- `require_receipt_endpoint` at line 109 and the enforcing
    /// check on the issuer/ingest list -- rather than on a helper's return
    /// value. An out-of-process test the way `daemon_wallet_signup_without_env`
    /// does it is not available here: `persist` is private, and producing this
    /// config in a spawned daemon would need a live HTTPS commons to sign up
    /// against.
    #[test]
    fn a_config_written_by_signup_reaches_the_admission_gates() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = super::super::account_onboarding::signup_written_config(
            dir.path(),
            Some("https://receipts.example/v1".into()),
        );
        assert!(
            cfg.allowed_hosts.is_none(),
            "signup writes no host list; the list has to come from the config's own hosts"
        );

        let allowlist = config_allowlist(&cfg);
        assert!(
            allowlist.is_enforcing(),
            "a signup-written config must yield an enforcing list, or every gate below refuses"
        );
        // The two gates that actually failed, driven exactly as `prepare`
        // drives them.
        require_receipt_endpoint(&cfg).expect("a signup-written receipt endpoint must be usable");
        validated_endpoint_allowlist(&cfg)
            .expect("a signup-written issuer and ingest must be dialable");
        // A config whose endpoints are not clean HTTPS is still refused, and
        // by the gate rather than by the list.
        let mut credentialed = cfg.clone();
        credentialed.issuer_url = "https://user@issuer.example".into();
        assert!(validated_endpoint_allowlist(&credentialed).is_err());

        // Every host this config points at, including the receipt endpoint --
        // which is on the inference provider and not on the commons, and which
        // `submit.rs` needs for every receipt.
        for url in [
            cfg.issuer_url.as_str(),
            cfg.ingest_url.as_str(),
            cfg.witness.as_ref().unwrap().url.as_str(),
            cfg.inference_receipt_endpoint.as_deref().unwrap(),
        ] {
            allowlist
                .check(&reqwest::Url::parse(url).unwrap())
                .unwrap_or_else(|_| panic!("{url} is named by the config and must be reachable"));
        }
        // And it is still a list, not a bypass.
        for outside in ["https://elsewhere.example", "https://commons.example.evil"] {
            assert!(
                allowlist
                    .check(&reqwest::Url::parse(outside).unwrap())
                    .is_err(),
                "{outside} is named by nothing and must not be reachable"
            );
        }
    }

    /// The derivation itself, with the configured list passed in rather than
    /// read from the environment, so this says the same thing on a machine
    /// that happens to have `TRACE_COMMONS_ALLOWED_HOSTS` set.
    #[test]
    fn an_operator_list_still_governs_and_an_empty_config_allows_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = super::super::account_onboarding::signup_written_config(dir.path(), None);

        let operator = crate::config::derive_config_allowlist(
            &trace_commons_operator_client::host_allowlist::HostAllowlist::from_csv(
                "commons.example",
            ),
            &cfg,
        );
        operator
            .check(&reqwest::Url::parse("https://commons.example").unwrap())
            .unwrap();
        assert!(
            operator
                .check(&reqwest::Url::parse("https://issuer.example").unwrap())
                .is_err(),
            "an operator who lists hosts keeps listing them; the config does not widen it"
        );

        // A receipt endpoint absent from the config is absent from the list.
        let derived = crate::config::derive_config_allowlist(
            &trace_commons_operator_client::host_allowlist::HostAllowlist::permissive(),
            &cfg,
        );
        assert!(
            derived
                .check(&reqwest::Url::parse("https://receipts.example/v1").unwrap())
                .is_err()
        );

        // A config naming no parseable host refuses everything rather than
        // allowing everything.
        let mut empty = cfg.clone();
        empty.issuer_url = String::new();
        empty.ingest_url = String::new();
        empty.witness = None;
        empty.inference_receipt_endpoint = None;
        let nothing = crate::config::derive_config_allowlist(
            &trace_commons_operator_client::host_allowlist::HostAllowlist::permissive(),
            &empty,
        );
        assert!(nothing.is_enforcing());
        assert!(
            nothing
                .check(&reqwest::Url::parse("https://anything.example").unwrap())
                .is_err()
        );
    }

    #[test]
    fn a_v61_tenant_id_is_not_derivable_from_its_anchor() {
        assert_ne!(TENANT_SUFFIX, ANCHOR, "the fixture must not restate V58");
        assert_eq!(TENANT_SUFFIX.len(), 64);
        assert_eq!(ANCHOR.len(), 64);
        for value in [TENANT_SUFFIX, ANCHOR] {
            assert!(
                value
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
                "{value} is not a lowercase hex digest"
            );
        }
    }

    #[test]
    fn challenge_is_canonical_account_bound_and_short_lived() {
        let tenant = format!("near-{TENANT_SUFFIX}");
        let make = |expiry| Challenge {
            binding: AdmissionBinding {
                account_anchor_sha256: ANCHOR.to_string(),
                nonce_hex: "cd".repeat(32),
                expires_at: expiry,
            }
            .encode()
            .unwrap(),
            expires_at: expiry,
        };
        assert!(validate_challenge(&make(1100), &tenant, 1000, 900).is_ok());
        // A tenant that is not on the wallet path at all is refused: the
        // `near-` namespace is a discriminator, and this is the only thing
        // about the tenant a client can still check.
        for outside in [
            "near-other",
            &format!("tenant-{TENANT_SUFFIX}"),
            &format!("near-{}", TENANT_SUFFIX.to_ascii_uppercase()),
            &format!("near-{}", &TENANT_SUFFIX[..63]),
            "near-",
        ] {
            assert!(
                validate_challenge(&make(1100), outside, 1000, 900).is_err(),
                "{outside} is not a wallet tenant"
            );
        }
        // And a DIFFERENT well-formed wallet tenant is accepted, deliberately.
        // Post-V61 a client cannot tell one anchor's tenant from another's --
        // that is what salting the anchor bought -- so this check is a
        // namespace test and never an account binding. What binds a challenge
        // to an account is the server's row lookup under the authenticated
        // session that minted it (#785). Restoring a comparison here would
        // reintroduce the offline-computability defect V61 fixed.
        assert!(
            validate_challenge(&make(1100), &format!("near-{ANCHOR}"), 1000, 900).is_ok(),
            "a wallet tenant must not have to match the anchor"
        );
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
}
