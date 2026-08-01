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
}
