// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Native passkey identity, slice S2
//! (`docs/superpowers/specs/2026-09-28-native-passkey-identity-design.md`).
//!
//! Unauthenticated, registered in `app()` beside `native/authorize` and
//! `native/token` and outside the account router:
//!
//! - `POST /v1/account/native/passkey/create/{start,finish}` creates an
//!   `unbound` passkey-origin account and returns a weak `tcn1_` session.
//! - `POST /v1/account/native/passkey/login/{start,finish}` signs in with a
//!   passkey and returns a weak `tcn1_` session. The assertion is verified by
//!   `verify_discoverable_passkey_assertion`, the same core the browser
//!   sign-in uses.
//!
//! Authenticated, registered through `AccountRoutes` and `Refused` for
//! unbound accounts:
//!
//! - `POST /v1/account/passkeys/native/register/{start,finish}` adds a passkey
//!   to a signed-in account from a native session, behind the Slice 3a gate.
//!
//! **Ceremonies are body-bound.** Where the browser carries the ceremony id in
//! the [`ACCOUNT_PASSKEY_CEREMONY_COOKIE`] cookie, these routes return it in the `start`
//! body and read it from the `finish` body. The id is a 160-bit CSPRNG value;
//! the stored state holds the challenge, which webauthn-rs requires the
//! response to have signed; `take` makes it single use; the store's TTL is
//! three minutes; and the id is capped per window. Each handler accepts only
//! its own [`CeremonyState`] variant, so a browser ceremony cannot finish a
//! native one or the reverse, and a creation cannot finish a sign-in.
//!
//! **Treat every caller as a script.** Attestation is `none`, so nothing here
//! can tell a platform authenticator from a program. Every refusal of the
//! unauthenticated routes is the one [`native_generic_deny`] response, both
//! finishes sit behind the same timing floor as `native/token`, `start` writes
//! nothing anywhere, and an abandoned ceremony costs one in-memory entry.
//! Creation is additionally bounded by the unbound-account ceiling, which is
//! checked at `start` and again inside the `finish` transaction, and by a
//! per-IP cap on successful creations in a rolling 24 hours
//! ([`PerSourceCreationCap`](trace_commons_server::account_native_passkey::PerSourceCreationCap)),
//! checked at `finish`.
//!
//! Nothing is logged or audited but fixed labels: no ceremony id, credential
//! id, challenge, token or label.

use super::*;
use trace_commons_server::account_binding::AccountBindingState;
use trace_commons_server::account_native_passkey::{CeilingCheck, normalize_passkey_label};
use trace_commons_server::db::{NewPasskeyOriginAccount, PasskeyOriginAccountOutcome};

/// Per-IP cap per window on each native passkey route, as `native/token`.
pub(crate) const NATIVE_PASSKEY_PER_IP_LIMIT: u32 = NATIVE_TOKEN_PER_IP_LIMIT;
/// Coarse global cap per window on each native passkey route.
pub(crate) const NATIVE_PASSKEY_GLOBAL_LIMIT: u32 = NATIVE_TOKEN_GLOBAL_LIMIT;
/// Attempts per window against one ceremony id, IP-independent. A ceremony is
/// single use, so this bounds replay against one id from rotating sources.
pub(crate) const NATIVE_PASSKEY_PER_CEREMONY_LIMIT: u32 = NATIVE_TOKEN_PER_CODE_LIMIT;
/// Authenticated native registration `start`s per account per window. Each
/// one parks a ceremony in the in-process store for its TTL, and the store has
/// no size cap of its own, so the unauthenticated routes are bounded by the
/// limits above and this route by this one.
pub(crate) const NATIVE_PASSKEY_REGISTER_PER_ACCOUNT_LIMIT: u32 = 10;
/// Ceremony ids are 27 base64url characters; anything much longer is refused
/// before it is hashed into a limiter key.
const NATIVE_PASSKEY_CEREMONY_ID_MAX_LEN: usize = 64;

/// `create/start` body. Unknown fields (a tenant, an account) are refused at
/// the parse boundary.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativePasskeyCreateStartRequest {
    #[serde(default)]
    label: Option<String>,
}

/// `create/finish` body. The label was fixed at `start` and is not accepted.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativePasskeyCreateFinishRequest {
    ceremony_id: String,
    credential: webauthn_rs::prelude::RegisterPublicKeyCredential,
}

/// `login/finish` body.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativePasskeyLoginFinishRequest {
    ceremony_id: String,
    credential: webauthn_rs::prelude::PublicKeyCredential,
}

/// Authenticated `passkeys/native/register/finish` body.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativePasskeyRegisterFinishRequest {
    ceremony_id: String,
    credential: webauthn_rs::prelude::RegisterPublicKeyCredential,
    #[serde(default)]
    label: Option<String>,
}

/// Every `start` answers with this: the body-borne ceremony id and the
/// WebAuthn options for the authenticator.
#[derive(Serialize)]
struct NativePasskeyStartResponse<T: Serialize> {
    ceremony_id: String,
    expires_in_secs: u64,
    public_key: T,
}

/// Every `finish` that issues a session answers with this: the
/// `NativeTokenResponse` shape plus the account's binding state. The token is
/// a SECRET: it appears in this body only, never in a log or audit row, and
/// only its hash reached the database.
#[derive(Serialize)]
struct NativePasskeySessionResponse {
    access_token: String,
    token_type: &'static str,
    expires_in_secs: i64,
    account_id: String,
    binding_state: &'static str,
}

fn no_store(mut response: axum::response::Response) -> axum::response::Response {
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    response
}

fn start_response<T: Serialize>(ceremony_id: String, public_key: T) -> axum::response::Response {
    no_store(
        Json(NativePasskeyStartResponse {
            ceremony_id,
            expires_in_secs: trace_commons_server::account_passkey::CEREMONY_TTL.as_secs(),
            public_key,
        })
        .into_response(),
    )
}

fn session_response(
    tenant_id: &str,
    secret: &str,
    account_id: uuid::Uuid,
    binding: AccountBindingState,
) -> axum::response::Response {
    no_store(
        Json(NativePasskeySessionResponse {
            access_token: native_token_value(tenant_id, secret),
            token_type: "Bearer",
            expires_in_secs: NATIVE_SESSION_TTL_HOURS * 3600,
            account_id: account_id.to_string(),
            binding_state: binding.label(),
        })
        .into_response(),
    )
}

/// Per-IP and global limits for one route (`bucket`). True when allowed.
fn within_rate_limits(headers: &HeaderMap, bucket: &str) -> bool {
    let client_ip = client_ip_for_rate_limit(headers);
    ACCOUNT_RATE_LIMITER.check(
        &format!("native-passkey-{bucket}-ip:{client_ip}"),
        NATIVE_PASSKEY_PER_IP_LIMIT,
    ) && ACCOUNT_RATE_LIMITER.check_global(
        &format!("native-passkey-{bucket}-global"),
        NATIVE_PASSKEY_GLOBAL_LIMIT,
    )
}

/// The per-ceremony-id ceiling. The limiter key is the id's hash, so the
/// limiter never holds an id, and an over-long id never reaches it.
fn ceremony_attempt_allowed(ceremony_id: &str) -> bool {
    !ceremony_id.is_empty()
        && ceremony_id.len() <= NATIVE_PASSKEY_CEREMONY_ID_MAX_LEN
        && ACCOUNT_RATE_LIMITER.check(
            &format!("native-passkey-ceremony:{}", hash_secret(ceremony_id)),
            NATIVE_PASSKEY_PER_CEREMONY_LIMIT,
        )
}

/// The ceiling check `create/start` makes: closed when unset, when the count
/// cannot be read, or when the count has reached the ceiling.
async fn unbound_ceiling_open(state: &AppState) -> bool {
    let ceiling = &state.account_unbound_ceiling;
    if ceiling.limit().is_none() {
        return false;
    }
    let Ok(db) = account_db(state) else {
        return false;
    };
    match db.count_unbound_passkey_accounts().await {
        Ok(unbound) => ceiling.check(unbound) == CeilingCheck::Open,
        Err(_) => false,
    }
}

/// `POST /v1/account/native/passkey/create/start` — begin creating a passkey
/// and the account it will sign in to. UNAUTHENTICATED. Writes no row: the
/// account id drawn here lives only in the in-memory ceremony until `finish`.
pub(crate) async fn native_passkey_create_start_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Result<Json<NativePasskeyCreateStartRequest>, axum::extract::rejection::JsonRejection>,
) -> axum::response::Response {
    if !within_rate_limits(&headers, "create-start") {
        return native_generic_deny();
    }
    let Ok(Json(body)) = body else {
        return native_generic_deny();
    };
    let Ok(label) = normalize_passkey_label(body.label.as_deref()) else {
        return native_generic_deny();
    };
    if !unbound_ceiling_open(state.as_ref()).await {
        return native_generic_deny();
    }
    let Ok(webauthn) = account_webauthn(state.as_ref()) else {
        return native_generic_deny();
    };

    // The account id is the WebAuthn user handle and can never change, so it
    // is drawn now. The label, when given, is the name the contributor's own
    // device shows for this passkey; otherwise the fixed generic label.
    let account_id = uuid::Uuid::new_v4();
    let name = label.as_deref().unwrap_or(ACCOUNT_PASSKEY_USER_LABEL);
    let Ok((challenge, reg_state)) =
        webauthn.start_passkey_registration(account_id, name, name, None)
    else {
        return native_generic_deny();
    };
    let ceremony_id = trace_commons_server::account_passkey::new_ceremony_id();
    account_ceremony_store(state.as_ref()).put(
        ceremony_id.clone(),
        CeremonyState::NativeCreate {
            reg_state,
            account_id,
            label,
        },
    );
    start_response(ceremony_id, challenge)
}

/// `POST /v1/account/native/passkey/create/finish` — verify the attestation
/// and create the account. UNAUTHENTICATED, behind the timing floor.
pub(crate) async fn native_passkey_create_finish_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Result<Json<NativePasskeyCreateFinishRequest>, axum::extract::rejection::JsonRejection>,
) -> axum::response::Response {
    let start = std::time::Instant::now();
    let response = native_passkey_create_finish_inner(state, headers, body).await;
    sleep_to_redeem_floor(start).await;
    response
}

async fn native_passkey_create_finish_inner(
    state: Arc<AppState>,
    headers: HeaderMap,
    body: Result<Json<NativePasskeyCreateFinishRequest>, axum::extract::rejection::JsonRejection>,
) -> axum::response::Response {
    if !within_rate_limits(&headers, "create-finish") {
        return native_generic_deny();
    }
    let Ok(Json(body)) = body else {
        return native_generic_deny();
    };
    if !ceremony_attempt_allowed(&body.ceremony_id) {
        return native_generic_deny();
    }
    // SINGLE USE, and only a creation ceremony: a browser registration, a
    // sign-in of either surface, or an authenticated native registration is
    // consumed and refused.
    let Some(CeremonyState::NativeCreate {
        reg_state,
        account_id,
        label,
    }) = account_ceremony_store(state.as_ref()).take(&body.ceremony_id)
    else {
        return native_generic_deny();
    };
    let Some(ceiling) = state.account_unbound_ceiling.limit() else {
        return native_generic_deny();
    };
    let Ok(webauthn) = account_webauthn(state.as_ref()) else {
        return native_generic_deny();
    };
    let Ok(passkey) = webauthn.finish_passkey_registration(&body.credential, &reg_state) else {
        return native_generic_deny();
    };
    let credential_id = credential_id_to_string(passkey.cred_id());
    let Ok(passkey_json) = serde_json::to_value(&passkey) else {
        return native_generic_deny();
    };
    let Ok(db) = account_db(state.as_ref()) else {
        return native_generic_deny();
    };
    // The per-IP daily cap, counted in successful creations. The slot is
    // taken before the write so concurrent finishes from one address cannot
    // overshoot, and given back (by dropping the reservation) on any refusal
    // below.
    let Some(reservation) = state.account_native_creation_cap.try_reserve(
        &client_ip_for_rate_limit(&headers),
        std::time::Instant::now(),
    ) else {
        return native_generic_deny();
    };

    // Server-minted, from the OS RNG, after the attestation verified. The
    // same generator and namespace as a NEAR AI login, so the account sits on
    // the account-admission path, where with no anchor it fails closed.
    let tenant_id = trace_commons_server::near_account_identity::random_near_ai_tenant_id();
    let secret = generate_session_secret();
    let token_hash = hash_secret(&secret);
    let outcome = db
        .create_passkey_origin_account(NewPasskeyOriginAccount {
            tenant_id: &tenant_id,
            account_id,
            credential_id: &credential_id,
            passkey: &passkey_json,
            label: label.as_deref(),
            session: trace_commons_server::db::NewSession {
                token_hash: &token_hash,
                client_kind: NATIVE_SESSION_CLIENT_KIND,
                expires_at: Utc::now() + Duration::hours(NATIVE_SESSION_TTL_HOURS),
            },
            ceiling,
        })
        .await;
    match outcome {
        Ok(PasskeyOriginAccountOutcome::Created) => reservation.commit(),
        Ok(PasskeyOriginAccountOutcome::CeilingReached) => {
            state.account_unbound_ceiling.note_reached();
            return native_generic_deny();
        }
        Err(_) => return native_generic_deny(),
    }
    session_response(
        &tenant_id,
        &secret,
        account_id,
        AccountBindingState::Unbound,
    )
}

/// `POST /v1/account/native/passkey/login/start` — begin a native
/// discoverable sign-in. UNAUTHENTICATED. Like the browser start it has no
/// timing floor: it looks nothing up, so there is no oracle to erase.
pub(crate) async fn native_passkey_login_start_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> axum::response::Response {
    if !within_rate_limits(&headers, "login-start") {
        return native_generic_deny();
    }
    let Ok(webauthn) = account_webauthn(state.as_ref()) else {
        return native_generic_deny();
    };
    let Ok((challenge, auth_state)) = webauthn.start_discoverable_authentication() else {
        return native_generic_deny();
    };
    let ceremony_id = trace_commons_server::account_passkey::new_ceremony_id();
    account_ceremony_store(state.as_ref()).put(
        ceremony_id.clone(),
        CeremonyState::NativeDiscoverable(auth_state),
    );
    start_response(ceremony_id, challenge)
}

/// `POST /v1/account/native/passkey/login/finish` — verify the assertion and
/// issue a WEAK native session. UNAUTHENTICATED, behind the timing floor.
pub(crate) async fn native_passkey_login_finish_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Result<Json<NativePasskeyLoginFinishRequest>, axum::extract::rejection::JsonRejection>,
) -> axum::response::Response {
    let start = std::time::Instant::now();
    let response = native_passkey_login_finish_inner(state, headers, body).await;
    sleep_to_redeem_floor(start).await;
    response
}

async fn native_passkey_login_finish_inner(
    state: Arc<AppState>,
    headers: HeaderMap,
    body: Result<Json<NativePasskeyLoginFinishRequest>, axum::extract::rejection::JsonRejection>,
) -> axum::response::Response {
    if !within_rate_limits(&headers, "login-finish") {
        return native_generic_deny();
    }
    let Ok(Json(body)) = body else {
        return native_generic_deny();
    };
    if !ceremony_attempt_allowed(&body.ceremony_id) {
        return native_generic_deny();
    }
    let Ok(webauthn) = account_webauthn(state.as_ref()) else {
        return native_generic_deny();
    };
    let Ok(db) = account_db(state.as_ref()) else {
        return native_generic_deny();
    };
    let Some(CeremonyState::NativeDiscoverable(auth_state)) =
        account_ceremony_store(state.as_ref()).take(&body.ceremony_id)
    else {
        return native_generic_deny();
    };
    let Some(VerifiedPasskeyAssertion {
        tenant,
        account_id,
        credential_id,
    }) =
        verify_discoverable_passkey_assertion(&webauthn, db.as_ref(), &body.credential, auth_state)
            .await
    else {
        return native_generic_deny();
    };
    // A closed passkey account (S3's existing-account branch) gets no session.
    let binding = match db.account_binding_state(&tenant, account_id).await {
        Ok(AccountBindingState::Closed) | Err(_) => return native_generic_deny(),
        Ok(binding) => binding,
    };

    // WEAK, by construction: `client_kind = 'native'` is the only kind
    // `resolve_account_ctx_native` accepts, and it pins the context weak, so
    // this token can never change an authenticator or a payout. The
    // credential is recorded so its removal revokes this session.
    let secret = generate_session_secret();
    let token_hash = hash_secret(&secret);
    if db
        .issue_passkey_session(
            &tenant,
            account_id,
            trace_commons_server::db::NewSession {
                token_hash: &token_hash,
                client_kind: NATIVE_SESSION_CLIENT_KIND,
                expires_at: Utc::now() + Duration::hours(NATIVE_SESSION_TTL_HOURS),
            },
            &credential_id,
            trace_commons_server::db::RedeemAudit {
                action: "account_passkey_native_login".to_string(),
                outcome: "success".to_string(),
                metadata: serde_json::json!({ "client_kind": NATIVE_SESSION_CLIENT_KIND }),
            },
        )
        .await
        .is_err()
    {
        return native_generic_deny();
    }
    session_response(&tenant, &secret, account_id, binding)
}

/// The authenticated native routes take a native session only. A browser
/// cookie session registers through the cookie-bound browser routes.
fn require_native_session(ctx: &AccountCtx) -> ApiResult<()> {
    if matches!(ctx.auth_method, AccountAuthMethod::NativeToken) {
        Ok(())
    } else {
        Err(api_error(StatusCode::FORBIDDEN, "native session required"))
    }
}

/// `POST /v1/account/passkeys/native/register/start` — begin adding a passkey
/// to the signed-in account from a native session. The Slice 3a gate applies:
/// a weak session may add the account's FIRST strong authenticator only.
pub(crate) async fn account_passkey_native_register_start_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
) -> ApiResult<axum::response::Response> {
    require_native_session(&ctx)?;
    // Before the gate, which reads the database and may write an audit row:
    // a start that is going to be refused anyway costs nothing past here.
    if !ACCOUNT_RATE_LIMITER.check_principal(
        &format!(
            "native-passkey-register-start-account:{}",
            ctx.account_id.as_uuid()
        ),
        NATIVE_PASSKEY_REGISTER_PER_ACCOUNT_LIMIT,
    ) {
        return Err(api_error(StatusCode::TOO_MANY_REQUESTS, "rate limited"));
    }
    require_authenticator_change_allowed(state.as_ref(), &ctx).await?;

    let webauthn = account_webauthn(state.as_ref())?;
    let db = account_db(state.as_ref())?;
    let existing = db
        .list_account_credentials(&ctx.tenant_id, ctx.account_id.as_uuid())
        .await
        .map_err(internal_error)?;
    let mut exclude = Vec::with_capacity(existing.len());
    for cred in &existing {
        exclude.push(credential_id_from_string(&cred.credential_id).map_err(internal_error)?);
    }
    let exclude = (!exclude.is_empty()).then_some(exclude);
    let (challenge, reg_state) = webauthn
        .start_passkey_registration(
            ctx.account_id.as_uuid(),
            ACCOUNT_PASSKEY_USER_LABEL,
            ACCOUNT_PASSKEY_USER_LABEL,
            exclude,
        )
        .map_err(|_| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not start passkey registration",
            )
        })?;
    let ceremony_id = trace_commons_server::account_passkey::new_ceremony_id();
    account_ceremony_store(state.as_ref()).put(
        ceremony_id.clone(),
        CeremonyState::NativeRegistration {
            reg_state,
            tenant_id: ctx.tenant_id.clone(),
            account_id: ctx.account_id.as_uuid(),
        },
    );
    db.append_account_audit(
        &ctx.tenant_id,
        "account_passkey_register_started",
        &ctx.actor_ref,
        "success",
        serde_json::json!({ "existing_credentials": existing.len() }),
    )
    .await
    .map_err(internal_error)?;
    Ok(start_response(ceremony_id, challenge))
}

/// `POST /v1/account/passkeys/native/register/finish` — complete it. The gate
/// runs again, and the ceremony must belong to this session's account.
pub(crate) async fn account_passkey_native_register_finish_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
    Json(body): Json<NativePasskeyRegisterFinishRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    require_native_session(&ctx)?;
    require_authenticator_change_allowed(state.as_ref(), &ctx).await?;

    let no_ceremony = || api_error(StatusCode::BAD_REQUEST, "no passkey ceremony in progress");
    if body.ceremony_id.len() > NATIVE_PASSKEY_CEREMONY_ID_MAX_LEN {
        return Err(no_ceremony());
    }
    let reg_state = match account_ceremony_store(state.as_ref()).take(&body.ceremony_id) {
        Some(CeremonyState::NativeRegistration {
            reg_state,
            tenant_id,
            account_id,
        }) if tenant_id == ctx.tenant_id && account_id == ctx.account_id.as_uuid() => reg_state,
        _ => return Err(no_ceremony()),
    };
    let label = normalize_passkey_label(body.label.as_deref())
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "invalid passkey label"))?;
    let webauthn = account_webauthn(state.as_ref())?;
    let passkey = webauthn
        .finish_passkey_registration(&body.credential, &reg_state)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "passkey registration failed"))?;
    let credential_id = credential_id_to_string(passkey.cred_id());
    let passkey_json = serde_json::to_value(&passkey).map_err(internal_error)?;

    let db = account_db(state.as_ref())?;
    db.insert_webauthn_credential(
        &ctx.tenant_id,
        ctx.account_id.as_uuid(),
        &credential_id,
        &passkey_json,
        label.as_deref(),
    )
    .await
    .map_err(internal_error)?;
    db.append_account_audit(
        &ctx.tenant_id,
        "account_passkey_enrolled",
        &ctx.actor_ref,
        "success",
        serde_json::json!({ "labeled": label.is_some() }),
    )
    .await
    .map_err(internal_error)?;
    Ok(Json(serde_json::json!({ "credential_id": credential_id })))
}
