//! Admin authentication and invite lifecycle routes for the upload-claim
//! issuer.
//!
//! Unlike `/v1/admin/allowlist-status`, these routes mint and revoke
//! credentials, so they are gated on an EdDSA admin JWT rather than on
//! loopback binding alone.

use jsonwebtoken::errors::ErrorKind as JwtErrorKind;
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminClaims {
    pub sub: String,
    pub role: String,
    pub iss: String,
    pub aud: String,
    pub jti: String,
    pub exp: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminAuthError {
    Malformed,
    WrongAlgorithm,
    NotAdmin,
    Expired,
    Invalid,
}

impl AdminAuthError {
    /// Public label. Deliberately coarse: an unauthenticated caller learns
    /// only that they are not authorized, never why.
    pub fn public_label(self) -> &'static str {
        match self {
            Self::NotAdmin => "AdminRoleRequired",
            _ => "AdminTokenInvalid",
        }
    }
}

/// Verify an EdDSA admin token minted by this issuer's own signing key.
/// Rejects any algorithm other than EdDSA before touching the signature, so a
/// caller cannot downgrade to `none` or to an HMAC the public key would
/// satisfy.
pub fn verify_admin_token(
    token: &str,
    decoding_key: &DecodingKey,
    expected_iss: &str,
    expected_aud: &str,
) -> Result<AdminClaims, AdminAuthError> {
    let header = jsonwebtoken::decode_header(token).map_err(|_| AdminAuthError::Malformed)?;
    if header.alg != Algorithm::EdDSA {
        return Err(AdminAuthError::WrongAlgorithm);
    }
    let mut validation = Validation::new(Algorithm::EdDSA);
    validation.set_issuer(&[expected_iss]);
    validation.set_audience(&[expected_aud]);
    validation.set_required_spec_claims(&["exp", "iss", "aud"]);

    let decoded = jsonwebtoken::decode::<AdminClaims>(token, decoding_key, &validation).map_err(
        |e| match e.kind() {
            JwtErrorKind::ExpiredSignature => AdminAuthError::Expired,
            JwtErrorKind::Base64(_) | JwtErrorKind::Json(_) => AdminAuthError::Malformed,
            _ => AdminAuthError::Invalid,
        },
    )?;

    if decoded.claims.role != "admin" {
        return Err(AdminAuthError::NotAdmin);
    }
    Ok(decoded.claims)
}

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde_json::{Value, json};

use crate::db::postgres::PgBackend;
use crate::db::{InviteGrantInsertOutcome, InviteGrantWrite};
use crate::trace_invite_registry::{
    DbInviteRegistry, InviteEntry, InviteRegistry, InviteTenantMode, generate_invite_code,
};
use crate::trace_upload_claim_allowlist::hash_invite_code;

#[derive(Clone)]
pub struct InviteAdminState {
    pub backend: Arc<PgBackend>,
    pub registry: Arc<DbInviteRegistry>,
    pub decoding_key: Arc<DecodingKey>,
    pub expected_iss: String,
    pub expected_aud: String,
    pub default_policy_label: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateInviteRequest {
    pub tenant_mode: String,
    #[serde(default)]
    pub fixed_tenant_id: Option<String>,
    #[serde(default)]
    pub tenant_template_id: Option<String>,
    #[serde(default = "default_policy_version")]
    pub policy_version: String,
    #[serde(default)]
    pub allowed_consent_scopes: Vec<String>,
    #[serde(default)]
    pub allowed_uses: Vec<String>,
    #[serde(default = "default_max_uses")]
    pub max_uses: u32,
    /// Relative expiry. Absent means no expiry.
    #[serde(default)]
    pub expires_in_days: Option<i64>,
    #[serde(default = "default_issuance_source")]
    pub issuance_source: String,
    #[serde(default)]
    pub issued_by_label: Option<String>,
    #[serde(default)]
    pub credential_binding_hash: Option<String>,
    #[serde(default)]
    pub note_label: Option<String>,
}

fn default_policy_version() -> String {
    "v1".to_string()
}
fn default_max_uses() -> u32 {
    3
}
fn default_issuance_source() -> String {
    "operator".to_string()
}

pub fn invite_admin_router(state: InviteAdminState) -> Router {
    Router::new()
        .route(
            "/v1/admin/invites",
            post(create_invite_handler).get(list_invites_handler),
        )
        .route(
            "/v1/admin/invites/{hash}/revoke",
            post(revoke_invite_handler),
        )
        .route(
            "/v1/admin/invite-registry-status",
            get(registry_status_handler),
        )
        .with_state(state)
}

fn authorize(
    state: &InviteAdminState,
    headers: &HeaderMap,
) -> Result<(), (StatusCode, Json<Value>)> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|t| !t.is_empty());
    let Some(token) = token else {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "AdminTokenMissing" })),
        ));
    };
    match verify_admin_token(
        token,
        &state.decoding_key,
        &state.expected_iss,
        &state.expected_aud,
    ) {
        Ok(_) => Ok(()),
        Err(AdminAuthError::NotAdmin) => Err((
            StatusCode::FORBIDDEN,
            Json(json!({ "error": AdminAuthError::NotAdmin.public_label() })),
        )),
        Err(e) => Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": e.public_label() })),
        )),
    }
}

async fn create_invite_handler(
    State(state): State<InviteAdminState>,
    headers: HeaderMap,
    Json(request): Json<CreateInviteRequest>,
) -> (StatusCode, Json<Value>) {
    if let Err((status, body)) = authorize(&state, &headers) {
        return (status, body);
    }

    let (tenant_mode, fixed_tenant_id, tenant_template_id) = match (
        request.tenant_mode.as_str(),
        &request.fixed_tenant_id,
        &request.tenant_template_id,
    ) {
        ("fixed", Some(t), None) => (InviteTenantMode::Fixed, Some(t.clone()), None),
        ("derived", None, Some(t)) => (InviteTenantMode::Derived, None, Some(t.clone())),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "InviteTenantModeMalformed" })),
            );
        }
    };

    if request.max_uses == 0 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "InviteMaxUsesMalformed" })),
        );
    }
    if let Some(h) = &request.credential_binding_hash {
        if !is_canonical_sha256(h) {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "InviteCredentialBindingMalformed" })),
            );
        }
    }

    let expires_at: Option<DateTime<Utc>> = request
        .expires_in_days
        .map(|d| Utc::now() + ChronoDuration::days(d));

    // The raw code exists here and in exactly one response body. It is never
    // stored, logged, or retrievable afterward.
    let code = generate_invite_code();
    let invite_subject_hash = hash_invite_code(&code);

    let write = InviteGrantWrite {
        invite_subject_hash: invite_subject_hash.clone(),
        policy_label: state.default_policy_label.clone(),
        tenant_mode,
        fixed_tenant_id: fixed_tenant_id.clone(),
        tenant_template_id: tenant_template_id.clone(),
        policy_version: request.policy_version.clone(),
        allowed_consent_scopes: request.allowed_consent_scopes.clone(),
        allowed_uses: request.allowed_uses.clone(),
        max_uses: request.max_uses,
        expires_at,
        issuance_source: request.issuance_source.clone(),
        issued_by_label: request.issued_by_label.clone(),
        credential_binding_hash: request.credential_binding_hash.clone(),
        note_label: request.note_label.clone(),
    };

    match state.backend.insert_invite_grant(write).await {
        Ok(InviteGrantInsertOutcome::Inserted) => {
            // Invalidate AFTER the commit so the cache never advertises an
            // invite the database rejected.
            state.registry.note_write(InviteEntry {
                invite_subject_hash: invite_subject_hash.clone(),
                policy_label: state.default_policy_label.clone(),
                tenant_mode,
                fixed_tenant_id,
                tenant_template_id,
                policy_version: request.policy_version,
                allowed_consent_scopes: request.allowed_consent_scopes,
                allowed_uses: request.allowed_uses,
                max_uses: request.max_uses,
                expires_at,
                issuance_source: request.issuance_source,
                issued_by_label: request.issued_by_label,
                credential_binding_hash: request.credential_binding_hash,
                note_label: request.note_label,
                revoked_at: None,
            });
            (
                StatusCode::CREATED,
                Json(json!({
                    "invite_code": code,
                    "invite_subject_hash": invite_subject_hash,
                    "expires_at": expires_at,
                })),
            )
        }
        Ok(InviteGrantInsertOutcome::CredentialAlreadyBound) => (
            StatusCode::CONFLICT,
            Json(json!({ "error": "InviteCredentialAlreadyBound" })),
        ),
        // A hash collision on a 16-character CSPRNG code is not a real event;
        // treat it as a transient failure rather than returning a code whose
        // grant fields belong to someone else.
        Ok(InviteGrantInsertOutcome::AlreadyExists) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "InviteCodeCollision" })),
        ),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "InviteRegistryBackendUnavailable" })),
        ),
    }
}

fn is_canonical_sha256(value: &str) -> bool {
    match value.strip_prefix("sha256:") {
        Some(hex) => {
            hex.len() == 64
                && hex
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }
        None => false,
    }
}

async fn list_invites_handler(
    State(state): State<InviteAdminState>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    if let Err((status, body)) = authorize(&state, &headers) {
        return (status, body);
    }
    match state.backend.list_invite_grants().await {
        Ok(entries) => {
            let items: Vec<Value> = entries
                .iter()
                .map(|e| {
                    json!({
                        "invite_subject_hash": e.invite_subject_hash,
                        "policy_label": e.policy_label,
                        "tenant_mode": match e.tenant_mode {
                            InviteTenantMode::Fixed => "fixed",
                            InviteTenantMode::Derived => "derived",
                        },
                        "policy_version": e.policy_version,
                        "max_uses": e.max_uses,
                        "expires_at": e.expires_at,
                        "issuance_source": e.issuance_source,
                        "issued_by_label": e.issued_by_label,
                        "note_label": e.note_label,
                        "credential_bound": e.credential_binding_hash.is_some(),
                    })
                })
                .collect();
            (
                StatusCode::OK,
                Json(json!({ "count": items.len(), "invites": items })),
            )
        }
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "InviteRegistryBackendUnavailable" })),
        ),
    }
}

async fn revoke_invite_handler(
    State(state): State<InviteAdminState>,
    headers: HeaderMap,
    Path(hash): Path<String>,
) -> (StatusCode, Json<Value>) {
    if let Err((status, body)) = authorize(&state, &headers) {
        return (status, body);
    }
    if !is_canonical_sha256(&hash) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "InviteSubjectHashMalformed" })),
        );
    }
    match state.backend.revoke_invite_grant(&hash).await {
        Ok(revoked) => {
            // Drop it from the cache either way: if it was already revoked,
            // the cache must not be the thing still advertising it.
            state.registry.note_revoke(&hash);
            (StatusCode::OK, Json(json!({ "revoked": revoked })))
        }
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "InviteRegistryBackendUnavailable" })),
        ),
    }
}

async fn registry_status_handler(
    State(state): State<InviteAdminState>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    if let Err((status, body)) = authorize(&state, &headers) {
        return (status, body);
    }
    let status = state.registry.status();
    (
        StatusCode::OK,
        Json(json!({
            "live": status.live,
            "cache_age_seconds": status.cache_age_seconds,
            "stale": status.stale,
            "max_stale_seconds": status.max_stale_seconds,
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, encode};

    const ISS: &str = "trace-commons-upload-claim-issuer";
    const AUD: &str = "trace-commons-issuer-admin";

    /// Ed25519 PKCS#8 v2 test keypair. Generated once for tests only; never
    /// used by any deployment.
    fn test_keys() -> (EncodingKey, DecodingKey) {
        let (private_pem, public_pem) = generate_test_ed25519_pem();
        (
            EncodingKey::from_ed_pem(private_pem.as_bytes()).expect("encoding key"),
            DecodingKey::from_ed_pem(public_pem.as_bytes()).expect("decoding key"),
        )
    }

    fn sign(enc: &EncodingKey, role: &str, exp_offset_secs: i64) -> String {
        let exp = (chrono::Utc::now().timestamp() + exp_offset_secs) as usize;
        let claims = AdminClaims {
            sub: "operator-1".to_string(),
            role: role.to_string(),
            iss: ISS.to_string(),
            aud: AUD.to_string(),
            jti: "jti-1".to_string(),
            exp,
        };
        encode(&Header::new(Algorithm::EdDSA), &claims, enc).expect("sign")
    }

    #[test]
    fn an_admin_role_token_verifies() {
        let (enc, dec) = test_keys();
        let token = sign(&enc, "admin", 300);
        let claims = verify_admin_token(&token, &dec, ISS, AUD).expect("verifies");
        assert_eq!(claims.role, "admin");
        assert_eq!(claims.sub, "operator-1");
    }

    #[test]
    fn a_non_admin_role_is_refused() {
        let (enc, dec) = test_keys();
        let token = sign(&enc, "reviewer", 300);
        assert!(matches!(
            verify_admin_token(&token, &dec, ISS, AUD),
            Err(AdminAuthError::NotAdmin)
        ));
    }

    #[test]
    fn an_expired_token_is_refused() {
        let (enc, dec) = test_keys();
        let token = sign(&enc, "admin", -300);
        assert!(matches!(
            verify_admin_token(&token, &dec, ISS, AUD),
            Err(AdminAuthError::Expired)
        ));
    }

    #[test]
    fn a_wrong_audience_token_is_refused() {
        let (enc, dec) = test_keys();
        let token = sign(&enc, "admin", 300);
        assert!(matches!(
            verify_admin_token(&token, &dec, ISS, "some-other-audience"),
            Err(AdminAuthError::Invalid)
        ));
    }

    #[test]
    fn a_wrong_issuer_token_is_refused() {
        let (enc, dec) = test_keys();
        let token = sign(&enc, "admin", 300);
        assert!(matches!(
            verify_admin_token(&token, &dec, "someone-else", AUD),
            Err(AdminAuthError::Invalid)
        ));
    }

    #[test]
    fn a_garbage_token_is_refused_without_panicking() {
        let (_, dec) = test_keys();
        assert!(matches!(
            verify_admin_token("not-a-jwt", &dec, ISS, AUD),
            Err(AdminAuthError::Malformed)
        ));
    }

    #[test]
    fn a_wrong_algorithm_token_is_refused() {
        // Classic algorithm-confusion forgery: sign with HS256 using the
        // Ed25519 public key PEM bytes as the HMAC secret, which is key
        // material an attacker who only has the public key already holds.
        let (_private_pem, public_pem) = generate_test_ed25519_pem();
        let dec = DecodingKey::from_ed_pem(public_pem.as_bytes()).expect("decoding key");
        let hs_key = EncodingKey::from_secret(public_pem.as_bytes());
        let exp = (chrono::Utc::now().timestamp() + 300) as usize;
        let claims = AdminClaims {
            sub: "operator-1".to_string(),
            role: "admin".to_string(),
            iss: ISS.to_string(),
            aud: AUD.to_string(),
            jti: "jti-1".to_string(),
            exp,
        };
        let token = encode(&Header::new(Algorithm::HS256), &claims, &hs_key).expect("sign hs256");
        assert!(matches!(
            verify_admin_token(&token, &dec, ISS, AUD),
            Err(AdminAuthError::WrongAlgorithm)
        ));
    }

    #[test]
    fn a_token_signed_by_a_different_key_is_refused() {
        // Well-formed admin claims, correct iss/aud/exp, but signed by an
        // independent keypair. Proves the role check cannot substitute for
        // signature verification.
        let (_enc1, dec1) = test_keys();
        let (enc2, _dec2) = test_keys();
        let token = sign(&enc2, "admin", 300);
        assert!(matches!(
            verify_admin_token(&token, &dec1, ISS, AUD),
            Err(AdminAuthError::Invalid)
        ));
    }

    /// Ring generates PKCS#8 v2 Ed25519 keys, which is what the issuer's own
    /// key loading requires.
    fn generate_test_ed25519_pem() -> (String, String) {
        use ring::signature::{Ed25519KeyPair, KeyPair};
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).expect("generate");
        let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).expect("parse");
        let private_pem = pem_wrap("PRIVATE KEY", pkcs8.as_ref());
        // SubjectPublicKeyInfo prefix for Ed25519.
        let mut spki = vec![
            0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
        ];
        spki.extend_from_slice(pair.public_key().as_ref());
        let public_pem = pem_wrap("PUBLIC KEY", &spki);
        (private_pem, public_pem)
    }

    fn pem_wrap(label: &str, der: &[u8]) -> String {
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::STANDARD.encode(der);
        let body = b64
            .as_bytes()
            .chunks(64)
            .map(|c| String::from_utf8_lossy(c).to_string())
            .collect::<Vec<_>>()
            .join("\n");
        format!("-----BEGIN {label}-----\n{body}\n-----END {label}-----\n")
    }

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn creating_an_invite_without_a_token_is_refused() {
        let Some(state) = test_invite_admin_state().await else {
            eprintln!("skipping: no test database configured");
            return;
        };
        let app = invite_admin_router(state.inner);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/admin/invites")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"tenant_mode":"derived","tenant_template_id":"tmpl-1"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn a_created_invite_is_returned_once_and_is_immediately_live() {
        let Some(state) = test_invite_admin_state().await else {
            eprintln!("skipping: no test database configured");
            return;
        };
        let registry = state.registry_handle();
        let token = state.test_admin_token();
        let app = invite_admin_router(state.inner);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/admin/invites")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"tenant_mode":"derived","tenant_template_id":"tmpl-1",
                            "policy_version":"v1","max_uses":3,
                            "allowed_consent_scopes":["model_training"],
                            "allowed_uses":["research"]}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);

        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let code = json["invite_code"]
            .as_str()
            .expect("raw code returned once");
        assert_eq!(code.len(), 16);

        // The registry must already know it: no refresh window.
        let hash = crate::trace_upload_claim_allowlist::hash_invite_code(code);
        assert_eq!(json["invite_subject_hash"], hash);
        assert!(
            registry.lookup(&hash).expect("lookup").is_some(),
            "a minted invite must be immediately redeemable"
        );
    }

    #[tokio::test]
    async fn listing_never_returns_raw_codes() {
        let Some(state) = test_invite_admin_state().await else {
            eprintln!("skipping: no test database configured");
            return;
        };
        let token = state.test_admin_token();
        let app = invite_admin_router(state.inner);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/admin/invites")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(
            !text.contains("invite_code"),
            "listing must never carry a raw code: {text}"
        );
    }

    #[tokio::test]
    async fn revoking_removes_the_invite_from_the_registry() {
        let Some(state) = test_invite_admin_state().await else {
            eprintln!("skipping: no test database configured");
            return;
        };
        let registry = state.registry_handle();
        let token = state.test_admin_token();
        let app = invite_admin_router(state.inner);

        let created = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/admin/invites")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"tenant_mode":"derived","tenant_template_id":"tmpl-1",
                            "policy_version":"v1"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(created.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let hash = json["invite_subject_hash"].as_str().unwrap().to_string();
        assert!(registry.lookup(&hash).unwrap().is_some());

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/admin/invites/{hash}/revoke"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            registry.lookup(&hash).unwrap().is_none(),
            "revoke must take effect with no refresh window"
        );
    }

    /// Builds real state against the test database. Returns None when no test
    /// database is configured.
    async fn test_invite_admin_state() -> Option<TestInviteAdminState> {
        use crate::config::{DatabaseConfig, SslMode};
        use crate::db::Database;
        use crate::db::postgres::PgBackend;
        use crate::trace_invite_registry::DbInviteRegistry;
        use secrecy::SecretString;
        use std::sync::Arc;
        use std::time::Duration;

        let url = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .ok()?;
        let config = DatabaseConfig {
            url: SecretString::from(url.clone()),
            invite_registry_url: Some(SecretString::from(url)),
            pool_size: 4,
            ssl_mode: SslMode::Prefer,
            login_resolver_url: None,
            gate_driver_url: None,
            pii_backstop_driver_url: None,
        };
        let backend = Arc::new(PgBackend::new(&config).await.ok()?);
        backend.run_migrations().await.ok()?;
        let registry = Arc::new(
            DbInviteRegistry::new(
                backend.clone(),
                Duration::from_secs(60),
                Duration::from_secs(300),
            )
            .await
            .ok()?,
        );
        let (private_pem, public_pem) = generate_test_ed25519_pem();
        Some(TestInviteAdminState {
            inner: InviteAdminState {
                backend,
                registry,
                decoding_key: Arc::new(DecodingKey::from_ed_pem(public_pem.as_bytes()).ok()?),
                expected_iss: ISS.to_string(),
                expected_aud: AUD.to_string(),
                default_policy_label: "test-pool".to_string(),
            },
            signing_pem: private_pem,
        })
    }

    /// Wrapper carrying the private key so tests can mint their own tokens.
    struct TestInviteAdminState {
        inner: InviteAdminState,
        signing_pem: String,
    }

    impl TestInviteAdminState {
        fn test_admin_token(&self) -> String {
            let enc = jsonwebtoken::EncodingKey::from_ed_pem(self.signing_pem.as_bytes())
                .expect("encoding key");
            sign(&enc, "admin", 300)
        }
        fn registry_handle(
            &self,
        ) -> std::sync::Arc<crate::trace_invite_registry::DbInviteRegistry> {
            self.inner.registry.clone()
        }
    }
}
