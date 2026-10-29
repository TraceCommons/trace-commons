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

/// The provisioning ceremony's routes, chosen by the build rather than by any
/// setting (#1574 review, finding H1).
///
/// A production build keeps the routes every deployed client calls, whose
/// readiness requires the commons's published witness. A `near-ai-staging`
/// build calls the v2 routes, which a staging ingest opens without the
/// admission ledger or the witness (staging spec S12); the old ones refuse
/// there, so a staging client on them could never sign up.
#[cfg(not(feature = "near-ai-staging"))]
pub const PROVISION_START_PATH: &str = "/v1/account/near-ai/provision/start";
#[cfg(not(feature = "near-ai-staging"))]
pub const PROVISION_FINISH_PATH: &str = "/v1/account/near-ai/provision/finish";
#[cfg(feature = "near-ai-staging")]
pub const PROVISION_START_PATH: &str = super::account_onboarding::NEAR_AI_START_V2_PATH;
#[cfg(feature = "near-ai-staging")]
pub const PROVISION_FINISH_PATH: &str = super::account_onboarding::NEAR_AI_FINISH_V2_PATH;

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
    /// Whether the referral code the finish carried was applied (referral
    /// spec section 3). Present only when one was sent, and never a reason
    /// the enrollment failed.
    #[serde(default)]
    referral: Option<trace_commons_protocol::referral::ReferralOutcome>,
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
///
/// `ingest_url` may be left out: the first run's "Sign in with near.ai"
/// without an invite names no commons, and enrolls against the native
/// account's (`enroll_origin`).
pub(super) async fn handle_enroll(shared: &DaemonShared, req: &Request) -> Response {
    let Ok(ingest_url) = enroll_origin(&shared.store, &req.params) else {
        return Response::err(req.id, ERR_BAD_PARAMS, "near_ai_enroll_invalid");
    };
    let Ok(referral) = SignupReferral::from_params(&req.params) else {
        return Response::err(req.id, ERR_BAD_PARAMS, "referral_code");
    };
    let ingest_url = ingest_url.as_str();
    let api = match CloudApi::live() {
        Ok(api) => api,
        Err(_) => {
            return Response::err(req.id, ERR_UNAVAILABLE, "near_ai_enroll_token_unavailable");
        }
    };
    match enroll(shared, &api, ingest_url, referral).await {
        Ok(value) => Response::ok(req.id, value),
        // Label only. Every failure below already carries a control name, and
        // the errors underneath them can quote a remote body or a URL.
        Err(error) => Response::err(req.id, ERR_UNAVAILABLE, label(&error)),
    }
}

/// The optional referral code a NEAR AI sign-up carries (referral spec
/// section 3, "At sign-up"). It never fails the enrollment: a code is applied
/// by the commons after the account is provisioned, and the answer says
/// whether it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SignupReferral {
    /// No code, or an empty field: the finish carries none and the answer
    /// has no `referral`.
    None,
    /// A code in canonical form, sent with the finish.
    Send(String),
    /// Text that is not a code. Never sent; the answer says `not_applied`.
    NotACode,
}

impl SignupReferral {
    /// `referral_code` from the request. A value that is not a string (or
    /// null) is refused, as any other malformed parameter is.
    pub(super) fn from_params(params: &serde_json::Value) -> Result<Self, ()> {
        match params.get("referral_code") {
            None | Some(serde_json::Value::Null) => Ok(Self::None),
            Some(serde_json::Value::String(typed)) if typed.trim().is_empty() => Ok(Self::None),
            Some(serde_json::Value::String(typed)) => Ok(
                match trace_commons_protocol::referral::normalize_referral_code(typed) {
                    Some(code) => Self::Send(code),
                    None => Self::NotACode,
                },
            ),
            Some(_) => Err(()),
        }
    }

    fn code(&self) -> Option<String> {
        match self {
            Self::Send(code) => Some(code.clone()),
            Self::None | Self::NotACode => None,
        }
    }

    /// The answer's `referral`, from what the commons said about the code
    /// sent. A commons that said nothing about a code it was sent applied
    /// none.
    fn outcome(
        &self,
        answered: Option<trace_commons_protocol::referral::ReferralOutcome>,
    ) -> Option<trace_commons_protocol::referral::ReferralOutcome> {
        use trace_commons_protocol::referral::ReferralOutcome;
        match self {
            Self::None => None,
            Self::NotACode => Some(ReferralOutcome::NotApplied),
            Self::Send(_) => Some(answered.unwrap_or(ReferralOutcome::NotApplied)),
        }
    }
}

/// The commons an enrollment is for: the one named, or, when none is, the
/// origin `native_identity` trusts for this store -- the native account's
/// commons on a daemon that holds no config. A non-string `ingest_url` is
/// refused rather than read as absent.
pub(super) fn enroll_origin(store: &ConfigStore, params: &serde_json::Value) -> Result<String> {
    match params.get("ingest_url") {
        Some(value) => value
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| anyhow!("near_ai_enroll_invalid")),
        None => super::native_identity::resolve_origin(store, &serde_json::json!({})),
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
        // Enrollment's shells already say "sign in to NEAR AI again" for
        // this one; the finer class is the renewal's (`account_session_renewal`).
        "near_ai_enroll_near_ai_sign_in_required" => "near_ai_enroll_token_unavailable",
        "near_ai_enroll_start_failed" => "near_ai_enroll_start_failed",
        "near_ai_enroll_commons_unreachable" => "near_ai_enroll_commons_unreachable",
        "near_ai_enroll_commons_unsupported" => "near_ai_enroll_commons_unsupported",
        "near_ai_enroll_invalid" => "near_ai_enroll_invalid",
        "near_ai_enroll_verification_failed" => "near_ai_enroll_verification_failed",
        ENROLL_DEVICE_KEY_REVOKED => ENROLL_DEVICE_KEY_REVOKED,
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
    referral: SignupReferral,
) -> Result<serde_json::Value> {
    let expected = super::commons_credentials::snapshot(
        &shared.store,
        super::commons_credentials::Kind::Account,
    )?;
    if shared.store.load_config()?.is_some() {
        bail!("near_ai_enroll_already_enrolled")
    }
    let prepared = prepare(shared, ingest_url)
        .await?
        .with_referral_code(referral.code());
    let enrolled = enroll_prepared(shared, api, ingest_url, prepared, expected).await?;
    Ok(enrolled.answer(&referral))
}

/// A published enrollment: the answer for the shell, and what the commons
/// said about the referral code the finish carried.
struct Enrolled {
    value: serde_json::Value,
    referral: Option<trace_commons_protocol::referral::ReferralOutcome>,
}

impl Enrolled {
    /// The shell's answer, with `referral` when the request carried a code.
    fn answer(mut self, referral: &SignupReferral) -> serde_json::Value {
        if let Some(outcome) = referral.outcome(self.referral) {
            self.value["referral"] = serde_json::json!(outcome.as_str());
        }
        self.value
    }
}

/// [`enroll`] from a prepared ceremony on: the device key, the ceremony and
/// the published enrollment. Separate so a test can prepare against a
/// loopback commons (see [`prepare_on`]) and still run every later step as
/// `enroll` runs it.
async fn enroll_prepared(
    shared: &DaemonShared,
    api: &CloudApi,
    ingest_url: &str,
    prepared: Prepared,
    expected: super::commons_credentials::Snapshot,
) -> Result<Enrolled> {
    let identity = DeviceIdentity::load_or_generate_async(&shared.store).await?;
    let provisioned = provision(shared, api, prepared, &identity).await?;
    let referral = provisioned.finished.referral;

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
    .map(|value| Enrolled { value, referral })
}

/// Everything checked before a device key is chosen or the refresh token is
/// spent: the address, the retained NEAR AI login, and the commons's own
/// facts. Shared by enrollment and by the legacy invite migration, which
/// provisions a staged key against the commons the daemon is already
/// enrolled in.
pub(super) struct Prepared {
    commons: trace_commons_operator_client::Client,
    session: super::settings::NearAiSession,
    /// Which sign-in `session` came from, read under the same lock. See
    /// [`exchange_following_rotation`].
    login: Option<Login>,
    issuer_url: String,
    audience: String,
    /// `None` only in a `near-ai-staging` build: see
    /// [`super::account_onboarding::LoginCapability::witness`].
    witness: Option<crate::config::WitnessSettings>,
    receipt_endpoint: Option<String>,
    /// A canonical referral code for the finish to carry: a first
    /// enrollment's only ([`Prepared::with_referral_code`]). A renewal or a
    /// migration carries none.
    referral_code: Option<String>,
}

impl Prepared {
    pub(super) fn with_referral_code(mut self, referral_code: Option<String>) -> Self {
        self.referral_code = referral_code;
        self
    }
}

/// A finished ceremony and the commons facts it ran against. Not yet
/// validated: `persist` and `provisioned_account` each check `finished`.
pub(super) struct Provisioned {
    issuer_url: String,
    audience: String,
    witness: Option<crate::config::WitnessSettings>,
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
    prepare_on(shared, commons, ingest_url).await
}

/// [`prepare`] after the address check, over the origin-only client
/// `prepare` built for `ingest_url`. Separate so a test can hand it a client
/// for a loopback commons, which `prepare`'s own address check refuses.
async fn prepare_on(
    shared: &DaemonShared,
    commons: trace_commons_operator_client::Client,
    ingest_url: &str,
) -> Result<Prepared> {
    let (session, login) = {
        let settings = shared.settings.lock().expect("settings lock");
        (
            settings
                .near_ai_session
                .clone()
                .ok_or_else(|| anyhow!("near_ai_enroll_no_session"))?,
            Login::of(&settings),
        )
    };

    // Fetched after the session presence check above, which is a local read
    // that spends nothing: a contributor who never logged in should be told
    // that, not that the commons is unreachable.
    //
    // The commons's own facts. A production build reads them from the same
    // route and the same validator the wallet ceremony uses; a staging build
    // from the v2 route, which publishes no witness. `issuer_url` and `audience` are what `submit`
    // mints upload claims with, so an enrollment without them is enrolled and
    // unable to upload -- a config that fails at the contributor's first
    // submission rather than here, where they could still act on it.
    //
    // This also extends the derived allowlist with the issuer, witness and
    // receipt hosts this origin publishes, which nothing on this path would
    // otherwise ask for. Before the session is read and well before the token
    // is spent: a commons that is unreachable or not offering enrollment must
    // not cost the contributor their refresh token either.
    //
    // See `account_onboarding::login_capability_from` for the two routes.
    let capability = super::account_onboarding::login_capability_from(&commons, ingest_url)
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
        login,
        issuer_url: capability.issuer_url,
        audience: capability.audience,
        witness: capability.witness,
        receipt_endpoint: capability.receipt_endpoint,
        referral_code: None,
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
        login,
        issuer_url,
        audience,
        witness,
        receipt_endpoint,
        referral_code,
    } = prepared;
    let finished = ceremony(
        shared,
        api,
        &commons,
        &session,
        login.as_ref(),
        identity,
        referral_code.as_deref(),
    )
    .await?;
    Ok(Provisioned {
        issuer_url,
        audience,
        witness,
        receipt_endpoint,
        finished,
    })
}

/// The ceremony itself, against `commons`: start, device proof, token
/// exchange, finish. Shared by [`provision`] and by
/// [`renew_account_session`], which runs it again for a device already
/// enrolled. Not validated: each caller checks the answer.
async fn ceremony(
    shared: &DaemonShared,
    api: &CloudApi,
    commons: &trace_commons_operator_client::Client,
    session: &super::settings::NearAiSession,
    login: Option<&Login>,
    identity: &DeviceIdentity,
    referral_code: Option<&str>,
) -> Result<Finished> {
    let verifier = random()?;
    let code_challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));

    let started: Started = commons
        .call_json(
            reqwest::Method::POST,
            PROVISION_START_PATH,
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
    //
    // NEAR AI refusing the login itself (a spent, revoked or agent-mismatched
    // refresh token, or none stored) is told apart from NEAR AI being
    // unreachable: no retry fixes the first, and the person has to sign in
    // to NEAR AI again. Before this, both were `token_unavailable`, and a
    // renewal that stopped here left only a start in the commons's log.
    let access_token = exchange_following_rotation(shared, api, session, login)
        .await
        .map_err(|error| match error.to_string().as_str() {
            "near_ai_credential_session_expired" | "near_ai_credential_session_missing" => {
                anyhow!("near_ai_enroll_near_ai_sign_in_required")
            }
            _ => anyhow!("near_ai_enroll_token_unavailable"),
        })?;

    // The JWT rides in the body, never a header: we are not
    // authenticating to the commons with it, we are handing it over to
    // be introspected.
    let body = serde_json::json!({
        "ceremony_id": started.ceremony_id,
        "code_verifier": verifier,
        "device_public_key": identity.public_key_b64,
        "device_signature": device_signature,
        "access_token": access_token.access_token,
    });
    let finish = |body: serde_json::Value| async move {
        commons
            .call_json::<serde_json::Value, Finished>(
                reqwest::Method::POST,
                PROVISION_FINISH_PATH,
                &[],
                Some(&body),
            )
            .await
    };
    let Some(code) = referral_code else {
        return finish(body)
            .await
            .map_err(|error| finish_refusal(&error, "near_ai_enroll_verification_failed"));
    };
    // Only when there is one: a finish without a code is the body every
    // shipped commons parses.
    let mut with_code = body.clone();
    with_code["referral_code"] = serde_json::Value::String(code.to_owned());
    match finish(with_code).await {
        // A commons from before referral codes parses the finish with
        // `deny_unknown_fields` and answers the field with its uniform deny,
        // before it takes the ceremony or spends the token. Sign-in never
        // fails because of the code, so the finish goes once more without
        // it, on the same ceremony and token; the answer then carries no
        // `referral` and the code reads as not applied. A real refusal took
        // the ceremony, so the retry is refused the same way.
        Err(error) if is_uniform_finish_deny(&error) => finish(body).await,
        other => other,
    }
    .map_err(|error| finish_refusal(&error, "near_ai_enroll_verification_failed"))
}

/// The label of the 400 an ingest answers for any refused native finish,
/// one whose body it could not parse included (`native_generic_deny`).
const UNIFORM_FINISH_DENY: &str = "native sign-in could not be completed";

/// That uniform deny, and nothing else: not a revoked key, not a 5xx, not
/// a transport failure.
fn is_uniform_finish_deny(error: &trace_commons_operator_client::Error) -> bool {
    matches!(
        error,
        trace_commons_operator_client::Error::ServerLabel { status, label, .. }
            if *status == reqwest::StatusCode::BAD_REQUEST && label == UNIFORM_FINISH_DENY
    )
}

/// The commons's label when the device's key was revoked by its account
/// (`DELETE /v1/account/devices/{id}`).
pub(super) const DEVICE_KEY_REVOKED_WIRE: &str = "device_key_revoked";

/// This module's label for [`DEVICE_KEY_REVOKED_WIRE`]: the key will never
/// sign in again, and the device recovers by signing out, which drops the
/// key, then signing in, which makes a new one.
pub const ENROLL_DEVICE_KEY_REVOKED: &str = "near_ai_enroll_device_key_revoked";

/// A finish refusal as this module's label: the revoked-key refusal by
/// name, anything else as `fallback`. Never the remote body.
pub(super) fn finish_refusal(
    error: &trace_commons_operator_client::Error,
    fallback: &'static str,
) -> anyhow::Error {
    if error.server_label() == Some(DEVICE_KEY_REVOKED_WIRE) {
        anyhow!(ENROLL_DEVICE_KEY_REVOKED)
    } else {
        anyhow!(fallback)
    }
}

/// Which sign-in a retained session belongs to: the inference key that
/// sign-in minted beside it. A rotation replaces the session and keeps the
/// key; a new sign-in replaces both. Ids and a timestamp only -- never the key.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct Login {
    key_id: String,
    minted_at: chrono::DateTime<Utc>,
}

impl Login {
    fn of(settings: &super::settings::DaemonSettings) -> Option<Self> {
        settings.near_ai_inference.as_ref().map(|key| Self {
            key_id: key.key_id.clone(),
            minted_at: key.minted_at,
        })
    }
}

/// The token exchange, following one rotation of the same login.
///
/// A session snapshot can go stale between `prepare` and the exchange. The
/// shell reads the balance the moment a sign-in lands, and that read
/// exchanges -- rotating the refresh token -- while the first run is already
/// enrolling. `exchange` then refuses the snapshot with
/// `near_ai_credential_session_changed`, **before sending it anywhere**: the
/// retired token is never presented. Enrollment used to end there, which is
/// the "start answered, finish never sent, a second click worked" failure.
///
/// So on that one refusal, the current session is read and exchanged once,
/// but only when it is the same login rotated: an inference key on both
/// sides, and the same one. A session from a different sign-in, a session
/// gone, or a login with no key to compare is refused as before -- a ceremony
/// prepared for one login never spends another's token. Every other error,
/// and a second refusal, is returned as is: this is bounded at one retry.
///
/// Rotation-safe by construction: the retry presents the token the rotation
/// stored, and the rotation the retry produces is persisted inside
/// `exchange` before its JWT is used, exactly as for the first attempt.
async fn exchange_following_rotation(
    shared: &DaemonShared,
    api: &CloudApi,
    session: &super::settings::NearAiSession,
    login: Option<&Login>,
) -> Result<super::nearai_credential::session::SessionAccess> {
    match super::nearai_credential::exchange(shared, api, session).await {
        Err(error) if error.to_string() == "near_ai_credential_session_changed" => {
            let current = {
                let settings = shared
                    .settings
                    .lock()
                    .map_err(|_| anyhow!("near_ai_credential_unavailable"))?;
                let same_login = login.is_some() && Login::of(&settings).as_ref() == login;
                same_login
                    .then(|| settings.near_ai_session.clone())
                    .flatten()
                    .filter(|current| current.user_agent == session.user_agent)
            };
            let Some(current) = current else {
                return Err(error);
            };
            super::nearai_credential::exchange(shared, api, &current).await
        }
        other => other,
    }
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

/// Mint a fresh account session for a device this commons already enrolled
/// through a NEAR AI login, and store it in place of `expected`'s expired one
/// -- or, with no `previous` record (Sign in again after a sign-out), as the
/// device's account session.
///
/// # Why this exists
///
/// Provisioning hands back a native account session that lives
/// `NATIVE_SESSION_TTL_HOURS` (12 hours) and is never extended: the server
/// rotates its secret on use but keeps `expires_at`. A passkey account signs
/// in again through the browser; a NEAR AI-provisioned account has no other
/// sign-in, and `enroll` refuses once a config exists. Without this, every
/// `/v1/account/*` call (payout rails, withdrawal, credit) on such an account
/// answers `account-session-required` from the thirteenth hour on, for good.
///
/// # What authorizes it
///
/// The same proof the first sign-in gave, and nothing weaker: the retained
/// NEAR AI login, exchanged for a JWT the commons introspects itself, and
/// this device's signature over the ceremony. The commons resolves the
/// login's anchor to the account it already holds and, for the same device,
/// issues a new session (`provision_near_ai_login` is idempotent for the
/// same device and login). No account id is sent; the answer must name the
/// enrolled tenant, the stored session's account and this device, or
/// nothing is written.
///
/// # What it writes
///
/// Only the session, through the Commons credential lifecycle against
/// `expected`: a sign-out or an enrollment change since `expected` was read
/// wins, and the config is never rewritten. Fields beyond the session (the
/// record's origin and binding metadata) are kept.
pub(super) async fn renew_account_session(
    shared: &DaemonShared,
    api: &CloudApi,
    commons: &trace_commons_operator_client::Client,
    cfg: &ContributorConfig,
    identity: &DeviceIdentity,
    previous: Option<serde_json::Value>,
    expected: super::commons_credentials::Snapshot,
) -> Result<()> {
    // The stored record's account, which the answer must name. Without a
    // record, the enrolled tenant is the binding: the commons resolves the
    // NEAR AI login to its account, and an answer for another tenant is
    // refused below.
    let account_id = previous_account_id(previous.as_ref())?;
    if !is_near_ai_tenant_id(&cfg.tenant_id) || identity.device_key_id != cfg.device_key_id {
        bail!("near_ai_enroll_invalid")
    }
    let (session, login) = {
        let settings = shared
            .settings
            .lock()
            .map_err(|_| anyhow!("near_ai_enroll_unavailable"))?;
        (
            settings
                .near_ai_session
                .clone()
                .ok_or_else(|| anyhow!("near_ai_enroll_no_session"))?,
            Login::of(&settings),
        )
    };
    let finished = ceremony(
        shared,
        api,
        commons,
        &session,
        login.as_ref(),
        identity,
        None,
    )
    .await?;
    store_renewed_session(
        shared, cfg, identity, previous, account_id, finished, expected,
    )
    .await
}

/// The stored record's account id, which a renewal's answer must name.
fn previous_account_id(previous: Option<&serde_json::Value>) -> Result<Option<String>> {
    match previous {
        Some(previous) => Ok(Some(
            previous
                .get("account_id")
                .and_then(serde_json::Value::as_str)
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| anyhow!("near_ai_enroll_invalid"))?,
        )),
        None => Ok(None),
    }
}

/// Check a renewal's answer against the enrolled tenant, the stored
/// record's account and this device, then store its session in place of
/// `expected`'s, keeping the record's other fields. Shared by the NEAR AI
/// renewal and the device-key renewal of a device that joined by code.
async fn store_renewed_session(
    shared: &DaemonShared,
    cfg: &ContributorConfig,
    identity: &DeviceIdentity,
    previous: Option<serde_json::Value>,
    account_id: Option<String>,
    finished: Finished,
    expected: super::commons_credentials::Snapshot,
) -> Result<()> {
    validate_finished(&finished, identity)?;
    if finished.tenant_id != cfg.tenant_id
        || account_id
            .as_ref()
            .is_some_and(|account_id| finished.account_id != *account_id)
    {
        bail!("account-enrollment-mismatch")
    }
    let account_id = account_id.unwrap_or_else(|| finished.account_id.clone());
    let mut payload = match previous {
        Some(serde_json::Value::Object(map)) => serde_json::Value::Object(map),
        None => serde_json::json!({}),
        Some(_) => bail!("near_ai_enroll_invalid"),
    };
    payload["access_token"] = serde_json::Value::String(finished.access_token);
    payload["expires_at"] =
        serde_json::to_value(Utc::now() + chrono::Duration::seconds(finished.expires_in_secs))?;
    payload["account_id"] = serde_json::Value::String(account_id);
    let body = serde_json::to_vec(&payload)?;
    let store = shared.store.clone();
    tokio::task::spawn_blocking(move || {
        super::commons_credentials::replace(&store, &expected, &body, None)
    })
    .await
    .map_err(|_| anyhow!("commons_credential_worker_unavailable"))?
}

impl From<trace_commons_protocol::account_device_code::DeviceSignInResponse> for Finished {
    fn from(response: trace_commons_protocol::account_device_code::DeviceSignInResponse) -> Self {
        Self {
            access_token: response.access_token,
            token_type: response.token_type,
            expires_in_secs: response.expires_in_secs,
            account_id: response.account_id,
            tenant_id: response.tenant_id,
            device_key_id: response.device_key_id,
            anchor_hash: response.anchor_hash,
            referral: None,
        }
    }
}

/// The commons facts a device-code sign-in enrolls against: everything
/// [`prepare`] reads except the NEAR AI login, which a device that joins by
/// code does not have. Checked the same way, before the code is spent.
pub(super) struct CodeCommons {
    pub(super) commons: trace_commons_operator_client::Client,
    issuer_url: String,
    audience: String,
    witness: Option<crate::config::WitnessSettings>,
    receipt_endpoint: Option<String>,
}

pub(super) async fn prepare_for_code(ingest_url: &str) -> Result<CodeCommons> {
    let allowed = signup_allowlist(ingest_url, &[])
        .map_err(|_| anyhow!("near_ai_enroll_endpoint_refused"))?;
    let commons =
        client(ingest_url, &allowed).map_err(|_| anyhow!("near_ai_enroll_endpoint_refused"))?;
    let capability = super::account_onboarding::login_capability_from(&commons, ingest_url)
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
    Ok(CodeCommons {
        commons,
        issuer_url: capability.issuer_url,
        audience: capability.audience,
        witness: capability.witness,
        receipt_endpoint: capability.receipt_endpoint,
    })
}

/// Publish a device-code sign-in as an enrollment, exactly as a NEAR AI
/// provisioning is published ([`persist`]).
pub(super) fn persist_code_sign_in(
    store: &ConfigStore,
    ingest_url: &str,
    commons: CodeCommons,
    identity: &DeviceIdentity,
    response: trace_commons_protocol::account_device_code::DeviceSignInResponse,
    expected: &super::commons_credentials::Snapshot,
) -> Result<serde_json::Value> {
    persist(
        store,
        Commons {
            ingest_url,
            issuer_url: &commons.issuer_url,
            audience: &commons.audience,
            witness: commons.witness,
            receipt_endpoint: commons.receipt_endpoint,
        },
        identity,
        response.into(),
        Some(expected),
    )
}

/// Renew the account session of a device that joined by code: it has no
/// NEAR AI login, so it signs a fresh nonce with its device key (owner
/// decision 2026-10-27). The commons answers only for a device that joined
/// by code and is not revoked; the answer is checked and stored exactly as
/// [`renew_account_session`]'s is.
pub(super) async fn renew_account_session_by_device_key(
    shared: &DaemonShared,
    commons: &trace_commons_operator_client::Client,
    cfg: &ContributorConfig,
    identity: &DeviceIdentity,
    previous: Option<serde_json::Value>,
    expected: super::commons_credentials::Snapshot,
) -> Result<()> {
    use trace_commons_protocol::account_device_code::{
        DEVICE_SESSION_RENEW_PATH, DeviceSessionRenewRequest, DeviceSignInResponse,
        device_session_renew_bytes,
    };
    let account_id = previous_account_id(previous.as_ref())?;
    if !is_near_ai_tenant_id(&cfg.tenant_id) || identity.device_key_id != cfg.device_key_id {
        bail!("near_ai_enroll_invalid")
    }
    let device: [u8; 32] = base64::engine::general_purpose::STANDARD
        .decode(&identity.public_key_b64)?
        .try_into()
        .map_err(|_| anyhow!("near_ai_enroll_invalid"))?;
    let mut nonce = [0u8; 32];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut nonce)
        .map_err(|_| anyhow!("near_ai_enroll_unavailable"))?;
    let issued_at = Utc::now().timestamp();
    let request = DeviceSessionRenewRequest {
        tenant_id: cfg.tenant_id.clone(),
        device_public_key: identity.public_key_b64.clone(),
        nonce: base64::engine::general_purpose::STANDARD.encode(nonce),
        issued_at,
        signature: identity.sign_b64(&device_session_renew_bytes(
            &cfg.tenant_id,
            &device,
            &nonce,
            issued_at,
        )),
    };
    let response: DeviceSignInResponse = commons
        .call_json(
            reqwest::Method::POST,
            DEVICE_SESSION_RENEW_PATH,
            &[],
            Some(&request),
        )
        .await
        .map_err(|_| anyhow!("device_session_renewal_refused"))?;
    store_renewed_session(
        shared,
        cfg,
        identity,
        previous,
        account_id,
        response.into(),
        expected,
    )
    .await
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
    witness: Option<crate::config::WitnessSettings>,
    receipt_endpoint: Option<String>,
}

/// The account a ceremony must have answered with, when the caller is signed
/// in to one and the outcome keeps it (bind's `bound`, enroll's `enrolled`).
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
/// enroll: the commons already refuses an enrollment into any other account,
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
        // A production enrollment always has its witness. A staging one has
        // none (staging spec S12), and records no witness origin either.
        witness_origin: commons.witness.as_ref().map(|witness| {
            crate::config::WitnessOriginRecord::for_witness(
                witness,
                crate::config::WitnessOrigin::PublishedAtJoin,
            )
        }),
        witness: commons.witness,
        inference_receipt_endpoint: commons.receipt_endpoint,
        consent_scopes_chosen: Some(false),
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
    // A NEAR AI enrollment renews through its NEAR AI login, never by
    // device key, so no joined-by-code marker may stand beside it. A code
    // sign-in writes its marker after this returns.
    store
        .remove_daemon_file(super::device_codes::JOINED_BY_CODE_FILE)
        .map_err(|_| anyhow!("near_ai_enroll_invalid"))?;
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
            let error = enroll(&shared, &api, address, SignupReferral::None)
                .await
                .unwrap_err();
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

    /// The other channel's commons is refused before anything is sent to it:
    /// a production build never fetches the staging commons's capability or
    /// hands it a NEAR AI token, nor the reverse, and the refresh token is
    /// not spent.
    #[tokio::test]
    async fn the_other_channel_s_commons_is_refused_before_any_request() {
        #[cfg(not(feature = "near-ai-staging"))]
        let other = format!(
            "https://ingest.{}",
            ["staging", "tracecommons", "ai"].join(".")
        );
        #[cfg(feature = "near-ai-staging")]
        let other = "https://ingest.tracecommons.ai".to_string();
        let shared = shared_with_session();
        let (api, hits) = counting_cloud_api().await;
        let error = enroll(&shared, &api, &other, SignupReferral::None)
            .await
            .unwrap_err();
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(label(&error), "near_ai_enroll_endpoint_refused");
        assert_eq!(stored_refresh(&shared).as_deref(), Some(REFRESH));
        assert!(shared.store.load_config().unwrap().is_none());
        // `prepare` itself refuses, so the legacy invite migration, which
        // calls it directly, is covered too.
        let refusal = prepare(&shared, &other).await.err().unwrap();
        assert_eq!(label(&refusal), "near_ai_enroll_endpoint_refused");
    }

    /// The first run's "Sign in with near.ai" without an invite names no
    /// commons: an unenrolled daemon enrolls against the native account's
    /// commons, the one origin `native_identity` trusts before any config
    /// exists. A named address is used as given, as before.
    #[test]
    fn an_enrollment_naming_no_commons_uses_the_native_account_origin() {
        let (dir, store) = crate::config::tests_support::temp_store();
        std::mem::forget(dir);
        assert_eq!(
            enroll_origin(&store, &serde_json::json!({})).unwrap(),
            crate::channel::NATIVE_ACCOUNT_ORIGIN
        );
        assert_eq!(
            enroll_origin(
                &store,
                &serde_json::json!({"ingest_url": "https://commons.example"})
            )
            .unwrap(),
            "https://commons.example"
        );
    }

    /// No login, no enrollment -- and no attempt to invent one.
    #[tokio::test]
    async fn without_a_near_ai_session_enrollment_refuses() {
        let (dir, store) = crate::config::tests_support::temp_store();
        std::mem::forget(dir);
        let shared = DaemonShared::load(store).unwrap();
        let (api, hits) = counting_cloud_api().await;
        let error = enroll(
            &shared,
            &api,
            "https://commons.example",
            SignupReferral::Send("ABCDE12345".into()),
        )
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
            witness: Some(
                serde_json::from_value(serde_json::json!({
                    "url": "https://witness.example",
                    "signing_address": format!("0x{}", "ab".repeat(20)),
                    "expected_measurements": [format!("mrtd={}", "ab".repeat(48))],
                    "admission_evidence": true,
                }))
                .unwrap(),
            ),
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
            referral: None,
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

    #[test]
    fn a_revoked_key_refusal_is_named_and_reaches_the_renewal_class() {
        let refused = |label: &str| trace_commons_operator_client::Error::ServerLabel {
            url: "https://commons.example/v1/account/near-ai/provision/finish".into(),
            status: reqwest::StatusCode::CONFLICT,
            label: label.into(),
            body: format!("{{\"error\":\"{label}\"}}"),
        };
        let revoked = finish_refusal(&refused(DEVICE_KEY_REVOKED_WIRE), "fallback");
        assert_eq!(revoked.to_string(), ENROLL_DEVICE_KEY_REVOKED);
        assert_eq!(label(&revoked), ENROLL_DEVICE_KEY_REVOKED);
        assert_eq!(
            super::super::account_session_renewal::classify(&revoked),
            super::super::account_session_renewal::FAILURE_DEVICE_KEY_REVOKED
        );
        assert_eq!(
            finish_refusal(&refused("anything_else"), "fallback").to_string(),
            "fallback"
        );
    }
}

/// Connect a retained NEAR AI login to the signed-in passkey account: bind an
/// unbound one, or enroll this Mac into a bound one. The authenticated bind
/// protocol has its own account-bound proof; the unauthenticated provisioning
/// preimage is never substituted for it.
///
/// An enrollment's expected account is the passkey session's, which the
/// commons reads from the session itself: nothing here names it. If this
/// Mac's near.ai login is someone else's, the commons refuses before it
/// writes the device key, and this answers `account-enrol-mismatch` with the
/// passkey session left as it was for the shell to sign out.
pub(super) async fn bind(shared: &DaemonShared) -> Result<serde_json::Value> {
    use super::native_identity::{authenticated, resolve_origin};
    use reqwest::Method;
    use serde_json::json;
    if let Some(config) = shared.store.load_config()? {
        return already_enrolled(shared, &config);
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

/// `account_bind` on a Mac that is already enrolled: the enrollment it holds
/// may be the signed-in account's own (Welcome back's sign-in, after the
/// first status reported an enrollment). Answered from local state alone,
/// with no request and nothing written:
///
/// - the session's tenant (decoded from its `tcn1_` token, as
///   `persist_session` reads it) is the enrollment's `tenant_id`, and
/// - the session was signed in to a `bound` account (the `binding_state`
///   stored with it).
///
/// Both hold: `{"outcome":"already_enrolled","binding_state":"bound"}`.
/// Anything else, including no session or a record without a stored state,
/// keeps the refusal `account-already-enrolled`.
///
/// The config records a tenant and no account, so this is a tenant match.
/// It names one account because a binding row, and so a `bound` state, is
/// only ever written for a passkey-origin account, and each of those is
/// created alone in a freshly minted tenant. It grants nothing
/// `persist_session` had not already accepted when it kept this session
/// under this enrollment.
fn already_enrolled(
    shared: &DaemonShared,
    config: &crate::config::ContributorConfig,
) -> Result<serde_json::Value> {
    let refused = || anyhow!("account-already-enrolled");
    let session = crate::account_auth::try_load_session_with_snapshot(&shared.store)
        .ok()
        .flatten()
        .ok_or_else(refused)?;
    let same_tenant =
        token_tenant(&session.session.access_token).as_deref() == Some(config.tenant_id.as_str());
    let bound = session.stored_binding_state().as_deref() == Some("bound");
    if !(same_tenant && bound) {
        return Err(refused());
    }
    Ok(serde_json::json!({"outcome":"already_enrolled","binding_state":"bound"}))
}

/// Whether `account_bind` runs a ceremony for the signed-in account's
/// `binding_state`: `unbound` binds it to this Mac's near.ai login; `bound`
/// (another Mac bound it, and this one signed in with the same passkey)
/// enrolls this Mac into it, which the commons allows only when this Mac's
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
    let token =
        exchange_following_rotation(shared, api, &prepared.session, prepared.login.as_ref())
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

    /// An unbound account binds and a bound one enrolls this Mac (a second
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
            let prepared=Prepared {commons:super::super::native_identity::client(&origin,"unauthenticated").unwrap(),session:near,login:None,issuer_url:"https://issuer.example".into(),audience:"trace-commons-upload".into(),witness:Some(serde_json::from_value(json!({"url":"https://witness.example","signing_address":format!("0x{}","ab".repeat(20)),"expected_measurements":[format!("mrtd={}","ab".repeat(48))],"admission_evidence":true})).unwrap()),receipt_endpoint:None,referral_code:None};
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
                assert_eq!(cfg.consent_scopes_chosen, Some(false));
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

/// Which provisioning routes each build's enrollment calls, run end to end
/// against a loopback commons (#1574 review, finding H1).
///
/// Every step after the address check is the real one: the capability fetch
/// and its validation, the start body, the device proof, the refresh-token
/// exchange, the finish and the published enrollment. The commons is a stub
/// that records each request and answers in the server's own shapes; the
/// server-side half of the same contract runs against the real handlers in
/// the ingest's suite.
#[cfg(test)]
mod route_tests {
    use super::*;
    use serde_json::{Value, json};
    use std::sync::{Arc, Mutex};

    const LEGACY_CAPABILITIES: &str = "/v1/account/near/provision/capabilities";
    const LEGACY_START: &str = "/v1/account/near-ai/provision/start";
    const LEGACY_FINISH: &str = "/v1/account/near-ai/provision/finish";
    const EXCHANGE: &str = "/v1/users/me/access-tokens";

    /// How the stub commons answers.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Commons {
        /// A production commons: the old capabilities route is ready and
        /// publishes its witness; v2 is ready as well.
        Production,
        /// A staging commons (S12): no witness, so the old route answers its
        /// bare not-ready document, and v2 is ready.
        Staging,
        /// A staging commons whose v2 document says the login path is closed.
        StagingLoginClosed,
        /// A staging commons whose v2 finish refuses (the uniform deny).
        StagingFinishRefused,
        /// A production commons from before referral codes: its finish is
        /// `deny_unknown_fields`, so a body carrying `referral_code` gets
        /// the uniform deny, and one without it is served.
        #[cfg_attr(feature = "near-ai-staging", allow(dead_code))]
        ProductionWithoutReferrals,
        /// A production commons whose finish answers the uniform deny to
        /// every body: a real verification failure.
        #[cfg_attr(feature = "near-ai-staging", allow(dead_code))]
        ProductionFinishDenied,
    }

    struct Stub {
        origin: String,
        requests: Arc<Mutex<Vec<String>>>,
        /// The refresh token each exchange presented, in order. Fixture
        /// values, never a real credential.
        spent: Arc<Mutex<Vec<String>>>,
        /// The `referral_code` each finish carried, in order: `None` for a
        /// finish without the field. Read only by the production-route
        /// referral test; a staging build records it unread.
        #[cfg_attr(feature = "near-ai-staging", allow(dead_code))]
        finish_codes: Arc<Mutex<Vec<Option<Value>>>>,
        server: tokio::task::JoinHandle<()>,
    }

    impl Stub {
        fn requests(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }

        fn spent(&self) -> Vec<String> {
            self.spent.lock().unwrap().clone()
        }
    }

    impl Drop for Stub {
        fn drop(&mut self) {
            self.server.abort();
        }
    }

    fn witness() -> Value {
        json!({
            "url": "https://witness.example",
            "signing_address": format!("0x{}", "ab".repeat(20)),
            "expected_measurements": [format!("mrtd={}", "cd".repeat(48))],
        })
    }

    async fn stub(commons: Commons, device: DeviceIdentity) -> Stub {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let spent: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let finish_codes: Arc<Mutex<Vec<Option<Value>>>> = Arc::new(Mutex::new(Vec::new()));
        let challenge = Arc::new(Mutex::new(String::new()));
        let nonce = [7u8; 32];
        let expires = Utc::now().timestamp() + 300;
        let record = {
            let requests = Arc::clone(&requests);
            move |method: &axum::http::Method, uri: &axum::http::Uri| {
                requests
                    .lock()
                    .unwrap()
                    .push(format!("{method} {}", uri.path()));
            }
        };
        let app = axum::Router::new()
            .route(
                LEGACY_CAPABILITIES,
                axum::routing::get(move || async move {
                    axum::Json(if commons == Commons::Production
                        || commons == Commons::ProductionWithoutReferrals
                        || commons == Commons::ProductionFinishDenied
                    {
                        json!({
                            "ready": false, "near_ai_login_ready": true, "network": null,
                            "witness": witness(), "issuer_url": "https://issuer.example",
                            "audience": "upload", "inference_receipt_endpoint": null,
                            "funding_available": false,
                        })
                    } else {
                        // What `capabilities` answers on a commons with no
                        // witness.
                        json!({"ready": false, "near_ai_login_ready": false, "funding_available": false})
                    })
                }),
            )
            .route(
                account_onboarding_v2::CAPABILITIES,
                axum::routing::get(move || async move {
                    axum::Json(json!({
                        "contract_version": "near-provision-capabilities-v2",
                        "ready": false,
                        "near_ai_login_ready": commons != Commons::StagingLoginClosed,
                        "network": null,
                        "wallet_start_path": "/v1/account/near/provision/start/v2",
                        "wallet_finish_path": "/v1/account/near/provision/finish/v2",
                        "near_ai_start_path": "/v1/account/near-ai/provision/start/v2",
                        "near_ai_finish_path": "/v1/account/near-ai/provision/finish/v2",
                        "inference_connection_selection_required": true,
                        "offers_available": false,
                        "funding_available": false,
                        "issuer_url": "https://issuer.staging-fixture.example",
                        "audience": "staging-upload",
                        "inference_receipt_endpoint": "https://receipts.staging-fixture.example",
                    }))
                }),
            );
        let start = {
            let challenge = Arc::clone(&challenge);
            move |axum::Json(body): axum::Json<Value>| {
                let challenge = Arc::clone(&challenge);
                async move {
                    // The server parses this with `deny_unknown_fields`.
                    assert_eq!(
                        body,
                        start_payload(
                            body["code_challenge"].as_str().unwrap(),
                            body["device_public_key"].as_str().unwrap()
                        )
                    );
                    *challenge.lock().unwrap() = body["code_challenge"].as_str().unwrap().into();
                    axum::Json(json!({
                        "ceremony_id": "ceremony-fixture",
                        "nonce": base64::engine::general_purpose::STANDARD.encode(nonce),
                        "expires_at": expires,
                        "device_signing_bytes": "",
                    }))
                }
            }
        };
        let finish = {
            let challenge = Arc::clone(&challenge);
            let device = Arc::new(device);
            let finish_codes = Arc::clone(&finish_codes);
            move |axum::Json(body): axum::Json<Value>| {
                let challenge = challenge.lock().unwrap().clone();
                let device = Arc::clone(&device);
                finish_codes
                    .lock()
                    .unwrap()
                    .push(body.get("referral_code").cloned());
                // The commons applies one code, and answers `referral` only
                // when the finish carried one.
                let referral = body.get("referral_code").map(|code| {
                    if code == "ABCDE12345" {
                        "applied"
                    } else {
                        "not_applied"
                    }
                });
                // The 400 an ingest answers for any refused finish, a
                // body it could not parse included (`native_generic_deny`).
                let generic_deny = commons == Commons::ProductionFinishDenied
                    || (commons == Commons::ProductionWithoutReferrals
                        && body.get("referral_code").is_some());
                async move {
                    if generic_deny {
                        return (
                            axum::http::StatusCode::BAD_REQUEST,
                            axum::Json(json!({"error": "native sign-in could not be completed"})),
                        );
                    }
                    if commons == Commons::StagingFinishRefused {
                        return (
                            axum::http::StatusCode::FORBIDDEN,
                            axum::Json(json!({"error": "forbidden"})),
                        );
                    }
                    let public: [u8; 32] = base64::engine::general_purpose::STANDARD
                        .decode(&device.public_key_b64)
                        .unwrap()
                        .try_into()
                        .unwrap();
                    let preimage =
                        trace_commons_protocol::onboarding::near_ai_provisioning_device_bytes(
                            &nonce,
                            "ceremony-fixture",
                            &public,
                            &challenge,
                            expires,
                        );
                    let signature = base64::engine::general_purpose::STANDARD
                        .decode(body["device_signature"].as_str().unwrap())
                        .unwrap();
                    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, public)
                        .verify(&preimage, &signature)
                        .expect("the device signed this ceremony");
                    assert_eq!(body["access_token"], "near-access");
                    assert!(body.get("account_id").is_none());
                    let tenant = format!("nearai-{}", "3c".repeat(32));
                    let mut answer = json!({
                        "access_token": format!("tcn1_{}.secret", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&tenant)),
                        "token_type": "Bearer",
                        "expires_in_secs": 43200,
                        "account_id": uuid::Uuid::new_v4().to_string(),
                        "tenant_id": tenant,
                        "device_key_id": device.device_key_id,
                        "anchor_hash": format!("sha256:{}", "ab".repeat(32)),
                    });
                    if let Some(referral) = referral {
                        answer["referral"] = json!(referral);
                    }
                    (axum::http::StatusCode::OK, axum::Json(answer))
                }
            }
        };
        let app = app
            .route(LEGACY_START, axum::routing::post(start.clone()))
            .route(account_onboarding_v2::START, axum::routing::post(start))
            .route(LEGACY_FINISH, axum::routing::post(finish.clone()))
            .route(account_onboarding_v2::FINISH, axum::routing::post(finish))
            .route(
                EXCHANGE,
                axum::routing::post({
                    let spent = Arc::clone(&spent);
                    move |headers: axum::http::HeaderMap| {
                        // Each exchange rotates: the first hands back
                        // `rt_rotated`, later ones `rt_rotated_<n>`.
                        let presented = headers
                            .get(axum::http::header::AUTHORIZATION)
                            .and_then(|v| v.to_str().ok())
                            .and_then(|v| v.strip_prefix("Bearer "))
                            .unwrap_or_default()
                            .to_string();
                        let n = {
                            let mut spent = spent.lock().unwrap();
                            spent.push(presented);
                            spent.len()
                        };
                        let rotated = if n == 1 {
                            "rt_rotated".to_string()
                        } else {
                            format!("rt_rotated_{n}")
                        };
                        async move {
                            axum::Json(json!({
                                "access_token": "near-access",
                                "refresh_token": rotated,
                                "refresh_token_expiration": Utc::now() + chrono::Duration::days(1),
                            }))
                        }
                    }
                }),
            )
            .layer(axum::middleware::from_fn(
                move |request: axum::extract::Request, next: axum::middleware::Next| {
                    record(request.method(), request.uri());
                    next.run(request)
                },
            ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Stub {
            origin,
            requests,
            spent,
            finish_codes,
            server,
        }
    }

    mod account_onboarding_v2 {
        pub const CAPABILITIES: &str =
            crate::daemon::account_onboarding::NEAR_AI_CAPABILITIES_V2_PATH;
        pub const START: &str = crate::daemon::account_onboarding::NEAR_AI_START_V2_PATH;
        pub const FINISH: &str = crate::daemon::account_onboarding::NEAR_AI_FINISH_V2_PATH;
    }

    fn shared_with_session() -> (tempfile::TempDir, DaemonShared) {
        let (dir, store) = crate::config::tests_support::temp_store();
        let shared = DaemonShared::load(store).unwrap();
        let mut settings = shared.settings.lock().unwrap();
        settings.near_ai_session = Some(crate::daemon::settings::NearAiSession {
            refresh_token: "rt_original".into(),
            refresh_token_expires_at: None,
            stored_at: Utc::now(),
            user_agent: "Mozilla/5.0 Test".into(),
        });
        settings.save_for_test(&shared.store).unwrap();
        drop(settings);
        (dir, shared)
    }

    /// `enroll` against the stub, from the step after the address check (the
    /// stub is loopback HTTP, which that check refuses).
    async fn enroll_against(
        commons: Commons,
    ) -> (Stub, DaemonShared, Result<Value>, tempfile::TempDir) {
        enroll_against_with(commons, SignupReferral::None).await
    }

    /// [`enroll_against`], with the referral code the request carried.
    async fn enroll_against_with(
        commons: Commons,
        referral: SignupReferral,
    ) -> (Stub, DaemonShared, Result<Value>, tempfile::TempDir) {
        let (dir, shared) = shared_with_session();
        let identity = DeviceIdentity::load_or_generate(&shared.store).unwrap();
        let stub = stub(commons, identity).await;
        let expected = super::super::commons_credentials::snapshot(
            &shared.store,
            super::super::commons_credentials::Kind::Account,
        )
        .unwrap();
        let client =
            super::super::native_identity::client(&stub.origin, "unauthenticated").unwrap();
        let api = CloudApi::for_test(&stub.origin).unwrap();
        let result = match prepare_on(&shared, client, &stub.origin).await {
            Ok(prepared) => enroll_prepared(
                &shared,
                &api,
                &stub.origin,
                prepared.with_referral_code(referral.code()),
                expected,
            )
            .await
            .map(|enrolled| enrolled.answer(&referral)),
            Err(error) => Err(error),
        };
        (stub, shared, result, dir)
    }

    fn stored_refresh(shared: &DaemonShared) -> String {
        crate::daemon::settings::DaemonSettings::load_with_cloud_credentials(&shared.store)
            .unwrap()
            .near_ai_session
            .unwrap()
            .refresh_token
    }

    /// A production build signs up exactly as every deployed client does: the
    /// old capabilities route, its witness, and the old start and finish.
    #[cfg(not(feature = "near-ai-staging"))]
    #[tokio::test]
    async fn a_production_enrollment_keeps_today_s_route_sequence() {
        assert_eq!(PROVISION_START_PATH, LEGACY_START);
        assert_eq!(PROVISION_FINISH_PATH, LEGACY_FINISH);
        let (stub, shared, result, _dir) = enroll_against(Commons::Production).await;
        let reply = result.expect("a production enrollment");
        assert_eq!(reply["enrolled"], true);
        assert_eq!(
            stub.requests(),
            vec![
                format!("GET {LEGACY_CAPABILITIES}"),
                format!("POST {LEGACY_START}"),
                format!("POST {EXCHANGE}"),
                format!("POST {LEGACY_FINISH}"),
            ]
        );
        let config = shared.store.load_config().unwrap().unwrap();
        assert_eq!(config.issuer_url, "https://issuer.example");
        assert_eq!(config.audience, "upload");
        assert_eq!(
            config.witness.as_ref().map(|w| w.url.as_str()),
            Some("https://witness.example")
        );
        assert_eq!(
            config.witness_origin_view(),
            Some(crate::config::WitnessOriginView::Recorded(
                crate::config::WitnessOrigin::PublishedAtJoin
            ))
        );
    }

    /// A referral code typed at sign-up rides in the finish, canonical, and
    /// the answer says whether the commons applied it. Without one the
    /// finish carries no field and the answer no `referral`, as before.
    #[cfg(not(feature = "near-ai-staging"))]
    #[tokio::test]
    async fn a_sign_up_referral_code_rides_the_finish_and_is_answered() {
        for (typed, sent, answered) in [
            (json!(null), None, None),
            (json!("   "), None, None),
            (
                json!("tc-abcde-12345"),
                Some(json!("ABCDE12345")),
                Some("applied"),
            ),
            (
                json!("ZZZZZ99999"),
                Some(json!("ZZZZZ99999")),
                Some("not_applied"),
            ),
            // Not a code: never sent, and the answer says it was not applied.
            (json!("not a code"), None, Some("not_applied")),
        ] {
            let referral = SignupReferral::from_params(&json!({ "referral_code": typed })).unwrap();
            let (stub, _shared, result, _dir) =
                enroll_against_with(Commons::Production, referral).await;
            let reply = result.expect("enrolled, whatever the code");
            assert_eq!(reply["enrolled"], true, "{typed}");
            assert_eq!(
                reply.get("referral").and_then(Value::as_str),
                answered,
                "{typed}"
            );
            assert_eq!(*stub.finish_codes.lock().unwrap(), vec![sent], "{typed}");
        }
    }

    /// A commons from before referral codes refuses a finish carrying
    /// `referral_code` (its finish is `deny_unknown_fields`, and the refusal
    /// is the uniform deny). Sign-in never fails because of the code: the
    /// finish is sent once more without it, on the same ceremony and the
    /// same NEAR AI token, and the answer says the code was not applied.
    #[cfg(not(feature = "near-ai-staging"))]
    #[tokio::test]
    async fn a_commons_without_referrals_still_signs_in_without_the_code() {
        let referral =
            SignupReferral::from_params(&json!({ "referral_code": "ABCDE12345" })).unwrap();
        let (stub, shared, result, _dir) =
            enroll_against_with(Commons::ProductionWithoutReferrals, referral).await;
        let reply = result.expect("enrolled without the code");
        assert_eq!(reply["enrolled"], true);
        assert_eq!(reply["referral"], "not_applied");
        assert_eq!(
            *stub.finish_codes.lock().unwrap(),
            vec![Some(json!("ABCDE12345")), None]
        );
        // One exchange: the retry reuses the token, never spends another.
        assert_eq!(stub.spent(), vec!["rt_original".to_string()]);
        assert!(shared.store.load_config().unwrap().is_some());
    }

    /// The retry is bounded to that one case: a finish without a code that
    /// is refused, or one with a code refused twice, is the verification
    /// failure it always was.
    #[cfg(not(feature = "near-ai-staging"))]
    #[tokio::test]
    async fn a_refused_finish_is_retried_only_once_and_only_for_a_code() {
        let (stub, _shared, result, _dir) = enroll_against(Commons::ProductionFinishDenied).await;
        assert_eq!(
            result.unwrap_err().to_string(),
            "near_ai_enroll_verification_failed"
        );
        assert_eq!(*stub.finish_codes.lock().unwrap(), vec![None]);

        let referral =
            SignupReferral::from_params(&json!({ "referral_code": "ABCDE12345" })).unwrap();
        let (stub, _shared, result, _dir) =
            enroll_against_with(Commons::ProductionFinishDenied, referral).await;
        assert_eq!(
            result.unwrap_err().to_string(),
            "near_ai_enroll_verification_failed"
        );
        assert_eq!(
            *stub.finish_codes.lock().unwrap(),
            vec![Some(json!("ABCDE12345")), None]
        );
    }

    /// `referral_code` is a string or absent; anything else is a malformed
    /// request, refused before anything is spent.
    #[test]
    fn a_referral_code_parameter_is_a_string_or_absent() {
        assert_eq!(
            SignupReferral::from_params(&json!({})),
            Ok(SignupReferral::None)
        );
        assert_eq!(
            SignupReferral::from_params(&json!({"referral_code": "TC-7K3M9-QX2RD"})),
            Ok(SignupReferral::Send("7K3M9QX2RD".into()))
        );
        assert_eq!(
            SignupReferral::from_params(&json!({"referral_code": "TC-7K3M9-QX2RU"})),
            Ok(SignupReferral::NotACode)
        );
        for bad in [
            json!(12),
            json!(["ABCDE12345"]),
            json!({"code": "ABCDE12345"}),
            json!(true),
        ] {
            assert_eq!(
                SignupReferral::from_params(&json!({ "referral_code": bad })),
                Err(()),
                "{bad}"
            );
        }
        // A renewal never carries one, so only a first enrollment answers
        // `referral`.
        assert_eq!(SignupReferral::None.outcome(None), None);
        assert_eq!(
            SignupReferral::Send("ABCDE12345".into()).outcome(None),
            Some(trace_commons_protocol::referral::ReferralOutcome::NotApplied),
            "a commons that said nothing applied nothing"
        );
    }

    /// A production build does not sign up on a commons with no witness,
    /// whatever its v2 route says: it never asks v2.
    #[cfg(not(feature = "near-ai-staging"))]
    #[tokio::test]
    async fn a_production_build_never_asks_the_v2_routes() {
        let (stub, shared, result, _dir) = enroll_against(Commons::Staging).await;
        assert_eq!(
            label(&result.unwrap_err()),
            "near_ai_enroll_commons_unsupported"
        );
        assert_eq!(stub.requests(), vec![format!("GET {LEGACY_CAPABILITIES}")]);
        assert_eq!(stored_refresh(&shared), "rt_original");
        assert!(shared.store.load_config().unwrap().is_none());
    }

    /// A staging build signs up through the v2 routes against a commons with
    /// no witness, and records the issuer and audience v2 published.
    #[cfg(feature = "near-ai-staging")]
    #[tokio::test]
    async fn a_staging_enrollment_uses_the_v2_routes_and_needs_no_witness() {
        assert_eq!(PROVISION_START_PATH, account_onboarding_v2::START);
        assert_eq!(PROVISION_FINISH_PATH, account_onboarding_v2::FINISH);
        let (stub, shared, result, _dir) = enroll_against(Commons::Staging).await;
        let reply = result.expect("a staging enrollment");
        assert_eq!(reply["enrolled"], true);
        assert!(!reply.to_string().contains("secret"));
        assert_eq!(
            stub.requests(),
            vec![
                format!("GET {}", account_onboarding_v2::CAPABILITIES),
                format!("POST {}", account_onboarding_v2::START),
                format!("POST {EXCHANGE}"),
                format!("POST {}", account_onboarding_v2::FINISH),
            ]
        );
        let config = shared.store.load_config().unwrap().unwrap();
        assert_eq!(config.issuer_url, "https://issuer.staging-fixture.example");
        assert_eq!(config.audience, "staging-upload");
        assert!(config.witness.is_none());
        assert!(config.witness_origin.is_none());
        assert_eq!(
            config.inference_receipt_endpoint.as_deref(),
            Some("https://receipts.staging-fixture.example")
        );
        assert_eq!(stored_refresh(&shared), "rt_rotated");
    }

    /// A v2 refusal reaches the shell under this module's own label, spends
    /// nothing it did not have to, and publishes no enrollment.
    #[cfg(feature = "near-ai-staging")]
    #[tokio::test]
    async fn a_v2_refusal_is_surfaced_with_its_label() {
        // The login path closed: refused before the refresh token is spent.
        let (stub, shared, result, _dir) = enroll_against(Commons::StagingLoginClosed).await;
        assert_eq!(
            label(&result.unwrap_err()),
            "near_ai_enroll_commons_unsupported"
        );
        assert_eq!(
            stub.requests(),
            vec![format!("GET {}", account_onboarding_v2::CAPABILITIES)]
        );
        assert_eq!(stored_refresh(&shared), "rt_original");
        assert!(shared.store.load_config().unwrap().is_none());

        // The finish refused.
        let (stub, shared, result, _dir) = enroll_against(Commons::StagingFinishRefused).await;
        assert_eq!(
            label(&result.unwrap_err()),
            "near_ai_enroll_verification_failed"
        );
        assert_eq!(
            stub.requests().last().map(String::as_str),
            Some(format!("POST {}", account_onboarding_v2::FINISH).as_str())
        );
        assert!(
            !stub
                .requests()
                .iter()
                .any(|r| r.ends_with(LEGACY_START) || r.ends_with(LEGACY_FINISH)),
            "a staging build asked the old routes"
        );
        assert!(shared.store.load_config().unwrap().is_none());
    }

    /// The inference credential a sign-in mints beside its session. Fixture
    /// values, never a real key.
    fn inference(key_id: &str) -> crate::daemon::settings::NearAiInferenceCredential {
        crate::daemon::settings::NearAiInferenceCredential {
            key: format!("sk-fixture-{key_id}"),
            key_id: key_id.into(),
            key_prefix: "sk-fixtur".into(),
            organization_id: "org-fixture".into(),
            workspace_id: "ws-fixture".into(),
            minted_at: Utc::now(),
        }
    }

    /// A daemon right after a sign-in: a minted key and its session.
    fn shared_signed_in() -> (tempfile::TempDir, DaemonShared) {
        let (dir, shared) = shared_with_session();
        let mut settings = shared.settings.lock().unwrap();
        settings.near_ai_inference = Some(inference("key-fixture-1"));
        settings.save_for_test(&shared.store).unwrap();
        drop(settings);
        (dir, shared)
    }

    /// Prepare against the stub, then let something else spend the session
    /// before the ceremony reaches its own exchange -- what the shell's
    /// balance read does the moment a sign-in lands, while the first run is
    /// already enrolling.
    async fn enroll_after_a_concurrent_rotation(
        shared: &DaemonShared,
        stub: &Stub,
        between: impl AsyncFnOnce(&DaemonShared, &CloudApi),
    ) -> Result<Value> {
        let expected = super::super::commons_credentials::snapshot(
            &shared.store,
            super::super::commons_credentials::Kind::Account,
        )
        .unwrap();
        let client =
            super::super::native_identity::client(&stub.origin, "unauthenticated").unwrap();
        let api = CloudApi::for_test(&stub.origin).unwrap();
        let prepared = prepare_on(shared, client, &stub.origin).await.unwrap();
        between(shared, &api).await;
        enroll_prepared(shared, &api, &stub.origin, prepared, expected)
            .await
            .map(|enrolled| enrolled.value)
    }

    /// The first click of "Sign in with near.ai" failed: start answered, the
    /// finish was never sent, a second click worked. The shell reads the
    /// balance as soon as the sign-in lands, and that read rotates the
    /// refresh token while enrollment is between its session snapshot and
    /// its own exchange. The exchange then refuses the stale token -- before
    /// sending it anywhere -- and enrollment used to give up there.
    ///
    /// The same login, rotated, is followed once: the retry spends the token
    /// the rotation stored, never the retired one, and the rotation it
    /// produces is stored in turn.
    #[tokio::test]
    async fn a_rotation_by_a_concurrent_balance_read_does_not_fail_enrollment() {
        let (_dir, shared) = shared_signed_in();
        let identity = DeviceIdentity::load_or_generate(&shared.store).unwrap();
        #[cfg(feature = "near-ai-staging")]
        let commons = Commons::Staging;
        #[cfg(not(feature = "near-ai-staging"))]
        let commons = Commons::Production;
        let stub = stub(commons, identity).await;
        let result = enroll_after_a_concurrent_rotation(&shared, &stub, async |shared, api| {
            let session = shared
                .settings
                .lock()
                .unwrap()
                .near_ai_session
                .clone()
                .unwrap();
            crate::daemon::nearai_credential::exchange(shared, api, &session)
                .await
                .expect("the balance read's exchange");
        })
        .await;
        let reply = result.expect("enrollment follows the rotation");
        assert_eq!(reply["enrolled"], true);
        assert_eq!(
            stub.spent(),
            vec!["rt_original".to_string(), "rt_rotated".to_string()],
            "each refresh token is spent exactly once, the retired one never again"
        );
        assert_eq!(
            stub.requests().last().map(String::as_str),
            Some(format!("POST {PROVISION_FINISH_PATH}").as_str())
        );
        assert_eq!(stored_refresh(&shared), "rt_rotated_2");
        assert!(shared.store.load_config().unwrap().is_some());
    }

    /// A different login is not a rotation. A sign-in that replaced the key
    /// and the session while enrollment was in flight is refused, and its
    /// new session is not spent on a ceremony prepared for the old one.
    #[tokio::test]
    async fn a_new_sign_in_during_enrollment_is_refused_not_followed() {
        let (_dir, shared) = shared_signed_in();
        let identity = DeviceIdentity::load_or_generate(&shared.store).unwrap();
        #[cfg(feature = "near-ai-staging")]
        let commons = Commons::Staging;
        #[cfg(not(feature = "near-ai-staging"))]
        let commons = Commons::Production;
        let stub = stub(commons, identity).await;
        let result = enroll_after_a_concurrent_rotation(&shared, &stub, async |shared, _| {
            let mut settings = shared.settings.lock().unwrap();
            settings.near_ai_inference = Some(inference("key-fixture-2"));
            settings.near_ai_session = Some(crate::daemon::settings::NearAiSession {
                refresh_token: "rt_second_sign_in".into(),
                refresh_token_expires_at: None,
                stored_at: Utc::now(),
                user_agent: "Mozilla/5.0 Test".into(),
            });
            settings.save_for_test(&shared.store).unwrap();
        })
        .await;
        assert_eq!(
            label(&result.unwrap_err()),
            "near_ai_enroll_token_unavailable"
        );
        assert!(stub.spent().is_empty(), "no refresh token was spent");
        assert_eq!(stored_refresh(&shared), "rt_second_sign_in");
        assert!(shared.store.load_config().unwrap().is_none());
    }
}
