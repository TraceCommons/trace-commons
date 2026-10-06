//! Enrolling this device against a commons using the NEAR AI login the
//! contributor already has, instead of a NEAR wallet.
//!
//! Invite-free admission needs an identity to count abuse limits against. It
//! does not need a wallet: a contributor cannot produce an admissible receipt
//! without a NEAR AI account in the first place, because the receipt comes
//! from their own inference calls. Requiring a wallet as well is a second
//! onboarding for an identity they already have. See #836.
//!
//! # What proves the login
//!
//! A client-asserted account id is worthless here — an attacker runs a
//! modified client and asserts a fresh one per submission, which is the sybil
//! problem the anchor exists to bound. So the client sends NEAR AI's own
//! short-lived JWT and the commons introspects it. **The account is whatever
//! the commons resolves that token to; this module never sends an account id
//! and there is no parameter for one.**
//!
//! # What the JWT alone cannot do
//!
//! Introspection says which NEAR AI account holds the token. It does not say
//! which device is asking, so a leaked JWT would otherwise provision an
//! attacker's device against someone else's identity. The device signature
//! closes that: the commons issues a nonce, the client signs a preimage
//! binding that nonce to the ceremony, to this device's public key and to the
//! PKCE challenge, and the commons checks it against the key it is being asked
//! to enroll. Both halves compute the preimage with
//! [`trace_commons_protocol::onboarding::near_ai_provisioning_device_bytes`],
//! whose domain string differs from the wallet ceremony's so a signature can
//! never cross between them.
//!
//! # Secrets
//!
//! The refresh token and the JWT are both bearer secrets and neither is this
//! module's to hold. The refresh token stays where
//! [`super::nearai_credential`] keeps it and is spent through the shared
//! `exchange`, which persists the rotation before the token is used. The JWT
//! exists for one request and goes in that request's **body** — never an
//! `Authorization` header, because we are not authenticating to the commons
//! with it, we are handing it over to be introspected. Neither value reaches a
//! log line, an error string, a config field, or an IPC response; failures
//! report a control name.
//!
//! # Hosts
//!
//! Two clients on two different bases of trust, and they do not mix. The
//! commons calls go through [`super::account_onboarding::client`] under the
//! derived signup allowlist. The token exchange goes to `cloud-api.near.ai`,
//! a compiled-in constant with no configuration that can move it — see
//! [`super::nearai_credential::api`]. It is deliberately **not** added to the
//! signup allowlist: that list constrains configurable destinations, and
//! putting a fixed endpoint on it would imply the host is subject to
//! configuration.

use super::account_onboarding::{client, signup_allowlist};
use super::ipc::{DaemonShared, ERR_BAD_PARAMS, ERR_UNAVAILABLE, Request, Response};
use super::nearai_credential::api::CloudApi;
use crate::config::{CONTRIBUTOR_CONFIG_SCHEMA_VERSION, ConfigStore, ContributorConfig};
use crate::identity::DeviceIdentity;
use anyhow::{Result, anyhow, bail};
use base64::Engine;
use chrono::Utc;
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// The tenant namespace this ceremony allocates. Distinct from the wallet
/// path's `near-`, and not a prefix of it in either direction.
const TENANT_PREFIX: &str = "nearai-";

#[derive(Deserialize)]
struct Started {
    ceremony_id: String,
    nonce: String,
    expires_at: i64,
}

#[derive(Deserialize)]
struct Finished {
    access_token: String,
    token_type: String,
    expires_in_secs: i64,
    account_id: String,
    tenant_id: String,
    device_key_id: String,
    anchor_hash: String,
}

/// Shape check for a NEAR AI tenant id: the `nearai-` namespace plus 64
/// lowercase hex characters.
///
/// Shape is all a client can check, exactly as for the wallet namespace: the
/// tenant id is drawn from the server's RNG and is a function of nothing, so
/// it cannot be re-derived here. The value is authenticated by the session
/// token issued alongside it, not by its own contents. Do not add a
/// relationship between this and `anchor_hash` in either direction — that
/// coupling is the offline-computability defect V61 removed.
#[must_use]
pub fn is_near_ai_tenant_id(tenant_id: &str) -> bool {
    tenant_id.strip_prefix(TENANT_PREFIX).is_some_and(|suffix| {
        suffix.len() == 64
            && suffix
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    })
}

/// Enroll this device against `ingest_url` using the retained NEAR AI login.
///
/// Deliberately takes no account id and no token: the account comes from the
/// commons's introspection of the JWT, and the JWT comes from the session this
/// daemon already holds. A caller cannot name either.
pub(super) async fn handle_enroll(shared: &DaemonShared, req: &Request) -> Response {
    let Some(ingest_url) = req.params.get("ingest_url").and_then(|v| v.as_str()) else {
        return Response::err(req.id, ERR_BAD_PARAMS, "near_ai_enroll_invalid");
    };
    let api = match CloudApi::live() {
        Ok(api) => api,
        Err(_) => {
            return Response::err(req.id, ERR_UNAVAILABLE, "near_ai_enroll_token_unavailable");
        }
    };
    match enroll(shared, &api, ingest_url).await {
        Ok(value) => Response::ok(req.id, value),
        // Label only. Every failure below already carries a control name, and
        // the errors underneath them can quote a remote body or a URL.
        Err(error) => Response::err(req.id, ERR_UNAVAILABLE, label(&error)),
    }
}

/// The labels this module is allowed to return. Anything unrecognised becomes
/// the generic one rather than being forwarded — an error string from a
/// transport or a remote body is not ours to hand to a shell.
fn label(error: &anyhow::Error) -> &'static str {
    match error.to_string().as_str() {
        "near_ai_enroll_already_enrolled" => "near_ai_enroll_already_enrolled",
        "near_ai_enroll_no_session" => "near_ai_enroll_no_session",
        "near_ai_enroll_endpoint_refused" => "near_ai_enroll_endpoint_refused",
        "near_ai_enroll_token_unavailable" => "near_ai_enroll_token_unavailable",
        "near_ai_enroll_start_failed" => "near_ai_enroll_start_failed",
        "near_ai_enroll_commons_unreachable" => "near_ai_enroll_commons_unreachable",
        "near_ai_enroll_commons_unsupported" => "near_ai_enroll_commons_unsupported",
        "near_ai_enroll_invalid" => "near_ai_enroll_invalid",
        "near_ai_enroll_verification_failed" => "near_ai_enroll_verification_failed",
        _ => "near_ai_enroll_unavailable",
    }
}

/// `api` is a parameter rather than constructed here so a test can pass a
/// double and observe whether the token exchange was reached at all. Without
/// that, a test asserting "the refresh token was not spent" cannot tell a
/// correct ordering from an exchange that merely failed for want of a network,
/// and passes either way.
/// Marshal one started ceremony into the device proof the commons will check.
///
/// # Why this is `pub` and a separate function
///
/// Not for reuse -- `enroll` is the only caller and must stay so. It is `pub`
/// so the **wire contract is testable**: this is the step where the client
/// decides how to read `nonce` off the response and what to feed the shared
/// preimage function, and that decision is exactly where the two halves of
/// #836 can disagree while each side's own tests stay green. Inline, it is
/// reachable only by standing up a capability route, a host allowlist, a
/// session and a NEAR AI token exchange -- four stubs that exercise none of it.
///
/// `witness::transport::witness_request_body` is `pub` for the same reason and
/// says so; this is the same boundary. **Do not fold it back inline as a
/// tidy-up** -- doing so removes the only seam at which a client and a server
/// can be shown to agree about these five values.
///
/// One shared function computes the preimage on both sides, so the byte layout
/// cannot diverge. What that does not close is whether both sides pass it the
/// same arguments, and this is the client's half of that.
///
/// `now` is a parameter rather than read here so the freshness refusal can be
/// tested without sleeping.
pub fn device_proof_for_ceremony(
    identity: &DeviceIdentity,
    ceremony_id: &str,
    nonce_wire: &str,
    code_challenge: &str,
    expires_at: i64,
    now: i64,
) -> Result<String> {
    let device: [u8; 32] = base64::engine::general_purpose::STANDARD
        .decode(&identity.public_key_b64)?
        .try_into()
        .map_err(|_| anyhow!("near_ai_enroll_invalid"))?;
    let nonce: [u8; 32] = base64::engine::general_purpose::STANDARD
        .decode(nonce_wire)
        .map_err(|_| anyhow!("near_ai_enroll_invalid"))?
        .try_into()
        .map_err(|_| anyhow!("near_ai_enroll_invalid"))?;
    // A ceremony that has already expired, or one whose window is longer than
    // the server is allowed to offer, is refused before this device signs
    // anything for it.
    if ceremony_id.is_empty()
        || ceremony_id.len() > 128
        || expires_at <= now
        || expires_at > now.saturating_add(600)
    {
        bail!("near_ai_enroll_invalid")
    }
    Ok(identity.sign_b64(
        &trace_commons_protocol::onboarding::near_ai_provisioning_device_bytes(
            &nonce,
            ceremony_id,
            &device,
            code_challenge,
            expires_at,
        ),
    ))
}

async fn enroll(
    shared: &DaemonShared,
    api: &CloudApi,
    ingest_url: &str,
) -> Result<serde_json::Value> {
    let expected = super::commons_credentials::snapshot(
        &shared.store,
        super::commons_credentials::Kind::Account,
    )?;
    if shared.store.load_config()?.is_some() {
        bail!("near_ai_enroll_already_enrolled")
    }
    let prepared = prepare(shared, ingest_url).await?;
    let identity = DeviceIdentity::load_or_generate_async(&shared.store).await?;
    let provisioned = provision(shared, api, prepared, &identity).await?;

    let store = shared.store.clone();
    let ingest_url = ingest_url.to_string();
    tokio::task::spawn_blocking(move || {
        persist(
            &store,
            Commons {
                ingest_url: &ingest_url,
                issuer_url: &provisioned.issuer_url,
                audience: &provisioned.audience,
                witness: provisioned.witness,
                receipt_endpoint: provisioned.receipt_endpoint,
            },
            &identity,
            provisioned.finished,
            Some(&expected),
        )
    })
    .await
    .map_err(|_| anyhow!("commons_credential_worker_unavailable"))?
}

/// Everything checked before a device key is chosen or the refresh token is
/// spent: the address, the retained NEAR AI login, and the commons's own
/// facts. Shared by enrollment and by the legacy invite migration, which
/// provisions a staged key against the commons the daemon is already
/// enrolled in.
pub(super) struct Prepared {
    commons: trace_commons_operator_client::Client,
    session: super::settings::NearAiSession,
    issuer_url: String,
    audience: String,
    witness: crate::config::WitnessSettings,
    receipt_endpoint: Option<String>,
}

/// A finished ceremony and the commons facts it ran against. Not yet
/// validated: `persist` and `provisioned_account` each check `finished`.
pub(super) struct Provisioned {
    issuer_url: String,
    audience: String,
    witness: crate::config::WitnessSettings,
    receipt_endpoint: Option<String>,
    finished: Finished,
}

pub(super) async fn prepare(shared: &DaemonShared, ingest_url: &str) -> Result<Prepared> {
    // Refuse the address before spending the refresh token. The exchange
    // retires the token that authenticated it, so a rotation burned against an
    // endpoint we were never going to accept costs the contributor a
    // credential for nothing.
    let allowed = signup_allowlist(ingest_url, &[])
        .map_err(|_| anyhow!("near_ai_enroll_endpoint_refused"))?;
    let commons =
        client(ingest_url, &allowed).map_err(|_| anyhow!("near_ai_enroll_endpoint_refused"))?;

    let session = shared
        .settings
        .lock()
        .expect("settings lock")
        .near_ai_session
        .clone()
        .ok_or_else(|| anyhow!("near_ai_enroll_no_session"))?;

    // Fetched after the session presence check above, which is a local read
    // that spends nothing: a contributor who never logged in should be told
    // that, not that the commons is unreachable.
    //
    // The commons's own facts, from the same route and the same validator the
    // wallet ceremony uses. `issuer_url` and `audience` are what `submit`
    // mints upload claims with, so an enrollment without them is enrolled and
    // unable to upload -- a config that fails at the contributor's first
    // submission rather than here, where they could still act on it.
    //
    // This also extends the derived allowlist with the issuer, witness and
    // receipt hosts this origin publishes, which nothing on this path would
    // otherwise ask for. Before the session is read and well before the token
    // is spent: a commons that is unreachable or not offering enrollment must
    // not cost the contributor their refresh token either.
    let (issuer_url, audience, witness, receipt_endpoint) =
        super::account_onboarding::validated_capability(
            ingest_url,
            super::account_onboarding::Path::NearAiLogin,
        )
        .await
        .map_err(|refusal| match refusal {
            super::account_onboarding::SignupRefusal::AddressRefused => {
                anyhow!("near_ai_enroll_endpoint_refused")
            }
            super::account_onboarding::SignupRefusal::Unreachable => {
                anyhow!("near_ai_enroll_commons_unreachable")
            }
            super::account_onboarding::SignupRefusal::Unsupported => {
                anyhow!("near_ai_enroll_commons_unsupported")
            }
        })?;
    Ok(Prepared {
        commons,
        session,
        issuer_url,
        audience,
        witness,
        receipt_endpoint,
    })
}

/// Run the ceremony for `identity`: start, device proof, token exchange,
/// finish. The refresh token is spent only here, after every local check.
pub(super) async fn provision(
    shared: &DaemonShared,
    api: &CloudApi,
    prepared: Prepared,
    identity: &DeviceIdentity,
) -> Result<Provisioned> {
    let Prepared {
        commons,
        session,
        issuer_url,
        audience,
        witness,
        receipt_endpoint,
    } = prepared;
    let verifier = random()?;
    let code_challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));

    let started: Started = commons
        .call_json(
            reqwest::Method::POST,
            "/v1/account/near-ai/provision/start",
            &[],
            Some(&start_payload(&code_challenge, &identity.public_key_b64)),
        )
        .await
        .map_err(|_| anyhow!("near_ai_enroll_start_failed"))?;

    let device_signature = device_proof_for_ceremony(
        identity,
        &started.ceremony_id,
        &started.nonce,
        &code_challenge,
        started.expires_at,
        Utc::now().timestamp(),
    )?;

    // Spend the refresh token as late as possible, for the reason above. The
    // rotation is persisted inside `exchange` before the JWT is used.
    let access_token = super::nearai_credential::exchange(shared, api, &session)
        .await
        .map_err(|_| anyhow!("near_ai_enroll_token_unavailable"))?;

    let finished: Finished = commons
        .call_json(
            reqwest::Method::POST,
            "/v1/account/near-ai/provision/finish",
            &[],
            // The JWT rides in the body, never a header: we are not
            // authenticating to the commons with it, we are handing it over to
            // be introspected.
            Some(&serde_json::json!({
                "ceremony_id": started.ceremony_id,
                "code_verifier": verifier,
                "device_public_key": identity.public_key_b64,
                "device_signature": device_signature,
                "access_token": access_token.access_token,
            })),
        )
        .await
        .map_err(|_| anyhow!("near_ai_enroll_verification_failed"))?;
    Ok(Provisioned {
        issuer_url,
        audience,
        witness,
        receipt_endpoint,
        finished,
    })
}

/// The account a ceremony provisioned `identity` into, for a caller that is
/// not enrolling from nothing: the legacy invite migration. Checked exactly
/// as `persist` checks it, plus the account id the link statement names.
pub(super) fn provisioned_account(
    provisioned: Provisioned,
    identity: &DeviceIdentity,
) -> Result<super::legacy_migration::ProvisionedAccount> {
    validate_finished(&provisioned.finished, identity)?;
    let account_id = uuid::Uuid::parse_str(&provisioned.finished.account_id)
        .map_err(|_| anyhow!("near_ai_enroll_invalid"))?;
    Ok(super::legacy_migration::ProvisionedAccount {
        tenant_id: provisioned.finished.tenant_id,
        account_id,
        access_token: provisioned.finished.access_token,
        expires_in_secs: provisioned.finished.expires_in_secs,
        issuer_url: provisioned.issuer_url,
        audience: provisioned.audience,
    })
}

fn validate_finished(result: &Finished, identity: &DeviceIdentity) -> Result<()> {
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
        || !is_near_ai_tenant_id(&result.tenant_id)
        || result.account_id.is_empty()
        || result.account_id.len() > 256
    {
        bail!("near_ai_enroll_invalid")
    }
    Ok(())
}

fn random() -> Result<String> {
    let mut bytes = [0; 32];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
        .map_err(|_| anyhow!("near_ai_enroll_unavailable"))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

/// Validate the server's answer and publish the enrollment, or publish
/// nothing.
///
/// A partial enrollment is worse than none: a config without its session is an
/// enrolled client that cannot authenticate and cannot re-enroll, because
/// `enroll` refuses once a config exists. So the session is written first and
/// the config is linked into place last, exactly as the wallet ceremony does.
/// The commons facts an enrollment is written against, so `persist` takes one
/// argument for them rather than five positional strings that can be swapped.
struct Commons<'a> {
    ingest_url: &'a str,
    issuer_url: &'a str,
    audience: &'a str,
    witness: crate::config::WitnessSettings,
    receipt_endpoint: Option<String>,
}

/// The account a ceremony must have answered with, when the caller is signed
/// in to one and the outcome keeps it (bind's `bound`, enrol's `enrolled`).
struct ExpectedAccount {
    tenant_id: String,
    account_id: String,
}

fn persist(
    store: &ConfigStore,
    commons: Commons<'_>,
    identity: &DeviceIdentity,
    result: Finished,
    expected: Option<&super::commons_credentials::Snapshot>,
) -> Result<serde_json::Value> {
    persist_for(store, commons, identity, result, expected, None)
}

/// [`persist`], refusing (before anything is written) a result whose tenant
/// or account is not `expected_account`. Defence in depth for bind and
/// enrol: the commons already refuses an enrolment into any other account,
/// and this makes a commons that answered otherwise publish nothing.
fn persist_for(
    store: &ConfigStore,
    commons: Commons<'_>,
    identity: &DeviceIdentity,
    result: Finished,
    expected: Option<&super::commons_credentials::Snapshot>,
    expected_account: Option<&ExpectedAccount>,
) -> Result<serde_json::Value> {
    validate_finished(&result, identity)?;
    if let Some(account) = expected_account {
        if result.tenant_id != account.tenant_id || result.account_id != account.account_id {
            bail!("account-enrollment-mismatch")
        }
    }

    let dir = store.dir().to_path_buf();
    let store = ConfigStore::open(dir.clone())?;
    if store.load_config()?.is_some()
        || DeviceIdentity::load(&store)?.is_none_or(|k| k.device_key_id != identity.device_key_id)
    {
        bail!("near_ai_enroll_invalid")
    }

    let config = ContributorConfig {
        schema_version: CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
        issuer_url: commons.issuer_url.to_string(),
        ingest_url: commons.ingest_url.to_string(),
        audience: commons.audience.to_string(),
        tenant_id: result.tenant_id.clone(),
        instance_id: String::new(),
        user_subject: identity.device_key_id.clone(),
        device_key_id: identity.device_key_id.clone(),
        consent_scopes: Vec::new(),
        pii_filter: None,
        allowed_hosts: None,
        display_handle: None,
        public_bio: None,
        public_since: None,
        witness_origin: Some(crate::config::WitnessOriginRecord::for_witness(
            &commons.witness,
            crate::config::WitnessOrigin::PublishedAtJoin,
        )),
        witness: Some(commons.witness),
        inference_receipt_endpoint: commons.receipt_endpoint,
        consent_scopes_chosen: false,
        inference_receipt_check_attestation: true,
    };

    let session = crate::account_auth::AccountSession {
        access_token: result.access_token,
        expires_at: Utc::now() + chrono::Duration::seconds(result.expires_in_secs),
        account_id: result.account_id.clone(),
    };

    let fresh;
    let expected = match expected {
        Some(expected) => expected,
        None => {
            fresh = super::commons_credentials::snapshot(
                &store,
                super::commons_credentials::Kind::Account,
            )?;
            &fresh
        }
    };
    super::commons_credentials::replace(
        &store,
        expected,
        &serde_json::to_vec(&session)?,
        Some(&config),
    )?;

    // Deliberately not the account id or anything token-shaped beyond what a
    // shell needs to render "you are enrolled".
    Ok(serde_json::json!({
        "enrolled": true,
        "tenant_id": result.tenant_id,
        "account_id": result.account_id,
        "device_key_id": identity.device_key_id,
    }))
}

/// The body `enroll` sends to `provision/start`.
///
/// # Why this is `pub`
///
/// The same reason as [`device_proof_for_ceremony`], and it is the same seam.
/// `enroll` is the only caller and must stay so; this is `pub` so the **wire
/// contract is testable**, because the server parses this body with
/// `deny_unknown_fields` and a **required** `code_challenge_method`.
///
/// That requirement is exactly what the shipped client missed. It sent
/// `code_challenge` and `device_public_key` alone, the missing field failed the
/// `Deserialize`, and `provision/start` was refused at the parse boundary on
/// every attempt -- while both halves' suites stayed green, because the
/// server's tests built their own well-formed body and the client's tests never
/// sent one. A cross-check that constructs its own start request cannot see
/// this class at all; only one that calls the function the client actually
/// calls can.
///
/// **Do not fold it back inline as a tidy-up.** Inline, it is reachable only by
/// standing up a capability route, a host allowlist, a session and a token
/// exchange, and the seam at which the two halves can be shown to agree about
/// this body disappears.
pub fn start_payload(code_challenge: &str, device_public_key: &str) -> serde_json::Value {
    serde_json::json!({
        "code_challenge": code_challenge,
        "code_challenge_method": "S256",
        "device_public_key": device_public_key,
    })
}

#[cfg(test)]
#[path = "nearai_onboarding_contract_tests.rs"]
mod contract_tests;

#[cfg(test)]
mod tests {
    use super::*;

    const REFRESH: &str = "rt_fixture-refresh-secret";

    fn shared_with_session() -> DaemonShared {
        let (dir, store) = crate::config::tests_support::temp_store();
        std::mem::forget(dir);
        let shared = DaemonShared::load(store).unwrap();
        let mut settings = shared.settings.lock().unwrap();
        settings.near_ai_session = Some(crate::daemon::settings::NearAiSession {
            refresh_token: REFRESH.into(),
            refresh_token_expires_at: None,
            stored_at: Utc::now(),
            user_agent: "Mozilla/5.0 Test".into(),
        });
        settings.save_for_test(&shared.store).unwrap();
        drop(settings);
        shared
    }

    fn stored_refresh(shared: &DaemonShared) -> Option<String> {
        crate::daemon::settings::DaemonSettings::load_with_cloud_credentials(&shared.store)
            .unwrap()
            .near_ai_session
            .map(|s| s.refresh_token)
    }

    /// A cloud-api stub that records every request it receives, so a test can
    /// assert the token exchange was never *attempted* rather than merely that
    /// it did not succeed.
    async fn counting_cloud_api() -> (CloudApi, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use std::sync::{Arc, atomic::AtomicUsize};
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&hits);
        let app = axum::Router::new().fallback(axum::routing::any(move || {
            let counter = Arc::clone(&counter);
            async move {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                axum::Json(serde_json::json!({}))
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (
            CloudApi::for_test(&format!("http://{address}")).unwrap(),
            hits,
        )
    }

    /// A refused address must not cost the contributor their credential.
    ///
    /// The exchange retires the refresh token that authenticated it, so
    /// spending one against an endpoint this daemon was never going to accept
    /// leaves them holding a dead credential for nothing. The address check
    /// therefore has to come first, and this is what pins that order.
    #[tokio::test]
    async fn a_refused_address_does_not_spend_the_refresh_token() {
        for address in [
            "http://commons.example",
            "https://user@commons.example",
            "https://commons.example/?token=abc",
            "not a url",
        ] {
            let shared = shared_with_session();
            let (api, hits) = counting_cloud_api().await;
            let error = enroll(&shared, &api, address).await.unwrap_err();
            // The assertion that actually pins the ordering: the exchange was
            // never reached. Checking only that the stored token is unchanged
            // passes just as well when the exchange ran and failed.
            assert_eq!(
                hits.load(std::sync::atomic::Ordering::SeqCst),
                0,
                "{address} reached the token exchange"
            );
            assert_eq!(
                label(&error),
                "near_ai_enroll_endpoint_refused",
                "{address}"
            );
            assert_eq!(
                stored_refresh(&shared).as_deref(),
                Some(REFRESH),
                "{address} spent the refresh token"
            );
            assert!(
                shared.store.load_config().unwrap().is_none(),
                "{address} left an enrollment behind"
            );
        }
    }

    /// No login, no enrollment -- and no attempt to invent one.
    #[tokio::test]
    async fn without_a_near_ai_session_enrollment_refuses() {
        let (dir, store) = crate::config::tests_support::temp_store();
        std::mem::forget(dir);
        let shared = DaemonShared::load(store).unwrap();
        let (api, hits) = counting_cloud_api().await;
        let error = enroll(&shared, &api, "https://commons.example")
            .await
            .unwrap_err();
        assert_eq!(label(&error), "near_ai_enroll_no_session");
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(shared.store.load_config().unwrap().is_none());
    }

    fn commons() -> Commons<'static> {
        Commons {
            ingest_url: "https://commons.example",
            issuer_url: "https://issuer.example",
            audience: "trace-commons-upload",
            witness: serde_json::from_value(serde_json::json!({
                "url": "https://witness.example",
                "signing_address": format!("0x{}", "ab".repeat(20)),
                "expected_measurements": [format!("mrtd={}", "ab".repeat(48))],
                "admission_evidence": true,
            }))
            .unwrap(),
            receipt_endpoint: Some("https://receipts.example/v1".into()),
        }
    }

    fn finished() -> Finished {
        Finished {
            access_token: "tcn1_fixture".into(),
            token_type: "Bearer".into(),
            expires_in_secs: 3600,
            account_id: "someone@example.test".into(),
            tenant_id: format!("nearai-{}", "3c".repeat(32)),
            device_key_id: String::new(),
            anchor_hash: format!("sha256:{}", "ab".repeat(32)),
        }
    }

    /// What a shell is handed must carry no bearer secret, and neither must
    /// the config on disk.
    #[test]
    fn the_reply_and_the_config_carry_no_token() {
        let shared = shared_with_session();
        let identity = DeviceIdentity::load_or_generate(&shared.store).unwrap();
        let mut result = finished();
        result.device_key_id = identity.device_key_id.clone();

        let reply = persist(&shared.store, commons(), &identity, result, None).unwrap();
        let rendered = serde_json::to_string(&reply).unwrap();
        for secret in ["tcn1_fixture", REFRESH, "rt_"] {
            assert!(
                !rendered.contains(secret),
                "the IPC reply carries {secret}: {rendered}"
            );
        }
        assert_eq!(reply["enrolled"], true);

        // The fields `submit` mints upload claims with. Empty ones produce a
        // config that is enrolled and cannot upload, which fails at the
        // contributor's first submission rather than here.
        let written = shared.store.load_config().unwrap().unwrap();
        assert_eq!(written.issuer_url, "https://issuer.example");
        assert_eq!(written.audience, "trace-commons-upload");
        assert_eq!(
            written.witness.as_ref().map(|w| w.url.as_str()),
            Some("https://witness.example")
        );
        // K11: the join says where the witness came from.
        assert_eq!(
            written.witness_origin_view(),
            Some(crate::config::WitnessOriginView::Recorded(
                crate::config::WitnessOrigin::PublishedAtJoin
            ))
        );
        assert_eq!(
            written.inference_receipt_endpoint.as_deref(),
            Some("https://receipts.example/v1")
        );

        let config = std::fs::read_to_string(shared.store.dir().join("contributor.json")).unwrap();
        for secret in ["tcn1_fixture", REFRESH, "rt_"] {
            assert!(
                !config.contains(secret),
                "contributor.json carries {secret}"
            );
        }
    }

    /// Every field the server returns is checked, and a rejection publishes
    /// nothing -- a config without its session would be an enrolled client
    /// that cannot authenticate and, because `enroll` refuses once a config
    /// exists, cannot re-enroll either.
    #[test]
    fn a_malformed_result_publishes_no_enrollment() {
        let shared = shared_with_session();
        let identity = DeviceIdentity::load_or_generate(&shared.store).unwrap();
        let good = || {
            let mut r = finished();
            r.device_key_id = identity.device_key_id.clone();
            r
        };
        let mutations: Vec<(&str, Box<dyn Fn(&mut Finished)>)> = vec![
            (
                "token_type",
                Box::new(|r: &mut Finished| r.token_type = "Basic".into()),
            ),
            (
                "access_token prefix",
                Box::new(|r: &mut Finished| r.access_token = "nope".into()),
            ),
            (
                "expiry zero",
                Box::new(|r: &mut Finished| r.expires_in_secs = 0),
            ),
            (
                "expiry too long",
                Box::new(|r: &mut Finished| r.expires_in_secs = 43_201),
            ),
            (
                "another device",
                Box::new(|r: &mut Finished| r.device_key_id = "sha256:other".into()),
            ),
            (
                "anchor prefix",
                Box::new(|r: &mut Finished| r.anchor_hash = "ab".repeat(32)),
            ),
            (
                "wallet tenant",
                Box::new(|r: &mut Finished| r.tenant_id = format!("near-{}", "ab".repeat(32))),
            ),
            (
                "empty account",
                Box::new(|r: &mut Finished| r.account_id = String::new()),
            ),
        ];
        for (name, mutate) in mutations {
            let mut result = good();
            mutate(&mut result);
            assert!(
                persist(&shared.store, commons(), &identity, result, None).is_err(),
                "{name} was accepted"
            );
            assert!(
                shared.store.load_config().unwrap().is_none(),
                "{name} published an enrollment"
            );
        }
        // And the unmutated fixture is accepted, so the cases above fail for
        // their own reason rather than because the fixture never worked.
        assert!(persist(&shared.store, commons(), &identity, good(), None).is_ok());
    }

    #[test]
    fn tenant_ids_are_accepted_on_shape_and_never_derived() {
        assert!(is_near_ai_tenant_id(&format!("nearai-{}", "3c".repeat(32))));
        assert!(is_near_ai_tenant_id(&format!("nearai-{}", "ab".repeat(32))));
        for bad in [
            format!("nearai-{}", "AB".repeat(32)),
            format!("nearai-{}", "ab".repeat(31)),
            format!("nearai-{}", "ab".repeat(33)),
            format!("nearai-{}", "gz".repeat(32)),
            "nearai-".to_string(),
            String::new(),
        ] {
            assert!(!is_near_ai_tenant_id(&bad), "{bad} must be refused");
        }
    }

    /// The two namespaces must not admit each other's tenants. `near-` and
    /// `nearai-` are close enough that a `strip_prefix` written the obvious
    /// way could accept both.
    #[test]
    fn the_two_tenant_namespaces_are_disjoint() {
        let wallet = format!("near-{}", "ab".repeat(32));
        let near_ai = format!("nearai-{}", "ab".repeat(32));
        assert!(is_near_ai_tenant_id(&near_ai));
        assert!(!is_near_ai_tenant_id(&wallet));
        assert!(crate::config::is_near_tenant_id(&wallet));
        assert!(!crate::config::is_near_tenant_id(&near_ai));
    }

    #[test]
    fn only_this_module_s_own_labels_are_forwarded() {
        // A transport error can quote a URL and a remote body can quote
        // anything. Neither is ours to hand to a shell.
        for (raw, expected) in [
            ("near_ai_enroll_no_session", "near_ai_enroll_no_session"),
            (
                "near_ai_enroll_endpoint_refused",
                "near_ai_enroll_endpoint_refused",
            ),
            (
                "error sending request for url (https://commons.example/v1/x): connection refused",
                "near_ai_enroll_unavailable",
            ),
            ("rt_leaked_secret_value", "near_ai_enroll_unavailable"),
        ] {
            assert_eq!(label(&anyhow!(raw.to_string())), expected, "{raw}");
        }
    }
}

/// Connect a retained NEAR AI login to the signed-in passkey account: bind an
/// unbound one, or enrol this Mac into a bound one. The authenticated bind
/// protocol has its own account-bound proof; the unauthenticated provisioning
/// preimage is never substituted for it.
///
/// An enrolment's expected account is the passkey session's, which the
/// commons reads from the session itself: nothing here names it. If this
/// Mac's near.ai login is someone else's, the commons refuses before it
/// writes the device key, and this answers `account-enrol-mismatch` with the
/// passkey session left as it was for the shell to sign out.
pub(super) async fn bind(shared: &DaemonShared) -> Result<serde_json::Value> {
    use super::native_identity::{authenticated, resolve_origin};
    use reqwest::Method;
    use serde_json::json;
    if shared.store.load_config()?.is_some() {
        bail!("account-already-enrolled");
    }
    let mut account = crate::account_auth::try_load_session_with_snapshot(&shared.store)?
        .ok_or_else(|| anyhow!("account-session-required"))?;
    let origin = resolve_origin(&shared.store, &json!({}))?;
    let account_id = uuid::Uuid::parse_str(&account.session.account_id)
        .map_err(|_| anyhow!("account-bind-invalid"))?;
    let binding = authenticated(
        shared,
        &origin,
        &mut account,
        Method::GET,
        "/v1/account/binding",
        None,
    )
    .await?;
    if !binding
        .get("binding_state")
        .and_then(|v| v.as_str())
        .is_some_and(bind_admits)
    {
        bail!("account-bind-refused");
    }
    let prepared = prepare(shared, &origin).await?;
    let api = CloudApi::live().map_err(|_| anyhow!("near_ai_enroll_token_unavailable"))?;
    bind_prepared(shared, &api, &origin, account, account_id, prepared).await
}

/// Whether `account_bind` runs a ceremony for the signed-in account's
/// `binding_state`: `unbound` binds it to this Mac's near.ai login; `bound`
/// (another Mac bound it, and this one signed in with the same passkey)
/// enrols this Mac into it, which the commons allows only when this Mac's
/// near.ai login is the one that account is bound to. Anything else is
/// refused before the refresh token is spent.
fn bind_admits(binding_state: &str) -> bool {
    matches!(binding_state, "unbound" | "bound")
}

async fn bind_prepared(
    shared: &DaemonShared,
    api: &CloudApi,
    origin: &str,
    mut account: crate::account_auth::LoadedAccountSession,
    account_id: uuid::Uuid,
    prepared: Prepared,
) -> Result<serde_json::Value> {
    use super::native_identity::authenticated;
    use reqwest::Method;
    use serde_json::json;
    let identity = DeviceIdentity::load_or_generate_async(&shared.store).await?;
    let verifier = random()?;
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    let started = authenticated(
        shared,
        origin,
        &mut account,
        Method::POST,
        "/v1/account/near-ai/provision/bind/start",
        Some(&start_payload(&challenge, &identity.public_key_b64)),
    )
    .await?;
    let started: Started =
        serde_json::from_value(started).map_err(|_| anyhow!("account-bind-invalid"))?;
    let signature = bind_device_proof(
        &identity,
        &started.ceremony_id,
        &started.nonce,
        &challenge,
        started.expires_at,
        Utc::now().timestamp(),
        &account_id,
    )?;
    // Capabilities, local authority, server ceremony and device proof have all
    // been validated before this one-use refresh credential is spent.
    if super::commons_credentials::snapshot(
        &shared.store,
        super::commons_credentials::Kind::Account,
    )? != account.snapshot
    {
        bail!("account-session-changed");
    }
    let token = super::nearai_credential::exchange(shared, api, &prepared.session)
        .await
        .map_err(|_| anyhow!("near_ai_enroll_token_unavailable"))?;
    let result = authenticated(shared,origin,&mut account,Method::POST,"/v1/account/near-ai/provision/bind/finish",Some(&json!({
        "ceremony_id":started.ceremony_id,"code_verifier":verifier,"device_public_key":identity.public_key_b64,
        "device_signature":signature,"access_token":token.access_token,
    }))).await?;
    let outcome = result
        .get("outcome")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("account-bind-invalid"))?
        .to_string();
    let state = result
        .get("binding_state")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("account-bind-invalid"))?
        .to_string();
    let finished: Finished =
        serde_json::from_value(result).map_err(|_| anyhow!("account-bind-invalid"))?;
    let session_tenant = token_tenant(&account.session.access_token);
    if token_tenant(&finished.access_token).as_deref() != Some(finished.tenant_id.as_str()) {
        bail!("account-bind-invalid");
    }
    let returned_id =
        uuid::Uuid::parse_str(&finished.account_id).map_err(|_| anyhow!("account-bind-invalid"))?;
    // `bound` and `enrolled` keep the passkey session's account, so persist
    // must find the same account AND tenant it was signed in to;
    // `existing_account` is the one outcome that deliberately switches.
    let keeps = match outcome.as_str() {
        "bound" | "enrolled" if state == "bound" && returned_id == account_id => true,
        "existing_account"
            if matches!(state.as_str(), "bound" | "legacy") && returned_id != account_id =>
        {
            false
        }
        _ => bail!("account-bind-invalid"),
    };
    let expected_account = if keeps {
        let tenant = session_tenant.ok_or_else(|| anyhow!("account-bind-invalid"))?;
        Some(ExpectedAccount {
            tenant_id: tenant,
            account_id: account_id.to_string(),
        })
    } else {
        None
    };
    // First enrollment is atomic with the replacement account session. A
    // bound or enrolled result keeps this account; existing_account
    // intentionally switches account and does not claim that the newly
    // created passkey moved.
    persist_for(
        &shared.store,
        Commons {
            ingest_url: origin,
            issuer_url: &prepared.issuer_url,
            audience: &prepared.audience,
            witness: prepared.witness,
            receipt_endpoint: prepared.receipt_endpoint,
        },
        &identity,
        finished,
        Some(&account.snapshot),
        expected_account.as_ref(),
    )?;
    Ok(json!({"outcome":outcome,"binding_state":state}))
}

/// The tenant a native `tcn1_` token names, as the server encodes it.
fn token_tenant(token: &str) -> Option<String> {
    token
        .strip_prefix("tcn1_")
        .and_then(|s| s.split_once('.'))
        .and_then(|(s, _)| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(s)
                .ok()
        })
        .and_then(|s| String::from_utf8(s).ok())
}

pub(super) fn bind_device_proof(
    identity: &DeviceIdentity,
    ceremony_id: &str,
    nonce_wire: &str,
    challenge: &str,
    expires_at: i64,
    now: i64,
    account_id: &uuid::Uuid,
) -> Result<String> {
    // Reuse provisioning's freshness and encoding validation, but discard its
    // signature: the signed bytes below are the distinct bind contract.
    device_proof_for_ceremony(
        identity,
        ceremony_id,
        nonce_wire,
        challenge,
        expires_at,
        now,
    )?;
    let nonce: [u8; 32] = base64::engine::general_purpose::STANDARD
        .decode(nonce_wire)?
        .try_into()
        .map_err(|_| anyhow!("account-bind-invalid"))?;
    let device: [u8; 32] = base64::engine::general_purpose::STANDARD
        .decode(&identity.public_key_b64)?
        .try_into()
        .map_err(|_| anyhow!("account-bind-invalid"))?;
    Ok(identity.sign_b64(
        &trace_commons_protocol::onboarding::near_ai_bind_device_bytes(
            &nonce,
            ceremony_id,
            &device,
            challenge,
            expires_at,
            account_id,
        ),
    ))
}

#[cfg(test)]
mod native_bind_tests {
    use super::*;
    use serde_json::{Value, json};
    use std::sync::{Arc, Mutex};

    /// An unbound account binds and a bound one enrols this Mac (a second
    /// Mac signed in with the same passkey); a legacy, closed or unknown
    /// state runs no ceremony and spends nothing.
    #[test]
    fn only_unbound_and_bound_accounts_run_a_ceremony() {
        assert!(bind_admits("unbound"));
        assert!(bind_admits("bound"));
        for refused in ["legacy", "closed", "", "Bound", "unknown"] {
            assert!(!bind_admits(refused), "{refused}");
        }
    }

    #[tokio::test]
    async fn bind_http_preserves_account_or_switches_only_on_existing_account_without_consent() {
        for outcome in [
            "bound",
            "existing_account",
            "stale",
            "wrong-account",
            "wrong-device",
            "wrong-tenant",
            // A second Mac joining its passkey's bound account.
            "enrolled",
            "enrolled-wrong-account",
            "enrolled-other-tenant",
            "bound-other-tenant",
            "mismatch",
        ] {
            let (_dir, store) = crate::config::tests_support::temp_store();
            let shared = DaemonShared::load(store).unwrap();
            let near = super::super::settings::NearAiSession {
                refresh_token: "rt_original".into(),
                refresh_token_expires_at: None,
                stored_at: Utc::now(),
                user_agent: "test-agent".into(),
            };
            {
                let mut settings = shared.settings.lock().unwrap();
                settings.near_ai_session = Some(near.clone());
                settings.save_for_test(&shared.store).unwrap();
            }
            let account_id = uuid::Uuid::new_v4();
            // The passkey session's tenant rides in its token, as the server
            // issues it.
            let session_tenant = format!("nearai-{}", "ab".repeat(32));
            let initial = json!({"access_token":format!("tcn1_{}.original",base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&session_tenant)),"expires_at":Utc::now()+chrono::Duration::hours(6),"account_id":account_id.to_string()});
            let snapshot = super::super::commons_credentials::snapshot(
                &shared.store,
                super::super::commons_credentials::Kind::Account,
            )
            .unwrap();
            super::super::commons_credentials::replace(
                &shared.store,
                &snapshot,
                &serde_json::to_vec(&initial).unwrap(),
                None,
            )
            .unwrap();
            let account = crate::account_auth::try_load_session_with_snapshot(&shared.store)
                .unwrap()
                .unwrap();
            let identity = DeviceIdentity::load_or_generate(&shared.store).unwrap();
            let device_id = identity.device_key_id.clone();
            let public = base64::engine::general_purpose::STANDARD
                .decode(&identity.public_key_b64)
                .unwrap();
            let challenge = Arc::new(Mutex::new(String::new()));
            let saved_challenge = Arc::clone(&challenge);
            let nonce = [9u8; 32];
            let expires = Utc::now().timestamp() + 300;
            let returned_id = if matches!(
                outcome,
                "existing_account" | "wrong-account" | "enrolled-wrong-account"
            ) {
                uuid::Uuid::new_v4()
            } else {
                account_id
            };
            let finished_id = returned_id.to_string();
            let store = shared.store.clone();
            let app = axum::Router::new()
                .route("/v1/account/near-ai/provision/bind/start",axum::routing::post(move |axum::Json(value):axum::Json<Value>| {
                    *saved_challenge.lock().unwrap()=value["code_challenge"].as_str().unwrap().into();
                    async move { axum::Json(json!({"ceremony_id":"bind-ceremony","nonce":base64::engine::general_purpose::STANDARD.encode(nonce),"expires_at":expires})) }
                }))
                .route("/v1/account/near-ai/provision/bind/finish",axum::routing::post(move |axum::Json(value):axum::Json<Value>| {
                    let challenge=challenge.lock().unwrap().clone();
                    let public=public.clone(); let device_id=device_id.clone(); let finished_id=finished_id.clone(); let store=store.clone();
                    async move {
                        let device:[u8;32]=public.clone().try_into().unwrap();
                        let proof=trace_commons_protocol::onboarding::near_ai_bind_device_bytes(&nonce,"bind-ceremony",&device,&challenge,expires,&account_id);
                        let signature=base64::engine::general_purpose::STANDARD.decode(value["device_signature"].as_str().unwrap()).unwrap();
                        ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519,public).verify(&proof,&signature).unwrap();
                        assert_eq!(value["access_token"],"near-access");
                        if outcome=="stale" {super::super::commons_credentials::clear(&store,&[super::super::commons_credentials::Kind::Account]).unwrap();}
                        if outcome=="mismatch" {
                            return (axum::http::StatusCode::CONFLICT, axum::Json(json!({"error":"near_ai_account_mismatch"})));
                        }
                        // A well-formed tenant, consistent with its own token,
                        // that is not the passkey session's.
                        let tenant=if matches!(outcome,"existing_account"|"enrolled-other-tenant"|"bound-other-tenant") {format!("nearai-{}","cd".repeat(32))} else {format!("nearai-{}","ab".repeat(32))};
                        let reported=if outcome.starts_with("enrolled") {"enrolled"} else if outcome=="existing_account" {"existing_account"} else {"bound"};
                        (axum::http::StatusCode::OK, axum::Json(json!({"access_token":format!("tcn1_{}.secret",base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(if outcome=="wrong-tenant" {"different-tenant"} else {&tenant})),"token_type":"Bearer","expires_in_secs":43200,"account_id":finished_id,"tenant_id":tenant,"device_key_id":if outcome=="wrong-device" {"wrong"} else {&device_id},"anchor_hash":format!("sha256:{}","ab".repeat(32)),"outcome":reported,"binding_state":if outcome=="existing_account" {"legacy"} else {"bound"}})))
                    }
                }))
                .route("/v1/users/me/access-tokens",axum::routing::post(||async {axum::Json(json!({"access_token":"near-access","refresh_token":"rt_rotated","refresh_token_expiration":Utc::now()+chrono::Duration::days(1)}))}));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let origin = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let prepared=Prepared {commons:super::super::native_identity::client(&origin,"unauthenticated").unwrap(),session:near,issuer_url:"https://issuer.example".into(),audience:"trace-commons-upload".into(),witness:serde_json::from_value(json!({"url":"https://witness.example","signing_address":format!("0x{}","ab".repeat(20)),"expected_measurements":[format!("mrtd={}","ab".repeat(48))],"admission_evidence":true})).unwrap(),receipt_endpoint:None};
            let result = bind_prepared(
                &shared,
                &CloudApi::for_test(&origin).unwrap(),
                &origin,
                account,
                account_id,
                prepared,
            )
            .await;
            server.abort();
            if matches!(
                outcome,
                "stale"
                    | "wrong-account"
                    | "wrong-device"
                    | "wrong-tenant"
                    | "enrolled-wrong-account"
                    | "enrolled-other-tenant"
                    | "bound-other-tenant"
                    | "mismatch"
            ) {
                let error = result.expect_err(outcome).to_string();
                assert!(shared.store.load_config().unwrap().is_none(), "{outcome}");
                if outcome == "mismatch" {
                    // The server's refusal, by its own daemon label, so the
                    // shell can say what happened; and the passkey session is
                    // left exactly as it was for the shell to sign out.
                    assert_eq!(error, "account-enrol-mismatch");
                    assert_eq!(
                        crate::account_auth::try_load_session_with_snapshot(&shared.store)
                            .unwrap()
                            .unwrap()
                            .session
                            .account_id,
                        account_id.to_string()
                    );
                }
            } else {
                let result = result.unwrap();
                assert_eq!(result["outcome"], outcome);
                assert_eq!(
                    result["binding_state"],
                    if outcome == "existing_account" {
                        "legacy"
                    } else {
                        "bound"
                    }
                );
                assert!(!result.to_string().contains("secret"));
                let cfg = shared.store.load_config().unwrap().unwrap();
                assert!(cfg.consent_scopes.is_empty());
                assert!(!cfg.consent_scopes_chosen);
                assert_eq!(
                    crate::account_auth::try_load_session_with_snapshot(&shared.store)
                        .unwrap()
                        .unwrap()
                        .session
                        .account_id,
                    returned_id.to_string()
                );
            }
            assert_eq!(
                shared
                    .settings
                    .lock()
                    .unwrap()
                    .near_ai_session
                    .as_ref()
                    .unwrap()
                    .refresh_token,
                "rt_rotated"
            );
        }
    }
}
