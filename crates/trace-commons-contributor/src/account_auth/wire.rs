//! The native sign-in wire constants and PKCE generation.
//!
//! Private in shipped builds. The test credential-store feature exposes this
//! module so the separate client/server contract suite can check the exact
//! client implementation without making ordinary client tests build the server.

use std::time::Duration;

use anyhow::Result;
use base64::Engine;
use sha2::{Digest, Sha256};

/// The loopback path the server accepts, and the ONLY one it accepts.
pub const NATIVE_REDIRECT_PATH: &str = "/trace-commons/native-auth/callback";

/// PKCE method. The server refuses `plain`.
pub const CODE_CHALLENGE_METHOD: &str = "S256";

/// How long to wait for the human to finish in their browser. Matches the
/// server's pending-request TTL, so the listener never outlives the flow it is
/// serving.
pub const LOOPBACK_WAIT: Duration = Duration::from_secs(5 * 60);

/// A 43-character PKCE verifier: 32 CSPRNG bytes, unpadded base64url, which is
/// exactly RFC 7636's minimum length and its unreserved alphabet.
pub fn generate_verifier() -> Result<String> {
    let mut bytes = [0u8; 32];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
        .map_err(|_| anyhow::anyhow!("could not generate a sign-in verifier"))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

pub fn challenge_for_verifier(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}
